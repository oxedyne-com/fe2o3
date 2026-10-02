// SPDX-License-Identifier: Apache-2.0
// Original work copyright (c) the Typst project authors (typst-library `model/link.rs`, version 0.15.1).
// Modified for Austenite: the element's fields, defaults and default show, ported to Hematite's types, IR and error handling.
// U5 owns this file: link. The show leaves the body in place under the internal style `link.current`,
// the resolved destination -- a URL string, a location, or a position dictionary -- which inline flow
// reads to make the body's text a link area, as Typst's `LinkElem::current` is read. A label destination
// is resolved to the labelled element's location here. Typst's alternative text for a link, for tagged
// PDF, is not produced.

use crate::diag::DiagnosticKind;
use crate::eval::args::Args;
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
use crate::eval::lib::model::lookup;
use crate::eval::styles::{
	StyleChain,
	Styles,
};
use crate::eval::value::Value;
use crate::eval::Engine;

use oxedyne_fe2o3_core::prelude::*;

const ANY: FieldType = FieldType::Any;

static LINK: [FieldSpec; 3] = [
	FieldSpec::required("dest",		ANY),
	FieldSpec::required("body",		ANY),
	FieldSpec::named("current",		ANY,	FieldDefault::None).internal(),
];

pub fn fields(kind: ElemKind) -> &'static [FieldSpec] {
	match kind {
		ElemKind::Link	=> &LINK,
		_				=> &[],
	}
}

pub fn cast(_kind: ElemKind, name: &str, v: Value) -> Result<Value, CastErr> {
	match name {
		"dest"	=> dest(v),
		"body"	=> expect(&v, &[K::Content]).map(|_| to_content(v)),
		_		=> Ok(v),
	}
}

/// A destination: a URL, a label, a location, or a position (`(page: 1, x: 0pt, y: 0pt)`).
fn dest(v: Value) -> Result<Value, CastErr> {
	match &v {
		Value::Str(s) if s.is_empty()		=> Err(CastErr::Value("URL must not be empty".to_string())),
		Value::Str(s) if s.len() > 8000		=> Err(CastErr::Value("URL is too long".to_string())),
		Value::Str(_) | Value::Label(_) | Value::Location(_)	=> Ok(v),
		Value::Dict(d) => {
			for key in ["page", "x", "y"] {
				if !d.contains(key) {
					return Err(CastErr::Value(fmt!("dictionary does not contain key \"{}\"", key)));
				}
			}
			Ok(v)
		}
		other => Err(CastErr::Type(common::mismatch("string, dictionary, location, or label", other))),
	}
}

/// The text a URL shows when no body is given: the URL, less a `mailto:` or `tel:` scheme.
pub fn body_from_url(url: &str) -> Content {
	let shown = url.strip_prefix("mailto:").or_else(|| url.strip_prefix("tel:")).unwrap_or(url);
	common::text(shown)
}

/// `link(dest, body)`: the body may be left out after a URL, which then shows itself.
pub fn construct(engine: &mut Engine, args: &mut Args) -> Outcome<Content> {
	let span = args.span;
	let a = match args.items.iter().position(|a| a.name.is_none()) {
		Some(p)	=> args.items.remove(p),
		None	=> return Err(engine.error(DiagnosticKind::Type, span, "missing argument: dest")),
	};
	let d = match dest(a.value) {
		Ok(d)	=> d,
		Err(e)	=> return Err(engine.error(DiagnosticKind::Type, a.span, e.message())),
	};
	let next = args.items.iter().position(|a| a.name.is_none()).map(|p| args.items.remove(p));
	let body = match (next, &d) {
		(Some(b), _) => match expect(&b.value, &[K::Content]) {
			Ok(())	=> to_content(b.value),
			Err(e)	=> return Err(engine.error(DiagnosticKind::Type, b.span, e.message())),
		},
		(None, Value::Str(url))	=> Value::Content(body_from_url(url).with_span(span)),
		(None, _)				=> return Err(engine.error(DiagnosticKind::Type, span, "missing argument: body")),
	};
	res!(super::finish(engine, args));
	common::raw_elem(ElemKind::Link, span, vec![("dest", d), ("body", body)])
}

pub fn show_set() -> Outcome<Styles> {
	common::props(vec![(ElemKind::Text, "hyphenate", Value::Bool(false))])
}

pub fn show(engine: &mut Engine, elem: &Content, _styles: &StyleChain) -> Outcome<Option<Content>> {
	let span = elem.span();
	let body = common::body(elem, "body");
	let dest = match elem.field("dest") {
		Some(Value::Label(l)) => {
			let l = l.clone();
			let target = res!(lookup::label(engine, &l, span));
			match target.location() {
				Some(loc)	=> Value::Location(loc),
				None		=> return Err(engine.error(DiagnosticKind::Type, span, "cannot link to an element without a location")),
			}
		}
		Some(v)	=> v.clone(),
		None	=> Value::None,
	};
	Ok(Some(res!(common::set(body, ElemKind::Link, "current", dest))))
}

/// Content linked to a location, as Typst's `DirectLinkElem` makes a reference or an outline entry.
pub fn to_location(body: Content, loc: crate::eval::locate::Location) -> Outcome<Content> {
	common::set(body, ElemKind::Link, "current", Value::Location(loc))
}
