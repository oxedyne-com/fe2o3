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
//! The MATH-font boundary, stated plainly. A real OpenType MATH font carries an italic maths alphabet,
//! glyph variants that grow a delimiter or a radical to its content, and a table of layout constants
//! (the axis height, the fraction rule thickness, the script shifts, the spacing). The embedded text
//! faces carry none of this. So this module approximates: a variable is set in the text italic rather
//! than a maths italic; the axis is a quarter of the em rather than the font's `AxisHeight`; the rule
//! thickness and the inter-atom spaces are TeX's plain defaults, not the face's; and a delimiter is
//! whatever the text face draws at the running size, with no growth. Radicals, big operators, growing
//! delimiters, matrices and a maths parser are later work; see the crate's phase notes.

use crate::doc::Style;
use crate::font::ShapedText;
use crate::ir::{
	BoxNode,
	Dims,
	Glue,
	Leaf,
	Node,
	Sp,
};

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_font::{
	face::Role,
	font::Font,
	set::FontSet,
	shape::Dir,
};

use std::sync::{
	Arc,
	OnceLock,
};

// The maths font: Latin Modern Math, the OpenType form of Computer Modern -- the faces TeX sets
// mathematics in -- so a variable, an operator and a radical wear the letterforms a reader knows from
// every mathematics paper. It carries the Mathematical Italic block, the upright operators and the
// large symbols, and an OpenType MATH table of layout constants and grown-delimiter variants. This
// increment draws its glyphs; reading the MATH table for the constants and the growing delimiters is
// the next step.
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

/// A node of the maths tree. `Sym` is the leaf: its text and its class. The recursive variants build
/// rows, fractions and scripts; grouping `{ ... }` is a [`Atom::Row`].
#[derive(Clone, Debug)]
pub enum Atom {
	Sym(String, Class),
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

/// The type size at a level: the body size at text, seven tenths of it at script, half at
/// script-of-script -- the ratios plain TeX uses, since the text faces carry no optical sizes.
fn size_for(style: &Style, level: Level) -> Sp {
	let body = style.body_size.raw();
	match level {
		Level::Text			=> style.body_size,
		Level::Script		=> Sp(body * 7 / 10),
		Level::ScriptScript	=> Sp(body / 2),
	}
}

/// What a drawable is: a shaped glyph run, or a horizontal bar of the given thickness (a fraction
/// rule). Both flatten to a leaf; the run to a [`LeafKind::Text`](crate::ir::LeafKind), the bar to a
/// [`LeafKind::Rule`](crate::ir::LeafKind).
enum Draw {
	Glyph(ShapedText),
	Bar(Sp),	// thickness
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
		Atom::Sym(text, class)			=> build_sym(style, text, *class, level),
		Atom::Row(items)				=> build_row(style, items, level, display),
		Atom::Frac { num, den }			=> build_frac(style, num, den, level, display),
		Atom::Script { base, sup, sub }	=> build_script(style, base, sup.as_deref(), sub.as_deref(), level, display),
	}
}

/// Sets one symbol from the maths font. A single-letter ordinary atom is drawn from the Mathematical
/// Italic block -- a real maths italic, not the text italic -- while a function name, a digit, an
/// operator or a delimiter is the font's upright glyph. The run's ascent and descent stand in for the
/// atom's height and depth.
fn build_sym(
	style:	&Style,
	text:	&str,
	class:	Class,
	level:	Level,
)
	-> Outcome<MBox>
{
	let size		= size_for(style, level);
	let font		= res!(math_font());
	let shown		= math_text(text, class);
	let shaped		= res!(ShapedText::new_with_font(font, Dir::Ltr, size, &shown));
	let width		= shaped.dims().width;			// the advance, the horizontal extent
	let (height, depth)	= res!(shaped.ink_extent());	// the true ink extent, not the font's global metric
	let piece		= Piece { x: Sp::ZERO, width, rel: Sp::ZERO, draw: Draw::Glyph(shaped) };
	Ok(MBox { pieces: vec![piece], width, height, depth, class })
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
		if let Some(pc) = prev {
			cursor += space_between(pc, m.class, size);
		}
		place_into(&mut pieces, m.pieces, cursor, Sp::ZERO);
		cursor += m.width;
		if m.height > height { height = m.height; }
		if m.depth > depth { depth = m.depth; }
		prev = Some(m.class);
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

/// Stacks a numerator over a denominator, centred over a bar seated on the maths axis. The parts are
/// set one level down (a fraction inside running text uses script size), the bar is the default rule
/// thickness, and the axis is a quarter of the em -- the height a relation and a fraction centre on.
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
		return build_row(style, &items, level, display);
	}

	let size	= size_for(style, level);
	let sub		= level.smaller();
	let n		= res!(build(style, num, sub, display));
	let d		= res!(build(style, den, sub, display));

	let axis	= Sp(size.raw() / 4);		// the maths axis, a quarter of the em
	let t		= style.rule_thin;			// the fraction bar, the default rule thickness
	let phi		= Sp(size.raw() / 6);		// clearance between a part and the bar
	let pad		= Sp(size.raw() / 6);		// the bar's overhang past the wider part

	let fw		= n.width.raw().max(d.width.raw());
	let width	= Sp(fw) + pad;

	// The bar's top edge, and the baselines of the two parts, all as `rel` offsets down from the maths
	// baseline. Above the baseline is negative.
	let bar_top		= -axis - Sp(t.raw() / 2);
	let num_base	= bar_top - phi - n.depth;			// numerator bottom clears the bar top
	let den_base	= -axis + Sp(t.raw() / 2) + phi + d.height;	// denominator top clears the bar bottom

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

/// Hangs a superscript and/or a subscript off a base. The scripts are set one level down and lifted or
/// dropped by a fixed fraction of the em -- half an em up for a superscript, a quarter down for a
/// subscript, the latter deepened when the base itself hangs below the baseline. Real script shifts
/// come from the font's maths table; these are the plain-TeX order of magnitude.
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
	let size	= size_for(style, level);
	let em		= size.raw();
	let b		= res!(build(style, base, level, display));

	let mut pieces:	Vec<Piece> = Vec::new();
	place_into(&mut pieces, b.pieces, Sp::ZERO, Sp::ZERO);

	let kern	= Sp(em / 24);			// a hair between the base and its scripts
	let sx		= b.width + kern;
	let mut width	= b.width;
	let mut height	= b.height;
	let mut depth	= b.depth;

	if let Some(s) = sup {
		let m		= res!(build(style, s, level.smaller(), display));
		let rise	= Sp(em / 2);		// half the em above the baseline
		place_into(&mut pieces, m.pieces, sx, -rise);
		let w = sx + m.width;
		if w > width { width = w; }
		let top = rise + m.height;
		if top > height { height = top; }
	}
	if let Some(s) = sub {
		let m		= res!(build(style, s, level.smaller(), display));
		let floor	= Sp(em / 4);		// a quarter of the em below the baseline
		let drop	= if b.depth + Sp(em / 20) > floor { b.depth + Sp(em / 20) } else { floor };
		place_into(&mut pieces, m.pieces, sx, drop);
		let w = sx + m.width;
		if w > width { width = w; }
		let bottom = drop + m.depth;
		if bottom > depth { depth = bottom; }
	}

	Ok(MBox { pieces, width, height, depth, class: b.class })
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
