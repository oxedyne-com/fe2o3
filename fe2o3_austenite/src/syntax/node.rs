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
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SyntaxNode {
	kind:	SyntaxKind,
	span:	Span,
	repr:	Repr,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum Repr {
	Leaf(Arc<str>),
	Inner(Arc<Vec<SyntaxNode>>),
	Error(Arc<SyntaxError>),
}

/// What an `Error` node records: the source text it swallowed, the message and any hints.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SyntaxError {
	pub text:		String,
	pub message:	String,
	pub hints:		Vec<String>,
}

impl SyntaxNode {
	pub fn leaf(kind: SyntaxKind, text: &str, span: Span) -> Self {
		Self { kind, span, repr: Repr::Leaf(Arc::from(text)) }
	}

	/// An inner node; its span covers its children's.
	pub fn inner(kind: SyntaxKind, children: Vec<SyntaxNode>) -> Self {
		let mut span = Span::detached();
		for c in &children {
			span = span.join(c.span);
		}
		Self { kind, span, repr: Repr::Inner(Arc::new(children)) }
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
		}
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
		}
	}

	/// The children of an inner node, empty for a leaf or an error.
	pub fn children(&self) -> &[SyntaxNode] {
		match &self.repr {
			Repr::Inner(c)	=> c,
			_				=> &[],
		}
	}

	pub fn is_leaf(&self) -> bool { matches!(self.repr, Repr::Leaf(_)) }

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
		self.kind == SyntaxKind::Error || self.children().iter().any(|c| c.erroneous())
	}

	/// Every error node beneath and including this one, in source order.
	pub fn errors(&self) -> Vec<(Span, SyntaxError)> {
		let mut out = Vec::new();
		self.collect_errors(&mut out);
		out
	}

	fn collect_errors(&self, out: &mut Vec<(Span, SyntaxError)>) {
		if let Repr::Error(e) = &self.repr {
			out.push((self.span, (**e).clone()));
		}
		for c in self.children() {
			c.collect_errors(out);
		}
	}

	/// The source text the node covers, rebuilt from its leaves -- equal to the source slice, since the
	/// tree is lossless.
	pub fn full_text(&self) -> String {
		let mut s = String::new();
		self.push_text(&mut s);
		s
	}

	fn push_text(&self, s: &mut String) {
		match &self.repr {
			Repr::Leaf(t)	=> s.push_str(t),
			Repr::Error(e)	=> s.push_str(&e.text),
			Repr::Inner(c)	=> for n in c.iter() { n.push_text(s); },
		}
	}
}
