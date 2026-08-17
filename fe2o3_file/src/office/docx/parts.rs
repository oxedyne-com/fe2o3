//! The two supporting parts a created `.docx` carries: what its styles are, and what its lists look
//! like.
//!
//! Both are generated rather than held as a blob of literal XML, because both are almost entirely
//! repetition -- six headings that differ by a size and an outline level, nine list levels that
//! differ by an indent -- and a literal blob is where a typo in level seven waits.
//!
//! # Styles are named, not drawn
//!
//! A heading is `<w:pStyle w:val="Heading1"/>` and never a bold run at 20 point. The difference shows
//! the first time somebody opens the document and uses the navigation pane, or generates a table of
//! contents, or applies their organisation's template: a named heading becomes their heading, and a
//! bold run stays a bold run forever.
//!
//! [Written entirely with AI](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use crate::office::docx::NS_W;
use oxedyne_fe2o3_text::xml::write::Out;

use oxedyne_fe2o3_core::prelude::*;

/// The point size, doubled as OOXML counts it, of each heading level.
const HEADING_SIZE: [&str; 6] = ["40", "32", "28", "24", "22", "22"];

// How many levels of list nesting are defined. Word's own lists define nine and so does this: a
// document that nested deeper than its numbering defines would lose its bullets at the bottom.
const LEVELS: usize = 9;

/// The bullet character and the font that draws it, at each of three levels, repeating.
const BULLETS: [(&str, &str); 3] = [
	("\u{F0B7}",	"Symbol"),
	("o",		"Courier New"),
	("\u{F0A7}",	"Wingdings"),
];

/// The styles part: what `Heading1`, `Quote` and the rest mean.
pub fn styles() -> Outcome<String> {
	let mut out = Out::declared();
	out.open("w:styles", &[("xmlns:w", NS_W)]);

	// What every paragraph and every run starts from.
	out.open("w:docDefaults", &[]);
	out.open("w:rPrDefault", &[]);
	out.open("w:rPr", &[]);
	out.empty("w:rFonts", &[("w:ascii", "Calibri"), ("w:hAnsi", "Calibri"), ("w:cs", "Calibri")]);
	out.empty("w:sz", &[("w:val", "22")]);
	out.empty("w:szCs", &[("w:val", "22")]);
	res!(out.close("w:rPr"));
	res!(out.close("w:rPrDefault"));
	out.open("w:pPrDefault", &[]);
	out.open("w:pPr", &[]);
	out.empty("w:spacing", &[("w:after", "160"), ("w:line", "259"), ("w:lineRule", "auto")]);
	res!(out.close("w:pPr"));
	res!(out.close("w:pPrDefault"));
	res!(out.close("w:docDefaults"));

	// Normal, which everything else is based on.
	out.open("w:style", &[("w:type", "paragraph"), ("w:default", "1"), ("w:styleId", "Normal")]);
	out.empty("w:name", &[("w:val", "Normal")]);
	out.empty("w:qFormat", &[]);
	res!(out.close("w:style"));

	// The six headings. `w:name` is the built-in name, lower case and spaced, which is what makes
	// Word treat these as ITS headings rather than as six styles that happen to be called that.
	for lvl in 1..=6usize {
		let id = fmt!("Heading{}", lvl);
		let name = fmt!("heading {}", lvl);
		let outline = fmt!("{}", lvl - 1);
		out.open("w:style", &[("w:type", "paragraph"), ("w:styleId", &id)]);
		out.empty("w:name", &[("w:val", &name)]);
		out.empty("w:basedOn", &[("w:val", "Normal")]);
		out.empty("w:next", &[("w:val", "Normal")]);
		out.empty("w:qFormat", &[]);
		out.open("w:pPr", &[]);
		out.empty("w:keepNext", &[]);
		out.empty("w:spacing", &[("w:before", "240"), ("w:after", "120")]);
		out.empty("w:outlineLvl", &[("w:val", &outline)]);
		res!(out.close("w:pPr"));
		out.open("w:rPr", &[]);
		out.empty("w:b", &[]);
		out.empty("w:sz", &[("w:val", HEADING_SIZE[lvl - 1])]);
		out.empty("w:szCs", &[("w:val", HEADING_SIZE[lvl - 1])]);
		res!(out.close("w:rPr"));
		res!(out.close("w:style"));
	}

	// A quotation: indented and italic, which is what a reader expects and what the tree says nothing
	// about. The tree carries that a passage IS a quotation; this is where that acquires a look.
	out.open("w:style", &[("w:type", "paragraph"), ("w:styleId", "Quote")]);
	out.empty("w:name", &[("w:val", "Quote")]);
	out.empty("w:basedOn", &[("w:val", "Normal")]);
	out.empty("w:next", &[("w:val", "Normal")]);
	out.empty("w:qFormat", &[]);
	out.open("w:pPr", &[]);
	out.empty("w:ind", &[("w:left", "720"), ("w:right", "720")]);
	res!(out.close("w:pPr"));
	out.open("w:rPr", &[]);
	out.empty("w:i", &[]);
	res!(out.close("w:rPr"));
	res!(out.close("w:style"));

	// A listing, which is scanned rather than read: monospaced, and without the spacing between
	// paragraphs that would put a gap between two lines of the same program.
	out.open("w:style", &[("w:type", "paragraph"), ("w:styleId", "SourceCode")]);
	out.empty("w:name", &[("w:val", "Source Code")]);
	out.empty("w:basedOn", &[("w:val", "Normal")]);
	out.empty("w:qFormat", &[]);
	out.open("w:pPr", &[]);
	out.empty("w:spacing", &[("w:after", "0"), ("w:line", "240"), ("w:lineRule", "auto")]);
	out.empty("w:contextualSpacing", &[]);
	res!(out.close("w:pPr"));
	out.open("w:rPr", &[]);
	out.empty("w:rFonts", &[("w:ascii", "Consolas"), ("w:hAnsi", "Consolas"), ("w:cs", "Consolas")]);
	out.empty("w:sz", &[("w:val", "20")]);
	res!(out.close("w:rPr"));
	res!(out.close("w:style"));

	// The style Word puts on every list item, and which its own list handling looks for.
	out.open("w:style", &[("w:type", "paragraph"), ("w:styleId", "ListParagraph")]);
	out.empty("w:name", &[("w:val", "List Paragraph")]);
	out.empty("w:basedOn", &[("w:val", "Normal")]);
	out.empty("w:qFormat", &[]);
	out.open("w:pPr", &[]);
	out.empty("w:spacing", &[("w:after", "0")]);
	out.empty("w:contextualSpacing", &[]);
	res!(out.close("w:pPr"));
	res!(out.close("w:style"));

	// A link, and a span of code within a line.
	out.open("w:style", &[("w:type", "character"), ("w:styleId", "Hyperlink")]);
	out.empty("w:name", &[("w:val", "Hyperlink")]);
	out.open("w:rPr", &[]);
	out.empty("w:color", &[("w:val", "0563C1")]);
	out.empty("w:u", &[("w:val", "single")]);
	res!(out.close("w:rPr"));
	res!(out.close("w:style"));

	out.open("w:style", &[("w:type", "character"), ("w:styleId", "InlineCode")]);
	out.empty("w:name", &[("w:val", "Inline Code")]);
	out.open("w:rPr", &[]);
	out.empty("w:rFonts", &[("w:ascii", "Consolas"), ("w:hAnsi", "Consolas"), ("w:cs", "Consolas")]);
	res!(out.close("w:rPr"));
	res!(out.close("w:style"));

	res!(out.close("w:styles"));
	out.finish()
}

/// The numbering part: one bulleted definition and one numbered one, nine levels each.
pub fn numbering() -> Outcome<String> {
	let mut out = Out::declared();
	out.open("w:numbering", &[("xmlns:w", NS_W)]);

	// Abstract zero: bullets.
	out.open("w:abstractNum", &[("w:abstractNumId", "0")]);
	out.empty("w:multiLevelType", &[("w:val", "hybridMultilevel")]);
	for lvl in 0..LEVELS {
		let (text, font) = BULLETS[lvl % BULLETS.len()];
		res!(level(&mut out, lvl, "bullet", text, Some(font)));
	}
	res!(out.close("w:abstractNum"));

	// Abstract one: numbers, each level counting on its own.
	out.open("w:abstractNum", &[("w:abstractNumId", "1")]);
	out.empty("w:multiLevelType", &[("w:val", "hybridMultilevel")]);
	for lvl in 0..LEVELS {
		let text = fmt!("%{}.", lvl + 1);
		res!(level(&mut out, lvl, "decimal", &text, None));
	}
	res!(out.close("w:abstractNum"));

	// The two the document refers to. A `w:numId` is what a paragraph names; the abstract definition
	// behind it is shared, which is how two lists can be the same shape and count separately.
	for (num, abstract_id) in [("1", "0"), ("2", "1")] {
		out.open("w:num", &[("w:numId", num)]);
		out.empty("w:abstractNumId", &[("w:val", abstract_id)]);
		res!(out.close("w:num"));
	}

	res!(out.close("w:numbering"));
	out.finish()
}

/// One level of a numbering definition.
fn level(
	out:	&mut Out,
	lvl:	usize,
	format:	&str,
	text:	&str,
	font:	Option<&str>,
)
	-> Outcome<()>
{
	let n = fmt!("{}", lvl);
	let left = fmt!("{}", 720 * (lvl + 1));
	out.open("w:lvl", &[("w:ilvl", &n)]);
	out.empty("w:start", &[("w:val", "1")]);
	out.empty("w:numFmt", &[("w:val", format)]);
	out.empty("w:lvlText", &[("w:val", text)]);
	out.empty("w:lvlJc", &[("w:val", "left")]);
	out.open("w:pPr", &[]);
	out.empty("w:ind", &[("w:left", &left), ("w:hanging", "360")]);
	res!(out.close("w:pPr"));
	if let Some(font) = font {
		out.open("w:rPr", &[]);
		out.empty("w:rFonts", &[("w:ascii", font), ("w:hAnsi", font), ("w:hint", "default")]);
		res!(out.close("w:rPr"));
	}
	res!(out.close("w:lvl"));
	Ok(())
}
