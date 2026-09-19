//! The document typefaces the demos set: Libertinus Serif, embedded so a build renders identically
//! anywhere.
//!
//! Libertinus is the maintained descendant of Linux Libertine -- the libre successor to Times, and the
//! face of the Wikipedia wordmark. It is chosen for the body against Latin Modern Math in the maths, so
//! prose and equations are two distinct, well-provenanced open families rather than one: a serif book
//! text beside the Computer Modern a reader knows from mathematics. Both are set out under permissive
//! licences carried beside the font files (`LibertinusSerif-OFL.txt`, `latinmodern-math-GUST-LICENSE.txt`).

use crate::vfs;

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

/// A loaded display-face family: the weight/slant variants found beside the book for one named face,
/// each optional since a family may ship only a Regular. Resolution falls back toward Regular when a
/// requested weight or slant has no file, rather than failing.
#[derive(Clone, Default)]
struct Family {
	regular:		Option<Arc<Font>>,
	bold:			Option<Arc<Font>>,
	italic:			Option<Arc<Font>>,
	bold_italic:	Option<Arc<Font>>,
}

impl Family {
	/// The best available variant for a requested weight and slant: the exact match where present, then a
	/// near relative, ending at whatever the family does hold. `None` only for a family that loaded nothing.
	fn pick(&self, bold: bool, italic: bool) -> Option<&Arc<Font>> {
		let order: [&Option<Arc<Font>>; 4] = match (bold, italic) {
			(true, true)	=> [&self.bold_italic, &self.bold, &self.italic, &self.regular],
			(true, false)	=> [&self.bold, &self.bold_italic, &self.regular, &self.italic],
			(false, true)	=> [&self.italic, &self.bold_italic, &self.regular, &self.bold],
			(false, false)	=> [&self.regular, &self.italic, &self.bold, &self.bold_italic],
		};
		order.into_iter().flatten().next()
	}

	/// Does the family hold the exact variant requested, with no fall-back?
	fn has_exact(&self, bold: bool, italic: bool) -> bool {
		match (bold, italic) {
			(true, true)	=> self.bold_italic.is_some(),
			(true, false)	=> self.bold.is_some(),
			(false, true)	=> self.italic.is_some(),
			(false, false)	=> self.regular.is_some(),
		}
	}

	fn is_empty(&self) -> bool {
		self.regular.is_none() && self.bold.is_none() && self.italic.is_none() && self.bold_italic.is_none()
	}
}

/// A document's named heading display faces, each family loaded once from the book's own font directory
/// across its weight and slant variants. The theme names a heading face by family (`"Graystroke"`,
/// `"Radley"`); the renderer resolves that name -- with the heading level's weight and slant -- through
/// this to a loaded [`Font`], falling back to a role face from the reading set when the name has no file.
/// A name with no `<name>-Regular.{ttf,otf}` beside the book is simply absent, so a theme that names the
/// body family (or a face the tree does not ship) renders in the body role exactly as before -- which is
/// what keeps a document naming no distinct display face byte-identical.
#[derive(Clone, Default)]
pub struct FaceResolver {
	families:	HashMap<String, Family>,
}

impl FaceResolver {
	/// Loads each named face's `<name>-Regular/Bold/Italic/BoldItalic.{ttf,otf}` files that exist under
	/// `dir`. A name with no file at all, or a variant whose file will not parse, is left out rather than
	/// failing the load, so a missing display face degrades to the body role rather than stopping the render.
	pub fn load(dir: &Path, names: &[String]) -> Self {
		let mut families: HashMap<String, Family> = HashMap::new();
		for name in names {
			if name.is_empty() || families.contains_key(name) {
				continue;
			}
			let mut fam = Family::default();
			load_variant(dir, name, "Regular", &mut fam.regular);
			load_variant(dir, name, "Bold", &mut fam.bold);
			load_variant(dir, name, "Italic", &mut fam.italic);
			load_variant(dir, name, "BoldItalic", &mut fam.bold_italic);
			if !fam.is_empty() {
				families.insert(name.clone(), fam);
			}
		}
		Self { families }
	}

	/// The upright regular face for `name` (or its nearest available variant), or `None` when the book
	/// ships no file for the name -- the signal for the renderer to fall back to a role face. Kept for
	/// callers that want the base face; a weighted heading uses [`FaceResolver::resolve_weighted`].
	pub fn resolve(&self, name: &str) -> Option<&Arc<Font>> {
		self.families.get(name).and_then(|f| f.pick(false, false))
	}

	/// The face for `name` at a requested weight and slant, falling back toward Regular when the exact
	/// variant has no file. `None` when the name has no file at all.
	pub fn resolve_weighted(&self, name: &str, bold: bool, italic: bool) -> Option<&Arc<Font>> {
		self.families.get(name).and_then(|f| f.pick(bold, italic))
	}

	/// Does `name` load at least one file? A name that does but lacks a requested weight/slant still
	/// resolves (to Regular); this only distinguishes a named-but-absent face from a loaded one.
	pub fn resolves(&self, name: &str) -> bool {
		self.families.contains_key(name)
	}

	/// Does `name` hold the exact weight/slant variant, with no fall-back to Regular? Used to record a note
	/// when a heading asks for a variant the book does not ship.
	pub fn has_variant(&self, name: &str, bold: bool, italic: bool) -> bool {
		self.families.get(name).map_or(false, |f| f.has_exact(bold, italic))
	}

	/// Does this hold no loaded face? A book naming only its body family resolves nothing.
	pub fn is_empty(&self) -> bool {
		self.families.is_empty()
	}
}

/// Loads one weight/slant variant of a named face -- `<name>-<suffix>.ttf` or `.otf` under `dir` -- into
/// `slot`, leaving it `None` when neither file exists or the one present will not parse.
fn load_variant(dir: &Path, name: &str, suffix: &str, slot: &mut Option<Arc<Font>>) {
	for ext in ["ttf", "otf"] {
		let path = dir.join(fmt!("{}-{}.{}", name, suffix, ext));
		if vfs::is_file(&path) {
			if let Ok(font) = font_from_file(&path) {
				*slot = Some(font);
			}
			return;
		}
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
	let bytes = match vfs::read(path) {
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
