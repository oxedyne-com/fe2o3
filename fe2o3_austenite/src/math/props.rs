// SPDX-License-Identifier: Apache-2.0
// Original work copyright (c) the Typst project authors (typst-library `math/` element fields and their defaults, version 0.15.1).
// Modified for Austenite: ported to Hematite's types, IR and error handling.
//! Reading and writing the styles maths layout depends on: the equation's internal fields (size,
//! cramped, alphabet, boldness, italics, script scale) and the text, paragraph and alignment properties
//! other units own, each read by its Typst field name with Typst's default when unset.

use crate::eval::content::{
	ElemKind,
	FieldId,
};
use crate::eval::styles::{
	Property,
	Style,
	StyleChain,
	Styles,
};
use crate::eval::value::{
	Alignment,
	Color,
	ColorSpace,
	HAlign,
	Length,
	Paint,
	Value,
};
use crate::math::style::{
	MathSize,
	MathVariant,
};
use crate::syntax::Span;

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_font::shape::Feature;

fn fid(kind: ElemKind, name: &str) -> Option<FieldId> { kind.field_id(name) }

/// The value of `kind.name` in force, or `None` when unset and the schema has no static default.
pub fn get(styles: &StyleChain, kind: ElemKind, name: &str) -> Outcome<Option<Value>> {
	match fid(kind, name) {
		Some(id)	=> styles.get(kind, id),
		None		=> Ok(None),
	}
}

// An equation internal field: its schema folds by replacement, so the innermost set value wins and no
// fold can fail.
fn own(styles: &StyleChain, name: &str) -> Option<Value> {
	let id = fid(ElemKind::Equation, name)?;
	styles.values(ElemKind::Equation, id).first().map(|v| (*v).clone())
}

/// A property style for `kind.name`, or `None` when the schema lacks the field.
pub fn property(kind: ElemKind, name: &str, value: Value) -> Option<Style> {
	fid(kind, name).map(|id| Style::Property(Property::new(kind, id, value, Span::detached())))
}

/// The chain with the given properties pushed innermost.
pub fn chain_with(styles: &StyleChain, props: Vec<Option<Style>>) -> StyleChain {
	let v: Vec<Style> = props.into_iter().flatten().collect();
	styles.chain(&Styles::from_vec(v))
}

// Equation internals

pub fn size(styles: &StyleChain) -> MathSize {
	match own(styles, "size") {
		Some(Value::Str(s))	=> MathSize::from_name(&s).unwrap_or(MathSize::Text),
		_					=> MathSize::Text,
	}
}

pub fn set_size(size: MathSize) -> Option<Style> {
	property(ElemKind::Equation, "size", Value::str(size.name()))
}

pub fn cramped(styles: &StyleChain) -> bool {
	matches!(own(styles, "cramped"), Some(Value::Bool(true)))
}

pub fn set_cramped(v: bool) -> Option<Style> { property(ElemKind::Equation, "cramped", Value::Bool(v)) }

/// Are dotless letter forms asked for (the base of a top accent)?
pub fn dtls(styles: &StyleChain) -> bool {
	matches!(own(styles, "dtls"), Some(Value::Bool(true)))
}

pub fn bold(styles: &StyleChain) -> bool {
	matches!(own(styles, "bold"), Some(Value::Bool(true)))
}

pub fn italic(styles: &StyleChain) -> Option<bool> {
	match own(styles, "italic") {
		Some(Value::Bool(b))	=> Some(b),
		_						=> None,
	}
}

pub fn variant(styles: &StyleChain) -> Option<MathVariant> {
	match own(styles, "variant") {
		Some(Value::Str(s))	=> MathVariant::from_name(&s),
		_					=> None,
	}
}

pub fn script_scale(styles: &StyleChain) -> (i16, i16) {
	match own(styles, "script-scale") {
		Some(Value::Array(a)) => match (a.first(), a.get(1)) {
			(Some(Value::Int(x)), Some(Value::Int(y)))	=> (*x as i16, *y as i16),
			_											=> (70, 50),
		},
		_ => (70, 50),
	}
}

pub fn set_script_scale(a: i16, b: i16) -> Option<Style> {
	property(ElemKind::Equation, "script-scale", Value::array(vec![Value::Int(a as i64), Value::Int(b as i64)]))
}

/// The superscript's size: one step down, bottoming out at script-script.
pub fn for_superscript(styles: &StyleChain) -> Option<Style> {
	set_size(match size(styles) {
		MathSize::Display | MathSize::Text			=> MathSize::Script,
		MathSize::Script | MathSize::ScriptScript	=> MathSize::ScriptScript,
	})
}

pub fn for_subscript(styles: &StyleChain) -> Vec<Option<Style>> {
	vec![for_superscript(styles), set_cramped(true)]
}

pub fn for_numerator(styles: &StyleChain) -> Option<Style> {
	set_size(match size(styles) {
		MathSize::Display							=> MathSize::Text,
		MathSize::Text								=> MathSize::Script,
		MathSize::Script | MathSize::ScriptScript	=> MathSize::ScriptScript,
	})
}

pub fn for_denominator(styles: &StyleChain) -> Vec<Option<Style>> {
	vec![for_numerator(styles), set_cramped(true)]
}

// Text

/// The font size scripts scale from, `text.size` in points, before the maths script factor.
pub fn text_size(styles: &StyleChain) -> f64 { styles.font_size() }

/// The resolved text size: `text.size` scaled by the script percentage the maths size calls for.
pub fn font_size(styles: &StyleChain) -> f64 {
	let (s, ss) = script_scale(styles);
	let factor = match size(styles) {
		MathSize::Display | MathSize::Text	=> 1.0,
		MathSize::Script					=> s as f64 / 100.0,
		MathSize::ScriptScript				=> ss as f64 / 100.0,
	};
	factor * text_size(styles)
}

/// A length in points against the resolved maths font size.
pub fn resolve(styles: &StyleChain, l: Length) -> f64 { l.resolve(font_size(styles)) }

pub fn black() -> Paint {
	Paint::Color(Color { space: ColorSpace::Luma, c: [0.0, 0.0, 0.0, 0.0], alpha: 1.0 })
}

pub fn fill(styles: &StyleChain) -> Outcome<Paint> {
	Ok(match res!(get(styles, ElemKind::Text, "fill")) {
		Some(Value::Color(c))		=> Paint::Color(c),
		Some(Value::Gradient(g))	=> Paint::Gradient(g),
		Some(Value::Tiling(t))		=> Paint::Tiling(t),
		_							=> black(),
	})
}

/// The family list `text.font` names. Unset, it is the maths face the equation's own show-set rule names,
/// not the document's default, so an equation is set in maths whatever the text schema supplies.
pub fn families(styles: &StyleChain) -> Outcome<Vec<String>> {
	fn push(v: &Value, out: &mut Vec<String>) {
		match v {
			Value::Str(s)	=> out.push(s.to_string()),
			Value::Dict(d)	=> if let Some(Value::Str(s)) = d.get("name") {
				out.push(s.to_string());
			},
			Value::Array(a)	=> for x in a.iter() {
				push(x, out);
			},
			_				=> (),
		}
	}
	let mut out = Vec::new();
	match res!(get(styles, ElemKind::Text, "font")) {
		Some(v)	=> push(&v, &mut out),
		None	=> out.push("new computer modern math".to_string()),
	}
	Ok(out)
}

pub fn fallback(styles: &StyleChain) -> Outcome<bool> {
	Ok(!matches!(res!(get(styles, ElemKind::Text, "fallback")), Some(Value::Bool(false))))
}

pub fn weight(styles: &StyleChain) -> Outcome<u16> {
	Ok(match res!(get(styles, ElemKind::Text, "weight")) {
		Some(Value::Int(i))	=> i.clamp(100, 900) as u16,
		Some(Value::Str(s))	=> match s.as_str() {
			"thin"			=> 100,
			"extralight"	=> 200,
			"light"			=> 300,
			"medium"		=> 500,
			"semibold"		=> 600,
			"bold"			=> 700,
			"extrabold"		=> 800,
			"black"			=> 900,
			_				=> 400,
		},
		_					=> 400,
	})
}

/// `text.baseline`: a shift of the glyphs, positive downward.
pub fn baseline(styles: &StyleChain) -> Outcome<f64> {
	Ok(match res!(get(styles, ElemKind::Text, "baseline")) {
		Some(Value::Length(l))	=> resolve(styles, l),
		_						=> 0.0,
	})
}

pub fn stroke(styles: &StyleChain) -> Outcome<Option<crate::eval::value::Stroke>> {
	Ok(res!(get(styles, ElemKind::Text, "stroke")).and_then(|v| crate::eval::styles::to_stroke(&v)))
}

/// The OpenType features maths text is shaped with: the script-style alternates by size, and whatever
/// `text.features` adds.
pub fn features(styles: &StyleChain) -> Outcome<Vec<Feature>> {
	let mut out = Vec::new();
	if matches!(res!(get(styles, ElemKind::Text, "kerning")), Some(Value::Bool(false))) {
		out.push(Feature { tag: *b"kern", value: 0 });
	}
	match size(styles) {
		MathSize::Script		=> out.push(Feature { tag: *b"ssty", value: 1 }),
		MathSize::ScriptScript	=> out.push(Feature { tag: *b"ssty", value: 2 }),
		_						=> (),
	}
	match res!(get(styles, ElemKind::Text, "features")) {
		Some(Value::Array(a)) => for v in a.iter() {
			if let Value::Str(s) = v {
				if let Some(tag) = tag_of(s) {
					out.push(Feature { tag, value: 1 });
				}
			}
		},
		Some(Value::Dict(d)) => for (k, v) in d.iter() {
			if let (Some(tag), Value::Int(n)) = (tag_of(k), v) {
				out.push(Feature { tag, value: *n as u32 });
			}
		},
		_ => (),
	}
	Ok(out)
}

fn tag_of(s: &str) -> Option<[u8; 4]> {
	let b = s.as_bytes();
	if b.len() != 4 {
		return None;
	}
	Some([b[0], b[1], b[2], b[3]])
}

/// Where a text edge sits.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Edge {
	Ascender,
	CapHeight,
	XHeight,
	Baseline,
	Descender,
	Bounds,
	Length(f64),
}

fn edge(styles: &StyleChain, name: &str, default: Edge) -> Outcome<Edge> {
	Ok(match res!(get(styles, ElemKind::Text, name)) {
		Some(Value::Str(s)) => match s.as_str() {
			"ascender"		=> Edge::Ascender,
			"cap-height"	=> Edge::CapHeight,
			"x-height"		=> Edge::XHeight,
			"baseline"		=> Edge::Baseline,
			"descender"		=> Edge::Descender,
			"bounds"		=> Edge::Bounds,
			_				=> default,
		},
		Some(Value::Length(l))	=> Edge::Length(resolve(styles, l)),
		_						=> default,
	})
}

pub fn top_edge(styles: &StyleChain) -> Outcome<Edge> { edge(styles, "top-edge", Edge::CapHeight) }
pub fn bottom_edge(styles: &StyleChain) -> Outcome<Edge> { edge(styles, "bottom-edge", Edge::Baseline) }

// Paragraph and alignment

/// `par.leading` in points: Typst's 0.65em.
pub fn leading(styles: &StyleChain) -> Outcome<f64> {
	Ok(match res!(get(styles, ElemKind::Par, "leading")) {
		Some(Value::Length(l))	=> l.resolve(text_size(styles)),
		_						=> 0.65 * text_size(styles),
	})
}

/// The horizontal alignment in force, `start` when unset.
pub fn align_x(styles: &StyleChain) -> Outcome<Option<HAlign>> {
	Ok(match res!(get(styles, ElemKind::Align, "alignment")) {
		Some(Value::Alignment(Alignment { x, .. }))	=> x,
		_											=> None,
	})
}

/// Is `block.breakable` on?
pub fn breakable(styles: &StyleChain) -> Outcome<bool> {
	Ok(!matches!(res!(get(styles, ElemKind::Block, "breakable")), Some(Value::Bool(false))))
}
