//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use oxedyne_fe2o3_file::office::odf;
use oxedyne_fe2o3_file::office::sheet::{
	Book,
	Cell,
	MAX_TAB,
	Range,
	Ref,
	Sheet,
	Value,
	col_name,
	tab_names,
};
use oxedyne_fe2o3_file::office::xlsx;

use oxedyne_fe2o3_text::doc::markdown;

use oxedyne_fe2o3_core::{
	prelude::*,
	test::test_it,
};

// A `.xlsx` LibreOffice wrote, holding one of everything that separates this format from a table:
// shared strings, a formula with the value the last calculation left beside it, a date stored as a
// serial under a CUSTOM number format, a boolean, an entirely absent row, and a row whose cells skip
// two columns.
//
// Its content is ours -- LibreOffice was handed a spreadsheet this crate wrote and asked to save its
// own -- so the intent is known and every byte of the encoding is somebody else's. Reading back our
// own output would prove that the writer and the reader share their assumptions, which is precisely
// the thing worth doubting in a format this convention-bound.
const FOREIGN: &[u8] = include_bytes!("data/foreign.xlsx");

fn book() -> Book {
	let mut s = Sheet::new("Sales");
	s.rows.push(vec![Cell::text("Region"), Cell::text("Units"), Cell::text("Total")]);
	s.rows.push(vec![
		Cell::text("North"),
		Cell::number(120.0),
		Cell::formula("B2*3.4", Value::Number(408.0)),
	]);
	s.rows.push(Vec::new());
	s.rows.push(vec![Cell {
		value:	Value::Date("2026-03-14".to_string()),
		formula:	None,
	}]);
	Book { sheets: vec![s] }
}

// Four headings that reduce to TWO tab names. `Q1/Q2` and `Q1:Q2` both lose their middle character to
// the set Excel refuses, and the two long ones agree for the first 31 characters.
const CLASH: &str = "\
## Q1/Q2

| A | B |
| --- | --- |
| 1 | 2 |

## Q1:Q2

| A | B |
| --- | --- |
| 3 | 4 |

## A very long heading that runs past the thirty-one character limit, one

| A | B |
| --- | --- |
| 5 | 6 |

## A very long heading that runs past the thirty-one character limit, two

| A | B |
| --- | --- |
| 7 | 8 |
";

pub fn test_xlsx(filter: &'static str) -> Outcome<()> {

	res!(test_it(filter, &["A column is named the way a person writes it 000", "all", "xlsx"], || {
		// The boundaries are where this goes wrong, and it goes wrong silently: an off-by-one puts
		// every value one column out and the sheet still looks like a sheet.
		assert_eq!(col_name(0), "A");
		assert_eq!(col_name(25), "Z");
		assert_eq!(col_name(26), "AA");
		assert_eq!(col_name(51), "AZ");
		assert_eq!(col_name(52), "BA");
		assert_eq!(col_name(701), "ZZ");
		assert_eq!(col_name(702), "AAA");
		assert_eq!(col_name(16_383), "XFD", "the last column a sheet has");
		// And it round-trips, which is the property that actually matters.
		for i in [0u32, 1, 25, 26, 27, 700, 701, 702, 16_383] {
			let r = Ref { col: i, row: 0 };
			assert_eq!(res!(Ref::parse(&r.name())).col, i, "{} did not round trip", r.name());
		}
		Ok(())
	}));

	res!(test_it(filter, &["A reference is read as a person types it 001", "all", "xlsx"], || {
		assert_eq!(res!(Ref::parse("A1")), Ref { col: 0, row: 0 });
		assert_eq!(res!(Ref::parse("D20")), Ref { col: 3, row: 19 });
		// A `$` is what a person copies out of a formula bar, and it names the same cell.
		assert_eq!(res!(Ref::parse("$B$4")), Ref { col: 1, row: 3 });
		assert_eq!(res!(Ref::parse("bc42")), res!(Ref::parse("BC42")));
		// A range given from its far corner is the rectangle somebody dragged, not an error.
		assert_eq!(res!(Range::parse("D20:A1")), res!(Range::parse("A1:D20")));
		assert_eq!(res!(Range::parse("B2")).cells(), 1);
		assert_eq!(res!(Range::parse("A1:D20")).cells(), 80);
		assert_eq!(res!(Range::parse("A1:D20")).name(), "A1:D20");
		for bad in ["", "1", "A", "A0", "$", "A1:", "ZZZZZ1", "A1048577"] {
			assert!(Ref::parse(bad).is_err() || Range::parse(bad).is_err(),
				"{:?} was accepted", bad);
		}
		Ok(())
	}));

	res!(test_it(filter, &["A foreign workbook reads back as what it says 002", "all", "xlsx"], || {
		let r = res!(xlsx::read(FOREIGN));
		assert_eq!(r.book.names(), vec!["Sales", "Notes"]);
		let s = res!(r.book.sheet("Sales").ok_or_else(|| err!("no Sales sheet"; Missing)));

		// A shared string is an INDEX and not a number. A reader that took the `<v>` at face value
		// returns a row of small integers where the column names were.
		assert_eq!(s.at(&res!(Ref::parse("A1"))).value, Value::Text("Region".to_string()));
		assert_eq!(s.at(&res!(Ref::parse("A2"))).value, Value::Text("North".to_string()));

		// A formula, and the value the last calculation left. BOTH, and the value is never
		// recomputed -- see `office::sheet` for why that is the correct answer and not a shortcut.
		let total = s.at(&res!(Ref::parse("D2")));
		assert_eq!(total.formula.as_deref(), Some("B2*C2"));
		assert_eq!(total.value, Value::Number(408.0), "the STORED value, not a fresh one");
		let sum = s.at(&res!(Ref::parse("D5")));
		assert_eq!(sum.formula.as_deref(), Some("SUM(D2:D3)"));
		assert_eq!(sum.value, Value::Number(1343.0));

		// A date is a number, and the ONLY thing that makes it a date is the number format its style
		// points at -- here a custom one, `m/d/yyyy`, which a reader checking only the built-in
		// format ids would miss. Serial 46095 is 14 March 2026, and that arithmetic is checked
		// against LibreOffice's rather than against itself.
		assert_eq!(s.at(&res!(Ref::parse("E2"))).value, Value::Date("2026-03-14".to_string()));
		assert_eq!(s.at(&res!(Ref::parse("E3"))).value, Value::Date("2026-04-01".to_string()));
		// And a number under the OTHER custom format in the same file, `General`, is still a number.
		assert_eq!(s.at(&res!(Ref::parse("B2"))).value, Value::Number(120.0));

		assert_eq!(s.at(&res!(Ref::parse("F2"))).value, Value::Bool(true));
		assert_eq!(s.at(&res!(Ref::parse("F3"))).value, Value::Bool(false));

		// The gap. Row 4 is absent from the file entirely and row 5 skips from A to D, so a reader
		// that pushed cells onto the end of a row would put the total in column B.
		assert!(s.at(&res!(Ref::parse("A4"))).is_empty(), "the absent row is absent");
		assert_eq!(s.at(&res!(Ref::parse("A5"))).value, Value::Text("Total".to_string()));
		assert!(s.at(&res!(Ref::parse("B5"))).is_empty(), "and the skipped columns are empty");
		assert!(s.at(&res!(Ref::parse("C5"))).is_empty());

		// The second sheet, and the two strings in it that are traps of their own.
		let n = res!(r.book.sheet("Notes").ok_or_else(|| err!("no Notes sheet"; Missing)));
		assert_eq!(n.at(&res!(Ref::parse("A1"))).value, Value::Text("   ".to_string()),
			"a string that is only spaces is still a string");
		assert_eq!(n.at(&res!(Ref::parse("B1"))).value, Value::Text("a < b & c".to_string()));
		assert_eq!(n.at(&res!(Ref::parse("C1"))).value, Value::Text("3.40".to_string()),
			"text that looks like a number is text, and keeps its trailing zero");

		assert!(!r.macros);
		assert!(r.missing.is_empty());
		assert!(r.formulas >= 3, "the formulas are counted: {}", r.formulas);
		Ok(())
	}));

	res!(test_it(filter, &["A window of a sheet is the window asked for 003", "all", "xlsx"], || {
		// What the range-reading tool spends. A spreadsheet's useful unit is a rectangle, not a file:
		// the whole of `xl/worksheets/sheet1.xml` for a real workbook is megabytes and says nothing a
		// reader wanted.
		let r = res!(xlsx::read(FOREIGN));
		let s = res!(r.book.sheet("Sales").ok_or_else(|| err!("no Sales sheet"; Missing)));
		let win = s.window(&res!(Range::parse("A1:C2")));
		assert_eq!(win.len(), 2);
		assert_eq!(win[0].len(), 3);
		assert_eq!(win[0][0].value.show(), "Region");
		assert_eq!(win[1][1].value.show(), "120", "a whole number shows without a trailing .0");
		assert_eq!(win[1][2].value.show(), "3.4");
		// Past the end is empty rather than an error: a person asking for A1:Z100 of a small sheet
		// wants the small sheet, not a refusal.
		let past = s.window(&res!(Range::parse("Y1:Z2")));
		assert_eq!(past.len(), 2);
		assert!(past.iter().all(|row| row.iter().all(|c| c.is_empty())));
		Ok(())
	}));

	res!(test_it(filter, &["A number shows as the file says it 004", "all", "xlsx"], || {
		// A spreadsheet showing 12.0 where the file says 12 has changed what the file says, and a
		// spreadsheet showing 0.30000000000000004 has changed it in the other direction.
		assert_eq!(Value::Number(12.0).show(), "12");
		assert_eq!(Value::Number(3.4).show(), "3.4");
		assert_eq!(Value::Number(0.1 + 0.2).show(), "0.3");
		assert_eq!(Value::Number(-0.0).show(), "0");
		assert_eq!(Value::Number(1e15).show(), "1000000000000000");
		assert_eq!(Value::Number(1.0 / 3.0).show(), "0.333333333333333");
		assert_eq!(Value::Bool(true).show(), "TRUE");
		assert_eq!(Value::Empty.show(), "");
		assert_eq!(Value::Error("#DIV/0!".to_string()).show(), "#DIV/0!");
		Ok(())
	}));

	res!(test_it(filter, &["A written workbook reads back as what was put in 005", "all", "xlsx"], || {
		// Weaker evidence than 002 and worth having for a different reason: it covers what the
		// foreign fixture cannot reach, and it is the pair the range tool actually runs on.
		let b = book();
		let bytes = res!(xlsx::write(&b));
		let r = res!(xlsx::read(&bytes));
		let s = res!(r.book.sheet("Sales").ok_or_else(|| err!("no Sales sheet"; Missing)));
		assert_eq!(s.at(&res!(Ref::parse("A1"))).value, Value::Text("Region".to_string()));
		assert_eq!(s.at(&res!(Ref::parse("B2"))).value, Value::Number(120.0));
		let f = s.at(&res!(Ref::parse("C2")));
		assert_eq!(f.formula.as_deref(), Some("B2*3.4"));
		assert_eq!(f.value, Value::Number(408.0));
		assert_eq!(s.at(&res!(Ref::parse("A4"))).value, Value::Date("2026-03-14".to_string()),
			"a date survives the trip through a serial number and back");
		// Written twice, the same bytes: nothing here comes from the clock.
		assert_eq!(bytes, res!(xlsx::write(&b)));
		// And the archive round-trips, which is what an edit will later rest on.
		let zip = res!(oxedyne_fe2o3_file::zip::Zip::read(bytes.clone()));
		assert_eq!(res!(zip.write()), bytes);
		Ok(())
	}));

	res!(test_it(filter, &["A workbook that cannot be read is named 006", "all", "xlsx"], || {
		// An OLE compound file is either an encrypted workbook or a `.xls` from before 2007, which is
		// a different format entirely. Both are refused by name rather than read as rubble.
		let ole = [0xD0u8, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1, 0, 0, 0, 0, 0, 0, 0, 0];
		let e = xlsx::read(&ole);
		assert!(e.is_err());
		match e {
			Err(e)	=> {
				let s = fmt!("{}", e);
				assert!(s.contains("OLE compound file"), "{}", s);
			}
			Ok(_)	=> panic!("an OLE file was read"),
		}
		assert!(xlsx::read(b"not a workbook").is_err());
		// A ZIP that is not a workbook says so rather than coming back empty.
		let mut zip = oxedyne_fe2o3_file::zip::Zip::new();
		zip.set("hello.txt", b"hi".to_vec(), oxedyne_fe2o3_file::zip::Method::Store);
		assert!(xlsx::read(&res!(zip.write())).is_err());
		Ok(())
	}));

	res!(test_it(filter, &["A sheet the workbook names and cannot supply is said 007", "all", "xlsx"], || {
		// A workbook that quietly came back with one of its two sheets is worse than one that says
		// which is absent, because nothing on screen would say a sheet was ever there.
		let b = book();
		let bytes = res!(xlsx::write(&b));
		let mut zip = res!(oxedyne_fe2o3_file::zip::Zip::read(bytes));
		assert!(zip.remove("xl/worksheets/sheet1.xml"));
		let r = res!(xlsx::read(&res!(zip.write())));
		assert_eq!(r.missing, vec!["Sales".to_string()]);
		assert!(r.book.sheets.is_empty());
		Ok(())
	}));

	res!(test_it(filter, &["Two headings that reduce to one tab name are told apart 008", "all", "xlsx"], || {
		// Excel refuses a workbook with two tabs of one name, and refuses the FILE rather than the
		// name. LibreOffice and openpyxl each silently RENAME instead, so a reader-based oracle
		// repairs this defect rather than reporting it and the user simply does not get the tab they
		// asked for.
		let doc = res!(markdown::parse(CLASH));
		let book = Book::from_doc(&doc);
		assert_eq!(book.sheets.len(), 4, "four tables, four sheets");
		// The MODEL keeps the heading whole. Truncating there would throw away the characters that
		// tell the two long headings apart before anything got the chance to use them.
		assert!(book.sheets[2].name.ends_with("one"), "{}", book.sheets[2].name);
		assert!(book.sheets[3].name.ends_with("two"), "{}", book.sheets[3].name);

		let tabs = tab_names(&book);
		// Legal, short enough, and no two alike -- the three things Excel refuses a file over.
		for t in &tabs {
			assert!(t.chars().count() <= MAX_TAB, "'{}' is {} characters", t, t.chars().count());
			assert!(!t.contains([':', '\\', '/', '?', '*', '[', ']']), "'{}' holds a refused character", t);
		}
		for (i, t) in tabs.iter().enumerate() {
			assert!(!tabs[..i].iter().any(|p| p.eq_ignore_ascii_case(t)),
				"'{}' is used twice, in {:?}", t, tabs);
		}
		// And the disambiguation is one a person would recognise, which is Excel's own.
		assert_eq!(tabs[0], "Q1 Q2");
		assert_eq!(tabs[1], "Q1 Q2 (2)");
		assert!(tabs[3].ends_with(" (2)"), "the second long name is not marked: {}", tabs[3]);

		// The names the WRITERS actually put in the file, which is the thing a reader sees. Asserting
		// on `tab_names` alone would pass even if neither writer called it.
		let bytes = res!(xlsx::write(&book));
		let read = res!(xlsx::read(&bytes));
		assert_eq!(read.book.names(), tabs, "the .xlsx does not carry the names that were settled on");
		// The `.ods` writer had no deduplication at all, and OpenDocument requires distinct table
		// names too. It takes the same names so one sheet answers to one name in either format.
		let ods = res!(odf::sheet::write(&book));
		let back = res!(odf::sheet::read(&ods));
		assert_eq!(back.book.names(), tabs, "the .ods does not carry the same names as the .xlsx");
		Ok(())
	}));

	Ok(())
}
