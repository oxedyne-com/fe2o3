// U4 owns this file: selectors and the `selector` functions and methods.

use crate::eval::args::Args;
use crate::eval::content::{
	Content,
	ElemKind,
	FieldId,
};
use crate::eval::func::unimplemented;
use crate::eval::locate::Location;
use crate::eval::styles::StyleChain;
use crate::eval::value::{
	Label,
	RegexValue,
	Value,
};
use crate::eval::Engine;

use oxedyne_fe2o3_core::prelude::*;

use std::sync::Arc;

#[derive(Clone, Debug)]
pub enum Selector {
	Elem(ElemKind, Option<Vec<(FieldId, Value)>>),	// `heading`, `heading.where(level: 1)`
	Label(Label),
	Text(String),									// `show "word"`
	Regex(Arc<RegexValue>),
	Location(Location),
	Or(Vec<Selector>),
	And(Vec<Selector>),
	Before { selector: Arc<Selector>, end: Arc<Selector>, inclusive: bool },
	After { selector: Arc<Selector>, start: Arc<Selector>, inclusive: bool },
}

impl Selector {
	/// Does the element match? Text and regex selectors match text runs, which `realise.rs` handles;
	/// here they never match an element.
	pub fn matches(&self, _elem: &Content, _styles: Option<&StyleChain>) -> Outcome<bool> {
		Err(unimplemented("selector", "matches"))
	}
}

native_fns! {
	pub enum StyleFn {
		Selector	=> "selector",
		Or			=> "or",
		And			=> "and",
		Before		=> "before",
		After		=> "after",
		Where		=> "where",
	}
}

pub fn method(_name: &str) -> Option<StyleFn> { None }

pub fn call(f: StyleFn, _engine: &mut Engine, _args: Args) -> Outcome<Value> {
	Err(unimplemented("selector", f.name()))
}
