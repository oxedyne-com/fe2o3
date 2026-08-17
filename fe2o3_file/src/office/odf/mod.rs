//! OpenDocument: `.odt`, `.ods` and `.odp`, which are one format wearing three media types.
//!
//! # Simpler than OOXML, and in one specific way
//!
//! There are no relationship parts. An OOXML package says what its parts are in
//! `[Content_Types].xml` and how they refer to one another in a `.rels` file beside each one; an
//! OpenDocument package has a single `META-INF/manifest.xml` listing every member, and the content
//! refers to things by path. That removes the whole class of bug where a body names an `r:id` that
//! the relationships part does not declare.
//!
//! It is also one `content.xml` rather than one part per sheet or per slide, which makes a large
//! spreadsheet a single large part -- the trade the other way.
//!
//! # `mimetype` is the one rule that must not be broken
//!
//! The first member of the archive must be named `mimetype`, must hold the media type as plain text,
//! and must be **stored uncompressed**. That is what lets a reader name the file from its opening
//! bytes, which is what [`oxedyne_fe2o3_stds::media`] now does. A package that writes it anywhere
//! else, or deflates it, is a file every reader calls a ZIP.
//!
//! [`Zip::set_first`](crate::zip::Zip::set_first) exists for this and is used by all three writers
//! here. It is not a detail that can be left to whoever writes the next one.
//!
//! # The vocabulary is flat where OOXML's is nested
//!
//! A paragraph is `<text:p>` and a heading is `<text:h text:outline-level="2">` -- the level is an
//! attribute rather than a style name, so nothing has to resolve a style to know a heading is one.
//! That is the reverse of WordprocessingML and it is the easier direction.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

pub mod pkg;
pub mod sheet;
pub mod slides;
pub mod text;

//// The namespace URIs, each fixed by the OpenDocument specification.
//
// A prefix does not always name its own URI: `fo:` is XSL formatting objects, `draw:` is drawing,
// `svg:` is OpenDocument's compatible form of somebody else's, and `xlink:` is W3C's own.
pub const NS_OFFICE: &str = "urn:oasis:names:tc:opendocument:xmlns:office:1.0";
pub const NS_TEXT: &str = "urn:oasis:names:tc:opendocument:xmlns:text:1.0";
pub const NS_TABLE: &str = "urn:oasis:names:tc:opendocument:xmlns:table:1.0";
pub const NS_STYLE: &str = "urn:oasis:names:tc:opendocument:xmlns:style:1.0";
pub const NS_FO: &str = "urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0";
pub const NS_DRAW: &str = "urn:oasis:names:tc:opendocument:xmlns:drawing:1.0";
pub const NS_PRES: &str = "urn:oasis:names:tc:opendocument:xmlns:presentation:1.0";
pub const NS_SVG: &str = "urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0";
pub const NS_XLINK: &str = "http://www.w3.org/1999/xlink";
pub const NS_MANIFEST: &str = "urn:oasis:names:tc:opendocument:xmlns:manifest:1.0";

/// The `of:` namespace, which is what a formula's `of:=` prefix refers to.
///
/// **A `.ods` that writes `table:formula="of:=..."` without binding this is a spreadsheet whose every
/// formula fails.** LibreOffice does not ignore the unbound prefix -- it fails to parse the formula,
/// RECALCULATES the cell, and writes `Err:510` over the value that was stored there. So the missing
/// declaration does not merely lose the formula; it destroys the number beside it, which is the one
/// thing this crate promises not to do.
///
/// The URI is `...xmlns:of:1.2` and not `...formula:1.0`, which is the plausible guess and is wrong.
pub const NS_OF: &str = "urn:oasis:names:tc:opendocument:xmlns:of:1.2";

// The `number:` namespace, which carries a data style -- what makes a number a date.
pub const NS_NUMBER: &str = "urn:oasis:names:tc:opendocument:xmlns:datastyle:1.0";
