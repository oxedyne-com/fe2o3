// U2 owns this file: Typst's operators over `Value`. `equal` has a scalar body from U0 so selector
// matching (U4) can be tested before U2 lands; U2 completes it.

use crate::eval::func::unimplemented;
use crate::eval::value::Value;

use oxedyne_fe2o3_core::prelude::*;

use std::cmp::Ordering;

pub fn pos(_v: Value) -> Outcome<Value> { Err(unimplemented("ops", "pos")) }
pub fn neg(_v: Value) -> Outcome<Value> { Err(unimplemented("ops", "neg")) }
pub fn not(_v: Value) -> Outcome<Value> { Err(unimplemented("ops", "not")) }
pub fn add(_a: Value, _b: Value) -> Outcome<Value> { Err(unimplemented("ops", "add")) }
pub fn sub(_a: Value, _b: Value) -> Outcome<Value> { Err(unimplemented("ops", "sub")) }
pub fn mul(_a: Value, _b: Value) -> Outcome<Value> { Err(unimplemented("ops", "mul")) }
pub fn div(_a: Value, _b: Value) -> Outcome<Value> { Err(unimplemented("ops", "div")) }

/// Joins two values as markup juxtaposition and `+` on content do: strings concatenate, content
/// sequences, `none` is the identity.
pub fn join(_a: Value, _b: Value) -> Outcome<Value> { Err(unimplemented("ops", "join")) }

/// Typst's `<`, `<=`, `>`, `>=`; comparing incomparable types is an error.
pub fn compare(_a: &Value, _b: &Value) -> Outcome<Ordering> { Err(unimplemented("ops", "compare")) }

/// Typst's `in`.
pub fn contains(_a: &Value, _b: &Value) -> Outcome<bool> { Err(unimplemented("ops", "contains")) }

/// Typst's `==`, which never fails: values of different types are unequal, except int and float.
pub fn equal(a: &Value, b: &Value) -> bool {
	match (a, b) {
		(Value::None, Value::None)				=> true,
		(Value::Auto, Value::Auto)				=> true,
		(Value::Bool(x), Value::Bool(y))		=> x == y,
		(Value::Int(x), Value::Int(y))			=> x == y,
		(Value::Float(x), Value::Float(y))		=> x == y,
		(Value::Int(x), Value::Float(y))		=> (*x as f64) == *y,
		(Value::Float(x), Value::Int(y))		=> *x == (*y as f64),
		(Value::Str(x), Value::Str(y))			=> x == y,
		(Value::Label(x), Value::Label(y))		=> x == y,
		(Value::Length(x), Value::Length(y))	=> x == y,
		(Value::Ratio(x), Value::Ratio(y))		=> x == y,
		(Value::Alignment(x), Value::Alignment(y))	=> x == y,
		(Value::Type(x), Value::Type(y))		=> x == y,
		(Value::Array(x), Value::Array(y))		=> x.len() == y.len()
			&& x.iter().zip(y.iter()).all(|(p, q)| equal(p, q)),
		_										=> false,
	}
}
