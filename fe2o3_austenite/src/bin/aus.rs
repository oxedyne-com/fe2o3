//! `aus` -- the Austenite command-line driver.
//!
//! Builds a small document, runs the two-pass driver to a fixed point, writes each page as SVG, and
//! prints the page count -- the pagination oracle against Typst on the same input. The body is real
//! shaped text, so the SVG shows actual letters.
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
	linebreak::break_paragraph,
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

/// A small multi-page document: a shaped heading, a forward-reference line with a reserved slot (to
/// exercise the ledger), and a real paragraph run through the Knuth-Plass breaker so the body is set
/// justified across the measure.
fn build_demo(fonts: Arc<FontSet>) -> Outcome<Document> {
	let geom	= PageGeometry::a4();
	let cw		= geom.content_width();
	let body_sz	= Sp::from_pt(11.0);

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

	// The body: one real paragraph, broken into justified lines at the measure by Knuth-Plass. The long
	// English words carry legal Liang hyphenation points, so at least one line ends on a discretionary
	// hyphen. The text is repeated to span a few pages, so pagination and two-pass convergence still get
	// a workout.
	let sentence = "The typographic algorithm favours justification over ragged setting, and its \
		discretionary hyphenation lets an unusually long word break across the boundary between two \
		consecutive lines. Hyphenation of a difficult word such as representation keeps every \
		paragraph beautiful.";
	let mut body = String::new();
	for _ in 0..14 {
		body.push_str(sentence);
		body.push(' ');
	}
	let leading = Sp::from_pt(13.2);	// 1.2x the 11pt body
	let lines	= res!(break_paragraph(
		fonts.clone(), Role::Body, Dir::Ltr, body_sz, body.trim_end(), cw, leading));
	nodes.extend(lines);

	Ok(Document::new(nodes, geom))
}
