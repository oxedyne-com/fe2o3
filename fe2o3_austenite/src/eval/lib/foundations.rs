// U3 owns this file. Foundations: `type`, `repr`, `assert`, `panic`, `eval`, `range`, the type
// constructors, `plugin` (refused with a diagnostic, owner decision 3), and methods on int, float, version
// and bytes. `repr` is the one text form every other unit prints a value by.
//
// Three entry points exist for the evaluator core (U2) beyond `call`: `constructor(ty)` is what calling a
// type value runs (`int("3")`, `str(x)`), `type_scope(ty, name)` answers `str.from-unicode`, `float.inf`
// and the unbound methods (`str.len`), and `field(value, name)` answers `1pt.abs`, `(50% + 1pt).ratio`,
// `(left + top).x`, `stroke.paint` and `version.major`. `method(ty, name)` is the one method table.

use crate::eval::args::Args;
use crate::eval::content::{
	Content,
	ElemKind,
};
use crate::eval::eval::{
	eval_string,
	EvalMode,
};
use crate::eval::func::{
	Func,
	NativeFunc,
};
use crate::eval::intro::{
	Counter,
	CounterKey,
	State,
};
use crate::eval::lib::{
	array,
	color,
	datetime,
	dict,
	geom,
	string,
	sym,
};
use crate::eval::lib;
use crate::eval::scope::Scope;
use crate::eval::select::Selector;
use crate::eval::value::{
	Alignment,
	Angle,
	Dash,
	DashItem,
	Datetime,
	Dict,
	Direction,
	Duration,
	Fraction,
	HAlign,
	Label,
	Length,
	LineCap,
	LineJoin,
	Module,
	Paint,
	Ratio,
	RegexValue,
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
	pub enum FoundFn {
		Type			=> "type",
		Repr			=> "repr",
		Assert			=> "assert",
		AssertEq		=> "eq",
		AssertNe		=> "ne",
		Panic			=> "panic",
		Eval			=> "eval",
		Range			=> "range",
		Int				=> "int",
		Float			=> "float",
		Str				=> "str",
		Bool			=> "bool",
		Label			=> "label",
		Regex			=> "regex",
		Version			=> "version",
		Bytes			=> "bytes",
		Arguments		=> "arguments",
		Plugin			=> "plugin",
		Decimal			=> "decimal",
		Target			=> "target",
		VersionAt		=> "at",
		BytesLen		=> "len",
		BytesAt			=> "at",
		BytesSlice		=> "slice",
		IntSignum		=> "signum",
		IntBitNot		=> "bit-not",
		IntBitAnd		=> "bit-and",
		IntBitOr		=> "bit-or",
		IntBitXor		=> "bit-xor",
		IntBitLshift	=> "bit-lshift",
		IntBitRshift	=> "bit-rshift",
		IntToBytes		=> "to-bytes",
		IntFromBytes	=> "from-bytes",
		FloatIsNan		=> "is-nan",
		FloatIsInfinite	=> "is-infinite",
		FloatSignum		=> "signum",
		FloatToBytes	=> "to-bytes",
		FloatFromBytes	=> "from-bytes",
	}
}

// The Typst version this library follows, for `sys.version`.
const TYPST_VERSION: [u32; 3] = [0, 15, 1];

pub fn define(scope: &mut Scope) {
	for ty in [
		Type::Bool, Type::Int, Type::Float, Type::Str, Type::Label, Type::Bytes, Type::Content,
		Type::Array, Type::Dict, Type::Func, Type::Args, Type::Type, Type::Module, Type::Regex,
		Type::Selector, Type::Datetime, Type::Symbol, Type::Duration, Type::Version, Type::Length,
		Type::Angle, Type::Ratio, Type::Relative, Type::Fraction, Type::Direction, Type::Alignment,
		Type::Color, Type::Gradient, Type::Tiling, Type::Stroke, Type::Location, Type::Counter,
		Type::State,
	] {
		scope.define(ty.name(), Value::Type(ty));
	}
	for f in [FoundFn::Repr, FoundFn::Panic, FoundFn::Assert, FoundFn::Eval, FoundFn::Plugin,
		FoundFn::Target, FoundFn::Range]
	{
		scope.define(f.name(), native(f));
	}
	scope.define("sys", Value::Module(Arc::new(sys_module())));
}

fn native(f: FoundFn) -> Value { Value::Func(Func::Native(NativeFunc::Found(f))) }

fn sys_module() -> Module {
	let mut s = Scope::new();
	s.define("version", Value::Version(Arc::new(TYPST_VERSION.to_vec())));
	s.define("inputs", Value::dict(Dict::new()));
	Module::new("sys", s)
}

/// What calling a type value runs: `int("3")`, `datetime(..)`, `counter(heading)`. `None` for a type
/// Typst gives no constructor ("type content does not have a constructor").
pub fn constructor(ty: Type) -> Option<NativeFunc> {
	use crate::eval::lib::intro::IntroFn;
	use crate::eval::lib::visual::VisualFn;
	use crate::eval::select::StyleFn;
	match ty {
		Type::Int		=> Some(NativeFunc::Found(FoundFn::Int)),
		Type::Float		=> Some(NativeFunc::Found(FoundFn::Float)),
		Type::Str		=> Some(NativeFunc::Found(FoundFn::Str)),
		Type::Label		=> Some(NativeFunc::Found(FoundFn::Label)),
		Type::Regex		=> Some(NativeFunc::Found(FoundFn::Regex)),
		Type::Version	=> Some(NativeFunc::Found(FoundFn::Version)),
		Type::Bytes		=> Some(NativeFunc::Found(FoundFn::Bytes)),
		Type::Args		=> Some(NativeFunc::Found(FoundFn::Arguments)),
		Type::Type		=> Some(NativeFunc::Found(FoundFn::Type)),
		Type::Array		=> Some(NativeFunc::Array(array::ArrayFn::Construct)),
		Type::Dict		=> Some(NativeFunc::Dict(dict::DictFn::Construct)),
		Type::Datetime	=> Some(NativeFunc::Datetime(datetime::DatetimeFn::Datetime)),
		Type::Duration	=> Some(NativeFunc::Datetime(datetime::DatetimeFn::Duration)),
		Type::Symbol	=> Some(NativeFunc::Sym(sym::SymFn::Symbol)),
		Type::Stroke	=> Some(NativeFunc::Visual(VisualFn::Stroke)),
		Type::Tiling	=> Some(NativeFunc::Visual(VisualFn::Tiling)),
		Type::Selector	=> Some(NativeFunc::Style(StyleFn::Selector)),
		Type::Counter	=> Some(NativeFunc::Intro(IntroFn::Counter)),
		Type::State		=> Some(NativeFunc::Intro(IntroFn::State)),
		_				=> None,
	}
}

/// The method `name` on a receiver of type `ty`: the one table the evaluator's method call and a type's
/// unbound methods (`str.len("ab")`) both read.
pub fn method(ty: Type, name: &str) -> Option<NativeFunc> {
	match ty {
		Type::Str		=> string::method(name).map(NativeFunc::Str),
		Type::Array		=> array::method(name).map(NativeFunc::Array),
		Type::Dict		=> dict::method(name).map(NativeFunc::Dict),
		Type::Color		=> color::method(name).map(NativeFunc::Color),
		Type::Gradient	=> color::gradient_method(name).map(NativeFunc::Color),
		Type::Datetime	=> datetime::method(name).map(NativeFunc::Datetime),
		Type::Duration	=> datetime::duration_method(name).map(NativeFunc::Datetime),
		Type::Symbol	=> sym::method(name).map(NativeFunc::Sym),
		Type::Length | Type::Angle | Type::Ratio | Type::Relative | Type::Fraction
			| Type::Alignment | Type::Direction => geom::method(ty, name).map(NativeFunc::Geom),
		Type::Selector	=> crate::eval::select::method(name).map(NativeFunc::Style),
		Type::Counter | Type::State | Type::Location
						=> lib::intro::method(name).map(NativeFunc::Intro),
		Type::Int | Type::Float | Type::Version | Type::Bytes
						=> found_method(ty, name).map(NativeFunc::Found),
		_				=> None,
	}
}

fn found_method(ty: Type, name: &str) -> Option<FoundFn> {
	let f = match (ty, name) {
		(Type::Int, "signum")			=> FoundFn::IntSignum,
		(Type::Int, "bit-not")			=> FoundFn::IntBitNot,
		(Type::Int, "bit-and")			=> FoundFn::IntBitAnd,
		(Type::Int, "bit-or")			=> FoundFn::IntBitOr,
		(Type::Int, "bit-xor")			=> FoundFn::IntBitXor,
		(Type::Int, "bit-lshift")		=> FoundFn::IntBitLshift,
		(Type::Int, "bit-rshift")		=> FoundFn::IntBitRshift,
		(Type::Int, "to-bytes")			=> FoundFn::IntToBytes,
		(Type::Float, "is-nan")			=> FoundFn::FloatIsNan,
		(Type::Float, "is-infinite")	=> FoundFn::FloatIsInfinite,
		(Type::Float, "signum")			=> FoundFn::FloatSignum,
		(Type::Float, "to-bytes")		=> FoundFn::FloatToBytes,
		(Type::Version, "at")			=> FoundFn::VersionAt,
		(Type::Bytes, "len")			=> FoundFn::BytesLen,
		(Type::Bytes, "at")				=> FoundFn::BytesAt,
		(Type::Bytes, "slice")			=> FoundFn::BytesSlice,
		_								=> return None,
	};
	Some(f)
}

/// `ty.name`: a type's static members (`float.inf`, `str.from-unicode`, `color.hsl`, `datetime.today`),
/// then its methods, which Typst also exposes unbound.
pub fn type_scope(ty: Type, name: &str) -> Option<Value> {
	let stat = match (ty, name) {
		(Type::Float, "inf")				=> Some(Value::Float(f64::INFINITY)),
		(Type::Float, "nan")				=> Some(Value::Float(f64::NAN)),
		(Type::Float, "from-bytes")			=> Some(native(FoundFn::FloatFromBytes)),
		(Type::Int, "from-bytes")			=> Some(native(FoundFn::IntFromBytes)),
		(Type::Array, "range")				=> Some(native(FoundFn::Range)),
		(Type::Str, _)						=> string::static_fn(name)
			.map(|f| Value::Func(Func::Native(NativeFunc::Str(f)))),
		(Type::Color, _)					=> color::color_static(name),
		(Type::Gradient, _)					=> color::gradient_static(name),
		(Type::Datetime, "today")			=> Some(Value::Func(Func::Native(
			NativeFunc::Datetime(datetime::DatetimeFn::Today)))),
		_									=> None,
	};
	match stat {
		Some(v)	=> Some(v),
		None	=> method(ty, name).map(|f| Value::Func(Func::Native(f))),
	}
}

/// `f.name` for native functions that carry members: `assert.eq`, `json.encode`.
pub fn func_scope(f: &Func, name: &str) -> Option<Value> {
	match f {
		Func::Native(NativeFunc::Found(FoundFn::Assert)) => match name {
			"eq"	=> Some(native(FoundFn::AssertEq)),
			"ne"	=> Some(native(FoundFn::AssertNe)),
			_		=> None,
		},
		Func::Native(NativeFunc::Data(d)) => crate::eval::lib::data::func_scope(*d, name),
		_ => None,
	}
}

/// `value.name` for the value types whose fields Typst exposes. `None` when the type has no such field.
pub fn field(v: &Value, name: &str) -> Option<Value> {
	match (v, name) {
		(Value::Length(l), "abs")			=> Some(Value::Length(Length::pt(l.abs))),
		(Value::Length(l), "em")			=> Some(Value::Float(l.em)),
		(Value::Relative(r), "ratio")		=> Some(Value::Ratio(r.rel)),
		(Value::Relative(r), "length")		=> Some(Value::Length(r.abs)),
		(Value::Alignment(a), "x")			=> Some(match a.x {
			Some(x)	=> Value::Alignment(Alignment { x: Some(x), y: None }),
			None	=> Value::None,
		}),
		(Value::Alignment(a), "y")			=> Some(match a.y {
			Some(y)	=> Value::Alignment(Alignment { x: None, y: Some(y) }),
			None	=> Value::None,
		}),
		// A component the version was not given is "unknown version component", not zero.
		(Value::Version(v), "major")		=> v.first().map(|x| Value::Int(*x as i64)),
		(Value::Version(v), "minor")		=> v.get(1).map(|x| Value::Int(*x as i64)),
		(Value::Version(v), "patch")		=> v.get(2).map(|x| Value::Int(*x as i64)),
		(Value::Stroke(s), _)				=> stroke_field(s, name),
		_									=> None,
	}
}

fn stroke_field(s: &Stroke, name: &str) -> Option<Value> {
	let v = match name {
		"paint"			=> match &s.paint {
			Some(p)	=> paint_value(p),
			None	=> Value::Auto,
		},
		"thickness"		=> s.thickness.map(Value::Length).unwrap_or(Value::Auto),
		"cap"			=> s.cap.map(|c| Value::str(cap_name(c))).unwrap_or(Value::Auto),
		"join"			=> s.join.map(|j| Value::str(join_name(j))).unwrap_or(Value::Auto),
		"dash"			=> match &s.dash {
			None			=> Value::Auto,
			Some(None)		=> Value::None,
			Some(Some(d))	=> dash_value(d),
		},
		"miter-limit"	=> s.miter_limit.map(Value::Float).unwrap_or(Value::Auto),
		_				=> return None,
	};
	Some(v)
}

pub fn paint_value(p: &Paint) -> Value {
	match p {
		Paint::Color(c)		=> Value::Color(*c),
		Paint::Gradient(g)	=> Value::Gradient(g.clone()),
		Paint::Tiling(t)	=> Value::Tiling(t.clone()),
	}
}

fn dash_value(d: &Dash) -> Value {
	let arr = d.array.iter().map(|i| match i {
		DashItem::Len(l)	=> Value::Length(*l),
		DashItem::Dot		=> Value::str("dot"),
	}).collect();
	let mut dict = Dict::new();
	dict.insert("array", Value::array(arr));
	dict.insert("phase", Value::Length(d.phase));
	Value::dict(dict)
}

fn cap_name(c: LineCap) -> &'static str {
	match c {
		LineCap::Butt	=> "butt",
		LineCap::Round	=> "round",
		LineCap::Square	=> "square",
	}
}

fn join_name(j: LineJoin) -> &'static str {
	match j {
		LineJoin::Miter	=> "miter",
		LineJoin::Round	=> "round",
		LineJoin::Bevel	=> "bevel",
	}
}

// Argument helpers shared by every U3 area.

/// The receiver of a method call: the first positional argument.
pub fn receiver(args: &mut Args) -> Outcome<Value> {
	args.expect::<Value>("self")
}

/// The next positional argument, required, with Typst's message when absent.
pub fn need(engine: &mut Engine, args: &mut Args, what: &str) -> Outcome<Value> {
	match res!(args.eat::<Value>()) {
		Some(v)	=> Ok(v),
		None	=> Err(engine.error(args.span, fmt!("missing argument: {}", what))),
	}
}

/// "expected X, found Y", as a spanned diagnostic.
pub fn mismatch(engine: &mut Engine, span: Span, expected: &str, found: &Value) -> Error<ErrTag> {
	engine.error(span, fmt!("expected {}, found {}", expected, type_desc(found.ty())))
}

/// A type as Typst names it in "expected ..., found ..." messages.
pub fn type_desc(ty: Type) -> &'static str {
	match ty {
		Type::None		=> "none",
		Type::Auto		=> "auto",
		Type::Bool		=> "boolean",
		Type::Int		=> "integer",
		Type::Float		=> "float",
		Type::Str		=> "string",
		Type::Dict		=> "dictionary",
		Type::Func		=> "function",
		Type::Args		=> "arguments",
		other			=> other.name(),
	}
}

/// Rejects arguments left over after a native took what it reads.
pub fn finish(engine: &mut Engine, args: Args) -> Outcome<()> {
	let span = args.span;
	match args.items.first() {
		None	=> Ok(()),
		Some(a)	=> {
			let msg = match &a.name {
				Some(n)	=> fmt!("unexpected argument: {}", n),
				None	=> "unexpected argument".to_string(),
			};
			let s = if a.span.is_detached() { span } else { a.span };
			Err(engine.error(s, msg))
		}
	}
}

/// An integer argument (floats are refused, as Typst does for `int` parameters).
pub fn int_of(engine: &mut Engine, span: Span, v: Value) -> Outcome<i64> {
	match v {
		Value::Int(i)	=> Ok(i),
		other			=> Err(mismatch(engine, span, "integer", &other)),
	}
}

/// A number argument: int or float, as f64.
pub fn num_of(engine: &mut Engine, span: Span, v: Value) -> Outcome<f64> {
	match v {
		Value::Int(i)	=> Ok(i as f64),
		Value::Float(f)	=> Ok(f),
		other			=> Err(mismatch(engine, span, "integer or float", &other)),
	}
}

pub fn named_bool(engine: &mut Engine, args: &mut Args, name: &str, default: bool) -> Outcome<bool> {
	match res!(args.named::<Value>(name)) {
		None				=> Ok(default),
		Some(Value::Bool(b))	=> Ok(b),
		Some(other)			=> Err(mismatch(engine, args.span, "boolean", &other)),
	}
}

pub fn str_of(engine: &mut Engine, span: Span, v: Value) -> Outcome<Arc<String>> {
	match v {
		Value::Str(s)	=> Ok(s),
		other			=> Err(mismatch(engine, span, "string", &other)),
	}
}

pub fn call(f: FoundFn, engine: &mut Engine, mut args: Args) -> Outcome<Value> {
	let span = args.span;
	let v = match f {
		FoundFn::Type => {
			let v = res!(need(engine, &mut args, "value"));
			res!(finish(engine, args));
			Value::Type(v.ty())
		}
		FoundFn::Repr => {
			let v = res!(need(engine, &mut args, "value"));
			res!(finish(engine, args));
			Value::str(repr(&v))
		}
		FoundFn::Assert => {
			let cond = res!(need(engine, &mut args, "condition"));
			let msg = res!(args.named::<Value>("message"));
			res!(finish(engine, args));
			match cond {
				Value::Bool(true)	=> Value::None,
				Value::Bool(false)	=> return Err(assert_fail(engine, span, msg, "assertion failed".to_string())),
				other				=> return Err(mismatch(engine, span, "boolean", &other)),
			}
		}
		FoundFn::AssertEq | FoundFn::AssertNe => {
			let a = res!(need(engine, &mut args, "left"));
			let b = res!(need(engine, &mut args, "right"));
			let msg = res!(args.named::<Value>("message"));
			res!(finish(engine, args));
			let eq = crate::eval::ops::equal(&a, &b);
			let want = f == FoundFn::AssertEq;
			if eq != want {
				let text = if want {
					fmt!("equality assertion failed: value {} was not equal to {}", repr(&a), repr(&b))
				} else {
					fmt!("inequality assertion failed: value {} was equal to {}", repr(&a), repr(&b))
				};
				return Err(assert_fail(engine, span, msg, text));
			}
			Value::None
		}
		FoundFn::Panic => {
			let vals = res!(args.all::<Value>());
			let mut msg = String::from("panicked");
			if !vals.is_empty() {
				msg.push_str(" with: ");
				// A string is shown as its text, anything else by its repr.
				let parts: Vec<String> = vals.iter().map(|v| match v {
					Value::Str(s)	=> (**s).clone(),
					other			=> repr(other),
				}).collect();
				msg.push_str(&parts.join(", "));
			}
			return Err(engine.error(span, msg));
		}
		FoundFn::Eval => {
			let text = res!(need(engine, &mut args, "source"));
			let text = res!(str_of(engine, span, text));
			let mode = match res!(args.named::<Value>("mode")) {
				None => EvalMode::Code,
				Some(Value::Str(s)) => match s.as_str() {
					"code"		=> EvalMode::Code,
					"markup"	=> EvalMode::Markup,
					"math"		=> EvalMode::Math,
					_			=> return Err(engine.error(span,
						"expected \"markup\", \"math\", or \"code\"")),
				},
				Some(other) => return Err(mismatch(engine, span, "string", &other)),
			};
			let mut scope = Scope::new();
			match res!(args.named::<Value>("scope")) {
				None				=> (),
				Some(Value::Dict(d))	=> for (k, v) in d.iter() { scope.define(k, v.clone()); },
				Some(other)			=> return Err(mismatch(engine, span, "dictionary", &other)),
			}
			res!(finish(engine, args));
			return eval_string(engine, &text, mode, scope, span);
		}
		FoundFn::Range => return range(engine, args),
		FoundFn::Int => {
			let v = res!(need(engine, &mut args, "value"));
			res!(finish(engine, args));
			res!(to_int(engine, span, v))
		}
		FoundFn::Float => {
			let v = res!(need(engine, &mut args, "value"));
			res!(finish(engine, args));
			res!(to_float(engine, span, v))
		}
		FoundFn::Str => {
			let v = res!(need(engine, &mut args, "value"));
			let base = res!(args.named::<Value>("base"));
			res!(finish(engine, args));
			res!(to_str(engine, span, v, base))
		}
		FoundFn::Bool => return Err(engine.error(span, "type boolean does not have a constructor")),
		FoundFn::Label => {
			let v = res!(need(engine, &mut args, "name"));
			res!(finish(engine, args));
			let s = res!(str_of(engine, span, v));
			if s.is_empty() {
				return Err(engine.error(span, "label name must not be empty"));
			}
			Value::Label(Label::new(&s))
		}
		FoundFn::Regex => {
			let v = res!(need(engine, &mut args, "regex"));
			res!(finish(engine, args));
			let s = res!(str_of(engine, span, v));
			match RegexValue::new(&s) {
				Ok(r)	=> Value::Regex(Arc::new(r)),
				Err(e)	=> return Err(engine.error(span, fmt!("invalid regular expression: {}",
					e.msgs().last().cloned().unwrap_or_default()))),
			}
		}
		FoundFn::Version => {
			let parts = res!(args.all::<Value>());
			res!(finish(engine, args));
			let mut out = Vec::new();
			for p in parts {
				res!(version_part(engine, span, p, &mut out));
			}
			Value::Version(Arc::new(out))
		}
		FoundFn::Bytes => {
			let v = res!(need(engine, &mut args, "value"));
			res!(finish(engine, args));
			match v {
				Value::Str(s)	=> Value::Bytes(Arc::new(s.as_bytes().to_vec())),
				Value::Bytes(b)	=> Value::Bytes(b),
				Value::Array(a)	=> {
					let mut out = Vec::with_capacity(a.len());
					for x in a.iter() {
						match x {
							Value::Int(i) if (0..=255).contains(i) => out.push(*i as u8),
							Value::Int(_) => return Err(engine.error(span, "number must be between 0 and 255")),
							other => return Err(mismatch(engine, span, "integer", other)),
						}
					}
					Value::Bytes(Arc::new(out))
				}
				other => return Err(mismatch(engine, span, "string, array, or bytes", &other)),
			}
		}
		FoundFn::Arguments => {
			let mut a = args.clone();
			a.span = span;
			Value::Args(Arc::new(a))
		}
		FoundFn::Plugin => return Err(engine.error_hint(span,
			"wasm plugins are not supported by Austenite",
			"a package that needs a plugin cannot be compiled here")),
		FoundFn::Decimal => return Err(engine.error_hint(span,
			"the decimal type is not supported by Austenite yet",
			"use a float, or an integer scaled to the precision you need")),
		FoundFn::Target => {
			res!(finish(engine, args));
			Value::str("paged")
		}
		FoundFn::VersionAt => {
			let v = res!(receiver(&mut args));
			let i = res!(need(engine, &mut args, "index"));
			let i = res!(int_of(engine, span, i));
			res!(finish(engine, args));
			let parts = match v {
				Value::Version(p)	=> p,
				other				=> return Err(mismatch(engine, span, "version", &other)),
			};
			let n = parts.len() as i64;
			let idx = if i < 0 { n + i } else { i };
			if idx < 0 {
				return Err(engine.error(span, fmt!("component index out of bounds (index: {}, len: {})", i, n)));
			}
			Value::Int(parts.get(idx as usize).copied().unwrap_or(0) as i64)
		}
		FoundFn::BytesLen => {
			let b = res!(bytes_recv(engine, &mut args));
			res!(finish(engine, args));
			Value::Int(b.len() as i64)
		}
		FoundFn::BytesAt => {
			let b = res!(bytes_recv(engine, &mut args));
			let i = res!(need(engine, &mut args, "index"));
			let i = res!(int_of(engine, span, i));
			let default = res!(args.named::<Value>("default"));
			res!(finish(engine, args));
			match locate_index(i, b.len()) {
				Some(k)	=> Value::Int(b[k] as i64),
				None	=> match default {
					Some(d)	=> d,
					None	=> return Err(engine.error(span, fmt!(
						"byte index out of bounds (index: {}, len: {}) and no default value was specified",
						i, b.len()))),
				},
			}
		}
		FoundFn::BytesSlice => {
			let b = res!(bytes_recv(engine, &mut args));
			let (s, e) = res!(slice_bounds(engine, &mut args, b.len(), "byte", |_| true));
			res!(finish(engine, args));
			Value::Bytes(Arc::new(b[s..e].to_vec()))
		}
		FoundFn::IntSignum => {
			let i = res!(int_recv(engine, &mut args));
			res!(finish(engine, args));
			Value::Int(i.signum())
		}
		FoundFn::IntBitNot => {
			let i = res!(int_recv(engine, &mut args));
			res!(finish(engine, args));
			Value::Int(!i)
		}
		FoundFn::IntBitAnd | FoundFn::IntBitOr | FoundFn::IntBitXor => {
			let i = res!(int_recv(engine, &mut args));
			let j = res!(need(engine, &mut args, "rhs"));
			let j = res!(int_of(engine, span, j));
			res!(finish(engine, args));
			Value::Int(match f {
				FoundFn::IntBitAnd	=> i & j,
				FoundFn::IntBitOr	=> i | j,
				_					=> i ^ j,
			})
		}
		FoundFn::IntBitLshift => {
			let i = res!(int_recv(engine, &mut args));
			let n = res!(need(engine, &mut args, "shift"));
			let n = res!(int_of(engine, span, n));
			res!(finish(engine, args));
			if n < 0 {
				return Err(engine.error(span, "number must be at least zero"));
			}
			match i.checked_shl(n as u32) {
				Some(r) if n < 64	=> Value::Int(r),
				_					=> return Err(engine.error(span, "the result is too large")),
			}
		}
		FoundFn::IntBitRshift => {
			let i = res!(int_recv(engine, &mut args));
			let n = res!(need(engine, &mut args, "shift"));
			let n = res!(int_of(engine, span, n));
			let logical = res!(named_bool(engine, &mut args, "logical", false));
			res!(finish(engine, args));
			if n < 0 {
				return Err(engine.error(span, "number must be at least zero"));
			}
			if logical {
				if n >= 64 { Value::Int(0) } else { Value::Int(((i as u64) >> n) as i64) }
			} else if n >= 64 {
				Value::Int(if i < 0 { -1 } else { 0 })
			} else {
				Value::Int(i >> n)
			}
		}
		FoundFn::IntToBytes => {
			let i = res!(int_recv(engine, &mut args));
			let big = res!(endian(engine, &mut args));
			let size = match res!(args.named::<Value>("size")) {
				None	=> 8,
				Some(v)	=> res!(int_of(engine, span, v)),
			};
			res!(finish(engine, args));
			if size < 0 {
				return Err(engine.error(span, "number must be at least zero"));
			}
			let size = size as usize;
			let le = i.to_le_bytes();
			let fill = if i < 0 { 0xffu8 } else { 0 };
			let mut out: Vec<u8> = (0..size).map(|k| le.get(k).copied().unwrap_or(fill)).collect();
			if big {
				out.reverse();
			}
			Value::Bytes(Arc::new(out))
		}
		FoundFn::IntFromBytes => {
			let b = res!(need(engine, &mut args, "bytes"));
			let b = match b {
				Value::Bytes(b)	=> b,
				other			=> return Err(mismatch(engine, span, "bytes", &other)),
			};
			let big = res!(endian(engine, &mut args));
			let signed = res!(named_bool(engine, &mut args, "signed", true));
			res!(finish(engine, args));
			if b.len() > 8 {
				return Err(engine.error(span, "too many bytes to convert to a 64 bit number"));
			}
			let mut le: Vec<u8> = b.to_vec();
			if big {
				le.reverse();
			}
			let neg = signed && le.last().map(|x| x & 0x80 != 0).unwrap_or(false);
			let mut buf = [if neg { 0xffu8 } else { 0 }; 8];
			for (k, x) in le.iter().enumerate() {
				buf[k] = *x;
			}
			Value::Int(i64::from_le_bytes(buf))
		}
		FoundFn::FloatIsNan => {
			let x = res!(float_recv(engine, &mut args));
			res!(finish(engine, args));
			Value::Bool(x.is_nan())
		}
		FoundFn::FloatIsInfinite => {
			let x = res!(float_recv(engine, &mut args));
			res!(finish(engine, args));
			Value::Bool(x.is_infinite())
		}
		FoundFn::FloatSignum => {
			let x = res!(float_recv(engine, &mut args));
			res!(finish(engine, args));
			Value::Float(x.signum())
		}
		FoundFn::FloatToBytes => {
			let x = res!(float_recv(engine, &mut args));
			let big = res!(endian(engine, &mut args));
			let size = match res!(args.named::<Value>("size")) {
				None	=> 8,
				Some(v)	=> res!(int_of(engine, span, v)),
			};
			res!(finish(engine, args));
			let mut out = match size {
				8	=> x.to_le_bytes().to_vec(),
				4	=> (x as f32).to_le_bytes().to_vec(),
				_	=> return Err(engine.error(span, "size must be either 4 or 8")),
			};
			if big {
				out.reverse();
			}
			Value::Bytes(Arc::new(out))
		}
		FoundFn::FloatFromBytes => {
			let b = res!(need(engine, &mut args, "bytes"));
			let b = match b {
				Value::Bytes(b)	=> b,
				other			=> return Err(mismatch(engine, span, "bytes", &other)),
			};
			let big = res!(endian(engine, &mut args));
			res!(finish(engine, args));
			let mut le: Vec<u8> = b.to_vec();
			if big {
				le.reverse();
			}
			match le.len() {
				8 => {
					let mut a = [0u8; 8];
					a.copy_from_slice(&le);
					Value::Float(f64::from_le_bytes(a))
				}
				4 => {
					let mut a = [0u8; 4];
					a.copy_from_slice(&le);
					Value::Float(f32::from_le_bytes(a) as f64)
				}
				_ => return Err(engine.error(span, "bytes must have a length of 4 or 8")),
			}
		}
	};
	Ok(v)
}

fn assert_fail(engine: &mut Engine, span: Span, msg: Option<Value>, dflt: String) -> Error<ErrTag> {
	match msg {
		Some(Value::Str(s))	=> engine.error(span, fmt!("assertion failed: {}", s)),
		_					=> engine.error(span, dflt),
	}
}

fn version_part(engine: &mut Engine, span: Span, v: Value, out: &mut Vec<u32>) -> Outcome<()> {
	match v {
		Value::Int(i) if i >= 0 && i <= u32::MAX as i64	=> out.push(i as u32),
		Value::Int(i) if i < 0	=> return Err(engine.error(span, "number must be at least zero")),
		Value::Int(_)			=> return Err(engine.error(span, "number too large")),
		Value::Array(a)			=> for x in a.iter() {
			res!(version_part(engine, span, x.clone(), out));
		},
		Value::Version(p)		=> out.extend(p.iter()),
		other					=> return Err(mismatch(engine, span, "integer or array", &other)),
	}
	Ok(())
}

fn endian(engine: &mut Engine, args: &mut Args) -> Outcome<bool> {
	match res!(args.named::<Value>("endian")) {
		None => Ok(false),
		Some(Value::Str(s)) => match s.as_str() {
			"big"		=> Ok(true),
			"little"	=> Ok(false),
			_			=> Err(engine.error(args.span, "expected \"big\" or \"little\"")),
		},
		Some(other) => Err(mismatch(engine, args.span, "string", &other)),
	}
}

fn int_recv(engine: &mut Engine, args: &mut Args) -> Outcome<i64> {
	let v = res!(receiver(args));
	int_of(engine, args.span, v)
}

fn float_recv(engine: &mut Engine, args: &mut Args) -> Outcome<f64> {
	match res!(receiver(args)) {
		Value::Float(f)	=> Ok(f),
		other			=> Err(mismatch(engine, args.span, "float", &other)),
	}
}

fn bytes_recv(engine: &mut Engine, args: &mut Args) -> Outcome<Arc<Vec<u8>>> {
	match res!(receiver(args)) {
		Value::Bytes(b)	=> Ok(b),
		other			=> Err(mismatch(engine, args.span, "bytes", &other)),
	}
}

/// A possibly negative index into a sequence of `len`, as Typst resolves it; `None` when out of range.
pub fn locate_index(i: i64, len: usize) -> Option<usize> {
	let n = len as i64;
	let k = if i < 0 { n.saturating_add(i) } else { i };
	if k >= 0 && k < n { Some(k as usize) } else { None }
}

/// An error's innermost words, without locations: for a diagnostic raised from another unit's error.
pub fn words(e: &Error<ErrTag>) -> String { e.msgs().last().cloned().unwrap_or_default() }

/// Resolves a possibly negative index against `len` as Typst's `locate` does: `Ok(None)` when out of
/// range, an error when it lands inside a character (strings only, via `boundary`).
pub fn locate_bound<B: Fn(usize) -> bool>(
	engine:		&mut Engine,
	span:		Span,
	i:			i64,
	len:		usize,
	boundary:	&B,
)
	-> Outcome<Option<usize>>
{
	let wrapped = if i >= 0 { Some(i) } else { (len as i64).checked_add(i) };
	let resolved = wrapped.filter(|v| *v >= 0 && *v as usize <= len).map(|v| v as usize);
	if let Some(k) = resolved {
		if !boundary(k) {
			return Err(engine.error(span, fmt!("string index {} is not a character boundary", i)));
		}
	}
	Ok(resolved)
}

/// `slice(start, end, count:)` over a sequence of `len`, as Typst resolves it: `count` counts from the
/// start as written, and an end before the start gives an empty slice. `what` names the sequence in the
/// error ("byte", "string", "array").
pub fn slice_bounds<B: Fn(usize) -> bool>(
	engine:		&mut Engine,
	args:		&mut Args,
	len:		usize,
	what:		&str,
	boundary:	B,
)
	-> Outcome<(usize, usize)>
{
	let span = args.span;
	let start = match res!(args.eat::<Value>()) {
		None	=> 0,
		Some(v)	=> res!(int_of(engine, span, v)),
	};
	let end = match res!(args.eat::<Value>()) {
		None | Some(Value::None)	=> None,
		Some(v)						=> Some(res!(int_of(engine, span, v))),
	};
	let count = match res!(args.named::<Value>("count")) {
		None | Some(Value::None)	=> None,
		Some(v)						=> Some(res!(int_of(engine, span, v))),
	};
	let end = end.or(count.map(|c| start.saturating_add(c))).unwrap_or(len as i64);
	let s = match res!(locate_bound(engine, span, start, len, &boundary)) {
		Some(k)	=> k,
		None	=> return Err(engine.error(span, fmt!("{} index out of bounds (index: {}, len: {})", what, start, len))),
	};
	let e = match res!(locate_bound(engine, span, end, len, &boundary)) {
		Some(k)	=> k.max(s),
		None	=> return Err(engine.error(span, fmt!("{} index out of bounds (index: {}, len: {})", what, end, len))),
	};
	Ok((s, e))
}

fn range(engine: &mut Engine, mut args: Args) -> Outcome<Value> {
	let span = args.span;
	let a = res!(need(engine, &mut args, "end"));
	let a = res!(int_of(engine, span, a));
	let (start, end) = match res!(args.eat::<Value>()) {
		Some(b)	=> (a, res!(int_of(engine, span, b))),
		None	=> (0, a),
	};
	let step = match res!(args.named::<Value>("step")) {
		None	=> 1,
		Some(v)	=> res!(int_of(engine, span, v)),
	};
	res!(finish(engine, args));
	if step == 0 {
		return Err(engine.error(span, "number must not be zero"));
	}
	let mut out = Vec::new();
	let mut x = start;
	while (step > 0 && x < end) || (step < 0 && x > end) {
		res!(engine.burn(span));
		out.push(Value::Int(x));
		x = match x.checked_add(step) {
			Some(n)	=> n,
			None	=> break,
		};
	}
	Ok(Value::array(out))
}

// Conversions

fn to_int(engine: &mut Engine, span: Span, v: Value) -> Outcome<Value> {
	match v {
		Value::Int(i)	=> Ok(Value::Int(i)),
		Value::Bool(b)	=> Ok(Value::Int(b as i64)),
		Value::Float(f)	=> {
			let t = f.trunc();
			if t.is_nan() || t < -9.223372036854776e18 || t >= 9.223372036854776e18 {
				return Err(engine.error(span, "number too large"));
			}
			Ok(Value::Int(t as i64))
		}
		Value::Str(s)	=> {
			let t = s.replace('\u{2212}', "-");
			let (neg, digits) = match t.strip_prefix('-') {
				Some(r)	=> (true, r),
				None	=> (false, t.strip_prefix('+').unwrap_or(&t)),
			};
			if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
				return Err(engine.error(span, "string contains invalid digits"));
			}
			let mut acc: i64 = 0;
			for b in digits.bytes() {
				let d = (b - b'0') as i64;
				let next = acc.checked_mul(10).and_then(|x| if neg { x.checked_sub(d) } else { x.checked_add(d) });
				acc = match next {
					Some(n)	=> n,
					None	=> {
						let msg = if neg { "integer value is too small" } else { "integer value is too large" };
						return Err(engine.error_hint(span, msg,
							"value does not fit into a signed 64-bit integer"));
					}
				};
			}
			Ok(Value::Int(acc))
		}
		other => Err(mismatch(engine, span, "integer, boolean, float, decimal, or string", &other)),
	}
}

fn to_float(engine: &mut Engine, span: Span, v: Value) -> Outcome<Value> {
	match v {
		Value::Float(f)	=> Ok(Value::Float(f)),
		Value::Int(i)	=> Ok(Value::Float(i as f64)),
		Value::Bool(b)	=> Ok(Value::Float(if b { 1.0 } else { 0.0 })),
		Value::Ratio(r)	=> Ok(Value::Float(r.0)),
		Value::Str(s)	=> {
			let t = s.replace('\u{2212}', "-");
			match parse_float(&t) {
				Some(f)	=> Ok(Value::Float(f)),
				None	=> Err(engine.error(span, fmt!("invalid float: {}", s))),
			}
		}
		other => Err(mismatch(engine, span, "integer, boolean, float, decimal, ratio, or string", &other)),
	}
}

/// Rust's float grammar, which Typst's `float(str)` uses: `inf`, `nan`, `1.`, `.5`, `1e3`.
fn parse_float(s: &str) -> Option<f64> {
	let body = s.strip_prefix(['+', '-']).unwrap_or(s);
	let lower = body.to_ascii_lowercase();
	if !(lower == "inf" || lower == "infinity" || lower == "nan") {
		let ok = !body.is_empty() && body.bytes().all(|b| b.is_ascii_digit() || matches!(b, b'.' | b'e' | b'E' | b'+' | b'-'));
		if !ok {
			return None;
		}
	}
	s.parse::<f64>().ok()
}

fn to_str(engine: &mut Engine, span: Span, v: Value, base: Option<Value>) -> Outcome<Value> {
	let base = match base {
		None	=> None,
		Some(b)	=> Some(res!(int_of(engine, span, b))),
	};
	if let Some(b) = base {
		match v {
			Value::Int(i) => {
				if !(2..=36).contains(&b) {
					return Err(engine.error(span, "base must be between 2 and 36"));
				}
				return Ok(Value::str(format_int_with_base(i, b)));
			}
			_ => if b != 10 {
				return Err(engine.error(span, "base is only supported for integers"));
			},
		}
	}
	let s = match v {
		Value::Int(i)		=> format_int_with_base(i, 10),
		Value::Float(f)		=> display_float(f),
		Value::Str(s)		=> (*s).clone(),
		Value::Label(l)		=> l.as_str().to_string(),
		Value::Type(t)		=> type_desc(t).to_string(),
		Value::Version(p)	=> version_text(&p),
		Value::Symbol(s)	=> sym::text(&s).to_string(),
		Value::Bytes(b)		=> match String::from_utf8(b.to_vec()) {
			Ok(s)	=> s,
			Err(_)	=> return Err(engine.error(span, "bytes are not valid UTF-8")),
		},
		other => return Err(mismatch(engine, span,
			"integer, float, decimal, version, bytes, label, type, or string", &other)),
	};
	Ok(Value::str(s))
}

pub const MINUS: char = '\u{2212}';

/// An integer as `str(n, base:)` prints it, a negative with a typographic minus.
pub fn format_int_with_base(n: i64, base: i64) -> String {
	if n == 0 {
		return "0".to_string();
	}
	let neg = n < 0;
	let mut m = (n as i128).unsigned_abs();
	let b = base as u128;
	let mut digits = Vec::new();
	while m > 0 {
		let d = (m % b) as u32;
		digits.push(std::char::from_digit(d, base as u32).unwrap_or('?'));
		m /= b;
	}
	let mut s = String::new();
	if neg {
		s.push(MINUS);
	}
	s.extend(digits.iter().rev());
	s
}

/// A float as it is displayed in text (`str(x)`, `[#x]`): no forced separator, a typographic minus,
/// `NaN` and `∞`.
pub fn display_float(f: f64) -> String {
	if f.is_nan() {
		return "NaN".to_string();
	}
	if f.is_infinite() {
		return if f < 0.0 { fmt!("{}∞", MINUS) } else { "∞".to_string() };
	}
	let s = fmt!("{}", f);
	match s.strip_prefix('-') {
		Some(r)	=> fmt!("{}{}", MINUS, r),
		None	=> s,
	}
}

pub fn version_text(p: &[u32]) -> String {
	p.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(".")
}

// Rounding and number text

/// Typst's `round_with_precision`: a no-op where rounding cannot change the value.
pub fn round_with_precision(value: f64, precision: i16) -> f64 {
	if value.is_infinite()
		|| value.is_nan()
		|| (precision >= 0 && value.abs() >= (1u64 << 53) as f64)
		|| precision >= 15
	{
		return value;
	}
	if precision < -308 {
		return value * 0.0;
	}
	if precision > 0 {
		let offset = 10f64.powf(precision as f64);
		(value * offset).round() / offset
	} else {
		let offset = 10f64.powf(-(precision as f64));
		(value / offset).round() * offset
	}
}

/// Typst's `format_float`: optionally rounded, with a decimal separator forced for a bare float's repr.
pub fn format_float(value: f64, precision: Option<u8>, force_separator: bool, suffix: &str) -> String {
	let v = match precision {
		Some(p)	=> round_with_precision(value, p as i16),
		None	=> value,
	};
	if v.is_nan() {
		return "NaN".to_string();
	}
	if v.is_infinite() {
		return fmt!("{}inf{}", if v < 0.0 { "-" } else { "" }, suffix);
	}
	if force_separator {
		fmt!("{:?}{}", v, suffix)
	} else {
		fmt!("{}{}", v, suffix)
	}
}

/// A unit-bearing number as `repr` prints it: two decimals at most, no forced separator.
pub fn format_unit(value: f64, unit: &str) -> String { format_float(value, Some(2), false, unit) }

pub fn repr_float(f: f64) -> String {
	if f.is_nan() {
		"float.nan".to_string()
	} else if f.is_infinite() {
		if f < 0.0 { "-float.inf".to_string() } else { "float.inf".to_string() }
	} else {
		format_float(f, None, true, "")
	}
}

// repr

/// `repr(value)`, as Typst prints it.
pub fn repr(v: &Value) -> String {
	match v {
		Value::None				=> "none".to_string(),
		Value::Auto				=> "auto".to_string(),
		Value::Bool(b)			=> b.to_string(),
		Value::Int(i)			=> i.to_string(),
		Value::Float(f)			=> repr_float(*f),
		Value::Length(l)		=> repr_length(l),
		Value::Angle(a)			=> repr_angle(*a),
		Value::Ratio(r)			=> repr_ratio(*r),
		Value::Relative(r)		=> fmt!("{} + {}", repr_ratio(r.rel), repr_length(&r.abs)),
		Value::Fraction(f)		=> repr_fraction(*f),
		Value::Color(c)			=> color::repr_color(c),
		Value::Gradient(g)		=> color::repr_gradient(g),
		Value::Tiling(t)		=> repr_tiling(t),
		Value::Stroke(s)		=> repr_stroke(s),
		Value::Alignment(a)		=> repr_alignment(a),
		Value::Direction(d)		=> direction_name(*d).to_string(),
		Value::Symbol(s)		=> sym::repr_symbol(s),
		Value::Str(s)			=> repr_str(s),
		Value::Bytes(b)			=> fmt!("bytes({})", b.len()),
		Value::Label(l)			=> repr_label(l),
		Value::Datetime(d)		=> repr_datetime(d),
		Value::Duration(d)		=> repr_duration(d),
		Value::Version(p)		=> fmt!("version{}", pretty_array_like(
			&p.iter().map(|x| x.to_string()).collect::<Vec<_>>(), false)),
		Value::Regex(r)			=> fmt!("regex({})", repr_str(&r.pattern)),
		Value::Content(c)		=> repr_content(c),
		Value::Array(a)			=> {
			let parts: Vec<String> = a.iter().map(repr).collect();
			pretty_array_like(&parts, a.len() == 1)
		}
		Value::Dict(d)			=> repr_dict(d),
		Value::Func(f)			=> repr_func(f),
		Value::Args(a)			=> {
			let parts: Vec<String> = a.items.iter().map(|arg| match &arg.name {
				Some(n)	=> fmt!("{}: {}", n, repr(&arg.value)),
				None	=> repr(&arg.value),
			}).collect();
			fmt!("arguments{}", pretty_array_like(&parts, false))
		}
		Value::Module(m)		=> fmt!("<module {}>", m.name),
		Value::Type(t)			=> match t {
			Type::None	=> "type(none)".to_string(),
			Type::Auto	=> "type(auto)".to_string(),
			other		=> other.name().to_string(),
		},
		Value::Styles(_)		=> "..".to_string(),
		Value::Selector(s)		=> repr_selector(s),
		Value::Counter(c)		=> repr_counter(c),
		Value::State(s)			=> repr_state(s),
		Value::Location(_)		=> "location(..)".to_string(),
	}
}

pub fn repr_length(l: &Length) -> String {
	match (l.abs == 0.0, l.em == 0.0) {
		(false, false)	=> fmt!("{} + {}", format_unit(l.abs, "pt"), format_unit(l.em, "em")),
		(true, false)	=> format_unit(l.em, "em"),
		(_, true)		=> format_unit(l.abs, "pt"),
	}
}

pub fn repr_angle(a: Angle) -> String { format_unit(a.0.to_degrees(), "deg") }

pub fn repr_ratio(r: Ratio) -> String { format_unit(r.0 * 100.0, "%") }

pub fn repr_fraction(f: Fraction) -> String { format_unit(f.0, "fr") }

pub fn direction_name(d: Direction) -> &'static str {
	match d {
		Direction::Ltr	=> "ltr",
		Direction::Rtl	=> "rtl",
		Direction::Ttb	=> "ttb",
		Direction::Btt	=> "btt",
	}
}

pub fn halign_name(h: HAlign) -> &'static str {
	match h {
		HAlign::Start	=> "start",
		HAlign::Left	=> "left",
		HAlign::Center	=> "center",
		HAlign::Right	=> "right",
		HAlign::End		=> "end",
	}
}

pub fn valign_name(v: VAlign) -> &'static str {
	match v {
		VAlign::Top		=> "top",
		VAlign::Horizon	=> "horizon",
		VAlign::Bottom	=> "bottom",
	}
}

fn repr_alignment(a: &Alignment) -> String {
	match (a.x, a.y) {
		(Some(x), Some(y))	=> fmt!("{} + {}", halign_name(x), valign_name(y)),
		(Some(x), None)		=> halign_name(x).to_string(),
		(None, Some(y))		=> valign_name(y).to_string(),
		(None, None)		=> "start".to_string(),
	}
}

/// A string literal as Typst's `repr` writes it.
pub fn repr_str(s: &str) -> String {
	let mut out = String::with_capacity(s.len() + 2);
	out.push('"');
	for c in s.chars() {
		match c {
			'\0'	=> out.push_str("\\u{0}"),
			'\''	=> out.push('\''),
			'"'		=> out.push_str("\\\""),
			'\\'	=> out.push_str("\\\\"),
			'\n'	=> out.push_str("\\n"),
			'\r'	=> out.push_str("\\r"),
			'\t'	=> out.push_str("\\t"),
			c if is_unprintable(c)	=> out.push_str(&fmt!("\\u{{{:x}}}", c as u32)),
			c		=> out.push(c),
		}
	}
	out.push('"');
	out
}

/// Would Rust's `escape_debug` (which Typst's `repr` uses) escape the character? Decided from the
/// Unicode 17 grapheme tables in `fe2o3_text` rather than the toolchain's own, so the answer follows
/// the Unicode version Typst is built with: controls, format characters, line and paragraph separators,
/// grapheme extenders (not emoji modifiers), non-ASCII spaces and private use. Unassigned code points
/// are printed, which Rust would escape; the tables do not say which are unassigned.
pub fn is_unprintable(c: char) -> bool {
	use oxedyne_fe2o3_text::unicode::lookup;
	use oxedyne_fe2o3_text::unicode::tables::prop::GraphemeClass;
	use oxedyne_fe2o3_text::unicode::tables::seg::{
		GCB_STARTS,
		GCB_VALS,
	};
	let u = c as u32;
	if matches!(u, 0x1F3FB..=0x1F3FF) {
		return false;
	}
	let class = lookup::get(&GCB_VALS, lookup::run(&GCB_STARTS, c), GraphemeClass::Other);
	matches!(class, GraphemeClass::Control | GraphemeClass::CR | GraphemeClass::LF
		| GraphemeClass::Extend | GraphemeClass::ZWJ)
		|| matches!(u, 0x600..=0x605 | 0x6DD | 0x70F | 0x890..=0x891 | 0x8E2 | 0x110BD | 0x110CD)
		|| matches!(u, 0xA0 | 0x1680 | 0x2000..=0x200A | 0x202F | 0x205F | 0x3000)
		|| matches!(u, 0xE000..=0xF8FF | 0xF0000..=0xFFFFD | 0x100000..=0x10FFFD)
}

/// Can `s` be written as a bare identifier (a dict key without quotes, a label in angle brackets)?
pub fn is_ident(s: &str) -> bool {
	let mut cs = s.chars();
	match cs.next() {
		Some(c) if c.is_alphabetic() || c == '_' => (),
		_ => return false,
	}
	cs.all(|c| c.is_alphanumeric() || c == '_' || c == '-' || is_mark(c))
}

// Combining marks and joiners count as identifier-continue characters, as XID_Continue has them.
fn is_mark(c: char) -> bool {
	matches!(c as u32, 0x300..=0x36F | 0x483..=0x487 | 0x591..=0x5BD | 0x610..=0x61A | 0x64B..=0x65F
		| 0x900..=0x903 | 0x93A..=0x94F | 0x1AB0..=0x1AFF | 0x1DC0..=0x1DFF | 0x200C | 0x200D
		| 0x20D0..=0x20FF | 0xFE00..=0xFE0F | 0xFE20..=0xFE2F)
}

fn is_label_char(c: char) -> bool { c.is_alphanumeric() || matches!(c, '_' | '-' | '.' | ':') || is_mark(c) }

fn repr_label(l: &Label) -> String {
	let s = l.as_str();
	if !s.is_empty() && s.chars().all(is_label_char) {
		fmt!("<{}>", s)
	} else {
		fmt!("label({})", repr_str(s))
	}
}

fn repr_dict(d: &Dict) -> String {
	if d.is_empty() {
		return "(:)".to_string();
	}
	let parts: Vec<String> = d.iter().map(|(k, v)| {
		let key = if is_ident(k) { k.to_string() } else { repr_str(k) };
		fmt!("{}: {}", key, repr(v))
	}).collect();
	pretty_array_like(&parts, false)
}

fn repr_func(f: &Func) -> String {
	match f {
		Func::Native(n)		=> n.name().to_string(),
		Func::Element(k)	=> k.name().to_string(),
		Func::Closure(c)	=> match &c.name {
			Some(n)	=> n.clone(),
			None	=> "(..) => ..".to_string(),
		},
		Func::With(_)		=> "(..) => ..".to_string(),
	}
}

fn repr_datetime(d: &Datetime) -> String {
	let mut parts = Vec::new();
	let fields = [
		("year", d.year.map(|y| y as i64)), ("month", d.month.map(|x| x as i64)),
		("day", d.day.map(|x| x as i64)), ("hour", d.hour.map(|x| x as i64)),
		("minute", d.minute.map(|x| x as i64)), ("second", d.second.map(|x| x as i64)),
	];
	for (n, v) in fields {
		if let Some(v) = v {
			parts.push(fmt!("{}: {}", n, v));
		}
	}
	fmt!("datetime{}", pretty_array_like(&parts, false))
}

fn repr_duration(d: &Duration) -> String {
	let total = d.secs.trunc() as i64;
	if total == 0 {
		return "duration()".to_string();
	}
	let mut rest = total;
	let mut parts = Vec::new();
	for (n, unit) in [("weeks", 604_800i64), ("days", 86_400), ("hours", 3_600), ("minutes", 60), ("seconds", 1)] {
		let q = rest / unit;
		rest -= q * unit;
		if q != 0 {
			parts.push(fmt!("{}: {}", n, q));
		}
	}
	fmt!("duration{}", pretty_array_like(&parts, false))
}

fn repr_tiling(t: &Tiling) -> String {
	let mut parts = Vec::new();
	if let Some((w, h)) = t.size {
		parts.push(fmt!("size: ({}, {})", repr_length(&w), repr_length(&h)));
	}
	parts.push(fmt!("spacing: ({}, {})", repr_length(&t.spacing.0), repr_length(&t.spacing.1)));
	parts.push(fmt!("relative: {}", relative_to_repr(t.relative)));
	parts.push(fmt!("body: {}", repr_content(&t.body)));
	fmt!("tiling{}", pretty_array_like(&parts, false))
}

pub fn relative_to_repr(r: crate::eval::value::RelativeTo) -> String {
	use crate::eval::value::RelativeTo;
	match r {
		RelativeTo::Auto	=> "auto".to_string(),
		RelativeTo::SelfBox	=> repr_str("self"),
		RelativeTo::Parent	=> repr_str("parent"),
	}
}

pub fn repr_paint(p: &Paint) -> String { repr(&paint_value(p)) }

fn repr_stroke(s: &Stroke) -> String {
	if s.cap.is_none() && s.join.is_none() && s.dash.is_none() && s.miter_limit.is_none() {
		return match (&s.paint, &s.thickness) {
			(Some(p), Some(t))	=> fmt!("{} + {}", repr_length(t), repr_paint(p)),
			(Some(p), None)		=> repr_paint(p),
			(None, Some(t))		=> repr_length(t),
			(None, None)		=> "1pt + black".to_string(),
		};
	}
	let mut parts = Vec::new();
	if let Some(p) = &s.paint {
		parts.push(fmt!("paint: {}", repr_paint(p)));
	}
	if let Some(t) = &s.thickness {
		parts.push(fmt!("thickness: {}", repr_length(t)));
	}
	if let Some(c) = s.cap {
		parts.push(fmt!("cap: {}", repr_str(cap_name(c))));
	}
	if let Some(j) = s.join {
		parts.push(fmt!("join: {}", repr_str(join_name(j))));
	}
	if let Some(d) = &s.dash {
		match d {
			None	=> parts.push("dash: none".to_string()),
			Some(d)	=> parts.push(fmt!("dash: {}", repr(&dash_value(d)))),
		}
	}
	if let Some(m) = s.miter_limit {
		parts.push(fmt!("miter-limit: {}", repr_float(m)));
	}
	fmt!("({})", parts.join(", "))
}

fn repr_selector(s: &Selector) -> String {
	match s {
		Selector::Elem(k, None)			=> k.name().to_string(),
		Selector::Elem(k, Some(fields))	=> {
			let parts: Vec<String> = fields.iter().map(|(id, v)| {
				let name = k.field_spec(*id).map(|f| f.name).unwrap_or("?");
				fmt!("{}: {}", name, repr(v))
			}).collect();
			fmt!("{}.where{}", k.name(), pretty_array_like(&parts, false))
		}
		Selector::Label(l)				=> repr_label(l),
		Selector::Text(t)				=> repr_str(t),
		Selector::Regex(r)				=> fmt!("regex({})", repr_str(&r.pattern)),
		Selector::Location(_)			=> "location(..)".to_string(),
		Selector::Or(v)					=> fmt!("selector.or{}", pretty_array_like(
			&v.iter().map(repr_selector).collect::<Vec<_>>(), false)),
		Selector::And(v)				=> fmt!("selector.and{}", pretty_array_like(
			&v.iter().map(repr_selector).collect::<Vec<_>>(), false)),
		Selector::Before { selector, end, inclusive } => fmt!("{}.before({}{})",
			repr_selector(selector), repr_selector(end), if *inclusive { "" } else { ", inclusive: false" }),
		Selector::After { selector, start, inclusive } => fmt!("{}.after({}{})",
			repr_selector(selector), repr_selector(start), if *inclusive { "" } else { ", inclusive: false" }),
	}
}

fn repr_counter(c: &Counter) -> String {
	match &c.key {
		CounterKey::Page		=> "counter(page)".to_string(),
		CounterKey::Selector(s)	=> fmt!("counter({})", repr_selector(s)),
		CounterKey::Str(s)		=> fmt!("counter({})", repr_str(s)),
	}
}

fn repr_state(s: &State) -> String { fmt!("state({}, {})", repr_str(&s.key), repr(&s.init)) }

/// Content as Typst's `repr` prints it: `[text]`, `[ ]` for a space, `sequence(..)`, and an element as
/// its name with its set fields in schema order.
pub fn repr_content(c: &Content) -> String {
	match c {
		Content::Sequence(s) => {
			if s.children.is_empty() {
				"[]".to_string()
			} else {
				let parts: Vec<String> = s.children.iter().map(repr_content).collect();
				fmt!("sequence{}", pretty_array_like(&parts, false))
			}
		}
		Content::Styled(s) => fmt!("styled(child: {}, ..)", repr_content(&s.child)),
		Content::Elem(e) => match e.kind {
			ElemKind::Text => match c.field("text").or_else(|| c.get(crate::eval::content::FieldId(0))) {
				Some(Value::Str(t))	=> fmt!("[{}]", t),
				_					=> "[]".to_string(),
			},
			ElemKind::Space => "[ ]".to_string(),
			kind => {
				let specs = kind.fields();
				let mut parts = Vec::new();
				for (i, spec) in specs.iter().enumerate() {
					let id = crate::eval::content::FieldId(i as u8);
					if let Some((_, v)) = e.fields.iter().find(|(f, _)| *f == id) {
						parts.push(fmt!("{}: {}", spec.name, repr(v)));
					}
				}
				// Fields outside the schema (a unit still filling its table) keep their ids.
				for (id, v) in &e.fields {
					if (id.0 as usize) >= specs.len() {
						parts.push(fmt!("{}: {}", id.0, repr(v)));
					}
				}
				fmt!("{}{}", kind.name(), pretty_array_like(&parts, false))
			}
		},
	}
}

// Pretty printing

const MAX_WIDTH: usize = 50;

/// Joins pieces with commas, one per line once they exceed Typst's fifty-byte width.
pub fn pretty_comma_list(pieces: &[String], trailing_comma: bool) -> String {
	let len: usize = pieces.iter().map(|s| s.len()).sum::<usize>() + 2 * pieces.len().saturating_sub(1);
	let mut buf = String::new();
	if len <= MAX_WIDTH {
		buf.push_str(&pieces.join(", "));
		if trailing_comma {
			buf.push(',');
		}
	} else {
		for p in pieces {
			buf.push_str(p.trim());
			buf.push_str(",\n");
		}
	}
	buf
}

/// A parenthesised list, wrapped and indented as Typst's `repr` wraps arrays, dicts and arguments.
pub fn pretty_array_like(parts: &[String], trailing_comma: bool) -> String {
	let list = pretty_comma_list(parts, trailing_comma);
	let mut buf = String::from("(");
	if list.contains('\n') {
		buf.push('\n');
		for (i, line) in list.lines().enumerate() {
			if i > 0 {
				buf.push('\n');
			}
			buf.push_str("  ");
			buf.push_str(line);
		}
		buf.push('\n');
	} else {
		buf.push_str(&list);
	}
	buf.push(')');
	buf
}

/// Converts a value to content text as Typst displays a value in markup: strings as they are, numbers
/// with a typographic minus, `none` as nothing. `None` for values that display as themselves (content).
pub fn display(v: &Value) -> Option<String> {
	let s = match v {
		Value::None			=> String::new(),
		Value::Str(s)		=> (**s).clone(),
		Value::Int(i)		=> format_int_with_base(*i, 10),
		Value::Float(f)		=> display_float(*f),
		Value::Symbol(s)	=> sym::text(s).to_string(),
		Value::Label(_) | Value::Content(_)	=> return None,
		other				=> repr(other),
	};
	Some(s)
}
