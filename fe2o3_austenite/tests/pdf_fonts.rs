//! The PDF font-embedding gate: exported text must be real text in real fonts, which a reader can select,
//! copy and search. Each test writes a PDF and hands it to tools that did not write it -- poppler's
//! `pdftotext` and `pdffonts`, and Ghostscript -- so the claim is checked by an outside reader rather
//! than by the writer's own idea of what it wrote.
//!
//! Non-vacuity: every test asserts the font's kind as `pdffonts` reports it (`CID Type 0C` for CFF,
//! `CID TrueType` for `glyf`), so reverting the emitter to outline glyphs -- which still extract, through a
//! Type-3 `/ToUnicode` -- turns them red; [`restricted_face_falls_back_to_outlines`] is the converse, red
//! if a face whose licence forbids embedding is embedded anyway. A tool that is not installed is reported
//! and its test skipped, never passed silently.

use oxedyne_fe2o3_austenite::compile;
use oxedyne_fe2o3_austenite::emit::pdf;
use oxedyne_fe2o3_austenite::font::ShapedText;
use oxedyne_fe2o3_austenite::fonts;
use oxedyne_fe2o3_austenite::ir::Sp;
use oxedyne_fe2o3_austenite::page::{
	Frame,
	Page,
	PageGeometry,
	Placed,
	PlacedKind,
};

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_font::{
	face::Role,
	font::Font,
	shape::Dir,
};

use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;

const DEJAVU: &[u8] = include_bytes!("../../fe2o3_font/fonts/DejaVuSans.ttf");

/// Is `tool` on the path? When it is not, the caller skips with a note rather than passing unexamined.
fn have(tool: &str) -> bool {
	match Command::new(tool).arg("-v").output() {
		Ok(_)	=> true,
		Err(_)	=> {
			eprintln!("SKIP: `{}` is not installed, so this PDF check cannot run.", tool);
			false
		},
	}
}

/// Writes `bytes` to a file named `name` in the test scratch directory and returns its path.
fn write(name: &str, bytes: &[u8]) -> Outcome<PathBuf> {
	let path = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
	res!(std::fs::write(&path, bytes));
	Ok(path)
}

fn run(tool: &str, args: &[&str]) -> Outcome<String> {
	let out = res!(Command::new(tool).args(args).output());
	if !out.status.success() {
		return Err(err!("{} {:?} failed: {}", tool, args, String::from_utf8_lossy(&out.stderr); Test));
	}
	Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// The text poppler extracts from the whole file.
fn extract(path: &PathBuf) -> Outcome<String> {
	run("pdftotext", &[&path.to_string_lossy(), "-"])
}

/// The rows of `pdffonts`, each as (name without subset tag, type, embedded, subset, has ToUnicode).
fn font_rows(path: &PathBuf) -> Outcome<Vec<(String, String, bool, bool, bool)>> {
	let out = res!(run("pdffonts", &[&path.to_string_lossy()]));
	let mut rows = Vec::new();
	for line in out.lines().skip(2) {
		let cols: Vec<&str> = line.split_whitespace().collect();
		// name, type words..., encoding, emb, sub, uni, object, generation
		if cols.len() < 7 {
			continue;
		}
		let n = cols.len();
		let name = cols[0].split('+').last().unwrap_or("").to_string();
		let kind = cols[1..n - 6].join(" ");
		rows.push((name, kind, cols[n - 5] == "yes", cols[n - 4] == "yes", cols[n - 3] == "yes"));
	}
	Ok(rows)
}

/// Ghostscript interprets the whole file and stops on the first error: the nearest thing here to a
/// validator, since it must parse every font program to render the pages.
fn gs_clean(path: &PathBuf) -> Outcome<()> {
	if !have("gs") {
		return Ok(());
	}
	let out = res!(Command::new("gs")
		.args(["-q", "-dNOPAUSE", "-dBATCH", "-dPDFSTOPONERROR", "-sDEVICE=nullpage"])
		.arg(path)
		.output());
	let noise = fmt!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
	assert!(out.status.success() && noise.trim().is_empty(),
		"Ghostscript reports the PDF as faulty: {}", noise);
	Ok(())
}

/// Compiles a sample through the real assemble/author/run path to one PDF.
fn sample_pdf(name: &str) -> Outcome<Vec<u8>> {
	let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("samples").join(fmt!("{}.typ", name));
	let (assembled, _refusals, _skip) = res!(compile::assemble(
		&path,
		|| Ok(Arc::new(res!(fonts::libertinus()))),
	));
	let rendered = res!(compile::author_and_run(assembled));
	pdf::render_document(&rendered.out.pages)
}

/// One A4 page carrying each `(font, text)` as a line of its own.
fn lines_pdf(lines: Vec<(Arc<Font>, &str)>) -> Outcome<Vec<u8>> {
	let mut frame = Frame::new();
	for (k, (font, text)) in lines.into_iter().enumerate() {
		let shaped = res!(ShapedText::new_with_font(font, Dir::Ltr, Sp::from_pt(12.0), text));
		frame.push(Placed::new(
			Sp::from_pt(60.0), Sp::from_pt(80.0 + 24.0 * k as f64), shaped.dims(), PlacedKind::Text(shaped)));
	}
	pdf::render_document(&[Page::new(1, PageGeometry::a4(), frame)])
}

/// Collapses runs of whitespace, so a comparison is about the words and not poppler's layout spacing.
fn words(s: &str) -> String {
	s.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[test]
fn a_sample_exports_selectable_text_in_embedded_subset_fonts() -> Outcome<()> {
	if !have("pdftotext") || !have("pdffonts") {
		return Ok(());
	}
	let path = res!(write("hierarchy.pdf", &res!(sample_pdf("hierarchy"))));
	res!(gs_clean(&path));

	let text = words(&res!(extract(&path)));
	for want in [
		"Field Notes on Structure",
		"papyrus was brittle and cracked when folded;",
		"ret the raw fibre until the cellulose loosens;",	// "fi" is one ligature glyph
	] {
		assert!(text.contains(want), "poppler did not extract {:?} from:\n{}", want, text);
	}

	let rows = res!(font_rows(&path));
	assert!(!rows.is_empty(), "pdffonts lists no font at all");
	for (name, kind, emb, sub, uni) in &rows {
		assert_eq!(kind, "CID Type 0C", "{} is not an embedded CFF CIDFont", name);
		assert!(*emb && *sub && *uni, "{} is not embedded, subset and mapped to Unicode", name);
	}
	for face in ["LibertinusSerif-Regular", "LibertinusSerif-Bold", "LibertinusSerif-Italic"] {
		assert!(rows.iter().any(|r| r.0 == face), "{} is missing from {:?}", face, rows);
	}
	Ok(())
}

#[test]
fn maths_embeds_its_own_font_and_extracts_its_symbols() -> Outcome<()> {
	if !have("pdftotext") || !have("pdffonts") {
		return Ok(());
	}
	let path = res!(write("maths.pdf", &res!(sample_pdf("maths"))));
	res!(gs_clean(&path));
	let rows = res!(font_rows(&path));
	assert!(rows.iter().any(|r| r.0 == "NewCMMath-Regular" && r.1 == "CID Type 0C" && r.2 && r.4),
		"the maths font is not embedded with a ToUnicode: {:?}", rows);
	let text = res!(extract(&path));
	// A mathematical italic a (U+1D44E) is a character outside the Basic Multilingual Plane, so this also
	// exercises a surrogate pair in the CMap.
	assert!(text.contains("\u{1D44E}"), "the maths italic a did not extract:\n{}", text);
	assert!(text.contains("Pythagoras"), "the prose around the maths did not extract:\n{}", text);
	Ok(())
}

#[test]
fn ligatures_and_a_truetype_face_round_trip_through_copy() -> Outcome<()> {
	if !have("pdftotext") || !have("pdffonts") {
		return Ok(());
	}
	let serif	= Arc::new(res!(Font::new(res!(std::fs::read(
		PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fonts").join("LibertinusSerif-Regular.otf"))))));
	let dejavu	= Arc::new(res!(Font::new(DEJAVU.to_vec())));

	// Libertinus sets "ffi" and "fl" as single ligature glyphs; the CMap must expand each back to its
	// letters, or a search for "office" misses the word.
	let lig = res!(ShapedText::new_with_font(serif.clone(), Dir::Ltr, Sp::from_pt(12.0), "affluent office"));
	assert!(lig.run().glyphs.len() < "affluent office".chars().count(),
		"no ligature formed, so this test would prove nothing about ligature mapping");

	let path = res!(write("lines.pdf", &res!(lines_pdf(vec![
		(serif, "affluent office"),
		(dejavu, "Grüße, Ωμέγα — naïve"),
	]))));
	res!(gs_clean(&path));
	let text = words(&res!(extract(&path)));
	assert!(text.contains("affluent office"), "the ligatures did not extract as letters: {:?}", text);
	assert!(text.contains("Grüße, Ωμέγα — naïve"), "the TrueType line did not extract: {:?}", text);

	let rows = res!(font_rows(&path));
	assert!(rows.iter().any(|r| r.0 == "DejaVuSans" && r.1 == "CID TrueType" && r.2 && r.3 && r.4),
		"DejaVu Sans is not an embedded TrueType subset: {:?}", rows);
	assert!(rows.iter().any(|r| r.0 == "LibertinusSerif-Regular" && r.1 == "CID Type 0C"),
		"Libertinus is not an embedded CFF: {:?}", rows);
	Ok(())
}

#[test]
fn restricted_face_falls_back_to_outlines() -> Outcome<()> {
	if !have("pdftotext") || !have("pdffonts") {
		return Ok(());
	}
	// DejaVu with its OS/2 fsType set to 2, a restricted licence: the face must not be embedded, so its
	// glyphs are drawn as Type-3 outlines -- and still extract, through that font's own ToUnicode.
	let mut bytes = DEJAVU.to_vec();
	let n = u16::from_be_bytes([bytes[4], bytes[5]]) as usize;
	let mut patched = false;
	for i in 0..n {
		let rec = 12 + 16 * i;
		if &bytes[rec..rec + 4] == b"OS/2" {
			let off = u32::from_be_bytes([bytes[rec + 8], bytes[rec + 9], bytes[rec + 10], bytes[rec + 11]]) as usize;
			bytes[off + 8] = 0;
			bytes[off + 9] = 2;
			patched = true;
		}
	}
	assert!(patched, "DejaVu has no OS/2 table to restrict");
	let font = Arc::new(res!(Font::new(bytes)));
	let path = res!(write("restricted.pdf", &res!(lines_pdf(vec![(font, "Restricted licence")]))));
	res!(gs_clean(&path));
	let rows = res!(font_rows(&path));
	assert!(rows.iter().all(|r| r.1 == "Type 3"), "a restricted face was embedded: {:?}", rows);
	assert!(words(&res!(extract(&path))).contains("Restricted licence"), "the fallback text did not extract");
	Ok(())
}

#[test]
fn embedded_output_is_deterministic() -> Outcome<()> {
	let a = res!(sample_pdf("maths"));
	let b = res!(sample_pdf("maths"));
	assert!(a == b, "two compiles of one sample differ");
	// The streaming writer the binary uses must agree with the buffered one to the byte.
	let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("samples").join("maths.typ");
	let (assembled, _refusals, _skip) = res!(compile::assemble(
		&path,
		|| Ok(Arc::new(res!(fonts::libertinus()))),
	));
	let rendered = res!(compile::author_and_run(assembled));
	let mut stream = res!(pdf::open_document(Vec::new(), rendered.out.pages.len()));
	for page in &rendered.out.pages {
		res!(pdf::write_page(&mut stream, page));
	}
	let c = res!(stream.finish());
	assert!(a == c, "the streamed file differs from the buffered one");
	Ok(())
}

#[test]
fn role_faces_share_one_embedded_font_per_file() -> Outcome<()> {
	if !have("pdffonts") {
		return Ok(());
	}
	// Two independently loaded sets of the same files are one font per file in the PDF, not two.
	let a = Arc::new(res!(fonts::libertinus()));
	let b = Arc::new(res!(fonts::libertinus()));
	let mut frame = Frame::new();
	for (k, set) in [a, b].into_iter().enumerate() {
		let s = res!(ShapedText::new(set, Role::Body, Dir::Ltr, Sp::from_pt(11.0), "Shared face"));
		frame.push(Placed::new(Sp::from_pt(60.0), Sp::from_pt(80.0 + 20.0 * k as f64), s.dims(), PlacedKind::Text(s)));
	}
	let path = res!(write("shared.pdf", &res!(pdf::render_document(&[Page::new(1, PageGeometry::a4(), frame)]))));
	let rows = res!(font_rows(&path));
	assert_eq!(rows.len(), 1, "one file loaded twice became {} fonts: {:?}", rows.len(), rows);
	assert_eq!(rows[0].1, "CID Type 0C", "the shared face is not embedded: {:?}", rows);
	Ok(())
}
