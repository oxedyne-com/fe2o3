use crate::{
    prelude::*,
    base::constant,
    bots::{
        base::bot_deps::*,
        worker::worker_deps::*,
    },
    data::{
        choose::ChooseCache,
    },
    file::{
        core::FileAccess,
        floc::{
            DataLocation,
            FileNum,
            StoredFileLocation,
        },
        state::{
            DataState,
            FileState,
        },
        stored::{
            StoredIndex,
            StoredKey,
            StoredValue,
        },
    },
};

use oxedize_fe2o3_iop_db::api::Meta;
use oxedize_fe2o3_jdat::id::NumIdDat;

use std::{
    fs::{
        self,
        File,
        OpenOptions,
    },
    io::{
        BufReader,
        BufWriter,
        Seek,
        SeekFrom,
        Read,
        Write,
    },
    path::PathBuf,
    sync::Arc,
};

/// `InitGarbageBot`s have two functions:
/// 1. Initialisation where they are asked to read files and fill the caches.
/// 2. Garbage collection where they subsequently regularly rewrite data files to remove stale
///    data.
pub struct InitGarbageBot<
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
    WorkerBot<UIDL, UID, ENC, KH, PR, CS> for InitGarbageBot<UIDL, UID, ENC, KH, PR, CS>
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
    OzoneBot<UIDL, UID, ENC, KH, PR, CS> for InitGarbageBot<UIDL, UID, ENC, KH, PR, CS>
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
    Bot<{ BID_LEN }, Bid, OzoneMsg<UIDL, UID, ENC, KH>> for InitGarbageBot<UIDL, UID, ENC, KH, PR, CS>
{
    bot_methods!();

    fn go(&mut self) {
        if self.no_init() { return; }
        self.now_listening();
        loop {
            if self.listen().must_end() { break; }
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
                        // Init
                        OzoneMsg::CacheDataFile {
                            fnum,
                            dat_file_size,
                            resp,
                        } => {
                            let result = self.cache_file(
                                fnum,
                                &FileType::Data,
                                dat_file_size,
                                0,
                            );
                            self.respond(result, &resp);
                        },
                        OzoneMsg::CacheIndexFile {
                            fnum,
                            dat_file_size,
                            ind_file_size,
                            resp,
                        } => {
                            let result = self.cache_file(
                                fnum,
                                &FileType::Index,
                                dat_file_size,
                                ind_file_size,
                            );
                            self.respond(result, &resp);
                        },
                        // Garbage collection
                        OzoneMsg::CollectGarbage {
                            fnum,
                            fstat,
                            fbot_index,
                        } => {
                            let result = self.collect_garbage(
                                fnum,
                                fstat,
                                fbot_index,
                            );
                            self.result(&result);
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
    InitGarbageBot<UIDL, UID, ENC, KH, PR, CS>
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
            inited:     false,
        }
    }

    /// Decide how to cache the file.
    fn cache_file(
        &mut self,
        fnum:       FileNum,
        typ:        &FileType,
        dat_size:   usize,
        ind_size:   usize,
    )
        -> Outcome<OzoneMsg<UIDL, UID, ENC, KH>>
    {
        let (_, file) = res!(self.zdir().open_ozone_file(
            fnum,
            typ,
            &FileAccess::Reading,
        ));
        let meta = res!(file.metadata());
        let reader = BufReader::new(file);
        match typ {
            FileType::Index => {
                if meta.len() == 0 {
                    // If there is nothing to cache, try caching the data file.
                    warn!("{}: The index file {} is empty, trying data file...",
                        self.ozid(), fnum);
                    let (_, file) = res!(self.zdir().open_ozone_file(
                        fnum,
                        &FileType::Data,
                        &FileAccess::Reading,
                    ));
                    let reader = BufReader::new(file);
                    res!(self.init_cache_data_file(
                        reader,
                        fnum,
                        dat_size,
                    ));
                } else {
                    match self.init_cache_index_file(
                        reader,
                        fnum,
                        dat_size,
                        ind_size,
                    ) {
                        Err(e) => {
                            // If caching the index file fails, cache the data file.
                            warn!("{}: Error caching index file {}, trying data file, caused by {}.",
                                self.ozid(), fnum, e);
                            let (_, file) = res!(self.zdir().open_ozone_file(
                                fnum,
                                &FileType::Data,
                                &FileAccess::Reading,
                            ));
                            let reader = BufReader::new(file);
                            res!(self.init_cache_data_file(
                                reader,
                                fnum,
                                dat_size,
                            ));
                        },
                        Ok(()) => (),
                    }
                }
            },
            FileType::Data => res!(self.init_cache_data_file(
                reader,
                fnum,
                dat_size,
            )),
        }
        Ok(OzoneMsg::Ok)
    }

    /// Use the index file to update the cache with data file value locations, which should be
    /// quicker than scanning the data file itself.
    #[allow(unused_assignments, unused_variables)]
    fn init_cache_index_file(
        &mut self,
        mut reader: BufReader<File>,
        fnum:       FileNum,
        dat_size1:  usize,
        ind_size:   usize,
    )
        -> Outcome<()>
    {
        let mut pos = 0;
        let mut count = 0;
        let mut dat_size2: u64 = 0;
        let typ = FileType::Index;

        loop {
            // 1. Load the key Daticle bytes and while we're at it, compare the checksum.
            let (key, meta, chash) = match StoredKey::load(
                &mut reader,
                self.api().schemes().checksummer().clone(),
            ) {
                Err(e) => return Err(err!(e,
                    "{}: While reading from position {} in {:?} file {}.",
                    self.ozid(), pos, typ, fnum;
                    IO, File, Read)),
                Ok(None) => break, // We're done.
                Ok(Some((skey, _, n))) => {
                    count += 1;
                    pos += n;
                    let meta = skey.meta().clone();
                    let chash = skey.ref_chash().clone();
                    (skey.into_key(), meta, chash)
                },
            };
            // 2. Read the StoredIndex.
            match StoredIndex::read(
                &mut reader,
                fnum,
                self.api().schemes().checksummer().clone(),
            ) {
                Err(e) => return Err(err!(e,
                    "{}: While reading from position {} in {:?} file {}.",
                    self.ozid(), pos, typ, fnum;
                    IO, File, Read)),
                Ok((None, _)) => return Err(err!(
                    "{}: Missing StoredIndex at end of {:?} file {}.",
                    self.ozid(), typ, fnum;
                    Missing)),
                Ok((Some(sindex), n)) => {
                    count += 1;
                    pos += n;
                    dat_size2 += sindex.keyval_len();
                    // 3. Insert the key and location into the bot cache, informing an fbot about
                    //    new data and old data that can be scheduled for garbage collection.  The
                    //    bot we advise actually performs any garbage collection, so instead of
                    //    choosing randomly, we allocate each bot to an exclusive fraction of files
                    //    based on their number.
                    let cind = key.index();
                    let kbyts = key.into_bytes();
                    let chash = res!(<alias::ChooseHash>::try_from(
                        &chash[..constant::CACHE_HASH_BYTES]));
                    let cbwind = ChooseCache::<PR>::choose_cbot_select(
                        alias::ChooseHashUint::from_be_bytes(chash),
                        self.cfg().num_zones,
                        self.cfg().num_cbots_per_zone,
                    );
                    let cbots = res!(self.cbots());
                    let bot = res!(cbots.get_bot(**cbwind.bpind()));
                    res!(bot.send(
                        OzoneMsg::Insert(
                            kbyts,
                            None,
                            cind,
                            sindex.ref_file_location().clone(),
                            sindex.ref_stored_file_location().buf.len(),
                            meta,
                            Responder::none(Some(self.ozid())),
                        )
                    ));
                },
            }
        }

        // 8. Do size check.
        if pos != ind_size {
            return Err(err!(
                "{}: After initial caching of data file {} using the index file, the \
                index file data count came to {} bytes, but the originally surveyed file \
                size was {}.", self.ozid(), fnum, pos, ind_size;
                Mismatch, Data));
        }
        if dat_size1 != res!(usize::try_from(dat_size2)) {
            return Err(err!(
                "{}: After initial caching of data file {} using the index file, the \
                file data count came to {} bytes, but the originally surveyed file \
                size was {}.", self.ozid(), fnum, dat_size2, dat_size1;
                Mismatch, Data));
        }

        Ok(())
    }

    /// When a valid index file is not available, scan the data file directly and update the cache
    /// with value locations.  We do not read and decode the values, so only the locations go into
    /// the cache.
    #[allow(unused_assignments, unused_variables)]
    pub fn init_cache_data_file(
        &mut self,
        mut reader: BufReader<File>,
        fnum:       FileNum,
        dat_size:   usize,
    )
        -> Outcome<()>
    {
        let typ = FileType::Data;
        let mut index_file_buffer = Vec::new();

        // 1. Make sure the index file really is gone.
        let mut path = self.zdir().dir.clone();
        path.push(ZoneDir::relative_file_path(&FileType::Index, fnum));
        if path.is_file() {
            res!(fs::remove_file(path));
        }

        // 2. Create the new index file.
        let (_, file) = res!(self.zdir().open_ozone_file(
            fnum,
            &FileType::Index,
            &FileAccess::Writing,
        ));
        let mut writer = BufWriter::new(file);

        let mut pos = 0;
        let mut kpos = 0;
        let mut klen = 0;
        let mut count = 0;

        let csum_len = res!(self.api().schemes().checksummer().len());

        loop {
            // 3. Load the key Daticle bytes and while we're at it, compare the checksum.
            let (key, meta, chash) = match StoredKey::load(
                &mut reader,
                self.api().schemes().checksummer().clone(),
            ) {
                Err(e) => return Err(err!(e,
                    "{}: While reading from position {} in {:?} file {}, \
                    having read {} items.",
                    self.ozid(), pos, typ, fnum, count;
                    IO, File, Read)),
                Ok(None) => break,
                Ok(Some((skey, mut skbyts, n))) => {
                    count += 1;
                    kpos = pos;
                    klen = n;
                    pos += n;
                    let chash = skey.ref_chash().clone();
                    index_file_buffer.extend_from_slice(skey.ref_chash());
                    index_file_buffer.append(&mut skbyts);
                    let meta = skey.meta().clone();
                    (skey.into_key(), meta, chash)
                },
            };
            // 4. Count the value Daticle bytes.  Dat::count_bytes also moves the
            //    reader cursor.
            match StoredValue::count(
                &mut reader,
                csum_len,
            ) {
                Err(e) => return Err(err!(e,
                    "{}: While reading from position {} in {:?} file {}, \
                    having read {} items.",
                    self.ozid(), pos, typ, fnum, count;
                    IO, File, Read)),
                Ok(0) => return Err(err!(
                    "{}: Missing value at end of {:?} file {}.",
                    self.ozid(), typ, fnum;
                    IO, File, Data, Missing)),
                Ok(n) => {
                    // 5. Create the FileLocation.
                    let sfloc = res!(StoredFileLocation::new( // do this before incrementing pos
                        fnum,
                        kpos as u64,
                        klen as u64,
                        n as u64,
                        self.api().schemes().checksummer().clone(),
                    ));
                    count += 1;
                    pos += n;

                    // 6. Insert the key and location into the bot cache, informing a gbot about
                    //    new data and old data that can be scheduled for garbage collection.  The
                    //    bot we advise actually performs any garbage collection, so instead of
                    //    choosing randomly, we allocate each bot to an exclusive fraction of files
                    //    based on their number.
                    let cind = key.index();
                    let kbyts = key.into_bytes();
                    let chash = res!(<alias::ChooseHash>::try_from(
                        &chash[..constant::CACHE_HASH_BYTES]));
                    let cbwind = ChooseCache::<PR>::choose_cbot_select(
                        alias::ChooseHashUint::from_be_bytes(chash),
                        self.cfg().num_zones,
                        self.cfg().num_cbots_per_zone,
                    );
                    let cbots = res!(self.cbots());
                    let bot = res!(cbots.get_bot(**cbwind.bpind()));
                    let ibuf = &sfloc.buf;
                    debug!("++++++ data");
                    res!(bot.send(
                        OzoneMsg::Insert(
                            kbyts,
                            None,
                            cind,
                            sfloc.ref_file_location().clone(),
                            ibuf.len(),
                            meta,
                            Responder::none(Some(self.ozid())),
                        )
                    ));

                    // 7. Append to the index file buffer.
                    //let ibuf = StoredIndex::as_bytes(&floc);
                    index_file_buffer.extend_from_slice(ibuf);
                },
            }
        }

        // 8. Do size check.
        if pos != dat_size {
            return Err(err!(
                "{}: After initial caching of data file {}, the total data count \
                came to {} bytes, but the originally surveyed file size was {}.",
                self.ozid(), fnum, pos, dat_size;
                Mismatch, Data));
        }

        // 10. Write the index file in one go.
        res!(writer.write(&index_file_buffer));

        Ok(())
    }

    /// Performs garbage collection on the given file.  Assumes that the move map for the file is
    /// empty.  The basic idea is to transcribe (re-write) the data file, skipping sections
    /// scheduled for deletion.  While this process is going on, deletion messages can continue to
    /// be queue up on the fbot write channel.  It is therefore necessary to create a "move map" in
    /// the file state which maps the old data locations to their new locations in the file, later
    /// allowing those queued deletions to correctly apply to the new locations.  After
    /// transcription, the data in the new file is cached using a similar approach to a cbot.
    ///```ignore
    ///                                                           Garbage collection of file fg  
    ///                               file to         live        involves transcription of      
    ///                               be gc'd         file        current key-value pairs       
    ///                                                           with starting locations s01    
    ///                    f1            fg            fL         and s02 to new starting        
    ///                 +------+      +------+      +------+      locations s11 and s12. Old         
    ///                 |      |      |      |      |      |      values in fg are not copied
    ///                 |      |      |      |      |      |      and thereby deleted.
    /// original        |      |   k1 |\\\\\\| s01  |      |      
    /// data            |      |      |      |      |      |      However, while this occurs we  
    /// files           |      |  ... |      | ...  |      |      want the writer bot(s) to      
    ///                 |      |      |      |      |      |      continue appending data to     
    ///                 |      |   k2 |//////| s02  |      |      live files, which could include
    ///                 |      |      |      |      |      |      new values for k1 and k2.               
    ///                 +------+      +------+      +------+      
    ///                                                            
    ///                                new fg                      
    /// new data                  
    /// files                     s11 |\\\\\\|
    /// (post-gc)                 s12 |//////|
    ///                                                                 cache changes
    /// 
    ///                     |     scheduled for      |      third party      |      gc updates
    ///     scenarios       |     deletion in fg     |        changes        |       required  
    ///                     |     after gc started   |       (examples)      |
    /// --------------------+------------------------+-----------------------+------------------------
    ///    1. None          |         <none>         |                       | k1:(fg,s01)->(fg,s11)
    ///                     |                        |                       |
    ///    2. New value(s)  |      s11 <- s01 <-     | k1:(fg,s01)->(fL,s21) |
    ///                     |                        | k1:(fL,s21)->(fL,s31) |
    ///                     |                        |                       |
    ///                     |                        |                       |
    ///                     |                        |                       |
    ///
    ///```
    /// Returns whether the file state can be eliminated because the data file has been completely
    /// deleted, or the new data file size.
    fn collect_garbage(
        &mut self,
        fnum:       FileNum,
        mut fstat:  FileState,
        fbot_index: usize,
    )
        -> Outcome<()>
    {
        // [19] Perform transcription from data_reader to data_writer.

        trace!("{}: Performing garbage collection on file {}...", self.ozid(), fnum);
        let typ = FileType::Data;
        // 1. Open the data file for reading.
        let (data_path, file) = res!(self.zdir().open_ozone_file(
            fnum,
            &typ,
            &FileAccess::Reading,
        ));
        let data_file_len = res!(file.metadata()).len();
        let old_size = data_file_len as usize;
        let mut data_reader = BufReader::new(file);

        // 2. Create new, temporary data file for writing.
        let mut tmp_data_path = self.zdir().dir.clone();
        let mut filename = PathBuf::from(".gc");
        filename.set_extension(
            ZoneDir::relative_file_path(&typ, fnum)
        );
        tmp_data_path.push(filename);
        let mut new_start: u64 = 0;
        let old_sum = try_into!(usize, fstat.get_old_sum());
        
        {
            let file = res!(ZoneDir::open_file(
                &tmp_data_path,
                &FileAccess::Writing,
            ));
            // Rust and/or linux seems to require that this BufWriter on a write-only file (
            // creation sets it to write-only) be closed before we can open a BufReader to the
            // same file, so we create this special scope for data_writer.
            let mut data_writer = BufWriter::new(file);

            // 3. Transcribe the existing data file to the temporary file, skipping old key-value pairs.
            let mut old_start1: u64 = 0;
            let mut dstat1 = None;
            let mut first = true;
            for old_start2 in res!(fstat.get_data_start_positions()) {
                if !first {
                    let dloc = DataLocation {
                        start:  old_start1,
                        len:    old_start2 - old_start1,
                    };
                    match dstat1 {
                        Some(DataState::Cur) => {
                            let mut buf = vec![0u8; dloc.len as usize];
                            res!(data_reader.seek(SeekFrom::Start(dloc.start)));
                            match data_reader.read_exact(&mut buf) {
                                Err(e) => return Err(err!(e,
                                    "{}: While trying to read exactly {} bytes from position \
                                    {} in file {} of {} bytes length, {:?}.  The file state is {:?}.",
                                    self.ozid(), dloc.len, dloc.start, fnum,
                                    data_file_len, data_path, fstat;
                                    IO, File, Read)),
                                Ok(()) => (),
                            }
                            res!(data_writer.write_all(&mut buf));
                            fstat.update_moved(&dloc, new_start);
                            new_start += dloc.len;
                        },
                        Some(DataState::Old) => {
                            res!(fstat.retire_old(&dloc));
                        },
                        None => break,
                    }
                } else {
                    first = false;
                }
                old_start1 = old_start2;
                dstat1 = fstat.get_data_state(old_start2).cloned();
            }
        }
        
        let new_size = new_start as usize;

        // 4. Do some checks.
        if new_size > old_size {
            return Err(err!(
                "{}: The file {} has grown in size from {} to {} after garbage \
                collection, this should not occur in Ozone.",
                self.ozid(), fnum, old_size, new_size;
                Bug, Missing, Data));
        }
        if old_sum != old_size - new_size {
            return Err(err!(
                "{}: The file {} was scheduled to remove {} bytes, but instead \
                removed {} bytes, going from {} to {} bytes.", 
                self.ozid(), fnum, old_sum, old_size - new_size, old_size, new_size;
                Bug, Mismatch, Data));
        }
        if !fstat.data_map_empty() {
            return Err(err!(
                "{}: Garbage collection for file {} should have cleared out the \
                data map, instead it still contains entries, {:?}.",
                self.ozid(), fnum, fstat.data_map();
                Bug, Mismatch, Data));
        }

        if new_size == 0 {
            return Err(err!(
                "{}: Garbage collection for file {} has deleted the entire data file, \
                however this should have been done by the fbot.",
                self.ozid(), fnum;
                Bug, Mismatch, Data));
        }

        let mut dat_ind_file_size_decrease = old_sum;
        fstat.set_data_file_size(new_size);
        let old_ind_size = fstat.get_index_file_size();

        // 6. Re-create the index file by scanning the new data file.  Any values remaining in the
        //    data file are unique and we must handle a few scenarios.
        let file = match OpenOptions::new().read(true).open(&tmp_data_path) {
            Err(e) => return Err(err!(e, "While opening file {:?}", tmp_data_path; IO, File, Read)),
            Ok(f) => f,
        };
        let new_data_reader = BufReader::new(file);
        fstat = res!(self.cache_data_file(
            new_data_reader,
            fnum,
            fstat,
        ));

        if fstat.get_index_file_size() > old_ind_size {
            return Err(err!(
                "{}: Indexing of the new data file {} after garbage collection \
                has resulted in unexpected growth of the index file from {} to \
                {} bytes.",
                self.ozid(), fnum, old_ind_size, fstat.get_index_file_size();
                Bug, Mismatch, Data));
        }
        dat_ind_file_size_decrease += old_ind_size - fstat.get_index_file_size();

        // 7. Replace the old data file with the new temporary file.
        res!(fs::rename(tmp_data_path, data_path));

        // 8. Reset FileState.
        fstat.reset_old_accounting();

        // [22] Send updated file state back to the fbot.
        let bots = res!(self.fbots());
        let bot = res!(bots.get_bot(fbot_index));
        if let Err(e) = bot.send(
            OzoneMsg::GcCompleted(
                fnum,
                fstat,
                dat_ind_file_size_decrease,
            )
        ) {
            return Err(err!(e,
                "{}: Cannot send updated file state for file number {} to fbot {}",
                self.ozid(), fnum, fbot_index;
                Channel, Write));
        }
        debug!("{}: Reduced file {} size by {:.1}% from {} to {} bytes.",
            self.ozid(),
            fnum,
            100.0 * ((old_size - new_size) as f32) / (old_size as f32),
            old_size,
            new_size,
        );
        Ok(())
    }

    /// This method is similar to the initialisation method `init_cache_data_file` in that the
    /// data file is scanned and a new index file is created, however we must deal with the cache
    /// and deletion scheduling differently.  Initialisation can rely on the chronological order of
    /// data and work its way sequentially through files, but here we want to allow values to be
    /// added to live files while we collect the garbage and therefore must update the cache and
    /// file state depending on live file changes.
    pub fn cache_data_file(
        &mut self,
        mut reader: BufReader<File>,
        fnum:       FileNum,
        mut fstat:  FileState,
    )
        -> Outcome<FileState>
    {
        let typ = FileType::Data;
        // 1. Make sure the index file really is gone.
        let mut path = self.zdir().dir.clone();
        path.push(ZoneDir::relative_file_path(&FileType::Index, fnum));
        if path.is_file() {
            res!(fs::remove_file(path));
        }
        fstat.reset_index_file_size();

        let mut pos = 0;
        let mut count = 0;

        // Prepare to buffer request to update caches.
        let resp_g1 = Responder::new(Some(self.ozid()));
        let nc = self.cfg().num_cbots_per_zone();
        let mut buffers = Vec::new();
        for _ in 0..nc {
            buffers.push(Vec::new());
        }

        {
            // 2. Create the new index file.
            let (_, file) = res!(self.zdir().open_ozone_file(
                fnum,
                &FileType::Index,
                &FileAccess::Writing,
            ));
            let mut writer = BufWriter::new(file);

            loop {
                // 3. Load the key Daticle bytes and while we're at it, compare the checksum.
                let (key, klen, kpos, _meta, mut index_entry, chash) =
                    match StoredKey::load(
                        &mut reader,
                        self.api().schemes().checksummer().clone(),
                    ) {
                        Err(e) => return Err(err!(e,
                            "{}: While reading from position {} in {:?} file {}, \
                            having read {} items.",
                            self.ozid(), pos, typ, fnum, count;
                            IO, File, Read)),
                        Ok(None) => break,
                        Ok(Some((skey, skbyts, n))) => {
                            count += 1;
                            let kpos = pos;
                            pos += n;
                            let klen = n;
                            let meta: Meta<UIDL, UID> = skey.meta().clone();
                            let chash = skey.ref_chash().clone();
                            let key = skey.into_key();
                            let index_entry = skbyts;
                            (key, klen, kpos, meta, index_entry, chash)
                        },
                    };
                // 4. Count the value Daticle bytes.  Dat::count_bytes also moves the
                //    reader cursor.
                match StoredValue::count(
                    &mut reader,
                    res!(self.api().schemes().checksummer().len()),
                ) {
                    Err(e) => return Err(err!(e,
                        "{}: While reading from position {} in {:?} file {}, \
                        having read {} items.",
                        self.ozid(), pos, typ, fnum, count;
                        IO, File, Read)),
                    Ok(0) => return Err(err!(
                        "{}: Missing value at end of {:?} file {}.",
                        self.ozid(), typ, fnum;
                        Missing, IO, File)),
                    Ok(n) => {
                        let new_start = kpos as u64;
                        // 5. Create the FileLocation.
                        let sfloc = res!(StoredFileLocation::new( // do this before incrementing pos
                            fnum,
                            new_start,
                            klen as u64,
                            n as u64,
                            self.api().schemes().checksummer().clone(),
                        ));
                        count += 1;
                        pos += n;

                        // 6. Update existing cache reference to this key.  If the cached location
                        //    still points to this file, update the starting position.
                        let chash = res!(<alias::ChooseHash>::try_from(
                            &chash[..constant::CACHE_HASH_BYTES]));
                        let cbwind = ChooseCache::<PR>::choose_cbot_select(
                            alias::ChooseHashUint::from_be_bytes(chash),
                            self.cfg().num_zones,
                            self.cfg().num_cbots_per_zone,
                        );
                        let bpind = cbwind.bpind();
                        buffers[**bpind].push((
                            key.into_bytes(),
                            sfloc.ref_file_location().clone(),
                        ));

                        // 8. Append to the index file and update the effect of the file size increase
                        //    on the file state and the directory size.
                        //let ibuf = StoredIndex::as_bytes(&mut floc); // update floc encoded length
                        let ibuf = sfloc.buf;
                        index_entry.extend_from_slice(&ibuf);
                        res!(writer.write(&index_entry));
                        res!(fstat.inc_index_file_size(index_entry.len()));
                    },
                }
            }
        } // close out that writer

        // Send cache update request batches to each cbot.
        let bots = res!(self.cbots());
        for i in (0..nc).rev() {
            let bot = res!(bots.get_bot(i));
            if let Some(buf) = buffers.pop() { // Transfer ownership to OzoneMsg.
                if let Err(e) = bot.send(
                    OzoneMsg::GcCacheUpdateRequest(
                        buf,
                        resp_g1.clone(),
                    )
                ) {
                    return Err(err!(e,
                        "Cannot send gc cache update requests to cbot {}", i;
                        Channel, Write));
                }
            } else {
                return Err(err!(
                    "The number of cache bots is {} should match the number of buffers, {}.",
                    nc, nc - i;
                    Bug, Mismatch, Size));
            }
        }
        // Wait for each response.
        for _ in 0..nc {
            match resp_g1.recv_timeout(constant::BOT_REQUEST_TIMEOUT) {
                Err(e) => return Err(err!(e,
                    "While collecting gc cache update response.";
                    IO, Channel, Read)),
                Ok(OzoneMsg::GcCacheUpdateResponse(old_flocs)) => {
                    for old_floc in old_flocs {
                        // A cache update was performed, meaning the value in this file is still
                        // the current value (i.e. the value was not updated during the garbage
                        // collection transcription process).  During transcription, the data
                        // location was mapped from old to new in via a FileState.smap DataState
                        // entry, e.g.  DataState::Cur(Some(new_start)).  We no longer need this
                        // mapping, because the cache now points to the new position.  So we update
                        // the FileState to reflect the new position, and remove the old DataState
                        // which provided the mapping.  There is no change in the data file size.
                        fstat.map_and_remove(&old_floc.keyval());
                    }
                },
                Ok(msg) => return Err(err!(
                    "Unrecognised cache initialisation request response: {:?}", msg;
                    Channel, Unknown)),
            }
        }
        Ok(fstat)
    }
}
