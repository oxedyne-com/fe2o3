// U6c owns this file. Schemas for grid and table and their cells, headers, footers and lines. `grid.cell`
// and `table.cell` fields are what the per-cell fill/stroke/align closures resolve against.

use crate::eval::args::Args;
use crate::eval::content::{
	Content,
	ElemKind,
	FieldSpec,
};
use crate::eval::scope::Scope;
use crate::eval::styles::StyleChain;
use crate::eval::value::Value;
use crate::eval::Engine;

use oxedyne_fe2o3_core::prelude::*;

native_fns! {
	pub enum GridFn {
	}
}

pub fn define(_scope: &mut Scope) {}

pub fn call(f: GridFn, _engine: &mut Engine, _args: Args) -> Outcome<Value> {
	match f {}
}

pub fn fields(_kind: ElemKind) -> &'static [FieldSpec] { &[] }

pub fn construct(_engine: &mut Engine, _kind: ElemKind, _args: &mut Args) -> Outcome<Option<Content>> {
	Ok(None)
}

pub fn show(_engine: &mut Engine, _elem: &Content, _styles: &StyleChain) -> Outcome<Option<Content>> {
	Ok(None)
}
