// U5 owns this file: footnote and footnote.entry. The footnote's show is its superscript number, linked
// to the entry; page layout collects each page's footnotes, builds their entries with [`entry`] and sets
// them below the separator, the entry's show linking its number back. The entry's location is the
// footnote's own location's variant one ([`entry_location`]), as Typst's `loc.variant(1)`, so page
// layout records the entry's position under that location.

use crate::diag::DiagnosticKind;
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
	to_content,
	CastErr,
	K,
};
use crate::eval::lib::model::{
	link,
	lookup,
};
use crate::eval::locate::Location;
use crate::eval::styles::{
	StyleChain,
	Styles,
};
use crate::eval::value::{
	Length,
	Ratio,
	Relative,
	Type,
	Value,
};
use crate::eval::Engine;
use crate::syntax::Span;

use oxedyne_fe2o3_core::prelude::*;

const ANY:		FieldType = FieldType::Any;
const LENGTH:	FieldType = FieldType::Of(Type::Length);

static FOOTNOTE: [FieldSpec; 2] = [
	FieldSpec::named("numbering",		ANY,	FieldDefault::Str("1")),
	FieldSpec::required("body",			ANY),
];

static ENTRY: [FieldSpec; 5] = [
	FieldSpec::required("note",			ANY),
	FieldSpec::named("separator",		ANY,	FieldDefault::Computed),
	FieldSpec::named("clearance",		LENGTH,	FieldDefault::Em(1.0)),
	FieldSpec::named("gap",				LENGTH,	FieldDefault::Em(0.5)),
	FieldSpec::named("indent",			LENGTH,	FieldDefault::Em(1.0)),
];

pub fn fields(kind: ElemKind) -> &'static [FieldSpec] {
	match kind {
		ElemKind::Footnote		=> &FOOTNOTE,
		ElemKind::FootnoteEntry	=> &ENTRY,
		_						=> &[],
	}
}

/// The separator: a line 30% wide, stroked at 0.05em.
pub fn default_value(kind: ElemKind, name: &str) -> Option<Value> {
	match (kind, name) {
		(ElemKind::FootnoteEntry, "separator") => {
			let line = ElemKind::Line;
			let mut fields = Vec::new();
			if let Some(id) = line.field_id("length") {
				fields.push((id, Value::Relative(Relative { abs: Length::zero(), rel: Ratio(0.3) })));
			}
			if let Some(id) = line.field_id("stroke") {
				fields.push((id, common::em(0.05)));
			}
			Some(Value::Content(Content::new(line, fields, Span::detached())))
		}
		_ => None,
	}
}

pub fn cast(kind: ElemKind, name: &str, v: Value) -> Result<Value, CastErr> {
	match (kind, name) {
		(ElemKind::Footnote, "numbering")	=> expect(&v, &[K::Str, K::Func]).map(|_| v),
		(ElemKind::Footnote, "body") => match v {
			Value::Label(_)	=> Ok(v),
			other			=> expect(&other, &[K::Content, K::Label]).map(|_| to_content(other)),
		},
		(ElemKind::FootnoteEntry, "note") => {
			ok!(expect(&v, &[K::Content]));
			match to_content(v) {
				Value::Content(c) if c.is(ElemKind::Footnote)	=> Ok(Value::Content(c)),
				Value::Content(c) => match common::raw_elem(ElemKind::Footnote, Span::detached(),
					vec![("body", Value::Content(c))])
				{
					Ok(f)	=> Ok(Value::Content(f)),
					Err(e)	=> Err(CastErr::Value(e.to_string())),
				},
				other => Ok(other),
			}
		}
		(ElemKind::FootnoteEntry, "separator")	=> expect(&v, &[K::Content]).map(|_| to_content(v)),
		(ElemKind::FootnoteEntry, _)			=> expect(&v, &[K::Length]).map(|_| v),
		_										=> Ok(v),
	}
}

/// Where a footnote's entry is recorded: the footnote's location, variant one.
pub fn entry_location(loc: Location) -> Location {
	Location(loc.0.rotate_left(1) ^ 0x9e37_79b9_7f4a_7c15)
}

/// The entry page layout sets for a footnote.
pub fn entry(note: &Content) -> Outcome<Content> {
	common::raw_elem(ElemKind::FootnoteEntry, note.span(), vec![("note", Value::Content(note.clone()))])
}

/// Does the footnote repeat another's mark (`footnote(<label>)`) rather than carry a note?
pub fn is_ref(note: &Content) -> bool { matches!(note.field("body"), Some(Value::Label(_))) }

/// The location of the footnote that carries the note: its own, or the one its label names.
fn declaration(engine: &mut Engine, note: &Content, depth: usize) -> Outcome<Location> {
	let span = note.span();
	match note.field("body") {
		Some(Value::Label(l)) => {
			let l = l.clone();
			let target = res!(lookup::label(engine, &l, span));
			if !target.is(ElemKind::Footnote) {
				return Err(engine.error(DiagnosticKind::Type, span, "referenced element should be a footnote"));
			}
			if target.location() == note.location() || depth > 64 {
				return Err(engine.error(DiagnosticKind::Type, span, "footnote cannot reference itself"));
			}
			declaration(engine, &target, depth + 1)
		}
		_ => match note.location() {
			Some(l)	=> Ok(l),
			None	=> Err(engine.error(DiagnosticKind::Type, span, "footnote must have a location")),
		},
	}
}

pub fn show_set_entry() -> Outcome<Styles> {
	common::props(vec![
		(ElemKind::Par,		"leading",	common::em(0.5)),
		(ElemKind::Text,	"size",		common::em(0.85)),
	])
}

pub fn show(engine: &mut Engine, elem: &Content, styles: &StyleChain) -> Outcome<Option<Content>> {
	let span = elem.span();
	match elem.kind() {
		Some(ElemKind::FootnoteEntry) => {
			let note = match elem.field("note") {
				Some(Value::Content(c))	=> c.clone(),
				_						=> return Err(engine.error(DiagnosticKind::Type, span, "missing argument: note")),
			};
			let dest = match note.location() {
				Some(l)	=> l,
				None	=> return Err(engine.error_hint(DiagnosticKind::Type, span, "footnote entry must have a location",
					"try using a query or a show rule to customize the footnote instead")),
			};
			let numbering = note.field("numbering").cloned().unwrap_or_else(|| Value::str("1"));
			let num = res!(lookup::display_counter(engine, &lookup::elem_counter(ElemKind::Footnote),
				dest, &numbering, span));
			let linked = res!(link::to_location(num, dest));
			let sup = res!(common::build(engine, ElemKind::Super, span, vec![Value::Content(linked)], Vec::new()));
			let indent = res!(common::get(elem, styles, "indent"));
			let lead = res!(common::h(engine, indent, false));
			let gap = res!(common::h(engine, common::em(0.05), true));
			Ok(Some(common::seq(vec![lead, sup, gap, common::body(&note, "body")])))
		}
		_ => {
			let loc = res!(declaration(engine, elem, 0));
			let numbering = res!(common::get(elem, styles, "numbering"));
			let num = res!(lookup::display_counter(engine, &lookup::elem_counter(ElemKind::Footnote),
				loc, &numbering, span));
			let linked = res!(link::to_location(num, entry_location(loc)));
			let sup = res!(common::build(engine, ElemKind::Super, span, vec![Value::Content(linked)], Vec::new()));
			// Zero-width weak spacing: a space before the mark is dropped.
			let hole = res!(common::h(engine, common::pt(0.0), true));
			Ok(Some(common::seq(vec![hole, sup])))
		}
	}
}
