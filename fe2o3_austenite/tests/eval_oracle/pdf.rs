// Austenite's own PDF against Typst's (level 4b, the U12b comparator). A source is compiled through the
// evaluator's pipeline with the streaming `PdfSink`, and the file written is read back by the same
// `pdftotext -bbox-layout` that reads Typst's, so the face-dependent offset of a box from its baseline is
// common to both and only the document's own layout is compared.

#![allow(dead_code)]

use crate::harness::layout::{
	read_bbox,
	OLine,
	OPage,
};

use oxedyne_fe2o3_austenite::compile;
use oxedyne_fe2o3_austenite::emit::sinks::PdfSink;
use oxedyne_fe2o3_austenite::flow::text::FontStore;

use oxedyne_fe2o3_core::prelude::*;

use std::path::{
	Path,
	PathBuf,
};
use std::process::Command;

pub const TYPST:	&str = "/home/jason/bin/typst";
pub const TOL:		f64 = 1.0;	// points: level 4b's tolerance

/// A scratch directory of the test's own, under the QC cache, never `/tmp`.
pub fn work_dir(name: &str) -> Outcome<PathBuf> {
	let home = res!(std::env::var("HOME"));
	let dir = Path::new(&home).join(".cache").join("austenite-qc").join(name);
	res!(std::fs::create_dir_all(&dir));
	Ok(dir)
}

/// What a compile through the pipeline left: the PDF's bytes, the report, and the page count.
pub struct Made {
	pub bytes:	Vec<u8>,
	pub report:	compile::Report,
	pub pages:	u32,
}

/// Compiles `src` (its directory the root) to a PDF through `compile::assemble_eval` with a `PdfSink`.
pub fn austenite_pdf(src: &Path) -> Outcome<Made> {
	let mut sink = res!(PdfSink::new());
	let root = src.parent().map(|p| p.to_path_buf()).unwrap_or_default();
	let done = res!(compile::assemble_eval(src, &root, FontStore::default(), &mut sink));
	let pages = res!(done.laid.as_ref().map_err(|e| err!("{} did not lay out: {}", src.display(), e.plain(); Test))).pages;
	let report = done.report();
	let out = res!(sink.output().ok_or_else(|| err!("The fixpoint finished with no PDF."; Test)));
	Ok(Made { bytes: out.to_vec(), report, pages })
}

/// Typst's PDF of `src`, under the same memory cap as every oracle run.
pub fn typst_pdf(src: &Path, out: &Path) -> Outcome<()> {
	let status = Command::new("systemd-run")
		.args(["--user", "--scope", "--quiet", "-p", "MemoryMax=3G", "--slice=claude-rc.slice", TYPST, "compile"])
		.arg(src).arg(out)
		.status();
	match status {
		Ok(s) if s.success()	=> Ok(()),
		other					=> Err(err!("typst did not compile {}: {:?}", src.display(), other; Test)),
	}
}

pub fn tool(name: &str, args: &[&str], file: &Path, tail: &[&str]) -> Outcome<String> {
	let out = Command::new(name).args(args).arg(file).args(tail).output();
	match out {
		Ok(o) if o.status.success()	=> Ok(String::from_utf8_lossy(&o.stdout).to_string()),
		other						=> Err(err!("{} failed on {}: {:?}", name, file.display(), other.map(|o| o.status); Test)),
	}
}

/// The PDF's text, by `pdftotext -layout`, whitespace squashed.
pub fn text_of(pdf: &Path) -> Outcome<String> {
	let t = res!(tool("pdftotext", &["-layout"], pdf, &["-"]));
	Ok(t.split_whitespace().collect::<Vec<_>>().join(" "))
}

/// The PDF's pages and lines, by `pdftotext -bbox-layout`.
pub fn lines_of(pdf: &Path) -> Outcome<Vec<OPage>> {
	let xml = res!(tool("pdftotext", &["-bbox-layout"], pdf, &["-"]));
	Ok(read_bbox(&xml))
}

/// The page size `pdfinfo` reads from a PDF, in points: Typst packs its page objects into compressed streams,
/// so its MediaBox is not in the bytes.
pub fn page_size(pdf: &Path) -> Outcome<(f64, f64)> {
	let info = res!(tool("pdfinfo", &[], pdf, &[]));
	let line = res!(info.lines().find(|l| l.starts_with("Page size:")).ok_or_else(|| err!("pdfinfo gave no page size"; Test)));
	let nums: Vec<f64> = line.split_whitespace().filter_map(|n| n.parse::<f64>().ok()).collect();
	if nums.len() < 2 {
		return Err(err!("pdfinfo's page size line has no two numbers: {}", line; Test));
	}
	Ok((nums[0], nums[1]))
}

/// Where Austenite's lines differ from Typst's: a line's text, the bottom of its box and where its first
/// word starts, each within [`TOL`].
pub fn k3_differences(want: &[OPage], got: &[OPage]) -> Vec<String> {
	let mut out = Vec::new();
	if want.len() != got.len() {
		out.push(fmt!("page count: typst {}, austenite {}", want.len(), got.len()));
	}
	for (pi, (w, g)) in want.iter().zip(got.iter()).enumerate() {
		if w.lines.len() != g.lines.len() {
			out.push(fmt!("page {}: typst has {} line(s), austenite {}", pi + 1, w.lines.len(), g.lines.len()));
		}
		for (wl, gl) in w.lines.iter().zip(g.lines.iter()) {
			let squash = |l: &OLine| l.text.split_whitespace().collect::<Vec<_>>().join(" ");
			if squash(wl) != squash(gl) {
				out.push(fmt!("page {}: line text typst {:?}, austenite {:?}", pi + 1, squash(wl), squash(gl)));
			}
			if (wl.y1 - gl.y1).abs() > TOL {
				out.push(fmt!("page {}: {:?}: box bottom typst {:.2}, austenite {:.2}", pi + 1, squash(wl), wl.y1, gl.y1));
			}
			if (wl.x0 - gl.x0).abs() > TOL {
				out.push(fmt!("page {}: {:?}: starts at x typst {:.2}, austenite {:.2}", pi + 1, squash(wl), wl.x0, gl.x0));
			}
		}
	}
	out
}

/// Compiles `src` both ways into `dir` and returns the level-4b differences of Austenite's own PDF from Typst's.
pub fn pdf_differences(src: &Path, dir: &Path) -> Outcome<Vec<String>> {
	res!(std::fs::create_dir_all(dir));
	let made	= res!(austenite_pdf(src));
	let ours	= dir.join("austenite.pdf");
	let theirs	= dir.join("typst.pdf");
	res!(std::fs::write(&ours, &made.bytes));
	res!(typst_pdf(src, &theirs));
	let mut out = k3_differences(&res!(lines_of(&theirs)), &res!(lines_of(&ours)));
	let (a, t) = (res!(text_of(&ours)), res!(text_of(&theirs)));
	if a != t {
		out.push(fmt!("pdftotext differs: typst {:?}, austenite {:?}", t, a));
	}
	Ok(out)
}
