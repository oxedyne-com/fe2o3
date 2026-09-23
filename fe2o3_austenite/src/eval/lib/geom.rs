// U3 owns this file. Methods on `length`, `angle`, `alignment` and `direction`, and the alignment and
// direction globals (`left`, `top`, `ltr`...). Fields (`1pt.abs`, `(left + top).x`) are
// `foundations::field`'s. `to-absolute` resolves `em` against the contextual text size, the one method
// here that needs context.

use crate::eval::args::Args;
use crate::eval::content::ElemKind;
use crate::eval::lib::foundations::{
	finish,
	mismatch,
	receiver,
	repr_length,
};
use crate::eval::scope::Scope;
use crate::eval::value::{
	Alignment,
	Direction,
	HAlign,
	Length,
	Type,
	VAlign,
	Value,
};
use crate::eval::Engine;
use crate::syntax::Span;

use oxedyne_fe2o3_core::prelude::*;

native_fns! {
	pub enum GeomFn {
		Pt				=> "pt",
		Mm				=> "mm",
		Cm				=> "cm",
		Inches			=> "inches",
		ToAbsolute		=> "to-absolute",
		Deg				=> "deg",
		Rad				=> "rad",
		Axis			=> "axis",
		Inv				=> "inv",
		Start			=> "start",
		End				=> "end",
		Sign			=> "sign",
		X				=> "x",
		Y				=> "y",
	}
}

const DEFAULT_TEXT_SIZE: f64 = 11.0;

pub fn define(scope: &mut Scope) {
	let h = |x| Value::Alignment(Alignment { x: Some(x), y: None });
	let v = |y| Value::Alignment(Alignment { x: None, y: Some(y) });
	scope.define("start", h(HAlign::Start));
	scope.define("left", h(HAlign::Left));
	scope.define("center", h(HAlign::Center));
	scope.define("right", h(HAlign::Right));
	scope.define("end", h(HAlign::End));
	scope.define("top", v(VAlign::Top));
	scope.define("horizon", v(VAlign::Horizon));
	scope.define("bottom", v(VAlign::Bottom));
	scope.define("ltr", Value::Direction(Direction::Ltr));
	scope.define("rtl", Value::Direction(Direction::Rtl));
	scope.define("ttb", Value::Direction(Direction::Ttb));
	scope.define("btt", Value::Direction(Direction::Btt));
}

pub fn method(ty: Type, name: &str) -> Option<GeomFn> {
	let f = match (ty, name) {
		(Type::Length, "pt")			=> GeomFn::Pt,
		(Type::Length, "mm")			=> GeomFn::Mm,
		(Type::Length, "cm")			=> GeomFn::Cm,
		(Type::Length, "inches")		=> GeomFn::Inches,
		(Type::Length, "to-absolute")	=> GeomFn::ToAbsolute,
		(Type::Angle, "deg")			=> GeomFn::Deg,
		(Type::Angle, "rad")			=> GeomFn::Rad,
		(Type::Alignment, "axis")		=> GeomFn::Axis,
		(Type::Alignment, "inv")		=> GeomFn::Inv,
		(Type::Direction, "axis")		=> GeomFn::Axis,
		(Type::Direction, "inv")		=> GeomFn::Inv,
		(Type::Direction, "start")		=> GeomFn::Start,
		(Type::Direction, "end")		=> GeomFn::End,
		(Type::Direction, "sign")		=> GeomFn::Sign,
		_								=> return None,
	};
	Some(f)
}

pub fn inv_h(h: HAlign) -> HAlign {
	match h {
		HAlign::Start	=> HAlign::End,
		HAlign::End		=> HAlign::Start,
		HAlign::Left	=> HAlign::Right,
		HAlign::Right	=> HAlign::Left,
		HAlign::Center	=> HAlign::Center,
	}
}

pub fn inv_v(v: VAlign) -> VAlign {
	match v {
		VAlign::Top		=> VAlign::Bottom,
		VAlign::Bottom	=> VAlign::Top,
		VAlign::Horizon	=> VAlign::Horizon,
	}
}

pub fn inv_dir(d: Direction) -> Direction {
	match d {
		Direction::Ltr	=> Direction::Rtl,
		Direction::Rtl	=> Direction::Ltr,
		Direction::Ttb	=> Direction::Btt,
		Direction::Btt	=> Direction::Ttb,
	}
}

fn is_horizontal(d: Direction) -> bool { matches!(d, Direction::Ltr | Direction::Rtl) }

// The absolute pt of a length, refusing an unresolved em part with Typst's message.
fn pt_of(engine: &mut Engine, span: Span, l: &Length, unit: &str) -> Outcome<f64> {
	if l.em != 0.0 {
		let e = engine.error_hint(span,
			fmt!("cannot convert a length with non-zero em units (`{}`) to {}", repr_length(l), unit),
			"use `length.to-absolute()` to resolve its em component (requires context)");
		if let Some(d) = engine.diags.last_mut() {
			d.hints.push("or use `length.abs.pt()` instead to ignore its em component".to_string());
		}
		return Err(e);
	}
	Ok(l.abs)
}

// The contextual text size in points, for resolving `em`.
fn text_size(engine: &mut Engine, span: Span) -> Outcome<f64> {
	let styles = match &engine.context.styles {
		Some(s)	=> s.clone(),
		None	=> return Err(engine.error_hint(span, "can only be used when context is known",
			"try wrapping this in a `context` expression")),
	};
	let size = match ElemKind::Text.field_id("size") {
		Some(id)	=> styles.get(ElemKind::Text, id),
		None		=> None,
	};
	Ok(match size {
		Some(Value::Length(l))	=> l.resolve(DEFAULT_TEXT_SIZE),
		_						=> DEFAULT_TEXT_SIZE,
	})
}

pub fn call(f: GeomFn, engine: &mut Engine, mut args: Args) -> Outcome<Value> {
	let span = args.span;
	let recv = res!(receiver(&mut args));
	let out = match (f, &recv) {
		(GeomFn::Pt, Value::Length(l))		=> Value::Float(res!(pt_of(engine, span, l, "pt"))),
		(GeomFn::Mm, Value::Length(l))		=> Value::Float(res!(pt_of(engine, span, l, "mm")) / 72.0 * 25.4),
		(GeomFn::Cm, Value::Length(l))		=> Value::Float(res!(pt_of(engine, span, l, "cm")) / 72.0 * 2.54),
		(GeomFn::Inches, Value::Length(l))	=> Value::Float(res!(pt_of(engine, span, l, "inches")) / 72.0),
		(GeomFn::ToAbsolute, Value::Length(l)) => {
			if l.em == 0.0 {
				Value::Length(*l)
			} else {
				let size = res!(text_size(engine, span));
				Value::Length(Length::pt(l.resolve(size)))
			}
		}
		(GeomFn::Deg, Value::Angle(a))		=> Value::Float(a.0.to_degrees()),
		(GeomFn::Rad, Value::Angle(a))		=> Value::Float(a.0),
		(GeomFn::Axis, Value::Alignment(a))	=> match (a.x, a.y) {
			(Some(_), None)	=> Value::str("horizontal"),
			(None, Some(_))	=> Value::str("vertical"),
			_				=> Value::None,
		},
		(GeomFn::Inv, Value::Alignment(a))	=> Value::Alignment(Alignment { x: a.x.map(inv_h), y: a.y.map(inv_v) }),
		(GeomFn::Axis, Value::Direction(d))	=> Value::str(if is_horizontal(*d) { "horizontal" } else { "vertical" }),
		(GeomFn::Inv, Value::Direction(d))	=> Value::Direction(inv_dir(*d)),
		(GeomFn::Start, Value::Direction(d)) | (GeomFn::End, Value::Direction(d)) => {
			let d = if f == GeomFn::End { inv_dir(*d) } else { *d };
			Value::Alignment(match d {
				Direction::Ltr	=> Alignment { x: Some(HAlign::Left), y: None },
				Direction::Rtl	=> Alignment { x: Some(HAlign::Right), y: None },
				Direction::Ttb	=> Alignment { x: None, y: Some(VAlign::Top) },
				Direction::Btt	=> Alignment { x: None, y: Some(VAlign::Bottom) },
			})
		}
		(GeomFn::Sign, Value::Direction(d))	=> Value::Int(match d {
			Direction::Ltr | Direction::Ttb	=> 1,
			_								=> -1,
		}),
		(GeomFn::X, Value::Alignment(a))	=> a.x.map(|x| Value::Alignment(Alignment { x: Some(x), y: None })).unwrap_or(Value::None),
		(GeomFn::Y, Value::Alignment(a))	=> a.y.map(|y| Value::Alignment(Alignment { x: None, y: Some(y) })).unwrap_or(Value::None),
		(_, other) => return Err(mismatch(engine, span, "length, angle, alignment, or direction", other)),
	};
	res!(finish(engine, args));
	Ok(out)
}
