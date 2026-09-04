//! The font layer: parse a font file, shape a string, measure it, and draw a glyph's outline.
//!
//! Extracted from Kiln. Shaping is handed to `harfrust` and glyph outlines to `skrifa`; everything
//! either hands back is turned into this crate's own types at once, so the rest of the stack never
//! sees them and either could be replaced without a line changing outside this crate.
//!
//! # A font is a chain, not a file
//!
//! No typeface holds every character: a face chosen for how it reads lacks the arrows, the summation
//! signs and the Arabic that a face chosen for coverage carries, and asking one file to be both gives
//! a page of tofu or a page that reads like a fallback. So a [`Font`](crate::font::Font) here is not a
//! file but a chain of [`Face`](crate::face::Face)s, the first that can draw a character drawing it,
//! as a browser does. The head of the chain can then be chosen purely for how it reads -- the chain
//! behind it means a change of face never costs coverage -- and each [`Glyph`](crate::shape::Glyph)
//! remembers which face drew it, so the caller still asks for a role and gets a run back.

pub mod face;
pub mod font;
pub mod prelude;
pub mod set;
pub mod shape;
