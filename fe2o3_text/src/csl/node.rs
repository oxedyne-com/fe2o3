//! Ported from hayagriva 0.10.1 `src/csl/elem.rs`, Copyright (c) Martin Haug and the hayagriva
//! contributors, MIT OR Apache-2.0.
//!
//! The shape is hayagriva's `ElemChildren`, which Typst realises: a host turns `Meta::Entry` into
//! a link to the bibliography entry, a `left-margin` element into the entry's prefix column, and
//! `Math` into an equation.
use crate::csl::person::Person;
use crate::csl::style::{
	Decoration,
	Display,
	Fmt,
	FontStyle,
	FontVariant,
	FontWeight,
	VAlign,
};

use std::fmt::Write;


/// Formatting as resolved for a run of text, every attribute set.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Formatting {
	pub style:	FontStyle,
	pub variant:	FontVariant,
	pub weight:	FontWeight,
	pub deco:	Decoration,
	pub valign:	VAlign,
}

impl Default for Formatting {
	fn default() -> Self {
		Self {
			style:		FontStyle::Normal,
			variant:	FontVariant::Normal,
			weight:		FontWeight::Normal,
			deco:		Decoration::None,
			valign:		VAlign::Baseline,
		}
	}
}

impl Formatting {

	/// Overlays an element's CSL formatting, leaving what it does not set.
	pub fn apply(mut self, f: &Fmt) -> Self {
		if let Some(v) = f.style	{ self.style = v; }
		if let Some(v) = f.variant	{ self.variant = v; }
		if let Some(v) = f.weight	{ self.weight = v; }
		if let Some(v) = f.deco		{ self.deco = v; }
		if let Some(v) = f.valign	{ self.valign = v; }
		self
	}
}

/// What an element stands for, so that a host can find or link it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Meta {
	Names(Vec<(Vec<Person>, String)>),	// every variable's people, rendered or not
	Date,
	Text,
	Number,
	Label,
	CitationNumber,
	Name(String, usize),	// variable and index of one name, for B2b
	Entry(usize),		// a citation's link to its bibliography entry
	CitationLabel,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Elem {
	pub children:	Vec<Node>,
	pub display:	Option<Display>,
	pub meta:	Option<Meta>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Node {
	Text(String, Formatting),
	Elem(Elem),
	Math(String),				// a math chunk's source, for the host to evaluate
	Link { text: String, fmt: Formatting, url: String },
	Cite { idx: usize, fmt: Formatting },	// a locator the host renders, by citation index
}

impl Node {

	pub fn str_len(&self) -> usize {
		match self {
			Self::Text(t, _)		=> t.len(),
			Self::Elem(e)			=> e.children.iter().map(Node::str_len).sum(),
			Self::Math(m)			=> m.len(),
			Self::Link { text, .. }		=> text.len(),
			Self::Cite { .. }		=> 0,
		}
	}

	pub fn has_content(&self) -> bool {
		match self {
			Self::Text(t, _) | Self::Math(t)	=> t.chars().any(|c| !c.is_whitespace()),
			Self::Elem(e)				=> has_content(&e.children),
			Self::Link { .. } | Self::Cite { .. }	=> true,
		}
	}

	pub fn is_empty(&self) -> bool {
		match self {
			Self::Text(t, _) | Self::Math(t)	=> t.is_empty(),
			Self::Elem(e)				=> is_empty(&e.children),
			Self::Link { text, .. }			=> text.is_empty(),
			Self::Cite { .. }			=> false,
		}
	}
}

pub fn has_content(nodes: &[Node]) -> bool {
	nodes.iter().any(Node::has_content)
}

pub fn is_empty(nodes: &[Node]) -> bool {
	nodes.iter().all(Node::is_empty)
}

/// The last text run, looking into the last element.
pub fn last_text_mut(nodes: &mut [Node]) -> Option<&mut String> {
	match nodes.last_mut() {
		Some(Node::Text(t, _))	=> Some(t),
		Some(Node::Elem(e))	=> last_text_mut(&mut e.children),
		_			=> None,
	}
}

pub fn last_text(nodes: &[Node]) -> Option<&str> {
	match nodes.last() {
		Some(Node::Text(t, _))	=> Some(t),
		Some(Node::Elem(e))	=> last_text(&e.children),
		_			=> None,
	}
}

/// Takes out the first element, depth first, that carries a meta: second-field-align's first
/// field, as hayagriva's `remove_any_meta` finds it.
pub fn remove_any_meta(nodes: &mut Vec<Node>) -> Option<Node> {
	for i in 0..nodes.len() {
		if let Node::Elem(e) = &mut nodes[i] {
			if e.meta.is_some() {
				return Some(nodes.remove(i));
			}
			if let Some(n) = remove_any_meta(&mut e.children) {
				return Some(n);
			}
		}
	}
	None
}

/// Merges neighbouring text of one formatting and inlines elements that carry neither display nor
/// meta.
pub fn simplify(nodes: Vec<Node>) -> Vec<Node> {
	let mut acc: Vec<Node> = Vec::new();
	for n in nodes {
		match n {
			Node::Text(t, f) => match acc.last_mut() {
				Some(Node::Text(last, lf)) if *lf == f => last.push_str(&t),
				_ => acc.push(Node::Text(t, f)),
			},
			// hayagriva extends without merging here, so inlined text stays a separate run.
			Node::Elem(e) if e.display.is_none() && e.meta.is_none() => acc.extend(e.children),
			Node::Elem(e) => acc.push(Node::Elem(Elem { children: simplify(e.children), ..e })),
			other => acc.push(other),
		}
	}
	acc
}

/// The text without formatting. A block element sits on lines of its own, as hayagriva's plain
/// writer puts it.
pub fn plain(nodes: &[Node]) -> String {
	let mut s = String::new();
	write_plain(nodes, &mut s);
	s
}

fn write_plain(nodes: &[Node], s: &mut String) {
	for n in nodes {
		match n {
			Node::Text(t, _)	=> s.push_str(t),
			Node::Math(m)		=> { let _ = write!(s, "${}$", m); }
			Node::Link { text, .. }	=> s.push_str(text),
			Node::Cite { .. }	=> {}
			Node::Elem(e) => {
				let block = e.display == Some(Display::Block);
				if block { s.push('\n'); }
				write_plain(&e.children, s);
				if block { s.push('\n'); }
			}
		}
	}
}
