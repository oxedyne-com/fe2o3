// U3 owns this file. Methods on `dictionary`; `insert` and `remove` change a place, so `methods.rs`
// carries them out. `dictionary(module)` turns a module's bindings into a dictionary.

use crate::eval::args::Args;
use crate::eval::lib::array;
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


native_fns! {
	pub enum DictFn {
		Construct	=> "dictionary",
		Len			=> "len",
		At			=> "at",
		Keys		=> "keys",
		Values		=> "values",
		Pairs		=> "pairs",
		Filter		=> "filter",
		Map			=> "map",
	}
}

pub fn define(_scope: &mut Scope) {}

pub fn method(name: &str) -> Option<DictFn> {
	DictFn::ALL.iter().copied().find(|f| f.name() == name && *f != DictFn::Construct)
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
		// Both see the value only; the keys stay as they are.
		DictFn::Filter | DictFn::Map => {
			let g = res!(need(engine, &mut args, if f == DictFn::Filter { "test" } else { "mapper" }));
			let g = res!(array::func_of(engine, span, g));
			let mut out = Dict::new();
			for (k, v) in d.iter() {
				let r = res!(array::apply(engine, span, &g, vec![v.clone()]));
				if f == DictFn::Map {
					out.insert(k, r);
					continue;
				}
				match r {
					Value::Bool(true)	=> out.insert(k, v.clone()),
					Value::Bool(false)	=> (),
					other				=> return Err(mismatch(engine, span, "boolean", &other)),
				}
			}
			Value::dict(out)
		}
		DictFn::Construct => Value::None,
	};
	res!(finish(engine, args));
	Ok(out)
}
