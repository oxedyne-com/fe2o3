//! A read-only check of the JDAT text that a store holds, against the decoder as it now refuses.
//!
//! The JDAT text decoder refuses what it once read loosely (a value glued to another with no ','
//! between them, a ',' that follows nothing, a fraction in a whole-number kind). Before a build
//! that refuses goes live, the text a store holds is read through it, and the refusals are counted
//! by class. Two kinds of store hold such text:
//!
//! - **Framed text files** ([`scan_dir`], [`scan_framed`]): Oxegen's peer store keeps one `.log`
//!   or `.tbl` file per table, each record a 4-byte big-endian length and the JDAT text of one
//!   entry. A truncated tail is reported as such.
//! - **An Ozone store** ([`scan_o3db`]): Steel and Daimond keep binary `Dat` values, and a value
//!   may carry text inside it. Every live key is read, the value tree is walked (lists, maps and
//!   their keys, boxes, options, notes, user kinds, tuples) and each `Str` or UTF-8 byte string
//!   that starts like JDAT text (`{`, `[` or `(`) is decoded.
//!
//! **Nothing a store holds is ever printed.** A refusal is reported by the store file (or "o3db"),
//! the record number (or the text's number within the value), its byte offset in the file, a
//! 16-hex-digit SHA3-256 digest to find it by (of the record's bytes, or of the key's display
//! text), the line and column of the refusal, and its class.
//!
//! The scan is read-only, but a read-only open of an Ozone store with garbage collection off still
//! rewrites its `config.jdat`: scan a `cp -a` COPY, never a live store, and never with a second
//! process on the same store.

use crate::{
    prelude::*,
    comm::response::Wait,
};

use oxedyne_fe2o3_hash::hash::HashScheme;
use oxedyne_fe2o3_iop_db::api::ScanOpts;
use oxedyne_fe2o3_iop_hash::api::HashForm;
use oxedyne_fe2o3_jdat::{
    prelude::*,
    id::NumIdDat,
    string::dec::{
        DecoderConfig,
        Located,
    },
    usr::{
        UsrKind,
        UsrKindCode,
        UsrKindId,
    },
};

use std::{
    collections::BTreeMap,
    fs,
    path::Path,
};


type Cfg = DecoderConfig<BTreeMap<UsrKindCode, UsrKind>, BTreeMap<String, UsrKindId>>;

// The scan's own class, for a framed record that is not text at all.
pub const NOT_UTF8: &str = "not UTF-8";

// The classes that are always printed, so that a count of nought is seen as one.
const NAMED: [&str; 3] = [
    "no separator after a comment",
    "whole-number kind with a fraction",
    "no separator",
];

#[derive(Clone, Debug)]
pub struct Refused {
    pub store:  String,         // The file name, or "o3db".
    pub rec:    usize,          // The record, or the text within the value, counting from 1.
    pub at:     u64,            // The record's byte offset in its file, 0 for an Ozone value.
    pub hash:   String,         // 16 hex digits of the SHA3-256 of the record or the key text.
    pub line:   usize,
    pub col:    usize,
    pub class:  &'static str,
}

#[derive(Clone, Debug, Default)]
pub struct TextScan {
    pub files:      usize,                          // Framed files read.
    pub records:    usize,                          // Complete records, or live keys, read.
    pub texts:      usize,                          // Texts given to the decoder.
    pub skipped:    usize,                          // Ozone strings that do not start like JDAT.
    pub truncated:  Vec<(String, u64)>,             // A file and the offset of its broken tail.
    pub refused:    Vec<Refused>,
    pub classes:    BTreeMap<&'static str, usize>,
}

impl TextScan {
    fn refuse(&mut self, r: Refused) {
        *self.classes.entry(r.class).or_insert(0) += 1;
        self.refused.push(r);
    }

    /// Was every text read without a refusal, and every file read to its end?
    pub fn is_clean(&self) -> bool {
        self.refused.is_empty() && self.truncated.is_empty()
    }

    /// The report: a line for each refusal and broken tail, the counts, and the counts by class.
    pub fn report(&self) -> String {
        let mut s = String::new();
        for r in &self.refused {
            s.push_str(&fmt!(
                "REFUSED store={} rec={} at={} hash={} line={} col={} class={}\n",
                r.store, r.rec, r.at, r.hash, r.line, r.col, r.class));
        }
        for (store, at) in &self.truncated {
            s.push_str(&fmt!("TRUNCATED store={} at={}\n", store, at));
        }
        s.push_str(&fmt!(
            "files={} records={} texts={} skipped={} refused={} truncated={}\n",
            self.files, self.records, self.texts, self.skipped,
            self.refused.len(), self.truncated.len()));
        for label in Located::CLASSES.iter().chain([NOT_UTF8].iter()) {
            let n = self.classes.get(label).copied().unwrap_or(0);
            if n > 0 || NAMED.contains(label) {
                s.push_str(&fmt!("class {}: {}\n", label, n));
            }
        }
        s
    }
}

// The first 16 hex digits of the SHA3-256 of some bytes.
fn short_hash(bytes: &[u8]) -> Outcome<String> {
    match HashScheme::new_sha3_256().hash(&[bytes], []).as_hashform() {
        HashForm::Bytes32(h) => Ok(h[..8].iter().map(|b| fmt!("{:02x}", b)).collect::<String>()),
        other => Err(err!(
            "Expected SHA3-256 to give 32 bytes, found {:?}.", other; Data, Mismatch)),
    }
}

// Where and why the decoder refuses a text, or None when it reads.
fn refusal(cfg: &Cfg, text: &str) -> Option<(usize, usize, &'static str)> {
    let got = Dat::decode_string_located(text, cfg); // RED-MARK
    match got {
        Ok(_)   => None,
        Err(at) => Some((at.line, at.col, at.class(text))),
    }
}

/// Reads one framed file's bytes: records of a 4-byte big-endian length and JDAT text.
pub fn scan_framed(
    store:  &str,
    bytes:  &[u8],
    out:    &mut TextScan,
)
    -> Outcome<()>
{
    let cfg = Cfg::default();
    out.files += 1;
    let mut i   = 0usize;
    let mut rec = 0usize;
    while i < bytes.len() {
        if i + 4 > bytes.len() {
            out.truncated.push((store.to_string(), i as u64));
            break;
        }
        let len = u32::from_be_bytes([bytes[i], bytes[i + 1], bytes[i + 2], bytes[i + 3]]) as usize;
        let start = i + 4;
        if start + len > bytes.len() {
            out.truncated.push((store.to_string(), i as u64));
            break;
        }
        let body = &bytes[start..start + len];
        rec += 1;
        out.records += 1;
        let spot = match std::str::from_utf8(body) {
            Ok(text) => {
                out.texts += 1;
                refusal(&cfg, text)
            },
            Err(_) => Some((0, 0, NOT_UTF8)),
        };
        if let Some((line, col, class)) = spot {
            out.refuse(Refused {
                store:  store.to_string(),
                rec,
                at:     i as u64,
                hash:   res!(short_hash(body)),
                line,
                col,
                class,
            });
        }
        i = start + len;
    }
    Ok(())
}

/// Reads every `.log` and `.tbl` file in a directory as framed text, in name order.
pub fn scan_dir(
    dir:    &Path,
    out:    &mut TextScan,
)
    -> Outcome<()>
{
    let mut paths = Vec::new();
    for entry in res!(fs::read_dir(dir)) {
        let path = res!(entry).path();
        let table = match path.extension().and_then(|e| e.to_str()) {
            Some("log") | Some("tbl")   => true,
            _                           => false,
        };
        if table && path.is_file() {
            paths.push(path);
        }
    }
    paths.sort();
    for path in paths {
        let bytes = res!(fs::read(&path));
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("?").to_string();
        res!(scan_framed(&name, &bytes, out));
    }
    Ok(())
}

// Every string and UTF-8 byte string in a value, however deep.
fn gather<'a>(d: &'a Dat, out: &mut Vec<&'a str>) {
    fn each<'a>(ds: &'a [Dat], out: &mut Vec<&'a str>) {
        for d in ds {
            gather(d, out);
        }
    }
    match d {
        Dat::Str(s) => out.push(s.as_str()),
        Dat::BU8(b) | Dat::BU16(b) | Dat::BU32(b) | Dat::BU64(b) => {
            if let Ok(s) = std::str::from_utf8(b) {
                out.push(s);
            }
        },
        Dat::List(v)    => each(v, out),
        Dat::Map(m)     => {
            for (k, v) in m {
                gather(k, out);
                gather(v, out);
            }
        },
        Dat::OrdMap(m)  => {
            for (k, v) in m {
                gather(k.dat(), out);
                gather(v, out);
            }
        },
        Dat::Box(b)             => gather(b, out),
        Dat::Opt(o)             => {
            if let Some(x) = &**o {
                gather(x, out);
            }
        },
        Dat::ABox(_, b, _)      => gather(b, out),
        Dat::Usr(_, Some(b))    => gather(b, out),
        Dat::Tup2(t)    => each(&**t, out),
        Dat::Tup3(t)    => each(&**t, out),
        Dat::Tup4(t)    => each(&**t, out),
        Dat::Tup5(t)    => each(&**t, out),
        Dat::Tup6(t)    => each(&**t, out),
        Dat::Tup7(t)    => each(&**t, out),
        Dat::Tup8(t)    => each(&**t, out),
        Dat::Tup9(t)    => each(&**t, out),
        Dat::Tup10(t)   => each(&**t, out),
        _ => {},
    }
}

/// Reads every live key of an Ozone store and decodes the JDAT text its values hold.
///
/// The store must be a COPY, opened with garbage collection off and used by nobody else; this
/// function issues no write. A key whose newest record is a deletion reads as absent and is
/// passed over.
///
/// # Arguments
/// * `scan_wait` - how long each zone's scan may take; a large store needs a generous wait.
pub fn scan_o3db<
    const UIDL: usize,
    UID:    NumIdDat<UIDL> + 'static,
    ENC:    Encrypter + 'static,
    KH:     Hasher + 'static,
    PR:     Hasher + 'static,
    CS:     Checksummer + 'static,
>(
    api:        &OzoneApi<UIDL, UID, ENC, KH, PR, CS>,
    scan_wait:  Wait,
    out:        &mut TextScan,
)
    -> Outcome<()>
{
    let cfg = Cfg::default();
    let entries = res!(api.scan_with_wait(&ScanOpts::all(), None, scan_wait));
    for (kdat, _, _) in &entries {
        let val = match res!(api.get_wait(kdat, None)) {
            None            => continue,
            Some((val, _))  => val,
        };
        out.records += 1;
        let mut found = Vec::new();
        gather(&val, &mut found);
        let mut rec = 0usize;
        for text in found {
            let t = text.trim_start();
            if !(t.starts_with('{') || t.starts_with('[') || t.starts_with('(')) {
                out.skipped += 1;
                continue;
            }
            rec += 1;
            out.texts += 1;
            if let Some((line, col, class)) = refusal(&cfg, text) {
                out.refuse(Refused {
                    store:  "o3db".to_string(),
                    rec,
                    at:     0,
                    hash:   res!(short_hash(fmt!("{}", kdat).as_bytes())),
                    line,
                    col,
                    class,
                });
            }
        }
    }
    Ok(())
}
