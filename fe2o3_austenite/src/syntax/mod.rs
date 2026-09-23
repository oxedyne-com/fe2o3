//! Contract (U0, 2026-09-23): the concrete syntax tree is lossless -- concatenating every leaf's text
//! reproduces the source byte for byte -- and its kinds and child order follow typst-syntax 0.14/0.15,
//! so typst-syntax's own `ast.rs` is the reference for which child an accessor reads. A parse error is an
//! `Error` node inside the tree, never an `Err` and never a panic. U1 owns `kind.rs`, `node.rs`,
//! `lexer.rs`, `parser.rs` and `ast.rs`; this file, `Span`, `FileId` and `Source` are fixed.

pub mod ast;
pub mod kind;
pub mod lexer;
pub mod node;
pub mod parser;

pub use kind::SyntaxKind;
pub use node::SyntaxNode;


use std::path::PathBuf;
use std::sync::Arc;

/// Which source file a span points into: an index into the evaluator's `World::sources`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FileId(pub u16);

impl FileId {
	pub const DETACHED: FileId = FileId(u16::MAX);	// no file: synthesised content
}

impl Default for FileId {
	fn default() -> Self { FileId::DETACHED }
}

/// A byte range within one source file. Unlike [`crate::ir::Span`] it names its file, so a diagnostic
/// raised deep inside an imported module still reads back as `file:line:col`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Span {
	pub file:	FileId,
	pub start:	u32,
	pub end:	u32,
}

impl Span {
	pub fn new(file: FileId, start: u32, end: u32) -> Self { Self { file, start, end } }

	pub const fn detached() -> Self { Self { file: FileId::DETACHED, start: 0, end: 0 } }

	/// Does the span point at no source (synthesised by native code)?
	pub fn is_detached(&self) -> bool { self.file == FileId::DETACHED }

	/// The smallest span covering both, or `self` when they lie in different files.
	pub fn join(self, other: Span) -> Span {
		if self.is_detached() {
			return other;
		}
		if other.is_detached() || other.file != self.file {
			return self;
		}
		Span::new(self.file, self.start.min(other.start), self.end.max(other.end))
	}

	/// The byte range alone, for the back end's file-less [`crate::ir::Span`].
	pub fn range(&self) -> crate::ir::Span { crate::ir::Span::new(self.start, self.end) }
}

/// One source file and its parsed tree. `path` is the vfs path it was read from, so a relative import
/// resolves against `path.parent()`.
#[derive(Clone, Debug)]
pub struct Source {
	pub id:		FileId,
	pub path:	PathBuf,
	pub text:	Arc<String>,
	pub root:	SyntaxNode,
}

impl Source {
	/// Parses `text` as markup, the mode a `.typ` file opens in.
	pub fn new(id: FileId, path: PathBuf, text: String) -> Self {
		let root = parser::parse(&text, id);
		Self { id, path, text: Arc::new(text), root }
	}

	/// The 1-based line and character column of a byte offset, as Typst counts them: a column is a
	/// count of `char`s, so `é` is one column, not two.
	pub fn line_col(&self, offset: u32) -> (usize, usize) {
		let mut end = (offset as usize).min(self.text.len());
		while !self.text.is_char_boundary(end) {
			end -= 1;
		}
		let before = &self.text[..end];
		let start = before.rfind('\n').map(|i| i + 1).unwrap_or(0);
		(before.matches('\n').count() + 1, before[start..].chars().count() + 1)
	}
}
