// SPDX-License-Identifier: Apache-2.0
// Original work copyright (c) the Typst project authors (typst-library `introspection/*` and `foundations/{styles,content}.rs`, version 0.15.1).
// Modified for Austenite: ported to Hematite's types, IR and error handling.
// U8 owns this file: the introspector, counter and state keys, and the read log the fixpoint compares.
// A counter or state is a fold of its update elements in document order; nothing here observes layout
// except through the positions the pages recorded.
//
// A port of Typst 0.15's `Introspector`, `Counter` and `State` semantics (typst-library, Apache-2.0):
// document order is the order the pages placed the elements' start tags, out-of-flow material being read
// where its parent stands; a counter's value at a location is the fold of every matching update at or
// before it; and `before`/`after` bound a query by the first match of their bound. Every question a pass
// asks is recorded with a hash of its answer, and the pass converges when each question answers the same
// against the introspector the pass built, as Typst's `Constraint::validate` decides.
//
// Streaming (addendum 2026-09-23): the introspector is the one structure that spans the document. It
// keeps one compact record per located element -- the prepared element, its position, and its place in
// document order -- and never a frame, a node or a page. [`Builder`] gathers the records while the pages
// stream past, and [`Builder::finish`] orders them once, at the end of the pass.

use crate::diag::DiagnosticKind;
use crate::doc::DocInfo;
use crate::eval::args::Args;
use crate::eval::content::{
	Content,
	ElemKind,
	FieldId,
};
use crate::eval::func::Func;
use crate::driver::Recorder;
use crate::eval::lib;
use crate::eval::lib::model;
use crate::eval::locate::Location;
use crate::eval::select::Selector;
use crate::eval::styles::{
	error_hints,
	StyleChain,
};
use crate::eval::value::{
	Dict,
	Label,
	Length,
	Value,
};
use crate::eval::{
	ops,
	Context,
	Engine,
};
use crate::ir::Sp;
use crate::ledger::{
	Anchor,
	Ledger,
	Position,
};
use crate::syntax::Span;

use oxedyne_fe2o3_core::prelude::*;

use std::collections::{
	HashMap,
	HashSet,
};
use std::sync::Arc;

// The introspector

/// One located element: as realisation prepared it (location and synthesised fields set, its body
/// `Arc`s shared with the module), and where the pages placed its start tag.
#[derive(Clone, Debug)]
pub struct Record {
	pub elem:	Content,
	pub pos:	Position,
}

/// One pass's layout facts. The records are in document order, so a record's index is its order key.
#[derive(Clone, Debug, Default)]
pub struct Introspector {
	records:	Vec<Record>,
	index:		HashMap<Location, u32>,			// location to record
	kinds:		HashMap<ElemKind, Vec<u32>>,	// records by element kind, ascending
	labels:		HashMap<Label, Vec<u32>>,		// records by label, ascending
	pages:		u32,
	numberings:	Vec<(u32, Value)>,				// `page.numbering` runs: first page, value
	info:		DocInfo,						// the document's metadata as the last page's styles set it
}

impl Introspector {
	/// The document's title, authors, description and keywords, as the styles in force on its last page
	/// set them: a `set document` anywhere at the top level, or inside a template the document is shown
	/// through. The PDF's Info dictionary is written from it.
	pub fn info(&self) -> &DocInfo { &self.info }

	pub fn len(&self) -> usize { self.records.len() }

	pub fn is_empty(&self) -> bool { self.records.is_empty() }

	pub fn pages(&self) -> u32 { self.pages }

	/// The records in document order.
	pub fn iter(&self) -> impl Iterator<Item = &Record> + '_ { self.records.iter() }

	/// The located elements in document order.
	pub fn elems(&self) -> impl Iterator<Item = &Content> + '_ { self.records.iter().map(|r| &r.elem) }

	/// Where `loc` stands in document order.
	pub fn order(&self, loc: Location) -> Option<usize> { self.index.get(&loc).map(|i| *i as usize) }

	pub fn get(&self, loc: Location) -> Option<&Content> {
		self.order(loc).and_then(|i| self.records.get(i)).map(|r| &r.elem)
	}

	pub fn position(&self, loc: Location) -> Option<Position> {
		self.order(loc).and_then(|i| self.records.get(i)).map(|r| r.pos)
	}

	pub fn page(&self, loc: Location) -> Option<u32> { self.position(loc).map(|p| p.page) }

	/// The `page.numbering` in force on a page, `none` for a page the pass did not lay out.
	pub fn numbering_of_page(&self, page: u32) -> Value {
		let n = self.numberings.partition_point(|(first, _)| *first <= page);
		match n.checked_sub(1).and_then(|i| self.numberings.get(i)) {
			Some((_, v)) if page >= 1 && page <= self.pages	=> v.clone(),
			_												=> Value::None,
		}
	}

	/// How many runs of equal page numbering the pass had.
	pub fn numbering_runs(&self) -> usize { self.numberings.len() }

	/// The numbering of the page `loc` landed on, `none` where the page has none or `loc` was not placed.
	pub fn page_numbering(&self, loc: Location) -> Value {
		match self.page(loc) {
			Some(p)	=> self.numbering_of_page(p),
			None	=> Value::None,
		}
	}

	pub fn query(&self, selector: &Selector) -> Outcome<Vec<Content>> {
		let found = res!(self.query_indices(selector));
		Ok(found.into_iter().filter_map(|i| self.records.get(i)).map(|r| r.elem.clone()).collect())
	}

	/// The first element in document order the selector matches.
	pub fn query_first(&self, selector: &Selector) -> Outcome<Option<Content>> {
		let first = match selector {
			Selector::Location(l)	=> self.order(*l),
			Selector::Label(l)		=> self.labels.get(l).and_then(|v| v.first()).map(|i| *i as usize),
			other					=> res!(self.query_indices(other)).first().copied(),
		};
		Ok(first.and_then(|i| self.records.get(i)).map(|r| r.elem.clone()))
	}

	/// The document-order indices of the matches, ascending. An element selector reads only its kind's
	/// records and a label only its own; `before`, `after`, `or` and `and` are answered over the matches
	/// of their parts, which is why they never match a single element.
	pub fn query_indices(&self, selector: &Selector) -> Outcome<Vec<usize>> {
		match selector {
			Selector::Location(loc)	=> Ok(self.order(*loc).into_iter().collect()),
			Selector::Label(l)		=> Ok(self.labels.get(l).map(|v| v.iter().map(|i| *i as usize).collect())
				.unwrap_or_default()),
			Selector::Elem(kind, fields) => {
				let ids = match self.kinds.get(kind) {
					Some(v)	=> v,
					None	=> return Ok(Vec::new()),
				};
				if fields.is_none() {
					return Ok(ids.iter().map(|i| *i as usize).collect());
				}
				let mut out = Vec::new();
				for i in ids {
					if let Some(r) = self.records.get(*i as usize) {
						if res!(selector.matches(&r.elem, None)) {
							out.push(*i as usize);
						}
					}
				}
				Ok(out)
			}
			Selector::Or(ss) => {
				let mut all = Vec::new();
				for s in ss {
					all.extend(res!(self.query_indices(s)));
				}
				all.sort_unstable();
				all.dedup();
				Ok(all)
			}
			Selector::And(ss) => {
				let mut acc: Option<Vec<usize>> = None;
				for s in ss {
					let found = res!(self.query_indices(s));
					acc = Some(match acc {
						None	=> found,
						Some(a)	=> {
							let keep: HashSet<usize> = found.into_iter().collect();
							a.into_iter().filter(|i| keep.contains(i)).collect()
						}
					});
				}
				Ok(acc.unwrap_or_default())
			}
			Selector::Before { selector, end, inclusive } => {
				let mut list = res!(self.query_indices(selector));
				if let Some(bound) = res!(self.query_indices(end)).first().copied() {
					list.retain(|i| if *inclusive { *i <= bound } else { *i < bound });
				}
				Ok(list)
			}
			Selector::After { selector, start, inclusive } => {
				let mut list = res!(self.query_indices(selector));
				if let Some(bound) = res!(self.query_indices(start)).first().copied() {
					list.retain(|i| if *inclusive { *i >= bound } else { *i > bound });
				}
				Ok(list)
			}
			// Text is never located, and `check_locatable` refuses these before a query is made.
			Selector::Text(_) | Selector::Regex(_) => Ok(Vec::new()),
		}
	}

	/// How many matches sit at or before `loc`. A location the pages did not place counts every match,
	/// as Typst's does.
	pub fn count_before(&self, selector: &Selector, loc: Location) -> Outcome<usize> {
		let list = res!(self.query_indices(selector));
		Ok(match self.order(loc) {
			Some(at)	=> list.partition_point(|i| *i <= at),
			None		=> list.len(),
		})
	}

	/// The `.prl` ledger, derived at finish: every located element's `Location` anchor and the page count.
	pub fn ledger(&self) -> Ledger {
		let mut ledger = Ledger::new();
		for r in &self.records {
			if let Some(loc) = r.elem.location() {
				ledger.record(Anchor::new(loc.anchor(), r.pos));
			}
		}
		ledger.total_pages = self.pages;
		ledger
	}

	/// The bytes the introspector's own tables hold, by capacity: the records, the location, kind and
	/// label indices and the numbering runs. The elements the records point to are not counted; they are
	/// realisation's prepared elements, whose bodies the module shares.
	pub fn footprint(&self) -> usize {
		use std::mem::size_of;
		// A hash table holds its entries and one control byte each, over a power-of-two bucket count.
		fn table<K, V>(cap: usize) -> usize {
			if cap == 0 { 0 } else { (cap * 8 / 7).next_power_of_two() * (size_of::<(K, V)>() + 1) }
		}
		let mut n = self.records.capacity() * size_of::<Record>();
		n += table::<Location, u32>(self.index.capacity());
		n += table::<ElemKind, Vec<u32>>(self.kinds.capacity());
		n += self.kinds.values().map(|v| v.capacity() * size_of::<u32>()).sum::<usize>();
		n += table::<Label, Vec<u32>>(self.labels.capacity());
		n += self.labels.values().map(|v| v.capacity() * size_of::<u32>()).sum::<usize>();
		n += self.numberings.capacity() * size_of::<(u32, Value)>();
		n
	}
}

// The builder

/// Where a recorded item stands before the pass is ordered: in the order the pages placed it, or under
/// a parent, read just after the parent's own place.
#[derive(Clone, Debug)]
enum Item {
	Record(Record),
	Mark(u64),
}

/// Gathers one pass's records as the pages are placed, in the pages' order, and orders them into an
/// [`Introspector`] at the end. Out-of-flow material (a float's body, a footnote's entry) is recorded
/// with the id of its logical parent, the parent's location or a [`mark`](Self::mark), and is read where
/// that parent stands, as Typst's introspector reads an insertion (U6b's `Order`, folded in here).
#[derive(Debug, Default)]
pub struct Builder {
	items:		Vec<(Option<u64>, Item)>,	// parent, item; arrival order
	seen:		HashSet<Location>,
	pages:		u32,
	numberings:	Vec<(u32, Value)>,
	last_hash:	Option<u64>,				// the hash of the latest numbering run's value
	info:		DocInfo,
}

impl Builder {
	pub fn new() -> Self { Self::default() }

	/// Records a located element's start tag where it was placed. A location placed twice (a repeated
	/// table header) keeps its first place, as Typst's does; an element with no location records
	/// nothing.
	pub fn record(&mut self, elem: &Content, pos: Position, parent: Option<u64>) {
		let loc = match elem.location() {
			Some(l)	=> l,
			None	=> return,
		};
		if self.seen.insert(loc) {
			self.items.push((parent, Item::Record(Record { elem: elem.clone(), pos })));
		}
	}

	/// Marks where out-of-flow material with this id logically stands.
	pub fn mark(&mut self, id: u64) {
		self.items.push((None, Item::Mark(id)));
	}

	/// Takes the document metadata in force on the page just placed. Each page's chain already holds every
	/// `set document` before it, so the last page's is the document's.
	pub fn document(&mut self, info: DocInfo) {
		self.info = info;
	}

	/// Counts page `number` and its numbering. Runs of equal numberings are held once; a page given
	/// twice counts once.
	pub fn page(&mut self, number: u32, numbering: &Value) {
		if number <= self.pages {
			return;
		}
		self.pages = number;
		let mut h = Fnv::new();
		hash_value(&mut h, numbering);
		let h = h.finish();
		if self.last_hash != Some(h) {
			self.numberings.push((number, numbering.clone()));
			self.last_hash = Some(h);
		}
	}

	/// Orders the records into document order and builds the introspector's indices.
	pub fn finish(self) -> Introspector {
		let Builder { items, pages, mut numberings, info, .. } = self;
		// A parent id that nothing marks leaves its children where they were placed, as U6b's `Order` did.
		let mut anchored: HashSet<u64> = HashSet::new();
		for (_, item) in &items {
			match item {
				Item::Mark(id)		=> { anchored.insert(*id); }
				Item::Record(r)		=> if let Some(l) = r.elem.location() { anchored.insert(l.0); },
			}
		}
		let mut top: Vec<Item> = Vec::with_capacity(items.len());
		let mut under: HashMap<u64, Vec<Item>> = HashMap::new();
		let mut parents: Vec<u64> = Vec::new();	// in the order their first child arrived, for leftovers
		for (parent, item) in items {
			match parent {
				Some(p) if anchored.contains(&p) => {
					let list = under.entry(p).or_default();
					if list.is_empty() {
						parents.push(p);
					}
					list.push(item);
				}
				_ => top.push(item),
			}
		}
		let mut records = Vec::with_capacity(top.len() + under.values().map(|v| v.len()).sum::<usize>());
		for item in top {
			emit(item, &mut under, &mut records, 0);
		}
		// A parent chain that loops, or runs deeper than any real nesting, is read in arrival order.
		for p in parents {
			if let Some(list) = under.remove(&p) {
				for item in list {
					emit(item, &mut under, &mut records, 0);
				}
			}
		}
		records.shrink_to_fit();
		numberings.shrink_to_fit();
		let mut index = HashMap::with_capacity(records.len());
		let mut kinds: HashMap<ElemKind, Vec<u32>> = HashMap::new();
		let mut labels: HashMap<Label, Vec<u32>> = HashMap::new();
		for (i, r) in records.iter().enumerate() {
			let i = i as u32;
			if let Some(l) = r.elem.location() {
				index.insert(l, i);
			}
			if let Some(k) = r.elem.kind() {
				kinds.entry(k).or_default().push(i);
			}
			if let Some(l) = r.elem.label() {
				labels.entry(l.clone()).or_default().push(i);
			}
		}
		for v in kinds.values_mut() {
			v.shrink_to_fit();
		}
		for v in labels.values_mut() {
			v.shrink_to_fit();
		}
		Introspector { records, index, kinds, labels, pages, numberings, info }
	}
}

// The pages place their located elements straight into the builder.
impl Recorder for Builder {
	fn record(&mut self, elem: &Content, pos: Position, parent: Option<u64>) {
		Builder::record(self, elem, pos, parent)
	}

	fn mark(&mut self, id: u64) {
		Builder::mark(self, id)
	}
}

/// Emits an item, then everything recorded under it, depth first.
fn emit(item: Item, under: &mut HashMap<u64, Vec<Item>>, out: &mut Vec<Record>, depth: usize) {
	let id = match item {
		Item::Mark(id)	=> Some(id),
		Item::Record(r)	=> {
			let id = r.elem.location().map(|l| l.0);
			out.push(r);
			id
		}
	};
	if depth >= 64 {
		return;
	}
	if let Some(children) = id.and_then(|id| under.remove(&id)) {
		for c in children {
			emit(c, under, out, depth + 1);
		}
	}
}

// Counters

#[derive(Clone, Debug)]
pub enum CounterKey {
	Page,
	Selector(Selector),	// `counter(heading)`, `counter(figure.where(kind: table))`
	Str(String),		// `counter("mine")`
}

impl CounterKey {
	/// The key as Typst shows it in a counter update's `key` field: the element function, the string, or
	/// the selector.
	pub fn to_value(&self) -> Value {
		match self {
			CounterKey::Page							=> Value::Func(Func::Element(ElemKind::Page)),
			CounterKey::Str(s)							=> Value::str(s.clone()),
			CounterKey::Selector(Selector::Elem(k, None))	=> Value::Func(Func::Element(*k)),
			CounterKey::Selector(Selector::Label(l))	=> Value::Label(l.clone()),
			CounterKey::Selector(Selector::Location(l))	=> Value::Location(*l),
			CounterKey::Selector(s)						=> Value::Selector(Arc::new(s.clone())),
		}
	}

	/// The key a stored `key` field names; `None` for a value no counter could have been made from.
	pub fn from_value(v: &Value) -> Option<CounterKey> {
		match v {
			Value::Func(f) => match f.element() {
				Some(ElemKind::Page)	=> Some(CounterKey::Page),
				Some(k)					=> Some(CounterKey::Selector(Selector::Elem(k, None))),
				None					=> None,
			},
			Value::Str(s)		=> Some(CounterKey::Str(s.to_string())),
			Value::Label(l)		=> Some(CounterKey::Selector(Selector::Label(l.clone()))),
			Value::Location(l)	=> Some(CounterKey::Selector(Selector::Location(*l))),
			Value::Selector(s)	=> Some(CounterKey::Selector((**s).clone())),
			_					=> None,
		}
	}

	pub fn same(&self, other: &CounterKey) -> bool {
		match (self, other) {
			(CounterKey::Page, CounterKey::Page)				=> true,
			(CounterKey::Str(a), CounterKey::Str(b))			=> a == b,
			(CounterKey::Selector(a), CounterKey::Selector(b))	=> selector_eq(a, b),
			_													=> false,
		}
	}

	/// A key equal for equal counters in every pass, for the sequence memo.
	fn memo_key(&self) -> String {
		match self {
			CounterKey::Page		=> "page".to_string(),
			CounterKey::Str(s)		=> fmt!("str {}", s),
			CounterKey::Selector(s)	=> fmt!("sel {}", selector_key(s)),
		}
	}
}

#[derive(Clone, Debug)]
pub struct Counter {
	pub key:	CounterKey,
}

impl Counter {
	pub fn new(key: CounterKey) -> Self { Self { key } }

	/// The counter an element steps: `counter(heading)` for a heading.
	pub fn of(kind: ElemKind) -> Self { Self::new(CounterKey::Selector(Selector::Elem(kind, None))) }

	pub fn page() -> Self { Self::new(CounterKey::Page) }

	pub fn is_page(&self) -> bool { matches!(self.key, CounterKey::Page) }
}

/// How one element moves a counter. `Step(n)` steps level `n`.
#[derive(Clone, Debug)]
pub enum CounterUpdate {
	Set(Vec<u64>),
	Step(usize),
	Func(Func),
}

impl CounterUpdate {
	/// The update as a counter update element stores it in its `update` field.
	pub fn to_value(&self) -> Value {
		match self {
			CounterUpdate::Set(ns)	=> Value::array(ns.iter().map(|n| Value::Int(*n as i64)).collect()),
			CounterUpdate::Step(l)	=> {
				let mut d = Dict::new();
				d.insert("step", Value::Int(*l as i64));
				Value::dict(d)
			}
			CounterUpdate::Func(f)	=> Value::Func(f.clone()),
		}
	}

	pub fn from_value(v: &Value) -> Option<CounterUpdate> {
		match v {
			Value::Func(f)	=> Some(CounterUpdate::Func(f.clone())),
			Value::Dict(d)	=> match d.get("step") {
				Some(Value::Int(l)) if *l >= 1	=> Some(CounterUpdate::Step(*l as usize)),
				_								=> None,
			},
			other			=> counter_state(other).ok().map(CounterUpdate::Set),
		}
	}
}

/// Typst's `CounterState::step`: missing levels down to `level` appear as 0, deeper ones are dropped,
/// and level `level` goes up by `by`.
pub fn step(state: &mut Vec<u64>, level: usize, by: u64) {
	let level = level.max(1);
	while state.len() < level {
		state.push(0);
	}
	state.truncate(level);
	if let Some(last) = state.last_mut() {
		*last = last.saturating_add(by);
	}
}

/// A counter state from a value: an integer, or an array of them, none negative.
pub fn counter_state(v: &Value) -> Outcome<Vec<u64>> {
	match v {
		Value::Int(n) => {
			if *n < 0 {
				return Err(err!("number must be at least zero"; Input, Invalid));
			}
			Ok(vec![*n as u64])
		}
		Value::Array(a) => {
			let mut out = Vec::with_capacity(a.len());
			for x in a.iter() {
				match x {
					Value::Int(n) if *n >= 0	=> out.push(*n as u64),
					Value::Int(_)				=> return Err(err!("number must be at least zero"; Input, Invalid)),
					other						=> return Err(err!(
						"expected integer, found {}", other.ty().long_name(); Input, Invalid)),
				}
			}
			Ok(out)
		}
		other => Err(err!("expected integer or array, found {}", other.ty().long_name(); Input, Invalid)),
	}
}

pub fn state_value(ns: &[u64]) -> Value {
	Value::array(ns.iter().map(|n| Value::Int(*n as i64)).collect())
}

/// Typst's `CounterState::first`: the top-level number, one for an empty state.
fn first(ns: &[u64]) -> u64 { ns.first().copied().unwrap_or(1) }

#[derive(Clone, Debug)]
pub struct State {
	pub key:	String,
	pub init:	Value,
}

// Reads

/// A question a pass asked the introspector. Counters and states are folds over queries, so these few
/// primitives are all a pass can observe of layout.
#[derive(Clone, Debug)]
pub enum Question {
	Query(Selector),
	CountBefore(Selector, Location),
	Page(Location),
	Position(Location),
	Pages,
	PageNumbering(Location),
}

impl Question {
	/// The answer, hashed, against an introspector.
	pub fn answer(&self, intro: &Introspector) -> Outcome<u64> {
		let mut h = Fnv::new();
		match self {
			Question::Query(sel) => {
				let found = res!(intro.query_indices(sel));
				h.u64(found.len() as u64);
				for i in found {
					if let Some(r) = intro.records.get(i) {
						hash_content(&mut h, &r.elem);
					}
				}
			}
			Question::CountBefore(sel, loc) => h.u64(res!(intro.count_before(sel, *loc)) as u64),
			Question::Page(loc) => h.u64(intro.page(*loc).map(|p| p as u64 + 1).unwrap_or(0)),
			Question::Position(loc) => match intro.position(*loc) {
				Some(p) => {
					h.u64(1);
					h.u64(p.page as u64);
					h.u64(p.x.0 as u64);
					h.u64(p.y.0 as u64);
				}
				None => h.u64(0),
			},
			Question::Pages => h.u64(intro.pages as u64),
			Question::PageNumbering(loc) => hash_value(&mut h, &intro.page_numbering(*loc)),
		}
		Ok(h.finish())
	}

	/// What was asked, as a key equal for equal questions in every pass.
	pub fn key(&self) -> String {
		match self {
			Question::Query(s)				=> fmt!("query {}", selector_key(s)),
			Question::CountBefore(s, l)		=> fmt!("count {} before {:016x}", selector_key(s), l.0),
			Question::Page(l)				=> fmt!("page {:016x}", l.0),
			Question::Position(l)			=> fmt!("position {:016x}", l.0),
			Question::Pages					=> "pages".to_string(),
			Question::PageNumbering(l)		=> fmt!("page-numbering {:016x}", l.0),
		}
	}
}

/// One introspection read: what was asked, a hash of the answer, and the counter or state it was
/// asked for, which is what a non-convergence warning names.
#[derive(Clone, Debug)]
pub struct Read {
	pub key:		String,
	pub answer:		u64,
	pub question:	Question,
	pub subject:	Option<Arc<str>>,	// "value of `counter(heading)`"
}

impl PartialEq for Read {
	fn eq(&self, other: &Self) -> bool { self.key == other.key && self.answer == other.answer }
}

impl Eq for Read {}

impl std::hash::Hash for Read {
	fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
		self.key.hash(state);
		self.answer.hash(state);
	}
}

/// Every distinct read of the pass, and the counter and state sequences already folded against the
/// introspector the pass reads, so that a document displaying its heading numbers folds the heading
/// counter once, not once a heading.
#[derive(Clone, Debug, Default)]
pub struct ReadLog {
	pub all:	Vec<Read>,
	seen:		HashSet<String>,
	subject:	Option<Arc<str>>,						// the counter or state being folded
	memo_of:	usize,									// the introspector the memo was folded against
	counters:	HashMap<String, Arc<Vec<(Vec<u64>, u32)>>>,
	states:		HashMap<String, Arc<Vec<Value>>>,
}

impl ReadLog {
	fn record(&mut self, q: Question, intro: &Introspector) -> Outcome<()> {
		let key = q.key();
		if self.seen.contains(&key) {
			return Ok(());
		}
		let answer = res!(q.answer(intro));
		self.seen.insert(key.clone());
		self.all.push(Read { key, answer, question: q, subject: self.subject.clone() });
		Ok(())
	}

	/// Does every read of the pass answer the same against `intro`? The fixpoint's convergence test.
	pub fn holds(&self, intro: &Introspector) -> Outcome<bool> {
		for r in &self.all {
			if res!(r.question.answer(intro)) != r.answer {
				return Ok(false);
			}
		}
		Ok(true)
	}

	/// The reads that answer differently against `intro`, for the non-convergence warnings.
	pub fn changed(&self, intro: &Introspector) -> Outcome<Vec<&Read>> {
		let mut out = Vec::new();
		for r in &self.all {
			if res!(r.question.answer(intro)) != r.answer {
				out.push(r);
			}
		}
		Ok(out)
	}

	/// Drops the folded sequences when the engine now reads another introspector.
	fn validate_memo(&mut self, intro: &Arc<Introspector>) {
		let id = Arc::as_ptr(intro) as usize;
		if self.memo_of != id {
			self.counters.clear();
			self.states.clear();
			self.memo_of = id;
		}
	}
}

fn ask(engine: &mut Engine, q: Question) -> Outcome<()> {
	let intro = engine.intro.clone();
	engine.reads.record(q, &intro)
}

/// Runs `f` with the reads it makes attributed to `subject`.
fn attributed<T, F: FnOnce(&mut Engine) -> Outcome<T>>(engine: &mut Engine, subject: Arc<str>, f: F) -> Outcome<T> {
	let outer = engine.reads.subject.replace(subject);
	let out = f(engine);
	engine.reads.subject = outer;
	out
}

fn counter_subject(counter: &Counter) -> Arc<str> {
	match &counter.key {
		CounterKey::Page	=> Arc::from("value of the page counter"),
		key					=> Arc::from(fmt!("value of `counter({})`", lib::foundations::repr(&key.to_value()))),
	}
}

fn state_subject(state: &State) -> Arc<str> {
	Arc::from(fmt!("value of `state({})`", lib::foundations::repr(&Value::str(state.key.clone()))))
}

/// What a read that did not settle is reported as, in Typst's words: the counter or state it was made
/// for, else the query or the location it asked about.
pub fn describe(read: &Read, intro: &Introspector) -> String {
	if let Some(s) = &read.subject {
		return fmt!("{} did not converge", s);
	}
	let kind = |loc: &Location| intro.get(*loc).map(|c| c.func_kind().name()).unwrap_or("element");
	match &read.question {
		Question::Query(sel) | Question::CountBefore(sel, _) =>
			fmt!("query for {} did not stabilize", format_selector(sel, "elements")),
		Question::Page(l)			=> fmt!("page number of the {} did not stabilize", kind(l)),
		Question::Position(l)		=> fmt!("{} position did not stabilize", kind(l)),
		Question::PageNumbering(l)	=> fmt!("numbering of the page on which the {} is located did not stabilize",
			kind(l)),
		Question::Pages				=> "page count did not stabilize".to_string(),
	}
}

/// A selector as Typst names what it matches in a warning.
fn format_selector(sel: &Selector, what: &str) -> String {
	match sel {
		Selector::Elem(k, None)	=> fmt!("{} {}", k.name(), what),
		Selector::Elem(k, _)	=> fmt!("matching {} {}", k.name(), what),
		Selector::Label(l)		=> fmt!("{} labelled `<{}>`", what, l.as_str()),
		other					=> fmt!("{} matching `{}`",
			what, lib::foundations::repr(&Value::Selector(Arc::new(other.clone())))),
	}
}

// Recorded introspection: what library code calls, so that every answer reaches the fixpoint.

/// Every element the selector matches, in document order, from the previous pass.
/// The document metadata a style chain carries: `set document(title:, author:, description:, keywords:)`,
/// Typst's own defaults where unset. Content is read as its plain text, an array of strings joined with
/// ", " for the one Info entry.
pub fn document_info(styles: &StyleChain) -> Outcome<DocInfo> {
	let plain = |v: Value| -> Option<String> {
		match v {
			Value::None			=> None,
			Value::Content(c)	=> Some(c.plain_text()),
			Value::Str(s)		=> Some(s.to_string()),
			_					=> None,
		}
	};
	let joined = |v: Value| -> Option<String> {
		match v {
			Value::Array(a)	=> {
				let parts: Vec<String> = a.iter().filter_map(|x| match x {
					Value::Str(s)	=> Some(s.to_string()),
					_				=> None,
				}).collect();
				if parts.is_empty() { None } else { Some(parts.join(", ")) }
			}
			_	=> None,
		}
	};
	Ok(DocInfo {
		title:		plain(res!(model::common::style(styles, ElemKind::Document, "title"))),
		author:		joined(res!(model::common::style(styles, ElemKind::Document, "author"))),
		subject:	plain(res!(model::common::style(styles, ElemKind::Document, "description"))),
		keywords:	joined(res!(model::common::style(styles, ElemKind::Document, "keywords"))),
	})
}

pub fn query(engine: &mut Engine, selector: &Selector) -> Outcome<Vec<Content>> {
	res!(ask(engine, Question::Query(selector.clone())));
	let intro = engine.intro.clone();
	intro.query(selector)
}

/// The one element carrying `label`, or Typst's error for none and for several. Answered without a
/// context, as an element's own show asks it (`ref`, `link`).
pub fn query_label(engine: &mut Engine, label: &Label, span: Span) -> Outcome<Content> {
	match res!(query_label_opt(engine, label, span)) {
		Some(c)	=> Ok(c),
		None	=> Err(engine.error(DiagnosticKind::Type, span, fmt!("label `<{}>` does not exist in the document", label.as_str()))),
	}
}

/// As [`query_label`], but a label absent from the document is `None`.
pub fn query_label_opt(engine: &mut Engine, label: &Label, span: Span) -> Outcome<Option<Content>> {
	let mut found = res!(query(engine, &Selector::Label(label.clone())));
	match found.len() {
		0	=> Ok(None),
		1	=> Ok(found.pop()),
		_	=> Err(engine.error(DiagnosticKind::Type, span, fmt!("label `<{}>` occurs multiple times in the document", label.as_str()))),
	}
}

pub fn position(engine: &mut Engine, loc: Location) -> Outcome<Option<Position>> {
	res!(ask(engine, Question::Position(loc)));
	Ok(engine.intro.position(loc))
}

/// The page `loc` landed on. Asked apart from its position, it settles as soon as the page does.
pub fn page(engine: &mut Engine, loc: Location) -> Outcome<Option<u32>> {
	res!(ask(engine, Question::Page(loc)));
	Ok(engine.intro.page(loc))
}

pub fn pages(engine: &mut Engine) -> Outcome<u32> {
	res!(ask(engine, Question::Pages));
	Ok(engine.intro.pages)
}

pub fn page_numbering(engine: &mut Engine, loc: Location) -> Outcome<Value> {
	res!(ask(engine, Question::PageNumbering(loc)));
	Ok(engine.intro.page_numbering(loc))
}

fn count_before(engine: &mut Engine, selector: &Selector, loc: Location) -> Outcome<usize> {
	res!(ask(engine, Question::CountBefore(selector.clone(), loc)));
	let intro = engine.intro.clone();
	intro.count_before(selector, loc)
}

/// The error Typst gives a context-only function called outside `context`.
pub fn no_context(engine: &mut Engine, span: Span) -> Error<ErrTag> {
	error_hints(engine, DiagnosticKind::Type, span, "can only be used when context is known", &[
		"try wrapping this in a `context` expression",
		"the `context` expression should wrap everything that depends on this function",
	])
}

/// `here()`: the location of the context in force, or the no-context error.
pub fn here(engine: &mut Engine, span: Span) -> Outcome<Location> {
	match engine.context.location {
		Some(l)	=> Ok(l),
		None	=> Err(no_context(engine, span)),
	}
}

/// The styles of the context in force, or the no-context error.
pub fn context_styles(engine: &mut Engine, span: Span) -> Outcome<StyleChain> {
	match &engine.context.styles {
		Some(s)	=> Ok(s.clone()),
		None	=> Err(no_context(engine, span)),
	}
}

/// Fails unless the selector can only match locatable elements, with Typst's message.
pub fn check_locatable(engine: &mut Engine, span: Span, selector: &Selector) -> Outcome<()> {
	match selector {
		Selector::Elem(k, _) if !k.locatable() => Err(engine.error(DiagnosticKind::Type, span, fmt!("{} is not locatable", k.name()))),
		Selector::Text(_) | Selector::Regex(_) => Err(engine.error(DiagnosticKind::Type, span, "text is not locatable")),
		Selector::Or(ss) | Selector::And(ss) => {
			for s in ss {
				res!(check_locatable(engine, span, s));
			}
			Ok(())
		}
		Selector::Before { selector, end, .. } => {
			res!(check_locatable(engine, span, selector));
			check_locatable(engine, span, end)
		}
		Selector::After { selector, start, .. } => {
			res!(check_locatable(engine, span, selector));
			check_locatable(engine, span, start)
		}
		_ => Ok(()),
	}
}

/// The one location a selector names, as `locate` and `counter.at` resolve it: a location stands for
/// itself; anything else needs a context, then must match exactly one element.
pub fn resolve_unique(engine: &mut Engine, span: Span, selector: &Selector) -> Outcome<Location> {
	if let Selector::Location(l) = selector {
		return Ok(*l);
	}
	if engine.context.location.is_none() {
		return Err(no_context(engine, span));
	}
	unique(engine, span, selector)
}

/// As [`resolve_unique`] without the context check, for an element's own show: a label must occur
/// exactly once; any other selector must match exactly one element.
pub fn unique(engine: &mut Engine, span: Span, selector: &Selector) -> Outcome<Location> {
	if let Selector::Location(l) = selector {
		return Ok(*l);
	}
	let found = res!(query(engine, selector));
	match (selector, found.len()) {
		(_, 1) => match found[0].location() {
			Some(l)	=> Ok(l),
			None	=> Err(err!("A queried element has no location."; Bug)),
		},
		(Selector::Label(l), 0) => Err(engine.error(DiagnosticKind::Type, span, fmt!(
			"label `<{}>` does not exist in the document", l.as_str()))),
		(Selector::Label(l), _) => Err(engine.error(DiagnosticKind::Type, span, fmt!(
			"label `<{}>` occurs multiple times in the document", l.as_str()))),
		(_, 0)	=> Err(engine.error(DiagnosticKind::Type, span, "selector does not match any element")),
		_		=> Err(engine.error(DiagnosticKind::Type, span, "selector matches multiple elements")),
	}
}

/// The update an element makes to the counters that select it, Typst's `Count`: a counter update
/// applies itself, the model's counted elements step as `model::count_step` says (a numbered heading at
/// its level, a numbered figure or block equation and a footnote that is not a reference once), and any
/// other element a counter selects steps once.
pub fn element_update(elem: &Content) -> Option<CounterUpdate> {
	match elem.kind() {
		Some(ElemKind::CounterUpdate)	=> elem.field("update").and_then(CounterUpdate::from_value),
		Some(ElemKind::Heading) | Some(ElemKind::Figure) | Some(ElemKind::Equation) | Some(ElemKind::Footnote)
			=> model::count_step(elem).map(CounterUpdate::Step),
		_								=> Some(CounterUpdate::Step(1)),
	}
}

/// The selector a counter's sequence is queried by: every counter update, or the key's own selector as
/// well. Updates of other counters are filtered out by key afterwards.
fn counter_selector(counter: &Counter) -> Selector {
	let updates = Selector::Elem(ElemKind::CounterUpdate, None);
	match &counter.key {
		CounterKey::Selector(s)	=> Selector::Or(vec![updates, s.clone()]),
		_						=> updates,
	}
}

/// Every state the counter passes through, with the page it was on, and the document order of each
/// element that moved it: the initial state, then one after each element. Folded once per pass.
fn counter_sequence(engine: &mut Engine, counter: &Counter) -> Outcome<Arc<Vec<(Vec<u64>, u32)>>> {
	let sel = counter_selector(counter);
	let found = res!(query(engine, &sel));
	let intro = engine.intro.clone();
	engine.reads.validate_memo(&intro);
	let key = counter.key.memo_key();
	if let Some(seq) = engine.reads.counters.get(&key) {
		return Ok(seq.clone());
	}
	let mut state = vec![if counter.is_page() { 1 } else { 0 }];
	let mut pg = 1u32;
	let mut stops = Vec::with_capacity(found.len() + 1);
	stops.push((state.clone(), pg));
	for e in &found {
		if e.is(ElemKind::CounterUpdate) {
			let key = e.field("key").and_then(CounterKey::from_value);
			if !key.map(|k| k.same(&counter.key)).unwrap_or(false) {
				continue;
			}
		}
		if counter.is_page() {
			if let Some(loc) = e.location() {
				let at = res!(page(engine, loc)).unwrap_or(1);
				if at > pg {
					step(&mut state, 1, (at - pg) as u64);
				}
				pg = at;
			}
		}
		if let Some(update) = element_update(e) {
			state = res!(apply_update(engine, e.span(), &state, update));
		}
		stops.push((state.clone(), pg));
	}
	let seq = Arc::new(stops);
	engine.reads.counters.insert(key, seq.clone());
	Ok(seq)
}

fn apply_update(engine: &mut Engine, span: Span, state: &[u64], update: CounterUpdate) -> Outcome<Vec<u64>> {
	match update {
		CounterUpdate::Set(ns)	=> Ok(ns),
		CounterUpdate::Step(l)	=> {
			let mut s = state.to_vec();
			step(&mut s, l, 1);
			Ok(s)
		}
		CounterUpdate::Func(f)	=> {
			// A closure's own span, where Typst reports an argument it cannot take.
			let span = match &f {
				Func::Closure(c) if !c.span.is_detached()	=> c.span,
				_											=> span,
			};
			let mut args = Args::new(span);
			for n in state {
				args.push(span, Value::Int(*n as i64));
			}
			// An update function runs without a context, as Typst calls it.
			let saved = std::mem::take(&mut engine.context);
			let v = engine.call_func(&f, args);
			engine.context = saved;
			match counter_state(&res!(v)) {
				Ok(s)	=> Ok(s),
				Err(e)	=> Err(engine.error(DiagnosticKind::Type, span, e.plain())),
			}
		}
	}
}

/// The stop of a sequence folded over the matches of `sel` that sits at `loc`: after every match at or
/// before it, or after all of them when the pages did not place `loc`. Only the updates of this counter
/// or state made stops, so the matches of other keys are skipped by recounting in document order.
fn stop_at<T: Clone, F: Fn(&Content) -> bool>(
	engine:	&mut Engine,
	sel:	&Selector,
	stops:	&[T],
	keep:	F,
	loc:	Location,
)
	-> Outcome<T>
{
	res!(count_before(engine, sel, loc));
	let intro = engine.intro.clone();
	let at = intro.order(loc);
	let mut n = 0;
	for i in res!(intro.query_indices(sel)) {
		if at.map(|a| i > a).unwrap_or(false) {
			break;
		}
		if let Some(r) = intro.records.get(i) {
			if keep(&r.elem) {
				n += 1;
			}
		}
	}
	match stops.get(n).or_else(|| stops.last()) {
		Some(s)	=> Ok(s.clone()),
		None	=> Err(err!("A sequence has no initial stop."; Bug)),
	}
}

/// Does `e` move `counter`? Its own updates do, and the elements its selector matches.
fn moves(counter: &Counter, e: &Content) -> bool {
	if e.is(ElemKind::CounterUpdate) {
		return e.field("key").and_then(CounterKey::from_value).map(|k| k.same(&counter.key)).unwrap_or(false);
	}
	true
}

/// The counter's value at a location: after every update at or before it.
pub fn counter_at(engine: &mut Engine, counter: &Counter, loc: Location) -> Outcome<Vec<u64>> {
	attributed(engine, counter_subject(counter), |engine| {
		let seq = res!(counter_sequence(engine, counter));
		let sel = counter_selector(counter);
		let (mut state, pg) = res!(stop_at(engine, &sel, &seq, |e| moves(counter, e), loc));
		if counter.is_page() {
			let at = res!(page(engine, loc)).unwrap_or(1);
			if at > pg {
				step(&mut state, 1, (at - pg) as u64);
			}
		}
		Ok(state)
	})
}

/// The counter's value at the end of the document.
pub fn counter_final(engine: &mut Engine, counter: &Counter) -> Outcome<Vec<u64>> {
	attributed(engine, counter_subject(counter), |engine| {
		let seq = res!(counter_sequence(engine, counter));
		let (mut state, pg) = match seq.last() {
			Some(s)	=> s.clone(),
			None	=> return Err(err!("A counter sequence has no initial state."; Bug)),
		};
		if counter.is_page() {
			let total = res!(pages(engine)).max(1);
			if total > pg {
				step(&mut state, 1, (total - pg) as u64);
			}
		}
		Ok(state)
	})
}

/// The top-level number at `loc` and at the end, together: what `display(both: true)` formats.
pub fn counter_both(engine: &mut Engine, counter: &Counter, loc: Location) -> Outcome<Vec<u64>> {
	let at = res!(counter_at(engine, counter, loc));
	let fin = res!(counter_final(engine, counter));
	Ok(vec![first(&at), first(&fin)])
}

/// The numbering a counter displays with when none is given, Typst's `matching_numbering`: the page's
/// for the page counter; for a counted element's counter, that of the element at `loc` when it is one,
/// else the style chain's.
fn matching_numbering(
	engine:		&mut Engine,
	counter:	&Counter,
	loc:		Location,
	styles:		Option<&StyleChain>,
)
	-> Outcome<Option<Value>>
{
	let kind = match &counter.key {
		CounterKey::Page							=> return page_numbering(engine, loc).map(|v| match v {
			Value::None	=> None,
			v			=> Some(v),
		}),
		CounterKey::Selector(Selector::Elem(k, _))	=> *k,
		_											=> return Ok(None),
	};
	if !matches!(kind, ElemKind::Heading | ElemKind::Figure | ElemKind::Equation | ElemKind::Footnote) {
		return Ok(None);
	}
	let here_elem = res!(query(engine, &Selector::Location(loc)));
	if let Some(e) = here_elem.first().filter(|e| e.is(kind)) {
		match e.field("numbering") {
			None | Some(Value::None) | Some(Value::Auto)	=> (),
			Some(n)											=> return Ok(Some(n.clone())),
		}
	}
	let chain = match styles {
		Some(c)	=> c,
		None	=> return Ok(None),
	};
	match kind.field_id("numbering") {
		Some(id) => Ok(match res!(chain.get(kind, id)) {
			None | Some(Value::None) | Some(Value::Auto)	=> None,
			Some(n)											=> Some(n),
		}),
		None => Ok(None),
	}
}

/// `counter.display()`: the counter at `loc` (both numbers with `both`) formatted with `numbering`, or
/// with the numbering the counted element or the page has, falling back to `"1.1"`. The numbering runs
/// in a context at `loc`.
pub fn counter_display(
	engine:		&mut Engine,
	counter:	&Counter,
	numbering:	Option<Value>,
	both:		bool,
	loc:		Location,
	styles:		Option<&StyleChain>,
)
	-> Outcome<Value>
{
	let state = if both {
		res!(counter_both(engine, counter, loc))
	} else {
		res!(counter_at(engine, counter, loc))
	};
	let numbering = match numbering {
		Some(n)	=> n,
		None	=> res!(matching_numbering(engine, counter, loc, styles)).unwrap_or_else(|| Value::str("1.1")),
	};
	let saved = std::mem::replace(&mut engine.context, Context {
		location:	Some(loc),
		styles:		styles.cloned(),
	});
	let out = lib::numbering::apply(engine, &numbering, &state);
	engine.context = saved;
	out
}

/// Typst's `Counter::display_at`: an element's counter at its location with a numbering, as a heading,
/// figure, equation or footnote shows its own number and a reference shows its target's. Recorded, so
/// the fixpoint runs again when a number moves.
pub fn display_at(
	engine:		&mut Engine,
	counter:	&Counter,
	loc:		Location,
	numbering:	&Value,
	styles:		Option<&StyleChain>,
)
	-> Outcome<Value>
{
	counter_display(engine, counter, Some(numbering.clone()), false, loc, styles)
}

// States

fn state_selector() -> Selector { Selector::Elem(ElemKind::StateUpdate, None) }

fn updates(state: &State, e: &Content) -> bool {
	matches!(e.field("key"), Some(Value::Str(k)) if **k == state.key)
}

/// Every value the state passes through: the initial one, then one after each of its updates. Folded
/// once per pass.
fn state_sequence(engine: &mut Engine, state: &State) -> Outcome<Arc<Vec<Value>>> {
	let found = res!(query(engine, &state_selector()));
	let intro = engine.intro.clone();
	engine.reads.validate_memo(&intro);
	let mut h = Fnv::new();
	h.str(&state.key);
	hash_value(&mut h, &state.init);
	let key = fmt!("{:016x}", h.finish());
	if let Some(seq) = engine.reads.states.get(&key) {
		return Ok(seq.clone());
	}
	let mut v = state.init.clone();
	let mut stops = Vec::with_capacity(found.len() + 1);
	stops.push(v.clone());
	for e in found.iter().filter(|e| updates(state, e)) {
		v = match e.field("update") {
			Some(Value::Func(f)) => {
				let f = f.clone();
				let mut args = Args::new(e.span());
				args.push(e.span(), v);
				// An update function runs without a context, as Typst calls it.
				let saved = std::mem::take(&mut engine.context);
				let out = engine.call_func(&f, args);
				engine.context = saved;
				res!(out)
			}
			Some(x)	=> x.clone(),
			None	=> v,
		};
		stops.push(v.clone());
	}
	let seq = Arc::new(stops);
	engine.reads.states.insert(key, seq.clone());
	Ok(seq)
}

pub fn state_at(engine: &mut Engine, state: &State, loc: Location) -> Outcome<Value> {
	attributed(engine, state_subject(state), |engine| {
		let seq = res!(state_sequence(engine, state));
		stop_at(engine, &state_selector(), &seq, |e| updates(state, e), loc)
	})
}

pub fn state_final(engine: &mut Engine, state: &State) -> Outcome<Value> {
	attributed(engine, state_subject(state), |engine| {
		let seq = res!(state_sequence(engine, state));
		Ok(seq.last().cloned().unwrap_or(Value::None))
	})
}

// Equality and hashing that are stable from pass to pass

/// Structural selector equality, for matching a counter update's key with its counter.
pub fn selector_eq(a: &Selector, b: &Selector) -> bool {
	match (a, b) {
		(Selector::Elem(ka, fa), Selector::Elem(kb, fb)) => ka == kb && match (fa, fb) {
			(None, None)			=> true,
			(Some(x), Some(y))		=> x.len() == y.len() && x.iter().all(|(id, v)|
				y.iter().any(|(id2, w)| id == id2 && ops::equal(v, w))),
			_						=> false,
		},
		(Selector::Label(x), Selector::Label(y))		=> x == y,
		(Selector::Text(x), Selector::Text(y))			=> x == y,
		(Selector::Regex(x), Selector::Regex(y))		=> x.pattern == y.pattern,
		(Selector::Location(x), Selector::Location(y))	=> x == y,
		(Selector::Or(x), Selector::Or(y)) | (Selector::And(x), Selector::And(y)) =>
			x.len() == y.len() && x.iter().zip(y.iter()).all(|(p, q)| selector_eq(p, q)),
		(Selector::Before { selector: s1, end: e1, inclusive: i1 },
			Selector::Before { selector: s2, end: e2, inclusive: i2 }) =>
			i1 == i2 && selector_eq(s1, s2) && selector_eq(e1, e2),
		(Selector::After { selector: s1, start: e1, inclusive: i1 },
			Selector::After { selector: s2, start: e2, inclusive: i2 }) =>
			i1 == i2 && selector_eq(s1, s2) && selector_eq(e1, e2),
		_ => false,
	}
}

/// A key for a selector that is equal for equal selectors in every pass.
fn selector_key(s: &Selector) -> String {
	let mut h = Fnv::new();
	hash_selector(&mut h, s);
	fmt!("{:016x}", h.finish())
}

/// FNV-1a, deterministic across runs, unlike the standard hasher.
pub struct Fnv(u64);

impl Fnv {
	pub fn new() -> Self { Fnv(0xcbf2_9ce4_8422_2325) }

	pub fn bytes(&mut self, bs: &[u8]) {
		for b in bs {
			self.0 ^= *b as u64;
			self.0 = self.0.wrapping_mul(0x0000_0100_0000_01b3);
		}
	}

	pub fn u64(&mut self, n: u64) { self.bytes(&n.to_le_bytes()); }

	pub fn str(&mut self, s: &str) {
		self.u64(s.len() as u64);
		self.bytes(s.as_bytes());
	}

	pub fn finish(&self) -> u64 { self.0 }
}

impl Default for Fnv {
	fn default() -> Self { Self::new() }
}

fn hash_selector(h: &mut Fnv, s: &Selector) {
	match s {
		Selector::Elem(k, fields) => {
			h.u64(1);
			h.u64(*k as u64);
			if let Some(fs) = fields {
				for (id, v) in fs {
					h.u64(id.0 as u64);
					hash_value(h, v);
				}
			}
		}
		Selector::Label(l)		=> { h.u64(2); h.str(l.as_str()); }
		Selector::Text(t)		=> { h.u64(3); h.str(t); }
		Selector::Regex(r)		=> { h.u64(4); h.str(&r.pattern); }
		Selector::Location(l)	=> { h.u64(5); h.u64(l.0); }
		Selector::Or(ss)		=> { h.u64(6); for x in ss { hash_selector(h, x); } }
		Selector::And(ss)		=> { h.u64(7); for x in ss { hash_selector(h, x); } }
		Selector::Before { selector, end, inclusive } => {
			h.u64(8);
			hash_selector(h, selector);
			hash_selector(h, end);
			h.u64(*inclusive as u64);
		}
		Selector::After { selector, start, inclusive } => {
			h.u64(9);
			hash_selector(h, selector);
			hash_selector(h, start);
			h.u64(*inclusive as u64);
		}
	}
}

/// Hashes content by what a reader of a query result can observe: kinds, fields, labels and locations.
pub fn hash_content(h: &mut Fnv, c: &Content) {
	match c {
		Content::Elem(e) => {
			h.u64(10);
			h.u64(e.kind as u64);
			if let Some(l) = e.location {
				h.u64(l.0);
			}
			if let Some(l) = &e.label {
				h.str(l.as_str());
			}
			let mut fields: Vec<&(FieldId, Value)> = e.fields.iter().collect();
			fields.sort_by_key(|(id, _)| id.0);
			for (id, v) in fields {
				h.u64(id.0 as u64);
				hash_value(h, v);
			}
		}
		Content::Sequence(s) => {
			h.u64(11);
			if let Some(l) = s.location {
				h.u64(l.0);
			}
			if let Some(l) = &s.label {
				h.str(l.as_str());
			}
			h.u64(s.children.len() as u64);
			for k in &s.children {
				hash_content(h, k);
			}
		}
		Content::Styled(s) => {
			h.u64(12);
			h.u64(s.styles.len() as u64);
			hash_content(h, &s.child);
		}
	}
}

/// Hashes a value deterministically. Maps are hashed in their insertion order, functions by name and
/// definition site, and plain data by its debug form, which holds no map.
pub fn hash_value(h: &mut Fnv, v: &Value) {
	match v {
		Value::Content(c)	=> hash_content(h, c),
		Value::Array(a)		=> {
			h.u64(20);
			h.u64(a.len() as u64);
			for x in a.iter() {
				hash_value(h, x);
			}
		}
		Value::Dict(d)		=> {
			h.u64(21);
			for (k, x) in d.iter() {
				h.str(k);
				hash_value(h, x);
			}
		}
		Value::Func(f)		=> {
			h.u64(22);
			h.str(f.name().unwrap_or(""));
			if let Func::Closure(c) = f {
				h.u64(c.span.file.0 as u64);
				h.u64(c.span.start as u64);
			}
		}
		Value::Module(m)	=> { h.u64(23); h.str(&m.name); }
		Value::Styles(s)	=> { h.u64(24); h.u64(s.len() as u64); }
		Value::Args(a)		=> {
			h.u64(25);
			for arg in a.items.iter() {
				h.str(arg.name.as_deref().unwrap_or(""));
				hash_value(h, &arg.value);
			}
		}
		Value::Selector(s)	=> hash_selector(h, s),
		Value::Location(l)	=> { h.u64(26); h.u64(l.0); }
		Value::Str(s)		=> { h.u64(27); h.str(s); }
		Value::Label(l)		=> { h.u64(28); h.str(l.as_str()); }
		Value::Counter(c)	=> { h.u64(29); hash_value(h, &c.key.to_value()); }
		Value::State(s)		=> { h.u64(30); h.str(&s.key); hash_value(h, &s.init); }
		other				=> h.str(&fmt!("{:?}", other)),
	}
}

/// `location.position()`: the page and the top-left point, as a dictionary.
pub fn position_dict(p: Position) -> Value {
	let mut d = Dict::new();
	d.insert("page", Value::Int(p.page as i64));
	d.insert("x", Value::Length(Length::pt(p.x.to_pt())));
	d.insert("y", Value::Length(Length::pt(p.y.to_pt())));
	Value::dict(d)
}

/// The space `measure` offers when a dimension is `auto`: as large as a length can be.
pub const UNBOUNDED: Sp = Sp(i32::MAX);

/// Runs `f` as `measure` runs its layout: under a fresh root locator, as Typst's `measure` lays out under
/// `Locator::root()`, so what it realises takes none of the document's ordinals, and the locator in force
/// is put back whatever `f` returns. Nothing a measured layout places is recorded: the builder is reached
/// only by `driver::place_page`, and a measured layout never places a page, so there is none to detach.
pub fn detached<T, F: FnOnce(&mut Engine) -> Outcome<T>>(engine: &mut Engine, f: F) -> Outcome<T> {
	let outer = std::mem::take(&mut engine.locator);
	let out = f(engine);
	engine.locator = outer;
	out
}
