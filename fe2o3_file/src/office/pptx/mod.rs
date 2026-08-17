//! PresentationML: the `.pptx` third of the Microsoft formats.
//!
//! # The heaviest skeleton of the three, and the reason is structural
//!
//! A `.docx` opens with four parts and a `.xlsx` with six. A `.pptx` needs **a slide master, a slide
//! layout and a theme before it can hold a single slide** -- a slide points at a layout, a layout
//! points at a master, and a master points at a theme, and PowerPoint refuses the file if any link
//! in that chain is missing. The theme in particular must carry a complete format scheme: three fill
//! styles, three line styles, three effect styles and three background fills, whether or not anything
//! uses them.
//!
//! None of that is optional and none of it is content. It is written once, in [`parts`], generated
//! rather than held as a literal blob so the repetition is a loop instead of a place for a typo.
//!
//! # Create, read, show. Not edit.
//!
//! [`write`] builds a deck from [`crate::office::deck`]. [`read`] takes the words back out, which is
//! what a reading view and a model both want.
//!
//! **Deck editing is deliberately absent.** Not because it is hard -- it is the same splice-and-copy
//! the other formats would use -- but because a slide is a position on a canvas, and an edit that
//! changed the words without knowing the geometry would produce a slide with text over the top of
//! other text. That is a failure a reader sees and an editor cannot check for. A deck is also the
//! least useful thing an agent generates, so the value on the other side of that risk is small.
//!
//! That last sentence is why [`write`] is the one Office verb behind a cargo feature, `deck-write`,
//! which is on by default. [`read`] is not behind it: a reading view offers six formats, and losing
//! one of them would change what a user already has.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

// The skeleton generator serves `write` and nothing else, so it goes with the feature.
#[cfg(feature = "deck-write")]
pub mod parts;
pub mod read;
pub mod write;

pub use read::read;
pub use write::write;

/// The PresentationML namespace.
pub const NS_P: &str = "http://schemas.openxmlformats.org/presentationml/2006/main";
/// The DrawingML namespace, which every shape and every run of text on a slide is in.
pub const NS_A: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";

//// Slide geometry, in EMU -- an EMU is 1/914,400 of an inch.
pub const SLIDE_W: i64 = 12_192_000;	// sixteen by nine
pub const SLIDE_H: i64 = 6_858_000;
pub const MARGIN: i64 = 457_200;	// half an inch
pub const TITLE_H: i64 = 1_143_000;	// where the title ends and the body begins
