// U5 owns this file: ref and cite. A reference to a label that a bibliography holds is a citation; any
// other is realised as the target's supplement and number, linked to it, as Typst's `RefElem::realize`.
// The number is displayed with the target's numbering; Typst trims a pattern's prefix and suffix there
// (`"1."` references as `1`), which waits on a trimmed form of `numbering::apply` (U3).

use crate::diag::DiagnosticKind;
use crate::eval::args::Args;
use crate::eval::content::{
	Content,
	ElemKind,
	FieldDefault,
	FieldSpec,
	FieldType,
};
use crate::eval::intro::Counter;
use crate::eval::lib::model::common::{
	self,
	choice,
	expect,
	to_content,
	CastErr,
	K,
};
use crate::eval::lib::model::{
	bibliography,
	heading,
	link,
	lookup,
};
use crate::eval::styles::StyleChain;
use crate::eval::value::{
	Label,
	Type,
	Value,
};
use crate::eval::Engine;
use crate::syntax::Span;

use oxedyne_fe2o3_core::prelude::*;

const ANY: FieldType = FieldType::Any;

static REF: [FieldSpec; 5] = [
	FieldSpec::required("target",		FieldType::Of(Type::Label)),
	FieldSpec::named("supplement",		ANY,	FieldDefault::Auto),
	FieldSpec::named("form",			ANY,	FieldDefault::Str("normal")),
	FieldSpec::named("citation",		ANY,	FieldDefault::None).synthesised(),
	FieldSpec::named("element",			ANY,	FieldDefault::None).synthesised(),
];

static CITE: [FieldSpec; 4] = [
	FieldSpec::required("key",			FieldType::Of(Type::Label)),
	FieldSpec::named("supplement",		ANY,	FieldDefault::None),
	FieldSpec::named("form",			ANY,	FieldDefault::Str("normal")),
	FieldSpec::named("style",			ANY,	FieldDefault::Auto),
];

pub fn fields(kind: ElemKind) -> &'static [FieldSpec] {
	match kind {
		ElemKind::Ref	=> &REF,
		ElemKind::Cite	=> &CITE,
		_				=> &[],
	}
}

pub fn cast(kind: ElemKind, name: &str, v: Value) -> Result<Value, CastErr> {
	match (kind, name) {
		(ElemKind::Ref, "target") | (ElemKind::Cite, "key")	=> expect(&v, &[K::Label]).map(|_| v),
		(ElemKind::Ref, "supplement")	=> heading::supplement(v),
		(ElemKind::Ref, "form")			=> choice(&v, &["normal", "page"], false, false).map(|_| v),
		(ElemKind::Cite, "supplement")	=> match v {
			Value::None	=> Ok(v),
			other		=> expect(&other, &[K::Content, K::None]).map(|_| to_content(other)),
		},
		(ElemKind::Cite, "form")		=> choice(&v, &["normal", "prose", "full", "author", "year"], false, true)
			.map(|_| v),
		(ElemKind::Cite, "style")		=> match &v {
			Value::Auto		=> Ok(v),
			Value::Str(s)	=> bibliography::style_name(s).map(|_| bibliography::style_value(s)),
			other			=> Err(CastErr::Type(common::mismatch("string or auto", other))),
		},
		_ => Ok(v),
	}
}

/// `cite(key, ..)`: the schema walk, with a named style checked against the styles Austenite sets.
pub fn construct_cite(engine: &mut Engine, args: &mut Args) -> Outcome<Content> {
	super::construct_schema(engine, ElemKind::Cite, args)
}

/// The citation a reference to a bibliography key stands for: `cite(target)`, with the reference's
/// content supplement.
fn citation(elem: &Content, styles: &StyleChain) -> Outcome<Content> {
	let target = elem.field("target").cloned().unwrap_or(Value::None);
	let supplement = match res!(common::get(elem, styles, "supplement")) {
		Value::Content(c)	=> Value::Content(c),
		_					=> Value::None,
	};
	common::raw_elem(ElemKind::Cite, elem.span(), vec![("key", target), ("supplement", supplement)])
}

pub fn synthesise(engine: &mut Engine, elem: &mut Content, styles: &StyleChain) -> Outcome<()> {
	let cite = res!(citation(elem, styles));
	elem.set(res!(common::fid(ElemKind::Ref, "citation")), Value::Content(cite));
	let mut found = Value::None;
	if let Some(Value::Label(l)) = elem.field("target") {
		let l = l.clone();
		if !lookup::bib_has(engine, &l) {
			// An absent or ambiguous label is the show's error to raise, not synthesis's.
			let before = engine.diags.len();
			match lookup::label_opt(engine, &l, elem.span()) {
				Ok(Some(t))	=> found = Value::Content(t),
				Ok(None)	=> (),
				Err(_)		=> engine.diags.truncate(before),
			}
		}
	}
	elem.set(res!(common::fid(ElemKind::Ref, "element")), found);
	Ok(())
}

pub fn show(engine: &mut Engine, elem: &Content, styles: &StyleChain) -> Outcome<Option<Content>> {
	match elem.kind() {
		Some(ElemKind::Cite)	=> bibliography::show_cite(engine, elem, styles).map(Some),
		_						=> show_ref(engine, elem, styles).map(Some),
	}
}

fn show_ref(engine: &mut Engine, elem: &Content, styles: &StyleChain) -> Outcome<Content> {
	let span = elem.span();
	let target = match elem.field("target") {
		Some(Value::Label(l))	=> l.clone(),
		_						=> return Err(engine.error(DiagnosticKind::Type, span, "missing argument: target")),
	};
	let form = res!(common::get(elem, styles, "form"));
	if matches!(&form, Value::Str(s) if s.as_str() == "page") {
		// The page the target lies on, in the page numbering there, after the page's supplement.
		let found = res!(lookup::label(engine, &target, span));
		let loc = res!(located(engine, &found, span));
		let page = crate::eval::intro::Counter { key: crate::eval::intro::CounterKey::Page };
		let numbers = res!(lookup::display_counter(engine, &page, loc, &Value::str("1"), span));
		let supplement = match res!(common::get(elem, styles, "supplement")) {
			Value::Auto	=> common::text(common::local(styles, "page")),
			other		=> res!(heading::resolve_supplement(engine, &other, styles, "page", &found))
				.unwrap_or_else(Content::empty),
		};
		let content = if supplement.is_empty() {
			numbers
		} else {
			common::seq(vec![supplement, common::text("\u{a0}"), numbers])
		};
		return link::to_location(common::spanned(content, span), loc);
	}
	if lookup::bib_has(engine, &target) {
		let before = engine.diags.len();
		let found = lookup::label_opt(engine, &target, span);
		engine.diags.truncate(before);
		if let Ok(Some(e)) = found {
			let name = e.kind().map(|k| k.name()).unwrap_or("content");
			return Err(engine.error_hint(DiagnosticKind::Type, span,
				fmt!("label `<{}>` occurs both in the document and a bibliography", target.as_str()),
				fmt!("change either the {}'s label or the bibliography key to resolve the ambiguity", name)));
		}
		return citation(elem, styles);
	}
	let found = res!(lookup::label(engine, &target, span));
	let kind = match found.kind() {
		Some(k)	=> k,
		None	=> return Err(engine.error(DiagnosticKind::Type, span, "cannot reference sequence")),
	};
	if kind == ElemKind::Footnote {
		// A reference to a footnote is the footnote's mark again, pointing at the same note.
		let mut note = found.clone();
		note.set(res!(common::fid(ElemKind::Footnote, "body")), Value::Label(target.clone()));
		if let Content::Elem(e) = &mut note {
			let e = std::sync::Arc::make_mut(e);
			e.location = None;
			e.prepared = false;
			e.label = None;
			e.span = span;
		}
		return Ok(note);
	}
	let (counter, default_supplement) = match kind {
		ElemKind::Heading | ElemKind::Figure	=> (res!(refable_counter(&found)), found.field("supplement").cloned()),
		ElemKind::Equation						=> (lookup::elem_counter(ElemKind::Equation), found.field("supplement").cloned()),
		ElemKind::Image | ElemKind::Table | ElemKind::Raw => return Err(engine.error(DiagnosticKind::Type, span, fmt!(
			"cannot reference {} directly, try putting it into a figure", kind.name()))),
		_ => return Err(engine.error(DiagnosticKind::Type, span, fmt!("cannot reference {}", kind.name()))),
	};
	let numbering = match found.field("numbering") {
		Some(Value::None) | None => {
			let set = if kind == ElemKind::Equation { "math.equation" } else { kind.name() };
			return Err(engine.error_hint(DiagnosticKind::Type, span,
				fmt!("cannot reference {} without numbering", kind.name()),
				fmt!("you can enable {} numbering with `#set {}(numbering: \"1.\")`", kind.name(), set)));
		}
		Some(n) => n.clone(),
	};
	let default_supplement = match default_supplement {
		Some(Value::Content(c))	=> c,
		Some(Value::Auto) | None if kind == ElemKind::Equation	=> common::text(common::local(styles, "equation")),
		_						=> Content::empty(),
	};
	let loc = res!(located(engine, &found, span));
	let numbers = res!(lookup::display_counter(engine, &counter, loc, &numbering, span));
	let supplement = match res!(common::get(elem, styles, "supplement")) {
		Value::Auto	=> default_supplement,
		other		=> res!(heading::resolve_supplement(engine, &other, styles, "heading", &found))
			.unwrap_or_else(Content::empty),
	};
	let mut content = numbers;
	if !supplement.is_empty() {
		content = common::seq(vec![supplement, common::text("\u{a0}"), content]);
	}
	link::to_location(common::spanned(content, span), loc)
}

fn located(engine: &mut Engine, e: &Content, span: Span) -> Outcome<crate::eval::locate::Location> {
	match e.location() {
		Some(l)	=> Ok(l),
		None	=> Err(engine.error(DiagnosticKind::Type, span, "cannot reference an element without a location")),
	}
}

/// The counter a referenceable element counts on: a figure's own (`figure.where(kind: ..)`), else its
/// kind's.
fn refable_counter(e: &Content) -> Outcome<Counter> {
	if let Some(Value::Counter(c)) = e.field("counter") {
		return Ok((**c).clone());
	}
	match e.kind() {
		Some(k)	=> Ok(lookup::elem_counter(k)),
		None	=> Err(err!("a referenced element has no kind"; Bug)),
	}
}

/// Is `l` a bibliography key rather than a document label? For the quote attribution and citations.
pub fn is_bib_key(engine: &Engine, l: &Label) -> bool { lookup::bib_has(engine, l) }
