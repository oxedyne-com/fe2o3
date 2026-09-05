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
	doc::{
		self,
		Style,
	},
	driver::{
		self,
		Config,
	},
	emit::Emitter,
	font::FontMetrics,
	lang,
	page::PageGeometry,
};

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_font::{
	face::Role,
	set::FontSet,
	shape::Dir,
};

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
	let blocks = res!(lang::to_blocks(&src));

	let fonts	= Arc::new(res!(FontSet::embedded()));
	let geom	= PageGeometry::a4();
	let style	= Style::default();

	let (document, heads)	= res!(doc::author(fonts.clone(), geom, style, &blocks));
	let metrics				= FontMetrics::new(fonts.clone(), Role::Body, Dir::Ltr, style.body_size);
	let mut out				= res!(driver::run(&document, &metrics, Config::default()));
	res!(doc::decorate(&mut out.pages, &out.ledger, &heads, &fonts, style, geom));

	res!(std::fs::create_dir_all(&out_dir));
	let emitter = Emitter::Svg;
	for page in &out.pages {
		let svg		= res!(emitter.render(page));
		let path	= fmt!("{}/page-{:03}.{}", out_dir, page.number, emitter.extension());
		res!(std::fs::write(&path, svg));
	}

	let ledger_path = fmt!("{}/ledger.jdat", out_dir);
	res!(out.ledger.to_file(&ledger_path));

	// The whole run as one PDF beside the per-page SVGs: the same placed frames and glyph outlines,
	// emitted as fill operators rather than <path> elements.
	let pdf = res!(oxedyne_fe2o3_austenite::emit::pdf::render_document(&out.pages));
	res!(std::fs::write(fmt!("{}/document.pdf", out_dir), pdf));

	println!(
		"ingot: {} -> {} page(s) in {} pass(es); {} anchor(s) in the ledger; written to {}/",
		source, out.pages.len(), out.passes, out.ledger.len(), out_dir);
	Ok(())
}
