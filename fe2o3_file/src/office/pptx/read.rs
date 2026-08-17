//! Reading a `.pptx` into the neutral deck.
//!
//! Simpler than the document side, because a slide carries less. The nesting is
//! `p:sld > p:cSld > p:spTree > p:sp > p:txBody > a:p > a:r > a:t`, and what separates a title from a
//! body is the placeholder type on the shape rather than anything about the text.
//!
//! # Slide order comes from the presentation, not from the file names
//!
//! `ppt/slides/slide10.xml` sorts before `slide2.xml` and is the ninth slide, not the second. The
//! order lives in `p:sldIdLst`, which names each slide by a relationship id, so that is what is read.
//! A reader that walked the archive would deal a deck of more than nine slides out of order, and it
//! would look like an authoring mistake rather than a reading one.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use crate::office::deck::{
	Bullet,
	Deck,
	Slide,
};
use crate::office::opc::{
	REL_DOC,
	REL_SLIDE,
};
use crate::zip::Zip;

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_text::doc::Inline;
use oxedyne_fe2o3_text::xml::{
	Elem,
	Xml,
};

use std::collections::BTreeMap;

// The most a single part is inflated to. A slide is small; a slide claiming otherwise is not one.
pub const MAX_PART: u64 = 32 * 1024 * 1024;

/// The leading bytes of an OLE compound file: an encrypted deck, or a `.ppt` from before 2007.
const OLE_MAGIC: [u8; 8] = [0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];

/// A deck read for reading, and what came with it.
#[derive(Clone, Debug, Default)]
pub struct Reading {
	pub deck:	Deck,
	pub macros:	bool,	// said, never run
	// Pictures, charts and other shapes holding no text, counted rather than named by kind: on a
	// slide the distinction a reader cares about is "there is something here you are not seeing",
	// and every one of them is a rectangle of pixels.
	pub shapes:	usize,
	// Slides the presentation names and whose part is missing or unreadable, by number.
	pub missing:	Vec<usize>,
}

pub fn read(bytes: &[u8]) -> Outcome<Reading> {
	if bytes.len() >= OLE_MAGIC.len() && bytes[..OLE_MAGIC.len()] == OLE_MAGIC {
		return Err(err!(
			"This is an OLE compound file, not a `.pptx`. Either it is encrypted, or it is a \
			`.ppt` from before 2007 -- a different format entirely, which this does not read.";
			Invalid, Input, Unimplemented));
	}
	let zip = res!(Zip::read(bytes.to_vec()));
	let mut out = Reading::default();
	out.macros = zip.names().iter().any(|n| n.ends_with("vbaProject.bin"));

	let root_rels = res!(rels_of(&zip, ""));
	let main = res!(root_rels.values()
		.find(|(kind, _)| kind == REL_DOC)
		.map(|(_, t)| t.clone())
		.ok_or_else(|| err!(
			"The package names no presentation part, so this is not a deck. It holds: {}.",
			zip.names().join(", "); Invalid, Input, Missing)));
	let pres = res!(Xml::parse(&res!(part_text(&zip, &main))));
	let rels = res!(rels_of(&zip, &main));

	// The ORDER is here, not in the file names. See the module's own note.
	let ids = match res!(pres.root()).child("p:sldIdLst") {
		Some(l)	=> l.children("p:sldId"),
		None		=> Vec::new(),
	};
	for (n, id) in ids.iter().enumerate() {
		let target = id.attr("r:id")
			.and_then(|i| rels.get(i))
			.filter(|(kind, _)| kind == REL_SLIDE)
			.map(|(_, t)| t.clone());
		let target = match target {
			Some(t) if zip.has(&t)	=> t,
			_			=> {
				out.missing.push(n + 1);
				continue;
			}
		};
		let part = match part_text(&zip, &target) {
			Ok(p)	=> p,
			Err(_)	=> {
				out.missing.push(n + 1);
				continue;
			}
		};
		let xml = match Xml::parse(&part) {
			Ok(x)	=> x,
			Err(_)	=> {
				out.missing.push(n + 1);
				continue;
			}
		};
		out.deck.slides.push(slide_of(&xml, &mut out.shapes));
	}
	Ok(out)
}

/// One slide: its title, and everything else as bullets.
fn slide_of(xml: &Xml, shapes: &mut usize) -> Slide {
	let mut slide = Slide::default();
	// Every shape anywhere in the tree, so a shape inside a group is read rather than lost. A deck
	// from a real template nests them two and three deep.
	for sp in xml.all("p:sp") {
		let body = match sp.child("p:txBody") {
			Some(b)	=> b,
			None		=> continue,
		};
		let is_title = sp.find(&["p:nvSpPr", "p:nvPr", "p:ph"])
			.and_then(|ph| ph.attr("type"))
			.map(|t| t == "title" || t == "ctrTitle")
			.unwrap_or(false);
		for p in body.children("a:p") {
			let content = inlines(xml, p);
			if content.is_empty() {
				continue;
			}
			// The first title paragraph is the title; a second one is a line of the body, because a
			// slide has one title and losing the rest would be worse than moving it.
			if is_title && slide.title.is_none() {
				slide.title = Some(content);
				continue;
			}
			let level = p.find(&["a:pPr"])
				.and_then(|pr| pr.attr("lvl"))
				.and_then(|v| v.parse::<usize>().ok())
				.unwrap_or(0);
			slide.bullets.push(Bullet { level, content });
		}
	}
	// A picture or a chart holds no text, so it is counted rather than read.
	*shapes += xml.all("p:pic").len() + xml.all("p:graphicFrame").len();
	slide
}

fn inlines(xml: &Xml, p: &Elem) -> Vec<Inline> {
	let mut out: Vec<Inline> = Vec::new();
	for kid in p.elems() {
		match kid.name.qname.as_str() {
			// Paragraph properties, not content.
			"a:pPr" | "a:endParaRPr"	=> {}
			"a:br"			=> out.push(Inline::Break),
			"a:r" | "a:fld"		=> {
				// A field is a run whose text was computed -- a slide number, a date -- and its
				// cached text is what is on the slide.
				let text = match kid.child("a:t") {
					Some(t)	=> xml.text_of(t),
					None		=> continue,
				};
				if text.is_empty() {
					continue;
				}
				let pr = kid.child("a:rPr");
				let bold = pr.and_then(|e| e.attr("b")).map(|v| v == "1").unwrap_or(false);
				let italic = pr.and_then(|e| e.attr("i")).map(|v| v == "1").unwrap_or(false);
				let mono = pr.and_then(|e| e.child("a:latin"))
					.and_then(|e| e.attr("typeface"))
					.map(|f| {
						let f = f.to_ascii_lowercase();
						f.contains("consol") || f.contains("courier") || f.contains("mono")
					})
					.unwrap_or(false);
				let mut item = match mono {
					true	=> Inline::Code(text),
					false	=> Inline::Text(text),
				};
				if italic {
					item = Inline::Emph { strong: false, content: vec![item] };
				}
				if bold {
					item = Inline::Emph { strong: true, content: vec![item] };
				}
				out.push(item);
			}
			// Anything else contributes nothing itself and its content is read where it stood.
			_	=> out.extend(inlines(xml, kid)),
		}
	}
	coalesce(out)
}

/// Joins adjacent inlines that are marked alike, so a phrase split across runs reads as one.
fn coalesce(items: Vec<Inline>) -> Vec<Inline> {
	let mut out: Vec<Inline> = Vec::with_capacity(items.len());
	for item in items {
		match (out.last_mut(), item) {
			(Some(Inline::Text(a)), Inline::Text(b))	=> a.push_str(&b),
			(Some(Inline::Code(a)), Inline::Code(b))	=> a.push_str(&b),
			(
				Some(Inline::Emph { strong: sa, content: ca }),
				Inline::Emph { strong: sb, content: cb },
			) if *sa == sb				=> {
				let mut joined = std::mem::take(ca);
				joined.extend(cb);
				*ca = coalesce(joined);
			}
			(_, item)					=> out.push(item),
		}
	}
	out
}

fn part_text(zip: &Zip, name: &str) -> Outcome<String> {
	let bytes = res!(zip.content_capped(name, MAX_PART));
	Ok(res!(String::from_utf8(bytes), Decode, String))
}

/// The directory a part sits in, with its trailing slash.
fn dir_of(part: &str) -> String {
	match part.rfind('/') {
		Some(k)	=> part[..k + 1].to_string(),
		None		=> String::new(),
	}
}

/// Where a relationship target actually is within the package.
///
/// A slide's rels point at `../slideLayouts/...`, so the `..` has to be resolved rather than left in
/// the path: a lookup for `ppt/slides/../slideLayouts/x.xml` finds nothing in an archive whose member
/// is named `ppt/slideLayouts/x.xml`.
fn resolve(dir: &str, target: &str) -> String {
	if let Some(rest) = target.strip_prefix('/') {
		return rest.to_string();
	}
	let mut parts: Vec<&str> = dir.split('/').filter(|p| !p.is_empty()).collect();
	for step in target.split('/') {
		match step {
			"" | "."	=> {}
			".."		=> { parts.pop(); }
			s		=> parts.push(s),
		}
	}
	parts.join("/")
}

/// The relationships a part owns, by id.
fn rels_of(zip: &Zip, part: &str) -> Outcome<BTreeMap<String, (String, String)>> {
	let dir = dir_of(part);
	let name = &part[dir.len()..];
	let path = fmt!("{}_rels/{}.rels", dir, name);
	let mut out = BTreeMap::new();
	if !zip.has(&path) {
		return Ok(out);
	}
	let xml = res!(Xml::parse(&res!(part_text(zip, &path))));
	for rel in res!(xml.root()).children("Relationship") {
		let id = match rel.attr("Id") {
			Some(id)	=> id.to_string(),
			None		=> continue,
		};
		let kind = rel.attr("Type").unwrap_or("").to_string();
		let target = rel.attr("Target").unwrap_or("").to_string();
		let target = match rel.attr("TargetMode") {
			Some("External")	=> target,
			_			=> resolve(&dir, &target),
		};
		out.insert(id, (kind, target));
	}
	Ok(out)
}
