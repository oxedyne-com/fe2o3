// U8 owns this file: the introspector, counter and state keys, and the read log the fixpoint compares.
// A counter or state is a fold of its update elements in document order; nothing here observes layout
// except through `positions`, which the ledger fills.

use crate::eval::content::Content;
use crate::eval::func::unimplemented;
use crate::eval::locate::Location;
use crate::eval::select::Selector;
use crate::eval::value::Value;
use crate::ledger::Position;

use oxedyne_fe2o3_core::prelude::*;

use std::collections::HashMap;

/// One pass's layout facts: the locatable elements in document order and where each landed.
#[derive(Clone, Debug, Default)]
pub struct Introspector {
	pub elems:		Vec<Content>,					// realised locatable elements, document order
	pub index:		HashMap<Location, usize>,		// location to its position in `elems`
	pub positions:	HashMap<Location, Position>,	// from the pass's ledger
	pub pages:		u32,
	pub numberings:	Vec<Value>,						// each page's `page.numbering`, for `counter(page).display()`
}

impl Introspector {
	pub fn query(&self, _selector: &Selector) -> Outcome<Vec<Content>> {
		Err(unimplemented("introspector", "query"))
	}

	pub fn position(&self, loc: Location) -> Option<Position> { self.positions.get(&loc).copied() }

	pub fn page(&self, loc: Location) -> Option<u32> { self.position(loc).map(|p| p.page) }

	pub fn get(&self, loc: Location) -> Option<&Content> {
		self.index.get(&loc).and_then(|i| self.elems.get(*i))
	}
}

#[derive(Clone, Debug)]
pub enum CounterKey {
	Page,
	Selector(Selector),	// `counter(heading)`, `counter(figure.where(kind: table))`
	Str(String),		// `counter("mine")`
}

#[derive(Clone, Debug)]
pub struct Counter {
	pub key:	CounterKey,
}

#[derive(Clone, Debug)]
pub struct State {
	pub key:	String,
	pub init:	Value,
}

/// One introspection read made while a `context` body ran: what was asked (as a stable key) and a hash
/// of the answer. The fixpoint re-runs the body only when some answer changed.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Read {
	pub key:	String,
	pub answer:	u64,
}

/// The reads of the context calls in progress, innermost last.
#[derive(Clone, Debug, Default)]
pub struct ReadLog {
	pub stack:	Vec<Vec<Read>>,
}
