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
	End				=> "end of file",
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
	Emph			=> "emphasised content",
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
	MathShorthand	=> "math shorthand",
	MathAlignPoint	=> "math alignment point",
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
	Prime			=> "prime",
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
