// U5 owns this file: outline and outline.entry, with the entry's scoped functions (`indented`, `prefix`,
// `inner`, `body`, `page`) a show rule on entries calls.
//
// Typst aligns entries of one level by their widest prefix through an introspected `PrefixInfo` element
// per entry. Here the outline measures its own entries' prefixes when it is shown and carries the widest
// inset per level on the copy of itself that its entries see as `outline.entry.parent`, which answers the
// same question without an extra element kind. An entry's page number is `counter(page)` at the element,
// which is U8's to answer.

use crate::diag::DiagnosticKind;
use crate::eval::args::Args;
use crate::eval::content::{
	Content,
	ElemKind,
	FieldDefault,
	FieldSpec,
	FieldType,
};
use crate::eval::intro::{
	Counter,
	CounterKey,
};
use crate::eval::lib::model::common::{
	self,
	expect,
	natural,
	to_content,
	CastErr,
	K,
};
use crate::eval::lib::model::{
	figure,
	link,
	lookup,
	ModelFn,
};
use crate::eval::select::Selector;
use crate::eval::styles::{
	StyleChain,
	Styles,
};
use crate::eval::value::{
	Fraction,
	Length,
	Ratio,
	Relative,
	Value,
};
use crate::eval::Engine;
use crate::flow::{
	self,
	Region,
};
use crate::ir::Sp;
use crate::syntax::Span;

use oxedyne_fe2o3_core::prelude::*;

use std::sync::Arc;

const ANY: FieldType = FieldType::Any;

static OUTLINE: [FieldSpec; 5] = [
	FieldSpec::named("title",			ANY,	FieldDefault::Auto),
	FieldSpec::named("target",			ANY,	FieldDefault::Computed),
	FieldSpec::named("depth",			ANY,	FieldDefault::None),
	FieldSpec::named("indent",			ANY,	FieldDefault::Auto),
	FieldSpec::named("prefix-widths",	ANY,	FieldDefault::EmptyArray).synthesised().internal(),
];

static ENTRY: [FieldSpec; 4] = [
	FieldSpec::required("level",		ANY),
	FieldSpec::required("element",		ANY),
	FieldSpec::named("fill",			ANY,	FieldDefault::Computed),
	FieldSpec::named("parent",			ANY,	FieldDefault::None).internal(),
];

pub fn fields(kind: ElemKind) -> &'static [FieldSpec] {
	match kind {
		ElemKind::Outline		=> &OUTLINE,
		ElemKind::OutlineEntry	=> &ENTRY,
		_						=> &[],
	}
}

pub fn default_value(kind: ElemKind, name: &str) -> Option<Value> {
	match (kind, name) {
		(ElemKind::Outline, "target") => Some(Value::Selector(Arc::new(Selector::Elem(ElemKind::Heading, None)))),
		(ElemKind::OutlineEntry, "fill") => {
			// `repeat([.], gap: 0.15em)`, with whichever of those fields `repeat`'s schema has.
			let r = ElemKind::Repeat;
			let mut fields = Vec::new();
			if let Some(id) = r.field_id("body") {
				fields.push((id, Value::Content(common::text("."))));
			}
			if let Some(id) = r.field_id("gap") {
				fields.push((id, common::em(0.15)));
			}
			Some(Value::Content(Content::new(r, fields, Span::detached())))
		}
		_ => None,
	}
}

pub fn cast(kind: ElemKind, name: &str, v: Value) -> Result<Value, CastErr> {
	match (kind, name) {
		(ElemKind::Outline, "title") => match v {
			Value::Auto | Value::None	=> Ok(v),
			other						=> expect(&other, &[K::Content, K::None, K::Auto]).map(|_| to_content(other)),
		},
		(ElemKind::Outline, "target")	=> target(v),
		(ElemKind::Outline, "depth")	=> match v {
			Value::None	=> Ok(v),
			Value::Int(_)	=> natural(&v, false).map(|_| v),
			other		=> Err(CastErr::Type(common::mismatch("integer or none", &other))),
		},
		(ElemKind::Outline, "indent")	=> expect(&v, &[K::Rel, K::Func, K::Auto]).map(|_| common::relative(v)),
		(ElemKind::OutlineEntry, "level")	=> natural(&v, false).map(|_| v),
		(ElemKind::OutlineEntry, "element")	=> expect(&v, &[K::Content]).map(|_| to_content(v)),
		(ElemKind::OutlineEntry, "fill")	=> match v {
			Value::None	=> Ok(v),
			other		=> expect(&other, &[K::Content, K::None]).map(|_| to_content(other)),
		},
		_ => Ok(v),
	}
}

/// What an outline lists: a label, an element function, a location or a selector, kept as a selector.
fn target(v: Value) -> Result<Value, CastErr> {
	let sel = match &v {
		Value::Selector(_)	=> return Ok(v),
		Value::Label(l)		=> Selector::Label(l.clone()),
		Value::Location(l)	=> Selector::Location(*l),
		Value::Func(f)		=> match f.element() {
			Some(k)	=> Selector::Elem(k, None),
			None	=> return Err(CastErr::Value("only element functions can be used as selectors".to_string())),
		},
		other => return Err(CastErr::Type(common::mismatch("label, function, location, or selector", other))),
	};
	Ok(Value::Selector(Arc::new(sel)))
}

/// Headings in an outline are not themselves outlined or numbered, the outline is not justified, and
/// it opens at the paragraph leading. Its entries find the outline through `outline.entry.parent`.
pub fn show_set(elem: &Content, styles: &StyleChain) -> Outcome<Styles> {
	let leading = res!(common::style(styles, ElemKind::Par, "leading"));
	common::props(vec![
		(ElemKind::Heading,			"outlined",	Value::Bool(false)),
		(ElemKind::Heading,			"numbering",	Value::None),
		(ElemKind::Par,				"justify",	Value::Bool(false)),
		(ElemKind::Block,			"above",	leading),
		(ElemKind::OutlineEntry,	"parent",	Value::Content(elem.clone())),
	])
}

// Outlinable elements

/// Is the element listed in an outline, and at which level? `None` for an element that cannot be
/// outlined at all.
fn outlinable(e: &Content) -> Option<(bool, i64)> {
	match e.kind() {
		Some(ElemKind::Heading) => {
			let level = match e.field("level") { Some(Value::Int(l)) => *l, _ => 1 };
			Some((!matches!(e.field("outlined"), Some(Value::Bool(false))), level))
		}
		Some(ElemKind::Figure)		=> Some((figure::outlined(e), 1)),
		Some(ElemKind::Equation)	=> {
			let block = matches!(e.field("block"), Some(Value::Bool(true)));
			let numbered = !matches!(e.field("numbering"), None | Some(Value::None));
			Some((block && numbered, 1))
		}
		_ => None,
	}
}

fn entry_element(engine: &mut Engine, entry: &Content, span: Span) -> Outcome<Content> {
	match entry.field("element") {
		Some(Value::Content(c))	=> Ok(c.clone()),
		_						=> Err(engine.error(DiagnosticKind::Type, span, "missing argument: element")),
	}
}

fn cannot(engine: &mut Engine, e: &Content, span: Span) -> Error<ErrTag> {
	let name = e.kind().map(|k| k.name()).unwrap_or("sequence");
	engine.error(DiagnosticKind::Type, span, fmt!("cannot outline {}", name))
}

/// The entry's prefix: the element's number in its numbering, with a figure's supplement; `none` for
/// an unnumbered element.
fn prefix(engine: &mut Engine, entry: &Content, span: Span) -> Outcome<Option<Content>> {
	let e = res!(entry_element(engine, entry, span));
	if outlinable(&e).is_none() {
		return Err(cannot(engine, &e, span));
	}
	let numbering = match e.field("numbering") {
		None | Some(Value::None)	=> return Ok(None),
		Some(n)						=> n.clone(),
	};
	let loc = res!(element_location(engine, &e, span));
	let counter = match (e.kind(), e.field("counter")) {
		(Some(ElemKind::Figure), Some(Value::Counter(c)))	=> (**c).clone(),
		(Some(k), _)										=> lookup::elem_counter(k),
		_													=> return Err(cannot(engine, &e, span)),
	};
	let numbers = res!(lookup::display_counter(engine, &counter, loc, &numbering, span));
	Ok(Some(match e.kind() {
		Some(ElemKind::Heading)	=> numbers,
		_						=> figure::prefix(&e, numbers),
	}))
}

fn element_location(engine: &mut Engine, e: &Content, span: Span) -> Outcome<crate::eval::locate::Location> {
	match e.location() {
		Some(l)	=> Ok(l),
		None if outlinable(e).is_some() => {
			let name = e.kind().map(|k| k.name()).unwrap_or("element");
			Err(engine.error_hint(DiagnosticKind::Type, span, fmt!("{} must have a location", name),
				"try using a show rule to customize the outline.entry instead"))
		}
		None => Err(cannot(engine, e, span)),
	}
}

/// The entry's body: a heading's body, a figure's caption body, nothing for an equation.
fn body(engine: &mut Engine, entry: &Content, span: Span) -> Outcome<Content> {
	let e = res!(entry_element(engine, entry, span));
	match e.kind() {
		Some(ElemKind::Heading)		=> Ok(common::body(&e, "body")),
		Some(ElemKind::Figure)		=> Ok(figure::caption_body(&e)),
		Some(ElemKind::Equation)	=> Ok(Content::empty()),
		_							=> Err(cannot(engine, &e, span)),
	}
}

/// The page the element is on, in the page numbering there (`1` when the page has none).
fn page(engine: &mut Engine, entry: &Content, span: Span) -> Outcome<Content> {
	let e = res!(entry_element(engine, entry, span));
	let loc = res!(element_location(engine, &e, span));
	let counter = Counter { key: CounterKey::Page };
	lookup::display_counter(engine, &counter, loc, &Value::str("1"), span)
}

/// Body, then the fill stretched to the line's end (or plain fractional space), then the page number,
/// joined to what precedes it so it never stands alone on a line.
fn inner(engine: &mut Engine, entry: &Content, styles: &StyleChain, body: Content, page: Content) -> Outcome<Content> {
	let span = entry.span();
	let rtl = common::is_rtl(styles);
	let mut seq = Vec::new();
	if rtl {
		seq.push(common::text("\u{202B}"));
	}
	seq.push(body);
	if rtl {
		seq.push(common::text("\u{202C}"));
	}
	match res!(common::get(entry, styles, "fill")) {
		Value::None => seq.push(res!(common::h(engine, Value::Fraction(Fraction(1.0)), false))),
		filler => {
			let space = Content::marker(ElemKind::Space, Span::detached());
			seq.push(space.clone());
			seq.push(res!(common::build(engine, ElemKind::Box, span, vec![filler], vec![
				("width", Value::Fraction(Fraction(1.0))),
			])));
			seq.push(space);
		}
	}
	seq.push(common::text("\u{2060}"));
	seq.push(page);
	Ok(common::seq(seq))
}

/// A relative length's parts in points, its em part resolved.
fn rel_parts(styles: &StyleChain, v: &Value) -> (f64, f64) {
	match v {
		Value::Length(l)	=> (styles.resolve_length(*l), 0.0),
		Value::Ratio(r)		=> (0.0, r.0),
		Value::Relative(r)	=> (styles.resolve_length(r.abs), r.rel.0),
		_					=> (0.0, 0.0),
	}
}

fn rel_value(abs: f64, rel: f64) -> Value {
	if rel == 0.0 {
		Value::Length(Length::pt(abs))
	} else {
		Value::Relative(Relative { abs: Length::pt(abs), rel: Ratio(rel) })
	}
}

fn infinite() -> Region {
	Region {
		width:		Sp(i32::MAX),
		height:		Sp(i32::MAX),
		base:		(Sp(i32::MAX), Sp(i32::MAX)),
		expand_x:	false,
		expand_y:	false,
	}
}

/// `entry.indented(prefix, inner, gap: 0.5em)`: the entry as a block indented by its level, the prefix
/// hanging in the indent so bodies of a level align.
fn indented(
	engine:		&mut Engine,
	entry:		&Content,
	styles:		&StyleChain,
	prefix:		Option<Content>,
	inner:		Content,
	gap:		Value,
	span:		Span,
)
	-> Outcome<Content>
{
	let parent = match common::style_or(styles, ElemKind::OutlineEntry, "parent", Value::None) {
		Value::Content(p)	=> p,
		_					=> return Err(engine.error(DiagnosticKind::Type, span, "must be called within the context of an outline")),
	};
	let level = match entry.field("level") {
		Some(Value::Int(l))	=> (*l).max(1),
		_					=> 1,
	};
	let prefix_width = match &prefix {
		Some(p)	=> Some(res!(flow::measure(engine, p, styles, infinite())).width.to_pt()),
		None	=> None,
	};
	let prefix_inset = prefix_width.map(|w| w + common::resolve_pt(styles, &gap));
	let indent = res!(common::get(&parent, styles, "indent"));
	let (base, hang): ((f64, f64), Option<f64>) = match &indent {
		Value::Auto => {
			let widths: Vec<Option<f64>> = match parent.field("prefix-widths") {
				Some(Value::Array(a)) => a.iter().map(|v| match v {
					Value::Length(l)	=> Some(l.abs),
					_					=> None,
				}).collect(),
				_ => Vec::new(),
			};
			let fallback = common::resolve_pt(styles, &common::em(1.2));
			let get = |i: usize| widths.get(i).copied().flatten().unwrap_or(fallback);
			let last = (level - 1) as usize;
			let base: f64 = (0..last).map(get).sum();
			(( base, 0.0), prefix_inset.map(|p| p.max(get(last))))
		}
		Value::Func(f) => {
			let mut args = Args::new(span);
			args.push(span, Value::Int(level - 1));
			let v = res!(engine.call_func(f, args));
			if let Err(e) = expect(&v, &[K::Rel]) {
				return Err(engine.error(DiagnosticKind::Type, span, e.message()));
			}
			(rel_parts(styles, &v), prefix_inset)
		}
		amount => {
			let (a, r) = rel_parts(styles, amount);
			let depth = (level - 1) as f64;
			((a * depth, r * depth), prefix_inset)
		}
	};
	let body = match (prefix, prefix_width, prefix_inset, hang) {
		(Some(p), Some(w), Some(_), Some(hang)) => {
			let pull = res!(common::h(engine, common::pt(-hang), false));
			let skip = res!(common::h(engine, common::pt(hang - w), false));
			common::seq(vec![pull, p, skip, inner])
		}
		_ => inner,
	};
	let inset = common::start_side(styles, rel_value(base.0 + hang.unwrap_or(0.0), base.1));
	common::block(engine, body, span, vec![("inset", inset)])
}

pub fn show(engine: &mut Engine, elem: &Content, styles: &StyleChain) -> Outcome<Option<Content>> {
	match elem.kind() {
		Some(ElemKind::OutlineEntry)	=> show_entry(engine, elem, styles).map(Some),
		_								=> show_outline(engine, elem, styles).map(Some),
	}
}

fn show_outline(engine: &mut Engine, elem: &Content, styles: &StyleChain) -> Outcome<Content> {
	let span = elem.span();
	let mut seq = Vec::new();
	let title = match res!(common::get(elem, styles, "title")) {
		Value::Auto	=> Some(common::text(common::local(styles, "outline"))),
		Value::None	=> None,
		v			=> Some(common::display(v)),
	};
	if let Some(t) = title {
		seq.push(res!(common::raw_elem(ElemKind::Heading, span, vec![
			("depth",	Value::Int(1)),
			("body",	Value::Content(common::spanned(t, span))),
		])));
	}
	let target = match res!(common::get(elem, styles, "target")) {
		Value::Selector(s)	=> (*s).clone(),
		_					=> Selector::Elem(ElemKind::Heading, None),
	};
	let depth = match res!(common::get(elem, styles, "depth")) {
		Value::Int(d)	=> d,
		_				=> i64::MAX,
	};
	let found = res!(lookup::query(engine, &target, span));
	let mut entries = Vec::new();
	for e in found {
		let (outlined, level) = match outlinable(&e) {
			Some(x)	=> x,
			None	=> return Err(cannot(engine, &e, span)),
		};
		if outlined && level <= depth {
			entries.push(res!(common::raw_elem(ElemKind::OutlineEntry, span, vec![
				("level",	Value::Int(level)),
				("element",	Value::Content(e)),
			])));
		}
	}
	// The widest prefix inset of each level, which auto-indented entries align to.
	let mut widths: Vec<Option<f64>> = Vec::new();
	if res!(common::get(elem, styles, "indent")).is_auto() {
		let gap = common::resolve_pt(styles, &common::em(0.5));
		for entry in &entries {
			if let Some(p) = res!(prefix(engine, entry, span)) {
				let w = res!(flow::measure(engine, &p, styles, infinite())).width.to_pt() + gap;
				let level = match entry.field("level") { Some(Value::Int(l)) => (*l).max(1) as usize, _ => 1 };
				if widths.len() < level {
					widths.resize(level, None);
				}
				let slot = &mut widths[level - 1];
				*slot = Some(slot.map_or(w, |m: f64| m.max(w)));
			}
		}
	}
	let widths_value = Value::array(widths.into_iter().map(|w| match w {
		Some(p)	=> common::pt(p),
		None	=> Value::None,
	}).collect());
	let mut parent = elem.clone();
	parent.set(res!(common::fid(ElemKind::Outline, "prefix-widths")), widths_value);
	let body = res!(common::set(common::seq(entries), ElemKind::OutlineEntry, "parent", Value::Content(parent)));
	seq.push(body);
	Ok(common::seq(seq))
}

fn show_entry(engine: &mut Engine, elem: &Content, styles: &StyleChain) -> Outcome<Content> {
	let span = elem.span();
	let e = res!(entry_element(engine, elem, span));
	let pre = res!(prefix(engine, elem, span));
	let b = res!(body(engine, elem, span));
	let p = res!(page(engine, elem, span));
	let inner = res!(inner(engine, elem, styles, b, p));
	let block = if e.is(ElemKind::Equation) {
		let body = common::seq(vec![pre.unwrap_or_else(Content::empty), inner]);
		res!(common::block(engine, body, span, Vec::new()))
	} else {
		res!(indented(engine, elem, styles, pre, inner, common::em(0.5), span))
	};
	let loc = res!(element_location(engine, &e, span));
	link::to_location(block, loc)
}

// Scoped functions

pub fn method(name: &str) -> Option<ModelFn> {
	match name {
		"indented"	=> Some(ModelFn::EntryIndented),
		"prefix"	=> Some(ModelFn::EntryPrefix),
		"inner"		=> Some(ModelFn::EntryInner),
		"body"		=> Some(ModelFn::EntryBody),
		"page"		=> Some(ModelFn::EntryPage),
		_			=> None,
	}
}

/// Runs an entry method; the receiver is the first positional argument, and all but `body` need the
/// context's styles, as Typst's contextual functions do.
pub fn call(f: ModelFn, engine: &mut Engine, mut args: Args) -> Outcome<Value> {
	let span = args.span;
	let entry = match res!(args.eat::<Value>()) {
		Some(Value::Content(c)) if c.is(ElemKind::OutlineEntry)	=> c,
		_ => return Err(engine.error(DiagnosticKind::Type, span, "expected outline entry")),
	};
	if f == ModelFn::EntryBody {
		res!(super::finish(engine, &mut args));
		return Ok(Value::Content(res!(body(engine, &entry, span))));
	}
	let styles = match &engine.context.styles {
		Some(s)	=> s.clone(),
		None	=> return Err(engine.error_hint(DiagnosticKind::Type, span, "can only be used when context is known",
			"try wrapping this in a `context` expression")),
	};
	let out = match f {
		ModelFn::EntryPrefix => {
			res!(super::finish(engine, &mut args));
			match res!(prefix(engine, &entry, span)) {
				Some(c)	=> Value::Content(c),
				None	=> Value::None,
			}
		}
		ModelFn::EntryPage => {
			res!(super::finish(engine, &mut args));
			Value::Content(res!(page(engine, &entry, span)))
		}
		ModelFn::EntryInner => {
			res!(super::finish(engine, &mut args));
			let b = res!(body(engine, &entry, span));
			let p = res!(page(engine, &entry, span));
			Value::Content(res!(inner(engine, &entry, &styles, b, p)))
		}
		_ => {
			let pre: Option<Content> = res!(args.eat::<Option<Content>>()).flatten();
			let inner = match res!(args.eat::<Content>()) {
				Some(c)	=> c,
				None	=> return Err(engine.error(DiagnosticKind::Type, span, "missing argument: inner")),
			};
			let gap = match res!(args.named::<Value>("gap")) {
				Some(g)	=> g,
				None	=> common::em(0.5),
			};
			res!(super::finish(engine, &mut args));
			Value::Content(res!(indented(engine, &entry, &styles, pre, inner, gap, span)))
		}
	};
	Ok(out)
}

