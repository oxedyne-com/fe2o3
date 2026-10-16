//! Ink against the `typst` 0.15.1 oracle (U6b-S2): what a page draws, compared on the rendered file. Each
//! fixture under `tests/fixtures/ink/`, outside the evaluator corpus whose level 4 reads text lines only, is compiled to a PDF by Austenite's pipeline and by Typst, both are
//! rasterised by `pdftoppm`, and the two pictures are compared: the box the ink spans must agree, and few
//! pixels may differ. Text under a transform, a clip and a stroke are geometry that the line reader of level 4
//! cannot see.

#![allow(dead_code)]

#[path = "eval_oracle/mod.rs"]
mod harness;

use harness::oracle::Oracle;
use harness::pdf::{
	austenite_pdf,
	tool,
	typst_pdf,
	work_dir,
};

use oxedyne_fe2o3_core::prelude::*;

use std::path::{
	Path,
	PathBuf,
};

const DPI:			&str	= "100";
const INK:			u8		= 200;		// a pixel darker than this is ink
const DIFF:			i32		= 100;		// grey levels two pixels must differ by to count
const SLACK_PX:		i64		= 2;		// the ink box may move this many pixels
const FRACTION:		f64		= 0.04;		// of Typst's ink pixels that may differ
const MIN_COMPARED:	usize	= 7;

/// A grey picture, a row at a time.
struct Grey {
	w:		usize,
	h:		usize,
	data:	Vec<u8>,
}

/// Reads the binary PGM `pdftoppm -gray` writes.
fn read_pgm(path: &Path) -> Outcome<Grey> {
	let bytes = res!(std::fs::read(path));
	let mut at = 0usize;
	let mut tokens: Vec<String> = Vec::new();
	while tokens.len() < 4 && at < bytes.len() {
		while at < bytes.len() && bytes[at].is_ascii_whitespace() {
			at += 1;
		}
		if at < bytes.len() && bytes[at] == b'#' {
			while at < bytes.len() && bytes[at] != b'\n' {
				at += 1;
			}
			continue;
		}
		let from = at;
		while at < bytes.len() && !bytes[at].is_ascii_whitespace() {
			at += 1;
		}
		tokens.push(String::from_utf8_lossy(&bytes[from..at]).to_string());
	}
	at += 1;	// the one whitespace byte after the maximum value
	if tokens.len() < 4 || tokens[0] != "P5" {
		return Err(err!("{} is not a binary PGM", path.display(); Test));
	}
	let w: usize = res!(tokens[1].parse::<usize>().map_err(|e| err!("PGM width: {}", e; Test)));
	let h: usize = res!(tokens[2].parse::<usize>().map_err(|e| err!("PGM height: {}", e; Test)));
	if bytes.len() < at + w * h {
		return Err(err!("{} holds {} pixel bytes, {} expected", path.display(), bytes.len() - at, w * h; Test));
	}
	Ok(Grey { w, h, data: bytes[at..at + w * h].to_vec() })
}

impl Grey {
	/// The box the ink spans, left, top, right, bottom, and how many pixels are ink.
	fn ink(&self) -> (i64, i64, i64, i64, usize) {
		let (mut x0, mut y0, mut x1, mut y1, mut n) = (i64::MAX, i64::MAX, -1i64, -1i64, 0usize);
		for y in 0..self.h {
			for x in 0..self.w {
				if self.data[y * self.w + x] < INK {
					x0 = x0.min(x as i64);
					y0 = y0.min(y as i64);
					x1 = x1.max(x as i64);
					y1 = y1.max(y as i64);
					n += 1;
				}
			}
		}
		(x0, y0, x1, y1, n)
	}
}

/// The pages of `pdf` as pictures, in order.
fn pages(pdf: &Path, dir: &Path, tag: &str) -> Outcome<Vec<Grey>> {
	let prefix = dir.join(tag);
	for entry in res!(std::fs::read_dir(dir)) {
		let p = res!(entry).path();
		let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
		if name.starts_with(&fmt!("{}-", tag)) && name.ends_with(".pgm") {
			res!(std::fs::remove_file(&p));
		}
	}
	let prefix_s = prefix.display().to_string();
	res!(tool("pdftoppm", &["-r", DPI, "-gray"], pdf, &[&prefix_s]));
	let mut files: Vec<std::path::PathBuf> = Vec::new();
	for entry in res!(std::fs::read_dir(dir)) {
		let p = res!(entry).path();
		let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
		if name.starts_with(&fmt!("{}-", tag)) && name.ends_with(".pgm") {
			files.push(p);
		}
	}
	files.sort();
	let mut out = Vec::new();
	for f in files {
		out.push(res!(read_pgm(&f)));
	}
	Ok(out)
}

/// Where Austenite's pictures differ from Typst's.
fn differences(want: &[Grey], got: &[Grey]) -> Vec<String> {
	let mut out = Vec::new();
	if want.len() != got.len() {
		out.push(fmt!("page count: typst {}, austenite {}", want.len(), got.len()));
	}
	for (i, (w, g)) in want.iter().zip(got.iter()).enumerate() {
		if (w.w, w.h) != (g.w, g.h) {
			out.push(fmt!("page {}: size typst {}x{}, austenite {}x{}", i + 1, w.w, w.h, g.w, g.h));
			continue;
		}
		let (a, b) = (w.ink(), g.ink());
		let moved = [a.0 - b.0, a.1 - b.1, a.2 - b.2, a.3 - b.3].iter().any(|d| d.abs() > SLACK_PX);
		if moved {
			out.push(fmt!("page {}: ink box typst {:?}, austenite {:?}", i + 1, (a.0, a.1, a.2, a.3), (b.0, b.1, b.2, b.3)));
		}
		let differ = w.data.iter().zip(g.data.iter()).filter(|(p, q)| (**p as i32 - **q as i32).abs() > DIFF).count();
		let allowed = ((a.4 as f64) * FRACTION).max(40.0) as usize;
		if differ > allowed {
			out.push(fmt!("page {}: {} pixel(s) differ (typst has {} ink, {} are allowed)", i + 1, differ, a.4, allowed));
		}
	}
	out
}

#[test]
fn ink_matches_the_typst_oracle_as_rendered() -> Outcome<()> {
	if res!(Oracle::find()).is_none() {
		return Ok(());
	}
	let only = std::env::var("EVAL_INK_FIXTURE").ok().filter(|s| !s.trim().is_empty());
	let root = res!(work_dir("eval-ink"));
	let (mut compared, mut failures) = (0usize, Vec::new());
	let mut files: Vec<PathBuf> = Vec::new();
	let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/ink");
	for entry in res!(std::fs::read_dir(&base)) {
		let p = res!(entry).path();
		if p.extension().map(|e| e == "typ").unwrap_or(false) {
			files.push(p);
		}
	}
	files.sort();
	for path in files {
		let name = path.file_stem().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
		if let Some(o) = &only {
			if !name.contains(o.as_str()) {
				continue;
			}
		}
		let dir = root.join(&name);
		res!(std::fs::create_dir_all(&dir));
		let ours = dir.join("austenite.pdf");
		let theirs = dir.join("typst.pdf");
		let made = match austenite_pdf(&path) {
			Ok(m)	=> m,
			Err(e)	=> {
				failures.push(fmt!("{}: {}", name, e));
				continue;
			},
		};
		res!(std::fs::write(&ours, &made.bytes));
		res!(typst_pdf(&path, &theirs));
		let want = res!(pages(&theirs, &dir, "typst"));
		let got = res!(pages(&ours, &dir, "aus"));
		let d = differences(&want, &got);
		compared += 1;
		if !d.is_empty() {
			failures.push(fmt!("{}: {}", name, d.join(" | ")));
		}
	}
	println!("[ink] {} fixture(s) compared, {} failure(s)", compared, failures.len());
	assert!(failures.is_empty(), "{} failure(s):\n{}", failures.len(), failures.join("\n"));
	if only.is_none() {
		assert!(compared >= MIN_COMPARED, "only {} fixture(s) compared; {} are expected", compared, MIN_COMPARED);
	}
	Ok(())
}
