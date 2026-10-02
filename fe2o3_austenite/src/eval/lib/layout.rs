// SPDX-License-Identifier: Apache-2.0
// Original work copyright (c) the Typst project authors (typst-library `layout/*`, version 0.15.1).
// Modified for Austenite: ported to Hematite's types, IR and error handling.
// U6b owns this file. Schemas for the layout family (box, block, align, pad, stack, h, v, place, columns,
// colbreak, pagebreak, page), with Typst 0.15's defaults. `align` shows as its body under the alignment
// style and `page(..)[body]` constructs a page-bounded sequence, as Typst's do; every other element is a
// primitive flow lays out itself, so its `show` is `None`.
//
// Typst parses a few arguments that are no field of their element: `block(spacing:)`, `pad(x:, y:, rest:)`
// and `page(paper:)`. A constructor call spreads them into the real fields here. A `set` rule stores them
// as settable fields of their own, and flow resolves the real field from whichever of the two is nearer in
// the style chain, which is what Typst's spreading at parse time amounts to.

use crate::eval::args::Args;
use crate::diag::DiagnosticKind;
use crate::eval::content::{
	Content,
	ElemKind,
	FieldDefault,
	FieldId,
	FieldSpec,
	FieldType,
	Fold,
};
use crate::eval::lib::visual as vis;
use crate::eval::scope::Scope;
use crate::eval::styles::{
	self,
	Property,
	Style,
	StyleChain,
	Styles,
};
use crate::eval::value::{
	Alignment,
	Dict,
	HAlign,
	Type,
	Value,
};
use crate::eval::Engine;

use oxedyne_fe2o3_core::prelude::*;

native_fns! {
	pub enum LayoutFn {
	}
}

// Schemas

const ANY: FieldType = FieldType::Any;
const BOOL: FieldType = FieldType::Of(Type::Bool);
const SPACING: FieldType = FieldType::OneOf(&[Type::Length, Type::Ratio, Type::Relative, Type::Fraction]);

const BOX: &[FieldSpec] = &[
	FieldSpec::named("width",		ANY,	FieldDefault::Auto),
	FieldSpec::named("height",		ANY,	FieldDefault::Auto),
	FieldSpec::named("baseline",	ANY,	FieldDefault::Auto),
	FieldSpec::named("fill",		ANY,	FieldDefault::None),
	FieldSpec::named("stroke",		ANY,	FieldDefault::None).fold(Fold::Sides),
	FieldSpec::named("radius",		ANY,	FieldDefault::Pt(0.0)).fold(Fold::Corners),
	FieldSpec::named("inset",		ANY,	FieldDefault::Pt(0.0)).fold(Fold::Sides),
	FieldSpec::named("outset",		ANY,	FieldDefault::Pt(0.0)).fold(Fold::Sides),
	FieldSpec::named("clip",		BOOL,	FieldDefault::Bool(false)),
	FieldSpec::named("body",		FieldType::Content,	FieldDefault::None).positional().unsettable(),
];

// `spacing` is Typst's shorthand for both `above` and `below`; see the file header. The three have no
// static default: unset, the spacing is the paragraph spacing, which flow reads.
const BLOCK: &[FieldSpec] = &[
	FieldSpec::named("width",		ANY,	FieldDefault::Auto),
	FieldSpec::named("height",		ANY,	FieldDefault::Auto),
	FieldSpec::named("breakable",	BOOL,	FieldDefault::Bool(true)),
	FieldSpec::named("fill",		ANY,	FieldDefault::None),
	FieldSpec::named("stroke",		ANY,	FieldDefault::None).fold(Fold::Sides),
	FieldSpec::named("radius",		ANY,	FieldDefault::Pt(0.0)).fold(Fold::Corners),
	FieldSpec::named("inset",		ANY,	FieldDefault::Pt(0.0)).fold(Fold::Sides),
	FieldSpec::named("outset",		ANY,	FieldDefault::Pt(0.0)).fold(Fold::Sides),
	FieldSpec::named("spacing",		ANY,	FieldDefault::Computed),
	FieldSpec::named("above",		ANY,	FieldDefault::Computed),
	FieldSpec::named("below",		ANY,	FieldDefault::Computed),
	FieldSpec::named("clip",		BOOL,	FieldDefault::Bool(false)),
	FieldSpec::named("sticky",		BOOL,	FieldDefault::Bool(false)),
	FieldSpec::named("body",		FieldType::Content,	FieldDefault::None).positional().unsettable(),
];

// Alignment folds axis by axis (`set align(center)` then `align(horizon)` is centre-horizon); flow folds
// the chain's values itself, so the field is stored as set. Its default is `start + top`.
const ALIGN: &[FieldSpec] = &[
	FieldSpec::named("alignment",	FieldType::Of(Type::Alignment),	FieldDefault::Computed).positional(),
	FieldSpec::required("body",		FieldType::Content),
];

// `x`, `y` and `rest` spread into the sides; see the file header.
const PAD: &[FieldSpec] = &[
	FieldSpec::named("left",	ANY,	FieldDefault::Pt(0.0)),
	FieldSpec::named("top",		ANY,	FieldDefault::Pt(0.0)),
	FieldSpec::named("right",	ANY,	FieldDefault::Pt(0.0)),
	FieldSpec::named("bottom",	ANY,	FieldDefault::Pt(0.0)),
	FieldSpec::named("x",		ANY,	FieldDefault::Computed),
	FieldSpec::named("y",		ANY,	FieldDefault::Computed),
	FieldSpec::named("rest",	ANY,	FieldDefault::Computed),
	FieldSpec::required("body",	FieldType::Content),
];

const STACK: &[FieldSpec] = &[
	FieldSpec::named("dir",			FieldType::Of(Type::Direction),	FieldDefault::Computed),
	FieldSpec::named("spacing",		ANY,	FieldDefault::None),
	FieldSpec::named("children",	ANY,	FieldDefault::EmptyArray).variadic().unsettable(),
];

const H: &[FieldSpec] = &[
	FieldSpec::required("amount",	SPACING),
	FieldSpec::named("weak",		BOOL,	FieldDefault::Bool(false)),
];

// `attach` is Typst's internal marker for spacing that only survives straight after a paragraph.
const V: &[FieldSpec] = &[
	FieldSpec::required("amount",	SPACING),
	FieldSpec::named("weak",		BOOL,	FieldDefault::Bool(false)),
	FieldSpec::named("attach",		BOOL,	FieldDefault::Bool(false)).unsettable(),
];

const PLACE: &[FieldSpec] = &[
	FieldSpec::named("alignment",	FieldType::OneOf(&[Type::Alignment, Type::Auto]),	FieldDefault::Computed).positional(),
	FieldSpec::named("scope",		FieldType::Of(Type::Str),	FieldDefault::Str("column")),
	FieldSpec::named("float",		BOOL,	FieldDefault::Bool(false)),
	FieldSpec::named("clearance",	ANY,	FieldDefault::Em(1.5)),
	FieldSpec::named("dx",			ANY,	FieldDefault::Pt(0.0)),
	FieldSpec::named("dy",			ANY,	FieldDefault::Pt(0.0)),
	FieldSpec::required("body",		FieldType::Content),
];

const COLUMNS: &[FieldSpec] = &[
	FieldSpec::named("count",	FieldType::Of(Type::Int),	FieldDefault::Int(2)).positional(),
	FieldSpec::named("gutter",	ANY,	FieldDefault::Ratio(0.04)),
	FieldSpec::required("body",	FieldType::Content),
];

const COLBREAK: &[FieldSpec] = &[
	FieldSpec::named("weak",	BOOL,	FieldDefault::Bool(false)),
];

// `boundary` marks the break realisation puts after a `set page` scope; it is never passed.
const PAGEBREAK: &[FieldSpec] = &[
	FieldSpec::named("weak",		BOOL,	FieldDefault::Bool(false)),
	FieldSpec::named("to",			ANY,	FieldDefault::None),
	FieldSpec::named("boundary",	BOOL,	FieldDefault::Bool(false)).unsettable(),
];

// `paper` spreads into `width` and `height` (see the file header), so it precedes them: within one `set`,
// an explicit width or height is the nearer of the two.
const PAGE: &[FieldSpec] = &[
	FieldSpec::named("paper",			FieldType::Of(Type::Str),	FieldDefault::Str("a4")),
	FieldSpec::named("width",			ANY,	FieldDefault::Computed),
	FieldSpec::named("height",			ANY,	FieldDefault::Computed),
	FieldSpec::named("flipped",			BOOL,	FieldDefault::Bool(false)),
	FieldSpec::named("margin",			ANY,	FieldDefault::Auto).fold(Fold::Sides),
	FieldSpec::named("bleed",			ANY,	FieldDefault::Pt(0.0)).fold(Fold::Sides),
	FieldSpec::named("binding",			ANY,	FieldDefault::Auto),
	FieldSpec::named("columns",			FieldType::Of(Type::Int),	FieldDefault::Int(1)),
	FieldSpec::named("fill",			ANY,	FieldDefault::Auto),
	FieldSpec::named("numbering",		ANY,	FieldDefault::None),
	FieldSpec::named("supplement",		ANY,	FieldDefault::Auto),
	FieldSpec::named("number-align",	ANY,	FieldDefault::Computed),
	FieldSpec::named("header",			ANY,	FieldDefault::Auto),
	FieldSpec::named("header-ascent",	ANY,	FieldDefault::Ratio(0.3)),
	FieldSpec::named("footer",			ANY,	FieldDefault::Auto),
	FieldSpec::named("footer-descent",	ANY,	FieldDefault::Ratio(0.3)),
	FieldSpec::named("background",		ANY,	FieldDefault::None),
	FieldSpec::named("foreground",		ANY,	FieldDefault::None),
	FieldSpec::required("body",			FieldType::Content),
];

pub fn fields(kind: ElemKind) -> &'static [FieldSpec] {
	match kind {
		ElemKind::Box			=> BOX,
		ElemKind::Block			=> BLOCK,
		ElemKind::Align			=> ALIGN,
		ElemKind::Pad			=> PAD,
		ElemKind::Stack			=> STACK,
		ElemKind::H				=> H,
		ElemKind::V				=> V,
		ElemKind::Place			=> PLACE,
		ElemKind::PlaceFlush	=> &[],
		ElemKind::Columns		=> COLUMNS,
		ElemKind::Colbreak		=> COLBREAK,
		ElemKind::Pagebreak		=> PAGEBREAK,
		ElemKind::Page			=> PAGE,
		_						=> &[],
	}
}

/// The field's id in the element's schema. Every name passed here is one this file declares.
pub fn fid(kind: ElemKind, name: &str) -> Outcome<FieldId> {
	kind.field_id(name).ok_or_else(|| err!(
		"{} has no field `{}` in its schema.", kind.path(), name; Bug, Missing))
}

// Library

pub fn define(_scope: &mut Scope) {}

pub fn call(f: LayoutFn, _engine: &mut Engine, _args: Args) -> Outcome<Value> {
	match f {}
}

// Construction

/// Spreads the shorthand arguments Typst parses into real fields, then leaves the element to the generic
/// schema walk; `page(..)[body]` alone builds something else, a page-bounded sequence.
pub fn construct(engine: &mut Engine, kind: ElemKind, args: &mut Args) -> Outcome<Option<Content>> {
	match kind {
		ElemKind::Page => construct_page(engine, args).map(Some),
		ElemKind::H | ElemKind::V => {
			// The amount is checked here for Typst's own message.
			if let Some(a) = args.items.iter().find(|a| a.name.is_none()) {
				if !SPACING.accepts(&a.value) {
					let msg = fmt!("expected relative length or fraction, found {}",
						a.value.ty().long_name());
					return Err(engine.error(DiagnosticKind::Type, a.span, msg));
				}
			}
			Ok(None)
		},
		_ => {
			res!(spread(kind, args));
			Ok(None)
		},
	}
}

/// Rewrites the arguments Typst parses into other fields: `block(spacing:)` into whichever of `above` and
/// `below` is not given, `pad(x:, y:, rest:)` into the sides by Typst's precedence (a side, then its axis,
/// then `rest`). A `set` rule wants the same rewriting before its arguments become styles.
pub fn spread(kind: ElemKind, args: &mut Args) -> Outcome<()> {
	let span = args.span;
	match kind {
		ElemKind::Block => {
			if let Some(s) = res!(args.named::<Value>("spacing")) {
				if !has_named(args, "above") {
					args.push_named(span, "above", s.clone());
				}
				if !has_named(args, "below") {
					args.push_named(span, "below", s);
				}
			}
		},
		ElemKind::Pad => {
			let mut rest = res!(args.named::<Value>("rest"));
			// `rest` is also the first positional, ahead of the body: `pad(1em)[..]`.
			if rest.is_none() && args.items.iter().filter(|a| a.name.is_none()).count() > 1 {
				if let Some(i) = args.items.iter().position(|a| a.name.is_none()) {
					if matches!(args.items[i].value, Value::Length(_) | Value::Relative(_) | Value::Ratio(_)) {
						rest = Some(args.items.remove(i).value);
					}
				}
			}
			let x		= res!(args.named::<Value>("x")).or_else(|| rest.clone());
			let y		= res!(args.named::<Value>("y")).or(rest);
			for (side, v) in [("left", &x), ("right", &x), ("top", &y), ("bottom", &y)] {
				if let Some(v) = v {
					if !has_named(args, side) {
						args.push_named(span, side, v.clone());
					}
				}
			}
		},
		_ => (),
	}
	Ok(())
}

fn has_named(args: &Args, name: &str) -> bool {
	args.items.iter().any(|a| a.name.as_deref() == Some(name))
}

/// `page(..)[body]`: no page element, but the body set apart on pages of its own under the page styles --
/// a weak break before, a flush so the page is kept even when the body is empty, the body, and a boundary
/// break after, as Typst's constructor builds it.
fn construct_page(engine: &mut Engine, args: &mut Args) -> Outcome<Content> {
	let span = args.span;
	let body = match args.items.iter().rposition(|a| a.name.is_none()) {
		Some(i)	=> {
			let a = args.items.remove(i);
			res!(a.value.cast::<Content>())
		},
		None	=> return Err(engine.error(DiagnosticKind::Type, span, "missing argument: body")),
	};
	let styles = res!(styles::set_rule(engine, ElemKind::Page, std::mem::take(args)));
	Ok(Content::sequence(vec![
		pagebreak(true, false),
		Content::marker(ElemKind::PlaceFlush, span),
		body,
		pagebreak(true, true),
	]).styled(styles))
}

/// A page break as realisation and `page(..)` produce them.
pub fn pagebreak(weak: bool, boundary: bool) -> Content {
	let mut fields = Vec::new();
	if let Some(id) = ElemKind::Pagebreak.field_id("weak") {
		fields.push((id, Value::Bool(weak)));
	}
	if boundary {
		if let Some(id) = ElemKind::Pagebreak.field_id("boundary") {
			fields.push((id, Value::Bool(true)));
		}
	}
	Content::new(ElemKind::Pagebreak, fields, crate::syntax::Span::detached())
}

// Casts

/// Casts a field's value to Typst's type for it, for a constructor and for a `set` rule alike, with Typst's
/// wording when the value is refused. Every cast is idempotent: flow reads the stored value again.
pub fn cast_field(kind: ElemKind, name: &str, v: Value) -> Outcome<Value> {
	use ElemKind as K;
	match (kind, name) {
		(K::Box, "width") | (K::Block, "height")			=> sizing(v),
		(K::Box, "height") | (K::Block, "width")			=> smart_rel(v),
		(K::Box, "baseline")								=> baseline(v),
		(K::Box | K::Block, "fill")							=> vis::cast_fill(v),
		(K::Box | K::Block, "stroke")						=> vis::cast_stroke_sides(v),
		(K::Box | K::Block, "radius")						=> vis::cast_corners_rel(v),
		(K::Box | K::Block, "inset") | (K::Box | K::Block, "outset")
															=> vis::cast_sides_rel(v),
		(K::Block, "spacing") | (K::Block, "above") | (K::Block, "below") => match v {
			Value::Auto	=> Ok(v),
			other		=> spacing(other),
		},
		(_, "clip") | (_, "breakable") | (_, "sticky") | (_, "weak") | (_, "float") | (_, "flipped")
			| (_, "boundary") | (_, "attach")				=> Ok(Value::Bool(res!(v.cast::<bool>()))),
		(K::Align, "alignment")								=> Ok(Value::Alignment(res!(v.cast::<Alignment>()))),
		(K::Place, "alignment") => match v {
			Value::Auto	=> Ok(v),
			other		=> Ok(Value::Alignment(res!(other.cast::<Alignment>()))),
		},
		(K::Pad, _) if name != "body"						=> Ok(Value::Relative(res!(vis::cast_rel(v)))),
		(K::Stack, "dir") => match v {
			Value::Direction(_)	=> Ok(v),
			other				=> Err(mismatch("direction", &other)),
		},
		(K::Stack, "spacing") => match v {
			Value::None	=> Ok(v),
			other		=> spacing(other),
		},
		(K::H | K::V, "amount")								=> spacing(v),
		(K::Place, "scope") => {
			let s = res!(v.cast::<String>());
			match s.as_str() {
				"column" | "parent"	=> Ok(Value::str(s)),
				_					=> Err(err!("expected \"column\" or \"parent\""; Input, Invalid)),
			}
		},
		(K::Place, "clearance")								=> Ok(Value::Length(res!(vis::cast_length(v)))),
		(K::Place, "dx") | (K::Place, "dy")					=> Ok(Value::Relative(res!(vis::cast_rel(v)))),
		(K::Columns, "count") | (K::Page, "columns") => {
			let n = res!(v.cast::<i64>());
			if n <= 0 {
				return Err(err!("number must be positive"; Input, Invalid));
			}
			Ok(Value::Int(n))
		},
		(K::Columns, "gutter")								=> Ok(Value::Relative(res!(vis::cast_rel(v)))),
		(K::Pagebreak, "to") => match v {
			Value::None => Ok(v),
			other => {
				let s = res!(other.cast::<String>());
				match s.as_str() {
					"even" | "odd"	=> Ok(Value::str(s)),
					_				=> Err(err!("expected \"even\", \"odd\", or none"; Input, Invalid)),
				}
			},
		},
		(K::Page, "paper") => {
			let s = res!(v.cast::<String>());
			match paper(&s) {
				Some(_)	=> Ok(Value::str(s)),
				None	=> {
					// Typst names every paper it knows, in its table's order.
					let names: Vec<String> = PAPERS.iter().map(|(n, _, _)| fmt!("\"{}\"", n)).collect();
					let (last, rest) = match names.split_last() {
						Some(x)	=> x,
						None	=> return Err(err!("unknown paper size"; Input, Invalid)),
					};
					Err(err!("expected {}, or {}", rest.join(", "), last; Input, Invalid))
				},
			}
		},
		(K::Page, "width") | (K::Page, "height") => match v {
			Value::Auto	=> Ok(v),
			other		=> Ok(Value::Length(res!(vis::cast_length(other)))),
		},
		(K::Page, "margin")									=> margin(v),
		(K::Page, "bleed")									=> vis::cast_sides_rel(v),
		(K::Page, "binding") => match v {
			Value::Auto => Ok(v),
			Value::Alignment(Alignment { x: Some(HAlign::Left), y: None })
				| Value::Alignment(Alignment { x: Some(HAlign::Right), y: None }) => Ok(v),
			Value::Alignment(_) => Err(err!("must be `left` or `right`"; Input, Invalid)),
			other => Err(mismatch("alignment or auto", &other)),
		},
		(K::Page, "fill") => match v {
			Value::Auto	=> Ok(v),
			other		=> vis::cast_fill(other),
		},
		(K::Page, "numbering") => match v {
			Value::None | Value::Str(_) | Value::Func(_)	=> Ok(v),
			other	=> Err(mismatch("string, function, or none", &other)),
		},
		(K::Page, "number-align")							=> Ok(Value::Alignment(res!(v.cast::<Alignment>()))),
		(K::Page, "header-ascent") | (K::Page, "footer-descent")
															=> Ok(Value::Relative(res!(vis::cast_rel(v)))),
		(K::Page, "supplement") | (K::Page, "header") | (K::Page, "footer") => match v {
			Value::Auto | Value::None	=> Ok(v),
			other						=> Ok(Value::Content(res!(other.cast::<Content>()))),
		},
		(K::Page, "background") | (K::Page, "foreground") => match v {
			Value::None	=> Ok(v),
			other		=> Ok(Value::Content(res!(other.cast::<Content>()))),
		},
		_ => Ok(v),
	}
}

fn mismatch(expected: &str, found: &Value) -> Error<ErrTag> {
	err!("expected {}, found {}", expected, found.ty().long_name(); Input, Mismatch)
}

// Typst's `Sizing`: `auto`, a relative length or a fraction.
fn sizing(v: Value) -> Outcome<Value> {
	match v {
		Value::Auto | Value::Fraction(_) => Ok(v),
		Value::Relative(_) | Value::Length(_) | Value::Ratio(_) => Ok(Value::Relative(res!(vis::cast_rel(v)))),
		other => Err(mismatch("relative length, fraction, or auto", &other)),
	}
}

fn smart_rel(v: Value) -> Outcome<Value> {
	match v {
		Value::Auto	=> Ok(v),
		Value::Relative(_) | Value::Length(_) | Value::Ratio(_) => Ok(Value::Relative(res!(vis::cast_rel(v)))),
		other		=> Err(mismatch("relative length or auto", &other)),
	}
}

// Typst's `Spacing`: a relative length or a fraction.
fn spacing(v: Value) -> Outcome<Value> {
	match v {
		Value::Fraction(_) => Ok(v),
		Value::Relative(_) | Value::Length(_) | Value::Ratio(_) => Ok(Value::Relative(res!(vis::cast_rel(v)))),
		other => Err(mismatch("relative length or fraction", &other)),
	}
}

// A box's baseline: an alignment or `auto` (where), a relative length (a shift), or both in a dictionary.
fn baseline(v: Value) -> Outcome<Value> {
	match v {
		Value::Auto | Value::Alignment(_) => Ok(v),
		Value::Relative(_) | Value::Length(_) | Value::Ratio(_) => Ok(Value::Relative(res!(vis::cast_rel(v)))),
		Value::Dict(d) => {
			let mut out = Dict::new();
			for (k, x) in d.iter() {
				match k {
					"at" => out.insert(k, match x {
						Value::Auto | Value::Alignment(_)	=> x.clone(),
						other								=> return Err(mismatch("alignment or auto", other)),
					}),
					"shift"	=> out.insert(k, Value::Relative(res!(vis::cast_rel(x.clone())))),
					other	=> return Err(err!("unexpected key \"{}\", valid keys are \"at\" and \"shift\"", other;
						Input, Invalid)),
				}
			}
			Ok(Value::dict(out))
		},
		other => Err(mismatch("relative length, alignment, dictionary, or auto", &other)),
	}
}

// A page margin: `auto`, one relative length, or a dictionary of sides that may each be `auto`, where
// `inside` and `outside` exclude `left` and `right`.
fn margin(v: Value) -> Outcome<Value> {
	match v {
		Value::Auto => Ok(v),
		Value::Relative(_) | Value::Length(_) | Value::Ratio(_) => Ok(Value::Relative(res!(vis::cast_rel(v)))),
		Value::Dict(d) => {
			let two		= d.contains("inside") || d.contains("outside");
			let one		= d.contains("left") || d.contains("right");
			if two && one {
				return Err(err!("`inside` and `outside` are mutually exclusive with `left` and `right`";
					Input, Invalid));
			}
			let mut out = Dict::new();
			for (k, x) in d.iter() {
				match k {
					"left" | "top" | "right" | "bottom" | "inside" | "outside" | "x" | "y" | "rest" => {
						let cast = match x {
							Value::Auto	=> Value::Auto,
							other		=> Value::Relative(res!(vis::cast_rel(other.clone()))),
						};
						out.insert(k, cast);
					},
					other => return Err(err!("unexpected key \"{}\", valid keys are \"left\", \"top\", \"right\", \
						\"bottom\", \"outside\", \"inside\", \"x\", \"y\", and \"rest\"", other; Input, Invalid)),
				}
			}
			Ok(Value::dict(out))
		},
		other => Err(mismatch("relative length, auto, or dictionary", &other)),
	}
}

// Show

pub fn show(_engine: &mut Engine, elem: &Content, styles: &StyleChain) -> Outcome<Option<Content>> {
	match elem.kind() {
		// `align` is its body under the alignment style; flow aligns every block and line by it.
		Some(ElemKind::Align) => {
			let body = match elem.field("body") {
				Some(v)	=> res!(v.clone().cast::<Content>()),
				None	=> Content::empty(),
			};
			let id = res!(fid(ElemKind::Align, "alignment"));
			let value = match res!(styles.resolve(elem, id)) {
				Some(v)	=> v,
				None	=> return Ok(Some(body)),
			};
			let set = Styles::from_style(Style::Property(Property::new(ElemKind::Align, id, value, elem.span())));
			Ok(Some(body.styled(set)))
		},
		_ => Ok(None),
	}
}

// Paper sizes

/// A paper's width and height in millimetres, by its Typst name.
pub fn paper(name: &str) -> Option<(f64, f64)> {
	let lower = name.to_lowercase();
	PAPERS.iter().find(|(n, _, _)| *n == lower).map(|(_, w, h)| (*w, *h))
}

// Typst 0.15.1's paper table, in millimetres.
const PAPERS: &[(&str, f64, f64)] = &[
	("a0",	841.0,	1189.0),
	("a1",	594.0,	841.0),
	("a2",	420.0,	594.0),
	("a3",	297.0,	420.0),
	("a4",	210.0,	297.0),
	("a5",	148.0,	210.0),
	("a6",	105.0,	148.0),
	("a7",	74.0,	105.0),
	("a8",	52.0,	74.0),
	("a9",	37.0,	52.0),
	("a10",	26.0,	37.0),
	("a11",	18.0,	26.0),
	("iso-b1",	707.0,	1000.0),
	("iso-b2",	500.0,	707.0),
	("iso-b3",	353.0,	500.0),
	("iso-b4",	250.0,	353.0),
	("iso-b5",	176.0,	250.0),
	("iso-b6",	125.0,	176.0),
	("iso-b7",	88.0,	125.0),
	("iso-b8",	62.0,	88.0),
	("iso-c3",	324.0,	458.0),
	("iso-c4",	229.0,	324.0),
	("iso-c5",	162.0,	229.0),
	("iso-c6",	114.0,	162.0),
	("iso-c7",	81.0,	114.0),
	("iso-c8",	57.0,	81.0),
	("din-d3",	272.0,	385.0),
	("din-d4",	192.0,	272.0),
	("din-d5",	136.0,	192.0),
	("din-d6",	96.0,	136.0),
	("din-d7",	68.0,	96.0),
	("din-d8",	48.0,	68.0),
	("sis-g5",	169.0,	239.0),
	("sis-e5",	115.0,	220.0),
	("ansi-a",	216.0,	279.0),
	("ansi-b",	279.0,	432.0),
	("ansi-c",	432.0,	559.0),
	("ansi-d",	559.0,	864.0),
	("ansi-e",	864.0,	1118.0),
	("arch-a",	229.0,	305.0),
	("arch-b",	305.0,	457.0),
	("arch-c",	457.0,	610.0),
	("arch-d",	610.0,	914.0),
	("arch-e1",	762.0,	1067.0),
	("arch-e",	914.0,	1219.0),
	("jis-b0",	1030.0,	1456.0),
	("jis-b1",	728.0,	1030.0),
	("jis-b2",	515.0,	728.0),
	("jis-b3",	364.0,	515.0),
	("jis-b4",	257.0,	364.0),
	("jis-b5",	182.0,	257.0),
	("jis-b6",	128.0,	182.0),
	("jis-b7",	91.0,	128.0),
	("jis-b8",	64.0,	91.0),
	("jis-b9",	45.0,	64.0),
	("jis-b10",	32.0,	45.0),
	("jis-b11",	22.0,	32.0),
	("sac-d0",	764.0,	1064.0),
	("sac-d1",	532.0,	760.0),
	("sac-d2",	380.0,	528.0),
	("sac-d3",	264.0,	376.0),
	("sac-d4",	188.0,	260.0),
	("sac-d5",	130.0,	184.0),
	("sac-d6",	92.0,	126.0),
	("iso-id-1",	85.6,	53.98),
	("iso-id-2",	74.0,	105.0),
	("iso-id-3",	88.0,	125.0),
	("asia-f4",	210.0,	330.0),
	("jp-shiroku-ban-4",	264.0,	379.0),
	("jp-shiroku-ban-5",	189.0,	262.0),
	("jp-shiroku-ban-6",	127.0,	188.0),
	("jp-kiku-4",	227.0,	306.0),
	("jp-kiku-5",	151.0,	227.0),
	("jp-business-card",	91.0,	55.0),
	("cn-business-card",	90.0,	54.0),
	("eu-business-card",	85.0,	55.0),
	("fr-tellière",	340.0,	440.0),
	("fr-couronne-écriture",	360.0,	460.0),
	("fr-couronne-édition",	370.0,	470.0),
	("fr-raisin",	500.0,	650.0),
	("fr-carré",	450.0,	560.0),
	("fr-jésus",	560.0,	760.0),
	("uk-brief",	406.4,	342.9),
	("uk-draft",	254.0,	406.4),
	("uk-foolscap",	203.2,	330.2),
	("uk-quarto",	203.2,	254.0),
	("uk-crown",	508.0,	381.0),
	("uk-book-a",	111.0,	178.0),
	("uk-book-b",	129.0,	198.0),
	("us-letter",	215.9,	279.4),
	("us-legal",	215.9,	355.6),
	("us-tabloid",	279.4,	431.8),
	("us-executive",	184.15,	266.7),
	("us-foolscap-folio",	215.9,	342.9),
	("us-statement",	139.7,	215.9),
	("us-ledger",	431.8,	279.4),
	("us-oficio",	215.9,	340.36),
	("us-gov-letter",	203.2,	266.7),
	("us-gov-legal",	215.9,	330.2),
	("us-business-card",	88.9,	50.8),
	("us-digest",	139.7,	215.9),
	("us-trade",	152.4,	228.6),
	("newspaper-compact",	280.0,	430.0),
	("newspaper-berliner",	315.0,	470.0),
	("newspaper-broadsheet",	381.0,	578.0),
	("presentation-16-9",	297.0,	167.0625),
	("presentation-4-3",	280.0,	210.0),
];

/// A layout field's default where the schema holds none (`Computed`) and Typst gives a value a read can show: the
/// page number's alignment, the spacing a block takes from its neighbours (`auto`, as the paragraph's own), the
/// alignment `align` and `place` take, and the direction a stack runs.
pub fn default_value(kind: ElemKind, name: &str) -> Option<Value> {
	match (kind, name) {
		(ElemKind::Page, "number-align")	=> Some(Value::Alignment(crate::eval::value::Alignment {
			x:	Some(crate::eval::value::HAlign::Center),
			y:	Some(crate::eval::value::VAlign::Bottom),
		})),
		(ElemKind::Block, "above") | (ElemKind::Block, "below")	=> Some(Value::Auto),
		(ElemKind::Align, "alignment")	=> Some(Value::Alignment(crate::eval::value::Alignment {
			x:	Some(crate::eval::value::HAlign::Start),
			y:	Some(crate::eval::value::VAlign::Top),
		})),
		(ElemKind::Place, "alignment")	=> Some(Value::Alignment(crate::eval::value::Alignment {
			x:	Some(crate::eval::value::HAlign::Start),
			y:	None,
		})),
		(ElemKind::Stack, "dir")		=> Some(Value::Direction(crate::eval::value::Direction::Ttb)),
		_	=> kind.field_id(name).and_then(|id| kind.field_spec(id)).and_then(|s| s.default.to_value()),
	}
}
