//! Whole-book assembly: follow a Typst root's `#include` chain into one ordered block stream, and read
//! the concrete production values the book's `config.typ` selects.
//!
//! A book is not one source file. Its root (`oxpecker.typ`, `lucronics.typ`) sets the page through a
//! template call and then pulls each chapter in with `#include "chap.typ"`; the geometry and type are
//! chosen in `config.typ` by a `format` switch. The reader ([`lang::to_blocks`](crate::lang)) sets one
//! file and skips code lines, so the include-following and the config-reading live here, above it: this
//! module resolves the includes in document order, feeds each chapter through the reader, and reads the
//! one branch of `config.typ` the book's `format` selects into a [`PageGeometry`] and [`Style`].
//!
//! This is targeted extraction, not a Typst evaluator. It reads the concrete fields these books define
//! -- page size, mirror margins, body and heading type -- from the arm the `format` string picks, and
//! nothing more. A field a book does not set keeps the engine default.

use crate::doc::{
	Block,
	Style,
};
use crate::fonts;
use crate::ir::Sp;
use crate::lang;
use crate::page::PageGeometry;

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_font::set::FontSet;

use std::path::Path;
use std::sync::Arc;

const MM_PER_PT: f64 = 72.0 / 25.4;	// points in one millimetre

/// A whole book, assembled and ready to author: the block stream in document order, the page geometry
/// and type style read from its config, and the faces loaded by path from its assets.
pub struct BookSpec {
	pub geom:	PageGeometry,
	pub style:	Style,
	pub fonts:	Arc<FontSet>,
	pub blocks:	Vec<Block>,
}

/// Does this source read as a book root -- a Typst file that assembles chapters through `#include`?
/// A single manuscript has none, so the binary can tell a book from a lone file by the source itself.
pub fn is_book_root(src: &str) -> bool {
	src.lines().any(|l| l.trim_start().starts_with("#include"))
}

/// Assembles the book rooted at `root_path`: reads its `config.typ`, loads its Libertinus faces by
/// path, and follows the root's includes into one block stream.
pub fn load(root_path: &Path) -> Outcome<BookSpec> {
	let root_dir = match root_path.parent() {
		Some(d)	=> d.to_path_buf(),
		None	=> return Err(err!("The book root {:?} has no parent directory.", root_path; Input, Invalid)),
	};
	let root_src = match std::fs::read_to_string(root_path) {
		Ok(s)	=> s,
		Err(e)	=> return Err(err!(e, "Could not read the book root {:?}.", root_path; File, Read)),
	};

	// The config sits beside the root; the assets tree is one level up (the project root), holding the
	// Libertinus directory both books share.
	let config_path	= root_dir.join("config.typ");
	let config_src	= match std::fs::read_to_string(&config_path) {
		Ok(s)	=> s,
		Err(e)	=> return Err(err!(e, "Could not read the book config {:?}.", config_path; File, Read)),
	};
	let project_dir = match root_dir.parent() {
		Some(d)	=> d.to_path_buf(),
		None	=> root_dir.clone(),
	};
	let libertinus_dir = project_dir.join("assets").join("fonts").join("libertinus");
	let fonts = Arc::new(res!(fonts::libertinus_from_dir(&libertinus_dir)));

	let (geom, raw) = res!(read_config(&config_src));
	let style		= build_style(&raw);
	let blocks		= res!(assemble(&root_src, &root_dir));

	Ok(BookSpec { geom, style, fonts, blocks })
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ INCLUDE FOLLOWING                                                          │
// └───────────────────────────────────────────────────────────────────────────┘

/// Follows a root's `#include "..."` lines in order, reading each chapter and setting it through the
/// reader, and lifts each `#part-page[...]` divider to a level-1 heading so the part titles keep their
/// place in the flow. Everything else in the root -- the `#show: doc.with(...)` template call and its
/// metadata -- is book furniture the reader cannot evaluate, and is passed over.
pub fn assemble(root_src: &str, root_dir: &Path) -> Outcome<Vec<Block>> {
	let mut blocks: Vec<Block> = Vec::new();
	for line in root_src.lines() {
		let t = line.trim_start();
		if let Some(rest) = t.strip_prefix("#include") {
			if let Some(rel) = first_quoted(rest) {
				let path	= root_dir.join(&rel);
				let src		= match std::fs::read_to_string(&path) {
					Ok(s)	=> s,
					Err(e)	=> return Err(err!(e,
						"Could not read the included chapter {:?}.", path; File, Read)),
				};
				blocks.extend(res!(lang::to_blocks(&src)));
			}
		} else if t.starts_with("#part-page") {
			// A part divider: its title is the last bracket group on the line.
			if let Some(title) = bracket_body(t) {
				blocks.push(Block::heading(1, title));
			}
		}
	}
	Ok(blocks)
}

/// The first double-quoted run in a slice, its contents without the quotes.
fn first_quoted(s: &str) -> Option<String> {
	let open	= s.find('"')?;
	let rest	= &s[open + 1..];
	let close	= rest.find('"')?;
	Some(rest[..close].to_string())
}

/// The contents of the first `[...]` group in a line, balanced so a nested bracket does not close it
/// early. Used to lift a `#part-page[Title]` divider's title.
fn bracket_body(s: &str) -> Option<String> {
	let open	= s.find('[')?;
	let bytes	= s.as_bytes();
	let mut depth	= 0i32;
	let mut i	= open;
	while i < bytes.len() {
		match bytes[i] {
			b'['	=> depth += 1,
			b']'	=> {
				depth -= 1;
				if depth == 0 {
					return Some(s[open + 1..i].trim().to_string());
				}
			},
			_	=> {},
		}
		i += 1;
	}
	None
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ CONFIG EXTRACTION                                                          │
// └───────────────────────────────────────────────────────────────────────────┘

/// The raw type values read from a config arm, before they are turned into a [`Style`]. Kept apart from
/// the geometry because the geometry is complete on its own, while the style also needs the loaded font
/// metrics to turn an em-leading into a baseline distance.
struct RawStyle {
	body_pt:	f64,
	leading_em:	f64,	// par leading, a multiple of the em
	par_skip_em:	f64,	// space between paragraphs, a multiple of the em
	indent_em:	f64,	// first-line indent, a multiple of the em
	h1_pt:		f64,	// chapter-title size
	h2_pt:		f64,	// first sub-heading size
	h3_pt:		f64,	// second sub-heading size
}

/// Reads the branch of a config the `format` switch selects into a geometry and the raw type values.
/// A book that omits a field falls back to a readable default rather than failing, so an unfamiliar
/// config still assembles.
fn read_config(src: &str) -> Outcome<(PageGeometry, RawStyle)> {
	let format = match read_let_string(src, "format") {
		Some(f)	=> f,
		None	=> return Err(err!(
			"The config sets no `#let format = \"...\"`, so no page branch can be chosen."; Input, Missing)),
	};

	let dims	= arm(src, "page-dims", &format);
	let margins	= arm(src, "page-margins", &format);
	let scale	= arm(src, "type-scale", &format);

	let width	= dims.as_deref().and_then(|a| num_after(a, "width:")).unwrap_or(148.0);
	let height	= dims.as_deref().and_then(|a| num_after(a, "height:")).unwrap_or(210.0);
	let inside	= margins.as_deref().and_then(|a| num_after(a, "inside:")).unwrap_or(19.0);
	let outside	= margins.as_deref().and_then(|a| num_after(a, "outside:")).unwrap_or(17.0);
	let top		= margins.as_deref().and_then(|a| num_after(a, "top:")).unwrap_or(19.0);
	let bottom	= margins.as_deref().and_then(|a| num_after(a, "bottom:")).unwrap_or(21.0);

	let geom = PageGeometry::with_margins(
		Sp::from_pt(width  * MM_PER_PT),
		Sp::from_pt(height * MM_PER_PT),
		Sp::from_pt(inside  * MM_PER_PT),
		Sp::from_pt(outside * MM_PER_PT),
		Sp::from_pt(top    * MM_PER_PT),
		Sp::from_pt(bottom * MM_PER_PT),
	);

	let body_pt		= arm(src, "body-text-size", &format).as_deref().and_then(first_num).unwrap_or(11.0);
	let leading_em	= arm(src, "body-line-spacing", &format).as_deref().and_then(first_num).unwrap_or(0.75);
	let par_skip_em	= arm(src, "body-par-spacing", &format).as_deref().and_then(first_num).unwrap_or(0.75);
	let indent_em	= arm(src, "body-par-indent", &format).as_deref().and_then(first_num).unwrap_or(0.0);
	let h1_pt		= scale.as_deref().and_then(|a| num_after(a, "chapter-title:")).unwrap_or(20.0);
	let subs		= scale.as_deref().and_then(|a| tuple_after(a, "sub-headings:")).unwrap_or_default();
	let h2_pt		= subs.first().copied().unwrap_or(15.0);
	let h3_pt		= subs.get(1).copied().unwrap_or(12.5);

	Ok((geom, RawStyle { body_pt, leading_em, par_skip_em, indent_em, h1_pt, h2_pt, h3_pt }))
}

// The Libertinus line box Typst sets, as a fraction of the em, measured from the oracle. Typst's config
// leading is the gap ADDED between line boxes; the baseline-to-baseline skip is that gap plus the box.
// The box is not the face's nominal ascender + descender (fe2o3_font reports ~1.14 em for Libertinus,
// which sets ~30% too loose); Typst's rendered Libertinus line box measures ~0.68 em -- 15.75 pt for an
// 11 pt body at 0.75 em leading, read straight off `oracle/oxpecker_body.png`. The driver takes a
// baseline distance, so the style carries box + leading, and the flow then lands on Typst's grid.
const LIBERTINUS_LINE_BOX_EM: f64 = 0.682;

/// Turns the raw config values into a [`Style`]. The leading is the one derived value: the config sets
/// a gap in ems, and the driver wants a baseline-to-baseline distance, so the Libertinus line box (see
/// [`LIBERTINUS_LINE_BOX_EM`]) is added to it -- the calibration that puts the line grid on the oracle's.
fn build_style(raw: &RawStyle) -> Style {
	let baseline = (LIBERTINUS_LINE_BOX_EM + raw.leading_em) * raw.body_pt;

	let mut style = Style::default();
	style.body_size	= Sp::from_pt(raw.body_pt);
	style.leading	= Sp::from_pt(baseline);
	style.para_skip	= Sp::from_pt(raw.par_skip_em * raw.body_pt);
	style.indent	= Sp::from_pt(raw.indent_em * raw.body_pt);
	style.h1_size	= Sp::from_pt(raw.h1_pt);
	style.h2_size	= Sp::from_pt(raw.h2_pt);
	style.h3_size	= Sp::from_pt(raw.h3_pt);
	style
}

/// The string a `#let <name> = "..."` binds, if the config sets one as a plain literal.
fn read_let_string(src: &str, name: &str) -> Option<String> {
	let needle	= fmt!("#let {} =", name);
	let at		= src.find(&needle)?;
	let rest	= &src[at + needle.len()..];
	first_quoted(rest)
}

/// The body of the `if`/`else if` arm a `#let <name> = if format == "<fmt>" {...}` chain selects for
/// `fmt`. Bounds the search to the one `#let` so a later binding's arms are not read by mistake, finds
/// the arm whose condition tests this format, and returns its balanced `{...}` body.
fn arm(src: &str, name: &str, fmt: &str) -> Option<String> {
	let needle	= fmt!("#let {} =", name);
	let start	= src.find(&needle)?;
	let tail	= &src[start + needle.len()..];
	// The binding ends at the next top-level `#let`, or the end of the file.
	let end		= tail.find("\n#let ").unwrap_or(tail.len());
	let block	= &tail[..end];

	let cond	= fmt!("== \"{}\"", fmt);
	let at		= block.find(&cond)?;
	let after	= &block[at..];
	let brace	= after.find('{')?;
	balanced_braces(&after[brace..])
}

/// The contents of a `{...}` at the start of `s`, matched by brace depth so a nested record does not
/// close it early.
fn balanced_braces(s: &str) -> Option<String> {
	let bytes	= s.as_bytes();
	let mut depth	= 0i32;
	let mut i	= 0usize;
	while i < bytes.len() {
		match bytes[i] {
			b'{'	=> depth += 1,
			b'}'	=> {
				depth -= 1;
				if depth == 0 {
					return Some(s[1..i].to_string());
				}
			},
			_	=> {},
		}
		i += 1;
	}
	None
}

/// The first number after `key` in `s` -- the digits and one decimal point that follow the key. The
/// unit (`mm`, `pt`, `em`) is known from the key, so it is read off and dropped.
fn num_after(s: &str, key: &str) -> Option<f64> {
	let at	= s.find(key)?;
	first_num(&s[at + key.len()..])
}

/// The first number appearing anywhere in `s`, as an `f64` -- the leading numeric run after any
/// non-numeric lead-in. `11pt` and `0.75em` both read as their number.
fn first_num(s: &str) -> Option<f64> {
	let bytes	= s.as_bytes();
	let mut i	= 0usize;
	// Skip to the first digit or a decimal point that starts a number.
	while i < bytes.len() && !(bytes[i].is_ascii_digit() || bytes[i] == b'.') {
		i += 1;
	}
	let begin = i;
	while i < bytes.len() && (bytes[i].is_ascii_digit() || bytes[i] == b'.') {
		i += 1;
	}
	if i == begin {
		return None;
	}
	s[begin..i].parse::<f64>().ok()
}

/// The numbers of the first `( ... )` tuple after `key` -- `sub-headings: (15pt, 12.5pt, ...)` reads as
/// `[15.0, 12.5, ...]`.
fn tuple_after(s: &str, key: &str) -> Option<Vec<f64>> {
	let at		= s.find(key)?;
	let after	= &s[at + key.len()..];
	let open	= after.find('(')?;
	let close	= after[open..].find(')')?;
	let inner	= &after[open + 1..open + close];
	let nums: Vec<f64> = inner.split(',').filter_map(first_num).collect();
	Some(nums)
}

#[cfg(test)]
mod tests {
	use super::*;

	// A miniature two-format config with the shape the real books use: a `format` switch and a chain of
	// `if format == "..." {...}` arms per setting.
	const CFG: &str = r#"
#let format = "ingram-5x8"
#let page-dims = if format == "ingram-5x8" {
  (width: 127mm, height: 203mm)
} else {
  (width: 148mm, height: 210mm)
}
#let page-margins = if format == "ingram-5x8" {
  (inside: 17mm, outside: 15mm, top: 18mm, bottom: 18mm)
} else {
  (inside: 19mm, outside: 17mm, top: 19mm, bottom: 21mm)
}
#let body-text-size = if format == "ingram-5x8" { 11pt } else { 12pt }
#let body-line-spacing = if format == "ingram-5x8" { 0.75em } else { 0.75em }
#let body-par-spacing = if format == "ingram-5x8" { 0.75em } else { 0.75em }
#let type-scale = if format == "ingram-5x8" {
  ( title: 24pt, chapter-title: 20pt, sub-headings: (15pt, 12.5pt, 11.5pt, 11pt) )
} else {
  ( title: 27pt, chapter-title: 23pt, sub-headings: (17pt, 14.5pt, 13pt, 12.5pt) )
}
"#;

	#[test]
	fn test_the_selected_format_arm_is_read_00() -> Outcome<()> {
		let (geom, raw) = res!(read_config(CFG));
		// 127 mm and 203 mm in points, not the a5 fallback branch.
		assert_eq!(geom.width.to_pt().round() as i64, 360, "width should be 127 mm = 360 pt");
		assert_eq!(geom.height.to_pt().round() as i64, 575, "height should be 203 mm = 575 pt");
		// Mirror margins: inside binds wider than the fore-edge.
		assert_eq!(geom.inside.to_pt().round() as i64, 48, "inside 17 mm = 48 pt");
		assert_eq!(geom.outside.to_pt().round() as i64, 43, "outside 15 mm = 43 pt");
		assert!(geom.inside > geom.outside, "the binding margin is the wider of the two");
		assert!((raw.body_pt - 11.0).abs() < 1e-9, "body 11 pt, found {}", raw.body_pt);
		assert!((raw.h1_pt - 20.0).abs() < 1e-9, "h1 = chapter-title 20 pt, found {}", raw.h1_pt);
		assert!((raw.h2_pt - 15.0).abs() < 1e-9, "h2 = first sub-heading 15 pt, found {}", raw.h2_pt);
		assert!((raw.h3_pt - 12.5).abs() < 1e-9, "h3 = second sub-heading 12.5 pt, found {}", raw.h3_pt);
		Ok(())
	}

	#[test]
	fn test_the_mirror_shift_moves_a_verso_to_the_fore_edge_01() -> Outcome<()> {
		let (geom, _) = res!(read_config(CFG));
		// Recto content starts at the inside margin; the verso shift lands it at the outside one.
		let verso_left = geom.content_left() + geom.mirror_shift();
		assert_eq!(verso_left, geom.outside, "a shifted verso page's left edge is the fore-edge margin");
		Ok(())
	}

	#[test]
	fn test_a_root_with_includes_reads_as_a_book_02() {
		assert!(is_book_root("#show: doc.with()\n#include \"chap_01.typ\"\n"));
		assert!(!is_book_root("= A lone heading\n\nSome prose.\n"));
	}

	#[test]
	fn test_a_part_page_divider_lifts_to_a_heading_03() -> Outcome<()> {
		let dir = std::path::Path::new("/nonexistent");
		let blocks = res!(assemble("#part-page(label: \"Part\")[The Pattern]\n", dir));
		assert_eq!(blocks.len(), 1, "one divider, one heading");
		match &blocks[0] {
			Block::Heading { level, text, .. } => {
				assert_eq!(*level, 1, "a part divider is a level-1 heading");
				assert_eq!(text, "The Pattern", "the title is the bracket body");
			},
			other => return Err(err!("expected a heading, found {:?}", other; Test, Bug)),
		}
		Ok(())
	}
}
