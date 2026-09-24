//! The one markup lexer every source scanner reads through. It follows `typst-syntax` 0.15.1: its
//! `Lexer` for the tokens, and the mode stack its `Parser` keeps, since whether a `[` opens content or is
//! prose, and where an embedded expression hands back to markup, is decided there rather than in the
//! lexer. A `[` opens content only where code opened it (`#name[`, `#[`, a call's trailing argument, a
//! reference's supplement); in prose it is text, balanced against a later `]` within its own markup
//! scope (a heading, a list item, strong or emphasis keeps a count of its own). A `"` is a character in
//! markup and a string only in code or maths. An automatic link is one token, so the `//` or `/*` in it
//! opens no comment. Block comments nest. Raw text opens with one backtick or three or more and closes
//! on a run as long, across lines.

/// What a character is, as Typst reads it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Tok {
	Text,		// markup text, a bracket in prose included
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

/// The keyword that heads a loop or a conditional, while its body is still to come.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kw {
	If,
	While,
	For,
	Done,	// an `else` branch's: nothing may follow it but a call or a field
}

/// Where an embedded expression stands: after its `#`, the parser reads one atomic expression (a name,
/// a literal or a group, then any call or field written directly after it), or one statement, which runs
/// to its line's end.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Embed {
	Start,								// the expression follows directly
	Post,								// after an operand: `(`, `[` or `.name` may follow directly
	Stmt,								// `let`, `set`, `show`, `import`, `include`, `return`: to the line's end, a `;` or a closer
	Context,							// after `context`: spaces, then the expression
	Head { kw: Kw, seen: bool, adj: bool },	// a head before its body; an operand seen, and one directly before
	Tail { kw: Kw, spaced: bool },		// after a body: `else` may follow on the line
	Else,								// after `else`: `if`, or the last body
}

/// What opened a markup scope, and so what ends it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
	Body,			// the markup's own: ends with its content block, or never
	Heading,		// ends at its line's end, or at a label
	Item(usize),	// a list, enum or term item, its marker at this column: ends at a line that does not indent past it
	Strong,			// ends at the next `*`, or at a paragraph break
	Emph,			// ends at the next `_`, or at a paragraph break
}

/// One markup scope, with the count of its prose `[` not yet balanced by a `]`. The parser keeps one count
/// per scope, so a heading's unbalanced `[` is forgotten at its line's end.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Scope {
	kind:	Kind,
	nest:	u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Markup {
	block:	bool,		// a content block, closed by its own `]`
	scopes:	Vec<Scope>,	// outermost first, the `Body` scope at the bottom
	start:	bool,		// no token yet since the markup opened or since a line break: a marker may open here
	head:	bool,		// no token yet since a line break: the next one ends the items it does not indent past
}

impl Markup {
	fn new(block: bool) -> Self {
		Markup { block, scopes: vec![Scope { kind: Kind::Body, nest: 0 }], start: true, head: false }
	}

	/// Ends the innermost scope of `kind`-like shape and every scope opened inside it.
	fn end_from(&mut self, pick: impl Fn(Kind) -> bool) {
		if let Some(k) = self.scopes.iter().position(|s| s.kind != Kind::Body && pick(s.kind)) {
			self.scopes.truncate(k);
		}
	}
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Frame {
	Markup(Markup),
	Code(char),		// a code group and its closer; `'\0'` for a code scan's outermost level
	Math,
	Str(bool),		// whether the last character was a `\` still to take its escape
	Comment(u32),	// nested this deep
	Raw(usize),		// opened by this many backticks
	Embed(Embed),
}

/// The lexer's state between steps, so a scan fed line by line reads a construct that runs across lines
/// exactly as it reads the whole text. A caller that feeds lines feeds each line break too
/// ([`Lexer::feed_line`]): an embedded statement ends at one, and an `else` on the next line is prose.
#[derive(Clone, Debug)]
pub(crate) struct Lexer {
	frames:	Vec<Frame>,
	col:	usize,			// characters since the last line break
	blank:	u32,			// line breaks in the run of spaces now open, for a paragraph break
	prev:	Option<char>,	// the character before the next one, for a `*` or `_` within a word
}

impl Lexer {
	/// A scan of a file's markup, as Typst reads a `.typ` file.
	pub(crate) fn markup() -> Self {
		Self::with(Frame::Markup(Markup::new(false)))
	}

	/// A scan of code, as the inside of a call's argument list reads.
	pub(crate) fn code() -> Self {
		Self::with(Frame::Code('\0'))
	}

	fn with(base: Frame) -> Self {
		Lexer { frames: vec![base], col: 0, blank: 0, prev: None }
	}

	/// Is anything open beyond the scan's own level: a group, a string, an equation, a comment, raw text,
	/// or an embedded expression not yet ended?
	pub(crate) fn is_open(&self) -> bool {
		self.frames.len() > 1
	}

	/// How many groups code opened are open: `(`, `{`, a content block. Prose brackets, strings,
	/// equations, comments and raw text are not counted.
	pub(crate) fn depth(&self) -> usize {
		self.frames.iter().skip(1).filter(|f| matches!(f, Frame::Code(_) | Frame::Markup(_))).count()
	}

	/// The number of groups open when the scan stands in markup -- the file's own, or a content block's --
	/// and `None` when it stands in code, a string, an equation, a comment, raw text or an expression.
	pub(crate) fn markup_level(&self) -> Option<usize> {
		match self.frames.last() {
			Some(Frame::Markup(_))	=> Some(self.depth()),
			_						=> None,
		}
	}

	/// Steps through `text`.
	pub(crate) fn feed(&mut self, text: &str) {
		let chars: Vec<char> = text.chars().collect();
		let mut i = 0;
		while i < chars.len() {
			i += self.step(&chars, i).0;
		}
	}

	/// Steps through `line` and then a line break, for a caller that holds its lines without their
	/// terminators.
	pub(crate) fn feed_line(&mut self, line: &str) {
		self.feed(line);
		self.feed("\n");
	}

	/// Reads the token, or the part of a longer one, at `i`: how many characters it takes (at least one)
	/// and what they are.
	pub(crate) fn step(&mut self, chars: &[char], i: usize) -> (usize, Tok) {
		let (n, tok) = self.dispatch(chars, i);
		let n = n.max(1).min(chars.len() - i);
		// A paragraph break is a run of spaces holding two line breaks; any token, a comment included,
		// ends the run.
		let space = matches!(tok, Tok::Text | Tok::Code | Tok::Math);
		for k in i..i + n {
			let c = chars[k];
			if c == '\n' && k > 0 && chars[k - 1] == '\r' {
				continue;	// the second half of a `\r\n`, counted once
			}
			if is_newline(c) {
				self.col	= 0;
				self.blank	= if space { self.blank.saturating_add(1) } else { 0 };
			} else {
				self.col = self.col.saturating_add(1);
				if !(space && (c == ' ' || c == '\t')) {
					self.blank = 0;
				}
			}
		}
		self.prev = Some(chars[i + n - 1]);
		(n, tok)
	}

	fn dispatch(&mut self, chars: &[char], i: usize) -> (usize, Tok) {
		loop {
			let c		= chars[i];
			let next	= chars.get(i + 1).copied();
			let embed = match self.frames.last_mut() {
				Some(Frame::Str(esc)) => {
					if *esc {
						*esc = false;
						return (1, Tok::Str);
					}
					match c {
						'\\'	=> { *esc = true; return (1, Tok::Str); },
						'"'		=> { self.pop(); return (1, Tok::Str); },
						_		=> return (1, Tok::Str),
					}
				},
				Some(Frame::Comment(depth)) => {
					match (c, next) {
						('*', Some('/')) => {
							*depth = depth.saturating_sub(1);
							if *depth == 0 {
								self.pop();
							}
							return (2, Tok::Comment);
						},
						('/', Some('*'))	=> { *depth = depth.saturating_add(1); return (2, Tok::Comment); },
						_					=> return (1, Tok::Comment),
					}
				},
				Some(Frame::Raw(n)) => {
					let n = *n;
					if c != '`' {
						return (1, Tok::Raw);
					}
					let run = backtick_run(chars, i);
					if run >= n {
						self.pop();
						return (n, Tok::Raw);
					}
					return (run, Tok::Raw);
				},
				Some(Frame::Markup(_))	=> return self.markup_step(chars, i),
				Some(Frame::Math)		=> return self.math_step(chars, i),
				Some(Frame::Code(close)) => {
					let close = *close;
					return self.code_step(chars, i, close);
				},
				Some(Frame::Embed(e))	=> *e,
				None					=> return (1, Tok::Text),
			};
			match self.embed_step(chars, i, embed) {
				Some(r)	=> return r,
				// The expression ends before this character, which the level beneath reads.
				None	=> self.pop(),
			}
		}
	}

	/// Closes the innermost frame, never the scan's own level. A group closing straight back into a
	/// loop's or conditional's head leaves an operand just before the next character.
	fn pop(&mut self) {
		if self.frames.len() > 1 {
			self.frames.pop();
		}
		if let Some(Frame::Embed(Embed::Head { adj, .. })) = self.frames.last_mut() {
			*adj = true;
		}
	}

	fn set(&mut self, e: Embed) {
		if let Some(Frame::Embed(cur)) = self.frames.last_mut() {
			*cur = e;
		}
	}

	fn markup_mut(&mut self) -> Option<&mut Markup> {
		match self.frames.last_mut() {
			Some(Frame::Markup(m))	=> Some(m),
			_						=> None,
		}
	}

	/// A comment, a stray `*/` or raw text, which Typst's lexer reads before any mode's own rules.
	fn trivia(&mut self, chars: &[char], i: usize, raw: bool) -> Option<(usize, Tok)> {
		match (chars[i], chars.get(i + 1)) {
			('/', Some('/'))	=> Some((line_comment_len(chars, i), Tok::Comment)),
			('/', Some('*'))	=> { self.frames.push(Frame::Comment(1)); Some((2, Tok::Comment)) },
			('*', Some('/'))	=> Some((2, Tok::Text)),	// a stray closer: an error token, not an opener
			('`', _) if raw		=> Some(self.open_raw(chars, i)),
			_					=> None,
		}
	}

	fn open_raw(&mut self, chars: &[char], i: usize) -> (usize, Tok) {
		match backtick_run(chars, i) {
			2	=> (2, Tok::Raw),	// empty raw text
			n	=> { self.frames.push(Frame::Raw(n)); (n, Tok::Raw) },
		}
	}

	fn markup_step(&mut self, chars: &[char], i: usize) -> (usize, Tok) {
		let c		= chars[i];
		let next	= chars.get(i + 1).copied();
		// Comments first: they are trivia, and a line's first token is what follows them.
		if c == '/' && (next == Some('/') || next == Some('*')) {
			if let Some(r) = self.trivia(chars, i, false) {
				return r;
			}
		}
		if is_newline(c) {
			let parbreak = self.blank >= 1;
			if let Some(m) = self.markup_mut() {
				m.start	= true;
				m.head	= true;
				m.end_from(|k| k == Kind::Heading);
				if parbreak {
					m.end_from(|k| matches!(k, Kind::Strong | Kind::Emph));
				}
			}
			return (if c == '\r' && next == Some('\n') { 2 } else { 1 }, Tok::Text);
		}
		if c == ' ' || c == '\t' {
			return (1, Tok::Text);
		}
		self.markup_token(chars, i);
		if let Some(r) = self.trivia(chars, i, true) {
			return r;
		}
		match c {
			'\\'	=> (escape_len(chars, i), Tok::Escape),
			'h' if at_lit(chars, i, "http://") || at_lit(chars, i, "https://")
					=> (link_len(chars, i), Tok::Link),
			'<' if next.is_some_and(is_id_continue)	=> {
				// A label ends the heading it closes, when the heading is the innermost scope.
				if let Some(m) = self.markup_mut() {
					if m.scopes.last().map(|s| s.kind) == Some(Kind::Heading) {
						m.scopes.pop();
					}
				}
				(label_len(chars, i), Tok::Label)
			},
			'@' if next.is_some_and(is_label_char)	=> self.reference(chars, i),
			'['		=> {
				if let Some(s) = self.markup_mut().and_then(|m| m.scopes.last_mut()) {
					s.nest = s.nest.saturating_add(1);
				}
				(1, Tok::Text)
			},
			']'		=> self.markup_close(),
			'#'		=> { self.frames.push(Frame::Embed(Embed::Start)); (1, Tok::Hash) },
			'$'		=> { self.frames.push(Frame::Math); (1, Tok::Open) },
			'*' if !self.in_word(chars, i)	=> { self.toggle(Kind::Strong); (1, Tok::Text) },
			'_' if !self.in_word(chars, i)	=> { self.toggle(Kind::Emph); (1, Tok::Text) },
			_		=> (1, Tok::Text),
		}
	}

	/// A token in markup: the first on its line ends the items it does not indent past, and one where a
	/// marker may stand opens a heading or an item.
	fn markup_token(&mut self, chars: &[char], i: usize) {
		let col = self.col;
		let Some(m) = self.markup_mut() else { return; };
		if m.head {
			m.head = false;
			m.end_from(|k| matches!(k, Kind::Item(at) if at >= col));
		}
		if !m.start {
			return;
		}
		m.start = false;
		let c = chars[i];
		let kind = match c {
			'=' => {
				let run = chars[i..].iter().take_while(|&&x| x == '=').count();
				space_or_end(chars, i + run).then_some(Kind::Heading)
			},
			'-' | '+' | '/' if space_or_end(chars, i + 1) => Some(Kind::Item(col)),
			'0'..='9' => {
				let run = chars[i..].iter().take_while(|x| x.is_ascii_digit()).count();
				(chars.get(i + run) == Some(&'.') && space_or_end(chars, i + run + 1)).then_some(Kind::Item(col))
			},
			_ => None,
		};
		if let Some(kind) = kind {
			m.scopes.push(Scope { kind, nest: 0 });
			// An item's own markup opens at its start, so a marker may follow the item's.
			m.start = matches!(kind, Kind::Item(_));
		}
	}

	/// Is the `*` or `_` at `i` inside a word, where it is text?
	fn in_word(&self, chars: &[char], i: usize) -> bool {
		let prev = if i > 0 { chars.get(i - 1).copied() } else { self.prev };
		wordy(prev) && wordy(chars.get(i + 1).copied())
	}

	/// Opens a strong or emphasis scope, or closes the innermost one of that kind.
	fn toggle(&mut self, kind: Kind) {
		let Some(m) = self.markup_mut() else { return; };
		if m.scopes.last().map(|s| s.kind) == Some(kind) {
			m.scopes.pop();
		} else {
			m.scopes.push(Scope { kind, nest: 0 });
		}
	}

	/// A `]` in markup: text balancing the innermost scope's own `[`; else the end of every scope with none
	/// open, and then of the content block, or, at a file's own level, a stray bracket Typst refuses.
	fn markup_close(&mut self) -> (usize, Tok) {
		let block = match self.markup_mut() {
			Some(m) => {
				while let Some(s) = m.scopes.last_mut() {
					if s.nest > 0 {
						s.nest -= 1;
						return (1, Tok::Text);
					}
					if m.scopes.len() == 1 {
						break;
					}
					m.scopes.pop();
				}
				m.block
			},
			None => false,
		};
		if block {
			self.pop();
			(1, Tok::Close)
		} else {
			(1, Tok::Text)
		}
	}

	/// A reference, `@name` without a trailing `.` or `:`, and the supplement's content block when a `[`
	/// follows directly.
	fn reference(&mut self, chars: &[char], i: usize) -> (usize, Tok) {
		let mut j = i + 1;
		while j < chars.len() && is_label_char(chars[j]) {
			j += 1;
		}
		while j > i + 1 && matches!(chars[j - 1], '.' | ':') {
			j -= 1;
		}
		if chars.get(j) == Some(&'[') {
			self.frames.push(Frame::Markup(Markup::new(true)));
			j += 1;
		}
		(j - i, Tok::Ref)
	}

	fn math_step(&mut self, chars: &[char], i: usize) -> (usize, Tok) {
		if let Some(r) = self.trivia(chars, i, false) {
			return r;
		}
		match chars[i] {
			'\\'	=> (escape_len(chars, i), Tok::Escape),
			'"'		=> { self.frames.push(Frame::Str(false)); (1, Tok::Str) },
			'$'		=> { self.pop(); (1, Tok::Close) },
			'#'		=> { self.frames.push(Frame::Embed(Embed::Start)); (1, Tok::Hash) },
			_		=> (1, Tok::Math),
		}
	}

	fn code_step(&mut self, chars: &[char], i: usize, close: char) -> (usize, Tok) {
		match chars[i] {
			')' | '}' if close != '\0'	=> { self.pop(); (1, Tok::Close) },
			_							=> self.code_char(chars, i),
		}
	}

	/// A character of code that closes nothing: trivia, a string, a group, an equation or a label opens
	/// here; anything else is code.
	fn code_char(&mut self, chars: &[char], i: usize) -> (usize, Tok) {
		if let Some(r) = self.trivia(chars, i, true) {
			return r;
		}
		match chars[i] {
			'"'		=> { self.frames.push(Frame::Str(false)); (1, Tok::Str) },
			'('		=> { self.frames.push(Frame::Code(')')); (1, Tok::Open) },
			'{'		=> { self.frames.push(Frame::Code('}')); (1, Tok::Open) },
			'['		=> { self.frames.push(Frame::Markup(Markup::new(true))); (1, Tok::Open) },
			'$'		=> { self.frames.push(Frame::Math); (1, Tok::Open) },
			'<' if chars.get(i + 1).copied().is_some_and(is_id_continue)	=> (label_len(chars, i), Tok::Label),
			_		=> (1, Tok::Code),
		}
	}

	/// One step of an embedded expression, or `None` where it has ended and the level beneath reads the
	/// character. The phases follow `embedded_code_expr`: newline mode `Stop`, so a line break ends it
	/// outside a group; an operand takes a call or a field only written directly after it.
	fn embed_step(&mut self, chars: &[char], i: usize, e: Embed) -> Option<(usize, Tok)> {
		let c		= chars[i];
		let next	= chars.get(i + 1).copied();
		let comment	= c == '/' && (next == Some('/') || next == Some('*'));
		match e {
			Embed::Start => {
				// A space or a comment after the `#` leaves it standing alone, which Typst refuses.
				if c.is_whitespace() || comment {
					return None;
				}
				if c == '`' {
					self.set(Embed::Post);
					return Some(self.open_raw(chars, i));
				}
				if is_id_start(c) {
					let j = ident_end(chars, i);
					let word: String = chars[i..j].iter().collect();
					self.set(match word.as_str() {
						"let" | "set" | "show" | "import" | "include" | "return"	=> Embed::Stmt,
						"context"	=> Embed::Context,
						"if"		=> Embed::Head { kw: Kw::If, seen: false, adj: false },
						"while"		=> Embed::Head { kw: Kw::While, seen: false, adj: false },
						"for"		=> Embed::Head { kw: Kw::For, seen: false, adj: false },
						_			=> Embed::Post,
					});
					return Some((j - i, Tok::Code));
				}
				let digit = c.is_ascii_digit() || (c == '.' && next.is_some_and(|d| d.is_ascii_digit()));
				if digit {
					self.set(Embed::Post);
					return Some((number_len(chars, i), Tok::Code));
				}
				if c == '<' && next.is_some_and(is_id_continue) {
					self.set(Embed::Post);
					return Some((label_len(chars, i), Tok::Label));
				}
				if matches!(c, '(' | '{' | '[' | '"' | '$') {
					self.set(Embed::Post);
					return Some(self.code_char(chars, i));
				}
				None
			},
			Embed::Post => {
				match c {
					'(' | '['							=> Some(self.code_char(chars, i)),
					'.' if next.is_some_and(is_id_start)	=> Some((ident_end(chars, i + 1) - i, Tok::Code)),
					';'									=> { self.pop(); Some((1, Tok::Code)) },
					_									=> None,
				}
			},
			Embed::Stmt => {
				if is_newline(c) || matches!(c, ']' | ')' | '}') {
					return None;
				}
				if c == ';' {
					self.pop();
					return Some((1, Tok::Code));
				}
				Some(self.code_char(chars, i))
			},
			Embed::Context => {
				if is_newline(c) {
					return None;
				}
				if c.is_whitespace() {
					return Some((1, Tok::Code));
				}
				if comment {
					return self.trivia(chars, i, false);
				}
				self.set(Embed::Start);
				self.embed_step(chars, i, Embed::Start)
			},
			Embed::Head { kw, seen, adj } => {
				if is_newline(c) || matches!(c, ';' | ']' | ')' | '}') {
					return None;
				}
				if comment {
					self.set(Embed::Head { kw, seen, adj: false });
					return self.trivia(chars, i, false);
				}
				if c.is_whitespace() {
					self.set(Embed::Head { kw, seen, adj: false });
					return Some((1, Tok::Code));
				}
				// The body: a block after the head's operand, a `[` only where it is not the operand's call.
				if (c == '[' && seen && !adj) || (c == '{' && seen) {
					self.set(Embed::Tail { kw, spaced: false });
					return Some(self.code_char(chars, i));
				}
				if is_id_continue(c) {
					let j = ident_end(chars, i);
					self.set(Embed::Head { kw, seen: true, adj: true });
					return Some((j - i, Tok::Code));
				}
				let opens = matches!(c, '(' | '{' | '[' | '"' | '$' | '`');
				self.set(Embed::Head { kw, seen: seen || opens, adj: false });
				Some(self.code_char(chars, i))
			},
			Embed::Tail { kw, spaced } => {
				if is_newline(c) {
					return None;
				}
				// A call or a field on the whole expression, written directly after its body.
				if !spaced && (matches!(c, '(' | '[') || (c == '.' && next.is_some_and(is_id_start))) {
					self.set(Embed::Post);
					return self.embed_step(chars, i, Embed::Post);
				}
				if kw != Kw::If {
					return None;
				}
				if c.is_whitespace() {
					self.set(Embed::Tail { kw, spaced: true });
					return Some((1, Tok::Code));
				}
				if comment {
					self.set(Embed::Tail { kw, spaced: true });
					return self.trivia(chars, i, false);
				}
				if at_word(chars, i, "else") {
					self.set(Embed::Else);
					return Some((4, Tok::Code));
				}
				None
			},
			Embed::Else => {
				if is_newline(c) {
					return None;
				}
				if c.is_whitespace() {
					return Some((1, Tok::Code));
				}
				if comment {
					return self.trivia(chars, i, false);
				}
				if at_word(chars, i, "if") {
					self.set(Embed::Head { kw: Kw::If, seen: false, adj: false });
					return Some((2, Tok::Code));
				}
				if matches!(c, '[' | '{') {
					self.set(Embed::Tail { kw: Kw::Done, spaced: false });
					return Some(self.code_char(chars, i));
				}
				None
			},
		}
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

fn is_label_char(c: char) -> bool {
	is_id_continue(c) || c == ':' || c == '.'
}

/// Does the character count as part of a word for a `*` or `_` beside it? Letters of the CJK scripts
/// do not, since those scripts put no space between words.
fn wordy(c: Option<char>) -> bool {
	match c {
		Some(c) => c.is_alphanumeric() && !matches!(c as u32,
			0x1100..=0x11FF | 0x3040..=0x30FF | 0x3130..=0x318F | 0x31F0..=0x31FF | 0x3400..=0x4DBF
			| 0x4E00..=0x9FFF | 0xAC00..=0xD7AF | 0xF900..=0xFAFF | 0x20000..=0x2FA1F),
		None	=> false,
	}
}

/// Is the character at `i` a space, the end, or a comment's opener, so a marker before it stands alone?
fn space_or_end(chars: &[char], i: usize) -> bool {
	match chars.get(i) {
		None		=> true,
		Some(&c)	=> c.is_whitespace() || (c == '/' && matches!(chars.get(i + 1), Some('/') | Some('*'))),
	}
}

/// If the literal `s` sits at `i` in `chars`.
fn at_lit(chars: &[char], i: usize, s: &str) -> bool {
	let mut k = i;
	for ch in s.chars() {
		if chars.get(k) != Some(&ch) {
			return false;
		}
		k += 1;
	}
	true
}

/// Is the keyword `w` at `i`, a whole name?
fn at_word(chars: &[char], i: usize, w: &str) -> bool {
	at_lit(chars, i, w) && !chars.get(i + w.chars().count()).copied().is_some_and(is_id_continue)
}

fn ident_end(chars: &[char], i: usize) -> usize {
	let mut j = i + 1;
	while j < chars.len() && is_id_continue(chars[j]) {
		j += 1;
	}
	j
}

fn backtick_run(chars: &[char], i: usize) -> usize {
	chars[i..].iter().take_while(|&&c| c == '`').count()
}

/// A `//` comment runs to its line's end, the break itself not taken.
fn line_comment_len(chars: &[char], i: usize) -> usize {
	chars[i..].iter().position(|&c| is_newline(c)).unwrap_or(chars.len() - i)
}

/// A backslash escapes the character after it, or reads `\u{...}`; before a space or the end it is a line
/// break alone.
fn escape_len(chars: &[char], i: usize) -> usize {
	if at_lit(chars, i + 1, "u{") {
		let mut j = i + 3;
		while j < chars.len() && chars[j].is_ascii_alphanumeric() {
			j += 1;
		}
		if chars.get(j) == Some(&'}') {
			j += 1;
		}
		return j - i;
	}
	match chars.get(i + 1) {
		Some(c) if !c.is_whitespace()	=> 2,
		_								=> 1,
	}
}

/// An automatic link: `http://` or `https://`, then the characters a link may hold, brackets balanced, and
/// not the punctuation that likely ends the sentence around it.
fn link_len(chars: &[char], i: usize) -> usize {
	let head = if at_lit(chars, i, "https://") { 8 } else { 7 };
	let mut j = i + head;
	let mut open: Vec<char> = Vec::new();
	while j < chars.len() {
		let c = chars[j];
		let ok = match c {
			'0'..='9' | 'a'..='z' | 'A'..='Z' | '!' | '#' | '$' | '%' | '&' | '*' | '+' | ',' | '-' | '.' | '/'
			| ':' | ';' | '=' | '?' | '@' | '_' | '~' | '\''	=> true,
			'[' | '('	=> { open.push(c); true },
			']'			=> open.pop() == Some('['),
			')'			=> open.pop() == Some('('),
			_			=> false,
		};
		if !ok {
			break;
		}
		j += 1;
	}
	while j > i + head && matches!(chars[j - 1], '!' | ',' | '.' | ':' | ';' | '?' | '\'') {
		j -= 1;
	}
	j - i
}

/// A label `<name>`, or as much of one as there is.
fn label_len(chars: &[char], i: usize) -> usize {
	let mut j = i + 1;
	while j < chars.len() && is_label_char(chars[j]) {
		j += 1;
	}
	if chars.get(j) == Some(&'>') {
		j += 1;
	}
	j - i
}

/// A number, as code reads one: digits, a fraction, an exponent and a unit or `%`.
fn number_len(chars: &[char], i: usize) -> usize {
	let mut j = i;
	while j < chars.len() && (chars[j].is_ascii_alphanumeric() || chars[j] == '%') {
		j += 1;
	}
	if chars.get(j) == Some(&'.') && chars.get(j + 1).is_some_and(|d| d.is_ascii_digit()) {
		j += 1;
		while j < chars.len() && (chars[j].is_ascii_alphanumeric() || chars[j] == '%') {
			j += 1;
		}
	}
	j.max(i + 1) - i
}

/// Each character of `src` read as a file's markup, with the byte it starts at.
pub(crate) fn tokens(src: &str) -> Vec<(usize, char, Tok)> {
	let chars: Vec<(usize, char)>	= src.char_indices().collect();
	let only: Vec<char>				= chars.iter().map(|&(_, c)| c).collect();
	let mut out						= Vec::with_capacity(only.len());
	let mut lx						= Lexer::markup();
	let mut i						= 0usize;
	while i < only.len() {
		let (n, tok) = lx.step(&only, i);
		for k in i..i + n {
			out.push((chars[k].0, chars[k].1, tok));
		}
		i += n;
	}
	out
}

/// Which characters of `src` a comment or raw text holds, delimiters included, one entry per character.
pub(crate) fn literal_chars(src: &str) -> Vec<bool> {
	tokens(src).into_iter().map(|(_, _, t)| matches!(t, Tok::Comment | Tok::Raw)).collect()
}

/// What each byte of `src` is, read as a file's markup: every byte of a character takes the character's.
pub(crate) fn byte_toks(src: &str) -> Vec<Tok> {
	let mut out = vec![Tok::Text; src.len()];
	for (at, c, tok) in tokens(src) {
		for t in &mut out[at..at + c.len_utf8()] {
			*t = tok;
		}
	}
	out
}

/// `src` with every character a comment holds blanked to spaces and every line break kept, so each byte
/// offset and line of the result is the source's own.
pub(crate) fn uncommented(src: &str) -> String {
	let mut out = String::with_capacity(src.len());
	for (_, c, tok) in tokens(src) {
		if tok == Tok::Comment && c != '\n' && c != '\r' {
			for _ in 0..c.len_utf8() {
				out.push(' ');
			}
		} else {
			out.push(c);
		}
	}
	out
}

/// `src` with every character a comment or raw text holds blanked to spaces and every line break kept, so
/// each byte offset and line of the result is the source's own. A scan that finds a declaration, an
/// `#include` or a field in this text finds none a comment holds or a raw block shows.
pub(crate) fn live_text(src: &str) -> String {
	let mut out = String::with_capacity(src.len());
	for (_, c, tok) in tokens(src) {
		if matches!(tok, Tok::Comment | Tok::Raw) && c != '\n' && c != '\r' {
			for _ in 0..c.len_utf8() {
				out.push(' ');
			}
		} else {
			out.push(c);
		}
	}
	out
}

/// The lines of `src` that stand at its own level, each with the byte it starts at: a line opening inside
/// a group, a content block, a string, an equation, a comment or raw text is none of them.
pub(crate) fn top_level_lines(src: &str) -> Vec<(usize, &str)> {
	let mut out		= Vec::new();
	let mut offset	= 0usize;
	let mut lx		= Lexer::markup();
	for raw in src.split_inclusive('\n') {
		if !lx.is_open() {
			out.push((offset, raw));
		}
		lx.feed(raw);
		offset = offset.saturating_add(raw.len());
	}
	out
}

/// The byte just past the group opening at byte `at` of `src` -- a `(`, `[` or `{` read as code opens one
/// -- or `None` when it never closes.
pub(crate) fn group_end(src: &str, at: usize) -> Option<usize> {
	let rest: Vec<(usize, char)>	= src[at..].char_indices().collect();
	let only: Vec<char>				= rest.iter().map(|&(_, c)| c).collect();
	if !matches!(only.first(), Some('(') | Some('[') | Some('{')) {
		return None;
	}
	let mut lx	= Lexer::code();
	let mut i	= 0usize;
	while i < only.len() {
		i += lx.step(&only, i).0;
		if !lx.is_open() {
			return Some(at + rest.get(i).map_or(src.len() - at, |&(b, _)| b));
		}
	}
	None
}

/// Where a field's value in an argument list or a dictionary ends, for `src` read as code from the value's
/// start: the byte of the first comma at its own level, or of the closer of the group it stands in, or the
/// length of `src` when there is neither.
pub(crate) fn top_comma(src: &str) -> usize {
	let chars: Vec<(usize, char)>	= src.char_indices().collect();
	let only: Vec<char>				= chars.iter().map(|&(_, c)| c).collect();
	let mut lx	= Lexer::code();
	let mut i	= 0usize;
	while i < only.len() {
		if matches!(only[i], ',' | ')' | ']' | '}') && !lx.is_open() {
			return chars[i].0;
		}
		i += lx.step(&only, i).0;
	}
	src.len()
}

/// Each group a `(` opens at the top level of `src` read as code, by the byte of its `(` and the byte just
/// past its `)`. A group that never closes is not listed.
pub(crate) fn top_parens(src: &str) -> Vec<(usize, usize)> {
	let chars: Vec<(usize, char)>	= src.char_indices().collect();
	let only: Vec<char>				= chars.iter().map(|&(_, c)| c).collect();
	let byte		= |i: usize| chars.get(i).map_or(src.len(), |&(b, _)| b);
	let mut out		= Vec::new();
	let mut lx		= Lexer::code();
	let mut open	= None;	// the character a top-level `(` opens at
	let mut i		= 0usize;
	while i < only.len() {
		let top = !lx.is_open();
		let (n, tok) = lx.step(&only, i);
		if top && tok == Tok::Open && only[i] == '(' {
			open = Some(i);
		}
		if let Some(o) = open.filter(|_| !lx.is_open()) {
			out.push((byte(o), byte(i + n)));
			open = None;
		}
		i += n;
	}
	out
}

#[cfg(test)]
mod tests {
	use super::*;

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
		let chars: Vec<char> = "https://x.io/a(b). /* c */".chars().collect();
		assert_eq!(link_len(&chars, 0), "https://x.io/a(b)".len());
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
		assert_eq!(tops("\\[ \\/* x\n#include \"a.typ\"\n"), vec!["\\[ \\/* x", "#include \"a.typ\""]);
		assert_eq!(tops("#box[a \\] b\n]\nc\n"), vec!["#box[a \\] b", "c"]);
	}

	#[test]
	fn a_string_holds_comment_markers_in_code_and_maths() {
		let src = "#let x = \"a // b /* c\"\n$ \"d /* e\" $\nAfter\n";
		assert_eq!(live_text(src), src);
		assert_eq!(tops(src).len(), 3);
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
	fn a_conditional_takes_else_only_on_its_own_line() {
		let src = "#if a [\nX\n] else [\nY\n]\nAfter\n";
		assert_eq!(tops(src), vec!["#if a [", "After"]);
		// On the next line, `else [` is prose, and its `[` a prose bracket.
		let src = "#if a [X]\nelse [Y\nAfter\n";
		assert_eq!(tops(src), vec!["#if a [X]", "else [Y", "After"]);
	}

	#[test]
	fn a_value_ends_at_its_comma_or_its_groups_closer() {
		assert_eq!(top_comma(" \"a, b\", c"), " \"a, b\"".len());
		assert_eq!(top_comma(" [x, y) z], c"), " [x, y) z]".len());
		assert_eq!(top_comma(" none)\n#let x = \"b\", c"), " none".len());
		assert_eq!(top_comma(" /* , */ 1"), " /* , */ 1".len());
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
}
