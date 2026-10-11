//! Ported from hayagriva 0.10.1 `src/lang/mod.rs`, Copyright (c) Martin Haug and the hayagriva
//! contributors, MIT OR Apache-2.0.
//!
//! hayagriva's `TitleCase` and `SentenceCase` options are fixed here at the defaults its CSL
//! renderer uses (capitalise after punctuation, hyphens separate words, keep all-capital words, use
//! the word lists, trim both ends; never force the last word), so only the branches those reach
//! are kept.
use crate::csl::{
	case_words::{
		ALWAYS_CAPITALIZE,
		NEVER_CAPITALIZE,
	},
	style::TextCase,
};


#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Case {
	Title,
	Sentence,
	Uppercase,
	Lowercase,
	#[default]
	NoTransform,
	FirstUpper,	// capitalize-first
	AllUpper,	// capitalize-all
}

impl From<TextCase> for Case {
	fn from(c: TextCase) -> Self {
		match c {
			TextCase::Uppercase		=> Self::Uppercase,
			TextCase::Lowercase		=> Self::Lowercase,
			TextCase::Title			=> Self::Title,
			TextCase::Sentence		=> Self::Sentence,
			TextCase::CapitaliseFirst	=> Self::FirstUpper,
			TextCase::CapitaliseAll		=> Self::AllUpper,
		}
	}
}

impl Case {
	pub fn transform(self, s: &str) -> String {
		let mut f = CaseFolder::with_case(self);
		f.push_str(s);
		f.finish()
	}
}

/// Does the case depend on the language, so that hayagriva applies it only to English?
pub fn is_language_dependent(c: TextCase) -> bool {
	matches!(c, TextCase::Title | TextCase::Sentence)
}

#[derive(Debug, Default, Copy, Clone, PartialEq, Eq)]
enum CharClass {
	#[default]
	NewSentence,
	MaybeNewSentence,
	NewSubclause,
	MaybeSubclause,
	NewWord,
	InWord,
}

impl CharClass {

	fn step(self, c: char, hyphen_separates: bool) -> Self {
		match c {
			'.' | '?' | '!' | '…' | '‽' | '。' | ':' => if self.is_in_word() { Self::MaybeNewSentence } else { self },
			'(' | '[' | '{'				=> Self::NewSubclause,
			')' | ']' | '}'				=> self,
			'"' | '”' | '“' | '»' | '›' | '«' | '‹' | '‘' | '\'' => self,
			',' | ';'				=> if self.is_in_word() { Self::MaybeSubclause } else { self },
			_ if c.is_whitespace() || (hyphen_separates && matches!(c, '-' | '–' | '—')) => match self {
				Self::MaybeNewSentence	=> Self::NewSentence,
				Self::MaybeSubclause	=> Self::NewSubclause,
				Self::InWord		=> Self::NewWord,
				_			=> self,
			},
			_ => Self::InWord,
		}
	}

	fn is_new_sentence(self) -> bool { matches!(self, Self::NewSentence) }
	fn is_new_subclause(self) -> bool { matches!(self, Self::NewSubclause | Self::NewSentence) }
	fn is_new_word(self) -> bool { matches!(self, Self::NewWord | Self::NewSubclause | Self::NewSentence) }
	fn is_in_word(self) -> bool { matches!(self, Self::InWord | Self::MaybeSubclause | Self::MaybeNewSentence) }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum WordCase { Lowercase, HasNonStartUpper, AllUpper }

impl WordCase {
	fn feed(self, is_upper: bool, second: bool) -> Self {
		match (self, is_upper, second) {
			(Self::Lowercase, true, _)	=> Self::HasNonStartUpper,
			(Self::AllUpper, false, false)	=> Self::HasNonStartUpper,
			(Self::AllUpper, false, true)	=> Self::Lowercase,
			_				=> self,
		}
	}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct WordData {
	start:			usize,
	end:			usize,
	case:			WordCase,
	follows_punctuation:	bool,
}

impl WordData {

	fn new(c: char, before: usize, now: usize, prev: CharClass, class: CharClass) -> Option<Self> {
		match class.is_in_word() {
			true => Some(Self {
				start:			before,
				end:			now,
				case:			if c.is_uppercase() { WordCase::AllUpper } else { WordCase::Lowercase },
				follows_punctuation:	prev.is_new_subclause(),
			}),
			false => None,
		}
	}

	fn feed(self, c: char, before: usize, now: usize, prev: CharClass, class: CharClass) -> Option<Self> {
		if class.is_new_word() {
			return None;
		}
		if !prev.is_in_word() && class == CharClass::InWord {
			return Self::new(c, before, now, prev, class);
		}
		let second = self.end - self.start == 1;
		let case = if c.is_ascii_punctuation() { self.case } else { self.case.feed(c.is_uppercase(), second) };
		Some(Self { end: now, case, ..self })
	}

	fn is_all_upper(self) -> bool {
		matches!(self.case, WordCase::AllUpper) && self.end - self.start > 1
	}
}

enum Verdict { AllUpper, Capitalise, Lowercase, Keep }

/// A string buffer that applies a text case as characters arrive, word by word.
#[derive(Debug, Clone)]
pub struct CaseFolder {
	case:		Case,
	pristine:	bool,
	class:		CharClass,
	buf:		String,
	last_reconfig:	usize,
	last_word:	Option<WordData>,
}

impl Default for CaseFolder {
	fn default() -> Self {
		Self {
			case:		Case::default(),
			pristine:	true,
			class:		CharClass::default(),
			buf:		String::new(),
			last_reconfig:	0,
			last_word:	None,
		}
	}
}

impl CaseFolder {

	pub fn with_case(case: Case) -> Self {
		Self { case, ..Default::default() }
	}

	pub fn push_str(&mut self, s: &str) {
		match self.case {
			Case::NoTransform => {
				self.buf.push_str(s);
				self.class = CharClass::default();
			}
			Case::Uppercase => {
				self.buf.extend(s.chars().flat_map(char::to_uppercase));
				self.class = CharClass::default();
			}
			Case::Lowercase => {
				self.buf.extend(s.chars().flat_map(char::to_lowercase));
				self.class = CharClass::default();
			}
			_ => for c in s.chars() { self.push(c) },
		}
		self.pristine = false;
	}

	/// Stops the end being trimmed back past this point, as math content would be.
	pub fn prevent_trimming(&mut self) {
		self.last_reconfig = self.buf.len();
		self.pristine = false;
	}

	/// Pushes a case-protected span untouched.
	pub fn push_verbatim(&mut self, s: &str) {
		let conf = std::mem::replace(&mut self.case, Case::NoTransform);
		self.last_reconfig = self.buf.len();
		self.push_str(s);
		self.last_reconfig = self.buf.len();
		self.case = conf;
		self.class = CharClass::InWord;
	}

	pub fn push(&mut self, c: char) {
		let prev = self.class;
		let hyphen = matches!(self.case, Case::Title);
		self.class = prev.step(c, hyphen);
		let before = self.buf.len();
		match self.case {
			Case::Title | Case::Sentence if self.pristine && c.is_whitespace() => {}
			Case::Title => match prev.is_new_word() {
				true	=> self.buf.extend(c.to_uppercase()),
				false	=> self.buf.extend(c.to_lowercase()),
			},
			Case::Sentence => {
				if prev.is_new_sentence() {
					self.buf.extend(c.to_uppercase());
				} else if matches!(prev, CharClass::MaybeNewSentence | CharClass::MaybeSubclause) {
					self.buf.push(c);
				} else {
					self.buf.extend(c.to_lowercase());
				}
			}
			Case::FirstUpper => match self.pristine && c.is_lowercase() {
				true	=> self.buf.extend(c.to_uppercase()),
				false	=> self.buf.push(c),
			},
			Case::AllUpper => match prev.is_new_word() && c.is_lowercase() {
				true	=> self.buf.extend(c.to_uppercase()),
				false	=> self.buf.push(c),
			},
			Case::Uppercase		=> self.buf.extend(c.to_uppercase()),
			Case::Lowercase		=> self.buf.extend(c.to_lowercase()),
			Case::NoTransform	=> self.buf.push(c),
		}
		let now = self.buf.len();
		self.last_word = match self.last_word {
			Some(data) => {
				let next = data.feed(c, before, now, prev, self.class);
				let continuing = matches!(next, Some(n) if n.start == data.start);
				if !continuing {
					self.process_word();
				}
				next
			}
			None => WordData::new(c, before, now, prev, self.class),
		};
		self.pristine = self.pristine && c.is_whitespace();
	}

	fn find_word(&self) -> &str {
		let data = match self.last_word {
			Some(d) => d,
			None	=> return "",
		};
		let mut end = data.start;
		while end < data.end && self.buf[end..].chars().next().map(char::is_alphanumeric).unwrap_or(false) {
			end += 1;
			while !self.buf.is_char_boundary(end) { end += 1; }
		}
		&self.buf[data.start..end]
	}

	fn process_word(&mut self) {
		let data = match self.last_word {
			Some(d) => d,
			None	=> return,
		};
		let verdict = match self.case {
			Case::Title => {
				if data.is_all_upper() {
					Verdict::AllUpper
				} else if !data.follows_punctuation && NEVER_CAPITALIZE.binary_search(&self.find_word()).is_ok() {
					Verdict::Lowercase
				} else {
					Verdict::Keep
				}
			}
			Case::Sentence => {
				if data.case == WordCase::HasNonStartUpper {
					Verdict::Capitalise
				} else if data.is_all_upper() {
					Verdict::AllUpper
				} else if ALWAYS_CAPITALIZE.binary_search(&self.find_word()).is_ok() {
					Verdict::Capitalise
				} else {
					Verdict::Keep
				}
			}
			_ => Verdict::Keep,
		};
		match verdict {
			Verdict::AllUpper	=> map_chars(&mut self.buf, data.start..data.end, |c| match c.is_lowercase() {
				true	=> Some(c.to_uppercase().collect()),
				false	=> None,
			}),
			Verdict::Capitalise	=> capitalise_char(&mut self.buf, data.start),
			Verdict::Lowercase	=> match self.buf[data.start..].chars().next() {
				Some(c) if c.is_uppercase() => {
					map_chars(&mut self.buf, data.start..data.start + c.len_utf8(), |c| Some(c.to_lowercase().collect()));
				}
				_ => {}
			},
			Verdict::Keep		=> {}
		}
	}

	fn may_trim_end(&mut self) {
		if matches!(self.case, Case::Title | Case::Sentence) {
			while self.buf.len() > self.last_reconfig && self.buf.ends_with(char::is_whitespace) {
				self.buf.pop();
			}
		}
	}

	pub fn case(&self) -> Case {
		self.case
	}

	/// Changes the case for what follows, trimming what the old case wrote.
	pub fn reconfigure(&mut self, case: Case) {
		if self.case == case {
			return;
		}
		self.may_trim_end();
		self.last_reconfig = self.buf.len();
		self.pristine = true;
		self.case = case;
	}

	pub fn finish(mut self) -> String {
		self.process_word();
		self.may_trim_end();
		self.buf
	}

	pub fn is_empty(&self) -> bool {
		self.buf.is_empty()
	}

	pub fn len(&self) -> usize {
		self.buf.len()
	}

	pub fn ends_with(&self, c: char) -> bool {
		self.buf.ends_with(c)
	}

	pub fn as_string_mut(&mut self) -> &mut String {
		&mut self.buf
	}

	pub fn as_str(&self) -> &str {
		&self.buf
	}

	/// Records an edit made through `as_string_mut`, so the word being folded is forgotten.
	pub fn mark_changed(&mut self) {
		self.last_reconfig = self.buf.len();
		self.last_word = None;
	}
}

fn map_chars<F: FnMut(char) -> Option<Vec<char>>>(buf: &mut String, range: std::ops::Range<usize>, mut f: F) {
	let mut i = range.start;
	let mut orig = range.start;
	while let Some(ch) = buf[i..].chars().next() {
		if orig >= range.end {
			break;
		}
		orig += ch.len_utf8();
		match f(ch) {
			None => i += ch.len_utf8(),
			Some(rep) => {
				let mut end = i + ch.len_utf8();
				for c in rep {
					let mut tmp = [0u8; 4];
					buf.replace_range(i..end, c.encode_utf8(&mut tmp));
					i += c.len_utf8();
					end = i;
				}
			}
		}
	}
}

fn capitalise_char(buf: &mut String, start: usize) {
	match buf[start..].chars().next() {
		Some(c) if c.is_lowercase() => map_chars(buf, start..start + c.len_utf8(), |c| Some(c.to_uppercase().collect())),
		_ => {}
	}
}
