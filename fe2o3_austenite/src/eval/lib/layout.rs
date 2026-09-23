// U6b owns this file. Schemas for the layout family (box, block, align, pad, stack, h, v, place, columns,
// colbreak, pagebreak, page), with Typst's defaults. All are primitives flow lays out, so `show` is
// `None`.

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
	pub enum LayoutFn {
	}
}

pub fn define(_scope: &mut Scope) {}

pub fn call(f: LayoutFn, _engine: &mut Engine, _args: Args) -> Outcome<Value> {
	match f {}
}

pub fn fields(_kind: ElemKind) -> &'static [FieldSpec] { &[] }

pub fn construct(_engine: &mut Engine, _kind: ElemKind, _args: &mut Args) -> Outcome<Option<Content>> {
	Ok(None)
}

pub fn show(_engine: &mut Engine, _elem: &Content, _styles: &StyleChain) -> Outcome<Option<Content>> {
	Ok(None)
}
