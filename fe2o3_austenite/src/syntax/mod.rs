// SPDX-License-Identifier: Apache-2.0
// Original work copyright (c) the Typst project authors (typst-syntax `span.rs`, `file.rs` and `source.rs`, version 0.15.1).
// Modified for Austenite: ported to Hematite's types, IR and error handling.
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

	/// The 1-based line and character column of a byte offset, as Typst counts them: a column is a count of
	/// `char`s, so `é` is one column, not two.
	pub fn line_col(&self, offset: u32) -> (usize, usize) {
		self.line_col_in(offset, Cols::Chars)
	}

	/// The 1-based line and column of a byte offset, the column in `cols`. A line ends at any of Typst's
	/// line breaks: `\n`, a vertical tab, a form feed, a lone `\r` and the Unicode next-line, line and
	/// paragraph separators, with `\r\n` one break.
	pub fn line_col_in(&self, offset: u32, cols: Cols) -> (usize, usize) {
		let mut end = (offset as usize).min(self.text.len());
		while !self.text.is_char_boundary(end) {
			end -= 1;
		}
		let mut line = 1usize;
		let mut col = 1usize;
		let mut chars = self.text[..end].chars().peekable();
		while let Some(c) = chars.next() {
			match c {
				'\r'	=> {
					if chars.peek() == Some(&'\n') {
						chars.next();
					}
					line += 1;
					col = 1;
				},
				'\n' | '\u{b}' | '\u{c}' | '\u{85}' | '\u{2028}' | '\u{2029}'	=> {
					line += 1;
					col = 1;
				},
				c	=> col += match cols {
					Cols::Chars	=> 1,
					Cols::Utf16	=> c.len_utf16(),
				},
			}
		}
		(line, col)
	}
}

/// The unit a column is counted in: characters, as Typst's own diagnostics count them, or UTF-16 code units,
/// as JavaScript indexes a string.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cols {
	Chars,
	Utf16,
}

#[cfg(test)]
mod tests {
	use super::*;

	fn at(text: &str, needle: &str, cols: Cols) -> (usize, usize) {
		let src = Source::new(FileId(0), PathBuf::from("/t.typ"), text.to_string());
		let off = text.find(needle).unwrap_or(0) as u32;
		src.line_col_in(off, cols)
	}

	/// Typst ends a line at `\n`, a vertical tab, a form feed, a lone `\r`, U+0085, U+2028 and U+2029,
	/// and takes `\r\n` as one break; a column restarts after each.
	#[test]
	fn a_line_ends_at_every_break_typst_ends_one_at() {
		for brk in ["\n", "\r", "\r\n", "\u{b}", "\u{c}", "\u{85}", "\u{2028}", "\u{2029}"] {
			let text = format!("ab{}#x", brk);
			assert_eq!(at(&text, "#x", Cols::Chars), (2, 1), "after {:?}", brk);
		}
		// Two breaks are two lines, and `\r\n` followed by `\n` is two.
		assert_eq!(at("a\r\n\n#x", "#x", Cols::Chars), (3, 1));
		assert_eq!(at("a\r\r#x", "#x", Cols::Chars), (3, 1));
	}

	/// A UTF-16 column counts a character outside the BMP twice; a character column counts it once.
	#[test]
	fn a_utf16_column_counts_a_character_outside_the_bmp_twice() {
		let text = "\u{3000}\u{1F600}x";
		assert_eq!(at(text, "x", Cols::Chars), (1, 3));
		assert_eq!(at(text, "x", Cols::Utf16), (1, 4));
		assert_eq!(at("abc", "c", Cols::Utf16), (1, 3));
		// A byte offset inside a character reads as the character's start.
		let src = Source::new(FileId(0), PathBuf::from("/t.typ"), "\u{1F600}x".to_string());
		assert_eq!(src.line_col_in(2, Cols::Utf16), (1, 1));
		assert_eq!(src.line_col(4), (1, 2));
	}
}
