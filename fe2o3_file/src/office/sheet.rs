//! A neutral spreadsheet: what a grid of cells *is*, free of the format it was stored in.
//!
//! The counterpart to [`oxedyne_fe2o3_text::doc`] for the other kind of document. That tree carries
//! prose and cannot carry a number that is also a formula that is also a date; this carries exactly
//! that and no prose beyond a cell's text.
//!
//! # The stored value is the value
//!
//! A cell holding a formula holds **two** things: the formula, and the value the last calculation
//! left beside it. [`Cell::value`] is that stored value and **nothing here ever recalculates**.
//!
//! That is not laziness about writing an expression evaluator. It is the correct answer twice over.
//! The stored value is the number the person who wrote the file *saw*, which is what a reader is
//! asking about. And recalculating would break byte-identity the moment a volatile function is
//! present -- `NOW`, `TODAY`, `RAND`, `RANDBETWEEN` all change on every open -- so a file opened and
//! saved with no edit would differ from itself, and the check that exists to catch a damaging edit
//! would fire on a healthy file instead. A gate that cries wolf is a gate somebody turns off.
//!
//! # Addressing
//!
//! [`Ref`] and [`Range`] are the `A1` and `A1:D20` a person types, parsed and printed. They are
//! zero-based inside and one-based on the page, because a spreadsheet counts rows from one and
//! nothing is served by pretending otherwise at the boundary.
//!
//! [Written entirely with AI](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_text::doc::{
	Block,
	Doc,
};

//// The largest a sheet may be, which is Excel's own limit.
pub const MAX_COL: u32 = 16_384;	// column XFD
pub const MAX_ROW: u32 = 1_048_576;

/// What one cell holds.
///
/// A closed set, on purpose. Everything a spreadsheet stores is one of these; what varies between
/// the formats is the spelling.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Value {
	#[default]
	Empty,	// never filled in, or emptied
	Text(String),
	Number(f64),
	// A date or a time, as the text it was rendered to. Held as text rather than as a number with a
	// format beside it, because the number alone is meaningless -- `45678` is a date only if
	// something says so -- and the format alone cannot be applied without a calendar. The conversion
	// happens once, where the format is known.
	Date(String),
	Bool(bool),
	Error(String),	// what a calculation left where it could not produce a value: `#DIV/0!`, `#N/A`
}

impl Value {

	/// The value as a person would see it, with nothing added.
	///
	/// A number prints without a trailing `.0`, because a spreadsheet showing `12.0` where the file
	/// says `12` has changed what the file says.
	pub fn show(&self) -> String {
		match self {
			Self::Empty		=> String::new(),
			Self::Text(t)		=> t.clone(),
			Self::Date(d)		=> d.clone(),
			Self::Error(e)		=> e.clone(),
			Self::Bool(b)		=> match b {
				true	=> "TRUE".to_string(),
				false	=> "FALSE".to_string(),
			},
			Self::Number(n)	=> show_number(*n),
		}
	}

	pub fn is_empty(&self) -> bool {
		matches!(self, Self::Empty)
	}
}

/// A number as a spreadsheet shows it.
///
/// **Fifteen significant figures, which is what a spreadsheet keeps.** Not the shortest text that
/// round-trips the `f64`, which is what Rust prints by default and which is a different thing: `0.1 +
/// 0.2` round-trips as `0.30000000000000004`, and the file says `0.3`. Showing the binary
/// representation's noise would be showing a number nobody entered and no reader displays.
///
/// A whole number prints without a trailing `.0`, because a cell showing `12.0` where the file says
/// `12` has changed what the file says.
fn show_number(n: f64) -> String {
	if !n.is_finite() {
		// A spreadsheet has no infinity and no NaN; one that arrived here came from a damaged file.
		return "#NUM!".to_string();
	}
	if n == n.trunc() && n.abs() < 1e15 {
		// `+ 0.0` so negative zero prints as `0`. A cell holding -0.0 is a cell holding nothing a
		// person would call negative.
		return fmt!("{}", (n + 0.0) as i64);
	}
	// Fourteen decimals in the mantissa is fifteen significant figures. Rounding through the text and
	// back lands on the nearest `f64` to the rounded value, which then prints as itself.
	match fmt!("{:.14e}", n).parse::<f64>() {
		Ok(r)	=> fmt!("{}", r),
		Err(_)	=> fmt!("{}", n),
	}
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Cell {
	// For a formula cell this is what the last calculation left, and is never recomputed. See the
	// module's own note.
	pub value:	Value,
	pub formula:	Option<String>,	// without its leading `=`
}

impl Cell {

	pub fn empty() -> Self {
		Self::default()
	}

	pub fn text(s: impl Into<String>) -> Self {
		Self { value: Value::Text(s.into()), formula: None }
	}

	pub fn number(n: f64) -> Self {
		Self { value: Value::Number(n), formula: None }
	}

	pub fn bool(b: bool) -> Self {
		Self { value: Value::Bool(b), formula: None }
	}

	/// A cell holding a formula and the value that was last computed for it.
	pub fn formula(text: impl Into<String>, value: Value) -> Self {
		Self { value, formula: Some(text.into()) }
	}

	/// Whether the cell holds nothing at all: no value and no formula.
	pub fn is_empty(&self) -> bool {
		self.value.is_empty() && self.formula.is_none()
	}
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Sheet {
	pub name:	String,	// the name on the tab
	// From row 1, each a run of cells from column A. Short rows are short; a reader fills nothing in,
	// because a sheet of a million empty cells is a sheet nobody can hold.
	pub rows:	Vec<Vec<Cell>>,
}

impl Sheet {

	pub fn new(name: impl Into<String>) -> Self {
		Self { name: name.into(), rows: Vec::new() }
	}

	/// The cell at a reference, or an empty one where the sheet does not reach that far.
	pub fn at(&self, at: &Ref) -> Cell {
		self.rows.get(at.row as usize)
			.and_then(|r| r.get(at.col as usize))
			.cloned()
			.unwrap_or_default()
	}

	/// How many rows the sheet holds, and how many columns its widest row does.
	pub fn size(&self) -> (usize, usize) {
		(self.rows.len(), self.rows.iter().map(|r| r.len()).max().unwrap_or(0))
	}

	pub fn extent(&self) -> Option<Range> {
		let (rows, cols) = self.size();
		if rows == 0 || cols == 0 {
			return None;
		}
		Some(Range {
			from:	Ref { col: 0, row: 0 },
			to:	Ref { col: cols as u32 - 1, row: rows as u32 - 1 },
		})
	}

	/// The cells of a rectangle, row by row.  The rectangle asked for is the
	/// rectangle returned: a position the sheet does not hold comes back as an
	/// empty cell rather than being clipped away, so a caller drawing a grid gets
	/// the shape it asked for.
	pub fn window(&self, range: &Range) -> Vec<Vec<Cell>> {
		let mut out = Vec::new();
		for row in range.from.row..=range.to.row {
			let mut line = Vec::new();
			for col in range.from.col..=range.to.col {
				line.push(self.at(&Ref { col, row }));
			}
			out.push(line);
		}
		out
	}
}

/// A workbook: the sheets it holds, in tab order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Book {
	pub sheets:	Vec<Sheet>,
}

impl Book {

	pub fn new() -> Self {
		Self::default()
	}

	/// The sheet of that name, matched exactly and then without regard to case.
	///
	/// A person typing a sheet name types what is on the tab, and a tab reading `Sales` is asked for
	/// as `sales` about half the time. Exact first, so two sheets differing only in case still each
	/// have a name that reaches them.
	pub fn sheet(&self, name: &str) -> Option<&Sheet> {
		self.sheets.iter().find(|s| s.name == name)
			.or_else(|| self.sheets.iter().find(|s| s.name.eq_ignore_ascii_case(name)))
	}

	pub fn names(&self) -> Vec<&str> {
		self.sheets.iter().map(|s| s.name.as_str()).collect()
	}
}

/// One cell's address: a column and a row, both counted from zero.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct Ref {
	pub col:	u32,	// `A` being zero
	pub row:	u32,	// row 1 being zero
}

impl Ref {

	/// The address as a person writes it: `A1`, `BC42`.
	pub fn name(&self) -> String {
		fmt!("{}{}", col_name(self.col), self.row + 1)
	}

	/// The address a string names.
	///
	/// A `$` is accepted and ignored: `$B$4` is the same cell as `B4`, and refusing the form a person
	/// copied out of a formula bar would be refusing the commonest way of typing one.
	pub fn parse(s: &str) -> Outcome<Self> {
		let s = s.trim();
		let mut col: u32 = 0;
		let mut seen = false;
		let mut rest = s;
		while let Some(c) = rest.chars().next() {
			match c {
				'$'				=> {},
				c if c.is_ascii_alphabetic()	=> {
					col = col * 26 + (c.to_ascii_uppercase() as u32 - 'A' as u32 + 1);
					seen = true;
					if col > MAX_COL {
						return Err(err!(
							"'{}' names a column past {}, the last a sheet has.", s, col_name(MAX_COL - 1);
							Invalid, Input, Range));
					}
				}
				_				=> break,
			}
			rest = &rest[c.len_utf8()..];
		}
		let digits: String = rest.chars().filter(|c| *c != '$').collect();
		if !seen || digits.is_empty() {
			return Err(err!(
				"'{}' is not a cell reference. One looks like `A1` or `BC42`.", s; Invalid, Input));
		}
		let row: u32 = res!(digits.parse().map_err(|_| err!(
			"'{}' is not a cell reference: '{}' is not a row number.", s, digits; Invalid, Input)));
		if row == 0 || row > MAX_ROW {
			return Err(err!(
				"'{}' names row {}, and a sheet has rows 1 to {}.", s, row, MAX_ROW;
				Invalid, Input, Range));
		}
		Ok(Self { col: col - 1, row: row - 1 })
	}
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Range {
	pub from:	Ref,	// the top left
	pub to:	Ref,	// the bottom right, inclusive
}

impl Range {

	/// The range as a person writes it: `A1:D20`.
	pub fn name(&self) -> String {
		fmt!("{}:{}", self.from.name(), self.to.name())
	}

	pub fn cells(&self) -> u64 {
		let w = (self.to.col as u64 + 1).saturating_sub(self.from.col as u64);
		let h = (self.to.row as u64 + 1).saturating_sub(self.from.row as u64);
		w * h
	}

	/// The range a string names: `A1:D20`, or `A1` for a single cell.
	///
	/// Corners given the wrong way round are put right rather than refused. `D20:A1` is the rectangle
	/// somebody dragged from the bottom right, and it is not an error anywhere a person would type it.
	pub fn parse(s: &str) -> Outcome<Self> {
		let s = s.trim();
		let (a, b) = match s.split_once(':') {
			Some((a, b))	=> (a, b),
			None		=> (s, s),
		};
		let a = res!(Ref::parse(a));
		let b = res!(Ref::parse(b));
		Ok(Self {
			from:	Ref { col: a.col.min(b.col), row: a.row.min(b.row) },
			to:	Ref { col: a.col.max(b.col), row: a.row.max(b.row) },
		})
	}
}

/// The letters a column index wears: 0 is `A`, 26 is `AA`.
pub fn col_name(col: u32) -> String {
	let mut out = Vec::new();
	let mut n = col as i64;
	loop {
		out.push(b'A' + (n % 26) as u8);
		n = n / 26 - 1;
		if n < 0 {
			break;
		}
	}
	out.reverse();
	String::from_utf8_lossy(&out).into_owned()
}

/// The value a typed string stands for, the way a spreadsheet decides it.
///
/// This is the rule a person meets when they type into a cell, and it has one subtlety worth stating:
/// a string is a number only where it is EXACTLY how that number prints. `007` and `1,000` and `+3`
/// and ` 4 ` therefore stay text, which is what a person typing a part number or an account code
/// wants, and `3.5` becomes 3.5, which is what a person typing a price wants. A rule that parsed
/// anything parseable would silently turn `007` into `7`.
pub fn typed(s: &str) -> Value {
	if s.is_empty() {
		return Value::Empty;
	}
	if s.eq_ignore_ascii_case("true") {
		return Value::Bool(true);
	}
	if s.eq_ignore_ascii_case("false") {
		return Value::Bool(false);
	}
	match s.parse::<f64>() {
		Ok(n) if n.is_finite() && stored(n) == s	=> Value::Number(n),
		_					=> Value::Text(s.to_string()),
	}
}

/// A number as a file stores it, which is not how a person reads it.
///
/// The shortest text that reads back as the same `f64`, so nothing the caller handed over is lost.
/// [`Value::show`] is what a person sees, and it rounds; this does not.
pub fn stored(n: f64) -> String {
	if !n.is_finite() {
		return "0".to_string();
	}
	if n == n.trunc() && n.abs() < 1e15 {
		return fmt!("{}", n as i64);
	}
	fmt!("{}", n)
}

impl Book {

	/// The workbook a document's tables make: one sheet per table, named by the heading above it.
	///
	/// The counterpart of [`crate::office::deck::Deck::from_doc`] and the same bargain. A model writes
	/// Markdown well and a file format badly, so the way to have it produce a spreadsheet is to have it
	/// produce a table; the conversion from there is code that cannot get the format wrong.
	///
	/// A cell's text becomes a number where it is exactly how that number prints -- see [`typed`], which
	/// is what stops a column of part numbers being renumbered. A cell beginning `=` becomes a formula
	/// with no value beside it, because nothing here calculates and the reader will.
	///
	/// A document with no table in it gives a workbook with one empty sheet, not an error: an empty
	/// spreadsheet is a spreadsheet.
	pub fn from_doc(doc: &Doc) -> Self {
		let mut out = Self::new();
		let mut title: Option<String> = None;
		for block in &doc.blocks {
			match block {
				Block::Heading { content, .. }	=> title = Some(oxedyne_fe2o3_text::doc::text_of(content)),
				Block::Table { head, rows, .. }	=> {
					let name = match title.take() {
						Some(t) if !t.trim().is_empty()	=> tab_name(&t),
						_				=> fmt!("Sheet{}", out.sheets.len() + 1),
					};
					let mut sheet = Sheet::new(name);
					// The header row is a row of the sheet and not a property of it. A spreadsheet has
					// no header row; it has a first row that people read as one.
					for row in head.iter().chain(rows.iter()) {
						sheet.rows.push(row.0.iter().map(|c| from_text(&c.text_of())).collect());
					}
					out.sheets.push(sheet);
				}
				_	=> {}
			}
		}
		if out.sheets.is_empty() {
			out.sheets.push(Sheet::new("Sheet1"));
		}
		out
	}
}

/// The cell a run of text stands for, a leading `=` making it a formula.
fn from_text(s: &str) -> Cell {
	let s = s.trim();
	match s.strip_prefix('=') {
		Some(f) if !f.is_empty()	=> Cell { value: Value::Empty, formula: Some(f.to_string()) },
		_			=> Cell { value: typed(s), formula: None },
	}
}

/// A heading as a name a tab can wear.
///
/// Excel refuses `: \ / ? * [ ]` in a sheet name and refuses one over 31 characters, and it refuses the
/// whole FILE rather than the name -- so a workbook built from a document whose heading held a colon
/// would not open at all.
fn tab_name(s: &str) -> String {
	let cleaned: String = s.trim()
		.chars()
		.map(|c| match c {
			':' | '\\' | '/' | '?' | '*' | '[' | ']'	=> ' ',
			c						=> c,
		})
		.collect();
	let cleaned = cleaned.trim();
	match cleaned.chars().count() > 31 {
		false	=> cleaned.to_string(),
		true	=> cleaned.chars().take(31).collect(),
	}
}
