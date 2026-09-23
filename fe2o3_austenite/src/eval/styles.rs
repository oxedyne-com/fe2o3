// U4 owns this file. The chain is `Arc`-linked rather than borrowed (`&Styles` links, as the design first
// sketched) so a realised element can carry its chain into flow, the fixpoint cache and the memo without a
// lifetime. `StyleChain::get` has a no-folding body from U0 so other units can read styles before U4 lands;
// U4 replaces it with schema folding.

use crate::eval::args::Args;
use crate::eval::content::{
	Content,
	ElemKind,
	FieldId,
};
use crate::eval::func::{
	unimplemented,
	Func,
};
use crate::eval::select::Selector;
use crate::eval::value::{
	FromValue,
	Value,
};
use crate::eval::Engine;
use crate::syntax::Span;

use oxedyne_fe2o3_core::prelude::*;

use std::sync::Arc;

#[derive(Clone, Debug)]
pub struct Property {
	pub elem:	ElemKind,
	pub field:	FieldId,
	pub value:	Value,
	pub span:	Span,
}

/// What a show rule does to a matched element.
#[derive(Clone, Debug)]
pub enum Transformation {
	Content(Content),	// `show sel: [replacement]`
	Func(Func),			// `show sel: it => ..`, and `show: f`
	Style(Styles),		// `show sel: set ..`
}

/// The identity of a recipe for the guard set: stable for one realisation of one document.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RecipeIndex(pub usize);

#[derive(Clone, Debug)]
pub struct Recipe {
	pub selector:	Option<Selector>,	// None: `show: f`, wrapping the rest of the scope
	pub transform:	Transformation,
	pub span:		Span,
}

#[derive(Clone, Debug)]
pub enum Style {
	Property(Property),
	Recipe(Recipe),
	Revocation(RecipeIndex),
}

/// A list of styles from one `set` or `show`, or several merged.
#[derive(Clone, Debug, Default)]
pub struct Styles(pub Arc<Vec<Style>>);

impl Styles {
	pub fn new() -> Self { Self::default() }

	pub fn from_style(s: Style) -> Self { Styles(Arc::new(vec![s])) }

	pub fn is_empty(&self) -> bool { self.0.is_empty() }

	pub fn push(&mut self, s: Style) { Arc::make_mut(&mut self.0).push(s); }

	pub fn iter(&self) -> impl Iterator<Item = &Style> { self.0.iter() }

	/// Appends `other`'s styles after this one's, as consecutive `set` rules accumulate.
	pub fn extend(&mut self, other: &Styles) {
		Arc::make_mut(&mut self.0).extend(other.0.iter().cloned());
	}
}

#[derive(Debug)]
pub struct ChainLink {
	pub styles:	Styles,
	pub parent:	Option<Arc<ChainLink>>,
}

/// The styles in force at a point, innermost first. Cloning is a reference-count bump.
#[derive(Clone, Debug, Default)]
pub struct StyleChain {
	pub head:	Option<Arc<ChainLink>>,
}

impl StyleChain {
	pub fn root() -> Self { Self::default() }

	/// This chain with `styles` pushed innermost.
	pub fn chain(&self, styles: &Styles) -> StyleChain {
		if styles.is_empty() {
			return self.clone();
		}
		StyleChain { head: Some(Arc::new(ChainLink { styles: styles.clone(), parent: self.head.clone() })) }
	}

	/// Every style, innermost first.
	pub fn walk(&self) -> impl Iterator<Item = &Style> {
		let mut links = Vec::new();
		let mut cur = self.head.as_ref();
		while let Some(l) = cur {
			links.push(l);
			cur = l.parent.as_ref();
		}
		links.into_iter().flat_map(|l| l.styles.0.iter().rev())
	}

	/// The value of `elem.field` in force: innermost set value, else the schema default. U4 adds the
	/// field's fold rule.
	pub fn get(&self, elem: ElemKind, field: FieldId) -> Option<Value> {
		for s in self.walk() {
			if let Style::Property(p) = s {
				if p.elem == elem && p.field == field {
					return Some(p.value.clone());
				}
			}
		}
		elem.field_spec(field).and_then(|f| f.default.to_value())
	}

	pub fn get_as<T: FromValue>(&self, elem: ElemKind, field: FieldId) -> Outcome<Option<T>> {
		match self.get(elem, field) {
			Some(v)	=> Ok(Some(res!(T::from_value(v)))),
			None	=> Ok(None),
		}
	}

	/// The value for an element in hand: its own field if set, else the chain's.
	pub fn resolve(&self, elem: &Content, field: FieldId) -> Option<Value> {
		match elem.get(field) {
			Some(v)	=> Some(v.clone()),
			None	=> elem.kind().and_then(|k| self.get(k, field)),
		}
	}
}

/// `set elem(args)`: the settable fields given, as styles.
pub fn set_rule(_engine: &mut Engine, kind: ElemKind, _args: Args) -> Outcome<Styles> {
	Err(unimplemented("set", kind.path()))
}
