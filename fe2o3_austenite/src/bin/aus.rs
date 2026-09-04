//! `aus` -- the Austenite command-line driver.
//!
//! Authors a real multi-section document -- headings and paragraphs of English prose -- through the
//! block layer, runs the two-pass driver to a fixed point, decorates each page with a running head
//! and a folio, and writes every page as SVG. The page count is the pagination oracle against Typst
//! on the same input. The body, the headings and the furniture are all real shaped text, so the SVG
//! shows actual letters flowing across several pages.
//!
//! Usage: `aus [OUTPUT_DIR]` (default `aus-out`).

use oxedyne_fe2o3_austenite::{
	doc::{
		self,
		Block,
		Style,
	},
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
		Ref,
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
	let geom	= PageGeometry::a4();
	let style	= Style::default();

	let (mut document, heads) = res!(doc::author(fonts.clone(), geom, style, &blocks()));

	// A table of contents at the very front: page 1 lists each section title against its resolved page,
	// the body follows on page 2. Prepended before the run, so its height is part of the vertical list
	// the driver converges -- the folios it prints are the very ones its own length helped fix.
	let toc = res!(doc::contents(fonts.clone(), geom, style, &heads));
	document.nodes.splice(0..0, toc);

	res!(append_colophon(&mut document, fonts.clone(), style));

	let metrics	= FontMetrics::new(fonts.clone(), Role::Body, Dir::Ltr, style.body_size);
	let mut out	= res!(driver::run(&document, &metrics, Config::default()));
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

	println!(
		"aus: composed {} page(s) in {} pass(es); {} anchor(s) in the ledger.",
		out.pages.len(), out.passes, out.ledger.len());
	println!("aus: wrote {} SVG page(s) and ledger.jdat to {}/", out.pages.len(), out_dir);
	Ok(())
}

/// A forward reference to the document's own length, set as a colophon at the very end: a shaped
/// label and a slot reserved three digit-ems wide. Pass A shows the empty reservation; Pass B
/// resolves `Ref::TotalPages` against Pass A's ledger and draws the count as shaped glyphs. Three
/// digit-ems clears any small page count, so the value fits its reservation and the document
/// converges in the usual two passes.
fn append_colophon(doc: &mut Document, fonts: Arc<FontSet>, style: Style) -> Outcome<()> {
	let cw		= doc.geom.content_width();
	let label	= res!(ShapedText::new(
		fonts.clone(), Role::Italic, Dir::Ltr, style.body_size, "This document runs to"));
	let ld		= label.dims();
	let ref_id	= AnchorId::new(AnchorKind::Citation, "total-count-ref");
	let ref_dims = Dims::new(style.body_size * 3, ld.height, ld.depth);
	let line_dims = Dims::new(cw, ld.height, ld.depth);

	doc.nodes.push(Node::Glue(Glue::fixed(Sp::from_pt(16.0))));
	doc.nodes.push(Node::HBox(BoxNode::new(
		vec![
			Node::Leaf(Leaf::text(label)),
			Node::Glue(Glue::fixed(Sp::from_pt(4.0))),
			Node::Leaf(Leaf::reserved(ref_id, Ref::TotalPages, ref_dims)),
		],
		line_dims)));
	Ok(())
}

/// The demo manuscript: four sections of real prose, long enough to flow across several pages so
/// running heads and folios are exercised on more than the first, and long words carry Liang
/// hyphenation points so at least one line breaks on a discretionary hyphen.
fn blocks() -> Vec<Block> {
	vec![
		Block::heading(1, "On the Setting of a Justified Page"),
		Block::paragraph(
			"Typesetting is the quiet craft of arranging letters so that a reader forgets they are \
			reading letters at all. A well set page presents an even, unhurried texture, its lines of \
			equal length and its spacing so consistent that the eye glides from one line to the next \
			without catching. The difficulty is that words have fixed widths and lines do not, so the \
			spaces between words must stretch or shrink to fill each measure exactly. Done badly, this \
			leaves rivers of white running down the page; done well, it is invisible."),
		Block::paragraph(
			"The mechanism that makes the difference is the treatment of the interword space as an \
			elastic rather than a constant. Each space is given a natural width, an amount by which it \
			is willing to stretch, and an amount by which it will shrink under pressure. When a line \
			is a little too short, every space in it grows by a proportional share of the slack; when \
			it is a little too long, every space gives up a share. Because the adjustment is spread \
			evenly across the whole line, no single gap yawns open while its neighbours stay tight, \
			and the resulting greyness is uniform."),
		Block::paragraph(
			"Justification, then, is not a decoration applied after the fact but a consequence of how \
			the line is built. The engine that sets this document carries the adjustment inside the \
			glue itself, so that laying a line from left to right, with no further arithmetic, still \
			fills the measure precisely. The representation is deliberate: a break decision, once \
			taken, needs no second thought when the line is finally drawn."),

		Block::heading(1, "Breaking Words Across the Line"),
		Block::paragraph(
			"However elastic the spaces, some words are simply too long to sit comfortably at the end \
			of a narrow line, and forcing them to do so either overfills the line or opens the \
			preceding spaces into unsightly caverns. The remedy is hyphenation: an unusually long \
			word may be broken across the boundary between two consecutive lines, a hyphen marking the \
			division. The art is in choosing where to break, for a hyphen in the wrong place is worse \
			than none at all."),
		Block::paragraph(
			"The classical solution, and the one used here, is Liang's algorithm of patterns. A large \
			dictionary of fragments, each carrying numbers that encode where a break is encouraged or \
			forbidden, is distilled into a compact set of rules. To hyphenate a word, the algorithm \
			pads it with boundary markers, scores every position by the strongest matching pattern, \
			and admits a break wherever an odd score survives. Words such as representation, \
			typographical, and consecutive yield their break points reliably, while short or \
			irregular words are left whole."),
		Block::paragraph(
			"A break inside a word is never taken lightly. The line breaker weighs the cost of the \
			hyphen against the cost of the loose or tight line it would avoid, and it declines to end \
			two consecutive lines on a hyphen, which the eye reads as a stammer. Only when the saving \
			is real does the word divide, and then the hyphen is added to the line as its final mark, \
			present in the ink precisely because the break was chosen."),

		Block::heading(1, "The Page as a Vertical List"),
		Block::paragraph(
			"Once the paragraphs are set into lines, the lines themselves must be stacked into pages. \
			Here the material is vertical rather than horizontal, but the model is the same: boxes of \
			fixed height, springs of adjustable space between them, and points at which a break is \
			permitted or penalised. A page fills until the next line would overflow the text block, \
			and then it breaks, carrying the remainder to a fresh page."),
		Block::paragraph(
			"Some breaks, though legal, are ugly. A heading marooned at the foot of a page, severed \
			from the paragraph it introduces, is one such fault, and this engine forbids it by binding \
			each heading to the first line beneath it as a single indivisible unit. Should the two not \
			fit together at the foot of a page, they move together to the next. The running head above \
			and the folio below are added last of all, in the margins, where they name the current \
			section and number the page without disturbing a single line of the text they frame."),
		Block::paragraph(
			"The vertical breaker in this first increment is deliberately simple. It fills greedily, \
			taking the first legal break that keeps the page from overflowing, and it does not yet \
			weigh a window of pages against one another as the horizontal breaker weighs a paragraph. \
			That refinement, together with the balancing of facing pages and the proper treatment of \
			figures that float away from their anchors, belongs to a later stage. What matters now is \
			that the structure is honest: pages are a vertical list of the same boxes and springs, and \
			the machinery that will weigh them is the machinery already proven on the line."),

		Block::heading(1, "The Ledger and the Running Head"),
		Block::paragraph(
			"Every heading, as it is set, records its identity and the page it fell upon in a ledger \
			that outlives the composition. Nothing in the document may look at the layout directly; it \
			may only consult this ledger, and only for the classes of fact the engine agreed in \
			advance to record. A running head is the first and simplest customer. To decide which \
			section title belongs at the top of a given page, the engine asks the ledger for the most \
			recent heading that resolved to an earlier page, and sets that title in the margin."),
		Block::paragraph(
			"This is the mechanism that older systems reconstruct laboriously through global queries \
			and repeated passes, asking after the fact which headings precede a point and comparing \
			their pages against the page being drawn. A streaming assembler already holds the answer \
			at the instant it closes a page, so the running head costs nothing beyond a lookup. The \
			first page of a section, where the title would merely repeat the heading printed inches \
			below it, conventionally shows no running head at all, and the engine suppresses it there \
			without special pleading."),
		Block::paragraph(
			"The same ledger will in time answer a table of contents, a cross reference to a numbered \
			figure, and an index gathered from the margins of the text. Each is a different question \
			put to the same record, and none of them reaches behind it to the layout. That discipline \
			is what lets the whole document be composed in a bounded window of memory, one page \
			assembled and written and forgotten before the next begins, however long the manuscript \
			runs.")
	]
}
