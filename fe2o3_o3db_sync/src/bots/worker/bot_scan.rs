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
    collections::{
        BTreeMap,
        HashMap,
    },
    fs,
    io::BufReader,
    sync::Arc,
    thread,
    time::Duration,
};

/// Number of passes a scan makes before it declares an index file short of its data file.
///
/// Measuring the data file on both sides of the walk (see `ScanBot::scan_pass`) accounts for
/// a record appended during the walk and for a garbage collection landing during it.  What it
/// does not cover is a walk that begins after a collection has renamed its rebuilt index into
/// place and ends before that same collection renames the transcribed data file over the old
/// one -- one statement apart, so the walk would have to be shorter than the gap between two
/// renames.  That is vanishingly unlikely and it is not impossible, and the difference between
/// vanishingly unlikely and impossible is what a retry is for.
///
/// Repeating the pass separates a momentary shortfall from a permanent one without taking a
/// lock that a collection would then have to wait on, and it costs nothing on the ordinary
/// path, because a scan that adds up never repeats.
const SCAN_COVERAGE_ATTEMPTS: usize = 3;

/// Pause between those passes.  Long enough that an in-flight record has certainly landed,
/// short enough that the caller waiting on a scan does not notice it.
const SCAN_COVERAGE_SETTLE: Duration = Duration::from_millis(20);

/// Which of a file number's two files were in the zone directory when it was listed.
#[derive(Clone, Copy, Debug, Default)]
struct ZoneFilePresence {
    /// Whether a data file was there.
    dat: bool,
    /// Whether an index file was there.
    ind: bool,
}

/// An index file that accounts for less than its data file holds.
///
/// The scan of that file is short by the difference, so the answer the caller would have been
/// given is missing records without being wrong in any way the caller could detect.
#[derive(Clone, Copy, Debug)]
struct Shortfall {
    /// The file number whose index came up short.
    fnum:       FileNum,
    /// Bytes of key-value data in the data file.
    dat_len:    u64,
    /// Bytes of key-value data the index file accounts for.
    covered:    u64,
}

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
    ///
    /// **A walk that cannot see everything fails rather than
    /// answering short.** Every index record names the length of the
    /// key and value it points at, so the records of one index file
    /// add up to exactly the length of its data file. When they add
    /// up to less, the index does not account for everything the
    /// data file holds and the answer would be missing records --
    /// with nothing in it to say so. That is not a hypothetical: an
    /// index file left at zero bytes beside a data file full of
    /// records makes this walk return an empty list, successfully,
    /// while `get()` on those same records goes on working. The
    /// gateway then read a limit through a scan, was told there was
    /// no override set, answered the operator `200` for the change,
    /// and enforced the old value for the life of the process.
    ///
    /// The caller is told, rather than the log, because a caller has
    /// no way to distinguish "nothing there" from "I could not look"
    /// and the log is not on the path of anyone waiting for the
    /// answer. A scan that cannot report an under-count is the same
    /// failure class as a check that cannot fail.
    fn scan_zone(
        &mut self,
        opts: &ScanOpts,
    )
        -> Outcome<Vec<(Dat, Dat, Meta<UIDL, UID>)>>
    {
        let mut live: HashMap<Vec<u8>, (Dat, Meta<UIDL, UID>)>
            = HashMap::new();
        let mut short: Vec<Shortfall> = Vec::new();

        // A pass that comes up short is repeated from scratch rather
        // than patched, because the deduplication depends on files
        // being visited in ascending order: re-walking one file after
        // the others would let an older record overwrite a newer one.
        for attempt in 0..SCAN_COVERAGE_ATTEMPTS {
            live.clear();
            short = res!(self.scan_pass(&mut live));
            if short.is_empty() {
                break;
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

    /// One walk of every index file in the zone, in ascending file-number
    /// order, reporting any file whose index does not account for its data
    /// file.
    ///
    /// Each data file is measured on **both** sides of the walk of its index,
    /// and the smaller of the two lengths is what the index has to account
    /// for. Measuring once is not enough, and neither single choice is safe:
    ///
    /// - Measure before, and a garbage collection that lands during the walk
    ///   makes an intact index look short. The collection replaces the index
    ///   with one describing the smaller transcribed data file, and the
    ///   earlier, larger measurement is then compared against it. Under a
    ///   backlog a collection lands during almost every walk, so this is not
    ///   a corner: it would be the ordinary case.
    /// - Measure after, and a record appended during the walk makes an intact
    ///   index look short, because the data file has grown past the point the
    ///   walk read up to.
    ///
    /// Both movements are in a known direction -- a collection only shrinks a
    /// data file, an append only grows one -- so the smaller of the two
    /// measurements is below the index's coverage in either case, and a
    /// genuine shortfall is below both.
    fn scan_pass(
        &mut self,
        live: &mut HashMap<Vec<u8>, (Dat, Meta<UIDL, UID>)>,
    )
        -> Outcome<Vec<Shortfall>>
    {
        let files = res!(self.list_zone_files());
        let mut short = Vec::new();
        for (fnum, present) in files {
            let before = self.data_file_len(fnum);
            // Every index file present is walked, including one whose data
            // file has gone: a collection that deletes a wholly superseded
            // file removes the data file first, and every key that file held
            // has a newer copy in a higher-numbered file which overwrites it
            // here anyway.
            let covered = if present.ind {
                res!(self.scan_walk_ind_file(fnum, live))
            } else {
                0
            };
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

    /// The length in bytes of one data file, or `None` if it is not there.
    ///
    /// A file that has gone is not an error: a collection deletes a data file
    /// whose records have all been superseded.
    fn data_file_len(&self, fnum: FileNum) -> Option<u64> {
        let mut path = self.zdir().dir.clone();
        path.push(ZoneDir::relative_file_path(&FileType::Data, fnum));
        match fs::metadata(&path) {
            Ok(m)  => Some(m.len()),
            Err(_) => None,
        }
    }

    /// Enumerate this bot's zone directory, ascending by file number, noting
    /// which of the two files each number has.
    ///
    /// Unparseable or foreign files are skipped silently; the zone survey
    /// at startup already rejects structurally invalid directories, and a
    /// garbage collection temporary carries a name that does not parse, so
    /// a rebuild in progress is passed over rather than read.
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
    ///
    /// Returns the number of bytes of key-value data the index file
    /// accounts for, which for an intact index is exactly the length
    /// of its data file. The caller compares the two; that comparison
    /// is the only thing standing between a short walk and an answer
    /// that looks complete.
    fn scan_walk_ind_file(
        &mut self,
        fnum: FileNum,
        live: &mut HashMap<Vec<u8>, (Dat, Meta<UIDL, UID>)>,
    )
        -> Outcome<u64>
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
                Ok((Some(sindex), n)) => {
                    pos += n;
                    // Counted before the chunk entries are elided below:
                    // the data file holds those records too, so leaving
                    // them out here would make every chunked value look
                    // like an under-count.
                    covered += sindex.keyval_len();
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
        Ok(covered)
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
