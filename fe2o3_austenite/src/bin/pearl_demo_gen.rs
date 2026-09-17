//! `pearl_demo_gen` -- THROWAWAY generator for the web reader's link + annotation demo sample.
//!
//! No author UI exists yet, so this exercises the create -> store -> read -> render path by hand: it
//! builds a small two-page document that carries an external `#link`, an internal `@ref` (resolving on
//! page two through the shipped ledger), then attaches one `highlight` and one `note` annotation to the
//! page block hashes with the public [`PearlDoc::add_annotation`] API, and writes the result to
//! `web/pearl-reader/samples/demo.prl`. It is a demo fixture builder, not part of the engine; delete
//! once a real annotation authoring path lands.
//!
//! Usage: `pearl_demo_gen <OUTPUT.prl>`.

use std::sync::Arc;

use oxedyne_fe2o3_austenite::emit::pearl::{
	Annotation,
	AnnotationKind,
	PearlBuilder,
	PearlDoc,
};
use oxedyne_fe2o3_austenite::font::ShapedText;
use oxedyne_fe2o3_austenite::fonts;
use oxedyne_fe2o3_austenite::ir::{
	Dims,
	DrawOp,
	Graphic,
	LinkTarget,
	Sp,
};
use oxedyne_fe2o3_austenite::ledger::{
	Anchor,
	AnchorId,
	AnchorKind,
	Ledger,
	Position,
};
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
	shape::Dir,
};
use oxedyne_fe2o3_graphics::{
	colour::Rgba,
	path::{
		Bounds,
		Path,
	},
};

fn main() -> Outcome<()> {
	let out = match std::env::args().nth(1) {
		Some(s)	=> s,
		None	=> return Err(err!(
			"Usage: pearl_demo_gen <OUTPUT.prl>"; Input, Invalid, Missing)),
	};

	let fonts	= Arc::new(res!(fonts::libertinus()));
	let geom	= PageGeometry::a4();
	let anchor	= AnchorId::new(AnchorKind::Label, "sec:page-two");

	// A helper placing a shaped line of body text at (x, y) points from the page's top-left, returning the
	// dimensions so the caller can line up an annotation rectangle over it.
	let text = |frame: &mut Frame, x: f64, y: f64, size: f64, s: &str| -> Outcome<Dims> {
		let shaped	= res!(ShapedText::new(fonts.clone(), Role::Body, Dir::Ltr, Sp::from_pt(size), s));
		let dims	= shaped.dims();
		frame.push(Placed::new(Sp::from_pt(x), Sp::from_pt(y), dims, PlacedKind::Text(shaped)));
		Ok(dims)
	};

	// A small filled box acting as a linked figure: the `link` leaf covers its placement box, so the reader
	// lays a clickable hotspot there.
	let link_box = |target: LinkTarget, w: f64, h: f64| -> Outcome<Graphic> {
		let fill	= res!(Path::rect(Bounds::new(0.0, 0.0, w as f32, h as f32)));
		let mut g	= Graphic::new(
			vec![DrawOp::Fill { path: fill, colour: Rgba::new(219, 234, 254, 255) }],
			Dims::new(Sp::from_pt(w), Sp::from_pt(h), Sp::ZERO));
		g.link = Some(target);
		Ok(g)
	};

	// Page one: a title, a highlighted body line, and two linked boxes -- one external, one internal.
	let mut f1 = Frame::new();
	res!(text(&mut f1, 72.0, 90.0, 20.0, "The Pearl Format"));
	let hl = res!(text(&mut f1, 72.0, 140.0, 12.0,
		"This line of body text carries a highlight annotation."));
	res!(text(&mut f1, 72.0, 190.0, 12.0, "External link (opens in a new tab):"));
	let ext = res!(link_box(LinkTarget::Uri("https://oxedyne.com".to_string()), 150.0, 16.0));
	f1.push(Placed::new(Sp::from_pt(300.0), Sp::from_pt(180.0), ext.dims, PlacedKind::Graphic(Arc::new(ext))));
	res!(text(&mut f1, 72.0, 230.0, 12.0, "Internal link (jumps to page two):"));
	let int = res!(link_box(LinkTarget::Anchor(anchor.clone()), 150.0, 16.0));
	f1.push(Placed::new(Sp::from_pt(300.0), Sp::from_pt(220.0), int.dims, PlacedKind::Graphic(Arc::new(int))));
	let page1 = Page::new(1, geom, f1);

	// Page two: the anchor target, and a line that carries the note annotation.
	let mut f2 = Frame::new();
	res!(text(&mut f2, 72.0, 90.0, 20.0, "Page Two"));
	res!(text(&mut f2, 72.0, 140.0, 12.0, "The internal link scrolled the reader here."));
	res!(text(&mut f2, 72.0, 170.0, 12.0, "This block carries a margin note annotation."));
	let page2 = Page::new(2, geom, f2);

	// The ledger fixes the internal anchor on page two, as a composition pass would.
	let mut ledger = Ledger::new();
	ledger.record(Anchor::new(anchor.clone(), Position::new(2, Sp::ZERO, Sp::ZERO)));

	let mut builder = res!(PearlBuilder::new(&ledger, geom));
	res!(builder.add_page(&page1));
	res!(builder.add_page(&page2));
	let mut doc = res!(PearlDoc::from_string(res!(builder.to_string())));

	// Attach the two annotations by block hash: a highlight over the body line on page one, a whole-block
	// note on page two.
	let hashes	= res!(doc.block_hashes());
	let hl_h	= hl.height.to_pt();
	res!(doc.add_annotation(Annotation::new(
		hashes[0].as_str(), AnnotationKind::Highlight,
		"The core claim of the document.", "jason", "2026-09-17T10:00:00Z")
		.with_rect(
			Sp::from_pt(70.0),
			Sp::from_pt(140.0 - 2.0),
			Sp::from_pt(330.0),
			Sp::from_pt(hl_h + 5.0))));
	res!(doc.add_annotation(Annotation::new(
		hashes[1].as_str(), AnnotationKind::Note,
		"This is an anchored note. It reveals its text on click, and follows the block across repagination.",
		"jason", "2026-09-17T11:00:00Z")));

	res!(doc.write_file(&out));
	println!("wrote {} ({} pages, {} annotations, links on page 1)",
		out, res!(doc.page_count()), res!(doc.annotations()).len());
	Ok(())
}
