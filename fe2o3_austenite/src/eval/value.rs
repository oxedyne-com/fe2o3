// U0 owns this file. Every heap payload is behind an `Arc`, so a `Value` clones in constant time;
// mutate through `Arc::make_mut` (copy on write). Equality and ordering are Typst's and live in `ops.rs`,
// which is why `Value` does not derive `PartialEq`.

use crate::eval::args::Args;
use crate::eval::content::Content;
use crate::eval::func::Func;
use crate::eval::lib::decimal::Decimal;
use crate::eval::intro::{
	Counter,
	State,
};
use crate::eval::locate::Location;
use crate::eval::scope::Scope;
use crate::eval::select::Selector;
use crate::eval::styles::Styles;

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_text::regex::Regex;

use std::collections::HashMap;
use std::sync::Arc;

#[derive(Clone, Debug, Default)]
pub enum Value {
	#[default]
	None,
	Auto,
	Bool(bool),
	Int(i64),
	Float(f64),
	Decimal(Decimal),
	Length(Length),
	Angle(Angle),
	Ratio(Ratio),
	Relative(Relative),
	Fraction(Fraction),
	Color(Color),
	Gradient(Arc<Gradient>),
	Tiling(Arc<Tiling>),
	Stroke(Arc<Stroke>),
	Alignment(Alignment),
	Direction(Direction),
	Symbol(Symbol),
	Str(Arc<String>),
	Bytes(Arc<Vec<u8>>),
	Label(Label),
	Datetime(Datetime),
	Duration(Duration),
	Version(Arc<Vec<u32>>),
	Regex(Arc<RegexValue>),
	Content(Content),
	Array(Arc<Vec<Value>>),
	Dict(Arc<Dict>),
	Func(Func),
	Args(Arc<Args>),
	Module(Arc<Module>),
	Type(Type),
	Styles(Styles),
	Selector(Arc<Selector>),
	Counter(Arc<Counter>),
	State(Arc<State>),
	Location(Location),
}

/// The type of a value, itself a value (`type(1) == int`) and, for most types, a constructor.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Type {
	None,
	Auto,
	Bool,
	Int,
	Float,
	Decimal,
	Length,
	Angle,
	Ratio,
	Relative,
	Fraction,
	Color,
	Gradient,
	Tiling,
	Stroke,
	Alignment,
	Direction,
	Symbol,
	Str,
	Bytes,
	Label,
	Datetime,
	Duration,
	Version,
	Regex,
	Content,
	Array,
	Dict,
	Func,
	Args,
	Module,
	Type,
	Styles,
	Selector,
	Counter,
	State,
	Location,
}

impl Type {
	/// Every type, Typst's registry of them, in the order they are declared.
	pub const ALL: [Type; 37] = [
		Type::None, Type::Auto, Type::Bool, Type::Int, Type::Float, Type::Decimal, Type::Length, Type::Angle,
		Type::Ratio, Type::Relative, Type::Fraction, Type::Color, Type::Gradient, Type::Tiling, Type::Stroke,
		Type::Alignment, Type::Direction, Type::Symbol, Type::Str, Type::Bytes, Type::Label, Type::Datetime,
		Type::Duration, Type::Version, Type::Regex, Type::Content, Type::Array, Type::Dict, Type::Func,
		Type::Args, Type::Module, Type::Type, Type::Styles, Type::Selector, Type::Counter, Type::State,
		Type::Location,
	];

	/// The type a name stands for, in either spelling: `int` or `integer`, `relative` or `relative length`.
	pub fn from_name(name: &str) -> Option<Type> {
		Self::ALL.iter().copied().find(|t| t.name() == name || t.long_name() == name)
	}

	/// The name `repr(type(x))` prints.
	pub fn name(self) -> &'static str {
		match self {
			Type::None		=> "none",
			Type::Auto		=> "auto",
			Type::Bool		=> "bool",
			Type::Int		=> "int",
			Type::Float		=> "float",
			Type::Decimal	=> "decimal",
			Type::Length	=> "length",
			Type::Angle		=> "angle",
			Type::Ratio		=> "ratio",
			Type::Relative	=> "relative",
			Type::Fraction	=> "fraction",
			Type::Color		=> "color",
			Type::Gradient	=> "gradient",
			Type::Tiling	=> "tiling",
			Type::Stroke	=> "stroke",
			Type::Alignment	=> "alignment",
			Type::Direction	=> "direction",
			Type::Symbol	=> "symbol",
			Type::Str		=> "str",
			Type::Bytes		=> "bytes",
			Type::Label		=> "label",
			Type::Datetime	=> "datetime",
			Type::Duration	=> "duration",
			Type::Version	=> "version",
			Type::Regex		=> "regex",
			Type::Content	=> "content",
			Type::Array		=> "array",
			Type::Dict		=> "dictionary",
			Type::Func		=> "function",
			Type::Args		=> "arguments",
			Type::Module	=> "module",
			Type::Type		=> "type",
			Type::Styles	=> "styles",
			Type::Selector	=> "selector",
			Type::Counter	=> "counter",
			Type::State		=> "state",
			Type::Location	=> "location",
		}
	}

	/// The name `str(type(x))` prints and error messages use: `integer` where `repr` says `int`.
	pub fn long_name(self) -> &'static str {
		match self {
			Type::Bool		=> "boolean",
			Type::Int		=> "integer",
			Type::Str		=> "string",
			Type::Relative	=> "relative length",
			other			=> other.name(),
		}
	}
}

impl Value {
	pub fn ty(&self) -> Type {
		match self {
			Value::None			=> Type::None,
			Value::Auto			=> Type::Auto,
			Value::Bool(_)		=> Type::Bool,
			Value::Int(_)		=> Type::Int,
			Value::Float(_)		=> Type::Float,
			Value::Decimal(_)	=> Type::Decimal,
			Value::Length(_)	=> Type::Length,
			Value::Angle(_)		=> Type::Angle,
			Value::Ratio(_)		=> Type::Ratio,
			Value::Relative(_)	=> Type::Relative,
			Value::Fraction(_)	=> Type::Fraction,
			Value::Color(_)		=> Type::Color,
			Value::Gradient(_)	=> Type::Gradient,
			Value::Tiling(_)	=> Type::Tiling,
			Value::Stroke(_)	=> Type::Stroke,
			Value::Alignment(_)	=> Type::Alignment,
			Value::Direction(_)	=> Type::Direction,
			Value::Symbol(_)	=> Type::Symbol,
			Value::Str(_)		=> Type::Str,
			Value::Bytes(_)		=> Type::Bytes,
			Value::Label(_)		=> Type::Label,
			Value::Datetime(_)	=> Type::Datetime,
			Value::Duration(_)	=> Type::Duration,
			Value::Version(_)	=> Type::Version,
			Value::Regex(_)		=> Type::Regex,
			Value::Content(_)	=> Type::Content,
			Value::Array(_)		=> Type::Array,
			Value::Dict(_)		=> Type::Dict,
			Value::Func(_)		=> Type::Func,
			Value::Args(_)		=> Type::Args,
			Value::Module(_)	=> Type::Module,
			Value::Type(_)		=> Type::Type,
			Value::Styles(_)	=> Type::Styles,
			Value::Selector(_)	=> Type::Selector,
			Value::Counter(_)	=> Type::Counter,
			Value::State(_)		=> Type::State,
			Value::Location(_)	=> Type::Location,
		}
	}

	pub fn str<S: Into<String>>(s: S) -> Self { Value::Str(Arc::new(s.into())) }

	pub fn array(v: Vec<Value>) -> Self { Value::Array(Arc::new(v)) }

	pub fn dict(d: Dict) -> Self { Value::Dict(Arc::new(d)) }

	pub fn is_none(&self) -> bool { matches!(self, Value::None) }

	pub fn is_auto(&self) -> bool { matches!(self, Value::Auto) }

	/// Casts to `T`, the error naming the expected and the found type.
	pub fn cast<T: FromValue>(self) -> Outcome<T> { T::from_value(self) }
}

// Units

/// A length whose `em` part stays unresolved until realisation knows the font size, as in Typst. The
/// conversion to [`crate::ir::Sp`] happens at the flow boundary.
#[derive(Clone, Copy, Debug, Default, PartialEq, PartialOrd)]
pub struct Length {
	pub abs:	f64,	// points
	pub em:		f64,	// multiples of the font size
}

impl Length {
	pub fn pt(abs: f64) -> Self { Self { abs, em: 0.0 } }
	pub fn em(em: f64) -> Self { Self { abs: 0.0, em } }
	pub fn zero() -> Self { Self::default() }

	/// The absolute length at a font size, in points.
	pub fn resolve(&self, font_size_pt: f64) -> f64 { self.abs + self.em * font_size_pt }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, PartialOrd)]
pub struct Angle(pub f64);		// radians

#[derive(Clone, Copy, Debug, Default, PartialEq, PartialOrd)]
pub struct Ratio(pub f64);		// 1.0 is 100%

#[derive(Clone, Copy, Debug, Default, PartialEq, PartialOrd)]
pub struct Fraction(pub f64);	// 1.0 is 1fr

/// `50% + 1em`: a ratio of the containing size plus a length.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Relative {
	pub rel:	Ratio,
	pub abs:	Length,
}

impl Relative {
	/// The absolute length against a containing size and a font size, both in points.
	pub fn resolve(&self, whole_pt: f64, font_size_pt: f64) -> f64 {
		self.rel.0 * whole_pt + self.abs.resolve(font_size_pt)
	}
}

// Colour and paint

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ColorSpace {
	Luma,
	Oklab,
	Oklch,
	LinearRgb,
	Rgb,
	Cmyk,
	Hsl,
	Hsv,
}

/// A colour in the space it was written in, so `repr`, `components` and `mix` round-trip as Typst's
/// do. Components are in the space's own order and range (percentages as 0..=1, hue in degrees); `alpha`
/// is 0..=1 and absent for cmyk. Conversion to [`fe2o3_graphics::colour::Rgba`] is `lib/color.rs` (U3).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Color {
	pub space:	ColorSpace,
	pub c:		[f32; 4],
	pub alpha:	f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RelativeTo {
	Auto,
	SelfBox,	// "self"
	Parent,
}

#[derive(Clone, Debug, PartialEq)]
pub enum GradientKind {
	Linear { angle: Angle },
	Radial { center: (Ratio, Ratio), radius: Ratio, focal_center: (Ratio, Ratio), focal_radius: Ratio },
	Conic { center: (Ratio, Ratio), angle: Angle },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Gradient {
	pub kind:		GradientKind,
	pub stops:		Vec<(Color, Ratio)>,
	pub space:		ColorSpace,		// the space interpolation runs in
	pub relative:	RelativeTo,
}

#[derive(Clone, Debug)]
pub struct Tiling {
	pub body:		Content,
	pub size:		Option<(Length, Length)>,	// None: auto, the body's own size
	pub spacing:	(Length, Length),
	pub relative:	RelativeTo,
}

#[derive(Clone, Debug)]
pub enum Paint {
	Color(Color),
	Gradient(Arc<Gradient>),
	Tiling(Arc<Tiling>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LineCap {
	Butt,
	Round,
	Square,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LineJoin {
	Miter,
	Round,
	Bevel,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DashItem {
	Len(Length),
	Dot,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Dash {
	pub array:	Vec<DashItem>,
	pub phase:	Length,
}

/// A stroke as written: every part optional, because `set` folds partial strokes onto one another
/// (`stroke: red` then `stroke: 2pt` gives a red 2pt stroke). `dash: Some(None)` is an explicit solid.
#[derive(Clone, Debug, Default)]
pub struct Stroke {
	pub paint:			Option<Paint>,
	pub thickness:		Option<Length>,
	pub cap:			Option<LineCap>,
	pub join:			Option<LineJoin>,
	pub dash:			Option<Option<Dash>>,
	pub miter_limit:	Option<f64>,
}

// Layout vocabulary

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum HAlign {
	Start,
	Left,
	Center,
	Right,
	End,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum VAlign {
	Top,
	Horizon,
	Bottom,
}

/// `left`, `top`, or `left + top`: either axis may be absent.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Alignment {
	pub x:	Option<HAlign>,
	pub y:	Option<VAlign>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Direction {
	Ltr,
	Rtl,
	Ttb,
	Btt,
}

// Text-ish values

#[derive(Clone, Debug)]
pub enum SymVariants {
	Single(&'static str),
	Static(&'static [(&'static str, &'static str)]),	// (dot-joined modifiers, text), the default first
	Runtime(Arc<Vec<(String, String)>>),				// from `symbol(...)`
}

/// A symbol and the modifiers applied so far (`arrow.r.long` is `arrow` with "r.long").
#[derive(Clone, Debug)]
pub struct Symbol {
	pub variants:	SymVariants,
	pub modifiers:	Arc<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Label(pub Arc<str>);

impl Label {
	pub fn new(s: &str) -> Self { Label(Arc::from(s)) }
	pub fn as_str(&self) -> &str { &self.0 }
}

/// A compiled `regex(...)`: the pattern as written, for `repr` and equality, and the `fe2o3_text` engine.
#[derive(Clone, Debug)]
pub struct RegexValue {
	pub pattern:	String,
	pub re:			Regex,
}

impl RegexValue {
	pub fn new(pattern: &str) -> Outcome<Self> {
		let re = res!(Regex::new(pattern));
		Ok(Self { pattern: pattern.to_string(), re })
	}
}

/// A date, a time, or both, each part present or absent as Typst's `datetime` allows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Datetime {
	pub year:	Option<i32>,
	pub month:	Option<u8>,
	pub day:	Option<u8>,
	pub hour:	Option<u8>,
	pub minute:	Option<u8>,
	pub second:	Option<u8>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, PartialOrd)]
pub struct Duration {
	pub secs:	f64,
}

// Collections

/// An insertion-ordered dictionary: Typst iterates a dict in the order keys were first inserted.
#[derive(Clone, Debug, Default)]
pub struct Dict {
	entries:	Vec<(Arc<str>, Value)>,
	index:		HashMap<Arc<str>, usize>,
}

impl Dict {
	pub fn new() -> Self { Self::default() }

	pub fn len(&self) -> usize { self.entries.len() }

	pub fn is_empty(&self) -> bool { self.entries.is_empty() }

	pub fn get(&self, key: &str) -> Option<&Value> {
		self.index.get(key).and_then(|i| self.entries.get(*i)).map(|(_, v)| v)
	}

	pub fn get_mut(&mut self, key: &str) -> Option<&mut Value> {
		match self.index.get(key) {
			Some(i)	=> self.entries.get_mut(*i).map(|(_, v)| v),
			None	=> None,
		}
	}

	pub fn contains(&self, key: &str) -> bool { self.index.contains_key(key) }

	/// Inserts or replaces; a replaced key keeps its original position, as in Typst.
	pub fn insert(&mut self, key: &str, value: Value) {
		match self.index.get(key) {
			Some(i) => {
				if let Some(e) = self.entries.get_mut(*i) {
					e.1 = value;
				}
			}
			None => {
				let k: Arc<str> = Arc::from(key);
				self.index.insert(k.clone(), self.entries.len());
				self.entries.push((k, value));
			}
		}
	}

	pub fn remove(&mut self, key: &str) -> Option<Value> {
		let i = match self.index.remove(key) {
			Some(i)	=> i,
			None	=> return None,
		};
		let (_, v) = self.entries.remove(i);
		for j in self.index.values_mut() {
			if *j > i {
				*j -= 1;
			}
		}
		Some(v)
	}

	pub fn iter(&self) -> impl Iterator<Item = (&str, &Value)> {
		self.entries.iter().map(|(k, v)| (&**k, v))
	}

	pub fn keys(&self) -> impl Iterator<Item = &str> {
		self.entries.iter().map(|(k, _)| &**k)
	}
}

/// An evaluated module: `import`'s result and `sym`/`calc`'s shape.
#[derive(Clone, Debug)]
pub struct Module {
	pub name:		Arc<String>,
	pub scope:		Scope,
	pub content:	Content,	// the module's markup, what `include` yields
}

impl Module {
	pub fn new<S: Into<String>>(name: S, scope: Scope) -> Self {
		Self { name: Arc::new(name.into()), scope, content: Content::empty() }
	}
}

// Casting

pub trait FromValue: Sized {
	fn from_value(v: Value) -> Outcome<Self>;
}

pub trait IntoValue {
	fn into_value(self) -> Value;
}

fn mismatch(expected: &str, found: &Value) -> Error<ErrTag> {
	err!("expected {}, found {}", expected, found.ty().long_name(); Input, Mismatch)
}

impl FromValue for Value {
	fn from_value(v: Value) -> Outcome<Self> { Ok(v) }
}

impl FromValue for bool {
	fn from_value(v: Value) -> Outcome<Self> {
		match v {
			Value::Bool(b)	=> Ok(b),
			other			=> Err(mismatch("boolean", &other)),
		}
	}
}

impl FromValue for i64 {
	fn from_value(v: Value) -> Outcome<Self> {
		match v {
			Value::Int(i)	=> Ok(i),
			other			=> Err(mismatch("integer", &other)),
		}
	}
}

impl FromValue for usize {
	fn from_value(v: Value) -> Outcome<Self> {
		match v {
			Value::Int(i) if i >= 0	=> Ok(i as usize),
			Value::Int(i)			=> Err(err!("number must be at least zero, found {}", i; Input, Range)),
			other					=> Err(mismatch("integer", &other)),
		}
	}
}

impl FromValue for f64 {
	fn from_value(v: Value) -> Outcome<Self> {
		match v {
			Value::Int(i)	=> Ok(i as f64),
			Value::Float(f)	=> Ok(f),
			other			=> Err(mismatch("float", &other)),
		}
	}
}

impl FromValue for String {
	fn from_value(v: Value) -> Outcome<Self> {
		match v {
			Value::Str(s)	=> Ok((*s).clone()),
			other			=> Err(mismatch("string", &other)),
		}
	}
}

impl FromValue for Arc<String> {
	fn from_value(v: Value) -> Outcome<Self> {
		match v {
			Value::Str(s)	=> Ok(s),
			other			=> Err(mismatch("string", &other)),
		}
	}
}

impl FromValue for Label {
	fn from_value(v: Value) -> Outcome<Self> {
		match v {
			Value::Label(l)	=> Ok(l),
			other			=> Err(mismatch("label", &other)),
		}
	}
}

impl FromValue for Length {
	fn from_value(v: Value) -> Outcome<Self> {
		match v {
			Value::Length(l)	=> Ok(l),
			other				=> Err(mismatch("length", &other)),
		}
	}
}

impl FromValue for Relative {
	fn from_value(v: Value) -> Outcome<Self> {
		match v {
			Value::Relative(r)	=> Ok(r),
			Value::Length(l)	=> Ok(Relative { rel: Ratio(0.0), abs: l }),
			Value::Ratio(r)		=> Ok(Relative { rel: r, abs: Length::zero() }),
			other				=> Err(mismatch("relative length", &other)),
		}
	}
}

impl FromValue for Color {
	fn from_value(v: Value) -> Outcome<Self> {
		match v {
			Value::Color(c)	=> Ok(c),
			other			=> Err(mismatch("color", &other)),
		}
	}
}

impl FromValue for Alignment {
	fn from_value(v: Value) -> Outcome<Self> {
		match v {
			Value::Alignment(a)	=> Ok(a),
			other				=> Err(mismatch("alignment", &other)),
		}
	}
}

impl FromValue for Func {
	fn from_value(v: Value) -> Outcome<Self> {
		match v {
			Value::Func(f)	=> Ok(f),
			other			=> Err(mismatch("function", &other)),
		}
	}
}

impl FromValue for Vec<Value> {
	fn from_value(v: Value) -> Outcome<Self> {
		match v {
			Value::Array(a)	=> Ok((*a).clone()),
			other			=> Err(mismatch("array", &other)),
		}
	}
}

impl FromValue for Dict {
	fn from_value(v: Value) -> Outcome<Self> {
		match v {
			Value::Dict(d)	=> Ok((*d).clone()),
			other			=> Err(mismatch("dictionary", &other)),
		}
	}
}

/// Content accepts what Typst displays as content: content itself, a string, a symbol, a number, `none`.
impl FromValue for Content {
	fn from_value(v: Value) -> Outcome<Self> {
		match v {
			Value::Content(c)	=> Ok(c),
			Value::None			=> Ok(Content::empty()),
			Value::Str(s)		=> Ok(Content::text(&s)),
			Value::Int(i)		=> Ok(Content::text(&fmt!("{}", i))),
			other				=> Err(mismatch("content", &other)),
		}
	}
}

impl<T: FromValue> FromValue for Option<T> {
	fn from_value(v: Value) -> Outcome<Self> {
		match v {
			Value::None	=> Ok(None),
			other		=> Ok(Some(res!(T::from_value(other)))),
		}
	}
}

impl IntoValue for Value {
	fn into_value(self) -> Value { self }
}

impl IntoValue for bool {
	fn into_value(self) -> Value { Value::Bool(self) }
}

impl IntoValue for i64 {
	fn into_value(self) -> Value { Value::Int(self) }
}

impl IntoValue for f64 {
	fn into_value(self) -> Value { Value::Float(self) }
}

impl IntoValue for String {
	fn into_value(self) -> Value { Value::str(self) }
}

impl IntoValue for &str {
	fn into_value(self) -> Value { Value::str(self) }
}

impl IntoValue for Content {
	fn into_value(self) -> Value { Value::Content(self) }
}

impl IntoValue for Length {
	fn into_value(self) -> Value { Value::Length(self) }
}

impl IntoValue for Label {
	fn into_value(self) -> Value { Value::Label(self) }
}

impl IntoValue for Func {
	fn into_value(self) -> Value { Value::Func(self) }
}

impl IntoValue for Vec<Value> {
	fn into_value(self) -> Value { Value::array(self) }
}

impl IntoValue for Dict {
	fn into_value(self) -> Value { Value::dict(self) }
}

impl<T: IntoValue> IntoValue for Option<T> {
	fn into_value(self) -> Value {
		match self {
			Some(v)	=> v.into_value(),
			None	=> Value::None,
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	// An exhaustive match, so a type added to the enum fails here until `Type::ALL` holds it.
	fn slot(t: Type) -> usize {
		match t {
			Type::None => 0, Type::Auto => 1, Type::Bool => 2, Type::Int => 3, Type::Float => 4,
			Type::Decimal => 5, Type::Length => 6, Type::Angle => 7, Type::Ratio => 8, Type::Relative => 9,
			Type::Fraction => 10, Type::Color => 11, Type::Gradient => 12, Type::Tiling => 13, Type::Stroke => 14,
			Type::Alignment => 15, Type::Direction => 16, Type::Symbol => 17, Type::Str => 18, Type::Bytes => 19,
			Type::Label => 20, Type::Datetime => 21, Type::Duration => 22, Type::Version => 23, Type::Regex => 24,
			Type::Content => 25, Type::Array => 26, Type::Dict => 27, Type::Func => 28, Type::Args => 29,
			Type::Module => 30, Type::Type => 31, Type::Styles => 32, Type::Selector => 33, Type::Counter => 34,
			Type::State => 35, Type::Location => 36,
		}
	}

	#[test]
	fn every_type_is_in_the_registry_once_and_found_by_either_name() {
		for (i, t) in Type::ALL.iter().enumerate() {
			assert_eq!(slot(*t), i, "{:?} sits at its declared place in Type::ALL", t);
			assert_eq!(Type::from_name(t.name()), Some(*t), "{:?} by its repr name", t);
			assert_eq!(Type::from_name(t.long_name()), Some(*t), "{:?} by its message name", t);
		}
		assert_eq!(Type::from_name("integer"), Some(Type::Int));
		assert_eq!(Type::from_name("relative length"), Some(Type::Relative));
		assert_eq!(Type::from_name("nonesuch"), None);
		assert_eq!(Type::from_name(""), None);
	}
}
