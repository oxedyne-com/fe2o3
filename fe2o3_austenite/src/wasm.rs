//! The wasm-bindgen compile surface, shaped to Daimond's existing Typst-wasm contract so the app can
//! swap the Typst compiler for Austenite without rewiring its callers.
//!
//! A [`DaimondTypst`] is a long-lived compiler instance: it builds the embedded Libertinus reading set
//! once and holds it across compiles, and it keeps the last compile's ledger so a section-rail query can
//! read heading pages back without recompiling. Nothing here touches a filesystem, a font service or a
//! package registry: every source, asset and font a document needs is injected as bytes and read back
//! through [`crate::vfs`], the same seam the native assembler reads the real filesystem through. This is
//! the wasm equivalent of Typst's dummy-access model -- a path not injected simply does not resolve.
//!
//! Every method resolves; none rejects or throws. A failure returns `{ error: "<file>:<line>: <message>" }`
//! (the engine's own message where a site is not localised to a line), never a bare access-denied line and
//! never a JavaScript exception, so a caller composes its diagnostics from a value it always receives.
//!
//! One capability is deliberately out of this lane and documented as a gap rather than stubbed: a recompile
//! is from scratch -- the instance is shaped to hold an incremental block cache, but this lane does not
//! build one. The per-page SVG does carry a transparent selectable-text layer -- a `.tsel` twin of the
//! glyph outlines, emitted by [`crate::emit::svg`] -- which is what [`DaimondTypst::compile_project_vector`]
//! returns and the section rail selects over.

use crate::book;
use crate::compile;
use crate::delta;
use crate::doc::Heading;
use crate::emit::{
	self,
	svg,
};
use crate::fonts;
use crate::lang;
use crate::ledger::Ledger;
use crate::memo::Memo;
use crate::page::Frame;
use crate::vfs;

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_jdat::prelude::*;
use oxedyne_fe2o3_font::set::FontSet;
use oxedyne_fe2o3_graphics::pdf::PdfPage;

use std::collections::HashMap;
use std::path::{
	Path,
	PathBuf,
};
use std::sync::Arc;

use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;

/// A long-lived Austenite compiler for the browser: the embedded reading set built once, and the last
/// compile's resolved ledger and heading table kept for [`DaimondTypst::query_project`].
#[wasm_bindgen]
pub struct DaimondTypst {
	// The embedded Libertinus reading set, built once and reused so a compile does not re-parse the five
	// faces each call. `None` only if the embedded bytes failed to parse -- which does not happen in
	// practice; a compile then reports it rather than the constructor throwing.
	fonts:			Option<Arc<FontSet>>,
	// The last compile's ledger and heading table, so a section-rail query reads heading pages back without
	// recompiling. Replaced by each successful compile's own values.
	last_ledger:	Option<Ledger>,
	last_heads:		Vec<Heading>,
	// A strictly monotonic compile tick for the changed-only delta (see `compile_project_delta`). The one
	// piece of delta state this instance keeps: NOT the prior id set, which lives with the consumer and is
	// supplied on each compile -- only a counter, so a consumer can discard a stale async return by version.
	// Zero until the first delta.
	version:		u32,
	// The incremental block-authoring memo, retained across delta compiles so an unedited block splices its
	// previously authored nodes back rather than re-shaping and re-breaking its paragraphs -- the whole point
	// of the swap (see `compile_project_delta`). Only the BLOCK-authoring layer fills here: the delta renders
	// each changed page through `svg::render_page` (not the page-emit memo), so the page-SVG cache stays empty
	// and the heap holds the working set of blocks, never the rendered document (the consumer holds that). The
	// non-delta `compileProject`/`compileProjectVector` paths pass no memo, so their bytes are untouched.
	memo:			Memo,
}

#[wasm_bindgen]
impl DaimondTypst {
	/// Builds a compiler instance, parsing the embedded reading set once. Infallible by contract -- a font
	/// that will not parse leaves the set unbuilt and is reported at compile time, never thrown here.
	#[wasm_bindgen(constructor)]
	pub fn new() -> DaimondTypst {
		DaimondTypst {
			fonts:			fonts::libertinus().ok().map(Arc::new),
			last_ledger:	None,
			last_heads:		Vec::new(),
			version:		0,
			memo:			Memo::new(),
		}
	}

	/// Compiles a project to a single PDF: `{ pdf: Uint8Array }` on success, `{ error: String }` otherwise.
	/// `project` is `{ main, sources: [[path, text], ...], assets: [[path, bytes], ...], fonts: [[path,
	/// bytes], ...] }`; every entry is injected into the source map and nothing outside it is read.
	#[wasm_bindgen(js_name = compileProject)]
	pub fn compile_project(&mut self, project: &JsValue) -> JsValue {
		match self.run(project, Mode::Pdf) {
			Ok(Product::Pdf(bytes))	=> ok_pdf(bytes),
			Ok(Product::Svg(_))		=> err_obj("internal: a PDF compile returned SVG"),
			Err(msg)				=> err_obj(&msg),
		}
	}

	/// The fast live-view path: compiles a project to Austenite's per-page SVG (already glyph outlines),
	/// `{ svg: string[] }` on success or `{ error }` otherwise. Cheaper than [`Self::compile_project`] --
	/// no PDF object graph, no cross-page stream -- so a watch loop can call it per keystroke.
	#[wasm_bindgen(js_name = compileProjectVector)]
	pub fn compile_project_vector(&mut self, project: &JsValue) -> JsValue {
		match self.run(project, Mode::Svg) {
			Ok(Product::Svg(pages))	=> ok_svg(pages),
			Ok(Product::Pdf(_))		=> err_obj("internal: an SVG compile returned PDF"),
			Err(msg)				=> err_obj(&msg),
		}
	}

	/// The live-view path Daimond consumes: compiles a project to a changed-only page delta, returning
	/// `{ version, order: string[], changed: [{ id, svg }], reset }` on success or `{ error }` otherwise.
	/// The project carries `known: string[]` -- the ids the consumer still holds in its own SVG cache -- and
	/// only the pages whose id is not among them carry their SVG in `changed`; the rest the consumer serves
	/// from that cache. The compiler keeps no cache state of its own, so a consumer that has cleared its
	/// cache (a document close or switch) sends `known: []` and gets a full resend (`reset: true`) rather
	/// than a blank preview against a cache that no longer holds anything. The memory-frugal successor to
	/// [`Self::compile_project_vector`] -- see [`crate::delta`] for the shape and the residency contract.
	/// Ids are opaque decimal strings, since a JavaScript number cannot hold every 64-bit hash exactly.
	/// The return also carries a `diagnostics: [{ file, line, col, message }]` array and, when non-empty, a
	/// terse `skipped` summary line: every construct the reader refused, each at its real source span, so a
	/// caller composes a `file:line:col` diagnostic exactly as the native `--explain` does rather than seeing
	/// the bare `<main>:0:` a swallowed refusal used to collapse to. These are additive -- the success fields
	/// `{ version, order, changed, reset }` are unchanged -- and `diagnostics` is empty on a clean compile.
	#[wasm_bindgen(js_name = compileProjectDelta)]
	pub fn compile_project_delta(&mut self, project: &JsValue) -> JsValue {
		match self.run_delta(project) {
			Ok(out)		=> ok_delta(&out),
			Err(msg)	=> err_obj(&msg),
		}
	}

	/// Compiles a single source string to PDF, wrapping it as the project's `/main.typ`. The convenience
	/// entry for a one-file document with no injected assets or fonts.
	#[wasm_bindgen]
	pub fn compile(&mut self, source: &str) -> JsValue {
		self.compile_project(&single_source_project(source))
	}

	/// The section rail's query: heading rows from the last compile's ledger as a JSON array of
	/// `{ kind, label, title, level, page }`, or `null` when nothing has compiled yet or the selector is
	/// one this lane does not resolve. The consumer degrades on `null`.
	///
	/// This is a heading/anchor query only, not full Typst `query` semantics: a selector naming headings
	/// (the section rail's use) returns heading rows; any other selector returns `null` so the caller falls
	/// back rather than receiving a wrong answer.
	#[wasm_bindgen(js_name = queryProject)]
	pub fn query_project(&self, selector: &str, _field: &str) -> JsValue {
		let ledger = match &self.last_ledger {
			Some(l)	=> l,
			None	=> return JsValue::NULL,
		};
		let sel = selector.to_lowercase();
		if !(sel.contains("head") || sel.contains("outline") || sel.is_empty()) {
			return JsValue::NULL;
		}
		let mut rows: Vec<Dat> = Vec::new();
		for h in &self.last_heads {
			let page = match ledger.page_of(&h.id) {
				Some(p)	=> p,
				None	=> continue,
			};
			rows.push(omapdat!{
				"kind"	=> dat!("heading"),
				"label"	=> dat!(h.id.key.clone()),
				"title"	=> dat!(h.title.clone()),
				"level"	=> dat!(h.level as u32),
				"page"	=> dat!(page),
			});
		}
		let json = match Dat::List(rows).json() {
			Ok(s)	=> s,
			Err(_)	=> return JsValue::NULL,
		};
		match js_sys::JSON::parse(&json) {
			Ok(v)	=> v,
			Err(_)	=> JsValue::NULL,
		}
	}

	/// The compiler's current wasm linear-memory size, in megabytes -- a heap-usage proxy for the app's
	/// memory gauge.
	#[wasm_bindgen(js_name = heapMB)]
	pub fn heap_mb(&self) -> f64 {
		// `memory_size` counts 64 KiB pages of the one linear memory, so pages / 16 is the size in MiB.
		core::arch::wasm32::memory_size(0) as f64 / 16.0
	}
}

impl Default for DaimondTypst {
	fn default() -> Self {
		Self::new()
	}
}

/// Which artefact a run produces.
#[derive(Clone, Copy)]
enum Mode {
	Pdf,
	Svg,
}

/// A run's product, matching the mode it was asked for.
enum Product {
	Pdf(Vec<u8>),
	Svg(Vec<String>),
}

impl DaimondTypst {
	/// Runs a compile and clears the source map before returning either way, so one instance compiles many
	/// documents in turn without a stale file leaking between them. Composes the engine's message with the
	/// main file for the app's `file:line: message` diagnostic.
	fn run(&mut self, project: &JsValue, mode: Mode) -> Result<Product, String> {
		let main = string_field(project, "main").unwrap_or_else(|| "/main.typ".to_string());
		let outcome = self.run_inner(project, &main, mode);
		let _ = vfs::clear();
		match outcome {
			Ok(p)	=> Ok(p),
			Err(e)	=> Err(fmt!("{}:0: {}", main, e)),
		}
	}

	/// Clears the source map before returning either way, as [`Self::run`] does, and steps the compile tick:
	/// computes the changed-only delta of this compile against the consumer's supplied `known` ids. The
	/// compiler retains no prior id set; only the rendered SVG of a newly-changed page is built, carried
	/// into the delta, and dropped.
	fn run_delta(&mut self, project: &JsValue) -> Result<DeltaOut, String> {
		let main = string_field(project, "main").unwrap_or_else(|| "/main.typ".to_string());
		let outcome = self.run_delta_inner(project, &main);
		let _ = vfs::clear();
		match outcome {
			Ok(out)	=> Ok(out),
			Err(e)	=> Err(fmt!("{}:0: {}", main, e)),
		}
	}

	fn run_delta_inner(&mut self, project: &JsValue, main: &str) -> Outcome<DeltaOut> {
		// The consumer owns the SVG cache, so it -- not this instance -- is the authority on which page ids
		// are already held. It supplies them as `known`; a page whose id is not among them is resent. Holding
		// the prior set here would blank the preview whenever the consumer cleared its cache (a document
		// close or switch) while this singleton compiler lived on: the delta would report nothing changed
		// against a cache holding nothing. So `reset` is `known.is_empty()` by construction -- true exactly
		// when the consumer has nothing to reuse, and a cleared cache recovers with a full resend.
		let known						= parse_known(project);
		// The delta path passes the persistent block-authoring memo, so an unedited block splices its cached
		// nodes rather than re-authoring: the incremental recompile the swap exists for. The refusal table and
		// skip line are carried out here (not discarded) for the diagnostics field.
		let (rendered, refusals, skip)	= res!(self.assemble_and_run(project, main, true));
		// Close the memo generation now the authoring pass is done, dropping block entries untouched for two
		// compiles. `author_and_run_memo` opened it with `Memo::begin`; the page-emit cache is never touched on
		// this path (the delta renders through `svg::render_page`), so nothing but blocks is swept, and the
		// retained heap is the working set of blocks -- never the rendered page SVG.
		self.memo.sweep();
		// Read each refused site's line and column from the source it was tagged with, still in the source map
		// here (the caller clears it after this returns), so a diagnostic carries a real `file:line:col`.
		let diagnostics	= diagnostics_from_refusals(&refusals);
		let d			= res!(delta::compute(&rendered.out.pages, &known, self.version));
		// The version is the one piece of delta state this instance keeps: a strictly monotonic tick, so a
		// consumer that dispatches compiles without awaiting each can discard a stale async return by its
		// version. It is deliberately not consumer-supplied -- two compiles dispatched before either returned
		// would carry the same supplied version and could not be ordered -- and a cache clear does not
		// disturb it, since recovery is driven by an empty `known`, not by the counter.
		self.version	= d.version;
		Ok(DeltaOut { delta: d, diagnostics, skip })
	}

	/// Assembles, authors, runs, decorates and mirror-shifts a project through the shared pipeline -- the
	/// same code the native binary drives, so the two surfaces cannot drift -- and keeps the resolved ledger
	/// and heading table for a later section-rail query. The lone-file path takes this instance's once-built
	/// reading set rather than rebuilding it; the base directory for `/assets/...` figures is set inside
	/// `assemble`. It returns the refusal table and terse skip line for the delta path's diagnostics; the
	/// caller clears the source map (through [`Self::run`] or [`Self::run_delta`]) once it has the result.
	///
	/// `use_memo` threads this instance's persistent block-authoring memo through the authoring stage (the
	/// delta path), so an unedited block reuses its cached layout across recompiles; the non-memo paths pass
	/// `false` and stay byte-identical to the native production compile.
	fn assemble_and_run(&mut self, project: &JsValue, main: &str, use_memo: bool)
		-> Outcome<(compile::Rendered, lang::Refusals, Option<String>)>
	{
		let fonts = match &self.fonts {
			Some(f)	=> f.clone(),
			None	=> return Err(err!("The embedded font set could not be built."; Init, Missing)),
		};

		let main_path = PathBuf::from(main);
		// Every source, asset and font becomes a source-map entry; the main path names the root among them.
		let mut files: HashMap<PathBuf, Vec<u8>> = HashMap::new();
		read_text_pairs(project, "sources", &mut files);
		read_byte_pairs(project, "assets", &mut files);
		// A font is installed at the path the consumer named AND at the location the lone-file face resolver
		// reads, so a face the document names resolves whatever path the consumer chose (see `read_font_pairs`).
		read_font_pairs(project, &main_path, &mut files);
		if !files.contains_key(&main_path) {
			return Err(err!("The project has no source for its main file {:?}.", main; Input, Missing));
		}
		res!(vfs::install(files));

		let (assembled, refusals, skip)	= res!(compile::assemble(&main_path, || Ok(fonts.clone())));
		let rendered = if use_memo {
			res!(compile::author_and_run_memo(assembled, Some(&mut self.memo)))
		} else {
			res!(compile::author_and_run(assembled))
		};

		// Keep the resolved ledger and heading table for a later section-rail query.
		self.last_ledger	= Some(rendered.out.ledger.clone());
		self.last_heads		= rendered.heads.clone();
		Ok((rendered, refusals, skip))
	}

	fn run_inner(&mut self, project: &JsValue, main: &str, mode: Mode) -> Outcome<Product> {
		let (rendered, _refusals, _skip)	= res!(self.assemble_and_run(project, main, false));
		let compile::Rendered { mut out, heads, geom: _ } = rendered;

		match mode {
			Mode::Svg => {
				let mut pages: Vec<String> = Vec::with_capacity(out.pages.len());
				for page in &out.pages {
					pages.push(res!(svg::render_page(page)));
				}
				Ok(Product::Svg(pages))
			},
			Mode::Pdf => {
				// Sequential emit: wasm has no threads, so the binary's parallel chunking becomes a
				// page-at-a-time write into an in-memory buffer, freeing each frame after its page is folded in.
				let mut buf: Vec<u8> = Vec::new();
				let outline = compile::build_outline(&heads, &out.ledger);
				let mut pdf = res!(emit::pdf::open_document_with_outline(&mut buf, out.pages.len(), outline));
				for page in &mut out.pages {
					let built: PdfPage = res!(emit::pdf::render_page(page));
					res!(emit::pdf::write_built_page(&mut pdf, &built));
					page.frame = Frame::new();
				}
				res!(pdf.finish());
				Ok(Product::Pdf(buf))
			},
		}
	}
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ JS INTEROP                                                                 │
// └───────────────────────────────────────────────────────────────────────────┘

/// A single-source project object `{ main: "/main.typ", sources: [["/main.typ", source]] }`.
fn single_source_project(source: &str) -> JsValue {
	let obj		= js_sys::Object::new();
	let _		= js_sys::Reflect::set(&obj, &JsValue::from_str("main"), &JsValue::from_str("/main.typ"));
	let sources	= js_sys::Array::new();
	let pair	= js_sys::Array::new();
	pair.push(&JsValue::from_str("/main.typ"));
	pair.push(&JsValue::from_str(source));
	sources.push(&pair);
	let _ = js_sys::Reflect::set(&obj, &JsValue::from_str("sources"), &sources);
	obj.into()
}

/// Reads a `[[path, text], ...]` field into the source map, each text UTF-8 encoded.
fn read_text_pairs(project: &JsValue, key: &str, out: &mut HashMap<PathBuf, Vec<u8>>) {
	let arr = match array_field(project, key) {
		Some(a)	=> a,
		None	=> return,
	};
	for entry in arr.iter() {
		if let Ok(pair) = entry.dyn_into::<js_sys::Array>() {
			if let (Some(path), Some(text)) = (pair.get(0).as_string(), pair.get(1).as_string()) {
				out.insert(PathBuf::from(path), text.into_bytes());
			}
		}
	}
}

/// Reads a `[[path, bytes], ...]` field into the source map, each value a `Uint8Array` or `ArrayBuffer`.
fn read_byte_pairs(project: &JsValue, key: &str, out: &mut HashMap<PathBuf, Vec<u8>>) {
	let arr = match array_field(project, key) {
		Some(a)	=> a,
		None	=> return,
	};
	for entry in arr.iter() {
		if let Ok(pair) = entry.dyn_into::<js_sys::Array>() {
			if let Some(path) = pair.get(0).as_string() {
				if let Some(bytes) = to_bytes(&pair.get(1)) {
					out.insert(PathBuf::from(path), bytes);
				}
			}
		}
	}
}

/// Reads the project's `fonts: [[path, bytes], ...]` into the source map, each installed at the path the
/// consumer named AND -- so the lone-file face resolver discovers it whatever path was chosen -- at the
/// resolver's own `<root>/assets/fonts/<basename>` location (see [`book::project_font_path`]). Without the
/// second placement an injected font is present in the map but invisible to the resolver, which reads only
/// its own directory: a face the document names would silently fall back to the reading role. The consumer
/// still names a usable face by its `<Family>-<Variant>.{ttf,otf}` basename and declares that family as a
/// heading face; this makes such a font resolve regardless of the path it was injected under.
fn read_font_pairs(project: &JsValue, main_path: &Path, out: &mut HashMap<PathBuf, Vec<u8>>) {
	let arr = match array_field(project, "fonts") {
		Some(a)	=> a,
		None	=> return,
	};
	for entry in arr.iter() {
		if let Ok(pair) = entry.dyn_into::<js_sys::Array>() {
			if let Some(path) = pair.get(0).as_string() {
				if let Some(bytes) = to_bytes(&pair.get(1)) {
					let given = PathBuf::from(path);
					if let Some(routed) = book::project_font_path(main_path, &given) {
						if routed != given {
							out.insert(routed, bytes.clone());
						}
					}
					out.insert(given, bytes);
				}
			}
		}
	}
}

/// The bytes of a `Uint8Array` or an `ArrayBuffer`, or `None` for anything else.
fn to_bytes(v: &JsValue) -> Option<Vec<u8>> {
	if let Ok(u8arr) = v.clone().dyn_into::<js_sys::Uint8Array>() {
		return Some(u8arr.to_vec());
	}
	if let Ok(buf) = v.clone().dyn_into::<js_sys::ArrayBuffer>() {
		return Some(js_sys::Uint8Array::new(&buf).to_vec());
	}
	None
}

/// A string-valued field of a JS object, or `None` when it is absent or not a string.
fn string_field(obj: &JsValue, key: &str) -> Option<String> {
	js_sys::Reflect::get(obj, &JsValue::from_str(key)).ok().and_then(|v| v.as_string())
}

/// An array-valued field of a JS object, or `None` when it is absent or not an array.
fn array_field(obj: &JsValue, key: &str) -> Option<js_sys::Array> {
	js_sys::Reflect::get(obj, &JsValue::from_str(key)).ok().and_then(|v| v.dyn_into::<js_sys::Array>().ok())
}

/// The consumer's currently-cached page ids, read from the project's `known: string[]` field and parsed
/// from decimal (the shape [`ok_delta`] emits them in). An absent field, or an entry that is not a decimal
/// string, is skipped; an empty result forces a full reset -- the safe direction, a resend over a blank.
fn parse_known(project: &JsValue) -> Vec<u64> {
	let arr = match array_field(project, "known") {
		Some(a)	=> a,
		None	=> return Vec::new(),
	};
	let mut ids = Vec::with_capacity(arr.length() as usize);
	for entry in arr.iter() {
		if let Some(s) = entry.as_string() {
			if let Ok(id) = s.parse::<u64>() {
				ids.push(id);
			}
		}
	}
	ids
}

/// `{ pdf: Uint8Array }`.
fn ok_pdf(bytes: Vec<u8>) -> JsValue {
	let obj = js_sys::Object::new();
	let arr = js_sys::Uint8Array::from(bytes.as_slice());
	let _ = js_sys::Reflect::set(&obj, &JsValue::from_str("pdf"), &arr);
	obj.into()
}

/// `{ svg: string[] }`.
fn ok_svg(pages: Vec<String>) -> JsValue {
	let obj = js_sys::Object::new();
	let arr = js_sys::Array::new();
	for s in &pages {
		arr.push(&JsValue::from_str(s));
	}
	let _ = js_sys::Reflect::set(&obj, &JsValue::from_str("svg"), &arr);
	obj.into()
}

/// One refused construct, resolved to the source position a caller shows the user: the file it was read
/// from, the 1-based line and column its span starts at, and a message naming the construct and why it was
/// refused. The wasm delta return carries a list of these so a swallowed refusal reads back as a real
/// `file:line:col` diagnostic rather than the bare `<main>:0:` it used to collapse to.
struct Diagnostic {
	file:	String,
	line:	usize,
	col:	usize,
	message:	String,
}

/// A delta compile's result plus the diagnostics the delta path now surfaces: the changed-only page delta,
/// every refused site at its source span, and the terse skip summary (`None` when nothing was skipped).
struct DeltaOut {
	delta:	delta::PageDelta,
	diagnostics:	Vec<Diagnostic>,
	skip:	Option<String>,
}

/// Resolves each refused site to a [`Diagnostic`], reading the tagged source file's line and column from
/// the source map (still installed when this runs, before the caller clears it). A site whose source is not
/// in the map -- which should not happen, every refusal is tagged with an injected file -- still reports,
/// at column one, so a refusal is never silently dropped.
fn diagnostics_from_refusals(refusals: &lang::Refusals) -> Vec<Diagnostic> {
	let mut out = Vec::new();
	let mut cache: HashMap<String, Option<String>> = HashMap::new();
	for r in refusals.sites() {
		let src = cache.entry(r.file.clone())
			.or_insert_with(|| vfs::read_to_string(&PathBuf::from(&r.file)).ok());
		let (line, col) = match src {
			Some(text)	=> { let (l, c, _) = lang::line_col_of(text, r.span.start); (l, c) },
			None		=> (0, 1),
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

/// `{ version, order: string[], changed: [{ id, svg }], reset, diagnostics: [{ file, line, col, message }],
/// skipped? }` -- the changed-only delta with its diagnostics. Ids (page content hashes) are decimal
/// strings, since a JavaScript number holds only 53 bits exactly and would silently corrupt a 64-bit hash;
/// the consumer treats them as opaque keys. `diagnostics` is always present (empty on a clean compile);
/// `skipped` is set only when the reader passed over a construct.
fn ok_delta(out: &DeltaOut) -> JsValue {
	let d = &out.delta;
	let obj = js_sys::Object::new();
	let _ = js_sys::Reflect::set(&obj, &JsValue::from_str("version"), &JsValue::from_f64(d.version as f64));
	let order = js_sys::Array::new();
	for id in &d.order {
		order.push(&JsValue::from_str(&fmt!("{}", id)));
	}
	let _ = js_sys::Reflect::set(&obj, &JsValue::from_str("order"), &order);
	let changed = js_sys::Array::new();
	for (id, svg) in &d.changed {
		let entry = js_sys::Object::new();
		let _ = js_sys::Reflect::set(&entry, &JsValue::from_str("id"), &JsValue::from_str(&fmt!("{}", id)));
		let _ = js_sys::Reflect::set(&entry, &JsValue::from_str("svg"), &JsValue::from_str(svg));
		changed.push(&entry);
	}
	let _ = js_sys::Reflect::set(&obj, &JsValue::from_str("changed"), &changed);
	let _ = js_sys::Reflect::set(&obj, &JsValue::from_str("reset"), &JsValue::from_bool(d.reset));
	let diags = js_sys::Array::new();
	for diag in &out.diagnostics {
		let entry = js_sys::Object::new();
		let _ = js_sys::Reflect::set(&entry, &JsValue::from_str("file"), &JsValue::from_str(&diag.file));
		let _ = js_sys::Reflect::set(&entry, &JsValue::from_str("line"), &JsValue::from_f64(diag.line as f64));
		let _ = js_sys::Reflect::set(&entry, &JsValue::from_str("col"), &JsValue::from_f64(diag.col as f64));
		let _ = js_sys::Reflect::set(&entry, &JsValue::from_str("message"), &JsValue::from_str(&diag.message));
		diags.push(&entry);
	}
	let _ = js_sys::Reflect::set(&obj, &JsValue::from_str("diagnostics"), &diags);
	if let Some(skip) = &out.skip {
		let _ = js_sys::Reflect::set(&obj, &JsValue::from_str("skipped"), &JsValue::from_str(skip));
	}
	obj.into()
}

/// `{ error: String }` -- the shape every method returns on failure, so a caller never sees a throw.
fn err_obj(msg: &str) -> JsValue {
	let obj = js_sys::Object::new();
	let _ = js_sys::Reflect::set(&obj, &JsValue::from_str("error"), &JsValue::from_str(msg));
	obj.into()
}
