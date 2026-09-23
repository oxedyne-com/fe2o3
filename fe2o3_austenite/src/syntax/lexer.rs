//! Ported from typst-syntax 0.15.1 (`src/lexer.rs`, Apache-2.0, (c) the Typst authors), token for token:
//! the same modes, the same kinds, the same error messages and hints, so a document lexes as Typst
//! lexes it. The Unicode predicates it needs (XID identifiers, the CJK scripts, the maths delimiter
//! classes) come from the generated tables in `lexer/unicode.rs` rather than crates.

mod unicode;

use crate::syntax::{
	SyntaxKind,
	SyntaxNode,
	Span,
};

use oxedyne_fe2o3_text::unicode::segment;

/// The three lexical modes Typst switches between.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LexMode {
	Markup,
	Math,
	Code,
}

// ┌───────────────────────────────────────────────────────────────────────────────────────────┐
// │ Scanner                                                                                   │
// └───────────────────────────────────────────────────────────────────────────────────────────┘

/// A cursor over a string, after `unscanny`. Every position is a byte offset on a char boundary.
#[derive(Clone, Copy, Debug)]
pub struct Scanner<'s> {
	text:	&'s str,
	cursor:	usize,
}

/// Something a [`Scanner`] can match at its cursor: a char, a string, a set of chars or a predicate.
pub trait Pattern {
	/// The byte length matched at the start of `s`, if any.
	fn matches(&mut self, s: &str) -> Option<usize>;
}

impl Pattern for char {
	fn matches(&mut self, s: &str) -> Option<usize> {
		if s.starts_with(*self) { Some(self.len_utf8()) } else { None }
	}
}

impl Pattern for &str {
	fn matches(&mut self, s: &str) -> Option<usize> {
		if s.starts_with(*self) { Some(self.len()) } else { None }
	}
}

impl<const N: usize> Pattern for [char; N] {
	fn matches(&mut self, s: &str) -> Option<usize> {
		match s.chars().next() {
			Some(c) if self.contains(&c)	=> Some(c.len_utf8()),
			_								=> None,
		}
	}
}

impl<F: FnMut(char) -> bool> Pattern for F {
	fn matches(&mut self, s: &str) -> Option<usize> {
		match s.chars().next() {
			Some(c) if self(c)	=> Some(c.len_utf8()),
			_					=> None,
		}
	}
}

impl<'s> Scanner<'s> {
	pub fn new(text: &'s str) -> Self { Self { text, cursor: 0 } }

	pub fn string(&self) -> &'s str { self.text }
	pub fn cursor(&self) -> usize { self.cursor }
	pub fn done(&self) -> bool { self.cursor >= self.text.len() }

	/// Moves to `index`, snapped back onto a char boundary and clamped to the text.
	pub fn jump(&mut self, index: usize) {
		let mut i = index.min(self.text.len());
		while !self.text.is_char_boundary(i) {
			i -= 1;
		}
		self.cursor = i;
	}

	pub fn advance(&mut self, by: usize) { self.jump(self.cursor + by); }

	pub fn before(&self) -> &'s str { self.get(0, self.cursor) }
	pub fn after(&self) -> &'s str { self.get(self.cursor, self.text.len()) }
	pub fn from(&self, start: usize) -> &'s str { self.get(start, self.cursor) }

	/// The text between two offsets, empty when they do not bound a slice.
	pub fn get(&self, a: usize, b: usize) -> &'s str {
		match self.text.get(a.min(b)..b) {
			Some(s)	=> s,
			None	=> "",
		}
	}

	pub fn peek(&self) -> Option<char> { self.after().chars().next() }

	/// The char `n` places from the cursor: `0` is the next, `-1` the previous.
	pub fn scout(&self, n: isize) -> Option<char> {
		if n >= 0 {
			self.after().chars().nth(n as usize)
		} else {
			self.before().chars().rev().nth((-n - 1) as usize)
		}
	}

	pub fn eat(&mut self) -> Option<char> {
		let c = self.peek();
		if let Some(c) = c {
			self.cursor += c.len_utf8();
		}
		c
	}

	/// Steps back over the previous char.
	pub fn uneat(&mut self) {
		if let Some(c) = self.before().chars().next_back() {
			self.cursor -= c.len_utf8();
		}
	}

	pub fn at<P: Pattern>(&self, mut pat: P) -> bool { pat.matches(self.after()).is_some() }

	pub fn eat_if<P: Pattern>(&mut self, mut pat: P) -> bool {
		match pat.matches(self.after()) {
			Some(n)	=> { self.cursor += n; true },
			None	=> false,
		}
	}

	pub fn eat_while<P: Pattern>(&mut self, mut pat: P) -> &'s str {
		let start = self.cursor;
		while let Some(n) = pat.matches(self.after()) {
			if n == 0 {
				break;
			}
			self.cursor += n;
		}
		self.from(start)
	}

	pub fn eat_until<P: Pattern>(&mut self, mut pat: P) -> &'s str {
		let start = self.cursor;
		while !self.done() && pat.matches(self.after()).is_none() {
			self.eat();
		}
		self.from(start)
	}

	/// Eats one newline, a CR LF pair counting as one.
	pub fn eat_newline(&mut self) -> bool {
		let ate = self.eat_if(is_newline);
		if ate && self.before().ends_with('\r') {
			self.eat_if('\n');
		}
		ate
	}
}

// ┌───────────────────────────────────────────────────────────────────────────────────────────┐
// │ Lexer                                                                                     │
// └───────────────────────────────────────────────────────────────────────────────────────────┘

/// Turns source text into tokens, one call to [`Lexer::next`] at a time. Each token comes back as a
/// detached node; the parser assigns spans once the tree is whole.
#[derive(Clone, Debug)]
pub struct Lexer<'s> {
	s:			Scanner<'s>,
	mode:		LexMode,
	newline:	bool,						// the last token held a newline
	error:		Option<(String, Vec<String>)>,	// the current token's error and hints
}

impl<'s> Lexer<'s> {
	pub fn new(text: &'s str, mode: LexMode) -> Self {
		Self { s: Scanner::new(text), mode, newline: false, error: None }
	}

	pub fn mode(&self) -> LexMode { self.mode }
	pub fn set_mode(&mut self, mode: LexMode) { self.mode = mode; }
	pub fn cursor(&self) -> usize { self.s.cursor() }
	pub fn jump(&mut self, index: usize) { self.s.jump(index); }

	/// Did the last token contain a newline?
	pub fn newline(&self) -> bool { self.newline }

	/// The number of chars between the last newline before `index` and `index`.
	pub fn column(&self, index: usize) -> usize {
		let mut s = self.s;
		s.jump(index);
		s.before().chars().rev().take_while(|c| !is_newline(*c)).count()
	}

	fn error(&mut self, message: String) -> SyntaxKind {
		self.error = Some((message, Vec::new()));
		SyntaxKind::Error
	}

	fn hint(&mut self, hint: &str) {
		if let Some((_, hints)) = &mut self.error {
			hints.push(hint.to_string());
		}
	}

	/// The next token, as its kind and its node. `End` at the end of the text.
	pub fn next(&mut self) -> (SyntaxKind, SyntaxNode) {
		self.error = None;
		let start = self.s.cursor();
		self.newline = false;
		let kind = match self.s.eat() {
			Some(c) if is_space(c, self.mode)				=> self.whitespace(start, c),
			Some('#') if start == 0 && self.s.eat_if('!')	=> self.shebang(),
			Some('/') if self.s.eat_if('/')					=> self.line_comment(),
			Some('/') if self.s.eat_if('*')					=> self.block_comment(),
			Some('*') if self.s.eat_if('/')					=> {
				let k = self.error("unexpected end of block comment".to_string());
				self.hint("consider escaping the `*` with a backslash or opening the block comment with `/*`");
				k
			},
			Some('`') if self.mode != LexMode::Math			=> return self.raw(),
			Some(c)											=> match self.mode {
				LexMode::Markup	=> self.markup(start, c),
				LexMode::Math	=> match self.math(start, c) {
					(kind, None)		=> kind,
					(kind, Some(node))	=> return (kind, node),
				},
				LexMode::Code	=> self.code(start, c),
			},
			None											=> SyntaxKind::End,
		};

		let text = self.s.from(start);
		let node = match self.error.take() {
			Some((message, hints)) => {
				let mut n = SyntaxNode::error(text, &message, Span::detached());
				for h in &hints {
					n.hint(h);
				}
				n
			},
			None => SyntaxNode::leaf(kind, text, Span::detached()),
		};
		(kind, node)
	}

	fn whitespace(&mut self, start: usize, c: char) -> SyntaxKind {
		let mode = self.mode;
		let more = self.s.eat_while(|c| is_space(c, mode));
		let newlines = match c {
			' ' if more.is_empty()	=> 0,
			_						=> count_newlines(self.s.from(start)),
		};
		self.newline = newlines > 0;
		if self.mode == LexMode::Markup && newlines >= 2 { SyntaxKind::Parbreak } else { SyntaxKind::Space }
	}

	fn shebang(&mut self) -> SyntaxKind {
		self.s.eat_until(is_newline);
		SyntaxKind::Shebang
	}

	fn line_comment(&mut self) -> SyntaxKind {
		self.s.eat_until(is_newline);
		SyntaxKind::LineComment
	}

	fn block_comment(&mut self) -> SyntaxKind {
		let mut state = '_';
		let mut depth = 1;
		// The first `*/` that closes the outermost `/*`.
		while let Some(c) = self.s.eat() {
			state = match (state, c) {
				('*', '/')	=> {
					depth -= 1;
					if depth == 0 {
						break;
					}
					'_'
				},
				('/', '*')	=> {
					depth += 1;
					'_'
				},
				_			=> c,
			}
		}
		SyntaxKind::BlockComment
	}
}

// Raw.
impl Lexer<'_> {
	/// A whole raw element, delimiters and all, lexed in one go.
	fn raw(&mut self) -> (SyntaxKind, SyntaxNode) {
		let start = self.s.cursor() - 1;
		let mut backticks = 1;
		while self.s.eat_if('`') {
			backticks += 1;
		}

		// Two backticks are an empty inline raw.
		if backticks == 2 {
			let nodes = vec![
				SyntaxNode::leaf(SyntaxKind::RawDelim, "`", Span::detached()),
				SyntaxNode::leaf(SyntaxKind::RawDelim, "`", Span::detached()),
			];
			return (SyntaxKind::Raw, SyntaxNode::inner(SyntaxKind::Raw, nodes));
		}

		let mut found = 0;
		while found < backticks {
			match self.s.eat() {
				Some('`')	=> found += 1,
				Some(_)		=> found = 0,
				None		=> {
					let node = SyntaxNode::error(self.s.from(start), "unclosed raw text", Span::detached());
					return (SyntaxKind::Error, node);
				},
			}
		}
		let end = self.s.cursor();

		let mut inner = Scanner::new(self.s.get(start + backticks, end - backticks));
		let inner_len = inner.string().len();

		let delim = SyntaxNode::leaf(SyntaxKind::RawDelim, self.s.get(end - backticks, end), Span::detached());
		let mut nodes = vec![delim.clone()];

		let mut tag = None;
		let mut future_len = None;
		if backticks >= 3 {
			let (t, f) = raw_lang_tag(&mut inner);
			tag = t;
			future_len = f;
			if let Some(t) = tag {
				nodes.push(SyntaxNode::leaf(SyntaxKind::RawLang, t, Span::detached()));
			}
			blocky_raw(&mut inner, &mut nodes);
		} else {
			inline_raw(&mut inner, &mut nodes);
		}
		nodes.push(delim);

		let mut raw = SyntaxNode::inner(SyntaxKind::Raw, nodes);
		add_raw_warnings(&mut raw, backticks, future_len, tag, inner_len);
		(SyntaxKind::Raw, raw)
	}
}

/// The language tag, if any, and the length the tag will have in the next version of Typst when that
/// differs (all text up to whitespace or a backtick).
fn raw_lang_tag<'a>(s: &mut Scanner<'a>) -> (Option<&'a str>, Option<usize>) {
	let start = s.cursor();
	let future = s.eat_until(|c: char| c.is_whitespace() || c == '`');
	if future.is_empty() {
		return (None, None);
	}
	s.jump(start);
	let tag = if s.eat_if(is_id_start) {
		s.eat_while(is_id_continue);
		Some(s.from(start))
	} else {
		None
	};
	let differs = match tag {
		Some(t)	=> t.len() != future.len(),
		None	=> true,
	};
	(tag, if differs { Some(future.len()) } else { None })
}

// A blocky raw trims the rest of the opening line when it is blank (else one space), dedents the inner
// lines by their common leading whitespace (the closing line counts, the opening line does not), and
// drops a blank last line; a last line ending in a backtick loses one trailing space.
fn blocky_raw(s: &mut Scanner, nodes: &mut Vec<SyntaxNode>) {
	let mut lines = split_newlines(s.after());

	let mut dedent = usize::MAX;
	for line in lines.iter().skip(1) {
		if !line.chars().all(char::is_whitespace) {
			dedent = dedent.min(line.chars().take_while(|c| c.is_whitespace()).count());
		}
	}
	if let Some(last) = lines.last() {
		dedent = dedent.min(last.chars().take_while(|c| c.is_whitespace()).count());
	}
	if dedent == usize::MAX {
		dedent = 0;
	}

	let last_blank = match lines.last() {
		Some(last)	=> last.chars().all(char::is_whitespace),
		None		=> false,
	};
	if last_blank {
		lines.pop();
	} else if let Some(last) = lines.last_mut() {
		if last.trim_end().ends_with('`') {
			if let Some(stripped) = last.strip_suffix(' ') {
				*last = stripped;
			}
		}
	}

	let mut prev = s.cursor();
	let mut push = |kind: SyntaxKind, s: &Scanner, prev: &mut usize| {
		nodes.push(SyntaxNode::leaf(kind, s.from(*prev), Span::detached()));
		*prev = s.cursor();
	};

	let mut lines = lines.into_iter();
	if let Some(first) = lines.next() {
		if first.chars().all(char::is_whitespace) {
			// Folded into the next `RawTrimmed`.
			s.advance(first.len());
		} else {
			let line_end = s.cursor() + first.len();
			if s.eat_if(' ') {
				push(SyntaxKind::RawTrimmed, s, &mut prev);
			}
			s.jump(line_end);
			push(SyntaxKind::Text, s, &mut prev);
		}
	}

	for line in lines {
		let offset: usize = line.chars().take(dedent).map(char::len_utf8).sum();
		s.eat_newline();
		s.advance(offset);
		push(SyntaxKind::RawTrimmed, s, &mut prev);
		s.advance(line.len() - offset);
		push(SyntaxKind::Text, s, &mut prev);
	}

	if !s.done() {
		nodes.push(SyntaxNode::leaf(SyntaxKind::RawTrimmed, s.get(prev, s.string().len()), Span::detached()));
	}
}

// Inline raw keeps all whitespace; each newline becomes a `RawTrimmed` between `Text` lines.
fn inline_raw(s: &mut Scanner, nodes: &mut Vec<SyntaxNode>) {
	let mut prev = s.cursor();
	while !s.done() {
		if s.at(is_newline) {
			nodes.push(SyntaxNode::leaf(SyntaxKind::Text, s.from(prev), Span::detached()));
			prev = s.cursor();
			s.eat_newline();
			nodes.push(SyntaxNode::leaf(SyntaxKind::RawTrimmed, s.from(prev), Span::detached()));
			prev = s.cursor();
			continue;
		}
		s.eat();
	}
	nodes.push(SyntaxNode::leaf(SyntaxKind::Text, s.from(prev), Span::detached()));
}

// Typst 0.15 warns where the next version will read the language tag differently, and on a tag with no
// text after it.
fn add_raw_warnings(
	raw:		&mut SyntaxNode,
	backticks:	usize,
	future_len:	Option<usize>,
	tag:		Option<&str>,
	inner_len:	usize,
) {
	if let Some(future_len) = future_len {
		let (a, b) = (backticks, backticks + future_len);
		match tag {
			Some(tag) => {
				raw.warn_at(a, b, "no whitespace between language tag and raw text");
				raw.hint(&format!("currently, Typst is treating `{}` as the language tag", tag));
				raw.hint("in the next version of Typst, this will change and we will treat all text \
					until the first whitespace as the language tag");
				let tag_end = backticks + tag.len();
				raw.hint_at(a, tag_end, &format!("if the current behavior is correct, please add a space after `{}`", tag));
				raw.hint_at(a, tag_end, "otherwise, add a space or newline after the initial backticks");
			},
			None => {
				raw.warn_at(a, b, "no whitespace before raw text");
				raw.hint("in the next version of Typst, this text will be treated as the language tag \
					for this element");
				raw.hint("to avoid this, add a space after the initial backticks");
			},
		}
	} else if let Some(tag) = tag {
		if inner_len == tag.len() {
			raw.warn("empty raw text");
			raw.hint(&format!("Typst is treating `{}` as the language tag", tag));
			raw.hint_at(backticks, backticks + tag.len(), "to treat this as text, add a space after the initial backticks");
		}
	}
}

// Markup.
impl Lexer<'_> {
	fn markup(&mut self, start: usize, c: char) -> SyntaxKind {
		match c {
			'\\'										=> self.backslash(),
			'h' if self.s.eat_if("ttp://")				=> self.link(),
			'h' if self.s.eat_if("ttps://")				=> self.link(),
			'<' if self.s.at(is_id_continue)			=> self.label(),
			'@' if self.s.at(is_id_continue)			=> self.ref_marker(),

			'.' if self.s.eat_if("..")					=> SyntaxKind::Shorthand,
			'-' if self.s.eat_if("--")					=> SyntaxKind::Shorthand,
			'-' if self.s.eat_if('-')					=> SyntaxKind::Shorthand,
			'-' if self.s.eat_if('?')					=> SyntaxKind::Shorthand,
			'-' if self.s.at(char::is_numeric)			=> SyntaxKind::Shorthand,
			'*' if !self.in_word()						=> SyntaxKind::Star,
			'_' if !self.in_word()						=> SyntaxKind::Underscore,

			'#'											=> SyntaxKind::Hash,
			'['											=> SyntaxKind::LeftBracket,
			']'											=> SyntaxKind::RightBracket,
			'\''										=> SyntaxKind::SmartQuote,
			'"'											=> SyntaxKind::SmartQuote,
			'$'											=> SyntaxKind::Dollar,
			'~'											=> SyntaxKind::Shorthand,
			':'											=> SyntaxKind::Colon,
			'='											=> {
				self.s.eat_while('=');
				if self.space_or_end() { SyntaxKind::HeadingMarker } else { self.text() }
			},
			'-' if self.space_or_end()					=> SyntaxKind::ListMarker,
			'+' if self.space_or_end()					=> SyntaxKind::EnumMarker,
			'/' if self.space_or_end()					=> SyntaxKind::TermMarker,
			'0'..='9'									=> self.numbering(start),

			_											=> self.text(),
		}
	}

	fn backslash(&mut self) -> SyntaxKind {
		if self.s.eat_if("u{") {
			let hex = self.s.eat_while(|c: char| c.is_ascii_alphanumeric());
			if !self.s.eat_if('}') {
				return self.error("unclosed Unicode escape sequence".to_string());
			}
			let valid = match u32::from_str_radix(hex, 16) {
				Ok(n)	=> char::from_u32(n).is_some(),
				Err(_)	=> false,
			};
			if !valid {
				return self.error(format!("invalid Unicode codepoint: {}", hex));
			}
			return SyntaxKind::Escape;
		}
		if self.s.done() || self.s.at(char::is_whitespace) {
			SyntaxKind::Linebreak
		} else {
			self.s.eat();
			SyntaxKind::Escape
		}
	}

	fn link(&mut self) -> SyntaxKind {
		let (link, balanced) = link_prefix(self.s.after());
		self.s.advance(link.len());
		if !balanced {
			return self.error("automatic links cannot contain unbalanced brackets, use the `link` function \
				instead".to_string());
		}
		SyntaxKind::Link
	}

	fn numbering(&mut self, start: usize) -> SyntaxKind {
		self.s.eat_while(|c: char| c.is_ascii_digit());
		let read = self.s.from(start);
		if self.s.eat_if('.') && self.space_or_end() && read.parse::<u64>().is_ok() {
			return SyntaxKind::EnumMarker;
		}
		self.text()
	}

	fn ref_marker(&mut self) -> SyntaxKind {
		self.s.eat_while(is_valid_in_label_literal);
		// Trailing dots and colons are likely prose.
		while matches!(self.s.scout(-1), Some('.' | ':')) {
			self.s.uneat();
		}
		SyntaxKind::RefMarker
	}

	fn label(&mut self) -> SyntaxKind {
		let label = self.s.eat_while(is_valid_in_label_literal);
		if label.is_empty() {
			return self.error("label cannot be empty".to_string());
		}
		if !self.s.eat_if('>') {
			return self.error("unclosed label".to_string());
		}
		SyntaxKind::Label
	}

	fn text(&mut self) -> SyntaxKind {
		loop {
			self.s.eat_until(|c: char| {
				if c.is_ascii() { TEXT_STOP[c as usize] } else { c.is_whitespace() }
			});
			// Carry on in the same text node where the stop char would become text anyway.
			let mut s = self.s;
			let go_on = match s.eat() {
				Some(' ')	=> s.at(char::is_alphanumeric),
				Some('/')	=> !s.at(['/', '*']),
				Some('-')	=> !s.at(['-', '?']),
				Some('.')	=> !s.at(".."),
				Some('h')	=> !s.at("ttp://") && !s.at("ttps://"),
				Some('@')	=> !s.at(is_valid_in_label_literal),
				_			=> false,
			};
			if !go_on {
				break;
			}
			self.s = s;
		}
		SyntaxKind::Text
	}

	/// Is the char just eaten flanked by word characters (outside the CJK scripts)?
	fn in_word(&self) -> bool {
		let wordy = |c: Option<char>| match c {
			Some(c)	=> c.is_alphanumeric() && !is_cjk(c),
			None	=> false,
		};
		wordy(self.s.scout(-2)) && wordy(self.s.peek())
	}

	fn space_or_end(&self) -> bool {
		self.s.done() || self.s.at(char::is_whitespace) || self.s.at("//") || self.s.at("/*")
	}
}

// The ASCII chars at which a markup text token stops.
const TEXT_STOP: [bool; 128] = {
	let mut t = [false; 128];
	let stops = b" \t\n\x0b\x0c\r\\/[]~-.'\"*_:h`$<>@#";
	let mut i = 0;
	while i < stops.len() {
		t[stops[i] as usize] = true;
		i += 1;
	}
	t
};

// Maths.
impl Lexer<'_> {
	fn math(&mut self, start: usize, c: char) -> (SyntaxKind, Option<SyntaxNode>) {
		let kind = match c {
			'\\'								=> self.backslash(),
			'"'									=> self.string(),

			'-' if self.s.eat_if(">>")			=> SyntaxKind::MathShorthand,
			'-' if self.s.eat_if('>')			=> SyntaxKind::MathShorthand,
			'-' if self.s.eat_if("->")			=> SyntaxKind::MathShorthand,
			':' if self.s.eat_if('=')			=> SyntaxKind::MathShorthand,
			':' if self.s.eat_if(":=")			=> SyntaxKind::MathShorthand,
			'!' if self.s.eat_if('=')			=> SyntaxKind::MathShorthand,
			'.' if self.s.eat_if("..")			=> SyntaxKind::MathShorthand,
			'<' if self.s.eat_if("==>")			=> SyntaxKind::MathShorthand,
			'<' if self.s.eat_if("-->")			=> SyntaxKind::MathShorthand,
			'<' if self.s.eat_if("--")			=> SyntaxKind::MathShorthand,
			'<' if self.s.eat_if("-<")			=> SyntaxKind::MathShorthand,
			'<' if self.s.eat_if("->")			=> SyntaxKind::MathShorthand,
			'<' if self.s.eat_if("<-")			=> SyntaxKind::MathShorthand,
			'<' if self.s.eat_if("<<")			=> SyntaxKind::MathShorthand,
			'<' if self.s.eat_if("=>")			=> SyntaxKind::MathShorthand,
			'<' if self.s.eat_if("==")			=> SyntaxKind::MathShorthand,
			'<' if self.s.eat_if("~~")			=> SyntaxKind::MathShorthand,
			'<' if self.s.eat_if('=')			=> SyntaxKind::MathShorthand,
			'<' if self.s.eat_if('<')			=> SyntaxKind::MathShorthand,
			'<' if self.s.eat_if('-')			=> SyntaxKind::MathShorthand,
			'<' if self.s.eat_if('~')			=> SyntaxKind::MathShorthand,
			'>' if self.s.eat_if("->")			=> SyntaxKind::MathShorthand,
			'>' if self.s.eat_if(">>")			=> SyntaxKind::MathShorthand,
			'=' if self.s.eat_if("=>")			=> SyntaxKind::MathShorthand,
			'=' if self.s.eat_if('>')			=> SyntaxKind::MathShorthand,
			'=' if self.s.eat_if(':')			=> SyntaxKind::MathShorthand,
			'>' if self.s.eat_if('=')			=> SyntaxKind::MathShorthand,
			'>' if self.s.eat_if('>')			=> SyntaxKind::MathShorthand,
			'|' if self.s.eat_if("->")			=> SyntaxKind::MathShorthand,
			'|' if self.s.eat_if("=>")			=> SyntaxKind::MathShorthand,
			'|' if self.s.eat_if('|')			=> SyntaxKind::MathShorthand,
			'~' if self.s.eat_if("~>")			=> SyntaxKind::MathShorthand,
			'~' if self.s.eat_if('>')			=> SyntaxKind::MathShorthand,
			'*' | '-' | '~'						=> SyntaxKind::MathShorthand,

			'.'									=> SyntaxKind::Dot,
			','									=> SyntaxKind::Comma,
			';'									=> SyntaxKind::Semicolon,

			'#'									=> SyntaxKind::Hash,
			'_'									=> SyntaxKind::Underscore,
			'$'									=> SyntaxKind::Dollar,
			'/'									=> SyntaxKind::Slash,
			'^'									=> SyntaxKind::Hat,
			'&'									=> SyntaxKind::MathAlignPoint,
			'√' | '∛' | '∜'						=> SyntaxKind::Root,
			'!'									=> SyntaxKind::Bang,

			'\''								=> {
				self.s.eat_while('\'');
				SyntaxKind::MathPrimes
			},

			// Delimiters lex as braces and parens; the parser turns them back into text.
			'('									=> SyntaxKind::LeftParen,
			')'									=> SyntaxKind::RightParen,
			'[' if self.s.eat_if('|')			=> SyntaxKind::LeftBrace,
			'|' if self.s.eat_if(']')			=> SyntaxKind::RightBrace,
			c if is_math_opening(c)				=> SyntaxKind::LeftBrace,
			c if is_math_closing(c)				=> SyntaxKind::RightBrace,

			c if is_math_id_start(c) && self.s.at(is_math_id_continue) => {
				self.s.eat_while(is_math_id_continue);
				let text = self.s.from(start);
				if grapheme_len(text) == text.len() {
					// One grapheme, a letter and its marks.
					SyntaxKind::MathText
				} else {
					let (kind, node) = self.math_ident_or_field(start);
					return (kind, Some(node));
				}
			},

			_									=> self.math_text(start, c),
		};
		(kind, None)
	}

	/// A `MathIdent`, or a whole `MathFieldAccess` chain when dots and identifiers follow.
	fn math_ident_or_field(&mut self, start: usize) -> (SyntaxKind, SyntaxNode) {
		let mut kind = SyntaxKind::MathIdent;
		let mut node = SyntaxNode::leaf(kind, self.s.from(start), Span::detached());
		while let Some(ident) = self.maybe_dot_ident() {
			kind = SyntaxKind::MathFieldAccess;
			node = SyntaxNode::inner(kind, vec![
				node,
				SyntaxNode::leaf(SyntaxKind::Dot, ".", Span::detached()),
				SyntaxNode::leaf(SyntaxKind::MathIdent, ident, Span::detached()),
			]);
		}
		(kind, node)
	}

	fn maybe_dot_ident(&mut self) -> Option<&str> {
		let next_starts = match self.s.scout(1) {
			Some(c)	=> is_math_id_start(c),
			None	=> false,
		};
		if next_starts && self.s.eat_if('.') {
			let start = self.s.cursor();
			self.s.eat();
			self.s.eat_while(is_math_id_continue);
			Some(self.s.from(start))
		} else {
			None
		}
	}

	fn math_text(&mut self, start: usize, c: char) -> SyntaxKind {
		if c.is_numeric() {
			// A number keeps its digits and one decimal part together.
			self.s.eat_while(char::is_numeric);
			let mut s = self.s;
			if s.eat_if('.') && !s.eat_while(char::is_numeric).is_empty() {
				self.s = s;
			}
		} else {
			let rest = self.s.get(start, self.s.string().len());
			self.s.jump(start + grapheme_len(rest));
		}
		SyntaxKind::MathText
	}

	/// A named argument's name, `name:` but not `:=` or `::=`, at `start`; the cursor is restored when
	/// there is none.
	pub fn maybe_math_named_arg(&mut self, start: usize) -> Option<SyntaxNode> {
		let cursor = self.s.cursor();
		self.s.jump(start);
		if self.s.eat_if(is_id_start) {
			self.s.eat_while(is_id_continue);
			if self.s.at(':') && !self.s.at(":=") && !self.s.at("::=") {
				let text = self.s.from(start);
				let node = if text != "_" {
					SyntaxNode::leaf(SyntaxKind::Ident, text, Span::detached())
				} else {
					SyntaxNode::error(text, "expected identifier, found underscore", Span::detached())
				};
				return Some(node);
			}
		}
		self.s.jump(cursor);
		None
	}

	/// A spread's `..` at `start`, unless it spreads nothing or is the `...` shorthand.
	pub fn maybe_math_spread_arg(&mut self, start: usize) -> Option<SyntaxNode> {
		let cursor = self.s.cursor();
		self.s.jump(start);
		if self.s.eat_if("..") && !self.space_or_end() && !self.s.at(['.', ',', ';', ')', '$']) {
			return Some(SyntaxNode::leaf(SyntaxKind::Dots, self.s.from(start), Span::detached()));
		}
		self.s.jump(cursor);
		None
	}
}

// Code.
impl Lexer<'_> {
	fn code(&mut self, start: usize, c: char) -> SyntaxKind {
		match c {
			'<' if self.s.at(is_id_continue)			=> self.label(),
			'0'..='9'									=> self.number(start, c),
			'.' if self.s.at(|c: char| c.is_ascii_digit())	=> self.number(start, c),
			'"'											=> self.string(),

			'=' if self.s.eat_if('=')					=> SyntaxKind::EqEq,
			'!' if self.s.eat_if('=')					=> SyntaxKind::ExclEq,
			'<' if self.s.eat_if('=')					=> SyntaxKind::LtEq,
			'>' if self.s.eat_if('=')					=> SyntaxKind::GtEq,
			'+' if self.s.eat_if('=')					=> SyntaxKind::PlusEq,
			'-' | '\u{2212}' if self.s.eat_if('=')		=> SyntaxKind::HyphEq,
			'*' if self.s.eat_if('=')					=> SyntaxKind::StarEq,
			'/' if self.s.eat_if('=')					=> SyntaxKind::SlashEq,
			'.' if self.s.eat_if('.')					=> SyntaxKind::Dots,
			'=' if self.s.eat_if('>')					=> SyntaxKind::Arrow,

			'{'											=> SyntaxKind::LeftBrace,
			'}'											=> SyntaxKind::RightBrace,
			'['											=> SyntaxKind::LeftBracket,
			']'											=> SyntaxKind::RightBracket,
			'('											=> SyntaxKind::LeftParen,
			')'											=> SyntaxKind::RightParen,
			'$'											=> SyntaxKind::Dollar,
			','											=> SyntaxKind::Comma,
			';'											=> SyntaxKind::Semicolon,
			':'											=> SyntaxKind::Colon,
			'.'											=> SyntaxKind::Dot,
			'+'											=> SyntaxKind::Plus,
			'-' | '\u{2212}'							=> SyntaxKind::Minus,
			'*'											=> SyntaxKind::Star,
			'/'											=> SyntaxKind::Slash,
			'='											=> SyntaxKind::Eq,
			'<'											=> SyntaxKind::Lt,
			'>'											=> SyntaxKind::Gt,

			c if is_id_start(c)							=> self.ident(start),

			c											=> self.invalid_char_in_code(c),
		}
	}

	// Typst's hints for the operators people bring from other languages.
	fn invalid_char_in_code(&mut self, c: char) -> SyntaxKind {
		let invalid_char = format!("the character `{}` is not valid in code", c);
		match c {
			_ if self.s.scout(-2) == Some('#')	=> {
				self.error(invalid_char);
				self.hint("the preceding hash is causing this to parse in code mode");
				self.hint("try escaping the preceding hash: `\\#`");
			},
			'#'									=> {
				self.error(invalid_char);
				self.hint("you are already in code mode");
				self.hint("try removing the `#`");
			},
			'&' if self.s.eat_if('&')			=> {
				self.error("`&&` is not valid in code".to_string());
				self.hint("in Typst, `and` is used for logical AND");
			},
			'|' if self.s.eat_if('|')			=> {
				self.error("`||` is not valid in code".to_string());
				self.hint("in Typst, `or` is used for logical OR");
			},
			'!'									=> {
				self.error(invalid_char);
				self.hint("in Typst, `not` is used for negation");
				self.hint("or did you mean to write `!=` for not-equal?");
			},
			'~' if self.s.eat_if('=')			=> {
				self.error("`~=` is not valid in code".to_string());
				self.hint("in Typst, `!=` is used for not-equal");
			},
			_									=> {
				self.error(invalid_char);
			},
		}
		SyntaxKind::Error
	}

	fn ident(&mut self, start: usize) -> SyntaxKind {
		self.s.eat_while(is_id_continue);
		let ident = self.s.from(start);
		let prev = self.s.get(0, start);
		// After a dot or an at sign the word is a field or a reference, never a keyword.
		if !prev.ends_with(['.', '@']) || prev.ends_with("..") {
			if let Some(k) = keyword(ident) {
				return k;
			}
		}
		if ident == "_" { SyntaxKind::Underscore } else { SyntaxKind::Ident }
	}

	/// An integer, a float or a numeric with a unit; only base-ten numbers may carry a unit.
	fn number(&mut self, start: usize, first: char) -> SyntaxKind {
		let base = match first {
			'0' if self.s.eat_if('b')	=> 2,
			'0' if self.s.eat_if('o')	=> 8,
			'0' if self.s.eat_if('x')	=> 16,
			_							=> 10,
		};

		if base == 16 {
			self.s.eat_while(|c: char| c.is_ascii_alphanumeric());
		} else {
			self.s.eat_while(|c: char| c.is_ascii_digit());
		}

		let mut is_float = false;
		if base == 10 {
			// Not a spread (`1..`) and not a method call (`1.abs`).
			if first == '.' {
				is_float = true;
			} else {
				let id_follows = match self.s.scout(1) {
					Some(c)	=> is_id_start(c),
					None	=> false,
				};
				if !self.s.at("..") && !id_follows && self.s.eat_if('.') {
					is_float = true;
					self.s.eat_while(|c: char| c.is_ascii_digit());
				}
			}
			if !self.s.at("em") && self.s.eat_if(['e', 'E']) {
				is_float = true;
				self.s.eat_if(['+', '-']);
				self.s.eat_while(|c: char| c.is_ascii_digit());
			}
		}

		let number = self.s.from(start);
		let suffix = self.s.eat_while(|c: char| c.is_ascii_alphanumeric() || c == '%');

		// A base-ten integer too large for i64 reads as a float.
		if base == 10 && !is_float && number.parse::<i64>().is_err() && number.parse::<f64>().is_ok() {
			let overflow = number.chars().all(|c| c.is_ascii_digit()) && !number.is_empty();
			if overflow {
				is_float = true;
			}
		}

		let mut suffix_res: Result<bool, String> = match suffix {
			""																	=> Ok(false),
			"pt" | "mm" | "cm" | "in" | "deg" | "rad" | "em" | "fr" | "%"		=> Ok(true),
			_																	=>
				Err(format!("invalid number suffix: `{}`", suffix)),
		};

		let number_res: Result<(), String> = if is_float && number.parse::<f64>().is_err() {
			Err(format!("invalid floating point number: `{}`", number))
		} else if base == 10 {
			Ok(())
		} else {
			let name = match base {
				2	=> "binary",
				8	=> "octal",
				_	=> "hexadecimal",
			};
			let digits = number.get(2..).unwrap_or("");
			match i64::from_str_radix(digits, base) {
				Ok(_) if suffix.is_empty()	=> Ok(()),
				Ok(value)					=> {
					if suffix_res.is_ok() {
						suffix_res = Err(format!("try using a decimal number: `{}{}`", value, suffix));
					}
					Err(format!("{} numbers cannot have a suffix", name))
				},
				Err(_) if digits.is_empty()	=>
					Err(format!("expected a{} {} number", if base == 8 { "n" } else { "" }, name)),
				Err(_)						=> Err(format!("invalid {} number: `{}`", name, number)),
			}
		};

		match (number_res, suffix_res) {
			(Ok(()), Ok(false)) if is_float	=> SyntaxKind::Float,
			(Ok(()), Ok(false))				=> SyntaxKind::Int,
			(Ok(()), Ok(true))				=> SyntaxKind::Numeric,
			(Err(n), Err(s))				=> {
				let k = self.error(n);
				self.hint(&s);
				k
			},
			(Ok(()), Err(m))				=> self.error(m),
			(Err(m), Ok(_))					=> self.error(m),
		}
	}

	fn string(&mut self) -> SyntaxKind {
		let mut escaped = false;
		self.s.eat_until(|c: char| {
			let stop = c == '"' && !escaped;
			escaped = c == '\\' && !escaped;
			stop
		});
		if !self.s.eat_if('"') {
			return self.error("unclosed string".to_string());
		}
		SyntaxKind::Str
	}
}

fn keyword(ident: &str) -> Option<SyntaxKind> {
	Some(match ident {
		"none"		=> SyntaxKind::None,
		"auto"		=> SyntaxKind::Auto,
		"true"		=> SyntaxKind::Bool,
		"false"		=> SyntaxKind::Bool,
		"not"		=> SyntaxKind::Not,
		"and"		=> SyntaxKind::And,
		"or"		=> SyntaxKind::Or,
		"let"		=> SyntaxKind::Let,
		"set"		=> SyntaxKind::Set,
		"show"		=> SyntaxKind::Show,
		"context"	=> SyntaxKind::Context,
		"if"		=> SyntaxKind::If,
		"else"		=> SyntaxKind::Else,
		"for"		=> SyntaxKind::For,
		"in"		=> SyntaxKind::In,
		"while"		=> SyntaxKind::While,
		"break"		=> SyntaxKind::Break,
		"continue"	=> SyntaxKind::Continue,
		"return"	=> SyntaxKind::Return,
		"import"	=> SyntaxKind::Import,
		"include"	=> SyntaxKind::Include,
		"as"		=> SyntaxKind::As,
		_			=> return None,
	})
}

// ┌───────────────────────────────────────────────────────────────────────────────────────────┐
// │ Character classes                                                                         │
// └───────────────────────────────────────────────────────────────────────────────────────────┘

fn in_table(table: &[(u32, u32)], c: char) -> bool {
	let c = c as u32;
	let i = table.partition_point(|(a, _)| *a <= c);
	match i.checked_sub(1).and_then(|i| table.get(i)) {
		Some((_, b))	=> c <= *b,
		None			=> false,
	}
}

fn is_space(c: char, mode: LexMode) -> bool {
	match mode {
		LexMode::Markup	=> matches!(c, ' ' | '\t') || is_newline(c),
		_				=> c.is_whitespace(),
	}
}

/// Is the char one Typst reads as a newline (LF, VT, FF, CR, NEL, LS, PS)?
pub fn is_newline(c: char) -> bool {
	matches!(c, '\n' | '\x0B' | '\x0C' | '\r' | '\u{0085}' | '\u{2028}' | '\u{2029}')
}

pub fn is_xid_start(c: char) -> bool {
	if c.is_ascii() { c.is_ascii_alphabetic() } else { in_table(unicode::XID_START, c) }
}

pub fn is_xid_continue(c: char) -> bool {
	if c.is_ascii() { c.is_ascii_alphanumeric() || c == '_' } else { in_table(unicode::XID_CONTINUE, c) }
}

/// Can the char start an identifier? XID_Start plus the underscore.
pub fn is_id_start(c: char) -> bool { is_xid_start(c) || c == '_' }

/// Can the char continue an identifier? XID_Continue plus the underscore and the hyphen.
pub fn is_id_continue(c: char) -> bool { is_xid_continue(c) || c == '_' || c == '-' }

/// Is the string a valid Typst identifier?
pub fn is_ident(s: &str) -> bool {
	let mut chars = s.chars();
	match chars.next() {
		Some(c)	=> is_id_start(c) && chars.all(is_id_continue),
		None	=> false,
	}
}

fn is_math_id_start(c: char) -> bool { is_xid_start(c) }

fn is_math_id_continue(c: char) -> bool { is_xid_continue(c) && c != '_' }

fn is_valid_in_label_literal(c: char) -> bool { is_id_continue(c) || matches!(c, ':' | '.') }

/// Is the string usable as the name inside a `<label>` literal?
pub fn is_valid_label_literal_id(id: &str) -> bool {
	!id.is_empty() && id.chars().all(is_valid_in_label_literal)
}

/// Is the char in the Han, Hiragana, Katakana or Hangul script, where `*` and `_` never sit inside a word?
fn is_cjk(c: char) -> bool { !c.is_ascii() && in_table(unicode::CJK, c) }

pub fn is_math_opening(c: char) -> bool { in_table(unicode::MATH_OPENING, c) }

pub fn is_math_closing(c: char) -> bool { in_table(unicode::MATH_CLOSING, c) }

/// Does the char have the Unicode maths class Alphabetic? Typst's own class overrides touch no letter.
pub fn is_math_alphabetic_class(c: char) -> bool { in_table(unicode::MATH_ALPHABETIC, c) }

/// The byte length of the first extended grapheme cluster of `s`.
pub fn grapheme_len(s: &str) -> usize {
	let mut chars = s.char_indices();
	let first = match chars.next() {
		Some((_, c))	=> c,
		None			=> return 0,
	};
	// A cluster always ends between two ASCII chars other than CR LF, so the search needs only the text up
	// to the first such pair; that keeps it linear over a long line of maths.
	let mut prev = first;
	let mut cut = s.len();
	for (i, c) in chars {
		if prev.is_ascii() && c.is_ascii() && !(prev == '\r' && c == '\n') {
			cut = i;
			break;
		}
		prev = c;
	}
	match s.get(..cut) {
		Some(head)	=> segment::next_grapheme(head, 0),
		None		=> first.len_utf8(),
	}
}

/// The prefix of `text` that an automatic link takes, and whether its brackets balance. Trailing
/// punctuation likely to be prose is left out.
pub fn link_prefix(text: &str) -> (&str, bool) {
	let mut s = Scanner::new(text);
	let mut brackets: Vec<u8> = Vec::new();
	s.eat_while(|c: char| match c {
		'0'..='9' | 'a'..='z' | 'A'..='Z'
		| '!' | '#' | '$' | '%' | '&' | '*' | '+'
		| ',' | '-' | '.' | '/' | ':' | ';' | '='
		| '?' | '@' | '_' | '~' | '\''				=> true,
		'['											=> { brackets.push(b'['); true },
		'('											=> { brackets.push(b'('); true },
		']'											=> brackets.pop() == Some(b'['),
		')'											=> brackets.pop() == Some(b'('),
		_											=> false,
	});
	while matches!(s.scout(-1), Some('!' | ',' | '.' | ':' | ';' | '?' | '\'')) {
		s.uneat();
	}
	(s.before(), brackets.is_empty())
}

/// Splits at newlines, a CR LF pair counting as one; the newlines are dropped.
pub fn split_newlines(text: &str) -> Vec<&str> {
	let mut s = Scanner::new(text);
	let mut lines = Vec::new();
	let mut start = 0;
	let mut end = 0;
	while let Some(c) = s.eat() {
		if is_newline(c) {
			if c == '\r' {
				s.eat_if('\n');
			}
			lines.push(s.get(start, end));
			start = s.cursor();
		}
		end = s.cursor();
	}
	lines.push(s.get(start, text.len()));
	lines
}

fn count_newlines(text: &str) -> usize {
	let mut n = 0;
	let mut s = Scanner::new(text);
	while let Some(c) = s.eat() {
		if is_newline(c) {
			if c == '\r' {
				s.eat_if('\n');
			}
			n += 1;
		}
	}
	n
}
