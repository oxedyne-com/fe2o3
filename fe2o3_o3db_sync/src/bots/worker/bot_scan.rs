use crate::{
    prelude::*,
    bots::{
        base::bot_deps::*,
        worker::worker_deps::*,
    },
    file::{
        core::FileAccess,
        floc::FileNum,
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
    collections::HashMap,
    fs,
    io::BufReader,
    sync::Arc,
};

/// Answers scans for one zone by walking its index files.
///
/// A scan is the only user request whose cost is the whole zone rather
/// than a single record, and it is the reason this bot exists. The walk
/// used to run on the init-garbage bot, sharing a queue with garbage
/// collection: a scan arriving while a collection was under way waited
/// for it, and on a store with a real collection backlog it waited past
/// `USER_REQUEST_TIMEOUT` and the caller was told only that a channel
/// had timed out. Reads and writes kept working throughout, which is
/// the signature of starvation rather than damage.
///
/// Putting the walk on its own queue means a scan's latency depends on
/// the size of the zone and on nothing else. The reader bots were the
/// other obvious home, and were rejected: a scan on that queue would
/// have made every `get` behind it wait for a walk of the whole zone,
/// trading a broken view for slow reads.
///
/// This bot only reads. It opens index files, decodes keys and metadata
/// and returns them; it never writes, never touches file state, and
/// holds no lock any other bot waits on. A scan can therefore run at
/// the same time as a collection without either having to know about
/// the other, which is safe because a collection replaces an index file
/// in one step rather than rewriting it in place.
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
    /// Creates a scan bot from the standard zone worker arguments.
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

    /// Walk every index file in this bot's zone and return the live
    /// user-visible `(key, value, meta)` entries that satisfy `opts`.
    ///
    /// Deduplication keeps the newest entry per raw key-bytes: index
    /// files are walked in ascending file-number order, and within a
    /// file in position order, so a later write of the same key
    /// naturally overwrites the earlier one in the local map.
    ///
    /// Internal chunk entries (`cind >= 1`) are elided; only the
    /// user-visible main keys are returned.
    ///
    /// Values are deferred to a later revision of scan: this handler
    /// returns `Dat::Empty` as the value for every entry. Callers
    /// that need the value fetch it with a separate `get()` call
    /// once the operator has chosen a specific key.
    ///
    /// Prefix and limit filters are applied per zone before the
    /// result ships to the coordinator, so the wire message stays
    /// bounded even when the caller sets a small limit against a
    /// large database. The filters do not make the walk any cheaper:
    /// every index record in the zone is read and checksummed
    /// whatever the caller asked for.
    fn scan_zone(
        &mut self,
        opts: &ScanOpts,
    )
        -> Outcome<Vec<(Dat, Dat, Meta<UIDL, UID>)>>
    {
        let fnums = res!(self.list_ind_fnums());
        let mut live: HashMap<Vec<u8>, (Dat, Meta<UIDL, UID>)>
            = HashMap::new();

        for fnum in fnums {
            res!(self.scan_walk_ind_file(fnum, &mut live));
        }

        let mut out: Vec<(Dat, Dat, Meta<UIDL, UID>)> =
            Vec::with_capacity(live.len());
        for (_kbyts, (kdat, meta)) in live.into_iter() {
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

    /// Enumerate the `.ind` file numbers in this bot's zone
    /// directory, ascending. Unparseable or non-ind files are
    /// skipped silently; the zone survey at startup already
    /// rejects structurally invalid directories, and a garbage
    /// collection temporary carries a name that does not parse, so
    /// a rebuild in progress is passed over rather than read.
    fn list_ind_fnums(&self) -> Outcome<Vec<FileNum>> {
        let mut fnums: Vec<FileNum> = Vec::new();
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
            if ftyp == FileType::Index {
                fnums.push(fnum);
            }
        }
        fnums.sort();
        Ok(fnums)
    }

    /// Walk a single index file, populating `live` with the
    /// user-visible entries it contains. Skips internal chunk
    /// entries. A later call with a higher `fnum` for the same key
    /// bytes overwrites the entry inserted here, which is the
    /// correct stale-filtering behaviour for an append-only store.
    ///
    /// A file that disappears between the directory listing and the
    /// open is not an error: garbage collection deletes a file whose
    /// records have all been superseded, and every key that file held
    /// has a newer copy elsewhere in the zone.
    fn scan_walk_ind_file(
        &mut self,
        fnum: FileNum,
        live: &mut HashMap<Vec<u8>, (Dat, Meta<UIDL, UID>)>,
    )
        -> Outcome<()>
    {
        let file = match self.zdir().open_ozone_file(
            fnum,
            &FileType::Index,
            &FileAccess::Reading,
        ) {
            Ok((_, file)) => file,
            Err(_) => {
                trace!(sync_log::stream(),
                    "{}: Index file {} went away between listing and opening, \
                    which is what collecting an entirely superseded file looks \
                    like; skipping it.", self.ozid(), fnum);
                return Ok(());
            },
        };
        let mut reader = BufReader::new(file);
        let typ = FileType::Index;
        let mut pos = 0usize;

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
            // 2. Skip the matching StoredIndex. We do not need the
            //    location -- we are not reading values in v1.
            match StoredIndex::read(
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
                Ok((Some(_sindex), n)) => {
                    pos += n;
                },
            }

            // 3. Elide internal chunk entries; only main user keys
            //    appear in the scan result. Main keys are either
            //    `Complete` (non-chunked values) or `Chunk(_, 0)`
            //    (the bunch-key pointer for a chunked value).
            let cind = key.index();
            if let Some(c) = cind {
                if c >= 1 {
                    continue;
                }
            }

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

            // 5. Insert into the live map. Later occurrences of the
            //    same raw key bytes (from higher fnum or later in
            //    the same file) overwrite, which is exactly the
            //    stale-filtering behaviour we want.
            live.insert(kbyts, (kdat, meta));
        }
        Ok(())
    }
}

/// Return `true` if `kdat` satisfies the optional `prefix` filter.
/// When the prefix is a `Dat::Str`, the comparison is a string
/// prefix match against `kdat` if it is also a `Dat::Str`. For
/// every other prefix variant the comparison is strict equality.
/// A `None` prefix matches everything.
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
