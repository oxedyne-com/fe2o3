// U3 owns this file. The `calc` module: `define` binds `calc` to a module whose scope holds these, plus
// `pi`, `tau`, `e`, `inf` and `nan`.

use crate::eval::args::Args;
use crate::eval::func::unimplemented;
use crate::eval::scope::Scope;
use crate::eval::value::Value;
use crate::eval::Engine;

use oxedyne_fe2o3_core::prelude::*;

native_fns! {
	pub enum CalcFn {
		Abs				=> "abs",
		Pow				=> "pow",
		Exp				=> "exp",
		Sqrt			=> "sqrt",
		Root			=> "root",
		Sin				=> "sin",
		Cos				=> "cos",
		Tan				=> "tan",
		Asin			=> "asin",
		Acos			=> "acos",
		Atan			=> "atan",
		Atan2			=> "atan2",
		Sinh			=> "sinh",
		Cosh			=> "cosh",
		Tanh			=> "tanh",
		Log				=> "log",
		Ln				=> "ln",
		Fact			=> "fact",
		Perm			=> "perm",
		Binom			=> "binom",
		Gcd				=> "gcd",
		Lcm				=> "lcm",
		Floor			=> "floor",
		Ceil			=> "ceil",
		Trunc			=> "trunc",
		Fract			=> "fract",
		Round			=> "round",
		Clamp			=> "clamp",
		Min				=> "min",
		Max				=> "max",
		Even			=> "even",
		Odd				=> "odd",
		Rem				=> "rem",
		DivEuclid		=> "div-euclid",
		RemEuclid		=> "rem-euclid",
		Quo				=> "quo",
		Norm			=> "norm",
	}
}

pub fn define(_scope: &mut Scope) {}

pub fn call(f: CalcFn, _engine: &mut Engine, _args: Args) -> Outcome<Value> {
	Err(unimplemented("calc", f.name()))
}
