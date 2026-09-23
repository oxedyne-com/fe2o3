// U2 owns this file: method dispatch by receiver type. A method belongs to the area that owns its
// receiver's type (`str` -> `lib/string.rs`, `counter` -> `lib/intro.rs`, `selector` -> `select.rs`); each
// such area exposes `method(name) -> Option<XFn>`, and this file maps receiver type to area. Methods that
// mutate their receiver (`array.push`, `dict.insert`) come through `call_method_mut`.

use crate::eval::args::Args;
use crate::eval::func::unimplemented;
use crate::eval::value::Value;
use crate::eval::Engine;
use crate::syntax::Span;

use oxedyne_fe2o3_core::prelude::*;

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

pub fn call(f: CoreFn, _engine: &mut Engine, _args: Args) -> Outcome<Value> {
	Err(unimplemented("core", f.name()))
}

/// `receiver.name(args)` for a receiver the call does not mutate.
pub fn call_method(
	_engine:	&mut Engine,
	_receiver:	Value,
	name:		&str,
	_args:		Args,
	_span:		Span,
)
	-> Outcome<Value>
{
	Err(unimplemented("method", name))
}

/// `receiver.name(args)` where the receiver is a place (`arr.push(x)`) and may be changed in it.
pub fn call_method_mut(
	_engine:	&mut Engine,
	_receiver:	&mut Value,
	name:		&str,
	_args:		Args,
	_span:		Span,
)
	-> Outcome<Value>
{
	Err(unimplemented("method", name))
}
