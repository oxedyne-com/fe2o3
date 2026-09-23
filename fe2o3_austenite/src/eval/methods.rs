// U2 owns this file: method dispatch by receiver type. A method belongs to the area that owns its
// receiver's type (`str` -> `lib/string.rs`, `counter` -> `lib/intro.rs`, `selector` -> `select.rs`); each
// such area exposes `method(name) -> Option<XFn>`, and this file maps receiver type to area. The four
// mutating methods (`push`, `pop`, `insert`, `remove` on arrays and dictionaries) are carried out here on
// the place itself, as Typst's own evaluator does, since a native function only ever sees a copy.

use crate::eval::args::Args;
use crate::eval::content::{
	Content,
	FieldId,
};
use crate::eval::func::{
	Func,
	NativeFunc,
};
use crate::eval::lib;
use crate::eval::select;
use crate::eval::value::{
	Dict,
	Type,
	Value,
};
use crate::eval::Engine;
use crate::syntax::Span;

use oxedyne_fe2o3_core::prelude::*;

use std::sync::Arc;

native_fns! {
	/// Methods on content, functions, arguments and modules, which no library area owns.
	pub enum CoreFn {
		With			=> "with",
		ContentFields	=> "fields",
		ContentHas		=> "has",
		ContentAt		=> "at",
		ContentFunc		=> "func",
		ContentLocation	=> "location",
		ArgsPos			=> "pos",
		ArgsNamed		=> "named",
		ArgsAt			=> "at",
	}
}

fn fail(msg: String) -> Error<ErrTag> { err!("{}", msg; Input, Invalid) }

/// The method a value of this type answers to by `name`, or a static function in the type's scope
/// (`str.from-unicode`, `datetime.today`), from the area that owns the type.
pub fn type_method(ty: Type, name: &str) -> Option<NativeFunc> {
	match ty {
		Type::Str		=> lib::string::method(name).map(NativeFunc::Str),
		Type::Array		=> lib::array::method(name).map(NativeFunc::Array),
		Type::Dict		=> lib::dict::method(name).map(NativeFunc::Dict),
		Type::Color | Type::Gradient
						=> lib::color::method(name).map(NativeFunc::Color),
		Type::Datetime | Type::Duration
						=> lib::datetime::method(name).map(NativeFunc::Datetime),
		Type::Version | Type::Bytes | Type::Int | Type::Float
						=> lib::foundations::method(name).map(NativeFunc::Found),
		Type::Symbol	=> lib::sym::method(name).map(NativeFunc::Sym),
		Type::Length | Type::Angle | Type::Ratio | Type::Relative | Type::Fraction | Type::Alignment
			| Type::Direction
						=> lib::geom::method(name).map(NativeFunc::Geom),
		Type::Counter | Type::State | Type::Location
						=> lib::intro::method(name).map(NativeFunc::Intro),
		Type::Selector	=> select::method(name).map(NativeFunc::Style),
		Type::Func		=> match name {
			"with"	=> Some(NativeFunc::Core(CoreFn::With)),
			"where"	=> select::method(name).map(NativeFunc::Style),
			_		=> None,
		},
		Type::Content	=> match name {
			"fields"	=> Some(NativeFunc::Core(CoreFn::ContentFields)),
			"has"		=> Some(NativeFunc::Core(CoreFn::ContentHas)),
			"at"		=> Some(NativeFunc::Core(CoreFn::ContentAt)),
			"func"		=> Some(NativeFunc::Core(CoreFn::ContentFunc)),
			"location"	=> Some(NativeFunc::Core(CoreFn::ContentLocation)),
			_			=> None,
		},
		Type::Args		=> match name {
			"pos"	=> Some(NativeFunc::Core(CoreFn::ArgsPos)),
			"named"	=> Some(NativeFunc::Core(CoreFn::ArgsNamed)),
			"at"	=> Some(NativeFunc::Core(CoreFn::ArgsAt)),
			_		=> None,
		},
		_				=> None,
	}
}

/// Is `name` one of the methods that change their receiver in place?
pub fn is_mutating(name: &str) -> bool { matches!(name, "push" | "pop" | "insert" | "remove") }

pub fn call(f: CoreFn, engine: &mut Engine, mut args: Args) -> Outcome<Value> {
	let _ = engine;
	let receiver = res!(args.expect::<Value>("self"));
	let out = match (f, receiver) {
		(CoreFn::With, Value::Func(func)) => {
			let bound = std::mem::take(&mut args);
			return Ok(Value::Func(func.with(bound)));
		}
		(CoreFn::ContentFields, Value::Content(c))	=> Value::dict(content_fields(&c)),
		(CoreFn::ContentHas, Value::Content(c)) => {
			let name = res!(args.expect::<String>("field"));
			Value::Bool(content_field(&c, &name).is_some())
		}
		(CoreFn::ContentAt, Value::Content(c)) => {
			let name = res!(args.expect::<String>("field"));
			let default = res!(args.named::<Value>("default"));
			match (content_field(&c, &name), default) {
				(Some(v), _)		=> v,
				(None, Some(d))		=> d,
				(None, None)		=> return Err(fail(fmt!(
					"{} does not have field \"{}\" and no default was specified", content_name(&c), name))),
			}
		}
		(CoreFn::ContentFunc, Value::Content(c)) => match c.kind() {
			Some(k)	=> Value::Func(Func::Element(k)),
			None	=> return Err(fail(fmt!(
				"the element function of {} content cannot be represented yet", content_name(&c)))),
		},
		(CoreFn::ContentLocation, Value::Content(c)) => match c.location() {
			Some(l)	=> Value::Location(l),
			None	=> Value::None,
		},
		(CoreFn::ArgsPos, Value::Args(a)) => Value::array(
			a.items.iter().filter(|i| i.name.is_none()).map(|i| i.value.clone()).collect()),
		(CoreFn::ArgsNamed, Value::Args(a)) => {
			let mut d = Dict::new();
			for i in a.items.iter() {
				if let Some(n) = &i.name {
					d.insert(n, i.value.clone());
				}
			}
			Value::dict(d)
		}
		(CoreFn::ArgsAt, Value::Args(a)) => {
			let key = res!(args.expect::<Value>("key"));
			let default = res!(args.named::<Value>("default"));
			let found = match &key {
				Value::Int(i) => {
					let pos: Vec<&Value> = a.items.iter().filter(|x| x.name.is_none()).map(|x| &x.value).collect();
					let idx = if *i < 0 { pos.len() as i64 + i } else { *i };
					if idx >= 0 { pos.get(idx as usize).map(|v| (*v).clone()) } else { None }
				}
				Value::Str(s) => a.items.iter().rev().find(|x| x.name.as_deref() == Some(s.as_str()))
					.map(|x| x.value.clone()),
				other => return Err(fail(fmt!("expected integer or string, found {}", other.ty().long_name()))),
			};
			match (found, default, key) {
				(Some(v), _, _)				=> v,
				(None, Some(d), _)			=> d,
				(None, None, Value::Int(i))	=> return Err(fail(fmt!(
					"no positional argument at index {} and no default value was specified", i))),
				(None, None, Value::Str(s))	=> return Err(fail(fmt!(
					"arguments do not contain key \"{}\" and no default value was specified", s))),
				(None, None, _)				=> Value::None,
			}
		}
		(f, other) => return Err(fail(fmt!("type {} has no method `{}`", other.ty().long_name(), f.name()))),
	};
	res!(args.finish());
	Ok(out)
}

/// The name Typst gives a content value's element: `strong`, `sequence`, `styled`.
pub fn content_name(c: &Content) -> &'static str {
	match c {
		Content::Elem(e)		=> e.kind.name(),
		Content::Sequence(_)	=> "sequence",
		Content::Styled(_)		=> "styled",
	}
}

/// A field as `it.name` reads it: a stored field, the label, a sequence's children or a styled
/// content's child and styles.
pub fn content_field(c: &Content, name: &str) -> Option<Value> {
	if name == "label" {
		return c.label().map(|l| Value::Label(l.clone()));
	}
	match c {
		Content::Elem(_)		=> c.field(name).cloned(),
		Content::Sequence(s)	=> match name {
			"children"	=> Some(Value::array(s.children.iter().cloned().map(Value::Content).collect())),
			_			=> None,
		},
		Content::Styled(s)		=> match name {
			"child"		=> Some(Value::Content(s.child.clone())),
			"styles"	=> Some(Value::Styles(s.styles.clone())),
			_			=> None,
		},
	}
}

/// `content.fields()`: every stored field in schema order, then the label.
pub fn content_fields(c: &Content) -> Dict {
	let mut d = Dict::new();
	match c {
		Content::Elem(e) => {
			let mut fields: Vec<&(FieldId, Value)> = e.fields.iter().collect();
			fields.sort_by_key(|(id, _)| *id);
			for (id, v) in fields {
				if let Some(spec) = e.kind.field_spec(*id) {
					d.insert(spec.name, v.clone());
				}
			}
		}
		Content::Sequence(s) => {
			d.insert("children", Value::array(s.children.iter().cloned().map(Value::Content).collect()));
		}
		Content::Styled(s) => {
			d.insert("child", Value::Content(s.child.clone()));
			d.insert("styles", Value::Styles(s.styles.clone()));
		}
	}
	if let Some(l) = c.label() {
		d.insert("label", Value::Label(l.clone()));
	}
	d
}

/// `receiver.name(args)` for a receiver the call does not mutate.
pub fn call_method(
	engine:		&mut Engine,
	receiver:	Value,
	name:		&str,
	mut args:	Args,
	span:		Span,
)
	-> Outcome<Value>
{
	if is_mutating(name) && matches!(receiver, Value::Array(_) | Value::Dict(_)) {
		return Err(engine.error(span, "cannot mutate a temporary value"));
	}
	match type_method(receiver.ty(), name) {
		Some(f) => {
			args.prepend(span, receiver);
			engine.call_func(&Func::Native(f), args)
		}
		None => Err(engine.error(span, fmt!("type {} has no method `{}`", receiver.ty().long_name(), name))),
	}
}

/// `receiver.name(args)` where the receiver is a place (`arr.push(x)`) and may be changed in it. A
/// method that does not mutate is passed on to [`call_method`] with a copy.
pub fn call_method_mut(
	engine:		&mut Engine,
	receiver:	&mut Value,
	name:		&str,
	args:		Args,
	span:		Span,
)
	-> Outcome<Value>
{
	let out = match receiver {
		Value::Array(a) if is_mutating(name)	=> array_mut(Arc::make_mut(a), name, args),
		Value::Dict(d) if is_mutating(name)		=> dict_mut(Arc::make_mut(d), name, args),
		_ => return call_method(engine, receiver.clone(), name, args, span),
	};
	match out {
		Ok(v)	=> Ok(v),
		Err(e)	=> Err(engine.error(span, e.plain())),
	}
}

fn index(len: usize, i: i64, allow_end: bool) -> Outcome<usize> {
	let idx = if i < 0 { len as i64 + i } else { i };
	let limit = if allow_end { len as i64 } else { len as i64 - 1 };
	if idx < 0 || idx > limit {
		return Err(fail(fmt!("array index out of bounds (index: {}, len: {})", i, len)));
	}
	Ok(idx as usize)
}

fn array_mut(a: &mut Vec<Value>, name: &str, mut args: Args) -> Outcome<Value> {
	let out = match name {
		"push" => {
			let v = res!(args.expect::<Value>("value"));
			a.push(v);
			Value::None
		}
		"pop" => match a.pop() {
			Some(v)	=> v,
			None	=> return Err(fail("array is empty".to_string())),
		},
		"insert" => {
			let i = res!(args.expect::<i64>("index"));
			let v = res!(args.expect::<Value>("value"));
			let at = res!(index(a.len(), i, true));
			a.insert(at, v);
			Value::None
		}
		_ => {
			let i = res!(args.expect::<i64>("index"));
			let default = res!(args.named::<Value>("default"));
			match (index(a.len(), i, false), default) {
				(Ok(at), _)		=> a.remove(at),
				(Err(_), Some(d))	=> d,
				(Err(_), None)	=> return Err(fail(fmt!(
					"array index out of bounds (index: {}, len: {}) and no default value was specified",
					i, a.len()))),
			}
		}
	};
	res!(args.finish());
	Ok(out)
}

fn dict_mut(d: &mut Dict, name: &str, mut args: Args) -> Outcome<Value> {
	let out = match name {
		"insert" => {
			let k = res!(args.expect::<String>("key"));
			let v = res!(args.expect::<Value>("value"));
			d.insert(&k, v);
			Value::None
		}
		"remove" => {
			let k = res!(args.expect::<String>("key"));
			let default = res!(args.named::<Value>("default"));
			match (d.remove(&k), default) {
				(Some(v), _)		=> v,
				(None, Some(v))		=> v,
				(None, None)		=> return Err(fail(fmt!("dictionary does not contain key \"{}\"", k))),
			}
		}
		other => return Err(fail(fmt!("type dictionary has no method `{}`", other))),
	};
	res!(args.finish());
	Ok(out)
}
