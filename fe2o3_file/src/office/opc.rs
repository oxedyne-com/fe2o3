//! Open Packaging Conventions: the two parts every Office document has before it has any content.
//!
//! `[Content_Types].xml` says what each part in the archive is, and a `.rels` part says how the parts
//! refer to one another. A document missing either is one Word declines to open, with a message that
//! names neither the part nor the reason, so both are built here rather than written out by hand in
//! three places.
//!
//! Relationship ids are `rId1`, `rId2`, and so on. They are *local to the part that owns the rels*,
//! which is why [`Rels`] hands them out rather than a counter somewhere global: the ids in
//! `word/_rels/document.xml.rels` have nothing to do with the ids in `_rels/.rels`, and a scheme that
//! shared them would work until the day two parts both had one.
//!
//! [Written entirely with AI](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use crate::zip::Zip;

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_text::xml::Xml;
use oxedyne_fe2o3_text::xml::write::Out;

use std::collections::BTreeMap;

//// The namespaces, relationship types and content types OOXML fixes. Every
//// value here is written into the package and read back by other programs, so
//// none of them is ours to change.
pub const NS_TYPES: &str = "http://schemas.openxmlformats.org/package/2006/content-types";
pub const NS_RELS: &str = "http://schemas.openxmlformats.org/package/2006/relationships";
pub const NS_R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";

pub const REL_DOC: &str =
	"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument";
pub const REL_STYLES: &str =
	"http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles";
pub const REL_NUMBERING: &str =
	"http://schemas.openxmlformats.org/officeDocument/2006/relationships/numbering";
pub const REL_HYPERLINK: &str =
	"http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink";

pub const CT_DOCUMENT: &str =
	"application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml";
pub const CT_STYLES: &str =
	"application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml";
pub const CT_NUMBERING: &str =
	"application/vnd.openxmlformats-officedocument.wordprocessingml.numbering+xml";
pub const REL_SHEET: &str =
	"http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet";
pub const REL_STRINGS: &str =
	"http://schemas.openxmlformats.org/officeDocument/2006/relationships/sharedStrings";

pub const CT_WORKBOOK: &str =
	"application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml";
pub const CT_SHEET: &str =
	"application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml";
pub const CT_STRINGS: &str =
	"application/vnd.openxmlformats-officedocument.spreadsheetml.sharedStrings+xml";
pub const CT_SHEET_STYLES: &str =
	"application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml";

pub const REL_MASTER: &str =
	"http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideMaster";
pub const REL_SLIDE: &str =
	"http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide";
pub const REL_LAYOUT: &str =
	"http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideLayout";
pub const REL_THEME: &str =
	"http://schemas.openxmlformats.org/officeDocument/2006/relationships/theme";

pub const CT_PRESENTATION: &str =
	"application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml";
pub const CT_SLIDE: &str =
	"application/vnd.openxmlformats-officedocument.presentationml.slide+xml";
pub const CT_MASTER: &str =
	"application/vnd.openxmlformats-officedocument.presentationml.slideMaster+xml";
pub const CT_LAYOUT: &str =
	"application/vnd.openxmlformats-officedocument.presentationml.slideLayout+xml";
pub const CT_THEME: &str = "application/vnd.openxmlformats-officedocument.theme+xml";

/// The content type of a `.rels` part, which is declared by extension rather than by name.
pub const CT_RELS: &str = "application/vnd.openxmlformats-package.relationships+xml";

/// One relationship: what it is, and what it points at.
#[derive(Clone, Debug, PartialEq)]
pub struct Rel {
	pub id:	String,	// unique within the part that owns it
	pub kind:	String,	// what the target is for
	pub target:	String,	// a part within the package, or a URL where it is external
	pub external:	bool,
}

/// The relationships one part owns, and the ids it has handed out.
#[derive(Clone, Debug, Default)]
pub struct Rels {
	items:	Vec<Rel>,	// in the order they were added
}

impl Rels {

	pub fn new() -> Self {
		Self::default()
	}

	/// Adds a relationship to another part of the package, giving back the id to refer to it by.
	pub fn add(&mut self, kind: &str, target: &str) -> String {
		self.push(kind, target, false)
	}

	/// Adds a relationship to something outside the package, giving back the id.
	pub fn add_external(&mut self, kind: &str, target: &str) -> String {
		self.push(kind, target, true)
	}

	pub fn items(&self) -> &[Rel] {
		&self.items
	}

	pub fn is_empty(&self) -> bool {
		self.items.is_empty()
	}

	fn push(&mut self, kind: &str, target: &str, external: bool) -> String {
		let id = fmt!("rId{}", self.items.len() + 1);
		self.items.push(Rel {
			id:	id.clone(),
			kind:	kind.to_string(),
			target:	target.to_string(),
			external,
		});
		id
	}

	/// The part as XML.
	pub fn write(&self) -> Outcome<String> {
		let mut out = Out::declared();
		out.open("Relationships", &[("xmlns", NS_RELS)]);
		for r in &self.items {
			match r.external {
				true	=> out.empty("Relationship", &[
					("Id", &r.id),
					("Type", &r.kind),
					("Target", &r.target),
					("TargetMode", "External"),
				]),
				false	=> out.empty("Relationship", &[
					("Id", &r.id),
					("Type", &r.kind),
					("Target", &r.target),
				]),
			}
		}
		res!(out.close("Relationships"));
		out.finish()
	}
}

/// What each part of the package is: by extension for the ones there are many of, and by name for the
/// ones there are not.
#[derive(Clone, Debug, Default)]
pub struct Types {
	defaults:	Vec<(String, String)>,	// extension, content type
	overrides:	Vec<(String, String)>,	// part name, content type
}

impl Types {

	/// A package declaring the two defaults every Office document needs: the relationship parts, and
	/// XML for everything else.
	pub fn new() -> Self {
		let mut t = Self::default();
		t.by_ext("rels", CT_RELS);
		t.by_ext("xml", "application/xml");
		t
	}

	/// Named `by_ext` rather than `default`, which would shadow the trait method of that name on the
	/// same type and make `Types::default()` mean two things.
	pub fn by_ext(&mut self, ext: &str, kind: &str) {
		self.defaults.push((ext.to_string(), kind.to_string()));
	}

	/// The name is absolute within the package, leading slash and all.
	pub fn over(&mut self, part: &str, kind: &str) {
		self.overrides.push((part.to_string(), kind.to_string()));
	}

	/// The part as XML.
	pub fn write(&self) -> Outcome<String> {
		let mut out = Out::declared();
		out.open("Types", &[("xmlns", NS_TYPES)]);
		for (ext, kind) in &self.defaults {
			out.empty("Default", &[("Extension", ext), ("ContentType", kind)]);
		}
		for (part, kind) in &self.overrides {
			out.empty("Override", &[("PartName", part), ("ContentType", kind)]);
		}
		res!(out.close("Types"));
		out.finish()
	}
}

// ---------------------------------------------------------------------------
// Finding a part in a package that was read
// ---------------------------------------------------------------------------

/// The directory a part sits in, with its trailing slash, so a relative target resolves against it.
pub fn dir_of(part: &str) -> String {
	match part.rfind('/') {
		Some(k)	=> part[..k + 1].to_string(),
		None		=> String::new(),
	}
}

/// Where a relationship target actually is within the package.
pub fn resolve(dir: &str, target: &str) -> String {
	match target.starts_with('/') {
		true	=> target[1..].to_string(),
		false	=> fmt!("{}{}", dir, target),
	}
}

/// The relationships a part owns, by id: the type, and the resolved target.
///
/// A part's relationships live beside it, in a `_rels` directory, in a file named after it. The
/// package's own are in `_rels/.rels`, which is the same rule with an empty name -- so `""` asks for
/// them.
///
/// A part with no `.rels` beside it has no relationships, which is not an error: most parts have none.
pub fn rels_of(zip: &Zip, part: &str, cap: u64) -> Outcome<BTreeMap<String, (String, String)>> {
	let dir = dir_of(part);
	let name = &part[dir.len()..];
	let path = fmt!("{}_rels/{}.rels", dir, name);
	let mut out = BTreeMap::new();
	if !zip.has(&path) {
		return Ok(out);
	}
	let text = res!(String::from_utf8(res!(zip.content_capped(&path, cap))), Decode, String);
	let xml = res!(Xml::parse(&text));
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

/// The part that IS the document: what `_rels/.rels` points at with [`REL_DOC`].
///
/// Named from the package rather than guessed at, because `word/document.xml` is a convention and not a
/// rule -- a `.docm` names `word/document.xml` too, and a document saved by a generator that used
/// another name still opens in Word.
pub fn main_part(zip: &Zip, cap: u64) -> Outcome<String> {
	let rels = res!(rels_of(zip, "", cap));
	Ok(res!(rels.values()
		.find(|(kind, _)| kind == REL_DOC)
		.map(|(_, t)| t.clone())
		.filter(|t| zip.has(t))
		.ok_or_else(|| err!(
			"The package names no document part, so this is not an Office document. It holds: {}.",
			zip.names().join(", "); Invalid, Input, Missing))))
}
