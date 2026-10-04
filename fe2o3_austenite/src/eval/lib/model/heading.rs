// SPDX-License-Identifier: Apache-2.0
// Original work copyright (c) the Typst project authors (typst-library `model/heading.rs`, version 0.15.1).
// Modified for Austenite: the element's fields, defaults and default show, ported to Hematite's types, IR and error handling.
// U5 owns this file: heading and title.

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
	alignment_name,
	expect,
	expect_cast,
	natural,
	to_content,
	CastErr,
	K,
};
use crate::eval::lib::model::lookup;
use crate::eval::styles::{
	StyleChain,
	Styles,
};
use crate::eval::args::Args;
use crate::eval::value::{
	Type,
	Value,
};
use crate::eval::Engine;
use crate::flow::{
	self,
	Region,
};
use crate::ir::Sp;

use oxedyne_fe2o3_core::prelude::*;

const ANY: FieldType = FieldType::Any;

static HEADING: [FieldSpec; 9] = [
	FieldSpec::named("level",			FieldType::OneOf(&[Type::Int, Type::Auto]),	FieldDefault::Auto),
	FieldSpec::named("depth",			FieldType::Of(Type::Int),		FieldDefault::Int(1)),
	FieldSpec::named("offset",			FieldType::Of(Type::Int),		FieldDefault::Int(0)),
	FieldSpec::named("numbering",		ANY,							FieldDefault::None),
	FieldSpec::named("supplement",		ANY,							FieldDefault::Auto),
	FieldSpec::named("outlined",		FieldType::Of(Type::Bool),		FieldDefault::Bool(true)),
	FieldSpec::named("bookmarked",		FieldType::OneOf(&[Type::Bool, Type::Auto]),	FieldDefault::Auto),
	FieldSpec::named("hanging-indent",	FieldType::OneOf(&[Type::Length, Type::Auto]),	FieldDefault::Auto),
	FieldSpec::required("body",			ANY),
];

static TITLE: [FieldSpec; 1] = [
	FieldSpec::named("body",			ANY,							FieldDefault::Auto).positional(),
];

pub fn fields(kind: ElemKind) -> &'static [FieldSpec] {
	match kind {
		ElemKind::Heading	=> &HEADING,
		ElemKind::Title		=> &TITLE,
		_					=> &[],
	}
}

pub fn cast(kind: ElemKind, name: &str, v: Value) -> Result<Value, CastErr> {
	match (kind, name) {
		(ElemKind::Heading, "level") => match v {
			Value::Auto	=> Ok(v),
			Value::Int(_)	=> natural(&v, false).map(|_| v),
			other		=> Err(CastErr::Type(common::mismatch("integer or auto", &other))),
		},
		(ElemKind::Heading, "depth")			=> natural(&v, false).map(|_| v),
		(ElemKind::Heading, "offset")			=> natural(&v, true).map(|_| v),
		(ElemKind::Heading, "numbering")		=> expect_cast(v, &[K::Str, K::Func, K::None]),
		(ElemKind::Heading, "supplement")		=> supplement(v),
		(ElemKind::Heading, "outlined")			=> expect(&v, &[K::Bool]).map(|_| v),
		(ElemKind::Heading, "bookmarked")		=> expect(&v, &[K::Bool, K::Auto]).map(|_| v),
		(ElemKind::Heading, "hanging-indent")	=> expect(&v, &[K::Length, K::Auto]).map(|_| v),
		(ElemKind::Heading, "body")				=> expect(&v, &[K::Content]).map(|_| to_content(v)),
		(ElemKind::Title, "body") => match v {
			Value::Auto	=> Ok(v),
			other		=> expect(&other, &[K::Content]).map(|_| to_content(other)),
		},
		_										=> Ok(v),
	}
}

/// A supplement: content, a function of the element, `none` or `auto`.
pub fn supplement(v: Value) -> Result<Value, CastErr> {
	match v {
		Value::Func(_) | Value::None | Value::Auto	=> Ok(v),
		other => expect(&other, &[K::Content, K::Func, K::None, K::Auto]).map(|_| to_content(other)),
	}
}

/// The level a heading has: set outright, or its depth plus the offset in force.
pub fn level(elem: &Content, styles: &StyleChain) -> Outcome<i64> {
	match res!(common::get(elem, styles, "level")) {
		Value::Int(l)	=> Ok(l),
		_ => {
			let depth = match res!(common::get(elem, styles, "depth")) { Value::Int(d) => d, _ => 1 };
			let offset = match res!(common::get(elem, styles, "offset")) { Value::Int(o) => o, _ => 0 };
			Ok((depth + offset).max(1))
		}
	}
}

/// Resolves a supplement for `target`: `auto` is the local word for `key`, a function is called with
/// the target, `none` is nothing.
pub fn resolve_supplement(
	engine:		&mut Engine,
	value:		&Value,
	styles:		&StyleChain,
	key:		&str,
	target:		&Content,
)
	-> Outcome<Option<Content>>
{
	match value {
		Value::Auto		=> Ok(Some(common::text(common::local(styles, key)))),
		Value::None		=> Ok(None),
		Value::Func(f)	=> {
			let mut args = Args::new(target.span());
			args.push(target.span(), Value::Content(target.clone()));
			let saved = std::mem::replace(&mut engine.context, crate::eval::Context {
				location:	None,
				styles:		Some(styles.clone()),
			});
			let out = engine.call_func(f, args);
			engine.context = saved;
			Ok(Some(res!(common::shown(engine, res!(out), target.span()))))
		}
		other			=> Ok(Some(common::display(other.clone()))),
	}
}

pub fn synthesise(engine: &mut Engine, elem: &mut Content, styles: &StyleChain) -> Outcome<()> {
	let sup = res!(common::get(elem, styles, "supplement"));
	let target = elem.clone();
	let resolved = res!(resolve_supplement(engine, &sup, styles, "heading", &target))
		.unwrap_or_else(Content::empty);
	let lvl = res!(level(elem, styles));
	elem.set(res!(common::fid(ElemKind::Heading, "level")), Value::Int(lvl));
	elem.set(res!(common::fid(ElemKind::Heading, "supplement")), Value::Content(resolved));
	Ok(())
}

/// A heading's size by level (1.4em, 1.2em, then 1em), bold, with space above and below scaled so it
/// reads the same at every level, and kept with what follows.
pub fn show_set(elem: &Content, styles: &StyleChain) -> Outcome<Styles> {
	let lvl = res!(level(elem, styles));
	let scale = match lvl {
		1	=> 1.4,
		2	=> 1.2,
		_	=> 1.0,
	};
	let above = if lvl == 1 { 1.8 } else { 1.44 } / scale;
	let below = 0.75 / scale;
	common::props(vec![
		(ElemKind::Text,	"size",		common::em(scale)),
		(ElemKind::Text,	"weight",	Value::str("bold")),
		(ElemKind::Block,	"above",	common::em(above)),
		(ElemKind::Block,	"below",	common::em(below)),
		(ElemKind::Block,	"sticky",	Value::Bool(true)),
	])
}

pub fn show_set_title() -> Outcome<Styles> {
	common::props(vec![
		(ElemKind::Text,	"size",		common::em(1.7)),
		(ElemKind::Text,	"weight",	Value::str("bold")),
		(ElemKind::Block,	"above",	common::em(1.125)),
		(ElemKind::Block,	"below",	common::em(0.75)),
		(ElemKind::Block,	"sticky",	Value::Bool(true)),
	])
}

const SPACING_TO_NUMBERING: f64 = 0.3;	// em, between a heading's number and its body

pub fn show(engine: &mut Engine, elem: &Content, styles: &StyleChain) -> Outcome<Option<Content>> {
	match elem.kind() {
		Some(ElemKind::Title)	=> show_title(engine, elem, styles),
		_						=> show_heading(engine, elem, styles),
	}
}

fn show_heading(engine: &mut Engine, elem: &Content, styles: &StyleChain) -> Outcome<Option<Content>> {
	let span = elem.span();
	let mut realised = common::body(elem, "body");
	let hanging = res!(common::get(elem, styles, "hanging-indent"));
	let mut indent = match &hanging {
		Value::Length(_)	=> hanging.clone(),
		_					=> common::pt(0.0),
	};
	let numbering = res!(common::get(elem, styles, "numbering"));
	if !numbering.is_none() {
		let loc = match elem.location() {
			Some(l)	=> l,
			None	=> return Err(engine.error(DiagnosticKind::Type, span, "heading must have a location to be numbered")),
		};
		let numbers = common::spanned(res!(lookup::display_counter(
			engine, &lookup::elem_counter(ElemKind::Heading), loc, &numbering, span)), span);
		let align = common::style_or(styles, ElemKind::Align, "alignment", Value::Auto);
		if common::is_auto(&hanging) && starts(&align, styles) {
			let pod = Region {
				width:		Sp(i32::MAX),
				height:		Sp(i32::MAX),
				base:		(Sp(i32::MAX), Sp(i32::MAX)),
				expand_x:	false,
				expand_y:	false,
			};
			let size = res!(flow::measure(engine, &numbers, styles, pod));
			let gap = common::resolve_pt(styles, &common::em(SPACING_TO_NUMBERING));
			indent = common::pt(size.width.to_pt() + gap);
		}
		let spacing = res!(common::h(engine, common::em(SPACING_TO_NUMBERING), true));
		realised = common::seq(vec![numbers, spacing, realised]);
	}
	if common::resolve_pt(styles, &indent) != 0.0 {
		let pull = res!(common::h(engine, common::neg(&indent), false));
		let body = common::seq(vec![pull, realised]);
		let inset = common::start_side(styles, indent);
		return Ok(Some(res!(common::block(engine, body, span, vec![("inset", inset)]))));
	}
	Ok(Some(res!(common::block(engine, realised, span, Vec::new()))))
}

/// Is the horizontal alignment in force the text's start: `start`, or the side the direction starts?
fn starts(align: &Value, styles: &StyleChain) -> bool {
	let name = match align {
		Value::Alignment(a) if a.x.is_some()	=> alignment_name(&crate::eval::value::Alignment { x: a.x, y: None }),
		_										=> return true,
	};
	match name.as_str() {
		"start"	=> true,
		"left"	=> !common::is_rtl(styles),
		"right"	=> common::is_rtl(styles),
		_		=> false,
	}
}

fn show_title(engine: &mut Engine, elem: &Content, styles: &StyleChain) -> Outcome<Option<Content>> {
	let span = elem.span();
	let body = match res!(common::get(elem, styles, "body")) {
		Value::Auto => match res!(common::style(styles, ElemKind::Document, "title")) {
			Value::None => return Err(engine.error_hint(DiagnosticKind::Type, span, "document title was not set",
				"set the title with `set document(title: [...])`")),
			v => common::display(v),
		},
		v => common::display(v),
	};
	Ok(Some(res!(common::block(engine, body, span, Vec::new()))))
}
