//! Says whether each file named on the command line reads as jdat, and where it does not.
//!
//! ```text
//! jdat_check <path>...
//! ```
//!
//! For each path one line is printed: `path: OK`, or `path: REFUSED line L col C key K: kind`, or
//! `path: UNREADABLE: kind`.  The text of a file is never printed, nor any part of it, and the
//! kind is taken from the error's tags and not from its message, which may quote the text.  The
//! key is the file's own dotted path to the member being read, such as `roles.qr.size`.  The exit
//! status is 0 when every file reads, 1 when one is refused or unreadable, and 2 with no paths.
//!
//! The decoder is the one `Dat::decode_string` uses, so a file that reads here reads for any
//! program that loads it that way.  Built static, this is run on a host to learn which of its
//! configuration files a stricter decoder would stop at, before the decoder is deployed.

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_jdat::{
    prelude::*,
    string::dec::DecoderConfig,
    usr::{
        UsrKind,
        UsrKindCode,
        UsrKindId,
    },
};

use std::{
    collections::BTreeMap,
    env,
    fs,
    process,
};

fn main() {
    let paths = env::args().skip(1).collect::<Vec<_>>();
    if paths.len() == 0 {
        eprintln!("Usage: jdat_check <path>...");
        process::exit(2);
    }
    let cfg = DecoderConfig::<BTreeMap<UsrKindCode, UsrKind>, BTreeMap<String, UsrKindId>>::default();
    let mut bad = 0;
    for path in &paths {
        let bytes = match fs::read(path) {
            Ok(bytes) => bytes,
            Err(e) => {
                println!("{}: UNREADABLE: {:?}", path, e.kind());
                bad += 1;
                continue;
            },
        };
        let text = match String::from_utf8(bytes) {
            Ok(text) => text,
            Err(_) => {
                println!("{}: UNREADABLE: NotUtf8", path);
                bad += 1;
                continue;
            },
        };
        match Dat::decode_string_located(text, &cfg) {
            Ok(_) => println!("{}: OK", path),
            Err(at) => {
                let kind = Error::<ErrTag>::tags_display(at.error.tags());
                println!("{}: REFUSED line {} col {} key {}: {}", path, at.line, at.col,
                    if at.key.is_empty() { "-" } else { at.key.as_str() }, kind);
                bad += 1;
            },
        }
    }
    process::exit(if bad == 0 { 0 } else { 1 });
}
