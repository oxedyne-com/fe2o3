//! The one compile pipeline, from a source root to resolved pages, shared by both entry points.
//!
//! The native `austenite` binary and the wasm `DaimondTypst` surface each
//! wrap this module: the binary drives it with a parallel, filesystem-writing emit; the wasm surface with a
//! sequential, in-memory one. Everything between the source and the decorated pages -- the book-vs-lone
//! dispatch, the term-dictionary install, authoring, the two-pass driver, decoration and the verso
//! mirror-shift -- lives here once, so the two callers cannot drift. It was split out because they had
//! drifted: [`build_outline`] was a verbatim copy in each, and the lone-file assembly had already diverged
//! (the binary lowered its root declarations and installed its `#let` furniture before reading blocks; the
//! wasm copy did neither).
//!
//! File reads route through [`crate::vfs`], so the same code reads the real filesystem on a native build and
//! the injected source map under wasm, with no `#cfg` at the call sites.

use crate::bib::Bibliography;
use crate::book;
use crate::doc::{
	self,
	Block,
	FrontMatter,
	Heading,
};
use crate::driver::{
	self,
	CompileOutput,
	Config,
};
use crate::font::FontMetrics;
use crate::fonts;
use crate::fonts::FaceResolver;
use crate::ledger::{
	AnchorId,
	AnchorKind,
	Ledger,
};
use crate::lang;
use crate::page::PageGeometry;
use crate::theme::Theme;
use crate::vfs;

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_font::{
	face::Role,
	set::FontSet,
	shape::Dir,
};
use oxedyne_fe2o3_graphics::pdf::OutlineItem;

use std::collections::HashMap;
use std::fmt;
use std::path::{
	Path,
	PathBuf,
};
use std::sync::Arc;

/// The pieces a compile needs after assembly, from either the whole-book path or the lone-file path.
pub struct Assembled {
	pub blocks:	Vec<Block>,
	pub fonts:	Arc<FontSet>,
	pub geom:	PageGeometry,
	pub style:	Theme,
	pub title:	String,
	pub faces:	FaceResolver,
	pub front:	Option<FrontMatter>,
	pub bib:	Option<Bibliography>,
}

/// The resolved output of a compile: the decorated, mirror-shifted pages with their ledger and pass count,
/// the heading table (for an outline or a section-rail query) and the geometry the emit stage reads.
pub struct Rendered {
	pub out:	CompileOutput,
	pub heads:	Vec<Heading>,
	pub geom:	PageGeometry,
}

/// Assembles the source at `main_path`: a book or doc root through [`book::load`], a lone file through the
/// reader with the lone-file styling, furniture, glossary and bibliography steps. Returns the document
/// pieces, the full refusal table (every refused site, for the binary's `--explain`) and the terse skip
/// line (`skipped: #show ×2, ...`, or `None` when the reader set everything it met).
///
/// `lone_fonts` supplies the reading set for the lone-file path only -- the embedded Libertinus -- and is
/// not called on the book path, which carries its own fonts. It is a thunk so the native binary builds the
/// set only when it is a lone file, while the wasm surface hands back its once-built cached instance.
///
/// The skip line and the refusal table are not the same object on the lone path: the line is snapshotted
/// from the block-reading refusals alone, before the styling rules and the missing-face check append their
/// own sites, so a lone compile's terse line matches what it always printed while `--explain` still walks
/// every site. On the book path the two coincide, both taken after the whole assembly.
pub fn assemble<F>(main_path: &Path, lone_fonts: F) -> Outcome<(Assembled, lang::Refusals, Option<String>)>
where
	F: FnOnce() -> Outcome<Arc<FontSet>>,
{
	let src = match vfs::read_to_string(main_path) {
		Ok(s)	=> s,
		Err(e)	=> return Err(err!(e,
			"Could not read the source file {:?}.", main_path; File, Read)),
	};

	// A figure's `/assets/...` image path is root-relative in Typst, not filesystem-absolute; the image
	// loader resolves it against this directory and, failing that, its ancestors, so a chapter compiled on
	// its own finds the shared assets through the book's `assets` entry just as a whole book does.
	if let Some(dir) = main_path.parent() {
		res!(crate::image::set_base_dir(dir.to_path_buf()));
	}

	if book::is_book_root(&src) {
		// A book or doc root assembles its chapters through the reader and merges each chapter's refusal
		// table into one, so a whole-book or whole-doc compile reports its skipped constructs on the same
		// terse line the lone-file path prints, and `--explain` walks every chapter's sites.
		let spec		= res!(book::load(main_path));
		let skip_line	= terse_skip_line(&spec.skips);
		// A root `#set text(font: ...)` sets the whole document in that family's reading set, in place of the
		// idiom's own; a document naming no family keeps its set untouched, byte for byte.
		let fonts = spec.faces.body_set(&spec.style.text.faces.body).unwrap_or(spec.fonts);
		let assembled = Assembled {
			blocks:	spec.blocks,
			fonts,
			geom:	spec.geom,
			style:	spec.style,
			title:	spec.title,
			faces:	spec.faces,
			front:	Some(spec.front),
			bib:	spec.bib,
		};
		return Ok((assembled, spec.skips, skip_line));
	}

	// A lone chapter installs the shared `term-dict` from a `terms.typ` beside or above it, so its
	// `#t`/`#g` term calls resolve to their values just as in a whole-book compile.
	if let Some(dir) = main_path.parent() {
		res!(book::install_term_dict(dir));
		res!(book::install_term_defs(dir));
	}
	// A lone file may carry its own `#show: doc.with(...)` or a lowerable top-level `#set`; the reader
	// captures those rather than refusing them, so their styling is lowered onto the theme here -- otherwise
	// the capture would be a silent skip. Lowered before the blocks are read so a furniture definition
	// resolves its `em` insets against the file's own body size.
	let mut style = Theme::default();
	lang::set::lower_root_declarations(&src, &mut style);
	// Collect the lone file's whole `#let` scope -- its own furniture (an `#aside-box`/`#pr-note` defined in
	// the file), its content bindings, and everything the files it `#import`s supply -- so a lone chapter
	// honours its furniture and content bindings exactly as the book assembler does for a whole book. A
	// furniture call expands into its padded box (or floating figure) and a content-binding reference into its
	// re-read markup, rather than being dropped as an unknown construct. The import walk resolves an `#import`
	// even with no `#include` present, which the lone path by definition has none of.
	let scope	= book::collect_scope(&src, main_path.parent().unwrap_or_else(|| Path::new(".")), style.text.body_size);
	let binds	= scope.bindings();
	let (mut blocks, mut skips)	= res!(lang::to_blocks_with_templates(&src, binds));
	skips.tag_file(&main_path.display().to_string());
	let skip_line	= terse_skip_line(&skips);
	let mut refusals	= skips;
	// Fill a `#print-glossary()` the lone chapter carries, as a whole-doc compile does after assembly.
	book::resolve_glossary(&mut blocks, false);
	// Resolve citations against a `refs.bib` found beside or above the chapter, so a lone-file compile sets
	// Chicago author-year in text and a reference list at the end rather than the raw cite key.
	let bib		= res!(book::load_lone_bibliography(main_path, &mut blocks));
	let fonts	= res!(lone_fonts());
	// The styling rule engine runs over the lone chapter's block tree here, at the blocks->author seam,
	// before its faces are resolved -- so a rule-named face reaches the resolver. The default rules re-assert
	// the theme's own heading sizes (byte-neutral); the file's own `#show <selector>: <transform>` rules are
	// appended, refused where a transform reads the page or an unread field.
	let rules = lang::rules::rule_set_for(&style, &src, &mut refusals);
	// A lone file sets on A4 (its geometry below), so the placement width a template resolves against is A4's.
	lang::rules::apply_rules(&mut blocks, &rules, PageGeometry::a4().content_width());
	// A lone file may name a heading font in its own `#show: doc.with(...)`, or a rule/scope inside its own
	// block tree may name one; resolve against the union of both against the tree's assets, the same way a
	// whole book or doc does, so a lone chapter's heading face reaches the page whichever source names it. A
	// heading asking for a weight/slant the tree ships no file for is noted, as for a book.
	let mut faces = match main_path.parent() {
		Some(dir)	=> book::face_resolver(dir, &style, &blocks),
		None		=> FaceResolver::default(),
	};
	// Every family the file names must be declared by a font it was given: a hard error otherwise, never a
	// silent fall-back (see `FaceResolver::require`).
	let (bodies, headings) = book::named_families(&style, &blocks);
	let font_dir = book::lone_font_dir(main_path.parent().unwrap_or_else(|| Path::new(".")));
	res!(faces.require(&font_dir, &bodies, &headings));
	book::note_missing_face_variants(&style, &blocks, &faces, &mut refusals);
	let fonts = faces.body_set(&style.text.faces.body).unwrap_or(fonts);
	let assembled = Assembled {
		blocks,
		fonts,
		geom:	PageGeometry::a4(),
		style,
		title:	String::new(),
		faces,
		front:	None,
		bib,
	};
	Ok((assembled, refusals, skip_line))
}

/// Authors the assembled blocks, runs the two-pass driver to its fixed point, decorates each page with a
/// running head and folio, and mirrors the verso margins. The result carries the resolved pages, their
/// ledger and pass count, the heading table and the geometry, ready for either caller's emit stage.
pub fn author_and_run(a: Assembled) -> Outcome<Rendered> {
	author_and_run_memo(a, None)
}

/// [`author_and_run`] with the incremental memo threaded through the authoring stage, and the body/
/// furniture split point recorded on each page for the page-emit memo. Passing `None` is exactly
/// [`author_and_run`], byte for byte. The caller reuses one [`Memo`](crate::memo::Memo) across recompiles
/// of the same document (see the native `--watch` path); the emit stage then renders each page through
/// [`crate::emit::svg::render_page_memo`] against that same memo.
pub fn author_and_run_memo(a: Assembled, memo: Option<&mut crate::memo::Memo>) -> Outcome<Rendered> {
	let (document, heads) = res!(doc::author_memo(
		a.fonts.clone(), a.geom, &a.style, &a.faces, &a.blocks, a.front.as_ref(), a.bib.as_ref(), memo));
	let metrics		= FontMetrics::new(a.fonts.clone(), Role::Body, Dir::Ltr, a.style.text.body_size);
	let mut out		= res!(driver::run(&document, &metrics, Config::default()));
	// Record where each page's body ends before decoration appends its running head and folio, so the
	// page-emit memo hashes the body alone and draws the furniture (whose folio differs page to page)
	// fresh. Recorded here, at the one point the split is known; a page never decorated leaves it at the
	// whole frame, which the non-memo emit path ignores.
	for page in &mut out.pages {
		page.set_body_len(page.frame.placed.len());
	}
	let footer_logo	= a.front.as_ref().and_then(|f| f.footer_logo.as_deref());
	res!(doc::decorate(&mut out.pages, &out.ledger, &heads, &a.fonts, &a.style, a.geom, &a.title, footer_logo));

	// Mirror the margins: the driver laid every page at the recto split (binding on the left). A verso page
	// -- an even folio -- is that whole frame shifted to the fore-edge, so the binding margin sits at the
	// spine on both sides of the leaf. Uniform margins give a zero shift, so a non-book run is untouched.
	let shift = a.geom.mirror_shift();
	if shift.raw() != 0 {
		for page in &mut out.pages {
			if page.number % 2 == 0 {
				for placed in &mut page.frame.placed {
					placed.x = placed.x + shift;
				}
			}
		}
	}

	Ok(Rendered { out, heads, geom: a.geom })
}

/// Builds the PDF document outline (the viewer's bookmark side panel) from the resolved ledger: the three
/// front-matter leaves first -- title page, meta (imprint) page and contents -- then every body heading in
/// reading order. The front-matter pages carry no heading of their own, so the block layer records a
/// `Label` anchor at the top of each (`frontmatter:title`, `frontmatter:meta`, `frontmatter:contents`);
/// this reads their page back from the ledger. A leaf the book omits sets no anchor, so its entry is simply
/// absent. Body headings resolve their page through the heading anchor, and their depth matches the contents
/// list -- a chapter or a part at the top, deeper headings nested under it. Pages are zero-based, as
/// [`OutlineItem`] wants; the ledger stores them one-based.
pub fn build_outline(heads: &[Heading], ledger: &Ledger) -> Vec<OutlineItem> {
	let mut items: Vec<OutlineItem> = Vec::new();

	// The front matter, at the top and at depth zero, so it stands as a sibling of the first body level.
	let front = [
		("frontmatter:title",		"Title"),
		("frontmatter:meta",		"Meta"),
		("frontmatter:contents",	"Contents"),
	];
	for (key, label) in front {
		let id = AnchorId::new(AnchorKind::Label, key);
		if let Some(page) = ledger.page_of(&id) {
			items.push(OutlineItem { title: label.to_string(), page: (page - 1) as usize, level: 0 });
		}
	}

	// Every body heading, its depth the contents indent: a chapter or a part at depth zero, a `==` section
	// at one, and so on. A heading the ledger has not fixed is skipped rather than guessed.
	for h in heads {
		if let Some(page) = ledger.page_of(&h.id) {
			let level = (h.level.max(1) - 1) as u8;
			items.push(OutlineItem { title: h.title.clone(), page: (page - 1) as usize, level });
		}
	}
	items
}

/// The one terse skip line -- `skipped: #show ×2, #columns ×1` -- built from the summary's per-name counts,
/// or `None` when the reader set everything it met. Ordered by the summary (descending count, then name),
/// so the line leads with the construct that cost the most.
fn terse_skip_line(skips: &lang::Refusals) -> Option<String> {
	if skips.is_empty() {
		return None;
	}
	let parts: Vec<String> = skips.entries().into_iter()
		.map(|(n, c)| fmt!("{} ×{}", n, c))
		.collect();
	Some(fmt!("skipped: {}", parts.join(", ")))
}

/// Builds the PDF for resolved pages in one sequential, in-memory pass: the document outline from the
/// heading table, then each page rendered and folded in, its frame freed as soon as it is written. The
/// browser has no threads, so this is the wasm surface's emit; the native binary chunks the same calls in
/// parallel.
pub fn emit_pdf(out: &mut CompileOutput, heads: &[Heading]) -> Outcome<Vec<u8>> {
	let mut buf: Vec<u8> = Vec::new();
	let outline = build_outline(heads, &out.ledger);
	let mut pdf = res!(crate::emit::pdf::open_document_with_outline(&mut buf, out.pages.len(), outline));
	for page in &mut out.pages {
		let built = res!(crate::emit::pdf::render_page(page));
		res!(crate::emit::pdf::write_built_page(&mut pdf, &built));
		page.frame = crate::page::Frame::new();
	}
	res!(pdf.finish());
	Ok(buf)
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ DIAGNOSTICS AND STRICT MODE                                                │
// └───────────────────────────────────────────────────────────────────────────┘

/// One problem at the source position a caller shows the user. `line` and `col` are 1-based; a hard error
/// whose cause could not be traced to a source line reports `0:0` against the main file rather than a
/// guessed position.
#[derive(Clone, Debug, PartialEq)]
pub struct Diagnostic {
	pub file:		String,
	pub line:		usize,
	pub col:		usize,
	pub message:	String,
}

impl fmt::Display for Diagnostic {
	fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
		write!(f, "{}:{}:{}: {}", self.file, self.line, self.col, self.message)
	}
}

/// What a finished compile reports beside its artefact: the page count, every refused construct at its
/// source position, the terse skip line, and whether the source set no content at all. Built while the
/// source map is still installed, since each refusal's line and column are read from its source text.
#[derive(Clone, Debug)]
pub struct Report {
	pub pages:			usize,
	pub diagnostics:	Vec<Diagnostic>,
	pub skipped:		Option<String>,
	pub empty:			bool,	// no content block was read
}

impl Report {
	pub fn new(pages: usize, refusals: &lang::Refusals, skipped: Option<String>, empty: bool) -> Self {
		Self { pages, diagnostics: diagnostics(refusals), skipped, empty }
	}

	/// Why a strict compile must refuse this result, or `None` when it may stand. A strict caller wants no
	/// false green: a PDF that silently passed over a construct, that has no pages, or that set nothing is
	/// an error, reported at the first refused site or, failing one, at the top of the main file.
	pub fn strict_failure(&self, main: &Path) -> Option<Diagnostic> {
		let at_main = |message: String| Diagnostic {
			file:		main.display().to_string(),
			line:		1,
			col:		1,
			message,
		};
		if let Some(first) = self.diagnostics.first() {
			let line = self.skipped.clone().unwrap_or_else(|| fmt!("skipped: {} site(s)", self.diagnostics.len()));
			return Some(Diagnostic {
				message: fmt!("strict: {} construct site(s) were not set ({}); first: {}",
					self.diagnostics.len(), line, first.message),
				..first.clone()
			});
		}
		if self.pages == 0 {
			return Some(at_main("strict: the compile produced no pages.".to_string()));
		}
		if self.empty {
			return Some(at_main("strict: the source sets no content.".to_string()));
		}
		None
	}
}

/// Resolves each refused site to a [`Diagnostic`], reading the tagged source file's line and column through
/// [`vfs`]. A site whose source cannot be read still reports, at `0:0`, so a refusal is never dropped.
pub fn diagnostics(refusals: &lang::Refusals) -> Vec<Diagnostic> {
	let mut out = Vec::with_capacity(refusals.total());
	let mut cache: HashMap<String, Option<String>> = HashMap::new();
	for r in refusals.sites() {
		let src = cache.entry(r.file.clone())
			.or_insert_with(|| vfs::read_to_string(&PathBuf::from(&r.file)).ok());
		let (line, col) = match src {
			Some(text)	=> { let (l, c, _) = lang::line_col_of(text, r.span.start); (l, c) },
			None		=> (0, 0),
		};
		out.push(Diagnostic {
			file:		r.file.clone(),
			line,
			col,
			message:	fmt!("skipped {} ({})", r.name, r.class.label()),
		});
	}
	out
}

/// Places a hard compile error at a source position. The engine's file errors name the path they could not
/// read (`Could not read the included chapter "/p/ch1.typ".`) but not the line that asked for it, so each
/// quoted path in the message is looked for among the string literals of `sources` -- resolved against the
/// citing file's directory, as the reader resolves them -- and the first citing literal gives the position.
/// An error naming no source-cited path reports `0:0` against `main`. The message is the error's plain
/// words, free of source-code frames and colour.
pub fn locate_error(e: &Error<ErrTag>, main: &Path, sources: &[PathBuf]) -> Diagnostic {
	let message = e.plain();
	for cited in quoted_literals(&message) {
		let target = match vfs::canonicalize(Path::new(&cited.1)) {
			Ok(p)	=> p,
			Err(_)	=> PathBuf::from(&cited.1),
		};
		for src_path in sources {
			let text = match vfs::read_to_string(src_path) {
				Ok(t)	=> t,
				Err(_)	=> continue,
			};
			let dir = src_path.parent().unwrap_or_else(|| Path::new("/"));
			for (off, lit) in quoted_literals(&text) {
				let resolved = match vfs::canonicalize(&dir.join(&lit)) {
					Ok(p)	=> p,
					Err(_)	=> dir.join(&lit),
				};
				if resolved == target {
					let (line, col, _) = lang::line_col_of(&text, off as u32);
					return Diagnostic { file: src_path.display().to_string(), line, col, message };
				}
			}
		}
	}
	Diagnostic { file: main.display().to_string(), line: 0, col: 0, message }
}

/// Every double-quoted literal in `s` with the byte offset of its opening quote. No escape handling beyond
/// a backslash-quote, which is all a path literal needs.
fn quoted_literals(s: &str) -> Vec<(usize, String)> {
	let mut out = Vec::new();
	let bytes = s.as_bytes();
	let mut i = 0;
	while i < bytes.len() {
		if bytes[i] == b'"' {
			let start = i;
			let mut j = i + 1;
			while j < bytes.len() && bytes[j] != b'"' && bytes[j] != b'\n' {
				if bytes[j] == b'\\' {
					j += 1;
				}
				j += 1;
			}
			if j < bytes.len() && bytes[j] == b'"' {
				out.push((start, s[start + 1..j].to_string()));
				i = j + 1;
				continue;
			}
		}
		i += 1;
	}
	out
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ FONT FAMILIES AND ENGINE IDENTITY                                          │
// └───────────────────────────────────────────────────────────────────────────┘

/// The families embedded in the crate, under the names a Typst source uses for them: the Libertinus
/// reading set ([`crate::fonts::libertinus`]) and the maths face ([`crate::math`]).
pub const EMBEDDED_FAMILIES: [&str; 3] = [
	"Libertinus Serif",
	"Libertinus Mono",
	"New Computer Modern Math",
];

// The weight/slant suffixes the named-face resolver loads, `<Family>-<Variant>.{ttf,otf}`.
const FACE_VARIANTS: [&str; 4] = ["Regular", "Bold", "Italic", "BoldItalic"];

/// The font families a compile of the project rooted at `main` can set: the embedded families, then each
/// injected font's family that the engine's own face resolver actually loads, sorted and deduplicated.
/// `injected` are the paths the consumer gave its fonts under; each is looked for where the wasm surface
/// routes it ([`book::project_font_path`]), so the source map must be installed as a compile installs it.
/// A file not named `<Family>-<Variant>.{ttf,otf}`, or one that will not parse, is not a family the engine
/// can resolve by name, so it is not listed.
pub fn font_families(main: &Path, injected: &[PathBuf]) -> Vec<String> {
	let mut out: Vec<String> = fonts::embedded_families();
	for given in injected {
		let routed = match book::project_font_path(main, given) {
			Some(p)	=> p,
			None	=> continue,
		};
		let family = match face_family(&routed) {
			Some(f)	=> f,
			None	=> continue,
		};
		let dir = routed.parent().unwrap_or_else(|| Path::new("/"));
		// SWITCH: the one call site to move to the font lane's family-list accessor on `FaceResolver`
		// once it lands; until then a family counts when the resolver itself loads it by that name.
		if FaceResolver::load(dir, &[family.clone()]).resolves(&family) {
			out.push(family);
		}
	}
	out.sort();
	out.dedup();
	out
}

/// The family named by a `<Family>-<Variant>.{ttf,otf}` file, or `None` for any other name.
fn face_family(path: &Path) -> Option<String> {
	let ext = path.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase());
	if !matches!(ext.as_deref(), Some("ttf") | Some("otf")) {
		return None;
	}
	let stem = match path.file_stem().and_then(|s| s.to_str()) {
		Some(s)	=> s,
		None	=> return None,
	};
	match stem.rsplit_once('-') {
		Some((family, variant)) if !family.is_empty() && FACE_VARIANTS.contains(&variant)
			=> Some(family.to_string()),
		_	=> None,
	}
}

/// The crate version, as released.
pub fn engine_version() -> &'static str { env!("CARGO_PKG_VERSION") }

/// The git commit the engine was built from (12 hex digits, with `-dirty` when the crate's tree had
/// uncommitted changes), or `unknown` when the build had no git to ask. Captured by `build.rs`.
pub fn engine_git_hash() -> &'static str { env!("AUSTENITE_GIT_HASH") }
