// U3 owns this file. Methods on `dictionary`; `insert` and `remove` mutate their receiver.

use crate::eval::args::Args;
use crate::eval::func::unimplemented;
use crate::eval::scope::Scope;
use crate::eval::value::Value;
use crate::eval::Engine;

use oxedyne_fe2o3_core::prelude::*;

native_fns! {
	pub enum DictFn {
		Len			=> "len",
		At			=> "at",
		Insert		=> "insert",
		Remove		=> "remove",
		Keys		=> "keys",
		Values		=> "values",
		Pairs		=> "pairs",
	}
}

pub fn define(_scope: &mut Scope) {}

pub fn method(_name: &str) -> Option<DictFn> { None }

pub fn call(f: DictFn, _engine: &mut Engine, _args: Args) -> Outcome<Value> {
	Err(unimplemented("dictionary", f.name()))
}

impl DictFn {
	pub fn mutates(self) -> bool { matches!(self, DictFn::Insert | DictFn::Remove) }
}
