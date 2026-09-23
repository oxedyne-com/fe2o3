// U2 owns this file: the tree-walking evaluator. Its public surface is `eval_module`, `eval_string` and
// `Engine::call_func`; the walker's own state (a VM with the scope chain and flow control) is U2's.

use crate::eval::args::Args;
use crate::eval::func::{
	unimplemented,
	Func,
};
use crate::eval::scope::Scope;
use crate::eval::value::{
	Module,
	Value,
};
use crate::eval::Engine;
use crate::syntax::{
	FileId,
	Span,
};

use oxedyne_fe2o3_core::prelude::*;

/// Which mode `eval(..)` parses its string in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EvalMode {
	Markup,
	Code,
	Math,
}

/// Evaluates a loaded source: its top-level bindings become the module's scope, its markup (with every
/// `set`/`show` applied to the rest of its scope) the module's content.
pub fn eval_module(_engine: &mut Engine, _id: FileId) -> Outcome<Module> {
	Err(unimplemented("eval", "module"))
}

/// `eval(text, mode:, scope:)`: evaluates a string against the standard library plus `scope`.
pub fn eval_string(
	_engine:	&mut Engine,
	_text:		&str,
	_mode:		EvalMode,
	_scope:		Scope,
	_span:		Span,
)
	-> Outcome<Value>
{
	Err(unimplemented("eval", "string"))
}

impl Engine {
	/// Calls any function value: native, element constructor, closure or `.with`. The one call path
	/// realisation (show-rule closures), the library (`array.map`) and flow (`layout`) all use.
	pub fn call_func(&mut self, func: &Func, _args: Args) -> Outcome<Value> {
		Err(unimplemented("call", func.name().unwrap_or("closure")))
	}
}
