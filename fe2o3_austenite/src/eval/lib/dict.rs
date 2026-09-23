// U3 owns this file. Methods on `dictionary`; `insert` and `remove` mutate their receiver and arrive
// through `call_mut`. `dictionary(module)` turns a module's bindings into a dictionary.

use crate::eval::args::Args;
use crate::eval::lib::foundations::{
	finish,
	mismatch,
	need,
	receiver,
	repr_str,
	str_of,
};
use crate::eval::scope::Scope;
use crate::eval::value::{
	Dict,
	Value,
};
use crate::eval::Engine;

use oxedyne_fe2o3_core::prelude::*;

use std::sync::Arc;

native_fns! {
	pub enum DictFn {
		Construct	=> "dictionary",
		Len			=> "len",
		At			=> "at",
		Insert		=> "insert",
		Remove		=> "remove",
		Keys		=> "keys",
		Values		=> "values",
		Pairs		=> "pairs",
	}
}

pub fn define(_scope: &mut Scope) {}

pub fn method(name: &str) -> Option<DictFn> {
	DictFn::ALL.iter().copied().find(|f| f.name() == name && *f != DictFn::Construct)
}

impl DictFn {
	pub fn mutates(self) -> bool { matches!(self, DictFn::Insert | DictFn::Remove) }
}

/// A mutating method on a place: `d.insert(k, v)`.
pub fn call_mut(f: DictFn, engine: &mut Engine, recv: &mut Value, mut args: Args) -> Outcome<Value> {
	let span = args.span;
	let d = match recv {
		Value::Dict(d)	=> Arc::make_mut(d),
		other			=> return Err(mismatch(engine, span, "dictionary", other)),
	};
	let out = match f {
		DictFn::Insert => {
			let k = res!(need(engine, &mut args, "key"));
			let k = res!(str_of(engine, span, k));
			let v = res!(need(engine, &mut args, "value"));
			d.insert(&k, v);
			Value::None
		}
		DictFn::Remove => {
			let k = res!(need(engine, &mut args, "key"));
			let k = res!(str_of(engine, span, k));
			let default = res!(args.named::<Value>("default"));
			match (d.remove(&k), default) {
				(Some(v), _)	=> v,
				(None, Some(x))	=> x,
				(None, None)	=> return Err(engine.error(span, fmt!(
					"dictionary does not contain key {}", repr_str(&k)))),
			}
		}
		_ => return Err(engine.error(span, fmt!("dictionary.{} does not mutate its receiver", f.name()))),
	};
	res!(finish(engine, args));
	Ok(out)
}

pub fn call(f: DictFn, engine: &mut Engine, mut args: Args) -> Outcome<Value> {
	let span = args.span;
	if f == DictFn::Construct {
		let v = res!(need(engine, &mut args, "value"));
		res!(finish(engine, args));
		return match v {
			Value::Module(m) => {
				let mut names: Vec<(&String, &Value, crate::syntax::Span)> =
					m.scope.map.iter().map(|(k, b)| (k, &b.value, b.span)).collect();
				// A module's bindings come out in definition order where spans say it, else by name.
				names.sort_by(|a, b| (a.2.file.0, a.2.start, a.0).cmp(&(b.2.file.0, b.2.start, b.0)));
				let mut d = Dict::new();
				for (k, v, _) in names {
					d.insert(k, v.clone());
				}
				Ok(Value::dict(d))
			}
			other => Err(mismatch(engine, span, "module", &other)),
		};
	}
	if f.mutates() {
		return Err(engine.error(span, "cannot mutate a temporary value"));
	}
	let d = match res!(receiver(&mut args)) {
		Value::Dict(d)	=> d,
		other			=> return Err(mismatch(engine, span, "dictionary", &other)),
	};
	let out = match f {
		DictFn::Len		=> Value::Int(d.len() as i64),
		DictFn::At		=> {
			let k = res!(need(engine, &mut args, "key"));
			let k = res!(str_of(engine, span, k));
			let default = res!(args.named::<Value>("default"));
			match (d.get(&k), default) {
				(Some(v), _)	=> v.clone(),
				(None, Some(x))	=> x,
				(None, None)	=> return Err(engine.error(span, fmt!(
					"dictionary does not contain key {} and no default value was specified", repr_str(&k)))),
			}
		}
		DictFn::Keys	=> Value::array(d.keys().map(Value::str).collect()),
		DictFn::Values	=> Value::array(d.iter().map(|(_, v)| v.clone()).collect()),
		DictFn::Pairs	=> Value::array(d.iter().map(|(k, v)| Value::array(vec![Value::str(k), v.clone()])).collect()),
		DictFn::Construct | DictFn::Insert | DictFn::Remove => Value::None,
	};
	res!(finish(engine, args));
	Ok(out)
}
