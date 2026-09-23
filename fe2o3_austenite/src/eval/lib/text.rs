// U6a owns this file: schemas for the text family and `lorem`, `upper`, `lower`. One fixed point: the
// `text` element's field 0 is its string, which `Content::text` and `plain_text` rely on; its style
// fields (font, size, weight, fill, lang, ...) follow it with Typst's defaults.

use crate::eval::args::Args;
use crate::eval::content::{
	Content,
	ElemKind,
	FieldSpec,
	FieldType,
};
use crate::eval::func::unimplemented;
use crate::eval::scope::Scope;
use crate::eval::styles::StyleChain;
use crate::eval::value::{
	Type,
	Value,
};
use crate::eval::Engine;

use oxedyne_fe2o3_core::prelude::*;

native_fns! {
	pub enum TextFn {
		Lorem	=> "lorem",
		Upper	=> "upper",
		Lower	=> "lower",
	}
}

const TEXT: &[FieldSpec] = &[
	FieldSpec::required("text", FieldType::Of(Type::Str)),
];

// By contract field 0, as `Content::symbol` builds it.
const SYMBOL: &[FieldSpec] = &[
	FieldSpec::required("text", FieldType::Of(Type::Str)),
];

pub fn define(_scope: &mut Scope) {}

pub fn call(f: TextFn, _engine: &mut Engine, _args: Args) -> Outcome<Value> {
	Err(unimplemented("text", f.name()))
}

pub fn fields(kind: ElemKind) -> &'static [FieldSpec] {
	match kind {
		ElemKind::Text		=> TEXT,
		ElemKind::Symbol	=> SYMBOL,
		_					=> &[],
	}
}

pub fn construct(_engine: &mut Engine, _kind: ElemKind, _args: &mut Args) -> Outcome<Option<Content>> {
	Ok(None)
}

pub fn show(_engine: &mut Engine, _elem: &Content, _styles: &StyleChain) -> Outcome<Option<Content>> {
	Ok(None)
}
