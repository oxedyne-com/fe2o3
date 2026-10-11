//! Ported from hayagriva 0.10.1, Copyright (c) Martin Haug and the hayagriva contributors,
//! MIT OR Apache-2.0 (`src/types/persons.rs`, `src/lang/name.rs`, `src/lang/en.rs`'s articles and
//! `src/lang/mod.rs`'s `is_cjk`).
//!
//! hayagriva's model is BibTeX's, not CSL's: `family` may carry a non-dropping particle ("van der
//! Berg" read from `{van der Berg}`), found again by [`Person::particle_in_family`], and
//! `particle` is BibTeX's "von" part, which CSL would call the dropping particle. A person with
//! no given name, particle or suffix is institutional (CSL's literal name).
use crate::unicode::segment::graphemes;

use std::fmt::Write;


/// A name as hayagriva holds it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Person {
	pub family:		String,
	pub given:		Option<String>,
	pub particle:		Option<String>,	// BibTeX's "von" part
	pub suffix:		Option<String>,
	pub comma_suffix:	bool,		// a comma before the suffix in given-first order
}

// Sorted, searched by bisection; the repeats are hayagriva's.
const NAME_PARTICLES: [&str; 115] = [
	"'t", "a", "ab", "af", "al", "am", "and", "ap", "auf", "auf der", "bar", "ben", "bin", "d'",
	"da", "dai", "dal", "dall'", "dalla", "dalle", "de", "de", "de", "de la", "de las", "de los",
	"de'", "degli", "dei", "del", "dela", "dell'", "della", "delle", "dello", "den", "den", "der",
	"der", "des", "di", "do", "dos", "du", "e", "el", "en", "et", "fan", "filho", "fitz", "grosse",
	"große", "i", "ibn", "in", "l'", "la", "las", "le", "li", "lo", "los", "m'", "mac", "mc", "n'",
	"no", "ní", "o", "och", "of", "op de", "saint", "sainte", "san", "sant", "santa", "santo",
	"st", "ste", "t'", "te", "ten", "ten", "ter", "ter", "thoe", "tot", "und", "und der", "van",
	"van", "van de", "van den", "van der", "van het", "vande", "vanden", "vander", "ver", "vom",
	"von", "von den", "von der", "von und zu", "y", "z", "ze", "zu", "zum", "zur", "à", "é", "ó",
];

const ARTICLES: [&str; 8] = ["A", "AN", "An", "THE", "The", "a", "an", "the"];

/// Is the character a CJK ideograph? Names holding one are written family first, unspaced.
pub fn is_cjk(c: char) -> bool {
	let cp = c as u32;
	(0x4E00..=0x9FFF).contains(&cp)
		|| (0x3400..=0x4DBF).contains(&cp)
		|| (0x20000..=0x2A6DF).contains(&cp)
		|| (0x2A700..=0x2B73F).contains(&cp)
		|| (0x2B740..=0x2B81F).contains(&cp)
		|| (0x2B820..=0x2CEAF).contains(&cp)
		|| (0xF900..=0xFAFF).contains(&cp)
		|| (0x2F800..=0x2FA1F).contains(&cp)
}

impl Person {

	pub fn new(family: &str, given: Option<&str>) -> Self {
		Self { family: family.to_string(), given: given.map(str::to_string), ..Self::default() }
	}

	/// An institutional name, written as it stands.
	pub fn literal(name: &str) -> Self {
		Self::new(name, None)
	}

	pub fn with_particle(mut self, p: &str) -> Self {
		self.particle = Some(p.to_string());
		self
	}

	pub fn with_suffix(mut self, s: &str) -> Self {
		self.suffix = Some(s.to_string());
		self
	}

	pub fn is_institutional(&self) -> bool {
		self.given.is_none() && self.suffix.is_none() && self.particle.is_none()
	}

	pub fn is_cjk(&self) -> bool {
		self.family.chars().any(is_cjk) || self.given.as_ref().map_or(false, |g| g.chars().any(is_cjk))
	}

	/// The non-dropping particle at the start of the family name: the longest leading run of
	/// words, ending before a space, that is a known particle.
	pub fn particle_in_family(&self) -> Option<&str> {
		for (i, c) in self.family.char_indices().rev() {
			if c != ' ' {
				continue;
			}
			let p = &self.family[..i];
			if NAME_PARTICLES.binary_search(&p.to_lowercase().as_str()).is_ok() {
				return Some(p);
			}
		}
		None
	}

	pub fn family_without_particle(&self) -> &str {
		match self.particle_in_family() {
			Some(p)	=> self.family[p.len()..].trim_start(),
			None	=> &self.family,
		}
	}

	/// The family name less a leading English article, for sorting an institution.
	pub fn family_without_article(&self) -> &str {
		match self.family.find(' ') {
			Some(i) if i + 1 != self.family.len() && ARTICLES.binary_search(&&self.family[..i]).is_ok() =>
				&self.family[i + 1..],
			_ => &self.family,
		}
	}

	/// Writes the given name's initials, each followed by `delim`. A lower-case word ("de") is
	/// written whole, and with `hyphen` a hyphenated name keeps its hyphen ("H.-P.").
	pub fn initials<W: Write>(&self, w: &mut W, delim: Option<&str>, hyphen: bool) -> std::fmt::Result {
		let given = match &self.given {
			Some(g)	=> g,
			None	=> return Ok(()),
		};
		let mut collect = true;
		let mut non_empty = false;
		let mut start_of_word = true;
		let mut last_was_lower = false;
		let delim_ends_ws = delim.and_then(|d| d.chars().last()).map_or(false, char::is_whitespace);
		for gr in graphemes(given) {
			if let Some(c) = gr.chars().next() {
				if c.is_whitespace() || c == '-' {
					if last_was_lower {
						collect = false;
					}
					if !collect {
						let hyphenate = hyphen && c == '-';
						if !last_was_lower {
							if let Some(d) = delim {
								let _ = w.write_str(if hyphenate { d.trim_end() } else { d });
							}
						}
						collect = true;
						start_of_word = true;
						if hyphenate {
							let _ = w.write_char('-');
						} else if last_was_lower {
							let _ = w.write_char(' ');
						}
					}
					continue;
				}
			}
			if collect {
				if gr.to_lowercase() != gr {
					collect = false;
					last_was_lower = false;
				} else if start_of_word {
					last_was_lower = true;
				}
				if last_was_lower && start_of_word {
					start_of_word = false;
					if !delim_ends_ws {
						let _ = w.write_char(' ');
					}
				}
				let _ = w.write_str(gr);
				non_empty = true;
			}
		}
		if non_empty && !collect {
			if let Some(d) = delim {
				let _ = w.write_str(d.trim_end());
			}
		}
		Ok(())
	}

	/// Writes the given name with `delim` after each single-letter word, which is how hayagriva
	/// honours `initialize-with` when `initialize` is false.
	pub fn given_with_delimiter<W: Write>(&self, w: &mut W, delim: Option<&str>) -> std::fmt::Result {
		let given = match &self.given {
			Some(g)	=> g,
			None	=> return Ok(()),
		};
		for (i, word) in given.split(' ').enumerate() {
			if i > 0 {
				let _ = w.write_char(' ');
			}
			let _ = w.write_str(word);
			if let Some(d) = delim {
				if graphemes(word).len() == 1 {
					let _ = w.write_str(d);
				}
			}
		}
		Ok(())
	}
}
