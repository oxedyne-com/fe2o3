//! Ported from typst-syntax 0.15.1 (`src/ast.rs`, Apache-2.0, (c) the Typst authors): typed, zero-cost
//! views over [`SyntaxNode`], with the same names and accessors, so the evaluator reads the tree as
//! typst-eval does. A view never panics on a malformed tree: where the child it wants is missing it
//! falls back to a placeholder, and the error node already in the tree carries the diagnostic.

use crate::syntax::{
	Span,
	SyntaxKind,
	SyntaxNode,
};
use crate::syntax::lexer::{
	Scanner,
	is_ident,
	is_newline,
};

use std::ops::Deref;
use std::path::Path;

/// A typed view of a node.
pub trait AstNode<'a>: Sized {
	fn from_untyped(node: &'a SyntaxNode) -> Option<Self>;
	fn to_untyped(self) -> &'a SyntaxNode;
	/// A view over a detached placeholder node, for a child the tree lacks.
	fn placeholder() -> Self;
	fn span(self) -> Span { self.to_untyped().span() }
}

impl SyntaxNode {
	/// Can the node be viewed as `T`?
	pub fn is<'a, T: AstNode<'a>>(&'a self) -> bool { self.cast::<T>().is_some() }

	pub fn cast<'a, T: AstNode<'a>>(&'a self) -> Option<T> { T::from_untyped(self) }

	pub fn try_cast_first<'a, T: AstNode<'a>>(&'a self) -> Option<T> {
		self.children().iter().find_map(|c| c.cast())
	}

	pub fn try_cast_last<'a, T: AstNode<'a>>(&'a self) -> Option<T> {
		self.children().iter().rev().find_map(|c| c.cast())
	}

	pub fn cast_first<'a, T: AstNode<'a>>(&'a self) -> T {
		match self.try_cast_first() {
			Some(t)			=> t,
			Option::None	=> T::placeholder(),
		}
	}

	pub fn cast_last<'a, T: AstNode<'a>>(&'a self) -> T {
		match self.try_cast_last() {
			Some(t)			=> t,
			Option::None	=> T::placeholder(),
		}
	}
}

macro_rules! node {
	($(#[$attr:meta])* struct $name:ident) => {
		$(#[$attr])*
		#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
		#[repr(transparent)]
		pub struct $name<'a>(&'a SyntaxNode);

		impl<'a> AstNode<'a> for $name<'a> {
			#[inline]
			fn from_untyped(node: &'a SyntaxNode) -> Option<Self> {
				if node.kind() == SyntaxKind::$name { Some(Self(node)) } else { Option::None }
			}

			#[inline]
			fn to_untyped(self) -> &'a SyntaxNode { self.0 }

			#[inline]
			fn placeholder() -> Self {
				static PLACEHOLDER: SyntaxNode = SyntaxNode::placeholder(SyntaxKind::$name);
				Self(&PLACEHOLDER)
			}
		}
	};
}

// ┌───────────────────────────────────────────────────────────────────────────────────────────┐
// │ Comments and markup                                                                       │
// └───────────────────────────────────────────────────────────────────────────────────────────┘

node! { struct LineComment }

impl<'a> LineComment<'a> {
	pub fn text(&self) -> &'a str {
		let t = self.0.text();
		match t.strip_prefix("//") {
			Some(s)			=> s,
			Option::None	=> t,
		}
	}
}

node! { struct BlockComment }

impl<'a> BlockComment<'a> {
	pub fn text(&self) -> &'a str {
		let t = self.0.text();
		match t.strip_prefix("/*").and_then(|s| s.strip_suffix("*/")) {
			Some(s)			=> s,
			Option::None	=> t,
		}
	}
}

node! {
	/// The root of a file, and the body of a content block, heading, list item and so on.
	struct Markup
}

impl<'a> Markup<'a> {
	/// The expressions, spaces included, except the line break straight after a statement written
	/// without a semicolon.
	pub fn exprs(self) -> impl DoubleEndedIterator<Item = Expr<'a>> {
		let mut was_stmt = false;
		self.0.children().iter()
			.filter(move |n| {
				let kind = n.kind();
				let keep = !was_stmt || kind != SyntaxKind::Space;
				was_stmt = kind.is_stmt();
				keep
			})
			.filter_map(Expr::cast_with_space)
	}
}

/// Any expression, in any of the three modes.
#[derive(Debug, Copy, Clone, Hash)]
pub enum Expr<'a> {
	Text(Text<'a>),
	Space(Space<'a>),
	Linebreak(Linebreak<'a>),
	Parbreak(Parbreak<'a>),
	Escape(Escape<'a>),
	Shorthand(Shorthand<'a>),
	SmartQuote(SmartQuote<'a>),
	Strong(Strong<'a>),
	Emph(Emph<'a>),
	Raw(Raw<'a>),
	Link(Link<'a>),
	Label(Label<'a>),
	Ref(Ref<'a>),
	Heading(Heading<'a>),
	ListItem(ListItem<'a>),
	EnumItem(EnumItem<'a>),
	TermItem(TermItem<'a>),
	Equation(Equation<'a>),
	Math(Math<'a>),
	MathText(MathText<'a>),
	MathIdent(MathIdent<'a>),
	MathFieldAccess(MathFieldAccess<'a>),
	MathShorthand(MathShorthand<'a>),
	MathAlignPoint(MathAlignPoint<'a>),
	MathCall(MathCall<'a>),
	MathDelimited(MathDelimited<'a>),
	MathAttach(MathAttach<'a>),
	MathPrimes(MathPrimes<'a>),
	MathFrac(MathFrac<'a>),
	MathRoot(MathRoot<'a>),
	Ident(Ident<'a>),
	None(None<'a>),
	Auto(Auto<'a>),
	Bool(Bool<'a>),
	Int(Int<'a>),
	Float(Float<'a>),
	Numeric(Numeric<'a>),
	Str(Str<'a>),
	CodeBlock(CodeBlock<'a>),
	ContentBlock(ContentBlock<'a>),
	Parenthesized(Parenthesized<'a>),
	Array(Array<'a>),
	Dict(Dict<'a>),
	Unary(Unary<'a>),
	Binary(Binary<'a>),
	FieldAccess(FieldAccess<'a>),
	FuncCall(FuncCall<'a>),
	Closure(Closure<'a>),
	LetBinding(LetBinding<'a>),
	DestructAssignment(DestructAssignment<'a>),
	SetRule(SetRule<'a>),
	ShowRule(ShowRule<'a>),
	Contextual(Contextual<'a>),
	Conditional(Conditional<'a>),
	WhileLoop(WhileLoop<'a>),
	ForLoop(ForLoop<'a>),
	ModuleImport(ModuleImport<'a>),
	ModuleInclude(ModuleInclude<'a>),
	LoopBreak(LoopBreak<'a>),
	LoopContinue(LoopContinue<'a>),
	FuncReturn(FuncReturn<'a>),
}

impl<'a> Expr<'a> {
	fn cast_with_space(node: &'a SyntaxNode) -> Option<Self> {
		match node.kind() {
			SyntaxKind::Space	=> Some(Self::Space(Space(node))),
			_					=> Self::from_untyped(node),
		}
	}
}

impl<'a> AstNode<'a> for Expr<'a> {
	fn from_untyped(node: &'a SyntaxNode) -> Option<Self> {
		let n = node;
		Some(match node.kind() {
			SyntaxKind::Space				=> return Option::None,	// only through `cast_with_space`
			SyntaxKind::Linebreak			=> Self::Linebreak(Linebreak(n)),
			SyntaxKind::Parbreak			=> Self::Parbreak(Parbreak(n)),
			SyntaxKind::Text				=> Self::Text(Text(n)),
			SyntaxKind::Escape				=> Self::Escape(Escape(n)),
			SyntaxKind::Shorthand			=> Self::Shorthand(Shorthand(n)),
			SyntaxKind::SmartQuote			=> Self::SmartQuote(SmartQuote(n)),
			SyntaxKind::Strong				=> Self::Strong(Strong(n)),
			SyntaxKind::Emph				=> Self::Emph(Emph(n)),
			SyntaxKind::Raw					=> Self::Raw(Raw(n)),
			SyntaxKind::Link				=> Self::Link(Link(n)),
			SyntaxKind::Label				=> Self::Label(Label(n)),
			SyntaxKind::Ref					=> Self::Ref(Ref(n)),
			SyntaxKind::Heading				=> Self::Heading(Heading(n)),
			SyntaxKind::ListItem			=> Self::ListItem(ListItem(n)),
			SyntaxKind::EnumItem			=> Self::EnumItem(EnumItem(n)),
			SyntaxKind::TermItem			=> Self::TermItem(TermItem(n)),
			SyntaxKind::Equation			=> Self::Equation(Equation(n)),
			SyntaxKind::Math				=> Self::Math(Math(n)),
			SyntaxKind::MathText			=> Self::MathText(MathText(n)),
			SyntaxKind::MathIdent			=> Self::MathIdent(MathIdent(n)),
			SyntaxKind::MathFieldAccess		=> Self::MathFieldAccess(MathFieldAccess(n)),
			SyntaxKind::MathShorthand		=> Self::MathShorthand(MathShorthand(n)),
			SyntaxKind::MathAlignPoint		=> Self::MathAlignPoint(MathAlignPoint(n)),
			SyntaxKind::MathCall			=> Self::MathCall(MathCall(n)),
			SyntaxKind::MathDelimited		=> Self::MathDelimited(MathDelimited(n)),
			SyntaxKind::MathAttach			=> Self::MathAttach(MathAttach(n)),
			SyntaxKind::MathPrimes			=> Self::MathPrimes(MathPrimes(n)),
			SyntaxKind::MathFrac			=> Self::MathFrac(MathFrac(n)),
			SyntaxKind::MathRoot			=> Self::MathRoot(MathRoot(n)),
			SyntaxKind::Ident				=> Self::Ident(Ident(n)),
			SyntaxKind::None				=> Self::None(None(n)),
			SyntaxKind::Auto				=> Self::Auto(Auto(n)),
			SyntaxKind::Bool				=> Self::Bool(Bool(n)),
			SyntaxKind::Int					=> Self::Int(Int(n)),
			SyntaxKind::Float				=> Self::Float(Float(n)),
			SyntaxKind::Numeric				=> Self::Numeric(Numeric(n)),
			SyntaxKind::Str					=> Self::Str(Str(n)),
			SyntaxKind::CodeBlock			=> Self::CodeBlock(CodeBlock(n)),
			SyntaxKind::ContentBlock		=> Self::ContentBlock(ContentBlock(n)),
			SyntaxKind::Parenthesized		=> Self::Parenthesized(Parenthesized(n)),
			SyntaxKind::Array				=> Self::Array(Array(n)),
			SyntaxKind::Dict				=> Self::Dict(Dict(n)),
			SyntaxKind::Unary				=> Self::Unary(Unary(n)),
			SyntaxKind::Binary				=> Self::Binary(Binary(n)),
			SyntaxKind::FieldAccess			=> Self::FieldAccess(FieldAccess(n)),
			SyntaxKind::FuncCall			=> Self::FuncCall(FuncCall(n)),
			SyntaxKind::Closure				=> Self::Closure(Closure(n)),
			SyntaxKind::LetBinding			=> Self::LetBinding(LetBinding(n)),
			SyntaxKind::DestructAssignment	=> Self::DestructAssignment(DestructAssignment(n)),
			SyntaxKind::SetRule				=> Self::SetRule(SetRule(n)),
			SyntaxKind::ShowRule			=> Self::ShowRule(ShowRule(n)),
			SyntaxKind::Contextual			=> Self::Contextual(Contextual(n)),
			SyntaxKind::Conditional			=> Self::Conditional(Conditional(n)),
			SyntaxKind::WhileLoop			=> Self::WhileLoop(WhileLoop(n)),
			SyntaxKind::ForLoop				=> Self::ForLoop(ForLoop(n)),
			SyntaxKind::ModuleImport		=> Self::ModuleImport(ModuleImport(n)),
			SyntaxKind::ModuleInclude		=> Self::ModuleInclude(ModuleInclude(n)),
			SyntaxKind::LoopBreak			=> Self::LoopBreak(LoopBreak(n)),
			SyntaxKind::LoopContinue		=> Self::LoopContinue(LoopContinue(n)),
			SyntaxKind::FuncReturn			=> Self::FuncReturn(FuncReturn(n)),
			_								=> return Option::None,
		})
	}

	fn to_untyped(self) -> &'a SyntaxNode {
		match self {
			Self::Text(v)				=> v.0,
			Self::Space(v)				=> v.0,
			Self::Linebreak(v)			=> v.0,
			Self::Parbreak(v)			=> v.0,
			Self::Escape(v)				=> v.0,
			Self::Shorthand(v)			=> v.0,
			Self::SmartQuote(v)			=> v.0,
			Self::Strong(v)				=> v.0,
			Self::Emph(v)				=> v.0,
			Self::Raw(v)				=> v.0,
			Self::Link(v)				=> v.0,
			Self::Label(v)				=> v.0,
			Self::Ref(v)				=> v.0,
			Self::Heading(v)			=> v.0,
			Self::ListItem(v)			=> v.0,
			Self::EnumItem(v)			=> v.0,
			Self::TermItem(v)			=> v.0,
			Self::Equation(v)			=> v.0,
			Self::Math(v)				=> v.0,
			Self::MathText(v)			=> v.0,
			Self::MathIdent(v)			=> v.0,
			Self::MathFieldAccess(v)	=> v.0,
			Self::MathShorthand(v)		=> v.0,
			Self::MathAlignPoint(v)		=> v.0,
			Self::MathCall(v)			=> v.0,
			Self::MathDelimited(v)		=> v.0,
			Self::MathAttach(v)			=> v.0,
			Self::MathPrimes(v)			=> v.0,
			Self::MathFrac(v)			=> v.0,
			Self::MathRoot(v)			=> v.0,
			Self::Ident(v)				=> v.0,
			Self::None(v)				=> v.0,
			Self::Auto(v)				=> v.0,
			Self::Bool(v)				=> v.0,
			Self::Int(v)				=> v.0,
			Self::Float(v)				=> v.0,
			Self::Numeric(v)			=> v.0,
			Self::Str(v)				=> v.0,
			Self::CodeBlock(v)			=> v.0,
			Self::ContentBlock(v)		=> v.0,
			Self::Parenthesized(v)		=> v.0,
			Self::Array(v)				=> v.0,
			Self::Dict(v)				=> v.0,
			Self::Unary(v)				=> v.0,
			Self::Binary(v)				=> v.0,
			Self::FieldAccess(v)		=> v.0,
			Self::FuncCall(v)			=> v.0,
			Self::Closure(v)			=> v.0,
			Self::LetBinding(v)			=> v.0,
			Self::DestructAssignment(v)	=> v.0,
			Self::SetRule(v)			=> v.0,
			Self::ShowRule(v)			=> v.0,
			Self::Contextual(v)			=> v.0,
			Self::Conditional(v)		=> v.0,
			Self::WhileLoop(v)			=> v.0,
			Self::ForLoop(v)			=> v.0,
			Self::ModuleImport(v)		=> v.0,
			Self::ModuleInclude(v)		=> v.0,
			Self::LoopBreak(v)			=> v.0,
			Self::LoopContinue(v)		=> v.0,
			Self::FuncReturn(v)			=> v.0,
		}
	}

	fn placeholder() -> Self { Self::None(None::placeholder()) }
}

impl Expr<'_> {
	/// Can the expression follow a `#` in markup or maths?
	pub fn hash(self) -> bool {
		matches!(self,
			Self::Ident(_) | Self::None(_) | Self::Auto(_) | Self::Bool(_) | Self::Int(_) | Self::Float(_)
			| Self::Numeric(_) | Self::Str(_) | Self::CodeBlock(_) | Self::ContentBlock(_) | Self::Array(_)
			| Self::Dict(_) | Self::Parenthesized(_) | Self::FieldAccess(_) | Self::FuncCall(_)
			| Self::LetBinding(_) | Self::SetRule(_) | Self::ShowRule(_) | Self::Contextual(_)
			| Self::Conditional(_) | Self::WhileLoop(_) | Self::ForLoop(_) | Self::ModuleImport(_)
			| Self::ModuleInclude(_) | Self::LoopBreak(_) | Self::LoopContinue(_) | Self::FuncReturn(_))
	}

	pub fn is_literal(self) -> bool {
		matches!(self,
			Self::None(_) | Self::Auto(_) | Self::Bool(_) | Self::Int(_) | Self::Float(_) | Self::Numeric(_)
			| Self::Str(_))
	}
}

node! { struct Text }

impl<'a> Text<'a> {
	pub fn get(self) -> &'a str { self.0.text() }
}

node! { struct Space }
node! { struct Linebreak }
node! { struct Parbreak }

node! { struct Escape }

impl Escape<'_> {
	/// The escaped char: `\#` is `#`, `\u{1F600}` is the code point.
	pub fn get(self) -> char {
		let mut s = Scanner::new(self.0.text());
		s.eat_if('\\');
		if s.eat_if("u{") {
			let hex = s.eat_while(|c: char| c.is_ascii_hexdigit());
			match u32::from_str_radix(hex, 16).ok().and_then(char::from_u32) {
				Some(c)			=> c,
				Option::None	=> char::default(),
			}
		} else {
			match s.eat() {
				Some(c)			=> c,
				Option::None	=> char::default(),
			}
		}
	}
}

node! { struct Shorthand }

impl Shorthand<'_> {
	pub const LIST: &'static [(&'static str, char)] = &[
		("...",	'…'),
		("~",	'\u{00A0}'),
		("-",	'\u{2212}'),	// only before a digit
		("--",	'\u{2013}'),
		("---",	'\u{2014}'),
		("-?",	'\u{00AD}'),
	];

	pub fn get(self) -> char { lookup(Self::LIST, self.0.text()) }
}

fn lookup(list: &[(&str, char)], text: &str) -> char {
	match list.iter().find(|(s, _)| *s == text) {
		Some((_, c))	=> *c,
		Option::None	=> char::default(),
	}
}

node! { struct SmartQuote }

impl SmartQuote<'_> {
	pub fn double(self) -> bool { self.0.text() == "\"" }
}

node! { struct Strong }

impl<'a> Strong<'a> {
	pub fn body(self) -> Markup<'a> { self.0.cast_first() }
}

node! { struct Emph }

impl<'a> Emph<'a> {
	pub fn body(self) -> Markup<'a> { self.0.cast_first() }
}

node! { struct Raw }

impl<'a> Raw<'a> {
	/// The lines of text, without the trimmed indentation and newlines.
	pub fn lines(self) -> impl DoubleEndedIterator<Item = Text<'a>> {
		self.0.children().iter().filter_map(|c| c.cast())
	}

	/// The language tag; only a raw of three or more backticks has one.
	pub fn lang(self) -> Option<RawLang<'a>> {
		let delim: RawDelim = match self.0.try_cast_first() {
			Some(d)			=> d,
			Option::None	=> return Option::None,
		};
		if delim.0.len() < 3 {
			return Option::None;
		}
		self.0.try_cast_first()
	}

	/// Is it a block: three or more backticks and a newline among the trimmed parts?
	pub fn block(self) -> bool {
		let blocky = match self.0.try_cast_first::<RawDelim>() {
			Some(d)			=> d.0.len() >= 3,
			Option::None	=> false,
		};
		blocky && self.0.children().iter().any(|e|
			e.kind() == SyntaxKind::RawTrimmed && e.text().chars().any(is_newline))
	}
}

node! { struct RawLang }

impl<'a> RawLang<'a> {
	pub fn get(self) -> &'a str { self.0.text() }
}

node! { struct RawDelim }

node! { struct Link }

impl<'a> Link<'a> {
	pub fn get(self) -> &'a str { self.0.text() }
}

node! { struct Label }

impl<'a> Label<'a> {
	/// The name between the angle brackets.
	pub fn get(self) -> &'a str { self.0.text().trim_start_matches('<').trim_end_matches('>') }
}

node! { struct Ref }

impl<'a> Ref<'a> {
	pub fn target(self) -> &'a str {
		match self.0.children().iter().find(|n| n.kind() == SyntaxKind::RefMarker) {
			Some(n)			=> n.text().trim_start_matches('@'),
			Option::None	=> "",
		}
	}

	pub fn supplement(self) -> Option<ContentBlock<'a>> { self.0.try_cast_last() }
}

node! { struct Heading }

impl<'a> Heading<'a> {
	pub fn body(self) -> Markup<'a> { self.0.cast_first() }

	/// The number of `=` in the marker, at least one.
	pub fn depth(self) -> usize {
		match self.0.children().iter().find(|n| n.kind() == SyntaxKind::HeadingMarker) {
			Some(n) if n.len() > 0	=> n.len(),
			_						=> 1,
		}
	}
}

node! { struct ListItem }

impl<'a> ListItem<'a> {
	pub fn body(self) -> Markup<'a> { self.0.cast_first() }
}

node! { struct EnumItem }

impl<'a> EnumItem<'a> {
	/// The explicit number of a `3.` marker; `None` for `+`.
	pub fn number(self) -> Option<u64> {
		self.0.children().iter().find_map(|n| match n.kind() {
			SyntaxKind::EnumMarker	=> n.text().trim_end_matches('.').parse().ok(),
			_						=> Option::None,
		})
	}

	pub fn body(self) -> Markup<'a> { self.0.cast_first() }
}

node! { struct TermItem }

impl<'a> TermItem<'a> {
	pub fn term(self) -> Markup<'a> { self.0.cast_first() }
	pub fn description(self) -> Markup<'a> { self.0.cast_last() }
}

node! { struct Equation }

impl<'a> Equation<'a> {
	pub fn body(self) -> Math<'a> { self.0.cast_first() }

	/// Is it a display equation, with space inside both dollars?
	pub fn block(self) -> bool {
		let c = self.0.children();
		let is_space = |n: Option<&SyntaxNode>| matches!(n.map(|n| n.kind()), Some(SyntaxKind::Space));
		c.len() >= 2 && is_space(c.get(1)) && is_space(c.len().checked_sub(2).and_then(|i| c.get(i)))
	}
}

// ┌───────────────────────────────────────────────────────────────────────────────────────────┐
// │ Maths                                                                                     │
// └───────────────────────────────────────────────────────────────────────────────────────────┘

node! { struct Math }

impl<'a> Math<'a> {
	pub fn exprs(self) -> impl DoubleEndedIterator<Item = Expr<'a>> {
		self.0.children().iter().filter_map(Expr::cast_with_space)
	}

	/// Did a fraction, attachment or root strip this sequence's round parentheses?
	pub fn was_deparenthesized(self) -> bool {
		let c = self.0.children();
		matches!(c.first().map(|n| n.kind()), Some(SyntaxKind::LeftParen))
			&& matches!(c.last().map(|n| n.kind()), Some(SyntaxKind::RightParen))
	}
}

node! { struct MathText }

/// What a `MathText` holds: a number, possibly several chars, or one grapheme.
pub enum MathTextKind<'a> {
	Grapheme(&'a str),
	Number(&'a str),
}

impl<'a> MathText<'a> {
	pub fn get(self) -> MathTextKind<'a> {
		let text = self.0.text();
		match text.chars().next() {
			Some(c) if c.is_numeric()	=> MathTextKind::Number(text),
			_							=> MathTextKind::Grapheme(text),
		}
	}
}

node! { struct MathIdent }

impl<'a> MathIdent<'a> {
	pub fn get(self) -> &'a str { self.0.text() }
	pub fn as_str(self) -> &'a str { self.get() }
}

impl Deref for MathIdent<'_> {
	type Target = str;
	fn deref(&self) -> &Self::Target { self.0.text() }
}

node! { struct MathFieldAccess }

impl<'a> MathFieldAccess<'a> {
	pub fn target(self) -> MathAccess<'a> { self.0.cast_first() }
	pub fn field(self) -> MathIdent<'a> { self.0.cast_last() }
}

/// The callee of a maths call: an identifier or a field access chain.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub enum MathAccess<'a> {
	MathIdent(MathIdent<'a>),
	MathFieldAccess(MathFieldAccess<'a>),
}

impl<'a> AstNode<'a> for MathAccess<'a> {
	fn from_untyped(node: &'a SyntaxNode) -> Option<Self> {
		match node.kind() {
			SyntaxKind::MathIdent		=> Some(Self::MathIdent(MathIdent(node))),
			SyntaxKind::MathFieldAccess	=> Some(Self::MathFieldAccess(MathFieldAccess(node))),
			_							=> Option::None,
		}
	}

	fn to_untyped(self) -> &'a SyntaxNode {
		match self {
			Self::MathIdent(v)			=> v.0,
			Self::MathFieldAccess(v)	=> v.0,
		}
	}

	fn placeholder() -> Self { Self::MathIdent(MathIdent::placeholder()) }
}

node! { struct MathShorthand }

impl MathShorthand<'_> {
	pub const LIST: &'static [(&'static str, char)] = &[
		("...",		'…'),
		("-",		'−'),
		("*",		'∗'),
		("~",		'∼'),
		("!=",		'≠'),
		(":=",		'≔'),
		("::=",		'⩴'),
		("=:",		'≕'),
		("<<",		'≪'),
		("<<<",		'⋘'),
		(">>",		'≫'),
		(">>>",		'⋙'),
		("<=",		'≤'),
		(">=",		'≥'),
		("->",		'→'),
		("-->",		'⟶'),
		("|->",		'↦'),
		(">->",		'↣'),
		("->>",		'↠'),
		("<-",		'←'),
		("<--",		'⟵'),
		("<-<",		'↢'),
		("<<-",		'↞'),
		("<->",		'↔'),
		("<-->",	'⟷'),
		("~>",		'⇝'),
		("~~>",		'⟿'),
		("<~",		'⇜'),
		("<~~",		'⬳'),
		("=>",		'⇒'),
		("|=>",		'⤇'),
		("==>",		'⟹'),
		("<==",		'⟸'),
		("<=>",		'⇔'),
		("<==>",	'⟺'),
		("[|",		'⟦'),
		("|]",		'⟧'),
		("||",		'‖'),
	];

	pub fn get(self) -> char { lookup(Self::LIST, self.0.text()) }
}

node! { struct MathCall }

impl<'a> MathCall<'a> {
	pub fn callee(self) -> MathAccess<'a> { self.0.cast_first() }
	pub fn args(self) -> MathArgs<'a> { self.0.cast_last() }
}

node! { struct MathArgs }

/// One argument of a maths call and whether a semicolon ends it, which makes rows of a matrix.
#[derive(Debug, Copy, Clone, Hash)]
pub struct MathArg<'a> {
	pub arg:				Arg<'a>,
	pub ends_in_semicolon:	bool,
}

/// An argument or a piece of punctuation of a maths call, for rebuilding the call as content when the
/// callee is not a function.
#[derive(Debug, Copy, Clone, Hash)]
pub enum MathArgItem<'a> {
	Arg(Arg<'a>),
	Space(Space<'a>),
	Comma(char, &'a SyntaxNode),
	Semicolon(char, &'a SyntaxNode),
	LeftParen(char, &'a SyntaxNode),
	RightParen(char, &'a SyntaxNode),
}

impl<'a> MathArgs<'a> {
	pub fn arg_items(self) -> impl Iterator<Item = MathArg<'a>> {
		let mut items = self.content_items().peekable();
		std::iter::from_fn(move || {
			let arg = items.find_map(|n| match n {
				MathArgItem::Arg(a)	=> Some(a),
				_					=> Option::None,
			});
			let arg = match arg {
				Some(a)			=> a,
				Option::None	=> return Option::None,
			};
			let ends_in_semicolon = loop {
				match items.peek() {
					Option::None | Some(MathArgItem::Arg(_))	=> break false,
					Some(MathArgItem::Semicolon(_, _))			=> break true,
					Some(_)										=> (),
				}
				items.next();
			};
			Some(MathArg { arg, ends_in_semicolon })
		})
	}

	/// The arguments with the punctuation between them; a semicolon that ends an embedded code
	/// expression (`#x;`) is not punctuation.
	pub fn content_items(self) -> impl Iterator<Item = MathArgItem<'a>> {
		let mut children = self.0.children().iter();
		let mut prev_hash = false;
		std::iter::from_fn(move || {
			for node in children.by_ref() {
				if let Some(arg) = node.cast() {
					return Some(MathArgItem::Arg(arg));
				}
				let ends_code = prev_hash;
				prev_hash = false;
				let item = match node.kind() {
					SyntaxKind::Space						=> MathArgItem::Space(Space(node)),
					SyntaxKind::Comma						=> MathArgItem::Comma(',', node),
					SyntaxKind::LeftParen					=> MathArgItem::LeftParen('(', node),
					SyntaxKind::RightParen					=> MathArgItem::RightParen(')', node),
					SyntaxKind::Semicolon if !ends_code		=> MathArgItem::Semicolon(';', node),
					SyntaxKind::Hash						=> {
						prev_hash = true;
						continue;
					},
					_										=> continue,
				};
				return Some(item);
			}
			Option::None
		})
	}
}

node! { struct MathAlignPoint }

node! { struct MathDelimited }

impl<'a> MathDelimited<'a> {
	pub fn open(self) -> Expr<'a> { self.0.cast_first() }
	pub fn body(self) -> Math<'a> { self.0.cast_first() }
	pub fn close(self) -> Expr<'a> { self.0.cast_last() }
}

node! { struct MathAttach }

impl<'a> MathAttach<'a> {
	pub fn base(self) -> Expr<'a> { self.0.cast_first() }

	pub fn bottom(self) -> Option<Expr<'a>> {
		self.0.children().iter()
			.skip_while(|n| n.kind() != SyntaxKind::Underscore)
			.find_map(|n| n.cast())
	}

	pub fn top(self) -> Option<Expr<'a>> {
		self.0.children().iter()
			.skip_while(|n| n.kind() != SyntaxKind::Hat)
			.find_map(|n| n.cast())
	}

	/// The primes straight after the base.
	pub fn primes(self) -> Option<MathPrimes<'a>> {
		self.0.children().iter()
			.skip_while(|n| n.cast::<Expr<'_>>().is_none())
			.nth(1)
			.and_then(|n| n.cast())
	}
}

node! { struct MathPrimes }

impl MathPrimes<'_> {
	pub fn count(self) -> usize { self.0.text().len() }
}

node! { struct MathFrac }

impl<'a> MathFrac<'a> {
	pub fn num(self) -> Expr<'a> { self.0.cast_first() }
	pub fn denom(self) -> Expr<'a> { self.0.cast_last() }
}

node! { struct MathRoot }

impl<'a> MathRoot<'a> {
	/// 3 for `∛`, 4 for `∜`, none for `√`.
	pub fn index(self) -> Option<u8> {
		match self.0.children().first().map(|n| n.text()) {
			Some("∜")	=> Some(4),
			Some("∛")	=> Some(3),
			_			=> Option::None,
		}
	}

	pub fn radicand(self) -> Expr<'a> { self.0.cast_first() }
}

// ┌───────────────────────────────────────────────────────────────────────────────────────────┐
// │ Literals                                                                                  │
// └───────────────────────────────────────────────────────────────────────────────────────────┘

node! { struct Ident }

impl<'a> Ident<'a> {
	pub fn get(self) -> &'a str { self.0.text() }
	pub fn as_str(self) -> &'a str { self.get() }
}

impl Deref for Ident<'_> {
	type Target = str;
	fn deref(&self) -> &Self::Target { self.0.text() }
}

node! { struct None }
node! { struct Auto }

node! { struct Bool }

impl Bool<'_> {
	pub fn get(self) -> bool { self.0.text() == "true" }
}

node! { struct Int }

impl Int<'_> {
	pub fn get(self) -> i64 {
		let text = self.0.text();
		let parsed = if let Some(rest) = text.strip_prefix("0x") {
			i64::from_str_radix(rest, 16)
		} else if let Some(rest) = text.strip_prefix("0o") {
			i64::from_str_radix(rest, 8)
		} else if let Some(rest) = text.strip_prefix("0b") {
			i64::from_str_radix(rest, 2)
		} else {
			text.parse()
		};
		match parsed {
			Ok(n)	=> n,
			Err(_)	=> 0,
		}
	}
}

node! { struct Float }

impl Float<'_> {
	pub fn get(self) -> f64 {
		match self.0.text().parse() {
			Ok(f)	=> f,
			Err(_)	=> 0.0,
		}
	}
}

node! { struct Numeric }

impl Numeric<'_> {
	pub fn get(self) -> (f64, Unit) {
		let text = self.0.text();
		let count = text.chars().rev().take_while(|c| matches!(c, 'a'..='z' | '%')).count();
		let split = text.len() - count;
		let value = match text.get(..split).map(|s| s.parse::<f64>()) {
			Some(Ok(v))	=> v,
			_			=> 0.0,
		};
		let unit = match text.get(split..) {
			Some("pt")	=> Unit::Pt,
			Some("mm")	=> Unit::Mm,
			Some("cm")	=> Unit::Cm,
			Some("in")	=> Unit::In,
			Some("deg")	=> Unit::Deg,
			Some("rad")	=> Unit::Rad,
			Some("em")	=> Unit::Em,
			Some("fr")	=> Unit::Fr,
			_			=> Unit::Percent,
		};
		(value, unit)
	}
}

/// The unit of a numeric literal.
#[derive(Debug, Copy, Clone, Eq, PartialEq, Hash)]
pub enum Unit {
	Pt,
	Mm,
	Cm,
	In,
	Rad,
	Deg,
	Em,
	Fr,
	Percent,
}

node! { struct Str }

impl Str<'_> {
	/// The string's value, escapes resolved.
	pub fn get(self) -> String { unescape_str(self.0.text()) }
}

/// Resolves a quoted string literal: `\\ \" \n \r \t \u{..}`; any other escape stays as written.
pub fn unescape_str(quoted: &str) -> String {
	let inner = match quoted.get(1..quoted.len().saturating_sub(1)) {
		Some(s) if quoted.len() >= 2	=> s,
		_								=> "",
	};
	if !inner.contains('\\') {
		return inner.to_string();
	}
	let mut out = String::with_capacity(inner.len());
	let mut s = Scanner::new(inner);
	while let Some(c) = s.eat() {
		if c != '\\' {
			out.push(c);
			continue;
		}
		let start = s.cursor() - 1;
		match s.eat() {
			Some('\\')				=> out.push('\\'),
			Some('"')				=> out.push('"'),
			Some('n')				=> out.push('\n'),
			Some('r')				=> out.push('\r'),
			Some('t')				=> out.push('\t'),
			Some('u') if s.eat_if('{')	=> {
				let seq = s.eat_while(|c: char| c.is_ascii_hexdigit());
				s.eat_if('}');
				match u32::from_str_radix(seq, 16).ok().and_then(char::from_u32) {
					Some(c)			=> out.push(c),
					Option::None	=> out.push_str(s.from(start)),
				}
			},
			_						=> out.push_str(s.from(start)),
		}
	}
	out
}

// ┌───────────────────────────────────────────────────────────────────────────────────────────┐
// │ Code                                                                                      │
// └───────────────────────────────────────────────────────────────────────────────────────────┘

node! { struct CodeBlock }

impl<'a> CodeBlock<'a> {
	pub fn body(self) -> Code<'a> { self.0.cast_first() }
}

node! { struct Code }

impl<'a> Code<'a> {
	pub fn exprs(self) -> impl DoubleEndedIterator<Item = Expr<'a>> {
		self.0.children().iter().filter_map(|c| c.cast())
	}
}

node! { struct ContentBlock }

impl<'a> ContentBlock<'a> {
	pub fn body(self) -> Markup<'a> { self.0.cast_first() }
}

node! { struct Parenthesized }

impl<'a> Parenthesized<'a> {
	pub fn expr(self) -> Expr<'a> { self.0.cast_first() }
	pub fn pattern(self) -> Pattern<'a> { self.0.cast_first() }
}

node! { struct Array }

impl<'a> Array<'a> {
	pub fn items(self) -> impl DoubleEndedIterator<Item = ArrayItem<'a>> {
		self.0.children().iter().filter_map(|c| c.cast())
	}
}

#[derive(Debug, Copy, Clone, Hash)]
pub enum ArrayItem<'a> {
	Pos(Expr<'a>),
	Spread(Spread<'a>),
}

impl<'a> AstNode<'a> for ArrayItem<'a> {
	fn from_untyped(node: &'a SyntaxNode) -> Option<Self> {
		match node.kind() {
			SyntaxKind::Spread	=> Some(Self::Spread(Spread(node))),
			_					=> node.cast().map(Self::Pos),
		}
	}

	fn to_untyped(self) -> &'a SyntaxNode {
		match self {
			Self::Pos(v)	=> v.to_untyped(),
			Self::Spread(v)	=> v.0,
		}
	}

	fn placeholder() -> Self { Self::Pos(Expr::placeholder()) }
}

node! { struct Dict }

impl<'a> Dict<'a> {
	pub fn items(self) -> impl DoubleEndedIterator<Item = DictItem<'a>> {
		self.0.children().iter().filter_map(|c| c.cast())
	}
}

#[derive(Debug, Copy, Clone, Hash)]
pub enum DictItem<'a> {
	Named(Named<'a>),
	Keyed(Keyed<'a>),
	Spread(Spread<'a>),
}

impl<'a> AstNode<'a> for DictItem<'a> {
	fn from_untyped(node: &'a SyntaxNode) -> Option<Self> {
		match node.kind() {
			SyntaxKind::Named	=> Some(Self::Named(Named(node))),
			SyntaxKind::Keyed	=> Some(Self::Keyed(Keyed(node))),
			SyntaxKind::Spread	=> Some(Self::Spread(Spread(node))),
			_					=> Option::None,
		}
	}

	fn to_untyped(self) -> &'a SyntaxNode {
		match self {
			Self::Named(v)	=> v.0,
			Self::Keyed(v)	=> v.0,
			Self::Spread(v)	=> v.0,
		}
	}

	fn placeholder() -> Self { Self::Spread(Spread::placeholder()) }
}

node! { struct Named }

impl<'a> Named<'a> {
	pub fn name(self) -> Ident<'a> { self.0.cast_first() }
	pub fn expr(self) -> Expr<'a> { self.0.cast_last() }
	pub fn pattern(self) -> Pattern<'a> { self.0.cast_last() }
}

node! { struct Keyed }

impl<'a> Keyed<'a> {
	pub fn key(self) -> Expr<'a> { self.0.cast_first() }
	pub fn expr(self) -> Expr<'a> { self.0.cast_last() }
}

node! { struct Spread }

impl<'a> Spread<'a> {
	pub fn expr(self) -> Expr<'a> { self.0.cast_first() }
	pub fn sink_ident(self) -> Option<Ident<'a>> { self.0.try_cast_first() }
	pub fn sink_expr(self) -> Option<Expr<'a>> { self.0.try_cast_first() }
}

node! { struct Unary }

impl<'a> Unary<'a> {
	pub fn op(self) -> UnOp {
		match self.0.children().iter().find_map(|n| UnOp::from_kind(n.kind())) {
			Some(op)		=> op,
			Option::None	=> UnOp::Pos,
		}
	}

	pub fn expr(self) -> Expr<'a> { self.0.cast_last() }
}

/// A unary operator.
#[derive(Debug, Copy, Clone, Eq, PartialEq, Hash)]
pub enum UnOp {
	Pos,
	Neg,
	Not,
}

impl UnOp {
	pub fn from_kind(token: SyntaxKind) -> Option<Self> {
		match token {
			SyntaxKind::Plus	=> Some(Self::Pos),
			SyntaxKind::Minus	=> Some(Self::Neg),
			SyntaxKind::Not		=> Some(Self::Not),
			_					=> Option::None,
		}
	}

	pub fn precedence(self) -> u8 {
		match self {
			Self::Pos | Self::Neg	=> 7,
			Self::Not				=> 4,
		}
	}

	pub fn as_str(self) -> &'static str {
		match self {
			Self::Pos	=> "+",
			Self::Neg	=> "-",
			Self::Not	=> "not",
		}
	}
}

node! { struct Binary }

impl<'a> Binary<'a> {
	pub fn op(self) -> BinOp {
		let mut not = false;
		let op = self.0.children().iter().find_map(|n| match n.kind() {
			SyntaxKind::Not			=> {
				not = true;
				Option::None
			},
			SyntaxKind::In if not	=> Some(BinOp::NotIn),
			k						=> BinOp::from_kind(k),
		});
		match op {
			Some(op)		=> op,
			Option::None	=> BinOp::Add,
		}
	}

	pub fn lhs(self) -> Expr<'a> { self.0.cast_first() }
	pub fn rhs(self) -> Expr<'a> { self.0.cast_last() }
}

/// A binary operator.
#[derive(Debug, Copy, Clone, Eq, PartialEq, Hash)]
pub enum BinOp {
	Add,
	Sub,
	Mul,
	Div,
	And,
	Or,
	Eq,
	Neq,
	Lt,
	Leq,
	Gt,
	Geq,
	Assign,
	In,
	NotIn,
	AddAssign,
	SubAssign,
	MulAssign,
	DivAssign,
}

impl BinOp {
	pub fn from_kind(token: SyntaxKind) -> Option<Self> {
		Some(match token {
			SyntaxKind::Plus	=> Self::Add,
			SyntaxKind::Minus	=> Self::Sub,
			SyntaxKind::Star	=> Self::Mul,
			SyntaxKind::Slash	=> Self::Div,
			SyntaxKind::And		=> Self::And,
			SyntaxKind::Or		=> Self::Or,
			SyntaxKind::EqEq	=> Self::Eq,
			SyntaxKind::ExclEq	=> Self::Neq,
			SyntaxKind::Lt		=> Self::Lt,
			SyntaxKind::LtEq	=> Self::Leq,
			SyntaxKind::Gt		=> Self::Gt,
			SyntaxKind::GtEq	=> Self::Geq,
			SyntaxKind::Eq		=> Self::Assign,
			SyntaxKind::In		=> Self::In,
			SyntaxKind::PlusEq	=> Self::AddAssign,
			SyntaxKind::HyphEq	=> Self::SubAssign,
			SyntaxKind::StarEq	=> Self::MulAssign,
			SyntaxKind::SlashEq	=> Self::DivAssign,
			_					=> return Option::None,
		})
	}

	pub fn precedence(self) -> u8 {
		match self {
			Self::Mul | Self::Div												=> 6,
			Self::Add | Self::Sub												=> 5,
			Self::Eq | Self::Neq | Self::Lt | Self::Leq | Self::Gt | Self::Geq
			| Self::In | Self::NotIn											=> 4,
			Self::And															=> 3,
			Self::Or															=> 2,
			Self::Assign | Self::AddAssign | Self::SubAssign | Self::MulAssign
			| Self::DivAssign													=> 1,
		}
	}

	pub fn assoc(self) -> Assoc {
		match self {
			Self::Assign | Self::AddAssign | Self::SubAssign | Self::MulAssign | Self::DivAssign	=>
				Assoc::Right,
			_	=> Assoc::Left,
		}
	}

	pub fn as_str(self) -> &'static str {
		match self {
			Self::Add		=> "+",
			Self::Sub		=> "-",
			Self::Mul		=> "*",
			Self::Div		=> "/",
			Self::And		=> "and",
			Self::Or		=> "or",
			Self::Eq		=> "==",
			Self::Neq		=> "!=",
			Self::Lt		=> "<",
			Self::Leq		=> "<=",
			Self::Gt		=> ">",
			Self::Geq		=> ">=",
			Self::In		=> "in",
			Self::NotIn		=> "not in",
			Self::Assign	=> "=",
			Self::AddAssign	=> "+=",
			Self::SubAssign	=> "-=",
			Self::MulAssign	=> "*=",
			Self::DivAssign	=> "/=",
		}
	}
}

/// Which side an operator chain groups from.
#[derive(Debug, Copy, Clone, Eq, PartialEq, Hash)]
pub enum Assoc {
	Left,
	Right,
}

node! { struct FieldAccess }

impl<'a> FieldAccess<'a> {
	pub fn target(self) -> Expr<'a> { self.0.cast_first() }
	pub fn field(self) -> Ident<'a> { self.0.cast_last() }
}

node! { struct FuncCall }

impl<'a> FuncCall<'a> {
	pub fn callee(self) -> Expr<'a> { self.0.cast_first() }
	pub fn args(self) -> Args<'a> { self.0.cast_last() }
}

node! { struct Args }

impl<'a> Args<'a> {
	pub fn items(self) -> impl DoubleEndedIterator<Item = Arg<'a>> {
		self.0.children().iter().filter_map(|c| c.cast())
	}

	/// Does the parenthesised list end in a comma, as `(a,)` does?
	pub fn trailing_comma(self) -> bool {
		self.0.children().iter().rev().skip(1)
			.find(|n| !n.kind().is_trivia())
			.is_some_and(|n| n.kind() == SyntaxKind::Comma)
	}
}

#[derive(Debug, Copy, Clone, Hash)]
pub enum Arg<'a> {
	Pos(Expr<'a>),
	Named(Named<'a>),
	Spread(Spread<'a>),
}

impl<'a> AstNode<'a> for Arg<'a> {
	fn from_untyped(node: &'a SyntaxNode) -> Option<Self> {
		match node.kind() {
			SyntaxKind::Named	=> Some(Self::Named(Named(node))),
			SyntaxKind::Spread	=> Some(Self::Spread(Spread(node))),
			_					=> node.cast().map(Self::Pos),
		}
	}

	fn to_untyped(self) -> &'a SyntaxNode {
		match self {
			Self::Pos(v)	=> v.to_untyped(),
			Self::Named(v)	=> v.0,
			Self::Spread(v)	=> v.0,
		}
	}

	fn placeholder() -> Self { Self::Pos(Expr::placeholder()) }
}

node! { struct Closure }

impl<'a> Closure<'a> {
	/// The name of a `let f(x) = ..` closure.
	pub fn name(self) -> Option<Ident<'a>> { self.0.children().first().and_then(|n| n.cast()) }
	pub fn params(self) -> Params<'a> { self.0.cast_first() }
	pub fn body(self) -> Expr<'a> { self.0.cast_last() }
}

node! { struct Params }

impl<'a> Params<'a> {
	pub fn children(self) -> impl DoubleEndedIterator<Item = Param<'a>> {
		self.0.children().iter().filter_map(|c| c.cast())
	}
}

#[derive(Debug, Copy, Clone, Hash)]
pub enum Param<'a> {
	Pos(Pattern<'a>),
	Named(Named<'a>),
	Spread(Spread<'a>),
}

impl<'a> AstNode<'a> for Param<'a> {
	fn from_untyped(node: &'a SyntaxNode) -> Option<Self> {
		match node.kind() {
			SyntaxKind::Named	=> Some(Self::Named(Named(node))),
			SyntaxKind::Spread	=> Some(Self::Spread(Spread(node))),
			_					=> node.cast().map(Self::Pos),
		}
	}

	fn to_untyped(self) -> &'a SyntaxNode {
		match self {
			Self::Pos(v)	=> v.to_untyped(),
			Self::Named(v)	=> v.0,
			Self::Spread(v)	=> v.0,
		}
	}

	fn placeholder() -> Self { Self::Pos(Pattern::placeholder()) }
}

/// A binding or reassignment target.
#[derive(Debug, Copy, Clone, Hash)]
pub enum Pattern<'a> {
	Normal(Expr<'a>),
	Placeholder(Underscore<'a>),
	Parenthesized(Parenthesized<'a>),
	Destructuring(Destructuring<'a>),
}

impl<'a> AstNode<'a> for Pattern<'a> {
	fn from_untyped(node: &'a SyntaxNode) -> Option<Self> {
		match node.kind() {
			SyntaxKind::Underscore		=> Some(Self::Placeholder(Underscore(node))),
			SyntaxKind::Parenthesized	=> Some(Self::Parenthesized(Parenthesized(node))),
			SyntaxKind::Destructuring	=> Some(Self::Destructuring(Destructuring(node))),
			_							=> node.cast().map(Self::Normal),
		}
	}

	fn to_untyped(self) -> &'a SyntaxNode {
		match self {
			Self::Normal(v)			=> v.to_untyped(),
			Self::Placeholder(v)	=> v.0,
			Self::Parenthesized(v)	=> v.0,
			Self::Destructuring(v)	=> v.0,
		}
	}

	fn placeholder() -> Self { Self::Normal(Expr::placeholder()) }
}

impl<'a> Pattern<'a> {
	/// The identifiers the pattern binds.
	pub fn bindings(self) -> Vec<Ident<'a>> {
		match self {
			Self::Normal(Expr::Ident(ident))	=> vec![ident],
			Self::Parenthesized(v)				=> v.pattern().bindings(),
			Self::Destructuring(v)				=> v.bindings(),
			_									=> Vec::new(),
		}
	}
}

node! { struct Underscore }

node! { struct Destructuring }

impl<'a> Destructuring<'a> {
	pub fn items(self) -> impl DoubleEndedIterator<Item = DestructuringItem<'a>> {
		self.0.children().iter().filter_map(|c| c.cast())
	}

	pub fn bindings(self) -> Vec<Ident<'a>> {
		self.items()
			.flat_map(|b| match b {
				DestructuringItem::Pattern(p)	=> p.bindings(),
				DestructuringItem::Named(n)		=> n.pattern().bindings(),
				DestructuringItem::Spread(s)	=> s.sink_ident().into_iter().collect(),
			})
			.collect()
	}
}

#[derive(Debug, Copy, Clone, Hash)]
pub enum DestructuringItem<'a> {
	Pattern(Pattern<'a>),
	Named(Named<'a>),
	Spread(Spread<'a>),
}

impl<'a> AstNode<'a> for DestructuringItem<'a> {
	fn from_untyped(node: &'a SyntaxNode) -> Option<Self> {
		match node.kind() {
			SyntaxKind::Named	=> Some(Self::Named(Named(node))),
			SyntaxKind::Spread	=> Some(Self::Spread(Spread(node))),
			_					=> node.cast().map(Self::Pattern),
		}
	}

	fn to_untyped(self) -> &'a SyntaxNode {
		match self {
			Self::Pattern(v)	=> v.to_untyped(),
			Self::Named(v)		=> v.0,
			Self::Spread(v)		=> v.0,
		}
	}

	fn placeholder() -> Self { Self::Pattern(Pattern::placeholder()) }
}

node! { struct LetBinding }

/// What a `let` binds: a pattern, or a named closure.
#[derive(Debug)]
pub enum LetBindingKind<'a> {
	Normal(Pattern<'a>),
	Closure(Ident<'a>),
}

impl<'a> LetBindingKind<'a> {
	pub fn bindings(self) -> Vec<Ident<'a>> {
		match self {
			LetBindingKind::Normal(p)	=> p.bindings(),
			LetBindingKind::Closure(i)	=> vec![i],
		}
	}
}

impl<'a> LetBinding<'a> {
	pub fn kind(self) -> LetBindingKind<'a> {
		match self.0.cast_first() {
			Pattern::Normal(Expr::Closure(c))	=> LetBindingKind::Closure(match c.name() {
				Some(n)			=> n,
				Option::None	=> Ident::placeholder(),
			}),
			pattern								=> LetBindingKind::Normal(pattern),
		}
	}

	/// The initial value; a named closure is its own.
	pub fn init(self) -> Option<Expr<'a>> {
		match self.kind() {
			LetBindingKind::Normal(Pattern::Normal(_) | Pattern::Parenthesized(_))	=>
				self.0.children().iter().filter_map(|c| c.cast::<Expr>()).nth(1),
			LetBindingKind::Normal(_)												=> self.0.try_cast_first(),
			LetBindingKind::Closure(_)												=> self.0.try_cast_first(),
		}
	}
}

node! { struct DestructAssignment }

impl<'a> DestructAssignment<'a> {
	pub fn pattern(self) -> Pattern<'a> { self.0.cast_first() }
	pub fn value(self) -> Expr<'a> { self.0.cast_last() }
}

node! { struct SetRule }

impl<'a> SetRule<'a> {
	pub fn target(self) -> Expr<'a> { self.0.cast_first() }
	pub fn args(self) -> Args<'a> { self.0.cast_last() }

	pub fn condition(self) -> Option<Expr<'a>> {
		self.0.children().iter()
			.skip_while(|c| c.kind() != SyntaxKind::If)
			.find_map(|c| c.cast())
	}
}

node! { struct ShowRule }

impl<'a> ShowRule<'a> {
	/// The selector before the colon; none for `show: f`.
	pub fn selector(self) -> Option<Expr<'a>> {
		self.0.children().iter().rev()
			.skip_while(|c| c.kind() != SyntaxKind::Colon)
			.find_map(|c| c.cast())
	}

	pub fn transform(self) -> Expr<'a> { self.0.cast_last() }
}

node! { struct Contextual }

impl<'a> Contextual<'a> {
	pub fn body(self) -> Expr<'a> { self.0.cast_first() }
}

node! { struct Conditional }

impl<'a> Conditional<'a> {
	pub fn condition(self) -> Expr<'a> { self.0.cast_first() }

	pub fn if_body(self) -> Expr<'a> {
		match self.0.children().iter().filter_map(|c| c.cast()).nth(1) {
			Some(e)			=> e,
			Option::None	=> Expr::placeholder(),
		}
	}

	pub fn else_body(self) -> Option<Expr<'a>> {
		self.0.children().iter().filter_map(|c| c.cast()).nth(2)
	}
}

node! { struct WhileLoop }

impl<'a> WhileLoop<'a> {
	pub fn condition(self) -> Expr<'a> { self.0.cast_first() }
	pub fn body(self) -> Expr<'a> { self.0.cast_last() }
}

node! { struct ForLoop }

impl<'a> ForLoop<'a> {
	pub fn pattern(self) -> Pattern<'a> { self.0.cast_first() }

	pub fn iterable(self) -> Expr<'a> {
		let found = self.0.children().iter()
			.skip_while(|c| c.kind() != SyntaxKind::In)
			.find_map(|c| c.cast());
		match found {
			Some(e)			=> e,
			Option::None	=> Expr::placeholder(),
		}
	}

	pub fn body(self) -> Expr<'a> { self.0.cast_last() }
}

node! { struct ModuleImport }

impl<'a> ModuleImport<'a> {
	pub fn source(self) -> Expr<'a> { self.0.cast_first() }

	pub fn imports(self) -> Option<Imports<'a>> {
		self.0.children().iter().find_map(|n| match n.kind() {
			SyntaxKind::Star		=> Some(Imports::Wildcard),
			SyntaxKind::ImportItems	=> n.cast().map(Imports::Items),
			_						=> Option::None,
		})
	}

	/// The name a bare `import "a/b.typ"` or `import "@preview/pkg:1.0.0"` binds: the file stem, the
	/// package name or the last identifier.
	pub fn bare_name(self) -> Result<String, BareImportError> {
		match self.source() {
			Expr::Ident(ident)			=> Ok(ident.get().to_string()),
			Expr::FieldAccess(access)	=> Ok(access.field().get().to_string()),
			Expr::Str(string)			=> {
				let string = string.get();
				let name = if string.starts_with('@') {
					match crate::eval::package::parse_spec(&string) {
						Ok(spec)	=> spec.name,
						Err(_)		=> return Err(BareImportError::PackageInvalid),
					}
				} else {
					match Path::new(&string).file_stem().and_then(|p| p.to_str()) {
						Some(n)			=> n.to_string(),
						Option::None	=> return Err(BareImportError::PathInvalid),
					}
				};
				if !is_ident(&name) {
					return Err(BareImportError::PathInvalid);
				}
				Ok(name)
			},
			_							=> Err(BareImportError::Dynamic),
		}
	}

	pub fn new_name(self) -> Option<Ident<'a>> {
		self.0.children().iter()
			.skip_while(|c| c.kind() != SyntaxKind::As)
			.find_map(|c| c.cast())
	}
}

/// Why a bare import has no name to bind.
#[derive(Debug, Copy, Clone, Eq, PartialEq, Hash)]
pub enum BareImportError {
	Dynamic,
	PathInvalid,
	PackageInvalid,
}

#[derive(Debug, Copy, Clone, Hash)]
pub enum Imports<'a> {
	Wildcard,
	Items(ImportItems<'a>),
}

node! { struct ImportItems }

impl<'a> ImportItems<'a> {
	pub fn iter(self) -> impl DoubleEndedIterator<Item = ImportItem<'a>> {
		self.0.children().iter().filter_map(|c| match c.kind() {
			SyntaxKind::RenamedImportItem	=> c.cast().map(ImportItem::Renamed),
			SyntaxKind::ImportItemPath		=> c.cast().map(ImportItem::Simple),
			_								=> Option::None,
		})
	}
}

node! { struct ImportItemPath }

impl<'a> ImportItemPath<'a> {
	pub fn iter(self) -> impl DoubleEndedIterator<Item = Ident<'a>> {
		self.0.children().iter().filter_map(|c| c.cast())
	}

	pub fn name(self) -> Ident<'a> { self.0.cast_last() }
}

#[derive(Debug, Copy, Clone, Hash)]
pub enum ImportItem<'a> {
	Simple(ImportItemPath<'a>),
	Renamed(RenamedImportItem<'a>),
}

impl<'a> ImportItem<'a> {
	pub fn path(self) -> ImportItemPath<'a> {
		match self {
			Self::Simple(p)		=> p,
			Self::Renamed(r)	=> r.path(),
		}
	}

	pub fn original_name(self) -> Ident<'a> {
		match self {
			Self::Simple(p)		=> p.name(),
			Self::Renamed(r)	=> r.original_name(),
		}
	}

	pub fn bound_name(self) -> Ident<'a> {
		match self {
			Self::Simple(p)		=> p.name(),
			Self::Renamed(r)	=> r.new_name(),
		}
	}
}

node! { struct RenamedImportItem }

impl<'a> RenamedImportItem<'a> {
	pub fn path(self) -> ImportItemPath<'a> { self.0.cast_first() }
	pub fn original_name(self) -> Ident<'a> { self.path().name() }
	pub fn new_name(self) -> Ident<'a> { self.0.cast_last() }
}

node! { struct ModuleInclude }

impl<'a> ModuleInclude<'a> {
	pub fn source(self) -> Expr<'a> { self.0.cast_last() }
}

node! { struct LoopBreak }
node! { struct LoopContinue }

node! { struct FuncReturn }

impl<'a> FuncReturn<'a> {
	pub fn body(self) -> Option<Expr<'a>> { self.0.try_cast_last() }
}
