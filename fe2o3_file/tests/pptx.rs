use oxedyne_fe2o3_file::office::deck::{
	Deck,
	Slide,
};
use oxedyne_fe2o3_file::office::pptx;

use oxedyne_fe2o3_core::{
	prelude::*,
	test::test_it,
};
use oxedyne_fe2o3_text::doc::{
	markdown,
	text_of,
};

/// A `.pptx` LibreOffice wrote: it was handed a deck this crate made and asked to save its own, so
/// the content is known and every byte of the encoding -- and of the skeleton, which is where all the
/// risk lives -- is somebody else's.
///
/// Its slide relationship ids run `rId3`, `rId4`, `rId5`, which is the case that catches a reader
/// assuming the first relationship is the first slide.
const FOREIGN: &[u8] = include_bytes!("data/foreign.pptx");

/// Prose with three headings, nested bullets and a listing.
const SOURCE: &str = "\
# Quarterly Review

An opening line with **bold words** and *emphasis*.

- First finding
- Second finding
    - A nested point

## Numbers

1. Revenue rose
2. Costs held

```rust
fn main() {}
```

## Closing

A final line.
";

/// The parts a `.pptx` cannot open without: the chain a slide hangs from.
const REQUIRED: [&str; 7] = [
	"[Content_Types].xml",
	"_rels/.rels",
	"ppt/presentation.xml",
	"ppt/_rels/presentation.xml.rels",
	"ppt/slideMasters/slideMaster1.xml",
	"ppt/slideLayouts/slideLayout1.xml",
	"ppt/theme/theme1.xml",
];

pub fn test_pptx(filter: &'static str) -> Outcome<()> {

	res!(test_it(filter, &["Every heading starts a slide 000", "all", "pptx"], || {
		// This split at the SHALLOWEST heading level first, borrowing a rule from `Doc::top_heading`
		// that is right for finding a title and wrong for ending a slide: the document below came out
		// as ONE slide holding everything, and the author who wrote three headings expected three.
		let doc = res!(markdown::parse(SOURCE));
		let deck = Deck::from_doc(&doc);
		assert_eq!(deck.slides.len(), 3, "one slide per heading");
		let titles: Vec<String> = deck.slides.iter()
			.map(|s| s.title.as_ref().map(|t| text_of(t)).unwrap_or_default())
			.collect();
		assert_eq!(titles, vec!["Quarterly Review", "Numbers", "Closing"]);
		// The nesting of a list becomes the indent level of a bullet, and nothing else does.
		let levels: Vec<usize> = deck.slides[0].bullets.iter().map(|b| b.level).collect();
		assert_eq!(levels, vec![0, 0, 0, 1], "got {:?}", levels);
		// A listing goes on as its lines rather than being dropped: a deck generated from a
		// technical document is mostly listings.
		assert!(deck.slides[1].text_of().contains("fn main() {}"), "{}", deck.slides[1].text_of());
		Ok(())
	}));

	res!(test_it(filter, &["A created deck carries the chain a slide hangs from 001", "all", "pptx"], || {
		// The whole of the risk in this format. A slide points at a layout, a layout at a master, a
		// master at a theme, and PowerPoint refuses a file with any link missing -- with a repair
		// prompt that names neither the part nor the reason.
		let doc = res!(markdown::parse(SOURCE));
		let (bytes, left) = res!(pptx::write(&Deck::from_doc(&doc)));
		assert!(left.is_empty(), "this source holds no image and no notes");
		let zip = res!(oxedyne_fe2o3_file::zip::Zip::read(bytes.clone()));
		for part in REQUIRED {
			assert!(zip.has(part), "the package is missing {}", part);
		}
		for i in 1..=3 {
			assert!(zip.has(&fmt!("ppt/slides/slide{}.xml", i)));
			// Each slide names its layout in its OWN rels part. A slide with none is a slide with no
			// geometry to inherit.
			assert!(zip.has(&fmt!("ppt/slides/_rels/slide{}.xml.rels", i)));
		}
		// The theme must carry three of each style, whether or not anything uses them.
		let theme = res!(zip.text("ppt/theme/theme1.xml"));
		for (list, n) in [("a:fillStyleLst", 3), ("a:lnStyleLst", 3), ("a:effectStyleLst", 3)] {
			let xml = res!(oxedyne_fe2o3_text::xml::Xml::parse(&theme));
			let found = res!(xml.root()).all(list);
			assert_eq!(found.len(), 1, "{} appears once", list);
			assert!(found[0].elems().count() >= n, "{} needs {} entries", list, n);
		}
		// And the master must map scheme colours to roles, or the deck comes out white on white.
		let master = res!(zip.text("ppt/slideMasters/slideMaster1.xml"));
		assert!(master.contains("p:clrMap"), "the colour map is written explicitly");
		// Written twice, the same bytes.
		let (again, _) = res!(pptx::write(&Deck::from_doc(&doc)));
		assert_eq!(bytes, again);
		// And it survives the archive's round trip.
		assert_eq!(res!(zip.write()), bytes);
		Ok(())
	}));

	res!(test_it(filter, &["A foreign deck reads back in the right order 002", "all", "pptx"], || {
		// Slide ORDER lives in `p:sldIdLst`, not in the file names: `slide10.xml` sorts before
		// `slide2.xml` and is the ninth slide. This fixture's slide relationships start at `rId3`,
		// because the master and the theme took the first two, so a reader that assumed the first
		// relationship was the first slide deals the whole deck wrong.
		let r = res!(pptx::read(FOREIGN));
		assert_eq!(r.deck.slides.len(), 3);
		let titles: Vec<String> = r.deck.slides.iter()
			.map(|s| s.title.as_ref().map(|t| text_of(t)).unwrap_or_default())
			.collect();
		assert_eq!(titles, vec!["Quarterly Review", "Numbers", "Closing"], "got {:?}", titles);
		// A title is a title because of the PLACEHOLDER TYPE on its shape, not because of where it
		// sits or how large it is.
		assert!(r.deck.slides[0].title.is_some());
		let first = r.deck.slides[0].text_of();
		assert!(first.contains("An opening line"), "{}", first);
		assert!(first.contains("A nested point"), "{}", first);
		assert!(r.deck.slides[1].text_of().contains("Revenue rose"));
		assert!(!r.macros);
		assert!(r.missing.is_empty());
		Ok(())
	}));

	res!(test_it(filter, &["A deck we wrote reads back as what we put in 003", "all", "pptx"], || {
		let doc = res!(markdown::parse(SOURCE));
		let deck = Deck::from_doc(&doc);
		let (bytes, _) = res!(pptx::write(&deck));
		let back = res!(pptx::read(&bytes));
		assert_eq!(back.deck.slides.len(), deck.slides.len());
		for (a, b) in deck.slides.iter().zip(&back.deck.slides) {
			assert_eq!(text_of(res!(a.title.as_ref().ok_or_else(|| err!("no title"; Missing)))),
				text_of(res!(b.title.as_ref().ok_or_else(|| err!("no title back"; Missing)))));
			assert_eq!(a.bullets.len(), b.bullets.len(), "slide bullet count");
			let levels_a: Vec<usize> = a.bullets.iter().map(|x| x.level).collect();
			let levels_b: Vec<usize> = b.bullets.iter().map(|x| x.level).collect();
			assert_eq!(levels_a, levels_b, "the indent levels survive");
		}
		Ok(())
	}));

	res!(test_it(filter, &["What a deck could not carry is counted 004", "all", "pptx"], || {
		// Speaker's notes need a notes master and a notes layout -- the whole skeleton again, for
		// content nobody sees on screen. Counted and said rather than written or silently dropped.
		let mut deck = Deck::new();
		let mut s = Slide::titled("With notes");
		s.notes = Some("Remember to pause here.".to_string());
		deck.slides.push(s);
		let (_, left) = res!(pptx::write(&deck));
		assert_eq!(left.notes, 1);
		assert!(!left.is_empty());
		// A deck with no slides gets one empty slide: a presentation holding none is a file that
		// opens and shows nothing, which reads as corruption.
		let (bytes, _) = res!(pptx::write(&Deck::new()));
		let back = res!(pptx::read(&bytes));
		assert_eq!(back.deck.slides.len(), 1);
		Ok(())
	}));

	res!(test_it(filter, &["A deck that cannot be read is named 005", "all", "pptx"], || {
		let ole = [0xD0u8, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1, 0, 0, 0, 0, 0, 0, 0, 0];
		match pptx::read(&ole) {
			Err(e)	=> {
				let s = fmt!("{}", e);
				assert!(s.contains("OLE compound file"), "{}", s);
			}
			Ok(_)	=> panic!("an OLE file was read"),
		}
		assert!(pptx::read(b"not a deck").is_err());
		let mut zip = oxedyne_fe2o3_file::zip::Zip::new();
		zip.set("hello.txt", b"hi".to_vec(), oxedyne_fe2o3_file::zip::Method::Store);
		assert!(pptx::read(&res!(zip.write())).is_err());
		Ok(())
	}));

	Ok(())
}
