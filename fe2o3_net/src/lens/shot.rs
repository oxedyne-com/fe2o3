//! A bounded store of pictures taken on request, the screenshot half of a debug lens.
//!
//! Written for Daimond's Lens (D-20261006-14), where the owner asks one of his own devices for a
//! picture of its screen and the device answers with a PNG or with the reason it will not. The
//! exchange is three files under one directory, so a reader on another machine needs nothing but
//! a copy of it (Daimond's `dev/lens.mjs` pulls it with rsync):
//!
//! * `want/<device>` holds the id of an ask (`Shots::ask`, or any writer that puts the id there).
//! * `out/<account>.<device>` holds the ask last handed to that account's device and when, which
//!   is both the rate floor and the proof an answer was asked for.
//! * `<stamp>.<account>.<device>.<id>.png` is a picture and `….refused` a refusal, written whole
//!   under a `.part` name and renamed, so a copy taken mid-write never holds half a picture.
//!
//! Every segment passes `sanitise`, whose alphabet holds no `.`, so a name splits unambiguously.
//! An answer is accepted once, only for the id handed out, and only within `answer_ms`; a
//! device asked again inside `floor_ms` is refused on the device's behalf and never troubled.

use super::sink::{
    sanitise,
    stamp,
    STAMP_LEN,
};

use oxedyne_fe2o3_core::prelude::*;

use std::{
    fs,
    io::Write,
    path::{
        Path,
        PathBuf,
    },
    time::SystemTime,
};


const PNG_MAGIC:    &[u8]   = b"\x89PNG\r\n\x1a\n";
const PICTURE_EXT:  &str    = "png";
const REFUSED_EXT:  &str    = "refused";
const PART_EXT:     &str    = "part";
const WANT_DIR:     &str    = "want";
const OUT_DIR:      &str    = "out";
const MAX_REASON:   usize   = 200;              // bytes of a refusal's reason kept

#[derive(Clone, Debug)]
pub struct ShotGates {
    pub max_bytes:  usize,  // the largest picture accepted
    pub max_total:  u64,    // the most bytes of pictures and refusals the directory may hold
    pub keep_ms:    u64,    // how long a picture or refusal is kept
    pub floor_ms:   u64,    // the shortest gap between two asks handed to one account's device
    pub answer_ms:  u64,    // how long an ask handed out may still be answered
    pub want_ms:    u64,    // how long an ask nobody collected waits
}

impl ShotGates {

    /// Daimond's numbers: a 3 MiB picture, 7 days, one ask a minute, two minutes to answer it.
    pub fn daimond() -> Self {
        Self {
            max_bytes:  3 * 1024 * 1024,
            max_total:  500 * 1024 * 1024,
            keep_ms:    7 * 86_400_000,
            floor_ms:   60_000,
            answer_ms:  120_000,
            want_ms:    600_000,
        }
    }
}

impl Default for ShotGates {
    fn default() -> Self { Self::daimond() }
}

/// What a device is told when it asks whether a picture is wanted.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Ask {
    Nothing,                // no ask waits for this device
    Take(String),           // take a picture and answer it under this id
    TooSoon(String, u64),   // the ask with this id was refused, the floor lifts in this many ms
}

/// A device's answer to an ask.
#[derive(Clone, Copy, Debug)]
pub enum Answer<'a> {
    Picture(&'a [u8]),
    Refused(&'a str),
}

/// Why an answer was turned away. Nothing was written.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ShotRefusal {
    Unasked,                // 409, the id was not handed to this device, was answered or expired
    NotPicture,             // 415, the body is not a PNG
    TooLarge,               // 413
}

impl ShotRefusal {

    pub fn status(&self) -> u16 {
        match self {
            Self::Unasked       => 409,
            Self::NotPicture    => 415,
            Self::TooLarge      => 413,
        }
    }

    pub fn message(&self) -> String {
        match self {
            Self::Unasked       => "No picture was asked of this device under that id.".to_string(),
            Self::NotPicture    => "A picture must be a PNG.".to_string(),
            Self::TooLarge      => "The picture is too large.".to_string(),
        }
    }
}

/// The result of an answer: the file written, or the reason none was.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Filed {
    Stored(PathBuf),
    Refused(ShotRefusal),
}

/// The store. It holds no state of its own beyond its directory, so two processes may share one.
#[derive(Clone, Debug)]
pub struct Shots {
    dir:    PathBuf,
    gates:  ShotGates,
}

impl Shots {

    pub fn new<P: Into<PathBuf>>(dir: P, gates: ShotGates) -> Self {
        Self { dir: dir.into(), gates }
    }

    pub fn dir(&self) -> &Path { &self.dir }

    pub fn gates(&self) -> &ShotGates { &self.gates }

    /// Asks `device` for a picture under `id`, replacing any ask not yet collected.
    pub fn ask(&self, device: &str, id: &str) -> Outcome<()> {
        let path = self.dir.join(WANT_DIR).join(sanitise(device));
        write_whole(&path, sanitise(id).as_bytes())
    }

    /// Collects the ask waiting for `account`'s `device`, if any. An ask is collected once: it is
    /// either handed out, or refused on the device's behalf when the last one was handed out
    /// less than `floor_ms` ago, and that refusal is filed under the ask's id for the asker.
    pub fn take(&self, account: &str, device: &str, now_ms: u64) -> Outcome<Ask> {
        let want = self.dir.join(WANT_DIR).join(sanitise(device));
        let id = match fs::read_to_string(&want) {
            Ok(s) => sanitise(s.trim()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Ask::Nothing),
            Err(e) => return Err(err!(e, "While reading the ask {:?}.", want; IO, File, Read)),
        };
        res!(fs::remove_file(&want), IO, File);
        let out = self.out_path(account, device);
        if let Some((_, at)) = res!(read_out(&out)) {
            let since = now_ms.saturating_sub(at);
            if since < self.gates.floor_ms {
                let wait = self.gates.floor_ms - since;
                let reason = fmt!("rate: the next picture may be taken in {} s", (wait + 999) / 1_000);
                res!(self.file(account, device, &id, REFUSED_EXT, reason.as_bytes(), now_ms));
                return Ok(Ask::TooSoon(id, wait));
            }
        }
        res!(write_whole(&out, fmt!("{} {}", id, now_ms).as_bytes()));
        Ok(Ask::Take(id))
    }

    /// Files `account`'s `device`'s answer to the ask handed out under `id`.
    pub fn answer(
        &self,
        account:    &str,
        device:     &str,
        id:         &str,
        answer:     Answer,
        now_ms:     u64,
    )
        -> Outcome<Filed>
    {
        let id = sanitise(id);
        let out = self.out_path(account, device);
        let at = match res!(read_out(&out)) {
            Some((handed, at)) if handed == id && now_ms.saturating_sub(at) <= self.gates.answer_ms => at,
            _ => return Ok(Filed::Refused(ShotRefusal::Unasked)),
        };
        let (ext, body) = match answer {
            Answer::Picture(png) => {
                if png.len() > self.gates.max_bytes {
                    return Ok(Filed::Refused(ShotRefusal::TooLarge));
                }
                if !png.starts_with(PNG_MAGIC) {
                    return Ok(Filed::Refused(ShotRefusal::NotPicture));
                }
                (PICTURE_EXT, png.to_vec())
            }
            Answer::Refused(reason) => (REFUSED_EXT, clip(&scrub(reason), MAX_REASON).into_bytes()),
        };
        // Consumed: the id is struck out but the instant stays, so the floor still holds.
        res!(write_whole(&out, fmt!("- {}", at).as_bytes()));
        let path = res!(self.file(account, device, &id, ext, &body, now_ms));
        res!(self.prune(now_ms));
        Ok(Filed::Stored(path))
    }

    /// The picture or refusal filed for `device` under `id`, if there is one.
    pub fn find(&self, device: &str, id: &str) -> Outcome<Option<PathBuf>> {
        let tail_png = fmt!(".{}.{}.{}", sanitise(device), sanitise(id), PICTURE_EXT);
        let tail_ref = fmt!(".{}.{}.{}", sanitise(device), sanitise(id), REFUSED_EXT);
        for name in res!(names(&self.dir)) {
            if name.ends_with(&tail_png) || name.ends_with(&tail_ref) {
                return Ok(Some(self.dir.join(name)));
            }
        }
        Ok(None)
    }

    /// Removes what has outlived the gates: pictures and refusals older than `keep_ms` by the
    /// stamp in their names, then the oldest of them until the rest fit `max_total`, and asks,
    /// hand-outs and abandoned `.part` files by modification time. Returns how many went.
    pub fn prune(&self, now_ms: u64) -> Outcome<usize> {
        let mut gone = 0;
        let cut = stamp(now_ms.saturating_sub(self.gates.keep_ms));
        let mut kept: Vec<(String, u64)> = Vec::new();
        for name in res!(names(&self.dir)) {
            let path = self.dir.join(&name);
            if name.ends_with(&fmt!(".{}", PART_EXT)) {
                if res!(older_than(&path, now_ms, self.gates.want_ms)) {
                    gone += res!(remove(&path));
                }
                continue;
            }
            let filed = name.ends_with(&fmt!(".{}", PICTURE_EXT))
                || name.ends_with(&fmt!(".{}", REFUSED_EXT));
            if !filed || name.len() <= STAMP_LEN || !super::sink::is_stamp(&name[..STAMP_LEN]) {
                continue; // Not ours, and nothing here should guess at its age.
            }
            if name[..STAMP_LEN] < *cut.as_str() {
                gone += res!(remove(&path));
            } else {
                let len = res!(fs::metadata(&path), IO, File).len();
                kept.push((name, len));
            }
        }
        kept.sort();
        let mut total: u64 = kept.iter().map(|(_, n)| *n).sum();
        for (name, len) in kept {
            if total <= self.gates.max_total {
                break;
            }
            gone += res!(remove(&self.dir.join(&name)));
            total -= len;
        }
        for (sub, age) in [(WANT_DIR, self.gates.want_ms), (OUT_DIR, self.gates.want_ms.max(self.gates.floor_ms))] {
            let dir = self.dir.join(sub);
            for name in res!(names(&dir)) {
                let path = dir.join(name);
                if res!(older_than(&path, now_ms, age)) {
                    gone += res!(remove(&path));
                }
            }
        }
        Ok(gone)
    }

    fn out_path(&self, account: &str, device: &str) -> PathBuf {
        self.dir.join(OUT_DIR).join(fmt!("{}.{}", sanitise(account), sanitise(device)))
    }

    // Writes `<stamp>.<account>.<device>.<id>.<ext>` whole.
    fn file(&self, account: &str, device: &str, id: &str, ext: &str, body: &[u8], now_ms: u64)
        -> Outcome<PathBuf>
    {
        let name = fmt!("{}.{}.{}.{}.{}",
            stamp(now_ms), sanitise(account), sanitise(device), sanitise(id), ext);
        let path = self.dir.join(name);
        res!(write_whole(&path, body));
        Ok(path)
    }
}

// Writes under a `.part` name, then renames, so a reader sees all of the file or none of it.
fn write_whole(path: &Path, body: &[u8]) -> Outcome<()> {
    if let Some(parent) = path.parent() {
        res!(fs::create_dir_all(parent), IO, File);
    }
    let part = path.with_extension(match path.extension() {
        Some(e) => fmt!("{}.{}", e.to_string_lossy(), PART_EXT),
        None    => PART_EXT.to_string(),
    });
    {
        let mut f = res!(fs::File::create(&part), IO, File, Write);
        res!(f.write_all(body), IO, File, Write);
        res!(f.sync_all(), IO, File, Write);
    }
    res!(fs::rename(&part, path), IO, File, Write);
    Ok(())
}

// `<id> <ms>`, the ask last handed out and when; the id is `-` once answered.
fn read_out(path: &Path) -> Outcome<Option<(String, u64)>> {
    let s = match fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(err!(e, "While reading the hand-out {:?}.", path; IO, File, Read)),
    };
    let mut it = s.split_whitespace();
    match (it.next(), it.next().and_then(|t| t.parse::<u64>().ok())) {
        (Some(id), Some(at))    => Ok(Some((id.to_string(), at))),
        _                       => Ok(None), // A torn record is no hand-out.
    }
}

// The plain files directly under `dir`, sorted; none when it does not exist yet.
fn names(dir: &Path) -> Outcome<Vec<String>> {
    let rd = match fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(err!(e, "While listing {:?}.", dir; IO, File, Read)),
    };
    let mut out = Vec::new();
    for ent in rd {
        let ent = res!(ent, IO, File, Read);
        let ft = res!(ent.file_type(), IO, File, Read);
        if ft.is_file() {
            out.push(ent.file_name().to_string_lossy().to_string());
        }
    }
    out.sort();
    Ok(out)
}

fn older_than(path: &Path, now_ms: u64, age_ms: u64) -> Outcome<bool> {
    let mt = res!(res!(fs::metadata(path), IO, File).modified());
    let ms = match mt.duration_since(SystemTime::UNIX_EPOCH) {
        Ok(d)   => d.as_millis() as u64,
        Err(_)  => 0,
    };
    Ok(now_ms.saturating_sub(ms) > age_ms)
}

// Removes a file another pruner may have removed first.
fn remove(path: &Path) -> Outcome<usize> {
    match fs::remove_file(path) {
        Ok(())                                                  => Ok(1),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound      => Ok(0),
        Err(e) => Err(err!(e, "While removing {:?}.", path; IO, File)),
    }
}

fn scrub(s: &str) -> String {
    s.chars().map(|c| if c.is_control() { ' ' } else { c }).collect()
}

fn clip(s: &str, n: usize) -> String {
    if s.len() <= n {
        return s.to_string();
    }
    let mut end = n;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    s[..end].to_string()
}
