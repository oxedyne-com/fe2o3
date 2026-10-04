// SPDX-License-Identifier: Apache-2.0
// Original work copyright (c) the Typst project authors (typst-layout `flow/*`, `pad.rs` and `stack.rs`, version 0.15.1).
// Modified for Austenite: ported to Hematite's types, IR and error handling.
// U6b owns this file: block-level flow -- block, pad, align, stack, v with fr, place (float and parent
// scope), columns and colbreak -- dispatching inline runs to `par` and atoms to `grid`, `visual`, `math`.
//
// A port of Typst 0.15.1's `typst-layout` flow (`flow/{mod,collect,distribute,compose,block}.rs`, `pad.rs`,
// `stack.rs`), Apache-2.0, (c) the Typst authors. Realised block-level pairs are collected into children
// (spacing with a weakness, paragraph lines with their widow and orphan need, unbreakable and breakable
// blocks, placed elements, breaks), distributed into one region at a time, and composed with the region's
// insertions: floats at the top or foot of a column or of the whole page, and footnotes at the foot of a
// column. A float or footnote that changes the space left relays the region out. Output is a `Frame`
// per region, in points; `Frame::into_node` lowers it to an `ir::Node::Frame` the driver places.
//
// Everything Typst shows as a block -- a shape, an image, a transform, a grid, a pad, a stack, columns, a
// block equation, `layout` -- is a block here too, whose width, spacing, fill and the rest come from the
// `block` styles in force, as Typst's show rules make them `BlockElem`s.

use crate::diag::DiagnosticKind;
use crate::eval::content::{
	Content,
	ElemKind,
	Family,
};
use crate::eval::lib::visual as vis;
use crate::eval::locate::{
	Location,
	Locator,
	Place,
};
use crate::eval::realise::{
	self,
	Pair,
	Tag,
};
use crate::eval::styles::StyleChain;
use crate::eval::value::{
	Alignment,
	Dict,
	Direction,
	HAlign,
	Length,
	Ratio,
	Relative,
	VAlign,
	Value,
};
use crate::eval::{
	Context,
	Engine,
};
use crate::flow::grid::RowSplit;
use crate::flow::inline::ParSituation;
use crate::flow::page::Level;
use crate::flow::Region;
use crate::ir::{
	BoxNode,
	ClipNode,
	Dims,
	FrameNode,
	Node,
	Sp,
	Transform,
	TransformNode,
};
use crate::syntax::Span;

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_graphics::path::Path;

use std::cell::RefCell;
use std::collections::{
	HashMap,
	VecDeque,
};
use std::rc::Rc;

// Entry points

/// Lays block-level content out into one region, as Typst's `layout_frame`, and returns the frame as a
/// one-node vertical list whose box is the frame's size.
pub fn layout_flow(
	engine:		&mut Engine,
	content:	&Content,
	styles:		&StyleChain,
	region:		Region,
)
	-> Outcome<Vec<Node>>
{
	let frame	= res!(layout_frame(engine, content, styles, Regions::from_region(region)));
	let node	= frame.into_node();
	let dims	= match &node {
		Node::Frame(f)	=> Dims::new(f.dims.width, f.dims.vextent(), Sp::ZERO),
		_				=> Dims::default(),
	};
	Ok(vec![Node::VBox(BoxNode::new(vec![node], dims))])
}

/// `measure(content)`: the size content takes in the region, laid out and discarded. The whole height is
/// `height` (and so `vextent`), with no depth.
pub fn measure(
	engine:		&mut Engine,
	content:	&Content,
	styles:		&StyleChain,
	region:		Region,
)
	-> Outcome<Dims>
{
	let frame = res!(layout_frame(engine, content, styles, Regions::from_region(region)));
	Ok(Dims::new(Sp::from_pt(frame.w), Sp::from_pt(frame.h), Sp::ZERO))
}

/// Lays content out into the first of `regions` alone, Typst's `layout_frame`.
pub fn layout_frame(engine: &mut Engine, content: &Content, styles: &StyleChain, regions: Regions) -> Outcome<Frame> {
	layout_frame_in(engine, None, content, styles, regions)
}

/// As [`layout_frame`], for the body of the element that holds `place`: the content is realised at that place
/// whenever it is laid out, so a measurement and every later attempt locate what is in it alike.
pub fn layout_frame_in(engine: &mut Engine, place: Option<Place>, content: &Content, styles: &StyleChain, regions: Regions)
	-> Outcome<Frame>
{
	let one = Regions::one(regions.w, regions.h, regions.expand_x, regions.expand_y);
	let mut frames = res!(layout_fragment_in(engine, place, content, styles, one));
	Ok(match frames.is_empty() {
		true	=> Frame::new(0.0, 0.0),
		false	=> frames.swap_remove(0),
	})
}

/// Lays content out into as many of `regions` as it needs, Typst's `layout_fragment`: realised one level,
/// then flowed as blocks, or as the lines of one paragraph when the content is inline only.
pub fn layout_fragment(engine: &mut Engine, content: &Content, styles: &StyleChain, regions: Regions) -> Outcome<Vec<Frame>> {
	layout_fragment_in(engine, None, content, styles, regions)
}

/// As [`layout_fragment`], for the body of the element that holds `place`.
pub fn layout_fragment_in(
	engine:		&mut Engine,
	place:		Option<Place>,
	content:	&Content,
	styles:		&StyleChain,
	regions:	Regions,
)
	-> Outcome<Vec<Frame>>
{
	engine.within(place, |engine| engine.descend_fragment(content.span(), |engine| {
		let (pairs, inline) = res!(realise::realise_fragment(engine, content, styles));
		let mode	= if inline { FlowMode::Inline } else { FlowMode::Block };
		layout_flow_pairs(engine, pairs, styles, regions, 1, Rel::zero(), mode, content.span())
	}))
}

/// What a flow may hold: a page's root flow also hosts footnotes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FlowMode {
	Root,
	Block,
	Inline,
}

/// Lays realised pairs out into regions, perhaps in columns: Typst's `layout_flow`, for a caller whose
/// content is bounded. `regions` supplies every region the content may take, as a [`FlowCursor`] is drained
/// through them.
#[allow(clippy::too_many_arguments)]
pub fn layout_flow_pairs(
	engine:		&mut Engine,
	pairs:		Vec<Pair>,
	shared:		&StyleChain,
	mut regions: Regions,
	columns:	usize,
	gutter:		Rel,
	mode:		FlowMode,
	span:		Span,
)
	-> Outcome<Vec<Frame>>
{
	let mut cursor = res!(FlowCursor::new(engine, Feed::list(pairs), shared, &regions, columns, gutter, mode, span));
	let mut finished = Vec::new();
	loop {
		finished.push(res!(cursor.region(engine, &regions)));
		if res!(cursor.is_done(engine)) && (!regions.expand_y || regions.backlog.is_empty()) {
			break;
		}
		regions.next();
	}
	Ok(finished)
}

// Feed

/// Where a flow's realised pairs come from, in order.
#[derive(Clone, Debug)]
pub enum Feed {
	List(Rc<Vec<Pair>>, usize),		// pairs held whole, and the next to take
	Run(Rc<RefCell<Level>>),		// the open run of the paginator
}

impl Feed {
	pub fn list(pairs: Vec<Pair>) -> Self { Feed::List(Rc::new(pairs), 0) }

	fn next(&mut self, engine: &mut Engine) -> Outcome<Option<Pair>> {
		match self {
			Feed::List(pairs, at)	=> {
				let p = pairs.get(*at).cloned();
				if p.is_some() {
					*at += 1;
				}
				Ok(p)
			},
			Feed::Run(level)		=> level.borrow_mut().pull(engine),
		}
	}

	fn has_more(&mut self, engine: &mut Engine) -> Outcome<bool> {
		match self {
			Feed::List(pairs, at)	=> Ok(*at < pairs.len()),
			Feed::Run(level)		=> level.borrow_mut().has_more(engine),
		}
	}
}

// Cursor

/// What lazy collection reads from the flow's configuration: the size paragraphs are set against.
#[derive(Clone, Copy, Debug)]
struct CollectCfg {
	width:		f64,
	height:		f64,
	expand:		bool,
}

/// The children of a flow collected so far, and the feed they are collected from. A child is made when the
/// composer first asks for it, so a paragraph is set into lines when the cursor reaches it; those before the
/// first still needed are dropped.
#[derive(Clone, Debug)]
struct Pull {
	feed:		Feed,
	children:	VecDeque<Rc<Child>>,
	base:		usize,		// the index of the first child held
	done:		bool,		// the feed has no more pairs
	pulled:		usize,		// pairs taken from the feed
	par:		ParSituation,	// where the flow stands for the next paragraph's first-line indent
	cfg:		CollectCfg,
	mode:		FlowMode,
	memo:		Vec<Memo>,
}

impl Pull {
	/// The child at `idx`, collecting from the feed as far as needed; none once the feed has run out.
	fn get(&mut self, engine: &mut Engine, idx: usize) -> Outcome<Option<Rc<Child>>> {
		if idx < self.base {
			return Err(err!("Flow child {} was asked for after those before {} were dropped.", idx, self.base; Bug));
		}
		while idx >= self.base + self.children.len() {
			if self.done {
				return Ok(None);
			}
			let mut out = Vec::new();
			if self.mode == FlowMode::Inline {
				// A fragment that is inline only is one paragraph: all its pairs at once.
				let mut pairs = Vec::new();
				while let Some(p) = res!(self.feed.next(engine)) {
					pairs.push(p);
				}
				self.done = true;
				res!(collect_inline(engine, &pairs, self.cfg, &mut out));
			} else {
				let p = match res!(self.feed.next(engine)) {
					Some(p)	=> p,
					None	=> {
						self.done = true;
						return Ok(None);
					},
				};
				let alone = self.pulled == 0 && !res!(self.feed.has_more(engine));
				self.pulled += 1;
				res!(collect_pair(engine, &p, alone, self.cfg, &mut self.par, &mut out));
			}
			self.children.extend(out.into_iter().map(Rc::new));
		}
		Ok(self.children.get(idx - self.base).cloned())
	}

	/// Drops the children before `idx`, which nothing will ask for again.
	fn drop_before(&mut self, idx: usize) {
		while self.base < idx && !self.children.is_empty() {
			self.children.pop_front();
			self.base += 1;
		}
	}
}

/// A flow laid out region by region: Typst's `layout_flow`, as a cursor. Each call to
/// [`region`](Self::region) fills one region and keeps what remains, so a flow holds the children it has
/// collected and not yet placed, and what waits for a later region, never its whole content.
#[derive(Clone, Debug)]
pub struct FlowCursor {
	config:		Config,
	pull:		Pull,
	work:		Work,
	finished:	bool,
}

impl FlowCursor {
	/// A flow over `feed`, whose first region is `regions`. Columns, the gutter and the mode are the flow's
	/// own; the size paragraphs are set against is taken from the first region.
	#[allow(clippy::too_many_arguments)]
	pub fn new(
		engine:		&mut Engine,
		feed:		Feed,
		shared:		&StyleChain,
		regions:	&Regions,
		columns:	usize,
		gutter:		Rel,
		mode:		FlowMode,
		span:		Span,
	)
		-> Outcome<Self>
	{
		if !regions.w.is_finite() && regions.expand_x {
			return Err(engine.error(DiagnosticKind::Type, span, "cannot expand into infinite width"));
		}
		if !regions.h.is_finite() && regions.expand_y {
			return Err(engine.error(DiagnosticKind::Type, span, "cannot expand into infinite height"));
		}
		let config	= res!(configuration(shared, regions, columns, gutter, mode));
		let cfg		= CollectCfg { width: config.columns.width, height: regions.full, expand: regions.expand_x };
		Ok(Self {
			config,
			pull:		Pull {
					feed, children: VecDeque::new(), base: 0, done: false, pulled: 0, par: ParSituation::First, cfg, mode,
					memo: Vec::new(),
				},
			work:		Work::new(),
			finished:	false,
		})
	}

	/// The next region's frame, or none after the flow was finished by the one before.
	pub fn next_region(&mut self, engine: &mut Engine, regions: &Regions) -> Outcome<Option<Frame>> {
		if self.finished {
			return Ok(None);
		}
		let frame = res!(self.region(engine, regions));
		self.finished = res!(self.is_done(engine));
		Ok(Some(frame))
	}

	/// Fills one region, whether or not anything is left: an empty frame is a region the flow spans after its
	/// content has ended, as a block of fixed height does.
	pub fn region(&mut self, engine: &mut Engine, regions: &Regions) -> Outcome<Frame> {
		let frame = res!(compose(engine, &mut self.pull, &mut self.work, &self.config, regions));
		self.pull.drop_before(self.work.idx);
		self.pull.memo.clear();
		self.work.prune();
		Ok(frame)
	}

	/// Is every child placed, and nothing queued for a later region?
	pub fn is_done(&mut self, engine: &mut Engine) -> Outcome<bool> {
		if self.work.spill.is_some()
			|| !self.work.floats.is_empty()
			|| self.work.footnote_spill.is_some()
			|| !self.work.footnotes.is_empty()
		{
			return Ok(false);
		}
		Ok(res!(self.pull.get(engine, self.work.idx)).is_none())
	}
}

// Units

const EPS: f64 = 1e-4;	// Typst's `Abs` tolerance, in points

/// Does `need` fit into `avail`, with Typst's slack?
fn fits(avail: f64, need: f64) -> bool { avail + EPS >= need }

/// Is the length close to zero or negative?
fn approx_empty(v: f64) -> bool { v <= EPS }

/// A relative length with its absolute part resolved to points.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rel {
	pub ratio:	f64,
	pub abs:	f64,
}

impl Rel {
	pub fn zero() -> Self { Self::default() }

	pub fn pt(abs: f64) -> Self { Self { ratio: 0.0, abs } }

	/// The length against `whole`; a ratio of an infinite whole is nothing, as Typst's `Ratio::of`.
	pub fn relative_to(&self, whole: f64) -> f64 {
		let part = self.ratio * whole;
		(if part.is_finite() { part } else { 0.0 }) + self.abs
	}

	pub fn is_zero(&self) -> bool { self.ratio == 0.0 && self.abs == 0.0 }
}

/// How a container is sized along an axis.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Sizing {
	Auto,
	Rel(Rel),
	Fr(f64),
}

/// Spacing: relative, or a share of what is left.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Spacing {
	Rel(Rel),
	Fr(f64),
}

/// An alignment resolved against the text direction: Typst's `FixedAlignment`, ordered start to end.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Fixed {
	Start,
	Center,
	End,
}

impl Fixed {
	/// The offset of something aligned so within `extent` of free space.
	pub fn position(self, extent: f64) -> f64 {
		match self {
			Fixed::Start	=> 0.0,
			Fixed::Center	=> extent / 2.0,
			Fixed::End		=> extent,
		}
	}
}

/// A resolved alignment on both axes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Align2 {
	pub x:	Fixed,
	pub y:	Fixed,
}

impl Align2 {
	pub const START: Align2 = Align2 { x: Fixed::Start, y: Fixed::Start };
}

/// A size in points, either axis possibly infinite.
fn sp(pt: f64) -> Sp {
	if !pt.is_finite() || pt >= Sp(1 << 30).to_pt() { Sp(i32::MAX) } else { Sp::from_pt(pt) }
}

fn pt_of(s: Sp) -> f64 {
	if s.0 >= (1 << 30) { f64::INFINITY } else { s.to_pt() }
}

// Regions

/// Typst's regions: the space left in the first region, the full height it started with, the heights of
/// the regions to come, and the height that repeats once they run out.
#[derive(Clone, Debug, PartialEq)]
pub struct Regions {
	pub w:			f64,
	pub h:			f64,
	pub full:		f64,
	pub backlog:	Vec<f64>,
	pub last:		Option<f64>,
	pub expand_x:	bool,
	pub expand_y:	bool,
}

impl Regions {
	/// One region only.
	pub fn one(w: f64, h: f64, expand_x: bool, expand_y: bool) -> Self {
		Self { w, h, full: h, backlog: Vec::new(), last: None, expand_x, expand_y }
	}

	/// A region repeated without end: a page run's pages.
	pub fn repeat(w: f64, h: f64, expand_x: bool, expand_y: bool) -> Self {
		Self { w, h, full: h, backlog: Vec::new(), last: Some(h), expand_x, expand_y }
	}

	/// The contract's region, as one region.
	pub fn from_region(r: Region) -> Self {
		let w = pt_of(r.width);
		let h = pt_of(r.height);
		let mut out = Self::one(w, h, r.expand_x, r.expand_y);
		out.full = pt_of(r.base.1);
		if out.full < h {
			out.full = h;
		}
		out
	}

	/// The first region as the contract's region, relative lengths resolving against its full size.
	pub fn region(&self) -> Region {
		Region {
			width:		sp(self.w),
			height:		sp(self.h),
			base:		(sp(self.w), sp(self.full)),
			expand_x:	self.expand_x,
			expand_y:	self.expand_y,
		}
	}

	/// The size relative lengths resolve against.
	pub fn base(&self) -> (f64, f64) { (self.w, self.full) }

	/// Is the first region full, with another one to go to?
	pub fn is_full(&self) -> bool { fits(0.0, self.h) && self.may_progress() }

	/// Is there a region after the first?
	pub fn may_break(&self) -> bool { !self.backlog.is_empty() || self.last.is_some() }

	/// Would moving to the next region give more room?
	pub fn may_progress(&self) -> bool {
		!self.backlog.is_empty() || self.last.map(|l| self.h != l).unwrap_or(false)
	}

	/// Moves to the next region, if there is one.
	pub fn next(&mut self) {
		let height = if self.backlog.is_empty() {
			self.last
		} else {
			Some(self.backlog.remove(0))
		};
		if let Some(h) = height {
			self.h		= h;
			self.full	= h;
		}
	}

	/// The height of the `i`th region from this one, the first being `self.h`.
	pub fn nth(&self, i: usize) -> Option<f64> {
		if i == 0 {
			return Some(self.h);
		}
		match self.backlog.get(i - 1) {
			Some(h)	=> Some(*h),
			None	=> self.last,
		}
	}

	/// A hash of everything a layout could depend on, for caching a child's layout.
	fn key(&self) -> u64 {
		let mut h = Fnv(0xcbf2_9ce4_8422_2325);
		for v in [self.w, self.h, self.full, self.last.unwrap_or(-1.0)] {
			h.f(v);
		}
		for v in &self.backlog {
			h.f(*v);
		}
		h.b(self.last.is_some());
		h.b(self.expand_x);
		h.b(self.expand_y);
		h.0
	}

	/// The regions with every height shrunk by `f`, the widths likewise.
	fn map<F: Fn(f64, f64) -> (f64, f64)>(&self, f: F) -> Regions {
		let (w, h)	= f(self.w, self.h);
		let (_, full)	= f(self.w, self.full);
		Regions {
			w,
			h,
			full,
			backlog:	self.backlog.iter().map(|y| f(self.w, *y).1).collect(),
			last:		self.last.map(|y| f(self.w, y).1),
			expand_x:	self.expand_x,
			expand_y:	self.expand_y,
		}
	}
}

struct Fnv(u64);

impl Fnv {
	fn f(&mut self, v: f64) {
		for b in v.to_bits().to_le_bytes() {
			self.0 ^= b as u64;
			self.0 = self.0.wrapping_mul(0x0000_0100_0000_01b3);
		}
	}

	fn b(&mut self, v: bool) {
		self.0 ^= v as u64;
		self.0 = self.0.wrapping_mul(0x0000_0100_0000_01b3);
	}
}

// Frames

/// Typst's frame: a sized box of items placed by their top-left, in points, y down. `baseline` is from the
/// top; none means the bottom.
#[derive(Clone, Debug, Default)]
pub struct Frame {
	pub w:			f64,
	pub h:			f64,
	pub baseline:	Option<f64>,
	pub items:		Vec<(f64, f64, Item)>,
	pub parent:		Option<u64>,	// the mark this frame is read at for introspection, as `FrameNode::parent`
}

/// What a frame holds.
#[derive(Clone, Debug)]
pub enum Item {
	Frame(Frame),						// a nested frame
	Node(Node),							// laid-out IR: a line, a leaf, a box
	Group(Frame, Transform, Option<Path>),	// a frame drawn under a transform and clipped to a path
	Tag(Tag),							// where a located element starts or ends
}

impl Frame {
	pub fn new(w: f64, h: f64) -> Self { Self { w, h, baseline: None, items: Vec::new(), parent: None } }

	/// An IR node as a frame of its size.
	pub fn from_node(node: Node) -> Self {
		let (w, h, base) = match node_dims(&node) {
			Some(d)	=> (d.width.to_pt(), d.vextent().to_pt(), d.height.to_pt()),
			None	=> (0.0, 0.0, 0.0),
		};
		let mut f = Frame::new(w, h);
		f.baseline = Some(base);
		f.items.push((0.0, 0.0, Item::Node(node)));
		f
	}

	pub fn is_empty(&self) -> bool { self.items.is_empty() }

	pub fn has_baseline(&self) -> bool { self.baseline.is_some() }

	pub fn baseline(&self) -> f64 { self.baseline.unwrap_or(self.h) }

	pub fn push(&mut self, x: f64, y: f64, item: Item) { self.items.push((x, y, item)); }

	pub fn push_frame(&mut self, x: f64, y: f64, frame: Frame) {
		if frame.is_empty() {
			return;
		}
		self.items.push((x, y, Item::Frame(frame)));
	}

	/// Puts items behind everything the frame holds.
	pub fn prepend(&mut self, items: Vec<(f64, f64, Item)>) {
		let mut v = items;
		v.append(&mut self.items);
		self.items = v;
	}

	/// Moves the contents and the baseline.
	pub fn translate(&mut self, dx: f64, dy: f64) {
		if dx == 0.0 && dy == 0.0 {
			return;
		}
		if let Some(b) = &mut self.baseline {
			*b += dy;
		}
		for (x, y, _) in &mut self.items {
			*x += dx;
			*y += dy;
		}
	}

	/// Wraps the contents in one group, as Typst does to carry a label: the frame is no longer empty, even
	/// when it held nothing, which is what lets a labelled empty block end a run of sticky blocks.
	pub fn label(&mut self) {
		let inner = Frame {
			w:			self.w,
			h:			self.h,
			baseline:	None,
			items:		std::mem::take(&mut self.items),
			parent:		None,
		};
		self.items.push((0.0, 0.0, Item::Frame(inner)));
	}

	/// Clips the contents to `path`, in the frame's coordinates.
	pub fn clip(&mut self, path: Path) {
		if self.items.is_empty() {
			return;
		}
		let inner = Frame { w: self.w, h: self.h, baseline: None, items: std::mem::take(&mut self.items), parent: None };
		self.items.push((0.0, 0.0, Item::Group(inner, Transform::identity(), Some(path))));
	}

	/// Drops every visible item, keeping the tags, as Typst's `hide` modifier does.
	pub fn hide(&mut self) {
		let mut kept = Vec::new();
		for (x, y, item) in std::mem::take(&mut self.items) {
			match item {
				Item::Tag(t) => kept.push((x, y, Item::Tag(t))),
				Item::Frame(mut f) | Item::Group(mut f, _, _) => {
					f.hide();
					if !f.is_empty() {
						kept.push((x, y, Item::Frame(f)));
					}
				},
				Item::Node(n) => {
					let mut tags = Vec::new();
					node_tags(&n, 0.0, &mut tags);
					for (ty, t) in tags {
						kept.push((x, y + ty, Item::Tag(t)));
					}
				},
			}
		}
		self.items = kept;
	}

	/// The frame as an IR frame node, its box split at the baseline.
	pub fn into_node(self) -> Node {
		let base	= self.baseline();
		let dims	= Dims::new(Sp::from_pt(self.w), Sp::from_pt(base), Sp::from_pt(self.h - base));
		let mut out	= FrameNode::new(dims);
		out.parent	= self.parent;
		for (x, y, item) in self.items {
			let node = match item {
				Item::Frame(f)	=> f.into_node(),
				Item::Node(n)	=> n,
				Item::Tag(t)	=> Node::Tag(t),
				Item::Group(f, t, clip) => {
					let fd = Dims::new(Sp::from_pt(f.w), Sp::from_pt(f.h), Sp::ZERO);
					let inner = f.into_node();
					let clipped = match clip {
						Some(path)	=> Node::Clip(ClipNode { list: vec![inner], dims: fd, path: Some(path) }),
						None		=> inner,
					};
					if t == Transform::identity() {
						clipped
					} else {
						Node::Transform(TransformNode { transform: t, list: vec![clipped], dims: fd })
					}
				},
			};
			out.items.push((Sp::from_pt(x), Sp::from_pt(y), node));
		}
		Node::Frame(out)
	}
}

/// A node's box, when it has one.
pub fn node_dims(node: &Node) -> Option<Dims> {
	match node {
		Node::HBox(b) | Node::VBox(b)	=> Some(b.dims),
		Node::Leaf(l)					=> Some(l.dims),
		Node::Frame(f)					=> Some(f.dims),
		Node::Transform(t)				=> Some(t.dims),
		Node::Clip(c)					=> Some(c.dims),
		_								=> None,
	}
}

/// The tags in a node tree, with their offsets down from its top.
fn node_tags(node: &Node, y: f64, out: &mut Vec<(f64, Tag)>) {
	match node {
		Node::Tag(t) => out.push((y, t.clone())),
		Node::VBox(b) => {
			let mut yy = y;
			for c in &b.list {
				node_tags(c, yy, out);
				yy += c.vextent().to_pt();
			}
		},
		Node::HBox(b) => {
			let base = y + b.dims.height.to_pt();
			for c in &b.list {
				match c {
					Node::Tag(t)	=> out.push((base, t.clone())),
					other			=> {
						let top = node_dims(other).map(|d| base - d.height.to_pt()).unwrap_or(y);
						node_tags(other, top, out);
					},
				}
			}
		},
		Node::Frame(f) => for (_, dy, c) in &f.items { node_tags(c, y + dy.to_pt(), out); },
		Node::Transform(t) => {
			let mut yy = y;
			for c in &t.list {
				node_tags(c, yy, out);
				yy += c.vextent().to_pt();
			}
		},
		Node::Clip(c) => {
			let mut yy = y;
			for n in &c.list {
				node_tags(n, yy, out);
				yy += n.vextent().to_pt();
			}
		},
		_ => (),
	}
}

/// The footnotes whose start tags a frame holds, with their offsets down from its top.
pub fn find_footnotes(frame: &Frame, y0: f64, out: &mut Vec<(f64, Content)>) {
	for (_, y, item) in &frame.items {
		let y = y0 + y;
		match item {
			Item::Frame(f) | Item::Group(f, _, _) => find_footnotes(f, y, out),
			Item::Tag(Tag::Start(c)) => if c.is(ElemKind::Footnote) {
				out.push((y, c.clone()));
			},
			Item::Tag(_) => (),
			Item::Node(n) => {
				let mut tags = Vec::new();
				node_tags(n, y, &mut tags);
				for (ty, t) in tags {
					if let Tag::Start(c) = t {
						if c.is(ElemKind::Footnote) {
							out.push((ty, c));
						}
					}
				}
			},
		}
	}
}

// Style access

/// A field's value in force for an element: its own (folded onto the chain's for a folding field), else
/// the chain's, else the schema default.
pub(super) fn field(elem: &Content, styles: &StyleChain, name: &str) -> Outcome<Option<Value>> {
	let kind = match elem.kind() {
		Some(k)	=> k,
		None	=> return Ok(None),
	};
	match kind.field_id(name) {
		Some(id)	=> styles.resolve(elem, id),
		None		=> Ok(None),
	}
}

/// A field of another element kind, as the chain has it.
pub(super) fn chain_field(styles: &StyleChain, kind: ElemKind, name: &str) -> Outcome<Option<Value>> {
	match kind.field_id(name) {
		Some(id)	=> styles.get(kind, id),
		None		=> Ok(None),
	}
}

/// A length in points, its em part at the font size in force.
pub(super) fn abs_of(styles: &StyleChain, l: Length) -> f64 { styles.resolve_length(l) }

/// A relative length from any value Typst accepts as one.
pub(super) fn rel_of(styles: &StyleChain, v: &Value) -> Option<Rel> {
	match v {
		Value::Length(l)	=> Some(Rel::pt(abs_of(styles, *l))),
		Value::Ratio(r)		=> Some(Rel { ratio: r.0, abs: 0.0 }),
		Value::Relative(r)	=> Some(Rel { ratio: r.rel.0, abs: abs_of(styles, r.abs) }),
		Value::Int(0)		=> Some(Rel::zero()),
		_					=> None,
	}
}

fn sizing_of(styles: &StyleChain, v: Option<Value>) -> Sizing {
	match v {
		Some(Value::Fraction(f))	=> Sizing::Fr(f.0),
		Some(other)					=> match rel_of(styles, &other) {
			Some(r)	=> Sizing::Rel(r),
			None	=> Sizing::Auto,
		},
		None						=> Sizing::Auto,
	}
}

fn spacing_of(styles: &StyleChain, v: &Value) -> Option<Spacing> {
	match v {
		Value::Fraction(f)	=> Some(Spacing::Fr(f.0)),
		other				=> rel_of(styles, other).map(Spacing::Rel),
	}
}

fn bool_of(v: Option<Value>, default: bool) -> bool {
	match v {
		Some(Value::Bool(b))	=> b,
		_						=> default,
	}
}

/// Is the text direction in force right to left?
pub(super) fn rtl(styles: &StyleChain) -> Outcome<bool> {
	Ok(matches!(res!(chain_field(styles, ElemKind::Text, "dir")), Some(Value::Direction(Direction::Rtl))))
}

/// A horizontal alignment as a position from the left: `left` and `right` are physical, `start` and
/// `end` follow the text direction.
fn fix_h(h: HAlign, rtl: bool) -> Fixed {
	match h {
		HAlign::Left	=> Fixed::Start,
		HAlign::Right	=> Fixed::End,
		HAlign::Center	=> Fixed::Center,
		HAlign::Start	=> if rtl { Fixed::End } else { Fixed::Start },
		HAlign::End		=> if rtl { Fixed::Start } else { Fixed::End },
	}
}

fn fix_v(v: VAlign) -> Fixed {
	match v {
		VAlign::Top		=> Fixed::Start,
		VAlign::Horizon	=> Fixed::Center,
		VAlign::Bottom	=> Fixed::End,
	}
}

/// The alignment in force, folded axis by axis from the innermost `align` style outward, `start + top`
/// where none says otherwise.
pub fn alignment(styles: &StyleChain) -> Outcome<Align2> {
	let (mut x, mut y) = (None, None);
	if let Some(id) = ElemKind::Align.field_id("alignment") {
		for v in styles.values(ElemKind::Align, id) {
			if let Value::Alignment(a) = v {
				if x.is_none() {
					x = a.x;
				}
				if y.is_none() {
					y = a.y;
				}
			}
			if x.is_some() && y.is_some() {
				break;
			}
		}
	}
	let r = res!(rtl(styles));
	Ok(Align2 {
		x:	fix_h(x.unwrap_or(HAlign::Start), r),
		y:	fix_v(y.unwrap_or(VAlign::Top)),
	})
}

/// The four sides of a sides value (`left, top, right, bottom`), each resolved to a relative length,
/// zero where unset: a single value, or a dictionary by Typst's precedence of a side over its axis over
/// `rest`.
fn sides_rel(styles: &StyleChain, v: Option<Value>) -> [Rel; 4] {
	let parts = sides_values(v);
	parts.map(|p| p.and_then(|v| rel_of(styles, &v)).unwrap_or_default())
}

fn sides_values(v: Option<Value>) -> [Option<Value>; 4] {
	match v {
		None | Some(Value::None) | Some(Value::Auto) => [None, None, None, None],
		Some(Value::Dict(d)) => {
			let get		= |k: &str| d.get(k).cloned();
			let rest	= get("rest");
			let x		= get("x").or_else(|| rest.clone());
			let y		= get("y").or_else(|| rest.clone());
			[
				get("left").or_else(|| get("inside")).or_else(|| x.clone()),
				get("top").or_else(|| y.clone()),
				get("right").or_else(|| get("outside")).or_else(|| x.clone()),
				get("bottom").or_else(|| y.clone()),
			]
		},
		Some(other) => [Some(other.clone()), Some(other.clone()), Some(other.clone()), Some(other)],
	}
}

fn sides_zero(s: &[Rel; 4]) -> bool { s.iter().all(|r| r.is_zero()) }

/// Shrinks a size by an inset relative to the size itself: Typst's `pad::shrink`.
fn shrink(w: f64, h: f64, inset: &[Rel; 4]) -> (f64, f64) {
	let x = Rel { ratio: inset[0].ratio + inset[2].ratio, abs: inset[0].abs + inset[2].abs };
	let y = Rel { ratio: inset[1].ratio + inset[3].ratio, abs: inset[1].abs + inset[3].abs };
	(w - x.relative_to(w), h - y.relative_to(h))
}

/// Grows a frame by an inset relative to the grown size, the inverse of [`shrink`]: Typst's `pad::grow`.
fn grow(frame: &mut Frame, inset: &[Rel; 4]) {
	let px = Rel { ratio: inset[0].ratio + inset[2].ratio, abs: inset[0].abs + inset[2].abs };
	let py = Rel { ratio: inset[1].ratio + inset[3].ratio, abs: inset[1].abs + inset[3].abs };
	let w = (frame.w + px.abs) / (1.0 - px.ratio);
	let h = (frame.h + py.abs) / (1.0 - py.ratio);
	let left	= inset[0].relative_to(w);
	let top		= inset[1].relative_to(h);
	frame.w = w;
	frame.h = h;
	frame.translate(left, top);
}

// Children

/// What waits to be placed with the next frame: a tag, or the mark of a float's place in the flow.
#[derive(Clone, Debug)]
enum Pending {
	Tag(Tag),
	Mark(u64),
}

impl Pending {
	fn item(self) -> Item {
		match self {
			Pending::Tag(t)		=> Item::Tag(t),
			Pending::Mark(m)	=> Item::Node(Node::Mark(m)),
		}
	}
}

/// A prepared child of a flow, Typst's `Child`.
#[derive(Debug)]
enum Child {
	Tag(Tag),
	Mark(u64),		// where a float's `place` stands
	Rel(Rel, u8),	// spacing and its weakness: 0 strong, then block (3), paragraph (4) and leading (5)
	Fr(f64, u8),
	Line(LineChild),
	Single(Rc<SingleChild>),
	Multi(Rc<MultiChild>),
	Placed(Rc<PlacedChild>),
	Flush,
	Break(bool),	// a column break, weak or not
}

/// A laid-out line of a paragraph, and the height it needs to keep with its widow or orphan partner.
#[derive(Debug)]
struct LineChild {
	frame:	Frame,
	align:	Align2,
	need:	f64,
}

/// What a block lays out: nothing, content, or its element by that element's own layout routine (a shape,
/// a grid, a pad, a stack, columns, an equation, `layout`).
#[derive(Clone, Debug)]
enum Body {
	Empty,
	Content(Content),
	Layouter,
}

/// A block and the styles it is laid out under. An explicit `block` reads its fields; an element Typst
/// shows as a block reads the `block` styles in force.
#[derive(Clone, Debug)]
struct BlockSpec {
	elem:		Content,
	styles:		StyleChain,
	body:		Body,
	explicit:	bool,
}

impl BlockSpec {
	fn field(&self, name: &str) -> Outcome<Option<Value>> {
		if self.explicit {
			field(&self.elem, &self.styles, name)
		} else {
			chain_field(&self.styles, ElemKind::Block, name)
		}
	}

	/// Width and height: an explicit block's own; an element's layout sizes itself.
	fn sizing(&self, name: &str) -> Outcome<Sizing> {
		if self.explicit {
			Ok(sizing_of(&self.styles, res!(self.field(name))))
		} else {
			Ok(Sizing::Auto)
		}
	}

	fn span(&self) -> Span { self.elem.span() }
}

#[derive(Debug)]
struct SingleChild {
	align:	Align2,
	sticky:	bool,
	alone:	bool,
	fr:		Option<f64>,
	spec:	BlockSpec,
	cell:	RefCell<Option<(u64, Frame)>>,
}

impl SingleChild {
	/// The block laid out in the region's base size; vertical expansion only when it is the flow's sole child.
	fn layout(&self, engine: &mut Engine, w: f64, h: f64, expand_x: bool, expand_y: bool) -> Outcome<Frame> {
		let region = Regions::one(w, h, expand_x, expand_y && self.alone);
		let key = region.key();
		if let Some((k, f)) = &*self.cell.borrow() {
			if *k == key {
				return Ok(f.clone());
			}
		}
		let frame = res!(engine.descend(|engine| layout_single(engine, &self.spec, &region)));
		*self.cell.borrow_mut() = Some((key, frame.clone()));
		Ok(frame)
	}
}

/// A breakable block. It lays out one fragment per region, each from the continuation the one before
/// left: see [`MultiChild::fragment`].
#[derive(Debug)]
struct MultiChild {
	align:	Align2,
	sticky:	bool,
	alone:	bool,
	spec:	BlockSpec,
}

/// A fragment laid out in the region being composed, kept so a region laid out again from the same state, as
/// a float or a footnote asks for, takes it back instead of laying the block out again. The flow keeps them,
/// not the block, so that a block holds no continuation that holds the block; they go when the region is
/// finished.
#[derive(Clone, Debug)]
struct Memo {
	child:	usize,					// the block, by address
	from:	usize,					// the continuation it came from, by address, or zero for the first
	key:	u64,					// the regions the fragment was laid out in
	out:	Fragment,
}

/// A breakable block's fragment as the flow places it, and what is needed to go on from it.
#[derive(Clone, Debug)]
struct Fragment {
	frame:	Frame,
	next:	Option<Rc<Spill>>,	// what lays out the fragment after, none once the block has ended here
	skip:	bool,				// empty here while a later fragment is not: the block waits for the next region
}

/// What carries a breakable block over a region break: the means of laying out its next fragment, and
/// what a block of fixed height has left. A [`Work`] holds one, and a copy of the work holds the same one,
/// so laying a region out again starts from the right place.
#[derive(Debug)]
struct Spill {
	child:	Rc<MultiChild>,
	nested:	Nested,
	k:		usize,					// fragments yielded
	fixed:	Option<(f64, f64)>,		// a fixed height, whole and what is left of it
}

/// What lays out a block's fragments.
#[derive(Clone, Debug)]
enum Nested {
	Flow(Box<FlowCursor>, Option<[Rel; 4]>),	// the body flowed through the regions, grown by a pad's padding
	Frames(VecDeque<Frame>, Regions),			// frames laid out as one piece, and the regions they were laid out in
}

/// A placed child's vertical alignment: `auto` (a float's choice), none (in the flow), or an edge.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PlaceY {
	Auto,
	Flow,
	At(Fixed),
}

#[derive(Debug)]
struct PlacedChild {
	align_x:	Fixed,
	align_y:	PlaceY,
	parent:		bool,	// `scope: "parent"`
	float:		bool,
	clearance:	f64,
	dx:			Rel,
	dy:			Rel,
	alignment:	Option<Alignment>,	// `auto` is none
	mark:		u64,				// the float's place in the flow, its frame's parent
	elem:		Content,
	styles:		StyleChain,
	cell:		RefCell<Option<(u64, Frame)>>,
}

impl PlacedChild {
	/// The body laid out in the base size, unexpanded, under the placement's alignment.
	fn layout(&self, engine: &mut Engine, base: (f64, f64)) -> Outcome<Frame> {
		let region = Regions::one(base.0, base.1, false, false);
		let key = region.key();
		if let Some((k, f)) = &*self.cell.borrow() {
			if *k == key {
				return Ok(f.clone());
			}
		}
		let align = match self.alignment {
			Some(a)	=> a,
			None	=> Alignment { x: Some(HAlign::Center), y: Some(VAlign::Horizon) },
		};
		let styles = match ElemKind::Align.field_id("alignment") {
			Some(id) => self.styles.chain(&crate::eval::styles::Styles::from_style(
				crate::eval::styles::Style::Property(crate::eval::styles::Property::new(
					ElemKind::Align, id, Value::Alignment(align), self.elem.span())))),
			None => self.styles.clone(),
		};
		let body = match res!(field(&self.elem, &self.styles, "body")) {
			Some(v)	=> res!(v.cast::<Content>()),
			None	=> Content::empty(),
		};
		let mut frame = res!(layout_frame_in(engine, self.elem.place(), &body, &styles, region));
		if res!(vis::is_hidden(&self.styles)) {
			frame.hide();
		}
		if self.float {
			frame.parent = Some(self.mark);
		}
		*self.cell.borrow_mut() = Some((key, frame.clone()));
		Ok(frame)
	}
}

// Collection

/// Collects one realised pair into prepared children, Typst's `collect`. A paragraph is set into lines
/// here, when the cursor reaches it, since line layout does not depend on the concrete regions. `alone` is
/// whether the pair is the whole of the flow, which lets a block expand into the region's height.
fn collect_pair(
	engine:		&mut Engine,
	p:			&Pair,
	alone:		bool,
	cfg:		CollectCfg,
	par:		&mut ParSituation,
	out:		&mut Vec<Child>,
)
	-> Outcome<()>
{
	if let Some(t) = &p.tag {
		out.push(Child::Tag(t.clone()));
		return Ok(());
	}
	let c		= &p.content;
	let styles	= &p.styles;
	let kind	= match c.kind() {
		Some(k)	=> k,
		None	=> return Ok(()),
	};
	match kind {
		ElemKind::V => {
			let amount	= res!(field(c, styles, "amount"));
			let weak	= bool_of(res!(field(c, styles, "weak")), false) as u8;
			match amount.as_ref().and_then(|a| spacing_of(styles, a)) {
				Some(Spacing::Rel(r))	=> out.push(Child::Rel(r, weak)),
				Some(Spacing::Fr(f))	=> out.push(Child::Fr(f, weak)),
				None					=> (),
			}
		},
		ElemKind::Par		=> {
			res!(collect_par(engine, c, styles, cfg, *par, out));
			*par = ParSituation::Consecutive;
		},
		ElemKind::Block		=> {
			let body = match res!(field(c, styles, "body")) {
				None | Some(Value::None)	=> Body::Empty,
				Some(v)						=> Body::Content(res!(v.cast::<Content>())),
			};
			res!(collect_block(BlockSpec { elem: c.clone(), styles: styles.clone(), body, explicit: true }, alone, None, out));
			*par = ParSituation::Other;
		},
		ElemKind::Place		=> res!(collect_place(engine, c, styles, out)),
		ElemKind::PlaceFlush	=> out.push(Child::Flush),
		ElemKind::Colbreak	=> {
			out.push(Child::Break(bool_of(res!(field(c, styles, "weak")), false)));
			*par = ParSituation::First;
		},
		ElemKind::Pagebreak	=> return Err(engine.error_hint(DiagnosticKind::Type, c.span(),
			"pagebreaks are not allowed inside of containers", "try using a `#colbreak()` instead")),
		// Shown by Typst as breakable blocks with a layout routine of their own.
		ElemKind::Pad | ElemKind::Stack | ElemKind::Columns | ElemKind::Grid | ElemKind::Table
			| ElemKind::Layout | ElemKind::Equation | ElemKind::List | ElemKind::Enum => {
			let spec = BlockSpec { elem: c.clone(), styles: styles.clone(), body: Body::Layouter, explicit: false };
			res!(collect_block(spec, alone, Some(true), out));
			*par = ParSituation::Other;
		},
		// Shown by Typst as unbreakable blocks with a layout routine of their own; a fractional height
		// makes the block a share of the region's free space.
		ElemKind::Image | ElemKind::Line | ElemKind::Rect | ElemKind::Square | ElemKind::Circle
			| ElemKind::Ellipse | ElemKind::Polygon | ElemKind::Curve | ElemKind::Move | ElemKind::Scale
			| ElemKind::Rotate | ElemKind::Skew | ElemKind::Repeat => {
			let spec = BlockSpec { elem: c.clone(), styles: styles.clone(), body: Body::Layouter, explicit: false };
			res!(collect_block(spec, alone, Some(false), out));
			*par = ParSituation::Other;
		},
		// A tag carried as an element marks where its element starts or ends.
		ElemKind::Tag => if let Some(t) = Tag::from_content(c) {
			out.push(Child::Tag(t));
		},
		// Introspection elements mark their place with their tags and lay out nothing.
		k if k.family() == Family::Intro => (),
		other => engine.warn(DiagnosticKind::Unsupported, c.span(), fmt!("{} was ignored during paged export", other.name())),
	}
	Ok(())
}

/// A fragment that is inline only: its lines with leading, the tags at either end kept outside them.
fn collect_inline(engine: &mut Engine, pairs: &[Pair], cfg: CollectCfg, out: &mut Vec<Child>) -> Outcome<()> {
	let start	= pairs.iter().take_while(|p| p.is_tag()).count();
	let end		= pairs.len() - pairs[start..].iter().rev().take_while(|p| p.is_tag()).count();
	let inner	= &pairs[start..end];
	let styles	= StyleChain::trunk(inner.iter().map(|p| &p.styles));
	for p in &pairs[..start] {
		if let Some(t) = &p.tag {
			out.push(Child::Tag(t.clone()));
		}
	}
	let region = Regions::one(cfg.width, cfg.height, cfg.expand, false).region();
	let nodes = res!(crate::flow::par::layout_inline(engine, inner, &styles, region, cfg.expand));
	res!(collect_lines(nodes, &styles, out));
	for p in &pairs[end..] {
		if let Some(t) = &p.tag {
			out.push(Child::Tag(t.clone()));
		}
	}
	Ok(())
}

/// A paragraph: its body set into lines, with the paragraph spacing on either side. `situation` is where the
/// flow stands, which decides the first-line indent.
fn collect_par(
	engine:		&mut Engine,
	par:		&Content,
	styles:		&StyleChain,
	cfg:		CollectCfg,
	situation:	ParSituation,
	out:		&mut Vec<Child>,
)
	-> Outcome<()>
{
	let region	= Regions::one(cfg.width, cfg.height, cfg.expand, false).region();
	let nodes	= res!(engine.descend(|engine| crate::flow::par::layout_par(engine, par, styles, region, situation)));
	let spacing	= res!(par_spacing(par, styles));
	out.push(Child::Rel(spacing, 4));
	res!(collect_lines(nodes, styles, out));
	out.push(Child::Rel(spacing, 4));
	Ok(())
}

/// The paragraph spacing in force, Typst's 1.2em by default.
fn par_spacing(par: &Content, styles: &StyleChain) -> Outcome<Rel> {
	let v = if par.is(ElemKind::Par) {
		res!(field(par, styles, "spacing"))
	} else {
		res!(chain_field(styles, ElemKind::Par, "spacing"))
	};
	Ok(match v.and_then(|v| rel_of(styles, &v)) {
		Some(r)	=> r,
		None	=> Rel::pt(1.2 * styles.font_size()),
	})
}

/// The widow and orphan costs in force, as ratios (Typst's `text.costs`, 100% each by default).
fn costs(styles: &StyleChain) -> Outcome<(f64, f64)> {
	let (mut widow, mut orphan) = (1.0, 1.0);
	if let Some(Value::Dict(d)) = res!(chain_field(styles, ElemKind::Text, "costs")) {
		if let Some(Value::Ratio(r)) = d.get("widow") {
			widow = r.0;
		}
		if let Some(Value::Ratio(r)) = d.get("orphan") {
			orphan = r.0;
		}
	}
	Ok((widow, orphan))
}

/// A paragraph's set lines as children: each line with the leading before it, and the space it needs to
/// keep an orphan or widow with its partner, as Typst's `lines`. The glue the line setter puts between two
/// lines is their leading.
fn collect_lines(nodes: Vec<Node>, styles: &StyleChain, out: &mut Vec<Child>) -> Outcome<()> {
	let align = res!(alignment(styles));
	let (widow, orphan) = res!(costs(styles));
	// The lines, each with the leading above it; tags between lines keep their order.
	enum Entry {
		Line(Frame, f64),
		Tag(Tag),
	}
	let mut entries: Vec<Entry> = Vec::new();
	let mut lead = 0.0;
	for n in nodes {
		match n {
			Node::Glue(g)		=> lead += g.natural.to_pt(),
			Node::Tag(t)		=> entries.push(Entry::Tag(t)),
			Node::Penalty(_)	=> (),
			Node::Anchor(_) | Node::Mark(_) | Node::Float(_) | Node::Columns(_) | Node::PageColumns(_)
				| Node::RepeatHead(_) | Node::RepeatFoot(_) => (),
			other => {
				entries.push(Entry::Line(Frame::from_node(other), lead));
				lead = 0.0;
			},
		}
	}
	let lines: Vec<(f64, f64)> = entries.iter().filter_map(|e| match e {
		Entry::Line(f, l)	=> Some((f.h, *l)),
		Entry::Tag(_)		=> None,
	}).collect();
	let len				= lines.len();
	let prevent_orphans	= orphan > 0.0 && len >= 2;
	let prevent_widows	= widow > 0.0 && len >= 2;
	let prevent_all		= len == 3 && prevent_orphans && prevent_widows;
	let height_at		= |i: usize| lines.get(i).map(|l| l.0).unwrap_or(0.0);
	let lead_at			= |i: usize| lines.get(i).map(|l| l.1).unwrap_or(0.0);
	let mut i = 0usize;
	for e in entries {
		match e {
			Entry::Tag(t) => out.push(Child::Tag(t)),
			Entry::Line(frame, l) => {
				if i > 0 {
					out.push(Child::Rel(Rel::pt(l), 5));
				}
				let need = if prevent_all && i == 0 {
					height_at(0) + lead_at(1) + height_at(1) + lead_at(2) + height_at(2)
				} else if prevent_orphans && i == 0 {
					height_at(0) + lead_at(1) + height_at(1)
				} else if prevent_widows && i >= 2 && i + 2 == len {
					height_at(len - 2) + lead_at(len - 1) + height_at(len - 1)
				} else {
					frame.h
				};
				out.push(Child::Line(LineChild { frame, align, need }));
				i += 1;
			},
		}
	}
	Ok(())
}

/// A block, with its spacing above and below: Typst's `block` collection. `breakable` is forced for an
/// element Typst shows through a layout routine: `Some(false)` for one that lays out in one region.
fn collect_block(spec: BlockSpec, alone: bool, breakable: Option<bool>, out: &mut Vec<Child>) -> Outcome<()> {
	let styles	= spec.styles.clone();
	let align	= res!(alignment(&styles));
	let sticky	= bool_of(res!(spec.field("sticky")), false);
	let breakable = match breakable {
		Some(false)	=> false,
		_			=> bool_of(res!(spec.field("breakable")), true),
	};
	// A fractional height: a block's own, or that of a shape or image, which Typst's show passes on.
	let fr = if spec.explicit {
		match res!(spec.sizing("height")) {
			Sizing::Fr(f)	=> Some(f),
			_				=> None,
		}
	} else {
		match res!(field(&spec.elem, &styles, "height")) {
			Some(Value::Fraction(f))	=> Some(f.0),
			_							=> None,
		}
	};
	let above = res!(block_spacing(&spec, "above"));
	let below = res!(block_spacing(&spec, "below"));
	out.push(above);
	if !breakable || fr.is_some() {
		out.push(Child::Single(Rc::new(SingleChild { align, sticky, alone, fr, spec, cell: RefCell::new(None) })));
	} else {
		out.push(Child::Multi(Rc::new(MultiChild { align, sticky, alone, spec })));
	}
	out.push(below);
	Ok(())
}

/// A block's spacing above or below: its own, or `spacing` where that side was not given, else the
/// paragraph spacing. Explicit spacing is weakness 3, fractional 2, the paragraph fallback 4.
fn block_spacing(spec: &BlockSpec, side: &str) -> Outcome<Child> {
	let v = match res!(spec.field(side)) {
		Some(v)	=> Some(v),
		None	=> res!(spec.field("spacing")),
	};
	Ok(match v {
		None | Some(Value::Auto) | Some(Value::None) => Child::Rel(res!(par_spacing(&spec.elem, &spec.styles)), 4),
		Some(v) => match spacing_of(&spec.styles, &v) {
			Some(Spacing::Rel(r))	=> Child::Rel(r, 3),
			Some(Spacing::Fr(f))	=> Child::Fr(f, 2),
			None					=> Child::Rel(res!(par_spacing(&spec.elem, &spec.styles)), 4),
		},
	})
}

/// A placed element: Typst's `place` collection, with its errors.
fn collect_place(engine: &mut Engine, elem: &Content, styles: &StyleChain, out: &mut Vec<Child>) -> Outcome<()> {
	let alignment = match res!(field(elem, styles, "alignment")) {
		None						=> Some(Alignment { x: Some(HAlign::Start), y: None }),
		Some(Value::Auto)			=> None,
		Some(Value::Alignment(a))	=> Some(a),
		Some(other) => return Err(engine.error(DiagnosticKind::Type, elem.span(), fmt!(
			"expected alignment or auto, found {}", other.ty().long_name()))),
	};
	let r = res!(rtl(styles));
	let align_x = match alignment {
		None	=> Fixed::Center,
		Some(a)	=> fix_h(a.x.unwrap_or(HAlign::Start), r),
	};
	let align_y = match alignment {
		None	=> PlaceY::Auto,
		Some(a)	=> match a.y {
			None	=> PlaceY::Flow,
			Some(v)	=> PlaceY::At(fix_v(v)),
		},
	};
	let parent = matches!(res!(field(elem, styles, "scope")), Some(Value::Str(s)) if s.as_str() == "parent");
	let float = bool_of(res!(field(elem, styles, "float")), false);
	match (float, align_y) {
		(true, PlaceY::Flow) | (true, PlaceY::At(Fixed::Center)) => return Err(engine.error(DiagnosticKind::Type, elem.span(),
			"vertical floating placement must be `auto`, `top`, or `bottom`")),
		(false, PlaceY::Auto) => return Err(engine.error_hint(DiagnosticKind::Type, elem.span(),
			"automatic positioning is only available for floating placement",
			"you can enable floating placement with `place(float: true, ..)`")),
		_ => (),
	}
	if !float && parent {
		return Err(engine.error_hint(DiagnosticKind::Type, elem.span(),
			"parent-scoped positioning is currently only available for floating placement",
			"you can enable floating placement with `place(float: true, ..)`"));
	}
	let clearance = match res!(field(elem, styles, "clearance")) {
		Some(Value::Length(l))	=> abs_of(styles, l),
		_						=> 1.5 * styles.font_size(),
	};
	let dx = res!(field(elem, styles, "dx")).and_then(|v| rel_of(styles, &v)).unwrap_or_default();
	let dy = res!(field(elem, styles, "dy")).and_then(|v| rel_of(styles, &v)).unwrap_or_default();
	// A float is read where its `place` stands: its location when it has one, else one handed out now.
	let mark = match elem.location() {
		Some(l)	=> l.0,
		None	=> engine.locator.locate(ElemKind::Place, elem.span()).0,
	};
	if float {
		out.push(Child::Mark(mark));
	}
	out.push(Child::Placed(Rc::new(PlacedChild {
		align_x,
		align_y,
		parent,
		float,
		clearance,
		dx,
		dy,
		alignment,
		mark,
		elem:	elem.clone(),
		styles:	styles.clone(),
		cell:	RefCell::new(None),
	})));
	Ok(())
}

// Composition

/// A control event in flow layout: finish the region (out of room, or `true` for a column break), relay a
/// scope out after an insertion, or a fatal error.
enum Stop {
	Finish(bool),
	Relayout(bool),	// `true` for the parent scope, `false` for the column
	Error(Error<ErrTag>),
}

type FlowResult<T> = std::result::Result<T, Stop>;

// Propagates an `Outcome` error as a fatal `Stop`.
macro_rules! fl {
	($e:expr) => {
		match $e {
			Ok(v)	=> v,
			Err(e)	=> return Err(Stop::Error(e)),
		}
	};
}

// Propagates any `Stop`.
macro_rules! st {
	($e:expr) => {
		match $e {
			Ok(v)	=> v,
			Err(s)	=> return Err(s),
		}
	};
}

/// What an insertion is known by once handled: a float by its child, a footnote by its location.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Skip {
	Child(usize),
	Loc(Location),
}

/// The work flow layout has left: Typst's `Work`.
#[derive(Clone, Debug)]
struct Work {
	idx:			usize,							// the first child not yet processed
	spill:			Option<Rc<Spill>>,
	floats:			Vec<(usize, Rc<PlacedChild>)>,	// floats queued for a later region, by child
	footnotes:		Vec<Content>,					// footnotes queued for a later region
	footnote_spill:	Option<Vec<Frame>>,				// the rest of a footnote that did not fit
	tags:			Vec<Pending>,					// tags and marks waiting for the next frame
	skips:			Rc<HashMap<Skip, usize>>,		// insertions already placed, and the child they belong to
}

impl Work {
	fn new() -> Self {
		Self {
			idx:			0,
			spill:			None,
			floats:			Vec::new(),
			footnotes:		Vec::new(),
			footnote_spill:	None,
			tags:			Vec::new(),
			skips:			Rc::new(HashMap::new()),
		}
	}

	fn extend_skips(&mut self, skips: &[(Skip, usize)]) {
		if !skips.is_empty() {
			let set = Rc::make_mut(&mut self.skips);
			for (s, at) in skips {
				set.insert(*s, *at);
			}
		}
	}

	/// Forgets the insertions of children already passed, which are never offered again.
	fn prune(&mut self) {
		if self.skips.values().any(|at| *at < self.idx) {
			let idx = self.idx;
			Rc::make_mut(&mut self.skips).retain(|_, at| *at >= idx);
		}
	}
}

#[derive(Clone, Debug)]
struct ColumnConfig {
	count:	usize,
	width:	f64,
	gutter:	f64,
	rtl:	bool,
}

#[derive(Clone, Debug)]
struct FootnoteConfig {
	separator:	Content,
	clearance:	f64,
	gap:		f64,
	expand:		bool,
}

#[derive(Clone, Debug)]
struct Config {
	mode:		FlowMode,
	shared:		StyleChain,
	columns:	ColumnConfig,
	footnote:	FootnoteConfig,
}

fn configuration(shared: &StyleChain, regions: &Regions, columns: usize, gutter: Rel, mode: FlowMode) -> Outcome<Config> {
	let count	= if regions.w.is_finite() { columns.max(1) } else { 1 };
	let gutter	= gutter.relative_to(regions.base().0);
	let width	= (regions.w - gutter * (count as f64 - 1.0)) / count as f64;
	let fs		= shared.font_size();
	let length	= |v: Option<Value>, default: f64| match v {
		Some(Value::Length(l))	=> abs_of(shared, l),
		_						=> default * fs,
	};
	let clearance	= length(res!(chain_field(shared, ElemKind::FootnoteEntry, "clearance")), 1.0);
	let gap			= length(res!(chain_field(shared, ElemKind::FootnoteEntry, "gap")), 0.5);
	let separator = match res!(chain_field(shared, ElemKind::FootnoteEntry, "separator")) {
		Some(Value::Content(c))	=> c,
		Some(Value::None)		=> Content::empty(),
		_						=> default_separator(),
	};
	Ok(Config {
		mode,
		shared:	shared.clone(),
		columns: ColumnConfig { count, width, gutter, rtl: res!(rtl(shared)) },
		footnote: FootnoteConfig {
			separator,
			clearance,
			gap,
			expand:		regions.expand_x,
		},
	})
}

/// Typst's footnote separator: `line(length: 30% + 0pt, stroke: 0.5pt)`.
fn default_separator() -> Content {
	let mut fields = Vec::new();
	if let Some(id) = ElemKind::Line.field_id("length") {
		fields.push((id, Value::Relative(Relative { rel: Ratio(0.3), abs: Length::zero() })));
	}
	if let Some(id) = ElemKind::Line.field_id("stroke") {
		fields.push((id, Value::Length(Length::pt(0.5))));
	}
	Content::new(ElemKind::Line, fields, Span::detached())
}

/// Lays out one region of a flow, with its column and page insertions: Typst's `compose`.
fn compose(
	engine:		&mut Engine,
	pull:		&mut Pull,
	work:		&mut Work,
	config:		&Config,
	regions:	&Regions,
)
	-> Outcome<Frame>
{
	let mut c = Composer {
		engine,
		pull,
		work,
		config,
		column:			0,
		page_base:		regions.base(),
		page_ins:		Insertions::default(),
		column_ins:		Insertions::default(),
		footnote_spill:	None,
		footnote_queue:	Vec::new(),
	};
	c.page(regions)
}

struct Composer<'a> {
	engine:			&'a mut Engine,
	pull:			&'a mut Pull,
	work:			&'a mut Work,
	config:			&'a Config,
	column:			usize,
	page_base:		(f64, f64),
	page_ins:		Insertions,
	column_ins:		Insertions,
	footnote_spill:	Option<Vec<Frame>>,
	footnote_queue:	Vec<Content>,
}

impl<'a> Composer<'a> {
	/// A page or container region with its page-scoped insertions, relaid out whenever a parent-scoped float
	/// is placed.
	fn page(&mut self, regions: &Regions) -> Outcome<Frame> {
		let checkpoint = self.work.clone();
		let output = loop {
			let mut pod = regions.clone();
			pod.h -= self.page_ins.height();
			match self.page_contents(&pod) {
				Ok(frame)					=> break frame,
				Err(Stop::Relayout(true))	=> {
					*self.work = checkpoint.clone();
					continue;
				},
				Err(Stop::Error(e))			=> return Err(e),
				Err(_)						=> return Err(err!(
					"A column finish or column relayout escaped a column; flow composition is inconsistent."; Bug)),
			}
		};
		let ins = std::mem::take(&mut self.page_ins);
		Ok(ins.finalize(self.work, self.config, output))
	}

	/// The inner contents of a page region: one column, or several stitched side by side.
	fn page_contents(&mut self, regions: &Regions) -> FlowResult<Frame> {
		let count = self.config.columns.count;
		if count == 1 {
			return self.column(regions);
		}
		let column_h	= regions.h;
		let mut backlog	= Vec::new();
		for h in std::iter::once(column_h).chain(regions.backlog.iter().copied()) {
			for _ in 0..count {
				backlog.push(h);
			}
		}
		backlog.remove(0);
		let mut inner = Regions {
			w:			self.config.columns.width,
			h:			column_h,
			full:		regions.full,
			backlog,
			last:		regions.last,
			expand_x:	true,
			expand_y:	regions.expand_y,
		};
		let mut output = Frame::new(regions.w, if regions.expand_y { regions.h } else { 0.0 });
		let mut offset = 0.0;
		for i in 0..count {
			self.column = i;
			let frame = st!(self.column(&inner));
			if !regions.expand_y && frame.h > output.h {
				output.h = frame.h;
			}
			let width = frame.w;
			let x = if self.config.columns.rtl { regions.w - offset - width } else { offset };
			offset += width + self.config.columns.gutter;
			if i == 0 && frame.has_baseline() {
				output.baseline = Some(frame.baseline());
			}
			output.push_frame(x, 0.0, frame);
			inner.next();
		}
		Ok(output)
	}

	/// A column with its column insertions, relaid out whenever a column float or a footnote is placed.
	fn column(&mut self, regions: &Regions) -> FlowResult<Frame> {
		self.column_ins = Insertions::default();
		if let Some(spill) = self.work.footnote_spill.take() {
			fl!(self.footnote_spill_in(spill, regions.base()));
		}
		let checkpoint = self.work.clone();
		let inner = loop {
			let mut pod = regions.clone();
			pod.h -= self.column_ins.height();
			match self.column_contents(&pod) {
				Ok(frame)					=> break frame,
				Err(Stop::Relayout(false))	=> {
					*self.work = checkpoint.clone();
					continue;
				},
				Err(Stop::Finish(_))		=> return Err(Stop::Error(err!(
					"A region finish escaped distribution; flow composition is inconsistent."; Bug))),
				Err(other)					=> return Err(other),
			}
		};
		let queued = std::mem::take(&mut self.footnote_queue);
		self.work.footnotes.extend(queued);
		if let Some(spill) = self.footnote_spill.take() {
			self.work.footnote_spill = Some(spill);
		}
		let ins = std::mem::take(&mut self.column_ins);
		Ok(ins.finalize(self.work, self.config, inner))
	}

	/// A column's in-flow contents, after the footnotes and floats queued from earlier regions.
	fn column_contents(&mut self, regions: &Regions) -> FlowResult<Frame> {
		for note in std::mem::take(&mut self.work.footnotes) {
			let mut r = regions.clone();
			st!(self.footnote(note, &mut r, 0.0, false));
		}
		for (idx, placed) in std::mem::take(&mut self.work.floats) {
			st!(self.float(idx, placed, regions, false, false));
		}
		distribute(self, regions)
	}

	/// Places a float in its scope's top or foot area and asks for the scope to be relaid out, or queues it
	/// for a later region when it does not fit: Typst's `Composer::float`. `clearance` is whether in-flow
	/// content already stands in the region; `migratable` whether a footnote in the float may move it.
	fn float(&mut self, idx: usize, placed: Rc<PlacedChild>, regions: &Regions, clearance: bool, migratable: bool)
		-> FlowResult<()>
	{
		let key = Skip::Child(idx);
		if self.skipped(key) {
			return Ok(());
		}
		if !self.work.floats.is_empty() {
			self.work.floats.push((idx, placed));
			return Ok(());
		}
		let base	= if placed.parent { self.page_base } else { regions.base() };
		let frame	= fl!(placed.layout(self.engine, base));
		let remaining = if placed.parent {
			let n = self.config.columns.count - self.column;
			let mut sum = 0.0;
			for i in 0..n {
				sum += regions.nth(i).unwrap_or(0.0);
			}
			sum / self.config.columns.count as f64
		} else {
			regions.h
		};
		let clear	= if clearance { placed.clearance } else { 0.0 };
		let need	= frame.h + clear;
		if !fits(remaining, need) && regions.may_progress() {
			self.work.floats.push((idx, placed));
			return Ok(());
		}
		st!(self.footnotes(regions, &frame, need, false, migratable));
		let align_y = match placed.align_y {
			PlaceY::At(a)	=> a,
			_ => {
				// The midpoint rule: to the top when the float's centre would sit in the upper half were it set
				// in the flow, else to the foot.
				let used	= base.1 - remaining;
				let ratio	= (used + need / 2.0) / base.1;
				if ratio <= 0.5 { Fixed::Start } else { Fixed::End }
			},
		};
		let clearance = placed.clearance;
		let parent = placed.parent;
		let area = if parent { &mut self.page_ins } else { &mut self.column_ins };
		area.push_float(placed, clearance, frame, align_y);
		area.skips.push((key, idx));
		Err(Stop::Relayout(parent))
	}

	/// Lays out the footnotes whose markers a frame (or the tags waiting for it) holds, at the root only:
	/// Typst's `Composer::footnotes`. `flow_need` is the in-flow space the frame takes; for a breakable
	/// frame each marker's own offset is used instead.
	fn footnotes(&mut self, regions: &Regions, frame: &Frame, flow_need: f64, breakable: bool, migratable: bool)
		-> FlowResult<()>
	{
		if self.config.mode != FlowMode::Root {
			return Ok(());
		}
		let mut notes: Vec<(f64, Content)> = Vec::new();
		for t in &self.work.tags {
			if let Pending::Tag(Tag::Start(c)) = t {
				if c.is(ElemKind::Footnote) {
					notes.push((0.0, c.clone()));
				}
			}
		}
		find_footnotes(frame, 0.0, &mut notes);
		if notes.is_empty() {
			return Ok(());
		}
		let mut relayout	= false;
		let mut regions		= regions.clone();
		let mut migratable	= migratable && !breakable;
		for (y, elem) in notes {
			let need = if breakable { y } else { flow_need };
			match self.footnote(elem, &mut regions, need, migratable) {
				Ok(())					=> (),
				Err(Stop::Relayout(_))	=> relayout = true,
				Err(other)				=> return Err(other),
			}
			migratable = false;
		}
		if relayout {
			return Err(Stop::Relayout(false));
		}
		Ok(())
	}

	/// One footnote into the column's foot area: Typst's `Composer::footnote`.
	fn footnote(&mut self, elem: Content, regions: &mut Regions, flow_need: f64, migratable: bool) -> FlowResult<()> {
		let loc = match elem.location() {
			Some(l)	=> l,
			None	=> return Ok(()),
		};
		if matches!(fl!(field(&elem, &self.config.shared, "body")), Some(Value::Label(_))) || self.skipped(Skip::Loc(loc)) {
			return Ok(());
		}
		if self.footnote_spill.is_some() || !self.footnote_queue.is_empty() {
			self.footnote_queue.push(elem);
			return Ok(());
		}
		let mut separator = None;
		let mut sep_need = 0.0;
		if self.column_ins.footnotes.is_empty() {
			let frame = fl!(self.layout_separator(regions.base()));
			sep_need += self.config.footnote.clearance + frame.h;
			separator = Some(frame);
		}
		let mut pod = regions.clone();
		pod.expand_y = false;
		pod.h -= flow_need + sep_need + self.config.footnote.gap;
		let mut frames	= fl!(self.layout_entry(&elem, pod));
		let mut nested	= Vec::new();
		for f in &frames {
			find_footnotes(f, 0.0, &mut nested);
		}
		let exist_non_empty = frames.iter().any(|f| !f.is_empty());
		if frames.is_empty() {
			frames.push(Frame::new(0.0, 0.0));
		}
		let first		= frames.remove(0);
		let note_need	= self.config.footnote.gap + first.h;
		if first.is_empty() && exist_non_empty {
			if migratable && regions.may_progress() {
				return Err(Stop::Finish(false));
			} else if regions.may_progress() || flow_need != 0.0 {
				self.footnote_queue.push(elem);
				return Ok(());
			}
		}
		if let Some(frame) = separator {
			self.column_ins.push_separator(self.config, frame);
			regions.h -= sep_need;
		}
		self.column_ins.push_footnote(self.config, first);
		self.column_ins.skips.push((Skip::Loc(loc), self.work.idx));
		regions.h -= note_need;
		if !frames.is_empty() {
			self.footnote_spill = Some(frames);
		}
		for (_, note) in nested {
			match self.footnote(note, regions, flow_need, migratable) {
				Ok(()) | Err(Stop::Relayout(_))	=> (),
				Err(other)						=> return Err(other),
			}
		}
		Err(Stop::Relayout(false))
	}

	/// The rest of a footnote that broke across regions, at the top of this column's foot area.
	fn footnote_spill_in(&mut self, mut frames: Vec<Frame>, base: (f64, f64)) -> Outcome<()> {
		let separator = res!(self.layout_separator(base));
		self.column_ins.push_separator(self.config, separator);
		if !frames.is_empty() {
			let first = frames.remove(0);
			self.column_ins.push_footnote(self.config, first);
		}
		if !frames.is_empty() {
			self.footnote_spill = Some(frames);
		}
		Ok(())
	}

	fn layout_separator(&mut self, base: (f64, f64)) -> Outcome<Frame> {
		let regions = Regions::one(base.0, base.1, self.config.footnote.expand, false);
		layout_frame(self.engine, &self.config.footnote.separator, &self.config.shared, regions)
	}

	/// A footnote's entry, Typst's `footnote.entry` of the note, laid out under the flow's shared styles.
	fn layout_entry(&mut self, elem: &Content, pod: Regions) -> Outcome<Vec<Frame>> {
		let id = match ElemKind::FootnoteEntry.field_id("note") {
			Some(id)	=> id,
			None		=> return Err(err!(
				"footnote.entry has no `note` field in its schema, so a footnote's entry cannot be built."; Unimplemented)),
		};
		let entry = Content::new(ElemKind::FootnoteEntry, vec![(id, Value::Content(elem.clone()))], elem.span());
		// The entry is laid out at a place made from the note's location, as Typst's `Locator::synthesize`, so it
		// is the same entry each time the note is composed.
		let place = elem.location().map(Place::synthesise);
		let mut frames = res!(layout_fragment_in(self.engine, place, &entry, &self.config.shared, pod));
		// The entry is read after its footnote for introspection.
		if let Some(loc) = elem.location() {
			for f in &mut frames {
				f.parent = Some(loc.0);
			}
		}
		Ok(frames)
	}

	/// A breakable block's fragment from the continuation `from`, or the one already laid out in this region
	/// from the same state.
	fn fragment(&mut self, child: &Rc<MultiChild>, from: Option<&Rc<Spill>>, regions: &Regions)
		-> Outcome<Fragment>
	{
		let key		= regions.key();
		let c		= Rc::as_ptr(child) as usize;
		let f		= from.map(|s| Rc::as_ptr(s) as usize).unwrap_or(0);
		if let Some(m) = self.pull.memo.iter().find(|m| m.child == c && m.from == f && m.key == key) {
			return Ok(m.out.clone());
		}
		let out = res!(child.fragment(self.engine, from, regions));
		self.pull.memo.push(Memo { child: c, from: f, key, out: out.clone() });
		Ok(out)
	}

	fn skipped(&self, key: Skip) -> bool {
		self.work.skips.contains_key(&key)
			|| self.page_ins.skips.iter().any(|(k, _)| *k == key)
			|| self.column_ins.skips.iter().any(|(k, _)| *k == key)
	}

	fn insertion_width(&self) -> f64 { self.column_ins.width.max(self.page_ins.width) }
}

/// The floats and footnotes of a region's top and foot areas.
#[derive(Default)]
struct Insertions {
	top:			Vec<(Rc<PlacedChild>, Frame)>,
	bottom:			Vec<(Rc<PlacedChild>, Frame)>,
	footnotes:		Vec<Frame>,
	separator:		Option<Frame>,
	top_size:		f64,
	bottom_size:	f64,
	width:			f64,
	skips:			Vec<(Skip, usize)>,
}

impl Insertions {
	fn push_float(&mut self, placed: Rc<PlacedChild>, clearance: f64, frame: Frame, align_y: Fixed) {
		self.width = self.width.max(frame.w);
		let amount = frame.h + clearance;
		if align_y == Fixed::Start {
			self.top_size += amount;
			self.top.push((placed, frame));
		} else {
			self.bottom_size += amount;
			self.bottom.push((placed, frame));
		}
	}

	fn push_footnote(&mut self, config: &Config, frame: Frame) {
		self.width = self.width.max(frame.w);
		self.bottom_size += config.footnote.gap + frame.h;
		self.footnotes.push(frame);
	}

	fn push_separator(&mut self, config: &Config, frame: Frame) {
		self.width = self.width.max(frame.w);
		self.bottom_size += config.footnote.clearance + frame.h;
		self.separator = Some(frame);
	}

	fn height(&self) -> f64 { self.top_size + self.bottom_size }

	/// The region's frame: top floats, the in-flow contents, then foot floats, the separator and the
	/// footnotes, as Typst orders them.
	fn finalize(self, work: &mut Work, config: &Config, inner: Frame) -> Frame {
		work.extend_skips(&self.skips);
		if self.top.is_empty() && self.bottom.is_empty() && self.separator.is_none() && self.footnotes.is_empty() {
			return inner;
		}
		let (w, h) = (inner.w, inner.h + self.height());
		let mut output = Frame::new(w, h);
		let mut top = 0.0;
		let mut bottom = h - self.bottom_size;
		for (p, frame) in self.top {
			let x = p.align_x.position(w - frame.w) + p.dx.relative_to(w);
			let y = top + p.dy.relative_to(h);
			top += frame.h + p.clearance;
			output.push_frame(x, y, frame);
		}
		if inner.has_baseline() {
			output.baseline = Some(self.top_size + inner.baseline());
		}
		output.push_frame(0.0, self.top_size, inner);
		for (p, frame) in self.bottom {
			bottom += p.clearance;
			let x = p.align_x.position(w - frame.w) + p.dx.relative_to(w);
			let y = bottom + p.dy.relative_to(h);
			bottom += frame.h;
			output.push_frame(x, y, frame);
		}
		if let Some(frame) = self.separator {
			bottom += config.footnote.clearance;
			let fh = frame.h;
			output.push_frame(0.0, bottom, frame);
			bottom += fh;
		}
		for frame in self.footnotes {
			bottom += config.footnote.gap;
			let fh = frame.h;
			output.push_frame(0.0, bottom, frame);
			bottom += fh;
		}
		output
	}
}

// Distribution

/// An item laid out in a region, not yet aligned.
#[derive(Clone, Debug)]
enum DItem {
	Tag(Pending),
	Abs(f64, u8),				// spacing and its weakness
	Fr(f64, u8, Option<Rc<SingleChild>>),	// fractional spacing, or a fractional block
	Frame(Frame, Align2),
	Placed(Frame, Rc<PlacedChild>),			// an absolutely placed child
}

struct Snapshot {
	work:	Work,
	items:	usize,
}

/// Distributes as many children as fit into the first region and returns its frame: Typst's `distribute`.
fn distribute(c: &mut Composer, regions: &Regions) -> FlowResult<Frame> {
	let mut d = Distributor { c, regions: regions.clone(), items: Vec::new(), sticky: None, stickable: None };
	let init = d.snapshot();
	let forced = match d.run() {
		Ok(())					=> fl!(d.all_placed()),
		Err(Stop::Finish(f))	=> f,
		Err(other)				=> return Err(other),
	};
	d.finalize(regions, init, forced)
}

struct Distributor<'c, 'a> {
	c:			&'c mut Composer<'a>,
	regions:	Regions,
	items:		Vec<DItem>,
	sticky:		Option<Snapshot>,
	stickable:	Option<bool>,
}

impl<'c, 'a> Distributor<'c, 'a> {
	fn run(&mut self) -> FlowResult<()> {
		if let Some(spill) = self.c.work.spill.take() {
			st!(self.multi_spill(spill));
		}
		loop {
			let idx = self.c.work.idx;
			let child = match fl!(self.c.pull.get(self.c.engine, idx)) {
				Some(c)	=> c,
				None	=> break,
			};
			st!(self.child(idx, &child));
			self.c.work.idx += 1;
		}
		Ok(())
	}

	/// Is everything placed and nothing left in the work, now the children have run out?
	fn all_placed(&mut self) -> Outcome<bool> {
		let w = &self.c.work;
		let pending = w.spill.is_some() || !w.floats.is_empty() || w.footnote_spill.is_some() || !w.footnotes.is_empty();
		Ok(!pending && res!(self.c.pull.get(self.c.engine, self.c.work.idx)).is_none())
	}

	fn child(&mut self, idx: usize, child: &Child) -> FlowResult<()> {
		match child {
			Child::Tag(t)		=> self.c.work.tags.push(Pending::Tag(t.clone())),
			Child::Mark(m)		=> self.c.work.tags.push(Pending::Mark(*m)),
			Child::Rel(r, w)	=> self.rel(*r, *w),
			Child::Fr(f, w)		=> self.fr(*f, *w),
			Child::Line(l)		=> st!(self.line(l)),
			Child::Single(s)	=> st!(self.single(s)),
			Child::Multi(m)		=> st!(self.multi(m)),
			Child::Placed(p)	=> st!(self.placed(idx, p)),
			Child::Flush		=> st!(self.flush()),
			Child::Break(weak)	=> st!(self.break_(*weak)),
		}
		Ok(())
	}

	fn flush_tags(&mut self) {
		if !self.c.work.tags.is_empty() {
			for t in self.c.work.tags.drain(..) {
				self.items.push(DItem::Tag(t));
			}
		}
	}

	fn rel(&mut self, r: Rel, weakness: u8) {
		let amount = r.relative_to(self.regions.base().1);
		if weakness > 0 && !self.keep_weak_rel(amount, weakness) {
			return;
		}
		self.regions.h -= amount;
		self.items.push(DItem::Abs(amount, weakness));
	}

	fn fr(&mut self, f: f64, weakness: u8) {
		if weakness > 0 && !self.keep_weak_fr(f, weakness) {
			return;
		}
		self.trim_spacing();
		self.items.push(DItem::Fr(f, weakness, None));
	}

	/// Keeps weak spacing only where something before it supports it, merging it into a preceding weak
	/// spacing: the stronger, or the larger of equal strength, wins.
	fn keep_weak_rel(&mut self, amount: f64, weakness: u8) -> bool {
		let mut i = self.items.len();
		while i > 0 {
			i -= 1;
			match self.items[i] {
				DItem::Abs(prev, pw) if pw >= 1 => {
					if weakness <= pw && (weakness < pw || amount > prev) {
						self.regions.h -= amount - prev;
						self.items[i] = DItem::Abs(amount, weakness);
					}
					return false;
				},
				DItem::Tag(_) | DItem::Abs(_, _) | DItem::Placed(..)	=> (),
				DItem::Fr(_, _, None)									=> return false,
				DItem::Frame(..) | DItem::Fr(_, _, Some(_))				=> return true,
			}
		}
		false
	}

	fn keep_weak_fr(&mut self, f: f64, weakness: u8) -> bool {
		let mut i = self.items.len();
		while i > 0 {
			i -= 1;
			match self.items[i] {
				DItem::Fr(prev, pw, None) if pw >= 1 => {
					if weakness <= pw && (weakness < pw || f > prev) {
						self.items[i] = DItem::Fr(f, weakness, None);
					}
					return false;
				},
				DItem::Tag(_) | DItem::Abs(..) | DItem::Placed(..)	=> (),
				DItem::Fr(_, _, None)								=> return true,
				DItem::Frame(..) | DItem::Fr(_, _, Some(_))			=> return true,
			}
		}
		false
	}

	/// Drops the last weak spacing, if nothing but tags, strong spacing and placed items follow it.
	fn trim_spacing(&mut self) {
		let mut i = self.items.len();
		while i > 0 {
			i -= 1;
			match self.items[i] {
				DItem::Abs(amount, w) if w >= 1 => {
					self.regions.h += amount;
					self.items.remove(i);
					break;
				},
				DItem::Fr(_, w, None) if w >= 1 => {
					self.items.remove(i);
					break;
				},
				DItem::Tag(_) | DItem::Abs(..) | DItem::Placed(..)	=> (),
				DItem::Frame(..) | DItem::Fr(..)					=> break,
			}
		}
	}

	/// The trailing weak spacing, which a float may reclaim because it would collapse at a break.
	fn weak_spacing(&self) -> f64 {
		for item in self.items.iter().rev() {
			match item {
				DItem::Abs(amount, w) if *w >= 1					=> return *amount,
				DItem::Tag(_) | DItem::Abs(..) | DItem::Placed(..)	=> (),
				DItem::Frame(..) | DItem::Fr(..)					=> break,
			}
		}
		0.0
	}

	fn line(&mut self, line: &LineChild) -> FlowResult<()> {
		if !fits(self.regions.h, line.frame.h) && self.regions.may_progress() {
			return Err(Stop::Finish(false));
		}
		// A line whose widow or orphan partner does not fit here, but would in the next region, goes there.
		if !fits(self.regions.h, line.need) && self.regions.nth(1).map(|h| fits(h, line.need)).unwrap_or(false) {
			return Err(Stop::Finish(false));
		}
		self.frame(line.frame.clone(), line.align, false, false)
	}

	fn single(&mut self, single: &Rc<SingleChild>) -> FlowResult<()> {
		let (bw, bh) = self.regions.base();
		let frame = fl!(single.layout(self.c.engine, bw, bh, self.regions.expand_x, self.regions.expand_y));
		if let Some(fr) = single.fr {
			let regions = self.regions.clone();
			st!(self.c.footnotes(&regions, &frame, 0.0, false, true));
			self.flush_tags();
			self.items.push(DItem::Fr(fr, 0, Some(single.clone())));
			return Ok(());
		}
		if !fits(self.regions.h, frame.h) && self.regions.may_progress() {
			return Err(Stop::Finish(false));
		}
		self.frame(frame, single.align, single.sticky, false)
	}

	/// A breakable block's first fragment, in this region. The rest of the block waits in the work, as the
	/// continuation that lays out its next fragment.
	fn multi(&mut self, multi: &Rc<MultiChild>) -> FlowResult<()> {
		if self.regions.is_full() {
			return Err(Stop::Finish(false));
		}
		let frag = fl!(self.c.fragment(multi, None, &self.regions));
		// A first fragment left empty while a later one is not leaves this region alone, so that no invisible
		// orphan ends it.
		if frag.skip && self.regions.may_progress() {
			return Err(Stop::Finish(false));
		}
		st!(self.frame(frag.frame, multi.align, multi.sticky, true));
		if let Some(next) = frag.next {
			self.c.work.spill = Some(next);
			self.c.work.idx += 1;
			return Err(Stop::Finish(false));
		}
		Ok(())
	}

	/// The rest of a breakable block, in this region: the fragment its continuation lays out.
	fn multi_spill(&mut self, spill: Rc<Spill>) -> FlowResult<()> {
		if self.regions.is_full() {
			self.c.work.spill = Some(spill);
			return Err(Stop::Finish(false));
		}
		let child = spill.child.clone();
		let frag = fl!(self.c.fragment(&child, Some(&spill), &self.regions));
		st!(self.frame(frag.frame, child.align, false, true));
		if let Some(next) = frag.next {
			self.c.work.spill = Some(next);
			return Err(Stop::Finish(false));
		}
		Ok(())
	}

	/// An in-flow frame, from a line or a block: sticky blocks remember where a run of them began, so the
	/// run can move with the block it sticks to.
	fn frame(&mut self, frame: Frame, align: Align2, sticky: bool, breakable: bool) -> FlowResult<()> {
		if sticky {
			if self.sticky.is_none() {
				let progress = self.regions.may_progress();
				if *self.stickable.get_or_insert(progress) {
					self.sticky = Some(self.snapshot());
				}
			}
		} else if !frame.is_empty() {
			self.sticky = None;
			self.stickable = None;
		}
		let regions = self.regions.clone();
		st!(self.c.footnotes(&regions, &frame, frame.h, breakable, true));
		self.regions.h -= frame.h;
		self.flush_tags();
		self.items.push(DItem::Frame(frame, align));
		Ok(())
	}

	fn placed(&mut self, idx: usize, placed: &Rc<PlacedChild>) -> FlowResult<()> {
		if placed.float {
			let weak = self.weak_spacing();
			self.regions.h += weak;
			let clearance = self.items.iter().any(|i| matches!(i, DItem::Frame(..)));
			let regions = self.regions.clone();
			st!(self.c.float(idx, placed.clone(), &regions, clearance, true));
			self.regions.h -= weak;
		} else {
			let frame = fl!(placed.layout(self.c.engine, self.regions.base()));
			let regions = self.regions.clone();
			st!(self.c.footnotes(&regions, &frame, 0.0, true, true));
			self.flush_tags();
			self.items.push(DItem::Placed(frame, placed.clone()));
		}
		Ok(())
	}

	fn flush(&mut self) -> FlowResult<()> {
		if !self.c.work.floats.is_empty() {
			return Err(Stop::Finish(false));
		}
		Ok(())
	}

	fn break_(&mut self, weak: bool) -> FlowResult<()> {
		if (!weak || !self.items.is_empty()) && (!self.regions.backlog.is_empty() || self.regions.last.is_some()) {
			self.c.work.idx += 1;
			return Err(Stop::Finish(true));
		}
		Ok(())
	}

	fn snapshot(&self) -> Snapshot { Snapshot { work: self.c.work.clone(), items: self.items.len() } }

	fn restore(&mut self, s: Snapshot) {
		*self.c.work = s.work;
		self.items.truncate(s.items);
	}

	fn migratable(&self, item: &DItem) -> bool {
		match item {
			DItem::Tag(_)			=> true,
			DItem::Frame(f, _)		=> f.w == 0.0 && f.h == 0.0 && f.items.iter().all(|(_, _, i)|
				matches!(i, Item::Tag(_) | Item::Node(Node::Mark(_)))),
			DItem::Placed(_, p)		=> !p.float,
			_						=> false,
		}
	}

	/// Arranges the items into the region's frame: alignment, and fractional spacing and blocks sharing the
	/// free space. A region that ended on a sticky run hands that run on to the next.
	fn finalize(mut self, region: &Regions, init: Snapshot, forced: bool) -> FlowResult<Frame> {
		if forced {
			self.flush_tags();
		} else if !self.items.is_empty() && self.items.iter().all(|i| self.migratable(i)) {
			self.restore(init);
		} else if let Some(s) = self.sticky.take() {
			self.restore(s);
		}
		self.trim_spacing();

		let mut frs			= 0.0;
		let mut used		= (0.0f64, 0.0f64);
		let mut has_fr_child = false;
		for item in &self.items {
			match item {
				DItem::Abs(v, _)			=> used.1 += v,
				DItem::Fr(v, _, child)		=> {
					frs += v;
					has_fr_child |= child.is_some();
				},
				DItem::Frame(f, _)			=> {
					used.1 += f.h;
					used.0 = used.0.max(f.w);
				},
				DItem::Tag(_) | DItem::Placed(..)	=> (),
			}
		}
		let mut fr_space = 0.0;
		if frs > 0.0 && region.h.is_finite() {
			fr_space = region.h - used.1;
			used.1 = region.h;
		}
		let mut fr_frames = Vec::new();
		if has_fr_child {
			for item in &self.items {
				if let DItem::Fr(v, _, Some(single)) = item {
					let length = share(*v, frs, fr_space);
					let frame = fl!(single.layout(self.c.engine, region.w, length, region.expand_x, region.expand_y));
					used.0 = used.0.max(frame.w);
					fr_frames.push(frame);
				}
			}
		}
		if !region.expand_x {
			used.0 = used.0.max(self.c.insertion_width());
		}
		let size = (
			if region.expand_x { region.w } else { used.0.min(region.w) },
			if region.expand_y { region.h } else { used.1.min(region.h) },
		);
		let free = size.1 - used.1;

		let mut output		= Frame::new(size.0, size.1);
		let mut ruler		= Fixed::Start;
		let mut offset		= 0.0;
		let mut fr_frames	= fr_frames.into_iter();
		let mut baseline_set = false;
		for item in std::mem::take(&mut self.items) {
			match item {
				DItem::Tag(t) => {
					let y = offset + ruler.position(free);
					output.push(0.0, y, t.item());
				},
				DItem::Abs(v, _) => offset += v,
				DItem::Fr(v, _, single) => {
					let length = share(v, frs, fr_space);
					if let Some(s) = single {
						if let Some(frame) = fr_frames.next() {
							let x = s.align.x.position(size.0 - frame.w);
							output.push_frame(x, offset, frame);
						}
					}
					offset += length;
				},
				DItem::Frame(frame, align) => {
					ruler = ruler.max(align.y);
					let x = align.x.position(size.0 - frame.w);
					let y = offset + ruler.position(free);
					offset += frame.h;
					if !baseline_set {
						if frame.has_baseline() {
							output.baseline = Some(y + frame.baseline());
						}
						baseline_set = true;
					}
					output.push_frame(x, y, frame);
				},
				DItem::Placed(frame, p) => {
					let x = p.align_x.position(size.0 - frame.w);
					let y = match p.align_y {
						PlaceY::At(a)	=> a.position(size.1 - frame.h),
						_				=> offset + ruler.position(free),
					};
					let dx = p.dx.relative_to(size.0);
					let dy = p.dy.relative_to(size.1);
					output.push_frame(x + dx, y + dy, frame);
				},
			}
		}
		Ok(output)
	}
}

/// A fraction's share of the remaining space, Typst's `Fr::share`.
fn share(fr: f64, total: f64, remaining: f64) -> f64 {
	let ratio = fr / total;
	if ratio.is_finite() && remaining.is_finite() { (ratio * remaining).max(0.0) } else { 0.0 }
}

// Blocks

/// An unbreakable block, Typst's `layout_single_block`: sized, inset, filled, stroked and clipped.
fn layout_single(engine: &mut Engine, spec: &BlockSpec, region: &Regions) -> Outcome<Frame> {
	let width = res!(spec.sizing("width"));
	layout_single_sized(engine, spec, region, width)
}

/// An inline `box` as one frame, Typst's `layout_box`: sized, inset, clipped, filled and stroked as an
/// unbreakable block is, then its baseline moved. `share` is the width a fractional `width` takes. The
/// region is the line's: relative sizes resolve against its base.
pub fn layout_box(engine: &mut Engine, elem: &Content, styles: &StyleChain, region: Region, share: Option<f64>)
	-> Outcome<Frame>
{
	let body = match res!(field(elem, styles, "body")) {
		Some(Value::Content(c))	=> Body::Content(c),
		_						=> Body::Empty,
	};
	let spec = BlockSpec { elem: elem.clone(), styles: styles.clone(), body, explicit: true };
	let width = match share {
		Some(w)	=> Sizing::Rel(Rel::pt(w)),
		None	=> res!(spec.sizing("width")),
	};
	let mut frame = res!(layout_single_sized(engine, &spec, &Regions::from_region(region), width));
	// The baseline moves after the size and inset are final, so a relative shift reads the final height.
	let (at, shift) = match res!(field(elem, styles, "baseline")) {
		Some(Value::Dict(d))	=> (d.get("at").cloned(), d.get("shift").cloned()),
		Some(v @ Value::Alignment(_))	=> (Some(v), None),
		Some(Value::Auto) | Some(Value::None) | None	=> (None, None),
		Some(v)					=> (None, Some(v)),
	};
	if let Some(Value::Alignment(a)) = at {
		let pos = match a.y {
			Some(VAlign::Top)		=> 0.0,
			Some(VAlign::Horizon)	=> frame.h / 2.0,
			Some(VAlign::Bottom)	=> frame.h,
			None					=> frame.baseline(),
		};
		frame.baseline = Some(pos);
	}
	if let Some(v) = shift {
		let by = rel_of(styles, &v).map(|r| r.relative_to(frame.h)).unwrap_or(0.0);
		if by != 0.0 {
			frame.baseline = Some(frame.baseline() - by);
		}
	}
	Ok(frame)
}

fn layout_single_sized(engine: &mut Engine, spec: &BlockSpec, region: &Regions, width: Sizing) -> Outcome<Frame> {
	let styles	= &spec.styles;
	let height	= res!(spec.sizing("height"));
	let inset	= sides_rel(styles, res!(spec.field("inset")));
	let pod		= unbreakable_pod(width, height, &inset, region.w, region.h);
	let mut frame = match &spec.body {
		Body::Empty			=> Frame::new(0.0, 0.0),
		Body::Content(c)	=> res!(layout_frame_in(engine, spec.elem.place(), c, styles, pod.clone())),
		Body::Layouter		=> {
			let mut p = pod.clone();
			p.expand_x = (pod.expand_x || region.expand_x) && pod.w.is_finite();
			p.expand_y = (pod.expand_y || region.expand_y) && pod.h.is_finite();
			res!(layout_single_layouter(engine, &spec.elem, styles, &p, &pod))
		},
	};
	if pod.expand_x {
		frame.w = pod.w;
	}
	if pod.expand_y {
		frame.h = pod.h;
	}
	if !sides_zero(&inset) {
		grow(&mut frame, &inset);
	}
	res!(decorate(engine, spec, &mut frame, true));
	if res!(vis::is_hidden(styles)) {
		frame.hide();
	}
	if spec.explicit && spec.elem.label().is_some() {
		frame.label();
	}
	Ok(frame)
}

/// Is the element a grid or a table? Typst sizes one to its columns and rows, never to the region it is in.
fn is_grid(elem: &Content) -> bool {
	matches!(elem.kind(), Some(ElemKind::Grid | ElemKind::Table))
}

/// A fragment's body laid out, before it is sized, inset or painted.
struct Laid {
	frame:	Frame,
	tall:	Option<f64>,		// the height an expanding block's frame is given
	pod:	Regions,
	next:	Option<Rc<Spill>>,
}

impl MultiChild {
	/// A breakable block, Typst's `layout_multi_block`, one fragment at a time: the first (`from` none), or
	/// the one after those the continuation `from` has laid out. The body is flowed into the region, the
	/// frame then sized, inset, clipped, filled and stroked, and the continuation returned unless the block
	/// ended here. A first fragment left empty while a later one is not takes no fill, and is flagged so that
	/// the flow puts the block in the next region whole, as Typst does.
	fn fragment(self: &Rc<Self>, engine: &mut Engine, from: Option<&Rc<Spill>>, regions: &Regions)
		-> Outcome<Fragment>
	{
		let spec	= &self.spec;
		let styles	= &spec.styles;
		let mut own	= regions.clone();
		own.expand_y &= self.alone;
		let inset	= sides_rel(styles, res!(spec.field("inset")));
		let Laid { mut frame, tall, pod, next } = res!(self.lay(engine, from, &own));
		let first	= from.is_none();
		let skip	= first && frame.is_empty() && match &next {
			Some(n)	=> res!(self.later(engine, n, &own)),
			None	=> false,
		};
		if pod.expand_x {
			frame.w = pod.w;
		}
		if pod.expand_y {
			if let Some(h) = tall {
				frame.h = h;
			}
		}
		if !sides_zero(&inset) {
			grow(&mut frame, &inset);
		}
		res!(decorate(engine, spec, &mut frame, !skip));
		if res!(vis::is_hidden(styles)) {
			frame.hide();
		}
		// A label goes on every frame but an empty orphan one, which it would make non-empty.
		if spec.explicit && spec.elem.label().is_some() && !skip {
			frame.label();
		}
		Ok(Fragment { frame, next, skip })
	}

	/// One fragment's body, as the continuation `from` (or the start) lays it out in `regions`, a layer of its
	/// own as Typst's `layout_multi_block` is.
	fn lay(self: &Rc<Self>, engine: &mut Engine, from: Option<&Rc<Spill>>, regions: &Regions) -> Outcome<Laid> {
		engine.descend(|engine| self.lay_in(engine, from, regions))
	}

	fn lay_in(self: &Rc<Self>, engine: &mut Engine, from: Option<&Rc<Spill>>, regions: &Regions) -> Outcome<Laid> {
		let spec	= &self.spec;
		let styles	= &spec.styles;
		let width	= res!(spec.sizing("width"));
		let height	= res!(spec.sizing("height"));
		let inset	= sides_rel(styles, res!(spec.field("inset")));
		let k		= from.map(|s| s.k).unwrap_or(0);
		// A fixed height is spread over the regions it spans: its whole, and what is left of it.
		let fixed = match (from, height) {
			(Some(s), _)			=> s.fixed,
			(None, Sizing::Rel(r))	=> {
				let whole = r.relative_to(regions.full);
				Some((whole, whole))
			},
			(None, _)				=> None,
		};
		let pod = breakable_pod(width, height, &inset, regions, fixed);
		let mut nested = match from {
			Some(s)	=> s.nested.clone(),
			None	=> res!(self.start(engine, &pod, regions)),
		};
		let (frame, done, tall) = match &mut nested {
			Nested::Flow(cursor, padding) => {
				let inner = self.inner(&pod, regions);
				let pod_k = match padding {
					Some(pad)	=> pad_regions(&inner, pad),
					None		=> inner,
				};
				// The body's flow is the fragment layer Typst's `layout_fragment` is.
				let (mut f, done) = res!(engine.descend(|engine| -> Outcome<(Frame, bool)> {
					let f = res!(cursor.region(engine, &pod_k));
					let done = res!(cursor.is_done(engine));
					Ok((f, done))
				}));
				if let Some(pad) = padding {
					grow(&mut f, pad);
				}
				let done = done && (!pod_k.expand_y || pod_k.backlog.is_empty());
				(f, done, Some(pod.h))
			},
			Nested::Frames(q, pod0) => {
				let f = q.pop_front().unwrap_or_default();
				(f, q.is_empty(), pod0.nth(k))
			},
		};
		let next = if done {
			None
		} else {
			let fixed = fixed.map(|(whole, left)| (whole, (left - pod.h).max(0.0)));
			Some(Rc::new(Spill { child: self.clone(), nested, k: k + 1, fixed }))
		};
		Ok(Laid { frame, tall, pod, next })
	}

	/// Does a fragment after the first hold anything? Typst judges this on the whole block laid out at once;
	/// here the later fragments are laid out one at a time, each dropped as soon as it is seen, until one with
	/// something in it turns up or the block ends. The regions are the ones the block's own come from.
	fn later(self: &Rc<Self>, engine: &mut Engine, from: &Rc<Spill>, regions: &Regions) -> Outcome<bool> {
		let mut r	= regions.clone();
		let mut cur	= from.clone();
		while r.may_break() {
			r.next();
			let laid = res!(self.lay(engine, Some(&cur), &r));
			if !laid.frame.is_empty() {
				return Ok(true);
			}
			match laid.next {
				Some(n)	=> cur = n,
				None	=> return Ok(false),
			}
		}
		Ok(false)
	}

	/// The regions a layout routine lays its body out in: the block's own, expanded as its parent's are. A
	/// grid or table takes none of the parent's expansion: its frame is as wide as its columns and as tall
	/// as its rows whatever the region, so that `align` has room to place it, alone on a page or not. A
	/// fractional column or row still fills the region, through the size of the plan.
	fn inner(&self, pod: &Regions, regions: &Regions) -> Regions {
		let mut p = pod.clone();
		if matches!(self.spec.body, Body::Layouter) {
			let from = !is_grid(&self.spec.elem);
			p.expand_x = (pod.expand_x || (from && regions.expand_x)) && pod.w.is_finite();
			p.expand_y = (pod.expand_y || (from && regions.expand_y)) && pod.h.is_finite();
		}
		p
	}

	/// What lays out the block's fragments, made when the first is asked for. A body laid out as one piece
	/// (an auto-width block, a grid, a stack) is laid out whole here, with the regions as they stand, and
	/// handed out a frame at a time: Typst equalises the widths of an auto-width block's fragments, which
	/// needs them all.
	fn start(&self, engine: &mut Engine, pod: &Regions, regions: &Regions) -> Outcome<Nested> {
		let spec	= &self.spec;
		let styles	= &spec.styles;
		match &spec.body {
			Body::Empty => {
				let mut v = VecDeque::new();
				v.push_back(Frame::new(0.0, 0.0));
				if pod.expand_y {
					for _ in 0..pod.backlog.len() {
						v.push_back(Frame::new(0.0, 0.0));
					}
				}
				Ok(Nested::Frames(v, pod.clone()))
			},
			Body::Content(c) => {
				let place = spec.elem.place();
				if pod.expand_x {
					let cursor = res!(flow_cursor(engine, place, c, styles, pod, 1, Rel::zero(), c.span()));
					return Ok(Nested::Flow(Box::new(cursor), None));
				}
				let mut frags = res!(layout_fragment_in(engine, place, c, styles, pod.clone()));
				// An auto-width body that came out at different widths in different regions is relaid out at the
				// widest, so every fragment of the block is as wide as the others.
				if frags.windows(2).any(|w| (w[0].w - w[1].w).abs() >= EPS) {
					let max = frags.iter().map(|f| f.w).fold(0.0, f64::max);
					let mut p = pod.clone();
					p.w = max;
					p.expand_x = true;
					frags = res!(layout_fragment_in(engine, place, c, styles, p));
				}
				Ok(Nested::Frames(frags.into(), pod.clone()))
			},
			Body::Layouter => {
				let inner	= self.inner(pod, regions);
				let elem	= &spec.elem;
				match elem.kind() {
					Some(ElemKind::Pad) => {
						let (padding, body) = res!(pad_parts(elem, styles));
						let cursor = res!(flow_cursor(
							engine, elem.place(), &body, styles, &pad_regions(&inner, &padding), 1, Rel::zero(), elem.span()));
						Ok(Nested::Flow(Box::new(cursor), Some(padding)))
					},
					Some(ElemKind::Columns) => {
						let (count, gutter, body) = res!(columns_parts(engine, elem, styles));
						let cursor = res!(flow_cursor(engine, elem.place(), &body, styles, &inner, count, gutter, elem.span()));
						Ok(Nested::Flow(Box::new(cursor), None))
					},
					Some(ElemKind::Layout) => {
						let content = res!(layout_content(engine, elem, styles, inner.base()));
						let cursor = res!(flow_cursor(engine, elem.place(), &content, styles, &inner, 1, Rel::zero(), content.span()));
						Ok(Nested::Flow(Box::new(cursor), None))
					},
					_ => {
						let frames = res!(layout_multi_layouter(engine, elem, styles, &inner));
						Ok(Nested::Frames(frames.into(), pod.clone()))
					},
				}
			},
		}
	}
}

/// A flow over the realised body of a container, to be filled region by region. The body is realised at
/// `place`, the container's.
#[allow(clippy::too_many_arguments)]
fn flow_cursor(
	engine:		&mut Engine,
	place:		Option<Place>,
	content:	&Content,
	styles:		&StyleChain,
	regions:	&Regions,
	columns:	usize,
	gutter:		Rel,
	span:		Span,
)
	-> Outcome<FlowCursor>
{
	engine.descend_fragment(span, |engine| {
		let (pairs, inline) = res!(engine.within(place, |engine| realise::realise_fragment(engine, content, styles)));
		let mode	= if inline { FlowMode::Inline } else { FlowMode::Block };
		FlowCursor::new(engine, Feed::list(pairs), styles, regions, columns, gutter, mode, span)
	})
}

/// The regions shrunk by a padding that each side resolves against the region itself: Typst's `pad`.
fn pad_regions(regions: &Regions, padding: &[Rel; 4]) -> Regions {
	let ix = Rel { ratio: padding[0].ratio + padding[2].ratio, abs: padding[0].abs + padding[2].abs };
	let iy = Rel { ratio: padding[1].ratio + padding[3].ratio, abs: padding[1].abs + padding[3].abs };
	regions.map(|w, h| (w - ix.relative_to(w), h - iy.relative_to(h)))
}

/// The region for an unbreakable container's content: its size resolved against the base, less the inset,
/// expanded on an axis the container sizes.
fn unbreakable_pod(width: Sizing, height: Sizing, inset: &[Rel; 4], bw: f64, bh: f64) -> Regions {
	let w = match width {
		Sizing::Auto | Sizing::Fr(_)	=> bw,
		Sizing::Rel(r)					=> r.relative_to(bw),
	};
	let h = match height {
		Sizing::Auto | Sizing::Fr(_)	=> bh,
		Sizing::Rel(r)					=> r.relative_to(bh),
	};
	let (w, h) = if sides_zero(inset) { (w, h) } else { shrink(w, h, inset) };
	let expand_x = width != Sizing::Auto && w.is_finite();
	let expand_y = height != Sizing::Auto && h.is_finite();
	Regions::one(w, h, expand_x, expand_y)
}

/// The regions for a breakable container's content: a fixed height (its whole, and what is left of it)
/// spread over the regions it spans, the inset taken from every region.
fn breakable_pod(width: Sizing, height: Sizing, inset: &[Rel; 4], regions: &Regions, fixed: Option<(f64, f64)>) -> Regions {
	let (bw, _) = regions.base();
	let mut pod = match fixed {
		Some((whole, left)) => {
			let (first, rest) = spread_height(left, regions);
			Regions {
				w:			regions.w,
				h:			first,
				full:		whole,
				backlog:	rest,
				last:		None,
				expand_x:	regions.expand_x,
				expand_y:	regions.expand_y,
			}
		},
		None => regions.clone(),
	};
	pod.w = match width {
		Sizing::Auto | Sizing::Fr(_)	=> regions.w,
		Sizing::Rel(r)					=> r.relative_to(bw),
	};
	if !sides_zero(inset) {
		pod = pad_regions(&pod, inset);
	}
	pod.expand_x = width != Sizing::Auto && pod.w.is_finite();
	pod.expand_y = height != Sizing::Auto && pod.h.is_finite();
	pod
}

/// A fixed height spread over the regions: as much as each takes, the rest overflowing the last. Typst's
/// `distribute`.
fn spread_height(height: f64, regions: &Regions) -> (f64, Vec<f64>) {
	let mut out		= Vec::new();
	let mut left	= height;
	if left <= 0.0 {
		return (left, Vec::new());
	}
	let mut r = regions.clone();
	loop {
		let limited = r.h.clamp(0.0, left);
		out.push(limited);
		left -= limited;
		if approx_empty(left) || !r.may_break() || (!r.may_progress() && approx_empty(limited)) {
			break;
		}
		r.next();
	}
	if !approx_empty(left) {
		if let Some(last) = out.last_mut() {
			*last += left;
		}
	}
	let first = out.remove(0);
	(first, out)
}

/// A block's clip, fill and stroke, drawn behind its contents. The fill and stroke are Typst's rectangle
/// with the block's paint, stroke, corner radii and outset, laid out as a `rect` of the frame's size.
fn decorate(engine: &mut Engine, spec: &BlockSpec, frame: &mut Frame, paint: bool) -> Outcome<()> {
	let styles	= &spec.styles;
	let fill	= res!(spec.field("fill")).filter(|v| !v.is_none());
	let stroke	= res!(spec.field("stroke")).filter(|v| !v.is_none());
	let radius	= res!(spec.field("radius"));
	let outset	= res!(spec.field("outset"));
	if bool_of(res!(spec.field("clip")), false) {
		let out	= sides_rel(styles, outset.clone());
		let outs = [
			out[0].relative_to(frame.w),
			out[1].relative_to(frame.h),
			out[2].relative_to(frame.w),
			out[3].relative_to(frame.h),
		];
		let path = res!(crate::flow::visual::clip_path(frame.w, frame.h, styles.font_size(),
			radius.clone(), stroke.clone(), outs));
		frame.clip(path);
	}
	if !paint || (fill.is_none() && stroke.is_none()) || res!(vis::is_hidden(styles)) {
		return Ok(());
	}
	let mut fields = Vec::new();
	let mut set = |name: &str, v: Value| {
		if let Some(id) = ElemKind::Rect.field_id(name) {
			fields.push((id, v));
		}
	};
	set("width",	Value::Length(Length::pt(frame.w)));
	set("height",	Value::Length(Length::pt(frame.h)));
	set("fill",		fill.unwrap_or(Value::None));
	set("stroke",	stroke.unwrap_or(Value::None));
	set("radius",	radius.unwrap_or(Value::Length(Length::zero())));
	set("outset",	outset.unwrap_or(Value::Length(Length::zero())));
	set("inset",	Value::Length(Length::zero()));
	let rect = Content::new(ElemKind::Rect, fields, spec.span());
	let region = Regions::one(frame.w, frame.h, false, false).region();
	let node = res!(crate::flow::visual::layout_visual(engine, &rect, styles, region));
	frame.prepend(vec![(0.0, 0.0, Item::Node(node))]);
	Ok(())
}

// Layout routines

/// An element Typst shows through a single-region layout routine.
fn layout_single_layouter(
	engine:	&mut Engine,
	elem:	&Content,
	styles:	&StyleChain,
	pod:	&Regions,
	sized:	&Regions,	// the pod before the region's own expansion, which a shape sizes itself against
)
	-> Outcome<Frame>
{
	match elem.kind() {
		Some(k) if k.family() == Family::Visual => {
			// A shape or an image with no body of its own has no baseline, as in Typst: its baseline is its
			// bottom, wherever a later height puts that.
			let laid = res!(crate::flow::visual::layout_frame(engine, elem, styles, sized.region()));
			let node = res!(crate::flow::visual::frame_to_node(engine, &laid, elem.span()));
			let mut frame = Frame::from_node(node);
			if laid.baseline.is_none() {
				frame.baseline = None;
			}
			Ok(frame)
		},
		_ => {
			let mut frames = res!(layout_multi_layouter(engine, elem, styles, pod));
			Ok(if frames.is_empty() { Frame::new(0.0, 0.0) } else { frames.swap_remove(0) })
		},
	}
}

/// An element Typst shows through a layout routine with the regions at hand.
fn layout_multi_layouter(engine: &mut Engine, elem: &Content, styles: &StyleChain, regions: &Regions) -> Outcome<Vec<Frame>> {
	let kind = match elem.kind() {
		Some(k)	=> k,
		None	=> return Ok(vec![Frame::new(0.0, 0.0)]),
	};
	match kind {
		ElemKind::Pad		=> layout_pad(engine, elem, styles, regions),
		ElemKind::Stack		=> layout_stack(engine, elem, styles, regions),
		ElemKind::Columns	=> layout_columns(engine, elem, styles, regions),
		ElemKind::Layout	=> layout_layout(engine, elem, styles, regions),
		ElemKind::List | ElemKind::Enum	=> crate::flow::lists::layout(engine, elem, styles, regions),
		ElemKind::Grid | ElemKind::Table => {
			let rows = res!(crate::flow::grid::layout_rows(engine, elem, styles, regions.region()));
			let (nodes, breaks) = (&rows.nodes, &rows.breaks);
			split_list(nodes.clone(), regions, breaks, |gi, r, head, skip| rows.split(engine, styles, gi, r, head, skip))
		},
		ElemKind::Equation => {
			let node = res!(crate::flow::math::layout_equation(engine, elem, styles, regions.region()));
			Ok(vec![Frame::from_node(node)])
		},
		k if k.family() == Family::Visual => {
			let node = res!(crate::flow::visual::layout_visual(engine, elem, styles, regions.region()));
			Ok(vec![Frame::from_node(node)])
		},
		other => Err(engine.error(DiagnosticKind::Type, elem.span(), fmt!("{} cannot be laid out as a block", other.path()))),
	}
}

/// A `pad`'s four paddings, left, top, right and bottom, and its body.
fn pad_parts(elem: &Content, styles: &StyleChain) -> Outcome<([Rel; 4], Content)> {
	let rest = res!(field(elem, styles, "rest"));
	let x = match res!(field(elem, styles, "x")) { Some(v) => Some(v), None => rest.clone() };
	let y = match res!(field(elem, styles, "y")) { Some(v) => Some(v), None => rest };
	let mut padding = [Rel::zero(); 4];
	for (i, (name, axis)) in [("left", &x), ("top", &y), ("right", &x), ("bottom", &y)].into_iter().enumerate() {
		let v = match res!(field(elem, styles, name)) { Some(v) => Some(v), None => axis.clone() };
		padding[i] = v.and_then(|v| rel_of(styles, &v)).unwrap_or_default();
	}
	let body = match res!(field(elem, styles, "body")) {
		Some(v)	=> res!(v.cast::<Content>()),
		None	=> Content::empty(),
	};
	Ok((padding, body))
}

/// `pad`: the body laid out in regions shrunk by the padding, each frame grown back by it.
fn layout_pad(engine: &mut Engine, elem: &Content, styles: &StyleChain, regions: &Regions) -> Outcome<Vec<Frame>> {
	let (padding, body) = res!(pad_parts(elem, styles));
	let mut frames = res!(layout_fragment_in(engine, elem.place(), &body, styles, pad_regions(regions, &padding)));
	for f in &mut frames {
		grow(f, &padding);
	}
	Ok(frames)
}

/// A `columns`' count, gutter and body.
fn columns_parts(engine: &mut Engine, elem: &Content, styles: &StyleChain) -> Outcome<(usize, Rel, Content)> {
	let count = match res!(field(elem, styles, "count")) {
		Some(Value::Int(n)) if n > 0	=> n as usize,
		Some(Value::Int(_))				=> return Err(engine.error(DiagnosticKind::Type, elem.span(), "number must be positive")),
		_								=> 2,
	};
	let gutter = res!(field(elem, styles, "gutter")).and_then(|v| rel_of(styles, &v)).unwrap_or(Rel { ratio: 0.04, abs: 0.0 });
	let body = match res!(field(elem, styles, "body")) {
		Some(v)	=> res!(v.cast::<Content>()),
		None	=> Content::empty(),
	};
	Ok((count, gutter, body))
}

/// `columns(n)`: the body flowed through `n` column regions per region, as a page's columns are.
fn layout_columns(engine: &mut Engine, elem: &Content, styles: &StyleChain, regions: &Regions) -> Outcome<Vec<Frame>> {
	let (count, gutter, body) = res!(columns_parts(engine, elem, styles));
	engine.descend_fragment(elem.span(), |engine| {
		let (pairs, inline) = res!(engine.within(elem.place(), |engine| realise::realise_fragment(engine, &body, styles)));
		let mode = if inline { FlowMode::Inline } else { FlowMode::Block };
		layout_flow_pairs(engine, pairs, styles, regions.clone(), count, gutter, mode, elem.span())
	})
}

/// `layout(size => ..)`: the function called under context with the base size of the regions, its result
/// displayed.
fn layout_content(engine: &mut Engine, elem: &Content, styles: &StyleChain, base: (f64, f64)) -> Outcome<Content> {
	let func = match res!(field(elem, styles, "func")) {
		Some(Value::Func(f))	=> f,
		_						=> return Err(err!(
			"layout has no `func` field in its schema, or it holds no function; `layout(..)` cannot run."; Unimplemented)),
	};
	let (w, h) = base;
	let mut size = Dict::new();
	size.insert("width", Value::Length(Length::pt(w)));
	size.insert("height", Value::Length(Length::pt(h)));
	let mut args = crate::eval::args::Args::new(elem.span());
	args.push(elem.span(), Value::dict(size));
	let outer = std::mem::replace(&mut engine.context, Context { location: elem.location(), styles: Some(styles.clone()) });
	let result = engine.call_func(&func, args);
	engine.context = outer;
	let value = res!(result);
	crate::eval::content::display(engine, value, elem.span())
}

/// `layout(size => ..)`: the function's result flowed through the regions.
fn layout_layout(engine: &mut Engine, elem: &Content, styles: &StyleChain, regions: &Regions) -> Outcome<Vec<Frame>> {
	let content = res!(layout_content(engine, elem, styles, regions.base()));
	layout_fragment_in(engine, elem.place(), &content, styles, regions.clone())
}

/// A vertical node list (a grid's rows) broken across the regions: each region takes the boxes that fit,
/// breaking only where the list allows -- at glue after a box, or at a penalty that is not forbidden. A
/// repeated header armed in the list is set again at the top of every region the rows spill onto, and a
/// repeated footer armed in it closes every region but the last, straight after the rows, its height kept
/// free in each.
fn split_list<F>(nodes: Vec<Node>, regions: &Regions, breaks: &[Option<usize>], mut split: F) -> Outcome<Vec<Frame>>
	where F: FnMut(usize, &Regions, f64, bool) -> Outcome<RowSplit>
{
	let mut s = Splitter {
		regions:	regions.clone(),
		frames:		Vec::new(),
		cur:		Frame::new(0.0, 0.0),
		y:			0.0,
		boxes:		false,
		tail:		0.0,
		base:		0.0,
		head:		None,
		foot:		None,
	};
	let mut at_break	= true;		// a break is allowed before the next box
	let mut prev_box	= false;
	let mut pending: Option<(usize, Vec<BoxNode>)> = None;	// the parts of a breakable row ahead
	let mut i = 0;
	while i < nodes.len() {
		match &nodes[i] {
			Node::Glue(g) => {
				if s.boxes {
					s.y += g.natural.to_pt();
					s.tail += g.natural.to_pt();
				}
				if prev_box {
					at_break = true;
				}
				prev_box = false;
			},
			Node::Penalty(p) => {
				if p.is_forced() && s.boxes && s.regions.may_break() {
					s.next_region();
					at_break = true;
				} else if p.is_forbidden() {
					at_break = false;
				} else {
					at_break = true;
				}
				prev_box = false;
			},
			Node::RepeatHead(h) => s.head = h.as_ref().map(|b| (**b).clone()),
			Node::RepeatFoot(f) => s.foot = f.as_ref().map(|b| (**b).clone()),
			Node::Anchor(_) | Node::Tag(_) | Node::Mark(_) => s.cur.push(0.0, s.y, Item::Node(nodes[i].clone())),
			Node::Float(_) | Node::Columns(_) | Node::PageColumns(_) => (),
			node => {
				if at_break {
					let atom = atom_height(&nodes, i);
					let foot = s.foot.as_ref().map(|f| f.dims.vextent().to_pt()).unwrap_or(0.0);
					let room = s.regions.h - s.y - foot;
					if !fits(room, atom) {
						// A row that breaks is set into the regions it needs, after what is welded above it; one
						// that does not goes whole to the next region, which a first row does too when the
						// region has not its full height.
						pending = res!(break_row(&nodes, breaks, i, &s, room, s.may_progress(), &mut split));
						if pending.is_none() && s.may_progress() {
							s.next_region();
							let room = s.regions.h - s.y;
							if !fits(room, atom) {
								pending = res!(break_row(&nodes, breaks, i, &s, room, false, &mut split));
							}
						}
					}
				}
				match pending.take() {
					Some((j, parts)) if j == i => {
						s.place_parts(parts);
						at_break = false;
						prev_box = true;
						i += 1;
						continue;
					},
					other => pending = other,
				}
				s.place(node.clone());
				at_break = false;
				prev_box = true;
			},
		}
		i += 1;
	}
	s.finish();
	Ok(s.frames)
}

/// The breakable row that ends the atom starting at `start` (a row alone, or one welded below a header), broken
/// to fit `room` when it can be: the node holding its first part's box and the parts, if it breaks.
fn break_row<F>(
	nodes:		&[Node],
	breaks:		&[Option<usize>],
	start:		usize,
	s:			&Splitter,
	room:		f64,
	skip:		bool,
	split:		&mut F,
)
	-> Outcome<Option<(usize, Vec<BoxNode>)>>
	where F: FnMut(usize, &Regions, f64, bool) -> Outcome<RowSplit>
{
	let (mut h, mut head, mut prev_box) = (0.0, s.head.as_ref().map(|b| b.dims.vextent().to_pt()), false);
	let mut row = None;
	for (k, n) in nodes.iter().enumerate().skip(start) {
		match n {
			Node::Glue(g) => {
				if prev_box {
					break;
				}
				h += g.natural.to_pt();
			},
			Node::Penalty(p) => {
				if !p.is_forbidden() {
					break;
				}
				prev_box = false;
			},
			Node::RepeatHead(b) => head = b.as_ref().map(|b| b.dims.vextent().to_pt()),
			Node::Anchor(_) | Node::Tag(_) | Node::Mark(_) | Node::RepeatFoot(_) | Node::Float(_)
				| Node::Columns(_) | Node::PageColumns(_) => (),
			other => {
				row = breaks.get(k).copied().flatten().map(|gi| (k, gi, h));
				h += other.vextent().to_pt();
				prev_box = true;
			},
		}
	}
	let (k, gi, lead) = match row {
		Some(r)	=> r,
		None	=> return Ok(None),
	};
	let mut r = s.regions.clone();
	r.h = room - lead;
	Ok(match res!(split(gi, &r, head.unwrap_or(0.0), skip)) {
		RowSplit::Parts(parts)	=> Some((k, parts)),
		RowSplit::Skip | RowSplit::Whole => None,
	})
}

struct Splitter {
	regions:	Regions,
	frames:		Vec<Frame>,
	cur:		Frame,
	y:			f64,
	boxes:		bool,			// the region holds a box
	tail:		f64,			// the gutters since the last box, left out of a region that ends there
	base:		f64,			// the height a region starts at: what its repeated header takes
	head:		Option<BoxNode>,
	foot:		Option<BoxNode>,
}

impl Splitter {
	fn place(&mut self, node: Node) {
		let d = node_dims(&node).unwrap_or_default();
		self.cur.w = self.cur.w.max(d.width.to_pt());
		if !self.boxes && self.cur.baseline.is_none() {
			self.cur.baseline = Some(self.y + d.height.to_pt());
		}
		let h = node.vextent().to_pt();
		self.cur.push(0.0, self.y, Item::Node(node));
		self.y += h;
		self.boxes = true;
		self.tail = 0.0;
	}

	/// Would the next region give more room? Typst's `may_progress` on the room left, which what has been placed
	/// beyond the repeated header has shortened.
	fn may_progress(&self) -> bool { self.y > self.base + EPS || self.regions.may_progress() }

	/// A row broken across regions: its first part here, each other in a region of its own.
	fn place_parts(&mut self, parts: Vec<BoxNode>) {
		for (k, part) in parts.into_iter().enumerate() {
			if k > 0 {
				self.next_region();
			}
			self.place(Node::VBox(part));
		}
	}

	fn finish(&mut self) {
		let mut done = std::mem::replace(&mut self.cur, Frame::new(0.0, 0.0));
		done.h = if self.regions.expand_y { self.regions.h } else { self.y - self.tail };
		// A grid is as wide as its columns, and no wider than the region where they overflow it: Typst places
		// an overflowing grid at the region's start, whatever the alignment.
		done.w = if self.regions.expand_x { self.regions.w } else { done.w.min(self.regions.w) };
		self.frames.push(done);
	}

	/// Closes the region (with the repeated footer, if one is armed) and opens the next (with the repeated
	/// header, if one is armed).
	fn next_region(&mut self) {
		if let Some(foot) = self.foot.clone() {
			self.place(Node::VBox(foot));
		}
		self.finish();
		self.regions.next();
		self.y		= 0.0;
		self.tail	= 0.0;
		self.boxes	= false;
		if let Some(head) = self.head.clone() {
			self.place(Node::VBox(head));
		}
		self.base	= self.y;
	}
}

/// The height of the boxes from `start` up to the list's next legal breakpoint.
fn atom_height(nodes: &[Node], start: usize) -> f64 {
	let mut h = 0.0;
	let mut prev_box = false;
	for n in &nodes[start..] {
		match n {
			Node::Glue(g) => {
				if prev_box {
					break;
				}
				h += g.natural.to_pt();
			},
			Node::Penalty(p) => {
				if !p.is_forbidden() {
					break;
				}
				prev_box = false;
			},
			Node::Anchor(_) | Node::Tag(_) | Node::Mark(_) | Node::RepeatHead(_) | Node::RepeatFoot(_)
				| Node::Float(_) | Node::Columns(_) | Node::PageColumns(_) => (),
			other => {
				h += other.vextent().to_pt();
				prev_box = true;
			},
		}
	}
	h
}

/// `stack`: children set along a direction with spacing between them, Typst's `layout_stack`.
fn layout_stack(engine: &mut Engine, elem: &Content, styles: &StyleChain, regions: &Regions) -> Outcome<Vec<Frame>> {
	let dir = match res!(field(elem, styles, "dir")) {
		Some(Value::Direction(d))	=> d,
		_							=> Direction::Ttb,
	};
	let spacing = match res!(field(elem, styles, "spacing")) {
		None | Some(Value::None)	=> None,
		Some(v)						=> spacing_of(styles, &v),
	};
	let children = match res!(field(elem, styles, "children")) {
		Some(Value::Array(a))	=> (*a).clone(),
		_						=> Vec::new(),
	};
	let mut s = Stacker::new(dir, regions.clone(), elem.span());
	let mut deferred: Option<Spacing> = None;
	// Each child is realised at a place of its own under the stack's, so it is located alike whenever the
	// stack is laid out again.
	let mut places = elem.place().map(Locator::new);
	for child in children {
		let content = match child {
			Value::Content(c)	=> c,
			Value::Str(t)		=> Content::text(&t),
			other				=> match spacing_of(styles, &other) {
				Some(sp) => {
					s.spacing(sp, styles);
					deferred = None;
					continue;
				},
				None => res!(other.cast::<Content>()),
			},
		};
		// `h` in a horizontal stack and `v` in a vertical one are spacing, transparently.
		let transparent = match (s.vertical, content.kind()) {
			(false, Some(ElemKind::H)) | (true, Some(ElemKind::V)) => res!(field(&content, styles, "amount"))
				.and_then(|a| spacing_of(styles, &a)),
			_ => None,
		};
		if let Some(sp) = transparent {
			s.spacing(sp, styles);
			deferred = None;
			continue;
		}
		if let Some(sp) = deferred {
			s.spacing(sp, styles);
		}
		let place = places.as_mut().map(|l| l.next(content.kind().unwrap_or(ElemKind::Sequence), content.span()));
		res!(s.block(engine, place, &content, styles));
		deferred = spacing;
	}
	res!(s.finish_region(engine));
	Ok(s.finished)
}

pub(super) enum StackItem {
	Abs(f64),
	Fr(f64),
	Frame(Frame, Align2),
}

pub(super) struct Stacker {
	vertical:	bool,
	positive:	bool,
	regions:	Regions,
	expand:		(bool, bool),
	initial:	(f64, f64),
	used_main:	f64,
	used_cross:	f64,
	fr:			f64,
	items:		Vec<StackItem>,
	finished:	Vec<Frame>,
	span:		Span,
}

impl Stacker {
	pub(super) fn new(dir: Direction, mut regions: Regions, span: Span) -> Self {
		let vertical = matches!(dir, Direction::Ttb | Direction::Btt);
		let expand = (regions.expand_x, regions.expand_y);
		// Children do not expand along the stacking axis.
		if vertical {
			regions.expand_y = false;
		} else {
			regions.expand_x = false;
		}
		let initial = (regions.w, regions.h);
		Self {
			vertical,
			positive:	matches!(dir, Direction::Ttb | Direction::Ltr),
			regions,
			expand,
			initial,
			used_main:	0.0,
			used_cross:	0.0,
			fr:			0.0,
			items:		Vec::new(),
			finished:	Vec::new(),
			span,
		}
	}

	pub(super) fn spacing(&mut self, sp: Spacing, _styles: &StyleChain) {
		match sp {
			Spacing::Rel(r) => {
				let base = if self.vertical { self.regions.base().1 } else { self.regions.base().0 };
				let resolved = r.relative_to(base);
				let remaining = if self.vertical { self.regions.h } else { self.regions.w };
				let limited = resolved.min(remaining);
				if self.vertical {
					self.regions.h -= limited;
				}
				self.used_main += limited;
				self.items.push(StackItem::Abs(resolved));
			},
			Spacing::Fr(f) => {
				self.fr += f;
				self.items.push(StackItem::Fr(f));
			},
		}
	}

	fn block(&mut self, engine: &mut Engine, place: Option<Place>, block: &Content, styles: &StyleChain) -> Outcome<()> {
		if self.regions.is_full() {
			res!(self.finish_region(engine));
		}
		// The alignment along the stacking axis a child asks for: an `align` child's own, a styled child's,
		// else the alignment in force.
		let align = match block {
			Content::Styled(st)	=> res!(alignment(&styles.chain(&st.styles))),
			c if c.is(ElemKind::Align) => {
				let a = res!(field(c, styles, "alignment"));
				let chained = match (a, ElemKind::Align.field_id("alignment")) {
					(Some(v), Some(id)) => styles.chain(&crate::eval::styles::Styles::from_style(
						crate::eval::styles::Style::Property(crate::eval::styles::Property::new(
							ElemKind::Align, id, v, c.span())))),
					_ => styles.clone(),
				};
				res!(alignment(&chained))
			},
			_ => res!(alignment(styles)),
		};
		let frames = res!(layout_fragment_in(engine, place, block, styles, self.regions.clone()));
		self.fragment(engine, align, frames)
	}

	/// A child with a layout routine of its own, Typst's `StackLayoutChild::CustomLayouter`: the routine is
	/// given the regions left and returns the child's frames, which take the alignment in force.
	pub(super) fn custom<F>(&mut self, engine: &mut Engine, styles: &StyleChain, lay: F) -> Outcome<()>
		where F: FnOnce(&mut Engine, &Regions) -> Outcome<Vec<Frame>>
	{
		if self.regions.is_full() {
			res!(self.finish_region(engine));
		}
		let align = res!(alignment(styles));
		let frames = res!(lay(engine, &self.regions));
		self.fragment(engine, align, frames)
	}

	/// Takes laid out frames, a block's or a custom routine's, into the regions.
	fn fragment(&mut self, engine: &mut Engine, align: Align2, frames: Vec<Frame>) -> Outcome<()> {
		let n = frames.len();
		for (i, frame) in frames.into_iter().enumerate() {
			let (main, cross) = if self.vertical { (frame.h, frame.w) } else { (frame.w, frame.h) };
			if self.vertical {
				self.regions.h -= frame.h;
			}
			self.used_main += main;
			self.used_cross = self.used_cross.max(cross);
			self.items.push(StackItem::Frame(frame, align));
			if i + 1 < n {
				res!(self.finish_region(engine));
			}
		}
		Ok(())
	}

	fn finish_region(&mut self, engine: &mut Engine) -> Outcome<()> {
		let used = if self.vertical { (self.used_cross, self.used_main) } else { (self.used_main, self.used_cross) };
		let mut size = (
			(if self.expand.0 { self.initial.0 } else { used.0 }).min(self.initial.0),
			(if self.expand.1 { self.initial.1 } else { used.1 }).min(self.initial.1),
		);
		let full = if self.vertical { self.initial.1 } else { self.initial.0 };
		let remaining = full - self.used_main;
		if self.fr > 0.0 && full.is_finite() {
			self.used_main = full;
			if self.vertical {
				size.1 = full;
			} else {
				size.0 = full;
			}
		}
		if !size.0.is_finite() || !size.1.is_finite() {
			return Err(engine.error(DiagnosticKind::Type, self.span, "stack spacing is infinite"));
		}
		let mut output	= Frame::new(size.0, size.1);
		let mut cursor	= 0.0;
		let mut ruler	= if self.positive { Fixed::Start } else { Fixed::End };
		let fr			= self.fr;
		for item in std::mem::take(&mut self.items) {
			match item {
				StackItem::Abs(v)	=> cursor += v,
				StackItem::Fr(v)	=> cursor += share(v, fr, remaining),
				StackItem::Frame(frame, align) => {
					let a_main	= if self.vertical { align.y } else { align.x };
					let a_cross	= if self.vertical { align.x } else { align.y };
					ruler = if self.positive { ruler.max(a_main) } else { ruler.min(a_main) };
					let parent	= if self.vertical { size.1 } else { size.0 };
					let child	= if self.vertical { frame.h } else { frame.w };
					let main	= ruler.position(parent - self.used_main)
						+ if self.positive { cursor } else { self.used_main - child - cursor };
					let cross_extent = if self.vertical { size.0 - frame.w } else { size.1 - frame.h };
					let cross	= a_cross.position(cross_extent);
					let (x, y)	= if self.vertical { (cross, main) } else { (main, cross) };
					cursor += child;
					output.push_frame(x, y, frame);
				},
			}
		}
		self.regions.next();
		self.initial	= (self.regions.w, self.regions.h);
		self.used_main	= 0.0;
		self.used_cross	= 0.0;
		self.fr			= 0.0;
		self.finished.push(output);
		Ok(())
	}

	/// Closes the last region and hands over every region's frame.
	pub(super) fn finish(mut self, engine: &mut Engine) -> Outcome<Vec<Frame>> {
		res!(self.finish_region(engine));
		Ok(self.finished)
	}
}
