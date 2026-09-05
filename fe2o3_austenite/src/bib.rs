//! Bibliography: a BibTeX reader and a Chicago author-date citation formatter.
//!
//! The engine reads a `.bib` file into [`Bibliography`], formats an in-text citation as
//! `(Surname Year)` through [`Bibliography::cite`], and sets a sorted reference list through
//! [`Bibliography::reference_list`]. It is standalone: the reader wires `#cite(<key>)` to `cite`
//! and the back-matter Bibliography section to `reference_list`; nothing here touches the parser or
//! the page.
//!
//! The oracle is Typst 0.15.1's `chicago-author-date` style, matched against a render of the
//! Lucronics bibliography. Three behaviours are Typst-specific and reproduced deliberately, each
//! noted at its site: the sort orders works by one author before works by that author with
//! coauthors (author count is the tie-break after the first surname); a journal article with no
//! volume sets a bare comma after the journal name (`*Journal*,.`); and page ranges are abbreviated
//! by the Chicago rule (`263--291` sets as `263-91`).
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use oxedyne_fe2o3_core::prelude::*;

use std::collections::BTreeMap;

/// The closed set of entry types the formatter styles. An unrecognised `@type` reads as
/// [`EntryKind::Misc`], which sets title-italic like a book -- a declared fallback, not a hidden gap.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntryKind {
	Book,
	Article,
	InCollection,
	InProceedings,
	TechReport,
	Report,
	Online,
	Unpublished,
	Misc,
}

impl EntryKind {
	fn from_type(s: &str) -> Self {
		match s.to_ascii_lowercase().as_str() {
			"book" | "booklet" | "proceedings"	=> Self::Book,
			"article"				=> Self::Article,
			"incollection" | "inbook"		=> Self::InCollection,
			"inproceedings" | "conference"		=> Self::InProceedings,
			"techreport"				=> Self::TechReport,
			"report"				=> Self::Report,
			"online" | "electronic" | "misc"	=> Self::Online,
			"unpublished"				=> Self::Unpublished,
			_					=> Self::Misc,
		}
	}
}

/// One personal or corporate name. A corporate name carries its whole literal in `family` with an
/// empty `given`; a personal name is inverted to `family, given` for the list and cited by `family`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Name {
	family:    String,
	given:     String,
	corporate: bool,
}

impl Name {
	pub fn family(&self) -> &str { &self.family }
	pub fn given(&self)  -> &str { &self.given }
	pub fn is_corporate(&self) -> bool { self.corporate }

	/// Sets the name inverted for a reference list: `Family, Given` (personal) or the literal
	/// (corporate).
	fn inverted(&self) -> String {
		if self.corporate || self.given.is_empty() {
			self.family.clone()
		} else {
			fmt!("{}, {}", self.family, self.given)
		}
	}

	/// Sets the name in natural order for a coauthor position: `Given Family`.
	fn natural(&self) -> String {
		if self.corporate || self.given.is_empty() {
			self.family.clone()
		} else {
			fmt!("{} {}", self.given, self.family)
		}
	}
}

/// A parsed BibTeX entry. Fields are held decoded (LaTeX accents and escapes resolved) in file
/// order; a reader wanting a raw field asks by name through [`Entry::field`].
#[derive(Clone, Debug)]
pub struct Entry {
	key:    String,
	kind:   EntryKind,
	fields: Vec<(String, String)>,
}

impl Entry {
	pub fn key(&self)  -> &str { &self.key }
	pub fn kind(&self) -> EntryKind { self.kind }

	/// Returns the decoded value of a field by name, case-insensitively.
	pub fn field(&self, name: &str) -> Option<&str> {
		self.fields.iter()
			.find(|(k, _)| k.eq_ignore_ascii_case(name))
			.map(|(_, v)| v.as_str())
	}

	/// The names credited for citation: authors, or editors when there is no author.
	pub fn credited(&self) -> Vec<Name> {
		match self.field("author") {
			Some(a) => parse_names(a),
			None    => match self.field("editor") {
				Some(e) => parse_names(e),
				None    => Vec::new(),
			},
		}
	}

	fn year(&self) -> Option<i64> {
		self.field("year").and_then(|y| {
			let digits: String = y.chars().filter(|c| c.is_ascii_digit()).collect();
			digits.parse::<i64>().ok()
		})
	}
}

/// A styled fragment of a reference line. The block layer sets an [`RefStyle::Italic`] run in
/// italic; everything else -- quotation marks, full stops, DOI URLs -- is literal text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RefRun {
	pub text:  String,
	pub style: RefStyle,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RefStyle {
	Normal,
	Italic,
}

/// One formatted reference: the entry key and its runs, ready for the block layer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reference {
	pub key:  String,
	pub runs: Vec<RefRun>,
}

impl Reference {
	/// The reference as one plain string, italics flattened away. For tests and diagnostics.
	pub fn plain(&self) -> String {
		let mut s = String::new();
		for r in &self.runs {
			s.push_str(&r.text);
		}
		s
	}
}

/// A parsed bibliography and the record of which keys a document has cited.
pub struct Bibliography {
	entries: Vec<Entry>,			// in file order
	index:   BTreeMap<String, usize>,	// key -> position in `entries`
	cited:   Vec<String>,			// cited keys, first-cite order, deduplicated
}

impl Bibliography {
	/// Reads a BibTeX source string into a bibliography. Comments, `@string`/`@preamble` blocks and
	/// stray text between entries are skipped; an entry with a duplicate key keeps the first seen.
	pub fn parse(src: &str) -> Outcome<Self> {
		let chars: Vec<char> = src.chars().collect();
		let mut entries: Vec<Entry> = Vec::new();
		let mut index:   BTreeMap<String, usize> = BTreeMap::new();
		let mut i = 0usize;
		let n = chars.len();
		while i < n {
			// Advance to the next entry opener.
			if chars[i] != '@' {
				i += 1;
				continue;
			}
			let (entry_opt, next) = res!(parse_entry(&chars, i));
			i = next;
			if let Some(entry) = entry_opt {
				if !index.contains_key(&entry.key) {
					index.insert(entry.key.clone(), entries.len());
					entries.push(entry);
				}
			}
		}
		Ok(Self { entries, index, cited: Vec::new() })
	}

	pub fn len(&self) -> usize { self.entries.len() }
	pub fn is_empty(&self) -> bool { self.entries.is_empty() }

	/// Borrows an entry by key.
	pub fn entry(&self, key: &str) -> Option<&Entry> {
		self.index.get(key).map(|&p| &self.entries[p])
	}

	/// Records a key as cited, in first-cite order, for the reference list. A key unknown to the
	/// bibliography is ignored here and reported by [`Bibliography::cite`] at formatting time.
	pub fn mark_cited(&mut self, key: &str) {
		if self.index.contains_key(key) && !self.cited.iter().any(|k| k == key) {
			self.cited.push(key.to_string());
		}
	}

	/// Formats one in-text citation for a run of keys and records each as cited. Sets
	/// `(Surname Year)` for one key and `(A Year; B Year)` for several, matching the oracle's
	/// semicolon separator.
	pub fn cite(&mut self, keys: &[&str]) -> Outcome<String> {
		for &k in keys {
			self.mark_cited(k);
		}
		self.format_citation(keys)
	}

	/// Formats an in-text citation without recording it. Used where the reader has already marked
	/// the keys, or for a pure formatting query.
	pub fn format_citation(&self, keys: &[&str]) -> Outcome<String> {
		if keys.is_empty() {
			return Err(err!("A citation needs at least one key."; Input, Invalid, Missing));
		}
		let mut labels: Vec<String> = Vec::with_capacity(keys.len());
		for &k in keys {
			let entry = res!(self.entry(k).ok_or_else(||
				err!("Citation key {:?} is not in the bibliography.", k; Input, Missing)));
			labels.push(cite_label(entry));
		}
		Ok(fmt!("({})", labels.join("; ")))
	}

	/// The in-text label for a single key without the surrounding parentheses, e.g. `Scott 1976`.
	pub fn label(&self, key: &str) -> Outcome<String> {
		let entry = res!(self.entry(key).ok_or_else(||
			err!("Citation key {:?} is not in the bibliography.", key; Input, Missing)));
		Ok(cite_label(entry))
	}

	/// The sorted, Chicago-styled reference list for the cited keys. If nothing has been marked
	/// cited, the list is empty -- Typst prints only cited works.
	pub fn reference_list(&self) -> Vec<Reference> {
		let mut refs: Vec<&Entry> = self.cited.iter()
			.filter_map(|k| self.entry(k))
			.collect();
		sort_entries(&mut refs);
		refs.iter().map(|e| format_reference(e)).collect()
	}

	/// Every entry in the file, sorted and formatted. For tests and for a "print all" mode.
	pub fn all_references(&self) -> Vec<Reference> {
		let mut refs: Vec<&Entry> = self.entries.iter().collect();
		sort_entries(&mut refs);
		refs.iter().map(|e| format_reference(e)).collect()
	}
}

// ---------------------------------------------------------------------------------------------
// Parsing.
// ---------------------------------------------------------------------------------------------

/// Parses one `@type{ key, field = value, ... }` starting at `chars[start]` (the `@`). Returns the
/// entry (or `None` for a `@string`/`@preamble`/`@comment` block) and the index just past its close.
fn parse_entry(chars: &[char], start: usize) -> Outcome<(Option<Entry>, usize)> {
	let n = chars.len();
	let mut i = start + 1; // past '@'
	// Read the type word.
	let mut kind_word = String::new();
	while i < n && (chars[i].is_ascii_alphabetic()) {
		kind_word.push(chars[i]);
		i += 1;
	}
	// Skip to the opening brace or paren.
	while i < n && chars[i] != '{' && chars[i] != '(' {
		if chars[i] == '@' {
			// A malformed entry with no body; bail without consuming the next '@'.
			return Ok((None, i));
		}
		i += 1;
	}
	if i >= n {
		return Ok((None, n));
	}
	let open  = chars[i];
	let close = if open == '{' { '}' } else { ')' };
	i += 1;

	let lower = kind_word.to_ascii_lowercase();
	if lower == "string" || lower == "preamble" || lower == "comment" {
		// Skip the balanced body.
		let end = skip_balanced(chars, i, open, close);
		return Ok((None, end));
	}

	// Read the citation key up to the first comma.
	let mut key = String::new();
	while i < n && chars[i] != ',' && chars[i] != close {
		if !chars[i].is_whitespace() {
			key.push(chars[i]);
		}
		i += 1;
	}
	let mut fields: Vec<(String, String)> = Vec::new();
	// Read fields.
	loop {
		// Skip separators and whitespace.
		while i < n && (chars[i] == ',' || chars[i].is_whitespace()) {
			i += 1;
		}
		if i >= n || chars[i] == close {
			if i < n {
				i += 1; // consume close
			}
			break;
		}
		// Field name.
		let mut name = String::new();
		while i < n && chars[i] != '=' && chars[i] != close && !chars[i].is_whitespace() {
			name.push(chars[i]);
			i += 1;
		}
		// Skip whitespace to '='.
		while i < n && chars[i].is_whitespace() {
			i += 1;
		}
		if i >= n || chars[i] != '=' {
			// No value; a trailing key or malformed field. Stop at close if present.
			if i < n && chars[i] == close {
				i += 1;
			}
			break;
		}
		i += 1; // past '='
		while i < n && chars[i].is_whitespace() {
			i += 1;
		}
		let (raw, next) = read_value(chars, i);
		i = next;
		if !name.is_empty() {
			fields.push((name.clone(), decode_value(&raw)));
		}
	}

	let entry = Entry {
		key,
		kind: EntryKind::from_type(&lower),
		fields,
	};
	Ok((Some(entry), i))
}

/// Reads a field value: a `{...}` group, a `"..."` string, or a bare token (number or macro name).
/// Returns the raw inner text (braces or quotes stripped at the outermost level only) and the next
/// index.
fn read_value(chars: &[char], start: usize) -> (String, usize) {
	let n = chars.len();
	let mut i = start;
	if i >= n {
		return (String::new(), i);
	}
	match chars[i] {
		'{' => {
			let mut depth = 0i32;
			let mut out = String::new();
			while i < n {
				let c = chars[i];
				if c == '{' {
					depth += 1;
					if depth > 1 {
						out.push(c);
					}
					i += 1;
				} else if c == '}' {
					depth -= 1;
					if depth == 0 {
						i += 1;
						break;
					}
					out.push(c);
					i += 1;
				} else {
					out.push(c);
					i += 1;
				}
			}
			(out, i)
		}
		'"' => {
			// Quoted value; braces inside protect nested quotes.
			let mut depth = 0i32;
			let mut out = String::new();
			i += 1; // past opening quote
			while i < n {
				let c = chars[i];
				if c == '{' {
					depth += 1;
					out.push(c);
					i += 1;
				} else if c == '}' {
					depth -= 1;
					out.push(c);
					i += 1;
				} else if c == '"' && depth == 0 {
					i += 1;
					break;
				} else {
					out.push(c);
					i += 1;
				}
			}
			(out, i)
		}
		_ => {
			// Bare token to the next comma or closing brace/paren.
			let mut out = String::new();
			while i < n && chars[i] != ',' && chars[i] != '}' && chars[i] != ')' {
				out.push(chars[i]);
				i += 1;
			}
			(out.trim().to_string(), i)
		}
	}
}

/// Returns the index just past the balanced group opened at `i` (already inside the opener).
fn skip_balanced(chars: &[char], start: usize, open: char, close: char) -> usize {
	let n = chars.len();
	let mut i = start;
	let mut depth = 1i32;
	while i < n && depth > 0 {
		if chars[i] == open {
			depth += 1;
		} else if chars[i] == close {
			depth -= 1;
		}
		i += 1;
	}
	i
}

// ---------------------------------------------------------------------------------------------
// Value decoding: LaTeX accents, escapes, dashes and quotes to Unicode.
// ---------------------------------------------------------------------------------------------

/// Decodes a raw BibTeX value to display text: resolves accent commands (`{\'e}` -> e-acute), the
/// literal escapes `\&`/`\%`/`\$` and the special letters (`\o`, `\L`, ...), collapses grouping
/// braces, and turns `--` into an en dash and TeX quotes into curly quotes.
fn decode_value(raw: &str) -> String {
	let chars: Vec<char> = raw.chars().collect();
	let n = chars.len();
	let mut out = String::new();
	let mut i = 0usize;
	while i < n {
		let c = chars[i];
		match c {
			'\\' => {
				let (text, next) = decode_control(&chars, i);
				out.push_str(&text);
				i = next;
			}
			'{' | '}' => {
				// Grouping brace: drop it, keep the contents.
				i += 1;
			}
			'-' if i + 1 < n && chars[i + 1] == '-' => {
				if i + 2 < n && chars[i + 2] == '-' {
					out.push('\u{2014}'); // em dash
					i += 3;
				} else {
					out.push('\u{2013}'); // en dash
					i += 2;
				}
			}
			'`' if i + 1 < n && chars[i + 1] == '`' => {
				out.push('\u{201C}'); // opening double quote
				i += 2;
			}
			'`' => {
				out.push('\u{2018}'); // opening single quote
				i += 1;
			}
			'\'' if i + 1 < n && chars[i + 1] == '\'' => {
				out.push('\u{201D}'); // closing double quote
				i += 2;
			}
			'~' => {
				out.push('\u{00A0}'); // non-breaking space
				i += 1;
			}
			_ => {
				out.push(c);
				i += 1;
			}
		}
	}
	out
}

/// Decodes a control sequence beginning at `chars[i]` (a backslash). Returns the replacement text
/// and the index past what it consumed.
fn decode_control(chars: &[char], i: usize) -> (String, usize) {
	let n = chars.len();
	let mut j = i + 1; // past backslash
	if j >= n {
		return (String::new(), j);
	}
	let c = chars[j];
	// Single-character escapes and accent markers.
	if matches!(c, '&' | '%' | '$' | '#' | '_' | '{' | '}') {
		return (c.to_string(), j + 1);
	}
	if matches!(c, '\'' | '`' | '"' | '^' | '~' | '=' | '.') {
		// Accent taking the following letter, which may be `{x}` or bare `x`.
		j += 1;
		let (letter, next) = read_accent_arg(chars, j);
		if let Some(a) = apply_accent(c, letter) {
			return (a.to_string(), next);
		}
		return (letter.map(|l| l.to_string()).unwrap_or_default(), next);
	}
	// Alphabetic control word: `\c{c}`, `\v{s}`, `\L`, `\o`, `\ss`, ...
	let mut word = String::new();
	while j < n && chars[j].is_ascii_alphabetic() {
		word.push(chars[j]);
		j += 1;
	}
	// Accent commands spelled as letters take an argument.
	if matches!(word.as_str(), "c" | "v" | "u" | "H" | "r" | "k") {
		let (letter, next) = read_accent_arg(chars, j);
		if let Some(a) = apply_letter_accent(&word, letter) {
			return (a.to_string(), next);
		}
		return (letter.map(|l| l.to_string()).unwrap_or_default(), next);
	}
	if let Some(s) = special_letter(&word) {
		return (s.to_string(), j);
	}
	// Unknown command: drop the backslash, keep the word.
	(word, j)
}

/// Reads the argument of an accent: `{x}`, a bare letter, or nothing. Skips a leading space that
/// separates a control word from its letter.
fn read_accent_arg(chars: &[char], start: usize) -> (Option<char>, usize) {
	let n = chars.len();
	let mut i = start;
	while i < n && chars[i] == ' ' {
		i += 1;
	}
	if i >= n {
		return (None, i);
	}
	if chars[i] == '{' {
		i += 1;
		let letter = if i < n && chars[i] != '}' { Some(chars[i]) } else { None };
		if letter.is_some() {
			i += 1;
		}
		if i < n && chars[i] == '}' {
			i += 1;
		}
		(letter, i)
	} else {
		let letter = Some(chars[i]);
		(letter, i + 1)
	}
}

fn apply_accent(mark: char, letter: Option<char>) -> Option<char> {
	let l = letter?;
	let r = match (mark, l) {
		('\'', 'a') => 'á', ('\'', 'e') => 'é', ('\'', 'i') => 'í', ('\'', 'o') => 'ó',
		('\'', 'u') => 'ú', ('\'', 'y') => 'ý', ('\'', 'n') => 'ń', ('\'', 'c') => 'ć',
		('\'', 's') => 'ś', ('\'', 'z') => 'ź', ('\'', 'A') => 'Á', ('\'', 'E') => 'É',
		('\'', 'I') => 'Í', ('\'', 'O') => 'Ó', ('\'', 'U') => 'Ú',
		('`', 'a')  => 'à', ('`', 'e')  => 'è', ('`', 'i')  => 'ì', ('`', 'o')  => 'ò',
		('`', 'u')  => 'ù', ('`', 'A')  => 'À', ('`', 'E')  => 'È', ('`', 'O')  => 'Ò',
		('"', 'a')  => 'ä', ('"', 'e')  => 'ë', ('"', 'i')  => 'ï', ('"', 'o')  => 'ö',
		('"', 'u')  => 'ü', ('"', 'y')  => 'ÿ', ('"', 'A')  => 'Ä', ('"', 'O')  => 'Ö',
		('"', 'U')  => 'Ü',
		('^', 'a')  => 'â', ('^', 'e')  => 'ê', ('^', 'i')  => 'î', ('^', 'o')  => 'ô',
		('^', 'u')  => 'û', ('^', 'A')  => 'Â', ('^', 'O')  => 'Ô',
		('~', 'a')  => 'ã', ('~', 'n')  => 'ñ', ('~', 'o')  => 'õ', ('~', 'A')  => 'Ã',
		('~', 'N')  => 'Ñ', ('~', 'O')  => 'Õ',
		('=', 'a')  => 'ā', ('=', 'e')  => 'ē', ('=', 'i')  => 'ī', ('=', 'o')  => 'ō',
		('=', 'u')  => 'ū',
		('.', 'z')  => 'ż', ('.', 'e')  => 'ė',
		_ => return None,
	};
	Some(r)
}

fn apply_letter_accent(cmd: &str, letter: Option<char>) -> Option<char> {
	let l = letter?;
	let r = match (cmd, l) {
		("c", 'c') => 'ç', ("c", 'C') => 'Ç', ("c", 's') => 'ş', ("c", 'S') => 'Ş',
		("v", 's') => 'š', ("v", 'S') => 'Š', ("v", 'c') => 'č', ("v", 'C') => 'Č',
		("v", 'z') => 'ž', ("v", 'Z') => 'Ž', ("v", 'r') => 'ř', ("v", 'e') => 'ě',
		("v", 'n') => 'ň',
		("u", 'a') => 'ă', ("u", 'g') => 'ğ', ("u", 'G') => 'Ğ',
		("H", 'o') => 'ő', ("H", 'u') => 'ű',
		("r", 'a') => 'å', ("r", 'A') => 'Å', ("r", 'u') => 'ů',
		("k", 'a') => 'ą', ("k", 'e') => 'ę',
		_ => return None,
	};
	Some(r)
}

fn special_letter(word: &str) -> Option<char> {
	let r = match word {
		"L"  => 'Ł', "l"  => 'ł', "o"  => 'ø', "O"  => 'Ø',
		"ss" => 'ß', "ae" => 'æ', "AE" => 'Æ', "oe" => 'œ', "OE" => 'Œ',
		"aa" => 'å', "AA" => 'Å', "i"  => 'ı', "j"  => 'ȷ', "dh" => 'ð', "DH" => 'Ð',
		"th" => 'þ', "TH" => 'Þ',
		_ => return None,
	};
	Some(r)
}

// ---------------------------------------------------------------------------------------------
// Name parsing.
// ---------------------------------------------------------------------------------------------

/// Parses a BibTeX author/editor field into names. Names are separated by ` and `; a name wrapped
/// in an extra brace group (`{{...}}` in the field) is corporate and kept whole. A personal name is
/// either `Family, Given` (comma form) or `Given ... Family` (natural form), the surname being the
/// text after the comma or the final whitespace-separated token.
fn parse_names(field: &str) -> Vec<Name> {
	let mut names: Vec<Name> = Vec::new();
	for part in split_top(field) {
		let trimmed = part.trim();
		if trimmed.is_empty() {
			continue;
		}
		// Corporate: the whole part is one brace group.
		if is_braced_whole(trimmed) {
			let inner = &trimmed[1..trimmed.len() - 1];
			names.push(Name {
				family:    decode_value(inner),
				given:     String::new(),
				corporate: true,
			});
			continue;
		}
		if let Some(comma) = trimmed.find(',') {
			let family = decode_value(trimmed[..comma].trim());
			let given  = decode_value(trimmed[comma + 1..].trim());
			names.push(Name { family, given, corporate: false });
		} else {
			// Natural order: last token is the surname.
			let toks: Vec<&str> = trimmed.split_whitespace().collect();
			if toks.len() <= 1 {
				names.push(Name {
					family:    decode_value(trimmed),
					given:     String::new(),
					corporate: false,
				});
			} else {
				let family = decode_value(toks[toks.len() - 1]);
				let given  = decode_value(&toks[..toks.len() - 1].join(" "));
				names.push(Name { family, given, corporate: false });
			}
		}
	}
	names
}

/// Splits an author field on top-level ` and ` (not inside braces).
fn split_top(field: &str) -> Vec<String> {
	let chars: Vec<char> = field.chars().collect();
	let n = chars.len();
	let mut parts: Vec<String> = Vec::new();
	let mut cur = String::new();
	let mut depth = 0i32;
	let mut i = 0usize;
	while i < n {
		let c = chars[i];
		if c == '{' {
			depth += 1;
			cur.push(c);
			i += 1;
		} else if c == '}' {
			depth -= 1;
			cur.push(c);
			i += 1;
		} else if depth == 0
			&& c == ' '
			&& i + 4 < n
			&& chars[i + 1] == 'a'
			&& chars[i + 2] == 'n'
			&& chars[i + 3] == 'd'
			&& chars[i + 4] == ' '
		{
			parts.push(cur.clone());
			cur.clear();
			i += 5;
		} else {
			cur.push(c);
			i += 1;
		}
	}
	if !cur.trim().is_empty() {
		parts.push(cur);
	}
	parts
}

fn is_braced_whole(s: &str) -> bool {
	let chars: Vec<char> = s.chars().collect();
	if chars.len() < 2 || chars[0] != '{' || chars[chars.len() - 1] != '}' {
		return false;
	}
	let mut depth = 0i32;
	for (k, &c) in chars.iter().enumerate() {
		if c == '{' {
			depth += 1;
		} else if c == '}' {
			depth -= 1;
			if depth == 0 && k != chars.len() - 1 {
				return false; // the first group closes early: not one whole group
			}
		}
	}
	depth == 0
}

// ---------------------------------------------------------------------------------------------
// In-text citation label.
// ---------------------------------------------------------------------------------------------

/// The author-year label for one entry, e.g. `Scott 1976`, `Kahneman and Tversky 1979`,
/// `Acemoglu et al. 2001`. Three or more names collapse to the first plus `et al.`.
fn cite_label(entry: &Entry) -> String {
	let names = entry.credited();
	let year  = entry.field("year").map(|y| year_display(y)).unwrap_or_else(|| "n.d.".to_string());
	let who = match names.len() {
		0 => entry.field("title").map(|t| chicago_title_case(t)).unwrap_or_default(),
		1 => names[0].family().to_string(),
		2 => fmt!("{} and {}", names[0].family(), names[1].family()),
		_ => fmt!("{} et al.", names[0].family()),
	};
	if who.is_empty() {
		year
	} else {
		fmt!("{} {}", who, year)
	}
}

/// The visible year: the leading run of digits (drops a BibTeX `{2024}` note or a trailing letter).
fn year_display(y: &str) -> String {
	let digits: String = y.chars().take_while(|c| c.is_ascii_digit()).collect();
	if digits.is_empty() { y.trim().to_string() } else { digits }
}

// ---------------------------------------------------------------------------------------------
// Reference list formatting (Chicago author-date).
// ---------------------------------------------------------------------------------------------

/// A small builder that accumulates runs, merging consecutive same-style text.
struct RunBuilder {
	runs: Vec<RefRun>,
}

impl RunBuilder {
	fn new() -> Self { Self { runs: Vec::new() } }

	fn push(&mut self, text: &str, style: RefStyle) {
		if text.is_empty() {
			return;
		}
		if let Some(last) = self.runs.last_mut() {
			if last.style == style {
				last.text.push_str(text);
				return;
			}
		}
		self.runs.push(RefRun { text: text.to_string(), style });
	}

	fn normal(&mut self, text: &str) { self.push(text, RefStyle::Normal); }
	fn italic(&mut self, text: &str) { self.push(text, RefStyle::Italic); }

	fn finish(mut self, key: &str) -> Reference {
		smartquote_runs(&mut self.runs);
		Reference { key: key.to_string(), runs: self.runs }
	}
}

/// Formats one entry as a Chicago author-date reference.
fn format_reference(entry: &Entry) -> Reference {
	let mut b = RunBuilder::new();
	// Author. Year.
	let author = author_list(&entry.credited(), entry.field("author").is_none());
	if !author.is_empty() {
		b.normal(&author);
		// A name ending in an initial already carries its full stop.
		if author.ends_with('.') {
			b.normal(" ");
		} else {
			b.normal(". ");
		}
	}
	if let Some(y) = entry.field("year") {
		b.normal(&year_display(y));
		b.normal(". ");
	}
	match entry.kind {
		EntryKind::Article => format_article(&mut b, entry),
		EntryKind::InCollection | EntryKind::InProceedings => format_incollection(&mut b, entry),
		EntryKind::TechReport | EntryKind::Report => format_report(&mut b, entry),
		EntryKind::Online => format_online(&mut b, entry),
		_ => format_book(&mut b, entry),
	}
	b.finish(entry.key())
}

/// The author string for the list: first name inverted, the rest natural, `and` before the last,
/// with an `, ed.`/`, eds.` tag when the credited names are editors.
fn author_list(names: &[Name], is_editor: bool) -> String {
	let body = match names.len() {
		0 => String::new(),
		1 => names[0].inverted(),
		2 => fmt!("{}, and {}", names[0].inverted(), names[1].natural()),
		_ => {
			let mut s = names[0].inverted();
			for name in &names[1..names.len() - 1] {
				s.push_str(", ");
				s.push_str(&name.natural());
			}
			s.push_str(", and ");
			s.push_str(&names[names.len() - 1].natural());
			s
		}
	};
	if is_editor && !body.is_empty() {
		let tag = if names.len() > 1 { ", eds." } else { ", ed." };
		fmt!("{}{}", body, tag)
	} else {
		body
	}
}

fn format_book(b: &mut RunBuilder, entry: &Entry) {
	if let Some(t) = entry.field("title") {
		b.italic(&chicago_title_case(t));
		b.normal(". ");
	}
	// Chicago author-date omits the place for a book with a named publisher.
	if let Some(p) = entry.field("publisher") {
		b.normal(p);
		b.normal(".");
	} else if let Some(a) = entry.field("address") {
		b.normal(a);
		b.normal(".");
	}
	append_url(b, entry);
}

fn format_article(b: &mut RunBuilder, entry: &Entry) {
	if let Some(t) = entry.field("title") {
		b.normal("\u{201C}");
		b.normal(&chicago_title_case(t));
		b.normal(".\u{201D} ");
	}
	if let Some(j) = entry.field("journal").or_else(|| entry.field("journaltitle")) {
		b.italic(&chicago_title_case(j));
		match entry.field("volume") {
			Some(v) => {
				b.normal(" ");
				b.normal(v);
				if let Some(num) = entry.field("number") {
					b.normal(&fmt!(" ({})", num));
				}
				if let Some(p) = entry.field("pages") {
					b.normal(&fmt!(": {}", compress_pages(p)));
				}
				b.normal(".");
			}
			// Typst sets a bare comma after the journal when there is no volume.
			None => b.normal(",."),
		}
	}
	append_url(b, entry);
}

fn format_incollection(b: &mut RunBuilder, entry: &Entry) {
	if let Some(t) = entry.field("title") {
		b.normal("\u{201C}");
		b.normal(&chicago_title_case(t));
		b.normal(".\u{201D} ");
	}
	b.normal("In ");
	if let Some(bt) = entry.field("booktitle") {
		b.italic(&chicago_title_case(bt));
	}
	if let Some(ed) = entry.field("editor") {
		let eds = parse_names(ed);
		b.normal(&fmt!(", edited by {}", natural_join(&eds)));
	}
	if let Some(p) = entry.field("pages") {
		b.normal(&fmt!(", {}", compress_pages(p)));
	}
	b.normal(". ");
	if let Some(p) = entry.field("publisher") {
		b.normal(p);
		b.normal(".");
	} else if let Some(a) = entry.field("address") {
		b.normal(a);
		b.normal(".");
	}
	append_url(b, entry);
}

fn format_report(b: &mut RunBuilder, entry: &Entry) {
	if let Some(t) = entry.field("title") {
		b.italic(&chicago_title_case(t));
		b.normal(". ");
	}
	let kind = entry.field("type").unwrap_or("Working Paper");
	match entry.field("number") {
		Some(num) => b.normal(&fmt!("{} No. {}. ", kind, num)),
		None => {
			b.normal(kind);
			b.normal(". ");
		}
	}
	if let Some(inst) = entry.field("institution") {
		// The address stands in for the imprint when present; otherwise the institution.
		match entry.field("address") {
			Some(a) => {
				b.normal(a);
				b.normal(".");
			}
			None => {
				b.normal(inst);
				b.normal(".");
			}
		}
	} else if let Some(a) = entry.field("address") {
		b.normal(a);
		b.normal(".");
	}
	append_url(b, entry);
}

fn format_online(b: &mut RunBuilder, entry: &Entry) {
	if let Some(t) = entry.field("title") {
		b.italic(&chicago_title_case(t));
		b.normal(". ");
	}
	if let Some(pubr) = entry.field("publisher").or_else(|| entry.field("organization")) {
		b.normal(pubr);
		b.normal(". ");
	}
	if let Some(u) = entry.field("url") {
		b.normal(u);
		b.normal(".");
	}
}

/// Appends a DOI (as an `https://doi.org/` URL) or a bare URL, whichever is present.
fn append_url(b: &mut RunBuilder, entry: &Entry) {
	if let Some(doi) = entry.field("doi") {
		b.normal(&fmt!(" https://doi.org/{}.", doi.trim()));
	} else if let Some(u) = entry.field("url") {
		b.normal(&fmt!(" {}.", u.trim()));
	}
}

/// Joins names in natural order with commas and a final `and`.
fn natural_join(names: &[Name]) -> String {
	match names.len() {
		0 => String::new(),
		1 => names[0].natural(),
		2 => fmt!("{} and {}", names[0].natural(), names[1].natural()),
		_ => {
			let mut s = String::new();
			for name in &names[..names.len() - 1] {
				s.push_str(&name.natural());
				s.push_str(", ");
			}
			s.push_str("and ");
			s.push_str(&names[names.len() - 1].natural());
			s
		}
	}
}

// ---------------------------------------------------------------------------------------------
// Sorting.
// ---------------------------------------------------------------------------------------------

/// Sorts references Chicago-style. Works are ordered by the first author's surname; among works by
/// the same first author, a work with fewer authors comes first (a solo work before coauthored
/// ones), then by the coauthors' surnames, then by year, then by title. This author-count tie-break
/// is Typst's behaviour and was verified against a render.
fn sort_entries(entries: &mut [&Entry]) {
	entries.sort_by(|a, b| sort_key(a).cmp(&sort_key(b)));
}

fn sort_key(entry: &Entry) -> (String, usize, Vec<String>, i64, String) {
	let names = entry.credited();
	let first = names.first().map(|n| fold_key(n.family())).unwrap_or_default();
	let count = names.len();
	let rest: Vec<String> = names.iter().skip(1).map(|n| fold_key(n.family())).collect();
	let year  = entry.year().unwrap_or(i64::MAX);
	let title = entry.field("title").map(|t| fold_key(t)).unwrap_or_default();
	(first, count, rest, year, title)
}

/// A case- and accent-folded sort key for a surname or title.
fn fold_key(s: &str) -> String {
	s.chars()
		.filter(|c| c.is_alphanumeric() || *c == ' ')
		.flat_map(|c| c.to_lowercase())
		.collect()
}

// ---------------------------------------------------------------------------------------------
// Chicago page-range abbreviation.
// ---------------------------------------------------------------------------------------------

/// Abbreviates a page range by the Chicago rule and sets the separator as an en dash. `263--291`
/// becomes `263-91`, `1369--1401` becomes `1369-401`, and `488--500` stays `488-500`. A range whose
/// ends differ in length, or a non-numeric range, keeps both ends whole.
fn compress_pages(pages: &str) -> String {
	let norm = pages.replace("--", "-").replace('\u{2013}', "-");
	let bits: Vec<&str> = norm.splitn(2, '-').collect();
	if bits.len() != 2 {
		return norm.trim().to_string();
	}
	let a = bits[0].trim();
	let z = bits[1].trim();
	let en = '\u{2013}';
	let (an, zn) = (a.parse::<i64>(), z.parse::<i64>());
	match (an, zn) {
		(Ok(av), Ok(_)) if a.len() == z.len() => {
			if av < 100 || av % 100 == 0 {
				fmt!("{}{}{}", a, en, z)
			} else {
				let ac: Vec<char> = a.chars().collect();
				let zc: Vec<char> = z.chars().collect();
				let mut common = 0usize;
				while common < ac.len() && ac[common] == zc[common] {
					common += 1;
				}
				let min_keep = if av % 100 <= 9 { 1 } else { 2 };
				let keep = std::cmp::max(zc.len() - common, min_keep);
				let tail: String = zc[zc.len() - keep..].iter().collect();
				fmt!("{}{}{}", a, en, tail)
			}
		}
		_ => fmt!("{}{}{}", a, en, z),
	}
}

// ---------------------------------------------------------------------------------------------
// Chicago headline-style title casing.
// ---------------------------------------------------------------------------------------------

/// Downcases the minor words of an already-capitalised title to Typst's Chicago headline style,
/// e.g. `Decision Under Risk` becomes `Decision under Risk`. The first word, and the first word
/// after a colon, stay capitalised; a minor word anywhere else is downcased, including in final
/// position (`Bringing the State Back In` becomes `... Back in`). Other words keep their source
/// capitalisation, so a deliberately capitalised acronym in the source is preserved.
fn chicago_title_case(title: &str) -> String {
	let words: Vec<&str> = title.split(' ').collect();
	let mut out: Vec<String> = Vec::with_capacity(words.len());
	let mut start_of_clause = true; // first word, or first after a colon
	for (k, w) in words.iter().enumerate() {
		let lowered = w.to_lowercase();
		let bare: String = lowered.chars().filter(|c| c.is_alphabetic()).collect();
		let is_minor = MINOR_WORDS.contains(&bare.as_str());
		// Typst downcases a minor word wherever it falls, save the start of a clause -- so even
		// a trailing "In" becomes "in" ("Bringing the State Back in").
		let downcase = is_minor && !start_of_clause && k != 0;
		if downcase {
			out.push(lowered);
		} else {
			out.push(w.to_string());
		}
		// The next word starts a clause if this word ends with a colon.
		start_of_clause = w.ends_with(':');
	}
	out.join(" ")
}

/// The articles, coordinating conjunctions and short prepositions Chicago sets lower case when they
/// fall inside a title.
const MINOR_WORDS: &[&str] = &[
	"a", "an", "the",
	"and", "but", "or", "nor", "for", "so", "yet",
	"as", "at", "by", "in", "of", "off", "on", "per", "to", "up", "via",
	"from", "into", "like", "near", "onto", "over", "than", "that", "till",
	"unto", "upon", "with", "about", "above", "after", "among", "under",
];

// ---------------------------------------------------------------------------------------------
// Smart quotes.
// ---------------------------------------------------------------------------------------------

/// Turns straight apostrophes and straight double quotes in the runs into curly quotes, matching
/// Typst's smart-quote pass. TeX quote pairs were already converted in [`decode_value`].
fn smartquote_runs(runs: &mut [RefRun]) {
	for run in runs.iter_mut() {
		run.text = smartquote(&run.text);
	}
}

fn smartquote(s: &str) -> String {
	let chars: Vec<char> = s.chars().collect();
	let n = chars.len();
	let mut out = String::with_capacity(n);
	let mut dq_open = false;
	for (i, &c) in chars.iter().enumerate() {
		match c {
			'\'' => {
				let prev_alnum = i > 0 && chars[i - 1].is_alphanumeric();
				if prev_alnum {
					out.push('\u{2019}'); // apostrophe / closing single
				} else {
					out.push('\u{2018}');
				}
			}
			'"' => {
				if dq_open {
					out.push('\u{201D}');
					dq_open = false;
				} else {
					out.push('\u{201C}');
					dq_open = true;
				}
			}
			_ => out.push(c),
		}
	}
	out
}

#[cfg(test)]
mod tests {
	use super::*;

	const SAMPLE: &str = r#"
% A comment line that must be skipped.
@book{scott1976moral,
  author    = {Scott, James C.},
  title     = {The Moral Economy of the Peasant: Rebellion and Subsistence in Southeast Asia},
  year      = {1976},
  publisher = {Yale University Press},
  address   = {New Haven}
}

@article{kahneman1979prospect,
  author    = {Kahneman, Daniel and Tversky, Amos},
  title     = {Prospect Theory: An Analysis of Decision Under Risk},
  journal   = {Econometrica},
  volume    = {47},
  number    = {2},
  pages     = {263--291},
  year      = {1979},
  doi       = {10.2307/1914185}
}

@article{acemoglu2001colonial,
  author    = {Acemoglu, Daron and Johnson, Simon and Robinson, James A.},
  title     = {The Colonial Origins of Comparative Development: An Empirical Investigation},
  journal   = {American Economic Review},
  volume    = {91},
  number    = {5},
  pages     = {1369--1401},
  year      = {2001},
  doi       = {10.1257/aer.91.5.1369}
}

@techreport{acemoglu2024simple,
  author      = {Acemoglu, Daron},
  title       = {The Simple Macroeconomics of {AI}},
  institution = {National Bureau of Economic Research},
  type        = {Working Paper},
  number      = {32487},
  year        = {2024},
  address     = {Cambridge, MA}
}

@book{worldbank2024,
  author    = {World Bank},
  title     = {Education Finance Watch 2024},
  year      = {2024},
  institution = {World Bank Group}
}

@incollection{tilly1985war,
  author    = {Tilly, Charles},
  title     = {War Making and State Making as Organized Crime},
  booktitle = {Bringing the State Back In},
  editor    = {Evans, Peter B. and Rueschemeyer, Dietrich and Skocpol, Theda},
  publisher = {Cambridge University Press},
  address   = {Cambridge},
  year      = {1985},
  pages     = {169--191}
}

@book{kornai1992socialist,
  author    = {Kornai, J{\'a}nos},
  title     = {The Socialist System},
  year      = {1992},
  publisher = {Princeton University Press}
}
"#;

	fn bib() -> Bibliography {
		match Bibliography::parse(SAMPLE) {
			Ok(b) => b,
			Err(e) => panic!("parse failed: {}", e),
		}
	}

	#[test]
	fn parses_all_entries() {
		let b = bib();
		assert_eq!(b.len(), 7);
		let scott = b.entry("scott1976moral").expect("scott present");
		assert_eq!(scott.kind(), EntryKind::Book);
		assert_eq!(scott.field("year"), Some("1976"));
		assert_eq!(scott.field("publisher"), Some("Yale University Press"));
	}

	#[test]
	fn decodes_accent_and_brace() {
		let b = bib();
		let k = b.entry("kornai1992socialist").expect("kornai present");
		let names = k.credited();
		assert_eq!(names[0].family(), "Kornai");
		assert_eq!(names[0].given(), "János"); // {\'a} decoded
		// The braced {AI} keeps its letters, braces dropped.
		let tr = b.entry("acemoglu2024simple").expect("report present");
		assert_eq!(tr.field("title"), Some("The Simple Macroeconomics of AI"));
	}

	#[test]
	fn parses_multiple_authors() {
		let b = bib();
		let a = b.entry("acemoglu2001colonial").expect("present");
		let names = a.credited();
		assert_eq!(names.len(), 3);
		assert_eq!(names[1].family(), "Johnson");
		assert_eq!(names[1].given(), "Simon");
	}

	#[test]
	fn corporate_single_brace_is_inverted_as_personal() {
		// Typst reads `{World Bank}` as First=World, Last=Bank and inverts it in the list.
		let b = bib();
		let refr = format_reference(b.entry("worldbank2024").expect("present"));
		assert!(refr.plain().starts_with("Bank, World. 2024."),
			"got: {}", refr.plain());
	}

	#[test]
	fn in_text_labels() {
		let b = bib();
		assert_eq!(b.label("scott1976moral").expect("ok"), "Scott 1976");
		assert_eq!(b.label("kahneman1979prospect").expect("ok"), "Kahneman and Tversky 1979");
		assert_eq!(b.label("acemoglu2001colonial").expect("ok"), "Acemoglu et al. 2001");
	}

	#[test]
	fn citation_single_and_multiple() {
		let mut b = bib();
		assert_eq!(b.cite(&["scott1976moral"]).expect("ok"), "(Scott 1976)");
		let multi = b.cite(&["scott1976moral", "kahneman1979prospect"]).expect("ok");
		assert_eq!(multi, "(Scott 1976; Kahneman and Tversky 1979)");
	}

	#[test]
	fn unknown_key_errors() {
		let b = bib();
		assert!(b.format_citation(&["nope"]).is_err());
	}

	#[test]
	fn book_reference_form() {
		let b = bib();
		let r = format_reference(b.entry("scott1976moral").expect("present"));
		assert_eq!(
			r.plain(),
			"Scott, James C. 1976. The Moral Economy of the Peasant: Rebellion and Subsistence in Southeast Asia. Yale University Press.");
		// The title is one italic run.
		assert!(r.runs.iter().any(|run| run.style == RefStyle::Italic
			&& run.text.starts_with("The Moral Economy")));
	}

	#[test]
	fn article_reference_form_with_page_compression() {
		let b = bib();
		let r = format_reference(b.entry("kahneman1979prospect").expect("present"));
		// Title down-cased "Under" -> "under"; pages 263--291 -> 263-91; DOI as a URL.
		assert_eq!(
			r.plain(),
			"Kahneman, Daniel, and Amos Tversky. 1979. \u{201C}Prospect Theory: An Analysis of Decision under Risk.\u{201D} Econometrica 47 (2): 263\u{2013}91. https://doi.org/10.2307/1914185.");
	}

	#[test]
	fn three_author_article_and_long_page_range() {
		let b = bib();
		let r = format_reference(b.entry("acemoglu2001colonial").expect("present"));
		assert_eq!(
			r.plain(),
			"Acemoglu, Daron, Simon Johnson, and James A. Robinson. 2001. \u{201C}The Colonial Origins of Comparative Development: An Empirical Investigation.\u{201D} American Economic Review 91 (5): 1369\u{2013}401. https://doi.org/10.1257/aer.91.5.1369.");
	}

	#[test]
	fn report_reference_form() {
		let b = bib();
		let r = format_reference(b.entry("acemoglu2024simple").expect("present"));
		assert_eq!(
			r.plain(),
			"Acemoglu, Daron. 2024. The Simple Macroeconomics of AI. Working Paper No. 32487. Cambridge, MA.");
	}

	#[test]
	fn incollection_reference_form() {
		let b = bib();
		let r = format_reference(b.entry("tilly1985war").expect("present"));
		assert_eq!(
			r.plain(),
			"Tilly, Charles. 1985. \u{201C}War Making and State Making as Organized Crime.\u{201D} In Bringing the State Back in, edited by Peter B. Evans, Dietrich Rueschemeyer, and Theda Skocpol, 169\u{2013}91. Cambridge University Press.");
	}

	#[test]
	fn page_compression_rule() {
		assert_eq!(compress_pages("263--291"), "263\u{2013}91");
		assert_eq!(compress_pages("1369--1401"), "1369\u{2013}401");
		assert_eq!(compress_pages("488--500"), "488\u{2013}500");
		assert_eq!(compress_pages("855--857"), "855\u{2013}57");
		assert_eq!(compress_pages("224--232"), "224\u{2013}32");
		assert_eq!(compress_pages("97--112"), "97\u{2013}112");
		assert_eq!(compress_pages("1--42"), "1\u{2013}42");
	}

	#[test]
	fn reference_list_is_cited_only_and_sorted() {
		let mut b = bib();
		// Cite three works by, or beginning with, Acemoglu plus Scott, out of order.
		let _ = b.cite(&["scott1976moral"]);
		let _ = b.cite(&["acemoglu2001colonial"]);
		let _ = b.cite(&["acemoglu2024simple"]);
		let list = b.reference_list();
		assert_eq!(list.len(), 3);
		// Acemoglu solo (2024) sorts before Acemoglu-and-coauthors (2001); Scott last.
		assert_eq!(list[0].key, "acemoglu2024simple");
		assert_eq!(list[1].key, "acemoglu2001colonial");
		assert_eq!(list[2].key, "scott1976moral");
	}

	#[test]
	fn empty_cited_list_is_empty() {
		let b = bib();
		assert!(b.reference_list().is_empty());
	}

	#[test]
	#[ignore]
	fn smoke_real_file() {
		let path = match std::env::var("REAL_BIB") { Ok(p) => p, Err(_) => return };
		let src = std::fs::read_to_string(&path).expect("read real bib");
		let b = Bibliography::parse(&src).expect("parse real bib");
		println!("REAL entries parsed: {}", b.len());
		let mut authored = 0usize;
		let mut yearless = 0usize;
		for e in &b.entries {
			if !e.credited().is_empty() { authored += 1; }
			if e.field("year").is_none() { yearless += 1; }
		}
		println!("with credited names: {} ; without year: {}", authored, yearless);
		// Spot-check a few keys against the oracle page 678.
		for key in ["scott1976moral", "kahneman1979prospect", "hirschman1970exit"] {
			if let Some(e) = b.entry(key) {
				println!("[{}] cite = {}", key, cite_label(e));
				println!("[{}] ref  = {}", key, format_reference(e).plain());
			}
		}
	}

	#[test]
	fn author_count_tie_break() {
		// A solo work sorts before a coauthored one by the same first author.
		let src = r#"
@book{a2012,author={Acemoglu, Daron and Robinson, James A.},title={Why Nations Fail},publisher={Crown},year={2012}}
@book{a2023,author={Acemoglu, Daron and Johnson, Simon},title={Power and Progress},publisher={PublicAffairs},year={2023}}
@techreport{a2024,author={Acemoglu, Daron},title={Simple},institution={NBER},year={2024}}
"#;
		let b = match Bibliography::parse(src) { Ok(b) => b, Err(e) => panic!("{}", e) };
		let all = b.all_references();
		assert_eq!(all[0].key, "a2024"); // solo first
		assert_eq!(all[1].key, "a2023"); // Johnson before Robinson
		assert_eq!(all[2].key, "a2012");
	}
}
