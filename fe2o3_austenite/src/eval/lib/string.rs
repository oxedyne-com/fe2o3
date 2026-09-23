// U3 owns this file. Methods on `str` (and `str.to-unicode`/`str.from-unicode`); regex matching uses
// `fe2o3_text::regex` (U3b extends it).

use crate::eval::args::Args;
use crate::eval::func::unimplemented;
use crate::eval::scope::Scope;
use crate::eval::value::Value;
use crate::eval::Engine;

use oxedyne_fe2o3_core::prelude::*;

native_fns! {
	pub enum StrFn {
		Len				=> "len",
		First			=> "first",
		Last			=> "last",
		At				=> "at",
		Slice			=> "slice",
		Clusters		=> "clusters",
		Codepoints		=> "codepoints",
		Contains		=> "contains",
		StartsWith		=> "starts-with",
		EndsWith		=> "ends-with",
		Find			=> "find",
		Position		=> "position",
		Match			=> "match",
		Matches			=> "matches",
		Replace			=> "replace",
		Rev				=> "rev",
		Split			=> "split",
		Trim			=> "trim",
		ToUnicode		=> "to-unicode",
		FromUnicode		=> "from-unicode",
		Normalize		=> "normalize",
	}
}

pub fn define(_scope: &mut Scope) {}

pub fn method(_name: &str) -> Option<StrFn> { None }

pub fn call(f: StrFn, _engine: &mut Engine, _args: Args) -> Outcome<Value> {
	Err(unimplemented("str", f.name()))
}
