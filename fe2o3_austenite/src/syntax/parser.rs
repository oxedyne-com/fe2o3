// U1 owns this file. The three entry points are the contract; each returns a whole tree, errors inside it.

use crate::syntax::{
	FileId,
	SyntaxKind,
	SyntaxNode,
	Span,
};

/// Parses a file in markup mode into a `Markup` root.
pub fn parse(text: &str, file: FileId) -> SyntaxNode {
	stub(text, file, SyntaxKind::Markup)
}

/// Parses code, as `eval(mode: "code")` and a `typst eval` probe need, into a `Code` root.
pub fn parse_code(text: &str, file: FileId) -> SyntaxNode {
	stub(text, file, SyntaxKind::Code)
}

/// Parses maths, as `eval(mode: "math")` needs, into a `Math` root.
pub fn parse_math(text: &str, file: FileId) -> SyntaxNode {
	stub(text, file, SyntaxKind::Math)
}

// Until U1 lands: the whole text as one error node, so the tree stays lossless and the failure is loud.
fn stub(text: &str, file: FileId, root: SyntaxKind) -> SyntaxNode {
	let span = Span::new(file, 0, text.len() as u32);
	SyntaxNode::inner(root, vec![SyntaxNode::error(text, "The Typst parser is not implemented yet.", span)])
}
