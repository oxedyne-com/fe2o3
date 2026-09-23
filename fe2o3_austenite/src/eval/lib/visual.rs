// U6d owns this file. Schemas for images, shapes, curves and transforms, and the `stroke` and `tiling`
// constructors.

use crate::eval::args::Args;
use crate::eval::content::{
	Content,
	ElemKind,
	FieldSpec,
};
use crate::eval::func::unimplemented;
use crate::eval::scope::Scope;
use crate::eval::styles::StyleChain;
use crate::eval::value::Value;
use crate::eval::Engine;

use oxedyne_fe2o3_core::prelude::*;

native_fns! {
	pub enum VisualFn {
		Stroke		=> "stroke",
		Tiling		=> "tiling",
	}
}

pub fn define(_scope: &mut Scope) {}

pub fn call(f: VisualFn, _engine: &mut Engine, _args: Args) -> Outcome<Value> {
	Err(unimplemented("visual", f.name()))
}

pub fn fields(_kind: ElemKind) -> &'static [FieldSpec] { &[] }

pub fn construct(_engine: &mut Engine, _kind: ElemKind, _args: &mut Args) -> Outcome<Option<Content>> {
	Ok(None)
}

pub fn show(_engine: &mut Engine, _elem: &Content, _styles: &StyleChain) -> Outcome<Option<Content>> {
	Ok(None)
}
