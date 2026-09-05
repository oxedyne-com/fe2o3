//! A parser for Typst's inline maths syntax into the engine's [`Atom`](crate::math::Atom) tree.
//!
//! The engine sets a rich mathematics already; this is the front end that reads `$...$` the way an
//! author writes it in Typst and hands the layout an `Atom`. The subset covered is the common one: a
//! variable is a letter, a number a run of digits; `^` and `_` attach a superscript and a subscript,
//! grouped with `(...)` or `{...}`; `/` and `frac(a, b)` make a fraction; `sqrt(x)` a radical; `(...)`
//! grows a fence; and a run of letters is looked up in a table of Greek letters and common operators
//! (`alpha`, `sum`, `times`, `->`) or, failing that, set as an identifier. Matrices, `cases`, `vec`,
//! big-operator limits beyond a plain script, and alignment are later work; an unknown control word is
//! set as itself rather than rejected, so a document still sets.

use crate::math::{
	Atom,
	Class,
};

use oxedyne_fe2o3_core::prelude::*;

/// Parses one maths expression (the text between the `$` delimiters) into an [`Atom`].
pub fn parse(src: &str) -> Outcome<Atom> {
	let mut p = Parser { chars: src.chars().collect(), i: 0 };
	let atom = res!(p.row(&[]));
	Ok(atom)
}

struct Parser {
	chars:	Vec<char>,
	i:		usize,
}

impl Parser {
	fn peek(&self) -> Option<char> { self.chars.get(self.i).copied() }
	fn peek2(&self) -> Option<char> { self.chars.get(self.i + 1).copied() }

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

	/// A sequence of factors up to end of input or a stop character, folding `/` into fractions. A row of
	/// one element unwraps to that element, so a bare `x` is a symbol, not a one-item row.
	fn row(&mut self, stop: &[char]) -> Outcome<Atom> {
		let mut items: Vec<Atom> = Vec::new();
		loop {
			self.skip_ws();
			match self.peek() {
				None => break,
				Some(c) if stop.contains(&c) => break,
				_ => {},
			}
			if self.peek() == Some('/') {
				// A fraction of the preceding factor and the next one.
				self.i += 1;
				self.skip_ws();
				let den = res!(self.factor(stop));
				let num = items.pop().unwrap_or_else(|| Atom::row(Vec::new()));
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

		// A word: a function, a named symbol, or an identifier.
		if c.is_alphabetic() {
			let word = self.take_while(|c| c.is_alphabetic());
			return self.word(&word);
		}

		// An operator, possibly two characters (`<=`, `->`).
		Ok(self.operator())
	}

	/// A run of letters, resolved as a function, a named symbol, or an identifier. `frac`, `sqrt` and
	/// `root` read their bracketed arguments; a name in the symbol table becomes that symbol; anything
	/// else is set as an identifier, a single letter as an italic variable.
	fn word(&mut self, word: &str) -> Outcome<Atom> {
		match word {
			"frac" => {
				let (a, b) = res!(self.two_args());
				Ok(Atom::frac(a, b))
			},
			"sqrt" => {
				let a = res!(self.one_arg());
				Ok(Atom::sqrt(a))
			},
			"root" => {
				// root(index, radicand): the index is not yet drawn, so the radicand alone is set.
				let (_, b) = res!(self.two_args());
				Ok(Atom::sqrt(b))
			},
			_ => Ok(symbol(word)),
		}
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
		"pi"		=> ("\u{03C0}", Ord),	"rho"		=> ("\u{03C1}", Ord),
		"sigma"		=> ("\u{03C3}", Ord),	"tau"		=> ("\u{03C4}", Ord),
		"phi"		=> ("\u{03C6}", Ord),	"chi"		=> ("\u{03C7}", Ord),
		"psi"		=> ("\u{03C8}", Ord),	"omega"		=> ("\u{03C9}", Ord),
		// Greek upper case, the ones with a distinct glyph.
		"Gamma"		=> ("\u{0393}", Ord),	"Delta"		=> ("\u{0394}", Ord),
		"Theta"		=> ("\u{0398}", Ord),	"Lambda"	=> ("\u{039B}", Ord),
		"Xi"		=> ("\u{039E}", Ord),	"Pi"		=> ("\u{03A0}", Ord),
		"Sigma"		=> ("\u{03A3}", Ord),	"Phi"		=> ("\u{03A6}", Ord),
		"Psi"		=> ("\u{03A8}", Ord),	"Omega"		=> ("\u{03A9}", Ord),
		// Big operators.
		"sum"		=> ("\u{2211}", Op),	"prod"		=> ("\u{220F}", Op),
		"int"		=> ("\u{222B}", Op),	"oint"		=> ("\u{222E}", Op),
		// Binary operators.
		"times"		=> ("\u{00D7}", Bin),	"div"		=> ("\u{00F7}", Bin),
		"cdot"		=> ("\u{22C5}", Bin),	"pm"		=> ("\u{00B1}", Bin),
		"mp"		=> ("\u{2213}", Bin),	"ast"		=> ("\u{2217}", Bin),
		// Relations.
		"leq"		=> ("\u{2264}", Rel),	"geq"		=> ("\u{2265}", Rel),
		"neq"		=> ("\u{2260}", Rel),	"approx"	=> ("\u{2248}", Rel),
		"equiv"		=> ("\u{2261}", Rel),	"sim"		=> ("\u{223C}", Rel),
		"propto"	=> ("\u{221D}", Rel),	"in"		=> ("\u{2208}", Rel),
		"notin"		=> ("\u{2209}", Rel),	"subset"	=> ("\u{2282}", Rel),
		"subseteq"	=> ("\u{2286}", Rel),	"supset"	=> ("\u{2283}", Rel),
		// Arrows and miscellany.
		"arrow"		=> ("\u{2192}", Rel),	"to"		=> ("\u{2192}", Rel),
		"infinity"	=> ("\u{221E}", Ord),	"oo"		=> ("\u{221E}", Ord),
		"partial"	=> ("\u{2202}", Ord),	"nabla"		=> ("\u{2207}", Ord),
		"dot"		=> ("\u{22C5}", Bin),	"dots"		=> ("\u{2026}", Ord),
		"forall"	=> ("\u{2200}", Ord),	"exists"	=> ("\u{2203}", Ord),
		_			=> return None,
	};
	Some(out)
}
