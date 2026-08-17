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
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

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
	Ref,
	Sheet,
	Value,
	stored,
	tab_names,
	typed,
};
use crate::zip::{
	Method,
	Zip,
};

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_text::xml::{
	Elem,
	Span,
	Xml,
};
use oxedyne_fe2o3_text::xml::write::{
	Out,
	escape_attr,
};

// Declared in the package's first member, which is what names the file.
pub const MEDIA: &str = "application/vnd.oasis.opendocument.spreadsheet";

// The most a single part is inflated to. An `.ods` is one `content.xml` holding every sheet, so this
// is the whole workbook rather than a piece of it -- which is the trade OpenDocument makes for
// having no relationship parts.
pub const MAX_PART: u64 = 96 * 1024 * 1024;

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
	// The same tab names the `.xlsx` writer settles on, and for the same reason: two tables of one
	// name is a document a reader silently renames. See `crate::office::sheet::tab_names`.
	let tabs = tab_names(book);
	for (i, s) in book.sheets.iter().enumerate() {
		out.open("table:table", &[("table:name", &tabs[i])]);
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
	zip.set("meta.xml", res!(pkg::meta()).into_bytes(), Method::Deflate);
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

/// A formula's references with the OpenFormula bracketing taken off: the inverse of [`openformula`].
///
/// `[.B2]*[.C2]` becomes `B2*C2` and `SUM([.D2:.D3])` becomes `SUM(D2:D3)`.
///
/// **This existed nowhere until a round-trip test asked for it, and its absence was invisible.** The
/// writer's test checked the bytes it produced and the reader's test checked that a formula came back
/// AT ALL, so between two passing tests sat the fact that a formula written by this crate read back as
/// `[.B2]*[.C2]` -- and the same workbook as a `.xlsx` read back as `B2*C2`. A caller comparing the two
/// formats, or handing a formula to a model, met a difference that is nothing to do with the data.
///
/// Anything inside quotation marks is text and passes through untouched, as it does on the way out.
pub fn plain(f: &str) -> String {
	let b = f.as_bytes();
	let mut out = String::with_capacity(f.len());
	let mut i = 0;
	while i < b.len() {
		let c = b[i] as char;
		if c == '"' {
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
		if c != '[' {
			out.push(c);
			i += 1;
			continue;
		}
		match f[i..].find(']') {
			// An unclosed bracket is left exactly as written. Guessing where it ended would rewrite an
			// expression nobody can check.
			None		=> {
				out.push(c);
				i += 1;
			}
			Some(k)	=> {
				let inside = &f[i + 1..i + k];
				// A range is bracketed once with both sides dotted, so each side loses its own dot.
				let parts: Vec<&str> = inside.split(':')
					.map(|p| p.strip_prefix('.').unwrap_or(p))
					.collect();
				out.push_str(&parts.join(":"));
				i += k + 1;
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

#[derive(Clone, Debug, Default)]
pub struct Reading {
	pub book:	Book,
	pub macros:	bool,		// a macro project is present; said, never run
	pub formulas:	usize,		// cells carrying a formula
}

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

fn cell_of(xml: &Xml, tc: &Elem) -> Cell {
	// The formula is read and never evaluated. The `of:=` prefix is the format's own namespace
	// marker, not part of the expression, so it comes off.
	let formula = tc.attr("table:formula").map(|f| {
		plain(f.strip_prefix("of:=").or_else(|| f.strip_prefix('=')).unwrap_or(f))
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

// ---------------------------------------------------------------------------
// Editing an `.ods` in place
// ---------------------------------------------------------------------------

/// One cell to write.
#[derive(Clone, Debug)]
pub struct Set {
	pub sheet:	Option<String>,	// by the name on the tab; None means the first
	pub at:	Ref,
	// The value as a person would type it. `sheet::typed` decides what it is; an empty string
	// empties the cell.
	pub value:	Option<String>,
	pub formula:	Option<String>,	// without its leading `=`, which is stripped if present
}

/// What an edit of an `.ods` produced.
#[derive(Clone, Debug, Default)]
pub struct Edited {
	pub bytes:	Vec<u8>,
	pub cells:	usize,
	pub sheets:	Vec<String>,	// the tabs that were touched
}

/// Writes cells into an `.ods`, leaving every other byte of the package as it arrived.
///
/// # Repetition is the whole of the difficulty
///
/// `.ods` addresses nothing. A run of identical cells is written once as
/// `<table:table-cell table:number-columns-repeated="8"/>`, and a run of identical rows the same way,
/// so writing `C4` means finding which run covers it and SPLITTING that run into the part before, the
/// cell itself, and the part after. Written any other way -- appended, or with the count left alone --
/// every value to the right of the edit moves one column, which is a corruption that looks like a
/// spreadsheet.
///
/// The rest of `content.xml` is copied byte for byte, and `styles.xml`, `meta.xml`, the manifest and
/// anything else in the package are never opened.
pub fn edit(bytes: &[u8], sets: &[Set]) -> Outcome<Edited> {
	if sets.is_empty() {
		return Err(err!("A write to a spreadsheet was asked for with no cells in it."; Invalid, Input));
	}
	let mut zip = res!(Zip::read(bytes.to_vec()));
	let src = res!(String::from_utf8(res!(zip.content_capped("content.xml", MAX_PART))),
		Decode, String);
	let mut xml = res!(Xml::parse(&src));
	let body = res!(res!(xml.root()).find(&["office:body", "office:spreadsheet"])
		.ok_or_else(|| err!(
			"This package has no <office:spreadsheet>, so it is not a spreadsheet.";
			Invalid, Input, Missing)))
		.clone();
	let tables: Vec<Elem> = body.children("table:table").into_iter().cloned().collect();
	if tables.is_empty() {
		return Err(err!("This spreadsheet holds no sheets, so there is nowhere to write.";
			Invalid, Input, Missing));
	}
	let names: Vec<String> = tables.iter()
		.map(|t| t.attr("table:name").unwrap_or("Sheet").to_string())
		.collect();

	let mut jobs: Vec<(usize, Vec<&Set>)> = Vec::new();
	for set in sets {
		let i = match &set.sheet {
			None		=> 0,
			Some(want)	=> {
				let found = names.iter().position(|n| n == want)
					.or_else(|| names.iter().position(|n| n.eq_ignore_ascii_case(want)));
				res!(found.ok_or_else(|| err!(
					"This workbook has no sheet named '{}'. It has: {}. Nothing has been written.",
					want, names.join(", "); Invalid, Input, Missing)))
			}
		};
		match jobs.iter_mut().find(|(k, _)| *k == i) {
			Some(j)	=> j.1.push(set),
			None		=> jobs.push((i, vec![set])),
		}
	}
	// Refused rather than resolved, for the reason `xlsx::edit` gives: there is no order for two
	// writes to one cell to be applied in, so picking one would be a rule about how a caller happened
	// to build its list. The two formats answer this the same way, which is the point.
	for (i, sets) in &jobs {
		for (k, one) in sets.iter().enumerate() {
			if sets[..k].iter().any(|s| s.at == one.at) {
				return Err(err!(
					"{} of sheet '{}' is written twice in one call, and there is no order in which \
					to apply the two. Nothing has been written.", one.at.name(), names[*i];
					Invalid, Input, Conflict));
			}
		}
	}

	let mut splices: Vec<(Span, String)> = Vec::new();
	let mut touched = Vec::new();
	for (i, sets) in &jobs {
		res!(table_splices(&xml, &tables[*i], sets, &mut splices));
		touched.push(names[*i].clone());
	}
	splices.sort_by_key(|(s, _)| (s.start, s.end));
	for (span, text) in splices {
		res!(xml.splice(span, text));
	}
	zip.set("content.xml", xml.render().into_bytes(), Method::Deflate);
	Ok(Edited { bytes: res!(zip.write()), cells: sets.len(), sheets: touched })
}

/// One run of identical rows or cells: the element that carries it, and the span of the grid it covers.
struct Run<'a> {
	elem:	&'a Elem,
	from:	u32,
	n:	u32,
}

/// The runs a table's rows make, and how many rows they cover between them.
fn runs<'a>(at: &'a Elem, kinds: &[&str], attr: &str) -> (Vec<Run<'a>>, u32) {
	let mut out = Vec::new();
	let mut from = 0u32;
	for kid in at.elems() {
		if !kinds.iter().any(|k| kid.name.qname == *k) {
			continue;
		}
		let n = kid.attr(attr)
			.and_then(|v| v.parse::<u32>().ok())
			.unwrap_or(1)
			.max(1);
		out.push(Run { elem: kid, from, n });
		from = from.saturating_add(n);
	}
	(out, from)
}

/// The splices one table needs, added to the list the whole document's edit will make.
fn table_splices(
	xml:	&Xml,
	table:	&Elem,
	sets:	&[&Set],
	into:	&mut Vec<(Span, String)>,
)
	-> Outcome<()>
{
	let (rows, total) = runs(table, &["table:table-row"], "table:number-rows-repeated");

	// By the run that covers them, because splitting a run is one replacement of one element however
	// many of its repeats are being written into.
	let mut by_run: Vec<(usize, Vec<&Set>)> = Vec::new();
	let mut appended: Vec<&Set> = Vec::new();
	for set in sets {
		match rows.iter().position(|r| set.at.row >= r.from && set.at.row < r.from + r.n) {
			Some(k)	=> match by_run.iter_mut().find(|(j, _)| *j == k) {
				Some(g)	=> g.1.push(set),
				None		=> by_run.push((k, vec![set])),
			},
			None		=> appended.push(set),
		}
	}

	for (k, group) in &by_run {
		let run = &rows[*k];
		let mut text = String::new();
		let mut at = 0u32;
		while at < run.n {
			let here = run.from + at;
			let mine: Vec<&Set> = group.iter().filter(|s| s.at.row == here).copied().collect();
			if mine.is_empty() {
				// The untouched repeats either side of an edit keep the run they were in, with the
				// count reduced to what is left of it.
				let next = group.iter()
					.filter_map(|s| s.at.row.checked_sub(run.from))
					.filter(|o| *o > at)
					.min()
					.unwrap_or(run.n);
				text.push_str(&repeated(xml, run.elem, "table:number-rows-repeated", next - at));
				at = next;
				continue;
			}
			text.push_str(&res!(row_markup(xml, run.elem, &mine)));
			at += 1;
		}
		into.push((run.elem.span.clone(), text));
	}

	if appended.is_empty() {
		return Ok(());
	}
	// Rows past the end of the sheet: the gap, then a row for each that is being written.
	let mut by_row: Vec<(u32, Vec<&Set>)> = Vec::new();
	for set in &appended {
		match by_row.iter_mut().find(|(r, _)| *r == set.at.row) {
			Some(g)	=> g.1.push(set),
			None		=> by_row.push((set.at.row, vec![set])),
		}
	}
	by_row.sort_by_key(|(r, _)| *r);
	let mut text = String::new();
	let mut at = total;
	for (row, group) in &by_row {
		if *row > at {
			text.push_str(&fmt!(
				"<table:table-row table:number-rows-repeated=\"{}\"><table:table-cell/>\
				</table:table-row>", row - at));
		}
		let mut cells = String::new();
		let mut col = 0u32;
		let mut group = group.clone();
		group.sort_by_key(|s| s.at.col);
		for set in &group {
			if set.at.col > col {
				cells.push_str(&fmt!(
					"<table:table-cell table:number-columns-repeated=\"{}\"/>", set.at.col - col));
			}
			cells.push_str(&cell_markup(set, None));
			col = set.at.col + 1;
		}
		text.push_str(&fmt!("<table:table-row>{}</table:table-row>", cells));
		at = row + 1;
	}
	let at = res!(table.inner.clone().ok_or_else(|| err!(
		"This sheet is written <table:table/>, with nothing inside it at all."; Invalid, Input)));
	into.push((at.end..at.end, text));
	Ok(())
}

/// One row of the source with a run count put on it, for the repeats an edit did not touch.
fn repeated(xml: &Xml, elem: &Elem, attr: &str, n: u32) -> String {
	let base = xml.raw(&elem.span);
	let held = elem.attrs.iter().find(|a| a.name.qname == attr);
	match held {
		// The attribute is in the open tag, so its span is inside the element's and the offsets are
		// the element's own.
		Some(a)	=> {
			let from = a.val_span.start - elem.span.start;
			let to = a.val_span.end - elem.span.start;
			fmt!("{}{}{}", &base[..from], n, &base[to..])
		}
		None if n == 1	=> base.to_string(),
		None		=> {
			// A run of one that has to become a run of many needs the attribute adding, which goes
			// straight after the element's name.
			let head = elem.name.span.end - elem.span.start;
			fmt!("{} {}=\"{}\"{}", &base[..head], attr, n, &base[head..])
		}
	}
}

/// One row of the source, with the cells an edit named replaced and the run count taken off it.
fn row_markup(xml: &Xml, tr: &Elem, sets: &[&Set]) -> Outcome<String> {
	let base = xml.raw(&tr.span).to_string();
	let start = tr.span.start;
	let mut edits: Vec<(usize, usize, String)> = Vec::new();

	// The row is now one row, so whatever count it carried goes.
	if let Some(a) = tr.attrs.iter().find(|a| a.name.qname == "table:number-rows-repeated") {
		edits.push((a.span.start - start, a.span.end - start, String::new()));
	}

	let kinds = ["table:table-cell", "table:covered-table-cell"];
	let (cells, total) = runs(tr, &kinds, "table:number-columns-repeated");
	let mut by_run: Vec<(usize, Vec<&Set>)> = Vec::new();
	let mut appended: Vec<&Set> = Vec::new();
	for set in sets {
		match cells.iter().position(|c| set.at.col >= c.from && set.at.col < c.from + c.n) {
			Some(k)	=> match by_run.iter_mut().find(|(j, _)| *j == k) {
				Some(g)	=> g.1.push(set),
				None		=> by_run.push((k, vec![*set])),
			},
			None		=> appended.push(set),
		}
	}

	for (k, group) in &by_run {
		let run = &cells[*k];
		if run.elem.name.qname == "table:covered-table-cell" {
			let names: Vec<String> = group.iter().map(|s| s.at.name()).collect();
			return Err(err!(
				"{} is covered by a merged cell, so writing to it would put a value where nothing is \
				drawn. Write to the top left cell of the merge instead. Nothing has been written.",
				names.join(", "); Invalid, Input));
		}
		let style = run.elem.attr("table:style-name").map(|s| s.to_string());
		let mut text = String::new();
		let mut at = 0u32;
		while at < run.n {
			let here = run.from + at;
			let mine = group.iter().find(|s| s.at.col == here);
			match mine {
				None	=> {
					let next = group.iter()
						.filter_map(|s| s.at.col.checked_sub(run.from))
						.filter(|o| *o > at)
						.min()
						.unwrap_or(run.n);
					text.push_str(&repeated(
						xml, run.elem, "table:number-columns-repeated", next - at));
					at = next;
				}
				Some(set)	=> {
					text.push_str(&cell_markup(set, style.as_deref()));
					at += 1;
				}
			}
		}
		edits.push((run.elem.span.start - start, run.elem.span.end - start, text));
	}

	if !appended.is_empty() {
		let mut appended = appended;
		appended.sort_by_key(|s| s.at.col);
		let mut text = String::new();
		let mut col = total;
		for set in &appended {
			if set.at.col > col {
				text.push_str(&fmt!(
					"<table:table-cell table:number-columns-repeated=\"{}\"/>", set.at.col - col));
			}
			text.push_str(&cell_markup(set, None));
			col = set.at.col + 1;
		}
		match &tr.inner {
			Some(inner)	=> {
				let at = inner.end - start;
				edits.push((at, at, text));
			}
			// A `<table:table-row/>` has no inside to append to, so the whole element is rebuilt.
			None		=> {
				let head = res!(base.strip_suffix("/>").ok_or_else(|| err!(
					"A row with no content did not end '/>': {}", base; Bug)));
				return Ok(fmt!("{}>{}</table:table-row>", head, text));
			}
		}
	}

	edits.sort_by_key(|(from, to, _)| (*from, *to));
	let mut out = String::with_capacity(base.len() + 64);
	let mut at = 0usize;
	for (from, to, text) in &edits {
		out.push_str(&base[at..*from]);
		out.push_str(text);
		at = *to;
	}
	out.push_str(&base[at..]);
	Ok(out)
}

/// One `<table:table-cell>` holding what the caller asked for.
///
/// Everything is on the element -- the type, the value and the formula -- which is what makes this
/// format the easier of the two to write into. The displayed `<text:p>` goes in as well, because a
/// reader that does not recalculate shows that and not `office:value`.
fn cell_markup(set: &Set, style: Option<&str>) -> String {
	let value = set.value.as_deref().map(typed).unwrap_or(Value::Empty);
	let mut out = String::from("<table:table-cell");
	if let Some(s) = style {
		out.push_str(&fmt!(" table:style-name=\"{}\"", escape_attr(s)));
	}
	if let Some(f) = &set.formula {
		let f = f.trim_start_matches('=');
		if !f.is_empty() {
			// Bracketed, because OpenFormula requires it: written as `of:=B2*C2` LibreOffice fails to
			// parse the formula, RECALCULATES the cell, and writes `Err:510` over the stored value.
			out.push_str(&fmt!(" table:formula=\"{}\"", escape_attr(&fmt!("of:={}", openformula(f)))));
		}
	}
	match &value {
		Value::Empty	=> {}
		Value::Text(_) | Value::Error(_)	=> out.push_str(" office:value-type=\"string\""),
		Value::Number(n)	=> out.push_str(&fmt!(
			" office:value-type=\"float\" office:value=\"{}\"", stored(*n))),
		Value::Bool(b)	=> out.push_str(&fmt!(
			" office:value-type=\"boolean\" office:boolean-value=\"{}\"", b)),
		Value::Date(d)	=> out.push_str(&fmt!(
			" office:value-type=\"date\" office:date-value=\"{}\"", escape_attr(d))),
	}
	let shown = value.show();
	if shown.is_empty() {
		out.push_str("/>");
		return out;
	}
	// THE DISPLAYED TEXT GOES IN A `<text:p>` AND NOT AS BARE CHARACTER DATA. Written bare, a numeric
	// cell still shows -- it has `office:value` -- and a STRING cell shows nothing at all, because a
	// string cell's value IS its paragraph. So the mistake loses exactly the cells it is hardest to
	// notice losing, and LibreOffice is what found it.
	let p = crate::office::odf::text::content_markup(&shown);
	out.push_str(">");
	out.push_str("<text:p>");
	out.push_str(&p);
	out.push_str("</text:p>");
	out.push_str("</table:table-cell>");
	out
}
