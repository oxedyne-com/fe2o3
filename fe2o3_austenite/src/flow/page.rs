// SPDX-License-Identifier: Apache-2.0
// Original work copyright (c) the Typst project authors (typst-layout `pages/*`, version 0.15.1).
// Modified for Austenite: ported to Hematite's types, IR and error handling.
// U6b owns this file: the document's top level as pages, one page at a time, honouring `pagebreak(weak:,
// to:)`. The driver's `place_page` takes each page's body as it comes.
//
// A port of Typst 0.15.1's `typst-layout` pages (`pages/{mod,collect,run}.rs`), Apache-2.0, (c) the Typst
// authors, streamed. Typst slices the realised document at its page breaks into runs of uniform page
// styles and flows each run through pages of its size as the page's root flow (which hosts footnotes).
// Here the slicing is a pull: [`Level`] reads the realised pairs in order with a bounded lookahead, a run
// is an open feed into a [`FlowCursor`] that stops at the next page break, and the [`Paginator`] yields
// one [`PageBody`] per call. Nothing spans a run: no run, page or pair list is held beyond the page being
// laid and the pairs pending at a break. A `pagebreak(to:)` becomes a blank page when the page count so
// far needs one for the parity. Tags that fall between pages wait for the next page's top-left corner,
// or land at the foot of the last page.
//
// The run's page-wide styles (Typst's `Styles::root` of the run's pairs) are taken over the first
// `TRUNK_WINDOW` pairs of the run, so a run of any length opens with a bounded lookahead. Page styles cannot
// differ inside a run, since realisation brackets a change with boundary breaks; only styles such as the
// text size can differ, and a pair beyond the window that drops one of them from the trunk no longer moves it.

use crate::diag::DiagnosticKind;
use crate::driver::{
	self,
	Recorder,
};
use crate::eval::content::{
	Content,
	ElemKind,
};
use crate::eval::lib::layout as lay;
use crate::eval::locate::Location;
use crate::eval::realise::{
	self,
	Pair,
	RealiseMode,
	Tag,
};
use crate::eval::styles::{
	Style,
	StyleChain,
	Styles,
};
use crate::eval::value::{
	Direction,
	HAlign,
	Paint,
	Value,
};
use crate::eval::Engine;
use crate::flow::block::{
	Feed,
	FlowCursor,
	FlowMode,
	Frame,
	Regions,
	Rel,
};
use crate::flow::{
	decorate,
	Parity,
	RunSetup,
	Slot,
};
use crate::ir::Sp;
use crate::page::{
	Page,
	PageGeometry,
	Swap,
};
use crate::syntax::Span;

use oxedyne_fe2o3_core::prelude::*;

use std::cell::RefCell;
use std::collections::{
	HashSet,
	VecDeque,
};
use std::rc::Rc;
use std::sync::Arc;

const PT_PER_MM:	f64		= 72.0 / 25.4;
const A4_WIDTH_MM:	f64		= 210.0;
const TRUNK_WINDOW:	usize	= 1024;	// pairs of a run read ahead to find its page-wide styles

/// One page's body, as the flow leaves it and the driver takes it.
#[derive(Debug)]
pub struct PageBody {
	pub setup:		Arc<RunSetup>,
	pub number:		u32,
	pub body:		Frame,			// inside the margins; empty for a parity blank
	pub tags:		Vec<Tag>,		// tags waiting for this page's top-left corner
	pub foot:		Vec<Tag>,		// tags after the last page, which land at its foot
	pub run_start:	bool,			// flow state is canonical here: the run cache's boundary
}

/// The document's realised pairs and the page-level state that slices them into runs.
#[derive(Debug)]
pub struct Level {
	src:		Source,
	staged:		bool,				// a break is owed an empty page if nothing follows it
	initial:	StyleChain,			// the styles an empty run takes
	out:		VecDeque<Item>,		// items made but not yet handed on
	finished:	bool,
	open:		bool,				// a run's pairs are being pulled
	ended:		bool,				// the open run has no more pairs
	held:		VecDeque<Pair>,		// pairs of the open run cleared to go out
}

/// Where the realised pairs come from. Interim: the whole document realised at the first pull, then popped
/// from the front, until a pulled `realise::Stream` replaces it.
#[derive(Debug)]
enum Source {
	Lazy(Content, StyleChain),
	Pairs(VecDeque<Pair>),
}

impl Source {
	fn ready(&mut self, engine: &mut Engine) -> Outcome<&mut VecDeque<Pair>> {
		if let Source::Lazy(content, styles) = self {
			let pairs = res!(realise::realise(engine, content, styles, RealiseMode::Document));
			*self = Source::Pairs(pairs.into());
		}
		match self {
			Source::Pairs(q)	=> Ok(q),
			Source::Lazy(..)	=> Err(err!("The page source was not realised after being readied."; Bug)),
		}
	}
}

/// A piece of the document at page level: Typst's page `Item`.
#[derive(Debug)]
enum Item {
	Run(StyleChain),				// content to lay out on pages of one style, from these initial styles
	Pairs(Vec<Pair>, StyleChain),	// a run of these pairs alone: tags with a page of their own, or none
	Parity(Parity, StyleChain),		// a blank page, if the page count needs one for the parity
	Tags(Vec<Pair>),				// tags between pages, for the next page's corner
}

impl Level {
	fn new(content: &Content, styles: &StyleChain) -> Self {
		Self {
			src:		Source::Lazy(content.clone(), styles.clone()),
			staged:		true,
			initial:	styles.clone(),
			out:		VecDeque::new(),
			finished:	false,
			open:		false,
			ended:		false,
			held:		VecDeque::new(),
		}
	}

	/// The next piece of the document: Typst's page `collect`, a step at a time. A strong break with nothing
	/// laid out since the last one yields an empty page; the styles after a break are the next run's initial
	/// ones, unless it is the boundary break that ends a `set page` scope.
	fn next_item(&mut self, engine: &mut Engine) -> Outcome<Option<Item>> {
		loop {
			if let Some(item) = self.out.pop_front() {
				return Ok(Some(item));
			}
			if self.finished {
				return Ok(None);
			}
			let q = res!(self.src.ready(engine));
			if q.is_empty() {
				self.finished = true;
				return Ok(if self.staged { Some(Item::Pairs(Vec::new(), self.initial.clone())) } else { None });
			}
			if is_pagebreak(&q[0]) {
				let p = match q.pop_front() {
					Some(p)	=> p,
					None	=> return Err(err!("A page break vanished between peeking and taking it."; Bug)),
				};
				let strong = !res!(pagebreak_flag(&p, "weak"));
				if strong && self.staged {
					self.out.push_back(Item::Pairs(Vec::new(), self.initial.clone()));
				}
				if let Some(parity) = res!(pagebreak_to(&p)) {
					self.out.push_back(Item::Parity(parity, p.styles.clone()));
				}
				if !res!(pagebreak_flag(&p, "boundary")) {
					self.initial = p.styles.clone();
				}
				self.staged |= strong;
				continue;
			}
			let k = leading_tags(q);
			if q.get(k).map(|p| !is_pagebreak(p)).unwrap_or(false) {
				// Content before the next break: a run opens here.
				self.open	= true;
				self.ended	= false;
				self.staged	= false;
				self.held.clear();
				return Ok(Some(Item::Run(self.initial.clone())));
			}
			// Tags only, up to a break or the end.
			let nb		= leading_breaks(q, k);
			let block: Vec<Pair> = q.drain(..k).collect();
			let (stay, go) = migrate(block);
			let group = if stay.is_empty() {
				if nb > 0 {
					reinsert(q, nb, go);
					continue;
				}
				// Unterminated start tags with no break after them stay where they are.
				go
			} else {
				reinsert(q, nb, go);
				stay
			};
			let mut rest_boundary = true;
			for p in q.iter() {
				if !(is_pagebreak(p) && res!(pagebreak_flag(p, "boundary"))) {
					rest_boundary = false;
					break;
				}
			}
			if !(self.staged && rest_boundary) {
				return Ok(Some(Item::Tags(group)));
			}
			self.staged = false;
			return Ok(Some(Item::Pairs(group, self.initial.clone())));
		}
	}

	/// The page-wide styles of the run that has just opened: the root styles of its first pairs, else
	/// the initial ones.
	fn run_styles(&mut self, engine: &mut Engine, initial: &StyleChain) -> Outcome<StyleChain> {
		let q = res!(self.src.ready(engine));
		let mut chains = Vec::new();
		for p in q.iter().take(TRUNK_WINDOW) {
			if is_pagebreak(p) {
				break;
			}
			if !p.is_tag() {
				chains.push(&p.styles);
			}
		}
		Ok(root_styles(chains, initial))
	}

	/// The next pair of the open run. Tags that end the run before a break are sorted as Typst's
	/// `migrate_unterminated_tags` has it: those that close an element stay, and start tags without their
	/// end tags move on, after the break.
	pub(crate) fn pull(&mut self, engine: &mut Engine) -> Outcome<Option<Pair>> {
		if let Some(p) = self.held.pop_front() {
			return Ok(Some(p));
		}
		if !self.open || self.ended {
			return Ok(None);
		}
		let q = res!(self.src.ready(engine));
		let k = leading_tags(q);
		if q.get(k).map(|p| !is_pagebreak(p)).unwrap_or(false) {
			return Ok(q.pop_front());
		}
		if k == 0 {
			self.ended = true;
			return Ok(None);
		}
		let nb		= leading_breaks(q, k);
		let block: Vec<Pair> = q.drain(..k).collect();
		let (stay, go) = migrate(block);
		reinsert(q, nb, go);
		self.held.extend(stay);
		self.ended = true;
		Ok(self.held.pop_front())
	}

	/// Does the open run have another pair?
	pub(crate) fn has_more(&mut self, engine: &mut Engine) -> Outcome<bool> {
		if !self.held.is_empty() {
			return Ok(true);
		}
		match res!(self.pull(engine)) {
			Some(p)	=> {
				self.held.push_front(p);
				Ok(true)
			},
			None	=> Ok(false),
		}
	}
}

/// The count of tags at the front of the queue.
fn leading_tags(q: &VecDeque<Pair>) -> usize { q.iter().take_while(|p| p.is_tag()).count() }

/// The count of page breaks in the queue from `from` on.
fn leading_breaks(q: &VecDeque<Pair>, from: usize) -> usize {
	q.iter().skip(from).take_while(|p| is_pagebreak(p)).count()
}

/// Puts tags back into the queue after its first `at` pairs.
fn reinsert(q: &mut VecDeque<Pair>, at: usize, tags: Vec<Pair>) {
	for (i, t) in tags.into_iter().enumerate() {
		q.insert(at + i, t);
	}
}

/// Splits a block of tags that ends a run into those that stay (the end tags, and the start tags whose end
/// tags are in the block) and the unterminated start tags that move after the page breaks that follow.
fn migrate(block: Vec<Pair>) -> (Vec<Pair>, Vec<Pair>) {
	let excluded: HashSet<Location> = block.iter().filter_map(|p| match &p.tag {
		Some(Tag::End(l))	=> Some(*l),
		_					=> None,
	}).collect();
	let (mut stay, mut go) = (Vec::new(), Vec::new());
	for p in block {
		match tag_location(&p) {
			Some(l) if excluded.contains(&l)	=> stay.push(p),
			_									=> go.push(p),
		}
	}
	(stay, go)
}

/// The open run: its setup, the cursor flowing its pairs, and the regions its pages are laid into.
#[derive(Debug)]
struct Active {
	setup:		Arc<RunSetup>,
	cursor:		FlowCursor,
	regions:	Regions,
	first:		bool,
}

/// The document as pages, one at a time. `next_page` yields each page's body; [`crate::driver::place_page`]
/// places it, and the caller drops it before asking for the next.
#[derive(Debug)]
pub struct Paginator {
	level:		Rc<RefCell<Level>>,
	span:		Span,
	active:		Option<Active>,
	blank:		Option<Arc<RunSetup>>,	// a parity blank page owed before the next run
	count:		u32,					// pages yielded
	waiting:	Vec<Tag>,				// tags between pages, for the next page's corner
	started:	bool,
	ended:		bool,
}

impl Paginator {
	pub fn new(content: &Content, styles: &StyleChain) -> Self {
		Self {
			level:		Rc::new(RefCell::new(Level::new(content, styles))),
			span:		content.span(),
			active:		None,
			blank:		None,
			count:		0,
			waiting:	Vec::new(),
			started:	false,
			ended:		false,
		}
	}

	/// The next page's body, or `None` after the last page.
	pub fn next_page(&mut self, engine: &mut Engine) -> Outcome<Option<PageBody>> {
		if !self.started {
			self.started = true;
			res!(self.advance(engine));
		}
		if let Some(setup) = self.blank.take() {
			self.count += 1;
			let body = blank_area(&setup);
			let mut page = PageBody {
				setup,
				number:		self.count,
				body,
				tags:		std::mem::take(&mut self.waiting),
				foot:		Vec::new(),
				run_start:	true,
			};
			res!(self.advance(engine));
			self.land_foot(&mut page);
			return Ok(Some(page));
		}
		let (mut page, done) = match &mut self.active {
			None		=> return Ok(None),
			Some(a)		=> {
				let frame = res!(a.cursor.region(engine, &a.regions));
				self.count += 1;
				let page = PageBody {
					setup:		a.setup.clone(),
					number:		self.count,
					body:		frame,
					tags:		std::mem::take(&mut self.waiting),
					foot:		Vec::new(),
					run_start:	a.first,
				};
				a.first = false;
				(page, res!(a.cursor.is_done(engine)))
			},
		};
		if done {
			self.active = None;
			res!(self.advance(engine));
			self.land_foot(&mut page);
		}
		Ok(Some(page))
	}

	/// The next page placed: its furniture laid out ([`decorate::decorate_page`]) and its body put on its page
	/// by [`driver::place_page`], the located elements they hold going to `rec`, with the setup of its run.
	pub fn next_placed<R: Recorder>(&mut self, engine: &mut Engine, rec: &mut R)
		-> Outcome<Option<(Page, Arc<RunSetup>)>>
	{
		match res!(self.next_page(engine)) {
			None		=> Ok(None),
			Some(body)	=> {
				let setup = body.setup.clone();
				let marginals = res!(decorate::decorate_page(engine, &setup, (body.body.w, body.body.h)));
				Ok(Some((res!(driver::place_page(body, marginals, rec)), setup)))
			},
		}
	}

	/// Tags still waiting once no page follows land at the foot of the page just made.
	fn land_foot(&mut self, page: &mut PageBody) {
		if self.ended && self.active.is_none() && self.blank.is_none() {
			page.foot = std::mem::take(&mut self.waiting);
		}
	}

	/// Takes the page-level pieces that follow the run just finished, up to the next that makes a page.
	fn advance(&mut self, engine: &mut Engine) -> Outcome<()> {
		loop {
			let item = res!(self.level.borrow_mut().next_item(engine));
			match item {
				None => {
					self.ended = true;
					return Ok(());
				},
				Some(Item::Run(initial)) => {
					let styles = res!(self.level.borrow_mut().run_styles(engine, &initial));
					let feed = Feed::Run(self.level.clone());
					return self.open(engine, feed, styles);
				},
				Some(Item::Pairs(pairs, initial)) => {
					let styles = styles_of(&pairs, &initial);
					return self.open(engine, Feed::list(pairs), styles);
				},
				Some(Item::Parity(parity, styles)) => {
					if parity_wants_blank(parity, self.count) {
						let setup = res!(PageSetup::of(engine, &styles, self.span));
						self.blank = Some(Arc::new(setup.run(styles)));
						return Ok(());
					}
				},
				Some(Item::Tags(pairs)) => {
					self.waiting.extend(pairs.into_iter().filter_map(|p| p.tag));
				},
			}
		}
	}

	/// Opens a run of pages: its setup, and a cursor over `feed` as the root flow of its area.
	fn open(&mut self, engine: &mut Engine, feed: Feed, styles: StyleChain) -> Outcome<()> {
		let setup = res!(PageSetup::of(engine, &styles, self.span));
		let (aw, ah) = setup.area;
		let regions = Regions::repeat(aw, ah, aw.is_finite(), ah.is_finite());
		let cursor = res!(FlowCursor::new(
			engine, feed, &styles, &regions, setup.columns, setup.gutter, FlowMode::Root, self.span));
		self.active = Some(Active { setup: Arc::new(setup.run(styles)), cursor, regions, first: true });
		Ok(())
	}
}

/// A parity blank page's body, the size of its run's area as Typst's flow makes it, so the furniture of a
/// blank page spans the page; nothing where the page fits its content.
fn blank_area(setup: &RunSetup) -> Frame {
	let g = &setup.geom;
	match (g.width == Sp::ZERO, g.height == Sp::ZERO) {
		(false, false)	=> Frame::new(g.content_width().to_pt(), g.content_height().to_pt()),
		(false, true)	=> Frame::new(g.content_width().to_pt(), 0.0),
		(true, false)	=> Frame::new(0.0, g.content_height().to_pt()),
		(true, true)	=> Frame::new(0.0, 0.0),
	}
}

/// The page-wide styles of a run held as pairs: the trunk of its contentful pairs' styles, else the initial
/// ones.
fn styles_of(pairs: &[Pair], initial: &StyleChain) -> StyleChain {
	root_styles(pairs.iter().filter(|p| !p.is_tag()).map(|p| &p.styles), initial)
}

/// The styles page furniture and footnote entries take: Typst's `Styles::root` of the run's pairs, which
/// keeps only the outside styles that were in force where the run began or that a set rule could lift, never
/// those a show rule or a call such as `text(size: ..)[..]` put on its own content.
fn root_styles<'a, I: IntoIterator<Item = &'a StyleChain>>(chains: I, initial: &StyleChain) -> StyleChain {
	StyleChain::root().chain(&Styles::root(chains, initial))
}

/// Does a `pagebreak(to: parity)` after `count` pages need a blank page first? Typst adds one when the
/// count itself has the parity asked of the next page's predecessor: `to: odd` after an odd count.
fn parity_wants_blank(parity: Parity, count: u32) -> bool {
	match parity {
		Parity::Even	=> count % 2 == 0,
		Parity::Odd		=> count % 2 == 1,
	}
}

fn is_pagebreak(p: &Pair) -> bool { !p.is_tag() && p.content.is(ElemKind::Pagebreak) }

fn pagebreak_field(p: &Pair, name: &str) -> Outcome<Option<Value>> {
	match ElemKind::Pagebreak.field_id(name) {
		Some(id)	=> p.styles.resolve(&p.content, id),
		None		=> Ok(None),
	}
}

fn pagebreak_flag(p: &Pair, name: &str) -> Outcome<bool> {
	Ok(matches!(res!(pagebreak_field(p, name)), Some(Value::Bool(true))))
}

fn pagebreak_to(p: &Pair) -> Outcome<Option<Parity>> {
	Ok(match res!(pagebreak_field(p, "to")) {
		Some(Value::Str(s)) if s.as_str() == "odd"	=> Some(Parity::Odd),
		Some(Value::Str(s)) if s.as_str() == "even"	=> Some(Parity::Even),
		_											=> None,
	})
}

fn tag_location(p: &Pair) -> Option<crate::eval::locate::Location> {
	match &p.tag {
		Some(Tag::Start(c))	=> c.location(),
		Some(Tag::End(l))	=> Some(*l),
		None				=> None,
	}
}

// Page runs

/// A run's page geometry and furniture, from its page-wide styles.
struct PageSetup {
	geom:		PageGeometry,
	area:		(f64, f64),	// inside the margins; infinite on an axis the page fits to its content
	columns:	usize,
	gutter:		Rel,
	gutter_pt:	f64,
	fill:		Option<Paint>,
	numbering:	Value,
	header:		Slot,
	footer:		Slot,
	background:	Option<Content>,
	foreground:	Option<Content>,
}

impl PageSetup {
	/// Typst's page-run setup: the size (from `paper`, `width`, `height` and `flipped`, `auto` fitting the
	/// content), the margins (`auto` is 2.5/21 of the shorter side, of A4's width when both fit), the
	/// binding for two-sided margins, the columns and their gutter, and the furniture.
	fn of(engine: &mut Engine, styles: &StyleChain, span: Span) -> Outcome<PageSetup> {
		let (mut w, mut h) = page_size(engine, styles, span);
		if matches!(res!(page_field(styles, "flipped")), Some(Value::Bool(true))) {
			std::mem::swap(&mut w, &mut h);
		}
		let mut min = w.min(h);
		if !min.is_finite() {
			min = A4_WIDTH_MM * PT_PER_MM;
		}
		let default = (2.5 / 21.0) * min;
		let margin_v = res!(page_field(styles, "margin"));
		let two_sided = matches!(&margin_v, Some(Value::Dict(d)) if d.contains("inside") || d.contains("outside"));
		let parts = margin_parts(margin_v);
		let side = |i: usize, whole: f64| -> f64 {
			match &parts[i] {
				None | Some(Value::Auto)	=> default,
				Some(v)						=> match rel(styles, v) {
					Some(r)	=> r.relative_to(whole),
					None	=> default,
				},
			}
		};
		let (left, top, right, bottom) = (side(0, w), side(1, h), side(2, w), side(3, h));
		// A bleed extends the page past its trim, which the writers do not draw.
		let bleed = margin_parts(res!(page_field(styles, "bleed")));
		if bleed.iter().flatten().any(|v| rel(styles, v).map(|r| !r.is_zero()).unwrap_or(false)) {
			engine.warn(DiagnosticKind::Unsupported, span, "page bleed is not drawn");
		}
		let dir = match ElemKind::Text.field_id("dir") {
			Some(id)	=> res!(styles.get(ElemKind::Text, id)),
			None		=> None,
		};
		let binding_left = match res!(page_field(styles, "binding")) {
			Some(Value::Alignment(a))	=> !matches!(a.x, Some(HAlign::Right)),
			_							=> !matches!(dir, Some(Value::Direction(Direction::Rtl))),
		};
		let swap = match (two_sided, binding_left) {
			(false, _)		=> Swap::None,
			(true, true)	=> Swap::Even,
			(true, false)	=> Swap::Odd,
		};
		let fit = |v: f64| if v.is_finite() { Sp::from_pt(v) } else { Sp::ZERO };
		let mut geom = PageGeometry::with_margins(
			fit(w), fit(h), Sp::from_pt(left), Sp::from_pt(right), Sp::from_pt(top), Sp::from_pt(bottom));
		geom.swap = swap;
		let columns = match res!(page_field(styles, "columns")) {
			Some(Value::Int(n)) if n > 0	=> n as usize,
			_								=> 1,
		};
		let gutter_v = match ElemKind::Columns.field_id("gutter") {
			Some(id)	=> res!(styles.get(ElemKind::Columns, id)),
			None		=> None,
		};
		let gutter = gutter_v.and_then(|v| rel(styles, &v)).unwrap_or(Rel { ratio: 0.04, abs: 0.0 });
		let area = (w - left - right, h - top - bottom);
		let fill = match res!(page_field(styles, "fill")) {
			Some(v) if !matches!(v, Value::None | Value::Auto) => res!(crate::eval::lib::visual::fill_of(&v)),
			_ => None,
		};
		let content = |name: &str| -> Outcome<Option<Content>> {
			match res!(page_field(styles, name)) {
				None | Some(Value::None) | Some(Value::Auto)	=> Ok(None),
				Some(v)											=> Ok(Some(res!(v.cast::<Content>()))),
			}
		};
		// A header or footer left `auto` is the page numbering's, and `none` clears it.
		let slot = |name: &str| -> Outcome<Slot> {
			match res!(page_field(styles, name)) {
				None | Some(Value::Auto)	=> Ok(Slot::Auto),
				Some(Value::None)			=> Ok(Slot::Off),
				Some(v)						=> Ok(Slot::On(res!(v.cast::<Content>()))),
			}
		};
		Ok(PageSetup {
			geom,
			area,
			columns,
			gutter,
			gutter_pt:	gutter.relative_to(area.0),
			fill,
			numbering:	res!(page_field(styles, "numbering")).unwrap_or(Value::None),
			header:		res!(slot("header")),
			footer:		res!(slot("footer")),
			background:	res!(content("background")),
			foreground:	res!(content("foreground")),
		})
	}

	fn run(&self, styles: StyleChain) -> RunSetup {
		RunSetup {
			geom:		self.geom,
			columns:	self.columns,
			gutter:		if self.gutter_pt.is_finite() { Sp::from_pt(self.gutter_pt) } else { Sp::ZERO },
			fill:		self.fill.clone(),
			numbering:	self.numbering.clone(),
			header:		self.header.clone(),
			footer:		self.footer.clone(),
			background:	self.background.clone(),
			foreground:	self.foreground.clone(),
			styles,
		}
	}
}

/// A `page` field as the chain has it.
pub(crate) fn page_field(styles: &StyleChain, name: &str) -> Outcome<Option<Value>> {
	match ElemKind::Page.field_id(name) {
		Some(id)	=> styles.get(ElemKind::Page, id),
		None		=> Ok(None),
	}
}

/// The page's width and height in points, infinite for `auto`. Each is the nearer of its own setting and
/// a `paper`, which sets both; A4 where neither is set.
fn page_size(engine: &mut Engine, styles: &StyleChain, span: Span) -> (f64, f64) {
	let ids = (
		ElemKind::Page.field_id("width"),
		ElemKind::Page.field_id("height"),
		ElemKind::Page.field_id("paper"),
	);
	let (mut w, mut h): (Option<f64>, Option<f64>) = (None, None);
	for s in styles.walk() {
		let p = match s {
			Style::Property(p) if p.elem == ElemKind::Page	=> p,
			_												=> continue,
		};
		let field = Some(p.field);
		if field == ids.2 {
			let (pw, ph) = match &p.value {
				Value::Str(name) => match lay::paper(name) {
					Some((a, b))	=> (a * PT_PER_MM, b * PT_PER_MM),
					None			=> {
						engine.warn(DiagnosticKind::Unsupported, span, fmt!("unknown paper size: {}", name));
						(A4_WIDTH_MM * PT_PER_MM, 297.0 * PT_PER_MM)
					},
				},
				_ => continue,
			};
			w = w.or(Some(pw));
			h = h.or(Some(ph));
		} else if field == ids.0 && w.is_none() {
			w = Some(length_or_inf(styles, &p.value));
		} else if field == ids.1 && h.is_none() {
			h = Some(length_or_inf(styles, &p.value));
		}
		if w.is_some() && h.is_some() {
			break;
		}
	}
	(w.unwrap_or(A4_WIDTH_MM * PT_PER_MM), h.unwrap_or(297.0 * PT_PER_MM))
}

fn length_or_inf(styles: &StyleChain, v: &Value) -> f64 {
	match v {
		Value::Length(l)	=> styles.resolve_length(*l),
		_					=> f64::INFINITY,
	}
}

pub(crate) fn rel(styles: &StyleChain, v: &Value) -> Option<Rel> {
	match v {
		Value::Length(l)	=> Some(Rel::pt(styles.resolve_length(*l))),
		Value::Ratio(r)		=> Some(Rel { ratio: r.0, abs: 0.0 }),
		Value::Relative(r)	=> Some(Rel { ratio: r.rel.0, abs: styles.resolve_length(r.abs) }),
		_					=> None,
	}
}

/// A margin's four sides (left or inside, top, right or outside, bottom), each possibly unset or `auto`,
/// by Typst's precedence of a side over its axis over `rest`.
fn margin_parts(v: Option<Value>) -> [Option<Value>; 4] {
	match v {
		None | Some(Value::Auto) | Some(Value::None) => [None, None, None, None],
		Some(Value::Dict(d)) => {
			let get		= |k: &str| d.get(k).cloned();
			let rest	= get("rest");
			let x		= get("x").or_else(|| rest.clone());
			let y		= get("y").or_else(|| rest.clone());
			[
				get("inside").or_else(|| get("left")).or_else(|| x.clone()),
				get("top").or_else(|| y.clone()),
				get("outside").or_else(|| get("right")).or_else(|| x.clone()),
				get("bottom").or_else(|| y.clone()),
			]
		},
		Some(other) => [Some(other.clone()), Some(other.clone()), Some(other.clone()), Some(other)],
	}
}
