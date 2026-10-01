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
	Answer,
	Answered,
	Asked,
	Block,
	DocInfo,
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
use std::io;
use std::path::{
	Path,
	PathBuf,
};
use std::sync::Arc;

/// The pieces a compile needs after assembly, from either the whole-book path or the lone-file path.
pub struct Assembled {
	pub blocks:		Vec<Block>,
	pub fonts:		Arc<FontSet>,
	pub geom:		PageGeometry,
	pub style:		Theme,
	pub title:		String,
	pub faces:		FaceResolver,
	pub front:		Option<FrontMatter>,
	pub bib:		Option<Bibliography>,
	pub doc_info:	DocInfo,	// the Info dictionary, from each file's own `#set document(...)` at its top level or in a bare content block
	pub refusals:	lang::Refusals,	// every site not set as written, each already carrying its file
}

/// The resolved output of a compile: the decorated, mirror-shifted pages with their ledger and pass count,
/// the heading table (for an outline or a section-rail query), the geometry and the Info dictionary the
/// emit stage reads, and every site of the compile not set as written.
pub struct Rendered {
	pub out:		CompileOutput,
	pub heads:		Vec<Heading>,
	pub geom:		PageGeometry,
	pub doc_info:	DocInfo,
	pub refusals:	lang::Refusals,
}

/// Assembles the source at `main_path`: a book or doc root through [`book::load`], a lone file through the
/// reader with the lone-file styling, furniture, glossary and bibliography steps. The result carries the
/// document pieces and the refusal table, every site assembly did not set as written, each recorded with
/// the file it stands in; authoring adds its own sites to the same table ([`author_and_run`]).
///
/// `lone_fonts` supplies the reading set for the lone-file path only -- the embedded Libertinus -- and is
/// not called on the book path, which carries its own fonts. It is a thunk so the native binary builds the
/// set only when it is a lone file, while the wasm surface hands back its once-built cached instance.
pub fn assemble<F>(main_path: &Path, lone_fonts: F) -> Outcome<Assembled>
where
	F: FnOnce() -> Outcome<Arc<FontSet>>,
{
	let src = match vfs::read_to_string(main_path) {
		Ok(s)	=> s,
		Err(e) if vfs::is_not_utf8(&e)	=> return Err(err!(e,
			"The source file {:?} is not valid UTF-8 text.", main_path; File, Decode, UTF8)),
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
			// Folded during assembly, from the root and every file it includes, where each stands.
			doc_info:	spec.doc_info,
			refusals:	spec.skips,
		};
		return Ok(assembled);
	}

	// A lone chapter installs the shared `term-dict` from a `terms.typ` beside or above it, so its
	// `#t`/`#g` term calls resolve to their values just as in a whole-book compile.
	let main_file		= main_path.display().to_string();
	let mut refusals	= lang::Refusals::default();
	if let Some(dir) = main_path.parent() {
		res!(book::install_terms(dir, &mut refusals));
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
	// A lone file's conditionals resolve in its own bindings and imports, where each stands.
	let guards	= lang::rules::GuardScope::of_file(&src, main_path.parent(), 0);
	let binds	= scope.bindings().with_guards(&guards, 0);
	let (mut blocks, parsed)	= res!(lang::to_blocks_in(&src, binds, &main_file, 0));
	refusals.merge(parsed);
	// The Info dictionary the file's own `#set document` rules build, a rule in a container refused at its site.
	let doc_info = lang::set::document_info(&src, &main_file, &mut refusals);
	// Fill a `#print-glossary()` the lone chapter carries, as a whole-doc compile does after assembly.
	book::resolve_glossary(&mut blocks, false);
	// Resolve citations against a `refs.bib` found beside or above the chapter, so a lone-file compile sets
	// Chicago author-year in text and a reference list at the end rather than the raw cite key.
	let bib		= res!(book::load_lone_bibliography(main_path, &mut blocks, &mut refusals));
	let fonts	= res!(lone_fonts());
	// The styling rule engine runs over the lone chapter's block tree here, at the blocks->author seam,
	// before its faces are resolved -- so a rule-named face reaches the resolver. The default rules re-assert
	// the theme's own heading sizes (byte-neutral); the file's own `#show <selector>: <transform>` rules are
	// appended, refused where a transform reads the page or an unread field.
	let rules = lang::rules::rule_set_for(&style, &src, &main_file, &mut refusals);
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
	let sites = book::FaceSites::new(&main_file, &src, &rules, Vec::new());
	book::note_missing_face_variants(&style, &blocks, &faces, &sites, &mut refusals);
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
		doc_info,
		refusals,
	};
	Ok(assembled)
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
	let (document, heads, mut answers) = res!(doc::author_memo(
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
	let footer_logo	= a.front.as_ref().and_then(|f| f.footer_logo.as_deref().map(|p| (p, &f.sites.footer_logo)));
	res!(doc::decorate(&mut out.pages, &out.ledger, &heads, &a.fonts, &a.style, a.geom, &a.title, footer_logo, &mut answers));

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

	// Every construct that asked for something is checked against what its setter answered, now the ledger
	// can say whether a reference set as a page slot found its label.
	let mut asks = Vec::new();
	doc::asks_of(&a.blocks, &mut asks);
	if let Some(fm) = &a.front {
		fm.asks(&mut asks);
	}
	let mut refusals = a.refusals;
	record_answers(&mut refusals, asks, answers, &out.ledger);
	Ok(Rendered { out, heads, geom: a.geom, doc_info: a.doc_info, refusals })
}

/// Records a site for every construct that asked for something ([`doc::asks_of`]) and did not get it,
/// by what its setter answered ([`Answered`]): a stand-in set in its place, a construct passed over where it
/// stands, a reference set as a page slot for a label the laid-out document never placed -- and an ask no
/// setter answered at all, which a setter dropped. The default is refusal: only a setter's own answer that
/// it set the construct as written clears an ask, so a setter that passes a construct over in silence is a
/// site, never a pass. An answer for the same construct beyond its asks -- a footer logo drawn on every
/// page -- answers nothing more.
pub(crate) fn record_answers(
	refusals:	&mut lang::Refusals,
	asks:		Vec<(crate::ir::Site, Asked)>,
	answers:	Vec<Answered>,
	ledger:		&Ledger,
)
{
	let mut given: HashMap<(crate::ir::Site, Asked), std::collections::VecDeque<Answer>> = HashMap::new();
	for a in answers {
		given.entry((a.site, a.what)).or_default().push_back(a.answer);
	}
	for (site, what) in asks {
		let answer	= given.get_mut(&(site.clone(), what.clone())).and_then(|q| q.pop_front());
		let name	= what.name();
		let file	= site.file.to_string();
		match answer {
			Some(Answer::Set) => {},
			Some(Answer::Slot(label)) => {
				if ledger.page_of(&AnchorId::new(AnchorKind::Label, label)).is_none() {
					refusals.record_stand_in_in(&file, &name, site.span, lang::RefusalClass::Unusable,
						"names no label in the document, so an empty space is set in its place");
				}
			},
			Some(Answer::StandIn(class, note))	=> refusals.record_stand_in_in(&file, &name, site.span, class, &note),
			Some(Answer::Passed(passed))		=> refusals.record_in(&file, &passed, site.span),
			None => refusals.record_stand_in_in(&file, &name, site.span, lang::RefusalClass::Unsupported,
				"is not set where it stands"),
		}
	}
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

/// Builds the PDF for resolved pages in one sequential, in-memory pass: the document outline from the
/// heading table, then each page rendered and folded in, its frame freed as soon as it is written. The
/// browser has no threads, so this is the wasm surface's emit; the native binary chunks the same calls in
/// parallel.
pub fn emit_pdf(out: &mut CompileOutput, heads: &[Heading], doc_info: &DocInfo) -> Outcome<Vec<u8>> {
	let mut buf: Vec<u8> = Vec::new();
	let outline = build_outline(heads, &out.ledger);
	let mut pdf = res!(crate::emit::pdf::open_document_with_outline(&mut buf, out.pages.len(), outline, doc_info));
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

/// Why a diagnostic was raised, fixed where its refusal or error is raised and never read back from the
/// message's wording, which drifts. [`DiagnosticKind::as_str`] is the word a caller switches on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiagnosticKind {
	MissingFile,		// a source, chapter, image or other file could not be found or read
	Encoding,			// a source or other text file that is there but is not valid UTF-8 text
	MissingFont,		// a named family that no supplied or embedded font declares
	Syntax,				// source that does not parse
	Type,				// a value of the wrong type
	UnknownVariable,	// a name with no binding in scope
	Package,			// a package import the host has not supplied
	Limit,				// a limit of the engine reached: layout that will not settle, a depth or a loop bound
	Unsupported,		// a construct passed over or refused, or one set with a stand-in for what it asked for
	Internal,			// no pages, no content, or an error raised with no more specific tag
}

impl DiagnosticKind {
	/// The word the kind is carried as. A caller switches on it, so it never changes.
	pub fn as_str(&self) -> &'static str {
		match self {
			Self::MissingFile		=> "missing_file",
			Self::Encoding			=> "encoding",
			Self::MissingFont		=> "missing_font",
			Self::Syntax			=> "syntax",
			Self::Type				=> "type",
			Self::UnknownVariable	=> "unknown_variable",
			Self::Package			=> "package",
			Self::Limit				=> "limit",
			Self::Unsupported		=> "unsupported",
			Self::Internal			=> "internal",
		}
	}

	/// Does a warning of this kind refuse a strict compile? It does where the document was not set as
	/// written: a construct passed over or set with a stand-in, or a file, its text, a package or a font
	/// family missing.
	pub fn refuses_strict(&self) -> bool {
		matches!(self, Self::Unsupported | Self::MissingFile | Self::Encoding | Self::Package | Self::MissingFont)
	}

	fn from_refusal_class(class: lang::RefusalClass) -> Self {
		match class {
			lang::RefusalClass::FixedPoint		=> Self::Unsupported,
			lang::RefusalClass::Introspective	=> Self::Unsupported,
			lang::RefusalClass::Unsupported		=> Self::Unsupported,
			lang::RefusalClass::MissingFile		=> Self::MissingFile,
			lang::RefusalClass::Unusable		=> Self::Unsupported,
			lang::RefusalClass::Encoding		=> Self::Encoding,
		}
	}

	/// A hard error's kind, from the tags it was raised with anywhere in its chain. A file that is there
	/// but not UTF-8 text is told from one that cannot be read at all, since the remedy differs.
	fn from_error_tags(e: &Error<ErrTag>) -> Self {
		let tags = e.tags();
		if tags.contains(&ErrTag::Font) {
			Self::MissingFont
		} else if tags.contains(&ErrTag::UTF8) {
			Self::Encoding
		} else if tags.contains(&ErrTag::File) {
			Self::MissingFile
		} else if tags.contains(&ErrTag::LimitReached) {
			Self::Limit
		} else {
			Self::Internal
		}
	}
}

impl fmt::Display for DiagnosticKind {
	fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
		write!(f, "{}", self.as_str())
	}
}

/// How a diagnostic bears on the compile.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
	Error,		// refuses the compile, strict or not
	Warning,	// stands beside the PDF, unless strict mode refuses its kind
}

impl Severity {
	/// The word the severity is carried as.
	pub fn as_str(&self) -> &'static str {
		match self {
			Self::Error		=> "error",
			Self::Warning	=> "warning",
		}
	}
}

/// One problem at the source position a caller shows the user. `line` and `col` are 1-based; a hard error
/// carrying no site reports `0:0` against the main file rather than a guessed position. `hint` is extra
/// remedial detail, such as a strict refusal's summary of what was skipped.
#[derive(Clone, Debug, PartialEq)]
pub struct Diagnostic {
	pub file:		String,
	pub line:		usize,
	pub col:		usize,
	pub message:	String,
	pub severity:	Severity,
	pub kind:		DiagnosticKind,
	pub hint:		Option<String>,
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
	pub skipped:		Option<String>,	// the terse line of the constructs passed over
	pub summary:		Option<String>,	// the line of every site not set as written, stand-ins included
	pub empty:			bool,	// no content block was read
}

impl Report {
	/// The report of a compile whose every site not set as written is in `refusals`: its diagnostics, and
	/// the lines taken from the same finished table, so the three cannot disagree.
	pub fn new(pages: usize, refusals: &lang::Refusals, empty: bool) -> Self {
		Self {
			pages,
			diagnostics:	diagnostics(refusals),
			skipped:		refusals.skip_line(),
			summary:		refusals.summary(),
			empty,
		}
	}

	/// Why a strict compile must refuse this result, or `None` when it may stand. A strict caller wants no
	/// false green: an error, a warning of a kind the document was not set as written by
	/// ([`DiagnosticKind::refuses_strict`]), no pages or no content is refused, at the first such site or,
	/// failing one, at the top of the main file. Any other warning stands beside the PDF.
	pub fn strict_failure(&self, main: &Path) -> Option<Diagnostic> {
		let at_main = |message: String| Diagnostic {
			file:		main.display().to_string(),
			line:		1,
			col:		1,
			message,
			severity:	Severity::Error,
			kind:		DiagnosticKind::Internal,
			hint:		None,
		};
		let refusing: Vec<&Diagnostic> = self.diagnostics.iter()
			.filter(|d| d.severity == Severity::Error || d.kind.refuses_strict())
			.collect();
		if let Some(first) = refusing.first() {
			let line = self.summary.clone().unwrap_or_else(|| fmt!("{} site(s)", refusing.len()));
			return Some(Diagnostic {
				message:	fmt!("strict: {} site(s) were not set as written ({}); first: {}",
					refusing.len(), line, first.message),
				severity:	Severity::Error,
				hint:		Some(line),
				..(*first).clone()
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
			// A construct passed over says so; a site set with a stand-in says what went wrong and what stands in.
			message:	match &r.note {
				Some(note)	=> fmt!("{} {}", r.name, note),
				None		=> fmt!("skipped {} ({})", r.name, r.class.label()),
			},
			severity:	Severity::Warning,
			kind:		DiagnosticKind::from_refusal_class(r.class),
			hint:		None,
		});
	}
	out
}

/// A failure charged to the construct that asked for what could not be had: its file, line and column.
/// Raised as a cause in the failure's own chain, so the report reads the site back rather than
/// reconstructing it from the message.
#[derive(Debug)]
pub struct Cited {
	file:	String,
	line:	usize,
	col:	usize,
	cause:	io::Error,	// why it could not be had
}

impl Cited {
	pub fn new(file: &str, line: usize, col: usize, cause: io::Error) -> Self {
		Self { file: file.to_string(), line, col, cause }
	}
}

impl fmt::Display for Cited {
	fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
		// The words only: the site is the diagnostic's to print.
		write!(f, "{}", self.cause)
	}
}

impl std::error::Error for Cited {
	fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
		Some(&self.cause)
	}
}

impl Diagnostic {
	/// A hard compile error as a caller shows it: at the site it was raised with ([`Cited`]), else at
	/// `0:0` against `main`. The message is the error's plain words, free of frames and colour.
	pub fn from_error(e: &Error<ErrTag>, main: &Path) -> Self {
		let (file, line, col) = match e.find_cause::<Cited>() {
			Some(c)	=> (c.file.clone(), c.line, c.col),
			None	=> (main.display().to_string(), 0, 0),
		};
		Self {
			file,
			line,
			col,
			message:	e.plain(),
			severity:	Severity::Error,
			kind:		DiagnosticKind::from_error_tags(e),
			hint:		None,
		}
	}
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

/// The font families a compile of the project rooted at `main` can set: the embedded families, then every
/// family a face under the project's font directory declares, where the wasm surface routes its injected
/// fonts ([`book::project_font_path`]). Read by the resolver's own scan, so a family is listed exactly when
/// a compile resolves it, and sorted, each once. The source map must be installed as a compile installs it.
pub fn font_families(main: &Path) -> Vec<String> {
	let root_dir = main.parent().unwrap_or_else(|| Path::new("."));
	fonts::available_families(&book::lone_font_dir(root_dir))
}

/// The crate version, as released.
pub fn engine_version() -> &'static str { env!("CARGO_PKG_VERSION") }

/// The git commit the engine was built from (12 hex digits, with `-dirty` when the crate's tree had
/// uncommitted changes), or `unknown` when the build had no git to ask. Captured by `build.rs`.
pub fn engine_git_hash() -> &'static str { env!("AUSTENITE_GIT_HASH") }

#[cfg(test)]
mod tests {
	use super::*;
	use crate::ir::Span;

	/// Each ask is judged by what its setter answered at its site: a stand-in and a construct passed over are
	/// sites at the construct that asked, a slot is one only when the ledger never placed its label, an ask
	/// no setter answered is one, and an ask answered as set, or a second answer for one ask, is none. Two
	/// asks alike at one site take one answer each, in order.
	#[test]
	fn every_ask_is_charged_by_its_own_answer() {
		let file: std::sync::Arc<str> = std::sync::Arc::from("/p/a.typ");
		let site = |at: u32| crate::ir::Site::new(&file, Span::new(at, at));
		let image = |path: &str| Asked::Image { path: path.to_string(), role: doc::ImageRole::Figure };
		let asks = vec![
			(site(10), Asked::Figure { kind: "diagram" }),
			(site(20), image("x.png")),
			(site(30), image("fine.png")),
			(site(40), Asked::Ref("placed".to_string())),
			(site(40), Asked::Ref("gone".to_string())),
			(site(50), Asked::Footnote),
			(site(60), Asked::ClaimRef),
			(site(70), Asked::Math),
			(site(70), Asked::Math),
		];
		let answered = |at: u32, what: Asked, answer: Answer| Answered { site: site(at), what, answer };
		let answers = vec![
			answered(10, Asked::Figure { kind: "diagram" }, Answer::StandIn(lang::RefusalClass::Unusable, "will not build".to_string())),
			answered(20, image("x.png"), Answer::StandIn(lang::RefusalClass::MissingFile, "is not in the project".to_string())),
			answered(30, image("fine.png"), Answer::Set),
			answered(30, image("fine.png"), Answer::StandIn(lang::RefusalClass::Unusable, "a second answer".to_string())),
			answered(40, Asked::Ref("placed".to_string()), Answer::Slot("placed".to_string())),
			answered(40, Asked::Ref("gone".to_string()), Answer::Slot("gone".to_string())),
			answered(60, Asked::ClaimRef, Answer::Passed("claim reference in a heading title is not indexed".to_string())),
			answered(70, Asked::Math, Answer::Set),
		];
		let mut ledger = Ledger::new();
		ledger.record(crate::ledger::Anchor::new(AnchorId::new(AnchorKind::Label, "placed"),
			crate::ledger::Position::new(1, crate::ir::Sp::ZERO, crate::ir::Sp::ZERO)));
		let mut refusals = lang::Refusals::default();
		record_answers(&mut refusals, asks, answers, &ledger);
		let got: Vec<(&str, &str, u32, lang::RefusalClass)> = refusals.sites().iter()
			.map(|r| (r.file.as_str(), r.name.as_str(), r.span.start, r.class))
			.collect();
		assert_eq!(got, [
			("/p/a.typ", "#figure (diagram)", 10, lang::RefusalClass::Unusable),
			("/p/a.typ", "image \"x.png\"", 20, lang::RefusalClass::MissingFile),
			("/p/a.typ", "@gone", 40, lang::RefusalClass::Unusable),
			("/p/a.typ", "#footnote", 50, lang::RefusalClass::Unsupported),
			("/p/a.typ", "claim reference in a heading title is not indexed", 60, lang::RefusalClass::Unsupported),
			("/p/a.typ", "inline maths", 70, lang::RefusalClass::Unsupported),
		]);
	}
}
