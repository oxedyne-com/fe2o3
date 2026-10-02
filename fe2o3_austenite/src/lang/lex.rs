//! The one markup lexer every source scanner reads through, as a view over the concrete syntax tree of
//! [`crate::syntax`]: the port of `typst-syntax` 0.15.1's lexer and parser. A text is parsed once, and every
//! answer -- what a character is, where a line stands, what a group holds, where a conditional ends -- is read
//! from that tree, so the scanners keep no second reading of what is a comment, a body or the top level.
//! A `[` opens content only where code opened it (`#name[`, `#[`, a call's trailing argument, a reference's
//! supplement); in prose it is text, balanced against a later `]` within its own markup scope. A `"` is a
//! character in markup and a string only in code or maths. An automatic link is one token, so the `//` or
//! `/*` in it opens no comment. Block comments nest. Raw text opens with one backtick or three or more and
//! closes on a run as long, across lines. A `#!` opening the file is a comment to its line's end. A `$`
//! whose maths never closes is read as a character, so the error stays at it and the markup after it is read
//! as usual, where Typst's parser takes the rest of the source into the equation: the text is parsed again
//! with each such `$` set aside, until none is left.

use crate::syntax::FileId;
use crate::syntax::SyntaxKind;
use crate::syntax::SyntaxNode;
use crate::syntax::parser;

/// What a character is, as Typst reads it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Tok {
	Text,		// markup text, a bracket in prose included, and a `$` whose maths never closes
	Code,		// code: a name, a number, an operator, a space
	Math,		// an equation's own text
	Str,		// a string in code or maths, its quotes included
	Comment,	// a line or block comment, its delimiters included
	Raw,		// raw text, its backticks included
	Link,		// an automatic link
	Escape,		// an escape or a forced line break, its backslash included
	Label,		// `<name>`
	Ref,		// `@name`, and a supplement's opening `[`
	Hash,		// the `#` that embeds an expression
	Open,		// a delimiter code opens: `(`, `{`, a content block's `[`, an equation's `$`
	Close,		// the delimiter that closes one
}

/// Where a line stands, decided at its first token, as Typst's parser and realiser read it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Place {
	Top,		// the file's own markup, with no list item, heading, strong or emphasis open
	Block,		// a bare content block `#[`, or a run of them, opened where no scope was: joined into the file's markup
	Contained,	// in a list item, a heading, strong or emphasis of that markup: a container, which Typst sets apart
	Content,	// in content code opened: a call's argument, a reference's supplement, a conditional's or loop's body, a value
}

/// The keyword that heads a loop or a conditional.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kw {
	If,
	While,
	For,
}

/// How the text of a node is read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
	Markup,
	Code,
	Math,
}

const NONE: usize = usize::MAX;	// no node

/// One node of the tree, flattened: the leaves and the nodes above them, in source order.
#[derive(Clone, Copy, Debug)]
struct Node {
	kind:	SyntaxKind,
	start:	usize,
	end:	usize,
	parent:	usize,		// `NONE` for the root
	child:	usize,		// the first child; `NONE` for a leaf
	next:	usize,		// the next sibling
	prev:	usize,		// the previous sibling
	mode:	Mode,		// of a leaf, how it is read; of an inner node, how its children are
	bare:	bool,		// a content block opened directly after a `#`
	leaf:	usize,		// the leaf's place in `leaves`; `NONE` for an inner node
}

/// A leaf's class, and what is open before it.
#[derive(Clone, Copy, Debug)]
struct Leaf {
	node:	usize,
	tok:	Tok,
	groups:	u32,		// groups code opened and not yet closed, before this leaf
	eqs:	u32,		// equations open before it
	shut:	bool,		// false for a string, raw text or block comment the text ends inside
}

/// A text read once. A file's markup is read with [`Lexer::markup`], a line or fragment as Typst reads it
/// with [`Lexer::markup_exact`], and the inside of a call's argument list with [`Lexer::code`].
#[derive(Clone, Debug)]
pub(crate) struct Lexer {
	text:		String,
	nodes:		Vec<Node>,
	leaves:		Vec<Leaf>,
	lone:		Vec<usize>,		// sorted: the source bytes of each `$` whose maths never closes
	groups:		u32,			// open at the end of the text
	eqs:		u32,
}

/// The tree under construction.
struct Build {
	nodes:		Vec<Node>,
	leaves:		Vec<Leaf>,
	pos:		usize,
	open:		Vec<usize>,						// the group nodes whose brackets are open, outermost first
	eqs:		u32,
	last:		Option<(SyntaxKind, usize)>,	// the last leaf made: its kind and where it ended
	dollars:	Vec<usize>,						// the byte of each `$` the parser found unclosed
	deep:		u32,							// error text being read again
}

// Where the kinds of the syntax tree take a text apart.

/// Is `kind` one whose brackets code opens and closes as groups?
fn is_group(kind: SyntaxKind) -> bool {
	matches!(kind,
		SyntaxKind::CodeBlock | SyntaxKind::ContentBlock | SyntaxKind::Args | SyntaxKind::Parenthesized
		| SyntaxKind::Array | SyntaxKind::Dict | SyntaxKind::Params | SyntaxKind::Destructuring)
}

/// Does `kind` open a markup scope of its own in the markup around it: an item, a heading, strong or emphasis?
fn is_scope(kind: SyntaxKind) -> bool {
	matches!(kind,
		SyntaxKind::Strong | SyntaxKind::Emph | SyntaxKind::Heading | SyntaxKind::ListItem
		| SyntaxKind::EnumItem | SyntaxKind::TermItem)
}

/// Is `kind` a statement an `#` embeds, which runs to its line's end?
fn is_stmt(kind: SyntaxKind) -> bool {
	matches!(kind,
		SyntaxKind::LetBinding | SyntaxKind::SetRule | SyntaxKind::ShowRule | SyntaxKind::ModuleImport
		| SyntaxKind::ModuleInclude | SyntaxKind::FuncReturn)
}

/// How the children of a node of `kind` are read, `mode` being how the node itself is.
fn child_mode(kind: SyntaxKind, mode: Mode) -> Mode {
	match kind {
		SyntaxKind::Markup | SyntaxKind::Heading | SyntaxKind::Strong | SyntaxKind::Emph
		| SyntaxKind::ListItem | SyntaxKind::EnumItem | SyntaxKind::TermItem | SyntaxKind::Ref
		| SyntaxKind::Raw
			=> Mode::Markup,
		SyntaxKind::Math | SyntaxKind::Equation | SyntaxKind::MathDelimited | SyntaxKind::MathCall
		| SyntaxKind::MathArgs | SyntaxKind::MathAttach | SyntaxKind::MathFrac | SyntaxKind::MathRoot
			=> Mode::Math,
		SyntaxKind::Code | SyntaxKind::CodeBlock | SyntaxKind::ContentBlock | SyntaxKind::Parenthesized
		| SyntaxKind::Array | SyntaxKind::Dict | SyntaxKind::Keyed | SyntaxKind::Unary | SyntaxKind::Binary
		| SyntaxKind::FieldAccess | SyntaxKind::FuncCall | SyntaxKind::Args | SyntaxKind::Closure
		| SyntaxKind::Params | SyntaxKind::LetBinding | SyntaxKind::SetRule | SyntaxKind::ShowRule
		| SyntaxKind::Contextual | SyntaxKind::Conditional | SyntaxKind::WhileLoop | SyntaxKind::ForLoop
		| SyntaxKind::ModuleImport | SyntaxKind::ImportItems | SyntaxKind::ImportItemPath
		| SyntaxKind::RenamedImportItem | SyntaxKind::ModuleInclude | SyntaxKind::LoopBreak
		| SyntaxKind::LoopContinue | SyntaxKind::FuncReturn | SyntaxKind::Destructuring
		| SyntaxKind::DestructAssignment
			=> Mode::Code,
		_	=> mode,
	}
}

/// Does a block comment's text close, its nesting balanced?
fn comment_closed(text: &str) -> bool {
	let b = text.as_bytes();
	let (mut i, mut depth) = (0usize, 0u32);
	while i + 1 < b.len() {
		match (b[i], b[i + 1]) {
			(b'/', b'*')	=> { depth = depth.saturating_add(1); i += 2; },
			(b'*', b'/')	=> { depth = depth.saturating_sub(1); i += 2; },
			_				=> i += 1,
		}
	}
	depth == 0
}

impl Build {
	fn new() -> Self {
		Build {
			nodes:		Vec::new(),
			leaves:		Vec::new(),
			pos:		0,
			open:		Vec::new(),
			eqs:		0,
			last:		None,
			dollars:	Vec::new(),
			deep:		0,
		}
	}

	/// An inner node, opening at the current byte; its end is set when its children are done.
	fn inner(&mut self, kind: SyntaxKind, parent: usize, mode: Mode) -> usize {
		let bare = kind == SyntaxKind::ContentBlock && self.last == Some((SyntaxKind::Hash, self.pos));
		self.nodes.push(Node { kind, start: self.pos, end: self.pos, parent, child: NONE, next: NONE, prev: NONE, mode, bare, leaf: NONE });
		self.nodes.len() - 1
	}

	/// A leaf of `len` bytes, read as `tok`.
	fn leaf(&mut self, kind: SyntaxKind, len: usize, tok: Tok, parent: usize, mode: Mode, shut: bool) -> usize {
		let idx = self.nodes.len();
		self.nodes.push(Node {
			kind, start: self.pos, end: self.pos + len, parent, child: NONE, next: NONE, prev: NONE, mode,
			bare: false, leaf: self.leaves.len(),
		});
		self.leaves.push(Leaf { node: idx, tok, groups: self.open.len() as u32, eqs: self.eqs, shut });
		self.pos += len;
		self.last = Some((kind, self.pos));
		idx
	}

	/// Reads the children `kids` of the node `parent`, of `pkind`, whose children are read as `mode`.
	fn kids(&mut self, kids: &[SyntaxNode], parent: usize, pkind: SyntaxKind, mode: Mode) {
		let mut last	= NONE;
		let mut k1: Option<SyntaxKind>	= None;	// the kind of the sibling before
		let mut k2: Option<SyntaxKind>	= None;	// and the one before that
		let mut first	= true;
		for c in kids {
			let kind = c.kind();
			let idx = if c.is_inner() {
				let cmode	= child_mode(kind, mode);
				let idx		= self.inner(kind, parent, cmode);
				self.kids(c.children(), idx, kind, cmode);
				self.nodes[idx].end = self.pos;
				idx
			} else if c.len() == 0 {
				continue;	// a zero-width error: nothing of the text
			} else if kind == SyntaxKind::Error {
				self.error(c, parent, mode, k1)
			} else {
				self.token(c, parent, pkind, mode, first, k1, k2)
			};
			if last == NONE {
				self.nodes[parent].child = idx;
			} else {
				self.nodes[last].next	= idx;
				self.nodes[idx].prev	= last;
			}
			last	= idx;
			k2		= k1;
			k1		= Some(kind);
			first	= false;
		}
	}

	/// A leaf that is no error.
	fn token(
		&mut self,
		c:		&SyntaxNode,
		parent:	usize,
		pkind:	SyntaxKind,
		mode:	Mode,
		first:	bool,
		k1:		Option<SyntaxKind>,
		k2:		Option<SyntaxKind>,
	)
		-> usize
	{
		let kind	= c.kind();
		let len		= c.len();
		let gp		= self.nodes[parent].parent;
		let gkind	= if gp == NONE { None } else { Some(self.nodes[gp].kind) };
		let group	= is_group(pkind);
		// The atom an `#` embeds, which follows it directly.
		let after_hash	= k1 == Some(SyntaxKind::Hash) && !matches!(kind, SyntaxKind::Space | SyntaxKind::Parbreak);
		let (mut opens, mut closes, mut eq, mut shut) = (false, false, 0i8, true);
		let tok = match kind {
			SyntaxKind::Shebang | SyntaxKind::LineComment	=> Tok::Comment,
			SyntaxKind::BlockComment	=> { shut = comment_closed(c.text()); Tok::Comment },
			SyntaxKind::Hash			=> Tok::Hash,
			SyntaxKind::Str				=> Tok::Str,
			SyntaxKind::Escape | SyntaxKind::Linebreak	=> Tok::Escape,
			SyntaxKind::Link			=> Tok::Link,
			SyntaxKind::Label			=> Tok::Label,
			SyntaxKind::RefMarker		=> Tok::Ref,
			SyntaxKind::Dollar if pkind == SyntaxKind::Equation	=> {
				if first {
					eq = 1;
					Tok::Open
				} else {
					eq = -1;
					Tok::Close
				}
			},
			SyntaxKind::LeftParen | SyntaxKind::LeftBrace | SyntaxKind::LeftBracket if group	=> {
				opens = true;
				// A reference's supplement is a content block, its opening bracket part of the reference.
				if kind == SyntaxKind::LeftBracket && pkind == SyntaxKind::ContentBlock && gkind == Some(SyntaxKind::Ref) {
					Tok::Ref
				} else {
					Tok::Open
				}
			},
			SyntaxKind::RightParen | SyntaxKind::RightBrace | SyntaxKind::RightBracket if group	=> {
				closes = true;
				Tok::Close
			},
			_ if pkind == SyntaxKind::Raw	=> Tok::Raw,
			_ if after_hash					=> Tok::Code,
			_	=> match mode {
				Mode::Code		=> Tok::Code,
				Mode::Markup	=> if kind == SyntaxKind::Semicolon { Tok::Code } else { Tok::Text },
				// A `;` straight after an embedded expression ends it, as it ends no maths argument.
				Mode::Math		=> if kind == SyntaxKind::Semicolon && k2 == Some(SyntaxKind::Hash) { Tok::Code } else { Tok::Math },
			},
		};
		let at = self.leaf(kind, len, tok, parent, mode, shut);
		self.shift(opens, closes, eq, parent);
		at
	}

	/// Moves the counts of what is open past a leaf that opens or closes a group or an equation.
	fn shift(&mut self, opens: bool, closes: bool, eq: i8, parent: usize) {
		if opens {
			self.open.push(parent);
		}
		// A closer ends its own group and any inside it that never closed, as the parser leaves them.
		if closes && self.open.contains(&parent) {
			while let Some(g) = self.open.pop() {
				if g == parent {
					break;
				}
			}
		}
		if eq > 0 {
			self.eqs = self.eqs.saturating_add(1);
		} else if eq < 0 {
			self.eqs = self.eqs.saturating_sub(1);
		}
	}

	/// A leaf the parser could not read. Most are one token, by what the message says it is; one that is
	/// the leftover of a construct is read again as code, so the strings and comments in it are still such.
	fn error(&mut self, c: &SyntaxNode, parent: usize, mode: Mode, k1: Option<SyntaxKind>) -> usize {
		let text	= c.text();
		let msg		= c.error_info().map(|e| e.message.as_str()).unwrap_or("");
		let len		= c.len();
		let (mut opens, mut eq, mut shut) = (false, 0i8, true);
		let tok = if msg.starts_with("unclosed delimiter") {
			match text {
				"$"					=> {
					eq = 1;
					self.dollars.push(self.pos);
					Some(Tok::Open)
				},
				"(" | "{" | "["		=> { opens = true; Some(Tok::Open) },
				_					=> None,
			}
		} else if msg.starts_with("unclosed raw text") {
			shut = false;
			Some(Tok::Raw)
		} else if msg.starts_with("unclosed string") {
			shut = false;
			Some(Tok::Str)
		} else if msg.starts_with("unexpected end of block comment") {
			Some(Tok::Text)
		} else if msg.starts_with("unclosed label") {
			Some(Tok::Label)
		} else if msg.contains("Unicode") {
			Some(Tok::Escape)
		} else if msg.starts_with("automatic links") {
			Some(Tok::Link)
		} else {
			None
		};
		if let Some(tok) = tok {
			let at = self.leaf(SyntaxKind::Error, len, tok, parent, mode, shut);
			self.shift(opens, false, eq, parent);
			return at;
		}
		// What the parser left over in code may hold a string, a comment, a group: read it again as code.
		if mode == Mode::Code && text.chars().nth(1).is_some() && self.deep < 3 {
			let root = parser::parse_code(text, FileId::DETACHED);
			let same = matches!(root.children(), [only] if only.kind() == SyntaxKind::Error && only.text() == text);
			if !same {
				let idx = self.inner(SyntaxKind::Error, parent, Mode::Code);
				self.deep += 1;
				self.kids(root.children(), idx, SyntaxKind::Code, Mode::Code);
				self.deep -= 1;
				self.nodes[idx].end = self.pos;
				return idx;
			}
		}
		let tok = match mode {
			// What follows an `#` and is no expression is still code.
			_ if k1 == Some(SyntaxKind::Hash)	=> Tok::Code,
			Mode::Markup	=> Tok::Text,
			Mode::Code		=> Tok::Code,
			Mode::Math		=> Tok::Math,
		};
		self.leaf(SyntaxKind::Error, len, tok, parent, mode, true)
	}
}

/// Is `c` a line break, as Typst counts one?
pub(crate) fn is_newline(c: char) -> bool {
	matches!(c, '\n' | '\x0B' | '\x0C' | '\r' | '\u{0085}' | '\u{2028}' | '\u{2029}')
}

/// Can `c` start a Typst name?
pub(crate) fn is_id_start(c: char) -> bool {
	c.is_alphabetic() || c == '_'
}

/// Can `c` continue a Typst name?
pub(crate) fn is_id_continue(c: char) -> bool {
	c.is_alphanumeric() || c == '_' || c == '-'
}

impl Lexer {
	/// A file's markup, as Typst reads a `.typ` file, each `$` whose maths never closes read as a character.
	pub(crate) fn markup(src: &str) -> Self {
		Self::read(src, false, true)
	}

	/// A line or fragment of markup exactly as Typst reads it, an unclosed `$` opening an equation that
	/// takes the rest.
	#[cfg(test)]
	pub(crate) fn markup_exact(src: &str) -> Self {
		Self::read(src, false, false)
	}

	/// Code, as the inside of a call's argument list or a value reads.
	pub(crate) fn code(src: &str) -> Self {
		Self::read(src, true, false)
	}

	fn read(src: &str, code: bool, recover: bool) -> Self {
		let mut lone: Vec<usize> = Vec::new();
		let passes = if recover { src.matches('$').count() + 1 } else { 1 };
		let mut done = None;
		for _ in 0..passes {
			let b = Self::pass(src, code, &lone);
			// The innermost equation that never closes is the last, as each one that does not takes the rest.
			match (recover, b.dollars.last()) {
				(true, Some(&at))	=> lone.insert(lone.partition_point(|&p| p < at), at),
				_					=> { done = Some(b); break; },
			}
		}
		let b = match done {
			Some(b)	=> b,
			None	=> Self::pass(src, code, &lone),
		};
		Lexer { text: src.to_string(), nodes: b.nodes, leaves: b.leaves, lone, groups: b.open.len() as u32, eqs: b.eqs }
	}

	/// One reading, with each `$` of `lone` set aside as a comma, which no mode reads as more than text.
	fn pass(src: &str, code: bool, lone: &[usize]) -> Build {
		let mut bytes = src.as_bytes().to_vec();
		for &at in lone {
			bytes[at] = b',';
		}
		let text = String::from_utf8(bytes).unwrap_or_else(|_| src.to_string());
		let (root, mode) = if code {
			(parser::parse_code(&text, FileId::DETACHED), Mode::Code)
		} else {
			(parser::parse(&text, FileId::DETACHED), Mode::Markup)
		};
		let mut b = Build::new();
		let idx = b.inner(root.kind(), NONE, mode);
		b.kids(root.children(), idx, root.kind(), mode);
		b.nodes[idx].end = b.pos;
		b
	}

	/// The source bytes of each `$` whose maths never closes, in order. Typst refuses the file there, with the
	/// rest of the source taken into the equation; the scan reads each of these as a character instead, so the
	/// error stays at the `$` and the markup after it is read.
	pub(crate) fn lone(&self) -> &[usize] {
		&self.lone
	}

	/// The leaf holding byte `p`, or the last one starting before it.
	fn leaf_at(&self, p: usize) -> Option<usize> {
		self.leaves.partition_point(|l| self.nodes[l.node].start <= p).checked_sub(1)
	}

	/// What stands around byte `p`: the leaf it falls inside, when it is no leaf's first byte, and the nodes
	/// that began before it and end after it, innermost first.
	fn around(&self, p: usize) -> (Option<usize>, Vec<usize>) {
		let Some(li) = self.leaf_at(p) else { return (None, Vec::new()); };
		let n = self.nodes[self.leaves[li].node];
		if p >= n.end {
			return (None, Vec::new());
		}
		let inside	= if n.start < p { Some(li) } else { None };
		let mut out	= Vec::new();
		let mut a	= n.parent;
		while a != NONE {
			let m = self.nodes[a];
			if m.start < p && m.end > p {
				out.push(a);
			}
			a = m.parent;
		}
		(inside, out)
	}

	/// Does the text stand in markup at the position `chain` and `inside` describe: neither in a string, a
	/// comment, raw text, a code group, an equation, nor anywhere but the file's own markup or a content block?
	fn in_markup(&self, inside: Option<usize>, chain: &[usize]) -> bool {
		if let Some(li) = inside {
			if matches!(self.leaves[li].tok, Tok::Str | Tok::Comment | Tok::Raw) {
				return false;
			}
		}
		let own = chain.iter().map(|&a| self.nodes[a].kind).find(|&k| k != SyntaxKind::Markup && !is_scope(k));
		own.map_or(true, |k| k == SyntaxKind::ContentBlock)
	}

	/// The scopes and frames of `chain`, outermost first, as a line standing in them is placed.
	fn place_of(&self, chain: &[usize]) -> Place {
		let mut place = Place::Top;
		for &a in chain.iter().rev() {
			let n = &self.nodes[a];
			match n.kind {
				SyntaxKind::Markup	=> {},
				k if is_scope(k)	=> return Place::Contained,
				SyntaxKind::ContentBlock if n.bare	=> place = Place::Block,
				_	=> return Place::Content,
			}
		}
		place
	}

	/// The first leaf of the line at byte `p` that is a token: past its spaces and any block comment that
	/// closes on it. `None` when the line holds none (spaces, a comment or its break alone).
	fn first_token(&self, p: usize) -> Option<usize> {
		let line_end = self.text[p..].find(is_newline).map_or(self.text.len(), |k| p + k);
		let mut li = self.leaf_at(p)?;
		loop {
			let n = self.nodes[self.leaves.get(li)?.node];
			if n.start >= line_end {
				return None;
			}
			match n.kind {
				SyntaxKind::Space | SyntaxKind::Parbreak	=> {},
				SyntaxKind::LineComment | SyntaxKind::Shebang	=> return None,
				SyntaxKind::BlockComment	=> if n.end > line_end { return None; },
				_	=> return Some(n.start.max(p)),
			}
			li += 1;
		}
	}

	/// The scopes still open where a line with no token stands, which ends none: those that held the last token
	/// before it. A heading ended with its line, and strong or emphasis that closed before it is closed; one
	/// that has not ended runs on through the trailing space the parser gives it.
	fn open_chain(&self, p: usize) -> Vec<usize> {
		let Some(mut li) = self.leaf_at(p) else { return Vec::new(); };
		// The last token ending before the line.
		loop {
			let n = self.nodes[self.leaves[li].node];
			let blank = matches!(n.kind, SyntaxKind::Space | SyntaxKind::Parbreak | SyntaxKind::LineComment
				| SyntaxKind::BlockComment | SyntaxKind::Shebang);
			if n.end <= p && !blank {
				break;
			}
			match li.checked_sub(1) {
				Some(k)	=> li = k,
				None	=> return Vec::new(),
			}
		}
		let mut out	= Vec::new();
		let mut a	= self.nodes[self.leaves[li].node].parent;
		while a != NONE {
			let m = self.nodes[a];
			let keep = match m.kind {
				SyntaxKind::Heading		=> false,
				SyntaxKind::Strong | SyntaxKind::Emph	=> m.end > p,
				SyntaxKind::Markup | SyntaxKind::ListItem | SyntaxKind::EnumItem | SyntaxKind::TermItem	=> true,
				_	=> m.end > p,
			};
			if keep {
				out.push(a);
			} else if out.last().is_some_and(|&l| self.nodes[l].kind == SyntaxKind::Markup && self.nodes[l].parent == a) {
				// The markup a scope that has ended held goes with it.
				out.pop();
			}
			a = m.parent;
		}
		out
	}

	/// The chain a line at byte `p` is placed by, and whether it opens in markup at all: the strict ancestors of
	/// its first token, or, for a line with none, the scopes left open before it.
	fn line_chain(&self, p: usize) -> Option<Vec<usize>> {
		let (inside, here) = self.around(p);
		if !self.in_markup(inside, &here) {
			return None;
		}
		Some(match self.first_token(p) {
			Some(t)	=> self.around(t).1,
			None	=> self.open_chain(p),
		})
	}

	/// Where the line starting at byte `p` stands, or `None` when it opens outside markup: in a string, a
	/// comment, raw text, an equation, a code group or an unfinished expression. The spaces that open a line
	/// change no scope, so the place is decided at its first token, which ends the items it does not indent past.
	pub(crate) fn place_at(&self, p: usize) -> Option<Place> {
		self.line_chain(p).map(|chain| self.place_of(&chain))
	}

	/// Is the line starting at byte `p` standing directly in the innermost markup, whatever opened it, with no
	/// list item, heading, strong or emphasis of that markup open around it?
	pub(crate) fn bare_line_at(&self, p: usize) -> bool {
		let Some(chain) = self.line_chain(p) else { return false; };
		match chain.first() {
			None		=> true,
			// A line in a content block whose markup has not begun, or that opens with the block's own closer,
			// stands in the block, bare.
			Some(&a) if self.nodes[a].kind == SyntaxKind::ContentBlock	=> true,
			Some(&a)	=> {
				self.nodes[a].kind == SyntaxKind::Markup
					&& chain.get(1).map_or(true, |&b| self.nodes[b].kind == SyntaxKind::ContentBlock)
			},
		}
	}

	/// How many groups code opened are open at byte `p`: `(`, `{`, a content block. Prose brackets, strings,
	/// equations, comments and raw text are not counted.
	pub(crate) fn depth_at(&self, p: usize) -> usize {
		match self.leaf_at(p) {
			Some(li) if p < self.nodes[self.leaves[li].node].end	=> self.leaves[li].groups as usize,
			_	=> self.groups as usize,
		}
	}

	/// The number of groups open at byte `p` when the text stands there in markup -- the file's own, or a
	/// content block's -- and `None` when it stands in code, a string, an equation, a comment or raw text.
	pub(crate) fn markup_level_at(&self, p: usize) -> Option<usize> {
		let (inside, chain) = self.around(p);
		self.in_markup(inside, &chain).then(|| self.depth_at(p))
	}

	/// How many frames are open at byte `p`: groups, equations, and the string, comment or raw text it falls
	/// inside.
	pub(crate) fn open_at(&self, p: usize) -> usize {
		let end = |this: &Self| -> usize {
			let shut = this.leaves.last().map_or(true, |l| l.shut);
			this.groups as usize + this.eqs as usize + usize::from(!shut)
		};
		let Some(li) = self.leaf_at(p) else { return end(self); };
		let l = self.leaves[li];
		let n = self.nodes[l.node];
		if p >= n.end {
			return end(self);
		}
		let mut open = l.groups as usize + l.eqs as usize;
		let in_raw	= l.tok == Tok::Raw
			&& (n.start < p || (self.nodes[n.parent].kind == SyntaxKind::Raw && self.nodes[n.parent].start < p));
		if (n.start < p && matches!(l.tok, Tok::Str | Tok::Comment)) || in_raw {
			open += 1;
		}
		open
	}

	/// Is anything open at the end of the text?
	#[cfg(test)]
	pub(crate) fn is_open(&self) -> bool {
		self.open_at(self.text.len()) > 0
	}

	/// What each byte of the text is: every byte of a character takes the character's.
	pub(crate) fn byte_toks(&self) -> Vec<Tok> {
		let mut out = vec![Tok::Text; self.text.len()];
		for l in &self.leaves {
			let n = &self.nodes[l.node];
			for t in &mut out[n.start..n.end] {
				*t = l.tok;
			}
		}
		for &at in &self.lone {
			out[at] = Tok::Text;
		}
		out
	}

	/// Each character of the text, with the byte it starts at.
	pub(crate) fn tokens(&self) -> Vec<(usize, char, Tok)> {
		let toks = self.byte_toks();
		self.text.char_indices().map(|(at, c)| (at, c, toks[at])).collect()
	}

	/// The text of a leaf.
	fn leaf_text(&self, li: usize) -> &str {
		let n = &self.nodes[self.leaves[li].node];
		&self.text[n.start..n.end]
	}

	/// The text with every character `blank` names turned into a space and every line break kept, so each byte
	/// offset and line of the result is the text's own.
	fn blanked(&self, blank: impl Fn(Tok) -> bool) -> String {
		let mut out = String::with_capacity(self.text.len());
		for (_, c, tok) in self.tokens() {
			if blank(tok) && c != '\n' && c != '\r' {
				for _ in 0..c.len_utf8() {
					out.push(' ');
				}
			} else {
				out.push(c);
			}
		}
		out
	}

	/// The text with every character a comment or raw text holds blanked to spaces and every line break kept,
	/// so each byte offset and line of the result is the text's own.
	pub(crate) fn live_text(&self) -> String {
		self.blanked(|t| matches!(t, Tok::Comment | Tok::Raw))
	}

	/// Each line of `src`, the text read, that opens in markup, with the byte it starts at and where it stands.
	pub(crate) fn lines<'a>(&self, src: &'a str) -> Vec<(usize, &'a str, Place)> {
		let mut out		= Vec::new();
		let mut offset	= 0usize;
		for raw in src.split_inclusive('\n') {
			if let Some(place) = self.place_at(offset) {
				out.push((offset, raw, place));
			}
			offset = offset.saturating_add(raw.len());
		}
		out
	}
}

/// The source bytes of each `$` in `src` whose maths never closes, in order.
#[cfg(test)]
pub(crate) fn lone_dollars(src: &str) -> Vec<usize> {
	if !src.contains('$') {
		return Vec::new();
	}
	Lexer::markup(src).lone
}

/// Each character of `src` read as a file's markup, with the byte it starts at.
pub(crate) fn tokens(src: &str) -> Vec<(usize, char, Tok)> {
	Lexer::markup(src).tokens()
}

/// Which characters of `src` a comment or raw text holds, delimiters included, one entry per character.
pub(crate) fn literal_chars(src: &str) -> Vec<bool> {
	tokens(src).into_iter().map(|(_, _, t)| matches!(t, Tok::Comment | Tok::Raw)).collect()
}

/// What each byte of `src` is, read as a file's markup: every byte of a character takes the character's.
#[cfg(test)]
pub(crate) fn byte_toks(src: &str) -> Vec<Tok> {
	Lexer::markup(src).byte_toks()
}

/// `src` with every character a comment holds blanked to spaces and every line break kept, so each byte
/// offset and line of the result is the source's own.
pub(crate) fn uncommented(src: &str) -> String {
	Lexer::markup(src).blanked(|t| t == Tok::Comment)
}

/// `src` with every character a comment or raw text holds blanked to spaces and every line break kept, so
/// each byte offset and line of the result is the source's own. A scan that finds a declaration, an
/// `#include` or a field in this text finds none a comment holds or a raw block shows.
pub(crate) fn live_text(src: &str) -> String {
	Lexer::markup(src).live_text()
}

/// Each line of `src` that opens in markup, with the byte it starts at and where it stands.
pub(crate) fn placed_lines(src: &str) -> Vec<(usize, &str, Place)> {
	Lexer::markup(src).lines(src)
}

/// Where the line starting at byte `at` stands, among `lines` from [`placed_lines`].
pub(crate) fn place_in(lines: &[(usize, &str, Place)], at: usize) -> Option<Place> {
	lines.binary_search_by_key(&at, |&(start, _, _)| start).ok().map(|k| lines[k].2)
}

/// The lines of `src` that stand at its own level, each with the byte it starts at: a line opening inside
/// a group, a content block, a string, an equation, a comment or raw text is none of them, and nor is a
/// line inside a list item, a heading, strong or emphasis.
pub(crate) fn top_level_lines(src: &str) -> Vec<(usize, &str)> {
	placed_lines(src).into_iter().filter(|&(_, _, p)| p == Place::Top).map(|(at, line, _)| (at, line)).collect()
}

/// Is anything left open at the end of `text`, read as Typst reads a fragment of markup: a group, a string,
/// an equation, a comment or raw text?
#[cfg(test)]
pub(crate) fn open_after(text: &str) -> bool {
	Lexer::markup_exact(text).is_open()
}

// Code fragments. A group's end, a value's end and the like are asked of text that goes on past the answer,
// so the text is read in growing prefixes: a prefix that holds the answer holds it as the whole would.

/// `text` cut at the first char boundary at or after `n` bytes.
fn prefix(text: &str, n: usize) -> &str {
	let mut end = n.min(text.len());
	while !text.is_char_boundary(end) {
		end += 1;
	}
	&text[..end]
}

/// The first answer `ask` finds, reading `text` as code in prefixes of growing size; the whole, last, is
/// asked with `full` set.
fn in_prefixes<T>(text: &str, ask: impl Fn(&Lexer, bool) -> Option<T>) -> Option<T> {
	let mut n = 512usize;
	loop {
		let head	= prefix(text, n);
		let full	= head.len() >= text.len();
		if let Some(r) = ask(&Lexer::code(head), full) {
			return Some(r);
		}
		if full {
			return None;
		}
		n = n.saturating_mul(4);
	}
}

impl Lexer {
	/// The byte just past the group the text opens with, a `(`, `[` or `{`; `None` when it never closes.
	fn group_close(&self) -> Option<usize> {
		let first = self.leaves.first()?;
		if first.tok != Tok::Open || self.nodes[first.node].start != 0 {
			return None;
		}
		let group = self.nodes[first.node].parent;
		(1..self.leaves.len()).find(|&li| self.closes_group(li) && self.nodes[self.leaves[li].node].parent == group)
			.map(|li| self.nodes[self.leaves[li].node].end)
	}

	/// Is the leaf `li` the `)`, `]` or `}` that closes a group?
	fn closes_group(&self, li: usize) -> bool {
		self.leaves[li].tok == Tok::Close
			&& matches!(self.nodes[self.leaves[li].node].kind, SyntaxKind::RightParen | SyntaxKind::RightBrace | SyntaxKind::RightBracket)
	}

	/// The byte of the first leaf that is a comma or a closer standing at the text's own level: in no group,
	/// equation, string, comment or raw text.
	fn comma(&self) -> Option<usize> {
		(0..self.leaves.len()).find(|&li| {
			let l = self.leaves[li];
			l.groups == 0 && l.eqs == 0 && l.tok == Tok::Code && matches!(self.leaf_text(li), "," | ")" | "]" | "}")
		}).map(|li| self.nodes[self.leaves[li].node].start)
	}
}

/// The byte just past the group opening at byte `at` of `src` -- a `(`, `[` or `{` read as code opens one
/// -- or `None` when it never closes.
pub(crate) fn group_end(src: &str, at: usize) -> Option<usize> {
	if !matches!(src[at..].chars().next(), Some('(') | Some('[') | Some('{')) {
		return None;
	}
	in_prefixes(&src[at..], |lx, _| lx.group_close()).map(|end| at + end)
}

/// As [`group_end`] over a slice of chars, at the char `at` and in chars: the index just past the closer.
pub(crate) fn group_end_chars(chars: &[char], at: usize) -> Option<usize> {
	if !matches!(chars.get(at), Some('(') | Some('[') | Some('{')) {
		return None;
	}
	let mut n = 128usize;
	loop {
		let upto	= (at + n).min(chars.len());
		let head: String = chars[at..upto].iter().collect();
		if let Some(end) = Lexer::code(&head).group_close() {
			return Some(at + head[..end].chars().count());
		}
		if upto >= chars.len() {
			return None;
		}
		n = n.saturating_mul(4);
	}
}

/// Where a field's value in an argument list or a dictionary ends, for `src` read as code from the value's
/// start: the byte of the first comma at its own level, or of the closer of the group it stands in, or the
/// length of `src` when there is neither.
pub(crate) fn top_comma(src: &str) -> usize {
	in_prefixes(src, |lx, full| lx.comma().or_else(|| full.then_some(src.len()))).unwrap_or(src.len())
}

/// The byte of the first colon at the own level of `src` read as code, or `None` when there is none.
pub(crate) fn top_colon(src: &str) -> Option<usize> {
	let lx = Lexer::code(src);
	(0..lx.leaves.len()).find(|&li| {
		let l = lx.leaves[li];
		l.groups == 0 && l.eqs == 0 && l.tok == Tok::Code && lx.leaf_text(li) == ":"
	}).map(|li| lx.nodes[lx.leaves[li].node].start)
}

/// Each group a `(` opens at the top level of `src` read as code, by the byte of its `(` and the byte just
/// past its `)`. A group that never closes is not listed.
pub(crate) fn top_parens(src: &str) -> Vec<(usize, usize)> {
	let lx		= Lexer::code(src);
	let mut out	= Vec::new();
	let mut open: Option<usize>	= None;
	let mut group: Option<usize> = None;	// the group node the open `(` belongs to
	for li in 0..lx.leaves.len() {
		let l = lx.leaves[li];
		let n = lx.nodes[l.node];
		if l.groups == 0 && l.eqs == 0 && l.tok == Tok::Open && lx.leaf_text(li) == "(" {
			open	= Some(n.start);
			group	= Some(n.parent);
		}
		if lx.closes_group(li) && Some(n.parent) == group {
			if let Some(o) = open.take() {
				out.push((o, n.end));
			}
			group = None;
		}
	}
	out
}

/// One argument of a call's list, or one entry of an array or a dictionary, as Typst's parser reads it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Arg {
	pub(crate) key:		Option<String>,	// a named argument's name; `None` when positional
	pub(crate) value:	String,			// the value's text, its comments dropped, trimmed
}

/// What a leaf of an argument list's text leaves of what it read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kept {
	Text,		// the characters as written
	Space,		// a comment in code or maths: one space, so the tokens either side of it stay apart
	Nothing,	// a comment in markup, where Typst sets nothing for it
}

impl Lexer {
	/// What the leaf `li` leaves of an argument list: a comment is trivia, so a reader that keeps the text of
	/// an argument keeps none of a comment's.
	fn kept(&self, li: usize) -> Kept {
		match self.leaves[li].tok {
			Tok::Comment if self.nodes[self.leaves[li].node].mode == Mode::Markup	=> Kept::Nothing,
			Tok::Comment	=> Kept::Space,
			_				=> Kept::Text,
		}
	}

	/// Is the leaf `li` a comma at the list's own level?
	fn parts(&self, li: usize) -> bool {
		let l = self.leaves[li];
		l.groups == 0 && l.eqs == 0 && l.tok == Tok::Code && self.leaf_text(li) == ","
	}
}

/// How much of an argument's head has been read, to tell a name before its `:` from a value.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Head {
	Start,									// nothing yet but spaces and comments
	Name { name: String, spaced: bool },	// a name, and whether a space has followed it
	Value,									// anything else: a positional value, or the value after a `:`
}

/// The arguments of the list `inner`, the text inside a call's parentheses (or an array's or a
/// dictionary's), read as code the way Typst's parser reads them. An argument ends at a comma only at the
/// list's own level. A comment, at any depth, is trivia: it adds one space to the value in code and nothing
/// in markup, where Typst sets nothing for it, so a field commented out is no field. Strings, raw text and
/// groups are copied as written and close where the lexer closes them. An argument is named only when it
/// opens with a name followed, past any spaces or comments, by a `:` at the list's own level; a spread, a
/// content block and every other value are positional. An empty argument, as a trailing comma leaves, is
/// not listed.
pub(crate) fn args(inner: &str) -> Vec<Arg> {
	let lx					= Lexer::code(inner);
	let mut out: Vec<Arg>	= Vec::new();
	let mut head			= Head::Start;
	let mut key				= None;
	let mut value			= String::new();
	let flush = |key: &mut Option<String>, value: &mut String, out: &mut Vec<Arg>| {
		let v = value.trim();
		if key.is_some() || !v.is_empty() {
			out.push(Arg { key: key.take(), value: v.to_string() });
		}
	};
	for li in 0..lx.leaves.len() {
		if lx.parts(li) {
			flush(&mut key, &mut value, &mut out);
			head	= Head::Start;
			key		= None;
			value.clear();
			continue;
		}
		match lx.kept(li) {
			Kept::Space		=> { value.push(' '); continue; },
			Kept::Nothing	=> continue,
			Kept::Text		=> {},
		}
		let l		= lx.leaves[li];
		let piece	= lx.leaf_text(li);
		let top		= l.groups == 0 && l.eqs == 0;
		let blank	= piece.chars().all(|c| c.is_whitespace());
		if top && l.tok == Tok::Code {
			let word = piece.chars().next().is_some_and(is_id_start) && piece.chars().all(is_id_continue);
			let next = match std::mem::replace(&mut head, Head::Value) {
				Head::Start if blank					=> Head::Start,
				Head::Start if word						=> Head::Name { name: piece.to_string(), spaced: false },
				Head::Name { name, .. } if blank		=> Head::Name { name, spaced: true },
				Head::Name { mut name, spaced: false } if piece.chars().all(is_id_continue) => {
					name.push_str(piece);
					Head::Name { name, spaced: false }
				},
				Head::Name { name, .. } if piece == ":"	=> {
					// The name is the key, and the value starts after its colon.
					key = Some(name);
					value.clear();
					continue;
				},
				Head::Start | Head::Name { .. } | Head::Value	=> Head::Value,
			};
			head = next;
		} else if !blank {
			head = Head::Value;
		}
		value.push_str(piece);
	}
	flush(&mut key, &mut value, &mut out);
	out
}

/// The arguments of the list `inner` as the text each keeps, comments as [`args`] reads them, split at the
/// commas of the list's own level. A last argument of nothing but space is not listed.
pub(crate) fn split_args(inner: &str) -> Vec<String> {
	let lx					= Lexer::code(inner);
	let mut out: Vec<String>	= Vec::new();
	let mut cur				= String::new();
	for li in 0..lx.leaves.len() {
		if lx.parts(li) {
			out.push(std::mem::take(&mut cur));
			continue;
		}
		match lx.kept(li) {
			Kept::Text		=> cur.push_str(lx.leaf_text(li)),
			Kept::Space		=> cur.push(' '),
			Kept::Nothing	=> {},
		}
	}
	if !cur.trim().is_empty() {
		out.push(cur);
	}
	out
}

/// The value of the argument named `key` in `list`, the last one when it is named twice.
pub(crate) fn named<'a>(list: &'a [Arg], key: &str) -> Option<&'a str> {
	list.iter().rev().find(|a| a.key.as_deref() == Some(key)).map(|a| a.value.as_str())
}

/// A name `list` gives twice, which Typst refuses as a duplicate argument.
pub(crate) fn duplicate_key(list: &[Arg]) -> Option<&str> {
	for (k, a) in list.iter().enumerate() {
		if let Some(name) = a.key.as_deref() {
			if list[..k].iter().any(|b| b.key.as_deref() == Some(name)) {
				return Some(name);
			}
		}
	}
	None
}

/// A conditional or a loop embedded in markup -- an `#if` with its `else` arms, a `#for` or a `#while` --
/// with its extent and each arm's condition and body.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Flow {
	pub(crate) kw:		Kw,			// `If`, `For` or `While`
	pub(crate) start:	usize,		// the byte of its `#`
	pub(crate) end:		usize,		// the byte just past its last body, or the end of what it took
	pub(crate) arms:	Vec<Arm>,
	pub(crate) whole:	bool,		// every arm closed, and no call or field follows the last
	pub(crate) coded:	bool,		// it stands in code, a code block or a function's body, whose names this scan cannot see
	pub(crate) math:	bool,		// it stands in an equation
}

/// Which of a text's conditionals and loops [`flows_in`] lists.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Level {
	Own,	// those standing in the text's own markup
	Deep,	// those too in any content block or equation inside it: a cell's, a caption's, a note's, a call's argument
}

/// One arm of a [`Flow`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Arm {
	pub(crate) cond:	Option<(usize, usize)>,	// the condition's bytes, or a loop's head's; `None` for `else`
	pub(crate) body:	(usize, usize),			// the body's bytes, its delimiters included
	pub(crate) content:	bool,					// a `[...]` content block, not a `{...}` code block
}

/// Every conditional and loop standing in `src`'s own markup, in order. One in another's body, or in any
/// other group, is that group's, and is not listed.
pub(crate) fn flows(src: &str) -> Vec<Flow> {
	flows_in(src, Level::Own)
}

/// The conditionals and loops of `src` that `level` names, in order, each the outermost of its kind: one in
/// another's head or body is that statement's. At [`Level::Deep`] a flow in a content block or an equation is
/// listed wherever it stands, a call's argument list included, with its `coded` set where it stands in code
/// whose own bindings (a code block's `let`, a function's parameters) this scan does not read. A flow inside
/// a `#let`, `#set` or `#show` statement is that statement's, read when it is, and not listed.
pub(crate) fn flows_in(src: &str, level: Level) -> Vec<Flow> {
	Lexer::markup(src).flows(level)
}

/// A `#let` or `#import` embedded in markup, and the stretch of the text its names are bound over.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Binding {
	pub(crate) start:	usize,			// the byte of its `#`
	pub(crate) text:	String,			// the statement from its `#`, its comments dropped
	pub(crate) at:		usize,			// the byte just past it, where its names come into force
	pub(crate) until:	Option<usize>,	// the byte of the closer of the block it stands in; `None` at the text's own level
}

/// Every `#let` and `#import` embedded in `src`'s markup, at any depth, in source order. Typst binds a
/// name in the innermost content block `[...]` or code block `{...}` the statement stands in, a
/// conditional's branch included, from the statement's end to the block's closer; a heading, a list item,
/// strong or emphasis opens no scope. A code block's own `let`, written with no `#`, is code rather than
/// markup and is not listed.
pub(crate) fn bindings(src: &str) -> Vec<Binding> {
	Lexer::markup(src).bindings()
}

impl Lexer {
	fn kind(&self, a: usize) -> SyntaxKind {
		self.nodes[a].kind
	}

	/// The expression an `#` at the leaf node `h` embeds, if one follows it directly.
	fn embedded(&self, h: usize) -> Option<usize> {
		let n = &self.nodes[h];
		(n.kind == SyntaxKind::Hash && n.next != NONE && self.nodes[n.next].start == n.end).then_some(n.next)
	}

	/// The nodes beneath `a`, in order.
	fn kids(&self, a: usize) -> Vec<usize> {
		let mut out	= Vec::new();
		let mut c	= self.nodes[a].child;
		while c != NONE {
			out.push(c);
			c = self.nodes[c].next;
		}
		out
	}

	/// The conditionals and loops `level` names.
	fn flows(&self, level: Level) -> Vec<Flow> {
		let mut out = Vec::new();
		self.find_flows(0, level, true, false, false, &mut out);
		out
	}

	/// Walks the children of `a`: `own` while only markup and its scopes lie between the root and `a`, `stmt` inside
	/// an embedded statement, `coded` inside a code block or a closure.
	fn find_flows(&self, a: usize, level: Level, own: bool, stmt: bool, coded: bool, out: &mut Vec<Flow>) {
		let mode		= self.nodes[a].mode;
		let mut skip	= NONE;	// the expression a flow took, which holds no other
		for c in self.kids(a) {
			if c == skip {
				continue;
			}
			let k = self.kind(c);
			if let Some(e) = self.embedded(c) {
				let listed = match level {
					Level::Own	=> own,
					Level::Deep	=> !stmt && matches!(mode, Mode::Markup | Mode::Math),
				};
				if listed {
					if let Some(flow) = self.flow(c, e, coded, mode == Mode::Math) {
						out.push(flow);
						skip = e;
						continue;
					}
				}
			}
			if self.nodes[c].child == NONE {
				continue;
			}
			// An embedded statement is the statement's, whatever stands inside it.
			let embedded_stmt = is_stmt(k) && self.nodes[c].prev != NONE && self.kind(self.nodes[c].prev) == SyntaxKind::Hash;
			let keeps = k == SyntaxKind::Markup || is_scope(k);
			self.find_flows(
				c, level, own && keeps, stmt || embedded_stmt,
				coded || matches!(k, SyntaxKind::CodeBlock | SyntaxKind::Closure), out,
			);
		}
	}

	/// The flow the `#` at `h` opens over the expression `e`, if `e` is a conditional or a loop, or a call or a
	/// field taken from one.
	fn flow(&self, h: usize, e: usize, coded: bool, math: bool) -> Option<Flow> {
		let mut x		= e;
		let mut whole	= true;
		while matches!(self.kind(x), SyntaxKind::FuncCall | SyntaxKind::FieldAccess) {
			x = self.nodes[x].child;
			whole = false;
			if x == NONE {
				return None;
			}
		}
		let kw = match self.kind(x) {
			SyntaxKind::Conditional	=> Kw::If,
			SyntaxKind::WhileLoop	=> Kw::While,
			SyntaxKind::ForLoop		=> Kw::For,
			_						=> return None,
		};
		let mut arms	= Vec::new();
		let complete	= self.arms(x, &mut arms);
		let whole		= whole && complete && !arms.is_empty();
		let last		= arms.last().map_or(0, |a| a.body.1);
		Some(Flow {
			kw,
			start:	self.nodes[h].start,
			end:	if whole { last } else { self.nodes[e].end.max(last) },
			arms,
			whole,
			coded,
			math,
		})
	}

	/// Pushes the arms of the conditional or loop `x`: false when one is missing its body or leaves it open.
	fn arms(&self, x: usize, out: &mut Vec<Arm>) -> bool {
		let kids	= self.kids(x);
		let Some(&kw) = kids.first() else { return false; };
		let from	= self.nodes[kw].end;
		let block	= |a: usize| matches!(self.kind(a), SyntaxKind::ContentBlock | SyntaxKind::CodeBlock);
		let at_else	= kids.iter().position(|&a| self.kind(a) == SyntaxKind::Else);
		let head	= &kids[1..at_else.unwrap_or(kids.len())];
		let Some(&body) = head.iter().rev().find(|&&a| block(a)) else { return false; };
		if !self.closed(body) {
			return false;
		}
		out.push(Arm {
			cond:		Some((from, self.nodes[body].start)),
			body:		(self.nodes[body].start, self.nodes[body].end),
			content:	self.kind(body) == SyntaxKind::ContentBlock,
		});
		let Some(e) = at_else else { return true; };
		let tail = kids[e + 1..].iter().copied().find(|&a| !matches!(self.kind(a), SyntaxKind::Space | SyntaxKind::LineComment | SyntaxKind::BlockComment));
		match tail {
			Some(a) if self.kind(a) == SyntaxKind::Conditional	=> self.arms(a, out),
			Some(a) if block(a) && self.closed(a)				=> {
				out.push(Arm {
					cond:		None,
					body:		(self.nodes[a].start, self.nodes[a].end),
					content:	self.kind(a) == SyntaxKind::ContentBlock,
				});
				true
			},
			_	=> false,
		}
	}

	/// Does the block `a` end with its closer?
	fn closed(&self, a: usize) -> bool {
		let mut c = self.nodes[a].child;
		let mut last = NONE;
		while c != NONE {
			last = c;
			c = self.nodes[c].next;
		}
		last != NONE && matches!(self.kind(last), SyntaxKind::RightBracket | SyntaxKind::RightBrace)
	}

	/// The statements `bindings` lists.
	fn bindings(&self) -> Vec<Binding> {
		let mut out = Vec::new();
		self.find_bindings(0, &mut out);
		out
	}

	fn find_bindings(&self, a: usize, out: &mut Vec<Binding>) {
		let mut skip = NONE;	// the statement a binding took, which holds no other
		for c in self.kids(a) {
			if c == skip {
				continue;
			}
			if let Some(e) = self.embedded(c) {
				if matches!(self.kind(e), SyntaxKind::LetBinding | SyntaxKind::ModuleImport) {
					out.push(self.binding(c, e));
					skip = e;
					continue;
				}
			}
			if self.nodes[c].child != NONE {
				self.find_bindings(c, out);
			}
		}
	}

	/// The binding the `#` at `h` makes of the statement `e`.
	fn binding(&self, h: usize, e: usize) -> Binding {
		let mut text = String::from("#");
		self.push_text(e, &mut text);
		// A `;` ends the statement, and is part of what it holds over, though not of its text.
		let mut at	= self.nodes[e].end;
		let mut n	= self.nodes[e].next;
		while n != NONE && matches!(self.kind(n), SyntaxKind::Space | SyntaxKind::LineComment | SyntaxKind::BlockComment) {
			n = self.nodes[n].next;
		}
		if n != NONE && self.kind(n) == SyntaxKind::Semicolon {
			at = self.nodes[n].end;
		}
		// The innermost block it stands in holds it to its closer.
		let mut until	= None;
		let mut p		= self.nodes[h].parent;
		while p != NONE {
			if matches!(self.kind(p), SyntaxKind::ContentBlock | SyntaxKind::CodeBlock) {
				let close = self.kids(p).last().copied().filter(|&k| matches!(self.kind(k), SyntaxKind::RightBracket | SyntaxKind::RightBrace));
				until = Some(close.map_or(self.text.len(), |k| self.nodes[k].start));
				break;
			}
			p = self.nodes[p].parent;
		}
		Binding { start: self.nodes[h].start, text, at, until }
	}

	/// Appends the text beneath `a`, its comments dropped.
	fn push_text(&self, a: usize, out: &mut String) {
		let n = &self.nodes[a];
		if n.leaf != NONE {
			if self.leaves[n.leaf].tok != Tok::Comment {
				out.push_str(&self.text[n.start..n.end]);
			}
			return;
		}
		for c in self.kids(a) {
			self.push_text(c, out);
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use oxedyne_fe2o3_core::prelude::*;

	/// The top-level lines of `src`, by their trimmed text.
	fn tops(src: &str) -> Vec<String> {
		top_level_lines(src).into_iter().map(|(_, l)| l.trim().to_string()).collect()
	}

	#[test]
	fn a_bracket_in_prose_is_text_and_hides_no_line() {
		let src = "The interval [0, 1) is half-open.\n#include \"ch1.typ\"\n= Intervals [0, 1)\n#set document(title: \"T\")\n";
		assert_eq!(tops(src), vec![
			"The interval [0, 1) is half-open.", "#include \"ch1.typ\"", "= Intervals [0, 1)",
			"#set document(title: \"T\")",
		]);
	}

	#[test]
	fn a_bracket_code_opens_is_content() {
		let src = "#box[\n#include \"a.typ\"\n]\n#link(\"x\")[\nB\n]\n#[\nC\n]\n@fig[\nD\n]\nE\n";
		assert_eq!(tops(src), vec!["#box[", "#link(\"x\")[", "#[", "@fig[", "E"]);
	}

	#[test]
	fn a_scope_forgets_its_own_open_brackets() {
		// A heading's, an item's and a strong span's unbalanced `[` does not keep the block open.
		let src = "#box[\n= Intervals [0, 1)\n- item [a\n*[b*\n]\nAfter\n";
		assert_eq!(tops(src), vec!["#box[", "After"]);
		// In the block's own markup, a prose `[` does balance the next `]`.
		let src = "#box[\nprose [a\n]\nStill in\n]\nAfter\n";
		assert_eq!(tops(src), vec!["#box[", "After"]);
	}

	#[test]
	fn a_quote_in_markup_is_a_character() {
		let src = "He said \"use /* here\" today.\n#set document(title: \"After Quote\")\nMore */ tail words.\n";
		assert_eq!(tops(src), vec!["He said \"use /* here\" today."]);
		let live = live_text(src);
		assert!(!live.contains("#set"), "the comment the quote does not hide holds the declaration");
	}

	#[test]
	fn a_link_is_one_token() {
		let src = "See https://example.com/x/*y here.\n#include \"ch1.typ\"\nAnd http://a.io//b too.\n";
		assert_eq!(live_text(src), src);
		assert_eq!(tops(src).len(), 3);
		// Only `http://` and `https://` make a link; another scheme's `//` opens a comment.
		assert_eq!(live_text("ftp://x.io y\n"), "ftp:        \n");
	}

	#[test]
	fn a_link_drops_the_punctuation_that_ends_its_sentence() {
		let src = "https://x.io/a(b). /* c */";
		let link = "https://x.io/a(b)".len();
		let toks = byte_toks(src);
		assert!(toks[..link].iter().all(|&t| t == Tok::Link));
		assert_eq!(toks[link], Tok::Text);
		let src = "https://x.io. /* c */ d\n";
		assert_eq!(live_text(src), "https://x.io.         d\n");
	}

	#[test]
	fn block_comments_nest() {
		let src = "/* outer /* inner */\n#set document(title: \"Commented Out\")\n*/\nAfter\n";
		assert_eq!(tops(src), vec!["/* outer /* inner */", "After"]);
		assert_eq!(live_text("/* a /* b */ (c */ d"), fmt_spaces(19, "d"));
	}

	fn fmt_spaces(n: usize, tail: &str) -> String {
		let mut s = " ".repeat(n);
		s.push_str(tail);
		s
	}

	#[test]
	fn a_stray_closer_opens_no_comment() {
		// Typst reads `*/` outside a comment as one error token, so the `/*` inside `*/*` is not an opener.
		assert_eq!(live_text("a */* b\nc\n"), "a */* b\nc\n");
	}

	#[test]
	fn raw_text_runs_across_lines_until_a_run_as_long() {
		let src = "`one\n#set text(size: 9pt)\n` two\n````\n```\n#set x()\n```\n````\nEnd\n";
		assert_eq!(tops(src), vec!["`one", "````", "End"]);
		// Two backticks are empty raw text, and open nothing.
		assert_eq!(tops("``\n#include \"a.typ\"\n"), vec!["``", "#include \"a.typ\""]);
	}

	#[test]
	fn an_escape_takes_the_character_after_it() {
		// The `*` after the escaped slash opens a strong the `x*` closes, so the line after it stands at the top.
		assert_eq!(tops("\\[ \\/* x*\n#include \"a.typ\"\n"), vec!["\\[ \\/* x*", "#include \"a.typ\""]);
		assert_eq!(tops("#box[a \\] b\n]\nc\n"), vec!["#box[a \\] b", "c"]);
	}

	#[test]
	fn a_string_holds_comment_markers_in_code_and_maths() {
		let src = "#let x = \"a // b /* c\"\n$ \"d /* e\" $\nAfter\n";
		assert_eq!(live_text(src), src);
		assert_eq!(tops(src).len(), 3);
	}

	#[test]
	fn lone_dollars_leaves_earlier_pairs() {
		// Typst 0.15.1 refuses each of these files with "unclosed delimiter" at exactly the `$` named.
		assert_eq!(lone_dollars("a $x$ b $ c"), [8]);
		// A `]` is maths inside an equation, so the second `$` closes the first; the group never closes.
		assert_eq!(lone_dollars("#emph[a $b] c $d$"), [16]);
		// An equation opened in a group inside another: neither closes, the inner found first.
		assert_eq!(lone_dollars("$ a #[ b $ c"), [0, 9]);
		assert!(lone_dollars("a $x$ b $ y $\n").is_empty());
		let toks: Vec<Tok> = tokens("a $x$ b $ c").into_iter().map(|(_, _, t)| t).collect();
		assert_eq!(toks[2..5], [Tok::Open, Tok::Math, Tok::Close]);
		assert_eq!(toks[8..], [Tok::Text; 3]);
	}

	#[test]
	fn a_dollar_that_never_closes_hides_no_line() {
		let src = "Alpha $x + y.\n\n= Heading Later\n#include \"a.typ\"\n#if a [B]\n";
		assert_eq!(tops(src), vec!["Alpha $x + y.", "", "= Heading Later", "#include \"a.typ\"", "#if a [B]"]);
		assert_eq!(flows_of(src).len(), 1);
		// A backtick after it opens raw text, as after any character of prose, so the `/*` in it opens nothing.
		assert_eq!(live_text("A $ `b /* c` d\n"), "A $".to_string() + &fmt_spaces(10, "d\n"));
	}

	#[test]
	fn a_statement_ends_at_its_line_and_an_expression_after_its_operand() {
		// The `[` after a closed call and a space is prose, and so is the `"` after it.
		let src = "#link(\"x\")[a] then [b and \"c\n#include \"e.typ\"\n";
		assert_eq!(tops(src), vec!["#link(\"x\")[a] then [b and \"c", "#include \"e.typ\""]);
		// A statement's group runs across lines; the statement ends at the line after it closes.
		let src = "#let x = (\n  a: 1,\n)\n#include \"e.typ\"\n";
		assert_eq!(tops(src), vec!["#let x = (", "#include \"e.typ\""]);
	}

	#[test]
	fn a_shebang_is_a_comment_to_its_line_end() {
		let src = "#!/usr/bin/env typst /*\n= Root\n#include \"ch1.typ\"\n";
		assert_eq!(tops(src), vec!["#!/usr/bin/env typst /*", "= Root", "#include \"ch1.typ\""]);
		let first = src.find('\n').unwrap_or(src.len());
		assert!(tokens(src).iter().filter(|&&(at, _, _)| at < first).all(|&(_, _, t)| t == Tok::Comment));
		assert_eq!(live_text(src), fmt_spaces(first, &src[first..]));
	}

	#[test]
	fn a_shebang_only_opens_the_file() {
		// Past the file's first character, `#!` is an embedded expression Typst refuses, and the `/*` after it
		// opens a comment.
		let src = "x\n#!y /*\n= Root\n";
		assert_eq!(tops(src), vec!["x", "#!y /*"]);
		assert_eq!(tokens(src)[2].2, Tok::Hash);
	}

	#[test]
	fn a_line_in_open_strong_or_emphasis_is_not_top_level() {
		for m in ["*", "_"] {
			let src = fmt!("{m}bold [\n#set document(title: \"T\")\nstill bold{m}\nAfter\n");
			assert_eq!(tops(&src), vec![fmt!("{m}bold ["), "After".to_string()], "{}", m);
		}
	}

	#[test]
	fn an_unclosed_star_holds_to_the_paragraph_break() {
		let src = "2 * 3 = 6\n#set document(title: \"S\")\n\n#set text(size: 9pt)\n";
		assert_eq!(tops(src), vec!["2 * 3 = 6", "#set text(size: 9pt)"]);
	}

	#[test]
	fn an_underscore_between_cjk_letters_opens_emphasis() {
		assert_eq!(tops("漢_字\n#set document(title: \"C\")\n"), vec!["漢_字"]);
		// Between Latin letters it is within a word, and opens nothing.
		assert_eq!(tops("a_b\n#set document(title: \"C\")\n"), vec!["a_b", "#set document(title: \"C\")"]);
	}

	#[test]
	fn an_indented_line_stays_in_its_item() {
		let src = "- item\n  #set heading(numbering: \"1.\")\n= Next\n";
		assert_eq!(tops(src), vec!["- item", "= Next"]);
	}

	#[test]
	fn a_line_that_does_not_indent_ends_the_item() {
		let src = "- item\n#set document(title: \"T\")\n";
		assert_eq!(tops(src), vec!["- item", "#set document(title: \"T\")"]);
	}

	#[test]
	fn a_comment_ahead_of_the_first_token_does_not_hide_where_it_stands() {
		// The `#` stands at column 4, which does not indent past an item at column 6: the comment ahead of it
		// is trivia, and the token is what ends the item.
		let src = "      - item\n/**/#set document(title: \"T\")\n";
		assert_eq!(tops(src), vec!["- item", "/**/#set document(title: \"T\")"]);
		// A comment alone on a line holds no token, so it ends no item and stands in it.
		assert_eq!(tops("- item\n// c\n  #set text(size: 9pt)\n"), vec!["- item"]);
		assert_eq!(tops("- item\n/* c\n  d */ #set text(size: 9pt)\n"), vec!["- item"]);
	}

	/// Each placed line of `src` by its trimmed text.
	fn places(src: &str) -> Vec<(String, Place)> {
		placed_lines(src).into_iter().map(|(_, l, p)| (l.trim().to_string(), p)).collect()
	}

	#[test]
	fn placed_lines_tells_a_bare_block_from_a_call() {
		let src = "#[\n#set document(title: \"B\")\n]\n#box[\n#set document(title: \"X\")\n]\n";
		let got = places(src);
		assert_eq!(got[1], ("#set document(title: \"B\")".to_string(), Place::Block));
		assert_eq!(got[4], ("#set document(title: \"X\")".to_string(), Place::Content));
		// A bare block in a bare block is joined too; one opened in a list item is held by the item.
		assert_eq!(places("#[\n#[\nX\n]\n]\n")[2], ("X".to_string(), Place::Block));
		assert_eq!(places("- a #[\nX\n]\n")[1], ("X".to_string(), Place::Contained));
		// Content a conditional, `context` or a reference opens is not joined as written.
		for src in ["#if a [\nX\n]\n", "#context [\nX\n]\n", "#context[\nX\n]\n", "@r[\nX\n]\n", "#f(a, [\nX\n])\n"] {
			assert_eq!(places(src)[1], ("X".to_string(), Place::Content), "{:?}", src);
		}
	}

	#[test]
	fn a_conditional_takes_else_only_on_its_own_line() {
		let src = "#if a [\nX\n] else [\nY\n]\nAfter\n";
		assert_eq!(tops(src), vec!["#if a [", "After"]);
		// On the next line, `else [` is prose, and its `[` a prose bracket.
		let src = "#if a [X]\nelse [Y\nAfter\n";
		assert_eq!(tops(src), vec!["#if a [X]", "else [Y", "After"]);
	}

	/// The byte ranges of `src`'s flows, as `(kw, whole, text, arms)` with each arm as its condition and body
	/// text.
	fn flows_of(src: &str) -> Vec<(Kw, bool, &str, Vec<(Option<&str>, &str, bool)>)> {
		flows(src).into_iter().map(|f| {
			let arms = f.arms.iter()
				.map(|a| (a.cond.map(|(x, y)| src[x..y].trim()), &src[a.body.0..a.body.1], a.content))
				.collect();
			(f.kw, f.whole, &src[f.start..f.end], arms)
		}).collect()
	}

	#[test]
	fn a_conditional_is_read_whole_with_its_arms() {
		let src = "Text #if a == \"x\" [A] else [B] more.\n";
		assert_eq!(flows_of(src), vec![(Kw::If, true, "#if a == \"x\" [A] else [B]",
			vec![(Some("a == \"x\""), "[A]", true), (None, "[B]", true)])]);
		let src = "#if a [\nA\n] else if b [\nB\n] else {\n[C]\n}\nAfter\n";
		assert_eq!(flows_of(src), vec![(Kw::If, true, "#if a [\nA\n] else if b [\nB\n] else {\n[C]\n}",
			vec![(Some("a"), "[\nA\n]", true), (Some("b"), "[\nB\n]", true), (None, "{\n[C]\n}", false)])]);
		// On the next line, `else` is prose, and the conditional ends with its first body.
		let src = "#if a [A]\nelse [B]\n";
		assert_eq!(flows_of(src), vec![(Kw::If, true, "#if a [A]", vec![(Some("a"), "[A]", true)])]);
	}

	#[test]
	fn a_loop_is_read_whole() {
		let src = "#for x in xs [\nX #x\n]\n#while n < 3 { n += 1 }\n";
		assert_eq!(flows_of(src), vec![
			(Kw::For, true, "#for x in xs [\nX #x\n]", vec![(Some("x in xs"), "[\nX #x\n]", true)]),
			(Kw::While, true, "#while n < 3 { n += 1 }", vec![(Some("n < 3"), "{ n += 1 }", false)]),
		]);
	}

	#[test]
	fn only_a_flow_at_the_scans_own_level_is_listed() {
		// One in another's body, in a content block, in maths or after `context` is not the scan's own.
		let src = "#if a [\n#if b [x]\n]\n#box[#if c [y]]\n$#if d [z]$\n#context if e [w]\n";
		assert_eq!(flows_of(src).len(), 1);
		assert_eq!(flows_of(src)[0].2, "#if a [\n#if b [x]\n]");
		// Two flows back to back are two.
		assert_eq!(flows_of("#if a [x]#if b [y]\n").len(), 2);
	}

	#[test]
	fn a_flow_that_does_not_close_as_one_statement_is_not_whole() {
		// A call on its value, a head with no body, and a body that never closes.
		assert_eq!(flows_of("#if a [x](y) tail\n")[0].2, "#if a [x](y)");
		assert!(!flows_of("#if a [x](y) tail\n")[0].1);
		assert!(!flows_of("#if a\n[x]\n")[0].1);
		assert_eq!(flows_of("#if a [x\ny\n")[0].2, "#if a [x\ny\n");
		assert!(!flows_of("#if a [x\ny\n")[0].1);
	}

	/// The statements `bindings` finds, as `(text, the source before it comes into force, the closer's source
	/// from where it stops)`.
	fn binds_of(src: &str) -> Vec<(String, String, Option<String>)> {
		bindings(src).into_iter()
			.map(|b| (b.text.clone(), src[..b.at].to_string(), b.until.map(|u| src[u..].to_string())))
			.collect()
	}

	#[test]
	fn a_binding_is_listed_from_its_end_to_its_blocks_closer() {
		// At the text's own level it holds to the end; a `;` ends it there, and is part of what it holds over.
		let src = "#let a = \"x\"\nText\n#let b = true; more\n";
		assert_eq!(binds_of(src), vec![
			("#let a = \"x\"".to_string(), "#let a = \"x\"".to_string(), None),
			("#let b = true".to_string(), "#let a = \"x\"\nText\n#let b = true;".to_string(), None),
		]);
		// In a callout's body, or a conditional's branch, it stops at the closer; text after the block is
		// outside it, and a heading, strong or emphasis opens no scope of its own.
		let src = "#box[\n#let a = 1\nIN\n]\n#if x [\n#let b = 2\n]\n= H #let c = 3;\nEND\n";
		let got = binds_of(src);
		assert_eq!(got.len(), 3);
		assert_eq!(got[0], ("#let a = 1".to_string(), "#box[\n#let a = 1".to_string(), Some("]\n#if x [\n#let b = 2\n]\n= H #let c = 3;\nEND\n".to_string())));
		assert_eq!(got[1].0, "#let b = 2");
		assert_eq!(got[1].2.as_deref(), Some("]\n= H #let c = 3;\nEND\n"));
		assert_eq!(got[2].0, "#let c = 3");
		assert_eq!(got[2].2, None);
	}

	#[test]
	fn a_binding_a_comment_a_raw_or_a_code_block_holds_is_not_listed_as_markup() {
		// Comments inside a statement are dropped from its text; one standing alone, a raw span, and a `let`
		// of a code block (no `#`) are not statements of the markup.
		let src = "#let a = /* c */ \"x\" // t\n// #let b = 1\n`#let c = 1`\n#{ let d = 1 }\n#include \"f.typ\"\n#import \"g.typ\": *\n";
		let got: Vec<String> = binds_of(src).into_iter().map(|(t, _, _)| t.split_whitespace().collect::<Vec<_>>().join(" ")).collect();
		assert_eq!(got, ["#let a = \"x\"", "#import \"g.typ\": *"]);
	}

	#[test]
	fn a_value_ends_at_its_comma_or_its_groups_closer() {
		assert_eq!(top_comma(" \"a, b\", c"), " \"a, b\"".len());
		assert_eq!(top_comma(" [x, y) z], c"), " [x, y) z]".len());
		assert_eq!(top_comma(" none)\n#let x = \"b\", c"), " none".len());
		assert_eq!(top_comma(" /* , */ 1"), " /* , */ 1".len());
	}

	/// Each argument of `inner` as its key and value.
	fn pairs(inner: &str) -> Vec<(Option<String>, String)> {
		args(inner).into_iter().map(|a| (a.key, a.value)).collect()
	}

	fn keys(inner: &str) -> Vec<String> {
		args(inner).into_iter().filter_map(|a| a.key).collect()
	}

	#[test]
	fn an_argument_list_reads_comments_as_trivia() {
		assert_eq!(pairs("title: \"T\" /* c */"), vec![(Some("title".to_string()), "\"T\"".to_string())]);
		// In code a comment parts the tokens beside it; in a content block it leaves nothing, as Typst sets
		// nothing for it.
		assert_eq!(pairs("a: 1/* c */+ 2, [x/* c */y // d\n]"), vec![
			(Some("a".to_string()), "1 + 2".to_string()), (None, "[xy \n]".to_string()),
		]);
		// A comment between a name and its colon leaves the name a key.
		assert_eq!(keys("title /* c */ : \"T\""), vec!["title"]);
	}

	#[test]
	fn a_commented_out_named_argument_is_no_argument() {
		let list = args("\n  // author: \"Old\",\n  title: \"T\",\n");
		assert_eq!(list.len(), 1);
		assert_eq!(list[0].key.as_deref(), Some("title"));
		assert_eq!(named(&list, "author"), None);
	}

	#[test]
	fn a_comment_between_arguments_parts_nothing() {
		assert_eq!(keys("a: 1, /* b: 2, */ c: 3"), vec!["a", "c"]);
		assert_eq!(keys("title: \"T\" // , author: \"X\"\n"), vec!["title"]);
		assert_eq!(pairs("(\"a\", /* \"b\", */ \"c\")")[0].1, "(\"a\",   \"c\")");
		assert_eq!(pairs("\"a\", /* \"b\", */ \"c\""), vec![(None, "\"a\"".to_string()), (None, "\"c\"".to_string())]);
	}

	#[test]
	fn a_key_inside_a_string_or_group_is_no_key() {
		let list = args("title: \"T, author: Q\" /* , author: \"X\" */, keywords: (\"k\",)");
		assert_eq!(keys("title: \"T, author: Q\" /* , author: \"X\" */, keywords: (\"k\",)"), vec!["title", "keywords"]);
		assert_eq!(named(&list, "title"), Some("\"T, author: Q\""));
		assert_eq!(named(&list, "keywords"), Some("(\"k\",)"));
		assert_eq!(keys("header: [page: 1]"), vec!["header"]);
		assert_eq!(named(&args("header: [page: 1]"), "page"), None);
		// A string keeps its comment markers, and raw text its commas.
		assert_eq!(pairs("author: \"A /* not */ B\""), vec![(Some("author".to_string()), "\"A /* not */ B\"".to_string())]);
		assert_eq!(keys("title: [T `raw, author: \"X\"` z]"), vec!["title"]);
		// A spread, a positional value and a name without its colon are positional.
		assert_eq!(pairs("..xs, 2pt, body"), vec![
			(None, "..xs".to_string()), (None, "2pt".to_string()), (None, "body".to_string()),
		]);
		assert_eq!(keys("a b: 1, (c: 1)"), Vec::<String>::new());
	}

	/// The text a reader of `src` as an argument list keeps, as [`split_args`] does.
	fn kept(src: &str) -> String {
		let lx = Lexer::code(src);
		let mut out = String::new();
		for li in 0..lx.leaves.len() {
			match lx.kept(li) {
				Kept::Text		=> out.push_str(lx.leaf_text(li)),
				Kept::Space		=> out.push(' '),
				Kept::Nothing	=> {},
			}
		}
		out
	}

	#[test]
	fn a_step_over_an_argument_list_keeps_no_comment_text() {
		// One space for a comment in code, however it nests, and none in a content block, as in Typst.
		assert_eq!(kept("a /* x, y */, b"), "a  , b");
		assert_eq!(kept("a/* b /* c */ d */e"), "a e");
		assert_eq!(kept("a // c, d\n, b"), "a  \n, b");
		assert_eq!(kept("[p // q\n r /* s */]"), "[p \n r ]");
		// A string and raw text keep their comment markers.
		assert_eq!(kept("\"/* x */\", `// y`"), "\"/* x */\", `// y`");
	}

	#[test]
	fn a_name_given_twice_is_a_duplicate() {
		let list = args("title: \"A\", title: \"B\"");
		assert_eq!(duplicate_key(&list), Some("title"));
		assert_eq!(named(&list, "title"), Some("\"B\""));
		assert_eq!(duplicate_key(&args("title: \"A\" /* , title: \"B\" */")), None);
		// A name is read whole, so a longer one holding it is another name.
		let list = args("heading-font: \"A\", first-line-indent: 1em");
		assert_eq!(named(&list, "font"), None);
		assert_eq!(named(&list, "indent"), None);
		assert_eq!(named(&list, "first-line-indent"), Some("1em"));
	}

	#[test]
	fn a_top_level_paren_group_is_read_as_code() {
		let src = "// (a\n(b, \")\"), /* ( */ [(], (c (d)), (e";
		let groups: Vec<&str> = top_parens(src).into_iter().map(|(a, b)| &src[a..b]).collect();
		assert_eq!(groups, ["(b, \")\")", "(c (d))"]);
	}

	#[test]
	fn a_group_end_is_read_as_code() {
		let src = "(a, \"b)\", [c (d], /* ) */ e) tail";
		assert_eq!(group_end(src, 0).map(|e| &src[e..]), Some(" tail"));
		assert_eq!(group_end("(a", 0), None);
	}

	/// The texts of the flows `level` lists in `src`, with whether each stands in code and in an equation.
	fn deep(src: &str, level: Level) -> Vec<(String, bool, bool)> {
		flows_in(src, level).into_iter().map(|f| (src[f.start..f.end].to_string(), f.coded, f.math)).collect()
	}

	#[test]
	fn a_flow_in_a_content_block_is_listed_at_the_deep_level_only() {
		let src = "#table(columns: 2, [c1], [#if a [X] else [Y]])\nNote#footnote[F #if b [Z]].\n#if c [W]\n";
		assert_eq!(deep(src, Level::Own), vec![("#if c [W]".to_string(), false, false)]);
		assert_eq!(deep(src, Level::Deep), vec![
			("#if a [X] else [Y]".to_string(), false, false),
			("#if b [Z]".to_string(), false, false),
			("#if c [W]".to_string(), false, false),
		]);
	}

	#[test]
	fn a_flow_in_an_equation_is_listed_in_one_flagged_math() {
		let src = "P $x #if a [MX] else [MY]$ q $y$ #if b [Z]\n";
		assert_eq!(deep(src, Level::Own), vec![("#if b [Z]".to_string(), false, false)]);
		assert_eq!(deep(src, Level::Deep), vec![
			("#if a [MX] else [MY]".to_string(), false, true),
			("#if b [Z]".to_string(), false, false),
		]);
	}

	#[test]
	fn a_flow_in_code_is_flagged_and_one_in_a_statement_is_not_listed() {
		let src = "#{ let a = 1; [#if a [X]] }\n#f(r => [#if r [Y]])\n#let blk = [#if a [Z]]\n#set text(fill: if a [W])\n";
		assert_eq!(deep(src, Level::Deep), vec![
			("#if a [X]".to_string(), true, false),
			("#if r [Y]".to_string(), true, false),
		]);
	}

	#[test]
	fn a_flow_inside_a_flow_is_that_flows_and_a_loop_is_listed_whole() {
		let src = "#table([#for x in (1, 2) [#if x [A]]])\n";
		let got = flows_in(src, Level::Deep);
		assert_eq!(got.len(), 1);
		assert_eq!(got[0].kw, Kw::For);
		assert_eq!(&src[got[0].start..got[0].end], "#for x in (1, 2) [#if x [A]]");
	}

	#[test]
	fn a_let_in_a_text_is_listed_with_the_byte_of_its_hash() {
		let src = "A [#let a = 1\nB]";
		let b = bindings(src);
		assert_eq!(b.len(), 1);
		assert_eq!(&src[b[0].start..b[0].at], "#let a = 1");
	}

	// What the syntax tree settles that the state machine it replaces read otherwise. Each source below was
	// compiled by typst 0.15.1 as a probe, and the diagnostics it gave are named where they decide the reading.

	#[test]
	fn a_space_after_a_statement_is_markup_beside_it() {
		// The spaces before a trailing comment are the markup's, outside the statement; typst compiles this
		// without a diagnostic.
		let src = "#let a = 230   // note\n#let b = 1; // c\n";
		let toks = byte_toks(src);
		let first = src.find("   //").unwrap_or(0);
		assert!(toks[first..first + 3].iter().all(|&t| t == Tok::Text));
		assert_eq!(toks[first + 3], Tok::Comment);
		let second = src.find("; //").map_or(0, |k| k + 1);
		assert_eq!(toks[second - 1], Tok::Code);
		assert_eq!(toks[second], Tok::Text);
		// The statement's own text ends before them, and what it binds holds from its end.
		let b = bindings(src);
		assert_eq!(b[0].text, "#let a = 230");
		assert_eq!(&src[..b[0].at], "#let a = 230");
		assert_eq!(&src[..b[1].at], "#let a = 230   // note\n#let b = 1;");
	}

	#[test]
	fn a_statement_ends_where_typst_ends_it() {
		// typst: `expected semicolon or line break` after the `1` and nothing about `*b*`, so what follows is
		// markup, a strong span included.
		let src = "#let x = 1 a *b*\n";
		let toks = byte_toks(src);
		let a = src.find(" a").unwrap_or(0);
		assert!(toks[a..a + 2].iter().all(|&t| t == Tok::Text));
		assert_eq!(bindings(src)[0].text, "#let x = 1");
		// typst: `expected colon` after `heading`, then the same.
		let src = "#show heading it => it\n";
		let toks = byte_toks(src);
		let it = src.find(" it").unwrap_or(0);
		assert!(toks[it..src.len() - 1].iter().all(|&t| t == Tok::Text));
	}

	#[test]
	fn an_unclosed_paren_ends_at_a_terminator() {
		// typst: `unclosed delimiter` at the `(`, `unexpected closing bracket` at the `]`, and the duplicate
		// argument of the call after it, so the `]` and what follows are markup.
		let src = "#let x = (1, 2\n\n*unclosed strong\n\ntext ] more\n\n#f(a: 1, a: 2)\n";
		let toks = byte_toks(src);
		let bracket = src.find(" ] more").unwrap_or(0);
		assert!(toks[bracket..bracket + 7].iter().all(|&t| t == Tok::Text));
		let call = src.find("#f(").unwrap_or(0);
		assert_eq!((toks[call], toks[call + 1], toks[call + 2]), (Tok::Hash, Tok::Code, Tok::Open));
		// The group never closed, and so is still counted open.
		assert_eq!(Lexer::markup(src).depth_at(call), 1);
	}

	#[test]
	fn an_unclosed_paren_ends_with_the_code_block_round_it() {
		// typst: `unclosed delimiter` at the `(` alone, so the `}` closes the block.
		let src = "#{ import \"x\": (a, b }\n";
		assert_eq!(group_end(src, 1), Some(src.len() - 1));
		assert_eq!(group_end(src, src.find('(').unwrap_or(0)), None);
		assert_eq!(byte_toks(src)[src.len() - 1], Tok::Text);
		// The block ended what was open inside it, so the line after it is not in a group.
		let src = "#{ import \"x\": (a, b }\nnext\n";
		let lx = Lexer::markup(src);
		let next = src.find("next").unwrap_or(0);
		assert_eq!((lx.depth_at(next), lx.open_at(next)), (0, 0));
	}

	#[test]
	fn an_error_token_after_a_hash_is_code() {
		// typst: `invalid number suffix`, `expected a hexadecimal number` and `unexpected minus`, each reported
		// at the `#` and each part of the embedded expression.
		for (src, upto) in [("#12p\n", 4), ("#0x\n", 3), ("#-1\n", 2)] {
			assert!(byte_toks(src)[1..upto].iter().all(|&t| t == Tok::Code), "{:?}", src);
		}
		assert_eq!(byte_toks("#-1\n")[2], Tok::Text);
	}

	#[test]
	fn what_the_parser_left_over_in_code_is_read_again() {
		// typst: `expected named or keyed pair`, and nothing about what follows. The expression it refuses
		// is one error node holding the text of its tokens, whose string stays a string.
		let src = "#let d = (a: 1, \"x\" + \"y // z\")\n#set x()\n";
		let toks = byte_toks(src);
		let slashes = src.find("//").unwrap_or(0);
		assert_eq!(toks[slashes], Tok::Str);
		assert_eq!(tops(src).len(), 2);
	}

	#[test]
	fn a_line_with_no_token_stands_where_the_last_token_left_it() {
		// The blank line is inside the strong span, which a paragraph break has not yet ended; the line after it
		// is not.
		let src = "2 * 3\n#set x()\n\n#set y()\n";
		let lx = Lexer::markup(src);
		let blank = src.find("\n\n").map_or(0, |k| k + 1);
		assert_eq!(lx.place_at(blank), Some(Place::Contained));
		assert_eq!(lx.place_at(blank + 1), Some(Place::Top));
		// A heading ended with its line, and an item has not ended until a token says so.
		let src = "= H\n\ntext\n";
		assert_eq!(Lexer::markup(src).place_at(4), Some(Place::Top));
		assert!(Lexer::markup(src).bare_line_at(4));
		let src = "- item\n\n  more\n";
		assert_eq!(Lexer::markup(src).place_at(7), Some(Place::Contained));
	}

	#[test]
	fn a_block_closer_and_a_blank_line_in_an_empty_block_are_bare() {
		let src = "#box[\n\n]\n";
		let lx = Lexer::markup(src);
		assert!(lx.bare_line_at(6), "a blank line in a block that holds nothing yet");
		assert!(lx.bare_line_at(7), "the closer stands in the block's markup");
		assert_eq!(lx.place_at(7), Some(Place::Content));
		// With a list item open in the block, the closer at its own margin ends it.
		let src = "#box[\n- a\n]\n";
		assert!(Lexer::markup(src).bare_line_at(10));
		assert!(!Lexer::markup(src).bare_line_at(6) || Lexer::markup(src).place_at(6) == Some(Place::Content));
		// A line in strong inside the block is not bare.
		let src = "#box[\n*a\nb*\n]\n";
		assert!(!Lexer::markup(src).bare_line_at(8));
	}

	#[test]
	fn what_is_open_at_a_line_start_counts_groups_equations_and_literals() {
		let src = "#let x = (\n  a,\n)\nb\n";
		let lx = Lexer::markup(src);
		assert_eq!((lx.open_at(0), lx.open_at(11), lx.open_at(src.len() - 2)), (0, 1, 0));
		assert_eq!(lx.depth_at(11), 1);
		assert_eq!(lx.markup_level_at(11), None, "inside a code group");
		assert_eq!(lx.markup_level_at(src.len() - 2), Some(0));
		for (src, line) in [("$ a\n b $\n", 4), ("#\"a\nb\"\n", 4), ("`a\nb`\n", 3), ("/* a\nb */\n", 5)] {
			let lx = Lexer::markup(src);
			assert_eq!(lx.open_at(line), 1, "{:?}", src);
			assert_eq!(lx.open_at(src.len()), 0, "{:?}", src);
			assert_eq!(lx.place_at(line), None, "{:?}", src);
		}
		// What is open at the end of a text that stops inside a construct.
		for src in ["#f(a,\n", "$ x\n", "#[a\n", "`raw\n", "/* c\n", "#\"s\n"] {
			assert!(open_after(src), "{:?}", src);
		}
		for src in ["#f(a)\n", "$ x $\n", "#[a]\n", "`raw`\n", "/* c */\n", "#\"s\"\n"] {
			assert!(!open_after(src), "{:?}", src);
		}
	}

	#[test]
	fn a_group_is_found_in_the_prefix_that_holds_it() {
		// A group far longer than the first prefix, nested and holding strings, comments and brackets that
		// close nothing, ends where the whole text says.
		let mut inner = String::new();
		for k in 0..400 {
			inner.push_str(&fmt!("s{}: \"a ] b ) c\", item{}: [text ) with \\] {} /* ) */ (n{})], ", k, k, k, k));
		}
		let src = fmt!("({}) tail ( not part", inner);
		let end = src.find(") tail").map(|k| k + 1);
		assert_eq!(group_end(&src, 0), end);
		let chars: Vec<char> = src.chars().collect();
		assert_eq!(group_end_chars(&chars, 0), end);
		assert_eq!(group_end_chars(&chars, chars.len() - 9), None);
		assert_eq!(group_end("(a, b", 0), None);
		assert_eq!(top_comma(&fmt!("{} , more", &src[..end.unwrap_or(0)])), end.unwrap_or(0) + 1);
		// An index in chars is not an index in bytes.
		let src = "(é, \"ü ) €\", [ñ])x";
		let chars: Vec<char> = src.chars().collect();
		assert_eq!((group_end_chars(&chars, 0), group_end(src, 0)), (Some(17), Some(22)));
	}

	#[test]
	fn the_first_colon_at_the_own_level_names_an_argument() {
		assert_eq!(top_colon("key: value"), Some(3));
		assert_eq!(top_colon("(a: 1) b: 2"), Some(8));
		assert_eq!(top_colon("[a: 1], \"b: 2\", $c: 3$"), None);
		assert_eq!(split_args("a, [b, c], \"d, e\" /* , */, f"), vec!["a", " [b, c]", " \"d, e\"  ", " f"]);
	}
}
