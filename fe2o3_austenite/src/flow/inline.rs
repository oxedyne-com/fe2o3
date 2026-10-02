// SPDX-License-Identifier: Apache-2.0
// Original work copyright (c) the Typst project authors (typst-layout `inline/{collect,prepare,line}.rs`, version 0.15.1).
// Modified for Austenite: ported to Hematite's types, IR and error handling.
// U6a owns this file: inline items -- shaped text, spacing, inline boxes, equations, footnote marks,
// link spans -- collected from a paragraph's children for the line breaker. Internal to U6a.
//
// A port of Typst 0.15.1's inline layout (`typst-layout/src/inline/{collect,prepare,line}.rs`,
// Apache-2.0, (c) the Typst project authors). A paragraph's children become one string -- text as it
// is, spacing as a space, an inline object as U+FFFC -- with a segment per style run; the text is
// shaped per bidi level and script; a line is any slice of it, whose end runs are re-shaped where the
// slice cuts them; and a chosen line is committed to a line box, justified and aligned as Typst does,
// with punctuation hanging into the end margin.

use crate::diag::DiagnosticKind;
use crate::eval::content::{
	Content,
	ElemKind,
};
use crate::eval::lib::model::quote::smart_quotes;
use crate::eval::realise::{
	Pair,
	Tag as RealiseTag,
};
use crate::eval::styles::{
	Property,
	Style,
	StyleChain,
	Styles,
};
use crate::eval::value::{
	HAlign,
	Value,
};
use crate::eval::Engine;
use crate::flow::text::{
	self as ftext,
	is_of_cj_script,
	is_space,
	shape_run,
	text_field,
	Costs,
	ShapedRun,
	TextProps,
	BEGIN_PUNCT,
	END_PUNCT,
};
use crate::flow::Region;
use crate::fonts::{
	is_default_ignorable,
	FontBook,
};
use crate::ir::{
	BoxNode,
	Dims,
	Glue,
	Graphic,
	Leaf,
	LinkTarget,
	Node,
	Sp,
};
use crate::linebreak::{
	dash_of,
	Breakpoint,
	Trim,
};

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_text::unicode::bidi;
use oxedyne_fe2o3_text::unicode::lookup::Partitioned;
use oxedyne_fe2o3_text::unicode::prop::Script;

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;

const SPACING_REPLACE:	&str = " ";
const OBJ_REPLACE:		&str = "\u{FFFC}";
const LTR_EMBEDDING:	&str = "\u{202A}";
const RTL_EMBEDDING:	&str = "\u{202B}";
const POP_EMBEDDING:	&str = "\u{202C}";
const LTR_ISOLATE:		&str = "\u{2066}";
const POP_ISOLATE:		&str = "\u{2069}";
const LINE_SEPARATOR:	char = '\u{2028}';

/// Is an item in a line a placeholder for the start hyphen, the end hyphen, or item `i` of the prepared
/// paragraph (`i + 1`)? Lines keep items in visual order; this keeps their logical order.
const START_HYPHEN:	usize = 0;
const END_HYPHEN:	usize = usize::MAX;

/// How a paragraph chooses its breaks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Linebreaks {
	Simple,
	Optimized,
}

/// Where a paragraph stands in its flow, which decides its first-line indent.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ParSituation {
	First,			// first in its container or page run, or after a column break
	Consecutive,	// right after another paragraph
	Other,
}

/// A horizontal alignment fixed against the text direction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FixedAlign {
	Start,
	Center,
	End,
}

impl FixedAlign {
	fn position(self, extent: f64) -> f64 {
		match self {
			FixedAlign::Start	=> 0.0,
			FixedAlign::Center	=> extent / 2.0,
			FixedAlign::End		=> extent,
		}
	}
}

/// The paragraph-wide settings of one inline layout.
#[derive(Clone, Debug)]
pub struct Config {
	pub justify:			bool,
	pub linebreaks:			Linebreaks,
	pub first_line_indent:	f64,
	pub hanging_indent:		f64,
	pub align:				FixedAlign,
	pub font_size:			f64,
	pub rtl:				bool,
	pub hyphenate:			Option<bool>,	// set only when the same for every child
	pub lang:				Option<String>,	// ditto
	pub cjk_latin_spacing:	bool,
	pub costs:				Costs,
}

/// Laid-out inline content: a list set from its top-left, `baseline` below which a line seats it.
#[derive(Clone, Debug)]
pub struct Frame {
	pub nodes:		Arc<Vec<Node>>,
	pub width:		f64,
	pub baseline:	f64,				// from the top
	pub height:		f64,
	pub link:		Option<LinkTarget>,	// where the whole frame links (`link.current`)
	pub shift:		(f64, f64),			// its contents moved right and down, the frame staying put
}

impl Frame {
	pub fn new(nodes: Vec<Node>, width: f64, baseline: f64, height: f64) -> Self {
		Self { nodes: Arc::new(nodes), width, baseline, height, link: None, shift: (0.0, 0.0) }
	}

	pub fn empty(width: f64) -> Self { Self::new(Vec::new(), width, 0.0, 0.0) }
}

/// One item of a prepared paragraph.
#[derive(Clone, Debug)]
pub enum Item {
	Text(ShapedRun),
	Absolute(f64, bool),						// spacing in points, and whether it is weak
	Fractional(f64, Option<Arc<(Content, StyleChain)>>),	// fractional spacing, or a box of fractional width
	Frame(Frame),
	Tag(RealiseTag),							// where a located element starts or ends
	Skip(&'static str),							// invisible, as a bidi isolate
}

impl Item {
	pub fn is_tag(&self) -> bool { matches!(self, Item::Tag(_)) }

	pub fn text(&self) -> Option<&ShapedRun> {
		match self {
			Item::Text(t)	=> Some(t),
			_				=> None,
		}
	}

	pub fn text_mut(&mut self) -> Option<&mut ShapedRun> {
		match self {
			Item::Text(t)	=> Some(t),
			_				=> None,
		}
	}

	/// What the item stands for in the paragraph's text.
	pub fn textual(&self) -> &str {
		match self {
			Item::Text(t)						=> &t.text,
			Item::Absolute(..) | Item::Fractional(..)	=> SPACING_REPLACE,
			Item::Frame(_)						=> OBJ_REPLACE,
			Item::Tag(_)						=> "",
			Item::Skip(s)						=> s,
		}
	}

	pub fn natural_width(&self) -> f64 {
		match self {
			Item::Text(t)		=> t.width(),
			Item::Absolute(v, _)	=> *v,
			Item::Frame(f)		=> f.width,
			_					=> 0.0,
		}
	}
}

/// A stretch of the text still to be shaped, or an item already made.
enum Segment {
	Text(usize, StyleChain),
	Item(Item),
}

impl Segment {
	fn textual_len(&self) -> usize {
		match self {
			Segment::Text(n, _)	=> *n,
			Segment::Item(i)	=> i.textual().len(),
		}
	}
}

/// A paragraph ready for breaking: its text, its items with their byte ranges, and the bidi levels.
pub struct Prep {
	pub text:		String,
	pub config:		Config,
	pub bidi:		Option<Vec<u8>>,					// a level per byte, when any is not the base's
	pub items:		Vec<((usize, usize), Item)>,
	pub indices:	Vec<usize>,							// the item holding each byte
	pub book:		Arc<FontBook>,
	cache:			RefCell<HashMap<(usize, usize, usize), ShapedRun>>,
}

impl Prep {
	/// The item holding byte `offset`.
	pub fn get(&self, offset: usize) -> &((usize, usize), Item) {
		let idx = self.indices.get(offset).copied().unwrap_or(0);
		&self.items[idx.min(self.items.len().saturating_sub(1))]
	}

	/// The items meeting `start..end`, with their indices; a line starting mid-text leaves out the
	/// empty items before it, which belong to the line before.
	fn slice(&self, start: usize, end: usize) -> Vec<usize> {
		let first = if start == 0 { 0 } else { self.indices.get(start).copied().unwrap_or(0) };
		let mut out = Vec::new();
		for i in first..self.items.len() {
			let (r, _) = &self.items[i];
			if r.0 < end || r.1 <= end {
				out.push(i);
			} else {
				break;
			}
		}
		out
	}

	/// Item `i`'s text re-shaped over `start..end`, remembered for the attempts that ask again.
	fn reshape(&self, i: usize, run: &ShapedRun, start: usize, end: usize) -> Outcome<ShapedRun> {
		if let Some(r) = self.cache.borrow().get(&(i, start, end)) {
			return Ok(r.clone());
		}
		let r = res!(run.reshape(&self.book, start, end));
		self.cache.borrow_mut().insert((i, start, end), r.clone());
		Ok(r)
	}
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ COLLECTION                                                                 │
// └───────────────────────────────────────────────────────────────────────────┘

/// Text properties, resolved once per style chain of a paragraph.
#[derive(Default)]
pub struct PropsCache {
	map:	Vec<(StyleChain, Arc<TextProps>)>,
}

impl PropsCache {
	pub fn get(&mut self, styles: &StyleChain) -> Outcome<Arc<TextProps>> {
		for (c, p) in &self.map {
			if c.ptr_eq(styles) {
				return Ok(p.clone());
			}
		}
		let p = Arc::new(res!(ftext::props(styles)));
		self.map.push((styles.clone(), p.clone()));
		Ok(p)
	}
}

struct Collector {
	full:		String,
	segments:	Vec<Segment>,
}

impl Collector {
	fn push_text(&mut self, text: &str, styles: &StyleChain) {
		self.full.push_str(text);
		let n = text.len();
		if let Some(Segment::Text(len, last)) = self.segments.last_mut() {
			if last.ptr_eq(styles) {
				*len += n;
				return;
			}
		}
		self.segments.push(Segment::Text(n, styles.clone()));
	}

	fn push_item(&mut self, item: Item) {
		if let (Some(Segment::Item(Item::Absolute(prev, true))), Item::Absolute(amount, true)) = (self.segments.last_mut(), &item) {
			*prev = prev.max(*amount);
			return;
		}
		self.full.push_str(item.textual());
		self.segments.push(Segment::Item(item));
	}
}

/// Tracks open quotation marks, as Typst's smart quoter does.
struct Quoter {
	depth:	u32,
	kinds:	u32,
}

impl Quoter {
	fn top(&self) -> Option<bool> {
		self.depth.checked_sub(1).map(|i| (self.kinds >> i) & 1 == 1)
	}

	fn push(&mut self, double: bool) {
		if self.depth < 32 {
			self.kinds |= (double as u32) << self.depth;
			self.depth += 1;
		}
	}

	fn pop(&mut self) {
		self.depth -= 1;
		self.kinds &= (1 << self.depth) - 1;
	}

	fn quote(&mut self, before: Option<char>, q: &Quotes, double: bool) -> String {
		let opened = self.top();
		let before = before.unwrap_or(' ');
		if before.is_numeric() && opened != Some(double) {
			return if double { "\u{2033}".to_string() } else { "\u{2032}".to_string() };
		}
		if !double && opened != Some(false) && (before.is_alphabetic() || before == '\u{FFFC}') {
			return "\u{2019}".to_string();
		}
		if opened == Some(double)
			&& !before.is_whitespace()
			&& !matches!(before, '\n' | '\r' | '\u{000B}' | '\u{000C}' | '\u{0085}' | '\u{2028}' | '\u{2029}')
			&& !matches!(before, '(' | '{' | '[')
		{
			self.pop();
			return if double { q.double_close.clone() } else { q.single_close.clone() };
		}
		self.push(double);
		if double { q.double_open.clone() } else { q.single_open.clone() }
	}
}

struct Quotes {
	single_open:	String,
	single_close:	String,
	double_open:	String,
	double_close:	String,
}

/// The quotation marks in force for a quote: the language's, from the model's table under the chain with
/// the element's own `alternative`, then any `quotes` given, as Typst's `SmartQuotes::get` takes them.
fn quotes_for(styles: &StyleChain, alternative: bool, custom: Option<&Value>) -> Quotes {
	let chain = match ElemKind::SmartQuote.field_id("alternative") {
		Some(id)	=> styles.chain(&Styles::from_style(Style::Property(Property::new(
			ElemKind::SmartQuote, id, Value::Bool(alternative), crate::syntax::Span::detached())))),
		None		=> styles.clone(),
	};
	let [so, sc, dopen, dc] = smart_quotes(&chain);
	let mut q = Quotes {
		single_open:	so.to_string(),
		single_close:	sc.to_string(),
		double_open:	dopen.to_string(),
		double_close:	dc.to_string(),
	};
	// A set is two graphemes in a string, or two strings; a bare set is the double quotes.
	let pair = |v: &Value| -> Option<(String, String)> {
		match v {
			Value::Str(s) => {
				let g = oxedyne_fe2o3_text::unicode::segment::graphemes(s);
				if g.len() == 2 { Some((g[0].to_string(), g[1].to_string())) } else { None }
			}
			Value::Array(a) if a.len() == 2 => match (&a[0], &a[1]) {
				(Value::Str(o), Value::Str(c))	=> Some((o.to_string(), c.to_string())),
				_								=> None,
			},
			_ => None,
		}
	};
	match custom {
		Some(Value::Dict(d)) => {
			if let Some((o, c)) = d.get("double").and_then(|v| pair(v)) { q.double_open = o; q.double_close = c; }
			if let Some((o, c)) = d.get("single").and_then(|v| pair(v)) { q.single_open = o; q.single_close = c; }
		}
		Some(v) => if let Some((o, c)) = pair(v) { q.double_open = o; q.double_close = c; },
		None => (),
	}
	q
}

/// A field of the element in hand, else the chain's.
fn own(pair: &Pair, name: &str) -> Outcome<Option<Value>> {
	match pair.content.kind().and_then(|k| k.field_id(name)) {
		Some(id)	=> pair.styles.resolve(&pair.content, id),
		None		=> Ok(None),
	}
}

fn bool_of(v: Option<Value>, dflt: bool) -> bool {
	match v {
		Some(Value::Bool(b))	=> b,
		_						=> dflt,
	}
}

/// Gathers the children into one string and its segments.
fn collect(
	engine:		&mut Engine,
	children:	&[Pair],
	config:		&Config,
	region:		Region,
	cache:		&mut PropsCache,
)
	-> Outcome<(String, Vec<Segment>)>
{
	let mut c = Collector { full: String::new(), segments: Vec::with_capacity(children.len() + 2) };
	let mut quoter = Quoter { depth: 0, kinds: 0 };
	if config.first_line_indent != 0.0 {
		c.push_item(Item::Absolute(config.first_line_indent, false));
	}
	if config.hanging_indent != 0.0 {
		c.push_item(Item::Absolute(-config.hanging_indent, false));
	}
	let region_w = region.width.to_pt();
	let book = res!(engine.fonts.book());
	let mut warned_position = false;
	for pair in children {
		if let Some(tag) = &pair.tag {
			c.push_item(Item::Tag(tag.clone()));
			continue;
		}
		let styles = &pair.styles;
		let kind = match pair.content.kind() {
			Some(k)	=> k,
			None	=> continue,
		};
		// A link to a page position has no place in the IR yet, so its content is left unlinked.
		if !warned_position && matches!(res!(ftext::elem_field(styles, ElemKind::Link, "current")), Some(Value::Dict(_))) {
			warned_position = true;
			engine.warn(DiagnosticKind::Unsupported, pair.content.span(), "a link to a position is not supported yet and was left unlinked");
		}
		match kind {
			ElemKind::Tag => {
				if let Some(tag) = RealiseTag::from_content(&pair.content) {
					c.push_item(Item::Tag(tag));
				}
			}
			ElemKind::Space => c.push_text(" ", styles),
			ElemKind::Text | ElemKind::Symbol => {
				let text = match pair.content.get(crate::eval::content::FieldId(0)) {
					Some(Value::Str(s))	=> s.clone(),
					_					=> continue,
				};
				let props = res!(cache.get(styles));
				let mut s = String::new();
				if props.rtl != config.rtl {
					s.push_str(if props.rtl { RTL_EMBEDDING } else { LTR_EMBEDDING });
				}
				match props.case {
					Some(true)	=> s.push_str(&text.to_uppercase()),
					Some(false)	=> s.push_str(&text.to_lowercase()),
					None		=> s.push_str(&text),
				}
				if props.rtl != config.rtl {
					s.push_str(POP_EMBEDDING);
				}
				c.push_text(&s, styles);
			}
			ElemKind::H => {
				match res!(own(pair, "amount")) {
					Some(Value::Fraction(f)) => {
						if f.0 != 0.0 {
							c.push_item(Item::Fractional(f.0, None));
						}
					}
					Some(v) => {
						let size = styles.font_size();
						let amount = match v {
							Value::Length(l)	=> l.resolve(size),
							Value::Ratio(r)		=> r.0 * region_w,
							Value::Relative(r)	=> r.rel.0 * region_w + r.abs.resolve(size),
							_					=> 0.0,
						};
						if amount != 0.0 {
							c.push_item(Item::Absolute(amount, bool_of(res!(own(pair, "weak")), false)));
						}
					}
					None => (),
				}
			}
			ElemKind::Linebreak => {
				let justify = bool_of(res!(own(pair, "justify")), false);
				c.push_text(if justify { "\u{2028}" } else { "\n" }, styles);
			}
			ElemKind::SmartQuote => {
				let double = bool_of(res!(own(pair, "double")), true);
				if bool_of(res!(own(pair, "enabled")), true) {
					let alternative = bool_of(res!(own(pair, "alternative")), false);
					let custom = res!(own(pair, "quotes")).filter(|v| !v.is_auto());
					let q = quotes_for(styles, alternative, custom.as_ref());
					let before = c.full.chars().rev().find(|&ch| !is_default_ignorable(ch));
					let mark = quoter.quote(before, &q, double);
					c.push_text(&mark, styles);
				} else {
					c.push_text(if double { "\"" } else { "'" }, styles);
				}
			}
			ElemKind::Equation => {
				c.push_item(Item::Skip(LTR_ISOLATE));
				let node = res!(crate::flow::math::layout_equation(engine, &pair.content, styles, region));
				let mut frame = frame_of_node(node);
				let props = res!(cache.get(styles));
				frame.link = props.link.clone();
				frame.shift = ftext::frame_shift(&book, &props);
				c.push_item(Item::Frame(frame));
				c.push_item(Item::Skip(POP_ISOLATE));
			}
			ElemKind::Box => {
				match res!(own(pair, "width")) {
					Some(Value::Fraction(f)) => c.push_item(Item::Fractional(f.0,
						Some(Arc::new((pair.content.clone(), styles.clone()))))),
					_ => {
						let frame = res!(layout_inline_box(engine, &pair.content, styles, region, None, cache));
						c.push_item(Item::Frame(frame));
					}
				}
			}
			other => {
				engine.warn(DiagnosticKind::Lint, pair.content.span(), fmt!(
					"{} may not occur inside of a paragraph and was ignored", other.name()));
			}
		}
	}
	Ok((c.full, c.segments))
}

/// A laid-out node as an inline frame, its baseline its height above the baseline.
pub fn frame_of_node(node: Node) -> Frame {
	let dims = match &node {
		Node::HBox(b) | Node::VBox(b)	=> b.dims,
		Node::Leaf(l)					=> l.dims,
		_								=> Dims::default(),
	};
	Frame::new(vec![node], dims.width.to_pt(), dims.height.to_pt(), dims.vextent().to_pt())
}

/// An inline `box` as a line holds it, Typst's `layout_and_modify` then `apply_shift`: laid out with any
/// link in force taken off its body, then linked as a whole and moved by the text's baseline shift.
fn layout_inline_box(
	engine:		&mut Engine,
	elem:		&Content,
	styles:		&StyleChain,
	region:		Region,
	share:		Option<f64>,
	cache:		&mut PropsCache,
)
	-> Outcome<Frame>
{
	let props = res!(cache.get(styles));
	let inner = match (&props.link, ElemKind::Link.field_id("current")) {
		(Some(_), Some(id))	=> styles.chain(&Styles::from_style(Style::Property(Property::new(
			ElemKind::Link, id, Value::None, crate::syntax::Span::detached())))),
		_					=> styles.clone(),
	};
	let mut frame = res!(layout_box(engine, elem, &inner, region, share));
	let book = res!(engine.fonts.book());
	frame.link = props.link.clone();
	frame.shift = ftext::frame_shift(&book, &props);
	Ok(frame)
}

/// Lays out an inline `box`: its body in the box's width (its natural width when `auto`, the share of a
/// fractional width when `share` is given), shifted by `baseline`. Inline content is set here with this
/// file's own inline layout; block content through the block flow.
///
/// Interim: a box's `height`, `inset`, `outset`, `fill`, `stroke`, `radius` and `clip` are U6b's to
/// honour, through a `layout_box` of theirs that replaces this one call site.
pub fn layout_box(
	engine:		&mut Engine,
	elem:		&Content,
	styles:		&StyleChain,
	region:		Region,
	share:		Option<f64>,
)
	-> Outcome<Frame>
{
	let get = |name: &str| -> Outcome<Option<Value>> {
		match elem.kind().and_then(|k| k.field_id(name)) {
			Some(id)	=> styles.resolve(elem, id),
			None		=> Ok(None),
		}
	};
	let size = styles.font_size();
	let base_w = region.base.0.to_pt();
	let width = match share {
		Some(w)	=> Some(w),
		None	=> match res!(get("width")) {
			Some(Value::Length(l))		=> Some(l.resolve(size)),
			Some(Value::Ratio(r))		=> Some(r.0 * base_w),
			Some(Value::Relative(r))	=> Some(r.rel.0 * base_w + r.abs.resolve(size)),
			_							=> None,
		},
	};
	let body = match res!(get("body")) {
		Some(Value::Content(c))	=> c,
		_						=> Content::empty(),
	};
	let pod = Region {
		width:		Sp::from_pt(width.unwrap_or(region.width.to_pt())),
		height:		region.height,
		base:		region.base,
		expand_x:	width.is_some(),
		expand_y:	false,
	};
	let pairs = res!(crate::eval::realise::realise(engine, &body, styles, crate::eval::realise::RealiseMode::Flow));
	let inline = pairs.iter().all(|p| p.is_tag() || crate::eval::realise::is_inline(&p.content)
		|| p.content.is(ElemKind::Space));
	let mut frame = if pairs.iter().all(|p| p.is_tag()) {
		Frame::empty(width.unwrap_or(0.0))
	} else if inline {
		let lines = res!(crate::flow::par::layout_lines(engine, &pairs, styles, pod, width.is_some()));
		res!(crate::flow::par::stack_frame(styles, lines))
	} else {
		let nodes = res!(crate::flow::layout_block(engine, &body, styles, pod));
		let mut h = Sp::ZERO;
		let mut w = Sp::ZERO;
		for n in &nodes {
			h += n.vextent();
			if let Node::HBox(b) | Node::VBox(b) = n {
				if b.dims.width > w { w = b.dims.width; }
			}
		}
		let wpt = width.unwrap_or(w.to_pt());
		Frame::new(nodes, wpt, h.to_pt(), h.to_pt())
	};
	if let Some(w) = width {
		frame.width = w;
	}
	let shift = match res!(get("baseline")) {
		Some(Value::Length(l))		=> l.resolve(size),
		Some(Value::Ratio(r))		=> r.0 * frame.height,
		Some(Value::Relative(r))	=> r.rel.0 * frame.height + r.abs.resolve(size),
		_							=> 0.0,
	};
	frame.baseline -= shift;
	Ok(frame)
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ PREPARATION                                                                │
// └───────────────────────────────────────────────────────────────────────────┘

/// Collects, shapes and indexes a paragraph's children.
pub fn prepare(
	engine:		&mut Engine,
	children:	&[Pair],
	config:		Config,
	region:		Region,
) -> Outcome<Prep> {
	let book = res!(engine.fonts.book());
	let mut cache = PropsCache::default();
	let (text, segments) = res!(collect(engine, children, &config, region, &mut cache));

	// Bidi levels, kept only when some text runs against the base direction.
	let base_level: u8 = if config.rtl { 1 } else { 0 };
	let needs = config.rtl || text.chars().any(|c| matches!(
		oxedyne_fe2o3_text::unicode::prop::BidiClass::of(c),
		oxedyne_fe2o3_text::unicode::prop::BidiClass::R | oxedyne_fe2o3_text::unicode::prop::BidiClass::AL
		| oxedyne_fe2o3_text::unicode::prop::BidiClass::RLE | oxedyne_fe2o3_text::unicode::prop::BidiClass::RLO
		| oxedyne_fe2o3_text::unicode::prop::BidiClass::RLI));
	let levels: Vec<u8> = if needs {
		let info = bidi::resolve(&text, if config.rtl { bidi::Direction::Rtl } else { bidi::Direction::Ltr });
		let mut per_byte = vec![base_level; text.len()];
		for (ci, (b, ch)) in text.char_indices().enumerate() {
			let lv = info.levels.get(ci).copied().unwrap_or(base_level);
			for k in b..b + ch.len_utf8() {
				if let Some(slot) = per_byte.get_mut(k) {
					*slot = lv;
				}
			}
		}
		per_byte
	} else {
		vec![base_level; text.len()]
	};
	let is_bidi = levels.iter().any(|l| (l % 2) != (base_level % 2));

	let mut cursor = 0;
	let mut items: Vec<((usize, usize), Item)> = Vec::with_capacity(segments.len());
	for seg in segments {
		let len = seg.textual_len();
		let end = cursor + len;
		match seg {
			Segment::Text(_, styles) => {
				let props = res!(cache.get(&styles));
				res!(shape_range(&mut items, &book, &text, &levels, cursor, end, &props));
			}
			Segment::Item(item) => items.push(((cursor, end), item)),
		}
		cursor = end;
	}
	let mut indices = Vec::with_capacity(text.len());
	for (i, (r, _)) in items.iter().enumerate() {
		for _ in r.0..r.1 {
			indices.push(i);
		}
	}
	if config.cjk_latin_spacing {
		add_cjk_latin_spacing(&mut items);
	}
	Ok(Prep {
		text,
		config,
		bidi:		if is_bidi { Some(levels) } else { None },
		items,
		indices,
		book,
		cache:		RefCell::new(HashMap::new()),
	})
}

fn is_generic_script(s: Script) -> bool {
	matches!(s, Script::Unknown | Script::Common | Script::Inherited)
}

fn is_compatible(a: Script, b: Script) -> bool {
	is_generic_script(a) || is_generic_script(b) || a == b
}

/// Shapes `start..end` in runs of one bidi level and one script.
fn shape_range(
	items:	&mut Vec<((usize, usize), Item)>,
	book:	&FontBook,
	text:	&str,
	levels:	&[u8],
	start:	usize,
	end:	usize,
	props:	&Arc<TextProps>,
)
	-> Outcome<()>
{
	let explicit = props.script.is_some();
	let process = |items: &mut Vec<((usize, usize), Item)>, a: usize, b: usize, level: u8| -> Outcome<()> {
		let rtl = level % 2 == 1;
		let sub = match text.get(a..b) {
			Some(s)	=> s,
			None	=> return Err(err!("A text run {}..{} is off a character boundary.", a, b; Bug)),
		};
		let shaped = res!(shape_run(book, a, sub, props, rtl));
		items.push(((a, b), Item::Text(shaped)));
		Ok(())
	};
	let mut prev_level: u8 = 0;
	let mut prev_script = Script::Unknown;
	let mut cursor = start;
	for i in start..end {
		if !text.is_char_boundary(i) {
			continue;
		}
		let level = levels.get(i).copied().unwrap_or(0);
		let cur = if explicit {
			Script::Unknown
		} else {
			text.get(i..).and_then(|t| t.chars().next()).map_or(Script::Unknown, Script::of)
		};
		if level != prev_level || !is_compatible(cur, prev_script) {
			if cursor < i {
				res!(process(items, cursor, i, prev_level));
			}
			cursor = i;
			prev_level = level;
			prev_script = cur;
		} else if is_generic_script(prev_script) {
			prev_script = cur;
		}
	}
	process(items, cursor, end, prev_level)
}

/// A quarter em between Han or kana and a western letter or number, shrinkable to an eighth.
fn add_cjk_latin_spacing(items: &mut [((usize, usize), Item)]) {
	// The glyphs in order, as (item, glyph) positions, with a break where a non-text item stands.
	let mut seq: Vec<Option<(usize, usize, bool)>> = Vec::new();	// (item, glyph, has shift)
	for (ii, (_, item)) in items.iter().enumerate() {
		match item {
			Item::Tag(_)	=> (),
			Item::Text(t)	=> for gi in t.kept.0..t.kept.1 {
				seq.push(Some((ii, gi, t.props.shift.is_some())));
			},
			_				=> seq.push(None),
		}
	}
	let glyph = |items: &[((usize, usize), Item)], ii: usize, gi: usize| -> Option<crate::flow::text::SGlyph> {
		match &items[ii].1 {
			Item::Text(t)	=> t.glyphs.get(gi).cloned(),
			_				=> None,
		}
	};
	for k in 0..seq.len() {
		let (ii, gi, shift) = match seq[k] {
			Some(x)	=> x,
			None	=> continue,
		};
		let g = match glyph(items, ii, gi) {
			Some(g)	=> g,
			None	=> continue,
		};
		if !g.is_cj_script() {
			continue;
		}
		let next = seq.get(k + 1).copied().flatten();
		let prev = if k > 0 { seq[k - 1] } else { None };
		let mut add_right = false;
		let mut add_left = false;
		if let Some((ni, ng, ns)) = next {
			if let Some(n) = glyph(items, ni, ng) {
				if n.is_letter_or_number() && ns == shift {
					add_right = true;
				}
			}
		}
		if let Some((pi, pg, ps)) = prev {
			if let Some(p) = glyph(items, pi, pg) {
				if p.is_letter_or_number() && ps == shift {
					add_left = true;
				}
			}
		}
		if let Item::Text(t) = &mut items[ii].1 {
			if let Some(gm) = Arc::make_mut(&mut t.glyphs).get_mut(gi) {
				if add_right {
					gm.x_advance += 0.25;
					gm.shrink.1 += 0.125;
				}
				if add_left {
					gm.x_advance += 0.25;
					gm.x_offset += 0.25;
					gm.shrink.0 += 0.125;
				}
			}
		}
		if add_left {
			if let Some((pi, pg, _)) = prev {
				if let Item::Text(t) = &mut items[pi].1 {
					if let Some(pm) = Arc::make_mut(&mut t.glyphs).get_mut(pg) {
						pm.justifiable = true;
					}
				}
			}
		}
	}
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ LINES                                                                      │
// └───────────────────────────────────────────────────────────────────────────┘

/// A dash at a line's end, for the cost of consecutive dashes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dash {
	Soft,	// a hyphen added to break a word
	Hard,	// a hyphen in a compound word
	Other,	// an en or em dash
}

/// A candidate or chosen line: its items in visual order with their logical indices, its natural
/// width, and whether it justifies.
#[derive(Clone, Debug)]
pub struct Line {
	pub items:		Vec<(usize, Item)>,
	pub width:		f64,
	pub justify:	bool,
	pub dash:		Option<Dash>,
}

impl Line {
	pub fn empty() -> Self {
		Self { items: Vec::new(), width: 0.0, justify: false, dash: None }
	}

	pub fn justifiables(&self) -> usize {
		let mut n: usize = self.items.iter().filter_map(|(_, i)| i.text()).map(|t| t.justifiables()).sum();
		if self.trailing_text().map_or(false, |t| t.cjk_justifiable_at_last()) {
			n = n.saturating_sub(1);
		}
		n
	}

	pub fn stretchability(&self) -> f64 {
		self.items.iter().filter_map(|(_, i)| i.text()).map(|t| t.stretchability()).sum()
	}

	pub fn shrinkability(&self) -> f64 {
		self.items.iter().filter_map(|(_, i)| i.text()).map(|t| t.shrinkability()).sum()
	}

	pub fn has_negative_width_items(&self) -> bool {
		self.items.iter().any(|(_, i)| match i {
			Item::Absolute(a, _)	=> *a < 0.0,
			Item::Frame(f)			=> f.width < 0.0,
			_						=> false,
		})
	}

	pub fn fr(&self) -> f64 {
		self.items.iter().map(|(_, i)| match i {
			Item::Fractional(f, _)	=> *f,
			_						=> 0.0,
		}).sum()
	}

	pub fn leading_text(&self) -> Option<&ShapedRun> {
		self.items.iter().find(|(_, i)| !i.is_tag()).and_then(|(_, i)| i.text())
	}

	pub fn trailing_text(&self) -> Option<&ShapedRun> {
		self.items.iter().rev().find(|(_, i)| !i.is_tag()).and_then(|(_, i)| i.text())
	}

	fn leading_text_mut(&mut self) -> Option<&mut ShapedRun> {
		self.items.iter_mut().find(|(_, i)| !i.is_tag()).and_then(|(_, i)| i.text_mut())
	}

	fn trailing_text_mut(&mut self) -> Option<&mut ShapedRun> {
		self.items.iter_mut().rev().find(|(_, i)| !i.is_tag()).and_then(|(_, i)| i.text_mut())
	}
}

/// The line spanning `start..end`, ending at `bp`, after line `pred`.
pub fn line(p: &Prep, start: usize, end: usize, bp: Breakpoint, pred: Option<&Line>) -> Outcome<Line> {
	let full = match p.text.get(start..end) {
		Some(t)	=> t,
		None	=> return Err(err!("A line {}..{} is off a character boundary.", start, end; Bug)),
	};
	let justify = full.ends_with(LINE_SEPARATOR) || (p.config.justify && bp != Breakpoint::Mandatory);
	let dash = dash_of(bp, full);
	let trim = bp.trim(start, full);
	let mut l = Line { items: Vec::new(), width: 0.0, justify, dash };

	// A hard hyphen repeated at the next line's start, where the language wants it.
	if let Some(pred) = pred {
		if pred.dash == Some(Dash::Hard) {
			if let Some(base) = pred.trailing_text() {
				if should_repeat_hyphen(&base.props.lang, full) {
					if let Some(h) = base.hyphen(&p.book, trim.shaping, false) {
						l.items.push((START_HYPHEN, Item::Text(h)));
					}
				}
			}
		}
	}

	res!(collect_items(&mut l.items, p, start, end, &trim));

	if dash == Some(Dash::Soft) {
		let base = l.trailing_text().cloned();
		if let Some(base) = base {
			if let Some(h) = base.hyphen(&p.book, trim.shaping, true) {
				l.items.push((END_HYPHEN, Item::Text(h)));
			}
		}
	}

	trim_weak_spacing(&mut l.items);
	adjust_cj_at_line_boundaries(p, start, trim.layout, &mut l);
	adjust_glyph_stretch_at_line_end(p, &mut l);
	l.width = l.items.iter().map(|(_, i)| i.natural_width()).sum();
	Ok(l)
}

fn collect_items(items: &mut Vec<(usize, Item)>, p: &Prep, start: usize, end: usize, trim: &Trim) -> Outcome<()> {
	let mut fallback: Option<(usize, Item)> = None;
	for (a, b, rtl) in reorder(p, start, end) {
		let from = items.len();
		res!(collect_range(p, a, b, trim, items, &mut fallback));
		if rtl {
			items[from..].reverse();
		}
	}
	if !items.iter().any(|(_, i)| matches!(i, Item::Text(_))) {
		if let Some(f) = fallback {
			items.push(f);
		}
	}
	Ok(())
}

/// The line's ranges in visual order, each with whether it runs right to left.
fn reorder(p: &Prep, start: usize, end: usize) -> Vec<(usize, usize, bool)> {
	let levels = match &p.bidi {
		None	=> return vec![(start, end, p.config.rtl)],
		Some(l)	=> l,
	};
	if start >= end {
		return vec![(start, end, p.config.rtl)];
	}
	// Runs of one level, then rule L2: reverse every run at or above each level down to the lowest odd.
	let mut runs: Vec<(usize, usize, u8)> = Vec::new();
	let mut a = start;
	let mut cur = levels.get(start).copied().unwrap_or(0);
	for i in start..end {
		let lv = levels.get(i).copied().unwrap_or(0);
		if lv != cur {
			runs.push((a, i, cur));
			a = i;
			cur = lv;
		}
	}
	runs.push((a, end, cur));
	let hi = runs.iter().map(|r| r.2).max().unwrap_or(0);
	let lo = runs.iter().map(|r| r.2).filter(|l| l % 2 == 1).min().unwrap_or(hi + 1);
	let mut level = hi;
	while level >= lo && level > 0 {
		let mut i = 0;
		while i < runs.len() {
			if runs[i].2 >= level {
				let mut j = i;
				while j < runs.len() && runs[j].2 >= level {
					j += 1;
				}
				runs[i..j].reverse();
				i = j;
			} else {
				i += 1;
			}
		}
		level -= 1;
	}
	runs.into_iter().map(|(a, b, l)| (a, b, l % 2 == 1)).collect()
}

fn collect_range(
	p:			&Prep,
	start:		usize,
	end:		usize,
	trim:		&Trim,
	items:		&mut Vec<(usize, Item)>,
	fallback:	&mut Option<(usize, Item)>,
)
	-> Outcome<()>
{
	for i in p.slice(start, end) {
		let (sub, item) = &p.items[i];
		let idx = i + 1;
		let shaped = match item {
			Item::Text(t)	=> t,
			other			=> {
				items.push((idx, other.clone()));
				continue;
			}
		};
		let a = start.max(sub.0);
		let b = end.min(sub.1).min(trim.shaping);
		let split = sub.0 < a || b < sub.1;
		if a >= b {
			*fallback = Some((idx, Item::Text(shaped.empty())));
			continue;
		}
		let mut run = if split { res!(p.reshape(i, shaped, a, b)) } else { shaped.clone() };
		if trim.layout < end {
			let lim = trim.layout;
			run.trim(|g| g.range.0 >= lim);
		}
		items.push((idx, Item::Text(run)));
	}
	Ok(())
}

fn trim_weak_spacing(items: &mut Vec<(usize, Item)>) {
	let prefix = items.iter().take_while(|(_, i)| matches!(i, Item::Absolute(_, true))).count();
	if prefix > 0 {
		items.drain(..prefix);
	}
	while matches!(items.last(), Some((_, Item::Absolute(_, true)))) {
		items.pop();
	}
}

fn adjust_cj_at_line_boundaries(p: &Prep, start: usize, end: usize, l: &mut Line) {
	let text = match p.text.get(start..end) {
		Some(t)	=> t,
		None	=> return,
	};
	let cjl = p.config.cjk_latin_spacing;
	if text.starts_with(BEGIN_PUNCT) || (cjl && text.chars().next().map_or(false, is_of_cj_script)) {
		if let Some(t) = l.leading_text_mut() {
			if let Some(g) = t.kept().first().cloned() {
				if g.is_cjk_right_aligned_punctuation() {
					if let Some(gm) = t.kept_mut().first_mut() {
						let s = gm.shrink.0;
						gm.shrink_left(s);
					}
				} else if cjl && g.is_cj_script() && g.x_offset > 0.0 {
					if let Some(gm) = t.kept_mut().first_mut() {
						let s = gm.x_offset;
						gm.x_advance -= s;
						gm.x_offset = 0.0;
						gm.shrink.0 = 0.0;
					}
				}
			}
		}
	}
	if text.ends_with(END_PUNCT) || (cjl && text.chars().next_back().map_or(false, is_of_cj_script)) {
		if let Some(t) = l.trailing_text_mut() {
			let style = ftext::cjk_punct_style(&t.props.lang, t.props.region.as_deref());
			if let Some(g) = t.kept().last().cloned() {
				if g.is_cjk_left_aligned_punctuation(style) {
					if let Some(gm) = t.kept_mut().last_mut() {
						let s = gm.shrink.1;
						gm.shrink_right(s);
					}
				} else if cjl && g.is_cj_script() && (g.x_advance - g.x_offset) > 1.0 {
					if let Some(gm) = t.kept_mut().last_mut() {
						let s = gm.x_advance - gm.x_offset - 1.0;
						gm.x_advance -= s;
						gm.shrink.1 = 0.0;
					}
				}
			}
		}
	}
}

/// With glyph-level justification on, the last glyph of a line gives none, so no tracking trails it.
fn adjust_glyph_stretch_at_line_end(p: &Prep, l: &mut Line) {
	let _ = p;
	let limits = match l.trailing_text() {
		Some(t)	=> t.props.limits,
		None	=> return,
	};
	if limits.tracking_min == 0.0 && limits.tracking_max == 0.0 {
		return;
	}
	if let Some(t) = l.trailing_text_mut() {
		if let Some(g) = t.kept_mut().last_mut() {
			g.stretch = (0.0, 0.0);
			g.shrink = (0.0, 0.0);
		}
	}
}

fn should_repeat_hyphen(lang: &str, following: &str) -> bool {
	match lang {
		"dsb" | "cs" | "hr" | "pl" | "pt" | "sk"	=> true,
		"es"	=> following.chars().next().map_or(false, |c| !c.is_uppercase()),
		_		=> false,
	}
}

/// How far a character hangs into the end margin, as a fraction of its advance.
pub fn overhang(c: char) -> f64 {
	match c {
		'\u{2013}' | '\u{2014}'	=> 0.2,
		'-' | '\u{00AD}'		=> 0.55,
		'.' | ','				=> 0.8,
		':' | ';'				=> 0.3,
		'\u{060C}' | '\u{06D4}'	=> 0.4,
		_						=> 0.0,
	}
}

/// A committed line: its box, and its extent above and below the baseline, in points.
#[derive(Clone, Debug)]
pub struct LineBox {
	pub node:	Node,
	pub width:	f64,
	pub top:	f64,
	pub bottom:	f64,
	pub text:	String,	// what the line reads, for diagnostics and the oracle comparison
}

/// Commits a line to a box `width` points wide: hanging punctuation, justification, fractional spacing
/// and alignment, as Typst's `commit`.
pub fn commit(engine: &mut Engine, p: &Prep, l: &Line, width: f64, full: f64) -> Outcome<LineBox> {
	let hang = p.config.hanging_indent;
	let mut remaining = width - l.width - hang;
	let mut offset = 0.0f64;
	if !p.config.rtl {
		offset += hang;
	}

	// Punctuation hangs into the margin: on the left only for right-to-left text, on the right for left
	// to right, and not for a line that is one glyph alone.
	if let Some(t) = l.leading_text() {
		if let Some(g) = t.kept().first() {
			if t.rtl && t.props.overhang && (l.items.len() > 1 || t.kept().len() > 1) {
				let amount = overhang(g.c) * g.x_advance * g.size;
				offset -= amount;
				remaining += amount;
			}
		}
	}
	if let Some(t) = l.trailing_text() {
		if let Some(g) = t.kept().last() {
			if !t.rtl && t.props.overhang && (l.items.len() > 1 || t.kept().len() > 1) {
				let amount = overhang(g.c) * g.x_advance * g.size;
				remaining += amount;
			}
		}
	}

	let fr = l.fr();
	let mut ratio = 0.0f64;
	let mut extra = 0.0f64;
	let shrink = l.shrinkability();
	let stretch = l.stretchability();
	if remaining < 0.0 && shrink > 0.0 {
		ratio = (remaining / shrink).max(-1.0);
		remaining = (remaining + shrink).min(0.0);
	} else if l.justify && fr == 0.0 {
		if stretch > 0.0 {
			ratio = (remaining / stretch).min(1.0);
			remaining = (remaining - stretch).max(0.0);
		}
		let justifiables = l.justifiables();
		if justifiables > 0 && remaining > 0.0 {
			extra = remaining / justifiables as f64;
			remaining = 0.0;
		}
	}

	// First the extent every item gives the line, then the nodes at their places, in logical order as
	// Typst orders a line's frames, so an element inside is met where it stands in the text.
	enum Placed {
		Text(ShapedRun, f64, f64),	// the run, and its extent above and below the baseline
		Frame(Frame),
		Tag(RealiseTag),
	}
	let mut placed: Vec<(usize, f64, Placed, f64)> = Vec::new();	// (logical index, x, what, width)
	let mut top = 0.0f64;
	let mut bottom = 0.0f64;
	let mut cache = PropsCache::default();
	for (idx, item) in &l.items {
		match item {
			Item::Absolute(v, _)	=> offset += *v,
			Item::Fractional(v, elem) => {
				let amount = if fr > 0.0 && remaining.is_finite() { (v / fr * remaining).max(0.0) } else { 0.0 };
				match elem {
					Some(e) => {
						let (content, styles) = (&e.0, &e.1);
						let region = Region {
							width:		Sp::from_pt(amount),
							height:		Sp::from_pt(full),
							base:		(Sp::from_pt(amount), Sp::from_pt(full)),
							expand_x:	true,
							expand_y:	false,
						};
						let f = res!(layout_inline_box(engine, content, styles, region, Some(amount), &mut cache));
						top = top.max(f.baseline);
						bottom = bottom.max(f.height - f.baseline);
						let w = f.width;
						placed.push((*idx, offset, Placed::Frame(f), w));
						offset += w;
					}
					None => offset += amount,
				}
			}
			Item::Text(t) => {
				let (tt, bb) = t.measure(&p.book);
				top = top.max(tt);
				bottom = bottom.max(bb);
				let w = justified_width(t, ratio, extra);
				placed.push((*idx, offset, Placed::Text(t.clone(), tt, bb), w));
				offset += w;
			}
			Item::Frame(f) => {
				top = top.max(f.baseline);
				bottom = bottom.max(f.height - f.baseline);
				placed.push((*idx, offset, Placed::Frame(f.clone()), f.width));
				offset += f.width;
			}
			Item::Tag(tag)	=> placed.push((*idx, offset, Placed::Tag(tag.clone()), 0.0)),
			Item::Skip(_)	=> (),
		}
	}
	if fr != 0.0 {
		remaining = 0.0;
	}
	let shift_x = p.config.align.position(remaining);
	placed.sort_by_key(|(idx, ..)| *idx);

	// Every child sits on the line's baseline, TeX's rule: a leaf's height is its extent above the
	// baseline, and a frame is a box whose height is its baseline below its top.
	let mut nodes: Vec<Node> = Vec::new();
	let mut cursor = 0.0f64;
	for (_, x, what, w) in placed {
		let x = match &what {
			Placed::Frame(f)	=> x + shift_x + f.shift.0,
			_					=> x + shift_x,
		};
		if (x - cursor).abs() > 0.0 {
			nodes.push(Node::Glue(Glue::fixed(Sp::from_pt(x - cursor))));
		}
		cursor = x;
		match what {
			Placed::Text(t, tt, bb) => {
				// A link covers the run and half the leading above and below it, as Typst's text frames do.
				if let (Some(link), false) = (&t.props.link, t.props.hidden) {
					let out = t.props.link_outset;
					nodes.push(link_area(link, w, tt + out, bb + out, 0.0));
					nodes.push(Node::Glue(Glue::fixed(Sp::from_pt(-w))));
				}
				let (ns, adv) = res!(t.build(&p.book, ratio, extra));
				nodes.extend(ns);
				cursor += adv;
			}
			Placed::Frame(f) => {
				if let Some(link) = &f.link {
					nodes.push(link_area(link, w, f.baseline, f.height - f.baseline, f.shift.1));
					nodes.push(Node::Glue(Glue::fixed(Sp::from_pt(-w))));
				}
				nodes.push(frame_node(&f));
				cursor += w;
			}
			// Until the IR carries a located element's tags, its start is recorded as its anchor.
			Placed::Tag(RealiseTag::Start(content)) => if let Some(loc) = content.location() {
				nodes.push(Node::Anchor(loc.anchor()));
			},
			Placed::Tag(RealiseTag::End(_)) => (),
		}
	}
	// Rounded up, so a region taken from the line's measured width still holds the line.
	let dims = Dims::new(Sp::from_pt_up(width), Sp::from_pt(top), Sp::from_pt(bottom));
	Ok(LineBox { node: Node::HBox(BoxNode::new(nodes, dims)), width, top, bottom, text: line_text(l) })
}

/// A link area: an inkless graphic `width` wide reaching `above` and `below` the baseline, lowered by
/// `down`, whose placement box the writers make a link.
fn link_area(target: &LinkTarget, width: f64, above: f64, below: f64, down: f64) -> Node {
	let mut g = Graphic::new(Vec::new(), Dims::new(Sp::from_pt(width), Sp::from_pt(above), Sp::from_pt(below)));
	g.link = Some(target.clone());
	Node::Leaf(Leaf::graphic(g).with_shift(Sp::from_pt(down)))
}

/// A frame as a line holds it: a vertical box whose height is the frame's baseline, so it seats on the
/// line's baseline, its contents lowered by the frame's shift.
fn frame_node(f: &Frame) -> Node {
	let mut list = Vec::with_capacity(f.nodes.len() + 1);
	if f.shift.1 != 0.0 {
		list.push(Node::Glue(Glue::fixed(Sp::from_pt(f.shift.1))));
	}
	list.extend(f.nodes.iter().cloned());
	let dims = Dims::new(Sp::from_pt(f.width), Sp::from_pt(f.baseline), Sp::from_pt(f.height - f.baseline));
	Node::VBox(BoxNode::new(list, dims))
}

/// What a line reads: its runs' kept text, an object replacement for each inline frame.
fn line_text(l: &Line) -> String {
	let mut text = String::new();
	for (_, item) in &l.items {
		match item {
			Item::Text(t) => {
				let k = t.kept();
				let a = k.iter().map(|g| g.range.0).min();
				let b = k.iter().map(|g| g.range.1).max();
				if let (Some(a), Some(b)) = (a, b) {
					text.push_str(t.text.get(a.saturating_sub(t.base)..b.saturating_sub(t.base)).unwrap_or(""));
				}
			}
			Item::Frame(_)	=> text.push('\u{FFFC}'),
			_				=> (),
		}
	}
	text
}

/// The width a run takes once justified.
fn justified_width(t: &ShapedRun, ratio: f64, extra: f64) -> f64 {
	let mut w = 0.0;
	for g in t.kept() {
		let left = if ratio < 0.0 { g.shrink.0 } else { g.stretch.0 };
		let right = if ratio < 0.0 { g.shrink.1 } else { g.stretch.1 };
		let mut adv = g.x_advance * g.size + (left + right) * ratio * g.size;
		if g.justifiable {
			adv += extra;
		}
		w += adv;
	}
	w
}

/// Is `c` a space for justification? Re-exported for the breaker's estimates.
pub fn is_justifiable_space(c: char) -> bool { is_space(c) }

/// The chain's `text.lang` in force, for a uniform-language check.
pub fn lang_of(styles: &StyleChain) -> Outcome<String> {
	Ok(match res!(text_field(styles, "lang")) {
		Some(Value::Str(s))	=> s.to_lowercase(),
		_					=> "en".to_string(),
	})
}

/// The horizontal alignment the chain's `align` puts in force, fixed against the direction.
pub fn fixed_align(styles: &StyleChain, rtl: bool) -> Outcome<FixedAlign> {
	let a = res!(ftext::elem_field(styles, ElemKind::Align, "alignment"));
	let x = match a {
		Some(Value::Alignment(al))	=> al.x,
		_							=> None,
	};
	Ok(match x {
		None | Some(HAlign::Start)	=> FixedAlign::Start,
		Some(HAlign::End)			=> FixedAlign::End,
		Some(HAlign::Center)		=> FixedAlign::Center,
		Some(HAlign::Left)			=> if rtl { FixedAlign::End } else { FixedAlign::Start },
		Some(HAlign::Right)			=> if rtl { FixedAlign::Start } else { FixedAlign::End },
	})
}
