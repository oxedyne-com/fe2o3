//! `.ods`: an OpenDocument spreadsheet, written from and read back into the neutral spreadsheet.
//!
//! # Everything a cell is, is on the cell
//!
//! `<table:table-cell office:value-type="float" office:value="3.4" table:formula="of:=B2*C2">` says
//! its type, its value and its formula in three attributes. A `.xlsx` needs a shared string table for
//! the first, a style table and a number format for a date, and two hops through `numbering.xml` for
//! a list -- none of which exists here. The three traps that make SpreadsheetML hard are all absent.
//!
//! What is present instead is repetition: a run of identical cells is written once with
//! `table:number-columns-repeated`, and a reader that ignored it puts every later value in the wrong
//! column. That is this format's version of the same mistake.
//!
//! # The stored value is still the value
//!
//! `office:value` is what the last calculation left, and nothing here recalculates. See
//! [`crate::office::sheet`] for why that is the correct answer rather than a shortcut.

use crate::office::odf::{
	NS_FO,
	NS_NUMBER,
	NS_OF,
	NS_OFFICE,
	NS_STYLE,
	NS_TABLE,
	NS_TEXT,
	pkg,
};
use crate::office::sheet::{
	Book,
	Cell,
	MAX_COL,
	Sheet,
	Value,
};
use crate::zip::{
	Method,
	Zip,
};

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_text::xml::{
	Elem,
	Xml,
};
use oxedyne_fe2o3_text::xml::write::Out;

/// The media type an `.ods` declares in its first member.
pub const MEDIA: &str = "application/vnd.oasis.opendocument.spreadsheet";

/// The most a single part is inflated to. An `.ods` is one `content.xml` holding every sheet, so this
/// is the whole workbook rather than a piece of it -- which is the trade OpenDocument makes for
/// having no relationship parts.
pub const MAX_PART: u64 = 96 * 1024 * 1024;

/// Writes a workbook as the bytes of an `.ods`.
pub fn write(book: &Book) -> Outcome<Vec<u8>> {
	let mut owned;
	let book = match book.sheets.is_empty() {
		false	=> book,
		true	=> {
			owned = Book::new();
			owned.sheets.push(Sheet::new("Sheet1"));
			&owned
		}
	};
	let mut out = Out::declared();
	out.open("office:document-content", &[
		("xmlns:office", NS_OFFICE),
		("xmlns:table", NS_TABLE),
		("xmlns:text", NS_TEXT),
		("xmlns:style", NS_STYLE),
		("xmlns:number", NS_NUMBER),
		("xmlns:fo", NS_FO),
		// Without this, every formula in the file fails to parse and its stored value is replaced
		// by an error. See `NS_OF`.
		("xmlns:of", NS_OF),
		("office:version", pkg::VERSION),
	]);
	// A date is a date because its cell says so, and the DATA STYLE is what makes a reader show it
	// as one rather than as a serial. One style serves every date in the book.
	out.open("office:automatic-styles", &[]);
	out.open("number:date-style", &[("style:name", "ND1")]);
	out.empty("number:year", &[("number:style", "long")]);
	out.leaf("number:text", &[], "-");
	out.empty("number:month", &[("number:style", "long")]);
	out.leaf("number:text", &[], "-");
	out.empty("number:day", &[("number:style", "long")]);
	res!(out.close("number:date-style"));
	out.empty("style:style", &[
		("style:name", "CD1"),
		("style:family", "table-cell"),
		("style:data-style-name", "ND1"),
	]);
	res!(out.close("office:automatic-styles"));
	out.open("office:body", &[]);
	out.open("office:spreadsheet", &[]);
	for s in &book.sheets {
		out.open("table:table", &[("table:name", &s.name)]);
		let (_, cols) = s.size();
		out.empty("table:table-column", &[
			("table:number-columns-repeated", &fmt!("{}", cols.max(1))),
		]);
		for row in &s.rows {
			out.open("table:table-row", &[]);
			for cell in row {
				res!(cell_part(&mut out, cell));
			}
			res!(out.close("table:table-row"));
		}
		res!(out.close("table:table"));
	}
	res!(out.close("office:spreadsheet"));
	res!(out.close("office:body"));
	res!(out.close("office:document-content"));

	let mut zip = pkg::start(MEDIA);
	zip.set("content.xml", res!(out.finish()).into_bytes(), Method::Deflate);
	zip.set("styles.xml", res!(pkg::styles_for(MEDIA)).into_bytes(), Method::Deflate);
	zip.set("meta.xml", res!(pkg::meta(MEDIA)).into_bytes(), Method::Deflate);
	res!(pkg::finish(&mut zip, MEDIA));
	zip.write()
}

/// One cell: its type, its value and its formula, all on the element.
fn cell_part(out: &mut Out, cell: &Cell) -> Outcome<()> {
	let shown = cell.value.show();
	// A formula is written in OpenFormula, which is NOT the A1 syntax a `.xlsx` uses: a reference
	// has to be bracketed, so `B2*C2` becomes `[.B2]*[.C2]`. Written the other way LibreOffice
	// cannot parse it, RECALCULATES, and replaces the cached value with `Err:510` -- so a wrong
	// formula does not merely fail to work, it destroys the number that was there.
	let formula = cell.formula.as_ref().map(|f| fmt!("of:={}", openformula(f)));
	let mut attrs: Vec<(&str, &str)> = Vec::new();
	if let Some(f) = &formula {
		attrs.push(("table:formula", f));
	}
	let num;
	match &cell.value {
		Value::Empty	=> {}
		Value::Text(_)	=> attrs.push(("office:value-type", "string")),
		Value::Number(n)	=> {
			num = repr(*n);
			attrs.push(("office:value-type", "float"));
			attrs.push(("office:value", &num));
		}
		Value::Bool(b)	=> {
			attrs.push(("office:value-type", "boolean"));
			attrs.push(("office:boolean-value", match b {
				true	=> "true",
				false	=> "false",
			}));
		}
		Value::Date(d)	=> {
			attrs.push(("table:style-name", "CD1"));
			attrs.push(("office:value-type", "date"));
			attrs.push(("office:date-value", d));
		}
		// An error has no value type of its own here; it is a string cell holding what the last
		// calculation produced, which is what a reader shows.
		Value::Error(_)	=> attrs.push(("office:value-type", "string")),
	}
	if cell.is_empty() {
		out.empty("table:table-cell", &[]);
		return Ok(());
	}
	out.open("table:table-cell", &attrs);
	if !shown.is_empty() {
		out.leaf("text:p", &[], &shown);
	}
	res!(out.close("table:table-cell"));
	Ok(())
}

/// A formula's references, bracketed as OpenFormula requires.
///
/// `B2*C2` becomes `[.B2]*[.C2]` and `SUM(D2:D3)` becomes `SUM([.D2:.D3])`. A leading `.` means "this
/// sheet", which is what every reference written without one means.
///
/// What is NOT a reference is left alone: a function name is followed by `(`, and anything inside
/// quotation marks is text. Those two rules are the whole of it, and they are the two that would
/// otherwise turn `SUM` into a cell and a quoted `A1` into a reference.
pub fn openformula(f: &str) -> String {
	let b = f.as_bytes();
	let mut out = String::with_capacity(f.len() + 8);
	let mut i = 0;
	while i < b.len() {
		let c = b[i] as char;
		if c == '"' {
			// A quoted run is text and passes through whole, quotes included.
			out.push(c);
			i += 1;
			while i < b.len() {
				out.push(b[i] as char);
				i += 1;
				if b[i - 1] == b'"' {
					break;
				}
			}
			continue;
		}
		match reference(b, i) {
			None		=> {
				out.push(c);
				i += 1;
			}
			Some(end)	=> {
				// A range is two references with a colon between them, and it is bracketed ONCE
				// with both sides dotted -- `[.D2:.D3]`, not `[.D2]:[.D3]`.
				let first = &f[i..end];
				let mut at = end;
				if b.get(at) == Some(&b':') {
					if let Some(second) = reference(b, at + 1) {
						out.push_str(&fmt!("[.{}:.{}]", first, &f[at + 1..second]));
						i = second;
						continue;
					}
					at = end;
				}
				let _ = at;
				out.push_str(&fmt!("[.{}]", first));
				i = end;
			}
		}
	}
	out
}

/// Where a cell reference starting at an offset ends, if one starts there.
///
/// A reference is letters then digits, not preceded by a letter, a digit or a `$` -- so the `A1` in
/// `BA12` is not one -- and not followed by `(`, which is what makes `LOG10(x)` a function rather
/// than a cell in column LOG.
fn reference(b: &[u8], at: usize) -> Option<usize> {
	if at > 0 {
		let prev = b[at - 1];
		if prev.is_ascii_alphanumeric() || prev == b'$' || prev == b'.' || prev == b'_' {
			return None;
		}
	}
	let mut i = at;
	let mut letters = 0;
	while i < b.len() && b[i].is_ascii_alphabetic() && letters < 3 {
		i += 1;
		letters += 1;
	}
	if letters == 0 {
		return None;
	}
	let digits_at = i;
	while i < b.len() && b[i].is_ascii_digit() {
		i += 1;
	}
	if i == digits_at {
		return None;
	}
	// A name followed by a bracket is a function, however much it looks like a cell.
	if b.get(i) == Some(&b'(') {
		return None;
	}
	// And a trailing letter means it was never a reference: `A1B` is a name.
	if b.get(i).map(|c| c.is_ascii_alphanumeric()).unwrap_or(false) {
		return None;
	}
	Some(i)
}

/// A number as the file stores it, which is not how a person reads it.
fn repr(n: f64) -> String {
	if !n.is_finite() {
		return "0".to_string();
	}
	if n == n.trunc() && n.abs() < 1e15 {
		return fmt!("{}", n as i64);
	}
	fmt!("{}", n)
}

/// A workbook read for reading.
#[derive(Clone, Debug, Default)]
pub struct Reading {
	/// The sheets and their cells.
	pub book:	Book,
	/// Whether the file carries a macro project. Said, never run.
	pub macros:	bool,
	/// How many cells carry a formula.
	pub formulas:	usize,
}

/// Reads an `.ods` into the workbook it holds.
pub fn read(bytes: &[u8]) -> Outcome<Reading> {
	let zip = res!(Zip::read(bytes.to_vec()));
	let mut out = Reading::default();
	out.macros = zip.names().iter().any(|n| n.starts_with("Basic/"));
	let src = res!(String::from_utf8(res!(zip.content_capped("content.xml", MAX_PART))),
		Decode, String);
	let xml = res!(Xml::parse(&src));
	let body = res!(res!(xml.root()).find(&["office:body", "office:spreadsheet"])
		.ok_or_else(|| err!(
			"This package has no <office:spreadsheet>, so it is not a spreadsheet.";
			Invalid, Input, Missing)));
	for t in body.children("table:table") {
		let mut sheet = Sheet::new(t.attr("table:name").unwrap_or("Sheet"));
		for tr in t.children("table:table-row") {
			// A run of identical rows is written once and repeated. Ignoring the count collapses
			// them, which moves every row after the run.
			let n = tr.attr("table:number-rows-repeated")
				.and_then(|v| v.parse::<usize>().ok())
				.unwrap_or(1);
			let line = row_of(&xml, tr, &mut out.formulas);
			// An INTERIOR run of empty rows has to be expanded, or everything after the gap moves up:
			// collapsing it to one put the total row two rows early. A run at the END is padding --
			// a sheet says "nothing until row 1048576" that way -- and the trailing trim below is
			// what deals with that, so the expansion only needs a bound rather than a special case.
			let n = n.min(4096);
			for _ in 0..n {
				sheet.rows.push(line.clone());
			}
		}
		// Trailing empty rows are the sheet's padding, not its content.
		while sheet.rows.last().map(|r| r.iter().all(|c| c.is_empty())).unwrap_or(false) {
			sheet.rows.pop();
		}
		out.book.sheets.push(sheet);
	}
	Ok(out)
}

/// One row of cells, expanding the repeats.
fn row_of(xml: &Xml, tr: &Elem, formulas: &mut usize) -> Vec<Cell> {
	let mut out: Vec<Cell> = Vec::new();
	for tc in tr.elems() {
		let covered = match tc.name.qname.as_str() {
			"table:table-cell"		=> false,
			"table:covered-table-cell"	=> true,
			_				=> continue,
		};
		let n = tc.attr("table:number-columns-repeated")
			.and_then(|v| v.parse::<usize>().ok())
			.unwrap_or(1);
		let cell = match covered {
			true	=> Cell::empty(),
			false	=> cell_of(xml, tc),
		};
		if cell.formula.is_some() {
			*formulas += 1;
		}
		// The same as the rows, and for the same reason: an interior run of empty cells is a GAP and
		// moves everything after it, while a run at the end is padding and is trimmed below.
		let n = n.min(MAX_COL as usize);
		for _ in 0..n {
			out.push(cell.clone());
		}
	}
	while out.last().map(|c| c.is_empty()).unwrap_or(false) {
		out.pop();
	}
	out
}

/// One cell.
fn cell_of(xml: &Xml, tc: &Elem) -> Cell {
	// The formula is read and never evaluated. The `of:=` prefix is the format's own namespace
	// marker, not part of the expression, so it comes off.
	let formula = tc.attr("table:formula").map(|f| {
		f.strip_prefix("of:=").or_else(|| f.strip_prefix('=')).unwrap_or(f).to_string()
	});
	let shown = || -> String {
		tc.children("text:p").iter().map(|p| xml.text_of(p)).collect::<Vec<_>>().join("\n")
	};
	let value = match tc.attr("office:value-type") {
		Some("float") | Some("percentage") | Some("currency")	=> {
			match tc.attr("office:value").and_then(|v| v.trim().parse::<f64>().ok()) {
				Some(n)	=> Value::Number(n),
				None		=> Value::Empty,
			}
		}
		Some("boolean")	=> Value::Bool(tc.attr("office:boolean-value") == Some("true")),
		Some("date")	=> match tc.attr("office:date-value") {
			Some(d)	=> Value::Date(d.to_string()),
			None		=> Value::Empty,
		},
		Some("time")	=> match tc.attr("office:time-value") {
			Some(d)	=> Value::Date(d.to_string()),
			None		=> Value::Empty,
		},
		Some("string")	=> {
			// The displayed text is the value, unless the cell carries one explicitly.
			let t = tc.attr("office:string-value").map(|s| s.to_string()).unwrap_or_else(shown);
			match t.is_empty() {
				true	=> Value::Empty,
				false	=> Value::Text(t),
			}
		}
		_	=> {
			let t = shown();
			match t.is_empty() {
				true	=> Value::Empty,
				false	=> Value::Text(t),
			}
		}
	};
	Cell { value, formula }
}
