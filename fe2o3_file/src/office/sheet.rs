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

use oxedyne_fe2o3_core::prelude::*;

/// The largest column a sheet may have: `XFD`, which is Excel's own limit of 16,384.
pub const MAX_COL: u32 = 16_384;

/// The largest row a sheet may have, which is Excel's own limit.
pub const MAX_ROW: u32 = 1_048_576;

/// What one cell holds.
///
/// A closed set, on purpose. Everything a spreadsheet stores is one of these; what varies between
/// the formats is the spelling.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Value {
	/// Nothing at all. A cell that was never filled in, and a cell somebody emptied.
	#[default]
	Empty,
	/// Text.
	Text(String),
	/// A number.
	Number(f64),
	/// A date or a time, as the text it was rendered to.
	///
	/// Held as text rather than as a number with a format beside it, because the number alone is
	/// meaningless -- `45678` is a date only if something says so -- and the format alone cannot be
	/// applied without a calendar. The conversion happens once, where the format is known.
	Date(String),
	/// A truth value.
	Bool(bool),
	/// What the last calculation left where it could not produce a value: `#DIV/0!`, `#N/A`.
	Error(String),
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

	/// Whether the cell holds nothing.
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

/// One cell: what it holds, and the formula that produced it where there is one.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Cell {
	/// The value the file holds. For a formula cell this is what the last calculation left, and is
	/// never recomputed. See the module's own note.
	pub value:	Value,
	/// The formula, without its leading `=`, where the cell carries one.
	pub formula:	Option<String>,
}

impl Cell {

	/// A cell holding nothing.
	pub fn empty() -> Self {
		Self::default()
	}

	/// A cell holding text.
	pub fn text(s: impl Into<String>) -> Self {
		Self { value: Value::Text(s.into()), formula: None }
	}

	/// A cell holding a number.
	pub fn number(n: f64) -> Self {
		Self { value: Value::Number(n), formula: None }
	}

	/// A cell holding a truth value.
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

/// One sheet of a workbook.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Sheet {
	/// The name on the tab.
	pub name:	String,
	/// The rows, from row 1, each a run of cells from column A. Short rows are short; a reader fills
	/// nothing in, because a sheet of a million empty cells is a sheet nobody can hold.
	pub rows:	Vec<Vec<Cell>>,
}

impl Sheet {

	/// A sheet with a name and nothing in it.
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

	/// The rectangle the sheet actually occupies, or nothing where it is empty.
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

	/// The cells of a rectangle, row by row, clipped to what the sheet holds.
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
	/// The sheets, in the order their tabs appear.
	pub sheets:	Vec<Sheet>,
}

impl Book {

	/// A workbook with nothing in it.
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

	/// The names on the tabs, in order.
	pub fn names(&self) -> Vec<&str> {
		self.sheets.iter().map(|s| s.name.as_str()).collect()
	}
}

/// One cell's address: a column and a row, both counted from zero.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct Ref {
	/// The column, `A` being zero.
	pub col:	u32,
	/// The row, row 1 being zero.
	pub row:	u32,
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

/// A rectangle of cells.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Range {
	/// The top left.
	pub from:	Ref,
	/// The bottom right, inclusive.
	pub to:	Ref,
}

impl Range {

	/// The range as a person writes it: `A1:D20`.
	pub fn name(&self) -> String {
		fmt!("{}:{}", self.from.name(), self.to.name())
	}

	/// How many cells it covers.
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
