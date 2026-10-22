// SPDX-License-Identifier: Apache-2.0
// Original work copyright (c) the Typst project authors (typst-syntax `kind.rs`, version 0.15.1).
// Modified for Austenite: ported to Hematite's types, IR and error handling.
// U1 owns this file. The variant list mirrors typst-syntax 0.14's `SyntaxKind`; add a kind rather than
// overloading one, since U2 dispatches on it.

macro_rules! syntax_kinds {
	($($(#[$m:meta])* $v:ident => $name:literal,)*) => {
		#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
		pub enum SyntaxKind {
			$($(#[$m])* $v,)*
		}

		impl SyntaxKind {
			/// A human-readable name, as a diagnostic quotes it ("expected closing bracket").
			pub fn name(self) -> &'static str {
				match self {
					$(SyntaxKind::$v => $name,)*
				}
			}
		}
	};
}

syntax_kinds! {
	// Trivia and errors
	End				=> "end of tokens",
	Error			=> "syntax error",
	Shebang			=> "shebang",
	LineComment		=> "line comment",
	BlockComment	=> "block comment",
	// Markup
	Markup			=> "markup",
	Text			=> "text",
	Space			=> "space",
	Linebreak		=> "line break",
	Parbreak		=> "paragraph break",
	Escape			=> "escape sequence",
	Shorthand		=> "shorthand",
	SmartQuote		=> "smart quote",
	Strong			=> "strong content",
	Emph			=> "emphasized content",	// Typst's spelling: the name reaches its messages
	Raw				=> "raw block",
	RawLang			=> "raw language tag",
	RawDelim		=> "raw delimiter",
	RawTrimmed		=> "raw trimmed",
	Link			=> "link",
	Label			=> "label",
	Ref				=> "reference",
	RefMarker		=> "reference marker",
	Heading			=> "heading",
	HeadingMarker	=> "heading marker",
	ListItem		=> "list item",
	ListMarker		=> "list marker",
	EnumItem		=> "enum item",
	EnumMarker		=> "enum marker",
	TermItem		=> "term list item",
	TermMarker		=> "term marker",
	Equation		=> "equation",
	// Maths
	Math			=> "math",
	MathText		=> "math text",
	MathIdent		=> "math identifier",
	MathFieldAccess	=> "math field access",
	MathShorthand	=> "math shorthand",
	MathAlignPoint	=> "math alignment point",
	MathCall		=> "math function call",
	MathArgs		=> "math call arguments",
	MathDelimited	=> "delimited math",
	MathAttach		=> "math attachments",
	MathPrimes		=> "math primes",
	MathFrac		=> "math fraction",
	MathRoot		=> "math root",
	// Punctuation
	Hash			=> "hash",
	LeftBrace		=> "opening brace",
	RightBrace		=> "closing brace",
	LeftBracket		=> "opening bracket",
	RightBracket	=> "closing bracket",
	LeftParen		=> "opening paren",
	RightParen		=> "closing paren",
	Comma			=> "comma",
	Semicolon		=> "semicolon",
	Colon			=> "colon",
	Star			=> "star",
	Underscore		=> "underscore",
	Dollar			=> "dollar sign",
	Plus			=> "plus",
	Minus			=> "minus",
	Slash			=> "slash",
	Hat				=> "hat",
	Prime			=> "prime",			// unused since 0.15: primes lex as one `MathPrimes` leaf
	Dot				=> "dot",
	Eq				=> "equals sign",
	EqEq			=> "equality operator",
	ExclEq			=> "inequality operator",
	Lt				=> "less-than operator",
	LtEq			=> "less-than or equal operator",
	Gt				=> "greater-than operator",
	GtEq			=> "greater-than or equal operator",
	PlusEq			=> "add-assign operator",
	HyphEq			=> "subtract-assign operator",
	StarEq			=> "multiply-assign operator",
	SlashEq			=> "divide-assign operator",
	Dots			=> "dots",
	Arrow			=> "arrow",
	Root			=> "root",
	Bang			=> "exclamation mark",
	// Keywords
	Not				=> "operator `not`",
	And				=> "operator `and`",
	Or				=> "operator `or`",
	None			=> "`none`",
	Auto			=> "`auto`",
	Let				=> "keyword `let`",
	Set				=> "keyword `set`",
	Show			=> "keyword `show`",
	Context			=> "keyword `context`",
	If				=> "keyword `if`",
	Else			=> "keyword `else`",
	For				=> "keyword `for`",
	In				=> "keyword `in`",
	While			=> "keyword `while`",
	Break			=> "keyword `break`",
	Continue		=> "keyword `continue`",
	Return			=> "keyword `return`",
	Import			=> "keyword `import`",
	Include			=> "keyword `include`",
	As				=> "keyword `as`",
	// Code
	Code			=> "code",
	Ident			=> "identifier",
	Bool			=> "boolean",
	Int				=> "integer",
	Float			=> "float",
	Numeric			=> "numeric value",
	Str				=> "string",
	CodeBlock		=> "code block",
	ContentBlock	=> "content block",
	Parenthesized	=> "group",
	Array			=> "array",
	Dict			=> "dictionary",
	Named			=> "named pair",
	Keyed			=> "keyed pair",
	Unary			=> "unary expression",
	Binary			=> "binary expression",
	FieldAccess		=> "field access",
	FuncCall		=> "function call",
	Args			=> "call arguments",
	Spread			=> "spread",
	Closure			=> "closure",
	Params			=> "closure parameters",
	LetBinding		=> "`let` expression",
	SetRule			=> "`set` expression",
	ShowRule		=> "`show` expression",
	Contextual		=> "`context` expression",
	Conditional		=> "`if` expression",
	WhileLoop		=> "while-loop expression",
	ForLoop			=> "for-loop expression",
	ModuleImport	=> "`import` expression",
	ImportItems		=> "import items",
	ImportItemPath	=> "imported item path",
	RenamedImportItem	=> "renamed import item",
	ModuleInclude	=> "`include` expression",
	LoopBreak		=> "`break` expression",
	LoopContinue	=> "`continue` expression",
	FuncReturn		=> "`return` expression",
	Destructuring	=> "destructuring pattern",
	DestructAssignment	=> "destructuring assignment expression",
}

impl SyntaxKind {
	/// Is this a comment or whitespace a typed accessor skips?
	pub fn is_trivia(self) -> bool {
		matches!(self, SyntaxKind::Shebang | SyntaxKind::LineComment | SyntaxKind::BlockComment
			| SyntaxKind::Space | SyntaxKind::Parbreak)
	}

	pub fn is_error(self) -> bool { self == SyntaxKind::Error }

	pub fn is_keyword(self) -> bool {
		matches!(self,
			SyntaxKind::Not | SyntaxKind::And | SyntaxKind::Or | SyntaxKind::None | SyntaxKind::Auto
			| SyntaxKind::Let | SyntaxKind::Set | SyntaxKind::Show | SyntaxKind::Context | SyntaxKind::If
			| SyntaxKind::Else | SyntaxKind::For | SyntaxKind::In | SyntaxKind::While | SyntaxKind::Break
			| SyntaxKind::Continue | SyntaxKind::Return | SyntaxKind::Import | SyntaxKind::Include
			| SyntaxKind::As)
	}
}

impl SyntaxKind {
	/// Is this an opening or closing bracket, brace or parenthesis?
	pub fn is_grouping(self) -> bool {
		matches!(self,
			SyntaxKind::LeftBracket | SyntaxKind::LeftBrace | SyntaxKind::LeftParen
			| SyntaxKind::RightBracket | SyntaxKind::RightBrace | SyntaxKind::RightParen)
	}

	/// Does this token end the expression before it?
	pub fn is_terminator(self) -> bool {
		matches!(self,
			SyntaxKind::End | SyntaxKind::Semicolon | SyntaxKind::RightBrace
			| SyntaxKind::RightParen | SyntaxKind::RightBracket)
	}

	/// Is this a code or content block?
	pub fn is_block(self) -> bool {
		matches!(self, SyntaxKind::CodeBlock | SyntaxKind::ContentBlock)
	}

	/// Must this node be ended by a semicolon or a line break when embedded in markup?
	pub fn is_stmt(self) -> bool {
		matches!(self,
			SyntaxKind::LetBinding | SyntaxKind::SetRule | SyntaxKind::ShowRule
			| SyntaxKind::ModuleImport | SyntaxKind::ModuleInclude)
	}
}

/// A set of syntax kinds, one bit each, built at compile time with [`syntax_set!`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SyntaxSet(u128, u128);

impl SyntaxSet {
	pub const fn new() -> Self { Self(0, 0) }

	pub const fn add(self, kind: SyntaxKind) -> Self {
		let i = kind as usize;
		if i < 128 { Self(self.0 | (1u128 << i), self.1) } else { Self(self.0, self.1 | (1u128 << (i - 128))) }
	}

	pub const fn remove(self, kind: SyntaxKind) -> Self {
		let i = kind as usize;
		if i < 128 { Self(self.0 & !(1u128 << i), self.1) } else { Self(self.0, self.1 & !(1u128 << (i - 128))) }
	}

	pub const fn union(self, other: Self) -> Self { Self(self.0 | other.0, self.1 | other.1) }

	pub const fn contains(&self, kind: SyntaxKind) -> bool {
		let i = kind as usize;
		if i < 128 { self.0 & (1u128 << i) != 0 } else { self.1 & (1u128 << (i - 128)) != 0 }
	}
}

/// A compile-time [`SyntaxSet`] of the named kinds.
macro_rules! syntax_set {
	($($kind:ident),* $(,)?) => {{
		const SET: $crate::syntax::kind::SyntaxSet = $crate::syntax::kind::SyntaxSet::new()
			$(.add($crate::syntax::SyntaxKind::$kind))*;
		SET
	}};
}

pub(crate) use syntax_set;

// Named sets, as typst-syntax declares them.
pub const STMT: SyntaxSet = syntax_set!(Let, Set, Show, Import, Include, Return);

pub const MATH_EXPR: SyntaxSet = syntax_set!(
	Hash, MathIdent, MathFieldAccess, LeftBrace, RightBrace, LeftParen, RightParen, MathText,
	MathShorthand, Linebreak, MathAlignPoint, MathPrimes, Escape, Str, Root, Dot, Comma, Semicolon, Bang,
);

pub const ATOMIC_CODE_EXPR: SyntaxSet = syntax_set!(
	Ident, LeftBrace, LeftBracket, LeftParen, Dollar, Let, Set, Show, Context, If, While, For, Import,
	Include, Break, Continue, Return, None, Auto, Int, Float, Bool, Numeric, Str, Label, Raw,
);

pub const UNARY_OP: SyntaxSet = syntax_set!(Plus, Minus, Not);

// An underscore starts only `_ => ..` or `_ = ..`.
pub const CODE_EXPR: SyntaxSet = ATOMIC_CODE_EXPR.union(UNARY_OP).add(SyntaxKind::Underscore);

pub const BINARY_OP: SyntaxSet = syntax_set!(
	Plus, Minus, Star, Slash, And, Or, EqEq, ExclEq, Lt, LtEq, Gt, GtEq, Eq, In, PlusEq, HyphEq, StarEq,
	SlashEq,
);

pub const ARRAY_OR_DICT_ITEM: SyntaxSet = CODE_EXPR.add(SyntaxKind::Dots);
pub const ARG: SyntaxSet = CODE_EXPR.add(SyntaxKind::Dots);
pub const PATTERN_LEAF: SyntaxSet = ATOMIC_CODE_EXPR;
pub const PATTERN: SyntaxSet = PATTERN_LEAF.add(SyntaxKind::LeftParen).add(SyntaxKind::Underscore);
pub const PARAM: SyntaxSet = PATTERN.add(SyntaxKind::Dots);
pub const DESTRUCTURING_ITEM: SyntaxSet = PATTERN.add(SyntaxKind::Dots);
