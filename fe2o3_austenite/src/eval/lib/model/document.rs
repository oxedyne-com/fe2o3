// U5 owns this file: set document(title, author, keywords, date), read by U9 into PDF metadata.

use crate::eval::content::{
	ElemKind,
	FieldSpec,
};

pub fn fields(_kind: ElemKind) -> &'static [FieldSpec] { &[] }
