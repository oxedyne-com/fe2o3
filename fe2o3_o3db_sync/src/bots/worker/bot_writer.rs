use crate::{
    prelude::*,
    base::constant,
    bots::{
        base::bot_deps::*,
        worker::{
            syncer::{
                Handed,
                LiveFiles,
                Shared,
                SyncPolicy,
                Syncer,
            },
            worker_deps::*,
        },
    },
    comm::{
        channels::{
            Cut,
            FaultCause,
        },
        msg::StoreRole,
    },
    file::{
        core::FileType,
        floc::{
            FileNum,
            StoredFileLocation,
        },
        live::LivePair,
    },
    test::hooks,
};

use oxedyne_fe2o3_iop_db::api::Meta;
use oxedyne_fe2o3_jdat::id::NumIdDat;

use std::{
    fs::File,
    io::Write,
    sync::Arc,
};

/// Each `WriterBot` in a zone has its own `LivePair`.
pub struct WriterBot<
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
    inited:     bool,
    lpair:      LivePair,
    syncer:     Syncer<UIDL, UID, ENC, KH>, // makes each record durable, then releases it
}

impl<
    const UIDL: usize,
    UID:    NumIdDat<UIDL> + 'static,
    ENC:    Encrypter + 'static,
    KH:     Hasher + 'static,
	PR:     Hasher,
    CS:     Checksummer,
>
    WorkerBot<UIDL, UID, ENC, KH, PR, CS> for WriterBot<UIDL, UID, ENC, KH, PR, CS>
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
    OzoneBot<UIDL, UID, ENC, KH, PR, CS> for WriterBot<UIDL, UID, ENC, KH, PR, CS>
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
    Bot<{ BID_LEN }, Bid, OzoneMsg<UIDL, UID, ENC, KH>> for WriterBot<UIDL, UID, ENC, KH, PR, CS>
{
    bot_methods!();

    fn go(&mut self) {

        sync_log::set_stream(self.log_stream_id());

        if self.no_init() { return; }
        self.now_listening();
        loop {
            if self.wind().b() < self.cfg().num_bots_per_zone((&self).wtyp()) {
                if self.listen().must_end() { break; }
            } else {
                // This bot is to be terminated. Forward incoming messages to the remaining bots of
                // this type.
            }
        }
        // Everything written is released, and made durable where the policy owes it, before this
        // bot ends, so a database closed after a write has that write on disk.
        let result = self.syncer.finish();
        self.result(&result);
    }

    fn listen(&mut self) -> LoopBreak {
        match self.chan_in().recv() {
            Err(e) => self.err_cannot_receive(err!(e,
                "{}: Waiting for message.", self.ozid();
                IO, Channel)),
            Ok(msg) => {
                if let Some(msg) = self.listen_worker(msg) {
                    match msg {
                        // COMMAND
                        OzoneMsg::NewLiveFile(fnum_opt, resp) => {
                            let result = match fnum_opt {
                                Some(fnum) => {
                                    // Direct initialization with provided file number.
                                    self.lpair.fnum = fnum;
                                    self.open_live_pair()
                                },
                                None => {
                                    // Routine request for new live file.
                                    self.new_live_pair().map(|_| ())
                                }
                            };
                            // Answered either way: a zone starting up waits on every writer.
                            if let Err(e) = &result {
                                self.error(e.clone());
                            }
                            self.respond(result.map(|()| OzoneMsg::Ok), &resp);
                        }
                        // WORK
                        OzoneMsg::Write{
                            kstored,
                            vstored,
                            klen_cache,
                            cind,
                            meta,
                            cbpind,
                            resp: resp_w1,
                            role,
                        } => {
                            let resp = resp_w1.clone();
                            if let Err(e) = self.write(
                                kstored,
                                vstored,
                                klen_cache,
                                cind,
                                meta,
                                cbpind,
                                resp_w1,
                                role,
                            ) {
                                // The caller is waiting on this answer.  Only logged, a failure
                                // here reached it as a responder timeout that named no cause.
                                self.error(e.clone());
                                self.respond(Err(e), &resp);
                            }
                        }
                        // A barrier failed: cut the files in doubt in this process too, so the
                        // tail past the mark cannot be read for the rest of this boot.  Done here,
                        // so it is ordered after any append in flight.
                        OzoneMsg::Cut(cuts) => self.cut(cuts),
                        OzoneMsg::Durable(z, fnum) => {
                            let result = self.fbots().and_then(|bots| {
                                let (bot, _) = bots.choose_bot(&ChooseBot::ByFile(fnum));
                                bot.send(OzoneMsg::Durable(z, fnum))
                            });
                            self.result(&result);
                        },
                        OzoneMsg::SyncNow(resp) => {
                            if let Err(e) = self.syncer.hand(Handed::SyncNow(resp.clone())) {
                                self.respond(Err(err!(e,
                                    "{}: The durability barrier thread could not take a sync.",
                                    self.ozid();
                                    Thread, Write)), &resp);
                            }
                        },
                        OzoneMsg::SyncOwed => {
                            let result = self.syncer.hand(Handed::SyncOwed);
                            self.result(&result);
                        },
                        //OzoneMsg::Delete(kv, resp_w1) => {
                        //    let result = self.write(kv, resp_w1);
                        //    self.result(result);
                        //},
                        _ => return self.listen_more(msg),
                    }
                }
            },
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
    WriterBot<UIDL, UID, ENC, KH, PR, CS>
{
    /// Starts the writer's durability barrier thread, so that a writer which could never confirm
    /// a write fails the database's start instead of its first write.
    pub fn new(
        args: ZoneWorkerInitArgs<UIDL, UID, ENC, KH, PR, CS>,
    )
        -> Outcome<Self>
    {
        let shared = Shared {
            zone:   args.wind.z(),
            bot:    args.wind.b(),
            root:   args.api.db_root().to_path_buf(),
            marks:  args.api.chans().marks().clone(),
            fault:  args.api.chans().fault().clone(),
            writer: args.chan_in.clone(),
        };
        let syncer = res!(Syncer::start(
            fmt!("{}", args.api.ozid),
            args.log_stream_id.clone(),
            shared,
        ));
        Ok(Self {
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
            api:        args.api,
            // State
            active:     false,
            inited:     false,
            lpair:      LivePair::default(),
            syncer,
        })
    }

    fn lpair(&self)                 -> &LivePair                    { &self.lpair }
    fn lpair_mut(&mut self)         -> &mut LivePair                { &mut self.lpair }

    //fn cached_livefile_len(&self) -> usize { self.lpair.dat_size as usize }

    /// This is the main writer method which:
    ///
    /// 1. Appends a checksum to the key and value bytes then appends these to the current zone
    ///    `LivePair` data file (creating the next `LivePair` if necessary in order not to exceed
    ///    the file size limit).
    /// 2. Inserts the key, location and possibly the value into the zone data cache.
    /// 3. Appends the key and location to the `LivePair` index file, each with an appended
    ///    checksum.
    ///
    /// Returns on a write error, with nothing written to the cache or index file.
    ///
    ///```ignore
    ///   
    ///   Appended to data file                      Appended to index file 
    ///  +---------------------+ -+               +- +---------------------+
    ///  |                     |  |               |  |                     |
    ///  |        key          |  |               |  |        key          |
    ///  |                     |  |               |  |                     |
    ///  +---------------------+  +- StoredKey ---+  +---------------------+
    ///  |        meta         |  |               |  |        meta         |
    ///  +---------------------+  |               |  +---------------------+
    ///  |      checksum       |  |               |  |      checksum       |
    ///  +---------------------+ -+               +- +---------------------+ -+
    ///  |                     |  |               |  |       start         |  |    part of a
    ///  |                     |  |               |  +---------------------+  +-- FileLocation
    ///  |                     |  |  StoredIndex -+  |       klen          |  |
    ///  |                     |  |               |  +---------------------+  |
    ///  |       value         |  |               |  |       vlen          |  |
    ///  |                     |  +- StoredValue  |  +---------------------+ -+
    ///  |                     |  |               |  |      checksum       |
    ///  |                     |  |               +- +---------------------+
    ///  |                     |  |     
    ///  |                     |  |     
    ///  +---------------------+  |     
    ///  |      checksum       |  |
    ///  +---------------------+ -+
    ///
    ///```
    fn write(
        &mut self,
        mut kbyts:  Vec<u8>,
        vstored:    Vec<u8>,
        klen_cache: usize,
        cind:       Option<usize>,
        meta:       Meta<UIDL, UID>,
        cbpind:     usize, // cbot pool index
        resp_w1:    Responder<UIDL, UID, ENC, KH>,
        role:       Option<StoreRole<UIDL, UID, ENC, KH>>,
    )
        -> Outcome<()>
    {
        // A store's head hands its chunk set's hold over with it.  It is released here only when
        // nothing reached the data file; once the record is there it comes back at the next
        // replay, so the hold lasts until the cache bot has it, or for the life of the process.
        // A record appended now could be neither confirmed nor withdrawn, so a writer whose syncer
        // has stopped refuses it before anything reaches the files.  Appended first, it was
        // reported failed and came back at the next start (2026-09-23).
        if !self.syncer.is_running() {
            self.release_set(&role);
            return Err(err!(
                "{}: The durability barrier thread has stopped, so the write was refused before \
                anything was written.", self.ozid();
                Thread, Missing, Write));
        }
        // A failed barrier has stopped the store: nothing appended now could ever be proven.
        if let Some(fault) = self.api().chans().fault().get() {
            self.release_set(&role);
            return Err(err!(
                "{}: {}, so the write was refused.  Restart the store; it recovers from the disk.",
                self.ozid(), fault.stopped();
                Write, Fatal));
        }
        // A failed append is cut back off, so the next record starts where this one began.  The
        // sizes are those of the pair the append went to, after any rollover, since a failure
        // leaves them unchanged.
        let start = match self.write_to_file(FileType::Data, vec![&kbyts[..], &vstored[..]]) {
            Ok(start) => start,
            Err(e) => {
                let upto = (self.lpair.dat.size, self.lpair.ind.size);
                if self.cut_back(upto).is_ok() {
                    self.release_set(&role);
                }
                return Err(e);
            },
        };

        // Define the location.
        let sfloc = res!(StoredFileLocation::new(
            self.lpair().fnum,
            start,
            kbyts.len() as u64,
            vstored.len() as u64,
            self.api().schemes().checksummer().clone(),
        ));
        let istored = &sfloc.buf;

        // Append key and location to the current index file.
        // A failed index append withdraws the whole record, its data too, so a write reported
        // failed does not come back when a later open rebuilds the index from the data.
        if let Err(e) = self.write_to_file(FileType::Index, vec![&kbyts[..], &istored[..]]) {
            let upto = (start, self.lpair.ind.size);
            if self.cut_back(upto).is_ok() {
                self.release_set(&role);
            }
            return Err(e);
        }
        let ends = (self.lpair().dat.size, self.lpair().ind.size);

        // [11] The record is in the live pair, so say so before anything waits on the disk.  The
        //      caller holds this answer to the short deadline, which then measures whether the
        //      writer is alive rather than how busy the machine's disk happens to be.
        self.respond(Ok(OzoneMsg::Written), &resp_w1);

        // [12] The syncer releases the record to a cbot once the barrier the policy asks for is
        //      behind it, so no observer sees a key before it is as durable as configured.  The
        //      cbot then makes it readable and gives the caller its final answer.
        let cbots = res!(self.cbots());
        let cbot = res!(cbots.get_bot(cbpind)).clone();
        kbyts.drain(..constant::CACHE_HASH_BYTES); // remove data pathway hash used to identify cbot
        kbyts.truncate(klen_cache); // remove metadata
        let resp = resp_w1.clone();
        let ticketed = matches!(role, Some(StoreRole::Head(_)));
        let insert = OzoneMsg::Insert(
            kbyts,
            Some(vstored),
            cind,
            sfloc.ref_file_location().clone(),
            istored.len(),
            meta,
            resp_w1, // The cbot responds to the caller.
            None,
            role,
        );
        let policy = SyncPolicy::of(self.cfg());
        let handed = if ticketed && hooks::head_hand_fails() {
            Err(err!("A store's head could not be handed to the syncer (test::hooks)."; Thread, Write))
        } else {
            self.syncer.hand(Handed::Record { cbot, cbpind, insert, resp, policy, ends })
        };
        if let Err(e) = handed {
            // The syncer stopped after the check above, and the record is in the files.
            return Err(err!(e,
                "{}: The record is written, but the durability barrier thread stopped before it \
                could take it, so it is not confirmed durable.", self.ozid();
                Thread, Write, Unconfirmed));
        }

        Ok(())
    }

    // A store's head that never reached the data file will not land, so its set is free.
    fn release_set(&self, role: &Option<StoreRole<UIDL, UID, ENC, KH>>) {
        if let Some(id) = role.as_ref().and_then(|r| r.set()) {
            if let Ok(mut held) = self.api().chans().pending_sets().lock() {
                held.release(&id);
            }
        }
    }

    /// Hands the syncer the live pair every record from here on is appended to.  It syncs through
    /// handles of its own, which reach the same open files.
    fn hand_pair(&self, lpair: &LivePair) -> Outcome<()> {
        if hooks::pair_hand_fails() {
            return Err(err!(
                "{}: Live pair {} could not be duplicated for the syncer (test::hooks).",
                self.ozid(), lpair.fnum;
                IO, File));
        }
        let dat = match &lpair.dat.file {
            Some(file) => res!(file.try_clone()),
            None => return Err(err!(
                "{}: The live data file {:?} is not open.", self.ozid(), lpair.dat.path;
                Bug, Missing)),
        };
        let ind = match &lpair.ind.file {
            Some(file) => res!(file.try_clone()),
            None => return Err(err!(
                "{}: The live index file {:?} is not open.", self.ozid(), lpair.ind.path;
                Bug, Missing)),
        };
        self.syncer.hand(Handed::Pair(LiveFiles {
            fnum:   lpair.fnum,
            dat,
            ind,
            paths:  (lpair.dat.path.clone(), lpair.ind.path.clone()),
            sizes:  (lpair.dat.size, lpair.ind.size),
        }))
    }

    /// Cuts each file in doubt back to its mark (R4a).  The open cuts them again should this
    /// fail or the process die first; a cut is only ever a shortening, so twice is harmless.
    fn cut(&mut self, cuts: Vec<Cut>) {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        for c in cuts {
            match c.apply(stamp) {
                Ok((d, i)) => error!(sync_log::stream(), err!(
                    "{}: After a failed sync, file {} was cut back to its durable mark, {} data \
                    and {} index bytes removed.", self.ozid(), c.fnum, d, i;
                    IO, File, Write, Fatal)),
                Err(e) => error!(sync_log::stream(), err!(e,
                    "{}: After a failed sync, file {} could not be cut back to its durable mark; \
                    the next open cuts it.", self.ozid(), c.fnum;
                    IO, File, Write, Fatal)),
            }
        }
        // The writer's own view of the pair must agree with the disk, should anything read it.
        if let Ok(len) = self.lpair.dat.get_file_len() {
            self.lpair.dat.size = len;
        }
        if let Ok(len) = self.lpair.ind.get_file_len() {
            self.lpair.ind.size = len;
        }
    }

    /// Takes the live file the zone assigned at start-up, new or partly written.
    fn open_live_pair(&mut self) -> Outcome<()> {
        let lpair = res!(self.zdir().open_live(self.lpair.fnum));
        // The syncer has the pair before the writer does, as at a rollover.
        res!(self.hand_pair(&lpair));
        self.lpair.close();
        self.lpair = lpair;
        self.register_live_file(self.lpair.fnum)
    }

    /// Closes a pair opened for a rollover that did not happen, and removes its files, which
    /// nothing was written to.
    fn abandon(&self, mut lpair: LivePair) {
        lpair.close();
        if lpair.dat.size > 0 || lpair.ind.size > 0 {
            return; // not new after all, so not this writer's to remove
        }
        // The index first, as collection removes a pair.
        for path in [&lpair.ind.path, &lpair.dat.path] {
            if let Err(e) = std::fs::remove_file(path) {
                warn!(sync_log::stream(),
                    "{}: Could not remove {:?}, created for a rollover that did not happen: {}",
                    self.ozid(), path, e);
            }
        }
    }

    /// Tells the file's bot that the file is live, as a rollover does for the file it opens.  The
    /// bot otherwise first heard of a new file from its first record, which reaches it through the
    /// syncer and a cache bot, so a writer could seal the file before the bot knew it existed and
    /// the seal failed; and a partly written file taken over at start-up was never flagged live,
    /// leaving it open to collection while it was still being written.  Its accounting starts
    /// empty, since the records already in such a file are counted as the zone loads them.
    fn register_live_file(&self, fnum: FileNum) -> Outcome<()> {
        let resp = Responder::new(Some(self.ozid()));
        let bots = res!(self.fbots());
        let (bot, _) = bots.choose_bot(&ChooseBot::ByFile(fnum));
        res!(bot.send(OzoneMsg::OpenNewLiveFileState {
            fnum_new:       fnum,
            new_dat_size:   0,
            new_ind_size:   0,
            resp:           resp.clone(),
        }));
        // Start-up work, held to the control deadline: see constant::CONTROL_REQUEST_TIMEOUT.
        match resp.recv_timeout(constant::CONTROL_REQUEST_TIMEOUT) {
            Err(e) => Err(err!(e,
                "{}: While registering live file {} with its file bot.", self.ozid(), fnum;
                IO, Channel, Read)),
            Ok(OzoneMsg::Ok) => Ok(()),
            Ok(OzoneMsg::Error(e)) => Err(err!(e,
                "{}: The file bot could not register live file {}.", self.ozid(), fnum;
                IO, File)),
            Ok(msg) => Err(err!(
                "{}: Unrecognised response to registering live file {}: {:?}", self.ozid(), fnum, msg;
                Channel, Unexpected)),
        }
    }

    fn new_live_pair(&mut self) -> Outcome<(FileNum, u64)> {
        // A stopped store takes no new file: nothing could prove it, and the fault's cut list
        // names the files as they stood.
        if let Some(fault) = self.api().chans().fault().get() {
            return Err(err!(
                "{}: {}, so no new live file was opened.  Restart the store; it recovers from \
                the disk.", self.ozid(), fault.stopped();
                Write, Fatal));
        }
        let fnum_old = self.lpair().fnum;
        // A writer given no incomplete file at start-up has none until its first write, so it
        // has nothing to seal and its first file is registered as one taken over at start-up is.
        let first = self.lpair().dat.file.is_none();
        // [3] Ask zbot for next live file number.
        let resp = Responder::new(Some(self.ozid()));
        match self.zbot() {
            None => return Err(err!(
                "{}: Could not get zbot work channel.", self.ozid();
                Missing, Data)),
            Some(zbot) => 
                res!(zbot.send(OzoneMsg::NextLiveFile(resp.clone()))),
        }
        let fnum_new = match resp.recv_timeout(constant::BOT_REQUEST_TIMEOUT) {
            Err(e) => return Err(err!(e,
                "While getting next live file info from zbot.";
                IO, Channel, Read)),
            Ok(OzoneMsg::UseLiveFile(fnum)) => fnum,
            Ok(OzoneMsg::Error(e)) => return Err(err!(e,
                "{}: The zone could not give this writer a new live file.", self.ozid();
                IO, File, Create)),
            Ok(msg) => return Err(err!(
                "Unrecognised new live file request response: {:?}", msg;
                Bug, Invalid, Input)),
        };

        // The syncer is handed the new pair before this writer switches to it, and nothing after
        // the hand-off can fail the switch.  Switched first, a writer whose hand-off failed went
        // on appending to a pair its syncer did not hold: records confirmed durable that no
        // barrier had covered, in a file its file bot never flagged live, while the file it had
        // left stayed flagged live for good (2026-09-23).  The file being sealed is made durable
        // by the syncer before it releases anything written to its successor, whatever the
        // configured policy, so crash loss stays bounded by the live file's tail.  This writer
        // does not wait for that: the disk is the syncer's to wait on, and a writer held by it
        // would hold every record queued behind the rollover.
        let lpair = res!(self.zdir().open_live(fnum_new));
        if let Err(e) = self.hand_pair(&lpair) {
            self.abandon(lpair);
            return Err(err!(e,
                "{}: New live file {} could not be handed to the durability barrier thread, so \
                this writer stays on file {}.", self.ozid(), fnum_new, fnum_old;
                IO, File));
        }
        self.lpair.close();
        self.lpair = lpair;
        let start = self.lpair().dat.size;
        if first {
            res!(self.register_live_file(fnum_new));
            return Ok((fnum_new, start));
        }

        // [5] Tell the fbot for the previous live file of the change and wait for the response.
        let resp_w3 = Responder::new(Some(self.ozid()));
        let bots = res!(self.fbots());
        let (bot, _) = bots.choose_bot(&ChooseBot::ByFile(fnum_old));
        res!(bot.send(OzoneMsg::CloseOldLiveFileState {
            fnum_old,
            fnum_new,
            new_dat_size: self.lpair.dat.size,
            new_ind_size: self.lpair.ind.size,
            resp: resp_w3.clone(),
        }));
        // [9] Wait to hear when the new live file is ready to go.
        match resp_w3.recv_timeout(constant::BOT_REQUEST_TIMEOUT) {
            Err(e) => return Err(err!(e,
                "While advising fbot to update live file states.";
                IO, Channel, Read)),
            Ok(OzoneMsg::Ok) => (),
            Ok(OzoneMsg::Error(e)) => return Err(err!(e,
                "{}: The file bot could not seal live file {} for file {}.",
                self.ozid(), fnum_old, fnum_new;
                IO, File)),
            Ok(msg) => return Err(err!(
                "Unrecognised response after advising fbot to update live file states: {:?}", msg;
                Channel)),
        }

        Ok((fnum_new, start))
    }

    /// Writes the given byte vector to the current `LivePair` for this zone, either to the data or
    /// index file.  This method starts a new `LivePair` data file when writing the given value
    /// would cause the current file to exceed the data file size limit.  The data to be written is
    /// not modified in any way.  An append that fails part-way leaves the writer's size of the
    /// file where it was, so that `cut_back` can remove what got through.
    ///
    /// Returns the starting position in the file for the data sequence.
    fn write_to_file(
        &mut self,
        typ:    FileType,
        v:      Vec<&[u8]>,
    )
        -> Outcome<u64>
    {
        // [2.1] Tally total length of data value.
        let mut vlen = 0;
        for vi in &v {
            vlen += vi.len();
        }
        let max_file_len = self.cfg().data_file_max_bytes as usize;
        if vlen > max_file_len {
            return Err(err!(
                "Attempt to store {:?} value of length {} bytes \
                exceeds the config setting of {}.",
                typ, vlen, max_file_len;
                IO, File, Write, Input, TooBig));
        }

        // [2.2] If the current live file is full, start a new one before anything is written.
        let mut new_file = false;
        let mut start = self.lpair().dat.size;
        let file_len = start as usize;
        if self.lpair.dat.file.is_none() || (typ == FileType::Data && (vlen + file_len > max_file_len)) {
            new_file = true;
            let (_, start2) = res!(self.new_live_pair());
            start = start2;
        }

        // [10] Append to the live file.  A failure leaves the size where it was, for the caller
        //      to cut the append back off.
        let before = match typ {
            FileType::Data  => self.lpair.dat.size,
            FileType::Index => self.lpair.ind.size,
        };
        let appended = {
            let lf = match typ {
                FileType::Data  => &mut self.lpair.dat,
                FileType::Index => &mut self.lpair.ind,
            };
            match lf.file.as_mut() {
                Some(file)  => Some(Self::append(file, &v, &typ)),
                None        => None,
            }
        };
        let (written, failed) = match appended {
            Some(a) => a,
            None    => return Err(err!(
                "{}: The {:?} file of live pair {} is not open.", self.ozid(), typ, self.lpair.fnum;
                Unreachable, Bug)),
        };
        match failed {
            None => {
                match typ {
                    FileType::Data  => self.lpair.dat.size = res!(self.lpair.dat.get_file_len()),
                    // The index end feeds the durable mark: left stale, a fault cut the live
                    // index back to where it began.
                    FileType::Index => self.lpair.ind.size += written as u64,
                }
                Ok(start)
            },
            Some(e) => Err(err!(e,
                "{}: Appending {} bytes to {:?} file of live pair {} at offset {} failed after \
                {} bytes{}.", self.ozid(), vlen, typ, self.lpair.fnum, before,
                written, if new_file { ", in a file just started" } else { "" };
                IO, File, Write)),
        }
    }

    /// Cuts a failed append back off the live pair, to the last good lengths `upto`, and takes
    /// the writer's sizes from the files.  They are opened for appending, so a seek back would
    /// leave the torn bytes in place with the next record after them and its location wrong (QA
    /// F4).  A cut that fails stops the store through its fault, whose record has the next open
    /// cut the pair back to `upto`.
    fn cut_back(&mut self, upto: (u64, u64)) -> Outcome<()> {
        let e = match self.try_cut_back(upto) {
            Ok(())  => return Ok(()),
            Err(e)  => e,
        };
        let e = err!(e,
            "{}: A failed append could not be cut back off live pair {}, so the store has \
            stopped writing.  Restart the store; the open cuts the pair back to {} data and {} \
            index bytes.", self.ozid(), self.lpair.fnum, upto.0, upto.1;
            IO, File, Write, Fatal);
        let cut = Cut {
            zone:       self.wind().z(),
            fnum:       self.lpair.fnum,
            data:       self.lpair.dat.path.clone(),
            data_upto:  upto.0,
            index:      self.lpair.ind.path.clone(),
            index_upto: upto.1,
        };
        error!(sync_log::stream(), e.clone());
        let root = self.api().db_root().to_path_buf();
        self.api().chans().fault().raise(&root, FaultCause::CutBack, vec![cut], &e);
        Err(e)
    }

    fn try_cut_back(&mut self, upto: (u64, u64)) -> Outcome<()> {
        if hooks::cut_back_fails() {
            return Err(err!(
                "Live pair {} could not be cut back to ({}, {}) (test::hooks).",
                self.lpair.fnum, upto.0, upto.1;
                IO, File, Write));
        }
        for (lf, to) in [(&mut self.lpair.dat, upto.0), (&mut self.lpair.ind, upto.1)] {
            let file = match &lf.file {
                Some(file)  => file,
                None        => continue, // nothing was opened, so nothing was appended
            };
            let len = res!(lf.get_file_len());
            if len > to {
                if let Err(e) = file.set_len(to) {
                    return Err(err!(e,
                        "Cannot cut the failed append off {:?} from {} back to {}.",
                        lf.path, len, to;
                        IO, File, Write));
                }
            }
            lf.size = res!(lf.get_file_len());
            if lf.size != to {
                return Err(err!(
                    "After cutting a failed append off {:?}, it holds {} bytes, not the last good \
                    length {}.", lf.path, lf.size, to;
                    IO, File, Write, Mismatch));
            }
        }
        Ok(())
    }

    // Appends the slices in order, returning the bytes that reached the file and the error that
    // stopped it short, if one did.
    fn append(file: &mut File, v: &[&[u8]], typ: &FileType) -> (usize, Option<std::io::Error>) {
        let mut left = match hooks::write_tears(typ) {
            Some(n) => n as usize,
            None    => usize::MAX,
        };
        let mut done = 0;
        for vi in v {
            let n = vi.len().min(left);
            if let Err(e) = file.write_all(&vi[..n]) {
                return (done, Some(e));
            }
            done += n;
            if n < vi.len() {
                hooks::write_torn();
                return (done, Some(std::io::Error::other(
                    "The append was torn part-way (test::hooks).")));
            }
            left -= n;
        }
        (done, None)
    }
}

