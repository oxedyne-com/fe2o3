// U5 owns this file: raw text and its lines. Lines are synthesised as Typst does -- tabs expanded to the
// tab size, split at every newline Typst recognises -- but each line's body is its plain text: syntax
// highlighting (Typst's syntect themes, and its own highlighter for `typ`/`typc`/`typm`) is not done,
// and custom `syntaxes` and `theme` files are accepted but not applied.

use crate::eval::content::{
	Content,
	ElemKind,
	FieldDefault,
	FieldSpec,
	FieldType,
	Fold,
};
use crate::eval::lib::model::common::{
	self,
	alignment_of,
	expect,
	natural,
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

static RAW: [FieldSpec; 8] = [
	FieldSpec::required("text",			FieldType::Of(Type::Str)),
	FieldSpec::named("block",			FieldType::Of(Type::Bool),	FieldDefault::Bool(false)),
	FieldSpec::named("lang",			FieldType::OneOf(&[Type::Str, Type::None]),	FieldDefault::None),
	FieldSpec::named("align",			ANY,	FieldDefault::Computed),
	FieldSpec::named("syntaxes",		ANY,	FieldDefault::EmptyArray).fold(Fold::Add),
	FieldSpec::named("theme",			ANY,	FieldDefault::Auto),
	FieldSpec::named("tab-size",		FieldType::Of(Type::Int),	FieldDefault::Int(2)),
	FieldSpec::named("lines",			ANY,	FieldDefault::EmptyArray).synthesised(),
];

static RAW_LINE: [FieldSpec; 4] = [
	FieldSpec::required("number",		FieldType::Of(Type::Int)),
	FieldSpec::required("count",		FieldType::Of(Type::Int)),
	FieldSpec::required("text",			FieldType::Of(Type::Str)),
	FieldSpec::required("body",			ANY),
];

pub fn fields(kind: ElemKind) -> &'static [FieldSpec] {
	match kind {
		ElemKind::Raw		=> &RAW,
		ElemKind::RawLine	=> &RAW_LINE,
		_					=> &[],
	}
}

pub fn default_value(kind: ElemKind, name: &str) -> Option<Value> {
	match (kind, name) {
		(ElemKind::Raw, "align") => Some(Value::Alignment(Alignment { x: Some(HAlign::Start), y: None })),
		_ => None,
	}
}

pub fn cast(kind: ElemKind, name: &str, v: Value) -> Result<Value, CastErr> {
	match (kind, name) {
		(ElemKind::Raw, "text") | (ElemKind::RawLine, "text")	=> expect(&v, &[K::Str]).map(|_| v),
		(ElemKind::Raw, "block")		=> expect(&v, &[K::Bool]).map(|_| v),
		(ElemKind::Raw, "lang")			=> expect(&v, &[K::Str, K::None]).map(|_| v),
		(ElemKind::Raw, "align")		=> alignment_of(&v, &["start", "left", "center", "right", "end"]).map(|_| v),
		(ElemKind::Raw, "syntaxes")		=> expect(&v, &[K::Path, K::Str, K::Bytes, K::Array]).map(|_| v),
		(ElemKind::Raw, "theme")		=> expect(&v, &[K::Path, K::Str, K::Bytes, K::None, K::Auto]).map(|_| v),
		(ElemKind::Raw, "tab-size")		=> natural(&v, true).map(|_| v),
		(ElemKind::RawLine, "number") | (ElemKind::RawLine, "count")	=> expect(&v, &[K::Int]).map(|_| v),
		(ElemKind::RawLine, "body")		=> expect(&v, &[K::Content]).map(|_| to_content(v)),
		_								=> Ok(v),
	}
}

pub fn synthesise(elem: &mut Content, styles: &StyleChain) -> Outcome<()> {
	let lines = res!(lines(elem, styles));
	elem.set(res!(common::fid(ElemKind::Raw, "lines")), Value::array(lines));
	Ok(())
}

/// The raw text as `raw.line` elements, numbered from one.
fn lines(elem: &Content, styles: &StyleChain) -> Outcome<Vec<Value>> {
	let span = elem.span();
	let mut text = match elem.field("text") {
		Some(Value::Str(s))	=> s.to_string(),
		_					=> String::new(),
	};
	if text.contains('\t') {
		let size = match res!(common::get(elem, styles, "tab-size")) {
			Value::Int(n)	=> n.max(0) as usize,
			_				=> 2,
		};
		text = align_tabs(&text, size);
	}
	let split = split_newlines(&text);
	let count = split.len() as i64;
	let mut out = Vec::with_capacity(split.len());
	for (i, line) in split.into_iter().enumerate() {
		let body = common::text(line).with_span(span);
		out.push(Value::Content(res!(common::raw_elem(ElemKind::RawLine, span, vec![
			("number",	Value::Int(i as i64 + 1)),
			("count",	Value::Int(count)),
			("text",	Value::str(line)),
			("body",	Value::Content(body)),
		]))));
	}
	Ok(out)
}

/// Typst's newlines: line feed, vertical tab, form feed, carriage return (alone or before a line feed),
/// next line, and the line and paragraph separators.
fn split_newlines(text: &str) -> Vec<&str> {
	let mut out = Vec::new();
	let mut start = 0;
	let mut it = text.char_indices().peekable();
	while let Some((i, c)) = it.next() {
		let nl = matches!(c, '\n' | '\x0B' | '\x0C' | '\r' | '\u{0085}' | '\u{2028}' | '\u{2029}');
		if !nl {
			continue;
		}
		out.push(&text[start..i]);
		let mut end = i + c.len_utf8();
		if c == '\r' {
			if let Some((j, '\n')) = it.peek().copied() {
				it.next();
				end = j + 1;
			}
		}
		start = end;
	}
	out.push(&text[start..]);
	out
}

/// Replaces each tab by the spaces that reach the next multiple of the tab size, counting characters
/// from the start of the line.
fn align_tabs(text: &str, size: usize) -> String {
	let divisor = size.max(1);
	let mut out = String::with_capacity(text.len());
	let mut column = 0;
	for c in text.chars() {
		match c {
			'\t' => {
				let required = size - column % divisor;
				for _ in 0..required {
					out.push(' ');
				}
				column += required;
			}
			'\n' | '\x0B' | '\x0C' | '\r' | '\u{0085}' | '\u{2028}' | '\u{2029}' => {
				out.push(c);
				column = 0;
			}
			_ => {
				out.push(c);
				column += 1;
			}
		}
	}
	out
}

/// Monospaced at 0.8em in English, unhyphenated; a block is not justified.
pub fn show_set(elem: &Content, styles: &StyleChain) -> Outcome<Styles> {
	let mut list = vec![
		(ElemKind::Text,	"overhang",				Value::Bool(false)),
		(ElemKind::Text,	"lang",					Value::str("en")),
		(ElemKind::Text,	"hyphenate",			Value::Bool(false)),
		(ElemKind::Text,	"size",					common::em(0.8)),
		(ElemKind::Text,	"font",					Value::str("DejaVu Sans Mono")),
		(ElemKind::Text,	"cjk-latin-spacing",	Value::None),
	];
	if matches!(res!(common::get(elem, styles, "block")), Value::Bool(true)) {
		list.push((ElemKind::Par, "justify", Value::Bool(false)));
	}
	common::props(list)
}

pub fn show(engine: &mut Engine, elem: &Content, styles: &StyleChain) -> Outcome<Option<Content>> {
	if elem.is(ElemKind::RawLine) {
		return Ok(Some(common::body(elem, "body")));
	}
	let span = elem.span();
	let lines = match elem.field("lines") {
		Some(Value::Array(a)) if !a.is_empty()	=> (**a).clone(),
		_										=> res!(lines(elem, styles)),
	};
	let mut seq = Vec::with_capacity(lines.len() * 2);
	for (i, line) in lines.into_iter().enumerate() {
		if i != 0 {
			seq.push(Content::marker(ElemKind::Linebreak, Span::detached()));
		}
		seq.push(common::display(line));
	}
	let mut realised = Content::sequence(seq);
	if matches!(res!(common::get(elem, styles, "block")), Value::Bool(true)) {
		let align = res!(common::get(elem, styles, "align"));
		realised = res!(common::build(engine, ElemKind::Align, span, vec![align, Value::Content(realised)], Vec::new()));
		realised = res!(common::block(engine, realised, span, Vec::new()));
	}
	Ok(Some(realised))
}
