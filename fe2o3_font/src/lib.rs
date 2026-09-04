//! The font layer: parse a font file, shape a string, measure it, and draw a glyph's outline.
//!
//! This crate is the only place third-party code touches the type-setting stack. Shaping is handed
//! to `harfrust` and glyph outlines to `skrifa`. Everything either of them hands back is turned into
//! this crate's own types at once, so the rest of the stack never sees them, and either could be
//! replaced without a line changing outside this crate.
//!
//! # A font is a chain, not a file
//!
//! No typeface holds every character. A face chosen for how it reads will not have an arrow, a
//! summation sign or a word of Arabic in it, and a face that has all of those was chosen for its
//! coverage rather than for how it reads. Asking one file to be both is what produces either a page
//! full of tofu or a page that looks like a fallback.
//!
//! So a [`Font`](crate::font::Font) here is not a file: it is a chain of [`Face`](crate::face::Face)s,
//! the first that can draw a character drawing it. That is what a browser does, and it is what lets
//! the face at the head of the chain be chosen purely for how it reads -- the chain behind it means a
//! change of face can never cost coverage. Each [`Glyph`](crate::shape::Glyph) remembers which face
//! drew it, so the chain costs a caller nothing: it still asks for a role and gets a run back.
//!
//! # The shape of the crate
//!
//! - [`shape`] holds the shaped-text types a caller reads back: [`Dir`](crate::shape::Dir),
//!   [`Glyph`](crate::shape::Glyph) and [`Run`](crate::shape::Run).
//! - [`face`] holds one typeface, [`Face`](crate::face::Face), with its [`Role`](crate::face::Role)
//!   and [`Metrics`](crate::face::Metrics).
//! - [`font`] holds the fallback chain, [`Font`](crate::font::Font).
//! - [`set`] holds a set of chains, one per role, [`FontSet`](crate::set::FontSet), and the faces
//!   the crate carries embedded.

pub mod face;
pub mod font;
pub mod prelude;
pub mod set;
pub mod shape;
