// U5 owns this file: the vocabulary every model show and synthesis is written in. A show builds other
// units' elements by field name through their own constructors, so a schema that lacks a field is
// reported as that unit's gap (tag `Unimplemented`), never papered over here.

use crate::eval::args::Args;
use crate::eval::content::{
	self,
	Content,
	ElemKind,
	FieldId,
};
use crate::eval::lib::model;
use crate::eval::styles::{
	Property,
	Style,
	StyleChain,
	Styles,
};
use crate::eval::value::{
	FromValue,
	Label,
	Length,
	Type,
	Value,
};
use crate::eval::Engine;
use crate::syntax::Span;

use oxedyne_fe2o3_core::prelude::*;

/// The id of a named field, or the owning unit's gap as an error.
pub fn fid(kind: ElemKind, name: &str) -> Outcome<FieldId> {
	match kind.field_id(name) {
		Some(id)	=> Ok(id),
		None		=> Err(err!("{} has no `{}` field in its schema, which the model's default show needs",
			kind.path(), name; Unimplemented)),
	}
}

/// Builds an element through its family's constructor, as `kind(..positional, ..named)` would in Typst
/// code, so the owning unit's normalisation and checks apply.
pub fn build(
	engine:		&mut Engine,
	kind:		ElemKind,
	span:		Span,
	positional:	Vec<Value>,
	named:		Vec<(&str, Value)>,
)
	-> Outcome<Content>
{
	let mut args = Args::new(span);
	for v in positional {
		args.push(span, v);
	}
	for (n, v) in named {
		args.push_named(span, n, v);
	}
	let c = res!(content::construct(engine, kind, &mut args));
	Ok(spanned(c, span))
}

/// An element with the given fields set directly, for this unit's own elements whose constructor would
/// refuse them (synthesised fields, `cannot be constructed manually`).
pub fn raw_elem(kind: ElemKind, span: Span, fields: Vec<(&str, Value)>) -> Outcome<Content> {
	let mut fv = Vec::with_capacity(fields.len());
	for (n, v) in fields {
		fv.push((res!(fid(kind, n)), v));
	}
	Ok(Content::new(kind, fv, span))
}

/// A property style, `set kind(name: value)` in code.
pub fn prop(kind: ElemKind, name: &str, value: Value) -> Outcome<Style> {
	Ok(Style::Property(Property::new(kind, res!(fid(kind, name)), value, Span::detached())))
}

/// Styles from a list of properties.
pub fn props(list: Vec<(ElemKind, &str, Value)>) -> Outcome<Styles> {
	let mut s = Styles::new();
	for (k, n, v) in list {
		s.push(res!(prop(k, n, v)));
	}
	Ok(s)
}

/// Content under one property, Typst's `content.set(field, value)`.
pub fn set(c: Content, kind: ElemKind, name: &str, value: Value) -> Outcome<Content> {
	Ok(c.styled(Styles::from_style(res!(prop(kind, name, value)))))
}

pub fn spanned(c: Content, span: Span) -> Content {
	if c.span().is_detached() { c.with_span(span) } else { c }
}

pub fn seq(parts: Vec<Content>) -> Content {
	Content::sequence(parts.into_iter().filter(|c| !is_void(c)).collect())
}

/// An empty sequence with no label, which contributes nothing to a sequence.
fn is_void(c: &Content) -> bool {
	matches!(c, Content::Sequence(s) if s.children.is_empty() && s.label.is_none())
}

pub fn text(s: &str) -> Content { Content::text(s) }

/// A field of an element in hand: its own value, else the style chain's, else its default (from the
/// schema, or computed by the model for a default no static can hold).
pub fn get(elem: &Content, styles: &StyleChain, name: &str) -> Outcome<Value> {
	let kind = match elem.kind() {
		Some(k)	=> k,
		None	=> return Err(err!("a field was asked of content that is not an element"; Bug)),
	};
	let id = res!(fid(kind, name));
	if let Some(v) = res!(styles.resolve(elem, id)) {
		return Ok(v);
	}
	match model::default_value(kind, name) {
		Some(v)	=> Ok(v),
		None	=> Ok(Value::None),
	}
}

pub fn get_as<T: FromValue>(elem: &Content, styles: &StyleChain, name: &str) -> Outcome<T> {
	T::from_value(res!(get(elem, styles, name)))
}

/// A style-only field of `kind` read from the chain alone, with its default.
pub fn style(styles: &StyleChain, kind: ElemKind, name: &str) -> Outcome<Value> {
	let id = res!(fid(kind, name));
	if let Some(v) = res!(styles.get(kind, id)) {
		return Ok(v);
	}
	match kind.family() {
		content::Family::Model => Ok(model::default_value(kind, name).unwrap_or(Value::None)),
		_ => Ok(kind.field_spec(id).and_then(|s| s.default.to_value()).unwrap_or(Value::None)),
	}
}

/// A value as content, for the values a cast has already narrowed to content, text, a symbol or a
/// number. A value of any other kind shows as its `repr`; where one can arrive (a function's result),
/// [`shown`] displays it exactly as Typst does.
pub fn display(v: Value) -> Content {
	match v {
		Value::None			=> Content::empty(),
		Value::Content(c)	=> c,
		Value::Str(s)		=> Content::text(&s),
		Value::Symbol(s)	=> Content::symbol(&crate::eval::ops::symbol_text(&s)),
		other				=> Content::text(&crate::eval::lib::foundations::repr(&other)),
	}
}

/// Typst's `Value::display` for any value, a function's result included.
pub fn shown(engine: &mut Engine, v: Value, span: Span) -> Outcome<Content> {
	content::display(engine, v, span)
}

/// A content-valued field (`body`), displayed.
pub fn body(elem: &Content, name: &str) -> Content {
	match elem.field(name) {
		Some(v)	=> display(v.clone()),
		None	=> Content::empty(),
	}
}

pub fn em(v: f64) -> Value { Value::Length(Length::em(v)) }

pub fn pt(v: f64) -> Value { Value::Length(Length::pt(v)) }

pub fn label_of(v: &Value) -> Option<Label> {
	match v {
		Value::Label(l)	=> Some(l.clone()),
		_				=> None,
	}
}

/// Is the value `auto`, or an absent optional read as `auto`?
pub fn is_auto(v: &Value) -> bool { matches!(v, Value::Auto) }

/// The type error Typst reports for a value a cast refuses.
pub fn mismatch(expected: &str, found: &Value) -> String {
	fmt!("expected {}, found {}", expected, found.ty().long_name())
}

/// Does the value have one of these types?
pub fn is_one_of(v: &Value, ts: &[Type]) -> bool { ts.contains(&v.ty()) }

/// Every element in `c`, depth first in document order, through fields holding content or arrays of
/// it: Typst's `Content::traverse`, which `query_first_naive` walks.
pub fn traverse<F: FnMut(&Content) -> bool>(c: &Content, f: &mut F) -> bool {
	match c {
		Content::Sequence(s)	=> {
			for x in &s.children {
				if traverse(x, f) {
					return true;
				}
			}
			false
		}
		Content::Styled(s)		=> traverse(&s.child, f),
		Content::Elem(e)		=> {
			if f(c) {
				return true;
			}
			for (_, v) in &e.fields {
				if traverse_value(v, f) {
					return true;
				}
			}
			false
		}
	}
}

fn traverse_value<F: FnMut(&Content) -> bool>(v: &Value, f: &mut F) -> bool {
	match v {
		Value::Content(c)	=> traverse(c, f),
		Value::Array(a)		=> {
			for x in a.iter() {
				if traverse_value(x, f) {
					return true;
				}
			}
			false
		}
		_					=> false,
	}
}

/// The first element in `c` (itself included) that the predicate accepts.
pub fn find_first<P: Fn(&Content) -> bool>(c: &Content, pred: P) -> Option<Content> {
	let mut found = None;
	traverse(c, &mut |e| {
		if pred(e) {
			found = Some(e.clone());
			true
		} else {
			false
		}
	});
	found
}

/// Typst's plain text of content, which also reads raw text and a smart quote.
pub fn plain_text(c: &Content) -> String {
	let mut s = String::new();
	push_plain(c, &mut s);
	s
}

fn push_plain(c: &Content, s: &mut String) {
	match c {
		Content::Sequence(q)	=> for x in &q.children { push_plain(x, s); },
		Content::Styled(q)		=> push_plain(&q.child, s),
		Content::Elem(e) => match e.kind {
			ElemKind::Text => if let Some(Value::Str(t)) = c.get(FieldId(0)) {
				s.push_str(t);
			},
			ElemKind::Space | ElemKind::Linebreak	=> s.push(' '),
			ElemKind::Parbreak						=> s.push_str("\n\n"),
			ElemKind::SmartQuote => {
				let double = !matches!(c.field("double"), Some(Value::Bool(false)));
				s.push(if double { '"' } else { '\'' });
			}
			ElemKind::Raw | ElemKind::RawLine => if let Some(Value::Str(t)) = c.field("text") {
				s.push_str(t);
			},
			_ => match c.field("body") {
				Some(Value::Content(b))	=> push_plain(b, s),
				Some(Value::Str(t))		=> s.push_str(t),
				_						=> (),
			},
		},
	}
}

// Casting field values

/// Why a field refused a value: its type (`find` skips such a positional argument) or its value.
#[derive(Clone, Debug)]
pub enum CastErr {
	Type(String),
	Value(String),
}

impl CastErr {
	pub fn message(&self) -> &str {
		match self {
			CastErr::Type(m) | CastErr::Value(m)	=> m,
		}
	}
}

/// The kinds of value a field cast names in its message, in Typst's order and words.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum K {
	Bool,
	Int,
	Length,
	Rel,
	Content,
	Str,
	Label,
	Func,
	Dict,
	Array,
	Location,
	Selector,
	Alignment,
	Path,
	Bytes,
	None,
	Auto,
}

impl K {
	pub fn name(self) -> &'static str {
		match self {
			K::Bool			=> "boolean",
			K::Int			=> "integer",
			K::Length		=> "length",
			K::Rel			=> "relative length",
			K::Content		=> "content",
			K::Str			=> "string",
			K::Label		=> "label",
			K::Func			=> "function",
			K::Dict			=> "dictionary",
			K::Array		=> "array",
			K::Location		=> "location",
			K::Selector		=> "selector",
			K::Alignment	=> "alignment",
			K::Path			=> "path",
			K::Bytes		=> "bytes",
			K::None			=> "none",
			K::Auto			=> "auto",
		}
	}

	pub fn accepts(self, v: &Value) -> bool {
		match self {
			K::Bool			=> matches!(v, Value::Bool(_)),
			K::Int			=> matches!(v, Value::Int(_)),
			K::Length		=> matches!(v, Value::Length(_)),
			K::Rel			=> matches!(v, Value::Length(_) | Value::Ratio(_) | Value::Relative(_)),
			// Content takes what displays as content, and `none` as nothing.
			K::Content		=> matches!(v, Value::Content(_) | Value::Str(_) | Value::Symbol(_) | Value::None),
			K::Str | K::Path	=> matches!(v, Value::Str(_)),
			K::Label		=> matches!(v, Value::Label(_)),
			K::Func			=> matches!(v, Value::Func(_)),
			K::Dict			=> matches!(v, Value::Dict(_)),
			K::Array		=> matches!(v, Value::Array(_)),
			K::Location		=> matches!(v, Value::Location(_)),
			K::Selector		=> matches!(v, Value::Selector(_)),
			K::Alignment	=> matches!(v, Value::Alignment(_)),
			K::Bytes		=> matches!(v, Value::Bytes(_)),
			K::None			=> matches!(v, Value::None),
			K::Auto			=> matches!(v, Value::Auto),
		}
	}
}

/// Checks a value against the kinds a field accepts, with Typst's message on refusal.
pub fn expect(v: &Value, ks: &[K]) -> Result<(), CastErr> {
	if ks.iter().any(|k| k.accepts(v)) {
		return Ok(());
	}
	let names: Vec<&str> = ks.iter().map(|k| k.name()).collect();
	Err(CastErr::Type(mismatch(&list(&names), v)))
}

/// "A, B, or C", as Typst lists what a cast accepts.
pub fn list(names: &[&str]) -> String {
	match names.len() {
		0	=> String::new(),
		1	=> names[0].to_string(),
		2	=> fmt!("{} or {}", names[0], names[1]),
		n	=> fmt!("{}, or {}", names[..n - 1].join(", "), names[n - 1]),
	}
}

/// One of a closed set of strings, optionally `auto` or `none`: Typst's message names the strings quoted
/// and does not say what was found.
pub fn choice(v: &Value, options: &[&str], auto: bool, none: bool) -> Result<(), CastErr> {
	match v {
		Value::Str(s) if options.contains(&s.as_str())	=> return Ok(()),
		Value::Auto if auto								=> return Ok(()),
		Value::None if none								=> return Ok(()),
		_												=> (),
	}
	let mut names: Vec<String> = options.iter().map(|o| fmt!("\"{}\"", o)).collect();
	if none {
		names.push("none".to_string());
	}
	if auto {
		names.push("auto".to_string());
	}
	let refs: Vec<&str> = names.iter().map(|s| s.as_str()).collect();
	let msg = fmt!("expected {}", list(&refs));
	// A string outside the set is a value error; any other type a type error.
	match v {
		Value::Str(_)	=> Err(CastErr::Value(msg)),
		_				=> Err(CastErr::Type(msg)),
	}
}

/// A positive integer (`number must be positive`) or, with `zero`, a non-negative one.
pub fn natural(v: &Value, zero: bool) -> Result<(), CastErr> {
	match v {
		Value::Int(i) if *i > 0 || (zero && *i == 0)	=> Ok(()),
		Value::Int(_) if zero	=> Err(CastErr::Value("number must be at least zero".to_string())),
		Value::Int(_)			=> Err(CastErr::Value("number must be positive".to_string())),
		other					=> Err(CastErr::Type(mismatch("integer", other))),
	}
}

/// An alignment restricted to some named positions: Typst's message names them in backticks and says
/// what was found.
pub fn alignment_of(v: &Value, names: &[&str]) -> Result<(), CastErr> {
	let name = match v {
		Value::Alignment(a) => alignment_name(a),
		other => {
			let quoted: Vec<String> = names.iter().map(|n| fmt!("`{}`", n)).collect();
			let refs: Vec<&str> = quoted.iter().map(|s| s.as_str()).collect();
			return Err(CastErr::Type(mismatch(&list(&refs), other)));
		}
	};
	if names.contains(&name.as_str()) {
		return Ok(());
	}
	let quoted: Vec<String> = names.iter().map(|n| fmt!("`{}`", n)).collect();
	let refs: Vec<&str> = quoted.iter().map(|s| s.as_str()).collect();
	Err(CastErr::Value(fmt!("expected {}, found {}", list(&refs), name)))
}

/// An alignment's Typst name: `left`, `top`, `center + horizon`.
pub fn alignment_name(a: &crate::eval::value::Alignment) -> String {
	use crate::eval::value::{
		HAlign,
		VAlign,
	};
	let x = a.x.map(|x| match x {
		HAlign::Start	=> "start",
		HAlign::Left	=> "left",
		HAlign::Center	=> "center",
		HAlign::Right	=> "right",
		HAlign::End		=> "end",
	});
	let y = a.y.map(|y| match y {
		VAlign::Top		=> "top",
		VAlign::Horizon	=> "horizon",
		VAlign::Bottom	=> "bottom",
	});
	match (x, y) {
		(Some(x), Some(y))	=> fmt!("{} + {}", x, y),
		(Some(x), None)		=> x.to_string(),
		(None, Some(y))		=> y.to_string(),
		(None, None)		=> "start".to_string(),
	}
}

/// A value displayed as content, after `expect(.., &[K::Content])` accepted it.
pub fn to_content(v: Value) -> Value {
	match v {
		Value::Content(_)	=> v,
		other				=> Value::Content(display(other)),
	}
}

/// A style of another unit's element that only tunes a default show, read with Typst's default when that
/// element's schema does not have the field yet: no rule can have set a field the schema lacks, so the
/// default is exactly the value in force.
pub fn style_or(styles: &StyleChain, kind: ElemKind, name: &str, default: Value) -> Value {
	match kind.field_id(name) {
		Some(id) => match styles.get(kind, id) {
			Ok(Some(v))	=> v,
			// A value its own family cannot fold is that family's error to report where it lays out.
			_			=> kind.field_spec(id).and_then(|s| s.default.to_value()).unwrap_or(default),
		},
		None => default,
	}
}

/// The text language and region in force, Typst's `en` and none by default.
pub fn lang(styles: &StyleChain) -> (String, Option<String>) {
	let lang = match style_or(styles, ElemKind::Text, "lang", Value::str("en")) {
		Value::Str(s)	=> s.to_string(),
		_				=> "en".to_string(),
	};
	let region = match style_or(styles, ElemKind::Text, "region", Value::None) {
		Value::Str(s)	=> Some(s.to_string()),
		_				=> None,
	};
	(lang, region)
}

/// The word for `key` in the language in force.
pub fn local(styles: &StyleChain, key: &str) -> &'static str {
	let (l, r) = lang(styles);
	crate::eval::lib::model::local::local_name(key, &l, r.as_deref())
}

/// Is the text direction in force right to left: `text.dir` when set, else the language's own?
pub fn is_rtl(styles: &StyleChain) -> bool {
	use crate::eval::value::Direction;
	match style_or(styles, ElemKind::Text, "dir", Value::Auto) {
		Value::Direction(Direction::Rtl)	=> true,
		Value::Direction(_)					=> false,
		_ => {
			let (l, _) = lang(styles);
			matches!(l.as_str(), "ar" | "dv" | "fa" | "he" | "ks" | "pa" | "ps" | "sd" | "ug" | "ur" | "yi")
		}
	}
}

/// Sides with only the text's start side set: `(left: v)`, or `(right: v)` right to left.
pub fn start_side(styles: &StyleChain, v: Value) -> Value {
	let mut d = crate::eval::value::Dict::new();
	d.insert(if is_rtl(styles) { "right" } else { "left" }, v);
	Value::dict(d)
}

/// `h(amount)`, weak when asked.
pub fn h(engine: &mut Engine, amount: Value, weak: bool) -> Outcome<Content> {
	let mut named = Vec::new();
	if weak {
		named.push(("weak", Value::Bool(true)));
	}
	build(engine, ElemKind::H, Span::detached(), vec![amount], named)
}

/// `block(body)`, with named fields.
pub fn block(engine: &mut Engine, body: Content, span: Span, named: Vec<(&str, Value)>) -> Outcome<Content> {
	build(engine, ElemKind::Block, span, vec![Value::Content(body)], named)
}

/// A length negated, for a hanging indent's pull back.
pub fn neg(v: &Value) -> Value {
	match v {
		Value::Length(l)	=> Value::Length(Length { abs: -l.abs, em: -l.em }),
		other				=> other.clone(),
	}
}

/// A length in points, its em part resolved against the font size in force.
pub fn resolve_pt(styles: &StyleChain, v: &Value) -> f64 {
	match v {
		Value::Length(l)	=> styles.resolve_length(*l),
		_					=> 0.0,
	}
}

/// A length or ratio as the relative length a `relative` field stores it as.
pub fn relative(v: Value) -> Value {
	use crate::eval::value::{
		Ratio,
		Relative,
	};
	match v {
		Value::Length(l)	=> Value::Relative(Relative { abs: l, rel: Ratio(0.0) }),
		Value::Ratio(r)		=> Value::Relative(Relative { abs: Length::zero(), rel: r }),
		other				=> other,
	}
}
