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
		Segment,
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
	math::Atom,
	page::PageGeometry,
	table::{
		Align,
		Cell,
		Row,
		Table,
	},
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

	let fonts	= Arc::new(res!(oxedyne_fe2o3_austenite::fonts::libertinus()));
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

	// The whole run as one PDF beside the per-page SVGs: same placed frames, same glyph outlines,
	// emitted as fill operators rather than <path> elements.
	let pdf = res!(oxedyne_fe2o3_austenite::emit::pdf::render_document(&out.pages));
	res!(std::fs::write(fmt!("{}/document.pdf", out_dir), pdf));

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
		Block::rich(vec![
			Segment::text(
				"The mechanism that makes the difference is the treatment of the interword space as an \
				elastic rather than a constant. Each space is given a natural width, an amount by which it \
				is willing to stretch, and an amount by which it will shrink under pressure."),
			Segment::footnote(
				"The natural width is the font's own space; the engine lets it grow by a half and give up \
				a third of itself, the elasticity Knuth chose for plain TeX."),
			Segment::text(
				" When a line is a little too short, every space in it grows by a proportional share of the \
				slack; when it is a little too long, every space gives up a share. Because the adjustment \
				is spread evenly across the whole line, no single gap yawns open while its neighbours stay \
				tight, and the resulting greyness is uniform."),
		]),
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
		Block::rich(vec![
			Segment::text(
				"Some breaks, though legal, are ugly. A heading marooned at the foot of a page, severed \
				from the paragraph it introduces, is one such fault, and this engine forbids it by binding \
				each heading to the first line beneath it as a single indivisible unit."),
			Segment::footnote(
				"This is the widow-and-orphan guard in its simplest form: a heading and its first line are \
				set inside one unbreakable box, so the greedy breaker moves the pair together."),
			Segment::text(
				" Should the two not fit together at the foot of a page, they move together to the next. \
				The running head above and the folio below are added last of all, in the margins, where \
				they name the current section and number the page without disturbing a single line of the \
				text they frame."),
		]),
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
			runs."),

		Block::heading(1, "Setting Mathematics on the Line"),
		Block::paragraph(
			"Mathematics is the sternest test of a typesetter, for it stacks symbols in two dimensions \
			where prose runs in one. This increment sets real expressions in a dedicated mathematics \
			font: a single-letter variable is drawn from the true mathematics italic alphabet, while a \
			digit, a function name and an operator stand upright, which is the oldest convention of the \
			craft."),
		Block::rich(vec![
			Segment::text("The equivalence of mass and energy is written "),
			Segment::math(Atom::row(vec![
				Atom::var("E"),
				Atom::rel("="),
				Atom::var("m"),
				Atom::sup(Atom::var("c"), Atom::num("2")),
			])),
			Segment::text(", a variable carrying a raised square. A quantity may equally be set as a \
				fraction in the running line, such as "),
			Segment::math(Atom::frac(Atom::num("1"), Atom::num("2"))),
			Segment::text(", set on the line with a solidus so it keeps within the line's height, though a \
				tall fraction is better shown displayed than crammed into the leading of a line of prose."),
		]),
		Block::paragraph(
			"A display equation is set on a line of its own, centred on the measure and, when it earns a \
			reference, numbered at the right margin. The quadratic formula gathers every feature of this \
			increment at once: a fraction with its rule, a superscript, upright digits and operators, an \
			italic unknown, and delimiters drawn at the running size."),
		Block::equation(
			Atom::row(vec![
				Atom::var("x"),
				Atom::rel("="),
				Atom::frac(
					Atom::row(vec![
						Atom::bin("\u{2212}"),	// a unary minus, set as the minus glyph
						Atom::var("b"),
						Atom::bin("\u{00B1}"),	// plus-or-minus
						Atom::op("\u{221A}"),	// a radical sign, drawn at the running size (it does not grow)
						Atom::open("("),
						Atom::sup(Atom::var("b"), Atom::num("2")),
						Atom::bin("\u{2212}"),
						Atom::num("4"),
						Atom::var("a"),
						Atom::var("c"),
						Atom::close(")"),
					]),
					Atom::row(vec![
						Atom::num("2"),
						Atom::var("a"),
					]),
				),
			]),
			true),
		Block::paragraph(
			"The mathematics font carries no OpenType MATH table, so the engine still approximates. The \
			radical and the parentheses cannot grow to embrace their contents, and the axis, the rule \
			thickness and the inter-symbol spaces are the classical defaults rather than a font's own \
			constants. What the increment sets is a real mathematics alphabet on a layout of stacked \
			symbols, raised scripts and a fraction centred on the axis, all built from the same boxes and \
			glue as the prose."),

		Block::heading(1, "A Table of the Three Stages"),
		Block::paragraph(
			"The three stages the earlier sections describe can be set side by side. The table below is \
			itself a block in this same vertical list: its columns are sized to their contents, its cells \
			wrapped to those columns by the very line breaker that sets the prose, and its rules drawn as \
			the same filled rectangles the engine uses for any rule. It is placed whole, so were it to meet \
			the foot of a page with too little room, it would move entire to the next."),
		Block::table(comparison_table()),
	]
}

/// A small comparison table: a bold header row, three body rows, and a right-aligned numeric column,
/// with two columns wide enough to wrap so cell line breaking is exercised.
fn comparison_table() -> Table {
	let head = Row::new(vec![
		Cell::new("Stage"),
		Cell::new("What it stacks"),
		Cell::new("Breaks it weighs"),
		Cell::aligned("Passes", Align::Right),
	]);
	let body = vec![
		Row::new(vec![
			Cell::new("Line breaking"),
			Cell::new("shaped words and elastic interword spaces"),
			Cell::new("the whole paragraph at once"),
			Cell::aligned("1", Align::Right),
		]),
		Row::new(vec![
			Cell::new("Page breaking"),
			Cell::new("finished lines and vertical springs"),
			Cell::new("one page, greedily, for now"),
			Cell::aligned("1", Align::Right),
		]),
		Row::new(vec![
			Cell::new("Running heads"),
			Cell::new("nothing at all; it consults the ledger"),
			Cell::new("none"),
			Cell::aligned("2", Align::Right),
		]),
	];
	let mut rows = vec![head];
	rows.extend(body);
	Table::new(true, rows)
}
