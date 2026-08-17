//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use oxedyne_fe2o3_file::office::deck::Deck;
use oxedyne_fe2o3_file::office::odf;
use oxedyne_fe2o3_file::office::sheet::{
	Book,
	Cell,
	Ref,
	Sheet,
	Value,
};
use oxedyne_fe2o3_file::zip::{
	Method,
	Zip,
};

use oxedyne_fe2o3_core::{
	prelude::*,
	test::test_it,
};
use oxedyne_fe2o3_stds::media::Media;
use oxedyne_fe2o3_text::doc::{
	Block,
	markdown,
	text_of,
};

// Three files LibreOffice wrote: it was handed each of ours and asked to save its own, so the
// content is known and every byte of the encoding is somebody else's.
const ODT: &[u8] = include_bytes!("data/foreign.odt");	// the prose
const ODS: &[u8] = include_bytes!("data/foreign.ods");	// the spreadsheet
const ODP: &[u8] = include_bytes!("data/foreign.odp");	// the presentation

// Prose with one of everything.
const SOURCE: &str = "\
# A Report

An opening paragraph with **bold** and *emphasis*.

## The second heading

- First bullet
- Second with a [link](https://example.com/page)
    - A nested bullet

> A quotation.

```rust
fn main() {
    println!(\"indented\");
}
```

A closing paragraph.
";

// A workbook holding one of everything that separates a spreadsheet from a table.
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

pub fn test_odf(filter: &'static str) -> Outcome<()> {

	res!(test_it(filter, &["mimetype is first and stored, in all three 000", "all", "odf"], || {
		// THE one mechanical rule of this format. It is what lets a reader name the file from its
		// opening bytes without inflating anything, and a package that breaks it is one every reader
		// calls a plain ZIP -- silently, opening as an archive rather than refusing.
		let doc = res!(markdown::parse(SOURCE));
		let (odt, _) = res!(odf::text::write(&doc));
		let ods = res!(odf::sheet::write(&book()));
		let (odp, _) = res!(odf::slides::write(&Deck::from_doc(&doc)));
		for (what, bytes, media, kind) in [
			("odt", &odt, odf::text::MEDIA, Media::Odt),
			("ods", &ods, odf::sheet::MEDIA, Media::Ods),
			("odp", &odp, odf::slides::MEDIA, Media::Odp),
		] {
			let zip = res!(Zip::read(bytes.clone()));
			assert_eq!(zip.names().first(), Some(&"mimetype"), "{}: mimetype must be FIRST", what);
			// AND THAT LAST CHECK ALONE IS VACUOUS, so it is not left alone. `mimetype` is the first
			// member these writers add, so it comes out first whether `set_first` or `set` was
			// called -- swapping one for the other leaves the assertion green. What has teeth is
			// the property itself, checked below on a package where something else went in first.
			let m = res!(zip.member("mimetype").ok_or_else(|| err!("no mimetype"; Missing)));
			assert_eq!(m.method, Method::Store, "{}: mimetype must be STORED", what);
			assert_eq!(res!(zip.text("mimetype")), media);
			// And so the format is readable from the leading bytes, which is the whole point of the
			// rule: `Media::sniff` reads the member rather than trusting the file's name.
			assert_eq!(Media::sniff(&bytes[..256.min(bytes.len())]), kind,
				"{} was not named from its own bytes", what);
			// A file somebody renamed is still what it is.
			assert_eq!(oxedyne_fe2o3_stds::media::identify("holiday.zip", bytes).media, kind);
		}
		// The property with teeth: `set_first` puts a member at the HEAD of an archive that already
		// has members, which is what the package writer is relying on and what a plain `set` would
		// not do. Written as its own check because the three above cannot tell the two apart.
		let mut z = Zip::new();
		z.set("content.xml", b"<x/>".to_vec(), Method::Deflate);
		z.set("styles.xml", b"<y/>".to_vec(), Method::Deflate);
		z.set_first("mimetype", odf::text::MEDIA.as_bytes().to_vec(), Method::Store);
		assert_eq!(z.names(), vec!["mimetype", "content.xml", "styles.xml"]);
		let out = res!(z.write());
		assert_eq!(Media::sniff(&out[..128.min(out.len())]), Media::Odt,
			"and so the format is readable from the opening bytes");
		Ok(())
	}));

	res!(test_it(filter, &["A foreign .odt reads back as what it says 001", "all", "odf"], || {
		let r = res!(odf::text::read(ODT));
		let md = oxedyne_fe2o3_text::doc::markdown::write::render(&r.doc);
		// A heading says its own level here: no style to resolve, no built-in name to recognise.
		assert!(md.contains("# A Report On Something"), "{}", md);
		assert!(md.contains("## The second heading"), "{}", md);
		assert!(md.contains("- First bullet"), "{}", md);
		assert!(md.contains("[link to somewhere](https://example.com/page)"), "{}", md);
		assert!(md.contains("**bold words**"), "{}", md);
		assert!(md.contains("| Widget | 12 | 3.40 |"), "the table came through: {}", md);
		// A listing keeps its indentation, which needs `<text:s/>` for the LEADING spaces: a reader
		// drops a leading literal space, so four became three until that was fixed.
		assert!(md.contains("    println!"), "the indentation survived:\n{}", md);
		assert!(r.doc.blocks.iter().any(|b| matches!(b, Block::Quote(_))), "the quotation is one");
		assert!(!r.macros);
		Ok(())
	}));

	res!(test_it(filter, &["A foreign .ods reads back, values and all 002", "all", "odf"], || {
		let r = res!(odf::sheet::read(ODS));
		let s = res!(r.book.sheets.first().ok_or_else(|| err!("no sheet"; Missing)));
		assert_eq!(s.at(&res!(Ref::parse("A1"))).value, Value::Text("Region".to_string()));
		assert_eq!(s.at(&res!(Ref::parse("B2"))).value, Value::Number(120.0));
		// The stored value, and the formula beside it. Never recalculated.
		let total = s.at(&res!(Ref::parse("D2")));
		assert_eq!(total.value, Value::Number(408.0));
		assert!(total.formula.is_some(), "the formula came back");
		assert_eq!(s.at(&res!(Ref::parse("E2"))).value, Value::Date("2026-03-14".to_string()));
		assert_eq!(s.at(&res!(Ref::parse("F2"))).value, Value::Bool(true));
		// The gap: row 4 is empty and row 5 skips to D.
		assert!(s.at(&res!(Ref::parse("A4"))).is_empty());
		assert_eq!(s.at(&res!(Ref::parse("A5"))).value, Value::Text("Total".to_string()));
		assert!(s.at(&res!(Ref::parse("B5"))).is_empty());
		assert_eq!(s.at(&res!(Ref::parse("D5"))).value, Value::Number(1343.0));
		assert!(r.formulas >= 3, "formulas counted: {}", r.formulas);
		Ok(())
	}));

	res!(test_it(filter, &["A formula is written in OpenFormula, not A1 003", "all", "odf"], || {
		// A reference must be bracketed. Written in the A1 syntax a `.xlsx` uses, LibreOffice cannot
		// parse it, RECALCULATES the cell, and writes `Err:510` over the stored value -- so a wrong
		// formula does not merely fail to work, it destroys the number that was there.
		use oxedyne_fe2o3_file::office::odf::sheet::openformula as of;
		assert_eq!(of("B2*C2"), "[.B2]*[.C2]");
		assert_eq!(of("SUM(D2:D3)"), "SUM([.D2:.D3])");
		assert_eq!(of("A1+B2*2"), "[.A1]+[.B2]*2");
		// A function name is followed by a bracket and is not a cell, however much it looks like one.
		assert_eq!(of("LOG10(A1)"), "LOG10([.A1])");
		// Text passes through untouched, or a quoted `A1` becomes a reference.
		assert_eq!(of("IF(A1>0,\"A1\",\"B2\")"), "IF([.A1]>0,\"A1\",\"B2\")");
		assert_eq!(of("2+2"), "2+2");
		// And the whole of it survives a trip through LibreOffice, which is what test 002 checks on a
		// file that has been through one.
		Ok(())
	}));

	res!(test_it(filter, &["A foreign .odp reads back in order 004", "all", "odf"], || {
		let r = res!(odf::slides::read(ODP));
		assert!(r.deck.slides.len() >= 2, "got {} slides", r.deck.slides.len());
		let titles: Vec<String> = r.deck.slides.iter()
			.map(|s| s.title.as_ref().map(|t| text_of(t)).unwrap_or_default())
			.collect();
		// The fixture is LibreOffice's re-save, which DROPS `presentation:class`. Where no frame
		// claims to be the title the first one is taken as it, which is what a person looking at the
		// slide sees -- so a re-saved deck still reads with its titles.
		assert_eq!(titles[0], "A Report On Something", "got {:?}", titles);
		assert!(r.deck.slides[1].text_of().contains("First bullet"));
		Ok(())
	}));

	res!(test_it(filter, &["What we write reads back as what we put in 005", "all", "odf"], || {
		let doc = res!(markdown::parse(SOURCE));
		let (odt, left) = res!(odf::text::write(&doc));
		assert!(left.is_empty());
		let back = res!(odf::text::read(&odt));
		let md = oxedyne_fe2o3_text::doc::markdown::write::render(&back.doc);
		for phrase in ["# A Report", "## The second heading", "**bold**", "- First bullet",
			"> A quotation.", "[link](https://example.com/page)", "    println!"] {
			assert!(md.contains(phrase), "the round trip lost {:?}\n{}", phrase, md);
		}
		let ods = res!(odf::sheet::write(&book()));
		let sback = res!(odf::sheet::read(&ods));
		let s = res!(sback.book.sheets.first().ok_or_else(|| err!("no sheet"; Missing)));
		assert_eq!(s.at(&res!(Ref::parse("C2"))).value, Value::Number(408.0));
		assert_eq!(s.at(&res!(Ref::parse("A4"))).value, Value::Date("2026-03-14".to_string()));
		// Written twice, the same bytes: nothing here comes from the clock.
		assert_eq!(ods, res!(odf::sheet::write(&book())));
		let (odp, _) = res!(odf::slides::write(&Deck::from_doc(&doc)));
		let dback = res!(odf::slides::read(&odp));
		assert_eq!(dback.deck.slides.len(), Deck::from_doc(&doc).slides.len());
		Ok(())
	}));

	res!(test_it(filter, &["A package that is not one is refused 006", "all", "odf"], || {
		assert!(odf::text::read(b"not a package").is_err());
		// A ZIP with no content.xml is not an OpenDocument file.
		let mut zip = Zip::new();
		zip.set("hello.txt", b"hi".to_vec(), Method::Store);
		let bytes = res!(zip.write());
		assert!(odf::text::read(&bytes).is_err());
		assert!(odf::sheet::read(&bytes).is_err());
		assert!(odf::slides::read(&bytes).is_err());
		// And a text document is not a spreadsheet, however well formed it is.
		let doc = res!(markdown::parse(SOURCE));
		let (odt, _) = res!(odf::text::write(&doc));
		assert!(odf::sheet::read(&odt).is_err(), "an .odt is not an .ods");
		Ok(())
	}));

	res!(test_it(filter, &["meta.xml carries no attribute the grammar forbids 010", "all", "odf"], || {
		// `office:mimetype` was on `office:document-meta` in all three writers, because all three go
		// through one `pkg::meta`. The OASIS grammar defines that attribute in `office-document-attrs`
		// and the ONLY element referring to those is `office:document`, the root of the flat
		// single-file form -- which has no `mimetype` member to carry the fact instead. A package does
		// have one, so the attribute is not merely forbidden here, it is redundant.
		let doc = res!(markdown::parse(SOURCE));
		let (odt, _) = res!(odf::text::write(&doc));
		let ods = res!(odf::sheet::write(&book()));
		let (odp, _) = res!(odf::slides::write(&Deck::from_doc(&doc)));
		for (what, bytes, media) in [
			("odt", &odt, odf::text::MEDIA),
			("ods", &ods, odf::sheet::MEDIA),
			("odp", &odp, odf::slides::MEDIA),
		] {
			let zip = res!(Zip::read(bytes.clone()));
			let meta = res!(zip.text("meta.xml"));
			assert!(!meta.contains("office:mimetype"),
				"{}: meta.xml still carries office:mimetype: {}", what, meta);
			// `office:version` is required on it and must not be lost with the other one.
			assert!(meta.contains("office:version"), "{}: meta.xml lost its version", what);
			// And the package still says what it is, in the member that is allowed to.
			assert_eq!(res!(zip.text("mimetype")), media,
				"{}: the package no longer declares its own type", what);
		}
		Ok(())
	}));

	res!(test_it(filter, &["styles.xml puts its children in the order required 011", "all", "odf"], || {
		// `office:document-styles` is a SEQUENCE, not a set: font-face-decls, then styles, then
		// automatic-styles, then master-styles. The presentation branch opened automatic-styles first,
		// so an `.odp` came out with two of them the wrong way round. `.odt` and `.ods` take neither
		// branch and were always in order, which is why only the deck was wrong -- so all three are
		// checked here rather than the one that broke.
		const ORDER: [&str; 4] = ["office:font-face-decls", "office:styles",
			"office:automatic-styles", "office:master-styles"];
		let doc = res!(markdown::parse(SOURCE));
		let (odt, _) = res!(odf::text::write(&doc));
		let ods = res!(odf::sheet::write(&book()));
		let (odp, _) = res!(odf::slides::write(&Deck::from_doc(&doc)));
		for (what, bytes) in [("odt", &odt), ("ods", &ods), ("odp", &odp)] {
			let zip = res!(Zip::read(bytes.clone()));
			let styles = res!(zip.text("styles.xml"));
			// Only the ones this document actually has, in the order it has them: the sequence permits
			// each to be absent, so a missing element is not a fault and an out-of-order pair is.
			let mut seen: Vec<(usize, &str)> = Vec::new();
			for name in ORDER {
				if let Some(at) = styles.find(&fmt!("<{}", name)) {
					seen.push((at, name));
				}
			}
			let mut sorted = seen.clone();
			sorted.sort_by_key(|(at, _)| *at);
			assert_eq!(sorted, seen,
				"{}: styles.xml has its children out of order -- found {:?}, the sequence wants {:?}",
				what, sorted.iter().map(|(_, n)| *n).collect::<Vec<_>>(),
				seen.iter().map(|(_, n)| *n).collect::<Vec<_>>());
		}
		// And the deck is the one that has to have all three, since its master page is the whole reason
		// the branch exists: a check that passed by finding only `office:styles` would prove nothing.
		let zip = res!(Zip::read(odp.clone()));
		let styles = res!(zip.text("styles.xml"));
		for name in ["office:styles", "office:automatic-styles", "office:master-styles"] {
			assert!(styles.contains(&fmt!("<{}", name)), "the .odp lost its {}", name);
		}
		Ok(())
	}));

	Ok(())
}
