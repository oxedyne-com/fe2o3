//! The no-regression trio: three fixed fixtures the oracle harness also checks on every run, each
//! guarding a real fix landed on `main` shortly before this unit -- the inline maths gallery, the
//! display-equation cases (aligned `&=`, a big-operator's under/over limits, `ceil`/`floor`/`binom`),
//! and the compact, searchable PDF writer. None of the three needs the Typst oracle or the corpus: they
//! are smoke-level guards against the engine itself regressing, run from the crate's own public API
//! exactly as `mathgallery` and `austenite` call it.

use oxedyne_fe2o3_austenite::{
	doc::{
		self,
	},
	fonts::FaceResolver,
	theme::Theme,
	driver::{
		self,
		Config,
	},
	emit,
	font::FontMetrics,
	lang,
	page::PageGeometry,
};

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_font::{
	face::Role,
	shape::Dir,
};

use std::path::Path;
use std::process::Command;
use std::sync::Arc;

/// Runs the `mathgallery` binary (the living maths-layout comparison harness, `src/bin/mathgallery.rs`)
/// into `work_dir` and checks it exited cleanly and left at least one non-empty equation PDF -- a smoke
/// guard against the inline maths pipeline (parse, layout, PDF emit) regressing outright, not a
/// pixel check, which is what the gallery itself exists for a person to do by eye.
pub fn inline_maths_gallery_renders(work_dir: &Path) -> Outcome<()> {
	let out_dir = work_dir.join("trio-mathgallery");
	let bin		= env!("CARGO_BIN_EXE_mathgallery");
	let output = match Command::new(bin).arg(&out_dir).output() {
		Ok(o)	=> o,
		Err(e)	=> return Err(err!(e, "Could not run the built mathgallery binary at {:?}.", bin; IO)),
	};
	if !output.status.success() {
		return Err(err!("mathgallery exited with {}:\n{}",
			output.status, String::from_utf8_lossy(&output.stderr); Invalid, Unexpected));
	}
	let entries = match std::fs::read_dir(&out_dir) {
		Ok(e)	=> e,
		Err(e)	=> return Err(err!(e, "Could not list {:?}.", out_dir; File, Read)),
	};
	let mut pdf_count = 0usize;
	for entry in entries.flatten() {
		let p = entry.path();
		if p.extension().and_then(|e| e.to_str()) == Some("pdf") {
			let len = res!(std::fs::metadata(&p)).len();
			if len == 0 {
				return Err(err!("mathgallery wrote an empty PDF at {:?}.", p; Invalid, Size));
			}
			pdf_count += 1;
		}
	}
	if pdf_count == 0 {
		return Err(err!("mathgallery wrote no equation PDFs into {:?}.", out_dir; Missing, Invalid));
	}
	Ok(())
}

/// Authors, lays out and renders one small document carrying the display-equation shapes now on
/// `main` -- a multi-line `&`-aligned block (`87ca15c`/`8893ff6` fixed exactly this being stolen line
/// by line before it reached the maths parser), a big operator's `_`/`^` limits, and `ceil`, `floor`
/// and `binom` (`59c6e3b`) -- through the real reader-to-PDF pipeline, checking only that the pipeline
/// completes and the result is a genuine one-page PDF: the geometry of each shape is `mathgallery`'s
/// and the oracle's job, this fixture's is to trip the moment any of the four stops setting at all.
pub fn display_equation_cases_render() -> Outcome<()> {
	let src = r#"= Display Equations Regression

$ a &= b + c \
&= d $ <eq_aligned>

$ sum_(i=1)^n a_i $ <eq_bigop_limits>

$ binom(n, k) + ceil(x) + floor(y) $ <eq_binom_ceil_floor>
"#;
	let blocks	= res!(lang::to_blocks(src));
	let fonts	= Arc::new(res!(oxedyne_fe2o3_austenite::fonts::libertinus()));
	let geom	= PageGeometry::a4();
	let style	= Theme::default();
	let (document, _heads) = res!(doc::author(fonts.clone(), geom, &style, &FaceResolver::default(), &blocks, None, None));
	let metrics	= FontMetrics::new(fonts.clone(), Role::Body, Dir::Ltr, style.text.body_size);
	let out		= res!(driver::run(&document, &metrics, Config::default()));
	if out.pages.is_empty() {
		return Err(err!("The display-equation regression doc rendered no pages."; Invalid, Missing));
	}
	let pdf = res!(emit::pdf::render_document(&out.pages));
	if pdf.len() < 200 {
		return Err(err!(
			"The display-equation regression PDF is only {} byte(s) -- too small to hold real content.",
			pdf.len(); Invalid, Size));
	}
	Ok(())
}

/// Renders a one-paragraph prose document and checks the compact/searchable PDF writer's two
/// invariants at once: the page's text comes back out through `pdftotext` (the Type-3-plus-ToUnicode
/// glyph encoding stays searchable), and the file stays well under a generous bound (compaction has not
/// regressed into bloat). The bound is generous on purpose -- this guards against a large regression,
/// not a byte-for-byte size lock a legitimate change would have to keep re-tuning.
pub fn pdf_stays_compact_and_extractable(work_dir: &Path) -> Outcome<()> {
	const NEEDLE:		&str	= "quick brown fox";
	const SIZE_BOUND:	u64		= 300_000;	// bytes; one page of one paragraph of body text

	let src = fmt!("= Regression Prose\n\nThe {} jumps over the lazy dog while the compact PDF writer \
		keeps its content stream small and its text extractable.\n", NEEDLE);
	let blocks	= res!(lang::to_blocks(&src));
	let fonts	= Arc::new(res!(oxedyne_fe2o3_austenite::fonts::libertinus()));
	let geom	= PageGeometry::a4();
	let style	= Theme::default();
	let (document, _heads) = res!(doc::author(fonts.clone(), geom, &style, &FaceResolver::default(), &blocks, None, None));
	let metrics	= FontMetrics::new(fonts.clone(), Role::Body, Dir::Ltr, style.text.body_size);
	let out		= res!(driver::run(&document, &metrics, Config::default()));
	let pdf		= res!(emit::pdf::render_document(&out.pages));

	let pdf_path = work_dir.join("trio-pdf-compact.pdf");
	res!(std::fs::write(&pdf_path, &pdf));

	let len = pdf.len() as u64;
	if len > SIZE_BOUND {
		return Err(err!(
			"The compact-PDF fixture wrote {} byte(s), over the {} byte bound -- the writer may have \
			regressed out of compaction.", len, SIZE_BOUND; Invalid, TooBig));
	}

	let output = match Command::new("pdftotext").arg(&pdf_path).arg("-").output() {
		Ok(o)	=> o,
		Err(e)	=> return Err(err!(e, "Could not run `pdftotext`."; IO)),
	};
	if !output.status.success() {
		return Err(err!("pdftotext on {:?} exited with {}.", pdf_path, output.status; Invalid, Unexpected));
	}
	let text = String::from_utf8_lossy(&output.stdout);
	if !text.contains(NEEDLE) {
		return Err(err!(
			"pdftotext on {:?} did not recover {:?}; extracted: {:?}", pdf_path, NEEDLE, text;
			Invalid, Mismatch));
	}
	Ok(())
}
