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

use std::path::Path;

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

/// Reads one face from a file, naming the path when the read fails so a missing font is obvious.
fn face_from_file(path: &Path) -> Outcome<Font> {
	let bytes = match std::fs::read(path) {
		Ok(b)	=> b,
		Err(e)	=> return Err(err!(e, "Could not read the font file {:?}.", path; File, Read)),
	};
	Font::new(bytes)
}

/// Builds a reading set from five explicit face files, one per role. A book supplies its own faces by
/// path -- Libertinus lives in the book's assets tree, not fontconfig, so the set is loaded at run
/// time from the paths the book uses rather than the faces embedded in the crate.
pub fn from_files(
	body:		&Path,
	bold:		&Path,
	italic:		&Path,
	bold_italic:	&Path,
	mono:		&Path,
)
	-> Outcome<FontSet>
{
	Ok(FontSet::new(
		res!(face_from_file(body)),
		res!(face_from_file(bold)),
		res!(face_from_file(italic)),
		res!(face_from_file(bold_italic)),
		res!(face_from_file(mono)),
	))
}

/// The Libertinus Serif reading set loaded by path from a book's Libertinus directory (the folder
/// holding `LibertinusSerif-*.otf` and `LibertinusMono-Regular.otf`). This is the book body face:
/// Typst's own default, so prose set here matches the oracle's.
pub fn libertinus_from_dir(dir: &Path) -> Outcome<FontSet> {
	from_files(
		&dir.join("LibertinusSerif-Regular.otf"),
		&dir.join("LibertinusSerif-Bold.otf"),
		&dir.join("LibertinusSerif-Italic.otf"),
		&dir.join("LibertinusSerif-BoldItalic.otf"),
		&dir.join("LibertinusMono-Regular.otf"),
	)
}
