// U3 owns this file. The `sym` and `emoji` modules as static tables (migrated from `lang/mathparse.rs`
// before U11 deletes it), `symbol(..)`, and modifier resolution (`arrow.r.long`).

use crate::eval::args::Args;
use crate::eval::func::unimplemented;
use crate::eval::scope::Scope;
use crate::eval::value::Value;
use crate::eval::Engine;

use oxedyne_fe2o3_core::prelude::*;

native_fns! {
	pub enum SymFn {
		Symbol		=> "symbol",
	}
}

pub fn define(_scope: &mut Scope) {}

pub fn method(_name: &str) -> Option<SymFn> { None }

pub fn call(f: SymFn, _engine: &mut Engine, _args: Args) -> Outcome<Value> {
	Err(unimplemented("sym", f.name()))
}
