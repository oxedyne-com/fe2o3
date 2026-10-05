//! A read-only check of JDAT text a store holds, against the decoder as it now refuses.
//!
//! The decoder refuses what it once read loosely (a value glued to another with no ',' between
//! them, a ',' that follows nothing, a fraction in a whole-number kind). Before a build that
//! refuses goes live, the text a store holds is read through it and the refusals are counted by
//! class. This module reads **framed text files** (`.log` and `.tbl`): each record is a 4-byte
//! big-endian length and the JDAT text of one entry, as Oxegen's peer store keeps its tables. A
//! truncated tail is reported as such. A store that holds text inside binary values (an Ozone
//! store) hands each text to [`refusal`] and records what it finds with [`TextScan::refuse`].
//!
//! **Nothing a store holds is ever printed.** A refusal is reported by the file, the record
//! number, its byte offset in the file, a 16-hex-digit tag to find it by, the line and column of
//! the refusal and its class. The scan is read-only, and is run on a `cp -a` COPY of a store,
//! never on a live one.

use crate::{
    prelude::*,
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

use oxedyne_fe2o3_core::prelude::*;

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
    pub hash:   String,         // The tag of the record, or of the key's text.
    pub line:   usize,
    pub col:    usize,
    pub class:  &'static str,
}

#[derive(Clone, Debug, Default)]
pub struct TextScan {
    pub files:      usize,                          // Framed files read.
    pub records:    usize,                          // Complete records, or live keys, read.
    pub texts:      usize,                          // Texts given to the decoder.
    pub skipped:    usize,                          // Strings that do not start like JDAT.
    pub truncated:  Vec<(String, u64)>,             // A file and the offset of its broken tail.
    pub refused:    Vec<Refused>,
    pub classes:    BTreeMap<&'static str, usize>,
}

impl TextScan {
    pub fn refuse(&mut self, r: Refused) {
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

/// A 16-hex-digit tag of some bytes (FNV-1a, 64 bits), to find a record by without printing it.
pub fn tag(bytes: &[u8]) -> String {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    fmt!("{:016x}", h)
}

/// Where and why the decoder refuses a text, as line, column and class, or None when it reads.
pub fn refusal(text: &str) -> Option<(usize, usize, &'static str)> {
    match Dat::decode_string_located(text, &Cfg::default()) {
        Ok(_)   => None,
        Err(at) => Some((at.line, at.col, at.class(text))),
    }
}

/// Is a file's text jdat, and where does it stop? The line `jdat_check` prints for a file, which
/// is `path: OK` or `path: REFUSED line L col C: tags`: the place and the error's tags, never the
/// refusal's words and never a key or a value.
pub fn check_line(path: &str, text: &str) -> (bool, String) {
    match Dat::decode_string_located(text, &Cfg::default()) {
        Ok(_)   => (true, fmt!("{}: OK", path)),
        Err(at) => {
            let kind = Error::<ErrTag>::tags_display(at.error.tags());
            (false, fmt!("{}: REFUSED line {} col {}: {}", path, at.line, at.col, kind))
        },
    }
}

/// Reads one framed file's bytes: records of a 4-byte big-endian length and JDAT text.
pub fn scan_framed(
    store:  &str,
    bytes:  &[u8],
    out:    &mut TextScan,
) {
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
                refusal(text)
            },
            Err(_) => Some((0, 0, NOT_UTF8)),
        };
        if let Some((line, col, class)) = spot {
            out.refuse(Refused {
                store:  store.to_string(),
                rec,
                at:     i as u64,
                hash:   tag(body),
                line,
                col,
                class,
            });
        }
        i = start + len;
    }
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
        scan_framed(&name, &bytes, out);
    }
    Ok(())
}
