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
//! Every method resolves; none rejects or throws. A failure returns `{ error: "<file>:<line>:<col>:
//! <message>", diagnostics, skipped }` -- `0:0` where the cause could not be traced to a source line --
//! never a bare access-denied line and never a JavaScript exception, so a caller composes its diagnostics
//! from a value it always receives. A project carrying `strict: true` turns a result that passed over a
//! construct, produced no pages or set no content into such a failure, so a partial PDF never reads as
//! success.
//!
//! One capability is deliberately out of this lane and documented as a gap rather than stubbed: a recompile
//! is from scratch -- the instance is shaped to hold an incremental block cache, but this lane does not
//! build one. The per-page SVG does carry a transparent selectable-text layer -- a `.tsel` twin of the
//! glyph outlines, emitted by [`crate::emit::svg`] -- which is what [`DaimondTypst::compile_project_vector`]
//! returns and the section rail selects over.

use crate::book;
use crate::compile::{
	self,
	Diagnostic,
	Report,
};
use crate::delta;
use crate::doc::Heading;
use crate::emit::svg;
use crate::fonts;
use crate::ledger::Ledger;
use crate::memo::Memo;
use crate::vfs;

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_jdat::prelude::*;
use oxedyne_fe2o3_font::set::FontSet;

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

	/// Compiles a project to a single PDF: `{ pdf: Uint8Array, pages, diagnostics: [{ file, line, col,
	/// message }], skipped: string | null }` on success, `{ error, diagnostics, skipped }` otherwise.
	/// `project` is `{ main, sources: [[path, text], ...], assets: [[path, bytes], ...], fonts: [[path,
	/// bytes], ...], strict?: bool }`; every entry is injected into the source map and nothing outside it
	/// is read. `diagnostics` lists every construct the reader passed over; under `strict` any such site,
	/// zero pages or a source setting no content is returned as `{ error }` instead of a PDF.
	#[wasm_bindgen(js_name = compileProject)]
	pub fn compile_project(&mut self, project: &JsValue) -> JsValue {
		match self.run(project, Mode::Pdf) {
			Ok((Product::Pdf(bytes), rep))	=> ok_pdf(bytes, &rep),
			Ok((Product::Svg(_), _))		=> err_obj(&internal("a PDF compile returned SVG")),
			Err(fail)						=> err_obj(&fail),
		}
	}

	/// The fast live-view path: compiles a project to Austenite's per-page SVG (already glyph outlines),
	/// `{ svg: string[], pages, diagnostics, skipped }` on success or `{ error, diagnostics, skipped }`
	/// otherwise, `strict` as for [`Self::compile_project`]. Cheaper than a PDF -- no object graph, no
	/// cross-page stream -- so a watch loop can call it per keystroke.
	#[wasm_bindgen(js_name = compileProjectVector)]
	pub fn compile_project_vector(&mut self, project: &JsValue) -> JsValue {
		match self.run(project, Mode::Svg) {
			Ok((Product::Svg(pages), rep))	=> ok_svg(pages, &rep),
			Ok((Product::Pdf(_), _))		=> err_obj(&internal("an SVG compile returned PDF")),
			Err(fail)						=> err_obj(&fail),
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
	/// The return also carries `pages`, `diagnostics: [{ file, line, col, message }]` and `skipped` (the
	/// terse summary line, or `null`) exactly as [`Self::compile_project`] does, and honours `strict` the
	/// same way. On a strict refusal the version does not step, so the consumer's cache stays valid.
	#[wasm_bindgen(js_name = compileProjectDelta)]
	pub fn compile_project_delta(&mut self, project: &JsValue) -> JsValue {
		match self.run_delta(project) {
			Ok(out)		=> ok_delta(&out),
			Err(fail)	=> err_obj(&fail),
		}
	}

	/// The font families a compile of `project` can set by name, as a sorted `string[]`: the embedded
	/// families (`Libertinus Serif`, `Libertinus Mono`, `Latin Modern Math`) and the family of each
	/// `project.fonts` entry named `<Family>-<Variant>.{ttf,otf}` that the engine's face resolver loads.
	/// `project` is optional; with none, or with no fonts, only the embedded families are listed. For a
	/// missing-font pre-check before a compile.
	#[wasm_bindgen(js_name = fontFamilies)]
	pub fn font_families(&self, project: &JsValue) -> JsValue {
		let main_path	= PathBuf::from(main_of(project));
		let mut files: HashMap<PathBuf, Vec<u8>> = HashMap::new();
		let injected	= read_font_pairs(project, &main_path, &mut files);
		let families	= if files.is_empty() {
			compile::font_families(&main_path, &[])
		} else {
			match vfs::install(files) {
				Ok(())	=> {
					let f = compile::font_families(&main_path, &injected);
					let _ = vfs::clear();
					f
				},
				Err(_)	=> compile::font_families(&main_path, &[]),
			}
		};
		let arr = js_sys::Array::new();
		for f in &families {
			arr.push(&JsValue::from_str(f));
		}
		arr.into()
	}

	/// The engine's identity, `{ engine: "austenite", version, git }`: the crate version and the commit it
	/// was built from (`<12 hex>`, `-dirty` when built from uncommitted changes, or `unknown`).
	#[wasm_bindgen(js_name = engineInfo)]
	pub fn engine_info(&self) -> JsValue {
		let obj = js_sys::Object::new();
		set(&obj, "engine", &JsValue::from_str("austenite"));
		set(&obj, "version", &JsValue::from_str(compile::engine_version()));
		set(&obj, "git", &JsValue::from_str(compile::engine_git_hash()));
		obj.into()
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

/// Runs a compile closure under [`std::panic::catch_unwind`], turning a panic into an ordinary error rather
/// than letting it escape as a wasm trap. A trap unwinds no Rust state and leaves the instance's shadow
/// stack unrestored, so a single panicked compile would corrupt the [`DaimondTypst`] instance until the page
/// reloaded; catching it here keeps the instance usable and surfaces an `{ error }` object instead. The
/// engine's own cycle and depth caps mean a well-formed document never panics -- this is the net for the
/// unforeseen. (Under a `panic = "abort"` build the abort still traps; the guard is effective wherever
/// unwinding is enabled, and is harmless otherwise.)
fn catch_compile<T, F>(f: F, main: &str) -> Outcome<T>
where
	F: FnOnce() -> Outcome<T>,
{
	match std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
		Ok(outcome)	=> outcome,
		Err(_)		=> Err(err!(
			"The compiler panicked while assembling {:?}; the document was not produced.", main; Bug)),
	}
}

/// A compile that did not produce its artefact: the leading diagnostic (the `{ error }` line) and whatever
/// report the compile got as far as building -- all its refusals under a strict refusal, none for a hard
/// error, which stops before the report exists.
struct Failure {
	head:	Diagnostic,
	report:	Option<Report>,
}

/// What [`DaimondTypst::run_inner`] ends with: the artefact and its report, or a strict refusal.
enum Ran<T> {
	Done(T, Report),
	Refused(Diagnostic, Report),
}

impl DaimondTypst {
	/// Installs the project, runs a compile and clears the source map before returning either way, so one
	/// instance compiles many documents in turn without a stale file leaking between them. A hard error is
	/// placed at a source position while the map is still installed.
	fn run(&mut self, project: &JsValue, mode: Mode) -> Result<(Product, Report), Failure> {
		let main	= main_of(project);
		let strict	= bool_field(project, "strict");
		self.guarded(project, &main, |me, main_path| me.run_inner(main_path, mode, strict))
	}

	/// As [`Self::run`] for the changed-only delta: steps the compile tick and computes the delta of this
	/// compile against the consumer's supplied `known` ids. The compiler retains no prior id set; only the
	/// rendered SVG of a newly-changed page is built, carried into the delta, and dropped.
	fn run_delta(&mut self, project: &JsValue) -> Result<DeltaOut, Failure> {
		let main	= main_of(project);
		let strict	= bool_field(project, "strict");
		let known	= parse_known(project);
		match self.guarded(project, &main, |me, main_path| me.run_delta_inner(main_path, &known, strict)) {
			Ok((delta, report))	=> Ok(DeltaOut { delta, report }),
			Err(fail)			=> Err(fail),
		}
	}

	/// The shared frame of every compile: install the project into the source map, run `body` under the
	/// panic guard, place a hard error at its source position, and clear the map whatever happened.
	fn guarded<T, F>(&mut self, project: &JsValue, main: &str, body: F) -> Result<(T, Report), Failure>
	where
		F: FnOnce(&mut Self, &Path) -> Outcome<Ran<T>>,
	{
		let main_path	= PathBuf::from(main);
		let sources		= match install_project(project, &main_path) {
			Ok(s)	=> s,
			Err(e)	=> {
				let _ = vfs::clear();
				return Err(Failure { head: compile::locate_error(&e, &main_path, &[]), report: None });
			},
		};
		let outcome	= catch_compile(|| body(self, &main_path), main);
		let result	= match outcome {
			Ok(Ran::Done(t, report))		=> Ok((t, report)),
			Ok(Ran::Refused(head, report))	=> Err(Failure { head, report: Some(report) }),
			Err(e)							=> Err(Failure {
				head:	compile::locate_error(&e, &main_path, &sources),
				report:	None,
			}),
		};
		let _ = vfs::clear();
		result
	}

	fn run_delta_inner(&mut self, main_path: &Path, known: &[u64], strict: bool)
		-> Outcome<Ran<delta::PageDelta>>
	{
		// The consumer owns the SVG cache, so it -- not this instance -- is the authority on which page ids
		// are already held. It supplies them as `known`; a page whose id is not among them is resent. Holding
		// the prior set here would blank the preview whenever the consumer cleared its cache (a document
		// close or switch) while this singleton compiler lived on: the delta would report nothing changed
		// against a cache holding nothing. So `reset` is `known.is_empty()` by construction -- true exactly
		// when the consumer has nothing to reuse, and a cleared cache recovers with a full resend.
		//
		// The delta path passes the persistent block-authoring memo, so an unedited block splices its cached
		// nodes rather than re-authoring: the incremental recompile the swap exists for.
		let (rendered, report)	= res!(self.assemble_and_run(main_path, true));
		// Close the memo generation now the authoring pass is done, dropping block entries untouched for two
		// compiles. `author_and_run_memo` opened it with `Memo::begin`; the page-emit cache is never touched on
		// this path (the delta renders through `svg::render_page`), so nothing but blocks is swept, and the
		// retained heap is the working set of blocks -- never the rendered page SVG.
		self.memo.sweep();
		if strict {
			if let Some(head) = report.strict_failure(main_path) {
				return Ok(Ran::Refused(head, report));
			}
		}
		let d = res!(delta::compute(&rendered.out.pages, known, self.version));
		// The version is the one piece of delta state this instance keeps: a strictly monotonic tick, so a
		// consumer that dispatches compiles without awaiting each can discard a stale async return by its
		// version. It is deliberately not consumer-supplied -- two compiles dispatched before either returned
		// would carry the same supplied version and could not be ordered -- and a cache clear does not
		// disturb it, since recovery is driven by an empty `known`, not by the counter.
		self.version = d.version;
		Ok(Ran::Done(d, report))
	}

	/// Assembles, authors, runs, decorates and mirror-shifts the installed project through the shared
	/// pipeline -- the same code the native binary drives, so the two surfaces cannot drift -- and keeps the
	/// resolved ledger and heading table for a later section-rail query. The lone-file path takes this
	/// instance's once-built reading set rather than rebuilding it; the base directory for `/assets/...`
	/// figures is set inside `assemble`. The report is built here, while the source map still holds the
	/// text each refusal's position is read from.
	///
	/// `use_memo` threads this instance's persistent block-authoring memo through the authoring stage (the
	/// delta path), so an unedited block reuses its cached layout across recompiles; the non-memo paths pass
	/// `false` and stay byte-identical to the native production compile.
	fn assemble_and_run(&mut self, main_path: &Path, use_memo: bool) -> Outcome<(compile::Rendered, Report)> {
		let fonts = match &self.fonts {
			Some(f)	=> f.clone(),
			None	=> return Err(err!("The embedded font set could not be built."; Init, Missing)),
		};
		let (assembled, refusals, skip)	= res!(compile::assemble(main_path, || Ok(fonts.clone())));
		let empty = assembled.blocks.is_empty();
		let rendered = if use_memo {
			res!(compile::author_and_run_memo(assembled, Some(&mut self.memo)))
		} else {
			res!(compile::author_and_run(assembled))
		};
		let report = Report::new(rendered.out.pages.len(), &refusals, skip, empty);

		// Keep the resolved ledger and heading table for a later section-rail query.
		self.last_ledger	= Some(rendered.out.ledger.clone());
		self.last_heads		= rendered.heads.clone();
		Ok((rendered, report))
	}

	fn run_inner(&mut self, main_path: &Path, mode: Mode, strict: bool) -> Outcome<Ran<Product>> {
		let (rendered, report)	= res!(self.assemble_and_run(main_path, false));
		// A strict refusal is decided before the emit, so a refused compile spends nothing on a PDF.
		if strict {
			if let Some(head) = report.strict_failure(main_path) {
				return Ok(Ran::Refused(head, report));
			}
		}
		let compile::Rendered { mut out, heads, geom: _ } = rendered;
		let product = match mode {
			Mode::Svg => {
				let mut pages: Vec<String> = Vec::with_capacity(out.pages.len());
				for page in &out.pages {
					pages.push(res!(svg::render_page(page)));
				}
				Product::Svg(pages)
			},
			Mode::Pdf => Product::Pdf(res!(compile::emit_pdf(&mut out, &heads))),
		};
		Ok(Ran::Done(product, report))
	}
}

/// Installs every source, asset and font of `project` into the source map, the main path naming the root
/// among them, and returns the installed paths for placing a later error.
fn install_project(project: &JsValue, main_path: &Path) -> Outcome<Vec<PathBuf>> {
	let mut files: HashMap<PathBuf, Vec<u8>> = HashMap::new();
	read_text_pairs(project, "sources", &mut files);
	read_byte_pairs(project, "assets", &mut files);
	// A font is installed at the path the consumer named AND at the location the lone-file face resolver
	// reads, so a face the document names resolves whatever path the consumer chose (see `read_font_pairs`).
	let _ = read_font_pairs(project, main_path, &mut files);
	if !files.contains_key(main_path) {
		return Err(err!("The project has no source for its main file {:?}.", main_path; Input, Missing));
	}
	let mut paths: Vec<PathBuf> = files.keys().cloned().collect();
	paths.sort();
	res!(vfs::install(files));
	Ok(paths)
}

/// The project's main path, `/main.typ` when it names none.
fn main_of(project: &JsValue) -> String {
	string_field(project, "main").unwrap_or_else(|| "/main.typ".to_string())
}

/// A failure the engine itself caused, with no source position to give.
fn internal(msg: &str) -> Failure {
	Failure {
		head:	Diagnostic { file: String::new(), line: 0, col: 0, message: fmt!("internal: {}", msg) },
		report:	None,
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
/// heading face; this makes such a font resolve regardless of the path it was injected under. Returns the
/// paths as the consumer gave them.
fn read_font_pairs(project: &JsValue, main_path: &Path, out: &mut HashMap<PathBuf, Vec<u8>>) -> Vec<PathBuf> {
	let mut given_paths = Vec::new();
	let arr = match array_field(project, "fonts") {
		Some(a)	=> a,
		None	=> return given_paths,
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
					given_paths.push(given.clone());
					out.insert(given, bytes);
				}
			}
		}
	}
	given_paths
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

/// A boolean field of a JS object, false when it is absent or not `true`.
fn bool_field(obj: &JsValue, key: &str) -> bool {
	match js_sys::Reflect::get(obj, &JsValue::from_str(key)) {
		Ok(v)	=> v.as_bool().unwrap_or(false),
		Err(_)	=> false,
	}
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

/// Sets one property; a failed set on a fresh plain object cannot happen, so it is not reported.
fn set(obj: &js_sys::Object, key: &str, val: &JsValue) {
	let _ = js_sys::Reflect::set(obj, &JsValue::from_str(key), val);
}

/// Sets `pages`, `diagnostics: [{ file, line, col, message }]` and `skipped` (string or `null`) from a
/// report, the fields every compile result carries.
fn set_report(obj: &js_sys::Object, rep: &Report) {
	set(obj, "pages", &JsValue::from_f64(rep.pages as f64));
	set(obj, "diagnostics", &diagnostics_array(&rep.diagnostics));
	let skipped = match &rep.skipped {
		Some(s)	=> JsValue::from_str(s),
		None	=> JsValue::NULL,
	};
	set(obj, "skipped", &skipped);
}

fn diagnostics_array(diags: &[Diagnostic]) -> js_sys::Array {
	let arr = js_sys::Array::new();
	for d in diags {
		let entry = js_sys::Object::new();
		set(&entry, "file",		&JsValue::from_str(&d.file));
		set(&entry, "line",		&JsValue::from_f64(d.line as f64));
		set(&entry, "col",		&JsValue::from_f64(d.col as f64));
		set(&entry, "message",	&JsValue::from_str(&d.message));
		arr.push(&entry);
	}
	arr
}

/// `{ pdf: Uint8Array, pages, diagnostics, skipped }`.
fn ok_pdf(bytes: Vec<u8>, rep: &Report) -> JsValue {
	let obj = js_sys::Object::new();
	set(&obj, "pdf", &js_sys::Uint8Array::from(bytes.as_slice()));
	set_report(&obj, rep);
	obj.into()
}

/// `{ svg: string[], pages, diagnostics, skipped }`.
fn ok_svg(pages: Vec<String>, rep: &Report) -> JsValue {
	let obj = js_sys::Object::new();
	let arr = js_sys::Array::new();
	for s in &pages {
		arr.push(&JsValue::from_str(s));
	}
	set(&obj, "svg", &arr);
	set_report(&obj, rep);
	obj.into()
}

/// A delta compile's result: the changed-only page delta and the compile's report.
struct DeltaOut {
	delta:	delta::PageDelta,
	report:	Report,
}

/// `{ version, order: string[], changed: [{ id, svg }], reset, pages, diagnostics, skipped }`. Ids (page
/// content hashes) are decimal strings, since a JavaScript number holds only 53 bits exactly and would
/// silently corrupt a 64-bit hash; the consumer treats them as opaque keys.
fn ok_delta(out: &DeltaOut) -> JsValue {
	let d = &out.delta;
	let obj = js_sys::Object::new();
	set(&obj, "version", &JsValue::from_f64(d.version as f64));
	let order = js_sys::Array::new();
	for id in &d.order {
		order.push(&JsValue::from_str(&fmt!("{}", id)));
	}
	set(&obj, "order", &order);
	let changed = js_sys::Array::new();
	for (id, svg) in &d.changed {
		let entry = js_sys::Object::new();
		set(&entry, "id", &JsValue::from_str(&fmt!("{}", id)));
		set(&entry, "svg", &JsValue::from_str(svg));
		changed.push(&entry);
	}
	set(&obj, "changed", &changed);
	set(&obj, "reset", &JsValue::from_bool(d.reset));
	set_report(&obj, &out.report);
	obj.into()
}

/// `{ error: "file:line:col: message", diagnostics, skipped }` -- the shape every compile returns on
/// failure, so a caller never sees a throw. `diagnostics` leads with the error's own entry, then every
/// refused site the compile reached; `skipped` is the terse line, or `null`.
fn err_obj(fail: &Failure) -> JsValue {
	let obj = js_sys::Object::new();
	set(&obj, "error", &JsValue::from_str(&fmt!("{}", fail.head)));
	let mut diags = vec![fail.head.clone()];
	let mut skipped = JsValue::NULL;
	if let Some(rep) = &fail.report {
		// A strict refusal's head is its first site restated; list the sites once, after it.
		diags.extend(rep.diagnostics.iter().cloned());
		set(&obj, "pages", &JsValue::from_f64(rep.pages as f64));
		if let Some(s) = &rep.skipped {
			skipped = JsValue::from_str(s);
		}
	}
	set(&obj, "diagnostics", &diagnostics_array(&diags));
	set(&obj, "skipped", &skipped);
	obj.into()
}
