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
        state::FileStateMap,
    },
};

use oxedize_fe2o3_core::channels::Recv;
use oxedize_fe2o3_jdat::id::NumIdDat;

use std::{
    collections::BTreeMap,
    fs::self,
    sync::Arc,
    time::Instant,
};

#[derive(Clone, Debug)]
pub enum GcControl {
    On(bool), // switch gc on or off
    Auto(bool), // set auto gc
    Manual(FileNum), // file number
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
    sem:        Semaphore,
    errc:       Arc<Mutex<usize>>,
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
            sem:        args.sem,
            errc:       Arc::new(Mutex::new(0)),
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
            OzoneMsg::ScheduleOld(floc, from_id) => {
                // [17] Schedule the old file location for deletion.
                if !self.gc_active(
                    floc.file_number(),
                    &msg,
                    processing_buffer,
                ) { 
                    let result = self.schedule_deletion(floc, from_id);
                    self.result(&result);
                }
            }
            OzoneMsg::UpdateData { floc_new, ilen, floc_old_opt, from_id } => {
                // [15] Add new data to the given live file state.
                let result = self.update_data(floc_new, *ilen, floc_old_opt.as_ref(), from_id);
                self.result(&result);
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
            OzoneMsg::ReadFileRequest(fnum, mloc, resp_r2) => {
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
                            if let Some(new_start) = fstat.map_and_remove(&mloc2.file_location().keyval()) {
                                mloc2.new_start_position(new_start);
                                postgc = true;
                            }
                            (
                                if postgc {
                                    Ok(())
                                } else {
                                    let result = fstat.inc_readers();
                                    result
                                },
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
                    // <6> Send file location back to rbot, representing permission to perform a read.
                    self.respond(Ok(msg2), resp_r2);
                }
            }
            OzoneMsg::ReadFinished(fnum) => {
                if !self.gc_active(
                    *fnum,
                    &msg,
                    processing_buffer,
                ) { 
                    // <9> Decrement the reader count now that a read has completed.
                    let result = match self.states_mut().get_state_mut(*fnum) {
                        Ok(fstat) => {
                            let result = fstat.dec_readers();
                            result
                        },
                        Err(_) => {
                            warn!(
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
                let result = self.close_old_live_file_state(
                    fnum_old,
                    fnum_new,
                    new_dat_size,
                    new_ind_size,
                    resp,
                );
                self.result(&result);
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
            OzoneMsg::GcControl(gc_ctrl, _) => {
                match gc_ctrl {
                    GcControl::On(state) => self.gc_on = state,
                    GcControl::Auto(state) => self.auto_gc = state,
                    GcControl::Manual(_) => warn!("Manual gc not yet implemented."),
                }
            }
            _ => return self.listen_more(msg),
        }
        LoopBreak(false)
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
            if let Some(gcbuf) = self.gc_buffer().get(&fnum) {
                // An immutable borrow of gc_buffer allows the mutable borrows needed for work_msg.
                for msg in gcbuf.clone() {
                    self.listen_work(&OzoneMsg::ProcessGcBuffer(Box::new(msg)));
                }
            }
            self.gc_buffer_mut().remove(&fnum);
        }
        gc_active
    }

    fn schedule_deletion(
        &mut self,
        floc:   &FileLocation,
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
                // have backed up during previous garbage collection.
                let mut floc2 = floc.clone();
                if let Some(new_start) = fstat.map_and_remove(&floc2.keyval()) {
                    floc2.start = new_start;
                }

                // Register data as old.
                if let Err(e) = fstat.register_old(&floc2.keyval()) {
                    return Err(err!(e, "{:?}: file {}.", self_id, floc2.file_number(); Data));
                }
            },
        }

        if self.gc_on {
            let fnum = floc.file_number();
            let mut gc_activated = false;
            // [17.2] Check whether garbage collection should be triggered for the file.
            match self.states().get_state(floc.file_number()) {
                Ok(fstat) => {
                    let oldvals = fstat.get_old_sum() as f64;
                    let datfilemax = self.cfg().data_file_max_bytes as f64;
                    let trigger = constant::OLD_DATA_PERCENT_GC_TRIGGER;
                    let oldfrac = 100.0 * (oldvals / datfilemax);
                    if ((oldfrac > trigger) || fstat.is_all_data_old()) &&
                        fstat.no_pending_moves() &&
                        !fstat.is_live() &&
                        fstat.no_readers() &&
                        self.gc_auto_active()
                    {
                        // [18.1] Select a gbot to collect the garbage.
                        debug!("{}: Automated garbage collection for file {}", self_id, fnum);
                        let bots = res!(self.igbots());
                        let (bot, _) = bots.choose_bot(&ChooseBot::Randomly);
                        if fstat.is_all_old() {
                            // [18.2] Just delete the data file and its index file if it has no current data.
                            for ftyp in [FileType::Data, FileType::Index] {
                                let mut path = self.zdir().dir.clone();
                                path.push(ZoneDir::relative_file_path(&ftyp, fnum));
                                if path.is_file() {
                                    res!(fs::remove_file(path));
                                }
                            }
                            debug!(
                                "{}: All the data in file {} is old, the file has therefore been deleted.",
                                self_id, fnum,
                            );
                        } else {
                            res!(bot.send(OzoneMsg::CollectGarbage {
                                fnum,
                                fstat:      fstat.clone(),
                                fbot_index: self.wind().b(),
                            }));
                            // [18.3] Create a gc buffer entry.
                            self.gc_buffer_mut().insert(fnum, Vec::new());
                            //fstat.set_gc(true); // [#] Moved out of scope due to borrow checker
                            gc_activated = true;
                        }
                    }
                },
                Err(e) => return Err(err!(e,
                    "{:?}: Request from {:?} to schedule old {:?}.", self_id, from, floc;
                    Bug, Missing, Data)),
            }
            // [#] Moved out to here due to borrow checker.
            if gc_activated {
                match self.states_mut().get_state_mut(floc.file_number()) {
                    Ok(fstat) => fstat.set_gc(true),
                    _ => (), // unreachable
                }
            }
        }
        Ok(())

    }

    fn update_data(
        &mut self,
        floc_new:       &FileLocation,
        ilen:           usize,
        floc_old_opt:   Option<&FileLocation>,
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
        if let Some(floc_old) = floc_old_opt {
            let bots = res!(self.fbots());
            let (bot, b) = bots.choose_bot(&ChooseBot::ByFile(floc_old.file_number()));
            if *b == self.wind().b() {
                // This could be itself...
                res!(self.schedule_deletion(
                    floc_old,
                    from,
                ));
            } else {
                // Or another fbot.
                res!(bot.send(OzoneMsg::ScheduleOld(
                    *floc_old,
                    from.clone(),
                )));
            }
        }

        Ok(())
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
