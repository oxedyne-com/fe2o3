//! An append-only NDJSON writer with the gates of Daimond's `/api/debug-trace`.
//!
//! The sink takes what `gateway/src/handlers/debug_trace.rs` took, a body `{v, device, rows:
//! [{ts, tag, data}]}` under a key the caller chose, and holds it to the same caps: a body over
//! `max_body` is refused, more than `max_rows` rows is refused and not truncated, a tag and a
//! data field are clipped, and one key may post at most once in `min_interval_ms`. It differs
//! from that handler in four ways, each on purpose.
//!
//! * It writes one JSON object a line instead of a text block, which a reader parses with
//!   nothing but `JSON.parse` (see `Entry::line`).
//! * It is keyed by what the caller passes and never by the body. Daimond names a file for the
//!   session's account and the body's device; here a key is any list of segments, each reduced
//!   to a safe alphabet, and the body's `device` is only a label on each line.
//! * It redacts. Every row passes `Redact` before it is written, so a caller that forgot to
//!   covers nothing: a secret-named field, a deny-listed place and a string the caller's test
//!   fires on never reach the disk. An `ev` row whose data is not JSON cannot be examined by
//!   name, and is replaced by its fingerprint.
//! * Its history is bounded by total bytes as well as by age and count, and a file that has gone
//!   quiet can age out, because a session-keyed directory grows by a file for every key.
//!
//! Why a rotation keeps its history: rotation once overwrote a single `<name>.log.1`, which held
//! a device to two files and made the older a sliding window minutes wide. On 2026-09-11 a
//! device in a live fault filled 2 MiB faster than anyone pulled, and the block holding the
//! fault had rotated away before the first pull existed. A rotation now closes the file under
//! the UTC instant it was closed at, and the history is bounded by age and count. Age is read
//! from the stamp in a file's name and never from its mtime, because a reader mirrors the
//! directory with `rsync`, which is free to rewrite timestamps.

use super::{
    chunk::{
        self,
        Verdict,
    },
    redact::{
        fingerprint,
        NoTest,
        Redact,
        StrTest,
    },
    row::{
        Entry,
        Row,
    },
};
use crate::guard::floor::KeyFloor;

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_jdat::{
    bdat::limits::DecodeLimits,
    prelude::*,
};

use std::{
    fs,
    io::Write,
    path::{
        Path,
        PathBuf,
    },
    sync::Mutex,
    time::{
        SystemTime,
        UNIX_EPOCH,
    },
};


const EXT:          &str    = "ndjson";
pub(crate) const STAMP_LEN: usize = 16;               // `YYYYMMDDTHHMMSSZ`
const DAY_MS:       u64     = 86_400_000;
const MALFORMED:    &str    = "(malformed row)";

/// The caps and the retention of a `Sink`.
#[derive(Clone, Debug)]
pub struct Gates {
    pub max_body:           usize,  // the largest body accepted, in bytes
    pub max_rows:           usize,  // the most rows a post may carry
    pub max_tag:            usize,  // the longest a tag is kept, in bytes
    pub max_data:           usize,  // the longest a data field is kept, in bytes
    pub min_interval_ms:    u64,    // the shortest gap between two accepted posts from one key
    pub max_file:           u64,    // the size past which a live file is rotated
    pub retain_days:        u64,    // how long a rotated file is kept
    pub max_rotated:        usize,  // the most rotated files kept for one key
    pub max_total:          u64,    // the most bytes the directory may hold
    pub sweep_idle:         bool,   // may a live file that has gone quiet age out
}

impl Gates {

    /// Daimond's own numbers, as `debug_trace.rs` holds them: a device is bounded, the directory
    /// is not, and nothing that has gone quiet is ever removed.
    pub fn daimond() -> Self {
        Self {
            max_body:           256 * 1024,
            max_rows:           1_000,
            max_tag:            64,
            max_data:           400,
            min_interval_ms:    2_000,
            max_file:           2 * 1024 * 1024,
            retain_days:        14,
            max_rotated:        200,
            max_total:          u64::MAX,
            sweep_idle:         false,
        }
    }
}

impl Default for Gates {
    /// Daimond's caps, and the retention the test site plans: 2 MiB files, 7 days or 20 files for
    /// a key, 200 MiB in all, and a quiet file ages out with the rest.
    fn default() -> Self {
        Self {
            retain_days:    7,
            max_rotated:    20,
            max_total:      200 * 1024 * 1024,
            sweep_idle:     true,
            ..Self::daimond()
        }
    }
}

/// Why a post was turned away. Nothing was written.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Refusal {
    TooLarge,               // 413
    Malformed(String),      // 400, the body is no JSON
    NoRows,                 // 400, there is no `rows` list
    TooManyRows,            // 400
    TooFast,                // 429
}

impl Refusal {

    /// The HTTP status Daimond answers with.
    pub fn status(&self) -> u16 {
        match self {
            Self::TooLarge      => 413,
            Self::Malformed(_)  => 400,
            Self::NoRows        => 400,
            Self::TooManyRows   => 400,
            Self::TooFast       => 429,
        }
    }

    /// The sentence Daimond answers with.
    pub fn message(&self) -> String {
        match self {
            Self::TooLarge      => "Diagnostic trace too large.".to_string(),
            Self::Malformed(m)  => m.clone(),
            Self::NoRows        => "A trace must carry a `rows` list.".to_string(),
            Self::TooManyRows   => "Too many rows in one trace.".to_string(),
            Self::TooFast       => "Wait a moment before sharing again.".to_string(),
        }
    }
}

/// The result of a post: the rows stored, or the reason none was.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Posted {
    Stored(usize),
    Refused(Refusal),
}

/// The instant of a rotation as `YYYYMMDDTHHMMSSZ` in UTC. Fixed width, so two stamps sort
/// chronologically as plain strings and the pruner needs no reverse calendar conversion.
pub fn stamp(ms: u64) -> String {
    let secs = (ms / 1_000) as i64;
    let (y, m, d) = civil_from_days(secs.div_euclid(86_400));
    let sod = secs.rem_euclid(86_400);                          // Second of day.
    fmt!("{:04}{:02}{:02}T{:02}{:02}{:02}Z",
        y, m, d, sod / 3_600, (sod % 3_600) / 60, sod % 60)
}

/// Is `s` a rotation stamp?
pub fn is_stamp(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == STAMP_LEN
        && b[8]  == b'T'
        && b[15] == b'Z'
        && b[..8].iter().all(u8::is_ascii_digit)
        && b[9..15].iter().all(u8::is_ascii_digit)
}

// Days since 1970-01-01 to a civil date, by Howard Hinnant's algorithm.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z   = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp  = (5 * doy + 2) / 153;
    let d   = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m   = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    (yoe + era * 400 + if m <= 2 { 1 } else { 0 }, m, d)
}

/// Reduces a segment to a safe filename alphabet, so nothing off the wire can walk out of the
/// directory or collide with a control character. Kept to 48 characters, since an id is opaque
/// and a prefix distinguishes it.
pub fn sanitise(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
            out.push(c);
        } else {
            out.push('_');
        }
        if out.len() >= 48 {
            break;
        }
    }
    if out.is_empty() {
        out.push_str("unknown");
    }
    out
}

/// The wall clock in milliseconds since the epoch.
pub fn now_ms() -> Outcome<u64> {
    let d = res!(SystemTime::now().duration_since(UNIX_EPOCH));
    Ok(d.as_millis() as u64)
}

// Strips control characters and line breaks from a value bound for one line, so a row cannot
// forge another by carrying its own.
fn scrub(s: &str) -> String {
    s.chars().map(|c| if c.is_control() { ' ' } else { c }).collect()
}

// Caps a string to `n` bytes on a character boundary.
fn clip(s: &str, n: usize) -> String {
    if s.len() <= n {
        return s.to_string();
    }
    let mut end = n;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    s[..end].to_string()
}

fn mtime_ms(meta: &fs::Metadata) -> u64 {
    meta.modified().ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_millis() as u64)
}

#[cfg(unix)]
fn make_dir(dir: &Path) -> Outcome<()> {
    use std::os::unix::fs::{
        DirBuilderExt,
        PermissionsExt,
    };
    res!(fs::DirBuilder::new().recursive(true).mode(0o700).create(dir));
    // A directory that was there already may be open to others. What it holds is debug data, so
    // it is closed rather than trusted.
    let meta = res!(fs::metadata(dir));
    if meta.permissions().mode() & 0o077 != 0 {
        res!(fs::set_permissions(dir, fs::Permissions::from_mode(0o700)));
    }
    Ok(())
}

#[cfg(not(unix))]
fn make_dir(dir: &Path) -> Outcome<()> {
    res!(fs::create_dir_all(dir));
    Ok(())
}

#[cfg(unix)]
fn open_append(path: &Path) -> Outcome<fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    Ok(res!(fs::OpenOptions::new().create(true).append(true).mode(0o600).open(path)))
}

#[cfg(not(unix))]
fn open_append(path: &Path) -> Outcome<fs::File> {
    Ok(res!(fs::OpenOptions::new().create(true).append(true).open(path)))
}

/// One directory of NDJSON files, a file for each key.
///
/// The type parameter is the caller's string test for its `Redact`, which a sink always has: a
/// plain `Sink::new` redacts by name alone. The directory is made `0700` and the files `0600`.
/// Nothing here opens a socket.
pub struct Sink<T: StrTest = NoTest> {
    dir:        PathBuf,
    gates:      Gates,
    redact:     Redact<T>,
    floor:      KeyFloor,
    lock:       Mutex<()>,      // one writer at a time, so a rotation cannot meet an append
}

impl Sink<NoTest> {

    /// A sink under `dir`, which is made when the first row is written.
    pub fn new<P: Into<PathBuf>>(dir: P, gates: Gates) -> Self {
        let floor = KeyFloor::new(gates.min_interval_ms);
        Self {
            dir:    dir.into(),
            gates,
            redact: Redact::new(),
            floor,
            lock:   Mutex::new(()),
        }
    }
}

impl<T: StrTest> Sink<T> {

    pub fn with_redact<U: StrTest>(self, redact: Redact<U>) -> Sink<U> {
        Sink {
            dir:    self.dir,
            gates:  self.gates,
            redact,
            floor:  self.floor,
            lock:   self.lock,
        }
    }

    pub fn dir(&self) -> &Path { &self.dir }

    pub fn gates(&self) -> &Gates { &self.gates }

    /// The live file for a key. Each segment is reduced to a safe alphabet and the segments are
    /// joined by a hyphen, so Daimond's `["<account>", "<device>"]` names the file it does.
    pub fn path(&self, key: &[&str]) -> PathBuf {
        let stem = if key.is_empty() {
            sanitise("")
        } else {
            key.iter().map(|k| sanitise(k)).collect::<Vec<String>>().join("-")
        };
        self.dir.join(fmt!("{}.{}", stem, EXT))
    }

    /// Takes a client's batch, `{v, device, rows:[{ts, tag, data}]}`, as Daimond's handler does.
    ///
    /// The key is the caller's, from a session it trusts and never from the body. The order of
    /// the checks is Daimond's: size, shape, row count, then the rate floor, so a refused post
    /// does not use up the key's interval. A refusal is a value and nothing was written.
    pub fn post(&self, key: &[&str], body: &[u8], now_ms: u64) -> Outcome<Posted> {
        if body.len() > self.gates.max_body {
            return Ok(Posted::Refused(Refusal::TooLarge));
        }
        let text = match std::str::from_utf8(body) {
            Ok(t)   => t,
            Err(_)  => return Ok(Posted::Refused(Refusal::Malformed(
                "Request body is not valid UTF-8.".to_string()))),
        };
        if text.trim().is_empty() {
            return Ok(Posted::Refused(Refusal::Malformed(
                "Request body is empty; expected JSON.".to_string())));
        }
        let req = match Dat::decode_json_strict(text, &DecodeLimits::default()) {
            Ok(d)   => d,
            Err(_)  => return Ok(Posted::Refused(Refusal::Malformed(
                "Request body is not valid JSON.".to_string()))),
        };
        let rows = match req.map_get_list(&Dat::Str("rows".to_string())) {
            Ok(v)   => v,
            Err(_)  => return Ok(Posted::Refused(Refusal::NoRows)),
        };
        if rows.len() > self.gates.max_rows {
            return Ok(Posted::Refused(Refusal::TooManyRows));
        }
        // The body's device is a label. It names no file and keys no rate.
        let device = get(&req, "device").and_then(|d| d.get_string()).unwrap_or_default();
        let device = self.label(&device);

        if !res!(self.floor.admit(&key.join("/"), now_ms)) {
            return Ok(Posted::Refused(Refusal::TooFast));
        }
        let mut posted = Vec::with_capacity(rows.len());
        for r in rows {
            let ts   = get(r, "ts").and_then(|d| d.get_i64()).unwrap_or(0);
            let tag  = get(r, "tag").and_then(|d| d.get_string()).unwrap_or_default();
            let data = get(r, "data").and_then(|d| d.get_string()).unwrap_or_default();
            posted.push((ts, tag, data));
        }
        // The chunk rows of a bundle are judged by the text they encode, together (see `chunk`).
        let seen: Vec<(String, String)> = posted.iter()
            .map(|(_, tag, data)| (
                clip(&scrub(tag), self.gates.max_tag),
                clip(&scrub(data), self.gates.max_data),
            ))
            .collect();
        let verdicts = chunk::judge(&self.redact, self.gates.max_data, &seen);
        let mut text = String::new();
        for (((ts, tag, data), (stag, sdata)), verdict) in posted.iter().zip(seen).zip(verdicts) {
            let entry = match verdict {
                Verdict::Row        => self.entry(*ts, tag, data),
                Verdict::Clean      => Entry { ts: *ts, tag: stag, data: sdata },
                Verdict::Hit        => Entry { ts: *ts, tag: stag, data: self.redact.mark(&sdata) },
                Verdict::Cover(d)   => Entry { ts: *ts, tag: stag, data: d },
            };
            text.push_str(&entry.line(&device, now_ms));
        }
        res!(self.append(key, &text, now_ms));
        Ok(Posted::Stored(rows.len()))
    }

    /// Writes rows built in process, such as a peer's own frames and state, under the same
    /// redaction, the same field caps and the same rotation as a post. There is no body to cap
    /// and no rate floor to keep, because the caller is this process.
    ///
    /// A row's JSON is fitted to `max_data` by dropping or trimming its largest string, which
    /// keeps it valid JSON, as `Row::fit` explains.
    pub fn write(&self, key: &[&str], device: &str, rows: &[Row], now_ms: u64) -> Outcome<usize> {
        let device = self.label(device);
        let mut text = String::new();
        for row in rows {
            let covered = self.redact.row(row);
            let mut e = res!(covered.entry(self.gates.max_data));
            e.tag = clip(&scrub(&e.tag), self.gates.max_tag);
            text.push_str(&e.line(&device, now_ms));
        }
        res!(self.append(key, &text, now_ms));
        Ok(rows.len())
    }

    // A device label as it is written beside each line: scrubbed, clipped, and covered like any
    // other string a client sends, since a label is as free to carry a secret as a row is.
    fn label(&self, device: &str) -> String {
        let device = clip(&scrub(device), self.gates.max_tag);
        match self.redact.text(&device) {
            Some(covered)   => covered,
            None            => device,
        }
    }

    // One posted row, as it will be written: scrubbed, covered, then clipped.
    fn entry(&self, ts: i64, tag: &str, data: &str) -> Entry {
        // A row that is not the expected shape is written as a marker rather than dropped: a
        // reader seeing it learns the client sent something unexpected, which is diagnostic.
        if tag.is_empty() {
            return Entry { ts, tag: MALFORMED.to_string(), data: String::new() };
        }
        let tag  = clip(&scrub(tag), self.gates.max_tag);
        let data = self.cover(&tag, &scrub(data));
        Entry { ts, tag, data: clip(&data, self.gates.max_data) }
    }

    // Covers a data field before it is clipped, so a clip cannot leave a secret-named field in a
    // JSON text too short to parse.
    fn cover(&self, tag: &str, data: &str) -> String {
        if tag.starts_with("ev ") {
            return match Dat::decode_json_strict(data, &DecodeLimits::default()) {
                Ok(d) => {
                    let (nd, changed) = self.redact.walk(&d);
                    if !changed {
                        // Untouched, so the client's own bytes stand.
                        data.to_string()
                    } else {
                        match super::row::compact(&nd) {
                            Ok(s)   => s,
                            Err(_)  => fingerprint(data, 0),
                        }
                    }
                },
                // An event that is no JSON cannot be examined by name, so it is not kept.
                Err(_) => fingerprint(data, 0),
            };
        }
        self.redact.text(data).unwrap_or_else(|| data.to_string())
    }

    // Appends, creating the directory and, where the live file has grown past `max_file`,
    // closing it under a stamped name first so that the append opens a fresh one.
    fn append(&self, key: &[&str], text: &str, now_ms: u64) -> Outcome<()> {
        let _one = lock_mutex!(self.lock);
        res!(make_dir(&self.dir));
        let path = self.path(key);
        let mut sweep = false;
        match fs::metadata(&path) {
            Ok(meta) => {
                if meta.len() > self.gates.max_file {
                    // A failed rotation must not lose the new rows: fall through to the append,
                    // which grows the file past the cap for one more round rather than dropping
                    // the diagnostic.
                    match self.rotate(&path, now_ms) {
                        Ok(_)   => sweep = true,
                        Err(e)  => warn!("lens: could not rotate {:?}: {}", path, e),
                    }
                }
            },
            // A new file is a new key, and the directory grows by it.
            Err(_) => sweep = true,
        }
        let mut f = res!(open_append(&path));
        res!(f.write_all(text.as_bytes()));
        // Sweeping runs only behind a rotation or a new file, so a quiet directory is never walked.
        if sweep {
            if let Err(e) = self.sweep(&path, now_ms) {
                warn!("lens: could not prune {:?} after writing {:?}: {}", self.dir, path, e);
            }
        }
        Ok(())
    }

    // Closes the live file as `<name>.<stamp>`, giving back the name it now has. An occupied stamp
    // takes a `-1`, `-2` suffix rather than replacing what is already there.
    fn rotate(&self, path: &Path, now_ms: u64) -> Outcome<PathBuf> {
        let name = res!(path.file_name().and_then(|n| n.to_str())
            .ok_or_else(|| err!("Lens path {:?} has no usable file name.", path; Invalid, Path)));
        let base = stamp(now_ms);
        for n in 0..100u32 {
            let suffix = if n == 0 { base.clone() } else { fmt!("{}-{}", base, n) };
            let target = self.dir.join(fmt!("{}.{}", name, suffix));
            if !target.exists() {
                res!(fs::rename(path, &target));
                return Ok(target);
            }
        }
        Err(err!("No free rotation name for {:?} at stamp {}.", path, base; Conflict, File))
    }

    // Prunes the directory. A key's rotated files go by age and by count, then an idle live file
    // goes by age if the gates say so, then the oldest rotated files (and idle live ones) go until
    // the directory fits `max_total`. The file being written is never swept.
    //
    // Age is read from the stamp in a rotated file's name, and only a live file's age comes from
    // its mtime, because only the sink writes a live file.
    fn sweep(&self, keep: &Path, now_ms: u64) -> Outcome<()> {
        struct Rot  { stem: String, st: String, path: PathBuf, len: u64 }
        struct Live { path: PathBuf, len: u64, at: u64 }

        let mut rots:  Vec<Rot>  = Vec::new();
        let mut lives: Vec<Live> = Vec::new();
        for entry in res!(fs::read_dir(&self.dir)) {
            let entry = res!(entry);
            // A name that is not UTF-8 was not written here.
            let fname = match entry.file_name().into_string() {
                Ok(s)   => s,
                Err(_)  => continue,
            };
            let meta = match entry.metadata() {
                Ok(m)   => m,
                Err(_)  => continue,
            };
            if !meta.is_file() {
                continue;
            }
            let dot = fmt!(".{}.", EXT);
            if let Some((stem, tail)) = fname.split_once(dot.as_str()) {
                // A pre-stamp `<name>.ndjson.1` fails `is_stamp` and is left where it is: it
                // predates retention, and nothing here should guess at its age.
                if let Some(st) = tail.get(..STAMP_LEN) {
                    if is_stamp(st) {
                        rots.push(Rot { stem: stem.to_string(), st: st.to_string(), path: entry.path(), len: meta.len() });
                    }
                }
            } else if fname.ends_with(&fmt!(".{}", EXT)) {
                lives.push(Live { path: entry.path(), len: meta.len(), at: mtime_ms(&meta) });
            }
        }

        // Age and count, a key at a time. Oldest first within a key, so the first `over` go.
        let retain  = self.gates.retain_days.saturating_mul(DAY_MS);
        let cutoff  = stamp(now_ms.saturating_sub(retain));
        rots.sort_by(|a, b| a.stem.cmp(&b.stem).then(a.st.cmp(&b.st)).then(a.path.cmp(&b.path)));
        let mut left: Vec<Rot> = Vec::new();
        while let Some(stem) = rots.first().map(|r| r.stem.clone()) {
            let n = rots.iter().take_while(|r| r.stem == stem).count();
            let over = n.saturating_sub(self.gates.max_rotated);
            for (k, r) in rots.drain(..n).enumerate() {
                if k < over || r.st.as_str() < cutoff.as_str() {
                    res!(fs::remove_file(&r.path));
                } else {
                    left.push(r);
                }
            }
        }
        let mut rots = left;

        // A live file that has gone quiet.
        if self.gates.sweep_idle {
            let mut kept = Vec::new();
            for l in lives {
                if l.path != keep && l.at.saturating_add(retain) < now_ms {
                    res!(fs::remove_file(&l.path));
                } else {
                    kept.push(l);
                }
            }
            lives = kept;
        }

        // The total. Rotated files first, oldest stamp first across every key, then idle live
        // ones, oldest first. The file being written is not a candidate.
        let mut total: u64 = rots.iter().map(|r| r.len).sum::<u64>() + lives.iter().map(|l| l.len).sum::<u64>();
        if total > self.gates.max_total {
            rots.sort_by(|a, b| a.st.cmp(&b.st).then(a.path.cmp(&b.path)));
            for r in &rots {
                if total <= self.gates.max_total {
                    break;
                }
                res!(fs::remove_file(&r.path));
                total = total.saturating_sub(r.len);
            }
            if self.gates.sweep_idle && total > self.gates.max_total {
                lives.sort_by_key(|l| l.at);
                for l in &lives {
                    if total <= self.gates.max_total {
                        break;
                    }
                    if l.path != keep {
                        res!(fs::remove_file(&l.path));
                        total = total.saturating_sub(l.len);
                    }
                }
            }
        }
        Ok(())
    }
}

// A member of a map, if it is one and has it.
fn get<'a>(d: &'a Dat, name: &str) -> Option<&'a Dat> {
    d.map_get(&Dat::Str(name.to_string())).ok().flatten()
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_from_days_is_right_at_the_calendar_edges() {
        assert_eq!(civil_from_days(0),          (1970, 1, 1));
        assert_eq!(civil_from_days(-1),         (1969, 12, 31));
        assert_eq!(civil_from_days(11_016),     (2000, 2, 29));  // A leap year divisible by 400.
        assert_eq!(civil_from_days(11_017),     (2000, 3, 1));
        assert_eq!(civil_from_days(-25_508),    (1900, 3, 1));   // Not a leap year, divisible by 100.
        assert_eq!(civil_from_days(47_541),     (2100, 3, 1));
        assert_eq!(civil_from_days(19_782),     (2024, 2, 29));
        assert_eq!(civil_from_days(20_343),     (2025, 9, 12));
    }

    #[test]
    fn clip_cuts_on_a_character_boundary_and_scrub_leaves_no_control() {
        assert_eq!(clip("abc", 3), "abc");
        assert_eq!(clip("abcd", 3), "abc");
        assert_eq!(clip("éé", 3), "é", "a cut through the second é backs up to the boundary");
        assert_eq!(clip("é", 0), "");
        assert_eq!(scrub("a\nb\tc\u{7f}d\u{85}"), "a b c d ");
        assert_eq!(scrub("plain é text"), "plain é text");
    }

    #[test]
    fn a_stamp_in_a_name_must_be_the_whole_sixteen() {
        assert!(is_stamp("20250912T000000Z"));
        assert!(!is_stamp("20250912T000000"));
        assert!(!is_stamp("20250912T00000Zx"));
        assert!(!is_stamp("2025091ZT000000Z"));
    }
}
