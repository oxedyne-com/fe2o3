// SPDX-License-Identifier: Apache-2.0
// Original work copyright (c) the Typst project authors (typst-library `math/ir/item.rs`, version 0.15.1).
// Modified for Austenite: ported to Hematite's types, IR and error handling.
//! The maths IR: an equation's realised elements resolved into items (glyphs, fractions, scripts,
//! fences, tables) with their classes, sizes and spacing decided, ready for layout. A port of Typst
//! 0.15's `typst-library/src/math/ir/item.rs`; items own their children, and the few properties layout
//! adjusts in place (a glyph's stretch) sit in cells.

use crate::eval::realise::Tag;
use crate::eval::styles::StyleChain;
use crate::eval::value::Relative;
use crate::eval::content::Content;
use crate::math::class::MathClass;
use crate::math::style::MathSize;
use crate::math::props;
use crate::syntax::Span;

use std::cell::Cell;
use std::rc::Rc;

/// A relative length with its length part resolved to points: Typst's `Rel<Abs>`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rel {
	pub rel:	f64,
	pub abs:	f64,
}

impl Rel {
	pub fn one() -> Self { Self { rel: 1.0, abs: 0.0 } }
	pub fn new(rel: f64, abs: f64) -> Self { Self { rel, abs } }

	/// The length against a whole; a ratio of an infinite whole counts for nothing.
	pub fn relative_to(&self, whole: f64) -> f64 {
		let part = self.rel * whole;
		(if part.is_finite() { part } else { 0.0 }) + self.abs
	}

	pub fn of(r: &Relative, font_size: f64) -> Self {
		Self { rel: r.rel.0, abs: r.abs.resolve(font_size) }
	}
}

/// Is a relative length exactly 100%?
pub fn is_one(r: &Relative) -> bool {
	r.rel.0 == 1.0 && r.abs.abs == 0.0 && r.abs.em == 0.0
}

/// When attachments go above and below rather than to the side.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Limits {
	Never,
	Display,
	Always,
}

impl Limits {
	pub fn for_char_with_class(c: char, class: Option<MathClass>) -> Self {
		match class {
			Some(MathClass::Large) => if is_integral_char(c) { Limits::Never } else { Limits::Display },
			Some(MathClass::Relation)	=> Limits::Always,
			_							=> Limits::Never,
		}
	}

	pub fn for_class(class: MathClass) -> Self {
		match class {
			MathClass::Large	=> Limits::Display,
			MathClass::Relation	=> Limits::Always,
			_					=> Limits::Never,
		}
	}

	pub fn active(&self, styles: &StyleChain) -> bool {
		match self {
			Limits::Always	=> true,
			Limits::Display	=> props::size(styles) == MathSize::Display,
			Limits::Never	=> false,
		}
	}
}

fn is_integral_char(c: char) -> bool {
	('\u{222b}'..='\u{2233}').contains(&c) || ('\u{2a0b}'..='\u{2a1c}').contains(&c)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Position {
	Above,
	Below,
}

/// Which way a row of cells alternates alignment: right, left, right, ... or not at all.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Alternator {
	None,
	Left,
	Right,
}

impl Alternator {
	/// The current alignment, advancing to the next.
	pub fn next(&mut self) -> Alternator {
		let r = *self;
		*self = match self {
			Alternator::None	=> Alternator::None,
			Alternator::Left	=> Alternator::Right,
			Alternator::Right	=> Alternator::Left,
		};
		r
	}
}

/// Horizontal alignment resolved to start, centre or end.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fixed {
	Start,
	Center,
	End,
}

impl Fixed {
	/// The offset to align something of `extent` within free space.
	pub fn position(self, free: f64) -> f64 {
		match self {
			Fixed::Start	=> 0.0,
			Fixed::Center	=> free / 2.0,
			Fixed::End		=> free,
		}
	}
}

/// How far to stretch along one axis.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StretchInfo {
	pub target:				Rel,
	buffer:					Option<Rel>,
	pub explicit:			bool,
	pub requested_target:	Option<Relative>,
	pub short_fall:			f64,			// ems
	pub relative_to:		Option<f64>,
	pub font_size:			Option<f64>,
}

impl StretchInfo {
	pub fn new(target: Rel, short_fall: f64) -> Self {
		Self { target, buffer: None, explicit: false, requested_target: None, short_fall, relative_to: None,
			font_size: None }
	}

	pub fn from_size(size: &Relative, short_fall: f64, font_size: f64) -> Self {
		Self {
			target:				Rel::of(size, font_size),
			buffer:				None,
			explicit:			false,
			requested_target:	if is_one(size) { None } else { Some(*size) },
			short_fall,
			relative_to:		None,
			font_size:			None,
		}
	}

	// Typst's `MulAssign`: stack a further request onto this one.
	fn mul(&mut self, rhs: StretchInfo) {
		if let Some(b) = self.buffer {
			self.target = Rel::new(self.target.rel * b.rel, b.rel * self.target.abs + b.abs);
		}
		self.buffer = Some(rhs.target);
		if let Some(req) = rhs.requested_target {
			self.requested_target = Some(match self.requested_target {
				None	=> req,
				Some(t)	=> {
					let abs = crate::eval::value::Length {
						abs:	req.rel.0 * t.abs.abs + req.abs.abs,
						em:		req.rel.0 * t.abs.em + req.abs.em,
					};
					Relative { rel: crate::eval::value::Ratio(t.rel.0 * req.rel.0), abs }
				}
			});
		}
		self.explicit = self.explicit || rhs.explicit;
		self.short_fall = rhs.short_fall;
	}
}

impl Default for StretchInfo {
	fn default() -> Self { Self::new(Rel::one(), 0.0) }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Stretch {
	pub x:	Option<StretchInfo>,
	pub y:	Option<StretchInfo>,
}

impl Stretch {
	pub fn new() -> Self { Self::default() }

	pub fn with_x(mut self, info: StretchInfo) -> Self { self.x = Some(info); self }
	pub fn with_y(mut self, info: StretchInfo) -> Self { self.y = Some(info); self }

	fn axis_mut(&mut self, vertical: bool) -> &mut Option<StretchInfo> {
		if vertical { &mut self.y } else { &mut self.x }
	}

	fn axis(&self, vertical: bool) -> Option<StretchInfo> {
		if vertical { self.y } else { self.x }
	}

	pub fn update(mut self, mut info: StretchInfo) -> Self {
		info.explicit = true;
		match &mut self.x {
			Some(v)	=> v.mul(info),
			None	=> self.x = Some(info),
		}
		match &mut self.y {
			Some(v)	=> v.mul(info),
			None	=> self.y = Some(info),
		}
		self
	}

	pub fn relative_to(mut self, rel: f64, vertical: bool) -> Self {
		if let Some(info) = self.axis_mut(vertical) {
			if info.relative_to.is_none() {
				info.relative_to = Some(rel);
			}
		}
		self
	}

	pub fn font_size(mut self, size: f64, vertical: bool) -> Self {
		if let Some(info) = self.axis_mut(vertical) {
			if info.font_size.is_none() {
				info.font_size = Some(size);
			}
		}
		self
	}

	/// The request along an axis with any buffered request folded in.
	pub fn resolve(mut self, vertical: bool) -> Option<StretchInfo> {
		if let Some(info) = self.axis_mut(vertical) {
			if let Some(b) = info.buffer {
				if info.relative_to.is_some() {
					info.target = b;
				} else {
					info.target = Rel::new(info.target.rel * b.rel, b.rel * info.target.abs + b.abs);
				}
			}
		}
		self.axis(vertical)
	}

	pub fn is_explicit(self, vertical: bool) -> bool {
		self.axis(vertical).map(|i| i.explicit).unwrap_or(false)
	}
}

/// An item and the per-item layout decisions made before layout.
#[derive(Clone, Copy, Debug)]
pub struct Props {
	pub limits:				Limits,
	pub class:				Option<MathClass>,
	pub size:				MathSize,
	pub cramped:			bool,
	pub ignorant:			bool,	// transparent to spacing (a tag, a placed element)
	pub spaced:				bool,	// takes a source space either side
	pub lspace:				Option<f64>,	// ems
	pub rspace:				Option<f64>,
	pub align_form_infix:	bool,
	pub span:				Span,
}

impl Props {
	pub fn new(styles: &StyleChain, class: Option<MathClass>, span: Span) -> Self {
		Self {
			limits:				Limits::Never,
			class,
			size:				props::size(styles),
			cramped:			props::cramped(styles),
			ignorant:			false,
			spaced:				false,
			lspace:				None,
			rspace:				None,
			align_form_infix:	false,
			span,
		}
	}

	pub fn class(&self) -> MathClass { self.class.unwrap_or(MathClass::Normal) }
}

pub struct Glyph {
	pub text:			String,
	pub class:			MathClass,
	pub stretch:		Cell<Stretch>,
	pub mid_stretched:	Cell<Option<bool>>,
	pub flac:			Cell<bool>,
}

pub struct Scripts {
	pub base:	Item,
	pub t:		Option<Item>,
	pub b:		Option<Item>,
	pub tl:		Option<Item>,
	pub bl:		Option<Item>,
	pub tr:		Option<Item>,
	pub br:		Option<Item>,
}

/// A matrix augmentation: line offsets and the stroke, all resolved.
#[derive(Clone, Debug, Default)]
pub struct Augment {
	pub hline:	Vec<i64>,
	pub vline:	Vec<i64>,
	pub stroke:	Option<crate::eval::value::Stroke>,
}

pub struct Table {
	pub cells:		Vec<Vec<Row>>,
	pub gap:		(Rel, Rel),		// column, row
	pub augment:	Option<Augment>,
	pub align:		Fixed,
	pub alternator:	Alternator,
}

pub struct Cancel {
	pub base:				Item,
	pub length:				Rel,
	pub stroke:				crate::math::frame::Pen,
	pub cross:				bool,
	pub invert_first_line:	bool,
	pub angle:				crate::eval::value::Value,	// auto, an angle, or a function
}

/// The body of a fenced item: its own, or one segment of a multiline fence whose delimiters are all
/// sized against every segment together.
pub enum FencedBody {
	Owned(Item),
	Shared { index: usize, sizing: Rc<SharedSizing> },
}

impl FencedBody {
	pub fn item(&self) -> &Item {
		match self {
			FencedBody::Owned(i)					=> i,
			FencedBody::Shared { index, sizing }	=> match sizing.items.get(*index) {
				Some(i)	=> i,
				None	=> &sizing.empty,
			},
		}
	}
}

pub struct SharedSizing {
	pub items:			Vec<Item>,
	pub relative_to:	Cell<Option<f64>>,
	pub styles:			StyleChain,
	empty:				Item,
}

impl SharedSizing {
	pub fn new(items: Vec<Item>, styles: StyleChain) -> Rc<Self> {
		Rc::new(Self { items, relative_to: Cell::new(None), styles, empty: Item::Space })
	}
}

pub enum Kind {
	Group(Vec<Item>),
	Multiline { rows: Vec<Row>, centered: bool },
	Radical { radicand: Item, index: Option<Item>, sqrt: Item },
	Fenced { open: Option<Item>, close: Option<Item>, body: FencedBody, balanced: bool },
	Fraction { num: Item, denom: Item, line: bool, padding: f64 },
	Skewed { num: Item, denom: Item, slash: Item },
	Table(Box<Table>),
	Scripts(Box<Scripts>),
	Accent { base: Item, accent: Item, position: Position, dotless: bool, exact: bool },
	Cancel(Box<Cancel>),
	Line { base: Item, position: Position },
	Primes(usize),
	Text(String),
	Number(String),
	Glyph(Glyph),
	External(Content),
}

pub struct Comp {
	pub kind:	Kind,
	pub props:	Props,
	pub styles:	StyleChain,
}

/// A row split at alignment points into columns, each one item.
pub struct Row(pub Vec<Item>);

impl Row {
	pub fn len(&self) -> usize { self.0.len() }
	pub fn is_empty(&self) -> bool { self.0.is_empty() }

	pub fn pad_to(&mut self, n: usize, styles: &StyleChain) {
		while self.0.len() < n {
			self.0.push(Item::wrap(Vec::new(), styles));
		}
	}
}

pub enum Item {
	Comp(Box<Comp>),
	Spacing(f64, bool),	// points, weak
	Space,
	Tag(Tag),
}

/// An item, or a linebreak or alignment point still to be resolved into rows and columns.
pub enum Raw {
	Item(Item),
	Linebreak,
	Align,
}

impl Raw {
	pub fn is_ignorant(&self) -> bool {
		match self {
			Raw::Item(i)	=> i.is_ignorant(),
			_				=> false,
		}
	}

	pub fn into_item(self) -> Option<Item> {
		match self {
			Raw::Item(i)	=> Some(i),
			_				=> None,
		}
	}
}

impl Item {
	pub fn comp(kind: Kind, props: Props, styles: StyleChain) -> Item {
		Item::Comp(Box::new(Comp { kind, props, styles }))
	}

	/// One item as it is, several as a group.
	pub fn wrap(mut items: Vec<Item>, styles: &StyleChain) -> Item {
		if items.len() == 1 {
			if let Some(i) = items.pop() {
				return i;
			}
		}
		Item::comp(Kind::Group(items), Props::new(styles, None, Span::detached()), styles.clone())
	}

	pub fn limits(&self) -> Limits {
		match self {
			Item::Comp(c)	=> c.props.limits,
			_				=> Limits::Never,
		}
	}

	pub fn class(&self) -> MathClass { self.raw_class().unwrap_or(MathClass::Normal) }

	pub fn raw_class(&self) -> Option<MathClass> {
		match self {
			Item::Comp(c)						=> c.props.class,
			Item::Spacing(..) | Item::Space		=> Some(MathClass::Space),
			Item::Tag(_)						=> Some(MathClass::Special),
		}
	}

	/// The class the item presents on its right: a fence with a closing delimiter closes.
	pub fn rclass(&self) -> MathClass {
		if let Item::Comp(c) = self {
			if let Kind::Fenced { close: Some(_), .. } = &c.kind {
				if c.props.class.is_none() {
					return MathClass::Closing;
				}
			}
		}
		self.class()
	}

	pub fn lclass(&self) -> MathClass {
		if let Item::Comp(c) = self {
			if let Kind::Fenced { open: Some(_), .. } = &c.kind {
				if c.props.class.is_none() {
					return MathClass::Opening;
				}
			}
		}
		self.class()
	}

	pub fn size(&self) -> Option<MathSize> {
		match self {
			Item::Comp(c)	=> Some(c.props.size),
			_				=> None,
		}
	}

	pub fn is_spaced(&self) -> bool {
		if self.class() == MathClass::Fence {
			return true;
		}
		match self {
			Item::Comp(c)	=> c.props.spaced && matches!(c.props.class(), MathClass::Normal | MathClass::Alphabetic),
			_				=> false,
		}
	}

	pub fn is_ignorant(&self) -> bool {
		match self {
			Item::Comp(c)	=> c.props.ignorant,
			Item::Tag(_)	=> true,
			_				=> false,
		}
	}

	pub fn span(&self) -> Span {
		match self {
			Item::Comp(c)	=> c.props.span,
			_				=> Span::detached(),
		}
	}

	pub fn styles(&self) -> Option<&StyleChain> {
		match self {
			Item::Comp(c)	=> Some(&c.styles),
			_				=> None,
		}
	}

	fn glyph(&self) -> Option<&Glyph> {
		match self {
			Item::Comp(c)	=> match &c.kind {
				Kind::Glyph(g)	=> Some(g),
				_				=> None,
			},
			_				=> None,
		}
	}

	pub fn mid_stretched(&self) -> Option<bool> {
		self.glyph().and_then(|g| g.mid_stretched.get())
	}

	pub fn is_multiline(&self) -> bool {
		matches!(self, Item::Comp(c) if matches!(c.kind, Kind::Multiline { .. }))
	}

	/// A group's items, or the item itself.
	pub fn as_slice(&self) -> &[Item] {
		if let Item::Comp(c) = self {
			if let Kind::Group(items) = &c.kind {
				return items;
			}
		}
		std::slice::from_ref(self)
	}

	pub fn set_limits(&mut self, limits: Limits) {
		if let Item::Comp(c) = self {
			c.props.limits = limits;
		}
	}

	pub fn set_class(&mut self, class: MathClass) {
		if let Item::Comp(c) = self {
			c.props.class = Some(class);
		}
	}

	pub fn set_explicit_class(&mut self, class: MathClass) {
		self.set_class(class);
		if let Item::Comp(c) = self {
			let display = c.props.size == MathSize::Display;
			if let Kind::Glyph(g) = &mut c.kind {
				g.class = class;
				if class == MathClass::Large && display && !g.stretch.get().is_explicit(true) {
					let s = g.stretch.get().with_y(StretchInfo::default());
					g.stretch.set(s);
				}
			}
		}
	}

	pub fn set_lspace(&mut self, space: Option<f64>) {
		if let Item::Comp(c) = self {
			if c.props.lspace.is_none() {
				c.props.lspace = space;
			}
		}
	}

	pub fn set_rspace(&mut self, space: Option<f64>) {
		if let Item::Comp(c) = self {
			if c.props.rspace.is_none() {
				c.props.rspace = space;
			}
		}
	}

	pub fn with_multiline_centering(mut self) -> Self {
		if let Item::Comp(c) = &mut self {
			if let Kind::Multiline { centered, .. } = &mut c.kind {
				*centered = true;
			}
		}
		self
	}

	pub fn set_mid_stretched(&self, v: Option<bool>) {
		if let Some(g) = self.glyph() {
			g.mid_stretched.set(v);
		}
	}

	/// Replaces the stretch request, marking it explicit.
	pub fn set_stretch(&self, mut stretch: Stretch) {
		if let Some(i) = &mut stretch.x {
			i.explicit = true;
		}
		if let Some(i) = &mut stretch.y {
			i.explicit = true;
		}
		self.replace_stretch(stretch);
	}

	pub fn replace_stretch(&self, stretch: Stretch) {
		if let Some(g) = self.glyph() {
			g.stretch.set(stretch);
		}
	}

	pub fn set_y_stretch(&self, mut info: StretchInfo) {
		if let Some(g) = self.glyph() {
			info.explicit = true;
			g.stretch.set(g.stretch.get().with_y(info));
		}
	}

	pub fn update_stretch(&self, info: StretchInfo) {
		if let Some(g) = self.glyph() {
			g.stretch.set(g.stretch.get().update(info));
		}
	}

	pub fn set_stretch_relative_to(&self, rel: f64, vertical: bool) {
		if let Some(g) = self.glyph() {
			g.stretch.set(g.stretch.get().relative_to(rel, vertical));
		}
	}

	pub fn set_stretch_font_size(&self, size: f64, vertical: bool) {
		if let Some(g) = self.glyph() {
			g.stretch.set(g.stretch.get().font_size(size, vertical));
		}
	}

	pub fn set_flac(&self) {
		if let Some(g) = self.glyph() {
			g.flac.set(true);
		}
	}
}
