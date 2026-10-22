// U3 owns this file. The `calc` module: `define` binds `calc` to a module whose scope holds these, plus
// `pi`, `tau`, `e` and `inf` (Typst has no `calc.nan`; `float.nan` is the type's). `erf` is FreeBSD's
// `s_erf.c`, the implementation Rust's `libm` ports, so results agree with Typst to the last bit.

use crate::diag::DiagnosticKind;
use crate::eval::args::Args;
use crate::eval::func::{
	Func,
	NativeFunc,
};
use crate::eval::lib::decimal::Decimal;
use crate::eval::lib::foundations::{
	finish,
	mismatch,
	need,
	round_with_precision,
	words,
};
use crate::eval::ops;
use crate::eval::scope::Scope;
use crate::eval::value::{
	Angle,
	Fraction,
	Length,
	Module,
	Ratio,
	Value,
};
use crate::eval::Engine;
use crate::syntax::Span;

use oxedyne_fe2o3_core::prelude::*;

use std::cmp::Ordering;
use std::sync::Arc;

native_fns! {
	pub enum CalcFn {
		Abs				=> "abs",
		Pow				=> "pow",
		Exp				=> "exp",
		Sqrt			=> "sqrt",
		Root			=> "root",
		Sin				=> "sin",
		Cos				=> "cos",
		Tan				=> "tan",
		Asin			=> "asin",
		Acos			=> "acos",
		Atan			=> "atan",
		Atan2			=> "atan2",
		Sinh			=> "sinh",
		Cosh			=> "cosh",
		Tanh			=> "tanh",
		Asinh			=> "asinh",
		Acosh			=> "acosh",
		Atanh			=> "atanh",
		Log				=> "log",
		Ln				=> "ln",
		Erf				=> "erf",
		Fact			=> "fact",
		Perm			=> "perm",
		Binom			=> "binom",
		Gcd				=> "gcd",
		Lcm				=> "lcm",
		Floor			=> "floor",
		Ceil			=> "ceil",
		Trunc			=> "trunc",
		Fract			=> "fract",
		Round			=> "round",
		Clamp			=> "clamp",
		Min				=> "min",
		Max				=> "max",
		Even			=> "even",
		Odd				=> "odd",
		Rem				=> "rem",
		DivEuclid		=> "div-euclid",
		RemEuclid		=> "rem-euclid",
		Quo				=> "quo",
		Norm			=> "norm",
	}
}

pub fn define(scope: &mut Scope) {
	let mut s = Scope::new();
	for (n, f) in CalcFn::ALL.iter().enumerate() {
		crate::eval::lib::foundations::define_nth(&mut s, n, f.name(), Value::Func(Func::Native(NativeFunc::Calc(*f))));
	}
	let n = CalcFn::ALL.len();
	crate::eval::lib::foundations::define_nth(&mut s, n, "inf", Value::Float(f64::INFINITY));
	crate::eval::lib::foundations::define_nth(&mut s, n + 1, "pi", Value::Float(std::f64::consts::PI));
	crate::eval::lib::foundations::define_nth(&mut s, n + 2, "tau", Value::Float(std::f64::consts::TAU));
	crate::eval::lib::foundations::define_nth(&mut s, n + 3, "e", Value::Float(std::f64::consts::E));
	scope.define("calc", Value::Module(Arc::new(Module::new("calc", s))));
}

// A number argument as Typst's `DecNum`: an int, a float or a decimal, kept apart. The functions that
// take Typst's plain `Num` read it through `arg_float`, which refuses a decimal.
#[derive(Clone, Copy, Debug)]
enum Num {
	Int(i64),
	Float(f64),
	Decimal(Decimal),
}

impl Num {
	fn float(self) -> Option<f64> {
		match self {
			Num::Int(i)		=> Some(i as f64),
			Num::Float(f)	=> Some(f),
			Num::Decimal(_)	=> None,
		}
	}

	fn decimal(self) -> Option<Decimal> {
		match self {
			Num::Int(i)		=> Some(Decimal::from(i)),
			Num::Float(_)	=> None,
			Num::Decimal(d)	=> Some(d),
		}
	}

	fn is_zero(self) -> bool {
		match self {
			Num::Int(i)		=> i == 0,
			Num::Float(f)	=> f == 0.0,
			Num::Decimal(d)	=> d.is_zero(),
		}
	}
}

fn num(engine: &mut Engine, span: Span, v: Value) -> Outcome<Num> {
	match v {
		Value::Int(i)		=> Ok(Num::Int(i)),
		Value::Float(f)		=> Ok(Num::Float(f)),
		Value::Decimal(d)	=> Ok(Num::Decimal(d)),
		other				=> Err(mismatch(engine, span, "integer, float, or decimal", &other)),
	}
}

fn arg_num(engine: &mut Engine, args: &mut Args, what: &str) -> Outcome<Num> {
	let v = res!(need(engine, args, what));
	num(engine, args.span, v)
}

// An int or a float as `f64`, for the functions Typst gives no decimal form.
fn float_of(engine: &mut Engine, span: Span, v: Value) -> Outcome<f64> {
	match v {
		Value::Int(i)	=> Ok(i as f64),
		Value::Float(f)	=> Ok(f),
		other			=> Err(mismatch(engine, span, "integer or float", &other)),
	}
}

fn arg_float(engine: &mut Engine, args: &mut Args, what: &str) -> Outcome<f64> {
	let v = res!(need(engine, args, what));
	float_of(engine, args.span, v)
}

fn decimal_and_float(engine: &mut Engine, span: Span) -> Error<ErrTag> {
	engine.error_hint(DiagnosticKind::Type, span, "cannot apply this operation to a decimal and a float",
		"if loss of precision is acceptable, explicitly cast the decimal to a float with `float(value)`")
}

fn arg_int(engine: &mut Engine, args: &mut Args, what: &str) -> Outcome<i64> {
	let v = res!(need(engine, args, what));
	match v {
		Value::Int(i)	=> Ok(i),
		other			=> Err(mismatch(engine, args.span, "integer", &other)),
	}
}

// A non-negative integer, as Typst's `u64` parameters demand.
fn arg_nat(engine: &mut Engine, args: &mut Args, what: &str) -> Outcome<u64> {
	let i = res!(arg_int(engine, args, what));
	if i < 0 {
		return Err(engine.error(DiagnosticKind::Type, args.span, "number must be at least zero"));
	}
	Ok(i as u64)
}

// An angle or a plain number of radians.
fn arg_angle(engine: &mut Engine, args: &mut Args) -> Outcome<f64> {
	let v = res!(need(engine, args, "angle"));
	match v {
		Value::Angle(a)	=> Ok(a.0),
		Value::Int(i)	=> Ok(i as f64),
		Value::Float(f)	=> Ok(f),
		other			=> Err(mismatch(engine, args.span, "angle, integer, or float", &other)),
	}
}

fn too_large(engine: &mut Engine, span: Span) -> Error<ErrTag> { engine.error(DiagnosticKind::Type, span, "the result is too large") }

fn not_real(engine: &mut Engine, span: Span) -> Error<ErrTag> { engine.error(DiagnosticKind::Type, span, "the result is not a real number") }

pub fn call(f: CalcFn, engine: &mut Engine, mut args: Args) -> Outcome<Value> {
	let span = args.span;
	let out = match f {
		CalcFn::Abs => {
			let v = res!(need(engine, &mut args, "value"));
			match v {
				Value::Int(i) => match i.checked_abs() {
					Some(a)	=> Value::Int(a),
					None	=> return Err(too_large(engine, span)),
				},
				Value::Float(x)		=> Value::Float(x.abs()),
				Value::Decimal(d)	=> Value::Decimal(d.abs()),
				Value::Length(l)	=> Value::Length(Length { abs: l.abs.abs(), em: l.em.abs() }),
				Value::Angle(a)		=> Value::Angle(Angle(a.0.abs())),
				Value::Ratio(r)		=> Value::Ratio(Ratio(r.0.abs())),
				Value::Fraction(x)	=> Value::Fraction(Fraction(x.0.abs())),
				other => return Err(mismatch(engine, span,
					"integer, float, length, angle, ratio, fraction, or decimal", &other)),
			}
		}
		CalcFn::Pow => {
			let base = res!(arg_num(engine, &mut args, "base"));
			let exp = res!(need(engine, &mut args, "exponent"));
			let exp = match exp {
				Value::Int(i)	=> Num::Int(i),
				other			=> Num::Float(res!(float_of(engine, span, other))),
			};
			res!(pow(engine, span, base, exp))
		}
		CalcFn::Exp => {
			let x = res!(arg_float(engine, &mut args, "exponent"));
			let r = x.exp();
			if r.is_nan() {
				return Err(not_real(engine, span));
			}
			Value::Float(r)
		}
		CalcFn::Sqrt => {
			let x = res!(arg_float(engine, &mut args, "value"));
			if x < 0.0 {
				return Err(engine.error(DiagnosticKind::Type, span, "cannot take square root of negative number"));
			}
			Value::Float(x.sqrt())
		}
		CalcFn::Root => {
			let r = res!(arg_float(engine, &mut args, "radicand"));
			let n = res!(arg_int(engine, &mut args, "index"));
			if n == 0 {
				return Err(engine.error(DiagnosticKind::Type, span, "cannot take the 0th root of a number"));
			}
			if r < 0.0 {
				if n % 2 == 0 {
					return Err(engine.error(DiagnosticKind::Type, span,
						"negative numbers do not have a real nth root when n is even"));
				}
				Value::Float(-(-r).powf(1.0 / n as f64))
			} else if n == 1 {
				Value::Float(r)
			} else if n == 2 {
				Value::Float(r.sqrt())
			} else if n == 3 {
				Value::Float(r.cbrt())
			} else {
				Value::Float(r.powf(1.0 / n as f64))
			}
		}
		CalcFn::Sin => Value::Float(res!(arg_angle(engine, &mut args)).sin()),
		CalcFn::Cos => Value::Float(res!(arg_angle(engine, &mut args)).cos()),
		CalcFn::Tan => Value::Float(res!(arg_angle(engine, &mut args)).tan()),
		CalcFn::Asin | CalcFn::Acos => {
			let x = res!(arg_float(engine, &mut args, "value"));
			if !(-1.0..=1.0).contains(&x) {
				return Err(engine.error(DiagnosticKind::Type, span, "value must be between -1 and 1"));
			}
			Value::Angle(Angle(if f == CalcFn::Asin { x.asin() } else { x.acos() }))
		}
		CalcFn::Atan => Value::Angle(Angle(res!(arg_float(engine, &mut args, "value")).atan())),
		CalcFn::Atan2 => {
			let x = res!(arg_float(engine, &mut args, "x"));
			let y = res!(arg_float(engine, &mut args, "y"));
			Value::Angle(Angle(y.atan2(x)))
		}
		CalcFn::Sinh => Value::Float(res!(arg_float(engine, &mut args, "value")).sinh()),
		CalcFn::Cosh => Value::Float(res!(arg_float(engine, &mut args, "value")).cosh()),
		CalcFn::Tanh => Value::Float(res!(arg_float(engine, &mut args, "value")).tanh()),
		CalcFn::Asinh => Value::Float(res!(arg_float(engine, &mut args, "value")).asinh()),
		CalcFn::Acosh => {
			let x = res!(arg_float(engine, &mut args, "value"));
			if x < 1.0 {
				return Err(engine.error(DiagnosticKind::Type, span, "value must be greater than or equal to 1"));
			}
			Value::Float(x.acosh())
		}
		CalcFn::Atanh => {
			let x = res!(arg_float(engine, &mut args, "value"));
			if x <= -1.0 || x >= 1.0 {
				return Err(engine.error(DiagnosticKind::Type, span, "value must be between -1 and 1 (exclusive)"));
			}
			Value::Float(x.atanh())
		}
		CalcFn::Log => {
			let x = res!(arg_float(engine, &mut args, "value"));
			let base = match res!(args.named::<Value>("base")) {
				None	=> 10.0,
				Some(v)	=> res!(float_of(engine, span, v)),
			};
			if x <= 0.0 {
				return Err(engine.error(DiagnosticKind::Type, span, "value must be strictly positive"));
			}
			if !base.is_normal() {
				return Err(engine.error(DiagnosticKind::Type, span, "base may not be zero, NaN, infinite, or subnormal"));
			}
			let r = if base == std::f64::consts::E {
				x.ln()
			} else if base == 2.0 {
				x.log2()
			} else if base == 10.0 {
				x.log10()
			} else {
				x.ln() / base.ln()
			};
			if r.is_infinite() || r.is_nan() {
				return Err(not_real(engine, span));
			}
			Value::Float(r)
		}
		CalcFn::Ln => {
			let x = res!(arg_float(engine, &mut args, "value"));
			if x <= 0.0 {
				return Err(engine.error(DiagnosticKind::Type, span, "value must be strictly positive"));
			}
			let r = x.ln();
			if r.is_infinite() {
				return Err(not_real(engine, span));
			}
			Value::Float(r)
		}
		CalcFn::Erf => Value::Float(erf(res!(arg_float(engine, &mut args, "value")))),
		CalcFn::Fact => {
			let n = res!(arg_nat(engine, &mut args, "number"));
			let mut acc: i64 = 1;
			for k in 2..=n {
				acc = match acc.checked_mul(k as i64) {
					Some(x)	=> x,
					None	=> return Err(too_large(engine, span)),
				};
			}
			Value::Int(acc)
		}
		CalcFn::Perm => {
			let base = res!(arg_nat(engine, &mut args, "base"));
			let k = res!(arg_nat(engine, &mut args, "numbers"));
			if base < k {
				Value::Int(0)
			} else {
				let mut acc: i64 = 1;
				for x in (base - k + 1)..=base {
					acc = match acc.checked_mul(x as i64) {
						Some(y)	=> y,
						None	=> return Err(too_large(engine, span)),
					};
				}
				Value::Int(acc)
			}
		}
		CalcFn::Binom => {
			let n = res!(arg_nat(engine, &mut args, "n"));
			let k = res!(arg_nat(engine, &mut args, "k"));
			if k > n {
				Value::Int(0)
			} else {
				// Typst's order: multiply by (n - i), then divide by (i + 1), from the smaller side.
				let k = k.min(n - k);
				let mut acc: u64 = 1;
				for i in 0..k {
					acc = match acc.checked_mul(n - i) {
						Some(y)	=> y / (i + 1),
						None	=> return Err(too_large(engine, span)),
					};
				}
				if acc > i64::MAX as u64 {
					return Err(too_large(engine, span));
				}
				Value::Int(acc as i64)
			}
		}
		CalcFn::Gcd => {
			let a = res!(arg_int(engine, &mut args, "a"));
			let b = res!(arg_int(engine, &mut args, "b"));
			Value::Int(gcd(a, b))
		}
		CalcFn::Lcm => {
			let a = res!(arg_int(engine, &mut args, "a"));
			let b = res!(arg_int(engine, &mut args, "b"));
			if a == b {
				Value::Int(a.abs())
			} else if a == 0 || b == 0 {
				Value::Int(0)
			} else {
				match (a / gcd(a, b)).checked_mul(b).and_then(|x| x.checked_abs()) {
					Some(x)	=> Value::Int(x),
					None	=> return Err(too_large(engine, span)),
				}
			}
		}
		CalcFn::Floor | CalcFn::Ceil | CalcFn::Trunc => {
			let n = res!(arg_num(engine, &mut args, "value"));
			match n {
				Num::Int(i)		=> Value::Int(i),
				Num::Float(x)	=> {
					let r = match f {
						CalcFn::Floor	=> x.floor(),
						CalcFn::Ceil	=> x.ceil(),
						_				=> x.trunc(),
					};
					res!(float_to_int(engine, span, r))
				}
				Num::Decimal(d)	=> {
					let r = match f {
						CalcFn::Floor	=> d.floor(),
						CalcFn::Ceil	=> d.ceil(),
						_				=> d.trunc(),
					};
					match r.to_i64() {
						Some(i)	=> Value::Int(i),
						None	=> return Err(too_large(engine, span)),
					}
				}
			}
		}
		CalcFn::Fract => match res!(arg_num(engine, &mut args, "value")) {
			Num::Int(_)		=> Value::Int(0),
			Num::Float(x)	=> Value::Float(x.fract()),
			Num::Decimal(d)	=> Value::Decimal(d.fract()),
		},
		CalcFn::Round => {
			let n = res!(arg_num(engine, &mut args, "value"));
			let digits = match res!(args.named::<Value>("digits")) {
				None				=> 0i64,
				Some(Value::Int(d))	=> d,
				Some(other)			=> return Err(mismatch(engine, span, "integer", &other)),
			};
			let d = digits.clamp(i16::MIN as i64, i16::MAX as i64) as i16;
			match n {
				Num::Int(i) => match round_int_with_precision(i, d) {
					Some(r)	=> Value::Int(r),
					None	=> return Err(too_large(engine, span)),
				},
				Num::Float(x) => Value::Float(round_with_precision(x, d)),
				Num::Decimal(x) => match x.round(digits.clamp(i32::MIN as i64, i32::MAX as i64)) {
					Some(r)	=> Value::Decimal(r),
					None	=> return Err(too_large(engine, span)),
				},
			}
		}
		CalcFn::Clamp => {
			let v = res!(arg_num(engine, &mut args, "value"));
			let lo = res!(arg_num(engine, &mut args, "min"));
			let hi = res!(arg_num(engine, &mut args, "max"));
			match (v, lo, hi) {
				(Num::Int(v), Num::Int(lo), Num::Int(hi)) => {
					if hi < lo {
						return Err(engine.error(DiagnosticKind::Type, span, "max must be greater than or equal to min"));
					}
					Value::Int(v.clamp(lo, hi))
				}
				(Num::Decimal(_), _, _) | (_, Num::Decimal(_), _) | (_, _, Num::Decimal(_)) => {
					// A decimal among them makes the others decimals; a float then cannot join.
					let pair = (lo.decimal(), hi.decimal());
					if let (Some(l), Some(h)) = pair {
						if h < l {
							return Err(engine.error(DiagnosticKind::Type, span, "max must be greater than or equal to min"));
						}
					}
					match (v.decimal(), pair.0, pair.1) {
						(Some(v), Some(l), Some(h))	=> Value::Decimal(if v < l { l } else if v > h { h } else { v }),
						_							=> return Err(decimal_and_float(engine, span)),
					}
				}
				_ => {
					let (v, lo, hi) = (v.float().unwrap_or(0.0), lo.float().unwrap_or(0.0), hi.float().unwrap_or(0.0));
					if hi < lo {
						return Err(engine.error(DiagnosticKind::Type, span, "max must be greater than or equal to min"));
					}
					Value::Float(v.clamp(lo, hi))
				}
			}
		}
		CalcFn::Min | CalcFn::Max => {
			let vals = res!(args.all::<Value>());
			let goal = if f == CalcFn::Min { Ordering::Less } else { Ordering::Greater };
			let mut it = vals.into_iter();
			let mut ext = match it.next() {
				Some(v)	=> v,
				None	=> return Err(engine.error(DiagnosticKind::Type, span, "expected at least one value")),
			};
			for v in it {
				let ord = match ops::compare(&v, &ext) {
					Ok(o)	=> o,
					Err(e)	=> return Err(engine.error(DiagnosticKind::Type, span, words(&e))),
				};
				if ord == goal {
					ext = v;
				}
			}
			ext
		}
		CalcFn::Even | CalcFn::Odd => {
			let i = res!(arg_int(engine, &mut args, "value"));
			Value::Bool((i % 2 == 0) == (f == CalcFn::Even))
		}
		CalcFn::Rem | CalcFn::DivEuclid | CalcFn::RemEuclid | CalcFn::Quo => {
			let a = res!(arg_num(engine, &mut args, "dividend"));
			let b = res!(arg_num(engine, &mut args, "divisor"));
			if b.is_zero() {
				return Err(engine.error(DiagnosticKind::Type, span, "divisor must not be zero"));
			}
			res!(division(engine, span, f, a, b))
		}
		CalcFn::Norm => {
			let p = match res!(args.named::<Value>("p")) {
				None	=> 2.0,
				Some(v)	=> res!(float_of(engine, span, v)),
			};
			let vals = res!(args.all::<Value>());
			let mut xs = Vec::with_capacity(vals.len());
			for v in vals {
				xs.push(res!(float_of(engine, span, v)));
			}
			if p <= 0.0 {
				return Err(engine.error(DiagnosticKind::Type, span, "p must be greater than zero"));
			}
			if p.is_infinite() {
				Value::Float(xs.iter().fold(0.0f64, |m, x| m.max(x.abs())))
			} else {
				Value::Float(xs.iter().map(|x| x.abs().powf(p)).sum::<f64>().powf(1.0 / p))
			}
		}
	};
	res!(finish(engine, args));
	Ok(out)
}

fn float_to_int(engine: &mut Engine, span: Span, x: f64) -> Outcome<Value> {
	if x.is_nan() {
		return Ok(Value::Int(0));
	}
	if x < -9.223372036854776e18 || x >= 9.223372036854776e18 {
		return Err(too_large(engine, span));
	}
	Ok(Value::Int(x as i64))
}

fn pow(engine: &mut Engine, span: Span, base: Num, exp: Num) -> Outcome<Value> {
	if exp.is_zero() && base.is_zero() {
		return Err(engine.error(DiagnosticKind::Type, span, "zero to the power of zero is undefined"));
	}
	match exp {
		Num::Int(i) if i32::try_from(i).is_err()
			=> return Err(engine.error(DiagnosticKind::Type, span, "exponent is too large")),
		Num::Float(x) if !x.is_normal() && x != 0.0
			=> return Err(engine.error(DiagnosticKind::Type, span, "exponent may not be infinite, subnormal, or NaN")),
		_ => (),
	}
	if let (Num::Int(a), Num::Int(b)) = (base, exp) {
		if b >= 0 {
			return match a.checked_pow(b as u32) {
				Some(r)	=> Ok(Value::Int(r)),
				None	=> Err(too_large(engine, span)),
			};
		}
	}
	if let (Num::Decimal(a), Num::Int(b)) = (base, exp) {
		return match a.checked_powi(b) {
			Some(r)	=> Ok(Value::Decimal(r)),
			None	=> Err(too_large(engine, span)),
		};
	}
	let a = match base.float() {
		Some(a)	=> a,
		None	=> return Err(decimal_and_float(engine, span)),
	};
	let e = exp.float().unwrap_or(0.0);
	let r = if a == std::f64::consts::E {
		e.exp()
	} else if a == 2.0 {
		e.exp2()
	} else if let Num::Int(b) = exp {
		a.powi(b as i32)
	} else {
		a.powf(e)
	};
	if r.is_nan() {
		return Err(not_real(engine, span));
	}
	Ok(Value::Float(r))
}

fn division(engine: &mut Engine, span: Span, f: CalcFn, a: Num, b: Num) -> Outcome<Value> {
	match (a, b) {
		(Num::Int(x), Num::Int(y)) => {
			let r = match f {
				CalcFn::Rem			=> if y == -1 { Some(0) } else { x.checked_rem(y) },
				CalcFn::DivEuclid	=> x.checked_div_euclid(y),
				CalcFn::RemEuclid	=> if y == -1 { Some(0) } else { x.checked_rem_euclid(y) },
				_					=> x.checked_div(y).map(|q| {
					// Floor division: truncation rounds towards zero, so step down when signs differ.
					if x % y != 0 && ((x < 0) != (y < 0)) { q - 1 } else { q }
				}),
			};
			match r {
				Some(v)	=> Ok(Value::Int(v)),
				None	=> Err(too_large(engine, span)),
			}
		}
		(Num::Decimal(_), _) | (_, Num::Decimal(_)) => {
			let (x, y) = match (a.decimal(), b.decimal()) {
				(Some(x), Some(y))	=> (x, y),
				_					=> return Err(decimal_and_float(engine, span)),
			};
			let r = match f {
				CalcFn::Rem			=> x.checked_rem(y),
				CalcFn::DivEuclid	=> x.checked_div_euclid(y),
				CalcFn::RemEuclid	=> x.checked_rem_euclid(y),
				_					=> x.checked_div(y).map(|q| q.floor()),
			};
			match (r, f) {
				(Some(r), CalcFn::Quo)	=> match r.to_i64() {
					Some(i)	=> Ok(Value::Int(i)),
					None	=> Err(too_large(engine, span)),
				},
				(Some(r), _)			=> Ok(Value::Decimal(r)),
				(None, CalcFn::Rem | CalcFn::RemEuclid)
										=> Err(engine.error(DiagnosticKind::Type, span, "dividend too small compared to divisor")),
				(None, _)				=> Err(too_large(engine, span)),
			}
		}
		_ => {
			let (x, y) = (a.float().unwrap_or(0.0), b.float().unwrap_or(1.0));
			match f {
				CalcFn::Rem			=> Ok(Value::Float(x % y)),
				CalcFn::DivEuclid	=> Ok(Value::Float(x.div_euclid(y))),
				CalcFn::RemEuclid	=> Ok(Value::Float(x.rem_euclid(y))),
				_					=> float_to_int(engine, span, (x / y).floor()),
			}
		}
	}
}

fn gcd(a: i64, b: i64) -> i64 {
	let (mut a, mut b) = (a, b);
	while b != 0 {
		let t = b;
		b = a % b;
		a = t;
	}
	a.abs()
}

fn round_int_with_precision(value: i64, precision: i16) -> Option<i64> {
	if precision >= 0 {
		return Some(value);
	}
	let digits = (-(precision as i32)) as u32;
	let ten = match 10i64.checked_pow(digits - 1) {
		Some(t)	=> t,
		None	=> return Some(0),
	};
	let truncated = value / ten;
	if truncated == 0 {
		return Some(0);
	}
	let last = (truncated % 10).abs();
	let rounded = if last >= 5 {
		match truncated.checked_add(truncated.signum() * (10 - last)) {
			Some(r)	=> r,
			None	=> return None,
		}
	} else {
		truncated - (truncated % 10)
	};
	rounded.checked_mul(ten)
}

// erf, after FreeBSD's s_erf.c (Sun Microsystems, 1993; freely redistributable with notice).

const ERX:	f64 = 8.45062911510467529297e-01;
const EFX8:	f64 = 1.02703333676410069053e+00;
const PP:	[f64; 5] = [1.28379167095512558561e-01, -3.25042107247001499370e-01,
	-2.84817495755985104766e-02, -5.77027029648944159157e-03, -2.37630166566501626084e-05];
const QQ:	[f64; 5] = [3.97917223959155352819e-01, 6.50222499887672944485e-02,
	5.08130628187576562776e-03, 1.32494738004321644526e-04, -3.96022827877536812320e-06];
const PA:	[f64; 7] = [-2.36211856075265944077e-03, 4.14856118683748331666e-01,
	-3.72207876035701323847e-01, 3.18346619901161753674e-01, -1.10894694282396677476e-01,
	3.54783043256182359371e-02, -2.16637559486879084300e-03];
const QA:	[f64; 6] = [1.06420880400844228286e-01, 5.40397917702171048937e-01,
	7.18286544141962662868e-02, 1.26171219808761642112e-01, 1.36370839120290507362e-02,
	1.19844998467991074170e-02];
const RA:	[f64; 8] = [-9.86494403484714822705e-03, -6.93858572707181764372e-01,
	-1.05586262253232909814e+01, -6.23753324503260060396e+01, -1.62396669462573470355e+02,
	-1.84605092906711035994e+02, -8.12874355063065934246e+01, -9.81432934416914548592e+00];
const SA:	[f64; 8] = [1.96512716674392571292e+01, 1.37657754143519042600e+02,
	4.34565877475229228821e+02, 6.45387271733267880336e+02, 4.29008140027567833386e+02,
	1.08635005541779435134e+02, 6.57024977031928170135e+00, -6.04244152148580987438e-02];
const RB:	[f64; 7] = [-9.86494292470009928597e-03, -7.99283237680523006574e-01,
	-1.77579549177547519889e+01, -1.60636384855821916062e+02, -6.37566443368389627722e+02,
	-1.02509513161107724954e+03, -4.83519191608651397019e+02];
const SB:	[f64; 7] = [3.03380607434824582924e+01, 3.25792512996573918826e+02,
	1.53672958608443695994e+03, 3.19985821950859553908e+03, 2.55305040643316442583e+03,
	4.74528541206955367215e+02, -2.24409524465858183362e+01];

fn erf(x: f64) -> f64 {
	let bits = x.to_bits();
	let hx = (bits >> 32) as u32;
	let sign = hx >> 31;
	let ix = hx & 0x7fff_ffff;
	if ix >= 0x7ff0_0000 {
		// erf(nan) is nan, erf(+-inf) is +-1.
		return 1.0 - 2.0 * sign as f64 + 1.0 / x;
	}
	if ix < 0x3feb_0000 {
		// |x| < 0.84375
		if ix < 0x3e30_0000 {
			// |x| < 2**-28
			return 0.125 * (8.0 * x + EFX8 * x);
		}
		let z = x * x;
		let r = PP[0] + z * (PP[1] + z * (PP[2] + z * (PP[3] + z * PP[4])));
		let s = 1.0 + z * (QQ[0] + z * (QQ[1] + z * (QQ[2] + z * (QQ[3] + z * QQ[4]))));
		return x + x * (r / s);
	}
	if ix < 0x3ff4_0000 {
		// 0.84375 <= |x| < 1.25
		let s = x.abs() - 1.0;
		let p = PA[0] + s * (PA[1] + s * (PA[2] + s * (PA[3] + s * (PA[4] + s * (PA[5] + s * PA[6])))));
		let q = 1.0 + s * (QA[0] + s * (QA[1] + s * (QA[2] + s * (QA[3] + s * (QA[4] + s * QA[5])))));
		return if sign == 0 { ERX + p / q } else { -ERX - p / q };
	}
	if ix >= 0x4018_0000 {
		// |x| >= 6
		return if sign == 0 { 1.0 - 1e-300 } else { 1e-300 - 1.0 };
	}
	let ax = x.abs();
	let s = 1.0 / (ax * ax);
	let (r, sv) = if ix < 0x4006_db6d {
		// |x| < 1/0.35
		(RA[0] + s * (RA[1] + s * (RA[2] + s * (RA[3] + s * (RA[4] + s * (RA[5] + s * (RA[6] + s * RA[7])))))),
		 1.0 + s * (SA[0] + s * (SA[1] + s * (SA[2] + s * (SA[3] + s * (SA[4] + s * (SA[5] + s * (SA[6] + s * SA[7]))))))))
	} else {
		(RB[0] + s * (RB[1] + s * (RB[2] + s * (RB[3] + s * (RB[4] + s * (RB[5] + s * RB[6]))))),
		 1.0 + s * (SB[0] + s * (SB[1] + s * (SB[2] + s * (SB[3] + s * (SB[4] + s * (SB[5] + s * SB[6])))))))
	};
	// z is ax with its low 32 bits cleared.
	let z = f64::from_bits(ax.to_bits() & 0xffff_ffff_0000_0000);
	let e = (-z * z - 0.5625).exp() * ((z - ax) * (z + ax) + r / sv).exp();
	if sign == 0 { 1.0 - e / ax } else { e / ax - 1.0 }
}
