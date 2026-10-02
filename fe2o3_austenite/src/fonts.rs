// SPDX-License-Identifier: Apache-2.0
// Original work copyright (c) the Typst project authors (typst-library `text/font/{book,info,variant,exceptions}.rs`, version 0.15.1).
// Modified for Austenite: ported to Hematite's types, IR and error handling.
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
use oxedyne_fe2o3_graphics::pdf_font::{
	collection_face,
	is_collection,
};

use oxedyne_fe2o3_font::face::{
	FaceClass,
	LayoutMetrics,
};
use oxedyne_fe2o3_font::shape::{
	Dir as ShapeDir,
	Feature,
	Glyph as RunGlyph,
	ShapeSpec,
};
use oxedyne_fe2o3_text::unicode::property::Binary;

use std::collections::{
	BTreeMap,
	HashMap,
};
use std::sync::Mutex;
use std::path::Path;
use std::sync::Arc;
use std::sync::OnceLock;

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
	/// Reads every `.ttf`/`.otf`/`.ttc`/`.otc` file beneath `dir`, at any depth and in any case, after the
	/// faces the crate embeds -- so a document may name an embedded family without supplying it, as Typst's
	/// own embedded fonts need no file. Each face of a collection is its own entry. A file that will not
	/// parse, or declares no family, is left out: it cannot answer to any name, so it can neither match nor
	/// mislead.
	fn scan(dir: &Path) -> Self {
		let mut faces: Vec<Declared> = Vec::new();
		for bytes in EMBEDDED {
			if let Ok(info) = FaceInfo::read(bytes) {
				faces.push(Declared { info, bytes: Arc::new(bytes.to_vec()) });
			}
		}
		for path in vfs::list_files(dir) {
			let ext = path.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase());
			if !matches!(ext.as_deref(), Some("ttf") | Some("otf") | Some("ttc") | Some("otc")) {
				continue;
			}
			let bytes = match vfs::read(&path) {
				Ok(b)	=> b,
				Err(_)	=> continue,
			};
			let bytes = Arc::new(bytes);
			if let Ok(infos) = FaceInfo::read_all(&bytes) {
				for info in infos {
					faces.push(Declared { info, bytes: bytes.clone() });
				}
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
				// A face of a collection is cut out into a file of its own, so the shaper, the outline
				// reader and the PDF font file all read it as a lone font.
				let bytes = if is_collection(&d.bytes) {
					Arc::new(res!(collection_face(&d.bytes, d.info.index as usize)))
				} else {
					d.bytes.clone()
				};
				let font = Arc::new(res!(Font::new(bytes.as_ref().clone())));
				*fam.slot_mut(bold, italic) = Some(Variant { font, bytes });
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

/// The families a document under the font directory `dir` can name: the embedded ones, under the names
/// Typst lists them by, and every family a face beneath `dir` declares, read by the one scan the resolver
/// matches a `font:` against, so a family is listed exactly when a compile can set it. Sorted, each family
/// once.
pub fn available_families(dir: &Path) -> Vec<String> {
	let mut out = embedded_families();
	for family in FontLibrary::scan(dir).families() {
		if !out.iter().any(|f| same_family(f, &family)) {
			out.push(family);
		}
	}
	out.sort();
	out
}

/// The family a font file declares in its own name table: the name a document's `font:` is matched
/// against, whatever the file is called. A caller supplying fonts can compare this, through
/// [`same_family`], with the families a document names before it compiles.
pub fn declared_family(bytes: &[u8]) -> Outcome<String> {
	Ok(res!(FaceInfo::read(bytes)).family)
}

/// Every family a font file declares, one per face of a collection.
pub fn declared_families(bytes: &[u8]) -> Outcome<Vec<String>> {
	Ok(res!(FaceInfo::read_all(bytes)).into_iter().map(|info| info.family).collect())
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
		name, site, dir, offered; Missing, Input, Font)
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

// The embedded faces are Typst 0.15.1's own files (`typst-assets`), byte for byte, so a document that
// names no font is shaped from the same glyph tables the oracle shapes from.
pub(crate) const SERIF:	&[u8] = include_bytes!("../fonts/LibertinusSerif-Regular.otf");
pub(crate) const BOLD:	&[u8] = include_bytes!("../fonts/LibertinusSerif-Bold.otf");
const ITALIC:			&[u8] = include_bytes!("../fonts/LibertinusSerif-Italic.otf");
const BOLD_ITALIC:		&[u8] = include_bytes!("../fonts/LibertinusSerif-BoldItalic.otf");
const SEMIBOLD:			&[u8] = include_bytes!("../fonts/LibertinusSerif-Semibold.otf");
const SEMIBOLD_ITALIC:	&[u8] = include_bytes!("../fonts/LibertinusSerif-SemiboldItalic.otf");
const DEJAVU_MONO:		&[u8] = include_bytes!("../fonts/DejaVuSansMono.ttf");
const DEJAVU_MONO_B:	&[u8] = include_bytes!("../fonts/DejaVuSansMono-Bold.ttf");
const DEJAVU_MONO_O:	&[u8] = include_bytes!("../fonts/DejaVuSansMono-Oblique.ttf");
const DEJAVU_MONO_BO:	&[u8] = include_bytes!("../fonts/DejaVuSansMono-BoldOblique.ttf");
// The curated reader's mono role; the evaluator sets `raw` in DejaVu Sans Mono, as Typst does.
const MONO:				&[u8] = include_bytes!("../fonts/LibertinusMono-Regular.otf");

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

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ SHAPED-RUN CACHE                                                           │
// └───────────────────────────────────────────────────────────────────────────┘
//
// The streaming rule (addendum section 1) holds a cache for shaping under a byte budget, evicting the
// least recently used entry, so a paragraph laid out again in a later pass, or a line cut again at the
// same place, is not shaped twice. The book owns it: face ids mean something only inside one book, and
// a rebuilt book starts with an empty cache.

/// The bytes the shaped-run cache may hold unless the font store says otherwise.
pub const DEFAULT_SHAPE_BUDGET: usize = 4 << 20;

/// What one shaping call depends on. The shaper runs at one pixel per font unit, so its glyphs are in
/// font units whatever size the text is set at; the key therefore carries no point size, and one entry
/// serves every size.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct ShapeKey {
	face:		usize,
	features:	Vec<Feature>,
	lang:		Option<String>,
	script:		Option<[u8; 4]>,
	rtl:		bool,
	ignorables:	bool,	// default-ignorable characters dropped
	text:		String,
}

impl ShapeKey {
	fn of(face: usize, text: &str, dir: ShapeDir, spec: &ShapeSpec) -> Self {
		Self {
			face,
			features:	spec.features.to_vec(),
			lang:		spec.language.map(|l| l.to_string()),
			script:		spec.script,
			rtl:		dir == ShapeDir::Rtl,
			ignorables:	spec.remove_ignorables,
			text:		text.to_string(),
		}
	}

	/// What the key costs, counted at its heap size and a flat allowance for the map's own bookkeeping.
	fn bytes(&self) -> usize {
		self.text.len()
			+ self.features.len() * std::mem::size_of::<Feature>()
			+ self.lang.as_ref().map_or(0, |l| l.len())
			+ std::mem::size_of::<Self>()
	}
}

/// The shaped-run cache's counters: what it holds, what it may hold, and how it has fared.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ShapeStats {
	pub entries:	usize,
	pub bytes:		usize,
	pub budget:		usize,
	pub hits:		u64,
	pub misses:		u64,
	pub evictions:	u64,
}

struct ShapeEntry {
	glyphs:	Arc<Vec<RunGlyph>>,
	bytes:	usize,
	tick:	u64,	// when it was last used, which is its place in `order`
}

/// Shaped runs keyed by what shaping depends on, held under a byte budget. Every entry is the glyphs of
/// one call, shared by `Arc` with every run built from it.
pub struct ShapeCache {
	budget:		usize,
	bytes:		usize,
	tick:		u64,
	map:		HashMap<Arc<ShapeKey>, ShapeEntry>,
	order:		BTreeMap<u64, Arc<ShapeKey>>,	// least recently used first
	hits:		u64,
	misses:		u64,
	evictions:	u64,
}

impl Default for ShapeCache {
	fn default() -> Self { Self::new(DEFAULT_SHAPE_BUDGET) }
}

impl ShapeCache {
	pub fn new(budget: usize) -> Self {
		Self {
			budget,
			bytes:		0,
			tick:		0,
			map:		HashMap::new(),
			order:		BTreeMap::new(),
			hits:		0,
			misses:		0,
			evictions:	0,
		}
	}

	fn get(&mut self, key: &ShapeKey) -> Option<Arc<Vec<RunGlyph>>> {
		self.tick += 1;
		let tick = self.tick;
		match self.map.get_mut(key) {
			Some(e)	=> {
				// Moved to the recent end of the order.
				if let Some(k) = self.order.remove(&e.tick) {
					self.order.insert(tick, k);
				}
				e.tick = tick;
				self.hits += 1;
				Some(e.glyphs.clone())
			}
			None	=> {
				self.misses += 1;
				None
			}
		}
	}

	/// Keeps a result, evicting the least recently used until the budget holds. A result larger than the
	/// whole budget is not kept, so the budget is never exceeded, not even for a moment.
	fn put(&mut self, key: ShapeKey, glyphs: Arc<Vec<RunGlyph>>) {
		let bytes = key.bytes() + glyphs.len() * std::mem::size_of::<RunGlyph>();
		if bytes > self.budget || self.map.contains_key(&key) {
			return;
		}
		self.evict_to(self.budget - bytes);
		self.tick += 1;
		let key = Arc::new(key);
		self.order.insert(self.tick, key.clone());
		self.map.insert(key, ShapeEntry { glyphs, bytes, tick: self.tick });
		self.bytes += bytes;
	}

	/// Drops the least recently used entries until at most `limit` bytes remain.
	fn evict_to(&mut self, limit: usize) {
		while self.bytes > limit {
			let key = match self.order.pop_first() {
				Some((_, k))	=> k,
				None			=> break,
			};
			if let Some(e) = self.map.remove(&key) {
				self.bytes -= e.bytes;
				self.evictions += 1;
			}
		}
	}

	pub fn set_budget(&mut self, budget: usize) {
		self.budget = budget;
		self.evict_to(budget);
	}

	pub fn stats(&self) -> ShapeStats {
		ShapeStats {
			entries:	self.map.len(),
			bytes:		self.bytes,
			budget:		self.budget,
			hits:		self.hits,
			misses:		self.misses,
			evictions:	self.evictions,
		}
	}
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ FONT BOOK (the evaluator's family and variant selection)                   │
// └───────────────────────────────────────────────────────────────────────────┘
//
// A port of Typst 0.15.1's font book (`typst-library`, `text/font/{book,info,variant,exceptions}.rs`,
// Apache-2.0, (c) the Typst project authors): how a face names its family, where it sits in it, and how a
// family and variant, or a character no family covers, choose a face. The selection order, the distance
// metric and the table of fonts whose own names mislead are Typst's, so `font: "X"` picks the same file.

/// The slant a face is drawn with, and the one a style asks for.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FaceStyle {
	#[default]
	Normal,
	Italic,
	Oblique,
}

impl FaceStyle {
	/// Typst's distance: none for the same style, one between italic and oblique, two to or from normal.
	pub fn distance(self, other: Self) -> u16 {
		if self == other {
			0
		} else if self != Self::Normal && other != Self::Normal {
			1
		} else {
			2
		}
	}
}

/// A style, weight (100 to 900) and stretch (per mille of normal width, 500 to 2000).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FaceVariant {
	pub style:		FaceStyle,
	pub weight:		u16,
	pub stretch:	u16,
}

impl Default for FaceVariant {
	fn default() -> Self { Self { style: FaceStyle::Normal, weight: 400, stretch: 1000 } }
}

impl FaceVariant {
	/// The weight moved by `delta` (a `strong`'s 300), clamped as Typst clamps it.
	pub fn thicken(self, delta: i64) -> Self {
		let d = delta.clamp(i16::MIN as i64, i16::MAX as i64) as i16;
		let w = (self.weight as i16).saturating_add(d).clamp(100, 900) as u16;
		Self { weight: w, ..self }
	}
}

/// The stretch of an OS/2 width class, 1 (ultra-condensed) to 9 (ultra-expanded).
fn stretch_of_class(class: u16) -> u16 {
	match class {
		0 | 1	=> 500,
		2		=> 625,
		3		=> 750,
		4		=> 875,
		5		=> 1000,
		6		=> 1125,
		7		=> 1250,
		8		=> 1500,
		_		=> 2000,
	}
}

/// A stretch from a ratio of normal width, clamped to Typst's half-to-double range.
pub fn stretch_of_ratio(ratio: f64) -> u16 {
	(ratio.clamp(0.5, 2.0) * 1000.0) as u16
}

/// One face the book holds: what it is called, where it sits in its family, what it looks like, and the
/// handle shaping and drawing use.
pub struct BookFace {
	pub family:		String,			// as declared, style words trimmed; Typst's `FontInfo::family`
	pub key:		String,			// the family lower-cased, as a document names it
	pub variant:	FaceVariant,
	pub monospace:	bool,
	pub serif:		bool,
	pub variable:	bool,
	pub math:		bool,
	pub metrics:	LayoutMetrics,	// font units
	pub font:		Arc<Font>,		// a chain of this one face
}

impl BookFace {
	/// Does the face map `c` to a glyph?
	pub fn covers(&self, c: char) -> bool {
		match self.font.face(0) {
			Ok(f)	=> f.covers(c),
			Err(_)	=> false,
		}
	}

	/// The face itself, for glyph lookups the chain does not expose.
	pub fn face(&self) -> Outcome<&Face> {
		self.font.face(0)
	}

	/// A length in font units as a fraction of the em.
	pub fn to_em(&self, units: f32) -> f64 {
		units as f64 / self.metrics.units_per_em as f64
	}
}

impl std::fmt::Debug for BookFace {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("BookFace")
			.field("family", &self.family)
			.field("variant", &self.variant)
			.finish()
	}
}

/// Every face one compilation may set text in, indexed by family: the faces the crate embeds (Typst's own
/// set) and any the document supplies.
#[derive(Default)]
pub struct FontBook {
	faces:		Vec<BookFace>,
	families:	HashMap<String, Vec<usize>>,	// lower-cased family to face ids, in insertion order
	shapes:		Mutex<ShapeCache>,				// shaped runs, which every holder of the book shares
}

impl std::fmt::Debug for FontBook {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("FontBook").field("faces", &self.faces.len()).finish()
	}
}

impl FontBook {
	/// The faces Typst embeds that Austenite carries: Libertinus Serif in six styles, New Computer Modern
	/// Math and DejaVu Sans Mono in four.
	pub fn embedded() -> Outcome<Self> {
		let mut book = Self::default();
		for bytes in [
			SERIF, BOLD, ITALIC, BOLD_ITALIC, SEMIBOLD, SEMIBOLD_ITALIC,
			MATH,
			DEJAVU_MONO, DEJAVU_MONO_B, DEJAVU_MONO_O, DEJAVU_MONO_BO,
		] {
			res!(book.add(bytes.to_vec()));
		}
		Ok(book)
	}

	/// Adds a font file's face. A file no parser can read is refused with the reason.
	pub fn add(&mut self, bytes: Vec<u8>) -> Outcome<usize> {
		let font = res!(Font::new(bytes));
		let face = res!(font.face(0));
		let class = res!(face.class());
		let metrics = res!(face.layout_metrics());
		let (family, variant) = res!(describe(&class));
		let key = family.to_lowercase();
		let id = self.faces.len();
		self.faces.push(BookFace {
			family,
			key:		key.clone(),
			variant,
			monospace:	class.monospace,
			serif:		class.serif,
			variable:	class.variable,
			math:		class.math,
			metrics,
			font:		Arc::new(font),
		});
		self.families.entry(key).or_default().push(id);
		Ok(id)
	}

	/// Adds every `.ttf`, `.otf`, `.ttc` or `.otc` file beneath `dir` in the vfs, in path order. A file
	/// that will not parse is passed over, as Typst passes over an unreadable font.
	pub fn add_dir(&mut self, dir: &Path) {
		let mut paths = vfs::list_files(dir);
		paths.sort();
		for path in paths {
			let ext = path.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase());
			if !matches!(ext.as_deref(), Some("ttf") | Some("otf") | Some("ttc") | Some("otc")) {
				continue;
			}
			if let Ok(bytes) = vfs::read(&path) {
				let _ = self.add(bytes);
			}
		}
	}

	pub fn face(&self, id: usize) -> Option<&BookFace> { self.faces.get(id) }

	/// The glyphs of `text` shaped in face `fid`, in font units, from the cache when the same call has been
	/// made before. The shaper runs outside the lock, so a slow shaping never holds up another.
	pub fn shape(&self, fid: usize, text: &str, dir: ShapeDir, spec: &ShapeSpec) -> Outcome<Arc<Vec<RunGlyph>>> {
		let bf = res!(self.faces.get(fid).ok_or_else(|| err!("Face {} is not in the font book.", fid; Bug)));
		let key = ShapeKey::of(fid, text, dir, spec);
		{
			let mut cache = lock_mutex!(self.shapes);
			if let Some(glyphs) = cache.get(&key) {
				return Ok(glyphs);
			}
		}
		let face = res!(bf.face());
		let run = res!(face.shape_spec(text, bf.metrics.units_per_em, dir, 0, 0, spec));
		let glyphs = Arc::new(run.glyphs);
		let mut cache = lock_mutex!(self.shapes);
		cache.put(key, glyphs.clone());
		Ok(glyphs)
	}

	/// Sets the bytes the shaped-run cache may hold, evicting down to it at once.
	pub fn set_shape_budget(&self, budget: usize) -> Outcome<()> {
		let mut cache = lock_mutex!(self.shapes);
		cache.set_budget(budget);
		Ok(())
	}

	pub fn shape_stats(&self) -> Outcome<ShapeStats> {
		let cache = lock_mutex!(self.shapes);
		Ok(cache.stats())
	}

	pub fn len(&self) -> usize { self.faces.len() }

	pub fn is_empty(&self) -> bool { self.faces.is_empty() }

	/// Does any face belong to the family (lower-cased)?
	pub fn has_family(&self, key: &str) -> bool {
		self.families.contains_key(key)
	}

	/// The families the book holds, as declared, sorted and without repeats.
	pub fn family_names(&self) -> Vec<String> {
		let mut out: Vec<String> = Vec::new();
		for f in &self.faces {
			if !out.iter().any(|o| o.to_lowercase() == f.key) {
				out.push(f.family.clone());
			}
		}
		out.sort_by_key(|f| f.to_lowercase());
		out
	}

	/// The face of family `key` (lower-cased) nearest `variant`.
	pub fn select(&self, key: &str, variant: FaceVariant) -> Option<usize> {
		let ids = self.families.get(key)?;
		self.best_variant(None, variant, ids.iter().copied())
	}

	/// The face for text no family covered: among the faces that draw the first character of `text` that
	/// is neither white space nor default-ignorable, the one most like face `like` and nearest `variant`.
	pub fn select_fallback(&self, like: Option<usize>, variant: FaceVariant, text: &str) -> Option<usize> {
		let c = text.chars().find(|&c| !c.is_whitespace() && !is_default_ignorable(c))?;
		let like = like.and_then(|i| self.faces.get(i));
		let ids: Vec<usize> = (0..self.faces.len()).filter(|&i| self.faces[i].covers(c)).collect();
		self.best_variant(like, variant, ids.into_iter())
	}

	/// Typst's `find_best_variant`: the highest score of similarity to `like`, then the least style,
	/// stretch and weight distance, then a variable face over a static one; the first wins a tie.
	fn best_variant<I: Iterator<Item = usize>>(
		&self,
		like:		Option<&BookFace>,
		variant:	FaceVariant,
		ids:		I,
	)
		-> Option<usize>
	{
		let mut best: Option<(usize, (Option<(bool, bool, usize, std::cmp::Reverse<usize>)>, std::cmp::Reverse<(u16, u16, u16)>, bool))> = None;
		for id in ids {
			let cur = &self.faces[id];
			let score = (
				like.map(|l| similarity(cur, l)),
				std::cmp::Reverse(distance(cur, variant)),
				cur.variable,
			);
			if best.as_ref().map_or(true, |(_, b)| score > *b) {
				best = Some((id, score));
			}
		}
		best.map(|(id, _)| id)
	}
}

/// How alike two faces are, for fallback: both monospaced or not, both serifed or not, the words their
/// families share from the start, and then the shorter family, as less specialised.
fn similarity(left: &BookFace, right: &BookFace) -> (bool, bool, usize, std::cmp::Reverse<usize>) {
	(
		left.monospace == right.monospace,
		left.serif == right.serif,
		shared_prefix_words(&left.family, &right.family),
		std::cmp::Reverse(left.family.len()),
	)
}

/// The style, stretch (per mille) and weight distances from a face to a wanted variant. Variable faces
/// are measured at their default instance, as their axes are not yet instanced.
fn distance(face: &BookFace, want: FaceVariant) -> (u16, u16, u16) {
	(
		face.variant.style.distance(want.style),
		(face.variant.stretch as i32 - want.stretch as i32).unsigned_abs() as u16,
		(face.variant.weight as i32 - want.weight as i32).unsigned_abs() as u16,
	)
}

/// How many words two family names share from their start, words being runs of letters and digits.
fn shared_prefix_words(left: &str, right: &str) -> usize {
	let words = |s: &str| -> Vec<String> {
		s.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(|w| w.to_string()).collect()
	};
	words(left).iter().zip(words(right).iter()).take_while(|(l, r)| l == r).count()
}

/// Is `c` a Unicode `Default_Ignorable_Code_Point`?
pub fn is_default_ignorable(c: char) -> bool {
	static PROP: OnceLock<Option<Binary>> = OnceLock::new();
	match PROP.get_or_init(|| Binary::find("Default_Ignorable_Code_Point")) {
		Some(p)	=> p.contains(c),
		None	=> false,
	}
}

/// A face's family and variant, as Typst reads them: an exception by PostScript name first, else the
/// legacy family name (ID 1) with its style words trimmed, the slant from the selection bits or the full
/// name, and the weight and width classes.
fn describe(class: &FaceClass) -> Outcome<(String, FaceVariant)> {
	let exc = class.postscript.as_deref().and_then(exception);
	let family = match exc.and_then(|e| e.1) {
		Some(f)	=> f.to_string(),
		None	=> match &class.family {
			Some(f)	=> typographic_family(f).to_string(),
			None	=> return Err(err!("A font file names no family in its name table."; Invalid, Input, Missing)),
		},
	};
	let style = match exc.and_then(|e| e.2) {
		Some(s)	=> s,
		None	=> {
			let full = class.full_name.clone().unwrap_or_default().to_ascii_lowercase();
			let italic = class.italic || full.contains("italic");
			let oblique = class.oblique || full.contains("oblique") || full.contains("slanted");
			match (italic, oblique) {
				(false, false)	=> FaceStyle::Normal,
				(true, _)		=> FaceStyle::Italic,
				(_, true)		=> FaceStyle::Oblique,
			}
		},
	};
	let weight = exc.and_then(|e| e.3).unwrap_or(class.weight.clamp(100, 900));
	let stretch = match exc.and_then(|e| e.4) {
		Some(s)	=> s,
		None	=> stretch_of_class(class.width),
	};
	Ok((family, FaceVariant { style, weight, stretch }))
}

/// Trims style words (`Bold`, `Semi Condensed`, ...) from a family name, as Typst does, since the legacy
/// family name of many files carries them.
pub fn typographic_family(family: &str) -> &str {
	const SEPARATORS: [char; 3] = [' ', '-', '_'];
	const MODIFIERS: &[&str] = &["extra", "ext", "ex", "x", "semi", "sem", "sm", "demi", "dem", "ultra"];
	const SUFFIXES: &[&str] = &[
		"normal", "italic", "oblique", "slanted",
		"thin", "th", "hairline", "light", "lt", "regular", "medium", "med",
		"md", "bold", "bd", "demi", "extb", "black", "blk", "bk", "heavy",
		"narrow", "condensed", "cond", "cn", "cd", "compressed", "expanded", "exp",
		"vf", "var", "variable",
	];
	let family = family.trim().trim_start_matches('.');
	let lower = family.to_ascii_lowercase();
	let mut len = usize::MAX;
	let mut trimmed = lower.as_str();
	while trimmed.len() < len {
		len = trimmed.len();
		let mut t = trimmed;
		let mut shortened = false;
		while let Some(s) = SUFFIXES.iter().find_map(|s| t.strip_suffix(s)) {
			shortened = true;
			t = s;
		}
		if !shortened {
			break;
		}
		if let Some(s) = t.strip_suffix(SEPARATORS) {
			trimmed = s;
			t = s;
		}
		if let Some(t2) = MODIFIERS.iter().find_map(|s| t.strip_suffix(s)) {
			if let Some(stripped) = t2.strip_suffix(SEPARATORS) {
				trimmed = stripped;
			}
		}
	}
	family.get(..len).unwrap_or(family)
}

type Exception = (&'static str, Option<&'static str>, Option<FaceStyle>, Option<u16>, Option<u16>);

/// The override for a face whose own names mislead, by PostScript name.
fn exception(postscript: &str) -> Option<&'static Exception> {
	EXCEPTIONS.iter().find(|e| e.0 == postscript)
}

// Typst's table of faces whose names or classes mislead: (PostScript name, family, style, weight,
// stretch per mille). Copied from `typst-library` 0.15.1, `text/font/exceptions.rs`.
static EXCEPTIONS: &[Exception] = &[
	("Arial-Black",	None,	None,	Some(900),	None),
	("ArchivoNarrow-Regular",	Some("Archivo Narrow"),	None,	None,	None),
	("ArchivoNarrow-Italic",	Some("Archivo Narrow"),	None,	None,	None),
	("ArchivoNarrow-Bold",	Some("Archivo Narrow"),	None,	None,	None),
	("ArchivoNarrow-BoldItalic",	Some("Archivo Narrow"),	None,	None,	None),
	("FandolHei-Bold",	None,	None,	Some(700),	None),
	("FandolSong-Bold",	None,	None,	Some(700),	None),
	("NotoNaskhArabicUISemi-Bold",	Some("Noto Naskh Arabic UI"),	None,	Some(600),	None),
	("NotoSansSoraSompengSemi-Bold",	Some("Noto Sans Sora Sompeng"),	None,	Some(600),	None),
	("NotoSans-DisplayBlackItalic",	Some("Noto Sans Display"),	None,	None,	None),
	("NotoSans-DisplayCondensedBlackItalic",	Some("Noto Sans Display"),	None,	None,	None),
	("NotoSans-DisplayCondensedBold",	Some("Noto Sans Display"),	None,	None,	None),
	("NotoSans-DisplayCondensedBoldItalic",	Some("Noto Sans Display"),	None,	None,	None),
	("NotoSans-DisplayCondensedExtraBoldItalic",	Some("Noto Sans Display"),	None,	None,	None),
	("NotoSans-DisplayCondensedExtraLightItalic",	Some("Noto Sans Display"),	None,	None,	None),
	("NotoSans-DisplayCondensedItalic",	Some("Noto Sans Display"),	None,	None,	None),
	("NotoSans-DisplayCondensedLightItalic",	Some("Noto Sans Display"),	None,	None,	None),
	("NotoSans-DisplayCondensedMediumItalic",	Some("Noto Sans Display"),	None,	None,	None),
	("NotoSans-DisplayCondensedSemiBoldItalic",	Some("Noto Sans Display"),	None,	None,	None),
	("NotoSans-DisplayCondensedThinItalic",	Some("Noto Sans Display"),	None,	None,	None),
	("NotoSans-DisplayExtraBoldItalic",	Some("Noto Sans Display"),	None,	None,	None),
	("NotoSans-DisplayExtraCondensedBlackItalic",	Some("Noto Sans Display"),	None,	None,	None),
	("NotoSans-DisplayExtraCondensedBold",	Some("Noto Sans Display"),	None,	None,	None),
	("NotoSans-DisplayExtraCondensedBoldItalic",	Some("Noto Sans Display"),	None,	None,	None),
	("NotoSans-DisplayExtraCondensedExtraBoldItalic",	Some("Noto Sans Display"),	None,	None,	None),
	("NotoSans-DisplayExtraCondensedExtraLightItalic",	Some("Noto Sans Display"),	None,	None,	None),
	("NotoSans-DisplayExtraCondensedItalic",	Some("Noto Sans Display"),	None,	None,	None),
	("NotoSans-DisplayExtraCondensedLightItalic",	Some("Noto Sans Display"),	None,	None,	None),
	("NotoSans-DisplayExtraCondensedMediumItalic",	Some("Noto Sans Display"),	None,	None,	None),
	("NotoSans-DisplayExtraCondensedSemiBoldItalic",	Some("Noto Sans Display"),	None,	None,	None),
	("NotoSans-DisplayExtraCondensedThinItalic",	Some("Noto Sans Display"),	None,	None,	None),
	("NotoSans-DisplayExtraLightItalic",	Some("Noto Sans Display"),	None,	None,	None),
	("NotoSans-DisplayLightItalic",	Some("Noto Sans Display"),	None,	None,	None),
	("NotoSans-DisplayMediumItalic",	Some("Noto Sans Display"),	None,	None,	None),
	("NotoSans-DisplaySemiBoldItalic",	Some("Noto Sans Display"),	None,	None,	None),
	("NotoSans-DisplaySemiCondensedBlackItalic",	Some("Noto Sans Display"),	None,	None,	None),
	("NotoSans-DisplaySemiCondensedBold",	Some("Noto Sans Display"),	None,	None,	None),
	("NotoSans-DisplaySemiCondensedBoldItalic",	Some("Noto Sans Display"),	None,	None,	None),
	("NotoSans-DisplaySemiCondensedExtraBoldItalic",	Some("Noto Sans Display"),	None,	None,	None),
	("NotoSans-DisplaySemiCondensedExtraLightItalic",	Some("Noto Sans Display"),	None,	None,	None),
	("NotoSans-DisplaySemiCondensedItalic",	Some("Noto Sans Display"),	None,	None,	None),
	("NotoSans-DisplaySemiCondensedLightItalic",	Some("Noto Sans Display"),	None,	None,	None),
	("NotoSans-DisplaySemiCondensedMediumItalic",	Some("Noto Sans Display"),	None,	None,	None),
	("NotoSans-DisplaySemiCondensedSemiBoldItalic",	Some("Noto Sans Display"),	None,	None,	None),
	("NotoSans-DisplaySemiCondensedThinItalic",	Some("Noto Sans Display"),	None,	None,	None),
	("NotoSans-DisplayThinItalic",	Some("Noto Sans Display"),	None,	None,	None),
	("NotoSerif-DisplayCondensedBold",	Some("Noto Serif Display"),	None,	None,	None),
	("NotoSerif-DisplayExtraCondensedBold",	Some("Noto Serif Display"),	None,	None,	None),
	("NotoSerif-DisplaySemiCondensedBold",	Some("Noto Serif Display"),	None,	None,	None),
	("NewCM08-Book",	Some("New Computer Modern 08"),	None,	Some(450),	None),
	("NewCM08-BookItalic",	Some("New Computer Modern 08"),	None,	Some(450),	None),
	("NewCM08-Italic",	Some("New Computer Modern 08"),	None,	None,	None),
	("NewCM08-Regular",	Some("New Computer Modern 08"),	None,	None,	None),
	("NewCM10-Bold",	Some("New Computer Modern"),	None,	None,	None),
	("NewCM10-BoldItalic",	Some("New Computer Modern"),	None,	None,	None),
	("NewCM10-Book",	Some("New Computer Modern"),	None,	Some(450),	None),
	("NewCM10-BookItalic",	Some("New Computer Modern"),	None,	Some(450),	None),
	("NewCM10-Italic",	Some("New Computer Modern"),	None,	None,	None),
	("NewCM10-Regular",	Some("New Computer Modern"),	None,	None,	None),
	("NewCMMath-Bold",	Some("New Computer Modern Math"),	None,	None,	None),
	("NewCMMath-Book",	Some("New Computer Modern Math"),	None,	Some(450),	None),
	("NewCMMath-Regular",	Some("New Computer Modern Math"),	None,	None,	None),
	("NewCMMono10-Bold",	Some("New Computer Modern Mono"),	None,	None,	None),
	("NewCMMono10-BoldOblique",	Some("New Computer Modern Mono"),	None,	None,	None),
	("NewCMMono10-Book",	Some("New Computer Modern Mono"),	None,	Some(450),	None),
	("NewCMMono10-BookItalic",	Some("New Computer Modern Mono"),	None,	Some(450),	None),
	("NewCMMono10-Italic",	Some("New Computer Modern Mono"),	None,	None,	None),
	("NewCMMono10-Regular",	Some("New Computer Modern Mono"),	None,	None,	None),
	("NewCMSans08-Book",	Some("New Computer Modern Sans 08"),	None,	Some(450),	None),
	("NewCMSans08-BookOblique",	Some("New Computer Modern Sans 08"),	None,	Some(450),	None),
	("NewCMSans08-Oblique",	Some("New Computer Modern Sans 08"),	None,	None,	None),
	("NewCMSans08-Regular",	Some("New Computer Modern Sans 08"),	None,	None,	None),
	("NewCMSans10-Bold",	Some("New Computer Modern Sans"),	None,	None,	None),
	("NewCMSans10-BoldOblique",	Some("New Computer Modern Sans"),	None,	None,	None),
	("NewCMSans10-Book",	Some("New Computer Modern Sans"),	None,	Some(450),	None),
	("NewCMSans10-BookOblique",	Some("New Computer Modern Sans"),	Some(FaceStyle::Oblique),	Some(450),	None),
	("NewCMSans10-Oblique",	Some("New Computer Modern Sans"),	Some(FaceStyle::Oblique),	None,	None),
	("NewCMSans10-Regular",	Some("New Computer Modern Sans"),	None,	None,	None),
	("NewCMSansMath-Regular",	Some("New Computer Modern Sans Math"),	None,	None,	None),
	("NewCMUncial08-Bold",	Some("New Computer Modern Uncial 08"),	None,	None,	None),
	("NewCMUncial08-Book",	Some("New Computer Modern Uncial 08"),	None,	Some(450),	None),
	("NewCMUncial08-Regular",	Some("New Computer Modern Uncial 08"),	None,	None,	None),
	("NewCMUncial10-Bold",	Some("New Computer Modern Uncial"),	None,	None,	None),
	("NewCMUncial10-Book",	Some("New Computer Modern Uncial"),	None,	Some(450),	None),
	("NewCMUncial10-Regular",	Some("New Computer Modern Uncial"),	None,	None,	None),
	("LMMono8-Regular",	Some("Latin Modern Mono 8"),	None,	None,	None),
	("LMMono9-Regular",	Some("Latin Modern Mono 9"),	None,	None,	None),
	("LMMono12-Regular",	Some("Latin Modern Mono 12"),	None,	None,	None),
	("LMMonoLt10-BoldOblique",	None,	Some(FaceStyle::Oblique),	None,	None),
	("LMMonoLt10-Regular",	None,	None,	Some(300),	None),
	("LMMonoLt10-Oblique",	None,	Some(FaceStyle::Oblique),	Some(300),	None),
	("LMMonoLtCond10-Regular",	None,	None,	Some(300),	Some(666)),
	("LMMonoLtCond10-Oblique",	None,	Some(FaceStyle::Oblique),	Some(300),	Some(666)),
	("LMMonoPropLt10-Regular",	None,	None,	Some(300),	None),
	("LMMonoPropLt10-Oblique",	None,	None,	Some(300),	None),
	("LMRoman5-Regular",	Some("Latin Modern Roman 5"),	None,	None,	None),
	("LMRoman6-Regular",	Some("Latin Modern Roman 6"),	None,	None,	None),
	("LMRoman7-Regular",	Some("Latin Modern Roman 7"),	None,	None,	None),
	("LMRoman8-Regular",	Some("Latin Modern Roman 8"),	None,	None,	None),
	("LMRoman9-Regular",	Some("Latin Modern Roman 9"),	None,	None,	None),
	("LMRoman12-Regular",	Some("Latin Modern Roman 12"),	None,	None,	None),
	("LMRoman17-Regular",	Some("Latin Modern Roman 17"),	None,	None,	None),
	("LMRoman7-Italic",	Some("Latin Modern Roman 7"),	None,	None,	None),
	("LMRoman8-Italic",	Some("Latin Modern Roman 8"),	None,	None,	None),
	("LMRoman9-Italic",	Some("Latin Modern Roman 9"),	None,	None,	None),
	("LMRoman12-Italic",	Some("Latin Modern Roman 12"),	None,	None,	None),
	("LMRoman5-Bold",	Some("Latin Modern Roman 5"),	None,	None,	None),
	("LMRoman6-Bold",	Some("Latin Modern Roman 6"),	None,	None,	None),
	("LMRoman7-Bold",	Some("Latin Modern Roman 7"),	None,	None,	None),
	("LMRoman8-Bold",	Some("Latin Modern Roman 8"),	None,	None,	None),
	("LMRoman9-Bold",	Some("Latin Modern Roman 9"),	None,	None,	None),
	("LMRoman12-Bold",	Some("Latin Modern Roman 12"),	None,	None,	None),
	("LMRomanSlant8-Regular",	Some("Latin Modern Roman 8"),	None,	None,	None),
	("LMRomanSlant9-Regular",	Some("Latin Modern Roman 9"),	None,	None,	None),
	("LMRomanSlant12-Regular",	Some("Latin Modern Roman 12"),	None,	None,	None),
	("LMRomanSlant17-Regular",	Some("Latin Modern Roman 17"),	None,	None,	None),
	("LMSans8-Regular",	Some("Latin Modern Sans 8"),	None,	None,	None),
	("LMSans9-Regular",	Some("Latin Modern Sans 9"),	None,	None,	None),
	("LMSans12-Regular",	Some("Latin Modern Sans 12"),	None,	None,	None),
	("LMSans17-Regular",	Some("Latin Modern Sans 17"),	None,	None,	None),
	("LMSans8-Oblique",	Some("Latin Modern Sans 8"),	None,	None,	None),
	("LMSans9-Oblique",	Some("Latin Modern Sans 9"),	None,	None,	None),
	("LMSans12-Oblique",	Some("Latin Modern Sans 12"),	None,	None,	None),
	("LMSans17-Oblique",	Some("Latin Modern Sans 17"),	None,	None,	None),
	("SimSun-ExtB",	Some("SimSun-ExtB"),	None,	None,	None),
	("STKaitiSC-Regular",	None,	None,	Some(400),	None),
	("STKaitiTC-Regular",	None,	None,	Some(400),	None),
	("STKaitiSC-Bold",	None,	None,	Some(700),	None),
	("STKaitiTC-Bold",	None,	None,	Some(700),	None),
	("STKaitiSC-Black",	None,	None,	Some(900),	None),
	("STKaitiTC-Black",	None,	None,	Some(900),	None),
];

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
