use crate::{
    prelude::*,
    base::constant,
    bots::{
        base::bot_deps::*,
        worker::worker_deps::*,
    },
    data::{
        cache::{
            MetaLocation,
        },
        core::{
            Key,
            Value,
        },
    },
    file::{
        core::{
            FileAccess,
            FileType,
        },
        fcache::{
            FileCache,
            FileCacheEntry,
            FileCacheIndex,
        },
        floc::{
            FileLocation,
            FileNum,
        },
    },
};

use oxedyne_fe2o3_iop_db::api::Meta;
use oxedyne_fe2o3_jdat::{
    prelude::*,
    id::NumIdDat,
};
use oxedyne_fe2o3_iop_hash::csum::Checksummer;

use std::{
    fs::File,
    io::{
        BufReader,
        Read,
        Seek,
        SeekFrom,
    },
    sync::{
        Arc,
        RwLock,
    },
};

#[derive(Clone, Debug)]
pub enum ReadResult<
    const UIDL: usize,
    UID: NumIdDat<UIDL>,
> {
    // Filebot issued
    None,
    Location(MetaLocation<UIDL, UID>, bool),
    // Cachebot issued
    Value(Vec<u8>, Meta<UIDL, UID>),
    Deleted(Meta<UIDL, UID>),
}

pub struct ReaderBot<
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
    fcache:     FileCache,
    inited:     bool,
}

impl<
    const UIDL: usize,
    UID:    NumIdDat<UIDL> + 'static,
    ENC:    Encrypter + 'static,
    KH:     Hasher + 'static,
	PR:     Hasher,
    CS:     Checksummer,
>
    WorkerBot<UIDL, UID, ENC, KH, PR, CS> for ReaderBot<UIDL, UID, ENC, KH, PR, CS>
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
    OzoneBot<UIDL, UID, ENC, KH, PR, CS> for ReaderBot<UIDL, UID, ENC, KH, PR, CS>
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
    Bot<{ BID_LEN }, Bid, OzoneMsg<UIDL, UID, ENC, KH>> for ReaderBot<UIDL, UID, ENC, KH, PR, CS>
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
                        OzoneMsg::FileReplaced(fnum, typ) => self.drop_cached_file(fnum, &typ),
                        // WORK
                        OzoneMsg::Read(key, cbpind, resp_r1) => {
                            let result = self.read(key, cbpind);
                            self.respond(result, &resp_r1);
                        },
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
    ReaderBot<UIDL, UID, ENC, KH, PR, CS>
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
            fcache:     FileCache::new(constant::FILE_CACHE_EXPIRY_SECS),
            inited:     false,
        }
    }

    fn ref_file_cache(&self)        -> &FileCache       { &self.fcache }
    fn mut_file_cache(&mut self)    -> &mut FileCache   { &mut self.fcache }

    /// Forgets any open handle on the given file, so that the next read of it opens the path
    /// afresh.  A collection renames a new file over an old one, which leaves a cached handle on
    /// the unlinked inode: correct bytes for the file that was, at offsets that belong to the file
    /// that is.  Dropping the entry is the whole of the repair, and it keeps the read path free of
    /// any per-read check for the same thing.
    fn drop_cached_file(
        &mut self,
        fnum:   FileNum,
        typ:    &FileType,
    ) {
        let k = FileCacheIndex { fnum, typ: typ.clone() };
        if self.mut_file_cache().mut_map().remove(&k).is_some() {
            trace!(sync_log::stream(), "{}: Dropped the cached handle on {:?} file {}, which a \
                collection has replaced.", self.ozid(), typ, fnum);
        }
    }

    fn get_file(
        &mut self,
        fnum:   FileNum,
        typ:    &FileType,
    )
        -> Outcome<Arc<RwLock<File>>>
    {
        let k = FileCacheIndex { fnum, typ: typ.clone() };
        // If the cache has the file and its not stale, return it.
        let mut delete = false;
        if let Some(FileCacheEntry{ t, file }) = self.ref_file_cache().ref_map().get(&k) {
            if t.elapsed() < *self.ref_file_cache().expiry() {
                return Ok(Arc::clone(file));
            } else {
                delete = true;
            }
        }
        if delete {
            self.mut_file_cache().mut_map().remove(&k);
        }
        self.open_file(fnum, typ)
    }

    fn open_file(
        &mut self,
        fnum:   FileNum,
        typ:    &FileType,
    )
        -> Outcome<Arc<RwLock<File>>>
    {
        let (_, file) = res!(self.zdir().open_ozone_file(
            fnum,
            typ,
            &FileAccess::Reading,
        ));
        let file_locked = Arc::new(RwLock::new(file));
        let len = self.ref_file_cache().len();
        if len < constant::MAX_CACHED_FILES {
            self.mut_file_cache().insert(fnum, typ, file_locked.clone());
        }
        Ok(file_locked)
    }

    /// Retrieves a value from the database.
	///	2. Asks the key-selected cbot for the value or file location, sending a new responder resp_r2.
	///	3. The cbot accesses its cache.
	///	4. In the case where only the file location is available, the cbot sends the read request (including resp_r2) to the file-selected fbot.
	///	5. The fbot either responds immediately giving the rbot permission to read the file because it is not being garbage collected, incrementing the file state reader count, or else adds the request to a buffer so that permission can be granted later when garbage collection is complete.
	///	6. The rbot waits to receive either the value (via the cbot) or the file location (via the fbot) through resp_r2.  If garbage collection has just been performed, there is a chance that the value was updated during the process.  A flag in the returned value message allows the caller to decide if they want to try the read again, or accept the possibility of an old value.
	///	7. If necessary the rbot reads the file location.
	///	8. Once reading is complete, a finish message is sent to the read channel of the file's fbot.
	///	9. The fbot decrements the reader count for the file state.
	///	10.The rbot returns the value to the caller using resp_r1.
    ///
    fn read(
        &mut self,
        key:    Key,
        cbpind: usize,
    )
        -> Outcome<OzoneMsg<UIDL, UID, ENC, KH>>
    {
        let cind = key.index();

        // A `postgc` location is an offset into the one generation of the file that the collector
        // had just rewritten when the fbot consumed its move map for this record.  A supersession
        // burst can trip the collection trigger on the same file again before this read reaches
        // it, and the reader count that pins a file against collection was not yet held while the
        // request sat in the fbot's gc buffer, so by the time the offset is read the file may have
        // been collected a second time and the offset now belongs to a superseded, unlinked inode.
        // Dropping the cached handle reopens the live file but cannot mend a stale offset; only a
        // fresh location can.  So a `postgc` read whose check fails is retried from the cbot, which
        // the collection re-anchored to the current generation (`cache_data_file`).  The retry is
        // bounded: a file the burst keeps collecting must not spin here for ever, and because each
        // attempt now holds the reader-count pin (see the `ReadFileRequest` arm in bot_file.rs) the
        // attempts converge as soon as the file settles.  A `postgc == false` failure is a genuine
        // fault -- its offset was never remapped -- so it is surfaced at once, never retried.
        for attempt in 0..constant::MAX_POSTGC_READ_ATTEMPTS {

            // <2> Send read request to cbot.
            let resp_r2 = Responder::new(Some(self.ozid()));
            let cbots = res!(self.cbots());
            let bot = res!(cbots.get_bot(cbpind));
            res!(bot.send(OzoneMsg::ReadCache(key.clone(), resp_r2.clone())));

            // <6> We receive either the value or the file location from the cbot or fbot.
            let (floc, meta, postgc) = match resp_r2.recv_timeout(constant::BOT_REQUEST_TIMEOUT) {
                Err(e) => return Err(err!(e,
                    "While waiting on value or location from cbot or fbot.";
                    IO, Channel, Read)),
                Ok(OzoneMsg::ReadResult(readres)) => {
                    match readres {
                        // <10> Return result to caller via resp_r1.
                        ReadResult::None        |
                        ReadResult::Deleted(_)  =>
                            return Ok(OzoneMsg::Value(Value::new(
                                None,
                                cind,
                                false,
                            ))),
                        // <10> Return result to caller via resp_r1.
                        ReadResult::Value(val, meta) => {
                            // All values are wrapped inside a Daticle
                            let (dat, _) = res!(Dat::from_bytes(&val));
                            return Ok(OzoneMsg::Value(Value::new(
                                Some((dat, meta)),
                                cind,
                                false,
                            )));
                        },
                        ReadResult::Location(mloc, postgc) => {
                            let floc = *mloc.file_location();
                            let meta = mloc.meta_move();
                            (floc, meta, postgc)
                        },
                    }
                },
                Ok(msg) => return Err(err!(
                    "Unrecognised response from cbot to read request: {:?}", msg;
                Bug, Invalid, Input)),
            };

            // <7> Read the value from the file location.  If the value was cached, it has been
            // returned above already.
            let vlen = floc.val().len as usize;
            let fnum = floc.file_number();
            // A `postgc == true` offset was remapped through the collector's move map, so it belongs
            // to the inode the rename put behind the path, and any handle this rbot still holds is
            // the pre-rename inode -- unlinked but open -- on which the new offset lands off a record
            // boundary.  The `FileReplaced` notice that would drop it is queued behind this read on a
            // channel the rbot cannot drain while parked inside `read`, so it arrives too late;
            // dropping here reopens the live file.  A `postgc == false` offset is NOT dropped on
            // before its first read: it may be an un-remapped offset that is correct in the handle
            // still cached, and forcing it onto a reopened inode would read a different record that
            // could pass its own checksum.  The drop for a `false` read happens only after its
            // checksum fails, and then only paired with a fresh re-fetch below -- never a reopen of
            // the same offset.
            if postgc {
                self.drop_cached_file(fnum, &FileType::Data);
            }
            let result = self.read_checked(floc);

            // <8> Advise the fbot that reading has finished so it can decrement the reader count it
            // took when it handed back the location.  The count has to come down whether the read
            // worked or not: the fbot will not collect a file whose reader count is above zero, so
            // a read that returned early -- a checksum mismatch is the one that arrives in bursts
            // -- left a count that never came down, and with it a file that could never be
            // collected again for the life of the process.  This is sent on every attempt,
            // matching the increment the fbot takes on every location it hands back.
            let bots = res!(self.fbots());
            let (bot, _) = bots.choose_bot(&ChooseBot::ByFile(fnum));
            res!(bot.send(OzoneMsg::ReadFinished(fnum)));

            match result {
                Ok(mut val) => {
                    val.truncate(vlen - res!(self.api().schms.checksummer().len()));
                    // All values are wrapped inside a Dat::BU64.
                    let (dat, _) = res!(Dat::from_bytes(&val));
                    return Ok(OzoneMsg::Value(Value::new(
                        Some((dat, meta)),
                        cind,
                        postgc,
                    )));
                },
                Err(e) => {
                    // A checksum failure on a store that is byte-perfect on disk means the offset
                    // just read and the handle it was read through disagree on which generation of
                    // the file they belong to: a `postgc` offset into an inode a later collection
                    // has already replaced, or a current offset read through a handle still open on
                    // the pre-rename inode.  The final attempt must never risk returning bytes from
                    // the wrong generation, so it fails loud.
                    if attempt + 1 == constant::MAX_POSTGC_READ_ATTEMPTS {
                        return Err(err!(e,
                            "{}: While reading {:?} file {} (postgc {}, attempt {} of {}).",
                            self.ozid(), FileType::Data, fnum, postgc,
                            attempt + 1, constant::MAX_POSTGC_READ_ATTEMPTS;
                            IO, File, Read));
                    }
                    // Drop the handle whose generation disagreed and loop.  The next attempt fetches
                    // a fresh location from the cbot, which the collection re-anchored to the
                    // current generation before it renamed the file (`cache_data_file`'s cbot cache
                    // update completes inside the call at collect_file step 6, before the rename at
                    // step 7), then reads THAT offset against the reopened live inode.  The offset
                    // that just failed is discarded, never re-read through a reopened inode: that
                    // pairing -- an un-remapped offset on the new inode -- is the one that could read
                    // a different record with a valid checksum, and it never happens here.
                    self.drop_cached_file(fnum, &FileType::Data);
                },
            }
        }

        // The loop returns on the first clean read and on a spent retry budget, so this is only
        // reached if the budget is zero, which the constant forbids.
        Err(err!(
            "{}: Read of a value exhausted its retry budget without a result.", self.ozid();
            Bug, IO, Read))
    }

    /// Reads the record at the given location and checks it against its stored checksum.  Split
    /// out of `read` only so that the caller can report the read finished to the fbot on the way
    /// out, whichever way this goes.
    fn read_checked(
        &mut self,
        floc: FileLocation,
    )
        -> Outcome<Vec<u8>>
    {
        let val = res!(self.read_from_file(floc));
        res!(self.api().schemes().checksummer().clone().verify(&val));
        Ok(val)
    }

    fn read_from_file(
        &mut self,
        floc: FileLocation,
    )
        -> Outcome<Vec<u8>>
    {
        let locked_file = res!(self.get_file(floc.file_number(), &FileType::Data));
        let mut file_write = lock_write!(locked_file, // seek requires mutability
            "{}: While trying to read from the cached data file number {}.",
            self.ozid(), floc.file_number(),
        );

        match file_write.seek(SeekFrom::Start(floc.val().start)) {
            Err(e) => return Err(err!(e,
                "{}: attempt to move to position {} in data file {}.",
                self.ozid(), floc.val().start, floc.file_number();
                IO, File, Seek)),
            Ok(actual_pos) => {
                if actual_pos != floc.val().start {
                    return Err(err!(
                        "{}: attempt to move to position {} in data file {} \
                        but only moved to {}.",
                        self.ozid(), floc.val().start, floc.file_number(), actual_pos;
                        IO, File, Seek));
                }
                let mut v = vec![0; floc.val().len as usize];
                let file_clone = res!((*file_write).try_clone());
                let mut reader = BufReader::new(file_clone);
                match reader.read(&mut v) {
                    Err(e) => {
                        return Err(err!(e,
                            "{}: attempt to read {} bytes from position {} in data file {}.",
                            self.ozid(), floc.val().len, floc.val().start, floc.file_number();
                            IO, File, Read));
                    },
                    Ok(actually_read) => {
                        if actually_read != floc.val().len as usize {
                            return Err(err!(
                                "{:?}: attempt to read {} bytes from position {} \
                                in data file {}, but only read {} bytes.",
                                self.ozid(), floc.val().len, floc.val().start, floc.file_number(),
                                actually_read;
                                IO, File, Read));
                        }
                        return Ok(v);
                    },
                }
            }
        }
    }
}
