use oxedyne_fe2o3_file::office::docx;
use oxedyne_fe2o3_file::office::docx::read::Undrawable;
use oxedyne_fe2o3_file::office::opc::{
	CT_DOCUMENT,
	REL_DOC,
	REL_HYPERLINK,
};
use oxedyne_fe2o3_file::zip::Zip;

use oxedyne_fe2o3_core::{
	prelude::*,
	test::test_it,
};
use oxedyne_fe2o3_text::doc::{
	Block,
	markdown,
};
use oxedyne_fe2o3_text::xml::Xml;

/// Prose exercising every block the tree has, so a created document is checked against something with
/// more in it than paragraphs.
const SOURCE: &str = "\
# A Report On Something

An opening paragraph with **bold words**, some *emphasis*, and `inline code`.

## The second heading

- First bullet
- Second with a [link](https://example.com/page)
    - A nested bullet

1. Step one
2. Step two

> A quotation.

| Name | Quantity | Price |
| :--- | ---: | :---: |
| Widget | 12 | 3.40 |

```rust
fn main() {}
```

---

A closing paragraph.
";

/// A `.docx` LibreOffice wrote from an HTML document with known content: two heading levels, a
/// bulleted list, a numbered list, a quotation, a table with a bold first row, a link, and bold and
/// italic runs.
///
/// The intent is ours and the bytes are somebody else's, which is the only useful shape for a reader
/// test. It has already earned its place: LibreOffice gives its heading 1 style NO outline level and
/// calls its quotation style `BlockQuotation`, and a reader written against Word's spellings alone
/// gets both wrong.
const RICH: &[u8] = include_bytes!("data/rich.docx");

/// A `.docx` LibreOffice wrote holding a picture, which is the one thing in this fixture set that a
/// reading view genuinely cannot draw. The band that says so is the whole point of it.
const WITHPIC: &[u8] = include_bytes!("data/withpic.docx");

/// The parts a `.docx` cannot open without.
const REQUIRED: [&str; 4] = [
	"[Content_Types].xml",
	"_rels/.rels",
	"word/document.xml",
	"word/_rels/document.xml.rels",
];

/// Builds the document under test, and its archive.
fn made() -> Outcome<(Vec<u8>, Zip)> {
	let doc = res!(markdown::parse(SOURCE));
	let (bytes, left) = res!(docx::write(&doc));
	assert!(left.is_empty(), "this source holds no image, so nothing was left out");
	let zip = res!(Zip::read(bytes.clone()));
	Ok((bytes, zip))
}

pub fn test_office(filter: &'static str) -> Outcome<()> {

	res!(test_it(filter, &["A created .docx is a package, not a heap of XML 000", "all", "office"], || {
		// The four parts and the two declarations. A document missing any of them is one Word declines
		// to open, with a message naming neither the part nor the reason.
		let (_, zip) = res!(made());
		for part in REQUIRED {
			assert!(zip.has(part), "the package is missing {}", part);
		}
		let types = res!(zip.text("[Content_Types].xml"));
		assert!(types.contains(CT_DOCUMENT), "the main part's content type is declared");
		assert!(types.contains("Extension=\"rels\""), "the relationship parts are declared");
		// The package's own rels must point at the document part, which is how a reader finds it.
		let root = res!(Xml::parse(&res!(zip.text("_rels/.rels"))));
		let rels = res!(root.root()).children("Relationship");
		assert_eq!(rels.len(), 1);
		assert_eq!(rels[0].attr("Type"), Some(REL_DOC));
		assert_eq!(rels[0].attr("Target"), Some("word/document.xml"));
		// And everything the package declares an override for must actually be in the archive: a
		// content type for a part that is not there is how a package passes a check on its own
		// manifest and fails in Word.
		let types = res!(Xml::parse(&types));
		for over in res!(types.root()).children("Override") {
			let part = res!(over.attr("PartName").ok_or_else(|| err!(
				"an override with no PartName"; Invalid, Input, Missing)));
			let name = part.trim_start_matches('/');
			assert!(zip.has(name), "'{}' is declared and is not in the archive", name);
		}
		Ok(())
	}));

	res!(test_it(filter, &["The document says what the tree said 001", "all", "office"], || {
		let (_, zip) = res!(made());
		let xml = res!(Xml::parse(&res!(zip.text("word/document.xml"))));
		let root = res!(xml.root());
		assert_eq!(root.name.qname, "w:document");
		let body = res!(root.child("w:body").ok_or_else(|| err!("no w:body"; Missing)));

		// A heading is Word's OWN heading, by style, so its navigation pane and a generated contents
		// page both find it. A bold run at 20 point would look the same and be neither.
		let styles: Vec<String> = xml.all("w:pStyle").iter()
			.filter_map(|e| e.attr("w:val").map(|s| s.to_string()))
			.collect();
		assert!(styles.contains(&"Heading1".to_string()), "got {:?}", styles);
		assert!(styles.contains(&"Heading2".to_string()));
		assert!(styles.contains(&"Quote".to_string()), "a quotation is a Quote");
		assert!(styles.contains(&"SourceCode".to_string()), "a listing is monospaced by style");

		// The section properties are last in the body, as the schema requires.
		let last = res!(body.elems().last().ok_or_else(|| err!("an empty body"; Missing)));
		assert_eq!(last.name.qname, "w:sectPr");

		// Lists are Word's lists: a numbering reference, not a bullet character typed into the text.
		let nums: Vec<(&str, &str)> = xml.all("w:numPr").iter()
			.filter_map(|e| {
				let lvl = e.child("w:ilvl")?.attr("w:val")?;
				let id = e.child("w:numId")?.attr("w:val")?;
				Some((id, lvl))
			})
			.collect();
		assert!(nums.contains(&("1", "0")), "a bullet at the top level: {:?}", nums);
		assert!(nums.contains(&("1", "1")), "a nested bullet is one level deeper: {:?}", nums);
		assert!(nums.contains(&("2", "0")), "a numbered list is the other definition: {:?}", nums);

		// And the prose is all there, in order.
		let text: String = xml.all("w:t").iter().map(|e| xml.text_of(e)).collect();
		for phrase in [
			"A Report On Something",
			"An opening paragraph with ",
			"bold words",
			"inline code",
			"First bullet",
			"A quotation.",
			"fn main() {}",
			"A closing paragraph.",
		] {
			assert!(text.contains(phrase), "the document lost {:?}", phrase);
		}
		// The spaces between runs survive, which is what `xml:space="preserve"` is for: a sentence
		// built from three runs otherwise arrives with the words jammed together.
		assert!(text.contains("with bold words, some emphasis"), "got {:?}", text);
		Ok(())
	}));

	res!(test_it(filter, &["A link's target lives in the rels and resolves 002", "all", "office"], || {
		// The one place a created document refers across parts, and so the one place it can be
		// internally inconsistent while every part is well formed on its own.
		let (_, zip) = res!(made());
		let xml = res!(Xml::parse(&res!(zip.text("word/document.xml"))));
		let rels = res!(Xml::parse(&res!(zip.text("word/_rels/document.xml.rels"))));
		let links = xml.all("w:hyperlink");
		assert_eq!(links.len(), 1);
		let id = res!(links[0].attr("r:id").ok_or_else(|| err!(
			"the hyperlink names no relationship"; Invalid, Missing)));
		let rel = res!(res!(rels.root()).children("Relationship").into_iter()
			.find(|r| r.attr("Id") == Some(id))
			.ok_or_else(|| err!("'{}' is named and not declared", id; Invalid, Missing)));
		assert_eq!(rel.attr("Type"), Some(REL_HYPERLINK));
		assert_eq!(rel.attr("Target"), Some("https://example.com/page"));
		assert_eq!(rel.attr("TargetMode"), Some("External"),
			"a URL outside the package must say so, or Word reads it as a part name");
		// And every id the body names is declared: an r:id with nothing behind it is a document Word
		// refuses to open.
		let declared: Vec<&str> = res!(rels.root()).children("Relationship").iter()
			.filter_map(|r| r.attr("Id"))
			.collect();
		for link in &links {
			let id = res!(link.attr("r:id").ok_or_else(|| err!("a link with no id"; Missing)));
			assert!(declared.contains(&id), "'{}' is named and not declared", id);
		}
		Ok(())
	}));

	res!(test_it(filter, &["A table becomes a table 003", "all", "office"], || {
		let (_, zip) = res!(made());
		let xml = res!(Xml::parse(&res!(zip.text("word/document.xml"))));
		let tbls = xml.all("w:tbl");
		assert_eq!(tbls.len(), 1);
		let tbl = tbls[0];
		assert_eq!(tbl.all("w:gridCol").len(), 3, "three columns are declared in the grid");
		let rows = tbl.children("w:tr");
		assert_eq!(rows.len(), 2, "a header row and one body row");
		assert_eq!(rows[0].children("w:tc").len(), 3);
		assert!(rows[0].find(&["w:trPr", "w:tblHeader"]).is_some(),
			"the header row repeats onto each page it runs on");
		// The tree names the sides Start and End because it does not know which way its text runs, and
		// OOXML has the same two words for the same reason, so nothing here decides what "left" means.
		let jc: Vec<&str> = tbl.all("w:jc").iter().filter_map(|e| e.attr("w:val")).collect();
		assert!(jc.contains(&"start"), "got {:?}", jc);
		assert!(jc.contains(&"end"), "got {:?}", jc);
		assert!(jc.contains(&"center"), "got {:?}", jc);
		Ok(())
	}));

	res!(test_it(filter, &["What could not be carried is said 004", "all", "office"], || {
		// An image's bytes are somewhere this crate cannot reach: the tree holds where an image IS, and
		// there is no filesystem here and no network. The alt text stands in its place and the omission
		// is COUNTED, so a caller can tell the reader rather than the reader having to notice.
		let doc = res!(markdown::parse("Before ![a diagram](pics/one.png) after.\n"));
		let (bytes, left) = res!(docx::write(&doc));
		assert_eq!(left.images, vec!["pics/one.png".to_string()]);
		assert!(!left.is_empty());
		let zip = res!(Zip::read(bytes));
		let xml = res!(Xml::parse(&res!(zip.text("word/document.xml"))));
		let text: String = xml.all("w:t").iter().map(|e| xml.text_of(e)).collect();
		assert_eq!(text, "Before a diagram after.", "the alt text stands in for the image");
		Ok(())
	}));

	res!(test_it(filter, &["The same tree gives the same bytes 005", "all", "office"], || {
		// Nothing in a created document comes from the clock or from a counter that survives a call, so
		// a build that has to be reproducible stays reproducible.
		let doc = res!(markdown::parse(SOURCE));
		let (a, _) = res!(docx::write(&doc));
		let (b, _) = res!(docx::write(&doc));
		assert_eq!(a, b);
		// And it survives the archive's own round trip, which is the property an edit will rest on.
		let zip = res!(Zip::read(a.clone()));
		assert_eq!(res!(zip.write()), a);
		Ok(())
	}));


	res!(test_it(filter, &["A foreign document reads back as what it says 006", "all", "office"], || {
		// Read against a document this crate did not write. Reading back our own output would prove
		// that the writer and the reader share their assumptions, which is exactly the thing worth
		// doubting.
		let r = res!(docx::read(RICH));
		let blocks = &r.doc.blocks;

		// Headings, by what the style RESOLVES to. LibreOffice gives `Heading1` no outline level at
		// all, so a reader that asked for one would find no title in this document.
		let heads: Vec<(u8, String)> = blocks.iter().filter_map(|b| match b {
			Block::Heading { level, content }	=> {
				Some((*level, oxedyne_fe2o3_text::doc::text_of(content)))
			}
			_					=> None,
		}).collect();
		assert_eq!(heads, vec![
			(1, "Quarterly Review".to_string()),
			(2, "Findings".to_string()),
		], "got {:?}", heads);

		// Both lists, and which is which. That answer comes from word/numbering.xml through two hops:
		// a paragraph names a w:num, which names an abstract definition, which holds the format.
		let lists: Vec<(bool, usize)> = blocks.iter().filter_map(|b| match b {
			Block::List { ordered, items }	=> Some((*ordered, items.len())),
			_				=> None,
		}).collect();
		assert_eq!(lists, vec![(false, 2), (true, 2)], "got {:?}", lists);

		// A quotation, which LibreOffice calls `BlockQuotation` and Word calls `Quote`.
		assert!(blocks.iter().any(|b| matches!(b, Block::Quote(_))), "the quotation is a quotation");

		// The table, with its bold first row read as the header.
		let table = res!(blocks.iter().find_map(|b| match b {
			Block::Table { head, rows, .. }	=> Some((head, rows)),
			_				=> None,
		}).ok_or_else(|| err!("no table"; Missing)));
		let head = res!(table.0.as_ref().ok_or_else(|| err!("the table lost its header"; Missing)));
		assert_eq!(head.text_of(), "Region Units");
		assert_eq!(table.1.len(), 2);
		assert_eq!(table.1[0].text_of(), "North 120");

		// The link, and its target out of the relationships part.
		let text = markdown::write::render(&r.doc);
		assert!(text.contains("[a link](https://example.org/detail)"), "got {}", text);
		// Bold and italic survive as emphasis rather than as font sizes.
		assert!(text.contains("**bold words**"), "got {}", text);
		assert!(text.contains("*italic ones*"), "got {}", text);

		// Nothing in this document is undrawable, and it carries no macros.
		assert!(r.undrawn.is_empty(), "got {:?}", r.undrawn);
		assert!(r.say_undrawn().is_none());
		assert!(!r.macros);
		Ok(())
	}));

	res!(test_it(filter, &["A picture in a foreign document is counted 012", "all", "office"], || {
		// The counting, on a real document rather than on a hand-made `Reading`. LibreOffice writes a
		// picture as `w:drawing`, and the prose around it has to survive it: a reader that stopped at
		// the drawing would lose the rest of the document and report one image.
		let r = res!(docx::read(WITHPIC));
		assert_eq!(r.undrawn, vec![(Undrawable::Image, 1)], "got {:?}", r.undrawn);
		assert_eq!(res!(r.say_undrawn().ok_or_else(|| err!("no phrase"; Missing))),
			"1 thing is not drawn: 1 image");
		let md = markdown::write::render(&r.doc);
		assert!(md.contains("# Report With A Picture"), "got {}", md);
		assert!(md.contains("Text before the picture."), "got {}", md);
		assert!(md.contains("Text after the picture."), "the prose after the drawing survived: {}", md);
		Ok(())
	}));

	res!(test_it(filter, &["What cannot be drawn is counted by kind 007", "all", "office"], || {
		// The band in the panel header says "4 things are not drawn: 3 text boxes, 1 chart". The number
		// and the kind are both the information: a reader told only that something is missing has been
		// told that the reader cannot be trusted, and nothing else.
		assert_eq!(Undrawable::TextBox.say(1), "1 text box");
		assert_eq!(Undrawable::TextBox.say(3), "3 text boxes");
		assert_eq!(Undrawable::Chart.say(1), "1 chart");
		let mut r = docx::read::Reading::default();
		assert!(r.say_undrawn().is_none(), "a document with everything drawn says nothing");
		r.undrawn = vec![(Undrawable::Image, 2), (Undrawable::Chart, 1)];
		assert_eq!(res!(r.say_undrawn().ok_or_else(|| err!("no phrase"; Missing))),
			"3 things are not drawn: 2 images, 1 chart");
		r.undrawn = vec![(Undrawable::Chart, 1)];
		assert_eq!(res!(r.say_undrawn().ok_or_else(|| err!("no phrase"; Missing))),
			"1 thing is not drawn: 1 chart");
		Ok(())
	}));

	res!(test_it(filter, &["A document we wrote reads back as what we put in 008", "all", "office"], || {
		// Weaker evidence than 006 and worth having for a different reason: it is the ROUND TRIP, and
		// it covers the constructs the foreign fixture has none of -- a listing, a thematic break, a
		// code span. It proves the writer and reader agree, and nothing about either being right.
		let doc = res!(markdown::parse(SOURCE));
		let (bytes, _) = res!(docx::write(&doc));
		let back = res!(docx::read(&bytes));
		let md = markdown::write::render(&back.doc);
		for phrase in [
			"# A Report On Something",
			"## The second heading",
			"**bold words**",
			"- First bullet",
			"1. Step one",
			"> A quotation.",
			"[link](https://example.com/page)",
			"| Widget | 12 | 3.40 |",
			"fn main() {}",
		] {
			assert!(md.contains(phrase), "the round trip lost {:?}\n---\n{}", phrase, md);
		}
		Ok(())
	}));

	res!(test_it(filter, &["An encrypted document is refused by name 009", "all", "office"], || {
		// An encrypted Office file is an OLE compound file with the real document inside it. There is
		// no password here, so it is refused rather than shown as the binary rubble it decodes to.
		let ole = [
			0xD0u8, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1,
			0, 0, 0, 0, 0, 0, 0, 0,
		];
		let e = docx::read(&ole);
		assert!(e.is_err());
		// And something that is not a document at all is refused too, rather than read as empty.
		assert!(docx::read(b"not a document").is_err());
		Ok(())
	}));

	res!(test_it(filter, &["A macro project is said rather than run 010", "all", "office"], || {
		// Never executed, never stripped: the binary travels with the file and the reader is TOLD.
		// A reader who is not told does not know what they have been sent.
		let doc = res!(markdown::parse("A document.\n"));
		let (bytes, _) = res!(docx::write(&doc));
		let mut zip = res!(Zip::read(bytes));
		assert!(!res!(docx::read(&res!(zip.write()))).macros);
		zip.set("word/vbaProject.bin", vec![0xCA, 0xFE], oxedyne_fe2o3_file::zip::Method::Store);
		let with = res!(zip.write());
		assert!(res!(docx::read(&with)).macros, "the macro project is seen");
		// And it is still there afterwards, byte for byte.
		let back = res!(Zip::read(with));
		assert_eq!(res!(back.content("word/vbaProject.bin")), vec![0xCA, 0xFE]);
		Ok(())
	}));

	res!(test_it(filter, &["Tracked changes are displayed as the document stands 011", "all", "office"], || {
		// An insertion's text IS in the document, so it is read. A deletion's is not, so it is not.
		// Display only: nothing here authors w:ins or w:del.
		let doc = res!(markdown::parse("Placeholder.\n"));
		let (bytes, _) = res!(docx::write(&doc));
		let mut zip = res!(Zip::read(bytes));
		let body = "<w:p><w:ins><w:r><w:t>kept </w:t></w:r></w:ins>\
			<w:del><w:r><w:delText>gone </w:delText></w:r></w:del>\
			<w:r><w:t>plain</w:t></w:r></w:p>";
		let part = fmt!(
			"<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
			<w:document xmlns:w=\"{}\"><w:body>{}</w:body></w:document>",
			docx::NS_W, body);
		zip.set("word/document.xml", part.into_bytes(), oxedyne_fe2o3_file::zip::Method::Deflate);
		let r = res!(docx::read(&res!(zip.write())));
		let text = markdown::write::render(&r.doc);
		assert!(text.contains("kept plain"), "got {:?}", text);
		assert!(!text.contains("gone"), "removed text is not what the document says: {:?}", text);
		assert_eq!(r.tracked, 1, "and the insertion is counted, so a reader can be told");
		Ok(())
	}));

	Ok(())
}
