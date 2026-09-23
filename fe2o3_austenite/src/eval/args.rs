// U2 owns this file. Every native function in every unit parses its arguments through these methods;
// their names and semantics are Typst's `Args`.

use crate::eval::value::{
	FromValue,
	Value,
};
use crate::syntax::Span;

use oxedyne_fe2o3_core::prelude::*;

#[derive(Clone, Debug)]
pub struct Arg {
	pub span:	Span,
	pub name:	Option<String>,
	pub value:	Value,
}

/// Call arguments in source order. Taking an argument removes it; `finish` then rejects leftovers.
#[derive(Clone, Debug, Default)]
pub struct Args {
	pub span:	Span,
	pub items:	Vec<Arg>,
}

impl Args {
	pub fn new(span: Span) -> Self { Self { span, items: Vec::new() } }

	pub fn push(&mut self, span: Span, value: Value) {
		self.items.push(Arg { span, name: None, value });
	}

	pub fn push_named<S: Into<String>>(&mut self, span: Span, name: S, value: Value) {
		self.items.push(Arg { span, name: Some(name.into()), value });
	}

	/// Puts a value in front of the positional arguments: a method's receiver, or `.with`'s bound ones.
	pub fn prepend(&mut self, span: Span, value: Value) {
		self.items.insert(0, Arg { span, name: None, value });
	}

	/// The next positional argument cast to `T`, or `None` when there is none.
	pub fn eat<T: FromValue>(&mut self) -> Outcome<Option<T>> {
		match self.items.iter().position(|a| a.name.is_none()) {
			Some(i) => {
				let a = self.items.remove(i);
				Ok(Some(res!(T::from_value(a.value))))
			}
			None => Ok(None),
		}
	}

	/// The next positional argument, required: absent is "missing argument: `what`".
	pub fn expect<T: FromValue>(&mut self, what: &str) -> Outcome<T> {
		match res!(self.eat::<T>()) {
			Some(v)	=> Ok(v),
			None	=> Err(err!("missing argument: {}", what; Input, Missing)),
		}
	}

	/// The first positional argument the predicate accepts, removed and cast.
	pub fn find<T: FromValue>(&mut self, pred: impl Fn(&Value) -> bool) -> Outcome<Option<T>> {
		match self.items.iter().position(|a| a.name.is_none() && pred(&a.value)) {
			Some(i) => {
				let a = self.items.remove(i);
				Ok(Some(res!(T::from_value(a.value))))
			}
			None => Ok(None),
		}
	}

	/// Every remaining positional argument, cast.
	pub fn all<T: FromValue>(&mut self) -> Outcome<Vec<T>> {
		let mut out = Vec::new();
		while let Some(v) = res!(self.eat::<T>()) {
			out.push(v);
		}
		Ok(out)
	}

	/// A named argument; given twice, the last wins, as in Typst.
	pub fn named<T: FromValue>(&mut self, name: &str) -> Outcome<Option<T>> {
		let mut found = None;
		let mut i = 0;
		while i < self.items.len() {
			if self.items[i].name.as_deref() == Some(name) {
				found = Some(self.items.remove(i).value);
			} else {
				i += 1;
			}
		}
		match found {
			Some(v)	=> Ok(Some(res!(T::from_value(v)))),
			None	=> Ok(None),
		}
	}

	/// Every argument, positional and named, moved out; the span stays.
	pub fn take(&mut self) -> Args {
		Args { span: self.span, items: std::mem::take(&mut self.items) }
	}

	/// How many positional arguments remain.
	pub fn pos_count(&self) -> usize { self.items.iter().filter(|a| a.name.is_none()).count() }

	pub fn is_empty(&self) -> bool { self.items.is_empty() }

	/// Fails on any argument not taken.
	pub fn finish(self) -> Outcome<()> {
		match self.items.first() {
			None						=> Ok(()),
			Some(Arg { name: Some(n), .. })	=> Err(err!("unexpected argument: {}", n; Input, Unexpected)),
			Some(_)						=> Err(err!("unexpected argument"; Input, Unexpected)),
		}
	}
}
