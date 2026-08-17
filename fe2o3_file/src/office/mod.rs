//! The Office formats: a ZIP of XML, in six flavours.
//!
//! `.docx`, `.xlsx` and `.pptx` from Microsoft, and `.odt`, `.ods` and `.odp` from OpenDocument, are
//! the same shape -- an archive whose members are XML parts, one of which is the content and the rest
//! of which say what the content means. What differs is the vocabulary, and Microsoft's is roughly
//! three times the size.
//!
//! # Three verbs, and they are not the same job
//!
//! **Creating** a document is the easy one and it goes through [`oxedyne_fe2o3_text::doc`], the neutral tree. You
//! control every byte, there is no round trip to preserve, and the reader does the layout when it
//! opens the file.
//!
//! **Reading** a document also goes through [`oxedyne_fe2o3_text::doc`], because what a reader wants is the prose.
//! What the tree cannot carry is not lost, it is simply not prose, and a reading view says what it
//! did not draw rather than pretending it drew everything.
//!
//! **Editing** a document does not go through [`oxedyne_fe2o3_text::doc`] and must never be made to.
//! [`oxedyne_fe2o3_text::doc::policy`] argues that the tree deliberately cannot carry markup it has no node for,
//! and that is exactly right for the first two verbs and exactly wrong for the third: everything the
//! tree cannot represent is everything an edit through it would silently destroy -- the comments, the
//! bookmarks, the tracked changes, the content controls, the custom XML, the theme, the tab stops.
//! Editing goes through [`oxedyne_fe2o3_text::xml`], which splices bytes and copies the rest, over
//! [`crate::zip`], which copies every member nobody touched.
//!
//! # What is deliberately not here
//!
//! No layout engine: a reading view maps to HTML and lets the browser shape and break the text, and
//! says so, rather than claiming to match what Word prints. No macro execution, ever -- a macro part
//! is copied through untouched and its presence is *said*. No authoring of tracked changes or
//! comments, which are read and displayed and not written, because `w:ins` and `w:del` subtly wrong
//! corrupts a legal review. No re-rendering of charts. No conversion between the two families, which
//! is re-serialisation through a lossy model wearing the word "export".
//!
//! [Written entirely with AI](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

pub mod deck;
pub mod docx;
pub mod edit;
pub mod odf;
pub mod opc;
pub mod pptx;
pub mod sheet;
pub mod xlsx;
