//! Contract (U0, 2026-09-23): the standard library. Every area file exposes
//! `define(&mut Scope)` for its globals, its inner native enum and `call(f, engine, args)`; an area
//! whose type has methods also exposes `method(name) -> Option<XFn>`. An element family file
//! (`text`, `layout`, `grid`, `visual`, `math`, `intro`, `model`) additionally exposes `fields(kind)`,
//! `construct(engine, kind, args) -> Outcome<Option<Content>>` (`None` to use the generic schema walk)
//! and `show(engine, elem, styles) -> Outcome<Option<Content>>` (`None` for a primitive flow lays out).
//! Global element functions are registered here generically; `math.*` elements are the math area's.
//! The file is `string.rs`, not `str.rs`, because a module named `str` shadows the primitive type.

pub mod array;
pub mod calc;
pub mod color;
pub mod data;
pub mod datetime;
pub mod decimal;
pub mod dict;
pub mod foundations;
pub mod geom;
pub mod grid;
pub mod intro;
pub mod layout;
pub mod math;
pub mod model;
pub mod numbering;
pub mod pdf;
pub mod string;
pub mod sym;
pub mod text;
pub mod visual;

use crate::eval::content::ElemKind;
use crate::eval::func::Func;
use crate::eval::scope::Scope;
use crate::eval::value::Value;

/// The root scope every module's scope chains to.
pub fn library() -> Scope {
	let mut s = Scope::new();
	for k in ElemKind::ALL {
		if k.is_global() {
			s.define(k.path(), Value::Func(Func::Element(*k)));
		}
	}
	foundations::define(&mut s);
	calc::define(&mut s);
	string::define(&mut s);
	array::define(&mut s);
	dict::define(&mut s);
	numbering::define(&mut s);
	color::define(&mut s);
	datetime::define(&mut s);
	data::define(&mut s);
	sym::define(&mut s);
	geom::define(&mut s);
	text::define(&mut s);
	layout::define(&mut s);
	grid::define(&mut s);
	visual::define(&mut s);
	model::define(&mut s);
	math::define(&mut s);
	intro::define(&mut s);
	pdf::define(&mut s);
	// `std` is the library as a module, so a name a document redefines stays reachable as `std.text`.
	let std = s.clone();
	s.define("std", Value::Module(std::sync::Arc::new(crate::eval::value::Module::new("std", std))));
	s
}

/// Defines every element whose path starts with `prefix.` under its last segment, for a module scope
/// such as `math`.
pub fn define_elements(scope: &mut Scope, prefix: &str) {
	for k in ElemKind::ALL {
		let p = k.path();
		if p.len() > prefix.len() + 1 && p.starts_with(prefix) && p.as_bytes().get(prefix.len()) == Some(&b'.') {
			scope.define(k.name(), Value::Func(Func::Element(*k)));
		}
	}
}
