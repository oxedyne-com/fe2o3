//! `.odp`: an OpenDocument presentation, written from the neutral deck and read back into it.
//!
//! # A slide is a page of frames
//!
//! `<draw:page>` holds `<draw:frame presentation:class="title">` and
//! `<draw:frame presentation:class="outline">`, and what makes a frame the title is that attribute
//! rather than its position. The correspondence with PresentationML is close enough that the reader
//! here and the one in [`crate::office::pptx`] answer the same questions of a different vocabulary.
//!
//! # A known limitation, measured rather than assumed
//!
//! The frames written here carry `presentation:class`, and a reader that re-saves the file **drops
//! it**: LibreOffice turns them back into plain drawing boxes. The words, their order, their nesting
//! and their positions all survive, and the deck renders correctly to PDF; what is lost is the
//! outline view, which is the thing that knows a title from a bullet.
//!
//! Making them true placeholders needs the master page to carry the placeholder shapes and each
//! frame to point at one, which is the PresentationML skeleton problem in another vocabulary. It is
//! not done, it is said, and [`read`] is built so it does not matter to anything reading a deck:
//! where no frame claims to be the title, the first one is taken as it, which is what a person
//! looking at the slide sees anyway.
//!
//! # Bullet depth is nesting, not an attribute
//!
//! PresentationML writes `<a:pPr lvl="2">` on a paragraph. OpenDocument nests a `text:list` inside a
//! `text:list-item` twice. The neutral deck carries a level either way, so the difference is confined
//! to these two files -- which is what the neutral model is for.

use crate::office::deck::{
	Bullet,
	Deck,
	MAX_LEVEL,
	Slide,
};
use crate::office::odf::{
	NS_DRAW,
	NS_FO,
	NS_OFFICE,
	NS_PRES,
	NS_STYLE,
	NS_SVG,
	NS_TEXT,
	NS_XLINK,
	pkg,
};
use crate::zip::{
	Method,
	Zip,
};

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_text::doc::Inline;
use oxedyne_fe2o3_text::xml::{
	Elem,
	Xml,
};
use oxedyne_fe2o3_text::xml::write::Out;

/// The media type an `.odp` declares in its first member.
pub const MEDIA: &str = "application/vnd.oasis.opendocument.presentation";

/// The most a single part is inflated to.
pub const MAX_PART: u64 = 64 * 1024 * 1024;

/// The width of a slide, as OpenDocument writes a length: with its unit.
const SLIDE_W: &str = "28cm";
/// The height of a slide.
const SLIDE_H: &str = "15.75cm";

/// What a created deck could not carry.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Left {
	/// The images whose bytes could not be reached.
	pub images:	Vec<String>,
	/// How many slides carried speaker's notes that were not written.
	pub notes:	usize,
}

impl Left {

	/// Whether everything in the deck reached the file.
	pub fn is_empty(&self) -> bool {
		self.images.is_empty() && self.notes == 0
	}
}

/// Writes a deck as the bytes of an `.odp`.
pub fn write(deck: &Deck) -> Outcome<(Vec<u8>, Left)> {
	let mut owned;
	let deck = match deck.slides.is_empty() {
		false	=> deck,
		true	=> {
			owned = Deck::new();
			owned.slides.push(Slide::default());
			&owned
		}
	};
	let mut left = Left::default();
	let mut out = Out::declared();
	out.open("office:document-content", &[
		("xmlns:office", NS_OFFICE),
		("xmlns:text", NS_TEXT),
		("xmlns:draw", NS_DRAW),
		("xmlns:presentation", NS_PRES),
		("xmlns:style", NS_STYLE),
		("xmlns:svg", NS_SVG),
		("xmlns:fo", NS_FO),
		("xmlns:xlink", NS_XLINK),
		("office:version", pkg::VERSION),
	]);
	// The one bulleted list style every outline frame refers to.
	out.open("office:automatic-styles", &[]);
	out.open("text:list-style", &[("style:name", "LB")]);
	for lvl in 1..=9 {
		out.open("text:list-level-style-bullet", &[
			("text:level", &fmt!("{}", lvl)),
			("text:bullet-char", "\u{2022}"),
		]);
		res!(out.close("text:list-level-style-bullet"));
	}
	res!(out.close("text:list-style"));
	res!(out.close("office:automatic-styles"));
	out.open("office:body", &[]);
	out.open("office:presentation", &[]);
	for (i, slide) in deck.slides.iter().enumerate() {
		if slide.notes.is_some() {
			left.notes += 1;
		}
		out.open("draw:page", &[
			("draw:name", &fmt!("page{}", i + 1)),
			("draw:master-page-name", "Default"),
		]);
		if let Some(title) = &slide.title {
			out.open("draw:frame", &[
				("presentation:class", "title"),
				("svg:x", "1cm"), ("svg:y", "1cm"),
				("svg:width", "26cm"), ("svg:height", "2.5cm"),
			]);
			out.open("draw:text-box", &[]);
			out.open("text:p", &[]);
			res!(inlines(&mut out, title, &mut left));
			res!(out.close("text:p"));
			res!(out.close("draw:text-box"));
			res!(out.close("draw:frame"));
		}
		if !slide.bullets.is_empty() {
			out.open("draw:frame", &[
				("presentation:class", "outline"),
				("svg:x", "1cm"), ("svg:y", "4cm"),
				("svg:width", "26cm"), ("svg:height", "10cm"),
			]);
			out.open("draw:text-box", &[]);
			res!(bullets(&mut out, &slide.bullets, &mut left));
			res!(out.close("draw:text-box"));
			res!(out.close("draw:frame"));
		}
		res!(out.close("draw:page"));
	}
	res!(out.close("office:presentation"));
	res!(out.close("office:body"));
	res!(out.close("office:document-content"));

	let mut zip = pkg::start(MEDIA);
	zip.set("content.xml", res!(out.finish()).into_bytes(), Method::Deflate);
	zip.set("styles.xml", res!(pkg::styles_for(MEDIA)).into_bytes(), Method::Deflate);
	zip.set("meta.xml", res!(pkg::meta(MEDIA)).into_bytes(), Method::Deflate);
	res!(pkg::finish(&mut zip, MEDIA));
	let _ = (SLIDE_W, SLIDE_H);
	Ok((res!(zip.write()), left))
}

/// Writes a slide's bullets, nesting a list inside an item for each level of depth.
fn bullets(out: &mut Out, items: &[Bullet], left: &mut Left) -> Outcome<()> {
	res!(at_level(out, items, 0, &mut 0, left));
	Ok(())
}

/// Writes the run of bullets at a level, descending where the next one is deeper.
///
/// The index is carried by reference because a nested call consumes items the caller must not write
/// again -- which is the whole difficulty of turning a flat list of levels back into a tree.
fn at_level(
	out:	&mut Out,
	items:	&[Bullet],
	level:	usize,
	i:	&mut usize,
	left:	&mut Left,
)
	-> Outcome<()>
{
	match level {
		0	=> out.open("text:list", &[("text:style-name", "LB")]),
		_	=> out.open("text:list", &[]),
	}
	while *i < items.len() {
		let b = &items[*i];
		let bl = b.level.min(MAX_LEVEL);
		if bl < level {
			break;
		}
		if bl > level {
			// A deeper item belongs inside the one before it. An item to hang it from is opened
			// where there was none, so a list that starts deep is still a list.
			out.open("text:list-item", &[]);
			res!(at_level(out, items, level + 1, i, left));
			res!(out.close("text:list-item"));
			continue;
		}
		*i += 1;
		out.open("text:list-item", &[]);
		out.open("text:p", &[]);
		res!(inlines(out, &b.content, left));
		res!(out.close("text:p"));
		// Everything deeper that follows goes inside this item.
		if items.get(*i).map(|n| n.level.min(MAX_LEVEL) > level).unwrap_or(false) {
			res!(at_level(out, items, level + 1, i, left));
		}
		res!(out.close("text:list-item"));
	}
	res!(out.close("text:list"));
	Ok(())
}

/// Writes a run of inline content.
fn inlines(out: &mut Out, content: &[Inline], left: &mut Left) -> Outcome<()> {
	for item in content {
		match item {
			Inline::Text(t)			=> out.text(t),
			Inline::Code(t)			=> {
				out.open("text:span", &[("text:style-name", "Source_20_Text")]);
				out.text(t);
				res!(out.close("text:span"));
			}
			Inline::Emph { strong, content }	=> {
				let style = match strong {
					true	=> "Strong_20_Emphasis",
					false	=> "Emphasis",
				};
				out.open("text:span", &[("text:style-name", style)]);
				res!(inlines(out, content, left));
				res!(out.close("text:span"));
			}
			Inline::Link { to, content }		=> {
				out.open("text:a", &[("xlink:href", to), ("xlink:type", "simple")]);
				res!(inlines(out, content, left));
				res!(out.close("text:a"));
			}
			Inline::Image { src, alt }		=> {
				left.images.push(src.clone());
				out.text(alt);
			}
			Inline::Span { content, .. }		=> res!(inlines(out, content, left)),
			Inline::Break				=> out.empty("text:line-break", &[]),
		}
	}
	Ok(())
}

/// A deck read for reading.
#[derive(Clone, Debug, Default)]
pub struct Reading {
	/// The slides and their words.
	pub deck:	Deck,
	/// Whether the file carries a macro project. Said, never run.
	pub macros:	bool,
	/// How many pictures the deck holds, which this does not draw.
	pub images:	usize,
}

/// Reads an `.odp` into the deck it holds.
pub fn read(bytes: &[u8]) -> Outcome<Reading> {
	let zip = res!(Zip::read(bytes.to_vec()));
	let mut out = Reading::default();
	out.macros = zip.names().iter().any(|n| n.starts_with("Basic/"));
	let src = res!(String::from_utf8(res!(zip.content_capped("content.xml", MAX_PART))),
		Decode, String);
	let xml = res!(Xml::parse(&src));
	let body = res!(res!(xml.root()).find(&["office:body", "office:presentation"])
		.ok_or_else(|| err!(
			"This package has no <office:presentation>, so it is not a deck.";
			Invalid, Input, Missing)));
	// The pages are in document order here, which is the one thing this format makes easier than
	// PresentationML -- there is no id list to go through and so no order to get wrong.
	for page in body.children("draw:page") {
		out.deck.slides.push(slide_of(&xml, page, &mut out.images));
	}
	Ok(out)
}

/// One slide.
fn slide_of(xml: &Xml, page: &Elem, images: &mut usize) -> Slide {
	let mut slide = Slide::default();
	// Where NO frame claims to be the title, the first one is taken as it. A deck whose frames were
	// written as plain drawing boxes -- which is what a reader saves when a master page is missing --
	// still has a title on every slide, and a reading view that showed it as a bullet would be
	// disagreeing with what the person looking at the slide sees.
	let classed = page.all("draw:frame").iter()
		.any(|f| matches!(f.attr("presentation:class"), Some("title") | Some("subtitle")));
	let mut first = true;
	for frame in page.all("draw:frame") {
		let class = frame.attr("presentation:class").unwrap_or("");
		if class == "graphic" || frame.child("draw:image").is_some() {
			*images += 1;
			continue;
		}
		let boxed = match frame.child("draw:text-box") {
			Some(b)	=> b,
			None		=> continue,
		};
		let is_title = match classed {
			true	=> class == "title" || class == "subtitle",
			false	=> first,
		};
		first = false;
		let mut lines = Vec::new();
		gather(xml, boxed, 0, &mut lines, images);
		for (level, content) in lines {
			if content.is_empty() {
				continue;
			}
			if is_title && slide.title.is_none() {
				slide.title = Some(content);
				continue;
			}
			slide.bullets.push(Bullet { level, content });
		}
		// Notes live in a `presentation:notes` element beside the frames, not in one.
	}
	if let Some(notes) = page.child("presentation:notes") {
		let text = xml.text_of(notes);
		let text = text.trim();
		if !text.is_empty() {
			slide.notes = Some(text.to_string());
		}
	}
	slide
}

/// The lines a text box holds, with the depth each sits at.
fn gather(
	xml:	&Xml,
	at:	&Elem,
	level:	usize,
	out:	&mut Vec<(usize, Vec<Inline>)>,
	images:	&mut usize,
) {
	for kid in at.elems() {
		match kid.name.qname.as_str() {
			"text:p"	=> out.push((level, read_inlines(xml, kid, images))),
			// Depth is nesting here, so each list inside a list is one level further in.
			"text:list"	=> gather(xml, kid, level + 1, out, images),
			"text:list-item"	=> gather(xml, kid, level, out, images),
			_		=> gather(xml, kid, level, out, images),
		}
	}
}

/// The inline content of one paragraph.
fn read_inlines(xml: &Xml, at: &Elem, images: &mut usize) -> Vec<Inline> {
	let mut out = Vec::new();
	for node in &at.kids {
		match node {
			oxedyne_fe2o3_text::xml::Node::Text(span)	=> {
				let t = xml.text(span);
				if !t.is_empty() {
					out.push(Inline::Text(t));
				}
			}
			oxedyne_fe2o3_text::xml::Node::Elem(e)	=> match e.name.qname.as_str() {
				"text:s"	=> {
					let n = e.attr("text:c").and_then(|v| v.parse::<usize>().ok()).unwrap_or(1);
					out.push(Inline::Text(" ".repeat(n.min(256))));
				}
				"text:tab"		=> out.push(Inline::Text("\t".to_string())),
				"text:line-break"	=> out.push(Inline::Break),
				"draw:image"		=> *images += 1,
				"text:a"	=> {
					let to = e.attr("xlink:href").unwrap_or("").to_string();
					let content = read_inlines(xml, e, images);
					if !content.is_empty() {
						out.push(Inline::Link { to, content });
					}
				}
				"text:span"	=> {
					let style = e.attr("text:style-name").unwrap_or("").to_ascii_lowercase();
					let content = read_inlines(xml, e, images);
					if content.is_empty() {
						continue;
					}
					if style.contains("strong") || style.contains("bold") {
						out.push(Inline::Emph { strong: true, content });
					} else if style.contains("emphasis") || style.contains("italic") {
						out.push(Inline::Emph { strong: false, content });
					} else if style.contains("source") || style.contains("teletype") {
						out.push(Inline::Code(oxedyne_fe2o3_text::doc::text_of(&content)));
					} else {
						out.extend(content);
					}
				}
				_	=> out.extend(read_inlines(xml, e, images)),
			},
			_	=> {}
		}
	}
	out
}
