// SPDX-License-Identifier: Apache-2.0
// Original work copyright (c) the Typst project authors (typst-library `model/par.rs`, version 0.15.1).
// Modified for Austenite: the element's fields, defaults and default show, ported to Hematite's types, IR and error handling.
// U5 owns this file: par, par.line and parbreak, primitives the flow sets. `par.line`'s fields are
// style-only (Typst's ghost fields), set by `set par.line(..)` and read by flow for line numbering.

use crate::eval::content::{
	ElemKind,
	FieldDefault,
	FieldSpec,
	FieldType,
	Fold,
};
use crate::eval::lib::model::common::{
	choice,
	expect,
	pt,
	to_content,
	CastErr,
	K,
};
use crate::eval::value::{
	Dict,
	Ratio,
	Relative,
	Length,
	Type,
	Value,
};

use oxedyne_fe2o3_core::prelude::*;

const ANY:		FieldType = FieldType::Any;
const BOOL:		FieldType = FieldType::Of(Type::Bool);
const LENGTH:	FieldType = FieldType::Of(Type::Length);

static PAR: [FieldSpec; 8] = [
	FieldSpec::named("leading",					LENGTH,	FieldDefault::Em(0.65)),
	FieldSpec::named("spacing",					LENGTH,	FieldDefault::Em(1.2)),
	FieldSpec::named("justify",					BOOL,	FieldDefault::Bool(false)),
	FieldSpec::named("justification-limits",	ANY,	FieldDefault::Computed).fold(Fold::Custom),
	FieldSpec::named("linebreaks",				ANY,	FieldDefault::Auto),
	FieldSpec::named("first-line-indent",		ANY,	FieldDefault::Computed).fold(Fold::Keyed("amount")),
	FieldSpec::named("hanging-indent",			LENGTH,	FieldDefault::Pt(0.0)),
	FieldSpec::required("body",					FieldType::Content),
];

static PAR_LINE: [FieldSpec; 5] = [
	FieldSpec::named("numbering",			ANY,	FieldDefault::None),
	FieldSpec::named("number-align",		ANY,	FieldDefault::Auto),
	FieldSpec::named("number-margin",		ANY,	FieldDefault::Computed),
	FieldSpec::named("number-clearance",	ANY,	FieldDefault::Auto),
	FieldSpec::named("numbering-scope",		ANY,	FieldDefault::Str("document")),
];

pub fn fields(kind: ElemKind) -> &'static [FieldSpec] {
	match kind {
		ElemKind::Par		=> &PAR,
		ElemKind::ParLine	=> &PAR_LINE,
		_					=> &[],
	}
}

pub fn default_value(kind: ElemKind, name: &str) -> Option<Value> {
	match (kind, name) {
		(ElemKind::Par, "justification-limits") => {
			let mut spacing = Dict::new();
			spacing.insert("min", rel(2.0 / 3.0));
			spacing.insert("max", rel(1.5));
			let mut tracking = Dict::new();
			tracking.insert("min", pt(0.0));
			tracking.insert("max", pt(0.0));
			let mut d = Dict::new();
			d.insert("spacing", Value::dict(spacing));
			d.insert("tracking", Value::dict(tracking));
			Some(Value::dict(d))
		}
		(ElemKind::Par, "first-line-indent") => {
			let mut d = Dict::new();
			d.insert("amount", pt(0.0));
			d.insert("all", Value::Bool(false));
			Some(Value::dict(d))
		}
		(ElemKind::ParLine, "number-margin") => Some(Value::Alignment(crate::eval::value::Alignment {
			x:	Some(crate::eval::value::HAlign::Start),
			y:	None,
		})),
		_ => None,
	}
}

fn rel(r: f64) -> Value { Value::Relative(Relative { abs: Length::zero(), rel: Ratio(r) }) }

pub fn cast(kind: ElemKind, name: &str, v: Value) -> Result<Value, CastErr> {
	match (kind, name) {
		(ElemKind::Par, "leading") | (ElemKind::Par, "spacing") | (ElemKind::Par, "hanging-indent")
			=> expect(&v, &[K::Length]).map(|_| v),
		(ElemKind::Par, "justify")				=> expect(&v, &[K::Bool]).map(|_| v),
		(ElemKind::Par, "justification-limits")	=> limits(v),
		(ElemKind::Par, "linebreaks")			=> choice(&v, &["simple", "optimized"], true, false).map(|_| v),
		(ElemKind::Par, "first-line-indent")	=> first_line_indent(v),
		(ElemKind::Par, "body")					=> expect(&v, &[K::Content]).map(|_| to_content(v)),
		(ElemKind::ParLine, "numbering")		=> expect(&v, &[K::Str, K::Func, K::None]).map(|_| v),
		(ElemKind::ParLine, "number-align")		=> expect(&v, &[K::Alignment, K::Auto]).map(|_| v),
		(ElemKind::ParLine, "number-margin")	=> expect(&v, &[K::Alignment]).map(|_| v),
		(ElemKind::ParLine, "number-clearance")	=> expect(&v, &[K::Length, K::Auto]).map(|_| v),
		(ElemKind::ParLine, "numbering-scope")	=> choice(&v, &["document", "page"], false, false).map(|_| v),
		_										=> Ok(v),
	}
}

/// A length is the amount alone; a dictionary may give `amount` and `all`, nothing else.
fn first_line_indent(v: Value) -> Result<Value, CastErr> {
	match &v {
		Value::Length(_) => {
			let mut d = Dict::new();
			d.insert("amount", v);
			Ok(Value::dict(d))
		}
		Value::Dict(d) => {
			for k in d.keys() {
				if k != "amount" && k != "all" {
					return Err(CastErr::Value(fmt!(
						"unexpected key \"{}\", valid keys are \"amount\" and \"all\"", k)));
				}
			}
			if let Some(a) = d.get("amount") {
				if let Err(e) = expect(a, &[K::Length]) {
					return Err(e);
				}
			}
			if let Some(a) = d.get("all") {
				if let Err(e) = expect(a, &[K::Bool]) {
					return Err(e);
				}
			}
			Ok(v)
		}
		other => Err(CastErr::Type(super::common::mismatch("length or dictionary", other))),
	}
}

/// `justification-limits`: a dictionary of `spacing` and `tracking`, each a `min`/`max` dictionary.
fn limits(v: Value) -> Result<Value, CastErr> {
	let d = match &v {
		Value::Dict(d)	=> d.clone(),
		other			=> return Err(CastErr::Type(super::common::mismatch("dictionary", other))),
	};
	for k in d.keys() {
		if k != "spacing" && k != "tracking" {
			return Err(CastErr::Value(fmt!(
				"unexpected key \"{}\", valid keys are \"spacing\" and \"tracking\"", k)));
		}
	}
	for (key, want) in [("spacing", K::Rel), ("tracking", K::Length)] {
		if let Some(x) = d.get(key) {
			let inner = match x {
				Value::Dict(i)	=> i.clone(),
				other			=> return Err(CastErr::Type(super::common::mismatch("dictionary", other))),
			};
			for m in ["min", "max"] {
				match inner.get(m) {
					Some(val) => if let Err(e) = expect(val, &[want]) {
						return Err(CastErr::Value(fmt!("`{}` value of `{}` is invalid ({})", m, key, e.message())));
					},
					None => return Err(CastErr::Value(fmt!("dictionary does not contain key \"{}\"", m))),
				}
			}
		}
	}
	Ok(v)
}
