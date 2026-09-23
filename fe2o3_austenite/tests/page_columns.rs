//! Does `#set page(columns: n)` flow the body down equal columns, and does a parent-scoped float span them?
//!
//! Every case drives the real [`compile::assemble`]/[`compile::author_and_run`] path over a synthetic lone
//! document installed in the source map, then reads the placed frame: each text run's page, x, width and y.
//! Five assertions map to one fix each, and reverting that fix reds it (checked 2026-09-23):
//!
//!   * [`two_columns_fill_left_then_right`]: the author's page-column marker and column measure.
//!   * [`a_parent_float_spans_both_columns_above_them`]: the parent-scoped seating in the driver's `Flow`.
//!   * [`a_parent_float_met_mid_page_relays_the_page`]: the relayout that rewinds the page for it.
//!   * [`a_column_float_stays_in_its_column`]: the column-scoped band in the driver's `Flow`.
//!   * [`colbreak_moves_to_the_next_column`]: the column eject.
//!
//! [`one_column_is_the_ordinary_page`] guards the other side: a document naming no columns sets full-width
//! lines exactly as before, which the oracle's pinned hashes confirm byte for byte.

use oxedyne_fe2o3_austenite::compile;
use oxedyne_fe2o3_austenite::fonts;
use oxedyne_fe2o3_austenite::page::PlacedKind;
use oxedyne_fe2o3_austenite::vfs;

use oxedyne_fe2o3_core::prelude::*;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{
	Arc,
	Mutex,
};

// The source map is a process-wide global, so the cases take turns.
static VFS: Mutex<()> = Mutex::new(());

/// One placed text run, in points: its page, left edge, right edge, top and source string.
#[derive(Debug, Clone)]
struct Run {
	page:	u32,
	x0:		f64,
	x1:		f64,
	y:		f64,
	src:	String,
}

/// The page geometry every case sets on: A4 with the lone-file margins.
struct Geom {
	left:	f64,
	width:	f64,
}

/// Compiles `src` as a lone `/doc/main.typ`, returning every placed text run and the content block's left
/// edge and width.
fn compile(src: &str) -> Outcome<(Vec<Run>, Geom)> {
	let _turn = match VFS.lock() {
		Ok(g)	=> g,
		Err(p)	=> p.into_inner(),
	};
	let main = PathBuf::from("/doc/main.typ");
	let mut files: HashMap<PathBuf, Vec<u8>> = HashMap::new();
	files.insert(main.clone(), src.as_bytes().to_vec());
	res!(vfs::install(files));
	let set		= Arc::new(res!(fonts::libertinus()));
	let result	= compile::assemble(&main, || Ok(set.clone()))
		.and_then(|(a, _, _)| compile::author_and_run(a));
	let _ = vfs::clear();
	let rendered = res!(result);
	let geom = Geom {
		left:	rendered.geom.content_left().to_pt(),
		width:	rendered.geom.content_width().to_pt(),
	};
	let mut runs = Vec::new();
	for page in &rendered.out.pages {
		for placed in &page.frame.placed {
			if let PlacedKind::Text(shaped) = &placed.kind {
				runs.push(Run {
					page:	page.number,
					x0:		placed.x.to_pt(),
					x1:		placed.x.to_pt() + placed.dims.width.to_pt(),
					y:		placed.y.to_pt(),
					src:	shaped.source().to_string(),
				});
			}
		}
	}
	Ok((runs, geom))
}

/// `n` paragraphs of placeholder prose, each tagged with a word the test can find it by.
fn prose(n: usize, tag: &str) -> String {
	let mut s = String::new();
	for i in 0..n {
		s.push_str(&fmt!("{}{}\n#lorem(110)\n\n", tag, i));
	}
	s
}

/// The runs on `page` whose source contains `word`.
fn find<'a>(runs: &'a [Run], word: &str) -> Vec<&'a Run> {
	runs.iter().filter(|r| r.src.contains(word)).collect()
}

/// The first run carrying `word`, or an error naming it.
fn first<'a>(runs: &'a [Run], word: &str) -> Outcome<&'a Run> {
	match runs.iter().find(|r| r.src.contains(word)) {
		Some(r)	=> Ok(r),
		None	=> Err(err!("No placed run carries {:?}.", word; Test, Missing)),
	}
}

// A quarter point, for positions computed in scaled integers and read back as points.
const TOL: f64 = 0.25;

/// Two columns 4% of the measure apart, filled left then right: page one's body runs all lie within one
/// column or the other, both columns carry text, and the prose that opens the right column follows the
/// prose that ends the left.
#[test]
fn two_columns_fill_left_then_right() -> Outcome<()> {
	let src = fmt!("#set page(columns: 2)\n\n{}", prose(12, "Para"));
	let (runs, g) = res!(compile(&src));
	let gutter	= g.width * 0.04;
	let col_w	= (g.width - gutter) / 2.0;
	let right	= g.left + col_w + gutter;
	// The body runs of page one, the folio at its foot aside.
	let foot = runs.iter().filter(|r| r.page == 1).map(|r| r.y).fold(0.0f64, f64::max);
	let body: Vec<&Run> = runs.iter().filter(|r| r.page == 1 && r.y < foot - TOL).collect();
	assert!(!body.is_empty());
	let mut left_n = 0;
	let mut right_n = 0;
	for r in &body {
		if r.x1 <= g.left + col_w + TOL {
			left_n += 1;
		} else if r.x0 >= right - TOL && r.x1 <= g.left + g.width + TOL {
			right_n += 1;
		} else {
			return Err(err!("A run straddles the gutter or overruns the page: {:?}.", r; Test, Mismatch));
		}
	}
	assert!(left_n > 20 && right_n > 20, "both columns carry text: {} left, {} right", left_n, right_n);
	// The paragraph tags run in order down the left column and on into the right.
	let tags: Vec<&Run> = body.iter().copied().filter(|r| r.src.starts_with("Para")).collect();
	assert!(tags.len() >= 2, "{:?}", tags);
	let first_right = tags.iter().position(|r| r.x0 >= right - TOL);
	match first_right {
		Some(k) => assert!(tags[..k].iter().all(|r| r.x1 <= g.left + col_w + TOL), "{:?}", tags),
		None	=> return Err(err!("No paragraph opened in the right column: {:?}.", tags; Test, Mismatch)),
	}
	// The flow ran on to a second page.
	assert!(runs.iter().any(|r| r.page == 2), "twelve paragraphs fill more than one page of two columns");
	Ok(())
}

/// A document naming no columns sets its lines across the whole measure.
#[test]
fn one_column_is_the_ordinary_page() -> Outcome<()> {
	let (runs, g) = res!(compile(&prose(3, "Para")));
	let widest = runs.iter().map(|r| r.x1).fold(0.0f64, f64::max);
	assert!(widest > g.left + g.width * 0.9, "a justified line reaches the right margin: {}", widest);
	Ok(())
}

/// A `#place(top, scope: "parent", float: true)` at the start of a two-column page spans both columns at
/// the page top, and both columns start below it.
#[test]
fn a_parent_float_spans_both_columns_above_them() -> Outcome<()> {
	let src = fmt!(
		"#set page(columns: 2)\n\n#place(top, scope: \"parent\", float: true)[Spanning banner words that run on across the \
		whole page width well past the gutter between the two columns of the body below.]\n\n{}",
		prose(8, "Para"));
	let (runs, g) = res!(compile(&src));
	let gutter	= g.width * 0.04;
	let col_w	= (g.width - gutter) / 2.0;
	let banner	= res!(first(&runs, "Spanning"));
	let line: Vec<&Run> = runs.iter().filter(|r| r.page == 1 && (r.y - banner.y).abs() < TOL).collect();
	let reach = line.iter().map(|r| r.x1).fold(0.0f64, f64::max);
	assert!(reach > g.left + col_w + gutter, "the banner line runs past the gutter: {:?}", line);
	// The banner's last line, and the prose of both columns below it.
	let last = res!(first(&runs, "below."));
	assert_eq!((banner.page, last.page), (1, 1));
	let para0 = res!(first(&runs, "Para0"));
	assert!(para0.page == 1 && para0.y > last.y, "the column text starts below the banner: {:?} vs {:?}", para0, last);
	// Everything else on the page lies below the banner: nothing shares its band but its own lines.
	let below: Vec<&Run> = runs.iter().filter(|r| r.page == 1 && r.y > last.y + TOL).collect();
	let words = "Spanning banner words that run on across the whole page width well past the gutter between the \
		two columns of the body below.";
	for r in runs.iter().filter(|r| r.page == 1 && r.y <= last.y + TOL) {
		assert!(words.contains(r.src.trim()), "only the banner's own lines sit in its band: {:?}", r);
	}
	assert!(below.iter().any(|r| r.x0 >= g.left + col_w + gutter - TOL), "both columns run beneath it");
	Ok(())
}

/// A parent-scoped float met after the columns already carry text is still seated at the top of that
/// page: the page is relaid beneath it, so the text that preceded it in the source sits below it.
#[test]
fn a_parent_float_met_mid_page_relays_the_page() -> Outcome<()> {
	let src = fmt!(
		"#set page(columns: 2)\n\nOpening words.\n\n#place(top, scope: \"parent\", float: true)[Spanning banner words that \
		run on across the whole page width well past the gutter between the two columns.]\n\n{}",
		prose(8, "Para"));
	let (runs, _) = res!(compile(&src));
	let banner	= res!(first(&runs, "Spanning"));
	let opening	= res!(first(&runs, "Opening"));
	assert_eq!(banner.page, 1, "the banner is seated on the page it was met on");
	assert_eq!(opening.page, 1);
	assert!(opening.y > banner.y, "the text before the banner in the source is relaid below it: {:?} {:?}", opening, banner);
	Ok(())
}

/// A column-scoped float (a figure placed at the top, Typst's default scope) met in the right column
/// settles at the top of that column: its runs stay within the right column's bounds, above its prose.
#[test]
fn a_column_float_stays_in_its_column() -> Outcome<()> {
	let src = fmt!(
		"#set page(columns: 2)\n\nLeft words.\n\n#colbreak()\n\n{}#figure(table(columns: 2, [Alpha], [Beta]), placement: top, \
		caption: [Tabled])\n\n{}",
		prose(1, "Para"), prose(1, "More"));
	let (runs, g) = res!(compile(&src));
	let gutter	= g.width * 0.04;
	let col_w	= (g.width - gutter) / 2.0;
	let right	= g.left + col_w + gutter;
	let alpha	= res!(first(&runs, "Alpha"));
	let cap		= res!(first(&runs, "Tabled"));
	let in_left		= |r: &Run| r.x1 <= g.left + col_w + TOL;
	let in_right	= |r: &Run| r.x0 >= right - TOL;
	assert!(in_right(alpha) && in_right(cap), "the figure sits within the right column: {:?} {:?}", alpha, cap);
	assert!(!in_right(res!(first(&runs, "Left"))), "the left column keeps its own text");
	// It rose to the top of its column, above that column's prose.
	let same_col: Vec<&Run> = runs.iter()
		.filter(|r| r.page == alpha.page && in_left(r) == in_left(alpha) && r.src.starts_with("Para"))
		.collect();
	assert!(same_col.iter().all(|r| r.y > alpha.y), "the figure tops its column: {:?} vs {:?}", alpha, same_col);
	Ok(())
}

/// `#colbreak()` moves the flow to the next column: the text after it opens the right column of the same
/// page, level with the left column's top.
#[test]
fn colbreak_moves_to_the_next_column() -> Outcome<()> {
	let src = "#set page(columns: 2)\n\nLeft words.\n\n#colbreak()\n\nRight words.\n";
	let (runs, g) = res!(compile(src));
	let gutter	= g.width * 0.04;
	let col_w	= (g.width - gutter) / 2.0;
	let left	= res!(first(&runs, "Left"));
	let right	= res!(first(&runs, "Right"));
	assert_eq!((left.page, right.page), (1, 1));
	assert!(right.x0 >= g.left + col_w + gutter - TOL, "{:?}", right);
	assert!((right.y - left.y).abs() < TOL, "both columns open at the same top: {:?} {:?}", left, right);
	// Without columns the break turns the page instead.
	let (flat, _) = res!(compile("Left words.\n\n#colbreak()\n\nRight words.\n"));
	assert_eq!(res!(first(&flat, "Right")).page, 2, "a column break on a page of one column breaks the page");
	assert!(find(&flat, "Left").iter().all(|r| r.page == 1));
	Ok(())
}

/// `#set columns(gutter: 30pt)` parts the columns by that length.
#[test]
fn the_gutter_is_the_set_length() -> Outcome<()> {
	let src = "#set page(columns: 2)\n#set columns(gutter: 30pt)\n\nLeft words.\n\n#colbreak()\n\nRight words.\n";
	let (runs, g) = res!(compile(src));
	let col_w	= (g.width - 30.0) / 2.0;
	let right	= res!(first(&runs, "Right"));
	assert!((right.x0 - (g.left + col_w + 30.0)).abs() < 0.5, "{:?} vs {}", right, g.left + col_w + 30.0);
	Ok(())
}

/// A scope setting its own page columns -- an included chapter's `#set page(columns: 2)` -- starts them on a
/// fresh page, sets its blocks at the column measure, and returns to one column on another fresh page when
/// it ends, as Typst's `set page` inside a scope does.
#[test]
fn a_scoped_column_layout_turns_the_page_both_ways() -> Outcome<()> {
	use oxedyne_fe2o3_austenite::doc::{self, Block};
	use oxedyne_fe2o3_austenite::driver::{self, Config};
	use oxedyne_fe2o3_austenite::font::FontMetrics;
	use oxedyne_fe2o3_austenite::fonts::FaceResolver;
	use oxedyne_fe2o3_austenite::page::PageGeometry;
	use oxedyne_fe2o3_austenite::theme::{Theme, ThemePatch};
	use oxedyne_fe2o3_font::{face::Role, shape::Dir};

	let words = |tag: &str| fmt!("{} {}", tag, "lorem ipsum dolor sit amet consectetur ".repeat(30));
	let mut patch = ThemePatch::default();
	patch.page.columns = Some(2);
	let blocks = vec![
		Block::paragraph(words("Dog")),
		Block::Scoped { patch, blocks: vec![Block::paragraph(words("Cat")), Block::paragraph(words("Owl"))] },
		Block::paragraph(words("Emu")),
	];
	let fonts	= Arc::new(res!(fonts::libertinus()));
	let geom	= PageGeometry::a4();
	let style	= Theme::default();
	let (document, _) = res!(doc::author(fonts.clone(), geom, &style, &FaceResolver::default(), &blocks, None, None));
	let metrics	= FontMetrics::new(fonts, Role::Body, Dir::Ltr, style.text.body_size);
	let out		= res!(driver::run(&document, &metrics, Config::default()));
	let mut runs: Vec<Run> = Vec::new();
	for page in &out.pages {
		for placed in &page.frame.placed {
			if let PlacedKind::Text(shaped) = &placed.kind {
				runs.push(Run {
					page:	page.number,
					x0:		placed.x.to_pt(),
					x1:		placed.x.to_pt() + placed.dims.width.to_pt(),
					y:		placed.y.to_pt(),
					src:	shaped.source().to_string(),
				});
			}
		}
	}
	let left	= geom.content_left().to_pt();
	let width	= geom.content_width().to_pt();
	let col_w	= (width - width * 0.04) / 2.0;
	let before	= res!(first(&runs, "Dog"));
	let inside	= res!(first(&runs, "Cat"));
	let after	= res!(first(&runs, "Emu"));
	assert_eq!((before.page, inside.page, after.page), (1, 2, 3), "the scope opens and closes on fresh pages");
	for r in runs.iter().filter(|r| r.page == 2) {
		assert!(r.x1 <= left + col_w + TOL || r.x0 >= left + col_w + width * 0.04 - TOL,
			"the scope's text keeps to its columns: {:?}", r);
	}
	let widest_after = runs.iter().filter(|r| r.page == 3).map(|r| r.x1).fold(0.0f64, f64::max);
	assert!(widest_after > left + width * 0.9, "after the scope the page is one column again: {}", widest_after);
	Ok(())
}
