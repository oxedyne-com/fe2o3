// U6c owns this file. Schemas for grid and table and their cells, headers, footers and lines. `grid.cell`
// and `table.cell` fields are what the per-cell fill/stroke/align closures resolve against.
//
// Typst normalises at construction, and so does this: `columns: 3` is stored as three `auto`s, `gutter`
// is not a field but the fallback for both `column-gutter` and `row-gutter`, and a child that is not a
// cell, header, footer or line is wrapped in the family's own cell -- so `it.children` reads as Typst's.

use crate::diag::DiagnosticKind;
use crate::eval::args::Args;
use crate::eval::content::{
	Content,
	ElemKind,
	FieldDefault,
	FieldId,
	FieldSpec,
	FieldType,
};
use crate::eval::scope::Scope;
use crate::eval::styles::{
	Property,
	Style,
	StyleChain,
	Styles,
};
use crate::eval::value::{
	Alignment,
	Color,
	ColorSpace,
	HAlign,
	Length,
	Ratio,
	Relative,
	Stroke,
	Type,
	Paint,
	VAlign,
	Value,
};
use crate::eval::Engine;
use crate::syntax::Span;

use oxedyne_fe2o3_core::prelude::*;

use std::sync::Arc;

native_fns! {
	pub enum GridFn {
	}
}

pub fn define(_scope: &mut Scope) {}

pub fn call(f: GridFn, _engine: &mut Engine, _args: Args) -> Outcome<Value> {
	match f {}
}

const ANY: FieldType = FieldType::Any;

// Grid and table: one schema, the defaults of `inset` and `stroke` differ and are computed.
static GRID_FIELDS: [FieldSpec; 9] = [
	FieldSpec::named("columns",			ANY,	FieldDefault::EmptyArray),
	FieldSpec::named("rows",			ANY,	FieldDefault::EmptyArray),
	FieldSpec::named("column-gutter",	ANY,	FieldDefault::EmptyArray),
	FieldSpec::named("row-gutter",		ANY,	FieldDefault::EmptyArray),
	FieldSpec::named("inset",			ANY,	FieldDefault::Computed),
	FieldSpec::named("align",			ANY,	FieldDefault::Auto),
	FieldSpec::named("fill",			ANY,	FieldDefault::None),
	FieldSpec::named("stroke",			ANY,	FieldDefault::Computed),
	FieldSpec::named("children",		ANY,	FieldDefault::EmptyArray).variadic().unsettable(),
];

static CELL_FIELDS: [FieldSpec; 10] = [
	FieldSpec::required("body",			FieldType::Content),
	FieldSpec::named("x",				ANY,	FieldDefault::Auto),
	FieldSpec::named("y",				ANY,	FieldDefault::Auto),
	FieldSpec::named("colspan",			FieldType::Of(Type::Int),	FieldDefault::Int(1)),
	FieldSpec::named("rowspan",			FieldType::Of(Type::Int),	FieldDefault::Int(1)),
	FieldSpec::named("inset",			ANY,	FieldDefault::Auto),
	FieldSpec::named("align",			ANY,	FieldDefault::Auto),
	FieldSpec::named("fill",			ANY,	FieldDefault::Auto),
	FieldSpec::named("stroke",			ANY,	FieldDefault::Computed),
	FieldSpec::named("breakable",		ANY,	FieldDefault::Auto),
];

static HEADER_FIELDS: [FieldSpec; 3] = [
	FieldSpec::named("repeat",			FieldType::Of(Type::Bool),	FieldDefault::Bool(true)),
	FieldSpec::named("level",			FieldType::Of(Type::Int),	FieldDefault::Int(1)),
	FieldSpec::named("children",		ANY,	FieldDefault::EmptyArray).variadic().unsettable(),
];

static FOOTER_FIELDS: [FieldSpec; 2] = [
	FieldSpec::named("repeat",			FieldType::Of(Type::Bool),	FieldDefault::Bool(true)),
	FieldSpec::named("children",		ANY,	FieldDefault::EmptyArray).variadic().unsettable(),
];

static HLINE_FIELDS: [FieldSpec; 5] = [
	FieldSpec::named("y",				ANY,	FieldDefault::Auto),
	FieldSpec::named("start",			FieldType::Of(Type::Int),	FieldDefault::Int(0)),
	FieldSpec::named("end",				ANY,	FieldDefault::None),
	FieldSpec::named("stroke",			ANY,	FieldDefault::Computed),
	FieldSpec::named("position",		ANY,	FieldDefault::Computed),
];

static VLINE_FIELDS: [FieldSpec; 5] = [
	FieldSpec::named("x",				ANY,	FieldDefault::Auto),
	FieldSpec::named("start",			FieldType::Of(Type::Int),	FieldDefault::Int(0)),
	FieldSpec::named("end",				ANY,	FieldDefault::None),
	FieldSpec::named("stroke",			ANY,	FieldDefault::Computed),
	FieldSpec::named("position",		ANY,	FieldDefault::Computed),
];

pub fn fields(kind: ElemKind) -> &'static [FieldSpec] {
	match kind {
		ElemKind::Grid | ElemKind::Table				=> &GRID_FIELDS,
		ElemKind::GridCell | ElemKind::TableCell		=> &CELL_FIELDS,
		ElemKind::GridHeader | ElemKind::TableHeader	=> &HEADER_FIELDS,
		ElemKind::GridFooter | ElemKind::TableFooter	=> &FOOTER_FIELDS,
		ElemKind::GridHLine | ElemKind::TableHLine		=> &HLINE_FIELDS,
		ElemKind::GridVLine | ElemKind::TableVLine		=> &VLINE_FIELDS,
		_												=> &[],
	}
}

/// Field ids of the grid family, fixed by the schemas above.
pub mod fid {
	use crate::eval::content::FieldId;

	// grid, table
	pub const COLUMNS:		FieldId = FieldId(0);
	pub const ROWS:			FieldId = FieldId(1);
	pub const COL_GUTTER:	FieldId = FieldId(2);
	pub const ROW_GUTTER:	FieldId = FieldId(3);
	pub const INSET:		FieldId = FieldId(4);
	pub const ALIGN:		FieldId = FieldId(5);
	pub const FILL:			FieldId = FieldId(6);
	pub const STROKE:		FieldId = FieldId(7);
	pub const CHILDREN:		FieldId = FieldId(8);
	// cells
	pub const BODY:			FieldId = FieldId(0);
	pub const X:			FieldId = FieldId(1);
	pub const Y:			FieldId = FieldId(2);
	pub const COLSPAN:		FieldId = FieldId(3);
	pub const ROWSPAN:		FieldId = FieldId(4);
	pub const CELL_INSET:	FieldId = FieldId(5);
	pub const CELL_ALIGN:	FieldId = FieldId(6);
	pub const CELL_FILL:	FieldId = FieldId(7);
	pub const CELL_STROKE:	FieldId = FieldId(8);
	pub const BREAKABLE:	FieldId = FieldId(9);
	// headers and footers
	pub const REPEAT:		FieldId = FieldId(0);
	pub const LEVEL:		FieldId = FieldId(1);
	pub const HEAD_CHILDREN:	FieldId = FieldId(2);
	pub const FOOT_CHILDREN:	FieldId = FieldId(1);
	// lines: `y` for an hline, `x` for a vline
	pub const LINE_AT:		FieldId = FieldId(0);
	pub const LINE_START:	FieldId = FieldId(1);
	pub const LINE_END:		FieldId = FieldId(2);
	pub const LINE_STROKE:	FieldId = FieldId(3);
	pub const LINE_POS:		FieldId = FieldId(4);
}

/// The `1pt + black` stroke Typst gives a table and every line by default.
pub fn default_line_stroke() -> Value {
	Value::Stroke(Arc::new(Stroke {
		paint:		Some(Paint::Color(Color { space: ColorSpace::Luma, c: [0.0, 0.0, 0.0, 0.0], alpha: 1.0 })),
		thickness:	Some(Length::pt(1.0)),
		..Stroke::default()
	}))
}

/// A field's default where the schema holds `Computed`: Typst's value, exactly as `repr` shows it.
pub fn computed_default(kind: ElemKind, field: FieldId) -> Option<Value> {
	use fid::*;
	match (kind, field) {
		(ElemKind::Grid, INSET) | (ElemKind::Grid, STROKE)	=> Some(Value::dict(Default::default())),
		(ElemKind::Table, INSET)	=> Some(Value::Relative(Relative { rel: Ratio(0.0), abs: Length::pt(5.0) })),
		(ElemKind::Table, STROKE)	=> Some(default_line_stroke()),
		(ElemKind::GridCell, CELL_STROKE) | (ElemKind::TableCell, CELL_STROKE)
			=> Some(Value::dict(Default::default())),
		(ElemKind::GridHLine, LINE_STROKE) | (ElemKind::TableHLine, LINE_STROKE)
		| (ElemKind::GridVLine, LINE_STROKE) | (ElemKind::TableVLine, LINE_STROKE)
			=> Some(default_line_stroke()),
		(ElemKind::GridHLine, LINE_POS) | (ElemKind::TableHLine, LINE_POS)
			=> Some(Value::Alignment(Alignment { x: None, y: Some(VAlign::Top) })),
		(ElemKind::GridVLine, LINE_POS) | (ElemKind::TableVLine, LINE_POS)
			=> Some(Value::Alignment(Alignment { x: Some(HAlign::Start), y: None })),
		_ => None,
	}
}

/// A field as a read shows it where nothing sets it: [`computed_default`], with a line's stroke as Typst holds it,
/// a stroke whose paint and thickness are both `auto` (it reads as `1pt + black`), where layout draws
/// [`default_line_stroke`].
pub fn read_default(kind: ElemKind, field: FieldId) -> Option<Value> {
	match computed_default(kind, field) {
		Some(Value::Stroke(_))	=> Some(Value::Stroke(Arc::new(Stroke::default()))),
		other					=> other,
	}
}

/// A field in force for an element in hand: its own value, a `set` rule's, or the default, computed
/// defaults included.
pub fn resolve(styles: &StyleChain, elem: &Content, field: FieldId) -> Outcome<Value> {
	if let Some(v) = res!(styles.resolve(elem, field)) {
		return Ok(v);
	}
	Ok(match elem.kind() {
		Some(k)	=> computed_default(k, field).unwrap_or(Value::None),
		None	=> Value::None,
	})
}

/// Does the grid family hold this element as a child of a grid or table?
pub fn is_grid_child(kind: ElemKind) -> bool { kind.family() == crate::eval::content::Family::Grid }

/// The family's own kinds: (cell, header, footer, hline, vline) of a grid or of a table.
pub fn family_of(kind: ElemKind) -> (ElemKind, ElemKind, ElemKind, ElemKind, ElemKind) {
	match kind {
		ElemKind::Table | ElemKind::TableCell | ElemKind::TableHeader | ElemKind::TableFooter
		| ElemKind::TableHLine | ElemKind::TableVLine => (
			ElemKind::TableCell, ElemKind::TableHeader, ElemKind::TableFooter,
			ElemKind::TableHLine, ElemKind::TableVLine),
		_ => (
			ElemKind::GridCell, ElemKind::GridHeader, ElemKind::GridFooter,
			ElemKind::GridHLine, ElemKind::GridVLine),
	}
}

/// The track sizings of `columns`, `rows` and the gutters, normalised as Typst stores them: an integer is
/// that many `auto`s, a single sizing is a one-element array, an array is checked element by element.
fn track_sizings(engine: &mut Engine, span: Span, name: &str, v: Value) -> Outcome<Value> {
	let one = |v: &Value| matches!(v,
		Value::Auto | Value::Length(_) | Value::Ratio(_) | Value::Relative(_) | Value::Fraction(_));
	match v {
		Value::Int(n) if n >= 0 => Ok(Value::array(vec![Value::Auto; n as usize])),
		Value::Array(a) => {
			for item in a.iter() {
				if !one(item) {
					return Err(engine.error(DiagnosticKind::Type, span, fmt!(
						"expected auto, relative length, or fraction, found {}", item.ty().name())));
				}
			}
			Ok(Value::Array(a))
		}
		v if one(&v)	=> Ok(Value::array(vec![v])),
		other			=> Err(engine.error(DiagnosticKind::Type, span, fmt!(
			"{}: expected integer, auto, relative length, fraction, or array of the latter three, found {}",
			name, other.ty().name()))),
	}
}

/// Gutters are relative lengths in Typst's stored form (`2pt` is kept as `0% + 2pt`); a fraction stays one.
fn gutter_sizings(engine: &mut Engine, span: Span, name: &str, v: Value) -> Outcome<Value> {
	let v = res!(track_sizings(engine, span, name, v));
	match v {
		Value::Array(a) => Ok(Value::array(a.iter().map(|g| match g {
			Value::Length(l)	=> Value::Relative(Relative { rel: Ratio(0.0), abs: *l }),
			Value::Ratio(r)		=> Value::Relative(Relative { rel: *r, abs: Length::zero() }),
			other				=> other.clone(),
		}).collect())),
		other => Ok(other),
	}
}

/// Wraps a child that is not one of the family's own elements into the family's cell, and refuses a grid
/// element inside a table and the reverse, with Typst's wording.
fn wrap_child(
	engine:	&mut Engine,
	span:	Span,
	parent:	ElemKind,
	v:		Value,
	in_part: bool,
)
	-> Outcome<Value>
{
	let (cell, header, footer, _, _) = family_of(parent);
	let content = res!(cell_content(v));
	if let Some(k) = content.kind() {
		if is_grid_child(k) && k != ElemKind::Grid && k != ElemKind::Table {
			let own = family_of(k).0 == cell;
			if !own {
				let (what, other) = match cell {
					ElemKind::TableCell	=> ("table", "grid"),
					_					=> ("grid", "table"),
				};
				return Err(engine.error_hint(DiagnosticKind::Type, span,
					fmt!("cannot use `{}.{}` as a {} cell", other, k.name(), what),
					fmt!("use `{}.{}` instead", what, k.name())));
			}
			if in_part && (k == header || k == footer) {
				return Err(engine.error(DiagnosticKind::Type, span, fmt!("cannot place a {} within another header or footer", k.path())));
			}
			return Ok(Value::Content(content));
		}
	}
	Ok(Value::Content(Content::new(cell, vec![(fid::BODY, Value::Content(content))], span)))
}

/// A child as content: what Typst displays as content becomes a cell's body.
fn cell_content(v: Value) -> Outcome<Content> {
	match v {
		Value::Content(c)	=> Ok(c),
		Value::None			=> Ok(Content::empty()),
		Value::Str(s)		=> Ok(Content::text(&s)),
		Value::Int(i)		=> Ok(Content::text(&fmt!("{}", i))),
		Value::Float(f)		=> Ok(Content::text(&fmt!("{}", f))),
		Value::Symbol(s)	=> Ok(Content::symbol(&crate::eval::ops::symbol_text(&s))),
		other				=> Err(err!("expected content, found {}", other.ty().name(); Input, Mismatch)),
	}
}

pub fn construct(engine: &mut Engine, kind: ElemKind, args: &mut Args) -> Outcome<Option<Content>> {
	let span = args.span;
	match kind {
		ElemKind::Grid | ElemKind::Table => {
			let mut fields: Vec<(FieldId, Value)> = Vec::new();
			if let Some(v) = res!(args.named::<Value>("columns")) {
				fields.push((fid::COLUMNS, res!(track_sizings(engine, span, "columns", v))));
			}
			if let Some(v) = res!(args.named::<Value>("rows")) {
				fields.push((fid::ROWS, res!(track_sizings(engine, span, "rows", v))));
			}
			let gutter = res!(args.named::<Value>("gutter"));
			let cg = res!(args.named::<Value>("column-gutter")).or_else(|| gutter.clone());
			let rg = res!(args.named::<Value>("row-gutter")).or(gutter);
			if let Some(v) = cg {
				fields.push((fid::COL_GUTTER, res!(gutter_sizings(engine, span, "column-gutter", v))));
			}
			if let Some(v) = rg {
				fields.push((fid::ROW_GUTTER, res!(gutter_sizings(engine, span, "row-gutter", v))));
			}
			for (id, name) in [(fid::INSET, "inset"), (fid::ALIGN, "align"), (fid::FILL, "fill"), (fid::STROKE, "stroke")] {
				if let Some(v) = res!(args.named::<Value>(name)) {
					fields.push((id, v));
				}
			}
			let mut children = Vec::new();
			for v in res!(args.all::<Value>()) {
				children.push(res!(wrap_child(engine, span, kind, v, false)));
			}
			fields.push((fid::CHILDREN, Value::array(children)));
			res!(std::mem::take(args).finish());
			Ok(Some(Content::new(kind, fields, span)))
		}
		ElemKind::GridHeader | ElemKind::TableHeader | ElemKind::GridFooter | ElemKind::TableFooter => {
			let is_head = matches!(kind, ElemKind::GridHeader | ElemKind::TableHeader);
			let mut fields: Vec<(FieldId, Value)> = Vec::new();
			if let Some(v) = res!(args.named::<Value>("repeat")) {
				fields.push((fid::REPEAT, v));
			}
			if is_head {
				if let Some(v) = res!(args.named::<Value>("level")) {
					match v {
						Value::Int(n) if n >= 1	=> fields.push((fid::LEVEL, v)),
						_ => return Err(engine.error(DiagnosticKind::Type, span, "number must be positive")),
					}
				}
			}
			let mut children = Vec::new();
			for v in res!(args.all::<Value>()) {
				let c = res!(wrap_child(engine, span, kind, v, true));
				children.push(c);
			}
			let at = if is_head { fid::HEAD_CHILDREN } else { fid::FOOT_CHILDREN };
			fields.push((at, Value::array(children)));
			res!(std::mem::take(args).finish());
			Ok(Some(Content::new(kind, fields, span)))
		}
		ElemKind::GridCell | ElemKind::TableCell => {
			// The generic walk, then Typst's range checks on the spans.
			for name in ["colspan", "rowspan"] {
				if let Some(Value::Int(n)) = args.items.iter().find(|a| a.name.as_deref() == Some(name)).map(|a| &a.value) {
					if *n < 1 {
						return Err(engine.error(DiagnosticKind::Type, span, "number must be positive"));
					}
				}
			}
			Ok(None)
		}
		_ => Ok(None),
	}
}

/// A cell realises as Typst's does: its body padded by the resolved inset, under the resolved alignment.
/// Grids, tables, headers, footers and lines are primitives the grid flow lays out itself.
pub fn show(engine: &mut Engine, elem: &Content, _styles: &StyleChain) -> Outcome<Option<Content>> {
	match elem.kind() {
		Some(ElemKind::GridCell | ElemKind::TableCell) => (),
		_ => return Ok(None),
	}
	let span = elem.span();
	let mut body = match elem.get(fid::BODY) {
		Some(v)	=> res!(cell_content(v.clone())),
		None	=> Content::empty(),
	};
	if let Some(Value::Dict(d)) = elem.get(fid::CELL_INSET) {
		let zero = |v: Option<&Value>| match v {
			None | Some(Value::None)	=> true,
			Some(Value::Length(l))		=> l.abs == 0.0 && l.em == 0.0,
			Some(Value::Relative(r))	=> r.rel.0 == 0.0 && r.abs.abs == 0.0 && r.abs.em == 0.0,
			Some(Value::Ratio(r))		=> r.0 == 0.0,
			Some(_)						=> false,
		};
		if !["left", "top", "right", "bottom"].iter().all(|k| zero(d.get(k))) {
			let mut fields = Vec::new();
			for k in ["left", "top", "right", "bottom"] {
				if let Some(v) = d.get(k) {
					let id = res!(layout_field(engine, span, ElemKind::Pad, k));
					fields.push((id, v.clone()));
				}
			}
			let id = res!(layout_field(engine, span, ElemKind::Pad, "body"));
			fields.push((id, Value::Content(body)));
			body = Content::new(ElemKind::Pad, fields, span);
		}
	}
	if let Some(Value::Alignment(a)) = elem.get(fid::CELL_ALIGN) {
		let field = res!(layout_field(engine, span, ElemKind::Align, "alignment"));
		body = body.styled(Styles::from_style(Style::Property(Property {
			elem: ElemKind::Align, field, value: Value::Alignment(*a), span,
		})));
	}
	Ok(Some(body))
}

/// A field of a layout element the cell realises into; the layout family's schema is U6b's.
fn layout_field(engine: &mut Engine, span: Span, kind: ElemKind, name: &str) -> Outcome<FieldId> {
	match kind.field_id(name) {
		Some(id)	=> Ok(id),
		None		=> Err(engine.error(DiagnosticKind::Type, span, fmt!("{} has no field `{}`, which a grid cell needs", kind.path(), name))),
	}
}
