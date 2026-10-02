// U4 owns this file: selectors and the `selector` functions and methods. Two casts, as Typst has two:
// `cast` for `selector(..)`, `query` and the combinators, and `cast_showable` for a `show` rule, which also
// takes a symbol and refuses what only introspection can answer (a location, `before`, `after`).

use crate::diag::DiagnosticKind;
use crate::eval::args::Args;
use crate::eval::content::{
	self,
	Content,
	ElemKind,
	FieldId,
};
use crate::eval::func::{
	Func,
	NativeFunc,
};
use crate::eval::locate::Location;
use crate::eval::ops;
use crate::eval::scope::Scope;
use crate::eval::styles::StyleChain;
use crate::eval::value::{
	Label,
	RegexValue,
	Value,
};
use crate::eval::Engine;
use crate::syntax::Span;

use oxedyne_fe2o3_core::prelude::*;

use std::sync::Arc;

#[derive(Clone, Debug)]
pub enum Selector {
	Elem(ElemKind, Option<Vec<(FieldId, Value)>>),	// `heading`, `heading.where(level: 1)`
	Label(Label),
	Text(String),									// `show "word"`
	Regex(Arc<RegexValue>),
	Location(Location),
	Or(Vec<Selector>),
	And(Vec<Selector>),
	Before { selector: Arc<Selector>, end: Arc<Selector>, inclusive: bool },
	After { selector: Arc<Selector>, start: Arc<Selector>, inclusive: bool },
}

impl Selector {
	/// Does the element match? A `where` field is compared with the element's own value, else the
	/// chain's (or the schema default without a chain), else the value synthesis gives an `auto` one, as Typst does. Text and regex selectors match
	/// text runs, which `realise.rs` handles, and `before`/`after` only filter a query; here none of
	/// them matches an element.
	pub fn matches(&self, elem: &Content, styles: Option<&StyleChain>) -> Outcome<bool> {
		match self {
			Selector::Elem(kind, fields) => {
				if elem.kind() != Some(*kind) {
					return Ok(false);
				}
				let fields = match fields {
					Some(f)	=> f,
					None	=> return Ok(true),
				};
				let root = StyleChain::root();
				let chain = styles.unwrap_or(&root);
				for (id, want) in fields {
					// A field that only synthesis fills in reads as it will once the element is prepared,
					// so a rule written for it matches the element before realisation has met it.
					let have = match res!(chain.resolve(elem, *id)) {
						Some(Value::Auto)	=> match res!(content::derived_field(elem, *id, chain)) {
							Some(v)	=> Some(v),
							None	=> Some(Value::Auto),
						},
						// Nothing sets it: the default a field has, as Typst compares it.
						None				=> content::default_field(*kind, *id),
						other				=> other,
					};
					match have {
						Some(have) if ops::equal(&have, want)	=> (),
						_										=> return Ok(false),
					}
				}
				Ok(true)
			}
			Selector::Label(l)		=> Ok(elem.label() == Some(l)),
			Selector::Location(loc)	=> Ok(elem.location() == Some(*loc)),
			Selector::Or(ss) => {
				for s in ss {
					if res!(s.matches(elem, styles)) {
						return Ok(true);
					}
				}
				Ok(false)
			}
			Selector::And(ss) => {
				for s in ss {
					if !res!(s.matches(elem, styles)) {
						return Ok(false);
					}
				}
				Ok(true)
			}
			Selector::Text(_) | Selector::Regex(_) | Selector::Before { .. } | Selector::After { .. } => Ok(false),
		}
	}

	/// Can a `show` rule use it? A location, `before` and `after` depend on layout, which a show rule may
	/// not.
	pub fn is_showable(&self) -> bool {
		match self {
			Selector::Location(_) | Selector::Before { .. } | Selector::After { .. }	=> false,
			Selector::Or(ss) | Selector::And(ss)	=> ss.iter().all(|s| s.is_showable()),
			_										=> true,
		}
	}

	/// The first match of a text or regex selector in `text`, as a byte range; `None` for any other
	/// selector.
	pub fn find_text(&self, text: &str) -> Outcome<Option<(usize, usize)>> {
		match self {
			Selector::Text(t) if !t.is_empty() => Ok(text.find(t.as_str()).map(|i| (i, i + t.len()))),
			Selector::Regex(re) => Ok(res!(re.re.find(text)).map(|m| (m.start, m.end))),
			_ => Ok(None),
		}
	}
}

/// A text selector; an empty one is refused.
fn text_selector(engine: &mut Engine, span: Span, text: &str) -> Outcome<Selector> {
	if text.is_empty() {
		return Err(engine.error(DiagnosticKind::Type, span, "text selector is empty"));
	}
	Ok(Selector::Text(text.to_string()))
}

fn func_selector(engine: &mut Engine, span: Span, f: &Func) -> Outcome<Selector> {
	match f.element() {
		Some(k)	=> Ok(Selector::Elem(k, None)),
		None	=> Err(engine.error(DiagnosticKind::Type, span, "only element functions can be used as selectors")),
	}
}

/// Casts a value to a selector as `selector(..)` and `query` accept it.
pub fn cast(engine: &mut Engine, span: Span, v: Value) -> Outcome<Selector> {
	match v {
		Value::Selector(s)	=> Ok((*s).clone()),
		Value::Func(f)		=> func_selector(engine, span, &f),
		Value::Label(l)		=> Ok(Selector::Label(l)),
		Value::Str(s)		=> text_selector(engine, span, &s),
		Value::Regex(r)		=> Ok(Selector::Regex(r)),
		Value::Location(l)	=> Ok(Selector::Location(l)),
		other				=> Err(engine.error(DiagnosticKind::Type, span, fmt!(
			"expected string, function, label, regex, location, or selector, found {}", other.ty().long_name()))),
	}
}

/// Casts a value to a selector as a `show` rule accepts it.
pub fn cast_showable(engine: &mut Engine, span: Span, v: Value) -> Outcome<Selector> {
	let sel = match v {
		Value::Selector(s)	=> (*s).clone(),
		Value::Func(f)		=> res!(func_selector(engine, span, &f)),
		Value::Label(l)		=> Selector::Label(l),
		Value::Str(s)		=> res!(text_selector(engine, span, &s)),
		Value::Symbol(s)	=> res!(text_selector(engine, span, &ops::symbol_text(&s))),
		Value::Regex(r)		=> Selector::Regex(r),
		other				=> return Err(engine.error(DiagnosticKind::Type, span, fmt!(
			"expected symbol, string, label, function, regex, or selector, found {}", other.ty().long_name()))),
	};
	if !sel.is_showable() {
		return Err(engine.error(DiagnosticKind::Type, span, "this selector cannot be used with show"));
	}
	Ok(sel)
}

native_fns! {
	pub enum StyleFn {
		Selector	=> "selector",
		Or			=> "or",
		And			=> "and",
		Before		=> "before",
		After		=> "after",
		Where		=> "where",
	}
}

/// Defines the global `selector` function. The standard library's root scope calls it.
pub fn define(scope: &mut Scope) {
	scope.define("selector", Value::Func(Func::Native(NativeFunc::Style(StyleFn::Selector))));
}

/// The selector methods; `where` is the element function's.
pub fn method(name: &str) -> Option<StyleFn> {
	match name {
		"or"		=> Some(StyleFn::Or),
		"and"		=> Some(StyleFn::And),
		"before"	=> Some(StyleFn::Before),
		"after"		=> Some(StyleFn::After),
		"where"		=> Some(StyleFn::Where),
		_			=> None,
	}
}

/// Takes the next positional argument, or reports it missing at the call.
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

fn finish(engine: &mut Engine, args: Args) -> Outcome<()> {
	match args.items.first() {
		None => Ok(()),
		Some(a) => {
			let span = if a.span.is_detached() { args.span } else { a.span };
			let msg = match &a.name {
				Some(n)	=> fmt!("unexpected argument: {}", n),
				None	=> "unexpected argument".to_string(),
			};
			Err(engine.error(DiagnosticKind::Type, span, msg))
		}
	}
}

fn inclusive(engine: &mut Engine, args: &mut Args) -> Outcome<bool> {
	match res!(args.named::<Value>("inclusive")) {
		None				=> Ok(true),
		Some(Value::Bool(b))	=> Ok(b),
		Some(other)			=> Err(engine.error(DiagnosticKind::Type, args.span, fmt!("expected boolean, found {}", other.ty().long_name()))),
	}
}

/// Runs a selector function. A method arrives with its receiver as the first positional argument.
pub fn call(f: StyleFn, engine: &mut Engine, mut args: Args) -> Outcome<Value> {
	let sel = match f {
		StyleFn::Selector => {
			let (span, v) = res!(take(engine, &mut args, "target"));
			res!(cast(engine, span, v))
		}
		StyleFn::Or | StyleFn::And => {
			let (span, v) = res!(take(engine, &mut args, "self"));
			let mut all = vec![res!(cast(engine, span, v))];
			while args.items.iter().any(|a| a.name.is_none()) {
				let (span, v) = res!(take(engine, &mut args, "others"));
				all.push(res!(cast(engine, span, v)));
			}
			if f == StyleFn::Or { Selector::Or(all) } else { Selector::And(all) }
		}
		StyleFn::Before | StyleFn::After => {
			let (span, v) = res!(take(engine, &mut args, "self"));
			let this = Arc::new(res!(cast(engine, span, v)));
			let what = if f == StyleFn::Before { "end" } else { "start" };
			let (span, v) = res!(take(engine, &mut args, what));
			let bound = Arc::new(res!(cast(engine, span, v)));
			let inclusive = res!(inclusive(engine, &mut args));
			if f == StyleFn::Before {
				Selector::Before { selector: this, end: bound, inclusive }
			} else {
				Selector::After { selector: this, start: bound, inclusive }
			}
		}
		StyleFn::Where => {
			let (span, v) = res!(take(engine, &mut args, "self"));
			let kind = match &v {
				Value::Func(func) => func.element(),
				_ => None,
			};
			let kind = match kind {
				Some(k)	=> k,
				None	=> return Err(engine.error(DiagnosticKind::Type, span, "`where()` can only be called on element functions")),
			};
			let mut fields = Vec::new();
			let mut i = 0;
			while i < args.items.len() {
				let name = match &args.items[i].name {
					Some(n)	=> n.clone(),
					None	=> { i += 1; continue; }
				};
				let a = args.items.remove(i);
				let id = match kind.field_id(&name) {
					Some(id)	=> id,
					None		=> {
						let span = if a.span.is_detached() { args.span } else { a.span };
						return Err(engine.error(DiagnosticKind::Type, span, fmt!(
							"element `{}` does not have field `{}`", kind.name(), name)));
					}
				};
				match fields.iter_mut().find(|(f, _): &&mut (FieldId, Value)| *f == id) {
					Some(slot)	=> slot.1 = a.value,
					None		=> fields.push((id, a.value)),
				}
			}
			Selector::Elem(kind, Some(fields))
		}
	};
	res!(finish(engine, args));
	Ok(Value::Selector(Arc::new(sel)))
}
