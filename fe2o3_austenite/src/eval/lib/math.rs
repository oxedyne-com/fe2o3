// SPDX-License-Identifier: Apache-2.0
// Original work copyright (c) the Typst project authors (typst-library `math/` elements, their field names and defaults, version 0.15.1).
// Modified for Austenite: ported to Hematite's types, IR and error handling.
// U7 owns this file. The `math` module: element schemas for `math.*` with Typst 0.15's field names and
// defaults, the style functions (`bold`, `display`, ...), the delimiter and accent functions a symbol
// becomes when called, and `define` binding the module (elements, functions, text operators, spacings
// and the `sym` symbols). Maths-mode syntax becomes these elements in U2's evaluator.

use crate::diag::DiagnosticKind;
use crate::eval::args::Args;
use crate::eval::content::{
	Content,
	ElemKind,
	FieldDefault,
	FieldSpec,
	FieldType,
	Fold,
};
use crate::eval::func::{
	Func,
	NativeFunc,
};
use crate::eval::lib;
use crate::eval::scope::Scope;
use crate::eval::content::display;
use crate::eval::styles::{
	Style,
	StyleChain,
	Styles,
};
use crate::eval::value::{
	Dict,
	Length,
	Module,
	Ratio,
	Relative,
	Type,
	Value,
};
use crate::eval::Engine;
use crate::math::class::MathClass;
use crate::math::props;
use crate::math::resolve::{
	accent_char,
	accent_combining,
	delim_char,
	matching,
};
use crate::math::style::{
	MathSize,
	MathVariant,
};
use crate::syntax::Span;

use oxedyne_fe2o3_core::prelude::*;

use std::sync::Arc;

native_fns! {
	pub enum MathFn {
		Display		=> "display",
		Inline		=> "inline",
		Script		=> "script",
		Sscript		=> "sscript",
		Bold		=> "bold",
		Upright		=> "upright",
		Italic		=> "italic",
		Serif		=> "serif",
		Sans		=> "sans",
		Frak		=> "frak",
		Mono		=> "mono",
		Bb			=> "bb",
		Cal			=> "cal",
		Scr			=> "scr",
		Abs			=> "abs",
		Norm		=> "norm",
		Floor		=> "floor",
		Ceil		=> "ceil",
		Round		=> "round",
		Sqrt		=> "sqrt",
		AccentOf	=> "accent-of",	// a combining accent's function, the accent bound first
		LrOf		=> "lr-of",		// a delimiter pair's function, both delimiters bound first
	}
}

// Schemas

const EQUATION: &[FieldSpec] = &[
	FieldSpec::named("block", FieldType::Of(Type::Bool), FieldDefault::Bool(false)),
	FieldSpec::named("numbering", FieldType::Any, FieldDefault::None),
	FieldSpec::named("number-align", FieldType::Of(Type::Alignment), FieldDefault::Computed),
	FieldSpec::named("supplement", FieldType::Any, FieldDefault::Auto),
	FieldSpec::named("alt", FieldType::OneOf(&[Type::Str, Type::None]), FieldDefault::None),
	FieldSpec::required("body", FieldType::Content),
	// Internal, set by the maths functions and layout, never materialised: Typst's ghost fields.
	FieldSpec::named("size", FieldType::Of(Type::Str), FieldDefault::Computed).synthesised(),
	FieldSpec::named("variant", FieldType::Any, FieldDefault::None).synthesised(),
	FieldSpec::named("cramped", FieldType::Of(Type::Bool), FieldDefault::Bool(false)).synthesised(),
	FieldSpec::named("bold", FieldType::Of(Type::Bool), FieldDefault::Bool(false)).synthesised(),
	FieldSpec::named("italic", FieldType::Any, FieldDefault::None).synthesised(),
	FieldSpec::named("script-scale", FieldType::Of(Type::Array), FieldDefault::Computed).synthesised(),
	FieldSpec::named("dtls", FieldType::Of(Type::Bool), FieldDefault::Bool(false)).synthesised(),
];

const REL: FieldType = FieldType::OneOf(&[Type::Relative, Type::Length, Type::Ratio]);

const ALIGN_POINT: &[FieldSpec] = &[];

const ATTACH: &[FieldSpec] = &[
	FieldSpec::required("base", FieldType::Content),
	FieldSpec::named("t", FieldType::Content, FieldDefault::None),
	FieldSpec::named("b", FieldType::Content, FieldDefault::None),
	FieldSpec::named("tl", FieldType::Content, FieldDefault::None),
	FieldSpec::named("bl", FieldType::Content, FieldDefault::None),
	FieldSpec::named("tr", FieldType::Content, FieldDefault::None),
	FieldSpec::named("br", FieldType::Content, FieldDefault::None),
];

const BODY: &[FieldSpec] = &[
	FieldSpec::required("body", FieldType::Content),
];

const LIMITS: &[FieldSpec] = &[
	FieldSpec::required("body", FieldType::Content),
	FieldSpec::named("inline", FieldType::Of(Type::Bool), FieldDefault::Bool(true)),
];

const PRIMES: &[FieldSpec] = &[
	FieldSpec::required("count", FieldType::Of(Type::Int)),
];

const FRAC: &[FieldSpec] = &[
	FieldSpec::required("num", FieldType::Content),
	FieldSpec::required("denom", FieldType::Content),
	FieldSpec::named("style", FieldType::Of(Type::Str), FieldDefault::Str("vertical")),
	FieldSpec::named("num-deparenthesized", FieldType::Of(Type::Bool), FieldDefault::Bool(false)).synthesised(),
	FieldSpec::named("denom-deparenthesized", FieldType::Of(Type::Bool), FieldDefault::Bool(false)).synthesised(),
];

const BINOM: &[FieldSpec] = &[
	FieldSpec::required("upper", FieldType::Content),
	FieldSpec::named("lower", FieldType::Of(Type::Array), FieldDefault::EmptyArray).variadic().unsettable(),
];

const LR: &[FieldSpec] = &[
	FieldSpec::named("size", REL, FieldDefault::Ratio(1.0)),
	FieldSpec::required("body", FieldType::Content),
];

const MAT: &[FieldSpec] = &[
	FieldSpec::named("delim", FieldType::Any, FieldDefault::Computed),
	FieldSpec::named("align", FieldType::Of(Type::Alignment), FieldDefault::Computed),
	FieldSpec::named("augment", FieldType::Any, FieldDefault::None),
	FieldSpec::named("row-gap", REL, FieldDefault::Em(0.2)),
	FieldSpec::named("column-gap", REL, FieldDefault::Em(0.5)),
	FieldSpec::named("rows", FieldType::Of(Type::Array), FieldDefault::EmptyArray).variadic().unsettable(),
];

const VEC: &[FieldSpec] = &[
	FieldSpec::named("delim", FieldType::Any, FieldDefault::Computed),
	FieldSpec::named("align", FieldType::Of(Type::Alignment), FieldDefault::Computed),
	FieldSpec::named("gap", REL, FieldDefault::Em(0.2)),
	FieldSpec::named("children", FieldType::Of(Type::Array), FieldDefault::EmptyArray).variadic().unsettable(),
];

const CASES: &[FieldSpec] = &[
	FieldSpec::named("delim", FieldType::Any, FieldDefault::Computed),
	FieldSpec::named("reverse", FieldType::Of(Type::Bool), FieldDefault::Bool(false)),
	FieldSpec::named("gap", REL, FieldDefault::Em(0.2)),
	FieldSpec::named("children", FieldType::Of(Type::Array), FieldDefault::EmptyArray).variadic().unsettable(),
];

const ROOT: &[FieldSpec] = &[
	FieldSpec::named("index", FieldType::Content, FieldDefault::None).positional(),
	FieldSpec::required("radicand", FieldType::Content),
];

const ACCENT: &[FieldSpec] = &[
	FieldSpec::required("base", FieldType::Content),
	FieldSpec::required("accent", FieldType::Any),
	FieldSpec::named("size", REL, FieldDefault::Ratio(1.0)),
	FieldSpec::named("dotless", FieldType::Of(Type::Bool), FieldDefault::Bool(true)),
];

const OP: &[FieldSpec] = &[
	FieldSpec::required("text", FieldType::Content),
	FieldSpec::named("limits", FieldType::Of(Type::Bool), FieldDefault::Bool(false)),
];

const CLASS: &[FieldSpec] = &[
	FieldSpec::required("class", FieldType::Of(Type::Str)),
	FieldSpec::required("body", FieldType::Content),
];

const CANCEL: &[FieldSpec] = &[
	FieldSpec::required("body", FieldType::Content),
	FieldSpec::named("length", REL, FieldDefault::Computed),
	FieldSpec::named("inverted", FieldType::Of(Type::Bool), FieldDefault::Bool(false)),
	FieldSpec::named("cross", FieldType::Of(Type::Bool), FieldDefault::Bool(false)),
	FieldSpec::named("angle", FieldType::Any, FieldDefault::Auto),
	FieldSpec::named("stroke", FieldType::Any, FieldDefault::Computed).fold(Fold::Stroke),
];

const STRETCH: &[FieldSpec] = &[
	FieldSpec::required("body", FieldType::Content),
	FieldSpec::named("size", REL, FieldDefault::Ratio(1.0)),
];

const SPREADER: &[FieldSpec] = &[
	FieldSpec::required("body", FieldType::Content),
	FieldSpec::named("annotation", FieldType::Content, FieldDefault::None).positional(),
];

pub fn fields(kind: ElemKind) -> &'static [FieldSpec] {
	match kind {
		ElemKind::Equation			=> EQUATION,
		ElemKind::MathAlignPoint	=> ALIGN_POINT,
		ElemKind::MathAttach		=> ATTACH,
		ElemKind::MathScripts		=> BODY,
		ElemKind::MathLimits		=> LIMITS,
		ElemKind::MathPrimes		=> PRIMES,
		ElemKind::MathFrac			=> FRAC,
		ElemKind::MathBinom			=> BINOM,
		ElemKind::MathLr			=> LR,
		ElemKind::MathMid			=> BODY,
		ElemKind::MathMat			=> MAT,
		ElemKind::MathVec			=> VEC,
		ElemKind::MathCases			=> CASES,
		ElemKind::MathRoot			=> ROOT,
		ElemKind::MathAccent		=> ACCENT,
		ElemKind::MathOp			=> OP,
		ElemKind::MathClass			=> CLASS,
		ElemKind::MathCancel		=> CANCEL,
		ElemKind::MathStretch		=> STRETCH,
		ElemKind::MathUnderline		=> BODY,
		ElemKind::MathOverline		=> BODY,
		ElemKind::MathUnderbrace | ElemKind::MathOverbrace | ElemKind::MathUnderbracket
			| ElemKind::MathOverbracket | ElemKind::MathUnderparen | ElemKind::MathOverparen
			| ElemKind::MathUndershell | ElemKind::MathOvershell	=> SPREADER,
		_							=> &[],
	}
}

// Element construction

fn elem(kind: ElemKind, fields: Vec<(&str, Value)>, span: Span) -> Outcome<Content> {
	let mut fv = Vec::with_capacity(fields.len());
	for (name, v) in fields {
		match kind.field_id(name) {
			// Built as a constructor argument is cast, so the field holds what Typst's would.
			Some(id)	=> fv.push((id, res!(cast_field(kind, name, v)))),
			None		=> return Err(err!("{} has no field {}", kind.path(), name; Bug)),
		}
	}
	Ok(Content::new(kind, fv, span))
}

/// Casts a field value as Typst's maths element fields do, for constructors and `set` rules alike.
pub fn cast_field(kind: ElemKind, name: &str, v: Value) -> Outcome<Value> {
	// A field that holds content keeps content: a string, symbol or number is shown as it would be
	// placed in markup, as Typst's content cast does.
	let v = match (kind.field_id(name).and_then(|id| kind.fields().get(id.0 as usize)), &v) {
		(Some(spec), Value::Str(_) | Value::Symbol(_) | Value::Int(_) | Value::Float(_))
			if matches!(spec.ty, FieldType::Content)	=> Value::Content(shown(&v)),
		// A length or ratio in a relative length's field is stored as the relative length it is.
		(Some(spec), Value::Length(l)) if spec.ty == REL	=> Value::Relative(Relative { rel: Ratio(0.0), abs: *l }),
		(Some(spec), Value::Ratio(r)) if spec.ty == REL		=> Value::Relative(Relative { rel: *r, abs: Length::zero() }),
		_													=> v,
	};
	match (kind, name) {
		(ElemKind::MathVec | ElemKind::MathMat | ElemKind::MathCases, "delim") => match delim_error(&v) {
			Some(m)	=> Err(err!("{}", m; Input, Invalid)),
			None	=> Ok(delim_pair(&v)),
		},
		(ElemKind::MathClass, "class") => match &v {
			Value::Str(s) if MathClass::from_name(s).is_some()	=> Ok(v),
			_ => Err(err!("expected \"normal\", \"punctuation\", \"opening\", \"closing\", \"fence\", \"large\", \
				\"relation\", \"unary\", \"binary\", or \"vary\""; Input, Invalid)),
		},
		(ElemKind::MathMat, "augment") => Ok(augment_value(v)),
		// The accent is kept as its combining form, as Typst's cast leaves it.
		(ElemKind::MathAccent, "accent") => match accent_char(&v) {
			Some(c)	=> Ok(Value::str(c.to_string())),
			None	=> Err(err!("{}", match v {
				Value::Str(_)	=> "expected exactly one character",
				_				=> "expected a single-codepoint symbol",
			}; Input, Invalid)),
		},
		(ElemKind::MathFrac, "style") => match &v {
			Value::Str(s) if matches!(s.as_str(), "vertical" | "skewed" | "horizontal")	=> Ok(v),
			_ => Err(err!("expected \"vertical\", \"skewed\", or \"horizontal\""; Input, Invalid)),
		},
		_ => Ok(v),
	}
}

// Why a `delim` value is not a delimiter pair, or `None` when it is one.
fn delim_error(v: &Value) -> Option<String> {
	let one = |v: &Value| -> Option<String> {
		match delim_char(v) {
			Some(None)	=> None,
			Some(Some(c)) => match crate::math::class::default_math_class(c) {
				Some(MathClass::Opening | MathClass::Closing | MathClass::Fence)	=> None,
				_	=> Some(fmt!("invalid delimiter: \"{}\"", c)),
			},
			None => Some("expected a single-codepoint symbol".to_string()),
		}
	};
	match v {
		Value::Array(a) if a.len() == 2	=> one(&a[0]).or_else(|| one(&a[1])),
		Value::Array(a)					=> Some(fmt!("expected 2 delimiters, found {}", a.len())),
		other							=> one(other),
	}
}

// An `augment` as Typst stores it: a dictionary of the horizontal and vertical offsets, each an array,
// and the stroke, with an automatic stroke and a single vertical line collapsing to that offset.
fn augment_value(v: Value) -> Value {
	let d = match &v {
		Value::Dict(d)	=> d.clone(),
		_				=> return v,
	};
	let offsets = |x: Option<&Value>| -> Vec<Value> {
		match x {
			Some(Value::Int(i))		=> vec![Value::Int(*i)],
			Some(Value::Array(a))	=> a.iter().cloned().collect(),
			_						=> Vec::new(),
		}
	};
	let (hline, vline) = (offsets(d.get("hline")), offsets(d.get("vline")));
	let stroke = d.get("stroke").cloned();
	if stroke.is_none() && hline.is_empty() && vline.len() == 1 {
		return vline[0].clone();
	}
	let mut out = Dict::new();
	out.insert("hline", Value::array(hline));
	out.insert("vline", Value::array(vline));
	out.insert("stroke", stroke.unwrap_or(Value::Auto));
	Value::dict(out)
}

// A string, symbol or number as the content it shows as, with no engine in hand.
fn shown(v: &Value) -> Content {
	match v {
		Value::Str(s)		=> Content::text(s),
		Value::Symbol(s)	=> Content::symbol(&crate::eval::ops::symbol_text(s)),
		other				=> Content::text(&crate::eval::lib::foundations::display(other).unwrap_or_default()),
	}
}

// A valid `delim` as Typst stores it: the opening and closing delimiters, each a one-character string or
// `none`. One delimiter is the opener, its closer found from it.
fn delim_pair(v: &Value) -> Value {
	let one = |c: Option<char>| -> Value {
		match c {
			Some(c)	=> Value::str(c.to_string()),
			None	=> Value::None,
		}
	};
	match v {
		Value::Array(a) if a.len() == 2 => {
			let o = a.first().and_then(delim_char).flatten();
			let c = a.get(1).and_then(delim_char).flatten();
			Value::array(vec![one(o), one(c)])
		}
		other => match delim_char(other) {
			Some(Some(c))	=> Value::array(vec![one(Some(c)), one(matching(c))]),
			_				=> Value::array(vec![Value::None, Value::None]),
		},
	}
}

fn check_delim(engine: &mut Engine, span: Span, v: &Value) -> Outcome<()> {
	match delim_error(v) {
		Some(m)	=> Err(engine.error(DiagnosticKind::Type, span, m)),
		None	=> Ok(()),
	}
}

/// Builds the elements whose arguments Typst parses specially; `None` for the generic schema walk.
pub fn construct(engine: &mut Engine, kind: ElemKind, args: &mut Args) -> Outcome<Option<Content>> {
	let span = args.span;
	match kind {
		ElemKind::MathMat => {
			let gap: Option<Value>	= res!(args.named("gap"));
			let row_gap				= res!(args.named::<Value>("row-gap")).or_else(|| gap.clone());
			let column_gap			= res!(args.named::<Value>("column-gap")).or(gap);
			let delim: Option<Value>	= res!(args.named("delim"));
			let align: Option<Value>	= res!(args.named("align"));
			let augment: Option<Value>	= res!(args.named("augment"));
			let values: Vec<Value> = res!(args.all::<Value>());
			res!(std::mem::take(args).finish());
			let mut rows: Vec<Vec<Value>> = Vec::new();
			if values.iter().any(|v| matches!(v, Value::Array(_))) {
				for v in values {
					match v {
						Value::Array(a)	=> {
							let mut row = Vec::with_capacity(a.len());
							for x in a.iter() {
								row.push(Value::Content(res!(display(engine, x.clone(), span))));
							}
							rows.push(row);
						}
						other			=> return Err(engine.error(DiagnosticKind::Type, span, fmt!(
							"expected array, found {}", other.ty().long_name()))),
					}
				}
			} else if !values.is_empty() {
				let mut row = Vec::with_capacity(values.len());
				for x in values {
					row.push(Value::Content(res!(display(engine, x, span))));
				}
				rows.push(row);
			}
			let width = rows.iter().map(|r| r.len()).max().unwrap_or(0);
			for r in rows.iter_mut() {
				while r.len() < width {
					r.push(Value::Content(Content::empty()));
				}
			}
			let mut fields = Vec::new();
			if let Some(d) = delim {
				match cast_field(kind, "delim", d) {
					Ok(d)	=> fields.push(("delim", d)),
					Err(e)	=> return Err(engine.error(DiagnosticKind::Type, span, crate::diag::message_of(&e))),
				}
			}
			if let Some(a) = align {
				fields.push(("align", a));
			}
			if let Some(a) = augment {
				fields.push(("augment", a));
			}
			if let Some(g) = row_gap {
				fields.push(("row-gap", g));
			}
			if let Some(g) = column_gap {
				fields.push(("column-gap", g));
			}
			fields.push(("rows", Value::array(rows.into_iter().map(Value::array).collect())));
			Ok(Some(res!(elem(kind, fields, span))))
		}
		ElemKind::MathVec | ElemKind::MathCases => {
			if let Some(d) = args.items.iter().find(|a| a.name.as_deref() == Some("delim")).map(|a| a.value.clone()) {
				res!(check_delim(engine, span, &d));
			}
			Ok(None)
		}
		ElemKind::MathLr => {
			let size: Option<Value> = res!(args.named("size"));
			let parts: Vec<Value> = res!(args.all::<Value>());
			res!(std::mem::take(args).finish());
			let mut body = Vec::new();
			for (i, p) in parts.into_iter().enumerate() {
				if i > 0 {
					body.push(Content::symbol(","));
				}
				body.push(res!(display(engine, p, span)));
			}
			let mut fields = Vec::new();
			if let Some(s) = size {
				fields.push(("size", s));
			}
			fields.push(("body", Value::Content(Content::sequence(body))));
			Ok(Some(res!(elem(kind, fields, span))))
		}
		ElemKind::MathBinom => {
			let upper: Value = match res!(args.eat::<Value>()) {
				Some(v)	=> v,
				None	=> return Err(engine.error(DiagnosticKind::Type, span, "missing argument: upper")),
			};
			let lower: Vec<Value> = res!(args.all::<Value>());
			if lower.is_empty() {
				return Err(engine.error(DiagnosticKind::Type, span, "missing argument: lower"));
			}
			res!(std::mem::take(args).finish());
			let mut lv = Vec::with_capacity(lower.len());
			for v in lower {
				lv.push(Value::Content(res!(display(engine, v, span))));
			}
			let lower = lv;
			Ok(Some(res!(elem(kind, vec![("upper", upper), ("lower", Value::array(lower))], span))))
		}
		ElemKind::MathRoot => {
			let pos: Vec<Value> = res!(args.all::<Value>());
			res!(std::mem::take(args).finish());
			let (index, radicand) = match pos.len() {
				1	=> (None, pos[0].clone()),
				2	=> (Some(pos[0].clone()), pos[1].clone()),
				0	=> return Err(engine.error(DiagnosticKind::Type, span, "missing argument: radicand")),
				_	=> return Err(engine.error(DiagnosticKind::Type, span, "unexpected argument")),
			};
			let mut fields = Vec::new();
			if let Some(i) = index {
				fields.push(("index", i));
			}
			fields.push(("radicand", radicand));
			Ok(Some(res!(elem(kind, fields, span))))
		}
		ElemKind::MathAccent => {
			if let Some(a) = args.items.iter().filter(|a| a.name.is_none()).nth(1).map(|a| a.value.clone()) {
				if accent_char(&a).is_none() {
					let msg = match a {
						Value::Str(_)	=> "expected exactly one character",
						_				=> "expected a single-codepoint symbol",
					};
					return Err(engine.error(DiagnosticKind::Type, span, msg));
				}
			}
			Ok(None)
		}
		ElemKind::MathClass => {
			if let Some(Value::Str(s)) = args.items.iter().find(|a| a.name.is_none()).map(|a| a.value.clone()) {
				if MathClass::from_name(&s).is_none() {
					return Err(engine.error(DiagnosticKind::Type, span, fmt!(
						"expected \"normal\", \"punctuation\", \"opening\", \"closing\", \"fence\", \"large\", \
						\"relation\", \"unary\", \"binary\", or \"vary\"")));
				}
			}
			Ok(None)
		}
		_ => Ok(None),
	}
}

/// Every maths element is a primitive the maths layout sets.
pub fn show(_engine: &mut Engine, _elem: &Content, _styles: &StyleChain) -> Outcome<Option<Content>> {
	Ok(None)
}

/// The styles an equation sets on itself (Typst's built-in show-set rule): a block equation is centred,
/// unbreakable and in display size, an inline one in text size, and both set in New Computer Modern
/// Math at weight 450. Realisation applies these outside the document's own show-set rules.
pub fn show_set(block: bool) -> Styles {
	let mut v: Vec<Option<Style>> = Vec::new();
	if block {
		v.push(props::property(ElemKind::Align, "alignment", Value::Alignment(crate::eval::value::Alignment {
			x: Some(crate::eval::value::HAlign::Center), y: None })));
		v.push(props::property(ElemKind::Block, "breakable", Value::Bool(false)));
		v.push(props::property(ElemKind::ParLine, "numbering", Value::None));
		v.push(props::set_size(MathSize::Display));
	} else {
		v.push(props::set_size(MathSize::Text));
	}
	v.push(props::property(ElemKind::Text, "weight", Value::Int(450)));
	v.push(props::property(ElemKind::Text, "font", Value::str("New Computer Modern Math")));
	Styles::from_vec(v.into_iter().flatten().collect())
}

/// The built-in show-set styles of a maths element: an equation's, by whether it is a block.
pub fn show_set_of(elem: &Content, styles: &StyleChain) -> Outcome<Styles> {
	if !elem.is(ElemKind::Equation) {
		return Ok(Styles::new());
	}
	let block = match ElemKind::Equation.field_id("block") {
		Some(id)	=> matches!(res!(styles.resolve(elem, id)), Some(Value::Bool(true))),
		None		=> false,
	};
	Ok(show_set(block))
}

/// The factor maths scales `text.size` by at the chain's maths size: 1 in display and text size, the
/// font's script percentages below. What a resolved text size multiplies by inside an equation.
pub fn script_factor(styles: &StyleChain) -> f64 {
	let base = props::text_size(styles);
	if base == 0.0 { 1.0 } else { props::font_size(styles) / base }
}

/// The function a symbol becomes when called in maths: an accent (`hat(x)`), or a delimiter pair
/// wrapping its argument (`paren.l`, `floor`, ...). `None` for any other symbol.
pub fn symbol_func(text: &str) -> Option<Func> {
	if let Some(_) = accent_combining(text) {
		let mut a = Args::new(Span::detached());
		a.push(Span::detached(), Value::str(text));
		return Some(Func::Native(NativeFunc::Math(MathFn::AccentOf)).with(a));
	}
	let mut it = text.chars();
	let left = match (it.next(), it.next()) {
		(Some(c), None)	=> c,
		_				=> return None,
	};
	match left {
		'\u{2308}'	=> return Some(Func::Native(NativeFunc::Math(MathFn::Ceil))),
		'\u{230a}'	=> return Some(Func::Native(NativeFunc::Math(MathFn::Floor))),
		_			=> (),
	}
	let right = DELIMS.iter().find(|(l, _)| *l == left).map(|(_, r)| *r)?;
	let mut a = Args::new(Span::detached());
	a.push(Span::detached(), Value::str(left.to_string()));
	a.push(Span::detached(), Value::str(right.to_string()));
	Some(Func::Native(NativeFunc::Math(MathFn::LrOf)).with(a))
}

const DELIMS: &[(char, char)] = &[
	('(', ')'), ('\u{27ee}', '\u{27ef}'), ('\u{2987}', '\u{2988}'), ('\u{2985}', '\u{2986}'),
	('\u{2993}', '\u{2994}'), ('\u{2995}', '\u{2996}'), ('{', '}'), ('\u{2983}', '\u{2984}'),
	('[', ']'), ('\u{298d}', '\u{2990}'), ('\u{298f}', '\u{298e}'), ('\u{27e6}', '\u{27e7}'),
	('\u{298b}', '\u{298c}'), ('\u{2772}', '\u{2773}'), ('\u{27ec}', '\u{27ed}'), ('\u{2997}', '\u{2998}'),
	('\u{27c5}', '\u{27c6}'), ('\u{23b0}', '\u{23b1}'), ('\u{23b1}', '\u{23b0}'), ('\u{29d8}', '\u{29d9}'),
	('\u{29da}', '\u{29db}'), ('\u{27e8}', '\u{27e9}'), ('\u{29fc}', '\u{29fd}'), ('\u{2991}', '\u{2992}'),
	('\u{2989}', '\u{298a}'), ('\u{27ea}', '\u{27eb}'), ('\u{231c}', '\u{231d}'), ('\u{231e}', '\u{231f}'),
	('|', '|'), ('\u{2016}', '\u{2016}'), ('\u{2980}', '\u{2980}'), ('\u{2999}', '\u{2999}'),
	('\u{299a}', '\u{299a}'),
];

// Functions

fn styled_body(engine: &mut Engine, args: &mut Args, props: Vec<Option<Style>>) -> Outcome<Value> {
	let body: Value = res!(args.expect("body"));
	res!(std::mem::take(args).finish());
	let body = res!(display(engine, body, args.span));
	Ok(Value::Content(body.styled(Styles::from_vec(props.into_iter().flatten().collect()))))
}

fn sized_body(engine: &mut Engine, args: &mut Args, size: MathSize, default_cramped: bool) -> Outcome<Value> {
	let cramped: bool = res!(args.named("cramped")).unwrap_or(default_cramped);
	styled_body(engine, args, vec![props::set_size(size), props::set_cramped(cramped)])
}

fn variant(v: MathVariant) -> Option<Style> {
	props::property(ElemKind::Equation, "variant", Value::str(v.name()))
}

fn delimited(engine: &mut Engine, args: &mut Args, left: &str, right: &str, span: Span) -> Outcome<Value> {
	let size: Option<Value> = res!(args.named("size"));
	let body: Value = res!(args.expect("body"));
	res!(std::mem::take(args).finish());
	let body = res!(display(engine, body, span));
	let bspan = body.span();
	let inner = Content::sequence(vec![Content::symbol(left), body, Content::symbol(right)]);
	let mut fields = Vec::new();
	if let Some(s) = size {
		fields.push(("size", s));
	}
	fields.push(("body", Value::Content(inner)));
	let lr = res!(elem(ElemKind::MathLr, fields, span));
	Ok(Value::Content(if bspan.is_detached() { lr } else { lr.with_span(bspan) }))
}

pub fn call(f: MathFn, engine: &mut Engine, mut args: Args) -> Outcome<Value> {
	let span = args.span;
	match f {
		MathFn::Display		=> sized_body(engine, &mut args, MathSize::Display, false),
		MathFn::Inline		=> sized_body(engine, &mut args, MathSize::Text, false),
		MathFn::Script		=> sized_body(engine, &mut args, MathSize::Script, true),
		MathFn::Sscript		=> sized_body(engine, &mut args, MathSize::ScriptScript, true),
		MathFn::Bold		=> styled_body(engine, &mut args, vec![props::property(ElemKind::Equation, "bold", Value::Bool(true))]),
		MathFn::Upright		=> styled_body(engine, &mut args, vec![props::property(ElemKind::Equation, "italic", Value::Bool(false))]),
		MathFn::Italic		=> styled_body(engine, &mut args, vec![props::property(ElemKind::Equation, "italic", Value::Bool(true))]),
		MathFn::Serif		=> styled_body(engine, &mut args, vec![variant(MathVariant::Plain)]),
		MathFn::Sans		=> styled_body(engine, &mut args, vec![variant(MathVariant::SansSerif)]),
		MathFn::Frak		=> styled_body(engine, &mut args, vec![variant(MathVariant::Fraktur)]),
		MathFn::Mono		=> styled_body(engine, &mut args, vec![variant(MathVariant::Monospace)]),
		MathFn::Bb			=> styled_body(engine, &mut args, vec![variant(MathVariant::DoubleStruck)]),
		MathFn::Cal			=> styled_body(engine, &mut args, vec![variant(MathVariant::Chancery)]),
		MathFn::Scr			=> styled_body(engine, &mut args, vec![variant(MathVariant::Roundhand)]),
		MathFn::Abs			=> delimited(engine, &mut args, "|", "|", span),
		MathFn::Norm		=> delimited(engine, &mut args, "\u{2016}", "\u{2016}", span),
		MathFn::Floor		=> delimited(engine, &mut args, "\u{230a}", "\u{230b}", span),
		MathFn::Ceil		=> delimited(engine, &mut args, "\u{2308}", "\u{2309}", span),
		MathFn::Round		=> delimited(engine, &mut args, "\u{230a}", "\u{2309}", span),
		MathFn::Sqrt		=> {
			let radicand: Value = res!(args.expect("radicand"));
			res!(std::mem::take(&mut args).finish());
			elem(ElemKind::MathRoot, vec![("radicand", radicand)], span).map(Value::Content)
		}
		MathFn::AccentOf	=> {
			let accent: Value = res!(args.expect("accent"));
			let accent = res!(cast_field(ElemKind::MathAccent, "accent", accent));
			let base: Value = res!(args.expect("base"));
			let size: Option<Value> = res!(args.named("size"));
			let dotless: Option<Value> = res!(args.named("dotless"));
			res!(std::mem::take(&mut args).finish());
			let mut fields = vec![("base", base), ("accent", accent)];
			if let Some(s) = size {
				fields.push(("size", s));
			}
			if let Some(d) = dotless {
				fields.push(("dotless", d));
			}
			elem(ElemKind::MathAccent, fields, span).map(Value::Content)
		}
		MathFn::LrOf		=> {
			let l: String = res!(args.expect("left"));
			let r: String = res!(args.expect("right"));
			delimited(engine, &mut args, &l, &r, span)
		}
	}
}

// The module

/// `name` as a horizontal space element of `em` ems, or `None` until the layout schema has `h.amount`.
fn h(em: f64, weak: bool) -> Option<Content> {
	let amount = ElemKind::H.field_id("amount")?;
	let mut fields = vec![(amount, Value::Length(Length::em(em)))];
	if weak {
		if let Some(w) = ElemKind::H.field_id("weak") {
			fields.push((w, Value::Bool(true)));
		}
	}
	Some(Content::new(ElemKind::H, fields, Span::detached()))
}

/// Typst's text operators: `(name, text, limits)`.
const OPS: &[(&str, &str, bool)] = &[
	("arccos", "arccos", false), ("arcsin", "arcsin", false), ("arctan", "arctan", false),
	("arg", "arg", false), ("cos", "cos", false), ("cosh", "cosh", false), ("cot", "cot", false),
	("coth", "coth", false), ("csc", "csc", false), ("csch", "csch", false), ("ctg", "ctg", false),
	("deg", "deg", false), ("det", "det", true), ("dim", "dim", false), ("exp", "exp", false),
	("gcd", "gcd", true), ("lcm", "lcm", true), ("hom", "hom", false), ("id", "id", false),
	("im", "im", false), ("inf", "inf", true), ("ker", "ker", false), ("lg", "lg", false),
	("lim", "lim", true), ("liminf", "lim inf", true), ("limsup", "lim sup", true), ("ln", "ln", false),
	("log", "log", false), ("max", "max", true), ("min", "min", true), ("mod", "mod", false),
	("Pr", "Pr", true), ("sec", "sec", false), ("sech", "sech", false), ("sin", "sin", false),
	("sinc", "sinc", false), ("sinh", "sinh", false), ("sup", "sup", true), ("tan", "tan", false),
	("tanh", "tanh", false), ("tg", "tg", false), ("tr", "tr", false),
];

fn math_scope(sym: Option<&Scope>) -> Outcome<Scope> {
	let mut s = Scope::new();
	// Symbols first, so an element or function of the same name wins.
	if let Some(sym) = sym {
		for (name, b) in sym.iter() {
			s.define(name.clone(), b.value.clone());
		}
	}
	lib::define_elements(&mut s, "math");
	s.define("text", Value::Func(Func::Element(ElemKind::Text)));
	for f in MathFn::ALL {
		if matches!(f, MathFn::AccentOf | MathFn::LrOf) {
			continue;
		}
		s.define(f.name(), Value::Func(Func::Native(NativeFunc::Math(*f))));
	}
	for (name, text, limits) in OPS {
		let op = res!(elem(ElemKind::MathOp, vec![
			("text", Value::Content(Content::text(text))),
			("limits", Value::Bool(*limits)),
		], Span::detached()));
		s.define(*name, Value::Content(op));
	}
	for (name, em) in [("thin", 1.0 / 6.0), ("med", 2.0 / 9.0), ("thick", 5.0 / 18.0), ("quad", 1.0), ("wide", 2.0)] {
		if let Some(c) = h(em, false) {
			s.define(name, Value::Content(c));
		}
	}
	for (name, d) in [("dif", "d"), ("Dif", "D")] {
		if let Some(space) = h(1.0 / 6.0, true) {
			let upright = Content::symbol(d).styled(Styles::from_vec(
				props::property(ElemKind::Equation, "italic", Value::Bool(false)).into_iter().collect()));
			let class = res!(elem(ElemKind::MathClass, vec![
				("class", Value::str("unary")),
				("body", Value::Content(upright)),
			], Span::detached()));
			s.define(name, Value::Content(Content::sequence(vec![space, class])));
		}
	}
	Ok(s)
}

/// Binds `math` in the library, with the `sym` symbols the library already holds.
pub fn define(scope: &mut Scope) {
	let sym = match scope.get("sym") {
		Some(Value::Module(m))	=> Some(m.scope.clone()),
		_						=> None,
	};
	if let Ok(ms) = math_scope(sym.as_ref()) {
		scope.define("math", Value::Module(Arc::new(Module::new("math", ms))));
	}
}
