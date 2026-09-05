//! The document typefaces the demos set: Libertinus Serif, embedded so a build renders identically
//! anywhere.
//!
//! Libertinus is the maintained descendant of Linux Libertine -- the libre successor to Times, and the
//! face of the Wikipedia wordmark. It is chosen for the body against Latin Modern Math in the maths, so
//! prose and equations are two distinct, well-provenanced open families rather than one: a serif book
//! text beside the Computer Modern a reader knows from mathematics. Both are set out under permissive
//! licences carried beside the font files (`LibertinusSerif-OFL.txt`, `latinmodern-math-GUST-LICENSE.txt`).

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_font::{
	font::Font,
	set::FontSet,
};

const SERIF:		&[u8] = include_bytes!("../fonts/LibertinusSerif-Regular.otf");
const BOLD:			&[u8] = include_bytes!("../fonts/LibertinusSerif-Bold.otf");
const ITALIC:		&[u8] = include_bytes!("../fonts/LibertinusSerif-Italic.otf");
const BOLD_ITALIC:	&[u8] = include_bytes!("../fonts/LibertinusSerif-BoldItalic.otf");
const MONO:			&[u8] = include_bytes!("../fonts/LibertinusMono-Regular.otf");

/// The Libertinus Serif reading set: one face per role. Libertinus covers the Latin, punctuation and
/// figures a set document needs, so each role is a single face; a symbol a document reaches for that the
/// family lacks would fall to the not-defined glyph, which the demos do not hit.
pub fn libertinus() -> Outcome<FontSet> {
	Ok(FontSet::new(
		res!(Font::new(SERIF.to_vec())),
		res!(Font::new(BOLD.to_vec())),
		res!(Font::new(ITALIC.to_vec())),
		res!(Font::new(BOLD_ITALIC.to_vec())),
		res!(Font::new(MONO.to_vec())),
	))
}
