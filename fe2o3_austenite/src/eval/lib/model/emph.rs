// SPDX-License-Identifier: Apache-2.0
// Original work copyright (c) the Typst project authors (typst-library `model/{emph,strong}.rs`, version 0.15.1).
// Modified for Austenite: the element's fields, defaults and default show, ported to Hematite's types, IR and error handling.
// U5 owns this file: strong and emph. Neither picks a font: `strong` adds its `delta` to the weight in
// force and `emph` toggles italics, through `text`'s internal `delta` and `emph` style fields, so nested
// emphasis cancels as Typst's does.

use crate::eval::content::{
	Content,
	ElemKind,
	FieldDefault,
	FieldSpec,
	FieldType,
};
use crate::eval::lib::model::common::{
	self,
	expect,
	to_content,
	CastErr,
	K,
};
use crate::eval::styles::StyleChain;
use crate::eval::value::{
	Type,
	Value,
};
use crate::eval::Engine;

use oxedyne_fe2o3_core::prelude::*;

static STRONG: [FieldSpec; 2] = [
	FieldSpec::named("delta",	FieldType::Of(Type::Int),	FieldDefault::Int(300)),
	FieldSpec::required("body",	FieldType::Any),
];

static EMPH: [FieldSpec; 1] = [
	FieldSpec::required("body",	FieldType::Any),
];

pub fn fields(kind: ElemKind) -> &'static [FieldSpec] {
	match kind {
		ElemKind::Strong	=> &STRONG,
		ElemKind::Emph		=> &EMPH,
		_					=> &[],
	}
}

pub fn cast(_kind: ElemKind, name: &str, v: Value) -> Result<Value, CastErr> {
	match name {
		"delta"	=> expect(&v, &[K::Int]).map(|_| v),
		"body"	=> expect(&v, &[K::Content]).map(|_| to_content(v)),
		_		=> Ok(v),
	}
}

pub fn show(_engine: &mut Engine, elem: &Content, styles: &StyleChain) -> Outcome<Option<Content>> {
	let body = common::body(elem, "body");
	match elem.kind() {
		Some(ElemKind::Strong) => {
			let delta = res!(common::get(elem, styles, "delta"));
			Ok(Some(res!(common::set(body, ElemKind::Text, "delta", delta))))
		}
		_ => Ok(Some(res!(common::set(body, ElemKind::Text, "emph", Value::Bool(true))))),
	}
}
