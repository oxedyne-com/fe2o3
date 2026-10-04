// SPDX-License-Identifier: Apache-2.0
// Original work copyright (c) the Typst project authors (typst-library `model/mod.rs`, version 0.15.1).
// Modified for Austenite: the element's fields, defaults and default show, ported to Hematite's types, IR and error handling.
//! Contract (U0, 2026-09-23; U5 owns this directory): the model family. Each element's schema and
//! native show live in the submodule named for it, and this file only dispatches by `ElemKind`. A native
//! show reproduces Typst's own default (heading sizes, list indents, figure supplement), never Austenite's
//! old `Theme`; `par` and `parbreak` are primitives flow sets, so their show is `None`.
//!
//! Beyond the contract's `fields`/`construct`/`show`, the family answers the hooks Typst's element traits
//! give it, each a plain function realisation and introspection call by kind: `cast` (a field's value
//! check, as `construct` and `set` apply it), `default_value` (the defaults no `static` can hold),
//! `synthesise` (Typst's `Synthesize`), `show_set` (`ShowSet`, the built-in styles a show rule sees),
//! `count_step` (`Count`) and `method` (functions scoped to an element, `outline.entry.indented`).
//! Schemas list Typst's public fields in Typst's order; a Typst-internal field (`list`'s depth) is marked
//! `internal`, so it is never set, constructed, materialised or shown by `fields()` and `repr`.

pub mod bibliography;
pub mod common;
pub mod divider;
pub mod document;
pub mod emph;
pub mod figure;
pub mod footnote;
pub mod heading;
pub mod link;
pub mod list;
pub mod local;
pub mod lookup;
pub mod outline;
pub mod par;
pub mod quote;
pub mod raw;
pub mod reference;

use crate::diag::DiagnosticKind;
use crate::eval::args::Args;
use crate::eval::content::{
	Content,
	ElemKind,
	FieldId,
	FieldSpec,
};
use crate::eval::scope::Scope;
use crate::eval::styles::{
	StyleChain,
	Styles,
};
use crate::eval::value::Value;
use crate::eval::Engine;
use crate::syntax::Span;

use common::CastErr;

use oxedyne_fe2o3_core::prelude::*;

native_fns! {
	pub enum ModelFn {
		EntryIndented	=> "indented",
		EntryPrefix		=> "prefix",
		EntryInner		=> "inner",
		EntryBody		=> "body",
		EntryPage		=> "page",
	}
}

pub fn define(_scope: &mut Scope) {}

pub fn call(f: ModelFn, engine: &mut Engine, args: Args) -> Outcome<Value> {
	match f {
		ModelFn::EntryIndented | ModelFn::EntryPrefix | ModelFn::EntryInner | ModelFn::EntryBody
			| ModelFn::EntryPage	=> outline::call(f, engine, args),
	}
}

/// The function scoped to an element that a method call on such content reaches:
/// `it.indented(it.prefix(), it.inner())` on an `outline.entry`.
pub fn method(kind: ElemKind, name: &str) -> Option<ModelFn> {
	match kind {
		ElemKind::OutlineEntry	=> outline::method(name),
		_						=> None,
	}
}

pub fn fields(kind: ElemKind) -> &'static [FieldSpec] {
	match kind {
		ElemKind::Par | ElemKind::ParLine | ElemKind::Parbreak	=> par::fields(kind),
		ElemKind::Strong | ElemKind::Emph						=> emph::fields(kind),
		ElemKind::Raw | ElemKind::RawLine						=> raw::fields(kind),
		ElemKind::Heading | ElemKind::Title						=> heading::fields(kind),
		ElemKind::List | ElemKind::ListItem | ElemKind::Enum | ElemKind::EnumItem
			| ElemKind::Terms | ElemKind::TermItem				=> list::fields(kind),
		ElemKind::Link											=> link::fields(kind),
		ElemKind::Ref | ElemKind::Cite							=> reference::fields(kind),
		ElemKind::Footnote | ElemKind::FootnoteEntry			=> footnote::fields(kind),
		ElemKind::Figure | ElemKind::FigureCaption				=> figure::fields(kind),
		ElemKind::Outline | ElemKind::OutlineEntry				=> outline::fields(kind),
		ElemKind::Quote											=> quote::fields(kind),
		ElemKind::Bibliography | ElemKind::CiteGroup			=> bibliography::fields(kind),
		ElemKind::Document										=> document::fields(kind),
		_														=> &[],
	}
}

/// A field's default: the schema's, or the one the model computes where no `static` can hold it (a
/// list's marker array, `terms`'s separator, `outline`'s target selector).
pub fn default_value(kind: ElemKind, name: &str) -> Option<Value> {
	let computed = match kind {
		ElemKind::Par | ElemKind::ParLine						=> par::default_value(kind, name),
		ElemKind::List | ElemKind::Enum | ElemKind::Terms		=> list::default_value(kind, name),
		ElemKind::FootnoteEntry									=> footnote::default_value(kind, name),
		ElemKind::Raw											=> raw::default_value(kind, name),
		ElemKind::FigureCaption									=> figure::default_value(kind, name),
		ElemKind::Outline | ElemKind::OutlineEntry				=> outline::default_value(kind, name),
		_														=> None,
	};
	if computed.is_some() {
		return computed;
	}
	kind.field_id(name).and_then(|id| kind.field_spec(id)).and_then(|s| s.default.to_value())
}

/// Checks, and where Typst does normalises, a value given for a field: what `construct` applies to an
/// argument and a `set` rule should apply to its value.
pub fn cast(kind: ElemKind, name: &str, v: Value) -> Result<Value, CastErr> {
	match kind {
		ElemKind::Par | ElemKind::ParLine | ElemKind::Parbreak	=> par::cast(kind, name, v),
		ElemKind::Strong | ElemKind::Emph						=> emph::cast(kind, name, v),
		ElemKind::Raw | ElemKind::RawLine						=> raw::cast(kind, name, v),
		ElemKind::Heading | ElemKind::Title						=> heading::cast(kind, name, v),
		ElemKind::List | ElemKind::ListItem | ElemKind::Enum | ElemKind::EnumItem
			| ElemKind::Terms | ElemKind::TermItem				=> list::cast(kind, name, v),
		ElemKind::Link											=> link::cast(kind, name, v),
		ElemKind::Ref | ElemKind::Cite							=> reference::cast(kind, name, v),
		ElemKind::Footnote | ElemKind::FootnoteEntry			=> footnote::cast(kind, name, v),
		ElemKind::Figure | ElemKind::FigureCaption				=> figure::cast(kind, name, v),
		ElemKind::Outline | ElemKind::OutlineEntry				=> outline::cast(kind, name, v),
		ElemKind::Quote											=> quote::cast(kind, name, v),
		ElemKind::Bibliography									=> bibliography::cast(kind, name, v),
		ElemKind::Document										=> document::cast(kind, name, v),
		ElemKind::CiteGroup										=> bibliography::cast_group(name, v),
		_														=> Ok(v),
	}
}

/// A `set` rule's value for a field, checked as the constructor checks it.
pub fn cast_field(kind: ElemKind, name: &str, v: Value) -> Outcome<Value> {
	match cast(kind, name, v) {
		Ok(v)	=> Ok(v),
		Err(e)	=> Err(err!("{}", e.message(); Input, Invalid)),
	}
}

/// Folds a `Fold::Custom` field: `par.justification-limits`, whose `spacing` and `tracking` are each
/// kept from the outer value where the inner one does not give them.
pub fn fold(kind: ElemKind, field: &str, inner: Value, outer: Value) -> Outcome<Value> {
	match (kind, field, &inner, &outer) {
		(ElemKind::Par, "justification-limits", Value::Dict(i), Value::Dict(o)) => {
			let mut d = (**o).clone();
			for (k, v) in i.iter() {
				d.insert(k, v.clone());
			}
			Ok(Value::dict(d))
		}
		_ => Ok(inner),
	}
}

pub fn construct(engine: &mut Engine, kind: ElemKind, args: &mut Args) -> Outcome<Option<Content>> {
	match kind {
		ElemKind::ParLine		=> Err(engine.error(DiagnosticKind::Type, args.span, "cannot be constructed manually")),
		ElemKind::Link			=> link::construct(engine, args).map(Some),
		ElemKind::Bibliography	=> bibliography::construct(engine, args).map(Some),
		ElemKind::Cite			=> reference::construct_cite(engine, args).map(Some),
		_						=> construct_schema(engine, kind, args).map(Some),
	}
}

/// Typst's generated constructor: required positionals in order, an optional positional from the
/// first argument of a castable type, variadics from the rest, named fields by name; then anything left
/// over is an unexpected argument. Each value passes the field's `cast`.
pub fn construct_schema(engine: &mut Engine, kind: ElemKind, args: &mut Args) -> Outcome<Content> {
	let span = args.span;
	let mut fields = Vec::new();
	for (i, spec) in kind.fields().iter().enumerate() {
		if spec.synthesised || spec.internal {
			continue;
		}
		let id = FieldId(i as u8);
		if spec.variadic {
			let mut items = Vec::new();
			while let Some(p) = args.items.iter().position(|a| a.name.is_none()) {
				let a = args.items.remove(p);
				items.push(res!(cast_arg(engine, kind, spec.name, a.value, a.span)));
			}
			fields.push((id, Value::array(items)));
		} else if spec.positional && spec.required {
			let a = match args.items.iter().position(|a| a.name.is_none()) {
				Some(p)	=> args.items.remove(p),
				None	=> return Err(engine.error(DiagnosticKind::Type, span, fmt!("missing argument: {}", spec.name))),
			};
			fields.push((id, res!(cast_arg(engine, kind, spec.name, a.value, a.span))));
		} else if spec.positional {
			let mut taken = None;
			let mut p = 0;
			while p < args.items.len() {
				if args.items[p].name.is_none() {
					let v = args.items[p].value.clone();
					match cast(kind, spec.name, v) {
						Err(CastErr::Type(_))	=> (),
						Err(CastErr::Value(m))	=> return Err(engine.error(DiagnosticKind::Type, args.items[p].span, m)),
						Ok(v)					=> {
							taken = Some(v);
							args.items.remove(p);
							break;
						}
					}
				}
				p += 1;
			}
			if let Some(v) = taken {
				fields.push((id, v));
			}
		} else {
			let mut found = None;
			let mut p = 0;
			while p < args.items.len() {
				if args.items[p].name.as_deref() == Some(spec.name) {
					found = Some(args.items.remove(p));
				} else {
					p += 1;
				}
			}
			if let Some(a) = found {
				fields.push((id, res!(cast_arg(engine, kind, spec.name, a.value, a.span))));
			}
		}
	}
	res!(finish(engine, args));
	Ok(Content::new(kind, fields, span))
}

fn cast_arg(engine: &mut Engine, kind: ElemKind, name: &str, v: Value, span: Span) -> Outcome<Value> {
	// Typst still takes an array as an enumeration or term list item, with a warning.
	if matches!(v, Value::Array(_)) && name == "children" {
		match kind {
			ElemKind::Enum	=> engine.warn(DiagnosticKind::Lint, span, "implicit conversion from array to `enum.item` is deprecated"),
			ElemKind::Terms	=> engine.warn(DiagnosticKind::Lint, span, "implicit conversion from array to `terms.item` is deprecated"),
			_				=> (),
		}
	}
	match cast(kind, name, v) {
		Ok(v)	=> Ok(v),
		Err(e)	=> Err(engine.error(DiagnosticKind::Type, span, e.message())),
	}
}

/// Refuses any argument left: Typst's `Args::finish`.
pub fn finish(engine: &mut Engine, args: &mut Args) -> Outcome<()> {
	match args.items.first() {
		None	=> Ok(()),
		Some(a)	=> {
			let span = if a.span.is_detached() { args.span } else { a.span };
			let msg = match &a.name {
				Some(n)	=> fmt!("unexpected argument: {}", n),
				None	=> "unexpected argument".to_string(),
			};
			Err(engine.error(DiagnosticKind::Type, span, msg))
		}
	}
}

/// Typst's `Synthesize`: fills the fields a show rule sees that only realisation can know (a heading's
/// resolved level and supplement, a figure's kind and counter, a reference's target). Realisation calls
/// it once the element has its location and its unset fields are materialised from `styles`.
pub fn synthesise(engine: &mut Engine, elem: &mut Content, styles: &StyleChain) -> Outcome<()> {
	// A default no schema constant holds is materialised here, as the chain cannot supply it.
	if let (Some(kind), Content::Elem(e)) = (elem.kind(), &mut *elem) {
		let e = std::sync::Arc::make_mut(e);
		for (i, spec) in kind.fields().iter().enumerate() {
			let id = FieldId(i as u8);
			if !spec.settable || spec.default != crate::eval::content::FieldDefault::Computed
				|| e.fields.iter().any(|(f, _)| *f == id)
			{
				continue;
			}
			if let Some(v) = default_value(kind, spec.name) {
				e.fields.push((id, v));
			}
		}
	}
	match elem.kind() {
		Some(ElemKind::Heading)			=> heading::synthesise(engine, elem, styles),
		Some(ElemKind::Figure)			=> figure::synthesise(engine, elem, styles),
		Some(ElemKind::FigureCaption)	=> figure::synthesise_caption(elem, styles),
		Some(ElemKind::Ref)				=> reference::synthesise(engine, elem, styles),
		Some(ElemKind::Raw)				=> raw::synthesise(elem, styles),
		_								=> Ok(()),
	}
}

/// [`content::derived_field`] for a model element: a heading's `level`, a figure's `kind`.
pub fn derived_field(elem: &Content, field: FieldId, styles: &StyleChain) -> Outcome<Option<Value>> {
	let kind = match elem.kind() {
		Some(k)	=> k,
		None	=> return Ok(None),
	};
	let name = match kind.field_spec(field) {
		Some(spec)	=> spec.name,
		None		=> return Ok(None),
	};
	match (kind, name) {
		(ElemKind::Heading, "level")	=> Ok(Some(Value::Int(res!(heading::level(elem, styles))))),
		(ElemKind::Figure, "kind")		=> Ok(Some(res!(figure::resolved_kind(elem, styles)))),
		_								=> Ok(None),
	}
}

/// Typst's `ShowSet`: the styles an element's own default applies to it and its show-rule output,
/// visible inside a user's `show` rule for it (a heading's size and weight).
pub fn show_set(elem: &Content, styles: &StyleChain) -> Outcome<Styles> {
	match elem.kind() {
		Some(ElemKind::Heading)			=> heading::show_set(elem, styles),
		Some(ElemKind::Title)			=> heading::show_set_title(),
		Some(ElemKind::Raw)				=> raw::show_set(elem, styles),
		Some(ElemKind::Link)			=> link::show_set(),
		Some(ElemKind::FootnoteEntry)	=> footnote::show_set_entry(),
		Some(ElemKind::Figure)			=> figure::show_set(),
		Some(ElemKind::Outline)			=> outline::show_set(elem, styles),
		Some(ElemKind::Quote)			=> quote::show_set(elem, styles),
		Some(ElemKind::Bibliography)	=> bibliography::show_set(),
		Some(ElemKind::Divider)			=> divider::show_set(),
		_								=> Ok(Styles::new()),
	}
}

/// Typst's `Count`: the level at which a realised element steps its own counter, if it does. A heading
/// steps at its level, a numbered figure, footnote or equation at level one.
pub fn count_step(elem: &Content) -> Option<usize> {
	match elem.kind() {
		Some(ElemKind::Heading) => {
			if matches!(elem.field("numbering"), None | Some(Value::None)) {
				return None;
			}
			match elem.field("level") {
				Some(Value::Int(l)) if *l > 0	=> Some(*l as usize),
				_ => {
					let depth = match elem.field("depth") { Some(Value::Int(d)) => *d, _ => 1 };
					let offset = match elem.field("offset") { Some(Value::Int(o)) => *o, _ => 0 };
					Some((depth + offset).max(1) as usize)
				}
			}
		}
		Some(ElemKind::Figure) => match elem.field("numbering") {
			None | Some(Value::None)	=> None,
			Some(_)						=> Some(1),
		},
		// Only a numbered block equation counts.
		Some(ElemKind::Equation) => {
			let block = matches!(elem.field("block"), Some(Value::Bool(true)));
			match elem.field("numbering") {
				Some(n) if block && !n.is_none()	=> Some(1),
				_									=> None,
			}
		}
		Some(ElemKind::Footnote) => match elem.field("body") {
			Some(Value::Label(_))	=> None,
			_						=> Some(1),
		},
		_ => None,
	}
}

pub fn show(engine: &mut Engine, elem: &Content, styles: &StyleChain) -> Outcome<Option<Content>> {
	let kind = match elem.kind() {
		Some(k)	=> k,
		None	=> return Ok(None),
	};
	match kind {
		ElemKind::Par | ElemKind::ParLine | ElemKind::Parbreak	=> Ok(None),
		ElemKind::Strong | ElemKind::Emph						=> emph::show(engine, elem, styles),
		ElemKind::Raw | ElemKind::RawLine						=> raw::show(engine, elem, styles),
		ElemKind::Heading | ElemKind::Title						=> heading::show(engine, elem, styles),
		ElemKind::List | ElemKind::ListItem | ElemKind::Enum | ElemKind::EnumItem
			| ElemKind::Terms | ElemKind::TermItem				=> list::show(engine, elem, styles),
		ElemKind::Link											=> link::show(engine, elem, styles),
		ElemKind::Ref | ElemKind::Cite							=> reference::show(engine, elem, styles),
		ElemKind::Footnote | ElemKind::FootnoteEntry			=> footnote::show(engine, elem, styles),
		ElemKind::Figure | ElemKind::FigureCaption				=> figure::show(engine, elem, styles),
		ElemKind::Outline | ElemKind::OutlineEntry				=> outline::show(engine, elem, styles),
		ElemKind::Quote											=> quote::show(engine, elem, styles),
		ElemKind::Bibliography | ElemKind::CiteGroup			=> bibliography::show(engine, elem, styles),
		ElemKind::Document										=> document::show(engine, elem, styles),
		ElemKind::Divider										=> divider::show(elem),
		_ => Err(err!("{} is not a model element", kind.path(); Bug)),
	}
}
