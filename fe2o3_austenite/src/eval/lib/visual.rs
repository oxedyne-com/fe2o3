// U6d owns this file. Schemas for images, shapes, curves and transforms, and the `stroke`, `tiling` and
// `polygon.regular` constructors. Every visual element but `hide` is a primitive flow lays out itself
// (`flow::visual`), so `show` is `None` for them; `hide` shows as its body under the `hidden` style.
//
// A field's value is cast to Typst's own type when the element is built, so `fields()` and field access
// see what Typst shows (`rect(width: 1pt).width` is `0% + 1pt`, a sides dictionary is expanded). A value
// that arrives through `set` is raw, so flow casts again on reading; every cast here is idempotent.

use crate::eval::args::Args;
use crate::eval::content::{
	Content,
	ElemKind,
	FieldDefault,
	FieldId,
	FieldSpec,
	FieldType,
	Fold,
};
use crate::eval::func::{
	Func,
	NativeFunc,
};
use crate::eval::scope::Scope;
use crate::eval::styles::{
	Property,
	Style,
	StyleChain,
	Styles,
};
use crate::eval::value::{
	Alignment,
	Angle,
	Dash,
	DashItem,
	Dict,
	HAlign,
	Length,
	LineCap,
	LineJoin,
	Paint,
	Ratio,
	Relative,
	RelativeTo,
	Stroke,
	Tiling,
	Type,
	VAlign,
	Value,
};
use crate::eval::Engine;
use crate::syntax::Span;

use oxedyne_fe2o3_core::prelude::*;

use std::sync::Arc;

native_fns! {
	pub enum VisualFn {
		Stroke			=> "stroke",
		Tiling			=> "tiling",
		PolygonRegular	=> "regular",
	}
}

// Schemas

const ANY: FieldType = FieldType::Any;

const RECT: &[FieldSpec] = &[
	FieldSpec::named("width",	ANY,	FieldDefault::Auto),
	FieldSpec::named("height",	ANY,	FieldDefault::Auto),
	FieldSpec::named("fill",	ANY,	FieldDefault::None),
	FieldSpec::named("stroke",	ANY,	FieldDefault::Auto).fold(Fold::Custom),
	FieldSpec::named("radius",	ANY,	FieldDefault::Pt(0.0)).fold(Fold::Custom),
	FieldSpec::named("inset",	ANY,	FieldDefault::Pt(5.0)).fold(Fold::Custom),
	FieldSpec::named("outset",	ANY,	FieldDefault::Pt(0.0)).fold(Fold::Custom),
	FieldSpec::named("body",	FieldType::Content,	FieldDefault::None).positional(),
];

// Square has rect's fields; `size` is an argument only, spread into width and height.
const SQUARE: &[FieldSpec] = RECT;

const ELLIPSE: &[FieldSpec] = &[
	FieldSpec::named("width",	ANY,	FieldDefault::Auto),
	FieldSpec::named("height",	ANY,	FieldDefault::Auto),
	FieldSpec::named("fill",	ANY,	FieldDefault::None),
	FieldSpec::named("stroke",	ANY,	FieldDefault::Auto).fold(Fold::Custom),
	FieldSpec::named("inset",	ANY,	FieldDefault::Pt(5.0)).fold(Fold::Custom),
	FieldSpec::named("outset",	ANY,	FieldDefault::Pt(0.0)).fold(Fold::Custom),
	FieldSpec::named("body",	FieldType::Content,	FieldDefault::None).positional(),
];

// Circle has ellipse's fields; `radius` is an argument only, spread into width and height.
const CIRCLE: &[FieldSpec] = ELLIPSE;

const LINE: &[FieldSpec] = &[
	FieldSpec::named("start",	ANY,	FieldDefault::Computed),
	FieldSpec::named("end",		ANY,	FieldDefault::None),
	FieldSpec::named("length",	ANY,	FieldDefault::Pt(30.0)),
	FieldSpec::named("angle",	ANY,	FieldDefault::Computed),
	FieldSpec::named("stroke",	ANY,	FieldDefault::Computed).fold(Fold::Custom),
];

const POLYGON: &[FieldSpec] = &[
	FieldSpec::named("fill",		ANY,	FieldDefault::None),
	FieldSpec::named("fill-rule",	ANY,	FieldDefault::Str("non-zero")),
	FieldSpec::named("stroke",		ANY,	FieldDefault::Auto).fold(Fold::Custom),
	FieldSpec::named("vertices",	ANY,	FieldDefault::EmptyArray).variadic().unsettable(),
];

const CURVE: &[FieldSpec] = &[
	FieldSpec::named("fill",		ANY,	FieldDefault::None),
	FieldSpec::named("fill-rule",	ANY,	FieldDefault::Str("non-zero")),
	FieldSpec::named("stroke",		ANY,	FieldDefault::Auto).fold(Fold::Custom),
	FieldSpec::named("components",	ANY,	FieldDefault::EmptyArray).variadic().unsettable(),
];

const CURVE_MOVE: &[FieldSpec] = &[
	FieldSpec::required("start",	ANY),
	FieldSpec::named("relative",	FieldType::Of(Type::Bool),	FieldDefault::Bool(false)),
];

const CURVE_LINE: &[FieldSpec] = &[
	FieldSpec::required("end",		ANY),
	FieldSpec::named("relative",	FieldType::Of(Type::Bool),	FieldDefault::Bool(false)),
];

const CURVE_QUAD: &[FieldSpec] = &[
	FieldSpec::required("control",	ANY),
	FieldSpec::required("end",		ANY),
	FieldSpec::named("relative",	FieldType::Of(Type::Bool),	FieldDefault::Bool(false)),
];

const CURVE_CUBIC: &[FieldSpec] = &[
	FieldSpec::required("control-start",	ANY),
	FieldSpec::required("control-end",		ANY),
	FieldSpec::required("end",				ANY),
	FieldSpec::named("relative",			FieldType::Of(Type::Bool),	FieldDefault::Bool(false)),
];

const CURVE_CLOSE: &[FieldSpec] = &[
	FieldSpec::named("mode",	ANY,	FieldDefault::Str("smooth")),
];

const MOVE: &[FieldSpec] = &[
	FieldSpec::named("dx",		ANY,	FieldDefault::Pt(0.0)),
	FieldSpec::named("dy",		ANY,	FieldDefault::Pt(0.0)),
	FieldSpec::required("body",	FieldType::Content),
];

const ROTATE: &[FieldSpec] = &[
	FieldSpec::named("angle",	ANY,	FieldDefault::Computed).positional(),
	FieldSpec::named("origin",	ANY,	FieldDefault::Computed).fold(Fold::Custom),
	FieldSpec::named("reflow",	FieldType::Of(Type::Bool),	FieldDefault::Bool(false)),
	FieldSpec::required("body",	FieldType::Content),
];

const SCALE: &[FieldSpec] = &[
	FieldSpec::named("x",		ANY,	FieldDefault::Ratio(1.0)),
	FieldSpec::named("y",		ANY,	FieldDefault::Ratio(1.0)),
	FieldSpec::named("origin",	ANY,	FieldDefault::Computed).fold(Fold::Custom),
	FieldSpec::named("reflow",	FieldType::Of(Type::Bool),	FieldDefault::Bool(false)),
	FieldSpec::required("body",	FieldType::Content),
];

const SKEW: &[FieldSpec] = &[
	FieldSpec::named("ax",		ANY,	FieldDefault::Computed),
	FieldSpec::named("ay",		ANY,	FieldDefault::Computed),
	FieldSpec::named("origin",	ANY,	FieldDefault::Computed).fold(Fold::Custom),
	FieldSpec::named("reflow",	FieldType::Of(Type::Bool),	FieldDefault::Bool(false)),
	FieldSpec::required("body",	FieldType::Content),
];

// `hidden` is Typst's internal ghost field: set as a style on the body by `hide`'s show, never passed.
const HIDE: &[FieldSpec] = &[
	FieldSpec::required("body",	FieldType::Content),
	FieldSpec::named("hidden",	FieldType::Of(Type::Bool),	FieldDefault::Bool(false)).unsettable(),
];

const REPEAT: &[FieldSpec] = &[
	FieldSpec::required("body",	FieldType::Content),
	FieldSpec::named("gap",		ANY,	FieldDefault::Pt(0.0)),
	FieldSpec::named("justify",	FieldType::Of(Type::Bool),	FieldDefault::Bool(true)),
];

const IMAGE: &[FieldSpec] = &[
	FieldSpec::required("source",	ANY),
	FieldSpec::named("format",		ANY,	FieldDefault::Auto),
	FieldSpec::named("width",		ANY,	FieldDefault::Auto),
	FieldSpec::named("height",		ANY,	FieldDefault::Auto),
	FieldSpec::named("alt",			ANY,	FieldDefault::None),
	FieldSpec::named("page",		FieldType::Of(Type::Int),	FieldDefault::Int(1)),
	FieldSpec::named("fit",			ANY,	FieldDefault::Str("cover")),
	FieldSpec::named("scaling",		ANY,	FieldDefault::Auto),
	FieldSpec::named("icc",			ANY,	FieldDefault::Auto),
];

pub fn fields(kind: ElemKind) -> &'static [FieldSpec] {
	match kind {
		ElemKind::Image			=> IMAGE,
		ElemKind::Line			=> LINE,
		ElemKind::Rect			=> RECT,
		ElemKind::Square		=> SQUARE,
		ElemKind::Circle		=> CIRCLE,
		ElemKind::Ellipse		=> ELLIPSE,
		ElemKind::Polygon		=> POLYGON,
		ElemKind::Curve			=> CURVE,
		ElemKind::CurveMove		=> CURVE_MOVE,
		ElemKind::CurveLine		=> CURVE_LINE,
		ElemKind::CurveQuad		=> CURVE_QUAD,
		ElemKind::CurveCubic	=> CURVE_CUBIC,
		ElemKind::CurveClose	=> CURVE_CLOSE,
		ElemKind::Move			=> MOVE,
		ElemKind::Scale			=> SCALE,
		ElemKind::Rotate		=> ROTATE,
		ElemKind::Skew			=> SKEW,
		ElemKind::Hide			=> HIDE,
		ElemKind::Repeat		=> REPEAT,
		// Typst 0.15 has no `path` element: `path` is the file-path type, which the drawing element's name
		// was given to. The kind stays in U0's closed list and has no fields.
		ElemKind::Path			=> &[],
		_						=> &[],
	}
}

/// The field's id in the element's schema. Every name passed here is one this file declares.
pub fn fid(kind: ElemKind, name: &str) -> Outcome<FieldId> {
	kind.field_id(name).ok_or_else(|| err!(
		"{} has no field `{}` in its schema.", kind.path(), name; Bug, Missing))
}

// Library

pub fn define(scope: &mut Scope) {
	scope.define("stroke", Value::Func(Func::Native(NativeFunc::Visual(VisualFn::Stroke))));
	scope.define("tiling", Value::Func(Func::Native(NativeFunc::Visual(VisualFn::Tiling))));
}

/// A native function reached through an element function, as `polygon.regular` is.
pub fn scoped(kind: ElemKind, name: &str) -> Option<VisualFn> {
	match (kind, name) {
		(ElemKind::Polygon, "regular")	=> Some(VisualFn::PolygonRegular),
		_								=> None,
	}
}

/// The constructor a type value calls to, for `stroke(..)` and `tiling(..)` reached through the type.
pub fn constructor(ty: Type) -> Option<VisualFn> {
	match ty {
		Type::Stroke	=> Some(VisualFn::Stroke),
		Type::Tiling	=> Some(VisualFn::Tiling),
		_				=> None,
	}
}

pub fn call(f: VisualFn, engine: &mut Engine, mut args: Args) -> Outcome<Value> {
	let span = args.span;
	let out = match f {
		VisualFn::Stroke			=> stroke_constructor(&mut args),
		VisualFn::Tiling			=> tiling_constructor(&mut args),
		VisualFn::PolygonRegular	=> polygon_regular(&mut args, span),
	};
	match out {
		Ok(v) => match args.finish() {
			Ok(())	=> Ok(v),
			Err(e)	=> Err(engine.error(span, err_text(&e))),
		},
		Err(e) => Err(engine.error(span, err_text(&e))),
	}
}

// The innermost message of an error made by a cast, which is what the diagnostic reports.
fn err_text(e: &Error<ErrTag>) -> String {
	match e.msgs().into_iter().last() {
		Some(m)	=> m,
		None	=> fmt!("{}", e),
	}
}

fn stroke_constructor(args: &mut Args) -> Outcome<Value> {
	if let Some(v) = res!(args.eat::<Value>()) {
		return Ok(Value::Stroke(Arc::new(res!(cast_stroke(v)))));
	}
	let mut s = Stroke::default();
	if let Some(v) = res!(args.named::<Value>("paint")) {
		s.paint = res!(smart(v, cast_paint_some));
	}
	if let Some(v) = res!(args.named::<Value>("thickness")) {
		s.thickness = res!(smart(v, cast_length));
	}
	if let Some(v) = res!(args.named::<Value>("cap")) {
		s.cap = res!(smart(v, cast_cap));
	}
	if let Some(v) = res!(args.named::<Value>("join")) {
		s.join = res!(smart(v, cast_join));
	}
	if let Some(v) = res!(args.named::<Value>("dash")) {
		s.dash = res!(smart(v, cast_dash));
	}
	if let Some(v) = res!(args.named::<Value>("miter-limit")) {
		s.miter_limit = res!(smart(v, |v| v.cast::<f64>()));
	}
	Ok(Value::Stroke(Arc::new(s)))
}

fn tiling_constructor(args: &mut Args) -> Outcome<Value> {
	let size = match res!(args.named::<Value>("size")) {
		None | Some(Value::Auto)	=> None,
		Some(v)						=> Some(res!(length_pair(v, "size"))),
	};
	let spacing = match res!(args.named::<Value>("spacing")) {
		None	=> (Length::zero(), Length::zero()),
		Some(v)	=> res!(length_pair(v, "spacing")),
	};
	let relative = match res!(args.named::<Value>("relative")) {
		None | Some(Value::Auto)	=> RelativeTo::Auto,
		Some(v)						=> res!(cast_relative_to(v)),
	};
	let body = res!(args.expect::<Content>("body"));
	Ok(Value::Tiling(Arc::new(Tiling { body, size, spacing, relative })))
}

fn polygon_regular(args: &mut Args, span: Span) -> Outcome<Value> {
	let fill	= res!(args.named::<Value>("fill"));
	let stroke	= res!(args.named::<Value>("stroke"));
	let size	= match res!(args.named::<Value>("size")) {
		Some(v)	=> res!(cast_length(v)),
		None	=> Length::em(1.0),
	};
	let n = match res!(args.named::<Value>("vertices")) {
		Some(v)	=> res!(v.cast::<i64>()),
		None	=> 3,
	};
	if n < 0 {
		return Err(err!("number must be at least zero"; Input, Range));
	}
	// Each coordinate is the radius times a factor, so the minima that normalise the polygon into its
	// box are taken on the factors, which keeps an `em` size unresolved as Typst keeps it.
	let nf		= n as f64;
	let angle	= |i: f64| 2.0 * std::f64::consts::PI * i / nf + std::f64::consts::PI * (0.5 - 1.0 / nf);
	let mut min_x = 1.0f64;
	let mut min_y = 1.0f64;
	for v in 0..=n {
		let a = angle(v as f64);
		min_x = min_x.min(a.cos() + 1.0);
		min_y = min_y.min(a.sin() + 1.0);
	}
	let r = Length { abs: size.abs / 2.0, em: size.em / 2.0 };
	let mut verts = Vec::new();
	for v in 0..=n {
		let a	= angle(v as f64);
		let fx	= a.cos() + 1.0 - min_x;
		let fy	= a.sin() + 1.0 - min_y;
		verts.push(Value::array(vec![
			Value::Relative(rel_of(Length { abs: r.abs * fx, em: r.em * fx })),
			Value::Relative(rel_of(Length { abs: r.abs * fy, em: r.em * fy })),
		]));
	}
	let kind		= ElemKind::Polygon;
	let mut fields	= Vec::new();
	if let Some(v) = fill {
		fields.push((res!(fid(kind, "fill")), res!(cast_fill(v))));
	}
	if let Some(v) = stroke {
		fields.push((res!(fid(kind, "stroke")), res!(cast_smart_stroke(v))));
	}
	fields.push((res!(fid(kind, "vertices")), Value::array(verts)));
	Ok(Value::Content(Content::new(kind, fields, span)))
}

// Construction

pub fn construct(engine: &mut Engine, kind: ElemKind, args: &mut Args) -> Outcome<Option<Content>> {
	let span = args.span;
	let built = match kind {
		ElemKind::Rect | ElemKind::Square | ElemKind::Ellipse | ElemKind::Circle
								=> build_shape(kind, args),
		ElemKind::Line			=> build_line(args),
		ElemKind::Polygon		=> build_polygon(args),
		ElemKind::Curve			=> build_curve(args),
		ElemKind::CurveMove		=> build_component(kind, args, &["start"]),
		ElemKind::CurveLine		=> build_component(kind, args, &["end"]),
		ElemKind::CurveQuad		=> build_component(kind, args, &["control", "end"]),
		ElemKind::CurveCubic	=> build_component(kind, args, &["control-start", "control-end", "end"]),
		ElemKind::CurveClose	=> build_close(args),
		ElemKind::Move			=> build_move(args),
		ElemKind::Rotate		=> build_rotate(args),
		ElemKind::Scale			=> build_scale(args),
		ElemKind::Skew			=> build_skew(args),
		ElemKind::Hide			=> build_body_only(kind, args),
		ElemKind::Repeat		=> build_repeat(args),
		ElemKind::Image			=> build_image(engine, args),
		ElemKind::Path			=> Err(err!(
			"Typst 0.15 has no `path` drawing element; use `curve` instead"; Input, Invalid)),
		_						=> return Ok(None),
	};
	let fields = match built {
		Ok(f)	=> f,
		Err(e)	=> return Err(engine.error(span, err_text(&e))),
	};
	if let Err(e) = std::mem::take(args).finish() {
		return Err(engine.error(span, err_text(&e)));
	}
	Ok(Some(Content::new(kind, fields, span)))
}

type Fields = Vec<(FieldId, Value)>;

// Takes a named argument and casts it into the field of the same name.
fn take_named(fields: &mut Fields, kind: ElemKind, args: &mut Args, name: &str) -> Outcome<()> {
	if let Some(v) = res!(args.named::<Value>(name)) {
		fields.push((res!(fid(kind, name)), res!(cast_field(kind, name, v))));
	}
	Ok(())
}

/// Casts a value into a visual element's field as Typst casts it. The constructors and `set` rules
/// both cast through here, so a value `set` accepts is one construction accepts.
pub fn cast_field(kind: ElemKind, name: &str, v: Value) -> Outcome<Value> {
	use ElemKind as K;
	match (kind, name) {
		(_, "width")						=> cast_smart_rel(v),
		(_, "height")						=> cast_sizing(v),
		(_, "fill")							=> cast_fill(v),
		(_, "fill-rule")					=> cast_fill_rule(v),
		(K::Rect | K::Square, "stroke")		=> cast_stroke_sides(v),
		(K::Line, "stroke")					=> Ok(Value::Stroke(Arc::new(res!(cast_stroke(v))))),
		(_, "stroke")						=> cast_smart_stroke(v),
		(_, "radius")						=> cast_corners_rel(v),
		(_, "inset") | (_, "outset")		=> cast_sides_rel(v),
		(K::Line, "start")					=> cast_point(v),
		(K::Line, "end")					=> if v.is_none() { Ok(v) } else { cast_point(v) },
		(K::Line, "length")					=> Ok(Value::Relative(res!(cast_rel(v)))),
		(K::Line, "angle") | (K::Rotate, "angle") | (K::Skew, "ax") | (K::Skew, "ay")
											=> Ok(Value::Angle(res!(cast_angle(v)))),
		(K::Move, "dx") | (K::Move, "dy")	=> Ok(Value::Relative(res!(cast_rel(v)))),
		(K::Scale, "x") | (K::Scale, "y")	=> cast_scale_amount(v),
		(_, "origin")						=> Ok(Value::Alignment(res!(v.cast::<Alignment>()))),
		(_, "reflow") | (_, "relative") | (_, "justify")
											=> Ok(Value::Bool(res!(v.cast::<bool>()))),
		(K::Repeat, "gap")					=> Ok(Value::Length(res!(cast_length(v)))),
		(K::CurveClose, "mode") => {
			let s = res!(v.cast::<String>());
			match s.as_str() {
				"smooth" | "straight"	=> Ok(Value::str(s)),
				_						=> Err(err!("expected \"smooth\" or \"straight\""; Input, Invalid)),
			}
		}
		(K::Image, "format") => match v {
			Value::Auto => Ok(v),
			other => {
				let s = res!(other.cast::<String>());
				match s.as_str() {
					"png" | "jpg" | "gif" | "webp" | "svg" | "pdf"	=> Ok(Value::str(s)),
					_ => Err(err!("expected \"png\", \"jpg\", \"gif\", \"webp\", \"svg\", \"pdf\", \
						dictionary, or auto"; Input, Invalid)),
				}
			},
		},
		(K::Image, "alt") => match v {
			Value::None	=> Ok(v),
			other		=> Ok(Value::str(res!(other.cast::<String>()))),
		},
		(K::Image, "page")					=> Ok(Value::Int(res!(v.cast::<i64>()))),
		(K::Image, "fit") => {
			let s = res!(v.cast::<String>());
			match s.as_str() {
				"cover" | "contain" | "stretch"	=> Ok(Value::str(s)),
				_ => Err(err!("expected \"cover\", \"contain\", or \"stretch\""; Input, Invalid)),
			}
		}
		(K::Image, "scaling") => match v {
			Value::Auto => Ok(v),
			other => {
				let s = res!(other.cast::<String>());
				match s.as_str() {
					"smooth" | "pixelated"	=> Ok(Value::str(s)),
					_ => Err(err!("expected \"smooth\", \"pixelated\", or auto"; Input, Invalid)),
				}
			},
		},
		_									=> Ok(v),
	}
}

fn build_shape(kind: ElemKind, args: &mut Args) -> Outcome<Fields> {
	let mut f = Vec::new();
	let round = matches!(kind, ElemKind::Ellipse | ElemKind::Circle);
	// The external sizing arguments come first, as Typst parses them: `square(size:)` and
	// `circle(radius:)` set width and height together and take precedence over them.
	let (w, h) = match kind {
		ElemKind::Square => match res!(args.named::<Value>("size")) {
			Some(Value::Auto) => {
				let _ = res!(args.named::<Value>("width"));
				let _ = res!(args.named::<Value>("height"));
				(Some(Value::Auto), Some(Value::Auto))
			},
			Some(v) => {
				let r = Value::Relative(rel_of(res!(cast_length(v))));
				let _ = res!(args.named::<Value>("width"));
				let _ = res!(args.named::<Value>("height"));
				(Some(r.clone()), Some(r))
			},
			None => (res!(args.named::<Value>("width")), res!(args.named::<Value>("height"))),
		},
		ElemKind::Circle => match res!(args.named::<Value>("radius")) {
			Some(v) => {
				let l = res!(cast_length(v));
				let r = Value::Relative(rel_of(Length { abs: l.abs * 2.0, em: l.em * 2.0 }));
				let _ = res!(args.named::<Value>("width"));
				let _ = res!(args.named::<Value>("height"));
				(Some(r.clone()), Some(r))
			},
			None => (res!(args.named::<Value>("width")), res!(args.named::<Value>("height"))),
		},
		_ => (res!(args.named::<Value>("width")), res!(args.named::<Value>("height"))),
	};
	if let Some(v) = w {
		f.push((res!(fid(kind, "width")), res!(cast_field(kind, "width", v))));
	}
	if let Some(v) = h {
		f.push((res!(fid(kind, "height")), res!(cast_field(kind, "height", v))));
	}
	res!(take_named(&mut f, kind, args, "fill"));
	if round {
		res!(take_named(&mut f, kind, args, "stroke"));
	} else {
		res!(take_named(&mut f, kind, args, "stroke"));
		res!(take_named(&mut f, kind, args, "radius"));
	}
	res!(take_named(&mut f, kind, args, "inset"));
	res!(take_named(&mut f, kind, args, "outset"));
	if let Some(v) = res!(args.eat::<Value>()) {
		f.push((res!(fid(kind, "body")), res!(cast_opt_content(v))));
	}
	Ok(f)
}

fn build_line(args: &mut Args) -> Outcome<Fields> {
	let kind	= ElemKind::Line;
	let mut f	= Vec::new();
	res!(take_named(&mut f, kind, args, "start"));
	res!(take_named(&mut f, kind, args, "end"));
	res!(take_named(&mut f, kind, args, "length"));
	res!(take_named(&mut f, kind, args, "angle"));
	res!(take_named(&mut f, kind, args, "stroke"));
	Ok(f)
}

fn build_polygon(args: &mut Args) -> Outcome<Fields> {
	let kind	= ElemKind::Polygon;
	let mut f	= Vec::new();
	res!(take_named(&mut f, kind, args, "fill"));
	res!(take_named(&mut f, kind, args, "fill-rule"));
	res!(take_named(&mut f, kind, args, "stroke"));
	let mut verts = Vec::new();
	for v in res!(args.all::<Value>()) {
		verts.push(res!(cast_point(v)));
	}
	f.push((res!(fid(kind, "vertices")), Value::array(verts)));
	Ok(f)
}

fn build_curve(args: &mut Args) -> Outcome<Fields> {
	let kind	= ElemKind::Curve;
	let mut f	= Vec::new();
	res!(take_named(&mut f, kind, args, "fill"));
	res!(take_named(&mut f, kind, args, "fill-rule"));
	res!(take_named(&mut f, kind, args, "stroke"));
	let mut comps = Vec::new();
	for v in res!(args.all::<Value>()) {
		let ok = match &v {
			Value::Content(c) => matches!(c.kind(), Some(ElemKind::CurveMove) | Some(ElemKind::CurveLine)
				| Some(ElemKind::CurveQuad) | Some(ElemKind::CurveCubic) | Some(ElemKind::CurveClose)),
			_ => false,
		};
		if !ok {
			return Err(err!("expected curve component, found {}", v.ty().long_name(); Input, Mismatch));
		}
		comps.push(v);
	}
	f.push((res!(fid(kind, "components")), Value::array(comps)));
	Ok(f)
}

// A curve component: its required points in order, then `relative`. A quad's control may be `auto` or
// `none`; a cubic's start control may be `auto` or `none` and its end control `none`.
fn build_component(kind: ElemKind, args: &mut Args, points: &[&str]) -> Outcome<Fields> {
	let mut f = Vec::new();
	for name in points {
		let v = res!(args.expect::<Value>(name));
		let cast = match (kind, *name) {
			(ElemKind::CurveQuad, "control") | (ElemKind::CurveCubic, "control-start") => match v {
				Value::Auto | Value::None	=> v,
				other						=> res!(cast_point(other)),
			},
			(ElemKind::CurveCubic, "control-end") => match v {
				Value::None	=> v,
				other		=> res!(cast_point(other)),
			},
			_ => res!(cast_point(v)),
		};
		f.push((res!(fid(kind, name)), cast));
	}
	res!(take_named(&mut f, kind, args, "relative"));
	Ok(f)
}

fn build_close(args: &mut Args) -> Outcome<Fields> {
	let kind	= ElemKind::CurveClose;
	let mut f	= Vec::new();
	res!(take_named(&mut f, kind, args, "mode"));
	Ok(f)
}

fn take_body(f: &mut Fields, kind: ElemKind, args: &mut Args) -> Outcome<()> {
	let body = res!(args.expect::<Value>("body"));
	f.push((res!(fid(kind, "body")), res!(cast_content(body))));
	Ok(())
}

fn take_origin_reflow(f: &mut Fields, kind: ElemKind, args: &mut Args) -> Outcome<()> {
	res!(take_named(f, kind, args, "origin"));
	res!(take_named(f, kind, args, "reflow"));
	Ok(())
}

fn build_move(args: &mut Args) -> Outcome<Fields> {
	let kind	= ElemKind::Move;
	let mut f	= Vec::new();
	res!(take_named(&mut f, kind, args, "dx"));
	res!(take_named(&mut f, kind, args, "dy"));
	res!(take_body(&mut f, kind, args));
	Ok(f)
}

fn build_rotate(args: &mut Args) -> Outcome<Fields> {
	let kind	= ElemKind::Rotate;
	let mut f	= Vec::new();
	// The angle is positional but optional: the first positional argument that is an angle.
	let named = res!(args.named::<Value>("angle"));
	let pos = res!(args.find::<Value>(|v| matches!(v, Value::Angle(_))));
	if let Some(v) = pos.or(named) {
		f.push((res!(fid(kind, "angle")), res!(cast_field(kind, "angle", v))));
	}
	res!(take_origin_reflow(&mut f, kind, args));
	res!(take_body(&mut f, kind, args));
	Ok(f)
}

fn build_scale(args: &mut Args) -> Outcome<Fields> {
	let kind	= ElemKind::Scale;
	let mut f	= Vec::new();
	// `factor` is external: a positional ratio or length (or `auto`) that sets both axes unless an axis
	// is given by name.
	let factor = res!(args.find::<Value>(|v| matches!(v,
		Value::Ratio(_) | Value::Length(_) | Value::Auto)));
	let x = res!(args.named::<Value>("x")).or_else(|| factor.clone());
	let y = res!(args.named::<Value>("y")).or(factor);
	if let Some(v) = x {
		f.push((res!(fid(kind, "x")), res!(cast_field(kind, "x", v))));
	}
	if let Some(v) = y {
		f.push((res!(fid(kind, "y")), res!(cast_field(kind, "y", v))));
	}
	res!(take_origin_reflow(&mut f, kind, args));
	res!(take_body(&mut f, kind, args));
	Ok(f)
}

fn build_skew(args: &mut Args) -> Outcome<Fields> {
	let kind	= ElemKind::Skew;
	let mut f	= Vec::new();
	res!(take_named(&mut f, kind, args, "ax"));
	res!(take_named(&mut f, kind, args, "ay"));
	res!(take_origin_reflow(&mut f, kind, args));
	res!(take_body(&mut f, kind, args));
	Ok(f)
}

fn build_body_only(kind: ElemKind, args: &mut Args) -> Outcome<Fields> {
	let mut f = Vec::new();
	res!(take_body(&mut f, kind, args));
	Ok(f)
}

fn build_repeat(args: &mut Args) -> Outcome<Fields> {
	let kind	= ElemKind::Repeat;
	let mut f	= Vec::new();
	res!(take_body(&mut f, kind, args));
	res!(take_named(&mut f, kind, args, "gap"));
	res!(take_named(&mut f, kind, args, "justify"));
	Ok(f)
}

fn build_image(engine: &mut Engine, args: &mut Args) -> Outcome<Fields> {
	let kind	= ElemKind::Image;
	let span	= args.span;
	let mut f	= Vec::new();
	let source	= res!(args.expect::<Value>("source"));
	match &source {
		// A path is resolved now, as Typst reads the file when the element is built, so a missing file
		// is reported at the call.
		Value::Str(p) => {
			let _ = res!(crate::eval::import::resolve_path(engine, p, span.file, span));
		},
		Value::Bytes(_) => (),
		other => return Err(err!("expected string or bytes, found {}", other.ty().long_name(); Input, Mismatch)),
	}
	f.push((res!(fid(kind, "source")), source));
	res!(take_named(&mut f, kind, args, "format"));
	res!(take_named(&mut f, kind, args, "width"));
	res!(take_named(&mut f, kind, args, "height"));
	res!(take_named(&mut f, kind, args, "alt"));
	res!(take_named(&mut f, kind, args, "page"));
	res!(take_named(&mut f, kind, args, "fit"));
	res!(take_named(&mut f, kind, args, "scaling"));
	res!(take_named(&mut f, kind, args, "icc"));
	Ok(f)
}

// Show

pub fn show(engine: &mut Engine, elem: &Content, _styles: &StyleChain) -> Outcome<Option<Content>> {
	match elem.kind() {
		// Typst's `hide`: the body under the `hidden` style, which every layout of ink honours by laying
		// the content out and drawing none of it.
		Some(ElemKind::Hide) => {
			let body = match elem.field("body") {
				Some(v)	=> res!(cast_content(v.clone()).and_then(|v| v.cast::<Content>())),
				None	=> Content::empty(),
			};
			let id = res!(fid(ElemKind::Hide, "hidden"));
			let styles = Styles::from_style(Style::Property(Property {
				elem:	ElemKind::Hide,
				field:	id,
				value:	Value::Bool(true),
				span:	elem.span(),
			}));
			let _ = engine;
			Ok(Some(body.styled(styles)))
		},
		_ => Ok(None),
	}
}

/// Is content under these styles hidden by an enclosing `hide`? Every producer of ink asks this and
/// lays the content out without drawing it.
pub fn is_hidden(styles: &StyleChain) -> Outcome<bool> {
	match ElemKind::Hide.field_id("hidden") {
		Some(id)	=> Ok(matches!(res!(styles.get(ElemKind::Hide, id)), Some(Value::Bool(true)))),
		None		=> Ok(false),
	}
}

// Folding

/// Folds a `Fold::Custom` field of a visual element: `inner` (the nearer value) over `outer`. Strokes fold
/// part by part (`stroke: red` over `stroke: 2pt` is a red 2pt stroke), sides and corners side by side,
/// and an origin axis by axis; `auto` and a plain value replace.
pub fn fold(kind: ElemKind, field: &str, inner: Value, outer: Value) -> Outcome<Value> {
	match field {
		"stroke" => match kind {
			ElemKind::Rect | ElemKind::Square	=> fold_sides(inner, outer, cast_stroke_sides, fold_opt_stroke),
			ElemKind::Line						=> fold_stroke_values(inner, outer),
			_									=> fold_opt_stroke(inner, outer),
		},
		"radius"			=> fold_corners(inner, outer),
		"inset" | "outset"	=> fold_sides(inner, outer, cast_sides_rel, |i, _| Ok(i)),
		"origin" => {
			let i = res!(inner.cast::<Alignment>());
			let o = match outer {
				Value::Alignment(a)	=> a,
				_					=> Alignment::default(),
			};
			Ok(Value::Alignment(Alignment { x: i.x.or(o.x), y: i.y.or(o.y) }))
		},
		_ => Ok(inner),
	}
}

// Two `Smart<Option<Stroke>>` values: strokes fold, `auto` or `none` inner replaces.
fn fold_opt_stroke(inner: Value, outer: Value) -> Outcome<Value> {
	match (&inner, &outer) {
		(Value::Auto, _) | (Value::None, _)	=> Ok(inner),
		(_, Value::Auto) | (_, Value::None)	=> Ok(res!(cast_smart_stroke(inner))),
		_									=> fold_stroke_values(inner, outer),
	}
}

fn fold_stroke_values(inner: Value, outer: Value) -> Outcome<Value> {
	let i = res!(cast_stroke(inner));
	let o = res!(cast_stroke(outer));
	Ok(Value::Stroke(Arc::new(Stroke {
		paint:			i.paint.or(o.paint),
		thickness:		i.thickness.or(o.thickness),
		cap:			i.cap.or(o.cap),
		join:			i.join.or(o.join),
		dash:			i.dash.or(o.dash),
		miter_limit:	i.miter_limit.or(o.miter_limit),
	})))
}

fn fold_sides<C, F>(inner: Value, outer: Value, cast: C, each: F) -> Outcome<Value>
	where
		C: Fn(Value) -> Outcome<Value>,
		F: Fn(Value, Value) -> Outcome<Value>,
{
	if inner.is_auto() || outer.is_auto() {
		return cast(inner);
	}
	let i = sides_of(&res!(cast(inner)));
	let o = sides_of(&res!(cast(outer)));
	let mut out = [None, None, None, None];
	for k in 0..4 {
		out[k] = match (i[k].clone(), o[k].clone()) {
			(Some(a), Some(b))	=> Some(res!(each(a, b))),
			(Some(a), None)		=> Some(a),
			(None, b)			=> b,
		};
	}
	Ok(sides_value(out))
}

fn fold_corners(inner: Value, outer: Value) -> Outcome<Value> {
	let i = corners_of(&res!(cast_corners_rel(inner)));
	let o = corners_of(&res!(cast_corners_rel(outer)));
	let mut out = [None, None, None, None];
	for k in 0..4 {
		out[k] = i[k].clone().or_else(|| o[k].clone());
	}
	Ok(corners_value(out))
}

// Casts

fn mismatch(expected: &str, found: &Value) -> Error<ErrTag> {
	err!("expected {}, found {}", expected, found.ty().long_name(); Input, Mismatch)
}

fn rel_of(l: Length) -> Relative { Relative { rel: Ratio(0.0), abs: l } }

fn smart<T, F>(v: Value, f: F) -> Outcome<Option<T>>
	where F: Fn(Value) -> Outcome<T>
{
	match v {
		Value::Auto	=> Ok(None),
		other		=> Ok(Some(res!(f(other)))),
	}
}

pub fn cast_length(v: Value) -> Outcome<Length> {
	match v {
		Value::Length(l)	=> Ok(l),
		other				=> Err(mismatch("length", &other)),
	}
}

pub fn cast_rel(v: Value) -> Outcome<Relative> {
	match v {
		Value::Relative(r)	=> Ok(r),
		Value::Length(l)	=> Ok(rel_of(l)),
		Value::Ratio(r)		=> Ok(Relative { rel: r, abs: Length::zero() }),
		other				=> Err(mismatch("relative length", &other)),
	}
}

pub fn cast_angle(v: Value) -> Outcome<Angle> {
	match v {
		Value::Angle(a)	=> Ok(a),
		other			=> Err(mismatch("angle", &other)),
	}
}

fn cast_smart_rel(v: Value) -> Outcome<Value> {
	match v {
		Value::Auto	=> Ok(v),
		other		=> Ok(Value::Relative(res!(cast_rel(other)))),
	}
}

// Typst's `Sizing`: `auto`, a relative length or a fraction.
fn cast_sizing(v: Value) -> Outcome<Value> {
	match v {
		Value::Auto | Value::Fraction(_)	=> Ok(v),
		Value::Relative(_) | Value::Length(_) | Value::Ratio(_)
											=> Ok(Value::Relative(res!(cast_rel(v)))),
		other => Err(mismatch("relative length, fraction, or auto", &other)),
	}
}

fn cast_scale_amount(v: Value) -> Outcome<Value> {
	match v {
		Value::Auto | Value::Ratio(_) | Value::Length(_)	=> Ok(v),
		other => Err(mismatch("ratio, length, or auto", &other)),
	}
}

/// A point: an array of exactly two relative lengths, stored as two `Relative`s.
pub fn cast_point(v: Value) -> Outcome<Value> {
	match v {
		Value::Array(a) => {
			if a.len() != 2 {
				return Err(err!("point array must contain exactly two entries"; Input, Invalid));
			}
			Ok(Value::array(vec![
				Value::Relative(res!(cast_rel(a[0].clone()))),
				Value::Relative(res!(cast_rel(a[1].clone()))),
			]))
		},
		other => Err(mismatch("array", &other)),
	}
}

/// A point's two coordinates, from a value `cast_point` accepts.
pub fn point_of(v: &Value) -> Outcome<(Relative, Relative)> {
	match res!(cast_point(v.clone())) {
		Value::Array(a) if a.len() == 2 => Ok((res!(cast_rel(a[0].clone())), res!(cast_rel(a[1].clone())))),
		_ => Err(err!("point array must contain exactly two entries"; Input, Invalid)),
	}
}

fn cast_content(v: Value) -> Outcome<Value> {
	match v {
		Value::Content(_)	=> Ok(v),
		Value::Str(s)		=> Ok(Value::Content(Content::text(&s))),
		Value::Int(i)		=> Ok(Value::Content(Content::text(&fmt!("{}", i)))),
		Value::Float(x)		=> Ok(Value::Content(Content::text(&fmt!("{}", x)))),
		Value::Symbol(_)	=> Ok(v),
		Value::None			=> Ok(Value::Content(Content::empty())),
		other				=> Err(mismatch("content", &other)),
	}
}

fn cast_opt_content(v: Value) -> Outcome<Value> {
	match v {
		Value::None	=> Ok(v),
		other		=> cast_content(other),
	}
}

fn cast_paint_some(v: Value) -> Outcome<Paint> {
	match v {
		Value::Color(c)		=> Ok(Paint::Color(c)),
		Value::Gradient(g)	=> Ok(Paint::Gradient(g)),
		Value::Tiling(t)	=> Ok(Paint::Tiling(t)),
		other				=> Err(mismatch("color, gradient, or tiling", &other)),
	}
}

/// A paint as a value, the inverse of the paint cast.
pub fn paint_value(p: &Paint) -> Value {
	match p {
		Paint::Color(c)		=> Value::Color(*c),
		Paint::Gradient(g)	=> Value::Gradient(g.clone()),
		Paint::Tiling(t)	=> Value::Tiling(t.clone()),
	}
}

/// `fill`: `none` or a paint.
pub fn cast_fill(v: Value) -> Outcome<Value> {
	match v {
		Value::None => Ok(v),
		other => match res!(cast_paint_some(other.clone()).map_err(|_| mismatch("color, gradient, tiling, or none", &other))) {
			p => Ok(paint_value(&p)),
		},
	}
}

pub fn fill_of(v: &Value) -> Outcome<Option<Paint>> {
	match v {
		Value::None | Value::Auto	=> Ok(None),
		other						=> Ok(Some(res!(cast_paint_some(other.clone())))),
	}
}

fn cast_fill_rule(v: Value) -> Outcome<Value> {
	let s = res!(v.cast::<String>());
	match s.as_str() {
		"non-zero" | "even-odd"	=> Ok(Value::str(s)),
		_ => Err(err!("expected \"non-zero\" or \"even-odd\""; Input, Invalid)),
	}
}

fn cast_cap(v: Value) -> Outcome<LineCap> {
	let s = res!(v.cast::<String>());
	match s.as_str() {
		"butt"		=> Ok(LineCap::Butt),
		"round"		=> Ok(LineCap::Round),
		"square"	=> Ok(LineCap::Square),
		_ => Err(err!("expected \"butt\", \"round\", or \"square\""; Input, Invalid)),
	}
}

fn cast_join(v: Value) -> Outcome<LineJoin> {
	let s = res!(v.cast::<String>());
	match s.as_str() {
		"miter"	=> Ok(LineJoin::Miter),
		"round"	=> Ok(LineJoin::Round),
		"bevel"	=> Ok(LineJoin::Bevel),
		_ => Err(err!("expected \"miter\", \"round\", or \"bevel\""; Input, Invalid)),
	}
}

fn dash_item(v: Value) -> Outcome<DashItem> {
	match v {
		Value::Length(l)				=> Ok(DashItem::Len(l)),
		Value::Str(s) if *s == "dot"	=> Ok(DashItem::Dot),
		other							=> Err(mismatch("length or \"dot\"", &other)),
	}
}

/// A dash pattern: `none`, a named pattern (TikZ's names), an array of lengths and `"dot"`, or a
/// dictionary of `array` and `phase`. `Some(None)` is an explicit solid line.
fn cast_dash(v: Value) -> Outcome<Option<Dash>> {
	let pt	= |p: f64| DashItem::Len(Length::pt(p));
	let dot	= DashItem::Dot;
	let named = |array: Vec<DashItem>| Ok(Some(Dash { array, phase: Length::zero() }));
	match v {
		Value::None => Ok(None),
		Value::Str(s) => match s.as_str() {
			"solid"					=> named(Vec::new()),
			"dotted"				=> named(vec![dot, pt(2.0)]),
			"densely-dotted"		=> named(vec![dot, pt(1.0)]),
			"loosely-dotted"		=> named(vec![dot, pt(4.0)]),
			"dashed"				=> named(vec![pt(3.0), pt(3.0)]),
			"densely-dashed"		=> named(vec![pt(3.0), pt(2.0)]),
			"loosely-dashed"		=> named(vec![pt(3.0), pt(6.0)]),
			"dash-dotted"			=> named(vec![pt(3.0), pt(2.0), dot, pt(2.0)]),
			"densely-dash-dotted"	=> named(vec![pt(3.0), pt(1.0), dot, pt(1.0)]),
			"loosely-dash-dotted"	=> named(vec![pt(3.0), pt(4.0), dot, pt(4.0)]),
			_ => Err(err!("expected \"solid\", \"dotted\", \"densely-dotted\", \"loosely-dotted\", \
				\"dashed\", \"densely-dashed\", \"loosely-dashed\", \"dash-dotted\", \
				\"densely-dash-dotted\", \"loosely-dash-dotted\", array, dictionary, dash, or none";
				Input, Invalid)),
		},
		Value::Array(a) => {
			let mut array = Vec::new();
			for x in a.iter() {
				array.push(res!(dash_item(x.clone())));
			}
			Ok(Some(Dash { array, phase: Length::zero() }))
		},
		Value::Dict(d) => {
			let mut array = Vec::new();
			match d.get("array") {
				Some(Value::Array(a)) => for x in a.iter() {
					array.push(res!(dash_item(x.clone())));
				},
				Some(other)	=> return Err(mismatch("array", other)),
				None		=> return Err(err!("dictionary does not contain key \"array\""; Input, Missing)),
			}
			let phase = match d.get("phase") {
				Some(v)	=> res!(cast_length(v.clone())),
				None	=> Length::zero(),
			};
			for k in d.keys() {
				if k != "array" && k != "phase" {
					return Err(err!("unexpected key \"{}\", valid keys are \"array\" and \"phase\"", k;
						Input, Invalid));
				}
			}
			Ok(Some(Dash { array, phase }))
		},
		other => Err(mismatch("string, array, dictionary, or none", &other)),
	}
}

/// Typst's stroke cast: a length, a paint, a dictionary of parts, or a stroke. Unset parts stay unset so
/// strokes fold.
pub fn cast_stroke(v: Value) -> Outcome<Stroke> {
	match v {
		Value::Stroke(s)	=> Ok((*s).clone()),
		Value::Length(l)	=> Ok(Stroke { thickness: Some(l), ..Stroke::default() }),
		Value::Color(c)		=> Ok(Stroke { paint: Some(Paint::Color(c)), ..Stroke::default() }),
		Value::Gradient(g)	=> Ok(Stroke { paint: Some(Paint::Gradient(g)), ..Stroke::default() }),
		Value::Tiling(t)	=> Ok(Stroke { paint: Some(Paint::Tiling(t)), ..Stroke::default() }),
		Value::Dict(d) => {
			let mut s = Stroke::default();
			for (k, v) in d.iter() {
				let v = v.clone();
				match k {
					"paint"			=> s.paint = res!(smart(v, cast_paint_some)),
					"thickness"		=> s.thickness = res!(smart(v, cast_length)),
					"cap"			=> s.cap = res!(smart(v, cast_cap)),
					"join"			=> s.join = res!(smart(v, cast_join)),
					"dash"			=> s.dash = res!(smart(v, cast_dash)),
					"miter-limit"	=> s.miter_limit = res!(smart(v, |v| v.cast::<f64>())),
					other => return Err(err!("unexpected key \"{}\", valid keys are \"paint\", \
						\"thickness\", \"cap\", \"join\", \"dash\", and \"miter-limit\"", other;
						Input, Invalid)),
				}
			}
			Ok(s)
		},
		other => Err(mismatch("length, color, gradient, tiling, dictionary, or stroke", &other)),
	}
}

/// `Smart<Option<Stroke>>`: `auto`, `none`, or a stroke.
pub fn cast_smart_stroke(v: Value) -> Outcome<Value> {
	match v {
		Value::Auto | Value::None	=> Ok(v),
		other						=> Ok(Value::Stroke(Arc::new(res!(cast_stroke(other))))),
	}
}

fn cast_opt_stroke(v: Value) -> Outcome<Value> {
	match v {
		Value::None	=> Ok(v),
		other		=> Ok(Value::Stroke(Arc::new(res!(cast_stroke(other))))),
	}
}

fn cast_opt_rel(v: Value) -> Outcome<Value> {
	Ok(Value::Relative(res!(cast_rel(v))))
}

// Sides and corners

const SIDES:	[&str; 4] = ["left", "top", "right", "bottom"];
const CORNERS:	[&str; 4] = ["top-left", "top-right", "bottom-right", "bottom-left"];

// Collapses four set parts to one value when they agree, as Typst's `Sides`/`Corners` do on output.
fn uniform(parts: &[Option<Value>; 4]) -> Option<Value> {
	let first = match &parts[0] {
		Some(v)	=> v,
		None	=> return None,
	};
	for p in parts.iter().skip(1) {
		match p {
			Some(v) if same(first, v)	=> (),
			_							=> return None,
		}
	}
	Some(first.clone())
}

// Structural equality over the values sides and corners hold.
fn same(a: &Value, b: &Value) -> bool {
	match (a, b) {
		(Value::None, Value::None)				=> true,
		(Value::Auto, Value::Auto)				=> true,
		(Value::Relative(x), Value::Relative(y))	=> x == y,
		(Value::Length(x), Value::Length(y))	=> x == y,
		(Value::Stroke(x), Value::Stroke(y))	=> Arc::ptr_eq(x, y) || stroke_eq(x, y),
		_										=> false,
	}
}

fn stroke_eq(a: &Stroke, b: &Stroke) -> bool {
	let paint = match (&a.paint, &b.paint) {
		(None, None)									=> true,
		(Some(Paint::Color(x)), Some(Paint::Color(y)))	=> x == y,
		(Some(Paint::Gradient(x)), Some(Paint::Gradient(y)))	=> x == y,
		(Some(Paint::Tiling(x)), Some(Paint::Tiling(y)))		=> Arc::ptr_eq(x, y),
		_												=> false,
	};
	paint && a.thickness == b.thickness && a.cap == b.cap && a.join == b.join
		&& a.dash == b.dash && a.miter_limit == b.miter_limit
}

fn sides_value(parts: [Option<Value>; 4]) -> Value {
	if let Some(v) = uniform(&parts) {
		return v;
	}
	let mut d = Dict::new();
	for (k, p) in SIDES.iter().zip(parts) {
		if let Some(v) = p {
			d.insert(k, v);
		}
	}
	Value::dict(d)
}

fn corners_value(parts: [Option<Value>; 4]) -> Value {
	if let Some(v) = uniform(&parts) {
		return v;
	}
	let mut d = Dict::new();
	for (k, p) in CORNERS.iter().zip(parts) {
		if let Some(v) = p {
			d.insert(k, v);
		}
	}
	Value::dict(d)
}

/// The four sides (left, top, right, bottom) of a normalised sides value; unset sides are `None`.
pub fn sides_of(v: &Value) -> [Option<Value>; 4] {
	match v {
		Value::Dict(d)	=> SIDES.map(|k| d.get(k).cloned()),
		other			=> [Some(other.clone()), Some(other.clone()), Some(other.clone()), Some(other.clone())],
	}
}

/// The four corners (top-left, top-right, bottom-right, bottom-left) of a normalised corners value.
pub fn corners_of(v: &Value) -> [Option<Value>; 4] {
	match v {
		Value::Dict(d)	=> CORNERS.map(|k| d.get(k).cloned()),
		other			=> [Some(other.clone()), Some(other.clone()), Some(other.clone()), Some(other.clone())],
	}
}

// A sides dictionary: `rest`, then `x`/`y`, then the named sides, each more specific key winning.
fn expand_sides<F>(d: &Dict, cast: &F) -> Outcome<[Option<Value>; 4]>
	where F: Fn(Value) -> Outcome<Value>
{
	for k in d.keys() {
		if !matches!(k, "left" | "top" | "right" | "bottom" | "x" | "y" | "rest") {
			return Err(err!("unexpected key \"{}\", valid keys are \"left\", \"top\", \"right\", \
				\"bottom\", \"x\", \"y\", and \"rest\"", k; Input, Invalid));
		}
	}
	let get = |k: &str| -> Outcome<Option<Value>> {
		match d.get(k) {
			Some(v)	=> Ok(Some(res!(cast(v.clone())))),
			None	=> Ok(None),
		}
	};
	let rest	= res!(get("rest"));
	let x		= res!(get("x")).or_else(|| rest.clone());
	let y		= res!(get("y")).or_else(|| rest.clone());
	Ok([
		res!(get("left")).or_else(|| x.clone()),
		res!(get("top")).or_else(|| y.clone()),
		res!(get("right")).or(x),
		res!(get("bottom")).or(y),
	])
}

fn expand_corners<F>(d: &Dict, cast: &F) -> Outcome<[Option<Value>; 4]>
	where F: Fn(Value) -> Outcome<Value>
{
	for k in d.keys() {
		if !matches!(k, "top-left" | "top-right" | "bottom-right" | "bottom-left"
			| "left" | "top" | "right" | "bottom" | "rest")
		{
			return Err(err!("unexpected key \"{}\", valid keys are \"top-left\", \"top-right\", \
				\"bottom-right\", \"bottom-left\", \"left\", \"top\", \"right\", \"bottom\", and \"rest\"", k;
				Input, Invalid));
		}
	}
	let get = |k: &str| -> Outcome<Option<Value>> {
		match d.get(k) {
			Some(v)	=> Ok(Some(res!(cast(v.clone())))),
			None	=> Ok(None),
		}
	};
	let rest	= res!(get("rest"));
	let left	= res!(get("left")).or_else(|| rest.clone());
	let top		= res!(get("top")).or_else(|| rest.clone());
	let right	= res!(get("right")).or_else(|| rest.clone());
	let bottom	= res!(get("bottom")).or_else(|| rest.clone());
	Ok([
		res!(get("top-left")).or_else(|| top.clone()).or_else(|| left.clone()),
		res!(get("top-right")).or_else(|| top.clone()).or_else(|| right.clone()),
		res!(get("bottom-right")).or_else(|| bottom.clone()).or_else(|| right.clone()),
		res!(get("bottom-left")).or_else(|| bottom.clone()).or_else(|| left.clone()),
	])
}

const SIDE_KEYS:	[&str; 7] = ["left", "top", "right", "bottom", "x", "y", "rest"];
const CORNER_KEYS:	[&str; 9] = ["top-left", "top-right", "bottom-right", "bottom-left",
	"left", "top", "right", "bottom", "rest"];

// As Typst casts `Sides`: a dictionary naming any side key is sides (other keys then an error), an empty
// one sets no side, and anything else, a stroke dictionary included, is one value for all four.
// A sides value. A plain value that fails the part cast is reported against everything the field
// accepts, `expected`, as Typst reports a failed cast to `Sides<..>`.
fn cast_sides<F>(v: Value, cast: F, expected: &str) -> Outcome<Value>
	where F: Fn(Value) -> Outcome<Value>
{
	match v {
		Value::Dict(d) if d.is_empty()								=> Ok(Value::dict(Dict::new())),
		Value::Dict(d) if d.keys().any(|k| SIDE_KEYS.contains(&k))	=> Ok(sides_value(res!(expand_sides(&d, &cast)))),
		other => match cast(other.clone()) {
			Ok(v)	=> Ok(v),
			Err(_)	=> Err(mismatch(expected, &other)),
		},
	}
}

/// `inset`/`outset`: a relative length or a sides dictionary of them.
pub fn cast_sides_rel(v: Value) -> Outcome<Value> { cast_sides(v, cast_opt_rel, "relative length or dictionary") }

/// A rect's `stroke`: `auto`, or a stroke (or `none`) or a sides dictionary of them.
pub fn cast_stroke_sides(v: Value) -> Outcome<Value> {
	match v {
		Value::Auto	=> Ok(v),
		other		=> cast_sides(other, cast_opt_stroke,
			"length, color, gradient, tiling, dictionary, stroke, none, or auto"),
	}
}

/// `radius`: a relative length or a corners dictionary of them.
pub fn cast_corners_rel(v: Value) -> Outcome<Value> {
	match v {
		Value::Dict(d) if d.is_empty()									=> Ok(Value::dict(Dict::new())),
		Value::Dict(d) if d.keys().any(|k| CORNER_KEYS.contains(&k))	=> Ok(corners_value(res!(expand_corners(&d, &cast_opt_rel)))),
		other => match cast_opt_rel(other.clone()) {
			Ok(v)	=> Ok(v),
			Err(_)	=> Err(mismatch("relative length or dictionary", &other)),
		},
	}
}

fn cast_relative_to(v: Value) -> Outcome<RelativeTo> {
	let s = res!(v.cast::<String>());
	match s.as_str() {
		"self"		=> Ok(RelativeTo::SelfBox),
		"parent"	=> Ok(RelativeTo::Parent),
		_			=> Err(err!("expected \"self\", \"parent\", or auto"; Input, Invalid)),
	}
}

fn length_pair(v: Value, what: &str) -> Outcome<(Length, Length)> {
	match v {
		Value::Array(a) if a.len() == 2 => Ok((res!(cast_length(a[0].clone())), res!(cast_length(a[1].clone())))),
		_ => Err(err!("{} must be an array of two lengths", what; Input, Invalid)),
	}
}

/// An alignment's axes with Typst's transform default filling the absent one: centre and horizon.
pub fn origin_of(v: Option<Value>) -> Outcome<(HAlign, VAlign)> {
	let a = match v {
		Some(Value::Alignment(a))	=> a,
		Some(Value::None) | None	=> Alignment::default(),
		Some(other)					=> return Err(mismatch("alignment", &other)),
	};
	Ok((a.x.unwrap_or(HAlign::Center), a.y.unwrap_or(VAlign::Horizon)))
}
