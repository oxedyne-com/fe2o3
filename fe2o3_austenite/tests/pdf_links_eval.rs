//! The link annotations of a PDF written through the evaluator are Typst's: a link around text covers the
//! text, and a link around a block-level element, an image or a block, covers the element's whole frame, as
//! Typst's frame modifiers make it. The expectation is Typst's own PDF of the same source, read back with
//! the same reader.

#![allow(dead_code)]

#[path = "eval_oracle/mod.rs"]
mod harness;

use harness::oracle::Oracle;
use harness::pdf::{
	austenite_pdf,
	typst_pdf,
	work_dir,
};

use oxedyne_fe2o3_core::prelude::*;

use std::path::Path;
use std::process::Command;

// A plain square badge, so the image case needs no file but this one.
const BADGE: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 300 160" width="300" height="160"><rect x="1.5" y="1.5" width="297" height="157" rx="18" fill="#fff" stroke="#000" stroke-width="3"/></svg>"##;

// The badge in the shape a template puts it, centred in a block under a show rule for links; a linked
// filled block; and a text link, which already worked, to hold the comparison to all three.
const SOURCE: &str = r##"#set page(width: 300pt, height: 300pt, margin: 20pt)
Text link: #link("https://example.com/text")[example text].

#align(center)[#block[
#show link: it => it
#link("https://need2know.ai/no-ai/doc")[#image("badge.svg", height: 1.4cm)]
]]

#link("https://example.com/block")[#block(width: 36pt, height: 36pt, fill: black)]
"##;

// Every link annotation's URI and rectangle as a JSON list, read by pikepdf; None where the tool is not here.
fn links(pdf: &Path) -> Option<Vec<(String, [f64; 4])>> {
	let script = "import sys, pikepdf\n\
		pdf = pikepdf.open(sys.argv[1])\n\
		for p in pdf.pages:\n\
		\tfor a in p.obj.get('/Annots', []):\n\
		\t\tif a.get('/Subtype') == '/Link' and '/A' in a and '/URI' in a.A:\n\
		\t\t\tr = [float(x) for x in a.Rect]\n\
		\t\t\tprint(str(a.A.URI), min(r[0], r[2]), min(r[1], r[3]), max(r[0], r[2]), max(r[1], r[3]))\n";
	let out = match Command::new("python3").arg("-c").arg(script).arg(pdf).output() {
		Ok(o) if o.status.success()	=> String::from_utf8_lossy(&o.stdout).to_string(),
		_							=> return None,
	};
	let mut v = Vec::new();
	for line in out.lines() {
		let f: Vec<&str> = line.split_whitespace().collect();
		if f.len() != 5 {
			continue;
		}
		let mut r = [0.0; 4];
		for i in 0..4 {
			r[i] = f[i + 1].parse().unwrap_or(f64::NAN);
		}
		v.push((f[0].to_string(), r));
	}
	v.sort_by(|a, b| a.0.cmp(&b.0));
	Some(v)
}

#[test]
fn a_link_around_an_image_or_a_block_covers_its_whole_frame_as_typst_does() -> Outcome<()> {
	if res!(Oracle::find()).is_none() {
		return Ok(());
	}
	let dir = res!(work_dir("pdf-links-eval"));
	res!(std::fs::write(dir.join("badge.svg"), BADGE));
	let src = dir.join("main.typ");
	res!(std::fs::write(&src, SOURCE));
	let typst_out = dir.join("typst.pdf");
	res!(typst_pdf(&src, &typst_out));
	let made = res!(austenite_pdf(&src));
	let ours = dir.join("austenite.pdf");
	res!(std::fs::write(&ours, &made.bytes));
	let (want, got) = match (links(&typst_out), links(&ours)) {
		(Some(w), Some(g))	=> (w, g),
		_					=> {
			println!("[pdf-links] SKIPPED: python3 with pikepdf is not here");
			return Ok(());
		}
	};
	let uris = |v: &[(String, [f64; 4])]| v.iter().map(|(u, _)| u.clone()).collect::<Vec<_>>();
	// Fixed by construction, so a comparison of two empty lists cannot pass.
	assert_eq!(uris(&want), vec![
		"https://example.com/block".to_string(),
		"https://example.com/text".to_string(),
		"https://need2know.ai/no-ai/doc".to_string(),
	], "Typst's links: {:?}", want);
	assert_eq!(uris(&got), uris(&want), "the links differ from Typst's: {:?}", got);
	for ((u, w), (_, g)) in want.iter().zip(got.iter()) {
		if u.contains("/text") {
			continue;	// a text link's height follows the leading, compared elsewhere
		}
		for i in 0..4 {
			assert!((w[i] - g[i]).abs() <= 0.5, "{}: rect {:?}, Typst's {:?}", u, g, w);
		}
	}
	Ok(())
}

// A contents page, two labelled headings on pages of their own, a link to a label and two references.
const INTERNAL: &str = r##"#set page(width: 300pt, height: 300pt, margin: 20pt)
#set heading(numbering: "1.")
#outline()
#pagebreak()
= Alpha <a>
See #link(<b>)[the second] and @b.
#pagebreak()
= Beta <b>
Back to @a.
"##;

// Every internal link annotation as (page it sits on, page it goes to, destination top), its destination
// read wherever a writer may put it: an explicit `/Dest` array, a GoTo action, or a name looked up in the
// catalogue's `/Dests` name tree or dictionary. None where python3 with pikepdf is not here.
fn internal_links(pdf: &Path) -> Option<Vec<(usize, usize, f64)>> {
	let script = "import sys, pikepdf\n\
		pdf = pikepdf.open(sys.argv[1])\n\
		idx = {p.obj.objgen: i for i, p in enumerate(pdf.pages)}\n\
		names = {}\n\
		r = pdf.Root\n\
		if '/Names' in r and '/Dests' in r.Names:\n\
		\tfor k, v in pikepdf.NameTree(r.Names.Dests).items(): names[str(k)] = v\n\
		if '/Dests' in r:\n\
		\tfor k, v in r.Dests.items(): names[str(k).lstrip('/')] = v\n\
		for i, p in enumerate(pdf.pages):\n\
		\tfor a in p.obj.get('/Annots', []):\n\
		\t\tif a.get('/Subtype') != '/Link': continue\n\
		\t\td = a.get('/Dest')\n\
		\t\tif d is None and '/A' in a and a.A.get('/S') == '/GoTo': d = a.A.D\n\
		\t\tif d is None: continue\n\
		\t\tif not isinstance(d, pikepdf.Array):\n\
		\t\t\td = names.get(str(d).lstrip('/'))\n\
		\t\t\tif isinstance(d, pikepdf.Dictionary): d = d.D\n\
		\t\tif d is None: print(i, -1, -1); continue\n\
		\t\ty = float(d[3]) if len(d) > 3 and d[1] == '/XYZ' else -1\n\
		\t\tprint(i, idx.get(d[0].objgen, -1), y)\n";
	let out = match Command::new("python3").arg("-I").arg("-c").arg(script).arg(pdf).output() {
		Ok(o) if o.status.success()	=> String::from_utf8_lossy(&o.stdout).to_string(),
		_							=> return None,
	};
	let mut v = Vec::new();
	for line in out.lines() {
		let f: Vec<&str> = line.split_whitespace().collect();
		if f.len() != 3 {
			continue;
		}
		let n = |s: &str| s.parse::<f64>().unwrap_or(f64::NAN);
		v.push((n(f[0]) as usize, n(f[1]) as usize, n(f[2])));
	}
	v.sort_by(|a, b| (a.0, a.1).cmp(&(b.0, b.1)));
	Some(v)
}

#[test]
fn contents_entries_and_references_link_inside_the_document_as_typst_does() -> Outcome<()> {
	if res!(Oracle::find()).is_none() {
		return Ok(());
	}
	let dir = res!(work_dir("pdf-links-internal"));
	let src = dir.join("main.typ");
	res!(std::fs::write(&src, INTERNAL));
	let typst_out = dir.join("typst.pdf");
	res!(typst_pdf(&src, &typst_out));
	let made = res!(austenite_pdf(&src));
	let ours = dir.join("austenite.pdf");
	res!(std::fs::write(&ours, &made.bytes));
	let (want, got) = match (internal_links(&typst_out), internal_links(&ours)) {
		(Some(w), Some(g))	=> (w, g),
		_					=> {
			println!("[pdf-links] SKIPPED: python3 with pikepdf is not here");
			return Ok(());
		}
	};
	let pages = |v: &[(usize, usize, f64)]| v.iter().map(|(a, b, _)| (*a, *b)).collect::<Vec<_>>();
	// Fixed by construction: two contents entries on the first page, the link and the reference to Beta
	// on the second, the reference back to Alpha on the third.
	assert_eq!(pages(&want), vec![(0, 1), (0, 2), (1, 2), (1, 2), (2, 1)], "Typst's links: {:?}", want);
	assert_eq!(pages(&got), pages(&want), "the internal links differ from Typst's: {:?}", got);
	for ((_, _, w), (_, _, g)) in want.iter().zip(got.iter()) {
		assert!((w - g).abs() <= 12.0, "a destination's top {} is not Typst's {}: {:?}", g, w, got);
	}
	Ok(())
}
