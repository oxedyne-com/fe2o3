// U3 owns this file. Methods and fields on `length`, `angle`, `ratio`, `relative`, `fraction`, `alignment`
// and `direction`, and the alignment/direction globals (`left`, `top`, `ltr`...).

use crate::eval::args::Args;
use crate::eval::func::unimplemented;
use crate::eval::scope::Scope;
use crate::eval::value::Value;
use crate::eval::Engine;

use oxedyne_fe2o3_core::prelude::*;

native_fns! {
	pub enum GeomFn {
		Pt				=> "pt",
		Mm				=> "mm",
		Cm				=> "cm",
		Inches			=> "inches",
		ToAbsolute		=> "to-absolute",
		Deg				=> "deg",
		Rad				=> "rad",
		Axis			=> "axis",
		Inv				=> "inv",
		Start			=> "start",
		End				=> "end",
		Sign			=> "sign",
		X				=> "x",
		Y				=> "y",
	}
}

pub fn define(_scope: &mut Scope) {}

pub fn method(_name: &str) -> Option<GeomFn> { None }

pub fn call(f: GeomFn, _engine: &mut Engine, _args: Args) -> Outcome<Value> {
	Err(unimplemented("geom", f.name()))
}
