//! WordprocessingML: the `.docx` half of the Office formats.
//!
//! # What a `.docx` is, in one paragraph
//!
//! A ZIP holding `[Content_Types].xml`, which says what each part is; `_rels/.rels`, which points at
//! the document part; `word/document.xml`, which is the content; and `word/_rels/document.xml.rels`,
//! which points at everything the content refers to. Add `word/styles.xml` and headings mean
//! something. Everything else -- the theme, the settings, the font table, the people, the comments --
//! is optional, and a document that omits them opens.
//!
//! Inside `word/document.xml` the nesting is `w:document > w:body > w:p > w:r > w:t`: a body of
//! paragraphs, each of runs, each of text. A run is the unit formatting applies to, which is why
//! bold text in the middle of a sentence splits a paragraph into three runs.
//!
//! # Creating comes first, and is not the same problem as editing
//!
//! [`write`] builds a document from the neutral tree in [`oxedyne_fe2o3_text::doc`]. It is the easy direction:
//! every byte is ours, there is nothing to preserve, and Word lays the result out when it opens it.
//! Editing a document somebody else wrote is a different problem with a different answer -- see
//! [`crate::office`] -- and sharing code between them is how the second one gets the first one's
//! assumptions.

pub mod edit;
pub mod parts;
pub mod read;
pub mod write;

pub use read::read;
pub use write::write;

/// The WordprocessingML namespace, which nearly every element of a document is in.
pub const NS_W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";

/// The width of an A4 page in twips, which is a twentieth of a point.
pub const PAGE_W: u32 = 11906;
/// The height of an A4 page in twips.
pub const PAGE_H: u32 = 16838;
/// The margin on each side, in twips: two centimetres.
pub const MARGIN: u32 = 1134;
/// The width a table has to lay itself out in.
pub const TEXT_W: u32 = PAGE_W - 2 * MARGIN;

/// The `w:numId` of the bulleted list definition. See [`parts::numbering`].
pub const NUM_BULLET: &str = "1";
/// The `w:numId` of the numbered list definition.
pub const NUM_ORDERED: &str = "2";
