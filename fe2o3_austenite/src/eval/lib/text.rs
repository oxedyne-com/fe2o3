// SPDX-License-Identifier: Apache-2.0
// Original work copyright (c) the Typst project authors (typst-library `text/` and typst-layout `rules.rs`, version 0.15.1).
// Modified for Austenite: ported to Hematite's types, IR and error handling.
// U6a owns this file: schemas for the text family and `lorem`, `upper`, `lower`. One fixed point: the
// `text` element's field 0 is its string, which `Content::text` and `plain_text` rely on; its style
// fields (font, size, weight, fill, lang, ...) follow it with Typst's defaults.
//
// Typst 0.15.1 is the reference (`typst-library/src/text`, Apache-2.0, (c) the Typst project authors):
// the field names and defaults, `text(..)`'s constructor that styles its body rather than making an
// element, and the native shows of `smallcaps`, `underline`, `overline`, `strike`, `highlight`, `super`
// and `sub`, each its body under an internal `text` style that shaping reads (`smallcaps`, `deco`,
// `shift-settings`). `strong` and `emph` (the model family) set the internal `delta` and `emph`.
// `lorem` reproduces the `lipsum` crate's Markov chain and seeded generator (MIT, (c) 2017 Martin
// Geisler), which Typst's `lorem` is built on, so it yields Typst's words.

use crate::diag::DiagnosticKind;
use crate::eval::args::Args;
use crate::eval::content::{
	Content,
	ElemKind,
	FieldDefault,
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
	set_rule,
	Property,
	Style,
	StyleChain,
	Styles,
};
use crate::eval::value::{
	Color,
	ColorSpace,
	Dict,
	Length,
	Type,
	Value,
};
use crate::eval::Engine;
use crate::syntax::Span;

use oxedyne_fe2o3_core::prelude::*;

use std::collections::HashMap;
use std::sync::OnceLock;

native_fns! {
	pub enum TextFn {
		Lorem	=> "lorem",
		Upper	=> "upper",
		Lower	=> "lower",
	}
}

// Keyword choices
const STYLE:		&str = "\"normal\", \"italic\", or \"oblique\"";
const WEIGHT:		&str = "integer, \"thin\", \"extralight\", \"light\", \"regular\", \"medium\", \"semibold\", \"bold\", \"extrabold\", or \"black\"";
const TOP_EDGE:		&str = "\"ascender\", \"cap-height\", \"x-height\", \"baseline\", \"bounds\", or length";
const BOTTOM_EDGE:	&str = "\"baseline\", \"descender\", \"bounds\", or length";
const NUMBER_TYPE:	&str = "\"lining\", \"old-style\", or auto";
const NUMBER_WIDTH:	&str = "\"proportional\", \"tabular\", or auto";

const PAINT:	FieldType = FieldType::OneOf(&[Type::Color, Type::Gradient, Type::Tiling]);
const REL:		FieldType = FieldType::OneOf(&[Type::Relative, Type::Length, Type::Ratio]);

const TEXT: &[FieldSpec] = &[
	FieldSpec::required("text", FieldType::Of(Type::Str)).unsettable(),
	FieldSpec::named("font", FieldType::OneOf(&[Type::Str, Type::Array, Type::Dict]), FieldDefault::Computed),
	FieldSpec::named("fallback", FieldType::Of(Type::Bool), FieldDefault::Bool(true)),
	FieldSpec::named("style", FieldType::Keyword(&[], STYLE), FieldDefault::Str("normal")),
	FieldSpec::named("weight", FieldType::Keyword(&[Type::Int], WEIGHT), FieldDefault::Int(400)),
	FieldSpec::named("stretch", FieldType::Of(Type::Ratio), FieldDefault::Ratio(1.0)),
	FieldSpec::named("size", FieldType::Of(Type::Length), FieldDefault::Pt(11.0)).fold(Fold::Add).positional(),
	FieldSpec::named("fill", PAINT, FieldDefault::Computed).positional(),
	FieldSpec::named("stroke", FieldType::Any, FieldDefault::None).fold(Fold::Stroke),
	FieldSpec::named("tracking", FieldType::Of(Type::Length), FieldDefault::Pt(0.0)),
	FieldSpec::named("spacing", REL, FieldDefault::Ratio(1.0)),
	FieldSpec::named("cjk-latin-spacing", FieldType::OneOf(&[Type::Auto, Type::None]), FieldDefault::Auto),
	FieldSpec::named("baseline", FieldType::Of(Type::Length), FieldDefault::Pt(0.0)),
	FieldSpec::named("overhang", FieldType::Of(Type::Bool), FieldDefault::Bool(true)),
	FieldSpec::named("top-edge", FieldType::Keyword(&[Type::Length], TOP_EDGE), FieldDefault::Str("cap-height")),
	FieldSpec::named("bottom-edge", FieldType::Keyword(&[Type::Length], BOTTOM_EDGE), FieldDefault::Str("baseline")),
	FieldSpec::named("lang", FieldType::Of(Type::Str), FieldDefault::Str("en")),
	FieldSpec::named("region", FieldType::OneOf(&[Type::Str, Type::None]), FieldDefault::None),
	FieldSpec::named("script", FieldType::OneOf(&[Type::Str, Type::Auto]), FieldDefault::Auto),
	FieldSpec::named("dir", FieldType::OneOf(&[Type::Direction, Type::Auto]), FieldDefault::Auto),
	FieldSpec::named("hyphenate", FieldType::OneOf(&[Type::Bool, Type::Auto]), FieldDefault::Auto),
	FieldSpec::named("costs", FieldType::Of(Type::Dict), FieldDefault::Computed).fold(Fold::Add),
	FieldSpec::named("kerning", FieldType::Of(Type::Bool), FieldDefault::Bool(true)),
	FieldSpec::named("alternates", FieldType::OneOf(&[Type::Bool, Type::Int]), FieldDefault::Bool(false)),
	FieldSpec::named("stylistic-set", FieldType::OneOf(&[Type::None, Type::Int, Type::Array]), FieldDefault::None),
	FieldSpec::named("ligatures", FieldType::Of(Type::Bool), FieldDefault::Bool(true)),
	FieldSpec::named("discretionary-ligatures", FieldType::Of(Type::Bool), FieldDefault::Bool(false)),
	FieldSpec::named("historical-ligatures", FieldType::Of(Type::Bool), FieldDefault::Bool(false)),
	FieldSpec::named("number-type", FieldType::Keyword(&[Type::Auto], NUMBER_TYPE), FieldDefault::Auto),
	FieldSpec::named("number-width", FieldType::Keyword(&[Type::Auto], NUMBER_WIDTH), FieldDefault::Auto),
	FieldSpec::named("slashed-zero", FieldType::Of(Type::Bool), FieldDefault::Bool(false)),
	FieldSpec::named("fractions", FieldType::Of(Type::Bool), FieldDefault::Bool(false)),
	FieldSpec::named("features", FieldType::OneOf(&[Type::Array, Type::Dict]), FieldDefault::Computed).fold(Fold::Custom),
	FieldSpec::named("variations", FieldType::Of(Type::Dict), FieldDefault::Computed).fold(Fold::Custom),
	// Internal: set by show rules, never by a document.
	FieldSpec::named("delta", FieldType::Of(Type::Int), FieldDefault::Int(0)).fold(Fold::Custom).unsettable(),
	FieldSpec::named("emph", FieldType::Of(Type::Bool), FieldDefault::Bool(false)).fold(Fold::Custom).unsettable(),
	FieldSpec::named("deco", FieldType::Of(Type::Array), FieldDefault::EmptyArray).fold(Fold::Custom).unsettable(),
	FieldSpec::named("case", FieldType::OneOf(&[Type::Str, Type::None]), FieldDefault::None).unsettable(),
	FieldSpec::named("smallcaps", FieldType::OneOf(&[Type::Str, Type::None]), FieldDefault::None).unsettable(),
	FieldSpec::named("shift-settings", FieldType::OneOf(&[Type::Dict, Type::None]), FieldDefault::None).unsettable(),
];

const LINEBREAK: &[FieldSpec] = &[
	FieldSpec::named("justify", FieldType::Of(Type::Bool), FieldDefault::Bool(false)),
];

// By contract field 0, as `Content::symbol` builds it.
const SYMBOL: &[FieldSpec] = &[
	FieldSpec::required("text", FieldType::Of(Type::Str)),
];

const SMARTQUOTE: &[FieldSpec] = &[
	FieldSpec::named("double", FieldType::Of(Type::Bool), FieldDefault::Bool(true)),
	FieldSpec::named("enabled", FieldType::Of(Type::Bool), FieldDefault::Bool(true)),
	FieldSpec::named("alternative", FieldType::Of(Type::Bool), FieldDefault::Bool(false)),
	FieldSpec::named("quotes", FieldType::OneOf(&[Type::Auto, Type::Str, Type::Array, Type::Dict]), FieldDefault::Auto),
];

const SMALLCAPS: &[FieldSpec] = &[
	FieldSpec::named("all", FieldType::Of(Type::Bool), FieldDefault::Bool(false)),
	FieldSpec::required("body", FieldType::Content),
];

const LINE_DECO: &[FieldSpec] = &[
	FieldSpec::named("stroke", FieldType::Any, FieldDefault::Auto).fold(Fold::Stroke),
	FieldSpec::named("offset", FieldType::OneOf(&[Type::Auto, Type::Length]), FieldDefault::Auto),
	FieldSpec::named("extent", FieldType::Of(Type::Length), FieldDefault::Pt(0.0)),
	FieldSpec::named("evade", FieldType::Of(Type::Bool), FieldDefault::Bool(true)),
	FieldSpec::named("background", FieldType::Of(Type::Bool), FieldDefault::Bool(false)),
	FieldSpec::required("body", FieldType::Content),
];

const STRIKE: &[FieldSpec] = &[
	FieldSpec::named("stroke", FieldType::Any, FieldDefault::Auto).fold(Fold::Stroke),
	FieldSpec::named("offset", FieldType::OneOf(&[Type::Auto, Type::Length]), FieldDefault::Auto),
	FieldSpec::named("extent", FieldType::Of(Type::Length), FieldDefault::Pt(0.0)),
	FieldSpec::named("background", FieldType::Of(Type::Bool), FieldDefault::Bool(false)),
	FieldSpec::required("body", FieldType::Content),
];

const HIGHLIGHT: &[FieldSpec] = &[
	FieldSpec::named("fill", FieldType::OneOf(&[Type::None, Type::Color, Type::Gradient, Type::Tiling]), FieldDefault::Computed),
	FieldSpec::named("stroke", FieldType::Any, FieldDefault::None).fold(Fold::Sides),
	FieldSpec::named("top-edge", FieldType::Keyword(&[Type::Length], TOP_EDGE), FieldDefault::Str("ascender")),
	FieldSpec::named("bottom-edge", FieldType::Keyword(&[Type::Length], BOTTOM_EDGE), FieldDefault::Str("descender")),
	FieldSpec::named("extent", FieldType::Of(Type::Length), FieldDefault::Pt(0.0)),
	FieldSpec::named("radius", FieldType::Any, FieldDefault::Pt(0.0)).fold(Fold::Corners),
	FieldSpec::required("body", FieldType::Content),
];

const SCRIPT: &[FieldSpec] = &[
	FieldSpec::named("typographic", FieldType::Of(Type::Bool), FieldDefault::Bool(true)),
	FieldSpec::named("baseline", FieldType::OneOf(&[Type::Auto, Type::Length]), FieldDefault::Auto),
	FieldSpec::named("size", FieldType::OneOf(&[Type::Auto, Type::Length]), FieldDefault::Auto),
	FieldSpec::required("body", FieldType::Content),
];

pub fn define(scope: &mut Scope) {
	for f in TextFn::ALL {
		scope.define(f.name(), Value::Func(Func::Native(NativeFunc::Text(*f))));
	}
}

pub fn call(f: TextFn, engine: &mut Engine, mut args: Args) -> Outcome<Value> {
	let span = args.span;
	match f {
		TextFn::Lorem => {
			let words = match res!(args.eat::<Value>()) {
				Some(Value::Int(n)) if n >= 0	=> n as usize,
				Some(Value::Int(_))				=> return Err(engine.error(DiagnosticKind::Type, span, "number must be at least zero")),
				Some(other)						=> return Err(engine.error(DiagnosticKind::Type, span, fmt!(
					"expected integer, found {}", other.ty().long_name()))),
				None							=> return Err(engine.error(DiagnosticKind::Type, span, "missing argument: words")),
			};
			res!(finish(engine, args));
			Ok(Value::str(lorem(words)))
		}
		TextFn::Upper | TextFn::Lower => {
			let upper = f == TextFn::Upper;
			let v = match res!(args.eat::<Value>()) {
				Some(v)	=> v,
				None	=> return Err(engine.error(DiagnosticKind::Type, span, "missing argument: text")),
			};
			res!(finish(engine, args));
			match v.symbol_as_str() {
				Value::Str(s) => Ok(Value::str(if upper { s.to_uppercase() } else { s.to_lowercase() })),
				Value::Content(c) => {
					let case = if upper { "upper" } else { "lower" };
					Ok(Value::Content(res!(internal(c, "case", Value::str(case)))))
				}
				other => Err(engine.error(DiagnosticKind::Type, span, fmt!(
					"expected string or content, found {}", other.ty().long_name()))),
			}
		}
	}
}

fn finish(engine: &mut Engine, args: Args) -> Outcome<()> {
	let span = args.span;
	match args.finish() {
		Ok(())	=> Ok(()),
		Err(e)	=> Err(engine.error(DiagnosticKind::Type, span, fmt!("{}", e))),
	}
}

pub fn fields(kind: ElemKind) -> &'static [FieldSpec] {
	match kind {
		ElemKind::Text									=> TEXT,
		ElemKind::Symbol								=> SYMBOL,
		ElemKind::Linebreak								=> LINEBREAK,
		ElemKind::SmartQuote							=> SMARTQUOTE,
		ElemKind::Smallcaps								=> SMALLCAPS,
		ElemKind::Underline | ElemKind::Overline		=> LINE_DECO,
		ElemKind::Strike								=> STRIKE,
		ElemKind::Highlight								=> HIGHLIGHT,
		ElemKind::Super | ElemKind::Sub					=> SCRIPT,
		_												=> &[],
	}
}

/// `text(..)` styles its body rather than making an element; everything else is built generically.
/// Its size and fill may be given without a name, as Typst allows.
pub fn construct(engine: &mut Engine, kind: ElemKind, args: &mut Args) -> Outcome<Option<Content>> {
	if kind != ElemKind::Text {
		return Ok(None);
	}
	let span = args.span;
	let mut rest = args.take();
	// The body is the last positional argument that is content-like; a positional length is the size
	// and a positional paint the fill.
	let body_at = rest.items.iter().rposition(|a| a.name.is_none() && FieldType::Content.accepts(&a.value)
		&& !matches!(a.value, Value::Int(_) | Value::Float(_) | Value::None));
	let body = match body_at {
		Some(i)	=> rest.items.remove(i).value,
		None	=> return Err(engine.error(DiagnosticKind::Type, span, "missing argument: body")),
	};
	let mut styles = Styles::new();
	for name in ["size", "fill"] {
		let pos = rest.items.iter().position(|a| a.name.is_none() && match name {
			"size"	=> matches!(a.value, Value::Length(_)),
			_		=> matches!(a.value, Value::Color(_) | Value::Gradient(_) | Value::Tiling(_)),
		});
		if let Some(p) = pos {
			let a = rest.items.remove(p);
			if !rest.items.iter().any(|b| b.name.as_deref() == Some(name)) {
				if let Some(id) = ElemKind::Text.field_id(name) {
					styles.push(Style::Property(Property::new(ElemKind::Text, id, a.value, a.span)));
				}
			}
		}
	}
	let named = res!(set_rule(engine, ElemKind::Text, rest));
	styles.extend(&named);
	let body: Content = res!(body.cast());
	Ok(Some(body.styled(styles)))
}

/// Typst's `check_font_list`: one `missing_font` warning, at the argument that named it, for each family
/// of a `font` value that neither the embedded faces nor a supplied font declare. A `set` rule and a
/// `text(..)` call both pass here, as both do in Typst.
pub fn check_font_list(engine: &mut Engine, list: &Value, span: Span) -> Outcome<()> {
	let mut names: Vec<String> = Vec::new();
	family_names(list, &mut names);
	if names.is_empty() {
		return Ok(());
	}
	let book = res!(engine.fonts.book());
	for name in names {
		if !book.has_family(&name) {
			engine.warn(DiagnosticKind::MissingFont, span, fmt!("unknown font family: {}", name));
		}
	}
	Ok(())
}

/// The lower-cased names in a `font` value: a name, a `(name: .., covers: ..)` dictionary, or an array
/// of either.
fn family_names(v: &Value, out: &mut Vec<String>) {
	match v {
		Value::Str(s)	=> out.push(s.to_lowercase()),
		Value::Dict(d)	=> if let Some(Value::Str(s)) = d.get("name") {
			out.push(s.to_lowercase());
		},
		Value::Array(a)	=> for item in a.iter() {
			family_names(item, out);
		},
		_				=> (),
	}
}

/// `content` under one of `text`'s internal fields.
fn internal(content: Content, field: &str, value: Value) -> Outcome<Content> {
	let id = match ElemKind::Text.field_id(field) {
		Some(id)	=> id,
		None		=> return Err(err!("The text schema has no field `{}`.", field; Bug)),
	};
	let span = content.span();
	Ok(content.styled(Styles::from_style(Style::Property(Property::new(ElemKind::Text, id, value, span)))))
}

/// A text field's default where the schema holds none (`Computed`): the font is the one Typst names, in the
/// lower case it keeps family names in, the fill is black, the costs are all 100%, and no feature or variation
/// is set. A highlight's fill is Typst's yellow.
pub fn default_value(kind: ElemKind, name: &str) -> Option<Value> {
	match (kind, name) {
		(ElemKind::Text, "font")	=> Some(Value::str("libertinus serif")),
		(ElemKind::Text, "fill")	=> Some(Value::Color(Color {
			space:	ColorSpace::Luma,
			c:		[0.0; 4],
			alpha:	1.0,
		})),
		// Every cost at its neutral 100%, in the order Typst lists them.
		(ElemKind::Text, "costs")	=> {
			let mut d = Dict::new();
			for k in ["hyphenation", "runt", "widow", "orphan"] {
				d.insert(k, Value::Ratio(crate::eval::value::Ratio(1.0)));
			}
			Some(Value::dict(d))
		},
		(ElemKind::Text, "features") | (ElemKind::Text, "variations")	=> Some(Value::dict(Dict::new())),
		// The highlight's own yellow, `#fffd11a1`.
		(ElemKind::Highlight, "fill")	=> Some(Value::Color(Color {
			space:	ColorSpace::Rgb,
			c:		[255.0 / 255.0, 253.0 / 255.0, 17.0 / 255.0, 0.0],
			alpha:	161.0 / 255.0,
		})),
		_							=> kind.field_id(name).and_then(|id| kind.field_spec(id)).and_then(|s| s.default.to_value()),
	}
}

/// A value as a read of its field gives it back, as Typst's `into_value` does: a list of font families in the lower
/// case it keeps them in, the first-line indent with both its keys.
pub fn as_read(kind: ElemKind, name: &str, v: Value) -> Outcome<Value> {
	Ok(match (kind, name, v) {
		(ElemKind::Text, "font", Value::Array(a))	=> {
			let names: Vec<Value> = a.iter().map(|x| match x {
				Value::Str(s)	=> Value::str(s.to_lowercase()),
				other			=> other.clone(),
			}).collect();
			Value::array(names)
		},
		// The indent is read as the dictionary it is written as, with both of its keys.
		(ElemKind::Par, "first-line-indent", Value::Dict(d))	=> {
			let mut full = Dict::new();
			full.insert("amount", d.get("amount").cloned().unwrap_or_else(|| Value::Length(Length::pt(0.0))));
			full.insert("all", d.get("all").cloned().unwrap_or(Value::Bool(false)));
			Value::dict(full)
		},
		(_, _, other)								=> other,
	})
}

/// A field of the element in hand, else the chain's, else the default.
fn field(elem: &Content, styles: &StyleChain, name: &str) -> Outcome<Option<Value>> {
	let (kind, id) = match elem.kind().and_then(|k| k.field_id(name).map(|id| (k, id))) {
		Some(x)	=> x,
		None	=> return Ok(None),
	};
	Ok(res!(styles.resolve(elem, id)).or_else(|| kind.field_spec(id).and_then(|s| s.default.to_value())))
}

fn body_of(elem: &Content) -> Content {
	match elem.field("body") {
		Some(Value::Content(c))	=> c.clone(),
		Some(Value::Str(s))		=> Content::text(s),
		_						=> Content::empty(),
	}
}

/// Typst's default highlight, `rgb("#fffd11a1")`.
fn highlight_default() -> Value {
	Value::Color(Color {
		space:	ColorSpace::Rgb,
		c:		[1.0, 253.0 / 255.0, 17.0 / 255.0, 0.0],
		alpha:	161.0 / 255.0,
	})
}

pub fn show(_engine: &mut Engine, elem: &Content, styles: &StyleChain) -> Outcome<Option<Content>> {
	let kind = match elem.kind() {
		Some(k)	=> k,
		None	=> return Ok(None),
	};
	let flag = |v: Option<Value>, dflt: bool| match v {
		Some(Value::Bool(b))	=> b,
		_						=> dflt,
	};
	match kind {
		ElemKind::Smallcaps => {
			let sc = if flag(res!(field(elem, styles, "all")), false) { "all" } else { "minuscules" };
			Ok(Some(res!(internal(body_of(elem), "smallcaps", Value::str(sc)))))
		}
		ElemKind::Underline | ElemKind::Overline | ElemKind::Strike | ElemKind::Highlight => {
			let mut d = Dict::new();
			let line = match kind {
				ElemKind::Underline	=> "underline",
				ElemKind::Overline	=> "overline",
				ElemKind::Strike	=> "strike",
				_					=> "highlight",
			};
			d.insert("line", Value::str(line));
			for name in ["stroke", "offset", "extent", "evade", "background", "top-edge", "bottom-edge", "radius"] {
				if let Some(v) = res!(field(elem, styles, name)) {
					d.insert(name, v);
				}
			}
			if kind == ElemKind::Highlight {
				let set = match kind.field_id("fill") {
					Some(id)	=> res!(styles.resolve(elem, id)),
					None		=> None,
				};
				d.insert("fill", set.unwrap_or_else(highlight_default));
			}
			let deco = Value::array(vec![Value::dict(d)]);
			Ok(Some(res!(internal(body_of(elem), "deco", deco))))
		}
		ElemKind::Super | ElemKind::Sub => {
			let size = styles.font_size();
			let mut d = Dict::new();
			d.insert("typographic", Value::Bool(flag(res!(field(elem, styles, "typographic")), true)));
			if let Some(Value::Length(l)) = res!(field(elem, styles, "baseline")) {
				d.insert("shift", Value::Float(-l.resolve(size) / size));
			}
			if let Some(Value::Length(l)) = res!(field(elem, styles, "size")) {
				d.insert("size", Value::Float(l.resolve(size) / size));
			}
			d.insert("kind", Value::str(if kind == ElemKind::Super { "super" } else { "sub" }));
			Ok(Some(res!(internal(body_of(elem), "shift-settings", Value::dict(d)))))
		}
		_ => Ok(None),
	}
}

/// Folds the text fields declared `Fold::Custom`: `strong`'s weight deltas add, `emph`'s toggles
/// cancel in pairs, decorations and features accumulate outermost first, variations overlay.
pub fn fold(_kind: ElemKind, field: &str, inner: Value, outer: Value) -> Outcome<Value> {
	Ok(match (field, inner, outer) {
		("delta", Value::Int(i), Value::Int(o))				=> Value::Int(i.saturating_add(o)),
		("emph", Value::Bool(i), Value::Bool(o))			=> Value::Bool(i ^ o),
		("deco" | "features", Value::Array(i), Value::Array(o))	=> {
			let mut v = (*o).clone();
			v.extend(i.iter().cloned());
			Value::array(v)
		}
		("features" | "variations", Value::Dict(i), Value::Dict(o))	=> {
			let mut d = (*o).clone();
			for (k, v) in i.iter() {
				d.insert(k, v.clone());
			}
			Value::dict(d)
		}
		(_, inner, _)										=> inner,
	})
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ LOREM                                                                      │
// └───────────────────────────────────────────────────────────────────────────┘

const LOREM_IPSUM:	&str = include_str!("../../../data/lipsum/lorem-ipsum.txt");
const LIBER_PRIMUS:	&str = include_str!("../../../data/lipsum/liber-primus.txt");

/// The order-two Markov chain `lipsum` learns: each pair of words and the words seen after it, and the
/// pairs sorted, which a jump to a random state picks from.
struct Chain {
	map:	HashMap<(&'static str, &'static str), Vec<&'static str>>,
	keys:	Vec<(&'static str, &'static str)>,
}

fn chain() -> &'static Chain {
	static CHAIN: OnceLock<Chain> = OnceLock::new();
	CHAIN.get_or_init(|| {
		let mut map: HashMap<(&'static str, &'static str), Vec<&'static str>> = HashMap::new();
		for text in [LOREM_IPSUM, LIBER_PRIMUS] {
			let words: Vec<&'static str> = text.split_whitespace().collect();
			for w in words.windows(3) {
				map.entry((w[0], w[1])).or_default().push(w[2]);
			}
		}
		let mut keys: Vec<(&'static str, &'static str)> = map.keys().cloned().collect();
		keys.sort_unstable();
		Chain { map, keys }
	})
}

/// `rand_chacha`'s `ChaCha20Rng` seeded by `seed_from_u64`, as `lipsum` seeds it (97), drawing 32-bit
/// words in its block order.
struct ChaCha20 {
	key:	[u32; 8],
	block:	u64,
	buf:	[u32; 64],
	index:	usize,
}

impl ChaCha20 {
	fn seed_from_u64(mut state: u64) -> Self {
		const MUL: u64 = 6_364_136_223_846_793_005;
		const INC: u64 = 11_634_580_027_462_260_723;
		let mut key = [0u32; 8];
		for k in key.iter_mut() {
			state = state.wrapping_mul(MUL).wrapping_add(INC);
			let xorshifted = (((state >> 18) ^ state) >> 27) as u32;
			let rot = (state >> 59) as u32;
			// The PCG output as little-endian bytes, read back as a little-endian key word.
			*k = xorshifted.rotate_right(rot);
		}
		Self { key, block: 0, buf: [0; 64], index: 64 }
	}

	fn refill(&mut self) {
		for b in 0..4 {
			let ctr = self.block.wrapping_add(b as u64);
			let init: [u32; 16] = [
				0x6170_7865, 0x3320_646e, 0x7962_2d32, 0x6b20_6574,
				self.key[0], self.key[1], self.key[2], self.key[3],
				self.key[4], self.key[5], self.key[6], self.key[7],
				ctr as u32, (ctr >> 32) as u32, 0, 0,
			];
			let mut x = init;
			for _ in 0..10 {
				qr(&mut x, 0, 4, 8, 12);
				qr(&mut x, 1, 5, 9, 13);
				qr(&mut x, 2, 6, 10, 14);
				qr(&mut x, 3, 7, 11, 15);
				qr(&mut x, 0, 5, 10, 15);
				qr(&mut x, 1, 6, 11, 12);
				qr(&mut x, 2, 7, 8, 13);
				qr(&mut x, 3, 4, 9, 14);
			}
			for i in 0..16 {
				self.buf[b * 16 + i] = x[i].wrapping_add(init[i]);
			}
		}
		self.block = self.block.wrapping_add(4);
		self.index = 0;
	}

	fn next_u32(&mut self) -> u32 {
		if self.index >= 64 {
			self.refill();
		}
		let v = self.buf[self.index];
		self.index += 1;
		v
	}

	/// An index below `n`, as `rand` 0.8's `gen_range(0..n)` draws one: widening multiply, rejecting
	/// the low part above the zone.
	fn below(&mut self, n: u32) -> u32 {
		if n == 0 {
			return self.next_u32();
		}
		let zone = (n << n.leading_zeros()).wrapping_sub(1);
		loop {
			let v = self.next_u32();
			let m = (v as u64) * (n as u64);
			let (hi, lo) = ((m >> 32) as u32, m as u32);
			if lo <= zone {
				return hi;
			}
		}
	}
}

fn qr(x: &mut [u32; 16], a: usize, b: usize, c: usize, d: usize) {
	x[a] = x[a].wrapping_add(x[b]); x[d] ^= x[a]; x[d] = x[d].rotate_left(16);
	x[c] = x[c].wrapping_add(x[d]); x[b] ^= x[c]; x[b] = x[b].rotate_left(12);
	x[a] = x[a].wrapping_add(x[b]); x[d] ^= x[a]; x[d] = x[d].rotate_left(8);
	x[c] = x[c].wrapping_add(x[d]); x[b] ^= x[c]; x[b] = x[b].rotate_left(7);
}

/// The next word of the chain from `state`, jumping to a random state where the chain has none.
fn next_word(ch: &Chain, state: &mut (&'static str, &'static str), rng: &mut ChaCha20) -> Option<&'static str> {
	if ch.map.is_empty() {
		return None;
	}
	let result = state.0;
	while !ch.map.contains_key(state) {
		let k = rng.below(ch.keys.len() as u32) as usize;
		*state = *ch.keys.get(k)?;
	}
	let words = ch.map.get(state)?;
	let w = *words.get(rng.below(words.len() as u32) as usize)?;
	*state = (state.1, w);
	Some(result)
}

/// `n` words of blind text, as Typst's `lorem(n)`.
pub fn lorem(n: usize) -> String {
	if n == 0 {
		return String::new();
	}
	let ch = chain();
	let mut rng = ChaCha20::seed_from_u64(97);
	let mut state: (&'static str, &'static str) = ("Lorem", "ipsum");
	let ends = |s: &str| s.ends_with(['.', '!', '?']);
	let mut out = String::new();
	let mut count = 0;
	let mut needs_cap = false;
	while count < n {
		let word = match next_word(ch, &mut state, &mut rng) {
			Some(w)	=> w,
			None	=> break,
		};
		if count > 0 {
			out.push(' ');
		}
		if word == "--" {
			out.push('\u{2013}');
			continue;
		}
		if needs_cap {
			let mut cs = word.chars();
			if let Some(c) = cs.next() {
				out.extend(c.to_uppercase());
				out.push_str(cs.as_str());
			}
		} else {
			out.push_str(word);
		}
		needs_cap = ends(&out);
		count += 1;
	}
	if !ends(&out) {
		let keep = out.trim_end_matches(|c: char| c.is_ascii_punctuation()).len();
		out.truncate(keep);
		out.push('.');
	}
	out
}
