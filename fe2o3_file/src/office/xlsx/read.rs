//! Reading a `.xlsx` into the neutral spreadsheet.
//!
//! The three traps [`crate::office::xlsx`] names are all here, and each of them is the kind of wrong
//! that looks right: a shared string read as its index gives a column of small integers, a date read
//! without its style gives a column of five-digit numbers, and a cell placed by its position rather
//! than by its address puts every value after a gap one column to the left.
//!
//! # The ceiling is stated rather than streamed past
//!
//! A sheet part is XML and inflates roughly ten to one, so [`MAX_PART`] is a ceiling on what comes
//! *out* rather than on what is on disk. Above it the file is refused, by name and with the number,
//! and it is not read a piece at a time.
//!
//! That is a real limit and it is written down rather than hidden: streaming would need a second XML
//! reader, of the kind that hands over events instead of a tree, and the tree is what the editing
//! path needs for its spans. One reader that refuses honestly above a stated ceiling beats two
//! readers that disagree about what a document says.
//!
//! [Written entirely with AI](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use crate::office::opc::{
	REL_DOC,
	REL_SHEET,
	REL_STRINGS,
	REL_STYLES,
};
use crate::office::sheet::{
	Book,
	Cell,
	MAX_COL,
	MAX_ROW,
	Ref,
	Sheet,
	Value,
};
use crate::zip::Zip;

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_text::xml::{
	Elem,
	Xml,
};

use std::collections::BTreeMap;

// The most a single part is inflated to. A sheet of a hundred thousand rows is about 30 MB of XML,
// so this admits every spreadsheet a person keeps and refuses the ones built to exhaust a reader.
pub const MAX_PART: u64 = 96 * 1024 * 1024;

/// The leading bytes of an OLE compound file: an encrypted workbook, or a `.xls` from before 2007.
const OLE_MAGIC: [u8; 8] = [0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];

/// A workbook read for reading, and what came with it.
#[derive(Clone, Debug, Default)]
pub struct Reading {
	pub book:	Book,
	pub macros:	bool,	// said, never run
	// Worth saying on screen, because it is the number that tells a reader whether what they are
	// looking at is data or the result of a calculation they cannot see.
	pub formulas:	usize,
	// Sheets the workbook names and whose part is missing or unreadable, named rather than dropped:
	// a workbook that quietly came back with four of its five sheets is worse than one that says
	// which is absent.
	pub missing:	Vec<String>,
}

pub fn read(bytes: &[u8]) -> Outcome<Reading> {
	if bytes.len() >= OLE_MAGIC.len() && bytes[..OLE_MAGIC.len()] == OLE_MAGIC {
		return Err(err!(
			"This is an OLE compound file, not a `.xlsx`. Either it is encrypted, or it is a `.xls` \
			from before 2007 -- a different format entirely, which this does not read. Nothing is \
			guessed at."; Invalid, Input, Unimplemented));
	}
	let zip = res!(Zip::read(bytes.to_vec()));
	let mut out = Reading::default();
	out.macros = zip.names().iter().any(|n| n.ends_with("vbaProject.bin"));

	let root_rels = res!(rels_of(&zip, ""));
	let main = res!(root_rels.values()
		.find(|(kind, _)| kind == REL_DOC)
		.map(|(_, t)| t.clone())
		.ok_or_else(|| err!(
			"The package names no workbook part, so this is not a spreadsheet. It holds: {}.",
			zip.names().join(", "); Invalid, Input, Missing)));
	let dir = dir_of(&main);
	let wb = res!(Xml::parse(&res!(part_text(&zip, &main))));
	let rels = res!(rels_of(&zip, &main));

	// A date is a number counted from one of two epochs, and which one is a property of the WORKBOOK.
	// A reader that assumed 1900 reads every date in a file written on a Mac before 2011 four years
	// and a day early.
	let epoch_1904 = res!(wb.root()).child("workbookPr")
		.and_then(|e| e.attr("date1904"))
		.map(|v| v == "1" || v == "true")
		.unwrap_or(false);

	let strings = res!(strings_of(&zip, &dir, &rels));
	let dates = res!(date_styles(&zip, &dir, &rels));

	let sheets = match res!(wb.root()).child("sheets") {
		Some(s)	=> s.children("sheet"),
		None		=> Vec::new(),
	};
	for s in sheets {
		let name = s.attr("name").unwrap_or("Sheet").to_string();
		let target = s.attr("r:id")
			.and_then(|id| rels.get(id))
			.filter(|(kind, _)| kind == REL_SHEET)
			.map(|(_, t)| t.clone());
		let target = match target {
			Some(t) if zip.has(&t)	=> t,
			_			=> {
				out.missing.push(name);
				continue;
			}
		};
		let part = match part_text(&zip, &target) {
			Ok(p)	=> p,
			Err(_)	=> {
				out.missing.push(name);
				continue;
			}
		};
		let xml = match Xml::parse(&part) {
			Ok(x)	=> x,
			Err(_)	=> {
				out.missing.push(name);
				continue;
			}
		};
		let mut sheet = Sheet::new(name);
		res!(cells(&xml, &mut sheet, &strings, &dates, epoch_1904, &mut out.formulas));
		out.book.sheets.push(sheet);
	}
	Ok(out)
}

/// Reads the cells of one sheet, placing each by the address it carries.
fn cells(
	xml:	&Xml,
	sheet:	&mut Sheet,
	strings:	&[String],
	dates:	&[bool],
	epoch_1904:	bool,
	formulas:	&mut usize,
)
	-> Outcome<()>
{
	let data = match res!(xml.root()).child("sheetData") {
		Some(d)	=> d,
		None		=> return Ok(()),
	};
	// The address is read from the cell and the row is read from the row, and where either is absent
	// the position is the fallback. Both exist in the wild: a generator that writes dense rows often
	// leaves the attributes off entirely.
	let mut at_row: u32 = 0;
	for row in data.children("row") {
		let r = row.attr("r")
			.and_then(|v| v.parse::<u32>().ok())
			.map(|v| v.saturating_sub(1))
			.unwrap_or(at_row);
		if r >= MAX_ROW {
			continue;
		}
		let mut at_col: u32 = 0;
		for c in row.children("c") {
			let pos = match c.attr("r") {
				Some(a)	=> match Ref::parse(a) {
					Ok(p)	=> p,
					// An address that does not parse is placed where it stood, which keeps the rest
					// of the row aligned rather than losing it.
					Err(_)	=> Ref { col: at_col, row: r },
				},
				None		=> Ref { col: at_col, row: r },
			};
			at_col = pos.col.saturating_add(1);
			if pos.col >= MAX_COL {
				continue;
			}
			let cell = cell_of(xml, c, strings, dates, epoch_1904);
			if cell.formula.is_some() {
				*formulas += 1;
			}
			if cell.is_empty() {
				continue;
			}
			// Grown to fit rather than allocated to the sheet's declared size: a sheet claiming a
			// million rows and holding four should cost four.
			while sheet.rows.len() <= pos.row as usize {
				sheet.rows.push(Vec::new());
			}
			let line = &mut sheet.rows[pos.row as usize];
			while line.len() <= pos.col as usize {
				line.push(Cell::empty());
			}
			line[pos.col as usize] = cell;
		}
		at_row = r.saturating_add(1);
	}
	Ok(())
}

fn cell_of(
	xml:	&Xml,
	c:	&Elem,
	strings:	&[String],
	dates:	&[bool],
	epoch_1904:	bool,
)
	-> Cell
{
	// A formula's text is read and NEVER evaluated. What is displayed is the `<v>` beside it, which
	// is what the last calculation left and what the person who wrote the file saw.
	let formula = c.child("f").map(|f| xml.text_of(f)).filter(|f| !f.is_empty());
	let raw = c.child("v").map(|v| xml.text_of(v));
	let kind = c.attr("t").unwrap_or("n");
	let value = match kind {
		// The trap: this is an INDEX into the shared string table, not a number.
		"s"	=> match raw.as_ref().and_then(|v| v.parse::<usize>().ok()).and_then(|i| strings.get(i)) {
			Some(s)	=> Value::Text(s.clone()),
			None		=> Value::Empty,
		},
		// A formula whose result is text carries it directly.
		"str"	=> match raw {
			Some(v)	=> Value::Text(v),
			None		=> Value::Empty,
		},
		"inlineStr"	=> {
			let text: String = match c.child("is") {
				Some(is)	=> xml.text_of(is),
				None		=> String::new(),
			};
			match text.is_empty() {
				true	=> Value::Empty,
				false	=> Value::Text(text),
			}
		}
		"b"	=> Value::Bool(raw.as_deref() == Some("1")),
		"e"	=> Value::Error(raw.unwrap_or_default()),
		// A date written as an ISO string rather than as a serial. Rare, and legal.
		"d"	=> match raw {
			Some(v)	=> Value::Date(v),
			None		=> Value::Empty,
		},
		_	=> match raw.as_ref().and_then(|v| v.trim().parse::<f64>().ok()) {
			None		=> Value::Empty,
			Some(n)	=> {
				// The other trap: a number is a date only because its style says so.
				let style = c.attr("s").and_then(|v| v.parse::<usize>().ok()).unwrap_or(0);
				match dates.get(style).copied().unwrap_or(false) {
					true	=> Value::Date(date_of(n, epoch_1904)),
					false	=> Value::Number(n),
				}
			}
		},
	};
	Cell { value, formula }
}

/// A serial number as the date it stands for, in ISO 8601.
///
/// The epoch is 30 December 1899 and not 1 January 1900, which looks like an off-by-two and is not:
/// Lotus 1-2-3 treated 1900 as a leap year, Excel copied the bug deliberately for compatibility, and
/// every spreadsheet since has kept it. Serial 60 is 29 February 1900, a day that did not happen; the
/// two errors cancel from 1 March 1900 onward, which is every date anybody stores.
pub fn date_of(serial: f64, epoch_1904: bool) -> String {
	let base = match epoch_1904 {
		true	=> super::write::days_from_civil(1904, 1, 1),
		false	=> super::write::days_from_civil(1899, 12, 30),
	};
	let days = serial.floor();
	let frac = serial - days;
	let (y, m, d) = civil_from_days(base + days as i64);
	// A whole number of days is a date; anything else carries a time, and a serial under one is a
	// time of day with no date at all.
	let secs = (frac * 86_400.0).round() as i64;
	if secs == 0 {
		return fmt!("{:04}-{:02}-{:02}", y, m, d);
	}
	let (h, mi, s) = (secs / 3600, (secs % 3600) / 60, secs % 60);
	if serial < 1.0 {
		return fmt!("{:02}:{:02}:{:02}", h, mi, s);
	}
	fmt!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}", y, m, d, h, mi, s)
}

/// The civil date a day count from 1970-01-01 names, by Howard Hinnant's algorithm.
fn civil_from_days(z: i64) -> (i64, i64, i64) {
	let z = z + 719_468;
	let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
	let doe = z - era * 146_097;
	let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
	let y = yoe + era * 400;
	let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
	let mp = (5 * doy + 2) / 153;
	let d = doy - (153 * mp + 2) / 5 + 1;
	let m = mp + if mp < 10 { 3 } else { -9 };
	(y + i64::from(m <= 2), m, d)
}

/// Which style indices mean "this number is a date".
///
/// Two hops, and both matter. A cell names a `cellXfs` entry by index; that entry names a number
/// format by id; and the format is a date either because its id is one of the built-in date formats
/// or because its own code says so. A reader that checked only the built-ins misses every workbook
/// whose author set their own date format, which is most of them.
fn date_styles(
	zip:	&Zip,
	dir:	&str,
	rels:	&BTreeMap<String, (String, String)>,
)
	-> Outcome<Vec<bool>>
{
	let part = match part_of(rels, REL_STYLES, dir, "styles.xml", zip) {
		Some(p)	=> p,
		None		=> return Ok(Vec::new()),
	};
	let xml = res!(Xml::parse(&res!(part_text(zip, &part))));
	let root = res!(xml.root());
	// The formats the workbook defined for itself.
	let mut custom: BTreeMap<u32, bool> = BTreeMap::new();
	if let Some(fmts) = root.child("numFmts") {
		for f in fmts.children("numFmt") {
			if let (Some(id), Some(code)) = (f.attr("numFmtId"), f.attr("formatCode")) {
				if let Ok(id) = id.parse::<u32>() {
					custom.insert(id, is_date_code(code));
				}
			}
		}
	}
	let mut out = Vec::new();
	if let Some(xfs) = root.child("cellXfs") {
		for xf in xfs.children("xf") {
			let id = xf.attr("numFmtId").and_then(|v| v.parse::<u32>().ok()).unwrap_or(0);
			out.push(match custom.get(&id) {
				Some(known)	=> *known,
				None		=> builtin_is_date(id),
			});
		}
	}
	Ok(out)
}

/// Whether a built-in number format id is a date or a time.
///
/// The built-ins are fixed by the specification: 14 to 22 are the dates and times, and 45 to 47 are
/// the elapsed-time formats.
fn builtin_is_date(id: u32) -> bool {
	matches!(id, 14..=22 | 45..=47)
}

/// Whether a format code describes a date or a time.
///
/// The letters are read outside quoted runs and outside the bracketed sections, because a format may
/// legitimately say `"day "0` or `[Red]`, and reading the `d` in a quoted word as a day is how a
/// column of money becomes a column of dates.
fn is_date_code(code: &str) -> bool {
	let mut chars = code.chars().peekable();
	let mut quoted = false;
	let mut bracket = false;
	while let Some(c) = chars.next() {
		match c {
			'"'			=> quoted = !quoted,
			'['			=> bracket = true,
			']'			=> bracket = false,
			// A backslash escapes the character after it, which is then literal.
			'\\'			=> { chars.next(); }
			_ if quoted || bracket	=> {}
			'y' | 'd' | 'h' | 's'	=> return true,
			// `m` is minutes or months and is a date either way. It is also the `m` in `mm` after an
			// `h`, which is still a time.
			'm'			=> return true,
			_			=> {}
		}
	}
	false
}

/// The shared string table, in index order.
fn strings_of(
	zip:	&Zip,
	dir:	&str,
	rels:	&BTreeMap<String, (String, String)>,
)
	-> Outcome<Vec<String>>
{
	let part = match part_of(rels, REL_STRINGS, dir, "sharedStrings.xml", zip) {
		Some(p)	=> p,
		None		=> return Ok(Vec::new()),
	};
	let xml = res!(Xml::parse(&res!(part_text(zip, &part))));
	// An entry may be one run or many -- a string with a bold word in the middle of it is three runs
	// -- and its text is all of them joined. Taking the first would silently truncate every styled
	// string in the workbook.
	Ok(res!(xml.root()).children("si").iter().map(|si| xml.text_of(si)).collect())
}

fn part_text(zip: &Zip, name: &str) -> Outcome<String> {
	let bytes = res!(zip.content_capped(name, MAX_PART));
	Ok(res!(String::from_utf8(bytes), Decode, String))
}

/// The directory a part sits in, with its trailing slash.
fn dir_of(part: &str) -> String {
	match part.rfind('/') {
		Some(k)	=> part[..k + 1].to_string(),
		None		=> String::new(),
	}
}

/// Where a relationship target actually is within the package.
fn resolve(dir: &str, target: &str) -> String {
	match target.starts_with('/') {
		true	=> target[1..].to_string(),
		false	=> fmt!("{}{}", dir, target),
	}
}

/// The relationships a part owns, by id.
fn rels_of(zip: &Zip, part: &str) -> Outcome<BTreeMap<String, (String, String)>> {
	let dir = dir_of(part);
	let name = &part[dir.len()..];
	let path = fmt!("{}_rels/{}.rels", dir, name);
	let mut out = BTreeMap::new();
	if !zip.has(&path) {
		return Ok(out);
	}
	let xml = res!(Xml::parse(&res!(part_text(zip, &path))));
	for rel in res!(xml.root()).children("Relationship") {
		let id = match rel.attr("Id") {
			Some(id)	=> id.to_string(),
			None		=> continue,
		};
		let kind = rel.attr("Type").unwrap_or("").to_string();
		let target = rel.attr("Target").unwrap_or("").to_string();
		let target = match rel.attr("TargetMode") {
			Some("External")	=> target,
			_			=> resolve(&dir, &target),
		};
		out.insert(id, (kind, target));
	}
	Ok(out)
}

/// Where a supporting part is: what the relationships say, or the conventional name where they say
/// nothing.
fn part_of(
	rels:	&BTreeMap<String, (String, String)>,
	kind:	&str,
	dir:	&str,
	usual:	&str,
	zip:	&Zip,
)
	-> Option<String>
{
	if let Some((_, target)) = rels.values().find(|(k, _)| k == kind) {
		if zip.has(target) {
			return Some(target.clone());
		}
	}
	let guess = fmt!("{}{}", dir, usual);
	match zip.has(&guess) {
		true	=> Some(guess),
		false	=> None,
	}
}
