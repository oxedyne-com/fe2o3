// SPDX-License-Identifier: Apache-2.0
// Original work copyright (c) the Typst project authors (typst-library `model/quote.rs`, version 0.15.1).
// Modified for Austenite: the element's fields, defaults and default show, ported to Hematite's types, IR and error handling.
// U5 owns this file: quote. An inline quote is wrapped in the language's quotation marks, alternating
// double and single with nesting (`quote.depth`, a style-only field); a block quote is padded, with its
// attribution set below it after an em dash. The quotation marks by language and region are Typst
// 0.15.1's `SmartQuotes::get` table; a custom `smartquote(quotes: ..)` set is not yet consulted.

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
use crate::eval::styles::{
	StyleChain,
	Styles,
};
use crate::eval::value::{
	Alignment,
	HAlign,
	Type,
	Value,
};
use crate::eval::Engine;
use crate::syntax::Span;

use oxedyne_fe2o3_core::prelude::*;

const ANY: FieldType = FieldType::Any;

static QUOTE: [FieldSpec; 5] = [
	FieldSpec::named("block",			FieldType::Of(Type::Bool),	FieldDefault::Bool(false)),
	FieldSpec::named("quotes",			FieldType::OneOf(&[Type::Bool, Type::Auto]),	FieldDefault::Auto),
	FieldSpec::named("attribution",		ANY,	FieldDefault::None),
	FieldSpec::required("body",			ANY),
	FieldSpec::named("depth",			ANY,	FieldDefault::Int(0)).internal(),
];

pub fn fields(kind: ElemKind) -> &'static [FieldSpec] {
	match kind {
		ElemKind::Quote	=> &QUOTE,
		_				=> &[],
	}
}

pub fn cast(_kind: ElemKind, name: &str, v: Value) -> Result<Value, CastErr> {
	match name {
		"block"			=> expect(&v, &[K::Bool]).map(|_| v),
		"quotes"		=> expect(&v, &[K::Bool, K::Auto]).map(|_| v),
		"attribution"	=> match v {
			Value::None | Value::Label(_)	=> Ok(v),
			other	=> expect(&other, &[K::Content, K::Label, K::None]).map(|_| to_content(other)),
		},
		"body"			=> expect(&v, &[K::Content]).map(|_| to_content(v)),
		_				=> Ok(v),
	}
}

/// A block quote is padded by 1em each side and set apart from its neighbours.
pub fn show_set(elem: &Content, styles: &StyleChain) -> Outcome<Styles> {
	if !matches!(res!(common::get(elem, styles, "block")), Value::Bool(true)) {
		return Ok(Styles::new());
	}
	common::props(vec![
		(ElemKind::Pad,		"left",		common::em(1.0)),
		(ElemKind::Pad,		"right",	common::em(1.0)),
		(ElemKind::Block,	"above",	common::em(2.4)),
		(ElemKind::Block,	"below",	common::em(1.8)),
	])
}

/// The quotation marks in force: single open, single close, double open, double close.
pub fn smart_quotes(styles: &StyleChain) -> [&'static str; 4] {
	let (lang, region) = common::lang(styles);
	let alternative = matches!(
		common::style_or(styles, ElemKind::SmartQuote, "alternative", Value::Bool(false)), Value::Bool(true));
	let region = region.as_deref();
	let default = ["\u{2018}", "\u{2019}", "\u{201C}", "\u{201D}"];
	let low_high = ["\u{201A}", "\u{2018}", "\u{201E}", "\u{201C}"];
	match lang.as_str() {
		"de" if matches!(region, Some("CH") | Some("LI")) => if alternative {
			low_high
		} else {
			["\u{2039}", "\u{203A}", "\u{AB}", "\u{BB}"]
		},
		"fr" if region == Some("CH") => if alternative {
			default
		} else {
			["\u{2039}\u{202F}", "\u{202F}\u{203A}", "\u{AB}\u{202F}", "\u{202F}\u{BB}"]
		},
		"cs" | "da" | "de" | "sk" | "sl" if alternative => ["\u{203A}", "\u{2039}", "\u{BB}", "\u{AB}"],
		"cs" | "de" | "et" | "is" | "lt" | "lv" | "sk" | "sl"	=> low_high,
		"da"						=> default,
		"fr" if alternative			=> default,
		"fr"						=> ["\u{201C}", "\u{201D}", "\u{AB}\u{202F}", "\u{202F}\u{BB}"],
		"fi" | "sv" if alternative	=> ["\u{2019}", "\u{2019}", "\u{BB}", "\u{BB}"],
		"gl"						=> ["\u{201C}", "\u{201D}", "\u{AB}", "\u{BB}"],
		"bs" | "fi" | "sv"			=> ["\u{2019}", "\u{2019}", "\u{201D}", "\u{201D}"],
		"it" if alternative			=> default,
		"la" if alternative			=> ["\u{201C}", "\u{201D}", "\u{AB}\u{202F}", "\u{202F}\u{BB}"],
		"it" | "la"					=> ["\u{201C}", "\u{201D}", "\u{AB}", "\u{BB}"],
		"es" if matches!(region, Some("ES") | None)	=> ["\u{201C}", "\u{201D}", "\u{AB}", "\u{BB}"],
		"hu" | "pl" | "ro"			=> ["\u{2019}", "\u{2019}", "\u{201E}", "\u{201D}"],
		"no" | "nb" | "nn" if alternative	=> low_high,
		"no" | "nb" | "nn"			=> ["\u{2019}", "\u{2019}", "\u{AB}", "\u{BB}"],
		"ru"						=> ["\u{201E}", "\u{201C}", "\u{AB}", "\u{BB}"],
		"uk"						=> ["\u{201C}", "\u{201D}", "\u{AB}", "\u{BB}"],
		"el"						=> ["\u{2018}", "\u{2019}", "\u{AB}", "\u{BB}"],
		"he"						=> ["\u{2019}", "\u{2019}", "\u{201D}", "\u{201D}"],
		"hr"						=> ["\u{2018}", "\u{2019}", "\u{201E}", "\u{201D}"],
		"bg"						=> ["\u{2019}", "\u{2019}", "\u{201E}", "\u{201C}"],
		"ar" if !alternative		=> ["\u{2019}", "\u{2018}", "\u{AB}", "\u{BB}"],
		_ if common::is_rtl(styles)	=> ["\u{2019}", "\u{2018}", "\u{201D}", "\u{201C}"],
		_							=> default,
	}
}

pub fn show(engine: &mut Engine, elem: &Content, styles: &StyleChain) -> Outcome<Option<Content>> {
	let span = elem.span();
	let block = matches!(res!(common::get(elem, styles, "block")), Value::Bool(true));
	let mut realised = common::body(elem, "body");
	let quoted = match res!(common::get(elem, styles, "quotes")) {
		Value::Bool(b)	=> b,
		_				=> !block,
	};
	if quoted {
		// Zero-width weak spacing either side keeps the marks against the text.
		let hole = res!(common::h(engine, common::pt(0.0), true));
		let depth = match common::style_or(styles, ElemKind::Quote, "depth", Value::Int(0)) {
			Value::Int(d)	=> d,
			_				=> 0,
		};
		let q = smart_quotes(styles);
		let (open, close) = if depth % 2 == 0 { (q[2], q[3]) } else { (q[0], q[1]) };
		let inner = common::seq(vec![hole.clone(), realised, hole]);
		let marked = common::seq(vec![common::text(open), inner, common::text(close)]);
		realised = res!(common::set(marked, ElemKind::Quote, "depth", Value::Int(depth + 1)));
	}
	let attribution = res!(common::get(elem, styles, "attribution"));
	if block {
		realised = res!(common::block(engine, realised, span, Vec::new()));
		if !attribution.is_none() {
			let gap = res!(common::build(engine, ElemKind::V, span, vec![common::em(0.9)], vec![
				("weak", Value::Bool(true)),
			]));
			let attr = res!(attribution_content(&attribution, span));
			let end = Value::Alignment(Alignment { x: Some(HAlign::End), y: None });
			let aligned = res!(common::build(engine, ElemKind::Align, span, vec![end, Value::Content(attr)], Vec::new()));
			let attr_block = res!(common::block(engine, aligned, span, Vec::new()));
			realised = common::seq(vec![realised, gap, attr_block]);
		}
		realised = res!(common::build(engine, ElemKind::Pad, span, vec![Value::Content(realised)], Vec::new()));
	} else if let Value::Label(l) = &attribution {
		let cite = res!(common::raw_elem(ElemKind::Cite, span, vec![("key", Value::Label(l.clone()))]));
		realised = common::seq(vec![realised, Content::marker(ElemKind::Space, Span::detached()), cite]);
	}
	Ok(Some(realised))
}

/// An em dash, a space, then the attribution: content as given, a label as a prose citation.
fn attribution_content(v: &Value, span: Span) -> Outcome<Content> {
	let who = match v {
		Value::Label(l) => {
			res!(common::raw_elem(ElemKind::Cite, span, vec![
				("key",		Value::Label(l.clone())),
				("form",	Value::str("prose")),
			]))
		}
		other => common::display(other.clone()),
	};
	Ok(common::seq(vec![common::text("\u{2014}"), Content::marker(ElemKind::Space, Span::detached()), who]))
}
