use crate::{
    prelude::*,
    base::constant,
    bots::{
        base::bot_deps::*,
        worker::worker_deps::*,
    },
    file::{
        core::FileType,
        floc::{
            FileNum,
            StoredFileLocation,
        },
        live::LivePair,
    },
};

use oxedize_fe2o3_iop_db::api::Meta;
use oxedize_fe2o3_jdat::id::NumIdDat;

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
    inited:     bool,
    lpair:      LivePair,
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
    }

    fn listen(&mut self) -> LoopBreak {
        match self.chan_in().recv() {
            Err(e) => self.err_cannot_receive(err!(e, errmsg!(
                "{}: Waiting for message.", self.ozid(),
            ), IO, Channel)),
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
                            match result {
                                Err(e) => self.error(e),
                                Ok(_) => self.respond(Ok(OzoneMsg::Ok), &resp),
                            }
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
                            let result = self.write(
                                kstored,
                                vstored,
                                klen_cache,
                                cind,
                                meta,
                                cbpind,
                                resp_w1,
                            );
                            self.result(&result);
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
            api:        args.api,
            // State
            active:     false,
            inited:     false,
            lpair:      LivePair::default(),
        }
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

        // [11] Send the data to a cbot.
        let cbots = res!(self.cbots());
        let bot = res!(cbots.get_bot(cbpind));
        kbyts.drain(..constant::CACHE_HASH_BYTES); // remove data pathway hash used to identify cbot
        kbyts.truncate(klen_cache); // remove metadata
        res!(bot.send(OzoneMsg::Insert(
            kbyts,
            Some(vstored),
            cind,
            sfloc.ref_file_location().clone(),
            istored.len(),
            meta,
            resp_w1, // The cbot responds to the caller.
        )));
        
        Ok(())
    }

    fn open_live_pair(&mut self) -> Outcome<()> {
        self.lpair.close();
        self.lpair = res!(self.zdir().open_live(self.lpair.fnum));
        Ok(())
    }

    fn new_live_pair(&mut self) -> Outcome<(FileNum, u64)> {
        let fnum_old = self.lpair().fnum;
        // [3] Ask zbot for next live file number.
        let resp = Responder::new(Some(self.ozid()));
        match self.zbot() {
            None => return Err(err!(errmsg!(
                "{}: Could not get zbot work channel.", self.ozid(),
            ), Missing, Data)),
            Some(zbot) => 
                res!(zbot.send(OzoneMsg::NextLiveFile(resp.clone()))),
        }
        let fnum_new = match resp.recv_timeout(constant::BOT_REQUEST_TIMEOUT) {
            Err(e) => return Err(err!(e, errmsg!(
                "While getting next live file info from zbot.",
            ), IO, Channel, Read)),
            Ok(OzoneMsg::UseLiveFile(fnum)) => fnum,
            Ok(msg) => return Err(err!(errmsg!(
                "Unrecognised new live file request response: {:?}", msg,
            ), Bug, Invalid, Input)),
        };

        self.lpair.close();
        self.lpair = res!(self.zdir().open_live(fnum_new));
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
            Err(e) => return Err(err!(e, errmsg!(
                "While advising fbot to update live file states.",
            ), IO, Channel, Read)),
            Ok(OzoneMsg::Ok) => (),
            Ok(msg) => return Err(err!(errmsg!(
                "Unrecognised response after advising fbot to update live file states: {:?}", msg)),
            ),
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
            return Err(err!(errmsg!(
                "Attempt to store {:?} value of length {} bytes \
                exceeds the config setting of {}.",
                typ, vlen, max_file_len,
            ), IO, File, Write, Input, TooBig));
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
                                    error!(err!(e, fmt!("{}: while writing to file, rewinding.", self.ozid())));
                                    break;
                                },
                                Ok(n) => bytes_written += n,
                            }
                        },
                        None => return Err(err!(errmsg!(
                            "{}: The data file should not be None.", self.ozid(),
                        ), Unreachable)),
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
                        None => return Err(err!(errmsg!(
                            "{}: The data file should not be None.", self.ozid(),
                        ), Unreachable, Bug)),
                    }

                    return Err(err!(errmsg!(
                        "{} The {} bytes of file space were fully recovered.",
                        msg, bytes_written,
                    ), IO, File, Write));
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
                                error!(err!(e, fmt!("{}: while writing to file, rewinding.", self.ozid())));
                                break;
                            },
                            Ok(n) => bytes_written += n,
                        },
                        None => return Err(err!(errmsg!(
                            "{}: The index file should not be None.", self.ozid(),
                        ), Unreachable, Bug)),
                    }
                }
                if bytes_written < vlen {
                    return Err(err!(errmsg!(
                        "{}: Only {} of {} bytes was written to index file {:?}, but \
                        the process has been aborted with no adverse effect on the \
                        integrity of the database{} The corruption will be detected on \
                        next start up, triggering a more laborious scan of the associated \
                        data file and a re-write of the index file.",
                        self.ozid(), bytes_written, vlen, self.lpair().dat.path,
                        if new_file { " (although a new file was started)" }
                            else { "." },
                    ), IO, File, Write));
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
            Err(e) => Err(err!(e, errmsg!("{}.", msg), IO, File, Seek)),
            Ok(actual_pos) => {
                if actual_pos != orig_pos {
                    Err(err!(errmsg!(
                        "{}, the file cursor only rewound {} of the required {} bytes.",
                        msg, actual_pos-orig_pos, bytes_written,
                    ), IO, File, Seek))
                } else {
                    Ok(())
                }
            },
        }
    }
}

