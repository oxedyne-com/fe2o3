// U5 owns this file: figure and figure.caption. Synthesis settles what a figure is (its kind from the
// first image, table or raw text in its body), what it is called (the kind's word in the language in
// force) and what counts it (`counter(figure.where(kind: ..))`), and hands all three to the caption,
// with the figure's location, so the caption can number itself.

use crate::eval::content::{
	Content,
	ElemKind,
	FieldDefault,
	FieldSpec,
	FieldType,
};
use crate::eval::func::Func;
use crate::eval::intro::{
	Counter,
	CounterKey,
};
use crate::eval::lib::model::common::{
	self,
	alignment_of,
	choice,
	expect,
	to_content,
	CastErr,
	K,
};
use crate::eval::lib::model::{
	heading,
	lookup,
};
use crate::eval::select::Selector;
use crate::eval::styles::{
	StyleChain,
	Styles,
};
use crate::eval::value::{
	Alignment,
	HAlign,
	Type,
	VAlign,
	Value,
};
use crate::eval::Engine;

use oxedyne_fe2o3_core::prelude::*;

const ANY: FieldType = FieldType::Any;

static FIGURE: [FieldSpec; 11] = [
	FieldSpec::required("body",			ANY),
	FieldSpec::named("alt",				FieldType::OneOf(&[Type::Str, Type::None]),	FieldDefault::None),
	FieldSpec::named("placement",		ANY,	FieldDefault::None),
	FieldSpec::named("scope",			ANY,	FieldDefault::Str("column")),
	FieldSpec::named("caption",			ANY,	FieldDefault::None),
	FieldSpec::named("kind",			ANY,	FieldDefault::Auto),
	FieldSpec::named("supplement",		ANY,	FieldDefault::Auto),
	FieldSpec::named("numbering",		ANY,	FieldDefault::Str("1")),
	FieldSpec::named("gap",				FieldType::Of(Type::Length),	FieldDefault::Em(0.65)),
	FieldSpec::named("outlined",		FieldType::Of(Type::Bool),		FieldDefault::Bool(true)),
	FieldSpec::named("counter",			ANY,	FieldDefault::None).synthesised(),
];

static CAPTION: [FieldSpec; 8] = [
	FieldSpec::named("position",		ANY,	FieldDefault::Computed),
	FieldSpec::named("separator",		ANY,	FieldDefault::Auto),
	FieldSpec::required("body",			ANY),
	FieldSpec::named("kind",			ANY,	FieldDefault::None).synthesised(),
	FieldSpec::named("supplement",		ANY,	FieldDefault::None).synthesised(),
	FieldSpec::named("numbering",		ANY,	FieldDefault::None).synthesised(),
	FieldSpec::named("counter",			ANY,	FieldDefault::None).synthesised(),
	FieldSpec::named("figure-location",	ANY,	FieldDefault::None).synthesised().internal(),
];

pub fn fields(kind: ElemKind) -> &'static [FieldSpec] {
	match kind {
		ElemKind::Figure		=> &FIGURE,
		ElemKind::FigureCaption	=> &CAPTION,
		_						=> &[],
	}
}

pub fn default_value(kind: ElemKind, name: &str) -> Option<Value> {
	match (kind, name) {
		(ElemKind::FigureCaption, "position")	=> Some(Value::Alignment(Alignment { x: None, y: Some(VAlign::Bottom) })),
		_										=> None,
	}
}

pub fn cast(kind: ElemKind, name: &str, v: Value) -> Result<Value, CastErr> {
	match (kind, name) {
		(ElemKind::Figure, "body")			=> expect(&v, &[K::Content]).map(|_| to_content(v)),
		(ElemKind::Figure, "alt")			=> expect(&v, &[K::Str, K::None]).map(|_| v),
		(ElemKind::Figure, "placement")		=> match v {
			Value::None | Value::Auto	=> Ok(v),
			other						=> alignment_of(&other, &["top", "horizon", "bottom"]).map(|_| other),
		},
		(ElemKind::Figure, "scope")			=> choice(&v, &["column", "parent"], false, false).map(|_| v),
		(ElemKind::Figure, "caption")		=> match v {
			Value::None	=> Ok(v),
			other		=> {
				ok!(expect(&other, &[K::Content, K::None]));
				caption_of(to_content(other))
			}
		},
		(ElemKind::Figure, "kind")			=> match &v {
			Value::Func(f) if f.element().is_some()	=> Ok(v),
			Value::Str(_) | Value::Auto				=> Ok(v),
			other	=> Err(CastErr::Type(common::mismatch("function, string, or auto", other))),
		},
		(ElemKind::Figure, "supplement")	=> heading::supplement(v),
		(ElemKind::Figure, "numbering")		=> expect(&v, &[K::Str, K::Func, K::None]).map(|_| v),
		(ElemKind::Figure, "gap")			=> expect(&v, &[K::Length]).map(|_| v),
		(ElemKind::Figure, "outlined")		=> expect(&v, &[K::Bool]).map(|_| v),
		(ElemKind::FigureCaption, "position")	=> alignment_of(&v, &["top", "bottom"]).map(|_| v),
		(ElemKind::FigureCaption, "separator")	=> match v {
			Value::Auto	=> Ok(v),
			other		=> expect(&other, &[K::Content, K::Auto]).map(|_| to_content(other)),
		},
		(ElemKind::FigureCaption, "body")	=> expect(&v, &[K::Content]).map(|_| to_content(v)),
		_									=> Ok(v),
	}
}

/// Content as a caption: a caption as it is, anything else as a caption's body.
fn caption_of(v: Value) -> Result<Value, CastErr> {
	match v {
		Value::Content(c) if c.is(ElemKind::FigureCaption)	=> Ok(Value::Content(c)),
		Value::Content(c) => match common::raw_elem(ElemKind::FigureCaption, c.span(), vec![("body", Value::Content(c))]) {
			Ok(cap)	=> Ok(Value::Content(cap)),
			Err(e)	=> Err(CastErr::Value(e.to_string())),
		},
		other => Ok(other),
	}
}

/// The elements a figure takes its kind from, Typst's `Figurable`.
fn figurable(k: ElemKind) -> bool { matches!(k, ElemKind::Image | ElemKind::Table | ElemKind::Raw) }

/// The word key an element kind is named by, Typst's `LocalName`.
pub fn local_key(k: ElemKind) -> Option<&'static str> {
	match k {
		ElemKind::Image			=> Some("figure"),
		ElemKind::Table			=> Some("table"),
		ElemKind::Raw			=> Some("raw"),
		ElemKind::Equation		=> Some("equation"),
		ElemKind::Heading		=> Some("heading"),
		ElemKind::Outline		=> Some("outline"),
		ElemKind::Bibliography	=> Some("bibliography"),
		ElemKind::Footnote		=> Some("footnote"),
		_						=> None,
	}
}

pub fn synthesise(engine: &mut Engine, elem: &mut Content, styles: &StyleChain) -> Outcome<()> {
	let span = elem.span();
	let numbering = res!(common::get(elem, styles, "numbering"));
	let body = common::body(elem, "body");
	let kind = match res!(common::get(elem, styles, "kind")) {
		Value::Auto => {
			let found = common::find_first(&body, |c| c.kind().map(figurable).unwrap_or(false));
			let k = found.and_then(|c| c.kind()).unwrap_or(ElemKind::Image);
			Value::Func(Func::Element(k))
		}
		other => other,
	};
	let kind_elem = match &kind {
		Value::Func(f)	=> f.element(),
		_				=> None,
	};
	let supplement = match res!(common::get(elem, styles, "supplement")) {
		Value::Auto => {
			let name = kind_elem.and_then(local_key).map(|key| common::text(common::local(styles, key)));
			if !numbering.is_none() && name.is_none() {
				return Err(engine.error(span, "please specify the figure's supplement"));
			}
			Some(name.unwrap_or_else(Content::empty))
		}
		Value::None => None,
		other => {
			// A function receives the first element of the figure's kind, or the body.
			let target = kind_elem
				.and_then(|k| common::find_first(&body, |c| c.is(k)))
				.unwrap_or_else(|| body.clone());
			res!(heading::resolve_supplement(engine, &other, styles, "figure", &target))
		}
	};
	let counter = res!(figure_counter(&kind));
	let supplement_value = match &supplement {
		Some(c)	=> Value::Content(c.clone()),
		None	=> Value::None,
	};
	let caption = match res!(common::get(elem, styles, "caption")) {
		Value::Content(mut cap) => {
			res!(synthesise_caption(&mut cap, styles));
			for (name, v) in [
				("kind",			kind.clone()),
				("supplement",		supplement_value.clone()),
				("numbering",		numbering.clone()),
				("counter",			Value::Counter(std::sync::Arc::new(counter.clone()))),
				("figure-location",	elem.location().map(Value::Location).unwrap_or(Value::None)),
			] {
				cap.set(res!(common::fid(ElemKind::FigureCaption, name)), v);
			}
			Value::Content(cap)
		}
		other => other,
	};
	elem.set(res!(common::fid(ElemKind::Figure, "kind")), kind);
	elem.set(res!(common::fid(ElemKind::Figure, "supplement")), supplement_value);
	elem.set(res!(common::fid(ElemKind::Figure, "counter")), Value::Counter(std::sync::Arc::new(counter)));
	elem.set(res!(common::fid(ElemKind::Figure, "caption")), caption);
	Ok(())
}

/// `counter(figure.where(kind: k))`.
fn figure_counter(kind: &Value) -> Outcome<Counter> {
	let id = res!(common::fid(ElemKind::Figure, "kind"));
	Ok(Counter { key: CounterKey::Selector(Selector::Elem(ElemKind::Figure, Some(vec![(id, kind.clone())]))) })
}

/// A caption's separator: set, else the one the language in force uses.
pub fn synthesise_caption(elem: &mut Content, styles: &StyleChain) -> Outcome<()> {
	let sep = res!(separator(elem, styles));
	elem.set(res!(common::fid(ElemKind::FigureCaption, "separator")), Value::Content(sep));
	Ok(())
}

fn separator(elem: &Content, styles: &StyleChain) -> Outcome<Content> {
	match res!(common::get(elem, styles, "separator")) {
		Value::Auto => {
			let (lang, _) = common::lang(styles);
			Ok(common::text(match lang.as_str() {
				"zh"	=> "\u{2003}",
				"fr"	=> ".\u{a0}\u{2013} ",
				"ru"	=> ". ",
				_		=> ": ",
			}))
		}
		v => Ok(common::display(v)),
	}
}

/// A figure does not break across pages unless a rule says so, and is centred.
pub fn show_set() -> Outcome<Styles> {
	common::props(vec![
		(ElemKind::Block,	"breakable",	Value::Bool(false)),
		(ElemKind::Align,	"alignment",	Value::Alignment(Alignment { x: Some(HAlign::Center), y: None })),
	])
}

pub fn show(engine: &mut Engine, elem: &Content, styles: &StyleChain) -> Outcome<Option<Content>> {
	match elem.kind() {
		Some(ElemKind::FigureCaption)	=> show_caption(engine, elem, styles).map(Some),
		_								=> show_figure(engine, elem, styles).map(Some),
	}
}

fn show_figure(engine: &mut Engine, elem: &Content, styles: &StyleChain) -> Outcome<Content> {
	let span = elem.span();
	let mut realised = common::body(elem, "body");
	if let Value::Content(caption) = res!(common::get(elem, styles, "caption")) {
		let position = res!(common::get(&caption, styles, "position"));
		let top = matches!(position, Value::Alignment(Alignment { y: Some(VAlign::Top), .. }));
		let gap = res!(common::get(elem, styles, "gap"));
		let v = res!(common::build(engine, ElemKind::V, span, vec![gap], vec![("weak", Value::Bool(true))]));
		realised = if top {
			common::seq(vec![caption, v, realised])
		} else {
			common::seq(vec![realised, v, caption])
		};
	}
	// The body always counts as a paragraph of its own.
	realised = common::seq(vec![realised, Content::marker(ElemKind::Parbreak, span)]);
	realised = res!(common::block(engine, realised, span, Vec::new()));
	let scope = res!(common::get(elem, styles, "scope"));
	match res!(common::get(elem, styles, "placement")) {
		Value::None => {
			if matches!(&scope, Value::Str(s) if s.as_str() == "parent") {
				return Err(engine.error_hint(span,
					"parent-scoped placement is only available for floating figures",
					"you can enable floating placement with `figure(placement: auto, ..)`"));
			}
			Ok(realised)
		}
		placement => {
			let alignment = match placement {
				Value::Alignment(a)	=> Value::Alignment(Alignment { x: Some(HAlign::Center), y: a.y }),
				other				=> other,
			};
			common::build(engine, ElemKind::Place, span, vec![alignment, Value::Content(realised)], vec![
				("scope",	scope),
				("float",	Value::Bool(true)),
			])
		}
	}
}

fn show_caption(engine: &mut Engine, elem: &Content, styles: &StyleChain) -> Outcome<Content> {
	let span = elem.span();
	let mut realised = common::body(elem, "body");
	let parts = (
		elem.field("supplement").cloned(),
		elem.field("numbering").cloned(),
		elem.field("counter").cloned(),
		elem.field("figure-location").cloned(),
	);
	if let (Some(Value::Content(sup)), Some(numbering), Some(Value::Counter(counter)), Some(Value::Location(loc)))
		= parts
	{
		if !numbering.is_none() {
			let numbers = res!(lookup::display_counter(engine, &counter, loc, &numbering, span));
			let mut seq = vec![sup.clone()];
			if !sup.is_empty() {
				seq.push(common::text("\u{a0}"));
			}
			seq.push(numbers);
			seq.push(res!(separator(elem, styles)));
			seq.push(realised);
			realised = common::seq(seq);
		}
	}
	common::block(engine, realised, span, Vec::new())
}

/// Is the figure outlined: asked to be, and captioned or numbered?
pub fn outlined(elem: &Content) -> bool {
	let asked = !matches!(elem.field("outlined"), Some(Value::Bool(false)));
	let captioned = matches!(elem.field("caption"), Some(Value::Content(_)));
	let numbered = !matches!(elem.field("numbering"), None | Some(Value::None));
	asked && (captioned || numbered)
}

/// A figure's prefix in an outline or reference: its supplement, a no-break space, its numbers.
pub fn prefix(elem: &Content, numbers: Content) -> Content {
	match elem.field("supplement") {
		Some(Value::Content(s)) if !s.is_empty() => common::seq(vec![s.clone(), common::text("\u{a0}"), numbers]),
		_ => numbers,
	}
}

/// The figure's caption body, or nothing: what an outline lists for it.
pub fn caption_body(elem: &Content) -> Content {
	match elem.field("caption") {
		Some(Value::Content(c))	=> common::body(c, "body"),
		_						=> Content::empty(),
	}
}

