//! Says whether each file named on the command line reads as jdat, and where it does not.
//!
//! ```text
//! jdat_check <path>...
//! jdat_check --logs DIR
//! ```
//!
//! For each path one line is printed: `path: OK`, or `path: REFUSED line L col C: kind`, or
//! `path: UNREADABLE: kind`.  The text of a file is never printed, nor any part of it, and the
//! kind is taken from the error's tags and not from its message.  No key is printed, since a key
//! may be personal data.  The exit status is 0 when every file reads, 1 when one is refused or
//! unreadable, and 2 with no paths.
//!
//! With `--logs DIR` the directory is an Oxegen data directory: each `.log` and `.tbl` file in it
//! is a run of records, each a 4-byte big-endian length and the jdat text of one entry. Every
//! record is read through the decoder and the output is one `REFUSED` line for each refusal (the
//! file, the record, its byte offset, a tag to find it by, the line, the column and the class), a
//! `TRUNCATED` line for a file that ends in a broken record, the counts, and the counts by class
//! (`oxedyne_fe2o3_jdat::string::scan`). Nothing a record holds is printed. The exit status is 0
//! when every record reads, 1 when one is refused or a file is truncated, and 2 on a usage error
//! or a directory that holds no table file. Run it on a `cp -a` COPY of the data directory.
//!
//! The decoder is the one `Dat::decode_string` uses, so a file that reads here reads for any
//! program that loads it that way.  Built static, this is run on a host to learn which of its
//! configuration files a stricter decoder would stop at, before the decoder is deployed.

use oxedyne_fe2o3_jdat::string::scan::{
    self,
    TextScan,
};

use std::{
    env,
    fs,
    path::Path,
    process,
};

// Reads every record of every table file in a directory, and reports the refusals by class.
fn logs(dir: &str) -> ! {
    let mut out = TextScan::default();
    if let Err(e) = scan::scan_dir(Path::new(dir), &mut out) {
        eprintln!("jdat_check FAILED: {}", e);
        process::exit(2);
    }
    if out.files == 0 {
        eprintln!("No .log or .tbl file in {:?}; this is not an Oxegen data directory.", dir);
        process::exit(2);
    }
    print!("{}", out.report());
    if out.is_clean() {
        println!("CLEAN: every record read.");
        process::exit(0);
    }
    println!("NOT CLEAN: see the REFUSED and TRUNCATED lines.");
    process::exit(1);
}

fn main() {
    let paths = env::args().skip(1).collect::<Vec<_>>();
    if paths.len() == 2 && paths[0] == "--logs" {
        logs(&paths[1]);
    }
    if paths.len() == 0 || paths[0] == "--logs" {
        eprintln!("Usage: jdat_check <path>...   or   jdat_check --logs DIR");
        process::exit(2);
    }
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
        let (read, line) = scan::check_line(path, &text);
        println!("{}", line);
        if !read {
            bad += 1;
        }
    }
    process::exit(if bad == 0 { 0 } else { 1 });
}
