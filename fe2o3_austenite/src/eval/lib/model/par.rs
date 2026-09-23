// U5 owns this file: par, par.line and parbreak, primitives the flow sets.

use crate::eval::content::{
	ElemKind,
	FieldSpec,
};

pub fn fields(_kind: ElemKind) -> &'static [FieldSpec] { &[] }
