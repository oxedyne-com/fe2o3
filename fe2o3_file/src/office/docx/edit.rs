//! Editing a `.docx` in place: the runs that were asked to change, and not one byte more.
//!
//! # What survives, and why that is the requirement rather than a nicety
//!
//! The document being edited is somebody else's. Round-tripping it through
//! [`oxedyne_fe2o3_text::doc`] -- read to Markdown, edit the Markdown, write a fresh `.docx` -- would
//! produce a file that opens, looks about right, and has silently lost the comments, the bookmarks, the
//! tracked changes, the content controls, the custom XML, the theme, the tab stops, the section
//! properties and the headers. The person who finds out is not the user; it is the colleague they sent
//! it to.
//!
//! So this changes `word/document.xml` by splicing bytes into the `<w:t>` elements that held the text
//! being replaced, and [`crate::zip`] copies every other member of the archive verbatim. A document
//! edited here differs from the one that arrived in exactly the runs that were edited.
//!
//! # Only the body, and it says so
//!
//! Headers, footers, footnotes, comments and text boxes outside the body are separate parts and are NOT
//! searched. A phrase in a header therefore reports as absent rather than being quietly changed in one
//! of two places -- and an absence that names the string is a caller's cue to look, where a partial
//! replacement is a document that disagrees with itself.

use crate::office::edit::{
	Find,
	Piece,
	Tally,
	apply,
};
use crate::office::opc;
use crate::zip::{
	Method,
	Zip,
};

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_text::xml::{
	Elem,
	Xml,
};
use oxedyne_fe2o3_text::xml::write::escape;

/// What an edit of a `.docx` produced.
#[derive(Clone, Debug, Default)]
pub struct Edited {
	pub bytes:	Vec<u8>,
	pub tallies:	Vec<Tally>,	// one per edit asked for, in order
	// Larger than the number of replacements wherever a phrase was spread across several runs, which
	// is the ordinary case in a document a person has formatted.
	pub runs:	usize,
}

/// Replaces text in a `.docx`, leaving everything else exactly as it arrived.
///
/// An edit whose `find` is nowhere in the body is refused by name and NOTHING is written -- see
/// [`crate::office::edit::apply`] on why a silent no-op is the failure this is written against.
pub fn edit(bytes: &[u8], edits: &[Find]) -> Outcome<Edited> {
	if edits.is_empty() {
		return Err(err!("An edit of a document was asked for with no edits in it."; Invalid, Input));
	}
	let mut zip = res!(Zip::read(bytes.to_vec()));
	let part = res!(opc::main_part(&zip, super::read::MAX_PART));
	let src = res!(String::from_utf8(res!(zip.content_capped(&part, super::read::MAX_PART))),
		Decode, String);
	let mut xml = res!(Xml::parse(&src));
	let groups = res!(paragraphs(&xml));
	let (changes, tallies) = res!(apply(&groups, edits));
	let runs = changes.len();
	for c in &changes {
		res!(xml.splice(c.piece.span.clone(), run_text(&c.text)));
	}
	zip.set(&part, xml.render().into_bytes(), Method::Deflate);
	Ok(Edited { bytes: res!(zip.write()), tallies, runs })
}

/// The `<w:t>` elements of each paragraph, in document order, one group per paragraph.
///
/// A paragraph is the unit a match may not cross, because a sentence never spans one and a find that
/// could would happily replace across a heading boundary.
///
/// A nested paragraph -- one inside a text box, which is inside a run, which is inside a paragraph --
/// gets a group of its own and its runs are not also in the enclosing one. Counted twice they would
/// produce two splices over the same bytes, which the splicer refuses; and it would be right to.
fn paragraphs(xml: &Xml) -> Outcome<Vec<Vec<Piece>>> {
	let mut out = Vec::new();
	let root = res!(xml.root());
	walk(xml, root, &mut out);
	Ok(out)
}

/// Adds every paragraph at or below an element to the groups.
fn walk(xml: &Xml, at: &Elem, out: &mut Vec<Vec<Piece>>) {
	if at.name.qname == "w:p" {
		// The slot is claimed before the nested paragraphs are walked, so an enclosing paragraph comes
		// before the text box inside it and `nth` counts in a stable order.
		let slot = out.len();
		out.push(Vec::new());
		let mut group = Vec::new();
		gather(xml, at, &mut group, out);
		out[slot] = group;
		return;
	}
	for kid in at.elems() {
		walk(xml, kid, out);
	}
}

/// Adds one paragraph's own text runs to its group, handing a nested paragraph to [`walk`].
fn gather(xml: &Xml, at: &Elem, group: &mut Vec<Piece>, out: &mut Vec<Vec<Piece>>) {
	for kid in at.elems() {
		match kid.name.qname.as_str() {
			"w:p"	=> walk(xml, kid, out),
			// `w:t` holds text a reader sees. `w:instrText` holds a field's INSTRUCTIONS -- a page
			// reference, a merge field -- and replacing text in one changes what the field does.
			"w:t"	=> group.push(Piece::new(kid.span.clone(), xml.text_of(kid))),
			_	=> gather(xml, kid, group, out),
		}
	}
}

/// A `<w:t>` holding this text.
///
/// `xml:space="preserve"` goes on wherever the text has whitespace at an end, and that is not
/// optional: without it Word and every other reader collapse it, so replacing `Q1` with ` Q1 ` in a
/// document that did not already carry the attribute writes a file whose text differs from what the
/// edit asked for.
fn run_text(text: &str) -> String {
	let ends = text.starts_with(|c: char| c.is_whitespace())
		|| text.ends_with(|c: char| c.is_whitespace());
	match ends {
		true	=> fmt!("<w:t xml:space=\"preserve\">{}</w:t>", escape(text)),
		false	=> fmt!("<w:t>{}</w:t>", escape(text)),
	}
}

/// The text of the body, run by run and paragraph by paragraph, as an edit sees it.
///
/// What a caller shows a person before asking them to confirm an edit, and what a test asserts against.
/// It is NOT the document as prose -- there is [`super::read`] for that -- it is the strings a `find`
/// is matched against, which is a different thing wherever a writer split a sentence.
pub fn body_text(bytes: &[u8]) -> Outcome<Vec<String>> {
	let zip = res!(Zip::read(bytes.to_vec()));
	let part = res!(opc::main_part(&zip, super::read::MAX_PART));
	let src = res!(String::from_utf8(res!(zip.content_capped(&part, super::read::MAX_PART))),
		Decode, String);
	let xml = res!(Xml::parse(&src));
	let groups = res!(paragraphs(&xml));
	Ok(groups.iter()
		.map(|g| g.iter().map(|p| p.text.as_str()).collect::<Vec<_>>().concat())
		.collect())
}
