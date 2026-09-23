//! The document typefaces: Libertinus Serif for prose and New Computer Modern Math for equations, embedded
//! so a build renders identically anywhere, and any family a document names from the fonts it supplies.
//!
//! Libertinus is the maintained descendant of Linux Libertine -- the libre successor to Times, and the
//! face of the Wikipedia wordmark. New Computer Modern Math is the Computer Modern a reader knows from
//! mathematics. Both are Typst's own defaults, so a document naming neither sets as the oracle sets it,
//! and both are carried under permissive licences beside the font files (`LibertinusSerif-OFL.txt`,
//! `NewCMMath-GUST-LICENSE.txt`).
//!
//! 2026-09-23: the maths face moved from Latin Modern Math to New Computer Modern Math, Typst's default,
//! when Daimond dropped the typst.ts vendor copy that had been its only source.

use crate::vfs;

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_font::{
	face::{
		Face,
		FaceInfo,
	},
	font::Font,
	set::FontSet,
};

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ NAMED FACE RESOLVER                                                        │
// └───────────────────────────────────────────────────────────────────────────┘

/// One face of a named family: the parsed font, for a heading drawn in it alone, and the file's bytes, so
/// a reading set can chain the face ahead of its fall-backs (a chain owns its faces, so it parses its own).
#[derive(Clone)]
struct Variant {
	font:	Arc<Font>,
	bytes:	Arc<Vec<u8>>,
}

/// A loaded family: the weight/slant variants found for one named face, each optional since a family may
/// ship only a Regular. Resolution falls back toward Regular when a requested weight or slant has no file,
/// rather than failing -- Typst's nearest-variant choice, with no synthesised bold or slant.
#[derive(Clone, Default)]
struct Family {
	regular:		Option<Variant>,
	bold:			Option<Variant>,
	italic:			Option<Variant>,
	bold_italic:	Option<Variant>,
}

impl Family {
	/// The best available variant for a requested weight and slant: the exact match where present, then a
	/// near relative, ending at whatever the family does hold. `None` only for a family that loaded nothing.
	fn pick(&self, bold: bool, italic: bool) -> Option<&Variant> {
		let order: [&Option<Variant>; 4] = match (bold, italic) {
			(true, true)	=> [&self.bold_italic, &self.bold, &self.italic, &self.regular],
			(true, false)	=> [&self.bold, &self.bold_italic, &self.regular, &self.italic],
			(false, true)	=> [&self.italic, &self.bold_italic, &self.regular, &self.bold],
			(false, false)	=> [&self.regular, &self.italic, &self.bold, &self.bold_italic],
		};
		order.into_iter().flatten().next()
	}

	/// Does the family hold the exact variant requested, with no fall-back?
	fn has_exact(&self, bold: bool, italic: bool) -> bool {
		self.slot(bold, italic).is_some()
	}

	fn slot(&self, bold: bool, italic: bool) -> &Option<Variant> {
		match (bold, italic) {
			(true, true)	=> &self.bold_italic,
			(true, false)	=> &self.bold,
			(false, true)	=> &self.italic,
			(false, false)	=> &self.regular,
		}
	}

	fn slot_mut(&mut self, bold: bool, italic: bool) -> &mut Option<Variant> {
		match (bold, italic) {
			(true, true)	=> &mut self.bold_italic,
			(true, false)	=> &mut self.bold,
			(false, true)	=> &mut self.italic,
			(false, false)	=> &mut self.regular,
		}
	}

	fn is_empty(&self) -> bool {
		self.regular.is_none() && self.bold.is_none() && self.italic.is_none() && self.bold_italic.is_none()
	}
}

/// One font file found under the document's font directory, known by what it declares about itself.
#[derive(Clone)]
struct Declared {
	info:	FaceInfo,
	bytes:	Arc<Vec<u8>>,
}

/// Every font file a document was given -- the files under its font directory, which is where a wasm
/// project's injected `fonts` are routed -- indexed by the family each declares in its own name table. This
/// is what a `font: "Name"` is matched against, as Typst matches it: by the designer's family name,
/// ignoring case, whatever the file is called.
#[derive(Clone, Default)]
struct FontLibrary {
	faces:	Vec<Declared>,
}

impl FontLibrary {
	/// Reads every `.ttf`/`.otf` file beneath `dir`, at any depth, after the faces the crate embeds -- so a
	/// document may name an embedded family without supplying it, as Typst's own embedded fonts need no
	/// file. A file that will not parse, or declares no family, is left out: it cannot answer to any name,
	/// so it can neither match nor mislead.
	fn scan(dir: &Path) -> Self {
		let mut faces: Vec<Declared> = Vec::new();
		for bytes in EMBEDDED {
			if let Ok(info) = FaceInfo::read(bytes) {
				faces.push(Declared { info, bytes: Arc::new(bytes.to_vec()) });
			}
		}
		for path in vfs::list_files(dir) {
			let ext = path.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase());
			if !matches!(ext.as_deref(), Some("ttf") | Some("otf")) {
				continue;
			}
			let bytes = match vfs::read(&path) {
				Ok(b)	=> b,
				Err(_)	=> continue,
			};
			if let Ok(info) = FaceInfo::read(&bytes) {
				faces.push(Declared { info, bytes: Arc::new(bytes) });
			}
		}
		Self { faces }
	}

	/// The distinct families the directory declares, sorted, for a diagnostic that names what was on offer.
	fn families(&self) -> Vec<String> {
		let mut out: Vec<String> = self.faces.iter().map(|d| d.info.family.clone()).collect();
		out.sort_by_key(|f| f.to_lowercase());
		out.dedup_by(|a, b| same_family(a, b));
		out
	}

	/// The family `name` names (ignoring case), each weight/slant slot holding the declared face nearest its
	/// canonical weight -- 400 for the upright and italic slots, 700 for the bold ones -- with a face of 600
	/// or heavier counting as bold. Empty when no file declares the family.
	fn family(&self, name: &str) -> Outcome<Family> {
		let mut fam = Family::default();
		let mut best: [Option<u16>; 4] = [None; 4];	// distance to the slot's canonical weight
		for d in &self.faces {
			if !same_family(&d.info.family, name) {
				continue;
			}
			let bold	= d.info.weight >= 600;
			let italic	= d.info.italic;
			let target	= if bold { 700i32 } else { 400i32 };
			let dist	= (d.info.weight as i32 - target).unsigned_abs() as u16;
			let i		= (bold as usize) * 2 + italic as usize;
			if best[i].map_or(true, |b| dist < b) {
				best[i] = Some(dist);
				let font = Arc::new(res!(Font::new(d.bytes.as_ref().clone())));
				*fam.slot_mut(bold, italic) = Some(Variant { font, bytes: d.bytes.clone() });
			}
		}
		Ok(fam)
	}
}

/// Do two family names name the same family? Typst matches a family ignoring case; this also ignores
/// white space, since one family is written both ways in the wild -- the New Computer Modern files declare
/// `NewComputerModern Math` where Typst's own list and every document write `New Computer Modern Math`.
pub fn same_family(a: &str, b: &str) -> bool {
	let fold = |s: &str| -> String { s.chars().filter(|c| !c.is_whitespace()).flat_map(|c| c.to_lowercase()).collect() };
	fold(a) == fold(b)
}

/// The families the crate embeds, which a document may name without supplying a file: the Libertinus
/// reading set and the maths face. Normalised to the spelling Typst lists them under.
pub fn embedded_families() -> Vec<String> {
	vec![
		"Libertinus Serif".to_string(),
		"Libertinus Mono".to_string(),
		"New Computer Modern Math".to_string(),
	]
}

/// The family a font file declares in its own name table: the name a document's `font:` is matched
/// against, whatever the file is called. A caller supplying fonts can compare this, through
/// [`same_family`], with the families a document names before it compiles.
pub fn declared_family(bytes: &[u8]) -> Outcome<String> {
	Ok(res!(FaceInfo::read(bytes)).family)
}

/// Is `name` the family of the embedded reading set? A document naming it asks for what it already has,
/// so it is resolved to that set rather than looked up, which keeps such a document byte-identical.
fn is_embedded_family(name: &str) -> bool {
	same_family(name, "Libertinus Serif")
}

/// A document's named faces: its heading display faces and its body families, each loaded once from the
/// document's own font directory. A heading face is named by family (`"Graystroke"`, `"Radley"`) and found
/// by its `<name>-Regular/Bold/Italic/BoldItalic.{ttf,otf}` files, else by the family its files declare; a
/// heading name with no file is simply absent, so a theme that names the body family (or a face the tree
/// does not ship) renders in the body role exactly as before. A body family list (`#set text(font: ...)`)
/// is required rather than hoped for: [`FaceResolver::require`] fails on a family no file declares, and
/// builds the reading set every such list is set in.
#[derive(Clone, Default)]
pub struct FaceResolver {
	families:	HashMap<String, Family>,
	bodies:		HashMap<Vec<String>, Arc<FontSet>>,	// keyed by the list as written; see `body_set`
	embedded:	Option<Arc<FontSet>>,	// the embedded set, built only when a scope names it back
}

impl FaceResolver {
	/// Loads each named face from `dir`: its `<name>-<Variant>.{ttf,otf}` files where they exist, else every
	/// file beneath `dir` declaring that family. A name matching neither is left out rather than failing the
	/// load, so a missing display face degrades to the body role rather than stopping the render. The
	/// directory is only scanned when a name has no file of its own, so a document naming no face -- or only
	/// faces its tree ships by filename -- reads exactly the files it read before.
	pub fn load(dir: &Path, names: &[String]) -> Self {
		let mut families: HashMap<String, Family> = HashMap::new();
		let mut library: Option<FontLibrary> = None;
		for name in names {
			if name.is_empty() || families.contains_key(name) {
				continue;
			}
			let mut fam = Family::default();
			load_variant(dir, name, "Regular", &mut fam.regular);
			load_variant(dir, name, "Bold", &mut fam.bold);
			load_variant(dir, name, "Italic", &mut fam.italic);
			load_variant(dir, name, "BoldItalic", &mut fam.bold_italic);
			// A heading named after the reading set's own family, with no file of its own beside the book, is
			// set in the reading set's role faces -- the same family -- exactly as before the library existed.
			if fam.is_empty() && !is_embedded_family(name) {
				let lib = library.get_or_insert_with(|| FontLibrary::scan(dir));
				if let Ok(found) = lib.family(name) {
					fam = found;
				}
			}
			if !fam.is_empty() {
				families.insert(name.clone(), fam);
			}
		}
		Self { families, bodies: HashMap::new(), embedded: None }
	}

	/// Typst's missing-family precheck, made a hard error: every family the document names by
	/// `text(font: ...)` -- `bodies`, each a fallback list -- and every heading face it names itself --
	/// `headings` -- must be declared by a file under `dir` (or be the embedded Libertinus Serif), else the
	/// compile fails naming the family and the families that were on offer, rather than setting the text in
	/// a face the author did not choose. Each body list's reading set is built here, once, for
	/// [`FaceResolver::body_set`] to hand out.
	pub fn require(&mut self, dir: &Path, bodies: &[Vec<String>], headings: &[String]) -> Outcome<()> {
		let mut library: Option<FontLibrary> = None;
		for list in bodies {
			// A list naming only the embedded family asks for the embedded set: the one the document already
			// has at its root, and one a scope returning to it needs built.
			if list.iter().all(|n| is_embedded_family(n)) {
				if !list.is_empty() && self.embedded.is_none() {
					self.embedded = Some(Arc::new(res!(libertinus())));
				}
				continue;
			}
			if self.bodies.contains_key(list) {
				continue;
			}
			let mut chosen: Vec<Option<Family>> = Vec::with_capacity(list.len());
			for name in list {
				if is_embedded_family(name) {
					chosen.push(None);	// the embedded face, at its place in the fall-back order
					continue;
				}
				let fam = match self.families.get(name) {
					Some(f)	=> f.clone(),
					None	=> {
						let lib = library.get_or_insert_with(|| FontLibrary::scan(dir));
						res!(lib.family(name))
					},
				};
				if fam.is_empty() {
					let lib = library.get_or_insert_with(|| FontLibrary::scan(dir));
					return Err(missing_family(name, "text(font:)", dir, lib));
				}
				self.families.entry(name.clone()).or_insert_with(|| fam.clone());
				chosen.push(Some(fam));
			}
			let set = res!(reading_set(&chosen));
			self.bodies.insert(list.clone(), Arc::new(set));
		}
		// A body set in another family no longer carries the embedded family in its roles, so a heading
		// named after the embedded family is then loaded as a face of its own.
		let body_moved = bodies.iter().any(|l| !l.iter().all(|n| is_embedded_family(n)));
		for name in headings {
			if name.is_empty() || self.resolves(name) {
				continue;
			}
			if is_embedded_family(name) {
				if body_moved {
					let lib = library.get_or_insert_with(|| FontLibrary::scan(dir));
					let fam = res!(lib.family(name));
					if !fam.is_empty() {
						self.families.insert(name.clone(), fam);
					}
				}
				continue;
			}
			let lib = library.get_or_insert_with(|| FontLibrary::scan(dir));
			return Err(missing_family(name, "heading", dir, lib));
		}
		Ok(())
	}

	/// The reading set a body family list is set in, or `None` for an empty list or one naming only the
	/// embedded family -- the signal to keep the document's own set. A list [`FaceResolver::require`] never saw also
	/// yields `None`; every caller requires first, so that is a construction error, not a fall-back.
	pub fn body_set(&self, families: &[String]) -> Option<Arc<FontSet>> {
		self.bodies.get(families).cloned()
	}

	/// The reading set a scope's `text(font: ...)` puts in force: the list's own set, or the embedded set
	/// for a list naming only the embedded family (or clearing the family). `None` only for a list
	/// [`FaceResolver::require`] never saw -- a construction error the caller reports.
	pub fn scope_set(&self, families: &[String]) -> Outcome<Option<Arc<FontSet>>> {
		if families.iter().all(|n| is_embedded_family(n)) {
			return Ok(Some(match &self.embedded {
				Some(e)	=> e.clone(),
				None	=> Arc::new(res!(libertinus())),
			}));
		}
		Ok(self.body_set(families))
	}

	/// The upright regular face for `name` (or its nearest available variant), or `None` when the book
	/// ships no file for the name -- the signal for the renderer to fall back to a role face. Kept for
	/// callers that want the base face; a weighted heading uses [`FaceResolver::resolve_weighted`].
	pub fn resolve(&self, name: &str) -> Option<&Arc<Font>> {
		self.families.get(name).and_then(|f| f.pick(false, false)).map(|v| &v.font)
	}

	/// The face for `name` at a requested weight and slant, falling back toward Regular when the exact
	/// variant has no file. `None` when the name has no file at all.
	pub fn resolve_weighted(&self, name: &str, bold: bool, italic: bool) -> Option<&Arc<Font>> {
		self.families.get(name).and_then(|f| f.pick(bold, italic)).map(|v| &v.font)
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

/// The hard error for a family no font declares: the family, where it was named, the directory searched
/// and the families that directory does declare, so the fix -- supply the file, or correct the name -- is
/// plain from the message alone.
fn missing_family(name: &str, site: &str, dir: &Path, lib: &FontLibrary) -> Error<ErrTag> {
	let offered = lib.families();
	let offered = if offered.is_empty() { "none".to_string() } else { offered.join(", ") };
	err!("The font family {:?} named by {} is not declared by any font file under {:?}; the families \
		there are: {}. Supply the font (a wasm project passes it in `fonts`) or correct the name.",
		name, site, dir, offered; Missing, Input)
}

/// The reading set for a body family list: each role a chain of the listed families' nearest variant for
/// that role, in list order (`None` standing for the embedded Libertinus face), ending in the embedded
/// face for the role when the list did not name it, so a character none of the listed families draws
/// still reaches ink (Typst's font fall-back). The mono role keeps Libertinus Mono, since Typst sets `raw`
/// in its own face whatever the body family.
fn reading_set(families: &[Option<Family>]) -> Outcome<FontSet> {
	let chain = |bold: bool, italic: bool, embedded: &[u8]| -> Outcome<Font> {
		let mut faces: Vec<Face> = Vec::with_capacity(families.len() + 1);
		let mut has_embedded = false;
		for fam in families {
			match fam {
				Some(f)	=> if let Some(v) = f.pick(bold, italic) {
					faces.push(res!(Face::new(v.bytes.as_ref().clone())));
				},
				None	=> if !has_embedded {
					faces.push(res!(Face::new(embedded.to_vec())));
					has_embedded = true;
				},
			}
		}
		if !has_embedded {
			faces.push(res!(Face::new(embedded.to_vec())));
		}
		Font::chain(faces)
	};
	Ok(FontSet::new(
		res!(chain(false, false, SERIF)),
		res!(chain(true, false, BOLD)),
		res!(chain(false, true, ITALIC)),
		res!(chain(true, true, BOLD_ITALIC)),
		res!(Font::new(MONO.to_vec())),
	))
}

/// Loads one weight/slant variant of a named face -- `<name>-<suffix>.ttf` or `.otf` under `dir` -- into
/// `slot`, leaving it `None` when neither file exists or the one present will not parse.
fn load_variant(dir: &Path, name: &str, suffix: &str, slot: &mut Option<Variant>) {
	for ext in ["ttf", "otf"] {
		let path = dir.join(fmt!("{}-{}.{}", name, suffix, ext));
		if vfs::is_file(&path) {
			if let Ok(bytes) = vfs::read(&path) {
				if let Ok(font) = Font::new(bytes.clone()) {
					*slot = Some(Variant { font: Arc::new(font), bytes: Arc::new(bytes) });
				}
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

// The maths face: New Computer Modern Math, Typst's own default for `math.equation`, so an equation sets
// in the face the oracle sets it in. Its OpenType MATH table drives the maths layout (see `crate::math`).
pub(crate) const MATH:	&[u8] = include_bytes!("../fonts/NewCMMath-Regular.otf");

// Every embedded face, for the family library a document's `font:` is matched against.
const EMBEDDED: [&[u8]; 6] = [SERIF, BOLD, ITALIC, BOLD_ITALIC, MONO, MATH];

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
