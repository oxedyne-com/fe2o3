// U5 owns this file: list, enum and terms, and their items. Realisation groups consecutive items into
// the list element; the list's show lays its items out.
//
// A bullet or numbered list is shown as a grid of four columns -- the indent, the markers (as wide as the
// widest), the body indent and the bodies -- with the item spacing as its row gutter. Typst 0.15 lays
// lists out with a dedicated layouter over the same geometry, which differs in two respects the grid
// cannot express: a marker is aligned to its body's first baseline rather than to the row's top (they
// coincide unless the body opens larger than the marker), and a list inside a container of automatic
// width sizes its bodies to the widest one. A term list is Typst's own realisation: a stack of padded
// blocks.
//
// Nesting is carried by style-only fields Typst keeps internal: `list.depth` picks the marker,
// `enum.parents` holds the enclosing numbers for `full` numbering, and `terms.within` marks a term list.
// Each is set to its absolute value for the items' bodies.

use crate::eval::args::Args;
use crate::eval::content::{
	Content,
	ElemKind,
	FieldDefault,
	FieldSpec,
	FieldType,
};
use crate::eval::lib::model::common::{
	self,
	expect,
	natural,
	to_content,
	CastErr,
	K,
};
use crate::eval::lib::numbering;
use crate::eval::styles::StyleChain;
use crate::eval::value::{
	Alignment,
	Fraction,
	HAlign,
	Type,
	Value,
};
use crate::eval::Engine;
use crate::syntax::Span;

use oxedyne_fe2o3_core::prelude::*;

const ANY:		FieldType = FieldType::Any;
const BOOL:		FieldType = FieldType::Of(Type::Bool);
const LENGTH:	FieldType = FieldType::Of(Type::Length);

static LIST: [FieldSpec; 8] = [
	FieldSpec::named("tight",			BOOL,	FieldDefault::Bool(true)),
	FieldSpec::named("marker",			ANY,	FieldDefault::Computed),
	FieldSpec::named("indent",			LENGTH,	FieldDefault::Pt(0.0)),
	FieldSpec::named("body-indent",		LENGTH,	FieldDefault::Em(0.5)),
	FieldSpec::named("spacing",			FieldType::OneOf(&[Type::Length, Type::Auto]),	FieldDefault::Auto),
	FieldSpec::named("marker-align",	ANY,	FieldDefault::Computed),
	FieldSpec::named("children",		ANY,	FieldDefault::EmptyArray).variadic().unsettable(),
	FieldSpec::named("depth",			ANY,	FieldDefault::Int(0)).unsettable(),
];

static LIST_ITEM: [FieldSpec; 1] = [
	FieldSpec::required("body",			ANY),
];

static ENUM: [FieldSpec; 11] = [
	FieldSpec::named("tight",			BOOL,	FieldDefault::Bool(true)),
	FieldSpec::named("numbering",		ANY,	FieldDefault::Str("1.")),
	FieldSpec::named("start",			ANY,	FieldDefault::Auto),
	FieldSpec::named("full",			BOOL,	FieldDefault::Bool(false)),
	FieldSpec::named("reversed",		BOOL,	FieldDefault::Bool(false)),
	FieldSpec::named("indent",			LENGTH,	FieldDefault::Pt(0.0)),
	FieldSpec::named("body-indent",		LENGTH,	FieldDefault::Em(0.5)),
	FieldSpec::named("spacing",			FieldType::OneOf(&[Type::Length, Type::Auto]),	FieldDefault::Auto),
	FieldSpec::named("number-align",	ANY,	FieldDefault::Computed),
	FieldSpec::named("children",		ANY,	FieldDefault::EmptyArray).variadic().unsettable(),
	FieldSpec::named("parents",			ANY,	FieldDefault::EmptyArray).unsettable(),
];

static ENUM_ITEM: [FieldSpec; 2] = [
	FieldSpec::named("number",			ANY,	FieldDefault::Auto).positional(),
	FieldSpec::required("body",			ANY),
];

static TERMS: [FieldSpec; 7] = [
	FieldSpec::named("tight",			BOOL,	FieldDefault::Bool(true)),
	FieldSpec::named("separator",		ANY,	FieldDefault::Computed),
	FieldSpec::named("indent",			LENGTH,	FieldDefault::Pt(0.0)),
	FieldSpec::named("hanging-indent",	LENGTH,	FieldDefault::Em(2.0)),
	FieldSpec::named("spacing",			FieldType::OneOf(&[Type::Length, Type::Auto]),	FieldDefault::Auto),
	FieldSpec::named("children",		ANY,	FieldDefault::EmptyArray).variadic().unsettable(),
	FieldSpec::named("within",			ANY,	FieldDefault::Bool(false)).unsettable(),
];

static TERM_ITEM: [FieldSpec; 2] = [
	FieldSpec::required("term",			ANY),
	FieldSpec::required("description",	ANY),
];

pub fn fields(kind: ElemKind) -> &'static [FieldSpec] {
	match kind {
		ElemKind::List		=> &LIST,
		ElemKind::ListItem	=> &LIST_ITEM,
		ElemKind::Enum		=> &ENUM,
		ElemKind::EnumItem	=> &ENUM_ITEM,
		ElemKind::Terms		=> &TERMS,
		ElemKind::TermItem	=> &TERM_ITEM,
		_					=> &[],
	}
}

pub fn default_value(kind: ElemKind, name: &str) -> Option<Value> {
	let end = Value::Alignment(Alignment { x: Some(HAlign::End), y: None });
	match (kind, name) {
		(ElemKind::List, "marker") => Some(Value::array(vec![
			Value::Content(common::text("\u{2022}")),	// bullet
			Value::Content(common::text("\u{2023}")),	// triangular bullet
			Value::Content(common::text("\u{2013}")),	// en dash
		])),
		(ElemKind::List, "marker-align")	=> Some(end),
		(ElemKind::Enum, "number-align")	=> Some(end),
		(ElemKind::Terms, "separator") => {
			// `h(0.6em, weak: true)`, with whichever of those fields `h`'s schema has.
			let h = ElemKind::H;
			let mut fields = Vec::new();
			if let Some(a) = h.field_id("amount") {
				fields.push((a, common::em(0.6)));
			}
			if let Some(w) = h.field_id("weak") {
				fields.push((w, Value::Bool(true)));
			}
			Some(Value::Content(Content::new(h, fields, Span::detached())))
		}
		_ => None,
	}
}

pub fn cast(kind: ElemKind, name: &str, v: Value) -> Result<Value, CastErr> {
	match (kind, name) {
		(_, "tight") | (ElemKind::Enum, "full") | (ElemKind::Enum, "reversed")
			=> expect(&v, &[K::Bool]).map(|_| v),
		(_, "indent") | (_, "body-indent") | (ElemKind::Terms, "hanging-indent")
			=> expect(&v, &[K::Length]).map(|_| v),
		(_, "spacing")					=> expect(&v, &[K::Length, K::Auto]).map(|_| v),
		(ElemKind::List, "marker")		=> marker(v),
		(ElemKind::List, "marker-align") | (ElemKind::Enum, "number-align")
			=> expect(&v, &[K::Alignment]).map(|_| v),
		(ElemKind::Enum, "numbering")	=> expect(&v, &[K::Str, K::Func]).map(|_| v),
		(ElemKind::Enum, "start") | (ElemKind::EnumItem, "number") => match v {
			Value::Auto	=> Ok(v),
			Value::Int(_)	=> natural(&v, true).map(|_| v),
			other		=> Err(CastErr::Type(common::mismatch("integer or auto", &other))),
		},
		(ElemKind::Terms, "separator")	=> expect(&v, &[K::Content]).map(|_| to_content(v)),
		(ElemKind::List, "children")	=> list_child(v),
		(ElemKind::Enum, "children")	=> enum_child(v),
		(ElemKind::Terms, "children")	=> term_child(v),
		(_, "body") | (_, "term") | (_, "description")
			=> expect(&v, &[K::Content]).map(|_| to_content(v)),
		_								=> Ok(v),
	}
}

fn marker(v: Value) -> Result<Value, CastErr> {
	match v {
		Value::Func(_)	=> Ok(v),
		Value::Array(a)	=> {
			if a.is_empty() {
				return Err(CastErr::Value("array must contain at least one marker".to_string()));
			}
			Ok(Value::array(a.iter().map(|x| Value::Content(common::display(x.clone()))).collect()))
		}
		other => expect(&other, &[K::Content, K::Array, K::Func]).map(|_| to_content(other)),
	}
}

fn list_child(v: Value) -> Result<Value, CastErr> {
	ok!(expect(&v, &[K::Content]));
	match to_content(v) {
		Value::Content(c) if c.is(ElemKind::ListItem)	=> Ok(Value::Content(c)),
		Value::Content(c)	=> item(ElemKind::ListItem, vec![("body", Value::Content(c))]),
		other				=> Ok(other),
	}
}

fn enum_child(v: Value) -> Result<Value, CastErr> {
	if let Value::Array(a) = &v {
		if a.len() != 2 {
			return Err(CastErr::Value("array must contain exactly two entries".to_string()));
		}
		let number = a[0].clone();
		match &number {
			Value::Auto		=> (),
			Value::Int(_)	=> ok!(natural(&number, true)),
			other			=> return Err(CastErr::Value(common::mismatch("integer or auto", other))),
		}
		ok!(expect(&a[1], &[K::Content]));
		return item(ElemKind::EnumItem, vec![("number", number), ("body", to_content(a[1].clone()))]);
	}
	ok!(expect(&v, &[K::Content, K::Array]));
	match to_content(v) {
		Value::Content(c) if c.is(ElemKind::EnumItem)	=> Ok(Value::Content(c)),
		Value::Content(c)	=> item(ElemKind::EnumItem, vec![("body", Value::Content(c))]),
		other				=> Ok(other),
	}
}

fn term_child(v: Value) -> Result<Value, CastErr> {
	if let Value::Array(a) = &v {
		if a.len() != 2 {
			return Err(CastErr::Value("array must contain exactly two entries".to_string()));
		}
		ok!(expect(&a[0], &[K::Content]));
		ok!(expect(&a[1], &[K::Content]));
		return item(ElemKind::TermItem, vec![
			("term",		to_content(a[0].clone())),
			("description",	to_content(a[1].clone())),
		]);
	}
	ok!(expect(&v, &[K::Content, K::Array]));
	match v {
		Value::Content(c) if c.is(ElemKind::TermItem)	=> Ok(Value::Content(c)),
		_ => Err(CastErr::Value("expected term item or array".to_string())),
	}
}

fn item(kind: ElemKind, fields: Vec<(&str, Value)>) -> Result<Value, CastErr> {
	match common::raw_elem(kind, Span::detached(), fields) {
		Ok(c)	=> Ok(Value::Content(c)),
		Err(e)	=> Err(CastErr::Value(e.to_string())),
	}
}

/// Is flow inside a list, an enumeration or a term list? A paragraph's first-line indent reads this.
pub fn in_list(styles: &StyleChain) -> bool {
	let depth = matches!(common::style_or(styles, ElemKind::List, "depth", Value::Int(0)), Value::Int(d) if d > 0);
	let parents = matches!(common::style_or(styles, ElemKind::Enum, "parents", Value::array(Vec::new())),
		Value::Array(a) if !a.is_empty());
	let within = matches!(common::style_or(styles, ElemKind::Terms, "within", Value::Bool(false)), Value::Bool(true));
	depth || parents || within
}

pub fn show(engine: &mut Engine, elem: &Content, styles: &StyleChain) -> Outcome<Option<Content>> {
	match elem.kind() {
		Some(ElemKind::List)		=> show_list(engine, elem, styles).map(Some),
		Some(ElemKind::Enum)		=> show_enum(engine, elem, styles).map(Some),
		Some(ElemKind::Terms)		=> show_terms(engine, elem, styles).map(Some),
		Some(ElemKind::TermItem)	=> Ok(Some(common::seq(vec![
			common::body(elem, "term"),
			common::body(elem, "description"),
		]))),
		_							=> Ok(Some(common::body(elem, "body"))),
	}
}

fn children(elem: &Content) -> Vec<Content> {
	match elem.field("children") {
		Some(Value::Array(a)) => a.iter().filter_map(|v| match v {
			Value::Content(c)	=> Some(c.clone()),
			_					=> None,
		}).collect(),
		_ => Vec::new(),
	}
}

/// The gap between items: `spacing` when set, else the paragraph leading for a tight list and the
/// paragraph spacing for a wide one.
fn gutter(elem: &Content, styles: &StyleChain, tight: bool) -> Outcome<Value> {
	match res!(common::get(elem, styles, "spacing")) {
		Value::Auto => Ok(if tight {
			res!(common::style(styles, ElemKind::Par, "leading"))
		} else {
			res!(common::style(styles, ElemKind::Par, "spacing"))
		}),
		v => Ok(v),
	}
}

/// A tight list keeps to the paragraph before it: weak spacing of the gutter, attached to it.
fn attach_tight(engine: &mut Engine, spacing: Value, realised: Content, span: Span) -> Outcome<Content> {
	let v = res!(common::build(engine, ElemKind::V, span, vec![spacing], vec![
		("weak",	Value::Bool(true)),
		("attach",	Value::Bool(true)),
	]));
	Ok(common::seq(vec![v, realised]))
}

/// Items as the four-column grid: indent, marker, body indent, body.
fn item_grid(
	engine:		&mut Engine,
	elem:		&Content,
	styles:		&StyleChain,
	rows:		Vec<(Content, Content)>,
	gutter:		Value,
)
	-> Outcome<Content>
{
	let span = elem.span();
	let indent = res!(common::get(elem, styles, "indent"));
	let body_indent = res!(common::get(elem, styles, "body-indent"));
	let columns = Value::array(vec![indent, Value::Auto, body_indent, Value::Fraction(Fraction(1.0))]);
	let mut cells = Vec::with_capacity(rows.len() * 4);
	for (marker, body) in rows {
		cells.push(Value::Content(Content::empty()));
		cells.push(Value::Content(marker));
		cells.push(Value::Content(Content::empty()));
		cells.push(Value::Content(body));
	}
	common::build(engine, ElemKind::Grid, span, cells, vec![
		("columns",		columns),
		("row-gutter",	gutter),
	])
}

fn show_list(engine: &mut Engine, elem: &Content, styles: &StyleChain) -> Outcome<Content> {
	let span = elem.span();
	let tight = matches!(res!(common::get(elem, styles, "tight")), Value::Bool(true));
	let gutter = res!(gutter(elem, styles, tight));
	let depth = match common::style_or(styles, ElemKind::List, "depth", Value::Int(0)) {
		Value::Int(d)	=> d.max(0),
		_				=> 0,
	};
	let align = res!(common::get(elem, styles, "marker-align"));
	let marker = match res!(common::get(elem, styles, "marker")) {
		Value::Array(a) => match a.get(depth as usize % a.len().max(1)) {
			Some(m)	=> common::display(m.clone()),
			None	=> Content::empty(),
		},
		Value::Func(f) => {
			let mut args = Args::new(span);
			args.push(span, Value::Int(depth));
			let saved = std::mem::replace(&mut engine.context, crate::eval::Context {
				location:	None,
				styles:		Some(styles.clone()),
			});
			let out = engine.call_func(&f, args);
			engine.context = saved;
			res!(common::shown(engine, res!(out), span))
		}
		v => common::display(v),
	};
	let marker = res!(common::build(engine, ElemKind::Align, span, vec![align, Value::Content(marker)], Vec::new()));
	let mut rows = Vec::new();
	for item in children(elem) {
		let mut body = common::body(&item, "body");
		if !tight {
			body = common::seq(vec![body, Content::marker(ElemKind::Parbreak, Span::detached())]);
		}
		let body = res!(common::set(body, ElemKind::List, "depth", Value::Int(depth + 1)));
		rows.push((marker.clone(), body));
	}
	let realised = res!(item_grid(engine, elem, styles, rows, gutter.clone()));
	if tight {
		return attach_tight(engine, gutter, realised, span);
	}
	Ok(realised)
}

fn show_enum(engine: &mut Engine, elem: &Content, styles: &StyleChain) -> Outcome<Content> {
	let span = elem.span();
	let tight = matches!(res!(common::get(elem, styles, "tight")), Value::Bool(true));
	let gutter = res!(gutter(elem, styles, tight));
	let numbering = res!(common::get(elem, styles, "numbering"));
	let reversed = matches!(res!(common::get(elem, styles, "reversed")), Value::Bool(true));
	let full = matches!(res!(common::get(elem, styles, "full")), Value::Bool(true));
	let align = res!(common::get(elem, styles, "number-align"));
	let items = children(elem);
	let mut number: u64 = match res!(common::get(elem, styles, "start")) {
		Value::Int(n)	=> n.max(0) as u64,
		_				=> if reversed { items.len() as u64 } else { 1 },
	};
	let parents: Vec<u64> = match common::style_or(styles, ElemKind::Enum, "parents", Value::array(Vec::new())) {
		Value::Array(a) => a.iter().filter_map(|v| match v {
			Value::Int(i)	=> Some(*i as u64),
			_				=> None,
		}).collect(),
		_ => Vec::new(),
	};
	let mut rows = Vec::new();
	for item in items {
		if let Some(Value::Int(n)) = item.field("number") {
			number = (*n).max(0) as u64;
		}
		let resolved = if full {
			let mut all = parents.clone();
			all.push(number);
			let v = res!(numbering::apply(engine, &numbering, &all));
			res!(common::shown(engine, v, item.span()))
		} else {
			let v = res!(numbering::apply(engine, &numbering, &[number]));
			res!(common::shown(engine, v, item.span()))
		};
		let resolved = res!(common::set(resolved, ElemKind::Text, "overhang", Value::Bool(false)));
		let resolved = res!(common::build(engine, ElemKind::Align, span,
			vec![align.clone(), Value::Content(resolved)], Vec::new()));
		let mut body = common::body(&item, "body");
		if !tight {
			body = common::seq(vec![body, Content::marker(ElemKind::Parbreak, Span::detached())]);
		}
		let mut inner = parents.iter().map(|p| Value::Int(*p as i64)).collect::<Vec<_>>();
		inner.push(Value::Int(number as i64));
		let body = res!(common::set(body, ElemKind::Enum, "parents", Value::array(inner)));
		rows.push((resolved, body));
		number = if reversed { number.saturating_sub(1) } else { number.saturating_add(1) };
	}
	let realised = res!(item_grid(engine, elem, styles, rows, gutter.clone()));
	if tight {
		return attach_tight(engine, gutter, realised, span);
	}
	Ok(realised)
}

fn show_terms(engine: &mut Engine, elem: &Content, styles: &StyleChain) -> Outcome<Content> {
	let span = elem.span();
	let tight = matches!(res!(common::get(elem, styles, "tight")), Value::Bool(true));
	let separator = common::display(res!(common::get(elem, styles, "separator")));
	let indent = res!(common::get(elem, styles, "indent"));
	let hanging = res!(common::get(elem, styles, "hanging-indent"));
	let gutter = res!(gutter(elem, styles, tight));
	let pad = common::pt(common::resolve_pt(styles, &hanging) + common::resolve_pt(styles, &indent));
	let unpad = if common::resolve_pt(styles, &hanging) != 0.0 {
		Some(res!(common::h(engine, common::neg(&hanging), false)))
	} else {
		None
	};
	let mut stack_children = Vec::new();
	for child in children(elem) {
		let mut parts = Vec::new();
		if let Some(u) = &unpad {
			parts.push(u.clone());
		}
		let term = res!(common::raw_elem(ElemKind::Strong, child.span(),
			vec![("body", Value::Content(common::body(&child, "term")))]));
		parts.push(term);
		parts.push(separator.clone());
		parts.push(common::body(&child, "description"));
		// Text in a wide term list is always set as paragraphs.
		if !tight {
			parts.push(Content::marker(ElemKind::Parbreak, Span::detached()));
		}
		let item = common::spanned(common::seq(parts), child.span());
		stack_children.push(Value::Content(res!(common::block(engine, item, child.span(), Vec::new()))));
	}
	let stack = res!(common::build(engine, ElemKind::Stack, span, stack_children, vec![("spacing", gutter.clone())]));
	let side = if common::is_rtl(styles) { "right" } else { "left" };
	let padded = res!(common::build(engine, ElemKind::Pad, span, vec![Value::Content(stack)], vec![(side, pad)]));
	let realised = res!(common::set(padded, ElemKind::Terms, "within", Value::Bool(true)));
	if tight {
		return attach_tight(engine, gutter, realised, span);
	}
	Ok(realised)
}
