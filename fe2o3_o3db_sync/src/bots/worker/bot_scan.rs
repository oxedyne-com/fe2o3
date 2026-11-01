use crate::{
    prelude::*,
    base::id,
    bots::{
        base::bot_deps::*,
        worker::worker_deps::*,
    },
    data::cache::supersedes,
    file::{
        floc::{
            FileLocation,
            FileNum,
        },
        stored::{
            StoredIndex,
            StoredKey,
        },
    },
};

use oxedyne_fe2o3_core::byte::FromBytes;
use oxedyne_fe2o3_iop_db::api::{
    Meta,
    ScanOpts,
};
use oxedyne_fe2o3_jdat::{
    Dat,
    id::NumIdDat,
};

use std::{
    collections::{
        BTreeMap,
        HashMap,
    },
    fs::{
        self,
        File,
    },
    io::{
        self,
        BufReader,
        Read,
        Seek,
        SeekFrom,
    },
    sync::Arc,
    thread,
    time::Duration,
};

const SCAN_COVERAGE_ATTEMPTS: usize = 3;

const SCAN_COVERAGE_SETTLE: Duration = Duration::from_millis(20);

type Live<const UIDL: usize, UID> =
    HashMap<Vec<u8>, (Dat, Meta<UIDL, UID>, Option<usize>, FileLocation, Tomb)>;

// Whether a record is a delete's tombstone, judged while the walk holds its data file open.
#[derive(Clone, Debug)]
enum Tomb {
    No,                         // Not of a tombstone's length, or not a tombstone.
    Yes,                        // A delete's tombstone.
    Unproven(Error<ErrTag>),    // Not readable from the generation its index named.
}

// The stored length of a tombstone's value, and of the checksum that ends every record.
#[derive(Clone, Copy, Debug)]
struct TombLen {
    vlen:       u64,
    csum_len:   usize,
}

#[derive(Clone, Copy, Debug, Default)]
struct ZoneFilePresence {
    dat: bool,
    ind: bool,
}

#[derive(Clone, Copy, Debug)]
struct Shortfall {
    fnum:       FileNum,
    dat_len:    u64,
    covered:    u64,
}

pub struct ScanBot<
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
    WorkerBot<UIDL, UID, ENC, KH, PR, CS> for ScanBot<UIDL, UID, ENC, KH, PR, CS>
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
    OzoneBot<UIDL, UID, ENC, KH, PR, CS> for ScanBot<UIDL, UID, ENC, KH, PR, CS>
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
    Bot<{ BID_LEN }, Bid, OzoneMsg<UIDL, UID, ENC, KH>> for ScanBot<UIDL, UID, ENC, KH, PR, CS>
{
    bot_methods!();

    fn go(&mut self) {

        sync_log::set_stream(self.log_stream_id());

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
                        OzoneMsg::ScanRequest {
                            opts,
                            schms2: _,
                            resp,
                        } => {
                            let result = self.scan_zone(&opts);
                            let msg = match result {
                                Ok(entries) => OzoneMsg::ScanEntries(entries),
                                Err(e) => OzoneMsg::Error(e),
                            };
                            self.respond(Ok(msg), &resp);
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
    ScanBot<UIDL, UID, ENC, KH, PR, CS>
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
            inited:     false,
        }
    }

    fn scan_zone(
        &mut self,
        opts: &ScanOpts,
    )
        -> Outcome<Vec<(Dat, Dat, Meta<UIDL, UID>)>>
    {
        // The chunk index of each record is carried alongside its key and meta so the
        // main-key / chunk-data classification is made on the newest record per key, not on
        // whichever happened to be walked last: a chunk-data record later superseded by a
        // Complete tombstone must classify as the tombstone, or the inverse scan would emit a
        // chunk key a delete has already begun reclaiming.
        let mut live: Live<UIDL, UID> = HashMap::new();
        let mut short: Vec<Shortfall> = Vec::new();
        let mut stale: Option<Error<ErrTag>> = None;

        // A pass that comes up short is repeated from scratch rather
        // than patched, because the deduplication depends on files
        // being visited in ascending order: re-walking one file after
        // the others would let an older record overwrite a newer one.
        // So is a pass whose tombstone check met a record other than
        // the one its index named, a collection having replaced the
        // file between the two opens.
        for attempt in 0..SCAN_COVERAGE_ATTEMPTS {
            live.clear();
            stale = None;
            short = res!(self.scan_pass(&mut live));
            if short.is_empty() {
                match Self::drop_deleted(&mut live) {
                    Ok(())  => break,
                    Err(e)  => stale = Some(e),
                }
            }
            if attempt + 1 < SCAN_COVERAGE_ATTEMPTS {
                thread::sleep(SCAN_COVERAGE_SETTLE);
            }
        }

        if !short.is_empty() {
            let mut detail = String::new();
            for s in &short {
                detail.push_str(&fmt!(
                    " file {} holds {} bytes of records and its index accounts for {};",
                    s.fnum, s.dat_len, s.covered));
            }
            return Err(err!(
                "{}: Scan of this zone is short: {} index file(s) do not account \
                for what their data files hold, and there is no way to say how \
                many records are missing, so nothing is reported rather than an \
                answer that would look complete.{} The records themselves are \
                intact and readable by key throughout; an index file that does \
                not account for its data file is rebuilt from that data file on \
                the next start.",
                self.ozid(), short.len(), detail;
                Data, Mismatch, Missing));
        }
        if let Some(e) = stale {
            return Err(err!(e,
                "{}: Scan of this zone could not tell its deleted keys from its \
                live ones in {} passes, each meeting a record other than the one \
                its index named, so nothing is reported rather than a list that \
                would name a deleted key.",
                self.ozid(), SCAN_COVERAGE_ATTEMPTS;
                Data, Mismatch));
        }

        let mut out: Vec<(Dat, Dat, Meta<UIDL, UID>)> =
            Vec::with_capacity(live.len());
        for (_kbyts, (kdat, meta, cind, _floc, _tomb)) in live.into_iter() {
            // A chunk-data record has a chunk index >= 1.  The default scan keeps the main user
            // keys (Complete, and the bunch key at index 0) and elides those; `chunk_data_only`
            // inverts it, keeping only the chunk-data keys.
            let is_chunk_data = matches!(cind, Some(c) if c >= 1);
            if opts.chunk_data_only != is_chunk_data {
                continue;
            }
            if !scan_matches_prefix(&kdat, opts.prefix.as_ref()) {
                continue;
            }
            out.push((kdat, Dat::Empty, meta));
            if let Some(lim) = opts.limit {
                if out.len() >= lim {
                    break;
                }
            }
        }
        if opts.include_values {
            warn!(sync_log::stream(),
                "{}: scan called with include_values=true; scan v1 \
                returns Dat::Empty for every value. Fetch individual \
                values via get() once a key is selected.",
                self.ozid());
        }
        Ok(out)
    }

    fn scan_pass(
        &mut self,
        live:   &mut Live<UIDL, UID>,
    )
        -> Outcome<Vec<Shortfall>>
    {
        let files = res!(self.list_zone_files());
        let tlen = res!(self.tomb_len());
        let mut short = Vec::new();
        for (fnum, present) in files {
            // Opened before its index, so that the offsets the index gives are into this
            // generation of the data file unless a whole collection, data rename then index
            // rename, falls between the two opens; the tombstone check catches that one.  It is
            // held only while its own index is walked, so a pass holds one data file open at a
            // time however many the zone has.  Holding every data file of the pass open until
            // its end made a zone with more files than the process may hold open come up short
            // (2026-11-01).
            let mut dat = match present.dat {
                true    => res!(self.open_if_present(fnum, &FileType::Data)),
                false   => None,
            };
            let before = self.data_file_len(fnum);
            // Every index file present is walked, including one whose data
            // file has gone: a collection that deletes a wholly superseded
            // file removes the data file first, and every key that file held
            // has a newer copy in a higher-numbered file which overwrites it
            // here anyway.
            let covered = if present.ind {
                res!(self.scan_walk_ind_file(fnum, dat.as_mut(), tlen, live))
            } else {
                0
            };
            drop(dat.take());
            let after = self.data_file_len(fnum);
            // A data file absent on either side has been collected away, or
            // was never there: there is nothing for the index to be short of.
            let dat_len = match (present.dat, before, after) {
                (true, Some(a), Some(b)) => std::cmp::min(a, b),
                _                        => continue,
            };
            if covered < dat_len {
                short.push(Shortfall { fnum, dat_len, covered });
            }
        }
        Ok(short)
    }

    /// Removes from `live` every key whose newest record is a tombstone.  A newest record whose
    /// tombstone check could not be made means a collection replaced its file during the pass.
    fn drop_deleted(live: &mut Live<UIDL, UID>) -> Outcome<()> {
        let mut gone: Vec<Vec<u8>> = Vec::new();
        for (kbyts, (_, _, _, _, tomb)) in live.iter() {
            match tomb {
                Tomb::No            => (),
                Tomb::Yes           => gone.push(kbyts.clone()),
                Tomb::Unproven(e)   => return Err(e.clone()),
            }
        }
        for k in gone {
            live.remove(&k);
        }
        Ok(())
    }

    /// The index gives the length of a record's value but not the value, and a tombstone, written
    /// unencrypted by every delete, always has one length, so only that length need be read.
    fn tomb_len(&self) -> Outcome<TombLen> {
        let tomb = Dat::Usr(id::usr_kind_id_deleted(), Some(Box::new(Dat::Empty)));
        let tomb_len = res!(tomb.as_bytes()).len();
        let csum_len = res!(self.api().schemes().checksummer().len());
        Ok(TombLen {
            vlen:       try_into!(u64, tomb_len + csum_len),
            csum_len,
        })
    }

    /// Judges whether the record at `floc` is a tombstone, reading it from `dat`, the
    /// generation of the data file opened before its index.
    fn tomb_of(
        &self,
        dat:    Option<&mut File>,
        kbyts:  &[u8],
        meta:   &Meta<UIDL, UID>,
        cind:   Option<usize>,
        floc:   &FileLocation,
        tlen:   TombLen,
    )
        -> Tomb
    {
        // A bunch key or chunk record is never a tombstone.
        if cind.is_some() || floc.vlen != tlen.vlen {
            return Tomb::No;
        }
        match self.read_tomb(dat, kbyts, meta, floc, tlen.csum_len) {
            Ok(true)    => Tomb::Yes,
            Ok(false)   => Tomb::No,
            Err(e)      => Tomb::Unproven(e),
        }
    }

    fn read_tomb(
        &self,
        dat:        Option<&mut File>,
        kbyts:      &[u8],
        meta:       &Meta<UIDL, UID>,
        floc:       &FileLocation,
        csum_len:   usize,
    )
        -> Outcome<bool>
    {
        let file = res!(dat.ok_or_else(|| err!(
            "{}: Data file {} has gone, so the record at {} its index names, stamped {:?}, \
            cannot be read.", self.ozid(), floc.fnum, floc.start, meta.time;
            Data, Missing)));
        let kv = floc.keyval();
        res!(file.seek(SeekFrom::Start(kv.start)));
        let mut rec = vec![0u8; try_into!(usize, kv.len)];
        res!(file.read_exact(&mut rec));
        let klen = try_into!(usize, floc.klen);
        if !res!(StoredKey::<UIDL, UID>::holds(&rec[..klen], kbyts, meta, csum_len)) {
            return Err(err!(
                "{}: The record at {} in data file {} is not the one its index named, \
                stamped {:?}.", self.ozid(), floc.start, floc.fnum, meta.time;
                Data, Mismatch));
        }
        res!(self.api().schemes().checksummer().clone().verify(&rec[klen..]));
        let (val, _) = res!(Dat::from_bytes(&rec[klen..rec.len() - csum_len]));
        Ok(matches!(&val, Dat::Usr(kind, _) if *kind == id::usr_kind_id_deleted()))
    }

    /// Opens a zone file for reading, or gives `None` when it has gone, which is what a
    /// collection that removed it looks like.  Any other failure, the process's limit on open
    /// files among them, is an error: taken for a collection, it made the scan of a whole zone
    /// come up short and blame its index files (2026-11-01).
    fn open_if_present(&self, fnum: FileNum, typ: &FileType) -> Outcome<Option<File>> {
        let path = self.zdir().ozone_file_path(fnum, typ);
        match File::open(&path) {
            Ok(f)                                           => Ok(Some(f)),
            Err(e) if e.kind() == io::ErrorKind::NotFound   => Ok(None),
            Err(e)                                          => Err(err!(e,
                "{}: While opening {:?} file {} at {:?} for a scan.",
                self.ozid(), typ, fnum, path;
                IO, File, Read)),
        }
    }

    fn data_file_len(&self, fnum: FileNum) -> Option<u64> {
        let mut path = self.zdir().dir.clone();
        path.push(ZoneDir::relative_file_path(&FileType::Data, fnum));
        match fs::metadata(&path) {
            Ok(m)  => Some(m.len()),
            Err(_) => None,
        }
    }

    fn list_zone_files(&self) -> Outcome<BTreeMap<FileNum, ZoneFilePresence>> {
        let mut files: BTreeMap<FileNum, ZoneFilePresence> = BTreeMap::new();
        for entry in res!(fs::read_dir(&self.zdir().dir)) {
            let entry = res!(entry);
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            if ZoneDir::is_gc_temp_file(&path) {
                continue;
            }
            let (fnum, ftyp) = match ZoneDir::ozone_file_number_and_type(&path) {
                Ok(t) => t,
                Err(_) => continue,
            };
            let present = files.entry(fnum).or_insert_with(ZoneFilePresence::default);
            match ftyp {
                FileType::Data  => present.dat = true,
                FileType::Index => present.ind = true,
            }
        }
        Ok(files)
    }

    fn scan_walk_ind_file(
        &mut self,
        fnum:       FileNum,
        mut dat:    Option<&mut File>,
        tlen:       TombLen,
        live:       &mut Live<UIDL, UID>,
    )
        -> Outcome<u64>
    {
        let file = match res!(self.open_if_present(fnum, &FileType::Index)) {
            Some(file) => file,
            None => {
                trace!(sync_log::stream(),
                    "{}: Index file {} went away between listing and opening, \
                    which is what collecting an entirely superseded file looks \
                    like; skipping it.", self.ozid(), fnum);
                return Ok(0);
            },
        };
        let mut reader = BufReader::new(file);
        let typ = FileType::Index;
        let mut pos = 0usize;
        // Bytes of key-value data this index accounts for.
        let mut covered = 0u64;

        loop {
            // 1. Load the StoredKey from the file.
            let (key, meta) = match StoredKey::load(
                &mut reader,
                self.api().schemes().checksummer().clone(),
            ) {
                Err(e) => return Err(err!(e,
                    "{}: While scanning {:?} file {} at position {}.",
                    self.ozid(), typ, fnum, pos;
                    IO, File, Read)),
                Ok(None) => break,
                Ok(Some((skey, _, n))) => {
                    pos += n;
                    let meta = skey.meta().clone();
                    (skey.into_key(), meta)
                },
            };
            // 2. Read the matching StoredIndex, for the record's location, which breaks a
            //    tie of stamps.  We do not read values in v1.
            let floc = match StoredIndex::read(
                &mut reader,
                fnum,
                self.api().schemes().checksummer().clone(),
            ) {
                Err(e) => return Err(err!(e,
                    "{}: While scanning stored index in {:?} file {} \
                    at position {}.",
                    self.ozid(), typ, fnum, pos;
                    IO, File, Read)),
                Ok((None, _)) => return Err(err!(
                    "{}: Missing StoredIndex at end of {:?} file {}.",
                    self.ozid(), typ, fnum;
                    Missing)),
                Ok((Some(sindex), n)) => {
                    pos += n;
                    // Counted before the chunk entries are elided below:
                    // the data file holds those records too, so leaving
                    // them out here would make every chunked value look
                    // like an under-count.
                    covered += sindex.keyval_len();
                    sindex.ref_file_location().clone()
                },
            };

            // 3. Record the chunk index so the caller can classify on the newest record.
            //    `Complete` keys have no index, a bunch key is index 0, and a chunk-data
            //    record is index >= 1.  The main-key / chunk-data split is applied in
            //    `scan_zone` after the newest-wins dedup below, not here, so a chunk-data
            //    record superseded by a later Complete tombstone classifies as the tombstone.
            let cind = key.index();

            // 4. Decode the raw key bytes to a Dat.
            let kbyts = key.into_bytes();
            let (kdat, _n_decoded) = match Dat::from_bytes(&kbyts) {
                Ok(pair) => pair,
                Err(e) => {
                    warn!(sync_log::stream(),
                        "{}: Could not decode scanned key bytes in file {} \
                        at position {}: {}. Skipping entry.",
                        self.ozid(), fnum, pos, e);
                    continue;
                },
            };

            // 5. Insert into the live map, keeping of two records of a key the one the cache
            //    keeps, by `supersedes`.  Walk order is not stamp order: a zone's writers append
            //    to files of their own, so a tombstone can sit in a lower-numbered file than the
            //    record it retires, and walk order alone revived that record (2026-10-09).
            //    Only a record that is kept is judged for a tombstone, from this generation of
            //    its data file.
            match live.get(&kbyts) {
                Some((_, held, _, hloc, _)) if !supersedes(&meta.time, &floc, &held.time, hloc) => (),
                _ => {
                    let tomb = self.tomb_of(dat.as_deref_mut(), &kbyts, &meta, cind, &floc, tlen);
                    live.insert(kbyts, (kdat, meta, cind, floc, tomb));
                },
            }
        }
        Ok(covered)
    }
}

fn scan_matches_prefix(kdat: &Dat, prefix: Option<&Dat>) -> bool {
    match prefix {
        None => true,
        Some(Dat::Str(p)) => match kdat {
            Dat::Str(s) => s.starts_with(p.as_str()),
            _ => false,
        },
        Some(other) => kdat == other,
    }
}
