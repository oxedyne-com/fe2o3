use crate::{
    prelude::*,
    bots::{
        base::bot_deps::*,
        worker::{
            bot_reader::ReadResult,
            worker_deps::*,
        },
    },
    file::{
        floc::{
            FileLocation,
            FileNum,
        },
        state::{
            FileState,
            FileStateMap,
        },
        stored::RecordDigest,
        zdir::sync_dir,
    },
    test::hooks,
};

use oxedyne_fe2o3_core::channels::Recv;
use oxedyne_fe2o3_jdat::id::NumIdDat;

use std::{
    collections::BTreeMap,
    fs::self,
    sync::Arc,
    time::Instant,
};

// How hard a call to collect a file pushes.  `Yes` waives the trigger fraction, `gc_on` and
// `auto_gc`, and nothing else.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Force {
    Yes,
    No,
}

// What stands between a file and its collection, the first that holds it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Hold {
    Live,       // still being written
    Collecting, // a collector is already at it
    Moves,      // a supersession or re-anchor is in flight
    Readers,    // a read has it open
    Undrained,  // writes not yet accounted for
    Fault,      // the evaluation failed
}

impl Hold {
    pub fn why(&self) -> &'static str {
        match self {
            Self::Live          => "live",
            Self::Collecting    => "collecting",
            Self::Moves         => "pending moves",
            Self::Readers       => "readers",
            Self::Undrained     => "undrained",
            Self::Fault         => "fault",
        }
    }
}

// What `maybe_collect` did.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Collect {
    Started,
    Deleted,
    Held(Hold),
    Declined, // the trigger, `gc_on` or `auto_gc` said no
}

// The decision, made on a borrow of the file's state and acted on after it.
enum Plan {
    Hold(Hold),
    Declined,
    Delete(usize), // the file's accounted size, to leave the shard's total
    Collect(FileState),
}

#[derive(Clone, Debug)]
pub enum GcControl {
    On(bool), // switch gc on or off
    Auto(bool), // set auto gc
    Compact, // collect every file with old bytes now; each fbot answers with a `CompactReport`
}

#[derive(Debug)]
pub struct FileBot<
    const UIDL: usize,
    UID:    NumIdDat<UIDL>,
    ENC:    Encrypter,
    KH:     Hasher,
	PR:     Hasher,
    CS:     Checksummer,
>{
    // Identity
    wind:       WorkerInd,
    wtyp:       WorkerType,
    // Bot
    sem:            Semaphore,
    errc:           Arc<Mutex<usize>>,
    log_stream_id:  String,
    // Config
    zdir:       ZoneDir,
    // Comms
    chan_in:    Simplex<OzoneMsg<UIDL, UID, ENC, KH>>,
    // API
    api:        OzoneApi<UIDL, UID, ENC, KH, PR, CS>,
    // State
    active:     bool,
    auto_gc:    bool,
    gcbuf:      BTreeMap<FileNum, Vec<OzoneMsg<UIDL, UID, ENC, KH>>>,
    gc_on:      bool,
    inited:     bool,
    states:     FileStateMap,
    trep:       Instant,
}

impl<
    const UIDL: usize,
    UID:    NumIdDat<UIDL> + 'static,
    ENC:    Encrypter + 'static,
    KH:     Hasher + 'static,
	PR:     Hasher,
    CS:     Checksummer,
>
    WorkerBot<UIDL, UID, ENC, KH, PR, CS> for FileBot<UIDL, UID, ENC, KH, PR, CS>
{
    workerbot_methods!();
}

impl<
    const UIDL: usize,
    UID:    NumIdDat<UIDL> + 'static,
    ENC:    Encrypter + 'static,
    KH:     Hasher + 'static,
	PR:     Hasher,
    CS:     Checksummer,
>
    OzoneBot<UIDL, UID, ENC, KH, PR, CS> for FileBot<UIDL, UID, ENC, KH, PR, CS>
{
    ozonebot_methods!();
}

impl<
    const UIDL: usize,
    UID:    NumIdDat<UIDL> + 'static,
    ENC:    Encrypter + 'static,
    KH:     Hasher + 'static,
	PR:     Hasher,
    CS:     Checksummer,
>
    Bot<{ BID_LEN }, Bid, OzoneMsg<UIDL, UID, ENC, KH>> for FileBot<UIDL, UID, ENC, KH, PR, CS>
{
    bot_methods!();

    fn go(&mut self) {

        sync_log::set_stream(self.log_stream_id());

        if self.no_init() { return; }
        self.now_listening();
        loop {
            if self.wind().b() < self.cfg().num_bots_per_zone((&self).wtyp()) {

                if self.trep.elapsed() > self.cfg().zone_state_update_interval() {
                    // Automated state reporting.
                    self.trep = Instant::now();
                    if let Some(zbot) = self.zbot() {
                        if let Err(e) = zbot.send(
                            OzoneMsg::ShardFileSize(self.wind().b(), self.states().get_size())
                        ) {
                            self.result(&Err(err!(e,
                                "{}: Cannot send cache size update to zbot.", self.ozid();
                                Channel, Write)));
                        }
                    }
                }
            
                if self.listen().must_end() { break; }

            } else {
                // This bot is to be terminated. Forward incoming messages to the remaining bots of
                // this type.
            }
        }
    }

    fn listen(&mut self) -> LoopBreak {
        match self.chan_in().recv_timeout(self.cfg().zone_state_update_interval()) {
            Recv::Result(Err(e)) => self.err_cannot_receive(err!(e,
                "{}: Waiting for message.", self.ozid();
                IO, Channel)),
            Recv::Result(Ok(msg)) => {
                if let Some(msg) = self.listen_worker(msg) {
                    if self.listen_work(&msg) {
                        return self.listen_cmd(msg);
                    }
                }
            },
            Recv::Empty => (),
        }    
        LoopBreak(false)
    }

}

impl<
    const UIDL: usize,
    UID:    NumIdDat<UIDL> + 'static,
    ENC:    Encrypter + 'static,
    KH:     Hasher + 'static,
	PR:     Hasher,
    CS:     Checksummer,
>
    FileBot<UIDL, UID, ENC, KH, PR, CS>
{
    pub fn new(
        args: ZoneWorkerInitArgs<UIDL, UID, ENC, KH, PR, CS>,
    )
        -> Self
    {
        Self {
            // Identity
            wind:       args.wind,
            wtyp:       args.wtyp,
            // Bot
            sem:            args.sem,
            errc:           Arc::new(Mutex::new(0)),
            log_stream_id:  args.log_stream_id,
            // Config
            zdir:       ZoneDir::default(),
            // Comms    
            chan_in:    args.chan_in,
            // API
            api:        args.api,
            // State    
            active:     false,
            auto_gc:    true,
            gcbuf:      BTreeMap::new(),
            gc_on:      false,
            inited:     false,
            states:     FileStateMap::default(),
            trep:       Instant::now(),
        }
    }

    fn states(&self)            -> &FileStateMap        { &self.states }
    fn states_mut(&mut self)    -> &mut FileStateMap    { &mut self.states }
    fn gc_buffer(&self)         -> &BTreeMap<FileNum, Vec<OzoneMsg<UIDL, UID, ENC, KH>>>        { &self.gcbuf }
    fn gc_buffer_mut(&mut self) -> &mut BTreeMap<FileNum, Vec<OzoneMsg<UIDL, UID, ENC, KH>>>    { &mut self.gcbuf }

    fn gc_auto_active(&self) -> bool { self.auto_gc }

    pub fn activate(mut self) -> Self {
        self.active = true;
        self
    }

    pub fn listen_work(
        &mut self,
        msg: &OzoneMsg<UIDL, UID, ENC, KH>,
    )
        -> bool
    {
        match msg {
            OzoneMsg::ProcessGcBuffer(msgbox) => self.process_work(&*msgbox, true),
            _ => self.process_work(msg, false),
        }

    }

    /// Returns a flag indicating whether to keep listening.
    pub fn process_work(
        &mut self,
        msg:                &OzoneMsg<UIDL, UID, ENC, KH>,
        processing_buffer:  bool,
    )
        -> bool
    {
        match msg {
            // WRITE
            OzoneMsg::ScheduleOld(floc, rid, from_id) => {
                if !processing_buffer {
                    hooks::schedule_delay();
                }
                // [17] Schedule the old file location for deletion.
                if !self.gc_active(
                    floc.file_number(),
                    &msg,
                    processing_buffer,
                ) { 
                    let result = self.schedule_deletion(floc, rid, from_id);
                    self.result(&result);
                }
            }
            OzoneMsg::UpdateData { floc_new, ilen, floc_old_opt, from_id } => {
                // [15] Add new data to the given live file state.
                let result = self.update_data(floc_new, *ilen, floc_old_opt.as_ref(), from_id);
                self.result(&result);
            }
            OzoneMsg::Settle(resp) => {
                // The `ScheduleOld` this bot forwarded are ahead of the echo in each queue it
                // goes to, this one's included.  Not held by a collection: a held `ScheduleOld` is
                // in the buffer, which is where the barrier promises it.
                let result = self.fbots().and_then(|bots| {
                    bots.send_to_all(OzoneMsg::SettleEcho(resp.clone()))
                });
                if let Err(e) = result {
                    self.error(e.clone());
                    self.respond(Err(e), resp);
                }
            }
            OzoneMsg::SettleEcho(resp) => {
                self.respond(Ok(OzoneMsg::Ok), resp);
            }
            OzoneMsg::GcAborted(fnum) => {
                // The collection replaced nothing, so the state held here is still the file's,
                // and what was buffered meanwhile applies to it as it stands.
                match self.states_mut().get_state_mut(*fnum) {
                    Ok(fstat) => {
                        fstat.set_gc(false);
                        self.gc_active(
                            *fnum,
                            &OzoneMsg::None,
                            false,
                        );
                    },
                    Err(e) => {
                        self.error(err!(e,
                            "{}: Cannot release file {} after an abandoned garbage collection \
                            because it cannot be found in the file state map.",
                            self.ozid(), fnum;
                            Bug, Missing, Data));
                        return false;
                    },
                }
            }
            OzoneMsg::GcCompleted(fnum, new_fstat, size_dec) => {
                match self.states_mut().get_state_mut(*fnum) {
                    Ok(fstat) => {
                        *fstat = new_fstat.clone();
                        fstat.set_gc(false);
                        // Process buffer.
                        self.gc_active(
                            *fnum,
                            &OzoneMsg::None,
                            false,
                        );
                    }
                    Err(e) => {
                        self.error(err!(e,
                            "{}: Cannot update file state for file {} after garbage collection \
                            because it cannot be found in the file state map.",
                            self.ozid(), fnum;
                            Bug, Missing, Data));
                        return false;
                    },
                };
                let result = self.states_mut().dec_size(*size_dec);
                self.result(&result);
            }
            // READ
            OzoneMsg::DumpFileStatesRequest(resp) => {
                // TODO send back files currently being gc'd.
                if let Err(e) = resp.send(OzoneMsg::DumpFileStatesResponse(
                    self.wind().clone(),
                    self.states().clone(),
                )) {
                    self.err_cannot_send(err!(e,
                        "{}: Responding to {:?} with file states dump.", self.ozid(), resp.ozid();
                        Data, IO, Channel));
                }
            }
            OzoneMsg::ReadFileRequest(fnum, kbyts, mloc, resp_r2) => {
                if !self.gc_active(
                    *fnum,
                    &msg,
                    processing_buffer,
                ) { 
                    // <5> Increment the reader count and send the read result, an implicit
                    // permission to read, to the rbot.
                    let (result, msg2) = match self.states_mut().get_state_mut(*fnum) {
                        Ok(fstat) => {
                            let mut mloc2 = mloc.clone();
                            let mut postgc = false;
                            // Only a file a collection has just moved records in has move entries,
                            // so the record is named only then.  A read looks up its move and
                            // leaves the entry: it is kept for the record's supersession, which
                            // finding it gone flagged whatever now sits at the old offset as old.
                            if !fstat.no_pending_moves() {
                                match RecordDigest::new(kbyts, mloc.meta()) {
                                    Ok(rid) => {
                                        let dloc = mloc2.file_location().keyval();
                                        if let Some(new_start) = fstat.moved_to(&dloc, &rid) {
                                            mloc2.new_start_position(new_start);
                                            postgc = true;
                                        }
                                    },
                                    // The reader confirms the record it reads, so an unmapped
                                    // location costs it a retry, never a wrong answer.
                                    Err(e) => error!(sync_log::stream(), err!(e,
                                        "Naming the record a read of file {} asks for.", fnum;
                                        Data, Encode)),
                                }
                            }
                            // Increment the reader count whether or not the location was remapped.
                            // The count is the pin that keeps a file from being collected while a
                            // read of it is in flight (`schedule_deletion` will not start a
                            // collection unless `no_readers`), and a `postgc` read needs that pin
                            // as much as any other: the burst that carried this record can trip the
                            // trigger again at once, and a second collection renaming the file
                            // between here and the rbot's read would leave the just-handed-back
                            // offset pointing into a superseded inode -- a checksum failure the
                            // rbot's handle drop cannot repair, because the offset itself is stale.
                            // The rbot sends `ReadFinished` on every path, so this decrements
                            // cleanly; leaving it out here was also what drove the reader count
                            // below zero when a `postgc` read reported a finish it never counted.
                            (
                                fstat.inc_readers(),
                                OzoneMsg::ReadResult(ReadResult::Location(mloc2, postgc)),
                            )
                        },
                        Err(e) => (
                            Ok(()),
                            OzoneMsg::Error(err!(e,
                                "Read file request for file {}.", fnum;
                                Bug, Missing, Data)),
                        ),
                    };
                    self.result(&result);
                    let pinned = result.is_ok();
                    // <6> Send file location back to rbot, representing permission to perform a read.
                    // A reader that gave up waiting while the request sat in a collection's buffer,
                    // or behind a slow cache bot, will never read this or send `ReadFinished` for
                    // it, and the file would keep its reader for the life of the process, neither
                    // collected nor deleted.  So the pin goes back here.
                    if !self.try_respond(Ok(msg2), resp_r2) && pinned {
                        let result = match self.states_mut().get_state_mut(*fnum) {
                            Ok(fstat) => fstat.dec_readers(),
                            Err(_) => Ok(()),
                        };
                        self.result(&result);
                    }
                }
            }
            OzoneMsg::ReadFinished(fnum) => {
                if !self.gc_active(
                    *fnum,
                    &msg,
                    processing_buffer,
                ) { 
                    // <9> Decrement the reader count now that a read has completed.  The last read
                    // finishing does not look at the file for collection (see `maybe_collect`).
                    let result = match self.states_mut().get_state_mut(*fnum) {
                        Ok(fstat) => {
                            let result = fstat.dec_readers();
                            result
                        },
                        Err(_) => {
                            warn!(sync_log::stream(), 
                                "A read completion for file {} has been received, but the file state \
                                no longer exists, ignoring.", fnum,
                            );
                            Ok(())
                        },
                    };
                    self.result(&result);
                }
            }
            _ => return true,
        }    
        false
    }

    pub fn listen_cmd(&mut self, msg: OzoneMsg<UIDL, UID, ENC, KH>) -> LoopBreak {
        match msg {
            // COMMAND
            OzoneMsg::NewFileStates(shard) => {
                self.states = shard;
            }
            //Ok(Config(cfg, tik)) => {
            //    self.cfg = cfg;
            //    let msg = OzoneMsg::ConfigConfirm(self.ozid().clone(), tik);
            //    if let Err(e) = self.sup().send(msg.clone()) {
            //        self.err_cannot_send(fmt!("{:?}", msg));
            //    }
            //},
            OzoneMsg::CloseOldLiveFileState {
                fnum_old,
                fnum_new,
                new_dat_size,
                new_ind_size,
                resp,
            } => {
                // The writer rolling over waits on this, and would otherwise wait out its deadline
                // on a failure only logged here.
                let caller = resp.clone();
                if let Err(e) = self.close_old_live_file_state(
                    fnum_old,
                    fnum_new,
                    new_dat_size,
                    new_ind_size,
                    resp,
                ) {
                    self.error(e.clone());
                    self.respond(Err(e), &caller);
                }
            },
            OzoneMsg::OpenNewLiveFileState {
                fnum_new,
                new_dat_size,
                new_ind_size,
                resp,
            } => {
                let result = self.open_new_live_file_state(
                    fnum_new,
                    new_dat_size,
                    new_ind_size,
                );
                self.respond(result, &resp);
            },
            OzoneMsg::GcControl(gc_ctrl, resp) => {
                match gc_ctrl {
                    GcControl::On(state) => self.gc_on = state,
                    GcControl::Auto(state) => self.auto_gc = state,
                    GcControl::Compact => self.compact(&resp),
                }
            }
            _ => return self.listen_more(msg),
        }
        LoopBreak(false)
    }

    /// Orders the collection of every sealed file of this shard that holds old bytes, or holds
    /// nothing, and answers with what each came to.  A file held by something is asked again
    /// when the caller asks again, which is what `OzoneApi::compact_now` does until none is.
    fn compact(&mut self, resp: &Responder<UIDL, UID, ENC, KH>) {
        let mut cands = Vec::new();
        for (fnum, fstat) in self.states().map() {
            if !fstat.is_live() && (fstat.get_old_sum() > 0 || fstat.is_all_old()) {
                let mut path = self.zdir().dir.clone();
                path.push(ZoneDir::relative_file_path(&FileType::Data, *fnum));
                if path.is_file() {
                    cands.push(*fnum);
                }
            }
        }
        let (mut started, mut deleted, mut waiting) = (Vec::new(), Vec::new(), Vec::new());
        for fnum in cands {
            match self.maybe_collect(fnum, Force::Yes) {
                Ok(Collect::Started)    => started.push(fnum),
                Ok(Collect::Deleted)    => deleted.push(fnum),
                Ok(Collect::Held(hold)) => waiting.push((fnum, hold)),
                Ok(Collect::Declined)   => (), // Force::Yes leaves nothing to decline.
                Err(e) => {
                    self.error(e);
                    waiting.push((fnum, Hold::Fault));
                },
            }
        }
        self.respond(Ok(OzoneMsg::CompactReport {
            wind: self.wind().clone(),
            started,
            deleted,
            waiting,
        }), resp);
    }

    /// Capture read and write messages related to a file in a buffer while its garbage is being
    /// collected.  There are two indications that a file is in the process of garbage collection;
    /// a flag `gc_active` in its file state, and a entry in the `gc_buffer` map keyed to the file
    /// number.  undergoing garbage collection.  If so, the incoming message is appended to the
    /// entry.  Otherwise
    fn gc_active(
        &mut self,
        fnum:       FileNum,
        msg:        &OzoneMsg<UIDL, UID, ENC, KH>,
        processing: bool,
    )
        -> bool
    {
        if processing { return false; }

        // Borrow check work around: read gc flag first.
        let flag = match self.states().get_state(fnum) {
            Ok(fstat) => Some(fstat.gc_active()),
            Err(_) => None,
        };
        let mut process_buffer = false;
        let gc_active = match self.gc_buffer_mut().get_mut(&fnum) {
            Some(gcbuf) => {
                match flag {
                    Some(gc_active) => {
                        if gc_active {
                            // Buffer the incoming message because the file garbage is being
                            // collected.
                            gcbuf.push(msg.clone());
                        } else {
                            // Process the buffer messages because garbage collection has finished.
                            // This needs to be pushed outside this match scope because work_msg
                            // requires another mutable borrow of self.
                            process_buffer = true;
                        }
                    },
                    None => self.error(err!(
                        "{}: A gc buffer exists for file {} but no file state exists. \
                        Could not buffer the received {:?}.", self.ozid(), fnum, msg;
                        Bug, Missing, Data)),
                }
                true
            },
            None => false, // No need to do anything.
        };
        if process_buffer {
            // Take the buffer out before replaying it.  Replaying a message can start a
            // fresh collection of the same file, which creates a new, empty buffer; removing
            // the entry after the replay would take that new buffer with it, and every
            // later message for the file would then be applied to a data file in the middle
            // of being transcribed, with a second collector free to start on it as well.
            if let Some(gcbuf) = self.gc_buffer_mut().remove(&fnum) {
                for msg in gcbuf {
                    // Once a fresh collection has started, the rest of the replay belongs to
                    // its buffer rather than to the file being transcribed.
                    match self.gc_buffer_mut().get_mut(&fnum) {
                        Some(newbuf) => newbuf.push(msg),
                        None => { self.listen_work(&OzoneMsg::ProcessGcBuffer(Box::new(msg))); },
                    }
                }
            }
        }
        gc_active
    }

    fn schedule_deletion(
        &mut self,
        floc:   &FileLocation,
        rid:    &RecordDigest,
        from:   &OzoneBotId,
    )
        -> Outcome<()>
    {
        let self_id = self.ozid().clone();

        // [17] Schedule the old file location for deletion.
        // [17.1] Update the current data in the file state data map to old.
        match self.states_mut().get_state_mut(floc.file_number()) {
            Err(e) => return Err(err!(e,
                "{:?}: Request from {:?} to delete {:?}.", self_id, from, floc;
                Bug, Missing, Data)),
            Ok(fstat) => {
                // Perform mapping to new start position, resulting from scheduling messages which
                // have backed up during previous garbage collection.  Only this record's move is
                // taken: another's at the same offset is waiting for its own supersession.
                let mut floc2 = floc.clone();
                if let Some(new_start) = fstat.map_and_remove(&floc2.keyval(), rid) {
                    floc2.start = new_start;
                }

                // Register data as old.
                if let Err(e) = fstat.register_old(&floc2.keyval(), *rid) {
                    return Err(err!(e, "{:?}: file {}.", self_id, floc2.file_number(); Data));
                }
            },
        }

        // [17.2] Check whether garbage collection should be triggered for the file.
        self.maybe_collect(floc.file_number(), Force::No).map(|_| ())
    }

    /// Starts a collection of the file if it is eligible now.  A file deferred on any input to
    /// eligibility is looked at again only when something calls this: a supersession, a seal, a
    /// record landing in a sealed file.  Until 2026-09-23 only a supersession did, and a file that
    /// crossed the trigger while it was live, or before its last writes had drained, kept its
    /// garbage until a later supersession happened to land in it: for a file whose remaining
    /// records are never superseded, for ever.
    ///
    /// Two changes that can make a file eligible deliberately do not call this.  The last read
    /// finishing would start a collection in the middle of a burst of reads of the file, a chunked
    /// value's, and a read queued behind the collection is replayed at its old offset with
    /// nothing to remap it, since the collection re-anchored its cache entry and dropped the move.
    /// With records of one size that offset holds another valid record, which the reader returned
    /// until it confirmed key and stamp (2026-09-23): starting collections there failed the first
    /// read of a same-key churn's value after a restart in 5 to 8 runs of 12.  The reader now
    /// retries such a read, but the trigger stays withdrawn until that is measured.  Switching
    /// collection on would hand every file a start-up load had found garbage in to the collectors
    /// at once, and a read of a file waiting its turn waits with it.
    ///
    /// `Force::Yes` is a caller's order to collect, an erase's, and waives the three things that
    /// make the collector wait for garbage to pile up: the trigger fraction, `gc_on` and `auto_gc`.
    /// Nothing that makes a collection unsafe is waived, and a file held by one of those is
    /// reported with the first that holds it.
    fn maybe_collect(&mut self, fnum: FileNum, force: Force) -> Outcome<Collect> {
        let self_id = self.ozid().clone();
        let ordered = force == Force::Yes;
        let plan = match self.states().get_state(fnum) {
            Ok(fstat) => {
                let oldvals = fstat.get_old_sum() as f64;
                let datfilemax = self.cfg().data_file_max_bytes as f64;
                let trigger = constant::OLD_DATA_PERCENT_GC_TRIGGER;
                let oldfrac = 100.0 * (oldvals / datfilemax);
                if fstat.is_live() {
                    Plan::Hold(Hold::Live)
                } else if fstat.gc_active() { // Never set a second collector on the same file.
                    Plan::Hold(Hold::Collecting)
                } else if !fstat.no_pending_moves() {
                    Plan::Hold(Hold::Moves)
                } else if !fstat.no_readers() {
                    Plan::Hold(Hold::Readers)
                } else if !ordered && !(self.gc_on && self.gc_auto_active()) {
                    Plan::Declined
                } else if !ordered && !((oldfrac > trigger) || fstat.is_all_data_old()) {
                    Plan::Declined
                } else {
                    // A sealed file may still have writes draining.  A record's bytes reach the
                    // data file in `WriterBot::write` before its accounting does: the accounting
                    // travels writer -> cbot -> fbot as an `UpdateData`, while the seal travels
                    // writer -> fbot directly and can overtake it.  So at the moment a sibling
                    // record's supersession trips this trigger, `dat_size` and `dmap` can still
                    // lag the physical file by the records whose `UpdateData` is in flight.
                    // Collecting then is doubly wrong: the snapshot's accounting disagrees with
                    // the file it transcribes (the `old_sum != old_size - new_size` abort), and a
                    // later in-flight `UpdateData` would insert a now-stale position into the
                    // rewritten file.  Defer until the on-disk size equals the accounted size --
                    // the point at which every write has drained.  This is rare on the unchunked
                    // path (old bytes accrue a small record at a time, long after the file has
                    // sealed and drained) but routine for a chunked value, whose single overwrite
                    // both rolls a file mid-burst and supersedes a whole value's worth of records
                    // at once.  The record whose landing completes the drain brings the file
                    // back here (`update_data`), so deferral only delays.
                    let mut dat_path = self.zdir().dir.clone();
                    dat_path.push(ZoneDir::relative_file_path(&FileType::Data, fnum));
                    let drained = match std::fs::metadata(&dat_path) {
                        Ok(m)  => m.len() == fstat.get_data_file_size() as u64,
                        // Cannot confirm the file has drained, so do not collect it yet.
                        Err(_) => false,
                    };
                    // Case (c) from `register_old`: once the file has fully drained, every parked
                    // supersession must have found its record.  Any left over refers to a record
                    // that is not on disk -- a genuine accounting fault, not the write-path race --
                    // so fail loudly rather than collect a file whose accounting is inconsistent.
                    if drained && !fstat.pending_old_empty() {
                        return Err(err!(
                            "{:?}: File {} has drained (on-disk size equals accounted size) yet \
                            {} superseded record(s) were never inserted: {:?}. This is an \
                            accounting inconsistency, not the write-path race.",
                            self_id, fnum, fstat.pending_old().len(), fstat.pending_old();
                            Bug, Missing, Data));
                    }
                    if !drained {
                        Plan::Hold(Hold::Undrained)
                    } else if fstat.is_all_old() {
                        Plan::Delete(fstat.get_data_file_size() + fstat.get_index_file_size())
                    } else {
                        Plan::Collect(fstat.clone())
                    }
                }
            },
            Err(e) => return Err(err!(e,
                "{:?}: Evaluating file {} for collection, which has no state.", self_id, fnum;
                Bug, Missing, Data)),
        };
        match plan {
            Plan::Hold(hold)    => Ok(Collect::Held(hold)),
            Plan::Declined      => Ok(Collect::Declined),
            Plan::Delete(_) if hooks::collect_fails_for(fnum) => Ok(Collect::Declined),
            Plan::Delete(size)  => {
                // [18.2] Just delete the data file and its index file if it has no current data.
                debug!(sync_log::stream(), "{}: Automated garbage collection for file {}", self_id, fnum);
                let gone = res!(self.states().get_state(fnum)).old_rids();
                for ftyp in [FileType::Data, FileType::Index] {
                    let mut path = self.zdir().dir.clone();
                    path.push(ZoneDir::relative_file_path(&ftyp, fnum));
                    if path.is_file() {
                        res!(fs::remove_file(path));
                    }
                }
                // The state outlived the files until 2026-10-08, with its old bytes still counted
                // and its sizes still in the shard's total, so that any account of the old bytes
                // left in the store counted a file that was gone.
                res!(self.states_mut().get_state_mut(fnum)).reset();
                res!(self.states_mut().dec_size(size));
                // The readers cache open handles, and one left on an unlinked file would keep its
                // bytes allocated for the life of the process, so they are told it has gone.
                res!(self.notify_file_replaced(fnum, &[FileType::Data, FileType::Index]));
                // The caches may forget the tombstones that shadowed these records once the
                // unlinks are durable, and not before.
                let synced = match hooks::dir_sync_fails() {
                    true => Err(err!(
                        "{}: The directory sync after deleting file {} failed on the \
                        test::hooks::set_dir_sync_failure switch.", self_id, fnum;
                        IO, File, Write)),
                    false => sync_dir(&self.zdir().dir),
                };
                match synced {
                    Err(e) => warn!(sync_log::stream(),
                        "{}: {} The tombstones shadowing the records of deleted file {} are kept.",
                        self_id, e, fnum),
                    Ok(()) => if !gone.is_empty() {
                        let bots = res!(self.cbots());
                        res!(bots.send_to_all(OzoneMsg::RecordsGone(fnum, gone)));
                    },
                }
                debug!(sync_log::stream(),
                    "{}: All the data in file {} is old, the file has therefore been deleted.",
                    self_id, fnum,
                );
                Ok(Collect::Deleted)
            },
            Plan::Collect(fstat) => {
                // [18.1] Select a gbot to collect the garbage.
                debug!(sync_log::stream(), "{}: Automated garbage collection for file {}", self_id, fnum);
                let bots = res!(self.igbots());
                let (bot, _) = bots.choose_bot(&ChooseBot::Randomly);
                res!(bot.send(OzoneMsg::CollectGarbage {
                    fnum,
                    fstat,
                    fbot_index: self.wind().b(),
                }));
                // [18.3] Create a gc buffer entry.
                self.gc_buffer_mut().insert(fnum, Vec::new());
                res!(self.states_mut().get_state_mut(fnum)).set_gc(true);
                Ok(Collect::Started)
            },
        }
    }

    fn update_data(
        &mut self,
        floc_new:       &FileLocation,
        ilen:           usize,
        floc_old_opt:   Option<&(FileLocation, RecordDigest)>,
        from:           &OzoneBotId,
    )
        -> Outcome<()>
    {
        // [15] Add new data to the given live file state.
        match self.states_mut().insert_new(floc_new, ilen) {
            Err(e) => return Err(err!(e,
                "{:?}: Request from {:?} to insert {:?}.", self.ozid(), from, floc_new;
                Data)),
            Ok(()) => (),
        };

        // [16] Advise the appropriate fbot to schedule the old data for deletion in its file state
        // data map.
        if let Some((floc_old, rid)) = floc_old_opt {
            let bots = res!(self.fbots());
            let (bot, b) = bots.choose_bot(&ChooseBot::ByFile(floc_old.file_number()));
            if *b == self.wind().b() {
                // This could be itself, and then the supersession passes the same guard as a
                // `ScheduleOld` message.  Applied directly to a file being collected, it went
                // into the state that the collection's result then replaced: the record was
                // carried into the rewritten file as current, its move entry was never cleared,
                // and a file with a move entry is never collected again.  With two file bots to a
                // zone, half of all supersessions come this way; the online sweep lost 50 to 65
                // of its 224 to it (2026-09-23).
                let msg = OzoneMsg::ScheduleOld(*floc_old, *rid, from.clone());
                if !self.gc_active(floc_old.file_number(), &msg, false) {
                    res!(self.schedule_deletion(
                        floc_old,
                        rid,
                        from,
                    ));
                }
            } else {
                // Or another fbot.
                hooks::forward_delay();
                res!(bot.send(OzoneMsg::ScheduleOld(
                    *floc_old,
                    *rid,
                    from.clone(),
                )));
            }
        }

        // A sealed file's last records land after its seal, and until they do it has not
        // drained and cannot be collected.
        self.maybe_collect(floc_new.file_number(), Force::No).map(|_| ())
    }
    
    fn close_old_live_file_state(
        &mut self,
        fnum_old:       FileNum,
        fnum_new:       FileNum,
        new_dat_size:   u64,
        new_ind_size:   u64,
        resp:           Responder<UIDL, UID, ENC, KH>,
    )
        -> Outcome<()>
    {
        if fnum_old > 0 {
            // [6] Update the state of the previous live file.
            match self.states_mut().get_state_mut(fnum_old) {
                Ok(fstat) => fstat.set_live(false),
                Err(e) => return Err(err!(e,
                    "{}: Request to close old live file {} state.", self.ozid(), fnum_old;
                    Bug, Missing, Data)),
            }
        }

        // [7] Advise the appropriate fbot to add the new live file to the file state map.
        let bots = res!(self.fbots());
        let (bot, b) = bots.choose_bot(&ChooseBot::ByFile(fnum_new));
        if *b == self.wind().b() {
            // This could be itself...
            res!(self.open_new_live_file_state(
                fnum_new,
                new_dat_size,
                new_ind_size,
            ));
            self.respond(Ok(OzoneMsg::Ok), &resp);
        } else {
            // Or another fbot.
            res!(bot.send(OzoneMsg::OpenNewLiveFileState{
                fnum_new,
                new_dat_size,
                new_ind_size,
                resp,
            }));
        }

        // Supersessions that reached the file while it was live could not start a collection.
        // The writer has its answer, so a failure here is only logged.
        if fnum_old > 0 {
            let result = self.maybe_collect(fnum_old, Force::No).map(|_| ());
            self.result(&result);
        }

        Ok(())
    }

    fn open_new_live_file_state(
        &mut self,
        fnum:       FileNum,
        dat_size:   u64,
        ind_size:   u64,
    )
        -> Outcome<OzoneMsg<UIDL, UID, ENC, KH>>
    {
        // [8] Update the state of the new live file.
        self.states_mut().new_live_file(
            fnum, 
            dat_size,
            ind_size,
        );
        Ok(OzoneMsg::Ok)
    }
}
