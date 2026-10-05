//! The framed text scan (`string::scan`), on a framed log built in memory and on a directory.
//!
//! The scan counts a clean record, a value glued to another, a comma-less comment, a fraction in
//! a whole-number kind, a record that is not UTF-8 and a truncated tail, each in its class, and
//! the report holds the place (line and column) and a tag but never a word of what was planted,
//! and `jdat_check`'s line for a file does the same.

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_jdat::string::scan;

use std::fs;

// Words planted in the bad text, which no report may ever repeat.
const SECRET: &str = "s3cret";

fn frame(text: &[u8]) -> Vec<u8> {
    let mut f = (text.len() as u32).to_be_bytes().to_vec();
    f.extend_from_slice(text);
    f
}

#[test]
fn test_a_framed_scan_counts_by_class_and_prints_no_text_00() -> Outcome<()> {
    let glued   = fmt!("{{\"seq\": 3, \"{}\": \"x\" \"y\": 1}}", SECRET);
    let note    = "{\"seq\": 4, \"a\": 1 # note\n \"b\": 2}";
    let frac    = "{\"seq\": 5, \"n\": (u8|2.5)}";
    let mut file = Vec::new();
    let mut offs = Vec::new();
    for rec in [
        frame(b"{\"seq\": 1, \"k\": \"v\"}"),
        frame(b"{\"seq\": 2, \"k\": [1, 2]}"),
        frame(glued.as_bytes()),
        frame(note.as_bytes()),
        frame(frac.as_bytes()),
        frame(&[0xff, 0xfe, 0x41]),
    ] {
        offs.push(file.len() as u64);
        file.extend_from_slice(&rec);
    }
    // A length that promises more than the file holds.
    offs.push(file.len() as u64);
    file.extend_from_slice(&100u32.to_be_bytes());
    file.extend_from_slice(b"abc");

    let mut out = scan::TextScan::default();
    scan::scan_framed("things.log", &file, &mut out);
    let got = |class: &str| out.classes.get(class).copied().unwrap_or(0);
    req!(out.records, 6);
    req!(out.texts, 5);
    req!(out.refused.len(), 4);
    req!(out.truncated, vec![("things.log".to_string(), offs[6])]);
    req!(got("no separator"), 1);
    req!(got("no separator after a comment"), 1);
    req!(got("whole-number kind with a fraction"), 1);
    req!(got(scan::NOT_UTF8), 1);
    // The first refusal is the third record, at its own offset, with its own tag.
    let first = &out.refused[0];
    req!((first.rec, first.at, first.line), (3, offs[2], 1));
    req!(first.hash.len(), 16);
    req!(first.hash.clone(), scan::tag(glued.as_bytes()));
    req!(first.class, "no separator");
    req!(out.is_clean(), false);
    // Every refusal of text says where, as a line and a column, and a report line carries both.
    for r in &out.refused {
        if r.class != scan::NOT_UTF8 {
            req!((r.line >= 1 && r.col >= 1), true);
        }
    }
    let report = out.report();
    for line in report.lines().filter(|l| l.starts_with("REFUSED ") && !l.ends_with(scan::NOT_UTF8)) {
        let num = |label: &str| line.split(' ').filter_map(|w| w.strip_prefix(label))
            .filter_map(|n| n.parse::<usize>().ok()).next().unwrap_or(0);
        if num("line=") < 1 || num("col=") < 1 {
            return Err(err!("A REFUSED line does not say where: {}", line; Test, Data));
        }
    }
    req!(report.contains(&fmt!("line={} col={}", first.line, first.col)), true);
    for word in [SECRET, "seq", "note", "2.5"] {
        if report.contains(word) {
            return Err(err!("The report repeats {:?}: {}", word, report; Test, Data));
        }
    }
    for line in ["REFUSED store=things.log rec=3", "TRUNCATED store=things.log at=",
        "class no separator after a comment: 1", "class whole-number kind with a fraction: 1"]
    {
        if !report.contains(line) {
            return Err(err!("The report lacks {:?}: {}", line, report; Test, Data));
        }
    }

    // A clean file reports nothing, and says so.
    let mut clean = scan::TextScan::default();
    scan::scan_framed("ok.log", &file[..offs[2] as usize], &mut clean);
    req!((clean.records, clean.texts, clean.refused.len()), (2, 2, 0));
    req!(clean.is_clean(), true);
    Ok(())
}

#[test]
fn test_a_directory_scan_reads_logs_and_tables_only_00() -> Outcome<()> {
    let name = "./test_textscan_dir";
    let _ = fs::remove_dir_all(name);
    res!(fs::create_dir_all(name));
    let dir = res!(std::path::Path::new(name).canonicalize());
    res!(fs::write(dir.join("b.log"), frame(b"{\"a\": 1 \"b\": 2}")));
    res!(fs::write(dir.join("a.tbl"), frame(b"{\"a\": 1}")));
    res!(fs::write(dir.join("notes.txt"), b"{\"a\": 1 \"b\": 2}"));
    let mut out = scan::TextScan::default();
    res!(scan::scan_dir(&dir, &mut out));
    req!((out.files, out.records, out.refused.len()), (2, 2, 1));
    req!(out.refused[0].store.as_str(), "b.log");
    // A directory that does not exist is an error, not an empty scan.
    let mut none = scan::TextScan::default();
    req!(scan::scan_dir(&dir.join("absent"), &mut none).is_err(), true);
    let _ = fs::remove_dir_all(&dir);
    Ok(())
}

#[test]
fn test_a_tag_is_sixteen_hex_digits_and_fixed_00() -> Outcome<()> {
    // The FNV-1a 64 test vectors: the empty input is its offset basis, "a" is af63dc4c8601ec8c.
    req!(scan::tag(b""), "cbf29ce484222325".to_string());
    req!(scan::tag(b"a"), "af63dc4c8601ec8c".to_string());
    req!(scan::tag(b"foobar"), "85944171f73967e8".to_string());
    Ok(())
}

#[test]
fn test_jdat_check_says_where_and_prints_no_text_00() -> Outcome<()> {
    // A refused file is one line: its path, the line and the column, and the error's tags. No
    // key, no value, and none of the refusal's words, which a caller's log would hold.
    let key = "Qv7kXp";
    let val = "Zq9xW";
    for (text, line) in [
        (fmt!("{{\"{}\": {} \"{}2\": 1}}", key, val, key), 1),
        (fmt!("{{\n\"a\": 1,\n\"{}\": (u8|{}.5)}}", key, val), 3),
        (fmt!("{{\"{}\": 1,\n \"{}\": 2}}", key, key), 2),
        (fmt!("{{\"{}\": [1,, {}]}}", key, val), 1),
    ] {
        let (read, said) = scan::check_line("f.jdat", &text);
        req!(read, false);
        let want = fmt!("f.jdat: REFUSED line {} col ", line);
        if !said.starts_with(&want) {
            return Err(err!("The line should start {:?}, and reads {:?}", want, said; Test, Data));
        }
        let col = said[want.len()..].split(':').next().and_then(|n| n.parse::<usize>().ok()).unwrap_or(0);
        req!((col >= 1), true);
        for word in [key, val, "7351846"] {
            if said.contains(word) {
                return Err(err!("The line repeats {:?}: {}", word, said; Test, Data));
            }
        }
    }
    req!(scan::check_line("g.jdat", "{\"a\": [1, 2]}"), (true, "g.jdat: OK".to_string()));
    Ok(())
}
