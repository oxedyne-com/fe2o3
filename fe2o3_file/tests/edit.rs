//! The third verb: changing a document somebody else wrote.
//!
//! Every fixture here is FOREIGN -- LibreOffice wrote it -- because the property being tested is that
//! an edit leaves a stranger's file intact, and a file this crate wrote has none of the constructs
//! that would be lost. The archive check is the load-bearing one: it compares the COMPRESSED BYTES of
//! every member nobody touched, not their content, because two members can hold the same content and
//! different bytes and it is the bytes a colleague's reader parses.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use oxedyne_fe2o3_file::office::docx;
use oxedyne_fe2o3_file::office::edit::Find;
use oxedyne_fe2o3_file::office::odf;
use oxedyne_fe2o3_file::office::sheet::{
	Ref,
	Value,
	typed,
};
use oxedyne_fe2o3_file::office::xlsx;
use oxedyne_fe2o3_file::zip::Zip;

use oxedyne_fe2o3_core::{
	prelude::*,
	test::test_it,
};

// A `.docx` LibreOffice wrote: three paragraphs, one of which splits a sentence across four runs.
const DOCX: &[u8] = include_bytes!("data/rich.docx");
// An `.odt` LibreOffice wrote, holding one of everything the tree can carry.
const ODT: &[u8] = include_bytes!("data/foreign.odt");
// A `.xlsx` LibreOffice wrote: shared strings, formulas with cached values, a styled date, a boolean,
// an absent row and a row with a gap in it.
const XLSX: &[u8] = include_bytes!("data/foreign.xlsx");
// The same spreadsheet as an `.ods`, where the empty row is one repeated cell.
const ODS: &[u8] = include_bytes!("data/foreign.ods");

/// Which members differ between two packages, and which are missing from the second.
///
/// The comparison is of the members' own bytes rather than of their content: a member rebuilt with the
/// same content and a different compression level is a member that was not copied, and copying is the
/// whole claim.
fn differs(before: &[u8], after: &[u8]) -> Outcome<Vec<String>> {
	let a = res!(Zip::read(before.to_vec()));
	let b = res!(Zip::read(after.to_vec()));
	let mut out = Vec::new();
	for m in a.members() {
		match b.member(&m.name) {
			None		=> out.push(fmt!("{} is gone", m.name)),
			Some(n)	=> {
				if res!(m.raw(&a)) != res!(n.raw(&b)) {
					out.push(m.name.clone());
				}
			}
		}
	}
	Ok(out)
}

/// The cells of the first sheet of either spreadsheet format, as the text a person would see.
fn grid(bytes: &[u8], ods: bool) -> Outcome<Vec<Vec<String>>> {
	let book = match ods {
		true	=> res!(odf::sheet::read(bytes)).book,
		false	=> res!(xlsx::read(bytes)).book,
	};
	let sheet = res!(book.sheets.first().ok_or_else(|| err!("no sheets"; Missing)));
	Ok(sheet.rows.iter()
		.map(|r| r.iter().map(|c| c.value.show()).collect())
		.collect())
}

// The two spreadsheet formats, so a test asserts the same property of both in one place. One test
// over both rather than two tests: the answer has to be the SAME answer, and two tests would let the
// formats drift while each still passed.
const FORMATS: [(bool, &str, &[u8]); 2] = [(false, "xlsx", XLSX), (true, "ods", ODS)];

/// Writes cells into whichever spreadsheet format, from `(ref, value, formula)` triples.
fn write_cells(ods: bool, src: &[u8], cells: &[(&str, Option<&str>, Option<&str>)])
	-> Outcome<Vec<u8>>
{
	match ods {
		true	=> {
			let mut sets = Vec::new();
			for (at, value, formula) in cells {
				sets.push(odf::sheet::Set {
					sheet:	None,
					at:	res!(Ref::parse(at)),
					value:	value.map(|v| v.to_string()),
					formula:	formula.map(|f| f.to_string()),
				});
			}
			Ok(res!(odf::sheet::edit(src, &sets)).bytes)
		}
		false	=> {
			let mut sets = Vec::new();
			for (at, value, formula) in cells {
				sets.push(xlsx::edit::Set {
					sheet:	None,
					at:	res!(Ref::parse(at)),
					value:	value.map(|v| v.to_string()),
					formula:	formula.map(|f| f.to_string()),
				});
			}
			Ok(res!(xlsx::edit::edit(src, &sets)).bytes)
		}
	}
}

pub fn test_edit(filter: &'static str) -> Outcome<()> {

	res!(test_it(filter, &["A replacement spread over four runs lands as one 000", "all", "edit"], || {
		// The sentence is `An opening `, `paragraph with bold words`, ` and `, `italic`, ` ones.` in
		// the file -- LibreOffice split it where the formatting changes. A find that matched inside one
		// run at a time would find none of this, which is the whole reason the pieces are joined.
		let before = res!(docx::edit::body_text(DOCX));
		let whole = "paragraph with bold words and italic";
		assert!(before.iter().any(|p| p.contains(whole)),
			"the fixture no longer holds the phrase this tests: {:?}", before);
		// And it is NOT one contiguous run: the part's own XML has markup through the middle of the
		// phrase, so a find that looked inside one element at a time would report it absent. Without
		// this the test above would pass on a fixture that made the join unnecessary.
		let zip = res!(Zip::read(DOCX.to_vec()));
		let part = res!(zip.text("word/document.xml"));
		assert!(!part.contains(whole),
			"the phrase stands in the XML as one string, so this fixture cannot prove the join works");

		let out = res!(docx::edit::edit(DOCX, &[Find::every(whole, "SENTENCE")]));
		assert!(out.runs >= 2, "a phrase spread over runs was rewritten in {} of them", out.runs);
		let after = res!(docx::edit::body_text(&out.bytes));
		assert!(after.iter().any(|p| p.contains("An opening SENTENCE ones.")),
			"the replacement did not land as one string: {:?}", after);
		// And the rest of the document says exactly what it said.
		assert_eq!(before.len(), after.len(), "an edit changed how many paragraphs there are");
		Ok(())
	}));

	res!(test_it(filter, &["Every part but the one edited is copied 010", "all", "edit"], || {
		let out = res!(docx::edit::edit(DOCX, &[Find::every("Findings", "Results")]));
		let moved = res!(differs(DOCX, &out.bytes));
		assert_eq!(moved, vec!["word/document.xml".to_string()],
			"an edit rewrote parts it was not asked to touch");
		Ok(())
	}));

	res!(test_it(filter, &["An unmatched find is an error naming it 020", "all", "edit"], || {
		// The failure this is written against is the SILENT one: a caller told the document was edited
		// has no way to find out that one of its four replacements did nothing.
		let e = docx::edit::edit(DOCX, &[Find::every("a phrase nobody wrote", "x")]);
		let msg = match e {
			Ok(_)	=> return Err(err!("an unmatched find was accepted"; Test)),
			Err(e)	=> fmt!("{}", e),
		};
		assert!(msg.contains("a phrase nobody wrote"), "the refusal does not name the string: {}", msg);
		// And nothing was written: the call answers with an error rather than with bytes.
		let e = odf::text::edit(ODT, &[
			Find::every("Widget", "Sprocket"),
			Find::every("a phrase nobody wrote", "x"),
		]);
		assert!(e.is_err(), "a list whose second edit matched nothing was accepted whole");
		Ok(())
	}));

	res!(test_it(filter, &["An occurrence is counted through the document 030", "all", "edit"], || {
		let out = res!(odf::text::edit(ODT, &[Find::at("bullet", "BULLET", 2)]));
		let after = res!(odf::text::body_text(&out.bytes));
		let hits: Vec<&String> = after.iter().filter(|p| p.contains("BULLET")).collect();
		assert_eq!(hits.len(), 1, "one occurrence was asked for and {} changed", hits.len());
		assert!(hits[0].contains("Second"), "the wrong occurrence changed: {}", hits[0]);
		// Past the end is refused, and the refusal says how many there are.
		let e = odf::text::edit(ODT, &[Find::at("bullet", "x", 99)]);
		let msg = match e {
			Ok(_)	=> return Err(err!("occurrence 99 of a phrase with four was accepted"; Test)),
			Err(e)	=> fmt!("{}", e),
		};
		assert!(msg.contains("99"), "the refusal does not name the occurrence asked for: {}", msg);
		Ok(())
	}));

	res!(test_it(filter, &["An .odt keeps its spaces when text is replaced 040", "all", "edit"], || {
		// OpenDocument collapses a literal run of spaces, so a replacement carrying one has to be
		// written as `<text:s>`. Left literal, the file opens and says something else.
		let out = res!(odf::text::edit(ODT, &[Find::every("Widget", "A  B   C")]));
		let after = res!(odf::text::body_text(&out.bytes));
		assert!(after.iter().any(|p| p.contains("A  B   C")),
			"the spaces did not survive the write: {:?}",
			after.iter().filter(|p| p.contains('A')).collect::<Vec<_>>());
		assert_eq!(res!(differs(ODT, &out.bytes)), vec!["content.xml".to_string()]);
		Ok(())
	}));

	res!(test_it(filter, &["A typed string becomes what a person meant 050", "all", "edit"], || {
		assert_eq!(typed("3.5"), Value::Number(3.5));
		assert_eq!(typed("12"), Value::Number(12.0));
		assert_eq!(typed("-0.25"), Value::Number(-0.25));
		assert_eq!(typed("true"), Value::Bool(true));
		assert_eq!(typed(""), Value::Empty);
		// The ones that must NOT become numbers. A part number, an account code and a padded figure
		// are all text, and a rule that parsed anything parseable would silently renumber them.
		assert_eq!(typed("007"), Value::Text("007".to_string()));
		assert_eq!(typed("+3"), Value::Text("+3".to_string()));
		assert_eq!(typed("1,000"), Value::Text("1,000".to_string()));
		assert_eq!(typed(" 4 "), Value::Text(" 4 ".to_string()));
		assert_eq!(typed("1e3"), Value::Text("1e3".to_string()));
		Ok(())
	}));

	res!(test_it(filter, &["A cell written into a gap lands in its own column 060", "all", "edit"], || {
		// Row 4 of both fixtures is empty -- in the `.ods` it is ONE cell with a repeat of six, which
		// is the case that goes silently wrong: a writer that ignored the repeat would move every
		// value after it. The neighbours either side are what prove it did not.
		//
		// Both formats in one test, because the answer has to be the SAME answer. Two tests would let
		// them drift and each still pass.
		for (ods, name, src) in FORMATS {
			let before = res!(grid(src, ods));
			let out = res!(write_cells(ods, src, &[("C4", Some("mid"), None), ("E4", Some("7.5"), None)]));
			let after = res!(grid(&out, ods));
			assert_eq!(after[3].get(2).map(|s| s.as_str()), Some("mid"),
				"{}: C4 did not land in column C: {:?}", name, after[3]);
			assert_eq!(after[3].get(4).map(|s| s.as_str()), Some("7.5"),
				"{}: E4 did not land in column E: {:?}", name, after[3]);
			assert_eq!(after[3].get(1).map(|s| s.as_str()).unwrap_or(""), "",
				"{}: a cell nobody wrote to gained a value: {:?}", name, after[3]);
			// And every OTHER row says what it said, which is what a moved run would break.
			for (i, row) in before.iter().enumerate() {
				if i == 3 {
					continue;
				}
				assert_eq!(after.get(i), Some(row),
					"{}: row {} changed and nobody asked it to", name, i + 1);
			}
		}
		Ok(())
	}));

	res!(test_it(filter, &["A written formula is left for the reader to work out 070", "all", "edit"], || {
		// Nothing here calculates, so a formula goes in without a cached value. What is checked is
		// that the EXPRESSION arrived intact -- and for the `.ods`, that its references came out
		// bracketed, because `of:=B2*C2` makes LibreOffice write `Err:510` over the value beside it.
		for (ods, name, src) in FORMATS {
			let out = res!(write_cells(ods, src, &[("D6", None, Some("=B2*C2"))]));
			let book = match ods {
				true	=> res!(odf::sheet::read(&out)).book,
				false	=> res!(xlsx::read(&out)).book,
			};
			let cell = res!(book.sheets.first().ok_or_else(|| err!("no sheets"; Missing)))
				.at(&res!(Ref::parse("D6")));
			assert_eq!(cell.formula.as_deref(), Some("B2*C2"),
				"{}: the formula did not arrive as written: {:?}", name, cell);
			assert!(cell.value.is_empty(),
				"{}: a value was invented for a formula nothing calculated: {:?}", name, cell);
			if ods {
				let zip = res!(Zip::read(out.clone()));
				let text = res!(zip.text("content.xml"));
				assert!(text.contains("of:=[.B2]*[.C2]"),
					"an OpenDocument formula was written without its brackets, which destroys the \
					value beside it: {}", text.len());
			}
		}
		Ok(())
	}));

	res!(test_it(filter, &["A sheet that is not there is named, not guessed at 080", "all", "edit"], || {
		for (ods, name, src) in FORMATS {
			let at = res!(Ref::parse("A1"));
			let e = match ods {
				true	=> odf::sheet::edit(src, &[odf::sheet::Set {
					sheet:	Some("Nowhere".to_string()),
					at,
					value:	Some("x".to_string()),
					formula:	None,
				}]).map(|e| e.bytes),
				false	=> xlsx::edit::edit(src, &[xlsx::edit::Set {
					sheet:	Some("Nowhere".to_string()),
					at,
					value:	Some("x".to_string()),
					formula:	None,
				}]).map(|e| e.bytes),
			};
			let msg = match e {
				Ok(_)	=> return Err(err!(
					"{}: a write to a sheet that does not exist was accepted", name; Test)),
				Err(e)	=> fmt!("{}", e),
			};
			assert!(msg.contains("Nowhere"), "{}: the refusal does not name the sheet: {}", name, msg);
			assert!(msg.contains("Sales"),
				"{}: the refusal does not say what sheets there are: {}", name, msg);
		}
		Ok(())
	}));

	res!(test_it(filter, &["One cell written twice is refused, in both formats 085", "all", "edit"], || {
		// There is no order for two writes to one cell to be applied in, so picking one would be a
		// rule about how a caller happened to build its list. The two formats have to answer the same
		// way -- and before this check they did not: the `.xlsx` path failed on an overlapping splice
		// and the `.ods` path silently took the first and dropped the second.
		for (ods, name, src) in FORMATS {
			let twice = [("B2", Some("first"), None), ("B2", Some("second"), None)];
			let msg = match write_cells(ods, src, &twice) {
				Ok(_)	=> return Err(err!("{}: one cell written twice was accepted", name; Test)),
				Err(e)	=> fmt!("{}", e),
			};
			assert!(msg.contains("B2"), "{}: the refusal does not name the cell: {}", name, msg);
			assert!(msg.contains("twice"), "{}: the refusal does not say what went wrong: {}", name, msg);
		}
		Ok(())
	}));

	res!(test_it(filter, &["A spreadsheet keeps the parts nobody wrote to 090", "all", "edit"], || {
		let out = res!(odf::sheet::edit(ODS, &[odf::sheet::Set {
			sheet:	None,
			at:	res!(Ref::parse("B2")),
			value:	Some("999".to_string()),
			formula:	None,
		}]));
		assert_eq!(res!(differs(ODS, &out.bytes)), vec!["content.xml".to_string()],
			"a cell edit rewrote parts of the package it had no business in");
		let out = res!(xlsx::edit::edit(XLSX, &[xlsx::edit::Set {
			sheet:	None,
			at:	res!(Ref::parse("B2")),
			value:	Some("999".to_string()),
			formula:	None,
		}]));
		// No formula was written, so the calculation chain is left where it is.
		let moved = res!(differs(XLSX, &out.bytes));
		assert!(moved.iter().all(|n| n.contains("sheet")),
			"a cell edit rewrote parts of the package it had no business in: {:?}", moved);
		Ok(())
	}));

	Ok(())
}
