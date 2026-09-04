//! `aus` -- the Austenite command-line driver.
//!
//! It builds a small in-memory document, runs the two-pass driver to a fixed point, writes each page
//! as an SVG, and prints the page count. That count is the pagination oracle: run against Typst on
//! the same input, it proves the streaming, flat-memory model paginates correctly. Phase 1 makes the
//! body real shaped text -- Latin sentences set with the embedded `fe2o3_font` set and drawn as glyph
//! outlines -- so the output SVG shows actual letters rather than the grey rules of Phase 0.
//!
//! Usage: `aus [OUTPUT_DIR]` (default `aus-out`).

use oxedyne_fe2o3_austenite::{
	driver::{
		self,
		Config,
		Document,
	},
	emit::Emitter,
	font::{
		FontMetrics,
		ShapedText,
	},
	ir::{
		BoxNode,
		Dims,
		Glue,
		Leaf,
		Node,
		Sp,
	},
	ledger::{
		AnchorId,
		AnchorKind,
	},
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
	let out_dir = match args.get(1) {
		Some(s)	=> s.clone(),
		None	=> "aus-out".to_string(),
	};

	let fonts	= Arc::new(res!(FontSet::embedded()));
	let doc		= res!(build_demo(fonts.clone()));
	let metrics	= FontMetrics::new(fonts.clone(), Role::Body, Dir::Ltr, Sp::from_pt(11.0));
	let out		= res!(driver::run(&doc, &metrics, Config::default()));

	res!(std::fs::create_dir_all(&out_dir));
	let emitter = Emitter::Svg;
	for page in &out.pages {
		let svg		= res!(emitter.render(page));
		let path	= fmt!("{}/page-{:03}.{}", out_dir, page.number, emitter.extension());
		res!(std::fs::write(&path, svg));
	}

	let ledger_path = fmt!("{}/ledger.jdat", out_dir);
	res!(out.ledger.to_file(&ledger_path));

	println!(
		"aus: composed {} page(s) in {} pass(es); {} anchor(s) in the ledger.",
		out.pages.len(), out.passes, out.ledger.len());
	println!("aus: wrote {} SVG page(s) and ledger.jdat to {}/", out.pages.len(), out_dir);
	Ok(())
}

/// Builds a small multi-page document: a shaped heading, a forward reference to the total page count,
/// and a run of shaped body lines. The heading and body are real text set with the embedded font set
/// and drawn as glyph outlines; the forward reference keeps a reserved slot, so the ledger and the
/// two-pass convergence are still exercised.
///
/// There is no line breaker yet, so each line is one pre-composed sentence chosen to sit within the
/// measure. The Knuth-Plass breaker that turns a paragraph of words into lines is the next increment;
/// see the TODO in this crate pointing at `fe2o3_text::unicode::linebreak`.
fn build_demo(fonts: Arc<FontSet>) -> Outcome<Document> {
	let geom	= PageGeometry::a4();
	let cw		= geom.content_width();
	let body_sz	= Sp::from_pt(11.0);
	let gap		= Glue::fixed(Sp::from_pt(4.0));

	let mut nodes: Vec<Node> = Vec::new();

	// A heading, set in the bold face: its anchor marks the page it lands on.
	nodes.push(Node::Anchor(AnchorId::new(AnchorKind::Heading, "intro")));
	let head = res!(ShapedText::new(
		fonts.clone(), Role::Bold, Dir::Ltr, Sp::from_pt(20.0),
		"Austenite -- real glyphs on the page"));
	let head_dims = head.dims();
	nodes.push(Node::HBox(BoxNode::new(vec![Node::Leaf(Leaf::text(head))], head_dims)));
	nodes.push(Node::Glue(Glue::fixed(Sp::from_pt(12.0))));

	// A forward reference to the total page count: a shaped label, then a slot reserved three digits
	// wide. Pass A shows nothing resolved; Pass B fills the slot from Pass A's page count. The label is
	// real text; the slot stays a reservation until a cross-reference machinery resolves it.
	let label = res!(ShapedText::new(
		fonts.clone(), Role::Body, Dir::Ltr, body_sz, "This document runs to"));
	let label_dims	= label.dims();
	let digit_em	= body_sz;
	let ref_dims	= Dims::new(digit_em * 3, label_dims.height, label_dims.depth);
	let ref_id		= AnchorId::new(AnchorKind::Citation, "total-count-ref");
	let ref_line_dims = Dims::new(cw, label_dims.height, label_dims.depth);
	nodes.push(Node::HBox(BoxNode::new(
		vec![
			Node::Leaf(Leaf::text(label)),
			Node::Glue(Glue::fixed(Sp::from_pt(4.0))),
			Node::Leaf(Leaf::reserved(ref_id, ref_dims)),
		],
		ref_line_dims)));
	nodes.push(Node::Glue(Glue::fixed(Sp::from_pt(12.0))));

	// The body: enough shaped lines to span a few pages. Each is one sentence set as a single run.
	let n_body = 90u32;
	for i in 0..n_body {
		let text	= fmt!(
			"{:>2}. Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod.",
			i + 1);
		let shaped	= res!(ShapedText::new(fonts.clone(), Role::Body, Dir::Ltr, body_sz, &text));
		let dims	= shaped.dims();
		nodes.push(Node::HBox(BoxNode::new(vec![Node::Leaf(Leaf::text(shaped))], dims)));
		nodes.push(Node::Glue(gap));
	}

	Ok(Document::new(nodes, geom))
}
