// U3 owns this file. `datetime` and `duration`: constructors and methods. `today` reads the host clock
// once per compilation, so a document is reproducible within one compile.

use crate::eval::args::Args;
use crate::eval::func::unimplemented;
use crate::eval::scope::Scope;
use crate::eval::value::Value;
use crate::eval::Engine;

use oxedyne_fe2o3_core::prelude::*;

native_fns! {
	pub enum DatetimeFn {
		Datetime	=> "datetime",
		Today		=> "today",
		Display		=> "display",
		Year		=> "year",
		Month		=> "month",
		Weekday		=> "weekday",
		Day			=> "day",
		Hour		=> "hour",
		Minute		=> "minute",
		Second		=> "second",
		Ordinal		=> "ordinal",
		Duration	=> "duration",
		Seconds		=> "seconds",
		Minutes		=> "minutes",
		Hours		=> "hours",
		Days		=> "days",
		Weeks		=> "weeks",
	}
}

pub fn define(_scope: &mut Scope) {}

pub fn method(_name: &str) -> Option<DatetimeFn> { None }

pub fn call(f: DatetimeFn, _engine: &mut Engine, _args: Args) -> Outcome<Value> {
	Err(unimplemented("datetime", f.name()))
}
