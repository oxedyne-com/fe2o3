//! The flow's carry state as a fingerprint, for `--timings` only (speed programme U1, 2026-10-05).
//!
//! A page is made from the state the paginator is in when it begins and the pairs it takes. Two compiles that
//! reach a page in the same state with the same pairs lay it out alike, so the fraction of pages whose
//! `(input, entry)` match the previous compile's is the most a re-run from a page boundary could reuse. The
//! fingerprint walks every struct by hand with no `..`, so a field added later does not compile until the
//! walk takes it, and a field the walk drops fails the plants in `tests/eval_carry.rs`.
//!
//! Spans never enter: a one-letter edit moves every later span, and with it every `Location`, `Place` and
//! mark, so those hash as their presence only and are counted. Layout caches and the future input are left
//! out by name. The walk is not memoised, so its cost is an upper bound on a production fingerprint's.

use crate::eval::args::{
	Arg,
	Args,
};
use crate::eval::content::{
	Content,
	Elem,
	ElemKind,
	FieldId,
	Sequence,
	Styled,
};
use crate::eval::func::{
	Closure,
	Func,
	Param,
};
use crate::eval::intro::{
	Counter,
	CounterKey,
	State,
};
use crate::eval::locate::Locator;
use crate::eval::realise::{
	Pair,
	Tag,
};
use crate::eval::scope::{
	Binding,
	Scope,
};
use crate::eval::select::Selector;
use crate::eval::styles::{
	ChainLink,
	Property,
	Recipe,
	RecipeIndex,
	Style,
	StyleChain,
	Styles,
	Transformation,
};
use crate::eval::value::{
	Alignment,
	Angle,
	Color,
	Datetime,
	Dict,
	Direction,
	Duration,
	Fraction,
	Gradient,
	Label,
	Length,
	Module,
	Paint,
	Ratio,
	RegexValue,
	Relative,
	Stroke,
	Symbol,
	Tiling,
	Type,
	Value,
};
use crate::eval::lib::decimal::Decimal;
use crate::flow::block::{
	Align2,
	Body,
	BlockSpec,
	Child,
	CollectCfg,
	Config,
	ColumnConfig,
	Feed,
	Fixed,
	FlowCursor,
	FlowMode,
	FootnoteConfig,
	Frame,
	Item,
	LineChild,
	MultiChild,
	Nested,
	PlaceY,
	PlacedChild,
	Pending,
	Pull,
	Regions,
	Rel,
	SingleChild,
	Skip,
	Spill,
	Work,
};
use crate::flow::inline::ParSituation;
use crate::flow::page::{
	Active,
	Item as PageItem,
	Level,
	Paginator,
	Source,
};
use crate::flow::{
	RunSetup,
	Slot,
};
use crate::font::ShapedText;
use crate::ir::{
	BoxNode,
	ClipNode,
	ColumnsNode,
	Dims,
	DrawOp,
	FloatNode,
	Footnote,
	FrameNode,
	Glue,
	Graphic,
	Leaf,
	LeafKind,
	LinkTarget,
	Node,
	PageColumns,
	Penalty,
	RasterImage,
	Sp,
	TransformNode,
};
use crate::ledger::{
	AnchorId,
	AnchorKind,
	Ref,
};
use crate::memo::Fnv;
use crate::syntax::SyntaxNode;
use crate::timings::{
	Entry,
	PageRec,
};

use std::collections::VecDeque;
use std::fmt::Write;
use std::rc::Rc;
use std::sync::Arc;

// The accumulator

/// A running fingerprint and the number of locations, places and marks it masked.
pub(super) struct Fp {
	h:		Fnv,
	masked:	u32,
}

// Feeds `Debug` text straight into the hash, so a leaf value costs no allocation.
struct FnvWriter<'a>(&'a mut Fnv);

impl Write for FnvWriter<'_> {
	fn write_str(&mut self, s: &str) -> std::fmt::Result {
		self.0.write(s.as_bytes());
		Ok(())
	}
}

impl Fp {
	pub(super) fn new() -> Self { Self { h: Fnv::new(), masked: 0 } }

	// Hashes a value by its derived `Debug` text, which holds every field and so cannot omit one.
	fn dbg<T: std::fmt::Debug + ?Sized>(&mut self, v: &T) {
		let _ = write!(FnvWriter(&mut self.h), "{:?}", v);
	}

	fn tag(&mut self, t: u8) { self.h.write_u8(t); }

	fn bytes(&mut self, b: &[u8]) {
		self.h.write_usize(b.len());
		self.h.write(b);
	}

	// A location, place or mark: present, and counted, never hashed.
	fn mask(&mut self) {
		self.h.write_u8(0xfe);
		self.masked += 1;
	}

	// Presence only of an optional location, place or mark.
	fn mask_opt<T>(&mut self, o: &Option<T>) {
		match o {
			Some(_)	=> self.mask(),
			None	=> self.tag(0),
		}
	}

	// Ends the walk, adding its masked count to `into`.
	fn seal(self, into: &mut u32) -> u64 {
		*into += self.masked;
		self.h.finish()
	}
}

/// What the fingerprint walks.
pub(super) trait Walk {
	fn mix(&self, fp: &mut Fp);
}

macro_rules! by_debug {
	($($t:ty),* $(,)?) => {
		$( impl Walk for $t { fn mix(&self, fp: &mut Fp) { fp.dbg(self); } } )*
	};
}

// Leaf values with no span or location, which the derived `Debug` prints in full.
by_debug!(
	Length, Angle, Ratio, Relative, Fraction, Color, Gradient, Alignment, Direction, Symbol, Datetime,
	Duration, Decimal, Type, Rel, Align2, Dims, Glue, Penalty, Regions, CollectCfg, ElemKind, FieldId,
	RecipeIndex, Sp, Fixed, PlaceY, ParSituation, FlowMode, crate::flow::Parity,
);

macro_rules! by_word {
	($($t:ty => $f:ident),* $(,)?) => {
		$( impl Walk for $t { fn mix(&self, fp: &mut Fp) { fp.h.$f(*self as _); } } )*
	};
}

by_word!(u8 => write_u8, u32 => write_u32, u64 => write_u64, usize => write_usize, i32 => write_i32, bool => write_bool);

impl Walk for i64 { fn mix(&self, fp: &mut Fp) { fp.h.write_u64(*self as u64); } }
impl Walk for f64 { fn mix(&self, fp: &mut Fp) { fp.h.write_u64(self.to_bits()); } }
impl Walk for str { fn mix(&self, fp: &mut Fp) { fp.h.write_str(self); } }
impl Walk for String { fn mix(&self, fp: &mut Fp) { fp.h.write_str(self); } }
impl Walk for Label { fn mix(&self, fp: &mut Fp) { fp.h.write_str(self.as_str()); } }

impl<T: Walk + ?Sized> Walk for Rc<T> { fn mix(&self, fp: &mut Fp) { (**self).mix(fp); } }
impl<T: Walk + ?Sized> Walk for Arc<T> { fn mix(&self, fp: &mut Fp) { (**self).mix(fp); } }
impl<T: Walk + ?Sized> Walk for Box<T> { fn mix(&self, fp: &mut Fp) { (**self).mix(fp); } }

impl<T: Walk> Walk for Option<T> {
	fn mix(&self, fp: &mut Fp) {
		match self {
			None	=> fp.tag(0),
			Some(v)	=> {
				fp.tag(1);
				v.mix(fp);
			},
		}
	}
}

impl<T: Walk> Walk for [T] {
	fn mix(&self, fp: &mut Fp) {
		fp.h.write_usize(self.len());
		for v in self {
			v.mix(fp);
		}
	}
}

impl<T: Walk> Walk for Vec<T> { fn mix(&self, fp: &mut Fp) { self.as_slice().mix(fp); } }

impl<T: Walk> Walk for VecDeque<T> {
	fn mix(&self, fp: &mut Fp) {
		fp.h.write_usize(self.len());
		for v in self {
			v.mix(fp);
		}
	}
}

impl<A: Walk, B: Walk> Walk for (A, B) {
	fn mix(&self, fp: &mut Fp) {
		self.0.mix(fp);
		self.1.mix(fp);
	}
}

impl<A: Walk, B: Walk, C: Walk> Walk for (A, B, C) {
	fn mix(&self, fp: &mut Fp) {
		self.0.mix(fp);
		self.1.mix(fp);
		self.2.mix(fp);
	}
}

// Content

impl Walk for Content {
	fn mix(&self, fp: &mut Fp) {
		match self {
			Content::Elem(e)		=> {
				fp.tag(0);
				e.mix(fp);
			},
			Content::Sequence(s)	=> {
				fp.tag(1);
				s.mix(fp);
			},
			Content::Styled(s)		=> {
				fp.tag(2);
				s.mix(fp);
			},
		}
	}
}

impl Walk for Elem {
	fn mix(&self, fp: &mut Fp) {
		let Elem { kind, fields, label, location, span: _, guards, prepared, place } = self;	// span: moves with an edit
		kind.mix(fp);
		fields.mix(fp);
		label.mix(fp);
		fp.mask_opt(location);
		guards.mix(fp);
		prepared.mix(fp);
		fp.mask_opt(place);
	}
}

impl Walk for Sequence {
	fn mix(&self, fp: &mut Fp) {
		let Sequence { children, label, span: _, location, guards } = self;	// span: moves with an edit
		children.mix(fp);
		label.mix(fp);
		fp.mask_opt(location);
		guards.mix(fp);
	}
}

impl Walk for Styled {
	fn mix(&self, fp: &mut Fp) {
		let Styled { child, styles } = self;
		child.mix(fp);
		styles.mix(fp);
	}
}

// Values

impl Walk for Value {
	fn mix(&self, fp: &mut Fp) {
		match self {
			Value::None			=> fp.tag(0),
			Value::Auto			=> fp.tag(1),
			Value::Bool(v)		=> { fp.tag(2); v.mix(fp); },
			Value::Int(v)		=> { fp.tag(3); v.mix(fp); },
			Value::Float(v)		=> { fp.tag(4); v.mix(fp); },
			Value::Decimal(v)	=> { fp.tag(5); v.mix(fp); },
			Value::Length(v)	=> { fp.tag(6); v.mix(fp); },
			Value::Angle(v)		=> { fp.tag(7); v.mix(fp); },
			Value::Ratio(v)		=> { fp.tag(8); v.mix(fp); },
			Value::Relative(v)	=> { fp.tag(9); v.mix(fp); },
			Value::Fraction(v)	=> { fp.tag(10); v.mix(fp); },
			Value::Color(v)		=> { fp.tag(11); v.mix(fp); },
			Value::Gradient(v)	=> { fp.tag(12); v.mix(fp); },
			Value::Tiling(v)	=> { fp.tag(13); v.mix(fp); },
			Value::Stroke(v)	=> { fp.tag(14); v.mix(fp); },
			Value::Alignment(v)	=> { fp.tag(15); v.mix(fp); },
			Value::Direction(v)	=> { fp.tag(16); v.mix(fp); },
			Value::Symbol(v)	=> { fp.tag(17); v.mix(fp); },
			Value::Str(v)		=> { fp.tag(18); v.mix(fp); },
			Value::Bytes(v)		=> { fp.tag(19); fp.bytes(v); },
			Value::Label(v)		=> { fp.tag(20); v.mix(fp); },
			Value::Datetime(v)	=> { fp.tag(21); v.mix(fp); },
			Value::Duration(v)	=> { fp.tag(22); v.mix(fp); },
			Value::Version(v)	=> { fp.tag(23); v.mix(fp); },
			Value::Regex(v)		=> { fp.tag(24); v.mix(fp); },
			Value::Content(v)	=> { fp.tag(25); v.mix(fp); },
			Value::Array(v)		=> { fp.tag(26); v.mix(fp); },
			Value::Dict(v)		=> { fp.tag(27); v.mix(fp); },
			Value::Func(v)		=> { fp.tag(28); v.mix(fp); },
			Value::Args(v)		=> { fp.tag(29); v.mix(fp); },
			Value::Module(v)	=> { fp.tag(30); v.mix(fp); },
			Value::Type(v)		=> { fp.tag(31); v.mix(fp); },
			Value::Styles(v)	=> { fp.tag(32); v.mix(fp); },
			Value::Selector(v)	=> { fp.tag(33); v.mix(fp); },
			Value::Counter(v)	=> { fp.tag(34); v.mix(fp); },
			Value::State(v)		=> { fp.tag(35); v.mix(fp); },
			Value::Location(_)	=> { fp.tag(36); fp.mask(); },
		}
	}
}

impl Walk for RegexValue {
	fn mix(&self, fp: &mut Fp) {
		let RegexValue { pattern, re: _ } = self;	// the compiled form of the pattern
		pattern.mix(fp);
	}
}

impl Walk for Dict {
	fn mix(&self, fp: &mut Fp) {
		fp.h.write_usize(self.len());
		for (k, v) in self.iter() {
			k.mix(fp);
			v.mix(fp);
		}
	}
}

impl Walk for Tiling {
	fn mix(&self, fp: &mut Fp) {
		let Tiling { body, size, spacing, relative } = self;
		body.mix(fp);
		match size {
			None		=> fp.tag(0),
			Some((w, h))	=> {
				fp.tag(1);
				w.mix(fp);
				h.mix(fp);
			},
		}
		spacing.0.mix(fp);
		spacing.1.mix(fp);
		fp.dbg(relative);
	}
}

impl Walk for Paint {
	fn mix(&self, fp: &mut Fp) {
		match self {
			Paint::Color(c)		=> { fp.tag(0); c.mix(fp); },
			Paint::Gradient(g)	=> { fp.tag(1); g.mix(fp); },
			Paint::Tiling(t)	=> { fp.tag(2); t.mix(fp); },
		}
	}
}

impl Walk for Stroke {
	fn mix(&self, fp: &mut Fp) {
		let Stroke { paint, thickness, cap, join, dash, miter_limit } = self;
		paint.mix(fp);
		thickness.mix(fp);
		fp.dbg(cap);
		fp.dbg(join);
		fp.dbg(dash);
		miter_limit.mix(fp);
	}
}

impl Walk for Module {
	fn mix(&self, fp: &mut Fp) {
		let Module { name, scope, content } = self;
		name.mix(fp);
		scope.mix(fp);
		content.mix(fp);
	}
}

impl Walk for Binding {
	fn mix(&self, fp: &mut Fp) {
		let Binding { value, span: _ } = self;	// where it was bound
		value.mix(fp);
	}
}

impl Walk for Scope {
	fn mix(&self, fp: &mut Fp) {
		let Scope { map, parent } = self;
		let mut named: Vec<(&String, &Binding)> = map.iter().collect();
		named.sort_by(|a, b| a.0.cmp(b.0));
		fp.h.write_usize(named.len());
		for (k, b) in named {
			k.mix(fp);
			b.mix(fp);
		}
		// A closure's parent is the standard library, the same in every compile: its size stands for it.
		match parent {
			None		=> fp.tag(0),
			Some(p)		=> {
				fp.tag(1);
				fp.h.write_usize(p.map.len());
			},
		}
	}
}

impl Walk for Arg {
	fn mix(&self, fp: &mut Fp) {
		let Arg { span: _, value_span: _, name, value } = self;	// spans move with an edit
		name.mix(fp);
		value.mix(fp);
	}
}

impl Walk for Args {
	fn mix(&self, fp: &mut Fp) {
		let Args { span: _, items } = self;	// span moves with an edit
		items.mix(fp);
	}
}

impl Walk for SyntaxNode {
	fn mix(&self, fp: &mut Fp) {
		// Its span and warnings are not read; the kind, the text and the tree are the syntax.
		fp.dbg(&self.kind());
		self.text().mix(fp);
		self.children().mix(fp);
	}
}

impl Walk for Param {
	fn mix(&self, fp: &mut Fp) {
		match self {
			Param::Pos(n)							=> { fp.tag(0); n.mix(fp); },
			Param::Named { name, default }			=> { fp.tag(1); name.mix(fp); default.mix(fp); },
			Param::Sink(n)							=> { fp.tag(2); n.mix(fp); },
		}
	}
}

impl Walk for Closure {
	fn mix(&self, fp: &mut Fp) {
		let Closure { name, params, body, captured, span: _ } = self;	// span moves with an edit
		name.mix(fp);
		params.mix(fp);
		body.mix(fp);
		captured.mix(fp);
	}
}

impl Walk for Func {
	fn mix(&self, fp: &mut Fp) {
		match self {
			Func::Native(n)		=> { fp.tag(0); fp.dbg(n); },
			Func::Element(k)	=> { fp.tag(1); k.mix(fp); },
			Func::Closure(c)	=> { fp.tag(2); c.mix(fp); },
			Func::With(w)		=> { fp.tag(3); w.0.mix(fp); w.1.mix(fp); },
		}
	}
}

impl Walk for Counter {
	fn mix(&self, fp: &mut Fp) {
		let Counter { key } = self;
		match key {
			CounterKey::Page		=> fp.tag(0),
			CounterKey::Selector(s)	=> { fp.tag(1); s.mix(fp); },
			CounterKey::Str(s)		=> { fp.tag(2); s.mix(fp); },
		}
	}
}

impl Walk for State {
	fn mix(&self, fp: &mut Fp) {
		let State { key, init } = self;
		key.mix(fp);
		init.mix(fp);
	}
}

impl Walk for Selector {
	fn mix(&self, fp: &mut Fp) {
		match self {
			Selector::Elem(k, f)	=> { fp.tag(0); k.mix(fp); f.mix(fp); },
			Selector::Label(l)		=> { fp.tag(1); l.mix(fp); },
			Selector::Text(t)		=> { fp.tag(2); t.mix(fp); },
			Selector::Regex(r)		=> { fp.tag(3); r.mix(fp); },
			Selector::Location(_)	=> { fp.tag(4); fp.mask(); },
			Selector::Or(v)			=> { fp.tag(5); v.mix(fp); },
			Selector::And(v)		=> { fp.tag(6); v.mix(fp); },
			Selector::Before { selector, end, inclusive }	=> {
				fp.tag(7);
				selector.mix(fp);
				end.mix(fp);
				inclusive.mix(fp);
			},
			Selector::After { selector, start, inclusive }	=> {
				fp.tag(8);
				selector.mix(fp);
				start.mix(fp);
				inclusive.mix(fp);
			},
		}
	}
}

// Styles

impl Walk for Property {
	fn mix(&self, fp: &mut Fp) {
		let Property { elem, field, value, span: _, liftable, outside } = self;	// span moves with an edit
		elem.mix(fp);
		field.mix(fp);
		value.mix(fp);
		liftable.mix(fp);
		outside.mix(fp);
	}
}

impl Walk for Transformation {
	fn mix(&self, fp: &mut Fp) {
		match self {
			Transformation::Content(c)	=> { fp.tag(0); c.mix(fp); },
			Transformation::Func(f)		=> { fp.tag(1); f.mix(fp); },
			Transformation::Style(s)	=> { fp.tag(2); s.mix(fp); },
		}
	}
}

impl Walk for Recipe {
	fn mix(&self, fp: &mut Fp) {
		let Recipe { selector, transform, span: _, outside } = self;	// span moves with an edit
		selector.mix(fp);
		transform.mix(fp);
		outside.mix(fp);
	}
}

impl Walk for Style {
	fn mix(&self, fp: &mut Fp) {
		match self {
			Style::Property(p)		=> { fp.tag(0); p.mix(fp); },
			Style::Recipe(r)		=> { fp.tag(1); r.mix(fp); },
			Style::Revocation(i)	=> { fp.tag(2); i.mix(fp); },
		}
	}
}

impl Walk for Styles {
	fn mix(&self, fp: &mut Fp) {
		let Styles(list) = self;
		list.mix(fp);
	}
}

impl Walk for StyleChain {
	fn mix(&self, fp: &mut Fp) {
		let StyleChain { head } = self;
		let mut at = head.as_ref();
		while let Some(link) = at {
			let ChainLink { styles, parent } = &**link;
			fp.tag(1);
			styles.mix(fp);
			at = parent.as_ref();
		}
		fp.tag(0);
	}
}

// Realised pairs

impl Walk for Tag {
	fn mix(&self, fp: &mut Fp) {
		match self {
			Tag::Start(c)	=> { fp.tag(0); c.mix(fp); },
			Tag::End(_)		=> { fp.tag(1); fp.mask(); },
		}
	}
}

impl Walk for Pair {
	fn mix(&self, fp: &mut Fp) {
		let Pair { content, styles, tag } = self;
		content.mix(fp);
		styles.mix(fp);
		tag.mix(fp);
	}
}

// IR

impl Walk for AnchorId {
	fn mix(&self, fp: &mut Fp) {
		let AnchorId { kind, key } = self;
		fp.dbg(kind);
		match kind {
			AnchorKind::Location	=> fp.mask(),	// keyed by a location's hash
			_						=> key.mix(fp),
		}
	}
}

impl Walk for Ref {
	fn mix(&self, fp: &mut Fp) {
		match self {
			Ref::TotalPages		=> fp.tag(0),
			Ref::PageOf(a)		=> { fp.tag(1); a.mix(fp); },
			Ref::FolioOf(a)		=> { fp.tag(2); a.mix(fp); },
			Ref::IndexFolios(v)	=> { fp.tag(3); v.mix(fp); },
		}
	}
}

impl Walk for ShapedText {
	fn mix(&self, fp: &mut Fp) { self.hash_into(&mut fp.h); }
}

impl Walk for RasterImage {
	fn mix(&self, fp: &mut Fp) {
		let RasterImage { width, height, rgba } = self;
		width.mix(fp);
		height.mix(fp);
		fp.bytes(rgba);
	}
}

impl Walk for DrawOp {
	fn mix(&self, fp: &mut Fp) {
		match self {
			DrawOp::Fill { path, colour }				=> { fp.tag(0); fp.dbg(path); fp.dbg(colour); },
			DrawOp::Stroke { path, colour, width }		=> { fp.tag(1); fp.dbg(path); fp.dbg(colour); fp.dbg(width); },
			DrawOp::Image { image, x, y, w, h }			=> {
				fp.tag(2);
				image.mix(fp);
				fp.dbg(&(x, y, w, h));
			},
		}
	}
}

impl Walk for LinkTarget {
	fn mix(&self, fp: &mut Fp) {
		match self {
			LinkTarget::Uri(u)		=> { fp.tag(0); u.mix(fp); },
			LinkTarget::Anchor(a)	=> { fp.tag(1); a.mix(fp); },
		}
	}
}

impl Walk for Graphic {
	fn mix(&self, fp: &mut Fp) {
		let Graphic { ops, dims, link } = self;
		ops.mix(fp);
		dims.mix(fp);
		link.mix(fp);
	}
}

impl Walk for Footnote {
	fn mix(&self, fp: &mut Fp) {
		let Footnote { number, mark, note, height } = self;
		number.mix(fp);
		mark.mix(fp);
		note.mix(fp);
		height.mix(fp);
	}
}

impl Walk for LeafKind {
	fn mix(&self, fp: &mut Fp) {
		match self {
			LeafKind::Rule								=> fp.tag(0),
			LeafKind::Reserved(a, r, width, bold)		=> {
				fp.tag(1);
				a.mix(fp);
				r.mix(fp);
				width.mix(fp);
				bold.mix(fp);
			},
			LeafKind::Text(t)							=> { fp.tag(2); t.mix(fp); },
			LeafKind::Mark(f)							=> { fp.tag(3); f.mix(fp); },
			LeafKind::Graphic(g)						=> { fp.tag(4); g.mix(fp); },
		}
	}
}

impl Walk for Leaf {
	fn mix(&self, fp: &mut Fp) {
		let Leaf { kind, dims, shift, span } = self;
		kind.mix(fp);
		dims.mix(fp);
		shift.mix(fp);
		fp.mask_opt(span);	// a source offset
	}
}

impl Walk for BoxNode {
	fn mix(&self, fp: &mut Fp) {
		let BoxNode { list, dims } = self;
		list.mix(fp);
		dims.mix(fp);
	}
}

impl Walk for FloatNode {
	fn mix(&self, fp: &mut Fp) {
		let FloatNode { list, height, clearance, placement, scope } = self;
		list.mix(fp);
		height.mix(fp);
		clearance.mix(fp);
		fp.dbg(placement);
		fp.dbg(scope);
	}
}

impl Walk for ColumnsNode {
	fn mix(&self, fp: &mut Fp) {
		let ColumnsNode { list, count, gutter } = self;
		list.mix(fp);
		count.mix(fp);
		gutter.mix(fp);
	}
}

impl Walk for TransformNode {
	fn mix(&self, fp: &mut Fp) {
		let TransformNode { transform, list, dims } = self;
		fp.dbg(transform);
		list.mix(fp);
		dims.mix(fp);
	}
}

impl Walk for ClipNode {
	fn mix(&self, fp: &mut Fp) {
		let ClipNode { list, dims, path } = self;
		list.mix(fp);
		dims.mix(fp);
		fp.dbg(path);
	}
}

impl Walk for FrameNode {
	fn mix(&self, fp: &mut Fp) {
		let FrameNode { dims, items, parent } = self;
		dims.mix(fp);
		items.mix(fp);
		fp.mask_opt(parent);
	}
}

impl Walk for PageColumns {
	fn mix(&self, fp: &mut Fp) {
		let PageColumns { count, gutter } = self;
		count.mix(fp);
		gutter.mix(fp);
	}
}

impl Walk for Node {
	fn mix(&self, fp: &mut Fp) {
		match self {
			Node::HBox(b)			=> { fp.tag(0); b.mix(fp); },
			Node::VBox(b)			=> { fp.tag(1); b.mix(fp); },
			Node::Leaf(l)			=> { fp.tag(2); l.mix(fp); },
			Node::Glue(g)			=> { fp.tag(3); g.mix(fp); },
			Node::Penalty(p)		=> { fp.tag(4); p.mix(fp); },
			Node::Anchor(a)			=> { fp.tag(5); a.mix(fp); },
			Node::Float(f)			=> { fp.tag(6); f.mix(fp); },
			Node::Columns(c)		=> { fp.tag(7); c.mix(fp); },
			Node::PageColumns(c)	=> { fp.tag(8); c.mix(fp); },
			Node::RepeatHead(b)		=> { fp.tag(9); b.mix(fp); },
			Node::RepeatFoot(b)		=> { fp.tag(10); b.mix(fp); },
			Node::Frame(f)			=> { fp.tag(11); f.mix(fp); },
			Node::Transform(t)		=> { fp.tag(12); t.mix(fp); },
			Node::Clip(c)			=> { fp.tag(13); c.mix(fp); },
			Node::Tag(t)			=> { fp.tag(14); t.mix(fp); },
			Node::Mark(_)			=> { fp.tag(15); fp.mask(); },
		}
	}
}

impl Walk for Frame {
	fn mix(&self, fp: &mut Fp) {
		let Frame { w, h, baseline, items, parent } = self;
		w.mix(fp);
		h.mix(fp);
		baseline.mix(fp);
		items.mix(fp);
		fp.mask_opt(parent);
	}
}

impl Walk for Item {
	fn mix(&self, fp: &mut Fp) {
		match self {
			Item::Frame(f)				=> { fp.tag(0); f.mix(fp); },
			Item::Node(n)				=> { fp.tag(1); n.mix(fp); },
			Item::Group(f, t, p)		=> { fp.tag(2); f.mix(fp); fp.dbg(t); fp.dbg(p); },
			Item::Tag(t)				=> { fp.tag(3); t.mix(fp); },
		}
	}
}

// The flow

impl Walk for Slot {
	fn mix(&self, fp: &mut Fp) {
		match self {
			Slot::Auto		=> fp.tag(0),
			Slot::Off		=> fp.tag(1),
			Slot::On(c)		=> { fp.tag(2); c.mix(fp); },
		}
	}
}

impl Walk for RunSetup {
	fn mix(&self, fp: &mut Fp) {
		let RunSetup { geom, columns, gutter, fill, numbering, header, footer, background, foreground, styles } = self;
		fp.dbg(geom);
		columns.mix(fp);
		gutter.mix(fp);
		fill.mix(fp);
		numbering.mix(fp);
		header.mix(fp);
		footer.mix(fp);
		background.mix(fp);
		foreground.mix(fp);
		styles.mix(fp);
	}
}

impl Walk for Pending {
	fn mix(&self, fp: &mut Fp) {
		match self {
			Pending::Tag(t)		=> { fp.tag(0); t.mix(fp); },
			Pending::Mark(_)	=> { fp.tag(1); fp.mask(); },
		}
	}
}

impl Walk for LineChild {
	fn mix(&self, fp: &mut Fp) {
		let LineChild { frame, align, need } = self;
		frame.mix(fp);
		align.mix(fp);
		need.mix(fp);
	}
}

impl Walk for Body {
	fn mix(&self, fp: &mut Fp) {
		match self {
			Body::Empty			=> fp.tag(0),
			Body::Content(c)	=> { fp.tag(1); c.mix(fp); },
			Body::Layouter		=> fp.tag(2),
		}
	}
}

impl Walk for BlockSpec {
	fn mix(&self, fp: &mut Fp) {
		let BlockSpec { elem, styles, body, explicit } = self;
		elem.mix(fp);
		styles.mix(fp);
		body.mix(fp);
		explicit.mix(fp);
	}
}

impl Walk for SingleChild {
	fn mix(&self, fp: &mut Fp) {
		let SingleChild { align, sticky, alone, fr, spec, cell: _ } = self;	// a layout cache
		align.mix(fp);
		sticky.mix(fp);
		alone.mix(fp);
		fr.mix(fp);
		spec.mix(fp);
	}
}

impl Walk for MultiChild {
	fn mix(&self, fp: &mut Fp) {
		let MultiChild { align, sticky, alone, spec } = self;
		align.mix(fp);
		sticky.mix(fp);
		alone.mix(fp);
		spec.mix(fp);
	}
}

impl Walk for PlacedChild {
	fn mix(&self, fp: &mut Fp) {
		let PlacedChild {
			align_x, align_y, parent, float, clearance, dx, dy, alignment, mark: _, elem, styles, cell: _,
		} = self;	// mark: a location's hash; cell: a layout cache
		align_x.mix(fp);
		align_y.mix(fp);
		parent.mix(fp);
		float.mix(fp);
		clearance.mix(fp);
		dx.mix(fp);
		dy.mix(fp);
		alignment.mix(fp);
		fp.mask();
		elem.mix(fp);
		styles.mix(fp);
	}
}

impl Walk for Child {
	fn mix(&self, fp: &mut Fp) {
		match self {
			Child::Tag(t)			=> { fp.tag(0); t.mix(fp); },
			Child::Mark(_)			=> { fp.tag(1); fp.mask(); },
			Child::Rel(r, w)		=> { fp.tag(2); r.mix(fp); w.mix(fp); },
			Child::Fr(f, w)			=> { fp.tag(3); f.mix(fp); w.mix(fp); },
			Child::Line(l)			=> { fp.tag(4); l.mix(fp); },
			Child::Single(s)		=> { fp.tag(5); s.mix(fp); },
			Child::Multi(m)			=> { fp.tag(6); m.mix(fp); },
			Child::Placed(p)		=> { fp.tag(7); p.mix(fp); },
			Child::Flush			=> fp.tag(8),
			Child::Break(w)			=> { fp.tag(9); w.mix(fp); },
		}
	}
}

impl Walk for Nested {
	fn mix(&self, fp: &mut Fp) {
		match self {
			Nested::Flow(c, pad)		=> { fp.tag(0); c.mix(fp); fp.dbg(pad); },
			Nested::Frames(f, r)		=> { fp.tag(1); f.mix(fp); r.mix(fp); },
		}
	}
}

impl Walk for Spill {
	fn mix(&self, fp: &mut Fp) {
		let Spill { child, nested, k, fixed } = self;
		child.mix(fp);
		nested.mix(fp);
		k.mix(fp);
		match fixed {
			None			=> fp.tag(0),
			Some((a, b))	=> { fp.tag(1); a.mix(fp); b.mix(fp); },
		}
	}
}

impl Walk for Feed {
	fn mix(&self, fp: &mut Fp) {
		match self {
			Feed::List(pairs, at)	=> { fp.tag(0); pairs.mix(fp); at.mix(fp); },
			Feed::Run(_)			=> fp.tag(1),	// the future input: the level holds it
		}
	}
}

impl Walk for ColumnConfig {
	fn mix(&self, fp: &mut Fp) {
		let ColumnConfig { count, width, gutter, rtl } = self;
		count.mix(fp);
		width.mix(fp);
		gutter.mix(fp);
		rtl.mix(fp);
	}
}

impl Walk for FootnoteConfig {
	fn mix(&self, fp: &mut Fp) {
		let FootnoteConfig { separator, clearance, gap, expand } = self;
		separator.mix(fp);
		clearance.mix(fp);
		gap.mix(fp);
		expand.mix(fp);
	}
}

impl Walk for Config {
	fn mix(&self, fp: &mut Fp) {
		let Config { mode, shared, columns, footnote } = self;
		mode.mix(fp);
		shared.mix(fp);
		columns.mix(fp);
		footnote.mix(fp);
	}
}

// A skip's child index, absolute or relative to the first child not yet processed, and its location masked.
fn skip_fp(skip: &Skip, at: usize, idx: usize, relative: bool) -> (u64, u32) {
	let mut fp = Fp::new();
	match skip {
		Skip::Child(i)	=> {
			fp.tag(0);
			fp.h.write_usize(if relative { i.wrapping_sub(idx) } else { *i });
		},
		Skip::Loc(_)	=> {
			fp.tag(1);
			fp.mask();
		},
	}
	fp.h.write_usize(if relative { at.wrapping_sub(idx) } else { at });
	let mut masked = 0;
	(fp.seal(&mut masked), masked)
}

// The skips as an unordered set, sorted by fingerprint.
fn skips_fp(work: &Work, relative: bool, fp: &mut Fp) {
	let mut each: Vec<u64> = Vec::with_capacity(work.skips.len());
	for (skip, at) in work.skips.iter() {
		let (h, m) = skip_fp(skip, *at, work.idx, relative);
		fp.masked += m;
		each.push(h);
	}
	each.sort_unstable();
	each.mix(fp);
}

impl Walk for Work {
	fn mix(&self, fp: &mut Fp) {
		let Work { idx, spill, floats, footnotes, footnote_spill, tags, skips: _ } = self;	// skips: below
		idx.mix(fp);
		spill.mix(fp);
		for (i, p) in floats {
			i.mix(fp);
			p.mix(fp);
		}
		footnotes.mix(fp);
		footnote_spill.mix(fp);
		tags.mix(fp);
		skips_fp(self, false, fp);
	}
}

impl Walk for Pull {
	fn mix(&self, fp: &mut Fp) {
		let Pull { feed, children, base, done, pulled, par, cfg, mode, memo } = self;
		feed.mix(fp);
		children.mix(fp);
		base.mix(fp);
		done.mix(fp);
		pulled.mix(fp);
		par.mix(fp);
		cfg.mix(fp);
		mode.mix(fp);
		memo.len().mix(fp);	// empty at a page boundary
	}
}

impl Walk for FlowCursor {
	fn mix(&self, fp: &mut Fp) {
		let FlowCursor { config, pull, work, finished } = self;
		config.mix(fp);
		pull.mix(fp);
		work.mix(fp);
		finished.mix(fp);
	}
}

// The paginator

impl Walk for PageItem {
	fn mix(&self, fp: &mut Fp) {
		match self {
			PageItem::Run(s)			=> { fp.tag(0); s.mix(fp); },
			PageItem::Pairs(p, s)		=> { fp.tag(1); p.mix(fp); s.mix(fp); },
			PageItem::Parity(p, s)		=> { fp.tag(2); p.mix(fp); s.mix(fp); },
			PageItem::Tags(p)			=> { fp.tag(3); p.mix(fp); },
		}
	}
}

impl Walk for Level {
	fn mix(&self, fp: &mut Fp) {
		let Level { src, staged, initial, out, finished, open, ended, held } = self;
		match src {
			Source::Lazy(_, _)	=> fp.tag(0),	// the pairs to come are the input, not the state
			Source::Pairs(_)	=> fp.tag(1),
		}
		staged.mix(fp);
		initial.mix(fp);
		out.mix(fp);
		finished.mix(fp);
		open.mix(fp);
		ended.mix(fp);
		held.mix(fp);
	}
}

// The probe

/// The fingerprint of one pair taken from the root feed.
pub(super) fn pair(p: &Pair) -> u64 {
	let mut fp = Fp::new();
	p.mix(&mut fp);
	fp.h.finish()
}

/// The carry state the paginator is in before it makes its next page, one fingerprint to a part.
pub(super) fn entry(p: &Paginator, loc: &Locator) -> Entry {
	let Paginator { level, span: _, active, blank, count, waiting, started, ended } = p;	// span: the content's
	let mut e = Entry { count: *count, ..Entry::default() };
	let mut m = 0;

	let mut fp = Fp::new();
	blank.mix(&mut fp);
	waiting.mix(&mut fp);
	started.mix(&mut fp);
	ended.mix(&mut fp);
	let cursor = match active {
		None		=> {
			fp.tag(0);
			None
		},
		Some(a)		=> {
			let Active { setup, cursor, regions, first } = a;
			fp.tag(1);
			setup.mix(&mut fp);
			regions.mix(&mut fp);
			first.mix(&mut fp);
			Some(cursor)
		},
	};
	e.paginator = fp.seal(&mut m);

	let mut fp = Fp::new();
	match level.try_borrow() {
		Ok(l)	=> {
			fp.tag(0);
			l.mix(&mut fp);
		},
		Err(_)	=> fp.tag(1),
	}
	e.level = fp.seal(&mut m);

	if let Some(FlowCursor { config, pull, work, finished }) = cursor {
		let Pull { feed, children, base, done, pulled, par, cfg, mode, memo } = pull;
		e.pulled = *pulled;
		e.base = *base;
		let mut fp = Fp::new();
		feed.mix(&mut fp);
		done.mix(&mut fp);
		par.mix(&mut fp);
		cfg.mix(&mut fp);
		mode.mix(&mut fp);
		memo.len().mix(&mut fp);
		finished.mix(&mut fp);
		e.pull_rest = fp.seal(&mut m);

		let mut fp = Fp::new();
		children.mix(&mut fp);
		e.children = fp.seal(&mut m);

		let mut fp = Fp::new();
		config.mix(&mut fp);
		e.config = fp.seal(&mut m);

		let Work { idx, spill, floats, footnotes, footnote_spill, tags, skips: _ } = work;	// skips: below
		e.work_idx = *idx;

		let mut fp = Fp::new();
		spill.mix(&mut fp);
		e.spill = fp.seal(&mut m);

		let mut fp = Fp::new();
		for (i, c) in floats {
			i.wrapping_sub(*idx).mix(&mut fp);
			c.mix(&mut fp);
		}
		e.floats = fp.seal(&mut m);

		let mut fp = Fp::new();
		footnotes.mix(&mut fp);
		e.footnotes = fp.seal(&mut m);

		let mut fp = Fp::new();
		footnote_spill.mix(&mut fp);
		e.foot_spill = fp.seal(&mut m);

		let mut fp = Fp::new();
		tags.mix(&mut fp);
		e.tags = fp.seal(&mut m);

		let mut fp = Fp::new();
		skips_fp(work, true, &mut fp);
		e.skips = fp.seal(&mut m);

		let mut fp = Fp::new();
		for (i, _) in floats {
			i.mix(&mut fp);
		}
		skips_fp(work, false, &mut fp);
		e.abs = fp.seal(&mut m);
	}

	// The place counters: how many were handed out and how often, never which.
	let (place, counts) = loc.carry();
	let mut fp = Fp::new();
	fp.mask_opt(&Some(place));
	counts.mix(&mut fp);
	e.locator = fp.seal(&mut m);

	e.masked = m;
	e
}

/// The record of a page made from `entry`: the pairs it took and the setup of its run.
pub(super) fn record(setup: &RunSetup, seen: &[u64], entry: Entry) -> PageRec {
	let mut fp = Fp::new();
	setup.mix(&mut fp);
	seen.mix(&mut fp);
	PageRec { pairs: seen.len(), input: fp.h.finish(), entry }
}
