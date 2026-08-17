//! Creating a `.pptx` from the neutral deck.
//!
//! Nine parts for one slide and one more for each slide after it. The nine are the chain a slide
//! hangs from -- presentation, master, layout, theme, and a relationship part for each -- and
//! [`super::parts`] writes the ones that are not content.
//!
//! # What a slide is, once the skeleton is out of the way
//!
//! Two shapes: a title placeholder and a body placeholder. The body's paragraphs carry an indent
//! level and nothing else, so a bullet three deep is `<a:pPr lvl="2">` and the layout decides what
//! that looks like. That is the whole mapping, and it is small because [`crate::office::deck`] is
//! small on purpose.

use crate::office::deck::{
	Deck,
	MAX_LEVEL,
};
use crate::office::opc::{
	CT_LAYOUT,
	CT_MASTER,
	CT_PRESENTATION,
	CT_SLIDE,
	CT_THEME,
	NS_R,
	REL_DOC,
	REL_LAYOUT,
	REL_MASTER,
	REL_SLIDE,
	REL_THEME,
	Rels,
	Types,
};
use crate::office::pptx::{
	MARGIN,
	NS_A,
	NS_P,
	SLIDE_H,
	SLIDE_W,
	TITLE_H,
	parts,
};
use crate::zip::{
	Method,
	Zip,
};

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_text::doc::Inline;
use oxedyne_fe2o3_text::xml::write::Out;

/// What a created deck could not carry.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Left {
	/// The images whose bytes could not be reached, by the source each was written with.
	pub images:	Vec<String>,
	/// How many slides carried speaker's notes that were not written.
	///
	/// A notes slide needs a notes master and a notes layout, which is the whole skeleton again for
	/// content nobody sees on screen. Counted and said rather than written or silently dropped.
	pub notes:	usize,
}

impl Left {

	/// Whether everything in the deck reached the file.
	pub fn is_empty(&self) -> bool {
		self.images.is_empty() && self.notes == 0
	}
}

/// Writes a deck as the bytes of a `.pptx`, and says what did not fit.
pub fn write(deck: &Deck) -> Outcome<(Vec<u8>, Left)> {
	let mut owned;
	// A presentation with no slides is a file PowerPoint opens and shows nothing in, which reads as
	// a corrupt deck. One empty slide is honest about being empty.
	let deck = match deck.slides.is_empty() {
		false	=> deck,
		true	=> {
			owned = Deck::new();
			owned.slides.push(Default::default());
			&owned
		}
	};
	let mut left = Left::default();

	let mut types = Types::new();
	types.over("/ppt/presentation.xml", CT_PRESENTATION);
	types.over("/ppt/slideMasters/slideMaster1.xml", CT_MASTER);
	types.over("/ppt/slideLayouts/slideLayout1.xml", CT_LAYOUT);
	types.over("/ppt/theme/theme1.xml", CT_THEME);

	let mut root = Rels::new();
	let _ = root.add(REL_DOC, "ppt/presentation.xml");

	// The presentation's relationships. The master comes FIRST, because `p:sldMasterIdLst` names it
	// by id and a reader that found a slide there instead refuses the file.
	let mut pres_rels = Rels::new();
	let master_id = pres_rels.add(REL_MASTER, "slideMasters/slideMaster1.xml");
	let mut slide_ids = Vec::with_capacity(deck.slides.len());
	for i in 0..deck.slides.len() {
		slide_ids.push(pres_rels.add(REL_SLIDE, &fmt!("slides/slide{}.xml", i + 1)));
		types.over(&fmt!("/ppt/slides/slide{}.xml", i + 1), CT_SLIDE);
	}
	let _ = pres_rels.add(REL_THEME, "theme/theme1.xml");

	let mut pres = Out::declared();
	pres.open("p:presentation", &[("xmlns:a", NS_A), ("xmlns:r", NS_R), ("xmlns:p", NS_P)]);
	pres.open("p:sldMasterIdLst", &[]);
	pres.empty("p:sldMasterId", &[("id", "2147483648"), ("r:id", &master_id)]);
	res!(pres.close("p:sldMasterIdLst"));
	pres.open("p:sldIdLst", &[]);
	for (i, id) in slide_ids.iter().enumerate() {
		// Slide ids must be at least 256 and unique. Counting from 256 is what PowerPoint does.
		pres.empty("p:sldId", &[("id", &fmt!("{}", 256 + i)), ("r:id", id)]);
	}
	res!(pres.close("p:sldIdLst"));
	pres.empty("p:sldSz", &[("cx", &fmt!("{}", SLIDE_W)), ("cy", &fmt!("{}", SLIDE_H))]);
	// The notes page is a different size from the slide, and the element is required even by a deck
	// that carries no notes.
	pres.empty("p:notesSz", &[("cx", &fmt!("{}", SLIDE_H)), ("cy", &fmt!("{}", SLIDE_W))]);
	res!(pres.close("p:presentation"));

	// The master points at its one layout and at the theme; the layout points back at the master.
	// A chain with a link missing is a file PowerPoint offers to repair rather than open.
	let mut master_rels = Rels::new();
	let _ = master_rels.add(REL_LAYOUT, "../slideLayouts/slideLayout1.xml");
	let _ = master_rels.add(REL_THEME, "../theme/theme1.xml");
	let mut layout_rels = Rels::new();
	let _ = layout_rels.add(REL_MASTER, "../slideMasters/slideMaster1.xml");

	let mut zip = Zip::new();
	zip.set("[Content_Types].xml", res!(types.write()).into_bytes(), Method::Deflate);
	zip.set("_rels/.rels", res!(root.write()).into_bytes(), Method::Deflate);
	zip.set("ppt/presentation.xml", res!(pres.finish()).into_bytes(), Method::Deflate);
	zip.set("ppt/_rels/presentation.xml.rels", res!(pres_rels.write()).into_bytes(), Method::Deflate);
	zip.set("ppt/slideMasters/slideMaster1.xml", res!(parts::master()).into_bytes(), Method::Deflate);
	zip.set("ppt/slideMasters/_rels/slideMaster1.xml.rels",
		res!(master_rels.write()).into_bytes(), Method::Deflate);
	zip.set("ppt/slideLayouts/slideLayout1.xml", res!(parts::layout()).into_bytes(), Method::Deflate);
	zip.set("ppt/slideLayouts/_rels/slideLayout1.xml.rels",
		res!(layout_rels.write()).into_bytes(), Method::Deflate);
	zip.set("ppt/theme/theme1.xml", res!(parts::theme()).into_bytes(), Method::Deflate);
	for (i, slide) in deck.slides.iter().enumerate() {
		if slide.notes.is_some() {
			left.notes += 1;
		}
		let part = res!(slide_part(slide, &mut left));
		zip.set(&fmt!("ppt/slides/slide{}.xml", i + 1), part.into_bytes(), Method::Deflate);
		// Every slide names the layout it hangs from, in its own relationship part.
		let mut rels = Rels::new();
		let _ = rels.add(REL_LAYOUT, "../slideLayouts/slideLayout1.xml");
		zip.set(&fmt!("ppt/slides/_rels/slide{}.xml.rels", i + 1),
			res!(rels.write()).into_bytes(), Method::Deflate);
	}
	Ok((res!(zip.write()), left))
}

/// One slide.
fn slide_part(slide: &crate::office::deck::Slide, left: &mut Left) -> Outcome<String> {
	let mut out = Out::declared();
	out.open("p:sld", &[("xmlns:a", NS_A), ("xmlns:r", NS_R), ("xmlns:p", NS_P)]);
	out.open("p:cSld", &[]);
	out.open("p:spTree", &[]);
	out.open("p:nvGrpSpPr", &[]);
	out.empty("p:cNvPr", &[("id", "1"), ("name", "")]);
	out.empty("p:cNvGrpSpPr", &[]);
	out.empty("p:nvPr", &[]);
	res!(out.close("p:nvGrpSpPr"));
	out.open("p:grpSpPr", &[]);
	res!(out.close("p:grpSpPr"));

	res!(shape(&mut out, 2, "Title", "title", None, MARGIN, MARGIN, SLIDE_W - 2 * MARGIN, TITLE_H,
		|out, left| {
			match &slide.title {
				Some(t)	=> para(out, t, 0, left),
				None		=> {
					out.open("a:p", &[]);
					res!(out.close("a:p"));
					Ok(())
				}
			}
		}, left));

	res!(shape(&mut out, 3, "Body", "body", Some("1"),
		MARGIN, MARGIN + TITLE_H, SLIDE_W - 2 * MARGIN, SLIDE_H - TITLE_H - 2 * MARGIN,
		|out, left| {
			if slide.bullets.is_empty() {
				out.open("a:p", &[]);
				res!(out.close("a:p"));
				return Ok(());
			}
			for b in &slide.bullets {
				res!(para(out, &b.content, b.level.min(MAX_LEVEL), left));
			}
			Ok(())
		}, left));

	res!(out.close("p:spTree"));
	res!(out.close("p:cSld"));
	out.open("p:clrMapOvr", &[]);
	out.empty("a:masterClrMapping", &[]);
	res!(out.close("p:clrMapOvr"));
	res!(out.close("p:sld"));
	out.finish()
}

/// One placeholder shape on a slide, with its body written by the caller.
fn shape<F>(
	out:	&mut Out,
	id:	u32,
	name:	&str,
	kind:	&str,
	idx:	Option<&str>,
	x:	i64,
	y:	i64,
	cx:	i64,
	cy:	i64,
	body:	F,
	left:	&mut Left,
)
	-> Outcome<()>
where
	F: FnOnce(&mut Out, &mut Left) -> Outcome<()>,
{
	out.open("p:sp", &[]);
	out.open("p:nvSpPr", &[]);
	out.empty("p:cNvPr", &[("id", &fmt!("{}", id)), ("name", name)]);
	out.open("p:cNvSpPr", &[]);
	out.empty("a:spLocks", &[("noGrp", "1")]);
	res!(out.close("p:cNvSpPr"));
	out.open("p:nvPr", &[]);
	match idx {
		Some(i)	=> out.empty("p:ph", &[("type", kind), ("idx", i)]),
		None		=> out.empty("p:ph", &[("type", kind)]),
	}
	res!(out.close("p:nvPr"));
	res!(out.close("p:nvSpPr"));
	out.open("p:spPr", &[]);
	out.open("a:xfrm", &[]);
	out.empty("a:off", &[("x", &fmt!("{}", x)), ("y", &fmt!("{}", y))]);
	out.empty("a:ext", &[("cx", &fmt!("{}", cx)), ("cy", &fmt!("{}", cy))]);
	res!(out.close("a:xfrm"));
	out.empty("a:prstGeom", &[("prst", "rect")]);
	res!(out.close("p:spPr"));
	out.open("p:txBody", &[]);
	out.empty("a:bodyPr", &[("wrap", "square")]);
	out.empty("a:lstStyle", &[]);
	res!(body(out, left));
	res!(out.close("p:txBody"));
	res!(out.close("p:sp"));
	Ok(())
}

/// One paragraph of a text body, at an indent level.
fn para(out: &mut Out, content: &[Inline], level: usize, left: &mut Left) -> Outcome<()> {
	match level {
		0	=> out.open("a:p", &[]),
		n	=> {
			out.open("a:p", &[]);
			out.empty("a:pPr", &[("lvl", &fmt!("{}", n))]);
		}
	}
	res!(runs(out, content, Fmt::default(), left));
	res!(out.close("a:p"));
	Ok(())
}

/// How a run of text on a slide is marked.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Fmt {
	/// Bold.
	bold:	bool,
	/// Italic.
	italic:	bool,
	/// A span of code, which on a slide is a monospaced run.
	code:	bool,
	/// Where the run links to, if it links anywhere.
	link:	bool,
}

/// The runs a piece of inline content makes.
fn runs(out: &mut Out, content: &[Inline], fmt: Fmt, left: &mut Left) -> Outcome<()> {
	for item in content {
		match item {
			Inline::Text(t)			=> res!(run(out, t, fmt)),
			Inline::Code(t)			=> res!(run(out, t, Fmt { code: true, ..fmt })),
			Inline::Emph { strong, content }	=> {
				let fmt = match strong {
					true	=> Fmt { bold: true, ..fmt },
					false	=> Fmt { italic: true, ..fmt },
				};
				res!(runs(out, content, fmt, left));
			}
			// A link on a slide is a relationship in the SLIDE's own rels part, and a generated
			// deck has no reader that would follow one. The text stands and is marked, which is
			// what a person reading the slide gets from it either way.
			Inline::Link { content, .. }		=> {
				res!(runs(out, content, Fmt { link: true, ..fmt }, left))
			}
			Inline::Image { src, alt }		=> {
				left.images.push(src.clone());
				res!(run(out, alt, Fmt { italic: true, ..fmt }));
			}
			Inline::Span { content, .. }		=> res!(runs(out, content, fmt, left)),
			// A slide has no soft break worth keeping: the shape wraps.
			Inline::Break				=> res!(run(out, " ", fmt)),
		}
	}
	Ok(())
}

/// One run of text.
fn run(out: &mut Out, text: &str, fmt: Fmt) -> Outcome<()> {
	if text.is_empty() {
		return Ok(());
	}
	out.open("a:r", &[]);
	let mut attrs: Vec<(&str, &str)> = vec![("lang", "en-AU"), ("dirty", "0")];
	if fmt.bold {
		attrs.push(("b", "1"));
	}
	if fmt.italic {
		attrs.push(("i", "1"));
	}
	if fmt.link {
		attrs.push(("u", "sng"));
	}
	match fmt.code {
		false	=> out.empty("a:rPr", &attrs),
		true	=> {
			out.open("a:rPr", &attrs);
			out.empty("a:latin", &[("typeface", "Consolas")]);
			res!(out.close("a:rPr"));
		}
	}
	out.leaf("a:t", &[], text);
	res!(out.close("a:r"));
	Ok(())
}
