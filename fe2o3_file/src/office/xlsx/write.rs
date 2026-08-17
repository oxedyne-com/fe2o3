//! Creating a `.xlsx` from the neutral spreadsheet.
//!
//! Six parts, and every byte of them is written here. A cell's text goes into the shared string table
//! rather than into the cell, because that is the shape every other writer produces and the shape a
//! reader is therefore obliged to handle -- writing the easier `inlineStr` everywhere would leave the
//! reader's shared-string path exercised only by files this project did not write.
//!
//! # Excel is stricter than the schema about `styles.xml`
//!
//! It wants a fill table whose first two entries are `none` and `gray125`, in that order, and it
//! wants at least one font, one border and one `cellStyleXfs` entry, whether or not anything uses
//! them. A file omitting any of them opens with a repair prompt rather than an error, which is the
//! worst kind of failure to debug: the file is "fixed" and the reason is never named. So they are all
//! written.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use crate::office::opc::{
	CT_SHEET,
	CT_SHEET_STYLES,
	CT_STRINGS,
	CT_WORKBOOK,
	NS_R,
	REL_DOC,
	REL_SHEET,
	REL_STRINGS,
	REL_STYLES,
	Rels,
	Types,
};
use crate::office::sheet::{
	Book,
	Cell,
	Ref,
	Value,
};
use crate::office::xlsx::NS_S;
use crate::zip::{
	Method,
	Zip,
};

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_text::xml::write::Out;

use std::collections::BTreeMap;

/// The style index a date cell wears. See [`styles`].
///
/// One fact written in two places -- here, and the order of the entries in `cellXfs` -- so the two
/// must move together. There is no third place.
const STYLE_DATE: &str = "1";

/// A book with no sheets gets one empty sheet, because a workbook with none is a file every reader
/// refuses, and refusing to write one here would be refusing at the wrong end.
pub fn write(book: &Book) -> Outcome<Vec<u8>> {
	let mut owned;
	let book = match book.sheets.is_empty() {
		false	=> book,
		true	=> {
			owned = Book::new();
			owned.sheets.push(crate::office::sheet::Sheet::new("Sheet1"));
			&owned
		}
	};

	// Every distinct string in the book, once, in the order first seen. The order is what the cells
	// refer to by index, so it has to be settled before a single cell is written.
	let mut strings: Vec<&str> = Vec::new();
	let mut seen: BTreeMap<&str, usize> = BTreeMap::new();
	for s in &book.sheets {
		for row in &s.rows {
			for cell in row {
				if let Value::Text(t) = &cell.value {
					if !seen.contains_key(t.as_str()) {
						seen.insert(t.as_str(), strings.len());
						strings.push(t.as_str());
					}
				}
			}
		}
	}

	let mut zip = Zip::new();
	let mut types = Types::new();
	types.over("/xl/workbook.xml", CT_WORKBOOK);
	types.over("/xl/styles.xml", CT_SHEET_STYLES);
	types.over("/xl/sharedStrings.xml", CT_STRINGS);

	let mut root = Rels::new();
	let _ = root.add(REL_DOC, "xl/workbook.xml");

	// The workbook's own relationships, and the ids the workbook part names its sheets by.
	let mut wb_rels = Rels::new();
	let mut ids = Vec::with_capacity(book.sheets.len());
	for i in 0..book.sheets.len() {
		ids.push(wb_rels.add(REL_SHEET, &fmt!("worksheets/sheet{}.xml", i + 1)));
		types.over(&fmt!("/xl/worksheets/sheet{}.xml", i + 1), CT_SHEET);
	}
	let _ = wb_rels.add(REL_STYLES, "styles.xml");
	let _ = wb_rels.add(REL_STRINGS, "sharedStrings.xml");

	let mut wb = Out::declared();
	wb.open("workbook", &[("xmlns", NS_S), ("xmlns:r", NS_R)]);
	wb.open("sheets", &[]);
	for (i, s) in book.sheets.iter().enumerate() {
		wb.empty("sheet", &[
			("name", &sheet_name(&s.name, i)),
			("sheetId", &fmt!("{}", i + 1)),
			("r:id", &ids[i]),
		]);
	}
	res!(wb.close("sheets"));
	res!(wb.close("workbook"));

	zip.set("[Content_Types].xml", res!(types.write()).into_bytes(), Method::Deflate);
	zip.set("_rels/.rels", res!(root.write()).into_bytes(), Method::Deflate);
	zip.set("xl/workbook.xml", res!(wb.finish()).into_bytes(), Method::Deflate);
	zip.set("xl/_rels/workbook.xml.rels", res!(wb_rels.write()).into_bytes(), Method::Deflate);
	for (i, s) in book.sheets.iter().enumerate() {
		let part = res!(sheet_part(s, &seen));
		zip.set(&fmt!("xl/worksheets/sheet{}.xml", i + 1), part.into_bytes(), Method::Deflate);
	}
	zip.set("xl/sharedStrings.xml", res!(shared(&strings)).into_bytes(), Method::Deflate);
	zip.set("xl/styles.xml", res!(styles()).into_bytes(), Method::Deflate);
	zip.write()
}

/// A sheet's name, made legal.
///
/// Excel refuses `: \ / ? * [ ]` in a tab name and refuses one over 31 characters, and refuses the
/// whole file rather than the name. A caller's name is corrected here rather than rejected, because
/// the name of a tab is not worth failing a document over.
fn sheet_name(name: &str, i: usize) -> String {
	let cleaned: String = name.chars()
		.filter(|c| !matches!(c, ':' | '\\' | '/' | '?' | '*' | '[' | ']'))
		.take(31)
		.collect();
	match cleaned.trim().is_empty() {
		true	=> fmt!("Sheet{}", i + 1),
		false	=> cleaned,
	}
}

fn sheet_part(s: &crate::office::sheet::Sheet, seen: &BTreeMap<&str, usize>) -> Outcome<String> {
	let mut out = Out::declared();
	out.open("worksheet", &[("xmlns", NS_S), ("xmlns:r", NS_R)]);
	// What rectangle the sheet occupies. A reader is not obliged to believe it -- this one reads the
	// cells' own addresses -- but a reader that sizes a grid before filling it wants the hint.
	if let Some(extent) = s.extent() {
		out.empty("dimension", &[("ref", &extent.name())]);
	}
	out.open("sheetData", &[]);
	for (r, row) in s.rows.iter().enumerate() {
		// A row of nothing is not written at all. A sheet is sparse and a reader reads the addresses,
		// so an omitted row costs nothing and a written empty one costs a line.
		if row.iter().all(|c| c.is_empty()) {
			continue;
		}
		out.open("row", &[("r", &fmt!("{}", r + 1))]);
		for (c, cell) in row.iter().enumerate() {
			if cell.is_empty() {
				continue;
			}
			res!(cell_part(&mut out, cell, &Ref { col: c as u32, row: r as u32 }, seen));
		}
		res!(out.close("row"));
	}
	res!(out.close("sheetData"));
	res!(out.close("worksheet"));
	out.finish()
}

/// One cell.
///
/// The address is written on every cell, because that is what makes a sheet sparse: a reader places a
/// cell by its `r` and not by its position, and a writer that left the attribute off would produce a
/// file every reader misaligns at the first gap.
fn cell_part(
	out:	&mut Out,
	cell:	&Cell,
	at:	&Ref,
	seen:	&BTreeMap<&str, usize>,
)
	-> Outcome<()>
{
	let name = at.name();
	let mut attrs: Vec<(&str, &str)> = vec![("r", &name)];
	let kind = match &cell.value {
		Value::Text(_)	=> Some("s"),
		Value::Bool(_)	=> Some("b"),
		Value::Error(_)	=> Some("e"),
		// A number and a date are both numbers; what separates them is the style.
		_		=> None,
	};
	if let Some(kind) = kind {
		attrs.push(("t", kind));
	}
	if matches!(cell.value, Value::Date(_)) {
		attrs.push(("s", STYLE_DATE));
	}
	out.open("c", &attrs);
	if let Some(f) = &cell.formula {
		// The formula, and then the value beside it. Both, always: a formula written without its
		// cached value shows as blank in every reader that does not calculate, which includes every
		// reader on a phone.
		out.leaf("f", &[], f);
	}
	let v = match &cell.value {
		Value::Empty	=> None,
		Value::Text(t)	=> Some(fmt!("{}", seen.get(t.as_str()).copied().unwrap_or(0))),
		Value::Number(n)	=> Some(repr(*n)),
		Value::Bool(b)	=> Some(match b {
			true	=> "1".to_string(),
			false	=> "0".to_string(),
		}),
		Value::Error(e)	=> Some(e.clone()),
		Value::Date(d)	=> Some(repr(serial_of(d))),
	};
	if let Some(v) = v {
		out.leaf("v", &[], &v);
	}
	res!(out.close("c"));
	Ok(())
}

/// A number as the file stores it, which is not how a person reads it.
///
/// Seventeen significant figures round-trips every `f64` exactly, and the file is not read by a
/// person -- `Value::show` is what a person sees. Writing fewer here would lose precision the caller
/// handed over.
fn repr(n: f64) -> String {
	if !n.is_finite() {
		return "0".to_string();
	}
	if n == n.trunc() && n.abs() < 1e15 {
		return fmt!("{}", n as i64);
	}
	let mut s = fmt!("{}", n);
	if s.parse::<f64>() != Ok(n) {
		s = fmt!("{:.17e}", n);
	}
	s
}

/// The serial number a date text stands for, counting days from the last day of 1899.
///
/// The epoch is 30 December 1899 and not 1 January 1900, which looks like an off-by-two and is not:
/// Lotus 1-2-3 treated 1900 as a leap year, Excel copied the bug deliberately for compatibility, and
/// every spreadsheet since has kept it. Serial 60 is 29 February 1900, a day that did not happen. The
/// two errors cancel for every date from 1 March 1900 onward, which is every date anybody stores.
fn serial_of(text: &str) -> f64 {
	let (date, time) = match text.split_once(['T', ' ']) {
		Some((d, t))	=> (d, Some(t)),
		None		=> (text, None),
	};
	let parts: Vec<&str> = date.split('-').collect();
	if parts.len() != 3 {
		return 0.0;
	}
	let y: i64 = parts[0].parse().unwrap_or(1900);
	let m: i64 = parts[1].parse().unwrap_or(1);
	let d: i64 = parts[2].parse().unwrap_or(1);
	let days = days_from_civil(y, m, d) - days_from_civil(1899, 12, 30);
	let frac = match time {
		None		=> 0.0,
		Some(t)	=> {
			let hms: Vec<&str> = t.trim_end_matches('Z').split(':').collect();
			let h: f64 = hms.first().and_then(|s| s.parse().ok()).unwrap_or(0.0);
			let mi: f64 = hms.get(1).and_then(|s| s.parse().ok()).unwrap_or(0.0);
			let se: f64 = hms.get(2).and_then(|s| s.parse().ok()).unwrap_or(0.0);
			(h * 3600.0 + mi * 60.0 + se) / 86_400.0
		}
	};
	days as f64 + frac
}

/// Days from 1970-01-01 to a civil date, by Howard Hinnant's algorithm.
///
/// Written out rather than reached for through a calendar crate: this is the only date arithmetic
/// here, it is eleven lines, and a dependency for eleven lines is a dependency.
pub(crate) fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
	let y = y - i64::from(m <= 2);
	let era = if y >= 0 { y } else { y - 399 } / 400;
	let yoe = y - era * 400;
	let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
	let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
	era * 146_097 + doe - 719_468
}

fn shared(strings: &[&str]) -> Outcome<String> {
	let n = fmt!("{}", strings.len());
	let mut out = Out::declared();
	out.open("sst", &[("xmlns", NS_S), ("count", &n), ("uniqueCount", &n)]);
	for s in strings {
		out.open("si", &[]);
		// `xml:space="preserve"` or a string that is spaces arrives as nothing, and a column of
		// deliberate blanks becomes a column of empties.
		out.leaf("t", &[("xml:space", "preserve")], s);
		res!(out.close("si"));
	}
	res!(out.close("sst"));
	out.finish()
}

/// The styles part: the tables Excel insists on, and the two formats this writer uses.
fn styles() -> Outcome<String> {
	let mut out = Out::declared();
	out.open("styleSheet", &[("xmlns", NS_S)]);

	out.open("fonts", &[("count", "1")]);
	out.open("font", &[]);
	out.empty("sz", &[("val", "11")]);
	out.empty("name", &[("val", "Calibri")]);
	res!(out.close("font"));
	res!(out.close("fonts"));

	// Both of these, in this order. Excel wants `none` then `gray125` whether or not anything uses
	// either, and a file without them opens with a repair prompt that names no reason.
	out.open("fills", &[("count", "2")]);
	for pattern in ["none", "gray125"] {
		out.open("fill", &[]);
		out.empty("patternFill", &[("patternType", pattern)]);
		res!(out.close("fill"));
	}
	res!(out.close("fills"));

	out.open("borders", &[("count", "1")]);
	out.open("border", &[]);
	for side in ["left", "right", "top", "bottom", "diagonal"] {
		out.empty(side, &[]);
	}
	res!(out.close("border"));
	res!(out.close("borders"));

	out.open("cellStyleXfs", &[("count", "1")]);
	out.empty("xf", &[("numFmtId", "0"), ("fontId", "0"), ("fillId", "0"), ("borderId", "0")]);
	res!(out.close("cellStyleXfs"));

	// Index 0 plain, index 1 a date. Nothing else, because nothing else is used: an unused style is
	// a number somebody later trusts.
	out.open("cellXfs", &[("count", "2")]);
	out.empty("xf", &[("numFmtId", "0"), ("fontId", "0"), ("fillId", "0"), ("borderId", "0"),
		("xfId", "0")]);
	// Number format 14 is the built-in short date, which each reader renders in its own locale --
	// which is right: a date is a date, and how it is written belongs to whoever is reading it.
	out.empty("xf", &[("numFmtId", "14"), ("fontId", "0"), ("fillId", "0"), ("borderId", "0"),
		("xfId", "0"), ("applyNumberFormat", "1")]);
	res!(out.close("cellXfs"));

	res!(out.close("styleSheet"));
	out.finish()
}
