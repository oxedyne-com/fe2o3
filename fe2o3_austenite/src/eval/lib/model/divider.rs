// The model family (U5's directory): divider. Typst's thematic break shows as a bare `line`; its full
// width, its 0.05em stroke and the 2em of block space around it are the element's built-in show-set styles,
// so `show divider: set line(..)` restyles it as in Typst.

use crate::eval::content::{
	Content,
	ElemKind,
};
use crate::eval::lib::model::common;
use crate::eval::styles::Styles;
use crate::eval::value::{
	Length,
	Ratio,
	Relative,
	Value,
};

use oxedyne_fe2o3_core::prelude::*;

pub fn show_set() -> Outcome<Styles> {
	common::props(vec![
		(ElemKind::Block,	"above",	common::em(2.0)),
		(ElemKind::Block,	"below",	common::em(2.0)),
		(ElemKind::Line,	"length",	Value::Relative(Relative { abs: Length::zero(), rel: Ratio(1.0) })),
		(ElemKind::Line,	"stroke",	common::em(0.05)),
	])
}

pub fn show(elem: &Content) -> Outcome<Option<Content>> {
	Ok(Some(Content::new(ElemKind::Line, Vec::new(), elem.span())))
}
