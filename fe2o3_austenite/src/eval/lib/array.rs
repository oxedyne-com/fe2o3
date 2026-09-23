// U3 owns this file. Methods on `array`. `push`, `pop`, `insert` and `remove` mutate their receiver and
// arrive through `methods::call_method_mut`; `mutates` says which.

use crate::eval::args::Args;
use crate::eval::func::unimplemented;
use crate::eval::scope::Scope;
use crate::eval::value::Value;
use crate::eval::Engine;

use oxedyne_fe2o3_core::prelude::*;

native_fns! {
	pub enum ArrayFn {
		Len				=> "len",
		First			=> "first",
		Last			=> "last",
		At				=> "at",
		Push			=> "push",
		Pop				=> "pop",
		Insert			=> "insert",
		Remove			=> "remove",
		Slice			=> "slice",
		Contains		=> "contains",
		Find			=> "find",
		Position		=> "position",
		Filter			=> "filter",
		Map				=> "map",
		Enumerate		=> "enumerate",
		Zip				=> "zip",
		Fold			=> "fold",
		Reduce			=> "reduce",
		Sum				=> "sum",
		Product			=> "product",
		Any				=> "any",
		All				=> "all",
		Flatten			=> "flatten",
		Rev				=> "rev",
		Split			=> "split",
		Join			=> "join",
		Intersperse		=> "intersperse",
		Chunks			=> "chunks",
		Windows			=> "windows",
		Sorted			=> "sorted",
		Dedup			=> "dedup",
		ToDict			=> "to-dict",
	}
}

pub fn define(_scope: &mut Scope) {}

pub fn method(_name: &str) -> Option<ArrayFn> { None }

pub fn call(f: ArrayFn, _engine: &mut Engine, _args: Args) -> Outcome<Value> {
	Err(unimplemented("array", f.name()))
}

impl ArrayFn {
	pub fn mutates(self) -> bool {
		matches!(self, ArrayFn::Push | ArrayFn::Pop | ArrayFn::Insert | ArrayFn::Remove)
	}
}
