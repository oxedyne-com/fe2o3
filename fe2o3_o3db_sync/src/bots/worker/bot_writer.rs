use crate::{
    prelude::*,
    base::constant,
    bots::{
        base::bot_deps::*,
        worker::{
            syncer::{
                Handed,
                SyncPolicy,
                Syncer,
            },
            worker_deps::*,
        },
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
    io::{
        Seek,
        SeekFrom,
        Write,
    },
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
                            ) {
                                // The caller is waiting on this answer.  Only logged, a failure
                                // here reached it as a responder timeout that named no cause.
                                self.error(e.clone());
                                self.respond(Err(e), &resp);
                            }
                        }
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
        let syncer = res!(Syncer::start(fmt!("{}", args.api.ozid), args.log_stream_id.clone()));
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
    )
        -> Outcome<()>
    {
        // A record appended now could be neither confirmed nor withdrawn, so a writer whose syncer
        // has stopped refuses it before anything reaches the files.  Appended first, it was
        // reported failed and came back at the next start (2026-09-23).
        if !self.syncer.is_running() {
            return Err(err!(
                "{}: The durability barrier thread has stopped, so the write was refused before \
                anything was written.", self.ozid();
                Thread, Missing, Write));
        }
        let start = res!(self.write_to_file(FileType::Data, vec![&kbyts[..], &vstored[..]]));

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
        res!(self.write_to_file(FileType::Index, vec![&kbyts[..], &istored[..]]));

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
        let insert = OzoneMsg::Insert(
            kbyts,
            Some(vstored),
            cind,
            sfloc.ref_file_location().clone(),
            istored.len(),
            meta,
            resp_w1, // The cbot responds to the caller.
            None,
        );
        let policy = SyncPolicy::of(self.cfg());
        if let Err(e) = self.syncer.hand(Handed::Record { cbot, insert, resp, policy }) {
            // The syncer stopped after the check above, and the record is in the files.
            return Err(err!(e,
                "{}: The record is written, but the durability barrier thread stopped before it \
                could take it, so it is not confirmed durable.", self.ozid();
                Thread, Write, Unconfirmed));
        }

        Ok(())
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
        self.syncer.hand(Handed::Pair(dat, ind))
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
        for path in [&lpair.dat.path, &lpair.ind.path] {
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
        let fnum_old = self.lpair().fnum;
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
    /// not modified in any way.
    ///
    /// # Arguments
    /// * `typ` - `FileType::Data` or `FileType::Index`.
    /// * `v` - bytes to write to the `LivePair`.
    ///
    /// # Errors
    /// * The data length cannot exceed the maximum data file size.
    /// * The process did not write all the data to file.
    /// * The partially written data space could not be recovered.
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

        // [10] Write the data to the live file.
        match typ {
            FileType::Data => {
                let mut bytes_written = 0;
                for vi in v {
                    // [10.1] Value bytes written here.  TODO concat and write once?
                    match self.lpair_mut().dat.file.as_mut() {
                        Some(file) => {
                            match file.write(vi) {
                                Err(e) => {
                                    error!(sync_log::stream(), err!(e,
                                        "{}: while writing to file, rewinding.", self.ozid();
                                        IO, File, Write));
                                    break;
                                },
                                Ok(n) => bytes_written += n,
                            }
                        },
                        None => return Err(err!(
                            "{}: The data file should not be None.", self.ozid();
                            Unreachable)),
                    }
                }
                if bytes_written < vlen {
                    // [10.2] We have a problem, try and rewind the data file pointer.
                    let msg = format!(
                        "{}: Only {} of {} bytes was written to data file {:?}, but \
                        the process has been aborted with no adverse effect on the \
                        integrity of the database{}",
                        self.ozid(), bytes_written, vlen, self.lpair().dat.path,
                        if new_file { " (although a new file was started)" }
                            else { "." },
                    );
                    // [10.3] Rewind the data file pointer.
                    let len = self.lpair().dat.size - 1; 
                    match self.lpair_mut().dat.file.as_mut() {
                        Some(file) => res!(Self::rewind_file_pos(
                            file,
                            len,
                            bytes_written,
                            format!("{} An attempt to recover file space also failed, \
                                again with no impact on database integrity", msg,
                        ))),
                        None => return Err(err!(
                            "{}: The data file should not be None.", self.ozid();
                            Unreachable, Bug)),
                    }

                    return Err(err!(
                        "{} The {} bytes of file space were fully recovered.",
                        msg, bytes_written;
                        IO, File, Write));
                } else {
                    // [10.2] Good write, refresh the cached data file length.
                    self.lpair_mut().dat.size = res!(self.lpair().dat.get_file_len());
                }
                Ok(start)
            },
            FileType::Index => {
                let mut bytes_written = 0;
                for vi in v {
                    // [10.4] Index bytes written here.
                    match self.lpair_mut().ind.file.as_mut() {
                        Some(file) => match file.write(vi) {
                            Err(e) => {
                                error!(sync_log::stream(), err!(e, 
                                    "{}: while writing to file, rewinding.", self.ozid();
                                    IO, File, Write));
                                break;
                            },
                            Ok(n) => bytes_written += n,
                        },
                        None => return Err(err!(
                            "{}: The index file should not be None.", self.ozid();
                            Unreachable, Bug)),
                    }
                }
                if bytes_written < vlen {
                    return Err(err!(
                        "{}: Only {} of {} bytes was written to index file {:?}, but \
                        the process has been aborted with no adverse effect on the \
                        integrity of the database{} The corruption will be detected on \
                        next start up, triggering a more laborious scan of the associated \
                        data file and a re-write of the index file.",
                        self.ozid(), bytes_written, vlen, self.lpair().dat.path,
                        if new_file { " (although a new file was started)" }
                            else { "." };
                        IO, File, Write));
                    // Retain the corrupted index data, it will be detected and dealt
                    // with on re-start.
                }
                Ok(start)
            },
        }
    }

    fn rewind_file_pos(
        file:           &mut File,
        orig_pos:       u64,
        bytes_written:  usize,
        msg:            String,
    )
        -> Outcome<()>
    {
        match file.seek(SeekFrom::Start(orig_pos)) {
            Err(e) => Err(err!(e, "{}.", msg; IO, File, Seek)),
            Ok(actual_pos) => {
                if actual_pos != orig_pos {
                    Err(err!(
                        "{}, the file cursor only rewound {} of the required {} bytes.",
                        msg, actual_pos-orig_pos, bytes_written;
                        IO, File, Seek))
                } else {
                    Ok(())
                }
            },
        }
    }
}

