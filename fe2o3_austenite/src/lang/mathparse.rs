//! A parser for Typst's inline maths syntax into the engine's [`Atom`](crate::math::Atom) tree.
//!
//! The engine sets a rich mathematics already; this is the front end that reads `$...$` the way an
//! author writes it in Typst and hands the layout an `Atom`. The subset covered is the common one: a
//! variable is a letter, a number a run of digits; `^` and `_` attach a superscript and a subscript,
//! grouped with `(...)` or `{...}`; `/` and `frac(a, b)` make a fraction; `sqrt(x)` a radical; `(...)`
//! grows a fence; a run of letters is looked up in a table of Greek letters and common operators
//! (`alpha`, `sum`, `times`, `->`) or, failing that, set as an identifier. Beyond that: the style
//! alphabets `cal`/`bold`/`bb`/`frak`/`sans`/`mono`/`upright`; the accents `dot`/`hat`/`tilde`/`bar`
//! and the rules `overline`/`underline`, and the explicit `accent(base, mark)`; a quoted `"..."` and a
//! `text("...")` as an upright run; the spacing words `thin`/`med`/`thick`/`quad`/`wide`/`space` and an
//! `#h(..em)`; a dotted symbol name (`arrow.r`, `plus.minus`, `dot.op`); the grids `mat`, `cases` and
//! `vec`; and a multi-line alignment block broken at `\` and aligned at `&`. An unknown control word is
//! set as itself rather than rejected, so a document still sets.

use crate::math::{
	Accent,
	Atom,
	Class,
	MatKind,
};

use oxedyne_fe2o3_core::prelude::*;

/// Parses one maths expression (the text between the `$` delimiters) into an [`Atom`].
pub fn parse(src: &str) -> Outcome<Atom> {
	let mut p = Parser { chars: src.chars().collect(), i: 0 };
	let atom = res!(p.aligned());
	Ok(atom)
}

struct Parser {
	chars:	Vec<char>,
	i:		usize,
}

impl Parser {
	fn peek(&self) -> Option<char> { self.chars.get(self.i).copied() }
	fn peek2(&self) -> Option<char> { self.chars.get(self.i + 1).copied() }
	fn at(&self, k: usize) -> Option<char> { self.chars.get(k).copied() }

	fn skip_ws(&mut self) {
		while matches!(self.peek(), Some(c) if c.is_whitespace()) {
			self.i += 1;
		}
	}

	fn eat(&mut self, c: char) -> bool {
		if self.peek() == Some(c) {
			self.i += 1;
			true
		} else {
			false
		}
	}

	/// Is the cursor at a `\` line break -- a backslash at the end of input or before whitespace, as
	/// distinct from a `\,` or `\/` escape whose backslash is followed by the character it escapes?
	fn is_linebreak(&self) -> bool {
		if self.peek() != Some('\\') {
			return false;
		}
		match self.peek2() {
			None		=> true,
			Some(c)		=> c.is_whitespace(),
		}
	}

	/// The whole expression, as a sequence of cells separated by `&` and rows separated by a `\` line
	/// break. A single cell in a single row unwraps to that atom -- the common case of one expression --
	/// while any `&` or break yields an alignment [`Atom::Matrix`] the display layout stacks and aligns.
	fn aligned(&mut self) -> Outcome<Atom> {
		let mut rows:	Vec<Vec<Atom>>	= Vec::new();
		let mut cells:	Vec<Atom>		= Vec::new();
		loop {
			let cell = res!(self.row(&['&']));
			cells.push(cell);
			self.skip_ws();
			match self.peek() {
				Some('&') => { self.i += 1; },
				_ if self.is_linebreak() => {
					self.i += 1;			// consume the backslash
					rows.push(std::mem::take(&mut cells));
					self.skip_ws();
					if self.peek().is_none() {
						break;				// a trailing break: no empty final row
					}
				},
				_ => { rows.push(std::mem::take(&mut cells)); break; },
			}
		}
		if rows.len() == 1 && rows[0].len() == 1 {
			return Ok(rows.pop().and_then(|mut r| r.pop()).unwrap_or_else(|| Atom::row(Vec::new())));
		}
		Ok(Atom::matrix(rows, None, None, MatKind::Align))
	}

	/// A sequence of factors up to end of input or a stop character, folding `/` into fractions. A row of
	/// one element unwraps to that element, so a bare `x` is a symbol, not a one-item row. A `\` line
	/// break stops the row, left for the caller to consume.
	fn row(&mut self, stop: &[char]) -> Outcome<Atom> {
		let mut items: Vec<Atom> = Vec::new();
		loop {
			self.skip_ws();
			if self.is_linebreak() {
				break;
			}
			match self.peek() {
				None => break,
				Some(c) if stop.contains(&c) => break,
				_ => {},
			}
			if self.peek() == Some('/') {
				// A fraction of the preceding factor and the next one. Parentheses directly around either
				// part are grouping, not literal, so Typst hides them -- `(a + b)/(c)` stacks without
				// parentheses -- and they are unwrapped here.
				self.i += 1;
				self.skip_ws();
				let den = ungroup(res!(self.factor(stop)));
				// A postfix that binds tighter than the fraction -- a factorial or a prime -- stays with the
				// denominator: `e^x / k!` sets `k!` under the bar, not the `!` beside it.
				let den = self.attach_postfix(den);
				let num = ungroup(items.pop().unwrap_or_else(|| Atom::row(Vec::new())));
				items.push(Atom::frac(num, den));
				continue;
			}
			items.push(res!(self.factor(stop)));
		}
		if items.len() == 1 {
			Ok(items.pop().unwrap_or_else(|| Atom::row(Vec::new())))
		} else {
			Ok(Atom::row(items))
		}
	}

	/// A base atom with any superscript and subscript that follow it, in either order.
	fn factor(&mut self, stop: &[char]) -> Outcome<Atom> {
		let base = res!(self.atom(stop));
		let mut sup: Option<Atom> = None;
		let mut sub: Option<Atom> = None;
		loop {
			self.skip_ws();
			match self.peek() {
				Some('^') => { self.i += 1; sup = Some(res!(self.script_arg(stop))); },
				Some('_') => { self.i += 1; sub = Some(res!(self.script_arg(stop))); },
				_ => break,
			}
		}
		Ok(match (sub, sup) {
			(None, None)			=> base,
			(Some(sb), None)		=> Atom::sub(base, sb),
			(None, Some(sp))		=> Atom::sup(base, sp),
			(Some(sb), Some(sp))	=> Atom::subsup(base, sb, sp),
		})
	}

	/// Appends any immediately following postfix marks -- a factorial `!` or a prime `'` -- to an atom, so
	/// a fraction denominator keeps a `k!` whole. Whitespace ends the run.
	fn attach_postfix(&mut self, atom: Atom) -> Atom {
		let mut items = vec![atom];
		loop {
			match self.peek() {
				Some('!')	=> { self.i += 1; items.push(Atom::sym("!", Class::Close)); },
				Some('\'')	=> { self.i += 1; items.push(Atom::sym("\u{2032}", Class::Ord)); },
				_			=> break,
			}
		}
		if items.len() == 1 {
			items.pop().unwrap_or_else(|| Atom::row(Vec::new()))
		} else {
			Atom::row(items)
		}
	}

	/// The operand of a `^` or `_`: a parenthesised or braced group, or a single atom.
	fn script_arg(&mut self, stop: &[char]) -> Outcome<Atom> {
		self.skip_ws();
		match self.peek() {
			Some('(') => { self.i += 1; let r = res!(self.row(&[')'])); self.eat(')'); Ok(r) },
			Some('{') => { self.i += 1; let r = res!(self.row(&['}'])); self.eat('}'); Ok(r) },
			_ => self.atom(stop),
		}
	}

	fn atom(&mut self, _stop: &[char]) -> Outcome<Atom> {
		self.skip_ws();
		let c = match self.peek() {
			Some(c)	=> c,
			None	=> return Ok(Atom::row(Vec::new())),
		};

		// A backslash escapes the next character, set literally: `\,` a comma, `\/` a plain slash (not a
		// fraction). A backslash before whitespace is a line break, handled by the row, not here.
		if c == '\\' {
			self.i += 1;
			return Ok(match self.peek() {
				Some(',')	=> { self.i += 1; Atom::sym(",", Class::Punct) },
				Some('/')	=> { self.i += 1; Atom::sym("/", Class::Ord) },
				Some(ch)	=> { self.i += 1; Atom::sym(ch.to_string(), Class::Ord) },
				None		=> Atom::row(Vec::new()),
			});
		}

		// A code escape inside maths: `#h(..)` is explicit spacing, anything else (`#150%`, `#move(..)`)
		// is dropped to nothing so a stray call does not derail the row.
		if c == '#' {
			return Ok(self.hash());
		}

		// A quoted string is an upright roman run. Typst keeps a space between such a run and a following
		// word (`"if" x`), where it drops one between bare symbols, so a space that separates the run from
		// a following letter or digit is preserved as a thin space.
		if c == '"' {
			self.i += 1;
			let s = self.take_while(|c| c != '"');
			self.eat('"');
			let spaced = self.peek() == Some(' ')
				&& matches!(self.peek_nonws(), Some(n) if n.is_alphanumeric());
			if spaced {
				return Ok(Atom::row(vec![Atom::text(s), Atom::space(220)]));
			}
			return Ok(Atom::text(s));
		}

		// A parenthesised group grows a fence around its row.
		if c == '(' {
			self.i += 1;
			let body = res!(self.row(&[')']));
			self.eat(')');
			return Ok(Atom::fence('(', body, ')'));
		}
		if c == '{' {
			self.i += 1;
			let body = res!(self.row(&['}']));
			self.eat('}');
			return Ok(body);
		}

		// A number: a run of digits and dots.
		if c.is_ascii_digit() {
			let n = self.take_while(|c| c.is_ascii_digit() || c == '.');
			return Ok(Atom::num(n));
		}

		// A word: a function, a named symbol (possibly dotted), or an identifier.
		if c.is_alphabetic() {
			let word = self.take_while(|c| c.is_alphabetic());
			return self.word(&word);
		}

		// An operator, possibly two characters (`<=`, `->`).
		Ok(self.operator())
	}

	/// A `#...` code escape inside maths. `#h(1em)` or `#h(0.5em)` becomes a fixed space; `#h(..)` in
	/// other units, and any other call or literal, is dropped to an empty row so it takes no space.
	fn hash(&mut self) -> Atom {
		self.i += 1;		// the '#'
		// `#h(<number>em)` -> a space of that many em.
		if self.peek() == Some('h') && self.peek2() == Some('(') {
			self.i += 2;
			let num = self.take_while(|c| c.is_ascii_digit() || c == '.' || c == '-');
			let unit = self.take_while(|c| c.is_alphabetic() || c == '%');
			// consume to the closing ')'
			while matches!(self.peek(), Some(c) if c != ')') {
				self.i += 1;
			}
			self.eat(')');
			if unit == "em" {
				let em: f64 = num.parse().unwrap_or(0.0);
				return Atom::space((em * 1000.0) as i32);
			}
			return Atom::row(Vec::new());
		}
		// Any other `#name`, `#name(...)`, `#123%`: consume the token and drop it.
		let _ = self.take_while(|c| c.is_alphanumeric() || c == '.' || c == '%');
		if self.peek() == Some('(') {
			self.skip_balanced();
		}
		Atom::row(Vec::new())
	}

	/// Consumes a balanced `(...)` from the opening parenthesis, so a dropped call takes its whole
	/// argument list with it.
	fn skip_balanced(&mut self) {
		if !self.eat('(') {
			return;
		}
		let mut depth = 1usize;
		while depth > 0 {
			match self.peek() {
				Some('(')	=> depth += 1,
				Some(')')	=> depth -= 1,
				None		=> break,
				_			=> {},
			}
			self.i += 1;
		}
	}

	/// A run of letters, resolved as a function, a named symbol or an identifier. Functions read their
	/// bracketed arguments; a name in the symbol table (perhaps extended by a dotted tail) becomes that
	/// symbol; anything else is set as an identifier, a single letter as an italic variable.
	fn word(&mut self, word: &str) -> Outcome<Atom> {
		// A style alphabet: restyle the argument's letters.
		if let Some(alpha) = alphabet(word) {
			let arg = res!(self.one_arg());
			return Ok(restyle(&arg, alpha));
		}

		match word {
			"frac" => {
				let (a, b) = res!(self.two_args());
				return Ok(Atom::frac(a, b));
			},
			"sqrt" => {
				let a = res!(self.one_arg());
				return Ok(Atom::sqrt(a));
			},
			"root" => {
				// root(index, radicand): the index is not yet drawn, so the radicand alone is set.
				let (_, b) = res!(self.two_args());
				return Ok(Atom::sqrt(b));
			},
			"text" => {
				// text("..."): the quoted argument as an upright run.
				let a = res!(self.one_arg());
				return Ok(a);		// one_arg parses the "..." to an Atom::Text already
			},
			"lr" => {
				// lr(content, size: ..): the auto-sized delimiters are already the content's own fence, so
				// the content is kept and the size argument dropped.
				if self.peek_nonws() == Some('(') {
					self.skip_ws();
					self.i += 1;
					let inner = res!(self.row(&[',', ')']));
					self.skip_commas_to_close();
					return Ok(inner);
				}
			},
			"mat" => return self.grid('(', Some(')'), ';', MatKind::Matrix),
			"vec" => return self.grid('(', Some(')'), ',', MatKind::Matrix),
			"cases" => return self.grid('{', None, ',', MatKind::Cases),
			"overline" => return Ok(Atom::accent(res!(self.one_arg()), Accent::OverRule)),
			"underline" => return Ok(Atom::accent(res!(self.one_arg()), Accent::UnderRule)),
			"accent" => {
				// accent(base, mark): the second argument names the accent, read as a bare word.
				self.skip_ws();
				if self.eat('(') {
					let base = res!(self.row(&[',', ')']));
					self.eat(',');
					self.skip_ws();
					let mark = self.take_while(|c| c.is_alphabetic() || c == '.');
					self.skip_commas_to_close();
					let a = accent_of(&mark).unwrap_or(Accent::Over("\u{02D9}".to_string()));
					return Ok(Atom::accent(base, a));
				}
				return Ok(symbol("accent"));
			},
			// The `dif` of a differential: a thin space and an upright `d`.
			"dif" => return Ok(Atom::row(vec![Atom::space(167), Atom::text("d")])),
			// The custom tight scientific-notation cross reads as a multiplication sign.
			"ttimes" => return Ok(Atom::sym("\u{00D7}", Class::Bin)),
			_ => {},
		}

		// A glyph accent taken as a function when an argument follows: `dot(x)`, `hat(y)`, `bar(z)`.
		if self.peek_nonws() == Some('(') {
			if let Some(mark) = accent_of(word) {
				return Ok(Atom::accent(res!(self.one_arg()), mark));
			}
		}

		// A spacing word.
		if let Some(mem) = spacing(word) {
			return Ok(Atom::space(mem));
		}

		// A dotted symbol name, `arrow.r`, `plus.minus`, resolved to the longest matching name.
		if self.peek() == Some('.') {
			if let Some(atom) = self.dotted_word(word) {
				return Ok(atom);
			}
		}

		Ok(symbol(word))
	}

	/// The next non-whitespace character, without consuming it.
	fn peek_nonws(&self) -> Option<char> {
		let mut k = self.i;
		while matches!(self.at(k), Some(c) if c.is_whitespace()) {
			k += 1;
		}
		self.at(k)
	}

	/// Resolves a dotted symbol name from a base word already read and a `.` at the cursor. Reads the
	/// dotted segments without committing, then matches the longest `word.seg.seg` in the dotted table,
	/// consuming exactly that; returns `None` (consuming nothing) when no dotted name matches, so the
	/// base word falls through to ordinary handling and the `.` stays as punctuation.
	fn dotted_word(&mut self, word: &str) -> Option<Atom> {
		// Probe the extension segments and the index just past each.
		let mut ends:	Vec<usize>	= Vec::new();		// index after segment k
		let mut names:	Vec<String>	= Vec::new();		// "word.seg1...segk"
		let mut probe				= self.i;
		let mut acc					= word.to_string();
		while self.at(probe) == Some('.') {
			let mut j = probe + 1;
			while matches!(self.at(j), Some(c) if c.is_alphabetic()) {
				j += 1;
			}
			if j == probe + 1 {
				break;		// a lone dot, no segment: punctuation
			}
			let seg: String = self.chars[probe + 1..j].iter().collect();
			acc.push('.');
			acc.push_str(&seg);
			names.push(acc.clone());
			ends.push(j);
			probe = j;
		}
		// Longest match first.
		for k in (0..names.len()).rev() {
			if let Some((s, class)) = dotted(&names[k]) {
				self.i = ends[k];
				return Some(Atom::sym(s, class));
			}
		}
		None
	}

	/// A grid function's body: `mat`/`vec` (rows by `;` or `,`), `cases` (rows by `,`, cells by `&`).
	/// The row separator is given; a cell is separated by `&` for a `cases`, else the grid has one cell
	/// per row. A trailing separator before the close does not open an empty final row.
	fn grid(&mut self, left: char, right: Option<char>, row_sep: char, kind: MatKind) -> Outcome<Atom> {
		if self.peek_nonws() != Some('(') {
			return Ok(symbol("mat"));		// not a call after all
		}
		self.skip_ws();
		self.i += 1;						// the '('
		let cell_sep = if kind == MatKind::Cases { '&' } else { row_sep };
		let stops = [cell_sep, row_sep, ')'];
		let mut rows:	Vec<Vec<Atom>>	= Vec::new();
		let mut cells:	Vec<Atom>		= Vec::new();
		loop {
			self.skip_ws();
			if self.peek() == Some(')') || self.peek().is_none() {
				break;
			}
			let cell = res!(self.row(&stops));
			cells.push(cell);
			self.skip_ws();
			match self.peek() {
				Some(c) if c == cell_sep && cell_sep != row_sep => { self.i += 1; },
				Some(c) if c == row_sep => {
					self.i += 1;
					rows.push(std::mem::take(&mut cells));
				},
				_ => break,
			}
		}
		if !cells.is_empty() {
			rows.push(cells);
		}
		self.skip_ws();
		self.eat(')');
		Ok(Atom::matrix(rows, Some(left), right, kind))
	}

	/// The single bracketed argument of a function: `(row)`.
	fn one_arg(&mut self) -> Outcome<Atom> {
		self.skip_ws();
		if !self.eat('(') {
			return Ok(Atom::row(Vec::new()));
		}
		let a = res!(self.row(&[')', ',']));
		self.skip_commas_to_close();
		Ok(a)
	}

	/// The two comma-separated bracketed arguments of a function: `(row, row)`.
	fn two_args(&mut self) -> Outcome<(Atom, Atom)> {
		self.skip_ws();
		if !self.eat('(') {
			return Ok((Atom::row(Vec::new()), Atom::row(Vec::new())));
		}
		let a = res!(self.row(&[',', ')']));
		self.eat(',');
		let b = res!(self.row(&[')', ',']));
		self.skip_commas_to_close();
		Ok((a, b))
	}

	/// Consumes any trailing arguments and the closing `)`, so an extra argument does not derail the row.
	fn skip_commas_to_close(&mut self) {
		loop {
			self.skip_ws();
			match self.peek() {
				Some(')')	=> { self.i += 1; break; },
				Some(',')	=> { self.i += 1; let _ = self.row(&[',', ')']); },
				None		=> break,
				_			=> break,
			}
		}
	}

	/// One or two operator characters, mapped to a symbol of the right spacing class.
	fn operator(&mut self) -> Atom {
		let a = self.peek().unwrap_or(' ');
		let b = self.peek2();
		// Two-character operators first.
		if let Some(b) = b {
			let pair = [a, b];
			let two: Option<(&str, Class)> = match pair {
				['<', '=']	=> Some(("\u{2264}", Class::Rel)),	// <=
				['>', '=']	=> Some(("\u{2265}", Class::Rel)),	// >=
				['!', '=']	=> Some(("\u{2260}", Class::Rel)),	// !=
				['-', '>']	=> Some(("\u{2192}", Class::Rel)),	// ->
				['=', '>']	=> Some(("\u{21D2}", Class::Rel)),	// =>
				['<', '-']	=> Some(("\u{2190}", Class::Rel)),	// <-
				_			=> None,
			};
			if let Some((s, class)) = two {
				self.i += 2;
				return Atom::sym(s, class);
			}
		}
		self.i += 1;
		let (s, class): (String, Class) = match a {
			'+'	=> ("+".to_string(), Class::Bin),
			'-'	=> ("\u{2212}".to_string(), Class::Bin),	// a true minus sign
			'*'	=> ("\u{22C5}".to_string(), Class::Bin),	// a centred dot for multiplication
			'='	=> ("=".to_string(), Class::Rel),
			'<'	=> ("<".to_string(), Class::Rel),
			'>'	=> (">".to_string(), Class::Rel),
			','	=> (",".to_string(), Class::Punct),
			'.'	=> (".".to_string(), Class::Punct),
			'!'	=> ("!".to_string(), Class::Close),
			_	=> (a.to_string(), Class::Ord),
		};
		Atom::sym(s, class)
	}

	fn take_while(&mut self, pred: impl Fn(char) -> bool) -> String {
		let start = self.i;
		while matches!(self.peek(), Some(c) if pred(c)) {
			self.i += 1;
		}
		self.chars[start..self.i].iter().collect()
	}
}

/// Unwraps a round-parenthesis fence to its body, used on a fraction's parts where Typst treats such
/// parentheses as grouping and hides them. Anything else is returned unchanged.
fn ungroup(atom: Atom) -> Atom {
	match atom {
		Atom::Fence { left: '(', body, right: ')' } => *body,
		other => other,
	}
}

/// Resolves a control word to a symbol: a Greek letter, a common operator or relation by name, or --
/// when the name is unknown -- an identifier set as itself (a single letter as an italic variable, a
/// longer word upright, the way Typst sets a known multi-letter operator like `sin`).
fn symbol(word: &str) -> Atom {
	if let Some((s, class)) = named(word) {
		return Atom::sym(s, class);
	}
	if word.chars().count() == 1 {
		Atom::var(word)		// a lone letter is an italic variable
	} else {
		Atom::op(word)		// an unknown multi-letter word, set upright like an operator name
	}
}

/// A style-alphabet function name mapped to its alphabet, or `None` when the word is not one.
fn alphabet(word: &str) -> Option<Alphabet> {
	Some(match word {
		"cal"		=> Alphabet::Cal,
		"bold"		=> Alphabet::BoldItalic,
		"bb"		=> Alphabet::Bb,
		"frak"		=> Alphabet::Frak,
		"sans"		=> Alphabet::Sans,
		"mono"		=> Alphabet::Mono,
		"upright"	=> Alphabet::Upright,
		_			=> return None,
	})
}

/// The maths style alphabets Typst offers, each a remapping of the Latin letters (and, for some, the
/// digits) to a Unicode mathematical-alphanumeric block.
#[derive(Clone, Copy)]
enum Alphabet {
	Cal,			// script / calligraphic
	BoldItalic,		// Typst's `bold` of a variable is bold italic
	Bb,				// blackboard bold
	Frak,			// Fraktur
	Sans,			// sans serif
	Mono,			// monospace
	Upright,		// roman upright (drops the default italic)
}

/// Restyles an atom's letters to a maths style alphabet, recursing through rows, scripts, fractions,
/// fences and accents. A single ASCII letter (or, for the alphabets that carry them, a digit) is
/// remapped to the alphabet's codepoint and set as an upright run so the layout shapes it as drawn
/// rather than remapping it a second time to the maths italic. Symbols the alphabet does not cover --
/// Greek, operators -- are left as they are.
fn restyle(atom: &Atom, alpha: Alphabet) -> Atom {
	match atom {
		Atom::Sym(s, Class::Ord) => {
			let mut it = s.chars();
			if let (Some(c), None) = (it.next(), it.next()) {
				if let Some(m) = styled_char(c, alpha) {
					return Atom::text(m.to_string());
				}
			}
			atom.clone()
		},
		Atom::Row(items)	=> Atom::row(items.iter().map(|a| restyle(a, alpha)).collect()),
		Atom::Frac { num, den } => Atom::frac(restyle(num, alpha), restyle(den, alpha)),
		Atom::Script { base, sup, sub } => Atom::Script {
			base:	Box::new(restyle(base, alpha)),
			sup:	sup.as_ref().map(|a| Box::new(restyle(a, alpha))),
			sub:	sub.as_ref().map(|a| Box::new(restyle(a, alpha))),
		},
		Atom::Fence { left, body, right } => Atom::fence(*left, restyle(body, alpha), *right),
		Atom::Accent { base, mark }	=> Atom::accent(restyle(base, alpha), mark.clone()),
		other => other.clone(),
	}
}

/// The mathematical-alphanumeric codepoint of an ASCII letter (or digit) in a style alphabet, or `None`
/// where the alphabet does not cover the character. The script, Fraktur and blackboard alphabets have
/// holes in their Unicode blocks that the letterlike block fills; those are given explicitly.
fn styled_char(c: char, alpha: Alphabet) -> Option<char> {
	let up = c.is_ascii_uppercase();
	let lo = c.is_ascii_lowercase();
	let digit = c.is_ascii_digit();
	let i = |base: u32, first: char| base + (c as u32 - first as u32);
	let cp = match alpha {
		Alphabet::Upright => {
			// Roman upright: the letter itself, shaped upright rather than in the maths italic.
			if up || lo { c as u32 } else { return None }
		},
		Alphabet::BoldItalic => {
			if up { i(0x1D468, 'A') } else if lo { i(0x1D482, 'a') }
			else if digit { i(0x1D7CE, '0') } else { return None }
		},
		Alphabet::Sans => {
			if up { i(0x1D5A0, 'A') } else if lo { i(0x1D5BA, 'a') }
			else if digit { i(0x1D7E2, '0') } else { return None }
		},
		Alphabet::Mono => {
			if up { i(0x1D670, 'A') } else if lo { i(0x1D68A, 'a') }
			else if digit { i(0x1D7F6, '0') } else { return None }
		},
		Alphabet::Cal => {
			return script_char(c);
		},
		Alphabet::Frak => {
			return frak_char(c);
		},
		Alphabet::Bb => {
			return bb_char(c);
		},
	};
	char::from_u32(cp)
}

/// The script (calligraphic) codepoint of a letter, filling the block's holes from the letterlike set.
fn script_char(c: char) -> Option<char> {
	let cp = match c {
		'B' => 0x212C, 'E' => 0x2130, 'F' => 0x2131, 'H' => 0x210B, 'I' => 0x2110,
		'L' => 0x2112, 'M' => 0x2133, 'R' => 0x211B,
		'e' => 0x212F, 'g' => 0x210A, 'o' => 0x2134,
		'A'..='Z' => 0x1D49C + (c as u32 - 'A' as u32),
		'a'..='z' => 0x1D4B6 + (c as u32 - 'a' as u32),
		_ => return None,
	};
	char::from_u32(cp)
}

/// The Fraktur codepoint of a letter, filling the block's holes from the letterlike set.
fn frak_char(c: char) -> Option<char> {
	let cp = match c {
		'C' => 0x212D, 'H' => 0x210C, 'I' => 0x2111, 'R' => 0x211C, 'Z' => 0x2128,
		'A'..='Z' => 0x1D504 + (c as u32 - 'A' as u32),
		'a'..='z' => 0x1D51E + (c as u32 - 'a' as u32),
		_ => return None,
	};
	char::from_u32(cp)
}

/// The blackboard-bold codepoint of a letter or digit, filling the block's holes from the letterlike set.
fn bb_char(c: char) -> Option<char> {
	let cp = match c {
		'C' => 0x2102, 'H' => 0x210D, 'N' => 0x2115, 'P' => 0x2119, 'Q' => 0x211A,
		'R' => 0x211D, 'Z' => 0x2124,
		'A'..='Z' => 0x1D538 + (c as u32 - 'A' as u32),
		'a'..='z' => 0x1D552 + (c as u32 - 'a' as u32),
		'0'..='9' => 0x1D7D8 + (c as u32 - '0' as u32),
		_ => return None,
	};
	char::from_u32(cp)
}

/// The accent a name denotes -- for a `dot(x)` glyph function or the second argument of `accent(x, .)`.
/// A glyph accent is a spacing modifier the maths font carries; `overline`/`underline` are rules.
fn accent_of(name: &str) -> Option<Accent> {
	Some(match name {
		"dot"			=> Accent::Over("\u{02D9}".to_string()),	// dot above
		"dot.double" | "ddot"	=> Accent::Over("\u{00A8}".to_string()),	// diaeresis
		"hat"			=> Accent::Over("\u{02C6}".to_string()),	// circumflex
		"tilde"			=> Accent::Over("\u{02DC}".to_string()),	// small tilde
		"bar" | "macron"	=> Accent::Over("\u{00AF}".to_string()),	// macron
		"breve"			=> Accent::Over("\u{02D8}".to_string()),
		"check" | "caron"	=> Accent::Over("\u{02C7}".to_string()),
		"acute"			=> Accent::Over("\u{00B4}".to_string()),
		"grave"			=> Accent::Over("\u{02CB}".to_string()),
		"arrow" | "vec"	=> Accent::Over("\u{2192}".to_string()),	// a right arrow over the base
		"overline"		=> Accent::OverRule,
		"underline"		=> Accent::UnderRule,
		_				=> return None,
	})
}

/// The width of a spacing word, in thousandths of an em, or `None` when the word is not a space. Thin,
/// medium and thick match TeX's three-, four- and five-`mu` spaces; `quad` is an em and `wide` two.
fn spacing(word: &str) -> Option<i32> {
	Some(match word {
		"thin"	=> 167,		// 3 mu
		"med"	=> 222,		// 4 mu
		"thick"	=> 278,		// 5 mu
		"quad"	=> 1000,	// 1 em
		"wide"	=> 2000,	// 2 em
		"space"	=> 250,		// a normal interword space
		_		=> return None,
	})
}

/// The table of named symbols Typst authors reach for: the Greek alphabet and the common operators and
/// relations. Returned as the symbol's characters and its spacing class.
fn named(word: &str) -> Option<(&'static str, Class)> {
	use Class::*;
	let out = match word {
		// Greek lower case.
		"alpha"		=> ("\u{03B1}", Ord),	"beta"		=> ("\u{03B2}", Ord),
		"gamma"		=> ("\u{03B3}", Ord),	"delta"		=> ("\u{03B4}", Ord),
		"epsilon"	=> ("\u{03B5}", Ord),	"zeta"		=> ("\u{03B6}", Ord),
		"eta"		=> ("\u{03B7}", Ord),	"theta"		=> ("\u{03B8}", Ord),
		"iota"		=> ("\u{03B9}", Ord),	"kappa"		=> ("\u{03BA}", Ord),
		"lambda"	=> ("\u{03BB}", Ord),	"mu"		=> ("\u{03BC}", Ord),
		"nu"		=> ("\u{03BD}", Ord),	"xi"		=> ("\u{03BE}", Ord),
		"omicron"	=> ("\u{03BF}", Ord),
		"pi"		=> ("\u{03C0}", Ord),	"rho"		=> ("\u{03C1}", Ord),
		"sigma"		=> ("\u{03C3}", Ord),	"tau"		=> ("\u{03C4}", Ord),
		"upsilon"	=> ("\u{03C5}", Ord),
		"phi"		=> ("\u{03C6}", Ord),	"chi"		=> ("\u{03C7}", Ord),
		"psi"		=> ("\u{03C8}", Ord),	"omega"		=> ("\u{03C9}", Ord),
		// Greek upper case, the ones with a distinct glyph.
		"Gamma"		=> ("\u{0393}", Ord),	"Delta"		=> ("\u{0394}", Ord),
		"Theta"		=> ("\u{0398}", Ord),	"Lambda"	=> ("\u{039B}", Ord),
		"Xi"		=> ("\u{039E}", Ord),	"Pi"		=> ("\u{03A0}", Ord),
		"Sigma"		=> ("\u{03A3}", Ord),	"Upsilon"	=> ("\u{03A5}", Ord),
		"Phi"		=> ("\u{03A6}", Ord),	"Psi"		=> ("\u{03A8}", Ord),
		"Omega"		=> ("\u{03A9}", Ord),
		// Big operators.
		"sum"		=> ("\u{2211}", Op),	"prod"		=> ("\u{220F}", Op),
		"product"	=> ("\u{220F}", Op),	"coprod"	=> ("\u{2210}", Op),
		"int"		=> ("\u{222B}", Op),	"integral"	=> ("\u{222B}", Op),
		"oint"		=> ("\u{222E}", Op),
		"union"		=> ("\u{22C3}", Op),	"sect"		=> ("\u{22C2}", Op),
		// Binary operators.
		"times"		=> ("\u{00D7}", Bin),	"div"		=> ("\u{00F7}", Bin),
		"cdot"		=> ("\u{22C5}", Bin),	"pm"		=> ("\u{00B1}", Bin),
		"mp"		=> ("\u{2213}", Bin),	"ast"		=> ("\u{2217}", Bin),
		"star"		=> ("\u{22C6}", Bin),	"circ"		=> ("\u{2218}", Bin),
		"bullet"	=> ("\u{2219}", Bin),	"dot"		=> ("\u{22C5}", Bin),
		"plus"		=> ("+", Bin),			"minus"		=> ("\u{2212}", Bin),
		"cup"		=> ("\u{222A}", Bin),	"cap"		=> ("\u{2229}", Bin),
		"slash"		=> ("/", Ord),
		// Relations.
		"leq"		=> ("\u{2264}", Rel),	"geq"		=> ("\u{2265}", Rel),
		"neq"		=> ("\u{2260}", Rel),	"approx"	=> ("\u{2248}", Rel),
		"equiv"		=> ("\u{2261}", Rel),	"sim"		=> ("\u{223C}", Rel),
		"simeq"		=> ("\u{2243}", Rel),	"cong"		=> ("\u{2245}", Rel),
		"propto"	=> ("\u{221D}", Rel),	"prop"		=> ("\u{221D}", Rel),
		"in"		=> ("\u{2208}", Rel),	"notin"		=> ("\u{2209}", Rel),
		"ni"		=> ("\u{220B}", Rel),	"mid"		=> ("\u{2223}", Rel),
		"subset"	=> ("\u{2282}", Rel),	"subseteq"	=> ("\u{2286}", Rel),
		"supset"	=> ("\u{2283}", Rel),	"supseteq"	=> ("\u{2287}", Rel),
		"ll"		=> ("\u{226A}", Rel),	"gg"		=> ("\u{226B}", Rel),
		"parallel"	=> ("\u{2225}", Rel),
		// Arrows and miscellany.
		"arrow"		=> ("\u{2192}", Rel),	"to"		=> ("\u{2192}", Rel),
		"mapsto"	=> ("\u{21A6}", Rel),
		"infinity"	=> ("\u{221E}", Ord),	"oo"		=> ("\u{221E}", Ord),
		"partial"	=> ("\u{2202}", Ord),	"nabla"		=> ("\u{2207}", Ord),
		"dots"		=> ("\u{2026}", Ord),	"ldots"		=> ("\u{2026}", Ord),
		"cdots"		=> ("\u{22EF}", Ord),	"emptyset"	=> ("\u{2205}", Ord),
		"angle"		=> ("\u{2220}", Ord),	"degree"	=> ("\u{00B0}", Ord),
		"forall"	=> ("\u{2200}", Ord),	"exists"	=> ("\u{2203}", Ord),
		"neg"		=> ("\u{00AC}", Ord),	"ell"		=> ("\u{2113}", Ord),
		"and"		=> ("\u{2227}", Bin),	"or"		=> ("\u{2228}", Bin),
		_			=> return None,
	};
	Some(out)
}

/// The table of dotted symbol names -- Typst's namespaced glyphs, `arrow.r`, `plus.minus`, `dot.op` --
/// mapped to the glyph and its spacing class. The longest matching name wins at the call site.
fn dotted(name: &str) -> Option<(&'static str, Class)> {
	use Class::*;
	let out = match name {
		"arrow.r"			=> ("\u{2192}", Rel),
		"arrow.l"			=> ("\u{2190}", Rel),
		"arrow.t"			=> ("\u{2191}", Rel),
		"arrow.b"			=> ("\u{2193}", Rel),
		"arrow.l.r"			=> ("\u{2194}", Rel),
		"arrow.r.double"	=> ("\u{21D2}", Rel),
		"arrow.l.double"	=> ("\u{21D0}", Rel),
		"arrow.l.r.double"	=> ("\u{21D4}", Rel),
		"arrow.r.long"		=> ("\u{27F6}", Rel),
		"arrow.l.long"		=> ("\u{27F5}", Rel),
		"arrow.r.bar"		=> ("\u{21A6}", Rel),
		"arrow.r.long.bar"	=> ("\u{27FC}", Rel),
		"plus.minus"		=> ("\u{00B1}", Bin),
		"minus.plus"		=> ("\u{2213}", Bin),
		"dot.op"			=> ("\u{22C5}", Bin),
		"dots.h"			=> ("\u{2026}", Ord),
		"dots.h.c"			=> ("\u{22EF}", Ord),
		"dots.v"			=> ("\u{22EE}", Ord),
		"dots.down"			=> ("\u{22F1}", Ord),
		"eq.not"			=> ("\u{2260}", Rel),
		"lt.eq"				=> ("\u{2264}", Rel),
		"gt.eq"				=> ("\u{2265}", Rel),
		"lt.eq.not"			=> ("\u{2270}", Rel),
		"gt.eq.not"			=> ("\u{2271}", Rel),
		"dash.em"			=> ("\u{2014}", Ord),
		"dash.en"			=> ("\u{2013}", Ord),
		"angle.l"			=> ("\u{27E8}", Open),
		"angle.r"			=> ("\u{27E9}", Close),
		// Greek variant forms.
		"pi.alt"			=> ("\u{03D6}", Ord),
		"phi.alt"			=> ("\u{03D5}", Ord),
		"epsilon.alt"		=> ("\u{03F5}", Ord),
		"theta.alt"			=> ("\u{03D1}", Ord),
		"rho.alt"			=> ("\u{03F1}", Ord),
		"sigma.alt"			=> ("\u{03C2}", Ord),
		"beta.alt"			=> ("\u{03D0}", Ord),
		_					=> return None,
	};
	Some(out)
}

#[cfg(test)]
mod tests {
	use super::*;

	// The `cal(E)` script letter resolves to the mathematical script E (U+2130).
	#[test]
	fn cal_script() {
		match parse("cal(E)") {
			Ok(Atom::Text(s))	=> assert_eq!(s, "\u{2130}"),
			other				=> panic!("cal(E) -> {:?}", other),
		}
	}

	// `bold(D)` restyles to the mathematical bold-italic D (U+1D46B = U+1D468 + 3).
	#[test]
	fn bold_letter() {
		match parse("bold(D)") {
			Ok(Atom::Text(s))	=> assert_eq!(s, "\u{1D46B}"),
			other				=> panic!("bold(D) -> {:?}", other),
		}
	}

	// A quoted run is an upright text atom, never a maths italic.
	#[test]
	fn quoted_text() {
		match parse("\"Pr\"") {
			Ok(Atom::Text(s))	=> assert_eq!(s, "Pr"),
			other				=> panic!("quoted -> {:?}", other),
		}
	}

	// `dot(H)` is an over-accent on its base; `overline(x)` a rule over it.
	#[test]
	fn accents() {
		assert!(matches!(parse("dot(H)"), Ok(Atom::Accent { mark: Accent::Over(_), .. })));
		assert!(matches!(parse("overline(x)"), Ok(Atom::Accent { mark: Accent::OverRule, .. })));
	}

	// A spacing word becomes a fixed space.
	#[test]
	fn spacing_word() {
		assert!(matches!(parse("quad"), Ok(Atom::Space(1000))));
	}

	// A dotted symbol name resolves through its longest match.
	#[test]
	fn dotted_arrow() {
		match parse("arrow.r.double") {
			Ok(Atom::Sym(s, _))	=> assert_eq!(s, "\u{21D2}"),
			other				=> panic!("arrow.r.double -> {:?}", other),
		}
	}

	// `cases` yields a two-row grid with a left brace and no right delimiter.
	#[test]
	fn cases_grid() {
		match parse("cases(1\\, & y, x & z)") {
			Ok(Atom::Matrix { rows, left, right, kind }) => {
				assert_eq!(rows.len(), 2);
				assert_eq!(left, Some('{'));
				assert_eq!(right, None);
				assert_eq!(kind, MatKind::Cases);
			},
			other => panic!("cases -> {:?}", other),
		}
	}

	// `mat` splits its rows on `;`.
	#[test]
	fn mat_grid() {
		match parse("mat(1; 2; 3)") {
			Ok(Atom::Matrix { rows, kind, .. }) => {
				assert_eq!(rows.len(), 3);
				assert_eq!(kind, MatKind::Matrix);
			},
			other => panic!("mat -> {:?}", other),
		}
	}

	// A `\`-broken, `&`-aligned expression yields an alignment grid.
	#[test]
	fn align_block() {
		match parse("a &= b \\ &= c") {
			Ok(Atom::Matrix { rows, kind, .. }) => {
				assert_eq!(rows.len(), 2);
				assert_eq!(kind, MatKind::Align);
			},
			other => panic!("align -> {:?}", other),
		}
	}

	// Parentheses around a fraction's parts are grouping, dropped, so no fence survives.
	#[test]
	fn frac_ungroups_parens() {
		match parse("(a)/(b)") {
			Ok(Atom::Frac { num, den }) => {
				assert!(!matches!(*num, Atom::Fence { .. }));
				assert!(!matches!(*den, Atom::Fence { .. }));
			},
			other => panic!("(a)/(b) -> {:?}", other),
		}
	}

	// `dif` sets a thin space and an upright d.
	#[test]
	fn differential() {
		match parse("dif") {
			Ok(Atom::Row(items)) => {
				assert!(matches!(items.first(), Some(Atom::Space(_))));
				assert!(matches!(items.get(1), Some(Atom::Text(s)) if s == "d"));
			},
			other => panic!("dif -> {:?}", other),
		}
	}
}
