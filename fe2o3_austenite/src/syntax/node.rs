// U1 owns this file. The public methods below are the contract U2 codes against; keep their names and
// meanings, add freely.

use crate::syntax::{
	FileId,
	SyntaxKind,
	Span,
};

use std::sync::Arc;

/// A node of the concrete syntax tree: a leaf holding source text, an inner node holding children, or an
/// error holding the text it could not read and why. Cloning is a reference-count bump.
///
/// A node may also carry warnings (an empty `*strong*`, an ambiguous raw language tag), which never
/// change its kind or text.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SyntaxNode {
	kind:	SyntaxKind,
	span:	Span,
	repr:	Repr,
	warns:	Option<Arc<Vec<SyntaxWarning>>>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum Repr {
	Empty,						// a leaf with no text, constructible in a `const`
	Leaf(Arc<str>),
	Inner(Arc<Inner>),
	Error(Arc<SyntaxError>),
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct Inner {
	children:	Vec<SyntaxNode>,
	len:		usize,		// byte length of the covered source
	erroneous:	bool,		// some descendant is an error
	warned:		bool,		// some descendant carries a warning
}

impl Inner {
	fn new(children: Vec<SyntaxNode>) -> Self {
		let mut len = 0;
		let mut erroneous = false;
		let mut warned = false;
		for c in &children {
			len += c.len();
			erroneous |= c.erroneous();
			warned |= c.has_warnings();
		}
		Self { children, len, erroneous, warned }
	}
}

/// What an `Error` node records: the source text it swallowed, the message and any hints.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SyntaxError {
	pub text:		String,
	pub message:	String,
	pub hints:		Vec<String>,
}

/// A warning attached to a node. `range` narrows it to a byte range relative to the node's start.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SyntaxWarning {
	pub message:	String,
	pub hints:		Vec<String>,
	pub range:		Option<(u32, u32)>,
	pub span_hints:	Vec<(u32, u32, String)>,	// hints pinned to a relative byte range
}

impl SyntaxNode {
	pub fn leaf(kind: SyntaxKind, text: &str, span: Span) -> Self {
		let repr = if text.is_empty() { Repr::Empty } else { Repr::Leaf(Arc::from(text)) };
		Self { kind, span, repr, warns: None }
	}

	/// An inner node; its span covers its children's.
	pub fn inner(kind: SyntaxKind, children: Vec<SyntaxNode>) -> Self {
		let mut span = Span::detached();
		for c in &children {
			span = span.join(c.span);
		}
		Self { kind, span, repr: Repr::Inner(Arc::new(Inner::new(children))), warns: None }
	}

	pub fn error(text: &str, message: &str, span: Span) -> Self {
		Self {
			kind:	SyntaxKind::Error,
			span,
			repr:	Repr::Error(Arc::new(SyntaxError {
				text:		text.to_string(),
				message:	message.to_string(),
				hints:		Vec::new(),
			})),
			warns:	None,
		}
	}

	/// An empty, detached leaf of the given kind, which a typed view falls back to when the tree lacks
	/// the child it expects.
	pub const fn placeholder(kind: SyntaxKind) -> Self {
		Self { kind, span: Span::detached(), repr: Repr::Empty, warns: None }
	}

	pub fn kind(&self) -> SyntaxKind { self.kind }
	pub fn span(&self) -> Span { self.span }
	pub fn file(&self) -> FileId { self.span.file }

	/// The text of a leaf or an error node, empty for an inner node.
	pub fn text(&self) -> &str {
		match &self.repr {
			Repr::Leaf(t)	=> t,
			Repr::Error(e)	=> &e.text,
			Repr::Inner(_)	=> "",
			Repr::Empty		=> "",
		}
	}

	/// The byte length of the source the node covers.
	pub fn len(&self) -> usize {
		match &self.repr {
			Repr::Leaf(t)	=> t.len(),
			Repr::Error(e)	=> e.text.len(),
			Repr::Inner(i)	=> i.len,
			Repr::Empty		=> 0,
		}
	}

	pub fn is_empty(&self) -> bool { self.len() == 0 }

	/// The children of an inner node, empty for a leaf or an error.
	pub fn children(&self) -> &[SyntaxNode] {
		match &self.repr {
			Repr::Inner(i)	=> &i.children,
			_				=> &[],
		}
	}

	pub fn is_leaf(&self) -> bool { matches!(self.repr, Repr::Leaf(_) | Repr::Empty) }

	pub fn is_inner(&self) -> bool { matches!(self.repr, Repr::Inner(_)) }

	/// The first child of the given kind.
	pub fn child(&self, kind: SyntaxKind) -> Option<&SyntaxNode> {
		self.children().iter().find(|c| c.kind == kind)
	}

	/// The children that are not trivia, in order.
	pub fn significant(&self) -> impl Iterator<Item = &SyntaxNode> {
		self.children().iter().filter(|c| !c.kind.is_trivia())
	}

	pub fn error_info(&self) -> Option<&SyntaxError> {
		match &self.repr {
			Repr::Error(e)	=> Some(e),
			_				=> None,
		}
	}

	/// Does this node or any descendant hold a syntax error?
	pub fn erroneous(&self) -> bool {
		match &self.repr {
			Repr::Error(_)	=> true,
			Repr::Inner(i)	=> i.erroneous,
			_				=> false,
		}
	}

	/// Does this node or any descendant carry a warning?
	pub fn has_warnings(&self) -> bool {
		self.warns.is_some() || match &self.repr {
			Repr::Inner(i)	=> i.warned,
			_				=> false,
		}
	}

	/// Every error node beneath and including this one, in source order.
	pub fn errors(&self) -> Vec<(Span, SyntaxError)> {
		let mut out = Vec::new();
		self.collect_errors(&mut out);
		out
	}

	fn collect_errors(&self, out: &mut Vec<(Span, SyntaxError)>) {
		match &self.repr {
			Repr::Error(e)	=> out.push((self.span, (**e).clone())),
			Repr::Inner(i)	=> if i.erroneous {
				for c in &i.children {
					c.collect_errors(out);
				}
			},
			_				=> (),
		}
	}

	/// Every warning beneath and including this one, in source order, each with the span it targets.
	pub fn warnings(&self) -> Vec<(Span, SyntaxWarning)> {
		let mut out = Vec::new();
		self.collect_warnings(&mut out);
		out
	}

	fn collect_warnings(&self, out: &mut Vec<(Span, SyntaxWarning)>) {
		if let Some(ws) = &self.warns {
			for w in ws.iter() {
				let span = match w.range {
					Some((a, b)) if !self.span.is_detached()	=>
						Span::new(self.span.file, self.span.start + a, self.span.start + b),
					_											=> self.span,
				};
				out.push((span, w.clone()));
			}
		}
		if let Repr::Inner(i) = &self.repr {
			if i.warned {
				for c in &i.children {
					c.collect_warnings(out);
				}
			}
		}
	}

	/// The source text the node covers, rebuilt from its leaves -- equal to the source slice, since the
	/// tree is lossless.
	pub fn full_text(&self) -> String {
		let mut s = String::with_capacity(self.len());
		self.push_text(&mut s);
		s
	}

	fn push_text(&self, s: &mut String) {
		match &self.repr {
			Repr::Leaf(t)	=> s.push_str(t),
			Repr::Error(e)	=> s.push_str(&e.text),
			Repr::Inner(i)	=> for n in i.children.iter() { n.push_text(s); },
			Repr::Empty		=> (),
		}
	}

	/// Does the node equal another, ignoring spans?
	pub fn spanless_eq(&self, other: &Self) -> bool {
		if self.kind != other.kind || self.warns != other.warns {
			return false;
		}
		match (&self.repr, &other.repr) {
			(Repr::Inner(a), Repr::Inner(b))	=> a.children.len() == b.children.len()
				&& a.children.iter().zip(b.children.iter()).all(|(x, y)| x.spanless_eq(y)),
			(Repr::Error(a), Repr::Error(b))	=> a == b,
			_									=> self.text() == other.text() && self.is_leaf() == other.is_leaf(),
		}
	}
}

// Mutation, for the parser only.
impl SyntaxNode {
	/// Re-kinds a leaf or inner node. An error node keeps its kind.
	pub(crate) fn convert_to_kind(&mut self, kind: SyntaxKind) {
		if !self.kind.is_error() && !kind.is_error() {
			self.kind = kind;
		}
	}

	/// Turns the node into an error covering the same text, unless it already is one.
	pub(crate) fn convert_to_error(&mut self, message: &str) {
		if !self.kind.is_error() {
			let text = self.full_text();
			*self = SyntaxNode::error(&text, message, self.span);
		}
	}

	/// Turns the node into an "expected `thing`, found ..." error.
	pub(crate) fn expected(&mut self, thing: &str) {
		let kind = self.kind;
		self.convert_to_error(&format!("expected {}, found {}", thing, kind.name()));
		if kind.is_keyword() && matches!(thing, "identifier" | "pattern") {
			let text = self.text().to_string();
			self.hint(&format!(
				"keyword `{}` is not allowed as an identifier; try `{}_` instead", text, text));
		}
	}

	/// Turns the node into an "unexpected ..." error.
	pub(crate) fn unexpected(&mut self) {
		let name = self.kind.name();
		self.convert_to_error(&format!("unexpected {}", name));
	}

	/// Adds a hint to the node's error, or to its latest warning when it is not an error.
	pub(crate) fn hint(&mut self, hint: &str) {
		if let Repr::Error(e) = &mut self.repr {
			Arc::make_mut(e).hints.push(hint.to_string());
			return;
		}
		if let Some(ws) = &mut self.warns {
			if let Some(w) = Arc::make_mut(ws).last_mut() {
				w.hints.push(hint.to_string());
			}
		}
	}

	pub(crate) fn warn(&mut self, message: &str) {
		self.push_warning(SyntaxWarning {
			message:	message.to_string(),
			hints:		Vec::new(),
			range:		None,
			span_hints:	Vec::new(),
		});
	}

	/// A warning narrowed to `start..end`, byte offsets relative to the node's start.
	pub(crate) fn warn_at(&mut self, start: usize, end: usize, message: &str) {
		let range = Some((start as u32, end.min(self.len()) as u32));
		self.push_warning(SyntaxWarning { message: message.to_string(), hints: Vec::new(), range, span_hints: Vec::new() });
	}

	/// A hint on the latest warning, pinned to `start..end` relative to the node's start.
	pub(crate) fn hint_at(&mut self, start: usize, end: usize, hint: &str) {
		let range = (start as u32, end.min(self.len()) as u32);
		if let Some(ws) = &mut self.warns {
			if let Some(w) = Arc::make_mut(ws).last_mut() {
				w.span_hints.push((range.0, range.1, hint.to_string()));
			}
		}
	}

	fn push_warning(&mut self, w: SyntaxWarning) {
		match &mut self.warns {
			Some(ws)	=> Arc::make_mut(ws).push(w),
			None		=> self.warns = Some(Arc::new(vec![w])),
		}
	}

	pub(crate) fn children_mut(&mut self) -> &mut [SyntaxNode] {
		match &mut self.repr {
			Repr::Inner(i)	=> &mut Arc::make_mut(i).children,
			_				=> &mut [],
		}
	}

	/// Assigns every node in the tree its byte span in `file`, starting at `start`, and returns the end.
	pub(crate) fn numberise(&mut self, file: FileId, start: u32) -> u32 {
		let end = match &mut self.repr {
			Repr::Inner(i)	=> {
				let mut at = start;
				for c in Arc::make_mut(i).children.iter_mut() {
					at = c.numberise(file, at);
				}
				at
			},
			_				=> start + self.len() as u32,
		};
		self.span = Span::new(file, start, end);
		end
	}
}
