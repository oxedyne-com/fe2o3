// U3 owns this file. The `sym` and `emoji` modules as static tables, `symbol(..)`, and modifier resolution
// (`arrow.r.long`). The tables in `sym_data.rs` are generated from the `typst` 0.15.1 oracle rather than
// migrated from `lang/mathparse.rs`, which knew a subset: every name, submodule (`sym.control`,
// `sym.gender`) and variant Typst has is here, in Typst's order, so variant choice and `repr` agree.
//
// Resolution follows codex: the applied modifiers must all be present in a variant, and among those the
// variant with the fewest modifiers wins, the first in table order on a tie.

use crate::eval::args::Args;
use crate::eval::lib::foundations::{
	finish,
	mismatch,
	pretty_array_like,
	repr_str,
};
use crate::eval::scope::Scope;
use crate::eval::value::{
	Module,
	Symbol,
	SymVariants,
	Value,
};
use crate::eval::Engine;

use oxedyne_fe2o3_core::prelude::*;

use std::sync::Arc;

native_fns! {
	pub enum SymFn {
		Symbol		=> "symbol",
	}
}

/// One entry of a generated table: a symbol with one text, a symbol with variants (the unmodified one,
/// when it exists, has empty modifiers), or a submodule.
#[derive(Clone, Copy, Debug)]
pub enum SymDef {
	One(&'static str),
	Many(&'static [(&'static str, &'static str)]),
	Module(&'static [(&'static str, SymDef)]),
}

include!("sym_data.rs");

pub fn define(scope: &mut Scope) {
	scope.define("sym", Value::Module(Arc::new(module("sym", SYM))));
	scope.define("emoji", Value::Module(Arc::new(module("emoji", EMOJI))));
}

fn module(name: &str, table: &'static [(&'static str, SymDef)]) -> Module {
	let mut s = Scope::new();
	for (n, (k, def)) in table.iter().enumerate() {
		crate::eval::lib::foundations::define_nth(&mut s, n, k, def_value(k, *def));
	}
	Module::new(name, s)
}

fn def_value(name: &str, def: SymDef) -> Value {
	match def {
		SymDef::One(t)		=> Value::Symbol(Symbol { variants: SymVariants::Single(t), modifiers: Arc::new(String::new()) }),
		SymDef::Many(v)		=> Value::Symbol(Symbol { variants: SymVariants::Static(v), modifiers: Arc::new(String::new()) }),
		SymDef::Module(m)	=> Value::Module(Arc::new(module(name, m))),
	}
}

/// A symbol by its dotted path in `sym` (`"arrow.r.long"`), for maths mode and shorthands.
pub fn lookup(path: &str) -> Option<Value> {
	lookup_in(SYM, path)
}

/// As [`lookup`], in `emoji`.
pub fn lookup_emoji(path: &str) -> Option<Value> {
	lookup_in(EMOJI, path)
}

fn lookup_in(table: &'static [(&'static str, SymDef)], path: &str) -> Option<Value> {
	let mut parts = path.split('.');
	let head = parts.next().unwrap_or("");
	let def = match table.iter().find(|(k, _)| *k == head) {
		Some((_, d))	=> *d,
		None			=> return None,
	};
	match def {
		SymDef::Module(m) => {
			let rest: Vec<&str> = parts.collect();
			if rest.is_empty() {
				return Some(def_value(head, def));
			}
			lookup_in(m, &rest.join("."))
		}
		_ => {
			let mut v = def_value(head, def);
			for p in parts {
				v = match &v {
					Value::Symbol(s) => match modify(s, p) {
						Some(m)	=> Value::Symbol(m),
						None	=> return None,
					},
					_ => return None,
				};
			}
			Some(v)
		}
	}
}

pub fn method(_name: &str) -> Option<SymFn> { None }

// Each variant as (modifiers, text), whatever the storage.
fn variants(s: &Symbol) -> Vec<(&str, &str)> {
	match &s.variants {
		SymVariants::Single(t)	=> vec![("", *t)],
		SymVariants::Static(v)	=> v.iter().map(|(m, t)| (*m, *t)).collect(),
		SymVariants::Runtime(v)	=> v.iter().map(|(m, t)| (m.as_str(), t.as_str())).collect(),
	}
}

fn mod_set(s: &str) -> Vec<&str> { s.split('.').filter(|m| !m.is_empty()).collect() }

// The best variant for a set of applied modifiers, per codex's `best_match_in`.
fn best<'a>(vars: &[(&'a str, &'a str)], applied: &[&str]) -> Option<&'a str> {
	let mut best: Option<(&str, usize)> = None;
	for (m, t) in vars {
		let set = mod_set(m);
		if !applied.iter().all(|a| set.contains(a)) {
			continue;
		}
		let total = set.len();
		if best.map(|(_, b)| total < b).unwrap_or(true) {
			best = Some((t, total));
		}
	}
	best.map(|(t, _)| t)
}

/// `symbol.modifier`: the symbol with one more modifier applied, or `None` when no variant has them all
/// ("unknown symbol modifier").
pub fn modify(s: &Symbol, modifier: &str) -> Option<Symbol> {
	let mut applied = mod_set(&s.modifiers);
	applied.push(modifier);
	let vars = variants(s);
	best(&vars, &applied).map(|_| Symbol { variants: s.variants.clone(), modifiers: Arc::new(applied.join(".")) })
}

/// The text a symbol stands for with its modifiers applied.
pub fn text(s: &Symbol) -> &str {
	let applied = mod_set(&s.modifiers);
	let vars = variants(s);
	best(&vars, &applied).unwrap_or("")
}

pub fn repr_symbol(s: &Symbol) -> String {
	if let SymVariants::Single(t) = &s.variants {
		return fmt!("symbol({})", repr_str(t));
	}
	let applied = mod_set(&s.modifiers);
	let vars = variants(s);
	if vars.len() == 1 && vars[0].0.is_empty() && applied.is_empty() {
		return fmt!("symbol({})", repr_str(vars[0].1));
	}
	let parts: Vec<String> = vars.iter()
		.filter(|(m, _)| {
			let set = mod_set(m);
			applied.iter().all(|a| set.contains(a))
		})
		.map(|(m, t)| {
			let rest: Vec<&str> = mod_set(m).into_iter().filter(|x| !applied.contains(x)).collect();
			if rest.is_empty() {
				repr_str(t)
			} else {
				fmt!("({}, {})", repr_str(&rest.join(".")), repr_str(t))
			}
		})
		.collect();
	fmt!("symbol{}", pretty_array_like(&parts, false))
}

pub fn call(_f: SymFn, engine: &mut Engine, mut args: Args) -> Outcome<Value> {
	let span = args.span;
	let items = res!(args.all::<Value>());
	res!(finish(engine, args));
	let mut vars: Vec<(String, String)> = Vec::with_capacity(items.len());
	for v in items {
		match v {
			Value::Str(s) => vars.push((String::new(), (*s).clone())),
			Value::Array(a) if a.len() == 2 => match (&a[0], &a[1]) {
				(Value::Str(m), Value::Str(t)) => vars.push(((**m).clone(), (**t).clone())),
				_ => return Err(engine.error(span, "variant must be a pair of strings")),
			},
			other => return Err(mismatch(engine, span, "string or array", &other)),
		}
	}
	if vars.is_empty() {
		return Err(engine.error(span, "expected at least one variant"));
	}
	for (i, (m, _)) in vars.iter().enumerate() {
		if vars[..i].iter().any(|(n, _)| {
			let (mut a, mut b) = (mod_set(n), mod_set(m));
			a.sort_unstable();
			b.sort_unstable();
			a == b
		}) {
			return Err(engine.error(span, if m.is_empty() {
				"duplicate default variant".to_string()
			} else {
				fmt!("duplicate variant: {}", repr_str(m))
			}));
		}
	}
	Ok(Value::Symbol(Symbol { variants: SymVariants::Runtime(Arc::new(vars)), modifiers: Arc::new(String::new()) }))
}
