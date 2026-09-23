// U2 owns this file. U0 wrote `define` and `get` for real because every library area's `define`
// registers through them.

use crate::eval::value::Value;
use crate::syntax::Span;

use std::collections::HashMap;
use std::sync::Arc;

#[derive(Clone, Debug)]
pub struct Binding {
	pub value:	Value,
	pub span:	Span,	// where it was bound, detached for the standard library
}

/// A chain of frames, innermost first. The root of every chain is the standard library.
#[derive(Clone, Debug, Default)]
pub struct Scope {
	pub map:	HashMap<String, Binding>,
	pub parent:	Option<Arc<Scope>>,
}

impl Scope {
	pub fn new() -> Self { Self::default() }

	/// A fresh frame whose lookups fall through to `parent`.
	pub fn child(parent: Arc<Scope>) -> Self { Self { map: HashMap::new(), parent: Some(parent) } }

	pub fn define<S: Into<String>>(&mut self, name: S, value: Value) {
		self.map.insert(name.into(), Binding { value, span: Span::detached() });
	}

	pub fn define_at<S: Into<String>>(&mut self, name: S, value: Value, span: Span) {
		self.map.insert(name.into(), Binding { value, span });
	}

	/// The innermost binding of a name.
	pub fn get(&self, name: &str) -> Option<&Value> {
		match self.map.get(name) {
			Some(b)	=> Some(&b.value),
			None	=> self.parent.as_ref().and_then(|p| p.get(name)),
		}
	}

	/// A binding in this frame only, for assignment (`x = 1` may not reach through a closure's capture).
	pub fn get_mut(&mut self, name: &str) -> Option<&mut Value> {
		self.map.get_mut(name).map(|b| &mut b.value)
	}
}
