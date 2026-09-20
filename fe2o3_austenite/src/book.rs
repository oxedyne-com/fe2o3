//! Whole-book assembly: follow a Typst root's `#include` chain into one ordered block stream, and read
//! the concrete production values the book's `config.typ` selects.
//!
//! A book is not one source file. Its root (`oxpecker.typ`, `lucronics.typ`) sets the page through a
//! template call and then pulls each chapter in with `#include "chap.typ"`; the geometry and type are
//! chosen in `config.typ` by a `format` switch. The reader ([`lang::to_blocks`](crate::lang)) sets one
//! file and skips code lines, so the include-following and the config-reading live here, above it: this
//! module resolves the includes in document order, feeds each chapter through the reader, and reads the
//! one branch of `config.typ` the book's `format` selects into a [`PageGeometry`] and [`Style`].
//!
//! This is targeted extraction, not a Typst evaluator. It reads the concrete fields these books define
//! -- page size, mirror margins, body and heading type -- from the arm the `format` string picks, and
//! nothing more. A field a book does not set keeps the engine default.
//!
//! Two root idioms are recognised. A *book* root (the elearnity manuscripts) carries a `config.typ`
//! beside it and selects its geometry and type from a `format` switch there. A *doc-template* root (the
//! oxedyne documentation trees -- the Hematite guide, the Austenite design) has no `config.typ`: it takes
//! its A4 page and 2.5 cm margins from the shared `template.typ` and its body size from the `#show:
//! doc.with(...)` call. The presence of `config.typ` picks the path; both follow the root's includes the
//! same way, and both degrade to a readable A4 default rather than failing on a field they cannot find.

use crate::bib::{
	Bibliography,
	RefStyle,
};
use crate::doc::{
	Block,
	FrontMatter,
	HeadingStyle,
	Segment,
};
use crate::theme::{
	Theme,
	ThemePatch,
};
use crate::fonts;
use crate::fonts::FaceResolver;
use crate::ir::Sp;
use crate::lang::parse::flatten_markup;
use crate::lang;
use crate::page::PageGeometry;
use crate::table::{
	Align,
	Cell,
	Row,
	Table,
};
use crate::vfs;

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_font::set::FontSet;

use std::collections::HashMap;
use std::collections::HashSet;
use std::path::{
	Path,
	PathBuf,
};
use std::sync::Arc;
use std::sync::RwLock;

/// The book's `term-defs`, read from a sibling `terms.typ` and installed before assembly, so
/// [`resolve_glossary`] can fill a `#print-glossary()` placeholder with each used term's definition. A
/// process-global, mirroring the term-dictionary the parser installs: the glossary table is built once
/// the whole document's blocks are assembled, from a map set at the same point the term-dictionary is.
/// `None` until the loader installs one, under which no term carries a definition and the glossary is
/// its header row alone. The value is already the lowered definition runs, parsed from the content once.
static TERM_DEFS: RwLock<Option<HashMap<String, Vec<Segment>>>> = RwLock::new(None);

const MM_PER_PT: f64 = 72.0 / 25.4;	// points in one millimetre

/// A whole book, assembled and ready to author: the block stream in document order, the page geometry
/// and type style read from its config, and the faces loaded by path from its assets.
pub struct BookSpec {
	pub geom:	PageGeometry,
	pub style:	Theme,
	pub fonts:	Arc<FontSet>,
	pub blocks:	Vec<Block>,
	pub title:	String,	// the book title, for the verso running head
	// The document's named heading display faces (Radley for a book, the doc template's `heading-font`
	// for a doc tree), each loaded by path from the book's assets and resolved by the renderer from the
	// theme's face names; empty when the document names only its body family, so headings fall to the body.
	pub faces:	FaceResolver,
	pub front:		FrontMatter,	// the title page, cover and imprint, read from the root's template call
	// The bibliography, parsed from the file the root names and marked with the keys the body cited, so
	// the block layer resolves an in-text `#cite` against it. None when the book names no bibliography.
	pub bib:		Option<Bibliography>,
	// The constructs the reader skipped across every chapter, merged into one tally, so the binary reports
	// a book or doc compile's skipped constructs on the same terse line a lone file already prints.
	pub skips:		lang::Refusals,
}

/// Does this source read as a book root -- a Typst file that assembles chapters through `#include`?
/// A single manuscript has none, so the binary can tell a book from a lone file by the source itself.
pub fn is_book_root(src: &str) -> bool {
	src.lines().any(|l| l.trim_start().starts_with("#include"))
}

/// The heading display-face names a theme carries, for the resolver to load: the role-default heading
/// face and every per-level face, deduplicated in first-seen order.
pub fn heading_face_names(theme: &Theme) -> Vec<String> {
	let mut names: Vec<String> = Vec::new();
	if let Some(n) = &theme.heading.face {
		names.push(n.clone());
	}
	for l in &theme.heading.levels {
		if let Some(n) = &l.face {
			if !names.contains(n) {
				names.push(n.clone());
			}
		}
	}
	names
}

/// Every heading display-face name reachable in the document: the root theme's, and every name a scoped
/// or box subtree's patch introduces, deduplicated in first-seen order. A rule or an included chapter may
/// name a face the root does not, so the resolver is built from this union rather than the root alone --
/// otherwise a scoped face could never resolve and would fall silently to the body role.
pub fn all_face_names(root_theme: &Theme, blocks: &[Block]) -> Vec<String> {
	let mut names = heading_face_names(root_theme);
	collect_patch_face_names(blocks, &mut names);
	names
}

/// Adds every heading face name a scoped or box subtree's patch names, descending through nested subtrees.
fn collect_patch_face_names(blocks: &[Block], out: &mut Vec<String>) {
	for b in blocks {
		match b {
			Block::Scoped { patch, blocks }	=> { patch_face_names(patch, out); collect_patch_face_names(blocks, out); },
			Block::Box { patch, blocks, .. }	=> { patch_face_names(patch, out); collect_patch_face_names(blocks, out); },
			_							=> {},
		}
	}
}

/// The heading face names a single patch introduces -- its role-default heading face and any per-level
/// face it sets to a name -- appended if not already present.
fn patch_face_names(patch: &crate::theme::ThemePatch, out: &mut Vec<String>) {
	if let Some(Some(n)) = &patch.heading.face {
		if !n.is_empty() && !out.contains(n) { out.push(n.clone()); }
	}
	for l in &patch.heading.levels {
		if let Some(Some(n)) = &l.face {
			if !n.is_empty() && !out.contains(n) { out.push(n.clone()); }
		}
	}
}

/// Records a note for each heading level that names a face and asks for a weight or slant the book ships
/// no file for -- so a bold or italic heading falling back to Regular is visible rather than silent. Checks
/// the root theme's own levels, then descends every scoped or box subtree, folding its patch onto the theme
/// in force at that point (mirroring the merge [`Theme::apply`] performs) so a rule- or chapter-scoped face
/// is checked with the same weight/italic the renderer would set, not only the root's own. A level whose
/// face has no file at all is not noted here: that is the ordinary role fall-back, not a missing variant.
pub fn note_missing_face_variants(theme: &Theme, blocks: &[Block], faces: &FaceResolver, skips: &mut lang::Refusals) {
	note_missing_variants_for_levels(theme, faces, skips);
	note_missing_face_variants_in(theme, blocks, faces, skips);
}

/// The per-level check [`note_missing_face_variants`] runs at the root and, folded onto a scope's merged
/// theme, at every scoped or box subtree.
fn note_missing_variants_for_levels(theme: &Theme, faces: &FaceResolver, skips: &mut lang::Refusals) {
	for (i, l) in theme.heading.levels.iter().enumerate() {
		let name = match l.face.as_deref().or(theme.heading.face.as_deref()) {
			Some(n)	=> n,
			None	=> continue,
		};
		if !faces.resolves(name) {
			continue;
		}
		let bold	= l.weight.map_or(false, |w| w >= 600);
		let italic	= l.italic;
		if (bold || italic) && !faces.has_variant(name, bold, italic) {
			let slant = match (bold, italic) {
				(true, true)	=> "bold-italic",
				(true, false)	=> "bold",
				(false, true)	=> "italic",
				(false, false)	=> "regular",
			};
			skips.record(
				&fmt!("heading face {:?} level {}: no {} file, set in Regular", name, i + 1, slant),
				crate::ir::Span::new(0, 0));
		}
	}
}

/// Descends every [`Block::Scoped`]/[`Block::Box`] subtree, folding its patch onto `parent` (the theme in
/// force at that point) before checking the merged levels and recursing, so a nested scope's own patch
/// folds onto its immediate parent's, not the document root's.
fn note_missing_face_variants_in(parent: &Theme, blocks: &[Block], faces: &FaceResolver, skips: &mut lang::Refusals) {
	for b in blocks {
		match b {
			Block::Scoped { patch, blocks }	=> {
				let mut scoped = parent.clone();
				scoped.apply(patch);
				note_missing_variants_for_levels(&scoped, faces, skips);
				note_missing_face_variants_in(&scoped, blocks, faces, skips);
			},
			Block::Box { patch, blocks, .. }	=> {
				let mut scoped = parent.clone();
				scoped.apply(patch);
				note_missing_variants_for_levels(&scoped, faces, skips);
				note_missing_face_variants_in(&scoped, blocks, faces, skips);
			},
			_							=> {},
		}
	}
}

/// Builds a face resolver for a lone chapter rooted at `root_dir`, loading every heading face the `theme`
/// and `blocks` name -- the root theme's own, and every name a scoped or box subtree's patch introduces --
/// from the tree's `assets/fonts` (one level up from the root, beside a shared template). This mirrors the
/// whole-book path's [`all_face_names`] union rather than the root theme alone, so a rule- or chapter-scoped
/// face resolves here too and not only when a whole book assembles it. A lone file with no such directory,
/// or naming only its body family, yields an empty resolver, so its headings set in the body role as before.
pub fn face_resolver(root_dir: &Path, theme: &Theme, blocks: &[Block]) -> FaceResolver {
	FaceResolver::load(&lone_font_dir(root_dir), &all_face_names(theme, blocks))
}

/// The directory a lone document's [`face_resolver`] reads its display faces from: the tree's
/// `assets/fonts`, one level up from the root (beside a shared template), or under the root itself when it
/// has no parent. Shared with the wasm surface, which routes a document's injected fonts here so a named
/// face resolves whatever path the consumer chose to inject its file under (see [`project_font_path`]).
pub fn lone_font_dir(root_dir: &Path) -> PathBuf {
	match root_dir.parent() {
		Some(d)	=> d.join("assets").join("fonts"),
		None	=> root_dir.join("assets").join("fonts"),
	}
}

/// Where an injected project font must sit for the lone-file [`face_resolver`] to discover it: the
/// resolver's [`lone_font_dir`] joined with the font file's own basename, so `Radley-Regular.otf` injected
/// under any path is found as the `Radley` face's Regular variant. `None` when `given` has no file name.
/// The resolver keys a face on its `<Family>-<Variant>.{ttf,otf}` basename and the family name a document
/// declares (a heading face), so the injected file's basename must follow that convention to be usable.
pub fn project_font_path(main_path: &Path, given: &Path) -> Option<PathBuf> {
	let root_dir = main_path.parent().unwrap_or(main_path);
	given.file_name().map(|name| lone_font_dir(root_dir).join(name))
}

/// Assembles the document rooted at `root_path` into a [`BookSpec`], recognising both root idioms the
/// house Typst trees use. A *book* root sets its geometry and type through a `config.typ` beside it,
/// chosen by a `format` switch (the elearnity manuscripts); a *doc-template* root has no `config.typ`
/// and takes its A4 geometry from the shared `template.typ` and its body size from the `#show:
/// doc.with(...)` call (the oxedyne documentation trees -- the Hematite guide, the Austenite design).
/// The presence of `config.typ` beside the root selects the path: the book reader is unchanged, and a
/// root without a config falls to the doc reader rather than failing on the missing file.
pub fn load(root_path: &Path) -> Outcome<BookSpec> {
	// Canonicalise the root so its parent and grandparent are real absolute directories. A root given
	// relatively (`lucronics.typ`) otherwise has an empty parent, and the project directory the shared
	// `refs.bib` and assets sit in cannot be found -- a symlinked `assets` masks this for fonts, but the
	// bibliography one level up is missed.
	let root_path = vfs::canonicalize(root_path).unwrap_or_else(|_| root_path.to_path_buf());
	let root_path = root_path.as_path();
	let root_dir = match root_path.parent() {
		Some(d)	=> d.to_path_buf(),
		None	=> return Err(err!("The book root {:?} has no parent directory.", root_path; Input, Invalid)),
	};
	let root_src = match vfs::read_to_string(root_path) {
		Ok(s)	=> s,
		Err(e)	=> return Err(err!(e, "Could not read the book root {:?}.", root_path; File, Read)),
	};

	// Install the book's `term-dict` and `term-defs` from a `terms.typ` beside or above the root: the
	// dictionary so the glossary family resolves each key to its value as the chapters are read, and the
	// definitions so a `#print-glossary()` can be filled once the document's used terms are known.
	res!(install_term_dict(&root_dir));
	res!(install_term_defs(&root_dir));

	// A `config.typ` beside the root marks the book (`format`-switch) idiom; without it, the root sets its
	// page through the shared `template.typ` and the `doc.with` call, which is the documentation idiom.
	let config_path = root_dir.join("config.typ");
	if !vfs::exists(&config_path) {
		return load_doc(root_path, &root_dir, &root_src);
	}
	load_book(root_path, &root_dir, &root_src)
}

/// The book (`format`-switch) path: reads the `config.typ` beside the root, loads the shared Libertinus
/// faces by path from the project assets tree, and follows the root's includes into one block stream.
fn load_book(root_path: &Path, root_dir: &Path, root_src: &str) -> Outcome<BookSpec> {
	// The config sits beside the root; the assets tree is one level up (the project root), holding the
	// Libertinus directory both books share.
	let config_path	= root_dir.join("config.typ");
	let config_src	= match vfs::read_to_string(&config_path) {
		Ok(s)	=> s,
		Err(e)	=> return Err(err!(e, "Could not read the book config {:?}.", config_path; File, Read)),
	};
	let project_dir = match root_dir.parent() {
		Some(d)	=> d.to_path_buf(),
		None	=> root_dir.to_path_buf(),
	};
	let assets_fonts	= project_dir.join("assets").join("fonts");
	let libertinus_dir	= assets_fonts.join("libertinus");
	let fonts = Arc::new(res!(fonts::libertinus_from_dir(&libertinus_dir)));

	let (geom, raw) = res!(read_config(&config_src));
	let mut style	= build_style(&raw);
	// A book sets its chapter and level-2 headings in Radley, the display face beside Libertinus in the
	// shared assets tree. It is named on the theme's role-default heading face here, then loaded by the
	// resolver below; a book whose tree ships no Radley resolves nothing and sets headings in the body
	// bold, the same fall-back as before.
	style.heading.face = Some("Radley".to_string());
	// The root's own declarative styling -- its `#show: doc.with(...)` application and any lowerable
	// top-level `#set` -- lowers onto the theme. The config file's `#let` type scale is read separately
	// by `read_config` above; this reads only the root's own top-level declarations.
	lang::set::lower_root_declarations(root_src, &mut style);
	// The book's `#let` furniture functions (`#pr-note`, `#aside-box`), collected once against the document's
	// body size so every `em` in a definition resolves to an absolute, then threaded into every chapter the
	// assembler reads so a call expands into its padded box rather than being tallied as a skipped construct.
	let tfns = collect_book_template_fns(root_src, root_dir, style.text.body_size);
	// The book config's `media` (and any other guard scalar) reaches the assembler here, so a chapter's
	// `#if media == "..."` include guard follows only its taken branch.
	let (mut blocks, mut skips)	= res!(assemble(root_src, root_dir, root_path, &tfns, &config_src));
	// The styling rule engine runs over the assembled tree here, BEFORE the face resolver is built: a rule
	// that names a heading face wraps its matched elements in a scope carrying that face, and the resolver's
	// face union descends into those scopes -- so a rule-named face must already be on the tree when the
	// union is taken. The default rules re-assert the theme's own heading sizes (byte-neutral); the root's
	// own `#show <selector>: <transform>` rules are appended, refused where a transform reads the page or
	// patches a field the renderer does not read.
	let rules = lang::rules::rule_set_for(&style, root_src, &mut skips);
	lang::rules::apply_rules(&mut blocks, &rules, geom.content_width());
	// The resolver is built from every heading face the document can name -- the root theme's, and every
	// name a scoped or box subtree's patch introduces -- so a face a chapter or a rule names still loads,
	// not only the root's own. A note is recorded where a heading asks for a weight or slant the book ships
	// no file for.
	let faces = FaceResolver::load(&assets_fonts, &all_face_names(&style, &blocks));
	note_missing_face_variants(&style, &blocks, &faces, &mut skips);
	// A book root may also place a `#print-glossary()`; fill it in place once its chapters are assembled.
	resolve_glossary(&mut blocks, false);
	let title		= content_field(root_src, "title").unwrap_or_default();
	let front		= read_front_matter(root_src, &config_src, &title);

	// The bibliography the root names, if any: parse it, mark every key the body cited, and append the
	// Chicago reference list as back matter. The marked bibliography then resolves each in-text `#cite`.
	let bib = res!(load_bibliography(root_src, &project_dir, &mut blocks));

	// The glossary and index back matter the root's `meta-data.glossary`/`meta-data.index` flags ask for,
	// after the bibliography and gated on the body actually carrying the content -- a book that sets a flag
	// but uses no glossary or index term emits neither section, matching the template's own gate.
	append_flag_back_matter(root_src, &mut blocks);

	Ok(BookSpec { geom, style, fonts, blocks, title, faces, front, bib, skips })
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ DOCUMENTATION IDIOM                                                        │
// └───────────────────────────────────────────────────────────────────────────┘

/// The template's `avg_reading_speed`, in words per minute, used to turn the word count into the reading
/// time the meta page shows.
const AVG_READING_SPEED: usize = 230;

/// The acknowledgement paragraph the documentation template seats at the foot of its meta page, a fixed
/// constant of the doc idiom rather than a `meta-data` field (`template.typ`'s `meta-page`).
const DOC_ACKNOWLEDGEMENT: &str = "We acknowledge the Indigenous peoples who have been traditional \
custodians of lands and waters around the world, past and present. Through their languages, stories, and \
practices, they have sought to maintain living connections to humanity's deepest roots and the ancient \
wisdom of sustainable coexistence with Earth's ecosystems.";

/// Maps an AI-declaration slug to its mark image path and caption, mirroring the template's
/// `ai-declarations` dictionary and its `doc` medium. The path is image-base-relative, resolved the same
/// way as the title logos; an unknown slug yields no mark, so the author cell sets the name alone.
fn ai_declaration_mark(slug: &str) -> Option<(String, String)> {
	let (file, words) = match slug {
		"no-ai"			=> ("doc_made_with_no_ai_opt.svg",			"Made with no AI"),
		"some-ai"		=> ("doc_made_with_some_ai_opt.svg",		"Made with some AI"),
		"with-ai"		=> ("doc_made_with_ai_opt.svg",				"Made with AI"),
		"mostly-ai"		=> ("doc_made_with_ai_mostly_opt.svg",		"Made mostly with AI"),
		"entirely-ai"	=> ("doc_made_with_ai_entirely_opt.svg",	"Made with AI entirely"),
		_				=> return None,
	};
	Some((fmt!("assets/svg/{}", file), words.to_string()))
}

/// The documentation (`doc.with`) path: the oxedyne doc trees (Hematite, Austenite) carry no
/// `config.typ`. Their A4 page and 2.5 cm margins live in the shared `template.typ`, and the body size
/// is the `text-size:` argument of the root's `#show: doc.with(...)` call. Geometry and type are read
/// from those two sources; the body font is the embedded Libertinus, which is the doc body and heading
/// family both (a doc heading is Libertinus bold, so no separate display face is loaded); and the
/// includes are followed exactly as for a book. A field the tree omits keeps a readable default.
fn load_doc(root_path: &Path, root_dir: &Path, root_src: &str) -> Outcome<BookSpec> {
	let (geom, raw, opener)	= res!(read_doc_config(root_dir, root_src));
	let mut style	= build_style(&raw);
	// The doc root's own `#show: doc.with(...)` application (and any lowerable top-level `#set`) lowers
	// onto the theme; its per-format type scale is read from the config by `read_doc_config` above.
	lang::set::lower_root_declarations(root_src, &mut style);
	let title		= content_field(root_src, "title").unwrap_or_default();
	// A doc tree sets `numbering: none`; how its top-level headings open turns first on the template
	// idiom read above. A grid template (oxeweb) opens level 1 with a fixed logo/title grid and no
	// number -- neither a banner bar nor an inline heading -- so its opener reserves the grid bands
	// (`DocGrid`). A banner template's top-level headings open with the grey banner bar unless the tree
	// draws its own per-section `#section-banner` logo bars, in which case each level-1 heading is set
	// inline beneath the section's banner. This mirrors the template's `chapter-banners` argument: an
	// explicit `true`/`false` decides, and its `auto` default turns the chapter banners off only for the
	// Hematite guide, whose sections carry logo banners instead.
	let want_banners = tri_bool(root_src, "chapter-banners").unwrap_or(title != "Hematite");
	style.heading.kind = if opener == DocOpener::Grid {
		HeadingStyle::DocGrid
	} else if want_banners {
		HeadingStyle::DocBanner
	} else {
		HeadingStyle::DocInline
	};
	let fonts		= Arc::new(res!(fonts::libertinus()));
	// The doc template's `#show: doc.with(heading-font: ...)` lowered its heading face onto the theme's
	// levels above; the resolver loads it from the tree's own `assets/fonts` (one level up from the root,
	// beside the shared `template.typ`). A tree naming its body family (or no face) resolves nothing, so
	// its headings fall to the body role -- Libertinus bold -- exactly as before. oxeweb's "Graystroke"
	// resolves and its chapter and level-2 headings now set in it, as the template renders them.
	let assets_fonts = match root_dir.parent() {
		Some(d)	=> d.join("assets").join("fonts"),
		None	=> root_dir.join("assets").join("fonts"),
	};
	let tfns = collect_book_template_fns(root_src, root_dir, style.text.body_size);
	// The documentation idiom carries no `config.typ`, so the guard evaluator sees an empty config and
	// falls back to each file's own `#let` bindings; a doc tree writing no include guard is unaffected.
	let (mut blocks, mut skips)	= res!(assemble(root_src, root_dir, root_path, &tfns, ""));
	// The styling rule engine runs over the assembled tree before the resolver is built, so a rule-named
	// face is in the union the resolver loads (see `load_book` for the same seam and why it sits here).
	let rules = lang::rules::rule_set_for(&style, root_src, &mut skips);
	lang::rules::apply_rules(&mut blocks, &rules, geom.content_width());
	// The resolver loads every heading face the document can name -- the root theme's and every scoped or
	// box subtree's -- so a face a chapter names still loads; a heading asking for a weight/slant with no
	// file is noted rather than silently set in Regular.
	let faces = FaceResolver::load(&assets_fonts, &all_face_names(&style, &blocks));
	note_missing_face_variants(&style, &blocks, &faces, &mut skips);
	// Fill each `#print-glossary()` placeholder with the Term/Definition table now the whole document's
	// blocks are assembled and its used glossary terms known, before the word count and layout walk them.
	resolve_glossary(&mut blocks, false);
	let mut front	= read_doc_front_matter(root_dir, root_src, &raw, &title);

	// The reading time the meta page appends to its notes cell: the whole-document word count over the
	// template's 230 words/min, rounded up, matching its `calc.ceil(words.final() / avg_reading_speed)`.
	let words		= crate::doc::count_words(&blocks);
	front.reading_min	= Some(((words + AVG_READING_SPEED - 1) / AVG_READING_SPEED) as u32);

	// A doc tree names its bibliography, glossary and index through raw Typst calls the reader skips, not
	// the book's `meta-data.bibliography` field, so no reference back matter is assembled here.
	Ok(BookSpec { geom, style, fonts, blocks, title, faces, front, bib: None, skips })
}

/// Which level-1 opener idiom a doc template uses, read from its `show heading` block. A grid template
/// (oxeweb) opens with a fixed `#grid` of logo/title bands and no number; the oxedyne banner template
/// draws a grey `#section-banner` bar. The two take different opener metrics and heading scales.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DocOpener {
	Grid,
	Banner,
}

/// Reads a doc-template root's geometry and type: the paper and margins from the shared `template.typ`
/// beside the root, and the body size from the root's `doc.with(text-size: ..)` argument. The template
/// fixes uniform margins with a slightly deeper foot (`margins.a4 + 0.25cm`), matching its `set page`.
/// Everything the tree does not state -- leading, paragraph spacing, heading sizes -- takes the Typst
/// default the template inherits, so an unfamiliar doc root still assembles onto a readable A4 page.
fn read_doc_config(root_dir: &Path, root_src: &str) -> Outcome<(PageGeometry, RawStyle, DocOpener)> {
	// The template is symlinked in beside the root; a tree without it falls back to A4 at 2.5 cm.
	let template = vfs::read_to_string(&root_dir.join("template.typ")).unwrap_or_default();

	let paper_name	= first_quoted_after(&template, "paper:").unwrap_or_else(|| "a4".to_string());
	let (pw_mm, ph_mm)	= paper_dims_mm(&paper_name);

	// The uniform margin: the `a4:` entry of the template's `#let margins = (...)` dictionary, a length.
	let margin_pt	= let_dict_field(&template, "margins", "a4")
		.and_then(|v| parse_len_pt(&v))
		.unwrap_or(2.5 * 10.0 * MM_PER_PT);	// 2.5 cm default
	let foot_extra	= 0.25 * 10.0 * MM_PER_PT;	// the template's `bottom: margins.a4 + 0.25cm`

	let geom = PageGeometry::with_margins(
		Sp::from_pt(pw_mm * MM_PER_PT),
		Sp::from_pt(ph_mm * MM_PER_PT),
		Sp::from_pt(margin_pt),
		Sp::from_pt(margin_pt),
		Sp::from_pt(margin_pt),
		Sp::from_pt(margin_pt + foot_extra),
	);

	// The body size the doc.with call sets, else the template's own `text-size: 11pt` default.
	let body_pt	= first_len_after(root_src, "text-size:")
		.or_else(|| first_len_after(&template, "text-size:"))
		.unwrap_or(11.0);

	// The doc template inherits Typst's default leading (0.65 em) but its OWN paragraph spacing: the
	// template sets no `#set par(spacing:)`, so a paragraph gap takes Typst's default `par.spacing` of
	// 1.2 em (the earlier 0.65 was wrong -- it under-set the gap and, once the block edges were pinned to
	// cap-height/baseline, drove the whole doc short of the oracle). Correct for both idioms.
	//
	// The level-1 opener idiom is read from the template's `show heading` block. A grid template
	// (oxeweb) opens with a fixed `#grid(rows: (240pt, 10pt, 40pt, 20pt))` -- a logo band, a gap, a 40 pt
	// title band set at `size: 32pt` small-caps, and a gap to the body -- and its sub-headings size by
	// `(18, 14, 13, 12).at(level - 1)`, so level 2 is 14 pt, level 3 13 pt, level 4 12 pt (the level-1
	// entry, 18 pt, is unused: level 1 takes the grid). The oxedyne banner template opens with a grey
	// `#section-banner` bar and no grid, and keeps its own smaller heading scale (chapter title 14 pt).
	// The presence of a `rows:` tuple after the rule tells the two apart; pinning the grid's metrics onto a
	// banner tree over-set its level-1 and back-matter headings. A future doc template with other values
	// should have them read from its own `template.typ` rather than pinned here. The anchor is the rule form
	// `show heading: it =>`, not the bare words: the banner template carries a `//` comment naming
	// `show heading`/`set heading`, and matching that comment (with a `rows:` tuple anywhere after it) would
	// misread the banner as a grid.
	let head_tail	= template.find("show heading: it =>").map(|at| &template[at..]);
	let grid_rows	= head_tail.and_then(|t| tuple_after(t, "rows:")).filter(|r| r.len() >= 4);
	let (opener, chap_grid, h1_pt, h2_pt, h3_pt, h4_pt) = match grid_rows {
		Some(rows)	=> {
			let h1	= head_tail.and_then(|t| num_after(t, "size:")).unwrap_or(32.0);	// the grid title, `size: 32pt`
			(DocOpener::Grid, [rows[0], rows[1], rows[2], rows[3]], h1, 14.0, 13.0, 12.0)
		},
		None		=> (DocOpener::Banner, [72.0, 8.0, 36.0, 20.0], 14.0, 12.0, 13.0, 12.0),
	};

	let raw = RawStyle {
		body_pt,
		leading_em:		0.65,
		par_skip_em:	1.2,
		indent_em:		0.0,
		chap_num_pt:	54.0,
		chap_grid,
		h1_pt,
		h2_pt,
		h3_pt,
		h4_pt,
	};
	Ok((geom, raw, opener))
}

/// The front matter a doc root states: its title and subtitle, the author from the first `meta-data`
/// entry, and the documentation template's two-column title-page furniture -- the sidebar width and
/// colour, the two sidebar logos with their declared widths, the small-caps flag, and the footer logo --
/// read from the `#show: doc.with(...)` call and the shared `template.typ`. A doc tree carries no imprint
/// (no ISBN, publisher or copyright tuple), so only a title page and the contents are composed from this.
fn read_doc_front_matter(root_dir: &Path, root_src: &str, raw: &RawStyle, title: &str) -> FrontMatter {
	let subtitle	= content_field(root_src, "subtitle");
	let meta		= meta_block(root_src).unwrap_or_default();
	let author		= string_field(&meta, "authors").unwrap_or_default();

	// The AI scheme address the mark links to, `<scheme>/<slug>/<medium>`, read from the shared template's
	// `ai-scheme-url` and `ai-medium` lets (the template's `link(ai-scheme-url + "/" + slug + "/" +
	// ai-medium, ..)`). A tree without the template falls back to the scheme's permanent home and doc medium.
	let template		= vfs::read_to_string(&root_dir.join("template.typ")).unwrap_or_default();
	let ai_scheme_url	= first_quoted_after(&template, "ai-scheme-url").unwrap_or_else(|| "https://need2know.ai".to_string());
	let ai_medium		= first_quoted_after(&template, "ai-medium").unwrap_or_else(|| "doc".to_string());

	// The revision rows the template's meta/colophon page draws: each row's version, date, notes, and the
	// AI declaration whose slug picks the mark image, its caption (a `declaration-words` field rescopes the
	// caption without changing the mark) and the scheme page the mark links to. A doc tree may state several
	// rows, newest first.
	let meta_rows: Vec<crate::doc::MetaRow> = meta_rows(&meta).iter().map(|row| {
		let (ai_mark_path, ai_mark_words, ai_mark_url) = match string_field(row, "declaration") {
			Some(slug)	=> match ai_declaration_mark(&slug) {
				Some((path, words))	=> {
					let words = string_field(row, "declaration-words").unwrap_or(words);
					let url = fmt!("{}/{}/{}", ai_scheme_url, slug, ai_medium);
					(Some(path), Some(words), Some(url))
				},
				None				=> (None, None, None),
			},
			None		=> (None, None, None),
		};
		crate::doc::MetaRow {
			version:	string_field(row, "version"),
			date:		string_field(row, "date"),
			authors:	string_field(row, "authors").unwrap_or_default(),
			notes:		string_field(row, "notes"),
			ai_mark_path,
			ai_mark_words,
			ai_mark_url,
		}
	}).collect();
	// The colophon furniture the template fixes for the doc idiom: the acknowledgement paragraph, and the
	// copyright line composed from the organisation the term dictionary names (`#t("org")`), falling back
	// to Oxedyne. Both are template constants rather than `meta-data`, so they are set here for every doc.
	let acknowledgement	= Some(DOC_ACKNOWLEDGEMENT.to_string());
	let org				= crate::lang::parse::term_value("org").unwrap_or_else(|| "Oxedyne".to_string());
	let copyright		= Some(fmt!("Copyright © 12025 {}. All rights reserved.", org));

	// The sidebar width is `margins.title_page` in the shared template (a percentage of the page); the fill
	// is the `title-colour` the call names, resolved to a grey level. A doc tree always draws the sidebar,
	// so `sidebar_grey` is set here (marking the two-column idiom) even when the call omits its colour.
	let template	= vfs::read_to_string(&root_dir.join("template.typ")).unwrap_or_default();
	let sidebar_frac	= let_dict_field(&template, "margins", "title_page")
		.and_then(|v| parse_percent(&v))
		.unwrap_or(0.45);
	let colour_name	= string_field(root_src, "title-colour").unwrap_or_default();
	let sidebar_grey	= Some(grey_luma(&colour_name));

	let non_empty	= |s: Option<String>| s.filter(|p| !p.is_empty());
	let top_logo	= non_empty(string_field(root_src, "title-top-logo-path"));
	let bottom_logo	= non_empty(string_field(root_src, "title-bottom-logo-path"));
	let footer_logo	= non_empty(string_field(root_src, "footer-left-logo-path"));
	let top_w		= first_len_after(root_src, "title-top-logo-width:").unwrap_or(80.0);
	let bottom_w	= first_len_after(root_src, "title-bottom-logo-width:").unwrap_or(120.0);
	let smallcaps	= bool_field(root_src, "title-smallcaps");

	FrontMatter {
		title:			title.to_string(),
		subtitle,
		author,
		cover_image:	None,
		logo_image:		None,
		publisher:		None,
		edition:		None,
		isbn:			None,
		copyright,
		rights:			None,
		ai_declaration:	None,
		website:		None,
		toolchain:		false,
		dedication:		None,
		about_author:	None,
		title_size:		Sp::from_pt(28.0),
		subtitle_size:	Sp::from_pt(16.0),
		author_size:	Sp::from_pt(17.0),
		back_title_size:	Sp::from_pt(raw.h1_pt),
		sidebar_grey,
		sidebar_frac,
		title_smallcaps:	smallcaps,
		top_logo,
		top_logo_width:		Sp::from_pt(top_w),
		bottom_logo,
		bottom_logo_width:	Sp::from_pt(bottom_w),
		footer_logo,
		meta_rows,
		reading_min:	None,	// set by `load_doc`, which has the body blocks to count
		acknowledgement,
	}
}

/// Resolves a `template.typ` colour name to a grey level. Only the greys the doc trees reach for are
/// mapped by name; every other name falls to the template's `colours.light` (luma 240), a light sidebar.
fn grey_luma(name: &str) -> u8 {
	match name {
		"white"			=> 255,
		"lightgrey"		=> 240,
		"light"			=> 240,
		_				=> 240,
	}
}

/// Reads a Typst percentage literal (`45%`) as a fraction (`0.45`). `None` when no number leads it.
fn parse_percent(s: &str) -> Option<f64> {
	first_num(s).map(|n| n / 100.0)
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ BIBLIOGRAPHY                                                               │
// └───────────────────────────────────────────────────────────────────────────┘

/// Parses the bibliography the root's `meta-data.bibliography` names, marks every key the body cited,
/// and appends the Bibliography back matter (a heading and the sorted, cited-only reference list) to the
/// block stream. Returns the marked bibliography for the in-text citation formatter, or `None` when the
/// book names no bibliography or the file cannot be read.
fn load_bibliography(root_src: &str, project_dir: &Path, blocks: &mut Vec<Block>) -> Outcome<Option<Bibliography>> {
	let meta = match meta_block(root_src) {
		Some(m)	=> m,
		None	=> return Ok(None),
	};
	let path_str = match string_field(&meta, "bibliography") {
		Some(p)	=> p,
		None	=> return Ok(None),	// no bibliography named
	};

	// The path is Typst-root-relative (`/refs.bib`); resolve it against the project directory.
	let rel		= path_str.trim_start_matches('/');
	let bib_path	= project_dir.join(rel);
	let src = match vfs::read_to_string(&bib_path) {
		Ok(s)	=> s,
		Err(_)	=> return Ok(None),	// a named bibliography that will not read is a reported gap, not a failure
	};
	let bib = res!(Bibliography::parse(&src));
	Ok(Some(append_bibliography(bib, blocks)))
}

/// Marks every key the body cited on `bib`, then appends the Bibliography back matter -- the section
/// heading and one Reference block per sorted, cited reference -- to the block stream, returning the
/// marked bibliography for the in-text citation formatter. Shared by the whole-book path and the lone
/// chapter path, so a chapter compiled on its own resolves its citations exactly as the book does.
fn append_bibliography(mut bib: Bibliography, blocks: &mut Vec<Block>) -> Bibliography {
	// Mark every key the body cited, so the reference list holds exactly the cited works.
	for keys in collect_cite_keys(blocks) {
		for k in keys {
			bib.mark_cited(&k);
		}
	}

	// Append the back matter: the section heading, then one Reference block per sorted, cited reference.
	blocks.push(Block::back_matter_heading("Bibliography"));
	for reference in bib.reference_list() {
		let runs: Vec<(String, bool)> = reference.runs.iter()
			.map(|r| (r.text.clone(), r.style == RefStyle::Italic))
			.collect();
		blocks.push(Block::reference(runs));
	}
	bib
}

/// Appends the glossary and index back matter the root's `meta-data` flags ask for, each gated on the body
/// carrying its content. The glossary section is a back-matter heading, a one-line note and a
/// [`Block::Glossary`] placeholder [`resolve_glossary`] fills with the Term/Definition table; it is set
/// only when the book uses at least one defined glossary term (`meta-data.glossary: true` alone, with no
/// used term, sets nothing, as the template's own gate does). The index section is a back-matter heading
/// and a [`Block::Index`] placeholder [`crate::doc::author`] fills from the index markers it gathers walking
/// the body; it is set only when the body carries at least one index marker.
fn append_flag_back_matter(root_src: &str, blocks: &mut Vec<Block>) {
	let meta = match meta_block(root_src) {
		Some(m)	=> m,
		None	=> return,
	};

	if bool_field(&meta, "glossary") && has_defined_glossary_terms(blocks) {
		blocks.push(Block::back_matter_heading("Glossary"));
		blocks.push(Block::RichParagraph {
			segments: vec![Segment::text("Terms are shown by their order of appearance.")],
		});
		blocks.push(Block::Glossary);
		// Fill the placeholder just appended: the earlier whole-book `resolve_glossary` ran before it existed.
		resolve_glossary(blocks, true);
	}

	if bool_field(&meta, "index") && has_index_occurrences(blocks) {
		blocks.push(Block::back_matter_heading("Index"));
		blocks.push(Block::Index);
	}
}

/// Does the body carry at least one glossary term that has a definition? The same walk
/// [`resolve_glossary`] makes, so the gate agrees with what the table would hold: a term with no `term-defs`
/// entry contributes no row and does not count.
fn has_defined_glossary_terms(blocks: &[Block]) -> bool {
	let mut seen:		HashSet<String>	= HashSet::new();
	let mut ordered:	Vec<String>		= Vec::new();
	for block in blocks {
		collect_glossary_terms(block, &mut seen, &mut ordered);
	}
	!ordered.is_empty()
}

/// Does the body carry at least one index marker anywhere -- in a heading, paragraph, list item, table cell
/// or callout, or a footnote's own runs? The gate that keeps the index section from being set for a book
/// that asks for one but marks no term.
fn has_index_occurrences(blocks: &[Block]) -> bool {
	blocks.iter().any(block_has_index)
}

/// Whether one block, or anything nested in it, carries an index marker segment.
fn block_has_index(block: &Block) -> bool {
	match block {
		Block::Heading { segments, .. }		=> segments_have_index(segments),
		Block::RichParagraph { segments }	=> segments_have_index(segments),
		Block::List { items, .. }			=> items.iter().any(|it|
			segments_have_index(&it.segments) || it.children.iter().any(block_has_index)),
		Block::Table(t)						=> table_has_index(t),
		Block::TableFigure { table, .. }	=> table_has_index(table),
		Block::Box { blocks, .. }			=> blocks.iter().any(block_has_index),
		Block::Scoped { blocks, .. }		=> blocks.iter().any(block_has_index),
		_									=> false,
	}
}

/// Whether a run of segments carries an index marker, descending into a footnote's own runs.
fn segments_have_index(segments: &[Segment]) -> bool {
	segments.iter().any(|seg| match seg {
		Segment::Index { .. }		=> true,
		Segment::Footnote { note }	=> segments_have_index(note),
		_						=> false,
	})
}

/// Whether any cell of a table carries an index marker.
fn table_has_index(table: &Table) -> bool {
	table.rows.iter().any(|row| row.cells.iter().any(|cell| segments_have_index(&cell.content)))
}

/// Locates a `refs.bib` beside a lone chapter or in an ancestor directory, parses it, marks the keys the
/// chapter cited, appends the reference list as back matter, and returns the marked bibliography so the
/// block layer resolves each in-text `#cite` to Chicago author-year -- as a whole-book compile does.
/// `None` when no `refs.bib` is found or it will not read, in which case the raw cite key stands as before.
pub fn load_lone_bibliography(source: &Path, blocks: &mut Vec<Block>) -> Outcome<Option<Bibliography>> {
	let start = match source.parent() {
		Some(d)	=> d,
		None	=> return Ok(None),
	};
	let bib_path = match find_up(start, "refs.bib") {
		Some(p)	=> p,
		None	=> return Ok(None),
	};
	let src = match vfs::read_to_string(&bib_path) {
		Ok(s)	=> s,
		Err(_)	=> return Ok(None),	// a bibliography found but unreadable is a reported gap, not a failure
	};
	let bib = res!(Bibliography::parse(&src));
	Ok(Some(append_bibliography(bib, blocks)))
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ TERM DICTIONARY                                                            │
// └───────────────────────────────────────────────────────────────────────────┘

/// Reads the book's `term-dict` from a `terms.typ` beside or above `start_dir` and installs it, so the
/// term-dictionary glossary family (`t`, `tcap`, `graw`, `g`, `gi`, `gcap`, `gcapi`) resolves each key to
/// its value while the chapters are read. An absent or `term-dict`-less `terms.typ` installs an empty
/// map, under which every key falls back to its own text.
pub fn install_term_dict(start_dir: &Path) -> Outcome<()> {
	let src = match find_up(start_dir, "terms.typ") {
		Some(p)	=> vfs::read_to_string(&p).unwrap_or_default(),
		None	=> String::new(),
	};
	res!(crate::lang::parse::set_term_dict(parse_term_dict(&src)));
	Ok(())
}

/// Reads the book's `term-defs` from a `terms.typ` beside or above `start_dir` and installs it, so
/// [`resolve_glossary`] can give each glossary term used in the document its definition row. An absent or
/// `term-defs`-less `terms.typ` installs an empty map, under which every term contributes no row and the
/// glossary sets its header alone -- the same early return the template's style makes for an undefined key.
pub fn install_term_defs(start_dir: &Path) -> Outcome<()> {
	let src = match find_up(start_dir, "terms.typ") {
		Some(p)	=> vfs::read_to_string(&p).unwrap_or_default(),
		None	=> String::new(),
	};
	let mut defs: HashMap<String, Vec<Segment>> = HashMap::new();
	for (key, content) in parse_term_defs(&src) {
		defs.insert(key, lang::inline_segments(&content));
	}
	let mut guard = lock_write!(TERM_DEFS, "While recording the term definitions");
	*guard = Some(defs);
	Ok(())
}

/// The lowered definition runs a `term-defs` key resolves to, or `None` when no map is installed or it
/// holds no such key. A poisoned lock reads as absent, so a missing definition drops the term's row
/// rather than failing the compile -- the safe degradation, matching the template's undefined-key branch.
fn term_def(key: &str) -> Option<Vec<Segment>> {
	match TERM_DEFS.read() {
		Ok(guard)	=> guard.as_ref().and_then(|m| m.get(key).cloned()),
		Err(_)		=> None,
	}
}

/// Parses the `#let term-defs = ( "key": [definition], ... )` block from a `terms.typ` source into
/// key→content pairs. Unlike the term-dictionary, whose values are quoted strings, a definition is Typst
/// *content* (`[...]`) carrying inline markup, so each value is the balanced-bracket group's inner source,
/// left for [`lang::inline_segments`] to parse. Pairs are returned in source order; a value that is not a
/// content group (a tuple or bare string) is skipped, since the live term files use plain content only.
fn parse_term_defs(src: &str) -> Vec<(String, String)> {
	let mut out: Vec<(String, String)> = Vec::new();
	// The assignment, not a `// term-defs: ...` mention: the name must be followed, after only whitespace,
	// by `=`, exactly as the term-dictionary reader guards its own literal.
	let at = match assignment_offset(src, "term-defs") {
		Some(a)	=> a,
		None	=> return out,
	};
	let chars: Vec<char> = src[at..].chars().collect();
	let n = chars.len();

	// Advance to the opening parenthesis of the dictionary literal, then step past it.
	let mut i = 0;
	while i < n && chars[i] != '(' {
		i += 1;
	}
	if i >= n {
		return out;
	}
	i += 1;

	loop {
		// Skip the whitespace, commas and comments between entries; stop at the closing parenthesis or the
		// source end. Comments must be skipped whole: `terms.typ` carries `// ...` banners and notes between
		// term groups, and a `)` inside one (a parenthetical aside) would otherwise read as the literal's
		// closing parenthesis and truncate the parse -- which dropped half of Lucronics' 346 definitions.
		loop {
			while i < n && (chars[i].is_whitespace() || chars[i] == ',') {
				i += 1;
			}
			if i + 1 < n && chars[i] == '/' && chars[i + 1] == '/' {
				i += 2;
				while i < n && chars[i] != '\n' {
					i += 1;
				}
				continue;
			}
			if i + 1 < n && chars[i] == '/' && chars[i + 1] == '*' {
				i += 2;
				while i + 1 < n && !(chars[i] == '*' && chars[i + 1] == '/') {
					i += 1;
				}
				i = (i + 2).min(n);
				continue;
			}
			break;
		}
		if i >= n || chars[i] == ')' {
			break;
		}
		if chars[i] != '"' {
			i += 1;	// a stray token inside the literal; step over it
			continue;
		}
		// The quoted key, honouring string escapes so a quote inside it does not end it early.
		let (key, next) = read_string(&chars, i);
		i = next;
		// The `:` between key and value, and the whitespace either side of it.
		while i < n && chars[i].is_whitespace() {
			i += 1;
		}
		if i < n && chars[i] == ':' {
			i += 1;
		}
		while i < n && chars[i].is_whitespace() {
			i += 1;
		}
		// The value: a `[...]` content group is the definition; anything else is skipped to the next entry.
		if i < n && chars[i] == '[' {
			let (content, next) = read_content(&chars, i);
			out.push((key, content));
			i = next;
		} else {
			// Not a content group: advance to the next top-level comma so the reader resynchronises.
			let mut depth = 0i32;
			while i < n {
				match chars[i] {
					'(' | '[' | '{'	=> depth += 1,
					')' | ']' | '}'	=> {
						if depth == 0 {
							break;
						}
						depth -= 1;
					},
					','	if depth == 0	=> break,
					_	=> {},
				}
				i += 1;
			}
		}
	}
	out
}

/// Reads a `"..."` string whose opening quote sits at `i`, returning its unescaped contents and the index
/// just past the closing quote. A backslash sets the next character literally, so a quote or backslash
/// inside the string does not end it early.
fn read_string(chars: &[char], i: usize) -> (String, usize) {
	let mut s	= String::new();
	let mut j	= i + 1;	// past the opening quote
	let mut esc	= false;
	while j < chars.len() {
		let c = chars[j];
		if esc			{ s.push(c); esc = false; }
		else if c == '\\'	{ esc = true; }
		else if c == '"'	{ j += 1; break; }
		else			{ s.push(c); }
		j += 1;
	}
	(s, j)
}

/// Reads a `[...]` content group whose opening bracket sits at `i`, returning its inner source and the
/// index just past the closing bracket. Brackets nested in the content (a `#emph[...]` inside a
/// definition) are balanced, and a quoted string inside the content is skipped whole so a `]` within it
/// does not close the group early.
fn read_content(chars: &[char], i: usize) -> (String, usize) {
	let mut depth	= 0i32;
	let mut j		= i;
	let mut inner	= String::new();
	while j < chars.len() {
		match chars[j] {
			'['	=> {
				depth += 1;
				if depth > 1 {
					inner.push('[');	// a nested opener is part of the content
				}
			},
			']'	=> {
				depth -= 1;
				if depth == 0 {
					j += 1;
					break;
				}
				inner.push(']');
			},
			'"'	=> {
				// Copy the whole quoted string verbatim so a bracket inside it is not read as structure.
				let (s, next) = read_string(chars, j);
				inner.push('"');
				inner.push_str(&s);
				inner.push('"');
				j = next;
				continue;
			},
			c	=> inner.push(c),
		}
		j += 1;
	}
	(inner.trim().to_string(), j)
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ GLOSSARY                                                                   │
// └───────────────────────────────────────────────────────────────────────────┘

/// The point size a `#print-glossary()` table sets at, matching the template's `text(size: 9pt)`.
const GLOSSARY_TEXT_PT: f64 = 9.0;

/// The cell padding a `#print-glossary()` table insets by, matching the template's `inset: 6pt`.
const GLOSSARY_INSET_PT: f64 = 6.0;

/// Fills each `#print-glossary()` placeholder in an assembled block stream with a Term/Definition table
/// of the glossary terms the document uses, in first-appearance order. The placeholder is replaced in
/// place, so the glossary sets where the author wrote the call rather than as appended back matter.
///
/// The terms are the [`Segment::Glossary`] runs the parser recorded, walked in document order and
/// deduplicated by term key -- the key the template's `glossary-seen` set keys by -- keeping the first
/// occurrence. A term with no `term-defs` entry contributes no row, matching the template style's early
/// return for an undefined key. The Term column shows the term-dictionary value where the key has one
/// (the `g`/`gcap` family) and the key itself otherwise (the `gs` family), reproducing the metadata
/// `value` the template stores; the Definition column carries the parsed definition content.
pub fn resolve_glossary(blocks: &mut Vec<Block>, breakable: bool) {
	// The placeholder may sit inside a scoped (or callout) subtree -- an included chapter's own
	// `#print-glossary()` -- not only at top level, so it is sought through the whole tree. With none
	// anywhere, nothing is built, exactly as before.
	if !any_glossary(blocks) {
		return;
	}
	// The glossary term keys in first-appearance order, deduplicated, keeping only those with a definition.
	let mut seen:		HashSet<String>	= HashSet::new();
	let mut ordered:	Vec<String>		= Vec::new();
	for block in blocks.iter() {
		collect_glossary_terms(block, &mut seen, &mut ordered);
	}

	// The header row, then one row per defined term: the Term column its display value, the Definition
	// column its parsed content. The header sets bold and centred (a header row's own face and alignment);
	// body cells set left, matching the template's `(left, left).at(col)`.
	let mut rows: Vec<Row> = Vec::new();
	rows.push(Row::new(vec![
		Cell::rich(vec![Segment::strong("Term")], Align::Centre),
		Cell::rich(vec![Segment::strong("Definition")], Align::Centre),
	]));
	for key in &ordered {
		let def = match term_def(key) {
			Some(d)	=> d,
			None	=> continue,
		};
		let value = crate::lang::parse::term_value(key).unwrap_or_else(|| key.clone());
		rows.push(Row::new(vec![
			Cell::rich(vec![Segment::text(value)], Align::Left),
			Cell::rich(def, Align::Left),
		]));
	}

	let mut table		= Table::with_weights(true, rows, vec![1.0, 3.0]);
	table.text_size		= Some(Sp::from_pt(GLOSSARY_TEXT_PT));
	table.inset			= Some(Sp::from_pt(GLOSSARY_INSET_PT));
	// A book's whole-document glossary runs to many pages, so it sets one keep box per row and paginates
	// between rows rather than clipping to a single box, matching the template's `block(breakable: true)`
	// glossary table. A short in-body `#print-glossary()` stays one box, so a doc's existing glossary is
	// byte-for-byte unchanged.
	table.breakable		= breakable;
	// Replace the first placeholder in document order, wherever in the tree it sits, with the built table.
	let _ = replace_first_glossary(blocks, Block::Table(table));
}

/// Is there a `#print-glossary()` placeholder anywhere in the tree, descending into scoped and callout
/// subtrees? The guard that keeps [`resolve_glossary`] from building a table no placeholder will consume.
fn any_glossary(blocks: &[Block]) -> bool {
	blocks.iter().any(|b| match b {
		Block::Glossary									=> true,
		Block::Scoped { blocks, .. } | Block::Box { blocks, .. }	=> any_glossary(blocks),
		_											=> false,
	})
}

/// Replaces the first [`Block::Glossary`] placeholder in document order -- at top level or inside a scoped
/// or callout subtree -- with `table`, returning `Ok(())` when it did and `Err(table)` (the table handed
/// back) when the slice held no placeholder, so the search threads on through the rest of the tree.
fn replace_first_glossary(blocks: &mut [Block], table: Block) -> Result<(), Block> {
	let mut slot = table;
	for b in blocks.iter_mut() {
		if matches!(b, Block::Glossary) {
			*b = slot;
			return Ok(());
		}
		if let Block::Scoped { blocks: inner, .. } | Block::Box { blocks: inner, .. } = b {
			match replace_first_glossary(inner, slot) {
				Ok(())			=> return Ok(()),
				Err(returned)	=> slot = returned,	// not in this subtree; keep the table and walk on
			}
		}
	}
	Err(slot)
}

/// Walks one block's rich runs, recording each glossary term key on its first appearance -- in document
/// order, deduplicated -- when the key carries a `term-defs` definition. Headings, paragraphs, list items
/// and table cells all carry glossary terms, and a term inside a footnote counts as a use, so each is
/// walked. A term with no definition is passed over, so the ordered set holds only rows the glossary sets.
fn collect_glossary_terms(block: &Block, seen: &mut HashSet<String>, ordered: &mut Vec<String>) {
	match block {
		Block::Heading { segments, .. }			=> collect_from_segments(segments, seen, ordered),
		Block::RichParagraph { segments }		=> collect_from_segments(segments, seen, ordered),
		Block::List { items, .. }				=> for it in items {
			collect_from_segments(&it.segments, seen, ordered);
			for child in &it.children { collect_glossary_terms(child, seen, ordered); }
		},
		Block::Table(t)							=> collect_from_table(t, seen, ordered),
		Block::TableFigure { table, .. }		=> collect_from_table(table, seen, ordered),
		Block::Box { blocks, .. }				=> for b in blocks { collect_glossary_terms(b, seen, ordered); },
		Block::Scoped { blocks, .. }			=> for b in blocks { collect_glossary_terms(b, seen, ordered); },
		_										=> {},
	}
}

/// Records each glossary term in a run of segments, descending into a footnote's own runs so a term first
/// used inside a note is ordered by the note's position, as the template's document-order query is.
fn collect_from_segments(segments: &[Segment], seen: &mut HashSet<String>, ordered: &mut Vec<String>) {
	for seg in segments {
		match seg {
			Segment::Glossary { term, .. }	=> {
				if term_def(term).is_some() && seen.insert(term.clone()) {
					ordered.push(term.clone());
				}
			},
			Segment::Footnote { note }		=> collect_from_segments(note, seen, ordered),
			_								=> {},
		}
	}
}

/// Records each glossary term across a table's cells, row-major, so a term first used in a table is
/// ordered by the cell it appears in.
fn collect_from_table(table: &Table, seen: &mut HashSet<String>, ordered: &mut Vec<String>) {
	for row in &table.rows {
		for cell in &row.cells {
			collect_from_segments(&cell.content, seen, ordered);
		}
	}
}

/// Parses the `#let term-dict = ( "key": "value", ... )` block from a `terms.typ` source into a key→value
/// map. `terms.typ` is a pure data file (no imports, state or side effects), so the literal is read
/// directly rather than evaluated: the quoted strings inside the dictionary's balanced parentheses come in
/// key, value order, and are paired off. An empty map when the source names no `term-dict`.
fn parse_term_dict(src: &str) -> HashMap<String, String> {
	let mut map = HashMap::new();
	// Find the assignment `term-dict =`, not a mention in a comment: the name must be followed, after only
	// whitespace, by `=`. The file opens with a `// term-dict: ...` comment whose own parentheses would
	// otherwise be read as the literal, so the first bare occurrence is not enough.
	let at = match assignment_offset(src, "term-dict") {
		Some(a)	=> a,
		None	=> return map,
	};
	let chars: Vec<char> = src[at..].chars().collect();

	// Advance to the opening parenthesis of the dictionary literal.
	let mut i = 0;
	while i < chars.len() && chars[i] != '(' {
		i += 1;
	}
	if i >= chars.len() {
		return map;
	}

	// Walk the balanced group, collecting each `"..."` string; string escapes are honoured so a quote or
	// backslash inside a value does not end it early. The strings alternate key, value, key, value.
	let mut depth	= 0i32;
	let mut strings:	Vec<String>	= Vec::new();
	while i < chars.len() {
		match chars[i] {
			'('	=> depth += 1,
			')'	=> {
				depth -= 1;
				if depth == 0 {
					break;
				}
			},
			'"'	=> {
				let mut s	= String::new();
				let mut esc	= false;
				i += 1;
				while i < chars.len() {
					let c = chars[i];
					if esc			{ s.push(c); esc = false; }
					else if c == '\\'	{ esc = true; }
					else if c == '"'	{ break; }
					else			{ s.push(c); }
					i += 1;
				}
				strings.push(s);
			},
			_	=> {},
		}
		i += 1;
	}

	let mut k = 0;
	while k + 1 < strings.len() {
		map.insert(strings[k].clone(), strings[k + 1].clone());
		k += 2;
	}
	map
}

/// The byte offset of a `name =` assignment in `src` -- the position of `name` where the next
/// non-whitespace character after it is `=`. Skips a mention of the name in a comment or another context
/// (say `// name: ...`), returning the first true assignment, or `None` when there is none.
fn assignment_offset(src: &str, name: &str) -> Option<usize> {
	let mut from = 0;
	while let Some(rel) = src[from..].find(name) {
		let at		= from + rel;
		let after	= at + name.len();
		let rest	= src[after..].trim_start();
		if rest.starts_with('=') {
			return Some(at);
		}
		from = after;
	}
	None
}

/// Searches `start` and up to a few ancestor directories for a file named `name`, returning the first
/// that exists. The bound keeps a lone-file compile from walking to the filesystem root: a book's shared
/// `terms.typ` or `refs.bib` sits at most a couple of levels above a chapter.
fn find_up(start: &Path, name: &str) -> Option<PathBuf> {
	const MAX_HOPS: usize = 6;
	let mut dir		= Some(start);
	let mut hops	= 0usize;
	while let Some(d) = dir {
		let cand = d.join(name);
		if vfs::exists(&cand) {
			return Some(cand);
		}
		if hops >= MAX_HOPS {
			break;
		}
		hops += 1;
		dir = d.parent();
	}
	None
}

/// Gathers the citation keys the body's blocks carry, in document order, so each can be marked cited.
fn collect_cite_keys(blocks: &[Block]) -> Vec<Vec<String>> {
	let mut out = Vec::new();
	for block in blocks {
		match block {
			Block::RichParagraph { segments }	=> collect_cite_segments(segments, &mut out),
			Block::List { items, .. }			=> for item in items {
				collect_cite_segments(&item.segments, &mut out);
				out.extend(collect_cite_keys(&item.children));
			},
			Block::Box { blocks, .. }			=> out.extend(collect_cite_keys(blocks)),
			Block::Scoped { blocks, .. }		=> out.extend(collect_cite_keys(blocks)),
			// A table cell is set through the body's own segment pipeline, so a `#cite` in a cell renders and
			// must be marked cited too, or its work would render but its reference vanish from the list.
			Block::Table(t)						=> collect_cite_from_table(t, &mut out),
			Block::TableFigure { table, .. }	=> collect_cite_from_table(table, &mut out),
			_									=> {},
		}
	}
	out
}

/// Pushes the keys of every citation in any cell of a table onto `out`.
fn collect_cite_from_table(table: &Table, out: &mut Vec<Vec<String>>) {
	for row in &table.rows {
		for cell in &row.cells {
			collect_cite_segments(&cell.content, out);
		}
	}
}

/// Pushes the keys of every citation segment in `segments` onto `out`.
fn collect_cite_segments(segments: &[Segment], out: &mut Vec<Vec<String>>) {
	for seg in segments {
		if let Segment::Cite(keys) = seg {
			out.push(keys.clone());
		}
	}
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ FRONT MATTER EXTRACTION                                                    │
// └───────────────────────────────────────────────────────────────────────────┘

/// Reads the front matter the root's `#show: doc.with(...)` sets: the title and subtitle, the author and
/// imprint from `meta-data`, the cover the config selects for a development build, and the display sizes
/// from the config's type scale. A field the book omits is left `None`, and its page or line is not set.
fn read_front_matter(root_src: &str, config_src: &str, title: &str) -> FrontMatter {
	let subtitle	= content_field(root_src, "subtitle");
	let meta		= meta_block(root_src).unwrap_or_default();

	let author		= string_field(&meta, "authors").unwrap_or_default();
	let publisher	= content_field(&meta, "publisher").map(|s| clean_content(&s));
	let edition		= string_field(&meta, "edition");
	let isbn		= string_field(&meta, "isbn");
	let copyright	= copyright_line(&meta);
	let rights		= content_field(&meta, "rights").map(|s| clean_content(&s));
	let ai_decl		= content_field(&meta, "ai-declaration").map(|s| clean_content(&s));
	let website		= content_field(&meta, "website").map(|s| clean_content(&s)).filter(|s| !s.is_empty());
	let toolchain	= bool_field(&meta, "show-toolchain");
	let dedication	= string_field(&meta, "dedication").filter(|s| s != "none" && !s.is_empty());
	let about		= content_field(&meta, "bio").map(|s| clean_content(&s)).filter(|s| !s.is_empty());
	let logo		= string_field(root_src, "title-logo-path");

	// The cover the config picks: none in an interior build, the format's raster in a development one.
	let format		= read_let_string(config_src, "format").unwrap_or_default();
	let mode		= read_let_string(config_src, "mode").unwrap_or_default();
	let cover		= if mode == "interior" {
		None
	} else {
		arm(config_src, "cover-image-path", &format).as_deref().and_then(first_quoted)
	};

	// The display sizes from the type scale the format selects.
	let scale		= arm(config_src, "type-scale", &format);
	let sz = |key: &str, default: f64| -> Sp {
		Sp::from_pt(scale.as_deref().and_then(|a| num_after(a, key)).unwrap_or(default))
	};

	FrontMatter {
		title:			title.to_string(),
		subtitle,
		author,
		cover_image:	cover,
		logo_image:		logo,
		publisher,
		edition,
		isbn,
		copyright,
		rights,
		ai_declaration:	ai_decl,
		website,
		toolchain,
		dedication,
		about_author:	about,
		title_size:		sz("title:", 28.0),
		subtitle_size:	sz("subtitle:", 16.0),
		author_size:	sz("author:", 17.0),
		back_title_size:	sz("back-matter-title:", 17.0),
		// A book draws its own plain centred title page, not the doc template's two-column one, so the
		// documentation sidebar and logos are left unset (`sidebar_grey: None` keeps the plain title page).
		sidebar_grey:		None,
		sidebar_frac:		0.0,
		title_smallcaps:	false,
		top_logo:			None,
		top_logo_width:		Sp::ZERO,
		bottom_logo:		None,
		bottom_logo_width:	Sp::ZERO,
		footer_logo:		None,
		// The doc template's meta/colophon fields; a book draws its own imprint page, not the doc colophon.
		meta_rows:			Vec::new(),
		reading_min:		None,
		acknowledgement:	None,
	}
}

/// The inner text of the root's `meta-data: ( ... )` argument, balanced across nested groups and
/// strings, or `None` when the root sets no `meta-data`.
fn meta_block(src: &str) -> Option<String> {
	let at		= src.find("meta-data:")?;
	let rest	= &src[at + "meta-data:".len()..];
	let open	= rest.find('(')?;
	let bytes	= rest.as_bytes();
	let mut depth	= 0i32;
	let mut in_str	= false;
	let mut esc		= false;
	let mut i		= open;
	while i < bytes.len() {
		let c = bytes[i] as char;
		if in_str {
			if esc			{ esc = false; }
			else if c == '\\'	{ esc = true; }
			else if c == '"'	{ in_str = false; }
			i += 1;
			continue;
		}
		match c {
			'"'	=> in_str = true,
			'('	=> depth += 1,
			')'	=> {
				depth -= 1;
				if depth == 0 {
					return Some(rest[open + 1..i].to_string());
				}
			},
			_	=> {},
		}
		i += 1;
	}
	None
}

/// Splits a `meta-data` block into its revision rows: the text inside each top-level parenthesised tuple,
/// in source order. Nested parentheses and strings are respected, so a row whose value carries a comma or
/// a bracket is not split early. A block with no nested tuple (a bare single row) yields no rows.
fn meta_rows(block: &str) -> Vec<String> {
	let bytes	= block.as_bytes();
	let mut rows:	Vec<String>	= Vec::new();
	let mut depth	= 0i32;
	let mut in_str	= false;
	let mut esc		= false;
	let mut start	= 0usize;
	let mut i		= 0usize;
	while i < bytes.len() {
		let c = bytes[i] as char;
		if in_str {
			if esc			{ esc = false; }
			else if c == '\\'	{ esc = true; }
			else if c == '"'	{ in_str = false; }
			i += 1;
			continue;
		}
		match c {
			'"'	=> in_str = true,
			'('	=> {
				if depth == 0 { start = i + 1; }
				depth += 1;
			},
			')'	=> {
				depth -= 1;
				if depth == 0 {
					rows.push(block[start..i].to_string());
				}
			},
			_	=> {},
		}
		i += 1;
	}
	rows
}

/// The string a `name: "..."` field binds: the first `"..."` in the field's value, which runs to the
/// next top-level comma (a comma inside the string does not end it). `None` when the value holds no
/// string literal -- a `name: none` reads as absent -- so a later field's value is never read by mistake.
fn string_field(src: &str, name: &str) -> Option<String> {
	let needle	= fmt!("{}:", name);
	let at		= src.find(&needle)?;
	let rest	= &src[at + needle.len()..];
	// Bound the value at the next depth-zero comma, respecting strings, so the search stays in this field.
	let bytes	= rest.as_bytes();
	let mut depth	= 0i32;
	let mut in_str	= false;
	let mut esc		= false;
	let mut end		= rest.len();
	let mut i		= 0usize;
	while i < bytes.len() {
		let c = bytes[i] as char;
		if in_str {
			if esc			{ esc = false; }
			else if c == '\\'	{ esc = true; }
			else if c == '"'	{ in_str = false; }
			i += 1;
			continue;
		}
		match c {
			'"'					=> in_str = true,
			'(' | '[' | '{'		=> depth += 1,
			')' | ']' | '}'		=> depth -= 1,
			',' if depth == 0	=> { end = i; break; },
			_					=> {},
		}
		i += 1;
	}
	first_quoted(&rest[..end])
}

/// The `Copyright © YEAR HOLDER. NOTICE` line the template composes from the `copyright: (year, [holder],
/// notice)` tuple, or `None` when the book sets no copyright tuple.
fn copyright_line(meta: &str) -> Option<String> {
	let at		= meta.find("copyright:")?;
	let rest	= &meta[at + "copyright:".len()..];
	let open	= rest.find('(')?;
	// The tuple's three parts: a year string, a `[holder]` content, and a notice string.
	let inner	= balanced_parens(&rest[open..])?;
	let parts	= split_top(&inner);
	if parts.is_empty() {
		return None;
	}
	let year	= parts.first().map(|s| unquote_or_content(s)).unwrap_or_default();
	let holder	= parts.get(1).map(|s| unquote_or_content(s)).unwrap_or_default();
	let notice	= parts.get(2).map(|s| unquote_or_content(s)).unwrap_or_default();
	Some(fmt!("Copyright © {} {}. {}", year.trim(), holder.trim(), notice.trim()))
}

/// The contents of a `(...)` at the start of `s`, balanced across nesting and strings.
fn balanced_parens(s: &str) -> Option<String> {
	let bytes	= s.as_bytes();
	let mut depth	= 0i32;
	let mut in_str	= false;
	let mut esc		= false;
	let mut i		= 0usize;
	while i < bytes.len() {
		let c = bytes[i] as char;
		if in_str {
			if esc			{ esc = false; }
			else if c == '\\'	{ esc = true; }
			else if c == '"'	{ in_str = false; }
			i += 1;
			continue;
		}
		match c {
			'"'	=> in_str = true,
			'('	=> depth += 1,
			')'	=> {
				depth -= 1;
				if depth == 0 {
					return Some(s[1..i].to_string());
				}
			},
			_	=> {},
		}
		i += 1;
	}
	None
}

/// Splits `s` at its top-level commas, respecting nesting and strings.
fn split_top(s: &str) -> Vec<String> {
	let mut out:	Vec<String>	= Vec::new();
	let mut cur					= String::new();
	let mut depth				= 0i32;
	let mut in_str				= false;
	let mut esc					= false;
	for c in s.chars() {
		if in_str {
			cur.push(c);
			if esc			{ esc = false; }
			else if c == '\\'	{ esc = true; }
			else if c == '"'	{ in_str = false; }
			continue;
		}
		match c {
			'"'					=> { in_str = true; cur.push(c); },
			'(' | '[' | '{'		=> { depth += 1; cur.push(c); },
			')' | ']' | '}'		=> { depth -= 1; cur.push(c); },
			',' if depth == 0	=> out.push(std::mem::take(&mut cur)),
			_					=> cur.push(c),
		}
	}
	if !cur.trim().is_empty() {
		out.push(cur);
	}
	out
}

/// Reads a tuple part as a plain string: a `"..."` literal unquoted, or a `[...]` content flattened.
fn unquote_or_content(part: &str) -> String {
	let t = part.trim();
	if t.starts_with('"') && t.ends_with('"') && t.len() >= 2 {
		return t[1..t.len() - 1].to_string();
	}
	if t.starts_with('[') && t.ends_with(']') && t.len() >= 2 {
		return flatten_markup(&t[1..t.len() - 1]);
	}
	t.to_string()
}

/// Whether a `name: true` boolean field is set true.
fn bool_field(src: &str, name: &str) -> bool {
	let needle	= fmt!("{}:", name);
	match src.find(&needle) {
		Some(at)	=> {
			let rest	= &src[at + needle.len()..];
			let end		= rest.find(',').unwrap_or(rest.len());
			rest[..end].trim().starts_with("true")
		},
		None		=> false,
	}
}

/// A three-way boolean argument in the root's template call: `Some(true)`/`Some(false)` when the field is
/// set to `true`/`false`, and `None` when it is absent or left `auto`, so the caller can supply its own
/// default for the `auto` case.
fn tri_bool(src: &str, name: &str) -> Option<bool> {
	let needle	= fmt!("{}:", name);
	let at		= src.find(&needle)?;
	let rest	= &src[at + needle.len()..];
	let end		= rest.find(',').unwrap_or(rest.len());
	let val		= rest[..end].trim();
	if val.starts_with("true") {
		Some(true)
	} else if val.starts_with("false") {
		Some(false)
	} else {
		None
	}
}

/// Reduces a content field to a single line of display text: markup flattened, Typst line breaks (`\`)
/// turned to spaces, and any leftover `#name[...]` term call stripped, so an imprint or biography line
/// reads as plain prose.
fn clean_content(s: &str) -> String {
	let flat	= flatten_markup(s);
	let mut out	= flat.replace('\\', " ");
	// Drop a `#ident[...]` or `#ident("...")` term call left after flattening (e.g. `#t[website]`).
	while let Some(h) = out.find('#') {
		let tail	= &out[h + 1..];
		let name_len	= tail.chars().take_while(|c| c.is_alphanumeric() || *c == '-' || *c == '_').count();
		let after	= &tail[name_len..];
		let end = if after.starts_with('[') {
			after.find(']').map(|e| h + 1 + name_len + e + 1)
		} else if after.starts_with('(') {
			after.find(')').map(|e| h + 1 + name_len + e + 1)
		} else {
			Some(h + 1 + name_len)
		};
		match end {
			Some(e)	=> { out.replace_range(h..e, ""); },
			None	=> break,
		}
	}
	out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The text of a `name: [ ... ]` content field in the root's template call -- the book title, say --
/// with the surrounding brackets dropped and inner whitespace trimmed. Bracket-balanced, so a nested
/// group does not close it early.
fn content_field(src: &str, name: &str) -> Option<String> {
	let needle	= fmt!("{}:", name);
	let at		= src.find(&needle)?;
	let rest	= &src[at + needle.len()..];
	let open	= rest.find('[')?;
	let bytes	= rest.as_bytes();
	let mut depth	= 0i32;
	let mut i	= open;
	while i < bytes.len() {
		match bytes[i] {
			b'['	=> depth += 1,
			b']'	=> {
				depth -= 1;
				if depth == 0 {
					return Some(rest[open + 1..i].trim().to_string());
				}
			},
			_	=> {},
		}
		i += 1;
	}
	None
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ #let FURNITURE FUNCTIONS (collected across the whole book)                  │
// └───────────────────────────────────────────────────────────────────────────┘

/// Collects every `#let name(params) = block/box(...)` furniture definition the book declares -- in the
/// root, in each `#include`d chapter (where `#pr-note` is defined, byte-identical, atop three chapters), and
/// in the shared template chain the root imports (where `#aside-box` lives, and, two hops on, the
/// `#let colours = (...)` palette it fills from) -- into one map, lowered against `body_size` so every `em`
/// resolves to an absolute and every `colours.<name>` fill/stroke resolves against the palette. A call to
/// one is then expanded rather than tallied as a skip. A tree that defines none yields an empty map.
///
/// The template chain is walked imports-first so a file's own furniture is lowered only once the palettes it
/// imports are in hand. A collected name that clashes with a built-in construct is refused inside
/// [`lang::rules::collect_template_fns`], so a template's own `#let styled-box` never shadows the reader's.
fn collect_book_template_fns(root_src: &str, root_dir: &Path, body_size: Sp) -> lang::rules::TemplateFns {
	let mut palette	= lang::rules::Palette::new();
	let mut tfns	= lang::rules::TemplateFns::new();
	// The template chain the root imports: builds the palette and collects the template's furniture (aside-box).
	for line in root_src.lines() {
		let t = line.trim_start();
		if let Some(rest) = t.strip_prefix("#import") {
			if let Some(rel) = first_quoted(rest) {
				walk_template_imports(root_dir, &rel, body_size, &mut palette, &mut tfns, 0);
			}
		}
	}
	// The root's own definitions, and each included chapter's (pr-note), with the palette now in hand.
	lang::rules::collect_palette(root_src, &mut palette);
	lang::rules::collect_template_fns(root_src, body_size, &palette, &mut tfns);
	for line in root_src.lines() {
		let t = line.trim_start();
		if let Some(rest) = t.strip_prefix("#include") {
			if let Some(rel) = first_quoted(rest) {
				if let Ok(src) = vfs::read_to_string(&root_dir.join(&rel)) {
					lang::rules::collect_template_fns(&src, body_size, &palette, &mut tfns);
				}
			}
		}
	}
	tfns
}

/// Follows a local `#import "<rel>"` from `dir`, imports-first, collecting each file's `#let colours`
/// palette and then its furniture definitions -- so a file's fills resolve against the palettes its own
/// imports supply. A package import (`@preview/...`), a missing file, or a cycle past the depth cap is
/// skipped.
fn walk_template_imports(
	dir:		&Path,
	rel:		&str,
	body_size:	Sp,
	palette:	&mut lang::rules::Palette,
	tfns:		&mut lang::rules::TemplateFns,
	depth:		u32,
)
{
	if depth > 4 || rel.starts_with('@') {
		return;
	}
	let path = dir.join(rel);
	let src = match vfs::read_to_string(&path) {
		Ok(s)	=> s,
		Err(_)	=> return,
	};
	let next_dir = path.parent().unwrap_or(dir);
	// Imports first, so a palette or furniture this file depends on is collected before its own.
	for line in src.lines() {
		let t = line.trim_start();
		if let Some(rest) = t.strip_prefix("#import") {
			if let Some(inner_rel) = first_quoted(rest) {
				walk_template_imports(next_dir, &inner_rel, body_size, palette, tfns, depth + 1);
			}
		}
	}
	lang::rules::collect_palette(&src, palette);
	lang::rules::collect_template_fns(&src, body_size, palette, tfns);
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ INCLUDE FOLLOWING                                                          │
// └───────────────────────────────────────────────────────────────────────────┘

/// Recursion depth cap on `#include` following. No real book nests chapters anywhere near this deep, so
/// hitting it is itself the sign of a self- or mutually-referential include cycle; that is reported as a
/// refusal rather than recursed into forever or dropped without a trace.
const MAX_INCLUDE_DEPTH: u32 = 64;

/// One open `#if` include guard on the assembler's stack while it walks a file's lines. `live` records
/// that this guard actually decides emission: its parent branch was being kept, and its condition was one
/// the evaluator could resolve. A guard nested inside a dropped branch, or one whose form was refused, is
/// not `live` and keeps neither branch. `then_taken` is the resolved condition; `in_else` tracks which of
/// the two branches the walk is currently inside.
///
/// `state` is the guard's own bracket balance, seeded from its opener line so it starts at depth one: a
/// lone `]` deeper inside the branch (a `#block[...]`/`#align(..)[...]`/`#quote[...]` closer) is then told
/// apart from the guard's own matching closer by depth alone, rather than by line text -- the marker-based
/// extent this replaces treated any bare `]` line as the guard's end, following both branches once one
/// closed early and leaking the markers and the truncated tail as prose. `refused` marks a guard pushed
/// only to keep this bracket balance for an unsupported form already reported at its opener, so the
/// balance reaching zero on an ordinary body line (its own closer, not the guard's `]`/`else` shape) is not
/// reported a second time.
struct GuardFrame {
	live:		bool,
	then_taken:	bool,
	in_else:	bool,
	refused:	bool,
	state:		lang::parse::SkipState,
}

impl GuardFrame {
	/// Should the branch currently open under this guard have its content and includes emitted? A
	/// non-live guard emits from neither branch; a live one emits the then-branch when the condition held
	/// and the else-branch when it did not.
	fn emits(&self) -> bool {
		self.live && (self.then_taken != self.in_else)
	}
}

/// Does this whitespace-trimmed line open an evaluable `#if` include guard -- `#if <cond> [` with the
/// content bracket last on the line? Returns the condition text between `#if ` and the `[`. The one-line
/// and non-bracket forms deliberately fail here, so [`assemble_into`] refuses rather than mis-follows them.
fn guard_open(marker: &str) -> Option<&str> {
	let Some(inner)	= marker.strip_prefix("#if ") else { return None; };
	let Some(cond)	= inner.strip_suffix('[') else { return None; };
	Some(cond.trim())
}

/// Is this whitespace-trimmed line the `] else [` divider between an include guard's two branches,
/// however its own internal spacing is written (`]else[`, `] else [`)?
fn is_guard_else(marker: &str) -> bool {
	let squeezed: String = marker.chars().filter(|c| !c.is_whitespace()).collect();
	squeezed == "]else["
}

/// Evaluates an include-guard condition to which branch to keep -- `Some(true)` for the then-branch,
/// `Some(false)` for the else-branch -- or `None` when the form is beyond the two the assembler reads or
/// its variable resolves to no value, so the caller refuses it rather than guessing.
///
/// The two forms are `<var> == "<literal>"` (kept when the resolved scalar equals the literal) and a bare
/// `<var>` (kept when the resolved boolean is true). The variable is resolved from the book's `config.typ`
/// first, then from the guard's own file -- so a book's `#import "config.typ": media` and a lone file's
/// own `#let` both answer.
fn eval_guard(cond: &str, config: &str, file_src: &str) -> Option<bool> {
	let cond = cond.trim();
	if let Some(eq) = cond.find("==") {
		let var = cond[..eq].trim();
		let rhs = cond[eq + 2..].trim();
		if !is_simple_ident(var) {
			return None;
		}
		let Some(lit) = string_literal(rhs) else { return None; };
		let Some(val) = guard_scalar(config, file_src, var) else { return None; };
		return Some(val == lit);
	}
	if is_simple_ident(cond) {
		return guard_bool(config, file_src, cond);
	}
	None
}

/// Is `s` a single plain identifier -- a config-variable name, no operator or call around it?
fn is_simple_ident(s: &str) -> bool {
	let mut cs = s.chars();
	match cs.next() {
		Some(c) if c.is_alphabetic() || c == '_'	=> {},
		_											=> return false,
	}
	cs.all(|c| c.is_alphanumeric() || c == '-' || c == '_')
}

/// The text inside a `"..."` string literal filling the whole of `s`, or `None` when `s` is not one.
fn string_literal(s: &str) -> Option<&str> {
	let Some(stripped)	= s.strip_prefix('"') else { return None; };
	let Some(inner)		= stripped.strip_suffix('"') else { return None; };
	// A stray interior quote would mean this is not one flat literal; the guard then refuses.
	if inner.contains('"') {
		return None;
	}
	Some(inner)
}

/// The scalar an include-guard variable resolves to: the book config's binding, else the guard file's own.
fn guard_scalar(config: &str, file_src: &str, name: &str) -> Option<String> {
	read_let_string(config, name).or_else(|| read_let_string(file_src, name))
}

/// The boolean an include-guard variable resolves to: the book config's binding, else the guard file's own.
fn guard_bool(config: &str, file_src: &str, name: &str) -> Option<bool> {
	read_let_bool(config, name).or_else(|| read_let_bool(file_src, name))
}

/// Follows a root's `#include "..."` lines in order, reading each chapter and setting it through the
/// reader, and lifts each `#part-page[...]` divider to a level-1 heading so the part titles keep their
/// place in the flow. The root's own inline markup between the code lines is read too, in document order:
/// a doc root opens with a section (`= Purpose ...`) written straight in the root before its includes,
/// where a book root carries only the template call. Non-code lines are accumulated and flushed through
/// the reader at each include or part boundary, so the reader sees whole markup runs -- a heading and its
/// paragraphs together -- and the root's opening section keeps its place ahead of the first chapter. The
/// template call itself (`#show: doc.with(...)`, `#import`, `#pagebreak`) is code the reader skips and
/// tallies, so it never leaks into the flow.
///
/// A chapter may itself `#include` a file -- Lucronics' `chap_dynstrat_captonic_dynamics.typ` pulls in
/// `../evidence/lucronics_evidence.typ` this way -- so the walk is recursive: [`assemble_into`] follows
/// every level's own includes, each resolved against *that file's own directory*, exactly as Typst
/// resolves one, rather than always against the book root's.
///
/// `config` is the book's `config.typ` source (empty for the documentation idiom, which has none), so a
/// `#if <var> == "..."` include guard in a chapter can be resolved against the same scalars the config
/// binds -- `media` above all -- and only the taken branch's includes followed. See [`assemble_into`].
pub fn assemble(root_src: &str, root_dir: &Path, root_path: &Path, tfns: &lang::rules::TemplateFns, config: &str)
	-> Outcome<(Vec<Block>, lang::Refusals)>
{
	let mut blocks: Vec<Block> = Vec::new();
	let mut skips = lang::Refusals::default();
	res!(assemble_into(root_src, root_dir, root_path, tfns, config, 0, &mut blocks, &mut skips));
	Ok((blocks, skips))
}

/// The recursive body of [`assemble`]. `dir` is the directory `src` was itself read from -- the book
/// root's directory at depth 0, an included chapter's own directory one level down -- so a `#include
/// "../x.typ"` climbs relative to wherever it is written, not the top of the book. An include that
/// cannot be read, or one written past [`MAX_INCLUDE_DEPTH`], is recorded as a refusal rather than left
/// to fall through into `buf`, where the generic reader would set its raw `#include "..."` line as
/// literal body text -- exactly the silent-loss failure this recursion exists to close.
fn assemble_into(
	src:	&str,
	dir:	&Path,
	path:	&Path,
	tfns:	&lang::rules::TemplateFns,
	config:	&str,
	depth:	u32,
	blocks:	&mut Vec<Block>,
	skips:	&mut lang::Refusals,
)
	-> Outcome<()>
{
	let mut buf = String::new();	// this file's own inline markup gathered since the last boundary
	// This file's own inline markup (its opening section, any tail after its last include) is tagged with
	// its own path, exactly as an included chapter's blocks are tagged with theirs -- see `Refusal`'s doc
	// comment on why the span alone does not already say which file it came from.
	let label = path.display().to_string();
	// The open `#if` include guards this file is walking inside, innermost last. A branch's content and
	// includes are followed only when every guard on the stack is keeping its currently-open branch;
	// otherwise they are dropped (reported once at the guard, never leaked as prose). See [`GuardFrame`].
	let mut guards: Vec<GuardFrame> = Vec::new();
	let mut byte: u32 = 0;	// running byte offset, so a refusal's span points at its own line (G4)
	for raw in src.split_inclusive('\n') {
		let start = byte;
		byte = byte.saturating_add(raw.len() as u32);
		// Strip the line terminator without treating it as a real character, exactly as the reader's own
		// line loop does (`lang::parse::to_blocks`), so the span below covers the line, not its newline.
		let mut line = raw;
		if let Some(s) = line.strip_suffix('\n') { line = s; }
		if let Some(s) = line.strip_suffix('\r') { line = s; }
		let end	= start.saturating_add(line.len() as u32);
		let span	= crate::ir::Span::new(start, end);

		let t = line.trim_start();
		let marker = t.trim_end();	// a guard marker line, matched clear of trailing whitespace
		// The innermost open guard's own bracket depth (`None` with no guard open at all). Seeded from the
		// opener line at one, this is what tells the guard's own matching closer apart from a `]` deeper
		// inside its branch -- see [`GuardFrame`].
		let guard_depth = guards.last().map(|g| g.state.open_brackets());

		// A guard closer `]` on its own line, exactly at the guard's own depth: close the innermost open
		// guard. A `]` deeper than that -- a `#block[...]`/`#align(..)[...]`/`#quote[...]` closer inside the
		// branch -- is not the guard's own and falls through below, to the generic per-line scan and then
		// to ordinary content. A lone `]` with no guard open at all is likewise ordinary content.
		if marker == "]" && guard_depth == Some(1) {
			res!(flush_inline(&mut buf, blocks, skips, &label, tfns));
			guards.pop();
			continue;
		}
		// A guard divider `] else [`, at the guard's own depth: switch the innermost guard to its else
		// branch. Its `]` closes the content bracket and its `[` reopens it, so the depth is unchanged and
		// the state is left as it stands.
		if is_guard_else(marker) && guard_depth == Some(1) {
			res!(flush_inline(&mut buf, blocks, skips, &label, tfns));
			if let Some(top) = guards.last_mut() {
				top.in_else = true;
			}
			continue;
		}
		// A guard opener `#if <cond> [`: evaluate the condition against the config (and this file's own
		// `#let` bindings) and open a guard, seeding its bracket state from this opener line so it starts
		// at depth one. A guard opened inside a dropped branch, or one whose form or variable the evaluator
		// cannot resolve, keeps neither branch -- the latter is reported, so an unsupported guard form is
		// never silently followed nor leaked.
		if let Some(cond) = guard_open(marker) {
			res!(flush_inline(&mut buf, blocks, skips, &label, tfns));
			let parent_active = guards.iter().all(|g| g.emits());
			let (live, then_taken) = if !parent_active {
				(false, false)
			} else {
				match eval_guard(cond, config, src) {
					Some(taken)	=> (true, taken),
					None		=> {
						skips.record(&fmt!("#if {} (unsupported include-guard form)", cond), span);
						skips.tag_file(&label);
						(false, false)
					},
				}
			};
			let mut state = lang::parse::SkipState::new();
			lang::parse::scan_brackets(marker, &mut state);
			guards.push(GuardFrame { live, then_taken, in_else: false, refused: false, state });
			continue;
		}
		// Any other `#if ...` line is a guard form the assembler does not evaluate (a one-line
		// `#if c [..] else [..]`, or a brace-bodied `#if cond {`): refuse and drop it rather than let its
		// raw source leak. A balanced one-liner refuses just this line, as before; a brace body still open
		// at the line's end pushes a refused guard so the generic per-line scan below consumes the whole
		// block -- its body, `} else {` and closing `}` -- instead of leaking it as prose. `#if(` with no
		// space is left to the reader's own code-skip path.
		if marker.starts_with("#if ") && guards.iter().all(|g| g.emits()) {
			res!(flush_inline(&mut buf, blocks, skips, &label, tfns));
			skips.record(&fmt!("#if (unsupported include-guard form): {:?}", marker), span);
			skips.tag_file(&label);
			let mut state = lang::parse::SkipState::new();
			lang::parse::scan_brackets(marker, &mut state);
			if state.has_open_bracket() {
				guards.push(GuardFrame { live: false, then_taken: false, in_else: false, refused: true, state });
			}
			continue;
		}
		// Any other line while a guard is open: fold its own brackets into the innermost guard's state,
		// whether or not the branch it stands in emits -- a dropped branch's own `#block[...]`/`{...}` still
		// balances the stack, so a later real closer is not mistaken for one of these (or vice versa). If
		// the state closes to zero here, rather than through one of the recognised `]`/`else`/`#if` shapes
		// above, the guard's own bracket has just ended on an ordinary body line: a refused guard already
		// reported its opener, so this is its expected close and stays silent; any other guard closing this
		// way is a shape the guard did not predict, so it is reported rather than left to leak whatever
		// follows as prose. Either way the line itself is the guard's own structural end, not content, so
		// it is consumed here rather than falling through to the buffer below.
		if let Some(top) = guards.last_mut() {
			lang::parse::scan_brackets(line, &mut top.state);
			if !top.state.has_open_bracket() {
				let refused = top.refused;
				res!(flush_inline(&mut buf, blocks, skips, &label, tfns));
				guards.pop();
				if !refused {
					skips.record(&fmt!("#if guard closed on an unrecognised line: {:?}", marker), span);
					skips.tag_file(&label);
				}
				continue;
			}
		}
		// Inside a dropped or refused branch: the content is the untaken alternative, dropped silently
		// (the guard already carries the report). Markers above are still tracked so the stack balances.
		if !guards.iter().all(|g| g.emits()) {
			continue;
		}
		if let Some(rest) = t.strip_prefix("#include") {
			res!(flush_inline(&mut buf, blocks, skips, &label, tfns));
			match first_quoted(rest) {
				Some(rel) if depth >= MAX_INCLUDE_DEPTH => {
					skips.record(&fmt!("#include {:?} (cycle: depth exceeds {})", rel, MAX_INCLUDE_DEPTH),
						span);
					skips.tag_file(&label);
				},
				Some(rel) => {
					let inc_path = dir.join(&rel);
					let inc_src = match vfs::read_to_string(&inc_path) {
						Ok(s)	=> s,
						Err(e)	=> return Err(err!(e,
							"Could not read the included chapter {:?}.", inc_path; File, Read)),
					};
					let inc_dir = inc_path.parent().unwrap_or(dir);
					let mut chap_blocks: Vec<Block> = Vec::new();
					let mut chap_skips = lang::Refusals::default();
					res!(assemble_into(&inc_src, inc_dir, &inc_path, tfns, config, depth + 1,
						&mut chap_blocks, &mut chap_skips));
					// The chapter's own top-level `#set`/`#show: doc.with(...)` declarations lower to a patch
					// scoped to this chapter's subtree (H1): the reader captures them but holds no theme to lower
					// them onto, so it is done here, where the chapter boundary is known. A chapter that declares
					// no styling -- every corpus chapter today -- lowers to an empty patch and nests nothing,
					// splicing its blocks in flat and keeping the block stream and the render byte-identical.
					let chap_patch = lang::set::lower_declarations(&inc_src);
					if chap_patch == ThemePatch::default() {
						blocks.extend(chap_blocks);
					} else {
						blocks.push(Block::Scoped { patch: chap_patch, blocks: chap_blocks });
					}
					skips.merge(chap_skips);
				},
				None => {
					// A malformed `#include` with no quoted path: reported, not left to fall through as a
					// literal line of body text.
					skips.record("#include", span);
					skips.tag_file(&label);
				},
			}
		} else if t.starts_with("#part-page") {
			res!(flush_inline(&mut buf, blocks, skips, &label, tfns));
			// A part divider: its title is the last bracket group on the line. A part is a level-0 heading
			// -- unnumbered and centred on its own page, outside the chapter numbering -- so a chapter keeps
			// its number across a part boundary and a part never appears in a running head.
			if let Some(title) = bracket_body(t) {
				blocks.push(Block::heading(0, title));
			}
		} else {
			buf.push_str(line);
			buf.push('\n');
		}
	}
	// A guard still open at end of file never met its own closer: reported so a truncated branch is never
	// silently accepted as complete. A refused guard already reported its opener, so only a guard that was
	// genuinely live and open is reported here, to avoid a duplicate on the one already-reported form.
	let eof = crate::ir::Span::new(byte, byte);
	for g in &guards {
		if !g.refused {
			skips.record("#if guard never closed (end of file)", eof);
			skips.tag_file(&label);
		}
	}
	// The tail after the last include: back-matter markup a doc root (or the last chapter of a nested
	// include) closes with, if any.
	res!(flush_inline(&mut buf, blocks, skips, &label, tfns));
	Ok(())
}

/// Reads the accumulated inline markup through the reader, appending its blocks and merging its skips
/// (tagged with `file`, the root's own path -- this buffer is always the root's inline text, never a
/// chapter's, which is tagged separately where it is read), then clears the buffer. A buffer holding
/// only code and whitespace yields no blocks -- a book root's template call reduces to nothing, so the
/// book path is unchanged.
fn flush_inline(
	buf:	&mut String,
	blocks:	&mut Vec<Block>,
	skips:	&mut lang::Refusals,
	file:	&str,
	tfns:	&lang::rules::TemplateFns,
)
	-> Outcome<()>
{
	if !buf.trim().is_empty() {
		let (b, mut s) = res!(lang::to_blocks_with_templates(buf, tfns));
		s.tag_file(file);
		blocks.extend(b);
		skips.merge(s);
	}
	buf.clear();
	Ok(())
}

/// The first double-quoted run in a slice, its contents without the quotes.
fn first_quoted(s: &str) -> Option<String> {
	let open	= s.find('"')?;
	let rest	= &s[open + 1..];
	let close	= rest.find('"')?;
	Some(rest[..close].to_string())
}

/// The contents of the first `[...]` group in a line, balanced so a nested bracket does not close it
/// early. Used to lift a `#part-page[Title]` divider's title.
fn bracket_body(s: &str) -> Option<String> {
	let open	= s.find('[')?;
	let bytes	= s.as_bytes();
	let mut depth	= 0i32;
	let mut i	= open;
	while i < bytes.len() {
		match bytes[i] {
			b'['	=> depth += 1,
			b']'	=> {
				depth -= 1;
				if depth == 0 {
					return Some(s[open + 1..i].trim().to_string());
				}
			},
			_	=> {},
		}
		i += 1;
	}
	None
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ CONFIG EXTRACTION                                                          │
// └───────────────────────────────────────────────────────────────────────────┘

/// The raw type values read from a config arm, before they are turned into a [`Style`]. Kept apart from
/// the geometry because the geometry is complete on its own, while the style also needs the loaded font
/// metrics to turn an em-leading into a baseline distance.
struct RawStyle {
	body_pt:	f64,
	leading_em:	f64,	// par leading, a multiple of the em
	par_skip_em:	f64,	// space between paragraphs, a multiple of the em
	indent_em:	f64,	// first-line indent, a multiple of the em
	chap_num_pt:	f64,	// the giant chapter-opener number size
	chap_grid:	[f64; 4],	// chapter-opener grid rows: number band, gap, title band, gap-to-body, in points
	h1_pt:		f64,	// chapter-title size
	h2_pt:		f64,	// level-2 sub-heading size
	h3_pt:		f64,	// level-3 sub-heading size
	h4_pt:		f64,	// level-4 sub-heading size
}

/// Reads the branch of a config the `format` switch selects into a geometry and the raw type values.
/// A book that omits a field falls back to a readable default rather than failing, so an unfamiliar
/// config still assembles.
fn read_config(src: &str) -> Outcome<(PageGeometry, RawStyle)> {
	let format = match read_let_string(src, "format") {
		Some(f)	=> f,
		None	=> return Err(err!(
			"The config sets no `#let format = \"...\"`, so no page branch can be chosen."; Input, Missing)),
	};

	let dims	= arm(src, "page-dims", &format);
	let margins	= arm(src, "page-margins", &format);
	let scale	= arm(src, "type-scale", &format);

	let width	= dims.as_deref().and_then(|a| num_after(a, "width:")).unwrap_or(148.0);
	let height	= dims.as_deref().and_then(|a| num_after(a, "height:")).unwrap_or(210.0);
	let inside	= margins.as_deref().and_then(|a| num_after(a, "inside:")).unwrap_or(19.0);
	let outside	= margins.as_deref().and_then(|a| num_after(a, "outside:")).unwrap_or(17.0);
	let top		= margins.as_deref().and_then(|a| num_after(a, "top:")).unwrap_or(19.0);
	let bottom	= margins.as_deref().and_then(|a| num_after(a, "bottom:")).unwrap_or(21.0);

	let geom = PageGeometry::with_margins(
		Sp::from_pt(width  * MM_PER_PT),
		Sp::from_pt(height * MM_PER_PT),
		Sp::from_pt(inside  * MM_PER_PT),
		Sp::from_pt(outside * MM_PER_PT),
		Sp::from_pt(top    * MM_PER_PT),
		Sp::from_pt(bottom * MM_PER_PT),
	);

	let body_pt		= arm(src, "body-text-size", &format).as_deref().and_then(first_num).unwrap_or(11.0);
	let leading_em	= arm(src, "body-line-spacing", &format).as_deref().and_then(first_num).unwrap_or(0.75);
	let par_skip_em	= arm(src, "body-par-spacing", &format).as_deref().and_then(first_num).unwrap_or(0.75);
	let indent_em	= arm(src, "body-par-indent", &format).as_deref().and_then(first_num).unwrap_or(0.0);
	let chap_num_pt	= scale.as_deref().and_then(|a| num_after(a, "chapter-num:")).unwrap_or(54.0);
	// The chapter-opener grid rows: the number band, the gap below it, the title band, and the gap down to
	// the body. Absent, the opener falls back to spacers roughly matching a 20 pt body scale.
	let grid		= scale.as_deref().and_then(|a| tuple_after(a, "chapter-grid-rows:")).unwrap_or_default();
	let chap_grid	= [
		grid.first().copied().unwrap_or(72.0),
		grid.get(1).copied().unwrap_or(8.0),
		grid.get(2).copied().unwrap_or(36.0),
		grid.get(3).copied().unwrap_or(20.0),
	];
	let h1_pt		= scale.as_deref().and_then(|a| num_after(a, "chapter-title:")).unwrap_or(20.0);
	// The template sizes a sub-heading by `sub-headings.at(level - 1)`: level 2 takes the second entry,
	// level 3 the third, level 4 the fourth. The first entry is the section-title reserve, unused by the
	// show rule, so the level-2 heading is only a step above the body.
	let subs		= scale.as_deref().and_then(|a| tuple_after(a, "sub-headings:")).unwrap_or_default();
	let h2_pt		= subs.get(1).copied().unwrap_or(12.5);
	let h3_pt		= subs.get(2).copied().unwrap_or(11.5);
	let h4_pt		= subs.get(3).copied().unwrap_or(11.0);

	Ok((geom, RawStyle { body_pt, leading_em, par_skip_em, indent_em, chap_num_pt, chap_grid, h1_pt, h2_pt, h3_pt, h4_pt }))
}

/// Turns the raw config values into a [`Theme`]. The leading is the one derived value: the config sets
/// a gap in ems, and the driver wants a baseline-to-baseline distance, so the Libertinus line box (the
/// theme's [`ThemeCalibration::line_box_em`](crate::theme::ThemeCalibration), which documents the
/// calibration) is added to it -- what puts the line grid on the oracle's.
fn build_style(raw: &RawStyle) -> Theme {
	let mut style = Theme::default();
	let baseline = (style.calibration.line_box_em + raw.leading_em) * raw.body_pt;

	style.text.body_size	= Sp::from_pt(raw.body_pt);
	style.text.leading	= Sp::from_pt(baseline);
	style.par.skip	= Sp::from_pt(raw.par_skip_em * raw.body_pt);
	style.par.indent	= Sp::from_pt(raw.indent_em * raw.body_pt);
	style.opener.chap_num_size	= Sp::from_pt(raw.chap_num_pt);
	style.opener.chap_grid		= [
		Sp::from_pt(raw.chap_grid[0]),
		Sp::from_pt(raw.chap_grid[1]),
		Sp::from_pt(raw.chap_grid[2]),
		Sp::from_pt(raw.chap_grid[3]),
	];
	style.heading.levels[0].size	= Sp::from_pt(raw.h1_pt);
	style.heading.levels[1].size	= Sp::from_pt(raw.h2_pt);
	style.heading.levels[2].size	= Sp::from_pt(raw.h3_pt);
	style.heading.levels[3].size	= Sp::from_pt(raw.h4_pt);
	style
}

/// The string a `#let <name> = "..."` binds, if the config sets one as a plain literal.
fn read_let_string(src: &str, name: &str) -> Option<String> {
	let needle	= fmt!("#let {} =", name);
	let at		= src.find(&needle)?;
	let rest	= &src[at + needle.len()..];
	first_quoted(rest)
}

/// The boolean a `#let <name> = true` / `= false` binds, if the source sets one as a plain literal. A
/// binding to anything else (a string, an expression) is not a boolean an include guard can test, so it
/// yields `None` and the guard refuses rather than inventing a truth value.
fn read_let_bool(src: &str, name: &str) -> Option<bool> {
	let needle		= fmt!("#let {} =", name);
	let Some(at)	= src.find(&needle) else { return None; };
	let rest	= src[at + needle.len()..].trim_start();
	let tok: String = rest.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
	match tok.as_str() {
		"true"	=> Some(true),
		"false"	=> Some(false),
		_		=> None,
	}
}

/// The body of the `if`/`else if` arm a `#let <name> = if format == "<fmt>" {...}` chain selects for
/// `fmt`. Bounds the search to the one `#let` so a later binding's arms are not read by mistake, finds
/// the arm whose condition tests this format, and returns its balanced `{...}` body.
fn arm(src: &str, name: &str, fmt: &str) -> Option<String> {
	let needle	= fmt!("#let {} =", name);
	let start	= src.find(&needle)?;
	let tail	= &src[start + needle.len()..];
	// The binding ends at the next top-level `#let`, or the end of the file.
	let end		= tail.find("\n#let ").unwrap_or(tail.len());
	let block	= &tail[..end];

	let cond	= fmt!("== \"{}\"", fmt);
	let at		= block.find(&cond)?;
	let after	= &block[at..];
	let brace	= after.find('{')?;
	balanced_braces(&after[brace..])
}

/// The contents of a `{...}` at the start of `s`, matched by brace depth so a nested record does not
/// close it early.
fn balanced_braces(s: &str) -> Option<String> {
	let bytes	= s.as_bytes();
	let mut depth	= 0i32;
	let mut i	= 0usize;
	while i < bytes.len() {
		match bytes[i] {
			b'{'	=> depth += 1,
			b'}'	=> {
				depth -= 1;
				if depth == 0 {
					return Some(s[1..i].to_string());
				}
			},
			_	=> {},
		}
		i += 1;
	}
	None
}

/// The first number after `key` in `s` -- the digits and one decimal point that follow the key. The
/// unit (`mm`, `pt`, `em`) is known from the key, so it is read off and dropped.
fn num_after(s: &str, key: &str) -> Option<f64> {
	let at	= s.find(key)?;
	first_num(&s[at + key.len()..])
}

/// The first number appearing anywhere in `s`, as an `f64` -- the leading numeric run after any
/// non-numeric lead-in. `11pt` and `0.75em` both read as their number.
fn first_num(s: &str) -> Option<f64> {
	let bytes	= s.as_bytes();
	let mut i	= 0usize;
	// Skip to the first digit or a decimal point that starts a number.
	while i < bytes.len() && !(bytes[i].is_ascii_digit() || bytes[i] == b'.') {
		i += 1;
	}
	let begin = i;
	while i < bytes.len() && (bytes[i].is_ascii_digit() || bytes[i] == b'.') {
		i += 1;
	}
	if i == begin {
		return None;
	}
	s[begin..i].parse::<f64>().ok()
}

/// The numbers of the first `( ... )` tuple after `key` -- `sub-headings: (15pt, 12.5pt, ...)` reads as
/// `[15.0, 12.5, ...]`.
fn tuple_after(s: &str, key: &str) -> Option<Vec<f64>> {
	let at		= s.find(key)?;
	let after	= &s[at + key.len()..];
	let open	= after.find('(')?;
	let close	= after[open..].find(')')?;
	let inner	= &after[open + 1..open + close];
	let nums: Vec<f64> = inner.split(',').filter_map(first_num).collect();
	Some(nums)
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ DOC TEMPLATE EXTRACTION HELPERS                                            │
// └───────────────────────────────────────────────────────────────────────────┘

/// The trim of a named paper, in millimetres. Only the sizes the doc trees reach for are tabulated;
/// an unknown name falls to A4, so a doc that sets an exotic paper still lands on a readable page.
fn paper_dims_mm(name: &str) -> (f64, f64) {
	match name {
		"a3"		=> (297.0, 420.0),
		"a4"		=> (210.0, 297.0),
		"a5"		=> (148.0, 210.0),
		"us-letter"	=> (215.9, 279.4),
		"us-legal"	=> (215.9, 355.6),
		_			=> (210.0, 297.0),
	}
}

/// The first `"..."` string after `key` anywhere in `src` -- `paper: "a4"` reads as `a4`. Used to read a
/// bare `name: "value"` setting that is not bounded by the field machinery the book path needs.
fn first_quoted_after(src: &str, key: &str) -> Option<String> {
	let at = src.find(key)?;
	first_quoted(&src[at + key.len()..])
}

/// The value bound to `field` inside a top-level `#let <dict> = ( ... )` dictionary -- the `a4:` entry of
/// the template's `#let margins = (a4: 2.5cm, ...)`, say. The dictionary is matched by paren depth from
/// the `#let`, and the field's value runs to the next depth-zero comma, so a nested group does not end it.
fn let_dict_field(src: &str, dict: &str, field: &str) -> Option<String> {
	let needle	= fmt!("#let {} =", dict);
	let start	= src.find(&needle)?;
	let tail	= &src[start + needle.len()..];
	let open	= tail.find('(')?;
	let body	= balanced_parens(&tail[open..])?;
	// Within the dictionary body, find `field:` and take its value up to the next top-level comma.
	let key		= fmt!("{}:", field);
	let at		= body.find(&key)?;
	let rest	= &body[at + key.len()..];
	let bytes	= rest.as_bytes();
	let mut depth	= 0i32;
	let mut end		= rest.len();
	let mut i		= 0usize;
	while i < bytes.len() {
		match bytes[i] as char {
			'(' | '[' | '{'		=> depth += 1,
			')' | ']' | '}'		=> depth -= 1,
			',' if depth == 0	=> { end = i; break; },
			_					=> {},
		}
		i += 1;
	}
	Some(rest[..end].trim().to_string())
}

/// A Typst length token as points: the leading number scaled by its unit (`cm`, `mm`, `in`, `pt`). A
/// bare number with no unit reads as points. `None` when no number leads the slice.
fn parse_len_pt(s: &str) -> Option<f64> {
	let n = first_num(s)?;
	let unit = s.trim_start_matches(|c: char| c.is_ascii_digit() || c == '.' || c == '-' || c.is_whitespace());
	let per_pt = if unit.starts_with("cm") {
		10.0 * MM_PER_PT
	} else if unit.starts_with("mm") {
		MM_PER_PT
	} else if unit.starts_with("in") {
		72.0
	} else {
		1.0	// `pt` or unitless
	};
	Some(n * per_pt)
}

/// The first length after `key` in `src`, in points -- `text-size: 11pt` reads as `11.0`.
fn first_len_after(src: &str, key: &str) -> Option<f64> {
	let at = src.find(key)?;
	parse_len_pt(&src[at + key.len()..])
}

#[cfg(test)]
mod tests {
	use super::*;

	// A miniature two-format config with the shape the real books use: a `format` switch and a chain of
	// `if format == "..." {...}` arms per setting.
	const CFG: &str = r#"
#let format = "ingram-5x8"
#let page-dims = if format == "ingram-5x8" {
  (width: 127mm, height: 203mm)
} else {
  (width: 148mm, height: 210mm)
}
#let page-margins = if format == "ingram-5x8" {
  (inside: 17mm, outside: 15mm, top: 18mm, bottom: 18mm)
} else {
  (inside: 19mm, outside: 17mm, top: 19mm, bottom: 21mm)
}
#let body-text-size = if format == "ingram-5x8" { 11pt } else { 12pt }
#let body-line-spacing = if format == "ingram-5x8" { 0.75em } else { 0.75em }
#let body-par-spacing = if format == "ingram-5x8" { 0.75em } else { 0.75em }
#let type-scale = if format == "ingram-5x8" {
  ( title: 24pt, chapter-title: 20pt, sub-headings: (15pt, 12.5pt, 11.5pt, 11pt) )
} else {
  ( title: 27pt, chapter-title: 23pt, sub-headings: (17pt, 14.5pt, 13pt, 12.5pt) )
}
"#;

	#[test]
	fn test_the_selected_format_arm_is_read_00() -> Outcome<()> {
		let (geom, raw) = res!(read_config(CFG));
		// 127 mm and 203 mm in points, not the a5 fallback branch.
		assert_eq!(geom.width.to_pt().round() as i64, 360, "width should be 127 mm = 360 pt");
		assert_eq!(geom.height.to_pt().round() as i64, 575, "height should be 203 mm = 575 pt");
		// Mirror margins: inside binds wider than the fore-edge.
		assert_eq!(geom.inside.to_pt().round() as i64, 48, "inside 17 mm = 48 pt");
		assert_eq!(geom.outside.to_pt().round() as i64, 43, "outside 15 mm = 43 pt");
		assert!(geom.inside > geom.outside, "the binding margin is the wider of the two");
		assert!((raw.body_pt - 11.0).abs() < 1e-9, "body 11 pt, found {}", raw.body_pt);
		assert!((raw.h1_pt - 20.0).abs() < 1e-9, "h1 = chapter-title 20 pt, found {}", raw.h1_pt);
		// The level-2 heading takes sub-headings[1], not the first (section-title) entry.
		assert!((raw.h2_pt - 12.5).abs() < 1e-9, "h2 = sub-heading[1] 12.5 pt, found {}", raw.h2_pt);
		assert!((raw.h3_pt - 11.5).abs() < 1e-9, "h3 = sub-heading[2] 11.5 pt, found {}", raw.h3_pt);
		assert!((raw.h4_pt - 11.0).abs() < 1e-9, "h4 = sub-heading[3] 11 pt, found {}", raw.h4_pt);
		Ok(())
	}

	#[test]
	fn test_the_mirror_shift_moves_a_verso_to_the_fore_edge_01() -> Outcome<()> {
		let (geom, _) = res!(read_config(CFG));
		// Recto content starts at the inside margin; the verso shift lands it at the outside one.
		let verso_left = geom.content_left() + geom.mirror_shift();
		assert_eq!(verso_left, geom.outside, "a shifted verso page's left edge is the fore-edge margin");
		Ok(())
	}

	#[test]
	fn test_a_root_with_includes_reads_as_a_book_02() {
		assert!(is_book_root("#show: doc.with()\n#include \"chap_01.typ\"\n"));
		assert!(!is_book_root("= A lone heading\n\nSome prose.\n"));
	}

	#[test]
	fn test_a_typst_length_reads_in_points_04() -> Outcome<()> {
		assert!((res!(parse_len_pt("11pt").ok_or_else(|| err!("no number"; Test, Bug))) - 11.0).abs() < 1e-9);
		// 2.5 cm = 25 mm = 25 * 72 / 25.4 = 70.866 pt.
		let cm = res!(parse_len_pt("2.5cm").ok_or_else(|| err!("no number"; Test, Bug)));
		assert!((cm - 70.866).abs() < 1e-2, "2.5 cm should be ~70.87 pt, found {}", cm);
		let mm = res!(parse_len_pt("18mm").ok_or_else(|| err!("no number"; Test, Bug)));
		assert!((mm - 51.024).abs() < 1e-2, "18 mm should be ~51.02 pt, found {}", mm);
		// A bare number reads as points.
		assert!((res!(parse_len_pt("150").ok_or_else(|| err!("no number"; Test, Bug))) - 150.0).abs() < 1e-9);
		Ok(())
	}

	#[test]
	fn test_the_margin_dict_field_and_paper_read_05() -> Outcome<()> {
		let tmpl = r#"
#let margins = (
  a4: 2.5cm,
  title_page: 45%,
  section_header: 150pt,
)
#let doc(body) = {
  set page(paper: "a4", margin: (top: 0pt))
  body
}
"#;
		let a4 = res!(let_dict_field(tmpl, "margins", "a4").ok_or_else(|| err!("no a4 field"; Test, Bug)));
		assert_eq!(a4, "2.5cm", "the margins.a4 entry is the 2.5cm length");
		let paper = res!(first_quoted_after(tmpl, "paper:").ok_or_else(|| err!("no paper"; Test, Bug)));
		assert_eq!(paper, "a4", "the page paper is a4");
		let (w, h) = paper_dims_mm(&paper);
		assert_eq!((w as i64, h as i64), (210, 297), "a4 is 210 by 297 mm");
		Ok(())
	}

	#[test]
	fn test_doc_front_matter_reads_title_subtitle_author_06() {
		let root = r#"
#show: doc.with(
  title: [Austenite],
  subtitle: [Design Document],
  text-size: 11pt,
  meta-data: (
    ( version: "0.1.0", authors: "J. D. Hoogland", notes: "Initial." ),
  ),
)
= Purpose
"#;
		let raw = RawStyle {
			body_pt: 11.0, leading_em: 0.65, par_skip_em: 0.65, indent_em: 0.0,
			chap_num_pt: 54.0, chap_grid: [72.0, 8.0, 36.0, 20.0],
			h1_pt: 14.0, h2_pt: 12.0, h3_pt: 13.0, h4_pt: 12.0,
		};
		let fm = read_doc_front_matter(std::path::Path::new("/nonexistent"), root, &raw, "Austenite");
		assert_eq!(fm.title, "Austenite");
		assert_eq!(fm.subtitle.as_deref(), Some("Design Document"));
		assert_eq!(fm.author, "J. D. Hoogland");
		// A doc tree carries no book imprint (no ISBN), but it does compose the template's colophon: the
		// revision fields, the copyright line and the acknowledgement paragraph the meta page seats.
		assert!(fm.isbn.is_none(), "a doc tree carries no book imprint");
		assert_eq!(fm.meta_rows.len(), 1, "one revision row");
		assert_eq!(fm.meta_rows[0].version.as_deref(), Some("0.1.0"));
		assert_eq!(fm.meta_rows[0].notes.as_deref(), Some("Initial."));
		assert!(fm.copyright.as_deref().unwrap_or_default().contains("All rights reserved."),
			"a doc meta page carries a copyright line");
		assert!(fm.acknowledgement.is_some(), "a doc meta page carries an acknowledgement");
		// A doc tree always draws the template's two-column title page, so the sidebar is marked even when
		// the miniature root names no colour; the fraction falls to the template default with no template file.
		assert!(fm.sidebar_grey.is_some(), "a doc title page draws the sidebar");
		assert!((fm.sidebar_frac - 0.45).abs() < 1e-9, "the sidebar default fraction is 0.45");
	}

	#[test]
	fn test_doc_front_matter_reads_declaration_mark_07() {
		let root = r#"
#show: doc.with(
  title: [Austenite],
  footer-left-logo-path: "assets/svg/fe2o3_logo_text_right.svg",
  meta-data: (
    ( version: "0.1.0", date: "12026-08-08", authors: "J. D. Hoogland", declaration: "with-ai", notes: "Created." ),
  ),
)
= Purpose
"#;
		let raw = RawStyle {
			body_pt: 11.0, leading_em: 0.65, par_skip_em: 0.65, indent_em: 0.0,
			chap_num_pt: 54.0, chap_grid: [72.0, 8.0, 36.0, 20.0],
			h1_pt: 14.0, h2_pt: 12.0, h3_pt: 13.0, h4_pt: 12.0,
		};
		let fm = read_doc_front_matter(std::path::Path::new("/nonexistent"), root, &raw, "Austenite");
		assert_eq!(fm.meta_rows.len(), 1, "one revision row");
		let mr = &fm.meta_rows[0];
		assert_eq!(mr.date.as_deref(), Some("12026-08-08"));
		assert_eq!(mr.ai_mark_words.as_deref(), Some("Made with AI"));
		assert!(mr.ai_mark_path.as_deref().unwrap_or_default().ends_with("doc_made_with_ai_opt.svg"),
			"the with-ai slug picks the doc mark image");
		assert_eq!(fm.footer_logo.as_deref(), Some("assets/svg/fe2o3_logo_text_right.svg"));
	}

	#[test]
	fn test_doc_front_matter_reads_multiple_rows_and_custom_words_09() {
		let root = r#"
#show: doc.with(
  title: [Hematite],
  meta-data: (
    ( version: "2.0.0", date: "12026-04-11", authors: "J. D. Hoogland", declaration: "entirely-ai", declaration-words: "Additions made entirely with AI", notes: "Restructured." ),
    ( version: "1.0.0", date: "12023-10-01", authors: "J. D. Hoogland", declaration: "some-ai", notes: "Initial release." ),
  ),
)
= Purpose
"#;
		let raw = RawStyle {
			body_pt: 11.0, leading_em: 0.65, par_skip_em: 0.65, indent_em: 0.0,
			chap_num_pt: 54.0, chap_grid: [72.0, 8.0, 36.0, 20.0],
			h1_pt: 14.0, h2_pt: 12.0, h3_pt: 13.0, h4_pt: 12.0,
		};
		let fm = read_doc_front_matter(std::path::Path::new("/nonexistent"), root, &raw, "Hematite");
		assert_eq!(fm.meta_rows.len(), 2, "both revision rows are read");
		assert_eq!(fm.meta_rows[0].version.as_deref(), Some("2.0.0"));
		// A `declaration-words` rescopes the caption without changing the mark image.
		assert_eq!(fm.meta_rows[0].ai_mark_words.as_deref(), Some("Additions made entirely with AI"));
		assert!(fm.meta_rows[0].ai_mark_path.as_deref().unwrap_or_default().ends_with("doc_made_with_ai_entirely_opt.svg"));
		assert_eq!(fm.meta_rows[1].version.as_deref(), Some("1.0.0"));
		assert_eq!(fm.meta_rows[1].ai_mark_words.as_deref(), Some("Made with some AI"));
	}

	#[test]
	fn test_doc_front_matter_reads_title_page_logos_08() {
		let root = r#"
#show: doc.with(
  title: [Austenite],
  subtitle: [Design Document],
  title-colour: "lightgrey",
  title-smallcaps: true,
  title-top-logo-path: "assets/svg/austenite_logo_text_right.svg",
  title-top-logo-width: 150pt,
  title-bottom-logo-path: "assets/svg/oxedyne_logo_dark_text_below_opt.svg",
  title-bottom-logo-width: 120pt,
  footer-left-logo-path: "assets/svg/fe2o3_logo_text_right.svg",
  meta-data: (
    ( version: "0.1.0", authors: "J. D. Hoogland", notes: "Initial." ),
  ),
)
= Purpose
"#;
		let raw = RawStyle {
			body_pt: 11.0, leading_em: 0.65, par_skip_em: 0.65, indent_em: 0.0,
			chap_num_pt: 54.0, chap_grid: [72.0, 8.0, 36.0, 20.0],
			h1_pt: 14.0, h2_pt: 12.0, h3_pt: 13.0, h4_pt: 12.0,
		};
		let fm = read_doc_front_matter(std::path::Path::new("/nonexistent"), root, &raw, "Austenite");
		assert_eq!(fm.sidebar_grey, Some(240), "lightgrey resolves to luma 240");
		assert!(fm.title_smallcaps, "the title sets in small caps");
		assert_eq!(fm.top_logo.as_deref(), Some("assets/svg/austenite_logo_text_right.svg"));
		assert_eq!(fm.bottom_logo.as_deref(), Some("assets/svg/oxedyne_logo_dark_text_below_opt.svg"));
		assert_eq!(fm.footer_logo.as_deref(), Some("assets/svg/fe2o3_logo_text_right.svg"));
		assert!((fm.top_logo_width.to_pt() - 150.0).abs() < 1e-6, "top logo width 150 pt");
		assert!((fm.bottom_logo_width.to_pt() - 120.0).abs() < 1e-6, "bottom logo width 120 pt");
	}

	#[test]
	fn test_root_inline_markup_is_read_before_includes_07() -> Outcome<()> {
		let dir = std::path::Path::new("/nonexistent");
		// A doc root opens with an inline section written straight in the root, ahead of its includes (the
		// Austenite design's `= Purpose`). With no include present, only that inline markup is read; its
		// heading and paragraph must both survive, and the template call above them must be skipped, not set.
		let root = "#import \"template.typ\": *\n#show: doc.with(title: [X])\n\n= Purpose\n\nAustenite is an engine.\n";
		let (blocks, _skips) = res!(assemble(root, dir, &dir.join("root.typ"), &lang::rules::TemplateFns::new(), ""));
		assert!(
			blocks.iter().any(|b| matches!(b, Block::Heading { level: 1, .. })),
			"the root's inline level-1 heading is read into the flow");
		assert!(
			blocks.iter().any(|b| matches!(b, Block::Paragraph { .. } | Block::RichParagraph { .. })),
			"the inline paragraph beneath the heading is read too");
		Ok(())
	}

	/// A `#if media == "..." [ ... ] else [ ... ]` include guard is evaluated, not both-branches-followed:
	/// only the taken branch's content survives, and the `#if`/`] else [`/`]` marker lines never leak as
	/// prose. The scalar resolves from the book config first (so a real `#import "config.typ": media`
	/// answers), then from the guard's own file; an unsupported guard form is refused and reported, never
	/// guessed or leaked. This is the DEFECT B regression gate at the unit level, beside the oracle fixture.
	#[test]
	fn if_media_guard_follows_only_the_taken_branch_08() -> Outcome<()> {
		let dir = std::path::Path::new("/nonexistent");
		let root = "#let media = \"ebook\"\n\nIntro paragraph.\n\n#if media == \"ebook\" [\nEbook only paragraph.\n] else [\nPrint only paragraph.\n]\n\nTail paragraph.\n";

		// File-local `#let media = "ebook"`, no config: the then-branch is taken.
		let (blocks, _skips) = res!(assemble(root, dir, &dir.join("root.typ"), &lang::rules::TemplateFns::new(), ""));
		let body = fmt!("{:?}", blocks);
		assert!(body.contains("Intro paragraph") && body.contains("Tail paragraph"),
			"prose around the guard must survive: {}", body);
		assert!(body.contains("Ebook only paragraph"), "the taken branch must render: {}", body);
		assert!(!body.contains("Print only paragraph"), "the untaken branch must be dropped: {}", body);
		assert!(!body.contains("] else [") && !body.contains("#if "),
			"no guard marker line may leak as prose: {}", body);

		// The book config binds `media = "print"` and takes precedence over the file's own `#let`: the
		// else-branch is taken instead, proving the config-first resolution the real books rely on.
		let (blocks, _skips) = res!(assemble(root, dir, &dir.join("root.typ"), &lang::rules::TemplateFns::new(), "#let media = \"print\"\n"));
		let body = fmt!("{:?}", blocks);
		assert!(body.contains("Print only paragraph"), "config `media` selects the else-branch: {}", body);
		assert!(!body.contains("Ebook only paragraph"), "the ebook branch is dropped under the config: {}", body);

		// An unsupported guard form is refused and reported, not followed nor leaked.
		let odd = "#if media > 3 [\nSomething.\n]\n";
		let (blocks, skips) = res!(assemble(odd, dir, &dir.join("root.typ"), &lang::rules::TemplateFns::new(), ""));
		let body = fmt!("{:?}", blocks);
		assert!(!body.contains("Something"), "a refused guard follows neither branch: {}", body);
		assert!(skips.report().map(|r| r.contains("#if")).unwrap_or(false),
			"a refused guard form must be reported: {:?}", skips.report());
		Ok(())
	}

	/// A lone `]` line deeper inside a taken branch -- here an `#emph[...]` aside's own closer -- must not
	/// be mistaken for the guard's own closing bracket (G1). Before the bracket-depth extent, ANY bare `]`
	/// line closed the guard: the aside's closer ended it early, so the rest of the taken branch, the
	/// guard's own markers and the untaken branch all leaked into the body as prose from that point on.
	#[test]
	fn if_guard_bracket_extent_survives_an_inner_content_closer() -> Outcome<()> {
		let dir = std::path::Path::new("/nonexistent");
		let root = "#let media = \"ebook\"\n\n#if media == \"ebook\" [\nEbook lead-in with an aside: #emph[\nspanning more than one line\n]\nand the branch continues here.\n] else [\nPrint branch text.\n]\n\nTail paragraph.\n";
		let (blocks, skips) = res!(assemble(root, dir, &dir.join("root.typ"), &lang::rules::TemplateFns::new(), ""));
		let body = fmt!("{:?}", blocks);
		assert!(body.contains("Ebook lead-in") && body.contains("spanning more than one line")
			&& body.contains("and the branch continues here"),
			"the taken branch's own content, before AND after the inner closer, must all survive: {}", body);
		assert!(body.contains("Tail paragraph"), "prose after the guard must still survive: {}", body);
		assert!(!body.contains("Print branch text"), "the untaken branch must stay dropped: {}", body);
		assert!(!body.contains("] else [") && !body.contains("#if "),
			"no guard marker line may leak as prose: {}", body);
		// The reader's own `#let` skip is expected and unrelated; the guard itself must report nothing.
		assert!(!skips.report().map(|r| r.contains("#if")).unwrap_or(false),
			"a correctly bracket-tracked guard reports no #if refusal of its own: {:?}", skips.report());
		Ok(())
	}

	/// A brace-bodied `#if <cond> { ... } else { ... }` guard is a form the assembler does not evaluate,
	/// but its refusal must consume the whole block -- body, `} else {` divider and closing `}` -- rather
	/// than only the opener line (G2). Before the fix, only `#if ... {` itself was refused; everything
	/// after it fell through as ordinary prose.
	#[test]
	fn if_brace_bodied_form_is_refused_as_one_block_not_leaked() -> Outcome<()> {
		let dir = std::path::Path::new("/nonexistent");
		let root = "Intro.\n\n#if media == \"ebook\" {\n  let x = 1\n} else {\n  let x = 2\n}\n\nTail.\n";
		let (blocks, skips) = res!(assemble(root, dir, &dir.join("root.typ"), &lang::rules::TemplateFns::new(), ""));
		let body = fmt!("{:?}", blocks);
		assert!(body.contains("Intro") && body.contains("Tail"), "prose around the guard must survive: {}", body);
		assert!(!body.contains("let x") && !body.contains("} else {"),
			"the brace-bodied guard's body and its else divider must not leak as prose: {}", body);
		assert_eq!(skips.total(), 1,
			"exactly one refusal for the whole brace-bodied guard, not one per leaked line: {:?}", skips.sites());
		Ok(())
	}

	#[test]
	fn test_a_part_page_divider_lifts_to_a_heading_03() -> Outcome<()> {
		let dir = std::path::Path::new("/nonexistent");
		let (blocks, _skips) = res!(assemble("#part-page(label: \"Part\")[The Pattern]\n", dir, &dir.join("root.typ"), &lang::rules::TemplateFns::new(), ""));
		assert_eq!(blocks.len(), 1, "one divider, one heading");
		match &blocks[0] {
			Block::Heading { level, segments, .. } => {
				assert_eq!(*level, 0, "a part divider is a level-0 heading, outside the chapter numbering");
				let title = segments.iter().map(|s| match s {
					Segment::Text(t)	=> t.clone(),
					_					=> String::new(),
				}).collect::<String>();
				assert_eq!(title, "The Pattern", "the title is the bracket body");
			},
			other => return Err(err!("expected a heading, found {:?}", other; Test, Bug)),
		}
		Ok(())
	}

	/// The term-dict reader picks the `#let term-dict = (...)` assignment, not the `// term-dict: ...`
	/// comment above it whose own parentheses would otherwise be read as the literal.
	#[test]
	fn test_term_dict_reader_skips_the_comment_04() {
		let src = r#"
// term-dict:  key -> display value (plain strings)
#let term-dict = (
  "org": "Elearnity Pty Ltd",
  "website": "elearnity.oxegen.io",
  "iniverse": "iniverse",
)
"#;
		let map = parse_term_dict(src);
		assert_eq!(map.get("org").map(String::as_str), Some("Elearnity Pty Ltd"));
		assert_eq!(map.get("website").map(String::as_str), Some("elearnity.oxegen.io"));
		assert_eq!(map.get("iniverse").map(String::as_str), Some("iniverse"));
		assert_eq!(map.len(), 3, "unexpected entries: {:?}", map);
	}

	/// A lone chapter finds a `refs.bib` in an ancestor directory, marks the key it cited, and returns a
	/// bibliography that resolves that key to an author-year citation rather than the raw key.
	#[test]
	fn test_lone_chapter_resolves_its_citation_05() -> Outcome<()> {
		// A unique scratch tree: refs.bib at the top, the chapter one level down, so the walk-up finds it.
		let base = std::env::temp_dir().join(fmt!("austenite-bibtest-{}",
			std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
				.map(|d| d.as_nanos()).unwrap_or(0)));
		let sub = base.join("chapters");
		res!(std::fs::create_dir_all(&sub));
		res!(std::fs::write(base.join("refs.bib"),
			"@article{smith2020, author = {Smith, John}, title = {A Title}, year = {2020}, journal = {J}}\n"));
		let chapter = sub.join("chap.typ");
		res!(std::fs::write(&chapter, "cited here"));

		let mut blocks = vec![Block::RichParagraph { segments: vec![Segment::Cite(vec!["smith2020".to_string()])] }];
		let bib = res!(load_lone_bibliography(&chapter, &mut blocks));

		// Clean up before asserting, so a failed assertion still leaves no scratch behind.
		let _ = std::fs::remove_dir_all(&base);

		let bib = res!(bib.ok_or_else(|| err!("no bibliography was found beside the chapter"; Test, Missing)));
		let cite = res!(bib.format_citation(&["smith2020"]));
		assert!(cite.contains("Smith") && cite.contains("2020"),
			"citation did not resolve to author-year: {:?}", cite);
		assert!(!cite.contains("smith2020"), "the raw cite key leaked: {:?}", cite);
		Ok(())
	}

	/// An included chapter's own `#set text(size: ...)` is lowered and scoped to that chapter's subtree
	/// (H1): `assemble` nests the chapter's blocks inside one `Block::Scoped` carrying the lowered patch,
	/// and a sibling chapter that declares nothing is left unwrapped. Before this, a chapter's `#set` was
	/// captured by the reader but never lowered, since lowering ran only over the root.
	#[test]
	fn test_included_chapter_set_is_scoped_to_its_subtree_h1() -> Outcome<()> {
		let base = std::env::temp_dir().join(fmt!("austenite-h1-{}",
			std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
				.map(|d| d.as_nanos()).unwrap_or(0)));
		res!(std::fs::create_dir_all(&base));
		// Chapter A declares a body size; chapter B declares nothing.
		res!(std::fs::write(base.join("chap_a.typ"), "#set text(size: 20pt)\n= Chapter A\nAlpha body.\n"));
		res!(std::fs::write(base.join("chap_b.typ"), "= Chapter B\nBeta body.\n"));
		let root_src	= "#include \"chap_a.typ\"\n#include \"chap_b.typ\"\n";
		let root_path	= base.join("root.typ");

		let (blocks, _skips) = res!(assemble(root_src, &base, &root_path, &lang::rules::TemplateFns::new(), ""));

		// Clean up before asserting, so a failed assertion leaves no scratch behind.
		let _ = std::fs::remove_dir_all(&base);

		// Exactly one scope, carrying chapter A's lowered body size, its nested blocks opening with that
		// chapter's heading -- the `#set` line emitted no block of its own (it lowered into the patch).
		let scopes: Vec<(&ThemePatch, &Vec<Block>)> = blocks.iter().filter_map(|b| match b {
			Block::Scoped { patch, blocks }	=> Some((patch, blocks)),
			_							=> None,
		}).collect();
		assert_eq!(scopes.len(), 1, "expected exactly one scope, got {}", scopes.len());
		let (patch, inner) = scopes[0];
		assert_eq!(patch.text.body_size, Some(Sp::from_pt(20.0)),
			"the chapter's #set text(size:) did not lower into the scope patch");
		assert!(matches!(inner.first(), Some(Block::Heading { .. })),
			"the scope's first nested block should be chapter A's heading");

		// Chapter B declares nothing, so it is a flat sibling of the scope: exactly one heading sits at the
		// top level (chapter B's), chapter A's being nested inside the scope rather than a flat sibling.
		let top_headings = blocks.iter().filter(|b| matches!(b, Block::Heading { .. })).count();
		assert_eq!(top_headings, 1,
			"chapter B's heading must be the one flat-sibling heading; A's is nested in the scope");
		Ok(())
	}

	/// The `term-defs` reader lifts each key's content group, not the leading comment's, keeping the
	/// definition's inner markup source and pairing it with its key in source order.
	#[test]
	fn test_term_defs_reader_reads_content_groups_10() {
		let src = r#"
// term-defs: key -> definition (content)
#let term-defs = (
  "org": [The Oxegence Foundation, a non-profit.],
  "ai": [Artificial Intelligence.],
)
"#;
		let defs = parse_term_defs(src);
		assert_eq!(defs.len(), 2, "unexpected entries: {:?}", defs);
		assert_eq!(defs[0].0, "org");
		assert_eq!(defs[0].1, "The Oxegence Foundation, a non-profit.");
		assert_eq!(defs[1].0, "ai");
		assert_eq!(defs[1].1, "Artificial Intelligence.");
	}

	/// `#print-glossary()` collects the document's glossary terms in first-appearance order, deduplicated
	/// by key, dropping a term with no definition, and fills the placeholder with a Term/Definition table
	/// whose Term column is the term-dictionary value where the key has one and the key itself otherwise.
	#[test]
	fn test_resolve_glossary_orders_dedupes_and_skips_undefined_11() -> Outcome<()> {
		// The term-dictionary gives the `g`-family its display value; `meet` has none, so its Term column is
		// the key itself, as the `gs`-family metadata stores.
		res!(crate::lang::parse::set_term_dict(HashMap::from([
			("org".to_string(), "Oxegence Foundation".to_string()),
		])));
		{
			let mut guard = lock_write!(TERM_DEFS, "test term-defs");
			let mut m: HashMap<String, Vec<Segment>> = HashMap::new();
			m.insert("org".to_string(),  vec![Segment::text("The Foundation.")]);
			m.insert("meet".to_string(), vec![Segment::text("To oxedize.")]);
			*guard = Some(m);
		}
		let mut blocks = vec![
			Block::rich(vec![
				Segment::glossary("meet", "oxedize"),
				Segment::text(" then "),
				Segment::glossary("org", "Oxegence Foundation"),
			]),
			Block::rich(vec![
				Segment::glossary("meet", "oxedize"),		// a second use adds no row
				Segment::glossary("surplus", "surplus"),	// no definition, so no row
			]),
			Block::Glossary,
		];
		resolve_glossary(&mut blocks, false);

		let table = match &blocks[2] {
			Block::Table(t)	=> t,
			other			=> return Err(err!("expected a glossary table, found {:?}", other; Test, Bug)),
		};
		assert!(table.header, "the glossary sets a header row");
		assert_eq!(table.weights, vec![1.0, 3.0], "columns are 1fr / 3fr");
		assert_eq!(table.rows.len(), 3, "header plus the two defined terms");

		// The Term column of a body row, flattened to its text.
		let term_of = |r: usize| -> String {
			table.rows[r].cells[0].content.iter().map(|s| match s {
				Segment::Text(t)	=> t.clone(),
				_					=> String::new(),
			}).collect()
		};
		assert_eq!(term_of(1), "meet", "first appearance, a key with no dict value shows the key itself");
		assert_eq!(term_of(2), "Oxegence Foundation", "second appearance, a key with a dict value shows it");
		Ok(())
	}

	/// A `#print-glossary()` placeholder inside a scoped subtree -- an included chapter's own call -- is found
	/// and filled in place, not dropped: the resolver descends into the scope and swaps the nested placeholder
	/// for the glossary table. Before it recursed, only a top-level placeholder was ever seen, so a scoped
	/// call silently vanished. Needs no term definitions: with none, a header-only table is built and set.
	#[test]
	fn test_resolve_glossary_fills_a_placeholder_inside_a_scope_12() -> Outcome<()> {
		let mut blocks = vec![
			Block::paragraph("Body."),
			Block::Scoped {
				patch:	ThemePatch::default(),
				blocks:	vec![Block::Glossary],
			},
		];
		resolve_glossary(&mut blocks, false);
		let inner = match &blocks[1] {
			Block::Scoped { blocks, .. }	=> blocks,
			other							=> return Err(err!("the scope must survive resolution, found {:?}", other; Test, Bug)),
		};
		match &inner[0] {
			Block::Table(_)	=> Ok(()),
			other			=> Err(err!("a #print-glossary inside a scope must be filled, found {:?}", other; Test, Bug)),
		}
	}

	/// `face_resolver` (the lone-file path's resolver builder) must resolve a face named only inside the
	/// file's own block tree -- by a rule or a `#styled-box`'s scope -- not only one the root theme names
	/// itself. Before this, it loaded from [`heading_face_names`] alone, so a rule-named face never resolved
	/// on the lone-file path even though the whole-book path (via [`all_face_names`]) already handled it.
	#[test]
	fn test_face_resolver_reads_a_block_scoped_face_name_13() -> Outcome<()> {
		let base = std::env::temp_dir().join(fmt!("austenite-facer-{}",
			std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
				.map(|d| d.as_nanos()).unwrap_or(0)));
		let fonts_dir = base.join("assets").join("fonts");
		res!(std::fs::create_dir_all(&fonts_dir));
		let src_font = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("fonts").join("LibertinusSerif-Regular.otf");
		res!(std::fs::copy(&src_font, fonts_dir.join("LibertinusSerif-Regular.otf")));

		// The root theme names no display face at all; only a scoped patch -- as a rule or a `#styled-box`
		// would build -- names one, so `heading_face_names(&theme)` alone would find nothing.
		let theme = Theme::default();
		assert!(heading_face_names(&theme).is_empty(), "the root theme must name no face for this to test the block path");
		let blocks = vec![
			Block::Scoped {
				patch: crate::theme::ThemePatch {
					heading: crate::theme::ThemeHeadingPatch {
						face: Some(Some("LibertinusSerif".to_string())),
						..Default::default()
					},
					..Default::default()
				},
				blocks: vec![],
			},
		];
		// `root_dir`'s parent is `base`, matching a lone chapter's own directory beside `base/assets/fonts`.
		let root_dir = base.join("chapter");
		let faces = face_resolver(&root_dir, &theme, &blocks);

		let _ = std::fs::remove_dir_all(&base);
		assert!(faces.resolves("LibertinusSerif"),
			"a face named only by a scoped patch in the block tree must still resolve on the lone-file path");
		Ok(())
	}

	/// `note_missing_face_variants` must also flag a scoped or box subtree's own heading levels, folding its
	/// patch onto the theme in force at that point -- not only the document root's levels. A rule or a
	/// `#styled-box` that sets a bold heading in a face the tree ships only Regular for must be noted, the
	/// same as a root-level heading would be.
	#[test]
	fn test_note_missing_face_variants_descends_scoped_patches_14() -> Outcome<()> {
		let base = std::env::temp_dir().join(fmt!("austenite-missvar-{}",
			std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
				.map(|d| d.as_nanos()).unwrap_or(0)));
		res!(std::fs::create_dir_all(&base));
		// Only a Regular file for "TestFace": `has_variant` for bold must come back false.
		let src_font = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("fonts").join("LibertinusSerif-Regular.otf");
		res!(std::fs::copy(&src_font, base.join("TestFace-Regular.otf")));
		let faces = FaceResolver::load(&base, &["TestFace".to_string()]);
		let _ = std::fs::remove_dir_all(&base);
		assert!(faces.resolves("TestFace"), "the Regular file must load");
		assert!(!faces.has_variant("TestFace", true, false), "no Bold file was shipped, so bold must not be an exact variant");

		// The root theme names no face at all, so the root-level pass records nothing; only the scoped
		// patch's level-1 override names "TestFace" in bold.
		let theme = Theme::default();
		let blocks = vec![
			Block::Scoped {
				patch: crate::theme::ThemePatch {
					heading: crate::theme::ThemeHeadingPatch {
						levels: vec![
							crate::theme::ThemeHeadingLevelPatch {
								face:	Some(Some("TestFace".to_string())),
								weight:	Some(Some(700)),
								..Default::default()
							},
						],
						..Default::default()
					},
					..Default::default()
				},
				blocks: vec![],
			},
		];
		let mut skips = lang::Refusals::default();
		note_missing_face_variants(&theme, &blocks, &faces, &mut skips);
		assert_eq!(skips.total(), 1, "the scoped bold heading in a Regular-only face must be noted exactly once");
		assert!(skips.sites()[0].name.contains("TestFace") && skips.sites()[0].name.contains("bold"),
			"the note must name the face and the missing slant, found {:?}", skips.sites()[0].name);
		Ok(())
	}

	/// A citation that sits only inside a table cell is still marked cited, so it appears in the Chicago
	/// reference list -- the fix for the silent loss where 15 cell-only bibliography entries vanished from the
	/// whole document. Reverting `collect_cite_keys` to skip `Block::Table` reds this: the reference list
	/// comes back empty because the cell's key is never marked.
	#[test]
	fn cite_in_a_table_cell_is_marked_for_the_bibliography() -> Outcome<()> {
		const MINI_BIB: &str = r#"
@book{scott1976moral,
  author    = {Scott, James C.},
  title     = {The Moral Economy of the Peasant},
  year      = {1976},
  publisher = {Yale University Press}
}
"#;
		let bib = res!(Bibliography::parse(MINI_BIB));
		// A document whose only citation is inside a table cell.
		let cell = Cell::rich(vec![
			Segment::text("Author "),
			Segment::cite(vec!["scott1976moral".to_string()]),
		], Align::Left);
		let mut blocks = vec![Block::Table(Table::new(false, vec![Row::new(vec![cell])]))];

		let marked = append_bibliography(bib, &mut blocks);
		let refs = marked.reference_list();
		if refs.len() != 1 {
			return Err(err!("A #cite inside a table cell was not marked for the bibliography: the reference \
				list holds {} entries, expected 1.", refs.len(); Test, Mismatch));
		}
		Ok(())
	}
}

