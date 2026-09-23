// U2 owns this file: Typst's operators over `Value`, with Typst's own error wording ("cannot add integer
// and string"), which names types by their long names rather than by `repr(type(x))`.

use crate::eval::args::Args;
use crate::eval::content::{
	Content,
	ElemKind,
	Sequence,
};
use crate::eval::func::Func;
use crate::eval::lib::decimal::Decimal;
use crate::eval::lib::foundations::repr;
use crate::eval::value::{
	Alignment,
	Angle,
	Color,
	Datetime,
	Dict,
	Duration,
	Fraction,
	Length,
	Paint,
	Ratio,
	Relative,
	Stroke,
	Symbol,
	Value,
};

use oxedyne_fe2o3_core::prelude::*;

use std::cmp::Ordering;
use std::sync::Arc;

/// The name a Typst diagnostic gives a type: `integer` where `repr(type(1))` is `int`.
fn fail(msg: String) -> Error<ErrTag> {
	err!("{}", msg; Input, Invalid)
}

fn mismatch(verb: &str, a: &Value, joiner: &str, b: &Value) -> Error<ErrTag> {
	fail(fmt!("cannot {} {} {} {}", verb, a.ty().long_name(), joiner, b.ty().long_name()))
}

fn too_large() -> Error<ErrTag> { fail("value is too large".to_string()) }

// Unary

pub fn pos(v: Value) -> Outcome<Value> {
	match v {
		Value::Int(_) | Value::Float(_) | Value::Decimal(_) | Value::Length(_) | Value::Angle(_)
			| Value::Ratio(_) | Value::Relative(_) | Value::Fraction(_) => Ok(v),
		other => Err(fail(fmt!("cannot apply unary '+' to {}", other.ty().long_name()))),
	}
}

pub fn neg(v: Value) -> Outcome<Value> {
	Ok(match v {
		Value::Int(i)		=> Value::Int(res!(i.checked_neg().ok_or_else(too_large))),
		Value::Float(f)		=> Value::Float(-f),
		Value::Decimal(d)	=> Value::Decimal(d.neg()),
		Value::Length(l)	=> Value::Length(len_neg(l)),
		Value::Angle(a)		=> Value::Angle(Angle(-a.0)),
		Value::Ratio(r)		=> Value::Ratio(Ratio(-r.0)),
		Value::Relative(r)	=> Value::Relative(Relative { rel: Ratio(-r.rel.0), abs: len_neg(r.abs) }),
		Value::Fraction(f)	=> Value::Fraction(Fraction(-f.0)),
		Value::Duration(d)	=> Value::Duration(Duration { secs: -d.secs }),
		other => return Err(fail(fmt!("cannot apply '-' to {}", other.ty().long_name()))),
	})
}

pub fn not(v: Value) -> Outcome<Value> {
	match v {
		Value::Bool(b)	=> Ok(Value::Bool(!b)),
		other			=> Err(fail(fmt!("cannot apply 'not' to {}", other.ty().long_name()))),
	}
}

/// `a and b` once both sides are evaluated; the evaluator short-circuits a `false` left side itself.
pub fn and(a: Value, b: Value) -> Outcome<Value> {
	match (&a, &b) {
		(Value::Bool(x), Value::Bool(y))	=> Ok(Value::Bool(*x && *y)),
		_									=> Err(mismatch("apply 'and' to", &a, "and", &b)),
	}
}

/// `a or b` once both sides are evaluated; the evaluator short-circuits a `true` left side itself.
pub fn or(a: Value, b: Value) -> Outcome<Value> {
	match (&a, &b) {
		(Value::Bool(x), Value::Bool(y))	=> Ok(Value::Bool(*x || *y)),
		_									=> Err(mismatch("apply 'or' to", &a, "and", &b)),
	}
}

// Unit arithmetic

// Typst's `Scalar`: a NaN from `0 * inf` or `inf - inf` becomes zero, so `1pt * float.inf` has no em part.
fn scalar(x: f64) -> f64 { if x.is_nan() { 0.0 } else { x } }

fn len_add(a: Length, b: Length) -> Length { Length { abs: scalar(a.abs + b.abs), em: scalar(a.em + b.em) } }
fn len_neg(a: Length) -> Length { Length { abs: -a.abs, em: -a.em } }
fn len_scale(a: Length, k: f64) -> Length { Length { abs: scalar(a.abs * k), em: scalar(a.em * k) } }
fn len_is_zero(a: Length) -> bool { a.abs == 0.0 && a.em == 0.0 }

fn rel(rel: Ratio, abs: Length) -> Value { Value::Relative(Relative { rel, abs }) }

fn rel_add(a: Relative, b: Relative) -> Relative {
	Relative { rel: Ratio(a.rel.0 + b.rel.0), abs: len_add(a.abs, b.abs) }
}

fn rel_neg(a: Relative) -> Relative { Relative { rel: Ratio(-a.rel.0), abs: len_neg(a.abs) } }

fn rel_scale(a: Relative, k: f64) -> Relative { Relative { rel: Ratio(scalar(a.rel.0 * k)), abs: len_scale(a.abs, k) } }

fn as_rel(v: &Value) -> Option<Relative> {
	match v {
		Value::Length(l)	=> Some(Relative { rel: Ratio(0.0), abs: *l }),
		Value::Ratio(r)		=> Some(Relative { rel: *r, abs: Length::zero() }),
		Value::Relative(r)	=> Some(*r),
		_					=> None,
	}
}

fn num(v: &Value) -> Option<f64> {
	match v {
		Value::Int(i)	=> Some(*i as f64),
		Value::Float(f)	=> Some(*f),
		_				=> None,
	}
}

// Binary

pub fn add(a: Value, b: Value) -> Outcome<Value> {
	Ok(match (a, b) {
		(a, Value::None)						=> a,
		(Value::None, b)						=> b,
		(Value::Int(x), Value::Int(y))			=> Value::Int(res!(x.checked_add(y).ok_or_else(too_large))),
		(Value::Int(x), Value::Float(y))		=> Value::Float(x as f64 + y),
		(Value::Float(x), Value::Int(y))		=> Value::Float(x + y as f64),
		(Value::Float(x), Value::Float(y))		=> Value::Float(x + y),
		(a @ (Value::Decimal(_) | Value::Int(_)), b @ Value::Decimal(_))
		| (a @ Value::Decimal(_), b @ Value::Int(_)) => res!(dec_op(&a, &b, Decimal::checked_add)),
		(Value::Angle(x), Value::Angle(y))		=> Value::Angle(Angle(x.0 + y.0)),
		(Value::Length(x), Value::Length(y))	=> Value::Length(len_add(x, y)),
		(Value::Ratio(x), Value::Ratio(y))		=> Value::Ratio(Ratio(x.0 + y.0)),
		(Value::Length(x), Value::Ratio(y))		=> rel(y, x),
		(Value::Ratio(x), Value::Length(y))		=> rel(x, y),
		(a @ (Value::Length(_) | Value::Ratio(_) | Value::Relative(_)),
			b @ (Value::Length(_) | Value::Ratio(_) | Value::Relative(_))) => {
			match (as_rel(&a), as_rel(&b)) {
				(Some(x), Some(y))	=> Value::Relative(rel_add(x, y)),
				_					=> return Err(mismatch("add", &a, "and", &b)),
			}
		}
		(Value::Fraction(x), Value::Fraction(y))	=> Value::Fraction(Fraction(x.0 + y.0)),
		(Value::Str(x), Value::Str(y))			=> Value::str(fmt!("{}{}", x, y)),
		(Value::Str(x), Value::Symbol(y))		=> Value::str(fmt!("{}{}", x, symbol_text(&y))),
		(Value::Symbol(x), Value::Str(y))		=> Value::str(fmt!("{}{}", symbol_text(&x), y)),
		(Value::Symbol(x), Value::Symbol(y))	=> Value::str(fmt!("{}{}", symbol_text(&x), symbol_text(&y))),
		(Value::Bytes(x), Value::Bytes(y))		=> {
			let mut v = (*x).clone();
			v.extend_from_slice(&y);
			Value::Bytes(Arc::new(v))
		}
		(Value::Content(x), Value::Content(y))	=> Value::Content(content_add(x, y)),
		(Value::Content(x), Value::Str(y))		=> Value::Content(content_add(x, Content::text(&y))),
		(Value::Str(x), Value::Content(y))		=> Value::Content(content_add(Content::text(&x), y)),
		(Value::Content(x), Value::Symbol(y))	=> Value::Content(content_add(x, Content::text(&symbol_text(&y)))),
		(Value::Symbol(x), Value::Content(y))	=> Value::Content(content_add(Content::text(&symbol_text(&x)), y)),
		(Value::Array(x), Value::Array(y))		=> {
			let mut v = (*x).clone();
			v.extend(y.iter().cloned());
			Value::array(v)
		}
		(Value::Dict(x), Value::Dict(y))		=> Value::dict(dict_add(&x, &y)),
		(Value::Args(x), Value::Args(y))		=> {
			let mut v = (*x).clone();
			v.items.extend(y.items.iter().cloned());
			Value::Args(Arc::new(v))
		}
		(Value::Color(c), Value::Length(l)) | (Value::Length(l), Value::Color(c))
			=> stroke(Paint::Color(c), l),
		(Value::Gradient(g), Value::Length(l)) | (Value::Length(l), Value::Gradient(g))
			=> stroke(Paint::Gradient(g), l),
		(Value::Tiling(t), Value::Length(l)) | (Value::Length(l), Value::Tiling(t))
			=> stroke(Paint::Tiling(t), l),
		(Value::Alignment(x), Value::Alignment(y))	=> Value::Alignment(res!(align_add(x, y))),
		(Value::Duration(x), Value::Duration(y))	=> Value::Duration(Duration { secs: x.secs + y.secs }),
		(Value::Datetime(x), Value::Duration(y))	=> Value::Datetime(res!(datetime_shift(x, y.secs))),
		(Value::Duration(x), Value::Datetime(y))	=> Value::Datetime(res!(datetime_shift(y, x.secs))),
		(a, b)									=> return Err(mismatch("add", &a, "and", &b)),
	})
}

pub fn sub(a: Value, b: Value) -> Outcome<Value> {
	Ok(match (a, b) {
		(Value::Int(x), Value::Int(y))			=> Value::Int(res!(x.checked_sub(y).ok_or_else(too_large))),
		(Value::Int(x), Value::Float(y))		=> Value::Float(x as f64 - y),
		(Value::Float(x), Value::Int(y))		=> Value::Float(x - y as f64),
		(Value::Float(x), Value::Float(y))		=> Value::Float(x - y),
		(a @ (Value::Decimal(_) | Value::Int(_)), b @ Value::Decimal(_))
		| (a @ Value::Decimal(_), b @ Value::Int(_)) => res!(dec_op(&a, &b, Decimal::checked_sub)),
		(Value::Angle(x), Value::Angle(y))		=> Value::Angle(Angle(x.0 - y.0)),
		(Value::Length(x), Value::Length(y))	=> Value::Length(len_add(x, len_neg(y))),
		(Value::Ratio(x), Value::Ratio(y))		=> Value::Ratio(Ratio(x.0 - y.0)),
		(a @ (Value::Length(_) | Value::Ratio(_) | Value::Relative(_)),
			b @ (Value::Length(_) | Value::Ratio(_) | Value::Relative(_))) => {
			match (as_rel(&a), as_rel(&b)) {
				(Some(x), Some(y))	=> Value::Relative(rel_add(x, rel_neg(y))),
				_					=> return Err(mismatch("subtract", &b, "from", &a)),
			}
		}
		(Value::Fraction(x), Value::Fraction(y))	=> Value::Fraction(Fraction(x.0 - y.0)),
		(Value::Duration(x), Value::Duration(y))	=> Value::Duration(Duration { secs: x.secs - y.secs }),
		(Value::Datetime(x), Value::Duration(y))	=> Value::Datetime(res!(datetime_shift(x, -y.secs))),
		(Value::Datetime(x), Value::Datetime(y))	=> Value::Duration(res!(datetime_diff(x, y))),
		(a, b)									=> return Err(mismatch("subtract", &b, "from", &a)),
	})
}

pub fn mul(a: Value, b: Value) -> Outcome<Value> {
	Ok(match (a, b) {
		(Value::Int(x), Value::Int(y))			=> Value::Int(res!(x.checked_mul(y).ok_or_else(too_large))),
		(Value::Int(x), Value::Float(y))		=> Value::Float(x as f64 * y),
		(Value::Float(x), Value::Int(y))		=> Value::Float(x * y as f64),
		(Value::Float(x), Value::Float(y))		=> Value::Float(x * y),
		(a @ (Value::Decimal(_) | Value::Int(_)), b @ Value::Decimal(_))
		| (a @ Value::Decimal(_), b @ Value::Int(_)) => res!(dec_op(&a, &b, Decimal::checked_mul)),
		(Value::Length(l), k @ (Value::Int(_) | Value::Float(_)))
		| (k @ (Value::Int(_) | Value::Float(_)), Value::Length(l)) => Value::Length(len_scale(l, num(&k).unwrap_or(0.0))),
		(Value::Length(l), Value::Ratio(r)) | (Value::Ratio(r), Value::Length(l))
			=> Value::Length(len_scale(l, r.0)),
		(Value::Angle(x), k @ (Value::Int(_) | Value::Float(_)))
		| (k @ (Value::Int(_) | Value::Float(_)), Value::Angle(x)) => Value::Angle(Angle(x.0 * num(&k).unwrap_or(0.0))),
		(Value::Angle(x), Value::Ratio(r)) | (Value::Ratio(r), Value::Angle(x))
			=> Value::Angle(Angle(x.0 * r.0)),
		(Value::Ratio(x), Value::Ratio(y))		=> Value::Ratio(Ratio(x.0 * y.0)),
		(Value::Ratio(x), k @ (Value::Int(_) | Value::Float(_)))
		| (k @ (Value::Int(_) | Value::Float(_)), Value::Ratio(x)) => Value::Ratio(Ratio(x.0 * num(&k).unwrap_or(0.0))),
		(Value::Relative(x), k @ (Value::Int(_) | Value::Float(_)))
		| (k @ (Value::Int(_) | Value::Float(_)), Value::Relative(x)) => Value::Relative(rel_scale(x, num(&k).unwrap_or(0.0))),
		(Value::Relative(x), Value::Ratio(r)) | (Value::Ratio(r), Value::Relative(x))
			=> Value::Relative(rel_scale(x, r.0)),
		(Value::Fraction(x), k @ (Value::Int(_) | Value::Float(_)))
		| (k @ (Value::Int(_) | Value::Float(_)), Value::Fraction(x)) => Value::Fraction(Fraction(x.0 * num(&k).unwrap_or(0.0))),
		(Value::Fraction(x), Value::Ratio(r)) | (Value::Ratio(r), Value::Fraction(x))
			=> Value::Fraction(Fraction(x.0 * r.0)),
		(Value::Str(s), Value::Int(n)) | (Value::Int(n), Value::Str(s))
			=> Value::str(s.repeat(res!(count(n)))),
		(Value::Array(a), Value::Int(n)) | (Value::Int(n), Value::Array(a)) => {
			let n = res!(count(n));
			let mut v = Vec::with_capacity(a.len() * n);
			for _ in 0..n {
				v.extend(a.iter().cloned());
			}
			Value::array(v)
		}
		(Value::Content(c), Value::Int(n)) | (Value::Int(n), Value::Content(c)) => {
			let n = res!(count(n));
			Value::Content(Content::sequence(vec![c; n]))
		}
		(Value::Duration(d), k @ (Value::Int(_) | Value::Float(_)))
		| (k @ (Value::Int(_) | Value::Float(_)), Value::Duration(d))
			=> Value::Duration(Duration { secs: d.secs * num(&k).unwrap_or(0.0) }),
		(a, b)									=> return Err(mismatch("multiply", &a, "with", &b)),
	})
}

fn count(n: i64) -> Outcome<usize> {
	if n < 0 {
		return Err(fail("number must be at least zero".to_string()));
	}
	Ok(n as usize)
}

// A decimal operation with an integer operand widened, "value is too large" when it overflows.
fn dec_op(a: &Value, b: &Value, op: fn(Decimal, Decimal) -> Option<Decimal>) -> Outcome<Value> {
	let d = |v: &Value| match v {
		Value::Decimal(d)	=> *d,
		Value::Int(i)		=> Decimal::from(*i),
		_					=> Decimal::ZERO,
	};
	match op(d(a), d(b)) {
		Some(r)	=> Ok(Value::Decimal(r)),
		None	=> Err(too_large()),
	}
}

fn is_zero(v: &Value) -> bool {
	match v {
		Value::Int(i)		=> *i == 0,
		Value::Float(f)		=> *f == 0.0,
		Value::Decimal(d)	=> d.is_zero(),
		Value::Length(l)	=> len_is_zero(*l),
		Value::Angle(a)		=> a.0 == 0.0,
		Value::Ratio(r)		=> r.0 == 0.0,
		Value::Relative(r)	=> r.rel.0 == 0.0 && len_is_zero(r.abs),
		Value::Fraction(f)	=> f.0 == 0.0,
		Value::Duration(d)	=> d.secs == 0.0,
		_					=> false,
	}
}

// Two lengths divide only when both are absolute or both em.
fn len_div(a: Length, b: Length) -> Outcome<f64> {
	if a.em == 0.0 && b.em == 0.0 {
		return Ok(a.abs / b.abs);
	}
	if a.abs == 0.0 && b.abs == 0.0 {
		return Ok(a.em / b.em);
	}
	Err(fail("cannot divide these two lengths".to_string()))
}

pub fn div(a: Value, b: Value) -> Outcome<Value> {
	if is_zero(&b) {
		return Err(fail("cannot divide by zero".to_string()));
	}
	Ok(match (a, b) {
		(a @ (Value::Int(_) | Value::Float(_)), b @ (Value::Int(_) | Value::Float(_)))
			=> Value::Float(num(&a).unwrap_or(0.0) / num(&b).unwrap_or(1.0)),
		(a @ (Value::Decimal(_) | Value::Int(_)), b @ Value::Decimal(_))
		| (a @ Value::Decimal(_), b @ Value::Int(_)) => res!(dec_op(&a, &b, Decimal::checked_div)),
		(Value::Length(l), k @ (Value::Int(_) | Value::Float(_)))
			=> Value::Length(len_scale(l, 1.0 / num(&k).unwrap_or(1.0))),
		(Value::Length(x), Value::Length(y))	=> Value::Float(res!(len_div(x, y))),
		(Value::Length(x), Value::Relative(y)) if y.rel.0 == 0.0	=> Value::Float(res!(len_div(x, y.abs))),
		(Value::Angle(x), k @ (Value::Int(_) | Value::Float(_)))
			=> Value::Angle(Angle(x.0 / num(&k).unwrap_or(1.0))),
		(Value::Angle(x), Value::Angle(y))		=> Value::Float(x.0 / y.0),
		(Value::Ratio(x), k @ (Value::Int(_) | Value::Float(_)))
			=> Value::Ratio(Ratio(x.0 / num(&k).unwrap_or(1.0))),
		(Value::Ratio(x), Value::Ratio(y))		=> Value::Float(x.0 / y.0),
		(Value::Ratio(x), Value::Relative(y)) if len_is_zero(y.abs)	=> Value::Float(x.0 / y.rel.0),
		(Value::Relative(x), k @ (Value::Int(_) | Value::Float(_)))
			=> Value::Relative(rel_scale(x, 1.0 / num(&k).unwrap_or(1.0))),
		(Value::Relative(x), Value::Length(y)) if x.rel.0 == 0.0	=> Value::Float(res!(len_div(x.abs, y))),
		(Value::Relative(x), Value::Ratio(y)) if len_is_zero(x.abs)	=> Value::Float(x.rel.0 / y.0),
		(Value::Relative(x), Value::Relative(y)) => {
			if len_is_zero(x.abs) && len_is_zero(y.abs) {
				Value::Float(x.rel.0 / y.rel.0)
			} else if x.rel.0 == 0.0 && y.rel.0 == 0.0 {
				Value::Float(res!(len_div(x.abs, y.abs)))
			} else {
				return Err(fail("cannot divide these two relative lengths".to_string()));
			}
		}
		(Value::Fraction(x), k @ (Value::Int(_) | Value::Float(_)))
			=> Value::Fraction(Fraction(x.0 / num(&k).unwrap_or(1.0))),
		(Value::Fraction(x), Value::Fraction(y))	=> Value::Float(x.0 / y.0),
		(Value::Duration(x), k @ (Value::Int(_) | Value::Float(_)))
			=> Value::Duration(Duration { secs: x.secs / num(&k).unwrap_or(1.0) }),
		(Value::Duration(x), Value::Duration(y))	=> Value::Float(x.secs / y.secs),
		(a, b)									=> return Err(mismatch("divide", &a, "by", &b)),
	})
}

/// Joins two values as markup juxtaposition and `+` on content do: strings concatenate, content
/// sequences, `none` is the identity.
pub fn join(a: Value, b: Value) -> Outcome<Value> {
	Ok(match (a, b) {
		(a, Value::None)						=> a,
		(Value::None, b)						=> b,
		(Value::Symbol(x), Value::Symbol(y))	=> Value::str(fmt!("{}{}", symbol_text(&x), symbol_text(&y))),
		(Value::Str(x), Value::Str(y))			=> Value::str(fmt!("{}{}", x, y)),
		(Value::Str(x), Value::Symbol(y))		=> Value::str(fmt!("{}{}", x, symbol_text(&y))),
		(Value::Symbol(x), Value::Str(y))		=> Value::str(fmt!("{}{}", symbol_text(&x), y)),
		(Value::Bytes(x), Value::Bytes(y))		=> {
			let mut v = (*x).clone();
			v.extend_from_slice(&y);
			Value::Bytes(Arc::new(v))
		}
		(Value::Content(x), Value::Content(y))	=> Value::Content(content_add(x, y)),
		(Value::Content(x), Value::Symbol(y))	=> Value::Content(content_add(x, Content::text(&symbol_text(&y)))),
		(Value::Content(x), Value::Str(y))		=> Value::Content(content_add(x, Content::text(&y))),
		(Value::Str(x), Value::Content(y))		=> Value::Content(content_add(Content::text(&x), y)),
		(Value::Symbol(x), Value::Content(y))	=> Value::Content(content_add(Content::text(&symbol_text(&x)), y)),
		(Value::Array(x), Value::Array(y))		=> {
			let mut v = (*x).clone();
			v.extend(y.iter().cloned());
			Value::array(v)
		}
		(Value::Dict(x), Value::Dict(y))		=> Value::dict(dict_add(&x, &y)),
		(Value::Args(x), Value::Args(y))		=> {
			let mut v = (*x).clone();
			v.items.extend(y.items.iter().cloned());
			Value::Args(Arc::new(v))
		}
		(Value::Type(t), Value::Str(s))			=> Value::str(fmt!("{}{}", t.name(), s)),
		(Value::Str(s), Value::Type(t))			=> Value::str(fmt!("{}{}", s, t.name())),
		(a, b)									=> return Err(mismatch("join", &a, "with", &b)),
	})
}

/// Typst's `+` on content: a sequence absorbs its neighbour rather than nesting.
pub fn content_add(a: Content, b: Content) -> Content {
	match (a, b) {
		(Content::Sequence(mut x), Content::Sequence(y)) => {
			Arc::make_mut(&mut x).children.extend(y.children.iter().cloned());
			Content::Sequence(x)
		}
		(Content::Sequence(mut x), b) => {
			Arc::make_mut(&mut x).children.push(b);
			Content::Sequence(x)
		}
		(a, Content::Sequence(mut y)) => {
			Arc::make_mut(&mut y).children.insert(0, a);
			Content::Sequence(y)
		}
		(a, b) => Content::Sequence(Arc::new(Sequence::new(vec![a, b]))),
	}
}

fn dict_add(x: &Dict, y: &Dict) -> Dict {
	let mut d = x.clone();
	for (k, v) in y.iter() {
		d.insert(k, v.clone());
	}
	d
}

fn stroke(paint: Paint, thickness: Length) -> Value {
	Value::Stroke(Arc::new(Stroke { paint: Some(paint), thickness: Some(thickness), ..Stroke::default() }))
}

fn align_add(a: Alignment, b: Alignment) -> Outcome<Alignment> {
	if a.x.is_some() && b.x.is_some() {
		return Err(fail("cannot add two horizontal alignments".to_string()));
	}
	if a.y.is_some() && b.y.is_some() {
		return Err(fail("cannot add two vertical alignments".to_string()));
	}
	Ok(Alignment { x: a.x.or(b.x), y: a.y.or(b.y) })
}

// Datetime arithmetic, on the proleptic Gregorian calendar.

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
	let y = if m <= 2 { y - 1 } else { y };
	let era = if y >= 0 { y } else { y - 399 } / 400;
	let yoe = y - era * 400;
	let mp = (m + 9) % 12;
	let doy = (153 * mp + 2) / 5 + d - 1;
	let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
	era * 146_097 + doe - 719_468
}

fn civil_from_days(z: i64) -> (i64, i64, i64) {
	let z = z + 719_468;
	let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
	let doe = z - era * 146_097;
	let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
	let y = yoe + era * 400;
	let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
	let mp = (5 * doy + 2) / 153;
	let d = doy - (153 * mp + 2) / 5 + 1;
	let m = if mp < 10 { mp + 3 } else { mp - 9 };
	(if m <= 2 { y + 1 } else { y }, m, d)
}

fn has_date(d: &Datetime) -> bool { d.year.is_some() && d.month.is_some() && d.day.is_some() }
fn has_time(d: &Datetime) -> bool { d.hour.is_some() && d.minute.is_some() && d.second.is_some() }

fn datetime_secs(d: &Datetime) -> i64 {
	let mut s = 0;
	if has_date(d) {
		s += days_from_civil(d.year.unwrap_or(0) as i64, d.month.unwrap_or(1) as i64, d.day.unwrap_or(1) as i64)
			* 86_400;
	}
	if has_time(d) {
		s += d.hour.unwrap_or(0) as i64 * 3600 + d.minute.unwrap_or(0) as i64 * 60 + d.second.unwrap_or(0) as i64;
	}
	s
}

fn datetime_shift(d: Datetime, secs: f64) -> Outcome<Datetime> {
	let total = datetime_secs(&d) + secs.trunc() as i64;
	let mut out = d;
	if has_time(&d) {
		let tod = total.rem_euclid(86_400);
		out.hour = Some((tod / 3600) as u8);
		out.minute = Some((tod / 60 % 60) as u8);
		out.second = Some((tod % 60) as u8);
	}
	if has_date(&d) {
		let (y, m, day) = civil_from_days(total.div_euclid(86_400));
		if y < i32::MIN as i64 || y > i32::MAX as i64 {
			return Err(fail("the resulting datetime is out of range".to_string()));
		}
		out.year = Some(y as i32);
		out.month = Some(m as u8);
		out.day = Some(day as u8);
	}
	Ok(out)
}

fn datetime_diff(a: Datetime, b: Datetime) -> Outcome<Duration> {
	if has_date(&a) != has_date(&b) || has_time(&a) != has_time(&b) {
		return Err(fail("cannot subtract datetimes of different kinds".to_string()));
	}
	Ok(Duration { secs: (datetime_secs(&a) - datetime_secs(&b)) as f64 })
}

// Symbols

/// The text of a symbol under its modifiers; the resolution is `lib::sym`'s, the one place it lives.
pub fn symbol_text(s: &Symbol) -> String { crate::eval::lib::sym::text(s).to_string() }

/// `sym.arrow.r`: the symbol with one more modifier, `None` when no variant carries them all.
pub fn symbol_modified(s: &Symbol, modifier: &str) -> Option<Symbol> { crate::eval::lib::sym::modify(s, modifier) }

// Comparison

fn cmp_f64(a: f64, b: f64, ra: &Value, rb: &Value) -> Outcome<Ordering> {
	a.partial_cmp(&b).ok_or_else(|| fail(fmt!("cannot compare {} with {}", repr(ra), repr(rb))))
}

fn cmp_len(a: Length, b: Length, ra: &Value, rb: &Value) -> Outcome<Ordering> {
	if a.em == 0.0 && b.em == 0.0 {
		return cmp_f64(a.abs, b.abs, ra, rb);
	}
	if a.abs == 0.0 && b.abs == 0.0 {
		return cmp_f64(a.em, b.em, ra, rb);
	}
	Err(fail(fmt!("cannot compare {} with {}", repr(ra), repr(rb))))
}

/// Typst's `<`, `<=`, `>`, `>=`; comparing incomparable types is an error.
pub fn compare(a: &Value, b: &Value) -> Outcome<Ordering> {
	match (a, b) {
		(Value::Bool(x), Value::Bool(y))		=> Ok(x.cmp(y)),
		(Value::Int(x), Value::Int(y))			=> Ok(x.cmp(y)),
		(Value::Float(x), Value::Float(y))		=> cmp_f64(*x, *y, a, b),
		(Value::Int(x), Value::Float(y))		=> cmp_f64(*x as f64, *y, a, b),
		(Value::Float(x), Value::Int(y))		=> cmp_f64(*x, *y as f64, a, b),
		(Value::Decimal(x), Value::Decimal(y))	=> Ok(x.cmp(y)),
		(Value::Int(x), Value::Decimal(y))		=> Ok(Decimal::from(*x).cmp(y)),
		(Value::Decimal(x), Value::Int(y))		=> Ok(x.cmp(&Decimal::from(*y))),
		(Value::Length(x), Value::Length(y))	=> cmp_len(*x, *y, a, b),
		(Value::Angle(x), Value::Angle(y))		=> cmp_f64(x.0, y.0, a, b),
		(Value::Ratio(x), Value::Ratio(y))		=> cmp_f64(x.0, y.0, a, b),
		(Value::Relative(x), Value::Relative(y)) => {
			if x.rel.0 == 0.0 && y.rel.0 == 0.0 {
				cmp_len(x.abs, y.abs, a, b)
			} else if len_is_zero(x.abs) && len_is_zero(y.abs) {
				cmp_f64(x.rel.0, y.rel.0, a, b)
			} else {
				Err(fail(fmt!("cannot compare {} with {}", repr(a), repr(b))))
			}
		}
		(Value::Length(x), Value::Relative(y)) if y.rel.0 == 0.0	=> cmp_len(*x, y.abs, a, b),
		(Value::Relative(x), Value::Length(y)) if x.rel.0 == 0.0	=> cmp_len(x.abs, *y, a, b),
		(Value::Ratio(x), Value::Relative(y)) if len_is_zero(y.abs)	=> cmp_f64(x.0, y.rel.0, a, b),
		(Value::Relative(x), Value::Ratio(y)) if len_is_zero(x.abs)	=> cmp_f64(x.rel.0, y.0, a, b),
		(Value::Fraction(x), Value::Fraction(y))	=> cmp_f64(x.0, y.0, a, b),
		(Value::Version(x), Value::Version(y))	=> Ok(version_cmp(x, y)),
		(Value::Str(x), Value::Str(y))			=> Ok(x.cmp(y)),
		(Value::Duration(x), Value::Duration(y))	=> cmp_f64(x.secs, y.secs, a, b),
		(Value::Datetime(x), Value::Datetime(y))	=> {
			if has_date(x) != has_date(y) || has_time(x) != has_time(y) {
				return Err(fail(fmt!("cannot compare {} and {}", repr(a), repr(b))));
			}
			Ok(datetime_secs(x).cmp(&datetime_secs(y)))
		}
		(Value::Array(x), Value::Array(y))		=> {
			for (p, q) in x.iter().zip(y.iter()) {
				match res!(compare(p, q)) {
					Ordering::Equal	=> continue,
					o				=> return Ok(o),
				}
			}
			Ok(x.len().cmp(&y.len()))
		}
		_ => Err(mismatch("compare", a, "and", b)),
	}
}

// Missing trailing components count as zero, so `version(1) == version(1, 0)`.
fn version_cmp(x: &[u32], y: &[u32]) -> Ordering {
	let n = x.len().max(y.len());
	for i in 0..n {
		let (p, q) = (x.get(i).copied().unwrap_or(0), y.get(i).copied().unwrap_or(0));
		match p.cmp(&q) {
			Ordering::Equal	=> continue,
			o				=> return o,
		}
	}
	Ordering::Equal
}

/// Typst's `a in b`.
pub fn contains(a: &Value, b: &Value) -> Outcome<bool> {
	match (a, b) {
		(Value::Str(x), Value::Str(y))		=> Ok(y.contains(x.as_str())),
		(Value::Regex(r), Value::Str(y))	=> r.re.is_match(y),
		(Value::Str(x), Value::Dict(d))		=> Ok(d.contains(x)),
		(x, Value::Array(arr))				=> Ok(arr.iter().any(|v| equal(x, v))),
		_ => Err(fail(fmt!("cannot apply 'in' to {} and {}", a.ty().long_name(), b.ty().long_name()))),
	}
}

/// Typst's `==`, which never fails: values of different types are unequal, except int with float, and
/// a length or ratio with the relative length it equals.
pub fn equal(a: &Value, b: &Value) -> bool {
	match (a, b) {
		(Value::None, Value::None)				=> true,
		(Value::Auto, Value::Auto)				=> true,
		(Value::Bool(x), Value::Bool(y))		=> x == y,
		(Value::Int(x), Value::Int(y))			=> x == y,
		(Value::Float(x), Value::Float(y))		=> x == y,
		(Value::Int(x), Value::Float(y))		=> (*x as f64) == *y,
		(Value::Float(x), Value::Int(y))		=> *x == (*y as f64),
		(Value::Decimal(x), Value::Decimal(y))	=> x == y,
		(Value::Int(i), Value::Decimal(d)) | (Value::Decimal(d), Value::Int(i))	=> Decimal::from(*i) == *d,
		(Value::Length(x), Value::Length(y))	=> x == y,
		(Value::Angle(x), Value::Angle(y))		=> x == y,
		(Value::Ratio(x), Value::Ratio(y))		=> x == y,
		(Value::Relative(x), Value::Relative(y))	=> x == y,
		(Value::Length(l), Value::Relative(r)) | (Value::Relative(r), Value::Length(l))
			=> r.rel.0 == 0.0 && r.abs == *l,
		(Value::Ratio(q), Value::Relative(r)) | (Value::Relative(r), Value::Ratio(q))
			=> len_is_zero(r.abs) && r.rel == *q,
		(Value::Fraction(x), Value::Fraction(y))	=> x == y,
		(Value::Color(x), Value::Color(y))		=> color_eq(x, y),
		(Value::Gradient(x), Value::Gradient(y))	=> x == y,
		(Value::Tiling(x), Value::Tiling(y))	=> Arc::ptr_eq(x, y),
		(Value::Stroke(x), Value::Stroke(y))	=> stroke_eq(x, y),
		(Value::Alignment(x), Value::Alignment(y))	=> x == y,
		(Value::Direction(x), Value::Direction(y))	=> x == y,
		(Value::Symbol(x), Value::Symbol(y))	=> symbol_text(x) == symbol_text(y) && x.modifiers == y.modifiers,
		(Value::Str(x), Value::Str(y))			=> x == y,
		(Value::Bytes(x), Value::Bytes(y))		=> x == y,
		(Value::Label(x), Value::Label(y))		=> x == y,
		(Value::Datetime(x), Value::Datetime(y))	=> x == y,
		(Value::Duration(x), Value::Duration(y))	=> x == y,
		(Value::Version(x), Value::Version(y))	=> version_cmp(x, y) == Ordering::Equal,
		(Value::Regex(x), Value::Regex(y))		=> x.pattern == y.pattern,
		(Value::Content(x), Value::Content(y))	=> content_eq(x, y),
		(Value::Array(x), Value::Array(y))		=> x.len() == y.len()
			&& x.iter().zip(y.iter()).all(|(p, q)| equal(p, q)),
		(Value::Dict(x), Value::Dict(y))		=> x.len() == y.len()
			&& x.iter().all(|(k, v)| y.get(k).map(|w| equal(v, w)).unwrap_or(false)),
		(Value::Func(x), Value::Func(y))		=> func_eq(x, y),
		(Value::Args(x), Value::Args(y))		=> args_eq(x, y),
		(Value::Module(x), Value::Module(y))	=> Arc::ptr_eq(x, y) || x.name == y.name,
		(Value::Type(x), Value::Type(y))		=> x == y,
		(Value::Styles(x), Value::Styles(y))	=> Arc::ptr_eq(&x.0, &y.0),
		(Value::Selector(x), Value::Selector(y))	=> Arc::ptr_eq(x, y),
		(Value::Counter(x), Value::Counter(y))	=> Arc::ptr_eq(x, y),
		(Value::State(x), Value::State(y))		=> x.key == y.key && equal(&x.init, &y.init),
		(Value::Location(x), Value::Location(y))	=> x == y,
		_										=> false,
	}
}

fn color_eq(x: &Color, y: &Color) -> bool { x == y }

fn paint_eq(x: &Paint, y: &Paint) -> bool {
	match (x, y) {
		(Paint::Color(p), Paint::Color(q))			=> color_eq(p, q),
		(Paint::Gradient(p), Paint::Gradient(q))	=> p == q,
		(Paint::Tiling(p), Paint::Tiling(q))		=> Arc::ptr_eq(p, q),
		_											=> false,
	}
}

fn stroke_eq(x: &Stroke, y: &Stroke) -> bool {
	let paint = match (&x.paint, &y.paint) {
		(None, None)		=> true,
		(Some(p), Some(q))	=> paint_eq(p, q),
		_					=> false,
	};
	paint && x.thickness == y.thickness && x.cap == y.cap && x.join == y.join && x.dash == y.dash
		&& x.miter_limit == y.miter_limit
}

fn func_eq(x: &Func, y: &Func) -> bool {
	match (x, y) {
		(Func::Native(p), Func::Native(q))		=> p == q,
		(Func::Element(p), Func::Element(q))	=> p == q,
		(Func::Closure(p), Func::Closure(q))	=> Arc::ptr_eq(p, q),
		(Func::With(p), Func::With(q))			=> Arc::ptr_eq(p, q),
		_										=> false,
	}
}

fn args_eq(x: &Args, y: &Args) -> bool {
	x.items.len() == y.items.len()
		&& x.items.iter().zip(y.items.iter()).all(|(p, q)| p.name == q.name && equal(&p.value, &q.value))
}

/// Structural equality of content: same element, same fields in any order, same label; sequences and
/// styled content child by child.
pub fn content_eq(x: &Content, y: &Content) -> bool {
	match (x, y) {
		(Content::Elem(p), Content::Elem(q)) => {
			if Arc::ptr_eq(p, q) {
				return true;
			}
			p.kind == q.kind && p.label == q.label && p.fields.len() == q.fields.len()
				&& p.fields.iter().all(|(id, v)| {
					q.fields.iter().find(|(j, _)| j == id).map(|(_, w)| equal(v, w)).unwrap_or(false)
				})
		}
		(Content::Sequence(p), Content::Sequence(q)) => p.label == q.label
			&& p.children.len() == q.children.len()
			&& p.children.iter().zip(q.children.iter()).all(|(a, b)| content_eq(a, b)),
		(Content::Styled(p), Content::Styled(q)) => Arc::ptr_eq(&p.styles.0, &q.styles.0)
			&& content_eq(&p.child, &q.child),
		_ => false,
	}
}

/// Does realisation leave the element out of label attachment, as Typst's `Unlabellable` does?
pub fn unlabellable(c: &Content) -> bool {
	matches!(c.kind(), Some(ElemKind::Space) | Some(ElemKind::Parbreak))
}
