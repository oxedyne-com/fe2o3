use oxedyne_fe2o3_file::office::docx;
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
use oxedyne_fe2o3_text::doc::markdown;
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

	Ok(())
}
