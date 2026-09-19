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

use std::path::Path;
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
		let assembled = Assembled {
			blocks:	spec.blocks,
			fonts:	spec.fonts,
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
	// Collect the file's own `#let` furniture (an `#aside-box`/`#pr-note` defined in the lone file) and its
	// `#let colours` palette, so a lone chapter honours its own furniture exactly as the book assembler does
	// for a whole book -- a call to one expands into its padded box (or floating figure) rather than being
	// dropped as an unknown construct.
	let mut palette	= lang::rules::Palette::new();
	let mut tfns	= lang::rules::TemplateFns::new();
	lang::rules::collect_palette(&src, &mut palette);
	lang::rules::collect_template_fns(&src, style.text.body_size, &palette, &mut tfns);
	let (mut blocks, mut skips)	= res!(lang::to_blocks_with_templates(&src, &tfns));
	skips.tag_file(&main_path.display().to_string());
	let skip_line	= terse_skip_line(&skips);
	let mut refusals	= skips;
	// Fill a `#print-glossary()` the lone chapter carries, as a whole-doc compile does after assembly.
	book::resolve_glossary(&mut blocks);
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
	let faces = match main_path.parent() {
		Some(dir)	=> book::face_resolver(dir, &style, &blocks),
		None		=> FaceResolver::default(),
	};
	book::note_missing_face_variants(&style, &blocks, &faces, &mut refusals);
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
	let (document, heads) = res!(doc::author(
		a.fonts.clone(), a.geom, &a.style, &a.faces, &a.blocks, a.front.as_ref(), a.bib.as_ref()));
	let metrics		= FontMetrics::new(a.fonts.clone(), Role::Body, Dir::Ltr, a.style.text.body_size);
	let mut out		= res!(driver::run(&document, &metrics, Config::default()));
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
