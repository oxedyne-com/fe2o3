// U5 owns this file: what the model's shows ask of the previous pass -- the element a label names, a
// counter's numbers at a location, a query's matches. Every such question goes through here, and here
// through U8's recorded introspection (`eval::intro`), so the fixpoint sees each answer the model's
// shows depend on and lays out again when one moves. A show runs as its element's own context, so a
// label is resolved without the `context` check `locate` makes.

use crate::eval::content::{
	Content,
	ElemKind,
};
use crate::eval::intro::{
	self,
	Counter,
};
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
	intro::query_label(engine, label, span)
}

/// As [`label`], but a label absent from the document is `None`, not an error.
pub fn label_opt(engine: &mut Engine, label: &Label, span: Span) -> Outcome<Option<Content>> {
	intro::query_label_opt(engine, label, span)
}

/// The elements a selector matches, in document order.
pub fn query(engine: &mut Engine, selector: &Selector, span: Span) -> Outcome<Vec<Content>> {
	match intro::query(engine, selector) {
		Ok(v)	=> Ok(v),
		Err(e)	=> Err(engine.adopt(engine.diags.len(), span, e)),
	}
}

/// A counter's numbers at `loc`: its initial state moved by every update and counted element up to and
/// including the one at `loc`. A location the previous pass did not see reads every update, as Typst's
/// first pass does.
pub fn counter_at(engine: &mut Engine, counter: &Counter, loc: Location, span: Span) -> Outcome<Vec<u64>> {
	match intro::counter_at(engine, counter, loc) {
		Ok(v)	=> Ok(v),
		Err(e)	=> Err(engine.adopt(engine.diags.len(), span, e)),
	}
}

/// A counter at `loc` displayed with `numbering` (a pattern string or a function), which runs in a
/// context at `loc`.
pub fn display_counter(
	engine:		&mut Engine,
	counter:	&Counter,
	loc:		Location,
	numbering:	&Value,
	span:		Span,
)
	-> Outcome<Content>
{
	display(engine, counter, loc, numbering, false, span)
}

/// As [`display_counter`], with a pattern's first prefix and its suffix dropped: the number a reference
/// shows, `1` for a heading numbered `"1."`. A numbering function is called as it is.
pub fn display_counter_trimmed(
	engine:		&mut Engine,
	counter:	&Counter,
	loc:		Location,
	numbering:	&Value,
	span:		Span,
)
	-> Outcome<Content>
{
	display(engine, counter, loc, numbering, true, span)
}

fn display(
	engine:		&mut Engine,
	counter:	&Counter,
	loc:		Location,
	numbering:	&Value,
	trimmed:	bool,
	span:		Span,
)
	-> Outcome<Content>
{
	let v = match intro::display_at(engine, counter, loc, numbering, trimmed, None) {
		Ok(v)	=> v,
		Err(e)	=> return Err(engine.adopt(engine.diags.len(), span, e)),
	};
	super::common::shown(engine, v, span)
}

/// The counter an element kind counts on: `counter(heading)`.
pub fn elem_counter(kind: ElemKind) -> Counter { Counter::of(kind) }

/// Does a bibliography in the document hold `key`?
pub fn bib_has(engine: &mut Engine, key: &Label) -> bool {
	match intro::query(engine, &Selector::Elem(ElemKind::Bibliography, None)) {
		Ok(bibs)	=> bibs.iter().any(|b| super::bibliography::has_key(b, key)),
		Err(_)		=> false,
	}
}
