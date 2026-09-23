// U1 owns this file; nothing outside `syntax/` uses the lexer, so its shape is U1's to choose.

use crate::syntax::{
	FileId,
	SyntaxKind,
	Span,
};

/// The three lexical modes Typst switches between.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LexMode {
	Markup,
	Math,
	Code,
}

pub struct Lexer<'s> {
	pub text:	&'s str,
	pub cursor:	usize,
	pub mode:	LexMode,
	pub file:	FileId,
}

impl<'s> Lexer<'s> {
	pub fn new(text: &'s str, mode: LexMode, file: FileId) -> Self {
		Self { text, cursor: 0, mode, file }
	}

	/// The next token's kind and span; `End` at the end of the text. Stub: reports the rest of the
	/// text as one error token until U1 lands.
	pub fn next_token(&mut self) -> (SyntaxKind, Span) {
		let start = self.cursor as u32;
		self.cursor = self.text.len();
		if start as usize >= self.text.len() {
			return (SyntaxKind::End, Span::new(self.file, start, start));
		}
		(SyntaxKind::Error, Span::new(self.file, start, self.text.len() as u32))
	}
}
