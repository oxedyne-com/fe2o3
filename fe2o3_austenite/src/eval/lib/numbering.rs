// U3 owns this file. `numbering(pattern, ..nums)`: the one implementation of Typst numbering patterns
// (`"1.a)"`, `"I"`, `"*"`); heading, figure, list, footnote and page numbering (U5, U8) all call `apply`.

use crate::eval::args::Args;
use crate::eval::func::unimplemented;
use crate::eval::scope::Scope;
use crate::eval::value::Value;
use crate::eval::Engine;

use oxedyne_fe2o3_core::prelude::*;

native_fns! {
	pub enum NumberingFn {
		Numbering		=> "numbering",
	}
}

pub fn define(_scope: &mut Scope) {}

pub fn call(f: NumberingFn, _engine: &mut Engine, _args: Args) -> Outcome<Value> {
	Err(unimplemented("numbering", f.name()))
}

/// Formats `nums` with a pattern string (`"1.1"`) or calls a numbering function.
pub fn apply(_engine: &mut Engine, _numbering: &Value, _nums: &[u64]) -> Outcome<Value> {
	Err(unimplemented("numbering", "apply"))
}
