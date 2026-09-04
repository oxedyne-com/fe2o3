//! `aus` -- the Austenite command-line driver, Phase 0.
//!
//! It builds a trivial in-memory document, runs the two-pass driver to a fixed point, writes each
//! page as an SVG, and prints the page count. That count is the Phase 0 oracle: run against Typst on
//! the same simple input, it proves the streaming, flat-memory model paginates correctly before any
//! font, line breaker or Pearl writer exists.
//!
//! Usage: `aus [OUTPUT_DIR]` (default `aus-out`).

use oxedyne_fe2o3_austenite::{
	driver::{
		self,
		Config,
		Document,
	},
	emit::Emitter,
	ir::{
		BoxNode,
		Dims,
		Glue,
		Leaf,
		Node,
		Sp,
		StubMetrics,
	},
	ledger::{
		AnchorId,
		AnchorKind,
	},
	page::PageGeometry,
};

use oxedyne_fe2o3_core::prelude::*;

fn main() -> Outcome<()> {
	let args: Vec<String> = std::env::args().collect();
	let out_dir = match args.get(1) {
		Some(s)	=> s.clone(),
		None	=> "aus-out".to_string(),
	};

	let doc		= res!(build_demo());
	let metrics	= StubMetrics::new(Sp::from_pt(6.0), Sp::from_pt(1.5));
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

/// Builds a small multi-page document: a heading, a forward reference to the total page count, and a
/// run of body lines. Lines are rules -- solid rectangles standing in for shaped text -- which is
/// all Phase 0 needs to exercise pagination and emission.
fn build_demo() -> Outcome<Document> {
	let geom	= PageGeometry::a4();
	let cw		= geom.content_width();
	let line_h	= Sp::from_pt(11.0);
	let line_d	= Sp::from_pt(2.5);
	let gap		= Glue::fixed(Sp::from_pt(3.0));

	let mut nodes: Vec<Node> = Vec::new();

	// A heading: its anchor marks the page it lands on, and the rule stands in for the title.
	nodes.push(Node::Anchor(AnchorId::new(AnchorKind::Heading, "intro")));
	let head_dims = Dims::new(cw, Sp::from_pt(16.0), Sp::from_pt(3.0));
	nodes.push(Node::HBox(BoxNode::new(
		vec![Node::Leaf(Leaf::rule(head_dims))],
		head_dims)));
	nodes.push(Node::Glue(Glue::fixed(Sp::from_pt(8.0))));

	// A forward reference to the total page count: a label rule, then a slot reserved three digits
	// wide. Pass A shows nothing resolved; Pass B fills the slot from Pass A's page count.
	let digit_em	= Sp::from_pt(6.0);
	let ref_dims	= Dims::new(digit_em * 3, digit_em, Sp::from_pt(1.5));
	let ref_id		= AnchorId::new(AnchorKind::Citation, "total-count-ref");
	let ref_line_dims = Dims::new(cw, digit_em, Sp::from_pt(1.5));
	nodes.push(Node::HBox(BoxNode::new(
		vec![
			Node::Leaf(Leaf::rule(Dims::new(Sp::from_pt(120.0), digit_em, Sp::from_pt(1.5)))),
			Node::Glue(Glue::fixed(Sp::from_pt(6.0))),
			Node::Leaf(Leaf::reserved(ref_id, ref_dims)),
		],
		ref_line_dims)));
	nodes.push(Node::Glue(Glue::fixed(Sp::from_pt(8.0))));

	// The body, enough lines to span several pages.
	let n_body = 120u32;
	for i in 0..n_body {
		let short	= Sp::from_pt(((i * 7) % 40) as f64);	// vary the measure so it reads as prose
		let w		= cw - short;
		let dims	= Dims::new(w, line_h, line_d);
		nodes.push(Node::HBox(BoxNode::new(
			vec![Node::Leaf(Leaf::rule(dims))],
			dims)));
		nodes.push(Node::Glue(gap));
	}

	Ok(Document::new(nodes, geom))
}
