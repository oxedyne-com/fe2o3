//! The OpenDocument package: the four members every one of them has, and the manifest that lists
//! them.
//!
//! Written once here rather than three times in the writers beside it, because the part that must not
//! be got wrong -- `mimetype` first and stored -- is the part it would be easiest to get wrong
//! separately in each.

use crate::office::odf::{
	NS_MANIFEST,
	NS_OFFICE,
	NS_STYLE,
	NS_TEXT,
};
use crate::zip::{
	Method,
	Zip,
};

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_text::xml::write::Out;

/// The version of OpenDocument written.
pub const VERSION: &str = "1.3";

/// Starts a package: the `mimetype` member, first and stored, and the manifest that lists what will
/// follow.
///
/// **`mimetype` FIRST and STORED, and neither half is negotiable.** The format requires it so a
/// reader can name the file from its opening bytes without inflating anything, which is exactly what
/// `oxedyne_fe2o3_stds::media` now does. A package that deflates it, or writes it second, is a file
/// every reader calls a plain ZIP -- and it fails that way silently, opening as an archive rather
/// than refusing.
pub fn start(media: &str) -> Zip {
	let mut zip = Zip::new();
	zip.set_first("mimetype", media.as_bytes().to_vec(), Method::Store);
	zip
}

/// Finishes a package: writes the manifest listing every member already in it.
///
/// Called last, so the manifest lists what is actually there rather than what a caller intended. A
/// manifest naming a member the archive does not hold is the OpenDocument equivalent of a content
/// type override with no part behind it.
pub fn finish(zip: &mut Zip, media: &str) -> Outcome<()> {
	let mut out = Out::declared();
	out.open("manifest:manifest", &[
		("xmlns:manifest", NS_MANIFEST),
		("manifest:version", VERSION),
	]);
	out.empty("manifest:file-entry", &[
		("manifest:full-path", "/"),
		("manifest:version", VERSION),
		("manifest:media-type", media),
	]);
	let names: Vec<String> = zip.names().iter().map(|n| n.to_string()).collect();
	for name in names {
		// `mimetype` is the package's own declaration and is not a member the manifest lists.
		if name == "mimetype" {
			continue;
		}
		out.empty("manifest:file-entry", &[
			("manifest:full-path", &name),
			("manifest:media-type", "text/xml"),
		]);
	}
	res!(out.close("manifest:manifest"));
	zip.set("META-INF/manifest.xml", res!(out.finish()).into_bytes(), Method::Deflate);
	Ok(())
}

/// The `styles.xml` every package carries, holding the styles a document refers to by name.
///
/// Minimal on purpose. OpenDocument's own defaults are sensible and a reader applies its template
/// where a document says nothing, so a writer that specified every font and every margin would be
/// overriding the reader's choices rather than expressing the author's.
pub fn styles() -> Outcome<String> {
	styles_for("")
}

/// The `styles.xml` for a package of a given media type.
///
/// A presentation needs one thing the other two do not: a MASTER PAGE. Without it a reader treats
/// every frame on a slide as a plain drawing box rather than as a placeholder, and
/// `presentation:class="title"` becomes meaningless -- LibreOffice drops the attribute on re-save and
/// the deck's titles stop being titles. The text still appears, so nothing looks broken until
/// somebody tries to use an outline view. Measured, not assumed: it is what the fixture came back as
/// before this was written.
pub fn styles_for(media: &str) -> Outcome<String> {
	let slides = media == "application/vnd.oasis.opendocument.presentation";
	let mut out = Out::declared();
	out.open("office:document-styles", &[
		("xmlns:office", NS_OFFICE),
		("xmlns:style", NS_STYLE),
		("xmlns:text", NS_TEXT),
		("xmlns:draw", "urn:oasis:names:tc:opendocument:xmlns:drawing:1.0"),
		("xmlns:fo", "urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0"),
		("office:version", VERSION),
	]);
	if slides {
		out.open("office:automatic-styles", &[]);
		out.open("style:page-layout", &[("style:name", "PM1")]);
		out.empty("style:page-layout-properties", &[
			("fo:page-width", "28cm"),
			("fo:page-height", "15.75cm"),
			("style:print-orientation", "landscape"),
			("fo:margin-top", "0cm"), ("fo:margin-bottom", "0cm"),
			("fo:margin-left", "0cm"), ("fo:margin-right", "0cm"),
		]);
		res!(out.close("style:page-layout"));
		out.empty("style:style", &[("style:name", "dp1"), ("style:family", "drawing-page")]);
		res!(out.close("office:automatic-styles"));
	}
	out.open("office:styles", &[]);
	// A quotation is indented and italic, which is the one thing a reader has no default for that
	// the document tree can actually carry.
	out.open("style:style", &[
		("style:name", "Quotations"),
		("style:family", "paragraph"),
		("style:parent-style-name", "Standard"),
	]);
	res!(out.close("style:style"));
	out.open("style:style", &[
		("style:name", "Preformatted_20_Text"),
		("style:display-name", "Preformatted Text"),
		("style:family", "paragraph"),
		("style:parent-style-name", "Standard"),
	]);
	res!(out.close("style:style"));
	// The three text styles the writers apply to a span. **A style name a document does not DEFINE
	// is dropped by the reader**, span and all: without these, every bold word written here arrived
	// in LibreOffice as plain text, and it was the round trip through it that showed so.
	for (name, display, weight, style, font) in [
		("Strong_20_Emphasis", "Strong Emphasis", Some("bold"), None, None),
		("Emphasis", "Emphasis", None, Some("italic"), None),
		("Source_20_Text", "Source Text", None, None, Some("Liberation Mono")),
	] {
		out.open("style:style", &[
			("style:name", name),
			("style:display-name", display),
			("style:family", "text"),
		]);
		let mut props: Vec<(&str, &str)> = Vec::new();
		if let Some(w) = weight {
			props.push(("fo:font-weight", w));
			props.push(("style:font-weight-asian", w));
			props.push(("style:font-weight-complex", w));
		}
		if let Some(i) = style {
			props.push(("fo:font-style", i));
			props.push(("style:font-style-asian", i));
			props.push(("style:font-style-complex", i));
		}
		if let Some(f) = font {
			props.push(("style:font-name", f));
			props.push(("fo:font-family", f));
		}
		out.empty("style:text-properties", &props);
		res!(out.close("style:style"));
	}
	res!(out.close("office:styles"));
	if slides {
		out.open("office:master-styles", &[]);
		out.empty("style:master-page", &[
			("style:name", "Default"),
			("style:page-layout-name", "PM1"),
			("draw:style-name", "dp1"),
		]);
		res!(out.close("office:master-styles"));
	}
	res!(out.close("office:document-styles"));
	out.finish()
}

/// The `meta.xml` every package carries.
///
/// It names the generator and nothing else. No date: a document written twice from the same source
/// must give the same bytes, and a timestamp is the one field that guarantees it will not.
pub fn meta(media: &str) -> Outcome<String> {
	let mut out = Out::declared();
	out.open("office:document-meta", &[
		("xmlns:office", NS_OFFICE),
		("xmlns:meta", "urn:oasis:names:tc:opendocument:xmlns:meta:1.0"),
		("office:version", VERSION),
		("office:mimetype", media),
	]);
	out.open("office:meta", &[]);
	out.leaf("meta:generator", &[], "Hematite/fe2o3_file");
	res!(out.close("office:meta"));
	res!(out.close("office:document-meta"));
	out.finish()
}
