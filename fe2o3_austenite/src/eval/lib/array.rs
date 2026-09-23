// U3 owns this file. Methods on `array`. `push`, `pop`, `insert` and `remove` mutate their receiver and
// arrive through `call_mut` (the evaluator's `call_method_mut` holds the place); called through
// `NativeFunc::call` on a temporary they fail as Typst's do. Comparison, equality, addition and joining
// are `ops.rs`'s, so `sorted`, `dedup`, `sum` and `join` agree with the operators by construction.

use crate::eval::args::Args;
use crate::eval::func::Func;
use crate::eval::lib::foundations::{
	finish,
	int_of,
	locate_bound,
	locate_index,
	mismatch,
	named_bool,
	need,
	receiver,
	slice_bounds,
	words,
};
use crate::eval::ops;
use crate::eval::scope::Scope;
use crate::eval::value::{
	Dict,
	Value,
};
use crate::eval::Engine;
use crate::syntax::Span;

use oxedyne_fe2o3_core::prelude::*;

use std::cmp::Ordering;
use std::sync::Arc;

native_fns! {
	pub enum ArrayFn {
		Construct		=> "array",
		Len				=> "len",
		First			=> "first",
		Last			=> "last",
		At				=> "at",
		Push			=> "push",
		Pop				=> "pop",
		Insert			=> "insert",
		Remove			=> "remove",
		Slice			=> "slice",
		Contains		=> "contains",
		Find			=> "find",
		Position		=> "position",
		Filter			=> "filter",
		Map				=> "map",
		Enumerate		=> "enumerate",
		Zip				=> "zip",
		Fold			=> "fold",
		Reduce			=> "reduce",
		Sum				=> "sum",
		Product			=> "product",
		Any				=> "any",
		All				=> "all",
		Flatten			=> "flatten",
		Rev				=> "rev",
		Split			=> "split",
		Join			=> "join",
		Intersperse		=> "intersperse",
		Chunks			=> "chunks",
		Windows			=> "windows",
		Sorted			=> "sorted",
		Dedup			=> "dedup",
		ToDict			=> "to-dict",
	}
}

pub fn define(_scope: &mut Scope) {}

pub fn method(name: &str) -> Option<ArrayFn> {
	ArrayFn::ALL.iter().copied().find(|f| f.name() == name && *f != ArrayFn::Construct)
}

impl ArrayFn {
	pub fn mutates(self) -> bool {
		matches!(self, ArrayFn::Push | ArrayFn::Pop | ArrayFn::Insert | ArrayFn::Remove)
	}
}

fn out_of_bounds(engine: &mut Engine, span: Span, i: i64, len: usize) -> Error<ErrTag> {
	engine.error(span, fmt!("array index out of bounds (index: {}, len: {})", i, len))
}

fn no_default(engine: &mut Engine, span: Span, i: i64, len: usize) -> Error<ErrTag> {
	engine.error(span, fmt!(
		"array index out of bounds (index: {}, len: {}) and no default value was specified", i, len))
}

pub fn func_of(engine: &mut Engine, span: Span, v: Value) -> Outcome<Func> {
	match crate::eval::lib::foundations::as_func(&v) {
		Some(f)	=> Ok(f),
		None	=> Err(mismatch(engine, span, "function", &v)),
	}
}

/// Calls `f` with the given positional arguments.
pub fn apply(engine: &mut Engine, span: Span, f: &Func, vals: Vec<Value>) -> Outcome<Value> {
	let mut a = Args::new(span);
	for v in vals {
		a.push(span, v);
	}
	engine.call_func(f, a)
}

fn test(engine: &mut Engine, span: Span, f: &Func, v: Value) -> Outcome<bool> {
	match res!(apply(engine, span, f, vec![v])) {
		Value::Bool(b)	=> Ok(b),
		other			=> Err(mismatch(engine, span, "boolean", &other)),
	}
}

// An operator's error, as a diagnostic at the call.
fn op_error(engine: &mut Engine, span: Span, e: Error<ErrTag>) -> Error<ErrTag> {
	let w = words(&e);
	engine.error(span, w)
}

/// A mutating method on a place: `arr.push(x)`. The evaluator passes the variable's value in place.
pub fn call_mut(f: ArrayFn, engine: &mut Engine, recv: &mut Value, mut args: Args) -> Outcome<Value> {
	let span = args.span;
	let arr = match recv {
		Value::Array(a)	=> Arc::make_mut(a),
		other			=> return Err(mismatch(engine, span, "array", other)),
	};
	let out = match f {
		ArrayFn::Push => {
			let v = res!(need(engine, &mut args, "value"));
			arr.push(v);
			Value::None
		}
		ArrayFn::Pop => match arr.pop() {
			Some(v)	=> v,
			None	=> return Err(engine.error(span, "array is empty")),
		},
		ArrayFn::Insert => {
			let i = res!(need(engine, &mut args, "index"));
			let i = res!(int_of(engine, span, i));
			let v = res!(need(engine, &mut args, "value"));
			match res!(locate_bound(engine, span, i, arr.len(), &|_| true)) {
				Some(k)	=> arr.insert(k, v),
				None	=> return Err(out_of_bounds(engine, span, i, arr.len())),
			}
			Value::None
		}
		ArrayFn::Remove => {
			let i = res!(need(engine, &mut args, "index"));
			let i = res!(int_of(engine, span, i));
			let default = res!(args.named::<Value>("default"));
			match (locate_index(i, arr.len()), default) {
				(Some(k), _)	=> arr.remove(k),
				(None, Some(d))	=> d,
				(None, None)	=> return Err(no_default(engine, span, i, arr.len())),
			}
		}
		_ => return Err(engine.error(span, fmt!("array.{} does not mutate its receiver", f.name()))),
	};
	res!(finish(engine, args));
	Ok(out)
}

pub fn call(f: ArrayFn, engine: &mut Engine, mut args: Args) -> Outcome<Value> {
	let span = args.span;
	if f == ArrayFn::Construct {
		let v = res!(need(engine, &mut args, "value"));
		res!(finish(engine, args));
		return match v {
			Value::Array(a)		=> Ok(Value::Array(a)),
			Value::Bytes(b)		=> Ok(Value::array(b.iter().map(|x| Value::Int(*x as i64)).collect())),
			Value::Version(p)	=> Ok(Value::array(p.iter().map(|x| Value::Int(*x as i64)).collect())),
			other				=> Err(mismatch(engine, span, "array, bytes, or version", &other)),
		};
	}
	if f.mutates() {
		return Err(engine.error(span, "cannot mutate a temporary value"));
	}
	let recv = res!(receiver(&mut args));
	let arr = match recv {
		Value::Array(a)	=> a,
		other			=> return Err(mismatch(engine, span, "array", &other)),
	};
	let out = match f {
		ArrayFn::Len => Value::Int(arr.len() as i64),
		ArrayFn::First | ArrayFn::Last => {
			let default = res!(args.named::<Value>("default"));
			let v = if f == ArrayFn::First { arr.first() } else { arr.last() };
			match (v, default) {
				(Some(v), _)	=> v.clone(),
				(None, Some(d))	=> d,
				(None, None)	=> return Err(engine.error(span, "array is empty")),
			}
		}
		ArrayFn::At => {
			let i = res!(need(engine, &mut args, "index"));
			let i = res!(int_of(engine, span, i));
			let default = res!(args.named::<Value>("default"));
			match (locate_index(i, arr.len()), default) {
				(Some(k), _)	=> arr[k].clone(),
				(None, Some(d))	=> d,
				(None, None)	=> return Err(no_default(engine, span, i, arr.len())),
			}
		}
		ArrayFn::Slice => {
			let (a, b) = res!(slice_bounds(engine, &mut args, arr.len(), "array", |_| true));
			Value::array(arr[a..b].to_vec())
		}
		ArrayFn::Contains => {
			let v = res!(need(engine, &mut args, "value"));
			Value::Bool(arr.iter().any(|x| ops::equal(x, &v)))
		}
		ArrayFn::Find | ArrayFn::Position | ArrayFn::Any | ArrayFn::All | ArrayFn::Filter => {
			let fv = res!(need(engine, &mut args, "searcher"));
			let func = res!(func_of(engine, span, fv));
			res!(finish(engine, args));
			return search(engine, span, f, &arr, &func);
		}
		ArrayFn::Map => {
			let fv = res!(need(engine, &mut args, "mapper"));
			let func = res!(func_of(engine, span, fv));
			res!(finish(engine, args));
			let mut out = Vec::with_capacity(arr.len());
			for v in arr.iter() {
				out.push(res!(apply(engine, span, &func, vec![v.clone()])));
			}
			return Ok(Value::array(out));
		}
		ArrayFn::Enumerate => {
			let start = match res!(args.named::<Value>("start")) {
				None	=> 0,
				Some(v)	=> res!(int_of(engine, span, v)),
			};
			let mut out = Vec::with_capacity(arr.len());
			for (i, v) in arr.iter().enumerate() {
				let k = match start.checked_add(i as i64) {
					Some(k)	=> k,
					None	=> return Err(engine.error(span, "array index is too large")),
				};
				out.push(Value::array(vec![Value::Int(k), v.clone()]));
			}
			Value::array(out)
		}
		ArrayFn::Zip => {
			let exact = res!(named_bool(engine, &mut args, "exact", false));
			let others = res!(args.all::<Value>());
			let one = others.len() == 1;
			let mut lists: Vec<Arc<Vec<Value>>> = vec![arr.clone()];
			for o in others {
				match o {
					Value::Array(a) => {
						if exact && a.len() != arr.len() {
							let which = if one { "second array" } else { "array" };
							return Err(engine.error(span, fmt!(
								"{} has different length ({}) from first array ({})", which, a.len(), arr.len())));
						}
						lists.push(a);
					}
					other => return Err(mismatch(engine, span, "array", &other)),
				}
			}
			let n = lists.iter().map(|l| l.len()).min().unwrap_or(0);
			let out = (0..n).map(|i| Value::array(lists.iter().map(|l| l[i].clone()).collect())).collect();
			Value::array(out)
		}
		ArrayFn::Fold => {
			let init = res!(need(engine, &mut args, "init"));
			let fv = res!(need(engine, &mut args, "folder"));
			let func = res!(func_of(engine, span, fv));
			res!(finish(engine, args));
			let mut acc = init;
			for v in arr.iter() {
				acc = res!(apply(engine, span, &func, vec![acc, v.clone()]));
			}
			return Ok(acc);
		}
		ArrayFn::Reduce => {
			let fv = res!(need(engine, &mut args, "reducer"));
			let func = res!(func_of(engine, span, fv));
			res!(finish(engine, args));
			let mut it = arr.iter();
			let mut acc = match it.next() {
				Some(v)	=> v.clone(),
				None	=> return Ok(Value::None),
			};
			for v in it {
				acc = res!(apply(engine, span, &func, vec![acc, v.clone()]));
			}
			return Ok(acc);
		}
		ArrayFn::Sum | ArrayFn::Product => {
			let default = res!(args.named::<Value>("default"));
			let mut it = arr.iter();
			let mut acc = match (it.next(), default) {
				(Some(v), _)	=> v.clone(),
				(None, Some(d))	=> d,
				(None, None)	=> return Err(engine.error(span, fmt!(
					"cannot calculate {} of empty array with no default",
					if f == ArrayFn::Sum { "sum" } else { "product" }))),
			};
			for v in it {
				let r = if f == ArrayFn::Sum { ops::add(acc, v.clone()) } else { ops::mul(acc, v.clone()) };
				acc = match r {
					Ok(x)	=> x,
					Err(e)	=> return Err(op_error(engine, span, e)),
				};
			}
			acc
		}
		ArrayFn::Flatten => {
			let mut out = Vec::new();
			flatten(&arr, &mut out);
			Value::array(out)
		}
		ArrayFn::Rev => {
			let mut v = (*arr).clone();
			v.reverse();
			Value::array(v)
		}
		ArrayFn::Split => {
			let at = res!(need(engine, &mut args, "at"));
			let mut out = Vec::new();
			let mut cur = Vec::new();
			for v in arr.iter() {
				if ops::equal(v, &at) {
					out.push(Value::array(std::mem::take(&mut cur)));
				} else {
					cur.push(v.clone());
				}
			}
			out.push(Value::array(cur));
			Value::array(out)
		}
		ArrayFn::Join => {
			let sep = res!(args.eat::<Value>()).unwrap_or(Value::None);
			let mut last = res!(args.named::<Value>("last"));
			let default = res!(args.named::<Value>("default"));
			res!(finish(engine, args));
			if arr.is_empty() {
				return Ok(default.unwrap_or(Value::None));
			}
			let len = arr.len();
			let mut acc = Value::None;
			for (i, v) in arr.iter().enumerate() {
				if i > 0 {
					let s = match (i + 1 == len, last.take()) {
						(true, Some(l))	=> l,
						(_, l)			=> {
							last = l;
							sep.clone()
						}
					};
					acc = match ops::join(acc, s) {
						Ok(x)	=> x,
						Err(e)	=> return Err(op_error(engine, span, e)),
					};
				}
				acc = match ops::join(acc, v.clone()) {
					Ok(x)	=> x,
					Err(e)	=> return Err(op_error(engine, span, e)),
				};
			}
			return Ok(acc);
		}
		ArrayFn::Intersperse => {
			let sep = res!(need(engine, &mut args, "separator"));
			let mut out = Vec::with_capacity(arr.len() * 2);
			for (i, v) in arr.iter().enumerate() {
				if i > 0 {
					out.push(sep.clone());
				}
				out.push(v.clone());
			}
			Value::array(out)
		}
		ArrayFn::Chunks | ArrayFn::Windows => {
			let n = res!(need(engine, &mut args, "chunk-size"));
			let n = res!(int_of(engine, span, n));
			let exact = if f == ArrayFn::Chunks {
				res!(named_bool(engine, &mut args, "exact", false))
			} else {
				false
			};
			if n <= 0 {
				return Err(engine.error(span, "number must be positive"));
			}
			let n = n as usize;
			let out: Vec<Value> = if f == ArrayFn::Chunks {
				arr.chunks(n).filter(|c| !exact || c.len() == n).map(|c| Value::array(c.to_vec())).collect()
			} else {
				arr.windows(n).map(|c| Value::array(c.to_vec())).collect()
			};
			Value::array(out)
		}
		ArrayFn::Sorted => {
			let key = match res!(args.named::<Value>("key")) {
				None | Some(Value::None)	=> None,
				Some(v)						=> Some(res!(func_of(engine, span, v))),
			};
			let by = match res!(args.named::<Value>("by")) {
				None | Some(Value::None)	=> None,
				Some(v)						=> Some(res!(func_of(engine, span, v))),
			};
			res!(finish(engine, args));
			return sorted(engine, span, &arr, key, by);
		}
		ArrayFn::Dedup => {
			let key = match res!(args.named::<Value>("key")) {
				None | Some(Value::None)	=> None,
				Some(v)						=> Some(res!(func_of(engine, span, v))),
			};
			res!(finish(engine, args));
			let mut keys: Vec<Value> = Vec::new();
			let mut out = Vec::new();
			for v in arr.iter() {
				let k = match &key {
					Some(kf)	=> res!(apply(engine, span, kf, vec![v.clone()])),
					None		=> v.clone(),
				};
				if !keys.iter().any(|x| ops::equal(x, &k)) {
					keys.push(k);
					out.push(v.clone());
				}
			}
			return Ok(Value::array(out));
		}
		ArrayFn::ToDict => {
			let mut d = Dict::new();
			for v in arr.iter() {
				let pair = match v {
					Value::Array(p) => p,
					other => return Err(engine.error(span, fmt!(
						"expected (str, any) pairs, found {}", other.ty().name()))),
				};
				if pair.len() != 2 {
					return Err(engine.error(span, fmt!("expected pairs of length 2, found length {}", pair.len())));
				}
				match &pair[0] {
					Value::Str(k)	=> d.insert(k, pair[1].clone()),
					other			=> return Err(engine.error(span, fmt!(
						"expected key of type str, found {}",
						crate::eval::lib::foundations::type_desc(other.ty())))),
				}
			}
			Value::dict(d)
		}
		ArrayFn::Construct | ArrayFn::Push | ArrayFn::Pop | ArrayFn::Insert | ArrayFn::Remove => Value::None,
	};
	res!(finish(engine, args));
	Ok(out)
}

fn search(engine: &mut Engine, span: Span, f: ArrayFn, arr: &[Value], func: &Func) -> Outcome<Value> {
	match f {
		ArrayFn::Find => {
			for v in arr {
				if res!(test(engine, span, func, v.clone())) {
					return Ok(v.clone());
				}
			}
			Ok(Value::None)
		}
		ArrayFn::Position => {
			for (i, v) in arr.iter().enumerate() {
				if res!(test(engine, span, func, v.clone())) {
					return Ok(Value::Int(i as i64));
				}
			}
			Ok(Value::None)
		}
		ArrayFn::Any => {
			for v in arr {
				if res!(test(engine, span, func, v.clone())) {
					return Ok(Value::Bool(true));
				}
			}
			Ok(Value::Bool(false))
		}
		ArrayFn::All => {
			for v in arr {
				if !res!(test(engine, span, func, v.clone())) {
					return Ok(Value::Bool(false));
				}
			}
			Ok(Value::Bool(true))
		}
		_ => {
			let mut out = Vec::new();
			for v in arr {
				if res!(test(engine, span, func, v.clone())) {
					out.push(v.clone());
				}
			}
			Ok(Value::array(out))
		}
	}
}

fn flatten(arr: &[Value], out: &mut Vec<Value>) {
	for v in arr {
		match v {
			Value::Array(a)	=> flatten(a, out),
			other			=> out.push(other.clone()),
		}
	}
}

fn sorted(engine: &mut Engine, span: Span, arr: &[Value], key: Option<Func>, by: Option<Func>) -> Outcome<Value> {
	if key.is_some() && by.is_some() {
		return Err(engine.error(span, "`key` and `by` are mutually exclusive"));
	}
	// Pair each element with its sort key, so a key function runs once per element.
	let mut items: Vec<(Value, Value)> = Vec::with_capacity(arr.len());
	for v in arr {
		let k = match &key {
			Some(kf)	=> res!(apply(engine, span, kf, vec![v.clone()])),
			None		=> v.clone(),
		};
		items.push((k, v.clone()));
	}
	let items = match by {
		Some(byf) => res!(merge_sort(items, &mut |a: &(Value, Value), b: &(Value, Value)| {
			let ab = res!(by_call(engine, span, &byf, &a.1, &b.1));
			if ab {
				return Ok(Ordering::Less);
			}
			let ba = res!(by_call(engine, span, &byf, &b.1, &a.1));
			Ok(if ba { Ordering::Greater } else { Ordering::Equal })
		})),
		None => res!(merge_sort(items, &mut |a: &(Value, Value), b: &(Value, Value)| {
			match ops::compare(&a.0, &b.0) {
				Ok(o)	=> Ok(o),
				Err(e)	=> Err(engine.error_hint(span, words(&e),
					"consider choosing a `key` or defining the comparison with `by`")),
			}
		})),
	};
	Ok(Value::array(items.into_iter().map(|(_, v)| v).collect()))
}

fn by_call(engine: &mut Engine, span: Span, f: &Func, a: &Value, b: &Value) -> Outcome<bool> {
	match res!(apply(engine, span, f, vec![a.clone(), b.clone()])) {
		Value::Bool(x)	=> Ok(x),
		other			=> Err(engine.error(span, fmt!(
			"expected boolean from `by` function, got {}",
			crate::eval::lib::foundations::type_desc(other.ty())))),
	}
}

/// A stable bottom-up merge sort whose comparison may fail (a closure erring, incomparable values).
fn merge_sort<T: Clone, F: FnMut(&T, &T) -> Outcome<Ordering>>(v: Vec<T>, cmp: &mut F) -> Outcome<Vec<T>> {
	let n = v.len();
	let mut src = v;
	let mut width = 1;
	while width < n {
		let mut dst: Vec<T> = Vec::with_capacity(n);
		let mut lo = 0;
		while lo < n {
			let mid = (lo + width).min(n);
			let hi = (lo + 2 * width).min(n);
			let (mut i, mut j) = (lo, mid);
			while i < mid && j < hi {
				// Take from the right only when strictly less, which keeps equal elements in order.
				if res!(cmp(&src[j], &src[i])) == Ordering::Less {
					dst.push(src[j].clone());
					j += 1;
				} else {
					dst.push(src[i].clone());
					i += 1;
				}
			}
			dst.extend_from_slice(&src[i..mid]);
			dst.extend_from_slice(&src[j..hi]);
			lo = hi;
		}
		src = dst;
		width *= 2;
	}
	Ok(src)
}
