// U3 owns this file. Data loading through `crate::vfs`, paths resolved with `import::resolve_path`. A
// format with no reader yet is a diagnostic, never an empty value.

use crate::eval::args::Args;
use crate::eval::func::unimplemented;
use crate::eval::scope::Scope;
use crate::eval::value::Value;
use crate::eval::Engine;

use oxedyne_fe2o3_core::prelude::*;

native_fns! {
	pub enum DataFn {
		Read	=> "read",
		Json	=> "json",
		Csv		=> "csv",
		Yaml	=> "yaml",
		Toml	=> "toml",
		Xml		=> "xml",
		Cbor	=> "cbor",
	}
}

pub fn define(_scope: &mut Scope) {}

pub fn call(f: DataFn, _engine: &mut Engine, _args: Args) -> Outcome<Value> {
	Err(unimplemented("data", f.name()))
}
