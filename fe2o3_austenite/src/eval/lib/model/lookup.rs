// U5 owns this file: what the model's shows ask of the previous pass -- the element a label names, a
// counter's numbers at a location, a query's matches. Every such question goes through here, so when U8's
// introspector lands (`Introspector::query`, `Counter::at`) only this file changes.
//
// Until then a label and an element counter are answered from `Introspector::elems`, the U0 table of
// realised locatable elements in document order: a label by scanning it, a counter by folding the steps
// the elements themselves declare (`model::count_step`, Typst's `Count`). `counter.update` elements and
// the page counter need U8 and are refused with `Unimplemented`; general queries go to
// `Introspector::query` as they stand.

use crate::eval::content::{
	Content,
	ElemKind,
};
use crate::eval::intro::{
	Counter,
	CounterKey,
};
use crate::eval::lib::model;
use crate::eval::lib::numbering;
use crate::eval::locate::Location;
use crate::eval::select::Selector;
use crate::eval::value::{
	Label,
	Value,
};
use crate::eval::Engine;
use crate::syntax::Span;

use oxedyne_fe2o3_core::prelude::*;

/// The one element carrying `label`, with Typst's errors for none and for several.
pub fn label(engine: &mut Engine, label: &Label, span: Span) -> Outcome<Content> {
	match res!(label_opt(engine, label, span)) {
		Some(c)	=> Ok(c),
		None	=> Err(engine.error(span, fmt!("label `<{}>` does not exist in the document", label.as_str()))),
	}
}

/// As [`label`], but a label absent from the document is `None`, not an error.
pub fn label_opt(engine: &mut Engine, label: &Label, span: Span) -> Outcome<Option<Content>> {
	let mut found: Vec<Content> = engine.intro.elems.iter()
		.filter(|e| e.label() == Some(label))
		.cloned()
		.collect();
	match found.len() {
		0	=> Ok(None),
		1	=> Ok(found.pop()),
		_	=> Err(engine.error(span, fmt!("label `<{}>` occurs multiple times in the document",
			label.as_str()))),
	}
}

/// The elements a selector matches, in document order.
pub fn query(engine: &mut Engine, selector: &Selector, span: Span) -> Outcome<Vec<Content>> {
	match engine.intro.query(selector) {
		Ok(v)	=> Ok(v),
		Err(e)	=> Err(engine.adopt(engine.diags.len(), span, e)),
	}
}

/// The element at a location, if the previous pass saw it.
pub fn at(engine: &Engine, loc: Location) -> Option<Content> { engine.intro.get(loc).cloned() }

/// A counter's numbers at `loc`: its initial state stepped by every counting element up to and
/// including the one at `loc`. A location the previous pass did not see reads the initial state, as
/// Typst's first pass does.
pub fn counter_at(engine: &mut Engine, counter: &Counter, loc: Location, span: Span) -> Outcome<Vec<u64>> {
	let sel = match &counter.key {
		CounterKey::Selector(s)	=> s.clone(),
		CounterKey::Page		=> return Err(engine.adopt(engine.diags.len(), span,
			err!("counter(page) needs U8's introspector"; Unimplemented))),
		CounterKey::Str(_)		=> return Err(engine.adopt(engine.diags.len(), span,
			err!("a named counter needs U8's introspector"; Unimplemented))),
	};
	let mut nums: Vec<u64> = vec![0];
	if !engine.intro.index.contains_key(&loc) {
		return Ok(nums);
	}
	let intro = engine.intro.clone();
	for e in intro.elems.iter() {
		if e.is(ElemKind::CounterUpdate) {
			return Err(engine.adopt(engine.diags.len(), span,
				err!("counter updates need U8's introspector"; Unimplemented)));
		}
		if res!(sel.matches(e, None)) {
			if let Some(level) = model::count_step(e) {
				step(&mut nums, level);
			}
		}
		if e.location() == Some(loc) {
			break;
		}
	}
	Ok(nums)
}

/// Typst's `CounterState::step`: pad to the level with zeros, add one there, drop deeper levels.
pub fn step(nums: &mut Vec<u64>, level: usize) {
	let level = level.max(1);
	while nums.len() < level {
		nums.push(0);
	}
	nums[level - 1] = nums[level - 1].saturating_add(1);
	nums.truncate(level);
}

/// A counter at `loc` displayed with `numbering` (a pattern string or a function).
pub fn display_counter(
	engine:		&mut Engine,
	counter:	&Counter,
	loc:		Location,
	numbering:	&Value,
	span:		Span,
)
	-> Outcome<Content>
{
	let nums = res!(counter_at(engine, counter, loc, span));
	let v = res!(numbering::apply(engine, numbering, &nums));
	super::common::shown(engine, v, span)
}

/// The counter an element kind counts on: `counter(heading)`.
pub fn elem_counter(kind: ElemKind) -> Counter {
	Counter { key: CounterKey::Selector(Selector::Elem(kind, None)) }
}

/// Does a bibliography in the document hold `key`?
pub fn bib_has(engine: &Engine, key: &Label) -> bool {
	engine.intro.elems.iter()
		.filter(|e| e.is(ElemKind::Bibliography))
		.any(|b| model::bibliography::has_key(b, key))
}
