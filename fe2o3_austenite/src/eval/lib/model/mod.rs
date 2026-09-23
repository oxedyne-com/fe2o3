//! Contract (U0, 2026-09-23; U5 owns this directory): the model family. Each element's schema and
//! native show live in the submodule named for it, and this file only dispatches by `ElemKind`. A native
//! show reproduces Typst's own default (heading sizes, list indents, figure supplement), never Austenite's
//! old `Theme`; `par` and `parbreak` are primitives flow sets, so their show is `None`.

pub mod bibliography;
pub mod document;
pub mod emph;
pub mod figure;
pub mod footnote;
pub mod heading;
pub mod link;
pub mod list;
pub mod outline;
pub mod par;
pub mod quote;
pub mod raw;
pub mod reference;

use crate::eval::args::Args;
use crate::eval::content::{
	Content,
	ElemKind,
	FieldSpec,
};
use crate::eval::func::unimplemented;
use crate::eval::scope::Scope;
use crate::eval::styles::StyleChain;
use crate::eval::value::Value;
use crate::eval::Engine;

use oxedyne_fe2o3_core::prelude::*;

native_fns! {
	pub enum ModelFn {
	}
}

pub fn define(_scope: &mut Scope) {}

pub fn call(f: ModelFn, _engine: &mut Engine, _args: Args) -> Outcome<Value> {
	match f {}
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
		ElemKind::Bibliography									=> bibliography::fields(kind),
		ElemKind::Document										=> document::fields(kind),
		_														=> &[],
	}
}

pub fn construct(_engine: &mut Engine, _kind: ElemKind, _args: &mut Args) -> Outcome<Option<Content>> {
	Ok(None)
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
		ElemKind::Bibliography									=> bibliography::show(engine, elem, styles),
		ElemKind::Document										=> Ok(None),
		_ => Err(unimplemented("model", kind.path())),
	}
}
