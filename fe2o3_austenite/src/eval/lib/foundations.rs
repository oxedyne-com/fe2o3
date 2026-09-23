// U3 owns this file. Foundations: `type`, `repr`, `assert`, `panic`, `eval`, `range`, the type
// constructors, `plugin` (refused with a diagnostic, owner decision 3), and methods on version and bytes.
// `repr` is the one text form every other unit prints a value by.

use crate::eval::args::Args;
use crate::eval::func::unimplemented;
use crate::eval::scope::Scope;
use crate::eval::value::Value;
use crate::eval::Engine;

use oxedyne_fe2o3_core::prelude::*;

native_fns! {
	pub enum FoundFn {
		Type			=> "type",
		Repr			=> "repr",
		Assert			=> "assert",
		AssertEq		=> "assert.eq",
		AssertNe		=> "assert.ne",
		Panic			=> "panic",
		Eval			=> "eval",
		Range			=> "range",
		Int				=> "int",
		Float			=> "float",
		Str				=> "str",
		Bool			=> "bool",
		Label			=> "label",
		Regex			=> "regex",
		Version			=> "version",
		Bytes			=> "bytes",
		Arguments		=> "arguments",
		Plugin			=> "plugin",
		Decimal			=> "decimal",
		VersionAt		=> "at",
		BytesLen		=> "len",
		BytesAt			=> "at",
		BytesSlice		=> "slice",
	}
}

pub fn define(_scope: &mut Scope) {}

pub fn method(_name: &str) -> Option<FoundFn> { None }

pub fn call(f: FoundFn, _engine: &mut Engine, _args: Args) -> Outcome<Value> {
	Err(unimplemented("std", f.name()))
}

/// `repr(value)`, as Typst prints it. Stub: the type name in angle brackets until U3 lands.
pub fn repr(v: &Value) -> String {
	fmt!("<{}>", v.ty().name())
}
