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

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ NAMED DISPLAY-FACE RESOLVER                                                │
// └───────────────────────────────────────────────────────────────────────────┘

/// A document's named heading display faces, each loaded once from the book's own font directory. The
/// theme names a heading face by family (`"Graystroke"`, `"Radley"`); the renderer resolves that name
/// through this to a loaded [`Font`], falling back to a role face from the reading set when the name has
/// no file. A name with no `<name>-Regular.{ttf,otf}` beside the book is simply absent, so a theme that
/// names the body family (or a face the tree does not ship) renders in the body role exactly as before --
/// which is what keeps a document naming no distinct display face byte-identical.
#[derive(Clone, Default)]
pub struct FaceResolver {
	faces:	HashMap<String, Arc<Font>>,
}

impl FaceResolver {
	/// Loads each named face that has a `<name>-Regular.ttf` or `.otf` file under `dir`. A name with no
	/// such file, or one whose file will not parse, is left out rather than failing the load, so a missing
	/// display face degrades to the body role rather than stopping the render.
	pub fn load(dir: &Path, names: &[String]) -> Self {
		let mut faces: HashMap<String, Arc<Font>> = HashMap::new();
		for name in names {
			if name.is_empty() || faces.contains_key(name) {
				continue;
			}
			for ext in ["ttf", "otf"] {
				let path = dir.join(fmt!("{}-Regular.{}", name, ext));
				if path.is_file() {
					if let Ok(font) = font_from_file(&path) {
						faces.insert(name.clone(), font);
					}
					break;
				}
			}
		}
		Self { faces }
	}

	/// The loaded face for `name`, or `None` when the theme named a face the book ships no file for -- the
	/// signal for the renderer to fall back to a role face.
	pub fn resolve(&self, name: &str) -> Option<&Arc<Font>> {
		self.faces.get(name)
	}

	/// Does this hold no loaded face? A book naming only its body family resolves nothing.
	pub fn is_empty(&self) -> bool {
		self.faces.is_empty()
	}
}

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

/// One face loaded from a file as a shareable handle, for a role outside the five-face reading set --
/// a heading face a book supplies by path (Radley, say). It is shaped through the `Solo` path the way
/// the maths font is. The error names the path, so the caller can choose to fall back rather than fail.
pub fn font_from_file(path: &Path) -> Outcome<std::sync::Arc<Font>> {
	Ok(std::sync::Arc::new(res!(face_from_file(path))))
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

#[cfg(test)]
mod tests {
	use super::*;

	/// The resolver loads a named face from a directory holding its `<name>-Regular.otf`, and returns
	/// `None` for a name with no file, so the renderer falls back to a role face rather than failing.
	#[test]
	fn face_resolver_loads_a_named_face() {
		let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("fonts");
		let r = FaceResolver::load(&dir, &["LibertinusSerif".to_string(), "NoSuchFace".to_string()]);
		assert!(r.resolve("LibertinusSerif").is_some(), "an existing face file must resolve to a font");
		assert!(r.resolve("NoSuchFace").is_none(), "a name with no file must not resolve");
		assert!(!r.is_empty());
	}
}
