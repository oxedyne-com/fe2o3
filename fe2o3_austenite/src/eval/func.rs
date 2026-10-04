// U0 owns this file. `NativeFunc`'s variant list is fixed; each inner enum, and the `call` it dispatches
// to, belongs to the file named beside it. Calling any `Func` is `Engine::call_func` in `eval.rs` (U2).

use crate::eval::args::Args;
use crate::eval::content::ElemKind;
use crate::eval::lib::{
	array::ArrayFn,
	calc::CalcFn,
	color::ColorFn,
	data::DataFn,
	datetime::DatetimeFn,
	dict::DictFn,
	foundations::FoundFn,
	geom::GeomFn,
	intro::IntroFn,
	layout::LayoutFn,
	math::MathFn,
	model::ModelFn,
	numbering::NumberingFn,
	pdf::PdfFn,
	string::StrFn,
	sym::SymFn,
	text::TextFn,
	visual::VisualFn,
};
use crate::eval::lib;
use crate::eval::methods::CoreFn;
use crate::eval::scope::Scope;
use crate::eval::select::StyleFn;
use crate::eval::value::Value;
use crate::eval::Engine;
use crate::syntax::{
	Span,
	SyntaxNode,
};

use oxedyne_fe2o3_core::prelude::*;

use std::sync::Arc;

#[derive(Clone, Debug)]
pub enum Func {
	Native(NativeFunc),
	Element(ElemKind),
	Closure(Arc<Closure>),
	With(Arc<(Func, Args)>),	// `f.with(..)`: arguments applied before the call's own
}

impl Func {
	/// The name `repr` prints, `None` for an anonymous closure.
	pub fn name(&self) -> Option<&str> {
		match self {
			Func::Native(n)		=> Some(n.name()),
			Func::Element(k)	=> Some(k.name()),
			Func::Closure(c)	=> c.name.as_deref(),
			Func::With(w)		=> w.0.name(),
		}
	}

	pub fn with(self, args: Args) -> Func { Func::With(Arc::new((self, args))) }

	/// The element this function constructs, seeing through `.with`, for `show heading: ..` and
	/// `heading.where(..)`.
	pub fn element(&self) -> Option<ElemKind> {
		match self {
			Func::Element(k)	=> Some(*k),
			Func::With(w)		=> w.0.element(),
			_					=> None,
		}
	}
}

/// A closure parameter. A named default is evaluated when the closure is defined, as in Typst.
#[derive(Clone, Debug)]
pub enum Param {
	Pos(SyntaxNode),				// an identifier or a destructuring pattern
	Named { name: String, default: Value },
	Sink(Option<String>),			// `..rest`, or bare `..`
}

/// A user function. `captured` is a snapshot of the defining scope: Typst closures capture by value.
#[derive(Clone, Debug)]
pub struct Closure {
	pub name:		Option<String>,	// set by `let f(x) = ..`, for recursion and `repr`
	pub params:		Vec<Param>,
	pub body:		SyntaxNode,
	pub captured:	Scope,
	pub span:		Span,
}

/// Every native function, grouped by the library area that implements it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NativeFunc {
	Core(CoreFn),				// methods.rs, U2: `with`, content/args/func methods
	Found(FoundFn),				// lib/foundations.rs, U3
	Calc(CalcFn),				// lib/calc.rs, U3
	Str(StrFn),					// lib/string.rs, U3
	Array(ArrayFn),				// lib/array.rs, U3
	Dict(DictFn),				// lib/dict.rs, U3
	Numbering(NumberingFn),		// lib/numbering.rs, U3
	Color(ColorFn),				// lib/color.rs, U3
	Datetime(DatetimeFn),		// lib/datetime.rs, U3
	Data(DataFn),				// lib/data.rs, U3
	Sym(SymFn),					// lib/sym.rs, U3
	Geom(GeomFn),				// lib/geom.rs, U3: length, angle, ratio, alignment, direction methods
	Style(StyleFn),				// select.rs, U4
	Text(TextFn),				// lib/text.rs, U6a
	Layout(LayoutFn),			// lib/layout.rs, U6b
	Visual(VisualFn),			// lib/visual.rs, U6d
	Model(ModelFn),				// lib/model/mod.rs, U5
	Math(MathFn),				// lib/math.rs, U7
	Intro(IntroFn),				// lib/intro.rs, U8
	Pdf(PdfFn),					// lib/pdf.rs
}

impl NativeFunc {
	pub fn name(&self) -> &'static str {
		match self {
			NativeFunc::Core(f)			=> f.name(),
			NativeFunc::Found(f)		=> f.name(),
			NativeFunc::Calc(f)			=> f.name(),
			NativeFunc::Str(f)			=> f.name(),
			NativeFunc::Array(f)		=> f.name(),
			NativeFunc::Dict(f)			=> f.name(),
			NativeFunc::Numbering(f)	=> f.name(),
			NativeFunc::Color(f)		=> f.name(),
			NativeFunc::Datetime(f)		=> f.name(),
			NativeFunc::Data(f)			=> f.name(),
			NativeFunc::Sym(f)			=> f.name(),
			NativeFunc::Geom(f)			=> f.name(),
			NativeFunc::Style(f)		=> f.name(),
			NativeFunc::Text(f)			=> f.name(),
			NativeFunc::Layout(f)		=> f.name(),
			NativeFunc::Visual(f)		=> f.name(),
			NativeFunc::Model(f)		=> f.name(),
			NativeFunc::Math(f)			=> f.name(),
			NativeFunc::Intro(f)		=> f.name(),
			NativeFunc::Pdf(f)			=> f.name(),
		}
	}

	/// Runs the function. A method call arrives with its receiver as the first positional argument.
	pub fn call(self, engine: &mut Engine, args: Args) -> Outcome<Value> {
		match self {
			NativeFunc::Core(f)			=> crate::eval::methods::call(f, engine, args),
			NativeFunc::Found(f)		=> lib::foundations::call(f, engine, args),
			NativeFunc::Calc(f)			=> lib::calc::call(f, engine, args),
			NativeFunc::Str(f)			=> lib::string::call(f, engine, args),
			NativeFunc::Array(f)		=> lib::array::call(f, engine, args),
			NativeFunc::Dict(f)			=> lib::dict::call(f, engine, args),
			NativeFunc::Numbering(f)	=> lib::numbering::call(f, engine, args),
			NativeFunc::Color(f)		=> lib::color::call(f, engine, args),
			NativeFunc::Datetime(f)		=> lib::datetime::call(f, engine, args),
			NativeFunc::Data(f)			=> lib::data::call(f, engine, args),
			NativeFunc::Sym(f)			=> lib::sym::call(f, engine, args),
			NativeFunc::Geom(f)			=> lib::geom::call(f, engine, args),
			NativeFunc::Style(f)		=> crate::eval::select::call(f, engine, args),
			NativeFunc::Text(f)			=> lib::text::call(f, engine, args),
			NativeFunc::Layout(f)		=> lib::layout::call(f, engine, args),
			NativeFunc::Visual(f)		=> lib::visual::call(f, engine, args),
			NativeFunc::Model(f)		=> lib::model::call(f, engine, args),
			NativeFunc::Math(f)			=> lib::math::call(f, engine, args),
			NativeFunc::Intro(f)		=> lib::intro::call(f, engine, args),
			NativeFunc::Pdf(f)			=> lib::pdf::call(f, engine, args),
		}
	}
}

/// The error every stub `call` returns until its unit lands.
pub fn unimplemented(area: &str, name: &str) -> Error<ErrTag> {
	err!("{}.{} is not implemented yet", area, name; Unimplemented)
}
