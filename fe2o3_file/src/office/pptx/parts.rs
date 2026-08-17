//! The three parts a `.pptx` needs before it can hold a slide: a theme, a slide master and a slide
//! layout.
//!
//! None of this is content and all of it is required. A slide points at a layout, a layout at a
//! master, and a master at a theme; PowerPoint refuses the file if any link is missing, and refuses
//! it with a repair prompt that names neither the part nor the reason. That is the worst kind of
//! failure to debug, so the whole chain is written whether or not a deck uses any of it.
//!
//! It is generated rather than held as a blob of literal XML because it is almost entirely
//! repetition -- twelve colours, three fill styles, three line styles, three effect styles, three
//! background fills -- and a blob is where a typo in the eleventh colour waits.
//!
//! [Written entirely with AI](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use crate::office::pptx::{
	MARGIN,
	NS_A,
	NS_P,
	SLIDE_H,
	SLIDE_W,
	TITLE_H,
};
use crate::office::opc::NS_R;

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_text::xml::write::Out;

/// The twelve entries of a colour scheme, in the order the schema requires them.
///
/// The order is not decoration: a reader takes the first as dark 1, the second as light 1, and so on
/// down, so a scheme written in another order is a deck whose text comes out the colour of its
/// background.
const SCHEME: [(&str, &str); 12] = [
	("dk1",	"000000"),
	("lt1",	"FFFFFF"),
	("dk2",	"44546A"),
	("lt2",	"E7E6E6"),
	("accent1",	"4472C4"),
	("accent2",	"ED7D31"),
	("accent3",	"A5A5A5"),
	("accent4",	"FFC000"),
	("accent5",	"5B9BD5"),
	("accent6",	"70AD47"),
	("hlink",	"0563C1"),
	("folHlink",	"954F72"),
];

/// The theme: a colour scheme, a font scheme, and the format scheme PowerPoint insists on.
pub fn theme() -> Outcome<String> {
	let mut out = Out::declared();
	out.open("a:theme", &[("xmlns:a", NS_A), ("name", "Daimond")]);
	out.open("a:themeElements", &[]);

	out.open("a:clrScheme", &[("name", "Daimond")]);
	for (name, rgb) in SCHEME {
		out.open(&fmt!("a:{}", name), &[]);
		// The first two are system colours in every theme Office writes, and a reader that expects
		// one and finds an RGB still renders. `srgbClr` throughout keeps this one file readable.
		out.empty("a:srgbClr", &[("val", rgb)]);
		res!(out.close(&fmt!("a:{}", name)));
	}
	res!(out.close("a:clrScheme"));

	out.open("a:fontScheme", &[("name", "Daimond")]);
	for which in ["a:majorFont", "a:minorFont"] {
		out.open(which, &[]);
		out.empty("a:latin", &[("typeface", "Calibri")]);
		out.empty("a:ea", &[("typeface", "")]);
		out.empty("a:cs", &[("typeface", "")]);
		res!(out.close(which));
	}
	res!(out.close("a:fontScheme"));

	// Three of each, always. The schema requires exactly three subtle-to-intense variants in each of
	// the four lists, and a deck with two opens with a repair prompt.
	out.open("a:fmtScheme", &[("name", "Daimond")]);
	out.open("a:fillStyleLst", &[]);
	for _ in 0..3 {
		out.open("a:solidFill", &[]);
		out.empty("a:schemeClr", &[("val", "phClr")]);
		res!(out.close("a:solidFill"));
	}
	res!(out.close("a:fillStyleLst"));
	out.open("a:lnStyleLst", &[]);
	for w in ["6350", "12700", "19050"] {
		out.open("a:ln", &[("w", w), ("cap", "flat"), ("cmpd", "sng"), ("algn", "ctr")]);
		out.open("a:solidFill", &[]);
		out.empty("a:schemeClr", &[("val", "phClr")]);
		res!(out.close("a:solidFill"));
		out.empty("a:prstDash", &[("val", "solid")]);
		res!(out.close("a:ln"));
	}
	res!(out.close("a:lnStyleLst"));
	out.open("a:effectStyleLst", &[]);
	for _ in 0..3 {
		out.open("a:effectStyle", &[]);
		out.open("a:effectLst", &[]);
		res!(out.close("a:effectLst"));
		res!(out.close("a:effectStyle"));
	}
	res!(out.close("a:effectStyleLst"));
	out.open("a:bgFillStyleLst", &[]);
	for _ in 0..3 {
		out.open("a:solidFill", &[]);
		out.empty("a:schemeClr", &[("val", "phClr")]);
		res!(out.close("a:solidFill"));
	}
	res!(out.close("a:bgFillStyleLst"));
	res!(out.close("a:fmtScheme"));

	res!(out.close("a:themeElements"));
	res!(out.close("a:theme"));
	out.finish()
}

/// The slide master: the shape tree every slide inherits, and the map from scheme colours to roles.
pub fn master() -> Outcome<String> {
	let mut out = Out::declared();
	out.open("p:sldMaster", &[("xmlns:a", NS_A), ("xmlns:r", NS_R), ("xmlns:p", NS_P)]);
	res!(shape_tree(&mut out, true));
	// Which scheme entry plays which role. Getting this wrong is how a deck comes out with white
	// text on a white background, so it is written explicitly rather than left to a default.
	out.empty("p:clrMap", &[
		("bg1", "lt1"), ("tx1", "dk1"), ("bg2", "lt2"), ("tx2", "dk2"),
		("accent1", "accent1"), ("accent2", "accent2"), ("accent3", "accent3"),
		("accent4", "accent4"), ("accent5", "accent5"), ("accent6", "accent6"),
		("hlink", "hlink"), ("folHlink", "folHlink"),
	]);
	out.open("p:sldLayoutIdLst", &[]);
	out.empty("p:sldLayoutId", &[("id", "2147483649"), ("r:id", "rId1")]);
	res!(out.close("p:sldLayoutIdLst"));
	res!(out.close("p:sldMaster"));
	out.finish()
}

/// The one slide layout: a title and a body, which is the only shape a generated deck needs.
pub fn layout() -> Outcome<String> {
	let mut out = Out::declared();
	out.open("p:sldLayout", &[
		("xmlns:a", NS_A), ("xmlns:r", NS_R), ("xmlns:p", NS_P),
		("type", "titleAndBody"), ("preserve", "1"),
	]);
	res!(shape_tree(&mut out, false));
	res!(out.close("p:sldLayout"));
	out.finish()
}

/// The common shell of a master's or a layout's shape tree: the two placeholders and the group that
/// holds them.
///
/// Identical in both but for the element name around it, so it is written once. The placeholders
/// carry explicit geometry rather than inheriting it, because a placeholder with no `xfrm` anywhere
/// up its chain is a shape a reader puts at the origin with no size.
fn shape_tree(out: &mut Out, master: bool) -> Outcome<()> {
	out.open("p:cSld", &[]);
	out.open("p:spTree", &[]);
	out.open("p:nvGrpSpPr", &[]);
	out.empty("p:cNvPr", &[("id", "1"), ("name", "")]);
	out.empty("p:cNvGrpSpPr", &[]);
	out.empty("p:nvPr", &[]);
	res!(out.close("p:nvGrpSpPr"));
	out.open("p:grpSpPr", &[]);
	out.open("a:xfrm", &[]);
	out.empty("a:off", &[("x", "0"), ("y", "0")]);
	out.empty("a:ext", &[("cx", "0"), ("cy", "0")]);
	out.empty("a:chOff", &[("x", "0"), ("y", "0")]);
	out.empty("a:chExt", &[("cx", "0"), ("cy", "0")]);
	res!(out.close("a:xfrm"));
	res!(out.close("p:grpSpPr"));
	res!(placeholder(out, 2, "Title", "title", None, MARGIN, MARGIN, SLIDE_W - 2 * MARGIN, TITLE_H));
	res!(placeholder(out, 3, "Body", "body", Some("1"),
		MARGIN, MARGIN + TITLE_H, SLIDE_W - 2 * MARGIN, SLIDE_H - TITLE_H - 2 * MARGIN));
	res!(out.close("p:spTree"));
	res!(out.close("p:cSld"));
	let _ = master;
	Ok(())
}

/// One placeholder shape, with its geometry written out.
pub fn placeholder(
	out:	&mut Out,
	id:	u32,
	name:	&str,
	kind:	&str,
	idx:	Option<&str>,
	x:	i64,
	y:	i64,
	cx:	i64,
	cy:	i64,
)
	-> Outcome<()>
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
	out.empty("a:bodyPr", &[]);
	out.empty("a:lstStyle", &[]);
	out.open("a:p", &[]);
	res!(out.close("a:p"));
	res!(out.close("p:txBody"));
	res!(out.close("p:sp"));
	Ok(())
}
