// U5 owns this file: bibliography, over crate::bib (Chicago author-date; another CSL style is a diagnostic).

use crate::eval::content::{
	Content,
	ElemKind,
	FieldSpec,
};
use crate::eval::func::unimplemented;
use crate::eval::styles::StyleChain;
use crate::eval::Engine;

use oxedyne_fe2o3_core::prelude::*;

pub fn fields(_kind: ElemKind) -> &'static [FieldSpec] { &[] }

pub fn show(_engine: &mut Engine, elem: &Content, _styles: &StyleChain) -> Outcome<Option<Content>> {
	Err(unimplemented("show", elem.kind().map(|k| k.path()).unwrap_or("content")))
}
