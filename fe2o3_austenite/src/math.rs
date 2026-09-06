//! Mathematics layout: TeX's mlist-to-hlist, simplified to the embedded text faces.
//!
//! A maths expression is an [`Atom`] tree -- symbols with a spacing [`Class`], rows, fractions and
//! scripts -- and [`layout`] sets it into the box-glue-penalty stream the rest of the engine draws.
//! The tree is measured into an internal box carrying, for each drawable, an x offset from the box's
//! left and a `rel` offset from the maths baseline; [`layout`] then flattens that into one [`Node::HBox`]
//! of leaves seated by glue and lifted by each leaf's own [`Leaf::with_shift`]. A fraction bar is a
//! [`LeafKind::Rule`](crate::ir::LeafKind); a numerator is raised and a script lowered by the shift, so
//! nothing bottoms out in a nested box -- which [`place_line`](crate::driver) would draw as a bare
//! rectangle. The one returned HBox is unwrapped by the caller: [`doc`](crate::doc) weaves its leaves
//! into a paragraph line for inline maths, or centres them on their own line for a display equation.
//!
//! The MATH-font boundary, stated plainly. Latin Modern Math carries a real OpenType MATH table, and
//! this module now sets to it: the maths italic alphabet, the axis height, the fraction rule thickness,
//! the display-style fraction shifts and gaps, the script shifts and their minima, and the two script
//! scale-down percentages are all read from the font, and a delimiter or a radical grows to its content
//! through the font's vertical glyph variants. The plain-TeX guesses -- a quarter-em axis, half-em
//! shifts, seven-tenths and one-half script sizes -- survive only as the fallback for a text face with
//! no MATH table. The inter-atom spacing is still TeX's reduced `mu` table rather than the font's, and a
//! delimiter taller than the largest pre-drawn variant is not yet assembled from repeating pieces; big
//! operators, matrices and a maths parser are later work. See the crate's phase notes.

use crate::doc::Style;
use crate::font::ShapedText;
use crate::ir::{
	BoxNode,
	Dims,
	DrawOp,
	Glue,
	Graphic,
	Leaf,
	Node,
	Sp,
};
use crate::mathtable::MathTable;

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_font::{
	face::Role,
	font::Font,
	set::FontSet,
	shape::Dir,
};
use oxedyne_fe2o3_graphics::{
	colour::Rgba,
	path::Path,
	transform::Transform,
};

use std::sync::{
	Arc,
	OnceLock,
};

// The maths font: Latin Modern Math, the OpenType form of Computer Modern -- the faces TeX sets
// mathematics in -- so a variable, an operator and a radical wear the letterforms a reader knows from
// every mathematics paper. It carries the Mathematical Italic block, the upright operators and the
// large symbols, and an OpenType MATH table of layout constants and grown-delimiter variants, which
// this module reads for its shifts, gaps and variants (see the header).
const MATH_FONT: &[u8] = include_bytes!("../fonts/latinmodern-math.otf");

/// The parsed maths font, built once and shared. Parsing the face is the costly part, so it is cached
/// behind a [`OnceLock`]; a lost race merely parses twice and keeps the first.
fn math_font() -> Outcome<Arc<Font>> {
	static MATH: OnceLock<Arc<Font>> = OnceLock::new();
	if let Some(f) = MATH.get() {
		return Ok(f.clone());
	}
	let f = Arc::new(res!(Font::new(MATH_FONT.to_vec())));
	let _ = MATH.set(f.clone());
	Ok(f)
}

/// The maths font's OpenType MATH table, parsed once and shared -- the layout constants and the grown
/// delimiter and radical variants. `None` if the font carries no such table, in which case the layout
/// falls back to the plain-TeX approximations.
fn math_table() -> Outcome<Option<Arc<MathTable>>> {
	static TABLE: OnceLock<Option<Arc<MathTable>>> = OnceLock::new();
	if let Some(t) = TABLE.get() {
		return Ok(t.clone());
	}
	let parsed = res!(MathTable::parse(MATH_FONT)).map(Arc::new);
	let _ = TABLE.set(parsed.clone());
	Ok(parsed)
}

/// The glyph id a single character shapes to in the maths font, for looking the character up in the
/// MATH table's variants.
fn glyph_id(font: &Arc<Font>, size: Sp, ch: &str) -> Outcome<u32> {
	let run = res!(font.shape(ch, size.to_pt() as f32, Dir::Ltr));
	match run.glyphs.first() {
		Some(g) => Ok(g.id),
		None => Err(err!("The maths font shaped {:?} to no glyph.", ch; Bug)),
	}
}

/// One glyph's outline flipped into the engine's y-down frame, with its ink bounding box. The outline
/// comes from the font y up with the baseline at zero; the flip leaves the baseline at zero and the ink
/// above the baseline at negative y. Returns the flipped path and its extent above and below and its
/// width, so a caller can seat a grown delimiter or a radical by its ink.
fn glyph_ink(font: &Arc<Font>, gid: u32, size: Sp) -> Outcome<(Path, Sp, Sp, Sp)> {
	let raw		= res!(font.outline(0, gid, size.to_pt() as f32));
	let path	= res!(raw.transform(&Transform::scale(1.0, -1.0)));
	match path.bounds(&Transform::IDENTITY) {
		Some(b) => {
			// y down: the top of the ink is the least y, the bottom the greatest.
			let height	= if b.y0 < 0.0 { Sp::from_pt((-b.y0) as f64) } else { Sp::ZERO };
			let depth	= if b.y1 > 0.0 { Sp::from_pt(b.y1 as f64) } else { Sp::ZERO };
			let width	= Sp::from_pt(b.x1.max(0.0) as f64);
			Ok((path, height, depth, width))
		},
		None => Ok((path, Sp::ZERO, Sp::ZERO, Sp::ZERO)),	// a blank glyph, nothing to seat
	}
}

/// An atom's spacing class, after TeX. The class of the atoms either side of a gap fixes the space
/// set there; the class also decides, for a symbol, whether it is set upright or in the maths italic.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Class {
	Ord,	// an ordinary atom: a variable or a number
	Op,		// a large operator or a function name (sin, log)
	Bin,	// a binary operator (+ -)
	Rel,	// a relation (= < >)
	Open,	// an opening delimiter
	Close,	// a closing delimiter
	Punct,	// punctuation
}

/// An accent riding above (or a rule below) a base: the dot of a derivative, the bar of a mean, the
/// hat of an estimate, the vinculum of an overline. A glyph accent is a mark centred over the base; a
/// rule accent is a bar drawn the width of the base, over it (`overline`) or under it (`underline`).
#[derive(Clone, Debug)]
pub enum Accent {
	Over(String),	// a mark centred above the base (dot, hat, tilde, ...)
	OverRule,		// a bar the width of the base, above it (an overline)
	UnderRule,		// a bar the width of the base, below it (an underline)
}

/// How a [`Atom::Matrix`] aligns and delimits its grid. A plain matrix centres each column and wraps
/// the grid in its delimiters; `cases` left-aligns and opens a single brace; an alignment block (a
/// multi-line display equation broken at `\` and aligned at `&`) alternates right- then left-aligned
/// columns with no gap at the alignment point, and carries no delimiters.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MatKind {
	Matrix,
	Cases,
	Align,
}

/// A node of the maths tree. `Sym` is the leaf: its text and its class. The recursive variants build
/// rows, fractions, scripts, accents and grids; grouping `{ ... }` is a [`Atom::Row`].
#[derive(Clone, Debug)]
pub enum Atom {
	Sym(String, Class),
	Text(String),		// an upright roman run: a quoted string, a `text()`/`upright()`, `d` of a differential
	Space(i32),			// a fixed horizontal space, in thousandths of an em (a `quad`, a `thin`, an `#h(..)`)
	Row(Vec<Atom>),
	Frac {
		num:	Box<Atom>,
		den:	Box<Atom>,
	},
	Script {
		base:	Box<Atom>,
		sup:	Option<Box<Atom>>,
		sub:	Option<Box<Atom>>,
	},
	Sqrt(Box<Atom>),	// a square root: the radical sign and a vinculum over the radicand
	Fence {
		left:	char,	// the opening delimiter, grown to the body
		body:	Box<Atom>,
		right:	char,	// the closing delimiter, grown to the body
	},
	Accent {
		base:	Box<Atom>,
		mark:	Accent,
	},
	Matrix {
		rows:	Vec<Vec<Atom>>,
		left:	Option<char>,	// an opening delimiter grown to the grid, or none
		right:	Option<char>,	// a closing delimiter grown to the grid, or none
		kind:	MatKind,
	},
}

impl Atom {
	pub fn sym<S: Into<String>>(text: S, class: Class) -> Self {
		Atom::Sym(text.into(), class)
	}

	/// A variable: ordinary, and set italic when it is a single letter.
	pub fn var<S: Into<String>>(text: S) -> Self { Atom::Sym(text.into(), Class::Ord) }

	/// A number: ordinary, but set upright because its text is not a single letter.
	pub fn num<S: Into<String>>(text: S) -> Self { Atom::Sym(text.into(), Class::Ord) }

	pub fn op<S: Into<String>>(text: S) -> Self { Atom::Sym(text.into(), Class::Op) }
	pub fn bin<S: Into<String>>(text: S) -> Self { Atom::Sym(text.into(), Class::Bin) }
	pub fn rel<S: Into<String>>(text: S) -> Self { Atom::Sym(text.into(), Class::Rel) }
	pub fn open<S: Into<String>>(text: S) -> Self { Atom::Sym(text.into(), Class::Open) }
	pub fn close<S: Into<String>>(text: S) -> Self { Atom::Sym(text.into(), Class::Close) }
	pub fn punct<S: Into<String>>(text: S) -> Self { Atom::Sym(text.into(), Class::Punct) }

	pub fn row(items: Vec<Atom>) -> Self { Atom::Row(items) }

	pub fn frac(num: Atom, den: Atom) -> Self {
		Atom::Frac { num: Box::new(num), den: Box::new(den) }
	}

	pub fn sup(base: Atom, sup: Atom) -> Self {
		Atom::Script { base: Box::new(base), sup: Some(Box::new(sup)), sub: None }
	}

	pub fn sub(base: Atom, sub: Atom) -> Self {
		Atom::Script { base: Box::new(base), sup: None, sub: Some(Box::new(sub)) }
	}

	pub fn subsup(base: Atom, sub: Atom, sup: Atom) -> Self {
		Atom::Script { base: Box::new(base), sup: Some(Box::new(sup)), sub: Some(Box::new(sub)) }
	}

	/// A square root over the radicand.
	pub fn sqrt(radicand: Atom) -> Self {
		Atom::Sqrt(Box::new(radicand))
	}

	/// A body between a pair of delimiters that grow to it, such as parentheses around a tall fraction.
	pub fn fence(left: char, body: Atom, right: char) -> Self {
		Atom::Fence { left, body: Box::new(body), right }
	}

	/// An upright roman run: a quoted string or a `text()`/`upright()`, never set in the maths italic.
	pub fn text<S: Into<String>>(text: S) -> Self { Atom::Text(text.into()) }

	/// A fixed horizontal space, its width in thousandths of an em (1000 = one em).
	pub fn space(milli_em: i32) -> Self { Atom::Space(milli_em) }

	/// A base with an accent riding above it, or a rule over or under it.
	pub fn accent(base: Atom, mark: Accent) -> Self {
		Atom::Accent { base: Box::new(base), mark }
	}

	/// A grid of atoms: a matrix, a `cases`, or a multi-line alignment block.
	pub fn matrix(rows: Vec<Vec<Atom>>, left: Option<char>, right: Option<char>, kind: MatKind) -> Self {
		Atom::Matrix { rows, left, right, kind }
	}
}

/// A size level, TeX's styles collapsed to the three that change the type size: running maths, a
/// script, and a script of a script. A fraction sets its parts one level down; a script sets its
/// scripts one level down; the smallest level does not shrink further.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Level {
	Text,
	Script,
	ScriptScript,
}

impl Level {
	fn smaller(self) -> Level {
		match self {
			Level::Text			=> Level::Script,
			Level::Script		=> Level::ScriptScript,
			Level::ScriptScript	=> Level::ScriptScript,
		}
	}
}

/// The type size at a level: the body size at text, and the two smaller sizes from the maths font's
/// own `scriptPercentScaleDown` and `scriptScriptPercentScaleDown`. Latin Modern's are 70 and 50, the
/// plain-TeX ratios, but a font may choose otherwise; the font's word is taken when it has a MATH table.
fn size_for(style: &Style, level: Level) -> Sp {
	let body = style.body_size.raw();
	let (s_pct, ss_pct) = script_percents();
	match level {
		Level::Text			=> style.body_size,
		Level::Script		=> Sp(body * s_pct / 100),
		Level::ScriptScript	=> Sp(body * ss_pct / 100),
	}
}

/// The maths font's two script scale-down percentages, or the plain-TeX 70 and 50 when the font carries
/// no MATH table. Read from the cached table, so this costs only a lookup.
fn script_percents() -> (i32, i32) {
	match math_table() {
		Ok(Some(t)) => {
			let c = t.constants();
			(c.script_percent_scale_down as i32, c.script_script_percent_scale_down as i32)
		},
		_ => (70, 50),
	}
}

/// A design-unit length scaled to a type size in scaled points, from the parsed table.
fn du(table: &Arc<MathTable>, value: i16, size_pt: f32) -> Sp {
	Sp::from_pt(table.scaled(value, size_pt) as f64)
}

/// What a drawable is: a shaped glyph run, or a horizontal bar of the given thickness (a fraction
/// rule). Both flatten to a leaf; the run to a [`LeafKind::Text`](crate::ir::LeafKind), the bar to a
/// [`LeafKind::Rule`](crate::ir::LeafKind).
enum Draw {
	Glyph(ShapedText),
	Bar(Sp),		// thickness
	Ink(Path),		// a raw outline in the y-down frame, baseline at zero: a grown delimiter or radical
}

/// One drawable within a maths box: its left `x` from the box's own left, its `width`, and `rel` --
/// the downward offset of its reference line from the maths baseline. For a glyph `rel` is the
/// baseline offset (negative raises it); for a bar it is the offset of the bar's top edge.
struct Piece {
	x:		Sp,
	width:	Sp,
	rel:	Sp,
	draw:	Draw,
}

/// A measured maths box: its drawables, its width, its extent above and below the maths baseline, and
/// the class it presents to whatever it sits beside. The internal form before [`emit`] flattens it to
/// leaves and glue.
struct MBox {
	pieces:	Vec<Piece>,
	width:	Sp,
	height:	Sp,		// above the maths baseline
	depth:	Sp,		// below the maths baseline
	class:	Class,
}

/// Lays a maths expression into one [`Node::HBox`] of leaves and glue. `display` centres the box on
/// its own baseline (the caller sets it on a line of its own); otherwise the box's baseline is seated
/// at the surrounding text's ascent, so its variables sit on the paragraph's baseline. The returned
/// HBox is meant to be unwrapped by the caller -- its `list` woven into a line, its `dims` read for the
/// line's extent -- because a maths box left nested inside a paragraph line would draw as a bare
/// rectangle.
pub fn layout(
	fonts:		Arc<FontSet>,
	style:		&Style,
	expr:		&Atom,
	display:	bool,
)
	-> Outcome<Node>
{
	let m = res!(build(style, expr, Level::Text, display));

	// The baseline's distance from the box top. A display box stands on its own, so its top is its
	// highest ink and the baseline sits `height` below it. An inline box shares the line, so its
	// baseline meets the surrounding text's baseline -- a body ascent below the line top.
	let base = if display {
		m.height
	} else {
		res!(ascent(&fonts, style.body_size))
	};

	let (nodes, dims) = emit(m, base);
	Ok(Node::HBox(BoxNode::new(nodes, dims)))
}

/// The body face's ascent at a size: how far the line top sits above the baseline, the reference an
/// inline maths box seats its baseline against.
fn ascent(fonts: &Arc<FontSet>, size: Sp) -> Outcome<Sp> {
	let sample = res!(ShapedText::new(fonts.clone(), Role::Body, Dir::Ltr, size, "0"));
	Ok(sample.dims().height)
}

/// Measures an atom into a maths box at a size level. Recursion mirrors the tree: a symbol shapes one
/// run, a row concatenates its atoms with class spacing, a fraction stacks two smaller boxes over a
/// bar or slashes them on the line, and a script hangs smaller boxes off a base. `display` distinguishes
/// a display equation, which stacks its fractions, from inline maths, which slashes them so a running
/// line never opens above a fraction.
fn build(
	style:		&Style,
	expr:		&Atom,
	level:		Level,
	display:	bool,
)
	-> Outcome<MBox>
{
	match expr {
		Atom::Sym(text, class)			=> build_sym(style, text, *class, level, display),
		Atom::Text(text)				=> build_text(style, text, level),
		Atom::Space(mem)				=> Ok(build_space(style, *mem, level)),
		Atom::Row(items)				=> build_row(style, items, level, display),
		Atom::Frac { num, den }			=> build_frac(style, num, den, level, display),
		Atom::Script { base, sup, sub }	=> build_script(style, base, sup.as_deref(), sub.as_deref(), level, display),
		Atom::Sqrt(radicand)			=> build_sqrt(style, radicand, level, display),
		Atom::Fence { left, body, right }	=> build_fence(style, *left, body, *right, level, display),
		Atom::Accent { base, mark }		=> build_accent(style, base, mark, level, display),
		Atom::Matrix { rows, left, right, kind }
										=> build_matrix(style, rows, *left, *right, *kind, level, display),
	}
}

/// Sets one symbol from the maths font. A single-letter ordinary atom is drawn from the Mathematical
/// Italic block -- a real maths italic, not the text italic -- while a function name, a digit, an
/// operator or a delimiter is the font's upright glyph. The run's ascent and descent stand in for the
/// atom's height and depth.
fn build_sym(
	style:		&Style,
	text:		&str,
	class:		Class,
	level:		Level,
	display:	bool,
)
	-> Outcome<MBox>
{
	let size		= size_for(style, level);
	// A big operator (a summation, an integral) is set from its larger display variant when it stands in
	// a display equation at running size, the way TeX's `\displaystyle` grows it; inline and in a script
	// it keeps its text-size glyph.
	if display && level == Level::Text && is_bigop(text) {
		if let Some(m) = res!(build_bigop(text, size)) {
			return Ok(m);
		}
	}
	let font		= res!(math_font());
	let shown		= math_text(text, class);
	let shaped		= res!(ShapedText::new_with_font(font, Dir::Ltr, size, &shown));
	let width		= shaped.dims().width;			// the advance, the horizontal extent
	let (height, depth)	= res!(shaped.ink_extent());	// the true ink extent, not the font's global metric
	let piece		= Piece { x: Sp::ZERO, width, rel: Sp::ZERO, draw: Draw::Glyph(shaped) };
	Ok(MBox { pieces: vec![piece], width, height, depth, class })
}

/// Is the symbol a large operator that grows in a display equation -- a summation, a product, an
/// integral, a big union or intersection?
fn is_bigop(text: &str) -> bool {
	matches!(text,
		"\u{2211}" | "\u{220F}" | "\u{2210}" |					// sum, product, coproduct
		"\u{222B}" | "\u{222C}" | "\u{222D}" | "\u{222E}" |		// integral and its multiples
		"\u{22C0}" | "\u{22C1}" | "\u{22C2}" | "\u{22C3}" |		// big and, or, intersection, union
		"\u{2A00}" | "\u{2A01}" | "\u{2A02}" | "\u{2A04}" | "\u{2A06}")	// big odot, oplus, otimes, uplus, sqcup
}

/// Does the symbol set its scripts as limits -- above and below in a display equation -- rather than
/// beside it? The big operators do, and the named limit operators (`lim`, `max`, `min`, ...), but an
/// integral does not: its bounds sit beside the sign even in display.
fn is_limits(text: &str) -> bool {
	let integral = matches!(text, "\u{222B}" | "\u{222C}" | "\u{222D}" | "\u{222E}");
	if integral {
		return false;
	}
	is_bigop(text) || matches!(text,
		"lim" | "max" | "min" | "sup" | "inf" | "det" | "gcd" | "limsup" | "liminf" | "Pr")
}

/// Draws a big operator from its tallest-but-one display variant, seated centred on the maths axis like
/// a delimiter, so a summation towers over its limits. `None` when the font offers no larger variant, in
/// which case the caller sets the plain glyph.
fn build_bigop(text: &str, size: Sp) -> Outcome<Option<MBox>> {
	let table = res!(math_table());
	let tb = match &table {
		Some(t)	=> t,
		None	=> return Ok(None),
	};
	let font		= res!(math_font());
	let size_pt		= size.to_pt() as f32;
	let base		= res!(glyph_id(&font, size, text));
	// A display big operator is about the next size up; ask for a variant around 1.4x the running size
	// and take the tallest the font offers below that, which is Latin Modern's display form.
	let target		= Sp(size.raw() * 7 / 5);
	let variant		= match tb.variant_for(base as u16, target.to_pt() as f32, size_pt) {
		Some(g) if g as u32 != base	=> g as u32,
		_							=> return Ok(None),		// no larger form: leave the plain glyph
	};
	let (path, gh, gd, gw) = res!(glyph_ink(&font, variant, size));
	let axis	= du(tb, tb.constants().axis_height, size_pt);
	// Centre the glyph's ink on the axis.
	let rel		= -axis - Sp((gd.raw() - gh.raw()) / 2);
	let piece	= Piece { x: Sp::ZERO, width: gw, rel, draw: Draw::Ink(path) };
	let height	= gh - rel;		// ink top above the baseline
	let depth	= rel + gd;		// ink bottom below the baseline
	Ok(Some(MBox { pieces: vec![piece], width: gw, height, depth, class: Class::Op }))
}

/// Sets an upright roman run from the maths font: a quoted string, a `text()`/`upright()`, the `d` of a
/// differential. Unlike a single-letter ordinary, it is never remapped to the maths italic. It presents
/// as ordinary so it abuts what follows without an operator's spacing.
fn build_text(
	style:	&Style,
	text:	&str,
	level:	Level,
)
	-> Outcome<MBox>
{
	let size		= size_for(style, level);
	let font		= res!(math_font());
	let shaped		= res!(ShapedText::new_with_font(font, Dir::Ltr, size, text));
	let width		= shaped.dims().width;
	let (height, depth)	= res!(shaped.ink_extent());
	let piece		= Piece { x: Sp::ZERO, width, rel: Sp::ZERO, draw: Draw::Glyph(shaped) };
	Ok(MBox { pieces: vec![piece], width, height, depth, class: Class::Ord })
}

/// A fixed horizontal space: a zero-ink box the requested fraction of an em wide. It presents as
/// ordinary, so it neither adds nor sheds the automatic inter-atom spacing around it -- its width is the
/// whole of the gap.
fn build_space(style: &Style, milli_em: i32, level: Level) -> MBox {
	let size	= size_for(style, level);
	let width	= Sp(size.raw() * milli_em / 1000);
	MBox { pieces: Vec::new(), width, height: Sp::ZERO, depth: Sp::ZERO, class: Class::Ord }
}

/// The characters actually shaped for an atom. A single-letter ordinary variable is remapped to its
/// Mathematical Italic codepoint, so `x` draws as a maths italic from the maths font; everything else
/// -- a digit, a multi-letter name, an operator -- is shaped as written, upright.
fn math_text(text: &str, class: Class) -> String {
	if class == Class::Ord {
		let mut it = text.chars();
		if let (Some(c), None) = (it.next(), it.next()) {
			if let Some(m) = math_italic(c) {
				return m.to_string();
			}
		}
	}
	text.to_string()
}

/// The Mathematical Italic codepoint of a Latin letter, or `None` for anything else. The italic small
/// `h` is the one hole in the block -- U+1D455 is unassigned -- and lives at U+210E, the Planck
/// constant, which every maths font draws as the italic h.
fn math_italic(c: char) -> Option<char> {
	let cp = match c {
		'h'			=> 0x210E,
		'a'..='z'	=> 0x1D44E + (c as u32 - 'a' as u32),
		'A'..='Z'	=> 0x1D434 + (c as u32 - 'A' as u32),
		_			=> return None,
	};
	char::from_u32(cp)
}

/// Concatenates a row's atoms left to right, opening a class-driven space before each atom but the
/// first. The row presents as ordinary to whatever encloses it.
fn build_row(
	style:		&Style,
	items:		&[Atom],
	level:		Level,
	display:	bool,
)
	-> Outcome<MBox>
{
	let size	= size_for(style, level);
	let mut pieces:	Vec<Piece>	= Vec::new();
	let mut cursor				= Sp::ZERO;
	let mut height				= Sp::ZERO;
	let mut depth				= Sp::ZERO;
	let mut prev:	Option<Class>	= None;

	for atom in items {
		let m = res!(build(style, atom, level, display));
		// TeX's binary-operator cancellation: a Bin with no atom to its left to bind -- at the row's
		// start, or after another operator, a relation or an opening -- is not binary but a sign on the
		// atom that follows, so it is retyped Ord and sheds its medium space. This is what turns the
		// leading minus of `-b` from a subtraction with a gap into a tight unary sign.
		let cls = match m.class {
			Class::Bin => match prev {
				None => Class::Ord,
				Some(Class::Ord) | Some(Class::Close) => Class::Bin,
				Some(_) => Class::Ord,
			},
			other => other,
		};
		if let Some(pc) = prev {
			cursor += space_between(pc, cls, size);
		}
		place_into(&mut pieces, m.pieces, cursor, Sp::ZERO);
		cursor += m.width;
		if m.height > height { height = m.height; }
		if m.depth > depth { depth = m.depth; }
		prev = Some(cls);
	}
	Ok(MBox { pieces, width: cursor, height, depth, class: Class::Ord })
}

/// The inter-atom space set between two classes, in scaled points at the running size. TeX measures
/// these in `mu` (eighteen to the em): a thin space is three, a medium four, a thick five. The table
/// is simplified -- it does not reclassify a binary operator to ordinary at a row edge or beside
/// another operator, which real maths spacing does -- but it sets the visible cases: a relation gets
/// thick space, a binary operator medium, an operator or punctuation thin.
fn space_between(left: Class, right: Class, size: Sp) -> Sp {
	let mu = mu_between(left, right);
	Sp(size.raw() * mu / 18)
}

/// The `mu` count between two atom classes, from TeX's spacing table, reduced to the pairs the text
/// faces set.
fn mu_between(left: Class, right: Class) -> i32 {
	use Class::*;
	match (left, right) {
		(Rel, _) | (_, Rel)		=> 5,	// thick around a relation
		(Bin, _) | (_, Bin)		=> 4,	// medium around a binary operator
		(Ord, Op) | (Op, Ord)	=> 3,	// thin between an operator and an ordinary
		(Op, Op)				=> 3,
		(Punct, _)				=> 3,	// thin after punctuation
		_						=> 0,	// delimiters and ordinaries abut
	}
}

/// Stacks a numerator over a denominator, centred over a bar seated on the maths axis. Following TeX's
/// style rule, a display fraction's immediate parts stay full size (Display's numerator style is Text)
/// while a script-level fraction's parts step down; the shifts, the gaps, the rule thickness and the
/// axis all come from the font's MATH constants -- the display-style variants of the fraction metrics --
/// falling back to the plain-TeX guesses only when the font has no table.
fn build_frac(
	style:		&Style,
	num:		&Atom,
	den:		&Atom,
	level:		Level,
	display:	bool,
)
	-> Outcome<MBox>
{
	// Inline, a fraction is slashed on the line -- numerator, solidus, denominator abreast -- so it
	// keeps within the line's height and never opens a gap above it. Only a display fraction stacks.
	if !display {
		let items = vec![
			num.clone(),
			Atom::Sym("/".to_string(), Class::Ord),
			den.clone(),
		];
		// A text/inline fraction's parts drop one size level (Text's numerator style is Script).
		return build_row(style, &items, level.smaller(), display);
	}

	let size	= size_for(style, level);
	let size_pt	= size.to_pt() as f32;
	let table	= res!(math_table());

	// TeX's style progression: at display style (level Text here) the parts stay full size; a fraction
	// nested inside a script steps its parts down. The parts keep the block (display) context so a big
	// operator in a numerator still sets its limits below and a nested fraction still stacks, both a size
	// smaller; only the running/inline distinction was ever about slashing, and that was handled above.
	let sub		= if level == Level::Text { Level::Text } else { level.smaller() };
	let n		= res!(build(style, num, sub, display));
	let d		= res!(build(style, den, sub, display));

	// The display-style fraction metrics from the MATH table, or the plain-TeX guesses without one.
	let (axis, t, num_shift, den_shift, num_gap, den_gap) = match &table {
		Some(tb) => {
			let c = tb.constants();
			(
				du(tb, c.axis_height, size_pt),
				du(tb, c.fraction_rule_thickness, size_pt),
				du(tb, c.fraction_num_display_shift_up, size_pt),
				du(tb, c.fraction_den_display_shift_down, size_pt),
				du(tb, c.fraction_num_display_gap_min, size_pt),
				du(tb, c.fraction_denom_display_gap_min, size_pt),
			)
		},
		None => (
			Sp(size.raw() / 4),		// axis
			style.rule_thin,		// bar thickness
			Sp(size.raw() / 2),		// numerator shift up
			Sp(size.raw() / 2),		// denominator shift down
			Sp(size.raw() / 6),		// numerator gap
			Sp(size.raw() / 6),		// denominator gap
		),
	};

	// The bar's top edge as a `rel` offset down from the maths baseline; above the baseline is negative.
	// Its foot sits `t` below, at `-axis + t/2`, which the gap arithmetic below uses directly.
	let bar_top	= -axis - Sp(t.raw() / 2);

	// The numerator baseline: the font's display shift, but pushed higher if that leaves less than the
	// least gap between the numerator's foot and the bar's top.
	let num_lift	= num_shift.raw().max(axis.raw() + t.raw() / 2 + n.depth.raw() + num_gap.raw());
	let num_base	= Sp(-num_lift);
	// The denominator baseline: the font's display shift, dropped further if the gap below the bar is
	// tighter than the least.
	let den_drop	= den_shift.raw().max(d.height.raw() - axis.raw() + t.raw() / 2 + den_gap.raw());
	let den_base	= Sp(den_drop);

	let pad		= Sp(size.raw() / 8);		// a small overhang of the bar past the wider part
	let fw		= n.width.raw().max(d.width.raw());
	let width	= Sp(fw) + pad;

	let mut pieces:	Vec<Piece> = Vec::new();
	let nx = Sp((width.raw() - n.width.raw()) / 2);
	let dx = Sp((width.raw() - d.width.raw()) / 2);
	place_into(&mut pieces, n.pieces, nx, num_base);
	place_into(&mut pieces, d.pieces, dx, den_base);
	pieces.push(Piece { x: Sp::ZERO, width, rel: bar_top, draw: Draw::Bar(t) });

	let height	= n.height - num_base;	// num_base is negative, so this reaches above the baseline
	let depth	= den_base + d.depth;
	Ok(MBox { pieces, width, height, depth, class: Class::Ord })
}

/// Hangs a superscript and/or a subscript off a base. The scripts are set one size level down and
/// lifted or dropped by the font's own MATH shifts: a superscript rides at least `superscriptShiftUp`
/// but higher off a tall base or to keep its foot above `superscriptBottomMin`; a subscript drops at
/// least `subscriptShiftDown`, deeper below a base that hangs below the baseline, and never letting its
/// top climb past `subscriptTopMax`. With both present, the pair is spread so the gap between the
/// superscript's foot and the subscript's top is no less than `subSuperscriptGapMin`. Without a MATH
/// table the plain-TeX fractions of the em stand in.
fn build_script(
	style:		&Style,
	base:		&Atom,
	sup:		Option<&Atom>,
	sub:		Option<&Atom>,
	level:		Level,
	display:	bool,
)
	-> Outcome<MBox>
{
	// A big operator or a named limit operator in a display equation sets its scripts as limits, centred
	// above and below the sign rather than beside it.
	if display && level == Level::Text {
		if let Atom::Sym(text, _) = base {
			if is_limits(text) {
				return build_limits(style, base, sup, sub, level);
			}
		}
	}

	let size	= size_for(style, level);
	let size_pt	= size.to_pt() as f32;
	let em		= size.raw();
	let table	= res!(math_table());
	// The base keeps the enclosing style; the scripts step down a size and are never display style.
	let b		= res!(build(style, base, level, display));

	let mut pieces:	Vec<Piece> = Vec::new();
	place_into(&mut pieces, b.pieces, Sp::ZERO, Sp::ZERO);

	let kern	= Sp(em / 24);			// a hair between the base and its scripts
	let sx		= b.width + kern;
	let mut width	= b.width;
	let mut height	= b.height;
	let mut depth	= b.depth;

	// The MATH script shifts scaled to the base size, or the plain-TeX guesses without a table.
	let (sup_shift, sup_bottom_min, sup_drop_max, sub_shift, sub_top_max, sub_drop_min, gap_min) =
		match &table {
			Some(t) => {
				let c = t.constants();
				(
					du(t, c.superscript_shift_up, size_pt),
					du(t, c.superscript_bottom_min, size_pt),
					du(t, c.superscript_baseline_drop_max, size_pt),
					du(t, c.subscript_shift_down, size_pt),
					du(t, c.subscript_top_max, size_pt),
					du(t, c.subscript_baseline_drop_min, size_pt),
					du(t, c.sub_superscript_gap_min, size_pt),
				)
			},
			None => (
				Sp(em / 2), Sp(em / 12), Sp(em / 4), Sp(em / 4), Sp(em / 3), Sp(em / 20), Sp(em / 6),
			),
		};

	// The superscript's rise, and the subscript's drop, as positive distances from the maths baseline.
	// Computed even when only one is present, so the combined-gap step can reconcile them.
	let sup_m = match sup {
		Some(s) => Some(res!(build(style, s, level.smaller(), false))),
		None => None,
	};
	let sub_m = match sub {
		Some(s) => Some(res!(build(style, s, level.smaller(), false))),
		None => None,
	};

	let mut rise = Sp::ZERO;
	if let Some(m) = &sup_m {
		// At least the font's shift; higher off a tall base; and enough that the foot clears the
		// baseline by `superscriptBottomMin`.
		let r = sup_shift.raw()
			.max(b.height.raw() - sup_drop_max.raw())
			.max(sup_bottom_min.raw() + m.depth.raw());
		rise = Sp(r);
	}
	let mut drop = Sp::ZERO;
	if let Some(m) = &sub_m {
		// At least the font's shift; deeper below a base that descends; and enough that the top does
		// not rise past `subscriptTopMax` above the baseline.
		let d = sub_shift.raw()
			.max(b.depth.raw() + sub_drop_min.raw())
			.max(m.height.raw() - sub_top_max.raw());
		drop = Sp(d);
	}

	// With both scripts, widen the split until the vertical gap between the superscript's foot and the
	// subscript's top reaches the font's minimum, deepening the subscript rather than raising the
	// superscript, so the superscript keeps its natural height.
	if let (Some(sm), Some(bm)) = (&sup_m, &sub_m) {
		let gap = (drop.raw() - bm.height.raw()) + (rise.raw() - sm.depth.raw());
		if gap < gap_min.raw() {
			drop = Sp(drop.raw() + (gap_min.raw() - gap));
		}
	}

	if let Some(m) = sup_m {
		place_into(&mut pieces, m.pieces, sx, Sp(-rise.raw()));
		let w = sx + m.width;
		if w > width { width = w; }
		let top = rise + m.height;
		if top > height { height = top; }
	}
	if let Some(m) = sub_m {
		place_into(&mut pieces, m.pieces, sx, drop);
		let w = sx + m.width;
		if w > width { width = w; }
		let bottom = drop + m.depth;
		if bottom > depth { depth = bottom; }
	}

	Ok(MBox { pieces, width, height, depth, class: b.class })
}

/// The maths axis at a size: the height a fraction bar, a relation and a grown delimiter centre on,
/// from the font's MATH table or the plain-TeX quarter-em without one.
fn axis_height(table: &Option<Arc<MathTable>>, size: Sp, size_pt: f32) -> Sp {
	match table {
		Some(t)	=> Sp::from_pt(t.scaled(t.constants().axis_height, size_pt) as f64),
		None	=> Sp(size.raw() / 4),
	}
}

/// Sets a big operator with its scripts as limits, centred above and below the sign. The operator keeps
/// the running size (and grows to its display variant through [`build`]); each limit is set one size
/// level down and centred on the operator, a small gap clear of it.
fn build_limits(
	style:	&Style,
	base:	&Atom,
	sup:	Option<&Atom>,
	sub:	Option<&Atom>,
	level:	Level,
)
	-> Outcome<MBox>
{
	let size	= size_for(style, level);
	let gap		= Sp(size.raw() / 6);		// clearance between the operator and a limit
	// The operator itself, at display size (a summation grows to its larger variant here).
	let op		= res!(build(style, base, level, true));
	let up		= match sup {
		Some(a)	=> Some(res!(build(style, a, level.smaller(), false))),
		None	=> None,
	};
	let lo		= match sub {
		Some(a)	=> Some(res!(build(style, a, level.smaller(), false))),
		None	=> None,
	};

	let mut width	= op.width;
	if let Some(m) = &up { if m.width > width { width = m.width; } }
	if let Some(m) = &lo { if m.width > width { width = m.width; } }

	let mut pieces:	Vec<Piece> = Vec::new();
	let centre = |w: Sp| Sp((width.raw() - w.raw()) / 2);
	place_into(&mut pieces, op.pieces, centre(op.width), Sp::ZERO);
	let mut height	= op.height;
	let mut depth	= op.depth;

	if let Some(m) = up {
		// The upper limit's baseline, so its foot clears the operator's top by `gap`.
		let drel	= -(op.height + gap + m.depth);
		let top		= -drel + m.height;
		place_into(&mut pieces, m.pieces, centre(m.width), drel);
		if top > height { height = top; }
	}
	if let Some(m) = lo {
		// The lower limit's baseline, so its top clears the operator's foot by `gap`.
		let drel	= op.depth + gap + m.height;
		let bottom	= drel + m.depth;
		place_into(&mut pieces, m.pieces, centre(m.width), drel);
		if bottom > depth { depth = bottom; }
	}

	Ok(MBox { pieces, width, height, depth, class: Class::Op })
}

/// Sets an accent over a base, or a rule over or under it. A glyph accent is shaped from the maths font
/// and centred over the base, its foot a hair above the base's ink; an overline or underline is a bar
/// the width of the base, a small gap clear of it. The base keeps its size and style.
fn build_accent(
	style:		&Style,
	base:		&Atom,
	mark:		&Accent,
	level:		Level,
	display:	bool,
)
	-> Outcome<MBox>
{
	let size	= size_for(style, level);
	let size_pt	= size.to_pt() as f32;
	let table	= res!(math_table());
	let b		= res!(build(style, base, level, display));

	let rule = match &table {
		Some(t)	=> Sp::from_pt(t.scaled(t.constants().fraction_rule_thickness, size_pt) as f64),
		None	=> style.rule_thin,
	};
	let gap		= Sp(size.raw() / 12);		// clearance between the base and its accent or rule

	let mut pieces:	Vec<Piece> = Vec::new();
	place_into(&mut pieces, b.pieces, Sp::ZERO, Sp::ZERO);
	let mut height	= b.height;
	let mut depth	= b.depth;

	match mark {
		Accent::OverRule => {
			let bar_top	= -(b.height + gap + rule);
			pieces.push(Piece { x: Sp::ZERO, width: b.width, rel: bar_top, draw: Draw::Bar(rule) });
			height = b.height + gap + rule;
		},
		Accent::UnderRule => {
			let bar_top	= b.depth + gap;
			pieces.push(Piece { x: Sp::ZERO, width: b.width, rel: bar_top, draw: Draw::Bar(rule) });
			depth = b.depth + gap + rule;
		},
		Accent::Over(glyph) => {
			// A spacing-accent glyph carries its own high position in the font, so it is placed by its
			// actual outline rather than by shaping: the mark is dropped until its ink foot sits a small
			// gap above the base's ink top, then centred over the base's width.
			let font		= res!(math_font());
			let gid			= res!(glyph_id(&font, size, glyph));
			let (path, _, _, gw)	= res!(glyph_ink(&font, gid, size));
			let (ink_top, ink_bottom) = match path.bounds(&Transform::IDENTITY) {
				Some(bd)	=> (Sp::from_pt(bd.y0 as f64), Sp::from_pt(bd.y1 as f64)),
				None		=> (Sp::ZERO, Sp::ZERO),
			};
			// The ink foot wants to land a gap above the base top (negative is up).
			let target		= Sp(-(b.height.raw() + gap.raw()));
			let dy			= target - ink_bottom;
			let moved		= res!(path.transform(&Transform::translate(0.0, dy.to_pt() as f32)));
			let x			= Sp((b.width.raw() - gw.raw()) / 2);
			pieces.push(Piece { x, width: gw, rel: Sp::ZERO, draw: Draw::Ink(moved) });
			let top			= Sp(-(ink_top.raw() + dy.raw()));
			if top > height { height = top; }
		},
	}

	Ok(MBox { pieces, width: b.width, height, depth, class: Class::Ord })
}

/// Sets a grid of atoms: a matrix, a `cases`, or a multi-line alignment block. Every cell is measured,
/// the columns sized to their widest cell and the rows to their tallest; the grid is stacked with a row
/// gap and centred on the maths axis, each column aligned by the grid's kind. A matrix and a `cases`
/// then wrap the grid in delimiters grown to it. An alignment block carries none and butts its columns
/// at the alignment point, so `a &= b` reads as one line.
fn build_matrix(
	style:		&Style,
	rows:		&[Vec<Atom>],
	left:		Option<char>,
	right:		Option<char>,
	kind:		MatKind,
	level:		Level,
	_display:	bool,		// a grid sets its own cell style by kind, not the enclosing display flag
)
	-> Outcome<MBox>
{
	let size	= size_for(style, level);
	let size_pt	= size.to_pt() as f32;
	let table	= res!(math_table());
	let axis	= axis_height(&table, size, size_pt);

	let nrows	= rows.len();
	if nrows == 0 {
		return Ok(MBox { pieces: Vec::new(), width: Sp::ZERO, height: Sp::ZERO, depth: Sp::ZERO, class: Class::Ord });
	}
	let ncols	= rows.iter().map(|r| r.len()).max().unwrap_or(0);
	// A cell keeps the grid's level; an alignment line is display style (its fractions stack), a matrix
	// or a `cases` cell is text style.
	let cell_display = kind == MatKind::Align;

	// Measure every cell.
	let mut cells:	Vec<Vec<MBox>> = Vec::with_capacity(nrows);
	for row in rows {
		let mut cs = Vec::with_capacity(ncols);
		for c in 0..ncols {
			let m = match row.get(c) {
				Some(a)	=> res!(build(style, a, level, cell_display)),
				None	=> MBox { pieces: Vec::new(), width: Sp::ZERO, height: Sp::ZERO, depth: Sp::ZERO, class: Class::Ord },
			};
			cs.push(m);
		}
		cells.push(cs);
	}

	// Column widths and per-row height and depth.
	let mut colw	= vec![Sp::ZERO; ncols];
	let mut rowh	= vec![Sp::ZERO; nrows];
	let mut rowd	= vec![Sp::ZERO; nrows];
	for (ri, row) in cells.iter().enumerate() {
		for (c, m) in row.iter().enumerate() {
			if m.width > colw[c]	{ colw[c] = m.width; }
			if m.height > rowh[ri]	{ rowh[ri] = m.height; }
			if m.depth > rowd[ri]	{ rowd[ri] = m.depth; }
		}
	}

	let (col_gap, row_gap) = match kind {
		MatKind::Matrix	=> (Sp(size.raw() * 11 / 20), Sp(size.raw() * 7 / 20)),	// 0.55em, 0.35em
		MatKind::Cases	=> (Sp(size.raw() * 4 / 5),   Sp(size.raw() * 2 / 5)),	// 0.80em, 0.40em
		// An alignment line breaks at `&` before a relation (`&=`), so the gap at the alignment point is
		// a relation's thick space, matching the space the relation carries on its other side.
		MatKind::Align	=> (Sp(size.raw() * 5 / 18),  Sp(size.raw() * 2 / 5)),	// 5 mu, 0.40em
	};

	// The grid's total width.
	let mut content_w = Sp::ZERO;
	for c in 0..ncols {
		content_w += colw[c];
		if c + 1 < ncols { content_w += col_gap; }
	}

	// Each row's baseline, relative to the first row's baseline (positive down), then the shift that puts
	// the grid's vertical centre on the axis.
	let mut base_rel	= vec![Sp::ZERO; nrows];
	let mut y			= Sp::ZERO;
	for i in 0..nrows {
		base_rel[i] = y;
		if i + 1 < nrows {
			y += rowd[i] + row_gap + rowh[i + 1];
		}
	}
	let span_below	= base_rel[nrows - 1] + rowd[nrows - 1];	// first baseline to grid foot
	let span_above	= rowh[0];									// first baseline to grid top
	let centre		= Sp((span_below.raw() - span_above.raw()) / 2);
	let shift		= Sp(-axis.raw()) - centre;					// grid centre onto the axis

	let content_h	= span_above - shift;			// grid top above the baseline
	let content_d	= span_below + shift;			// grid foot below the baseline

	// Place the cells, with the content offset for a left delimiter added later.
	let mut pieces:	Vec<Piece> = Vec::new();
	for (ri, row) in cells.into_iter().enumerate() {
		let by = base_rel[ri] + shift;
		let mut cx = Sp::ZERO;
		for (c, m) in row.into_iter().enumerate() {
			let dx = match kind {
				MatKind::Cases						=> Sp::ZERO,						// left aligned
				MatKind::Align if c % 2 == 0		=> Sp(colw[c].raw() - m.width.raw()),	// right aligned
				MatKind::Align						=> Sp::ZERO,						// left aligned
				MatKind::Matrix						=> Sp((colw[c].raw() - m.width.raw()) / 2),	// centred
			};
			place_into(&mut pieces, m.pieces, cx + dx, by);
			cx += colw[c] + col_gap;
		}
	}

	// Wrap the grid in its delimiters, each grown to span the grid symmetrically about the axis.
	if left.is_none() && right.is_none() {
		return Ok(MBox { pieces, width: content_w, height: content_h, depth: content_d, class: Class::Ord });
	}
	let font	= res!(math_font());
	let target	= delim_target(content_h + content_d);
	let gap		= Sp(size.raw() / 8);

	let mut out:	Vec<Piece> = Vec::new();
	let mut cursor	= Sp::ZERO;
	let mut height	= content_h;
	let mut depth	= content_d;
	if let Some(ch) = left {
		let (w, h, d) = res!(place_delim(&font, &table, ch, target, axis, size, size_pt, &mut out, cursor));
		cursor += w + gap;
		if h > height { height = h; }
		if d > depth { depth = d; }
	}
	for p in pieces {
		out.push(Piece { x: p.x + cursor, width: p.width, rel: p.rel, draw: p.draw });
	}
	cursor += content_w;
	if let Some(ch) = right {
		cursor += gap;
		let (w, h, d) = res!(place_delim(&font, &table, ch, target, axis, size, size_pt, &mut out, cursor));
		cursor += w;
		if h > height { height = h; }
		if d > depth { depth = d; }
	}

	Ok(MBox { pieces: out, width: cursor, height, depth, class: Class::Ord })
}

/// Sets a square root: a radical sign grown to the radicand, a vinculum ruled over it, and the radicand
/// seated beneath the vinculum. The gaps, the rule thickness and the space above come from the font's
/// MATH constants when it has them; the radical sign is the tallest-fitting vertical variant the MATH
/// table offers, or the plain sign when the font carries no table.
fn build_sqrt(
	style:		&Style,
	radicand:	&Atom,
	level:		Level,
	display:	bool,
)
	-> Outcome<MBox>
{
	let size	= size_for(style, level);
	let size_pt	= size.to_pt() as f32;
	let r		= res!(build(style, radicand, level, display));
	let table	= res!(math_table());

	let (gap, rule, extra) = match &table {
		Some(t) => {
			let c = t.constants();
			(
				Sp::from_pt(t.scaled(c.radical_vertical_gap, size_pt) as f64),
				Sp::from_pt(t.scaled(c.radical_rule_thickness, size_pt) as f64),
				Sp::from_pt(t.scaled(c.radical_extra_ascender, size_pt) as f64),
			)
		},
		None => (Sp(size.raw() / 18), style.rule_thin, Sp(size.raw() / 18)),
	};

	let target	= r.height + r.depth + gap + rule;	// the radical sign must at least span this
	let font	= res!(math_font());
	let base	= res!(glyph_id(&font, size, "\u{221A}"));
	let variant	= match &table {
		Some(t)	=> t.variant_for(base as u16, target.to_pt() as f32, size_pt).map(|g| g as u32),
		None	=> None,
	}.unwrap_or(base);
	let (path, gh, gd, gw) = res!(glyph_ink(&font, variant, size));

	let mut pieces:	Vec<Piece> = Vec::new();
	let kern	= Sp(size.raw() / 24);

	// The vinculum's top edge, a rel above the baseline (negative is up): clearing the radicand by `gap`.
	let bar_top	= -(r.height + gap + rule);
	// The radical sign, seated so its ink top meets the bar top. A taller variant then reaches below the
	// radicand, the way a radical encloses it.
	let sign_rel	= bar_top + gh;
	pieces.push(Piece { x: Sp::ZERO, width: gw, rel: sign_rel, draw: Draw::Ink(path) });

	// The radicand, to the right of the sign, and the vinculum ruled across it.
	let rx = gw + kern;
	place_into(&mut pieces, r.pieces, rx, Sp::ZERO);
	pieces.push(Piece { x: gw, width: r.width + kern, rel: bar_top, draw: Draw::Bar(rule) });

	let sign_bottom	= sign_rel + gd;
	let depth		= if sign_bottom > r.depth { sign_bottom } else { r.depth };
	Ok(MBox {
		pieces,
		width:	gw + kern + r.width,
		height:	r.height + gap + rule + extra,
		depth,
		class:	Class::Ord,
	})
}

/// The height a grown delimiter must reach for a content of the given symmetric height, after LaTeX's
/// allowance that a delimiter may fall a little short of its content rather than jump a whole variant
/// larger. The least acceptable clearance is the greater of `DelimiterFactor` (901/1000) of the content
/// and the content less `DelimiterShortfall` (5 pt); the caller then takes the tightest variant reaching
/// it. Covering the content outright instead runs a fence one variant larger than the New CM oracle
/// wherever the content sits just above a variant -- the oversize the fidelity sweep measured -- because
/// Latin Modern's variant ladder is coarse and the next size up overshoots by 15-20 %.
fn delim_target(content: Sp) -> Sp {
	let factor		= Sp(content.raw() * 901 / 1000);
	let shortfall	= Sp::from_pt(5.0);
	let floored		= if content > shortfall { content - shortfall } else { Sp::ZERO };
	if factor > floored { factor } else { floored }
}

/// Sets a body between a pair of delimiters grown to it. The delimiters span symmetrically about the
/// maths axis, tall enough to cover the body's reach above and below that axis, less LaTeX's shortfall
/// allowance ([`delim_target`]); each is the tightest-fitting vertical variant the MATH table offers, or
/// the plain delimiter when the font has no table (in which case it does not grow).
fn build_fence(
	style:		&Style,
	left:		char,
	body:		&Atom,
	right:		char,
	level:		Level,
	display:	bool,
)
	-> Outcome<MBox>
{
	let size	= size_for(style, level);
	let size_pt	= size.to_pt() as f32;
	let b		= res!(build(style, body, level, display));
	let table	= res!(math_table());
	let font	= res!(math_font());

	let axis = match &table {
		Some(t)	=> Sp::from_pt(t.scaled(t.constants().axis_height, size_pt) as f64),
		None	=> Sp(size.raw() / 4),
	};
	// The content's symmetric reach about the axis: a delimiter is centred on the axis, so it must span
	// twice the body's greater reach from it.
	let above	= b.height - axis;
	let below	= b.depth + axis;
	let half	= if above > below { above } else { below };
	let content	= half + half;

	let target	= delim_target(content);

	let mut pieces:	Vec<Piece> = Vec::new();
	let gap			= Sp(size.raw() / 12);

	let (lw, lh, ld) = res!(place_delim(&font, &table, left, target, axis, size, size_pt, &mut pieces, Sp::ZERO));
	let mut cursor = lw + gap;
	let body_h = b.height;
	let body_d = b.depth;
	let body_w = b.width;
	place_into(&mut pieces, b.pieces, cursor, Sp::ZERO);
	cursor += body_w + gap;
	let (rw, rh, rd) = res!(place_delim(&font, &table, right, target, axis, size, size_pt, &mut pieces, cursor));
	cursor += rw;

	Ok(MBox {
		pieces,
		width:	cursor,
		height:	body_h.max(lh).max(rh),
		depth:	body_d.max(ld).max(rd),
		class:	Class::Ord,
	})
}

/// Places one delimiter for [`build_fence`], centred on the maths axis, and returns its width and its
/// reach above and below the baseline.
fn place_delim(
	font:		&Arc<Font>,
	table:		&Option<Arc<MathTable>>,
	ch:			char,
	target:		Sp,
	axis:		Sp,
	size:		Sp,
	size_pt:	f32,
	pieces:		&mut Vec<Piece>,
	x:			Sp,
)
	-> Outcome<(Sp, Sp, Sp)>
{
	let s		= ch.to_string();
	let base	= res!(glyph_id(font, size, &s));
	let variant	= match table {
		Some(t)	=> t.variant_for(base as u16, target.to_pt() as f32, size_pt).map(|g| g as u32),
		None	=> None,
	}.unwrap_or(base);
	let (path, gh, gd, gw) = res!(glyph_ink(font, variant, size));

	// Centre the glyph's ink on the axis: a glyph at rel R has its ink centre at R + (gd - gh)/2, wanted
	// at -axis (the axis, above the baseline).
	let rel			= -axis - Sp((gd.raw() - gh.raw()) / 2);
	pieces.push(Piece { x, width: gw, rel, draw: Draw::Ink(path) });

	let height	= gh - rel;		// ink top above the baseline, as a positive reach
	let depth	= rel + gd;		// ink bottom below the baseline
	Ok((gw, height, depth))
}

/// Copies a child box's drawables into a parent, offset right by `dx` and down by `drel`. This is the
/// one move composition needs: a row slides a child along, a fraction lifts and lowers its parts, a
/// script hangs them off the base.
fn place_into(dst: &mut Vec<Piece>, src: Vec<Piece>, dx: Sp, drel: Sp) {
	for p in src {
		dst.push(Piece { x: p.x + dx, width: p.width, rel: p.rel + drel, draw: p.draw });
	}
}

/// Flattens a measured box to leaves and glue. Drawables are laid left to right; the glue before each
/// carries the jump from the running cursor to the drawable's `x`, which may be negative where a
/// numerator sits back over its denominator. Each leaf is shifted to `base + rel`, seating it on the
/// line at its computed height above or below the baseline. A trailing glue pads the cursor out to the
/// box width, so whatever follows the maths abuts it cleanly.
fn emit(mbox: MBox, base: Sp) -> (Vec<Node>, Dims) {
	let mut pieces = mbox.pieces;
	pieces.sort_by_key(|p| p.x.raw());

	let mut nodes:	Vec<Node>	= Vec::new();
	let mut cursor				= Sp::ZERO;
	for p in pieces {
		let gap = p.x - cursor;
		if gap != Sp::ZERO {
			nodes.push(Node::Glue(Glue::fixed(gap)));
		}
		let shift = base + p.rel;
		match p.draw {
			Draw::Glyph(shaped) => {
				// The run draws its baseline at the leaf's placement y, so a zero height plus the shift
				// puts the baseline exactly at `base + rel` below the line top.
				let dims = Dims::new(p.width, Sp::ZERO, Sp::ZERO);
				nodes.push(Node::Leaf(Leaf::text_dims(shaped, dims).with_shift(shift)));
			},
			Draw::Bar(t) => {
				let dims = Dims::new(p.width, t, Sp::ZERO);
				nodes.push(Node::Leaf(Leaf::rule(dims).with_shift(shift)));
			},
			Draw::Ink(path) => {
				// A grown delimiter or radical: its flipped outline as a one-op graphic, seated on the
				// line by the same shift as a glyph. The box is zero-height, so the shift alone seats it.
				let g = Graphic::new(
					vec![DrawOp::Fill { path, colour: Rgba::BLACK }],
					Dims::new(p.width, Sp::ZERO, Sp::ZERO));
				nodes.push(Node::Leaf(Leaf::graphic(g).with_shift(shift)));
			},
		}
		cursor = p.x + p.width;
	}
	if cursor < mbox.width {
		nodes.push(Node::Glue(Glue::fixed(mbox.width - cursor)));
	}

	// The reported extent is the maths box's own: its height above the baseline and its depth below.
	// The caller reads these -- an inline caller compares the height against the surrounding ascent to
	// find how far the maths climbs above the line, a display caller takes the height as the box's own.
	let dims = Dims::new(mbox.width, mbox.height, mbox.depth);
	(nodes, dims)
}
