// SPDX-License-Identifier: Apache-2.0
// Original work copyright (c) the Typst project authors (typst-library `model/document.rs`, version 0.15.1).
// Modified for Austenite: the element's fields, defaults and default show, ported to Hematite's types, IR and error handling.
// U5 owns this file: document. In Typst 0.15 a document element is only constructed in the bundle
// target; in a paged document `set document(..)` carries the metadata (title, author, ...) that the PDF
// writer and `title()` read from the style chain.

use crate::diag::DiagnosticKind;
use crate::eval::content::{
	Content,
	ElemKind,
	FieldDefault,
	FieldSpec,
	FieldType,
};
use crate::eval::lib::model::common::{
	self,
	choice,
	expect,
	expect_cast,
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

const ANY: FieldType = FieldType::Any;

static DOCUMENT: [FieldSpec; 8] = [
	FieldSpec::required("path",			FieldType::Of(Type::Str)),
	FieldSpec::named("format",			ANY,	FieldDefault::Auto),
	FieldSpec::named("title",			ANY,	FieldDefault::None),
	FieldSpec::named("author",			ANY,	FieldDefault::EmptyArray),
	FieldSpec::named("description",		ANY,	FieldDefault::None),
	FieldSpec::named("keywords",		ANY,	FieldDefault::EmptyArray),
	FieldSpec::named("date",			ANY,	FieldDefault::Auto),
	FieldSpec::required("body",			ANY),
];

pub fn fields(kind: ElemKind) -> &'static [FieldSpec] {
	match kind {
		ElemKind::Document	=> &DOCUMENT,
		_					=> &[],
	}
}

pub fn cast(_kind: ElemKind, name: &str, v: Value) -> Result<Value, CastErr> {
	match name {
		"path"		=> expect_cast(v, &[K::Str]),
		"format"	=> choice(&v, &["pdf", "png", "svg", "html"], true, false).map(|_| v),
		"title" | "description" => match v {
			Value::None	=> Ok(v),
			other		=> expect(&other, &[K::Content, K::None]).map(|_| to_content(other)),
		},
		"author" | "keywords" => match v {
			Value::Str(_)	=> Ok(Value::array(vec![v])),
			Value::Array(a)	=> {
				for x in a.iter() {
					ok!(expect(x, &[K::Str]));
				}
				Ok(Value::Array(a))
			}
			other => Err(CastErr::Type(common::mismatch("string or array", &other))),
		},
		"date"		=> match v {
			Value::Datetime(_) | Value::None | Value::Auto	=> Ok(v),
			other	=> Err(CastErr::Type(common::mismatch("datetime, none, or auto", &other))),
		},
		"body"		=> expect(&v, &[K::Content]).map(|_| to_content(v)),
		_			=> Ok(v),
	}
}

pub fn show(engine: &mut Engine, elem: &Content, _styles: &StyleChain) -> Outcome<Option<Content>> {
	Err(engine.error_hint(DiagnosticKind::Type, elem.span(), "constructing a document is only supported in the bundle target",
		"or use a `set document(..)` rule to configure metadata"))
}
