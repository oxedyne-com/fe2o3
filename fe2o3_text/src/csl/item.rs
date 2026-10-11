//! Ported from hayagriva 0.10.1, Copyright (c) Martin Haug and the hayagriva contributors,
//! MIT OR Apache-2.0 (`src/types/strings.rs`, `src/types/numeric.rs`, `src/csl/taxonomy.rs`).
//!
//! The item model is the least the renderers read. Names, dates and page ranges arrive with B2b
//! through the hooks named on [`Item`].
use crate::csl::style::VarForm;



// Chunked strings
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChunkKind {
	Normal,
	Verbatim,	// case-protected, BibTeX's `{DNA}`
	Math,		// BibTeX's `$...$`, rendered by the host
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Chunk {
	pub kind:	ChunkKind,
	pub value:	String,
}

/// A text variable as pieces, so that text-case leaves protected and mathematical spans alone.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Chunked(pub Vec<Chunk>);

impl Chunked {

	pub fn normal(s: &str) -> Self {
		Self(vec![Chunk { kind: ChunkKind::Normal, value: s.to_string() }])
	}

	pub fn push(&mut self, kind: ChunkKind, s: &str) {
		match self.0.last_mut() {
			Some(last) if last.kind == kind && kind != ChunkKind::Math => last.value.push_str(s),
			_ => self.0.push(Chunk { kind, value: s.to_string() }),
		}
	}

	pub fn is_empty(&self) -> bool {
		self.0.iter().all(|c| c.value.is_empty())
	}

	/// Is it empty or only whitespace?
	pub fn is_blank(&self) -> bool {
		self.0.iter().all(|c| c.value.chars().all(char::is_whitespace))
	}
}

impl std::fmt::Display for Chunked {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		for c in &self.0 {
			match c.kind {
				ChunkKind::Math	=> { let _ = write!(f, "${}$", c.value); }
				_		=> { let _ = f.write_str(&c.value); }
			}
		}
		Ok(())
	}
}

impl From<&str> for Chunked {
	fn from(s: &str) -> Self {
		Self::normal(s)
	}
}

// Numbers
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NumDelim { Comma, Ampersand, Hyphen }

impl NumDelim {

	fn from_char(c: char) -> Option<Self> {
		match c {
			','		=> Some(Self::Comma),
			'&'		=> Some(Self::Ampersand),
			'-' | '–'	=> Some(Self::Hyphen),
			_		=> None,
		}
	}

	pub fn as_str(self) -> &'static str {
		match self {
			Self::Comma	=> ", ",
			Self::Ampersand	=> " & ",
			Self::Hyphen	=> "–",
		}
	}
}

fn is_num_delim(c: char) -> bool {
	NumDelim::from_char(c).is_some()
}

/// A number variable that parsed as numbers: one, or a set joined by `,`, `&` or a hyphen, with an
/// optional non-numeric prefix and suffix (`2nd`, `S12`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Numeric {
	pub nums:	Vec<(i32, Option<NumDelim>)>,	// a single number has one entry and no delimiter
	pub prefix:	Option<String>,
	pub suffix:	Option<String>,
}

impl Numeric {

	pub fn new(n: i32) -> Self {
		Self { nums: vec![(n, None)], prefix: None, suffix: None }
	}

	/// Reads a number as hayagriva's `Numeric::from_str` does, which is stricter than CSL's
	/// "contains a number": `12a` is numeric (suffix `a`) but `12 a` with text after the suffix
	/// is not.
	pub fn parse(src: &str) -> Option<Self> {
		let chars: Vec<char> = src.chars().collect();
		let mut i = 0;
		let eat_ws = |i: &mut usize| {
			let start = *i;
			while *i < chars.len() && chars[*i].is_whitespace() { *i += 1; }
			chars[start..*i].iter().collect::<String>()
		};
		eat_ws(&mut i);
		// Prefix: anything up to a digit or '-', leaving a run of zeros before a non-digit.
		let pstart = i;
		while i < chars.len() && !chars[i].is_numeric() && chars[i] != '-' { i += 1; }
		let zstart = i;
		while i < chars.len() && chars[i] == '0' { i += 1; }
		if i > zstart && (i >= chars.len() || !chars[i].is_numeric()) {
			i -= 1; // unscanny's `uneat` gives back one character
		}
		let prefix: String = chars[pstart..i].iter().collect();
		let number = |i: &mut usize| -> Option<i32> {
			let neg = *i < chars.len() && chars[*i] == '-';
			if neg { *i += 1; }
			let start = *i;
			while *i < chars.len() && chars[*i].is_numeric() { *i += 1; }
			if *i == start {
				return None;
			}
			let s: String = chars[start..*i].iter().collect();
			s.parse::<i32>().ok().map(|n| if neg { -n } else { n })
		};
		let first = number(&mut i)?;
		let space_after = eat_ws(&mut i);
		let mut nums = Vec::new();
		match chars.get(i) {
			Some(&c) if is_num_delim(c) => {
				i += 1;
				while i < chars.len() && is_num_delim(chars[i]) { i += 1; }
				nums.push((first, NumDelim::from_char(c)));
				loop {
					eat_ws(&mut i);
					let n = number(&mut i)?;
					eat_ws(&mut i);
					let dstart = i;
					while i < chars.len() && is_num_delim(chars[i]) { i += 1; }
					let d: String = chars[dstart..i].iter().collect();
					let delim = match d.chars().next() {
						Some(c0) if c0 != '-' && d.len() > c0.len_utf8()	=> None,
						Some(c0)						=> NumDelim::from_char(c0),
						None							=> None,
					};
					match delim {
						Some(d)	=> nums.push((n, Some(d))),
						None	=> { nums.push((n, None)); break; }
					}
				}
			}
			_ => nums.push((first, None)),
		}
		eat_ws(&mut i);
		let pstart2 = i;
		while i < chars.len() && !chars[i].is_numeric() && !chars[i].is_whitespace() { i += 1; }
		let post: String = chars[pstart2..i].iter().collect();
		if i < chars.len() {
			return None;
		}
		Some(Self {
			nums,
			prefix: if prefix.is_empty() { None } else { Some(prefix) },
			suffix: if post.is_empty() { None } else { Some(format!("{}{}", space_after, post)) },
		})
	}

	/// Can a number form (ordinal, roman) be applied, there being no prefix or suffix?
	pub fn will_transform(&self) -> bool {
		self.prefix.is_none() && self.suffix.is_none()
	}

	/// Is it plural for a label? `number-of-*` variables count, others are plural when a set.
	pub fn is_plural(&self, is_number_of: bool) -> bool {
		match self.nums.len() {
			1 if is_number_of	=> self.nums[0].0 != 1,
			1			=> false,
			_			=> true,
		}
	}
}

impl std::fmt::Display for Numeric {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		if let Some(p) = &self.prefix { let _ = f.write_str(p); }
		for (n, d) in &self.nums {
			let _ = write!(f, "{}", n);
			if let Some(d) = d { let _ = f.write_str(d.as_str()); }
		}
		if let Some(s) = &self.suffix { let _ = f.write_str(s); }
		Ok(())
	}
}

/// A number variable's value: parsed when it reads as a number, otherwise the string.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NumVal {
	Typed(Numeric),
	Str(String),
	Transparent(usize),	// a locator the host renders itself, by citation index
}

impl NumVal {
	pub fn from_str(s: &str) -> Self {
		match Numeric::parse(s) {
			Some(n)	=> Self::Typed(n),
			None	=> Self::Str(s.to_string()),
		}
	}
}

// Variable taxonomy
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VarKind { Standard, Number, Page, Date, Name }

const NUMBER_VARS: [&str; 17] = [
	"chapter-number", "citation-number", "collection-number", "edition",
	"first-reference-note-number", "issue", "locator", "number", "number-of-pages",
	"number-of-volumes", "page-first", "part-number", "printing-number", "section",
	"supplement-number", "version", "volume",
];
const DATE_VARS: [&str; 6] = [
	"accessed", "available-date", "event-date", "issued", "original-date", "submitted",
];
const NAME_VARS: [&str; 30] = [
	"author", "chair", "collection-editor", "compiler", "composer", "container-author",
	"contributor", "curator", "director", "editor", "editor-translator", "editorial-director",
	"executive-producer", "guest", "host", "illustrator", "interviewer", "narrator", "organizer",
	"original-author", "performer", "producer", "recipient", "reviewed-author", "script-writer",
	"series-creator", "translator", "commenter", "co-author", "colorist",
];

/// What kind of CSL variable a name is.
pub fn var_kind(name: &str) -> VarKind {
	if name == "page" {
		VarKind::Page
	} else if NUMBER_VARS.contains(&name) {
		VarKind::Number
	} else if DATE_VARS.contains(&name) {
		VarKind::Date
	} else if NAME_VARS.contains(&name) {
		VarKind::Name
	} else {
		VarKind::Standard
	}
}

/// Does a number variable count things, so that a single `1` is singular and any other plural?
pub fn is_number_of(name: &str) -> bool {
	name.starts_with("number-of-")
}

/// The entry being rendered, as the renderers read it.
///
/// Variable names are CSL's. `citation-number`, `citation-label`, `locator` and
/// `first-reference-note-number` are answered by the citation (see `render::Cite`), not here.
pub trait Item {

	/// A standard (text) variable, in the form asked for. A short form the entry lacks should
	/// return the long one, as hayagriva does.
	fn text(&self, var: &str, form: VarForm) -> Option<Chunked>;

	/// A number variable's raw value. The renderer parses it.
	fn number(&self, var: &str) -> Option<String>;

	/// The `page` variable's raw value. B2b turns it into page ranges.
	fn page(&self) -> Option<String> { None }

	/// The CSL item type, such as `article-journal`.
	fn csl_type(&self) -> &str;

	/// Does the entry match a CSL type in `choose`? hayagriva lets one entry match several.
	fn is_type(&self, t: &str) -> bool { self.csl_type() == t }

	/// Is the entry in English? `None` when its language is unknown.
	fn is_english(&self) -> Option<bool> { None }

	/// Hook for B2b: how many names a name variable holds.
	fn name_count(&self, _var: &str) -> usize { 0 }

	/// Hook for B2b: whether a date variable is present, and if so whether it is approximate.
	fn date_state(&self, _var: &str) -> Option<bool> { None }
}
