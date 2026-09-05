//! `ingot` -- compile an Ingot source file to a set page.
//!
//! Reads a `.ingot` manuscript, parses and lowers it through the [`lang`](oxedyne_fe2o3_austenite::lang)
//! front end to a block list, authors that through the block layer, runs the two-pass driver to a
//! fixed point, decorates each page with a running head and a folio, and writes every page as SVG
//! alongside the resolved ledger and a single PDF of the whole run.
//!
//! Increment 1 sets the markup spine faithfully: what the source says, and no more. There is no
//! table of contents, because the language has no `#outline()` yet.
//!
//! Usage: `ingot <SOURCE.ingot> [OUTPUT_DIR]` (default output `ingot-out`).

use oxedyne_fe2o3_austenite::{
	book,
	doc::{
		self,
		Style,
	},
	driver::{
		self,
		Config,
	},
	emit::{
		self,
		Emitter,
	},
	font::FontMetrics,
	lang,
	page::{
		Frame,
		PageGeometry,
	},
};

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_font::{
	face::Role,
	shape::Dir,
};

use std::fs::File;
use std::io::BufWriter;
use std::sync::Arc;

fn main() -> Outcome<()> {
	let args: Vec<String> = std::env::args().collect();
	let source = match args.get(1) {
		Some(s)	=> s.clone(),
		None	=> return Err(err!(
			"Usage: ingot <SOURCE.ingot> [OUTPUT_DIR]"; Input, Invalid, Missing)),
	};
	let out_dir = match args.get(2) {
		Some(s)	=> s.clone(),
		None	=> "ingot-out".to_string(),
	};

	let src = match std::fs::read_to_string(&source) {
		Ok(s)	=> s,
		Err(e)	=> return Err(err!(e,
			"Could not read the Ingot source file {:?}.", source; File, Read)),
	};

	// A book root assembles chapters and carries its own geometry, fonts and type; a lone file sets on
	// A4 with the embedded Libertinus, as before. The block stream, geometry, style and faces come from
	// one place or the other, and the rest of the run is identical.
	let (blocks, fonts, geom, style, title) = if book::is_book_root(&src) {
		let spec = res!(book::load(std::path::Path::new(&source)));
		(spec.blocks, spec.fonts, spec.geom, spec.style, spec.title)
	} else {
		let blocks	= res!(lang::to_blocks(&src));
		let fonts	= Arc::new(res!(oxedyne_fe2o3_austenite::fonts::libertinus()));
		(blocks, fonts, PageGeometry::a4(), Style::default(), String::new())
	};

	let (document, heads)	= res!(doc::author(fonts.clone(), geom, style, &blocks));
	let metrics				= FontMetrics::new(fonts.clone(), Role::Body, Dir::Ltr, style.body_size);
	let mut out				= res!(driver::run(&document, &metrics, Config::default()));
	res!(doc::decorate(&mut out.pages, &out.ledger, &heads, &fonts, style, geom, &title));

	// Mirror the margins: the driver laid every page at the recto split (binding on the left). A verso
	// page -- an even folio -- is that whole frame shifted to the fore-edge, so the binding margin sits
	// at the spine on both sides of the leaf. Uniform margins give a zero shift, so a non-book run is
	// untouched.
	let shift = geom.mirror_shift();
	if shift.raw() != 0 {
		for page in &mut out.pages {
			if page.number % 2 == 0 {
				for placed in &mut page.frame.placed {
					placed.x = placed.x + shift;
				}
			}
		}
	}

	res!(std::fs::create_dir_all(&out_dir));

	// The ledger is small and independent of the pages, so it is written first and out of the way.
	let ledger_path = fmt!("{}/ledger.jdat", out_dir);
	res!(out.ledger.to_file(&ledger_path));

	// Emit each page and drop its frame before the next. Both writers are streaming: the SVG is one file
	// per page, and the PDF is written object by object into the file as each page is composed, never
	// accumulated. Holding one page's glyph outlines at a time -- rather than every page's at once, as a
	// buffered whole-document PDF would -- is what keeps a whole-book compile flat in memory. The bytes
	// are identical to the buffered path; only the peak memory differs.
	let emitter		= Emitter::Svg;
	let pdf_file	= res!(File::create(fmt!("{}/document.pdf", out_dir)));
	let mut pdf		= res!(emit::pdf::open_document(BufWriter::new(pdf_file), out.pages.len()));
	for page in &mut out.pages {
		let svg		= res!(emitter.render(page));
		let path	= fmt!("{}/page-{:03}.{}", out_dir, page.number, emitter.extension());
		res!(std::fs::write(&path, svg));

		res!(emit::pdf::write_page(&mut pdf, page));

		// The page is written; free its frame so the next page's outlines are the only ones live.
		page.frame = Frame::new();
	}
	res!(pdf.finish());

	println!(
		"ingot: {} -> {} page(s) in {} pass(es); {} anchor(s) in the ledger; written to {}/",
		source, out.pages.len(), out.passes, out.ledger.len(), out_dir);
	Ok(())
}
