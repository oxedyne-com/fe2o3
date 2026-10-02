// U8 owns this file. Introspection functions and the methods of counter, state and location, plus schemas
// for metadata, counter/state updates, context and layout. Each read a context body makes is recorded on
// `engine.reads` for the fixpoint.
//
// A `context` body's errors are delayed, as Typst delays them: the diagnostic is kept and the body shows
// as nothing, so a first pass that cannot yet see a later label does not stop the document. The fixpoint
// clears a pass's diagnostics before the next, so only an error the final pass still makes survives.

use crate::diag::DiagnosticKind;
use crate::eval::args::Args;
use crate::eval::content::{
	self,
	Content,
	ElemKind,
	FieldDefault,
	FieldSpec,
	FieldType,
};
use crate::eval::func::{
	Func,
	NativeFunc,
};
use crate::eval::intro::{
	self as introspect,
	Counter,
	CounterKey,
	CounterUpdate,
	State,
};
use crate::eval::scope::Scope;
use crate::eval::select::{
	self,
	Selector,
};
use crate::eval::styles::StyleChain;
use crate::eval::value::{
	Dict,
	Length,
	Type,
	Value,
};
use crate::eval::{
	Context,
	Engine,
};
use crate::flow::{
	self,
	Region,
};
use crate::ir::Sp;
use crate::ledger::Position;
use crate::syntax::Span;

use oxedyne_fe2o3_core::prelude::*;

use std::sync::Arc;

native_fns! {
	pub enum IntroFn {
		Counter						=> "counter",
		State						=> "state",
		Query						=> "query",
		Here						=> "here",
		Locate						=> "locate",
		Measure						=> "measure",
		CounterGet					=> "get",
		CounterAt					=> "at",
		CounterFinal				=> "final",
		CounterDisplay				=> "display",
		CounterStep					=> "step",
		CounterUpdate				=> "update",
		StateGet					=> "get",
		StateAt						=> "at",
		StateFinal					=> "final",
		StateUpdate					=> "update",
		LocationPage				=> "page",
		LocationPosition			=> "position",
		LocationPageNumbering		=> "page-numbering",
	}
}

// Schemas

const METADATA: &[FieldSpec] = &[
	FieldSpec::required("value", FieldType::Any),
];

const COUNTER_UPDATE: &[FieldSpec] = &[
	FieldSpec::required("key", FieldType::Any),
	FieldSpec::named("update", FieldType::Any, FieldDefault::None).synthesised(),	// set, step or function
];

const STATE_UPDATE: &[FieldSpec] = &[
	FieldSpec::required("key", FieldType::Of(Type::Str)),
	FieldSpec::named("update", FieldType::Any, FieldDefault::None).synthesised(),	// a value or a function
];

const FUNC_ONLY: &[FieldSpec] = &[
	FieldSpec::required("func", FieldType::Of(Type::Func)),
];

pub fn fields(kind: ElemKind) -> &'static [FieldSpec] {
	match kind {
		ElemKind::Metadata		=> METADATA,
		ElemKind::CounterUpdate	=> COUNTER_UPDATE,
		ElemKind::StateUpdate	=> STATE_UPDATE,
		ElemKind::Context		=> FUNC_ONLY,
		ElemKind::Layout		=> FUNC_ONLY,
		_						=> &[],
	}
}

pub fn define(scope: &mut Scope) {
	scope.define("counter",	Value::Type(Type::Counter));
	scope.define("state",	Value::Type(Type::State));
	for f in [IntroFn::Query, IntroFn::Here, IntroFn::Locate, IntroFn::Measure] {
		scope.define(f.name(), Value::Func(Func::Native(NativeFunc::Intro(f))));
	}
}

/// The method of a counter, state or location by name. The names are shared, so `call` settles which
/// type's method runs from the receiver.
pub fn method(name: &str) -> Option<IntroFn> {
	match name {
		"get"				=> Some(IntroFn::CounterGet),
		"at"				=> Some(IntroFn::CounterAt),
		"final"				=> Some(IntroFn::CounterFinal),
		"display"			=> Some(IntroFn::CounterDisplay),
		"step"				=> Some(IntroFn::CounterStep),
		"update"			=> Some(IntroFn::CounterUpdate),
		"page"				=> Some(IntroFn::LocationPage),
		"position"			=> Some(IntroFn::LocationPosition),
		"page-numbering"	=> Some(IntroFn::LocationPageNumbering),
		_					=> None,
	}
}

/// The receiver a method was called on.
enum Receiver {
	Counter(Arc<Counter>),
	State(Arc<State>),
	Location(crate::eval::locate::Location),
}

pub fn call(f: IntroFn, engine: &mut Engine, mut args: Args) -> Outcome<Value> {
	let span = args.span;
	let out = match f {
		IntroFn::Counter	=> {
			let (at, v) = res!(take(engine, &mut args, "key"));
			let key = res!(counter_key(engine, at, v));
			Value::Counter(Arc::new(Counter::new(key)))
		}
		IntroFn::State		=> {
			let key = match res!(take(engine, &mut args, "key")) {
				(_, Value::Str(s))	=> s.to_string(),
				(at, other)			=> return Err(engine.error(DiagnosticKind::Type, at, fmt!(
					"expected string, found {}", other.ty().long_name()))),
			};
			let init = res!(args.eat::<Value>()).unwrap_or(Value::None);
			Value::State(Arc::new(State { key, init }))
		}
		IntroFn::Query		=> {
			let (at, v) = res!(take(engine, &mut args, "target"));
			let sel = res!(select::cast(engine, at, v));
			res!(introspect::check_locatable(engine, at, &sel));
			if engine.context.location.is_none() {
				return Err(introspect::no_context(engine, span));
			}
			let found = res!(introspect::query(engine, &sel));
			Value::array(found.into_iter().map(Value::Content).collect())
		}
		IntroFn::Here		=> Value::Location(res!(introspect::here(engine, span))),
		IntroFn::Locate		=> {
			let (at, v) = res!(take(engine, &mut args, "selector"));
			let sel = res!(select::cast(engine, at, v));
			res!(introspect::check_locatable(engine, at, &sel));
			Value::Location(res!(introspect::resolve_unique(engine, span, &sel)))
		}
		IntroFn::Measure	=> res!(measure(engine, span, &mut args)),
		_					=> {
			let receiver = match res!(args.eat::<Value>()) {
				Some(Value::Counter(c))		=> Receiver::Counter(c),
				Some(Value::State(s))		=> Receiver::State(s),
				Some(Value::Location(l))	=> Receiver::Location(l),
				Some(other)					=> return Err(engine.error(DiagnosticKind::Type, span, fmt!(
					"type {} has no method `{}`", other.ty().name(), f.name()))),
				None						=> return Err(engine.error(DiagnosticKind::Type, span, "missing argument: self")),
			};
			match receiver {
				Receiver::Counter(c)	=> res!(counter_method(engine, span, f, &c, &mut args)),
				Receiver::State(s)		=> res!(state_method(engine, span, f, &s, &mut args)),
				Receiver::Location(l)	=> res!(location_method(engine, span, f, l)),
			}
		}
	};
	res!(finish(engine, args));
	Ok(out)
}

/// The next positional argument and the span to report it at, or "missing argument" at the call.
fn take(engine: &mut Engine, args: &mut Args, what: &str) -> Outcome<(Span, Value)> {
	match args.items.iter().position(|a| a.name.is_none()) {
		Some(i) => {
			let a = args.items.remove(i);
			let span = if a.span.is_detached() { args.span } else { a.span };
			Ok((span, a.value))
		}
		None => Err(engine.error(DiagnosticKind::Type, args.span, fmt!("missing argument: {}", what))),
	}
}

fn expect(engine: &mut Engine, args: &mut Args, what: &str) -> Outcome<Value> {
	take(engine, args, what).map(|(_, v)| v)
}

fn finish(engine: &mut Engine, args: Args) -> Outcome<()> {
	match args.items.first() {
		None	=> Ok(()),
		Some(a)	=> {
			let span = if a.span.is_detached() { args.span } else { a.span };
			let msg = match &a.name {
				Some(n)	=> fmt!("unexpected argument: {}", n),
				None	=> "unexpected argument".to_string(),
			};
			Err(engine.error(DiagnosticKind::Type, span, msg))
		}
	}
}

/// Typst's `CounterKey` cast: a string, a label, a location, an element function (`page` is the page
/// counter) or a selector, the last two over locatable elements only.
fn counter_key(engine: &mut Engine, span: Span, v: Value) -> Outcome<CounterKey> {
	match v {
		Value::Str(s)		=> Ok(CounterKey::Str(s.to_string())),
		Value::Label(l)		=> Ok(CounterKey::Selector(Selector::Label(l))),
		Value::Location(l)	=> Ok(CounterKey::Selector(Selector::Location(l))),
		Value::Func(f) => match f.element() {
			Some(ElemKind::Page)	=> Ok(CounterKey::Page),
			Some(k)					=> {
				let sel = Selector::Elem(k, None);
				res!(introspect::check_locatable(engine, span, &sel));
				Ok(CounterKey::Selector(sel))
			}
			None => Err(engine.error(DiagnosticKind::Type, span, "only element functions can be used as selectors")),
		},
		Value::Selector(s) => {
			res!(introspect::check_locatable(engine, span, &s));
			Ok(CounterKey::Selector((*s).clone()))
		}
		other => Err(engine.error(DiagnosticKind::Type, span, fmt!(
			"expected string, label, function, location, or selector, found {}", other.ty().long_name()))),
	}
}

/// A selector a method resolves to one location: `counter.at`, `state.at`.
fn locatable_arg(engine: &mut Engine, args: &mut Args) -> Outcome<Selector> {
	let (at, v) = res!(take(engine, args, "selector"));
	let sel = res!(select::cast(engine, at, v));
	res!(introspect::check_locatable(engine, at, &sel));
	Ok(sel)
}

/// Fails outside a context that has a location, as Typst's `Context::introspect` does.
fn introspect_here(engine: &mut Engine, span: Span) -> Outcome<crate::eval::locate::Location> {
	introspect::here(engine, span)
}

fn counter_method(engine: &mut Engine, span: Span, f: IntroFn, c: &Counter, args: &mut Args) -> Outcome<Value> {
	Ok(match f {
		IntroFn::CounterGet		=> {
			let loc = res!(introspect_here(engine, span));
			introspect::state_value(&res!(introspect::counter_at(engine, c, loc)))
		}
		IntroFn::CounterAt		=> {
			let sel = res!(locatable_arg(engine, args));
			let loc = res!(introspect::resolve_unique(engine, span, &sel));
			introspect::state_value(&res!(introspect::counter_at(engine, c, loc)))
		}
		IntroFn::CounterFinal	=> {
			res!(introspect_here(engine, span));
			introspect::state_value(&res!(introspect::counter_final(engine, c)))
		}
		IntroFn::CounterDisplay	=> {
			let numbering = match res!(args.eat::<Value>()) {
				None | Some(Value::Auto)	=> None,
				Some(v @ Value::Str(_)) | Some(v @ Value::Func(_))	=> Some(v),
				Some(other)					=> return Err(engine.error(DiagnosticKind::Type, span, fmt!(
					"expected string, function, or auto, found {}", other.ty().long_name()))),
			};
			let both = match res!(args.named::<Value>("both")) {
				None				=> false,
				Some(Value::Bool(b))	=> b,
				Some(other)			=> return Err(engine.error(DiagnosticKind::Type, span, fmt!(
					"expected boolean, found {}", other.ty().long_name()))),
			};
			// `at:` displays the counter somewhere else, one element the selector names.
			let loc = match res!(args.named::<Value>("at")) {
				None | Some(Value::Auto) => res!(introspect_here(engine, span)),
				Some(v) => {
					let sel = res!(select::cast(engine, span, v));
					res!(introspect::check_locatable(engine, span, &sel));
					res!(introspect::resolve_unique(engine, span, &sel))
				}
			};
			let styles = engine.context.styles.clone();
			res!(introspect::counter_display(engine, c, numbering, both, loc, styles.as_ref()))
		}
		IntroFn::CounterStep	=> {
			let level = match res!(args.named::<Value>("level")) {
				None				=> 1,
				Some(Value::Int(l)) if l >= 1	=> l as usize,
				Some(Value::Int(_))	=> return Err(engine.error(DiagnosticKind::Type, span, "number must be positive")),
				Some(other)			=> return Err(engine.error(DiagnosticKind::Type, span, fmt!(
					"expected integer, found {}", other.ty().long_name()))),
			};
			Value::Content(res!(counter_update(engine, span, c, CounterUpdate::Step(level))))
		}
		IntroFn::CounterUpdate	=> {
			let v = res!(expect(engine, args, "update"));
			let update = match v {
				Value::Func(f)	=> CounterUpdate::Func(f),
				other			=> match introspect::counter_state(&other) {
					Ok(s)	=> CounterUpdate::Set(s),
					Err(_)	=> return Err(engine.error(DiagnosticKind::Type, span, fmt!(
						"expected integer, array, or function, found {}", other.ty().long_name()))),
				},
			};
			Value::Content(res!(counter_update(engine, span, c, update)))
		}
		other => return Err(engine.error(DiagnosticKind::Type, span, fmt!("type counter has no method `{}`", other.name()))),
	})
}

fn counter_update(engine: &mut Engine, span: Span, c: &Counter, update: CounterUpdate) -> Outcome<Content> {
	content::build(engine, ElemKind::CounterUpdate, vec![
		("key",		c.key.to_value()),
		("update",	update.to_value()),
	], span)
}

fn state_method(engine: &mut Engine, span: Span, f: IntroFn, s: &State, args: &mut Args) -> Outcome<Value> {
	Ok(match f {
		IntroFn::CounterGet | IntroFn::StateGet => {
			let loc = res!(introspect_here(engine, span));
			res!(introspect::state_at(engine, s, loc))
		}
		IntroFn::CounterAt | IntroFn::StateAt => {
			let sel = res!(locatable_arg(engine, args));
			let loc = res!(introspect::resolve_unique(engine, span, &sel));
			res!(introspect::state_at(engine, s, loc))
		}
		IntroFn::CounterFinal | IntroFn::StateFinal => {
			res!(introspect_here(engine, span));
			res!(introspect::state_final(engine, s))
		}
		IntroFn::CounterUpdate | IntroFn::StateUpdate => {
			let v = res!(expect(engine, args, "update"));
			Value::Content(res!(content::build(engine, ElemKind::StateUpdate, vec![
				("key",		Value::str(s.key.clone())),
				("update",	v),
			], span)))
		}
		other => return Err(engine.error(DiagnosticKind::Type, span, fmt!("type state has no method `{}`", other.name()))),
	})
}

fn location_method(engine: &mut Engine, span: Span, f: IntroFn, loc: crate::eval::locate::Location) -> Outcome<Value> {
	// An unplaced location reads as the top of page one, as Typst's introspector answers. The page alone
	// is asked for on its own, so it settles as soon as the page does.
	Ok(match f {
		IntroFn::LocationPage			=> Value::Int(res!(introspect::page(engine, loc)).unwrap_or(1) as i64),
		IntroFn::LocationPosition		=> introspect::position_dict(res!(introspect::position(engine, loc))
			.unwrap_or(Position::new(1, Sp::ZERO, Sp::ZERO))),
		IntroFn::LocationPageNumbering	=> res!(introspect::page_numbering(engine, loc)),
		other => return Err(engine.error(DiagnosticKind::Type, span, fmt!("type location has no method `{}`", other.name()))),
	})
}

/// `measure(content, width: auto, height: auto)`: the size the content takes, laid out under the
/// context's styles in a region as large as given, `auto` being unbounded. The layout is discarded, and
/// it runs [detached](introspect::detached), so measuring leaves no trace on the document's locations.
fn measure(engine: &mut Engine, span: Span, args: &mut Args) -> Outcome<Value> {
	let body = match res!(expect(engine, args, "content")) {
		Value::Content(c)	=> c,
		other				=> res!(content::display(engine, other, span)),
	};
	let width = res!(args.named::<Value>("width"));
	let height = res!(args.named::<Value>("height"));
	let styles = res!(introspect::context_styles(engine, span));
	let w = res!(extent(engine, span, width, &styles));
	let h = res!(extent(engine, span, height, &styles));
	let region = Region { width: w, height: h, base: (w, h), expand_x: false, expand_y: false };
	let dims = res!(introspect::detached(engine, |e| flow::measure(e, &body, &styles, region)));
	let mut d = Dict::new();
	d.insert("width", Value::Length(Length::pt(dims.width.to_pt())));
	d.insert("height", Value::Length(Length::pt((dims.height + dims.depth).to_pt())));
	Ok(Value::dict(d))
}

fn extent(engine: &mut Engine, span: Span, v: Option<Value>, styles: &StyleChain) -> Outcome<Sp> {
	match v {
		None | Some(Value::Auto)	=> Ok(introspect::UNBOUNDED),
		Some(Value::Length(l))		=> Ok(Sp::from_pt(styles.resolve_length(l))),
		Some(other)					=> Err(engine.error(DiagnosticKind::Type, span, fmt!(
			"expected length or auto, found {}", other.ty().long_name()))),
	}
}

pub fn construct(_engine: &mut Engine, _kind: ElemKind, _args: &mut Args) -> Outcome<Option<Content>> {
	Ok(None)
}

/// Metadata and the update elements show as nothing, only their tags marking where they sit; a
/// `context` element shows as its body's result; `layout` is flow's, which knows the region.
pub fn show(engine: &mut Engine, elem: &Content, styles: &StyleChain) -> Outcome<Option<Content>> {
	let kind = match elem.kind() {
		Some(k)	=> k,
		None	=> return Ok(None),
	};
	match kind {
		ElemKind::Metadata | ElemKind::CounterUpdate | ElemKind::StateUpdate => Ok(Some(Content::empty())),
		ElemKind::Context	=> show_context(engine, elem, styles).map(Some),
		_					=> Ok(None),
	}
}

fn show_context(engine: &mut Engine, elem: &Content, styles: &StyleChain) -> Outcome<Content> {
	let span = elem.span();
	let func = match elem.field("func") {
		Some(Value::Func(f))	=> f.clone(),
		_						=> return Err(engine.error(DiagnosticKind::Type, span, "context element has no function")),
	};
	let saved = std::mem::replace(&mut engine.context, Context {
		location:	elem.location(),
		styles:		Some(styles.clone()),
	});
	let mark = engine.diags.len();
	let result = engine.call_func(&func, Args::new(span));
	engine.context = saved;
	// An error is delayed: its diagnostic stands, and the body shows as nothing this pass.
	match result.and_then(|v| content::display(engine, v, span)) {
		Ok(c)	=> Ok(c),
		Err(e)	=> {
			engine.adopt(mark, span, e);
			Ok(Content::empty())
		}
	}
}
