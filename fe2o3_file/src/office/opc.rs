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

use oxedyne_fe2o3_text::xml::write::Out;

use oxedyne_fe2o3_core::prelude::*;

/// The namespace of `[Content_Types].xml`.
pub const NS_TYPES: &str = "http://schemas.openxmlformats.org/package/2006/content-types";
/// The namespace of a `.rels` part.
pub const NS_RELS: &str = "http://schemas.openxmlformats.org/package/2006/relationships";
/// The namespace an `r:id` attribute is in, which the content parts declare.
pub const NS_R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";

/// The relationship type of the part that is the document itself.
pub const REL_DOC: &str =
	"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument";
/// The relationship type of a styles part.
pub const REL_STYLES: &str =
	"http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles";
/// The relationship type of a numbering definitions part.
pub const REL_NUMBERING: &str =
	"http://schemas.openxmlformats.org/officeDocument/2006/relationships/numbering";
/// The relationship type of a link out of the document.
pub const REL_HYPERLINK: &str =
	"http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink";

/// The content type of the main part of a word-processing document.
pub const CT_DOCUMENT: &str =
	"application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml";
/// The content type of a word-processing styles part.
pub const CT_STYLES: &str =
	"application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml";
/// The content type of a word-processing numbering part.
pub const CT_NUMBERING: &str =
	"application/vnd.openxmlformats-officedocument.wordprocessingml.numbering+xml";
/// The content type of a `.rels` part, which is declared by extension rather than by name.
pub const CT_RELS: &str = "application/vnd.openxmlformats-package.relationships+xml";

/// One relationship: what it is, and what it points at.
#[derive(Clone, Debug, PartialEq)]
pub struct Rel {
	/// The id the owning part refers to it by, unique within that part.
	pub id:	String,
	/// The relationship type, which says what the target is *for*.
	pub kind:	String,
	/// Where it points: a part within the package, or a URL where it is external.
	pub target:	String,
	/// Whether the target is outside the package, which a link out of the document is.
	pub external:	bool,
}

/// The relationships one part owns, and the ids it has handed out.
#[derive(Clone, Debug, Default)]
pub struct Rels {
	/// The relationships, in the order they were added.
	items:	Vec<Rel>,
}

impl Rels {

	/// A part with no relationships yet.
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

	/// The relationships, in order.
	pub fn items(&self) -> &[Rel] {
		&self.items
	}

	/// Whether there are none.
	pub fn is_empty(&self) -> bool {
		self.items.is_empty()
	}

	/// Adds a relationship and gives back its id.
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
	/// Extension and content type, for every part whose name ends that way.
	defaults:	Vec<(String, String)>,
	/// Part name and content type, for one part.
	overrides:	Vec<(String, String)>,
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

	/// Declares what parts with an extension are.
	///
	/// Named `by_ext` rather than `default`, which would shadow the trait method of that name on the
	/// same type and make `Types::default()` mean two things.
	pub fn by_ext(&mut self, ext: &str, kind: &str) {
		self.defaults.push((ext.to_string(), kind.to_string()));
	}

	/// Declares what one named part is. The name is absolute within the package, leading slash and all.
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
