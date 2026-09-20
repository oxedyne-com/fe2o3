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

use crate::compile;
use crate::delta;
use crate::doc::Heading;
use crate::emit::{
	self,
	svg,
};
use crate::fonts;
use crate::ledger::Ledger;
use crate::page::Frame;
use crate::vfs;

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_jdat::prelude::*;
use oxedyne_fe2o3_font::set::FontSet;
use oxedyne_fe2o3_graphics::pdf::PdfPage;

use std::collections::HashMap;
use std::path::PathBuf;
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
	// The last vector-delta compile's page-id sequence and version counter (see `compile_project_delta`).
	// The only state a delta keeps between compiles: the ids alone, never the rendered SVG, so the heap
	// holds one id per page rather than the whole rendered document. Empty and zero until the first delta.
	last_order:		Vec<u64>,
	version:		u32,
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
			last_order:		Vec::new(),
			version:		0,
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
	/// Only the pages whose rendered SVG changed since the last call carry their SVG in `changed`; the rest
	/// the consumer serves from its own id-keyed cache. This is the memory-frugal successor to
	/// [`Self::compile_project_vector`] -- see [`crate::delta`] for the shape and the residency contract.
	/// Ids are decimal strings, since a JavaScript number cannot hold every 64-bit hash exactly.
	#[wasm_bindgen(js_name = compileProjectDelta)]
	pub fn compile_project_delta(&mut self, project: &JsValue) -> JsValue {
		match self.run_delta(project) {
			Ok(d)		=> ok_delta(&d),
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

	/// Clears the source map before returning either way, as [`Self::run`] does, and steps the delta state:
	/// computes the changed-only delta of this compile against the last `order` and stores the new `order`
	/// and version. Only the ids are kept; the rendered SVG is emitted into the delta and dropped.
	fn run_delta(&mut self, project: &JsValue) -> Result<delta::PageDelta, String> {
		let main = string_field(project, "main").unwrap_or_else(|| "/main.typ".to_string());
		let outcome = self.run_delta_inner(project, &main);
		let _ = vfs::clear();
		match outcome {
			Ok(d)	=> Ok(d),
			Err(e)	=> Err(fmt!("{}:0: {}", main, e)),
		}
	}

	fn run_delta_inner(&mut self, project: &JsValue, main: &str) -> Outcome<delta::PageDelta> {
		let rendered	= res!(self.assemble_and_run(project, main));
		let d			= res!(delta::compute(&rendered.out.pages, &self.last_order, self.version));
		// Retain the id sequence and version alone -- never the SVG -- so the next compile diffs against it.
		self.last_order	= d.order.clone();
		self.version	= d.version;
		Ok(d)
	}

	/// Assembles, authors, runs, decorates and mirror-shifts a project through the shared pipeline -- the
	/// same code the native binary drives, so the two surfaces cannot drift -- and keeps the resolved ledger
	/// and heading table for a later section-rail query. The lone-file path takes this instance's once-built
	/// reading set rather than rebuilding it; the base directory for `/assets/...` figures is set inside
	/// `assemble`. The refusal table and skip line are not surfaced by this lane, so they are discarded. The
	/// caller clears the source map (through [`Self::run`] or [`Self::run_delta`]) once it has the result.
	fn assemble_and_run(&mut self, project: &JsValue, main: &str) -> Outcome<compile::Rendered> {
		let fonts = match &self.fonts {
			Some(f)	=> f.clone(),
			None	=> return Err(err!("The embedded font set could not be built."; Init, Missing)),
		};

		// Every source, asset and font becomes a source-map entry; the main path names the root among them.
		let mut files: HashMap<PathBuf, Vec<u8>> = HashMap::new();
		read_text_pairs(project, "sources", &mut files);
		read_byte_pairs(project, "assets", &mut files);
		read_byte_pairs(project, "fonts", &mut files);
		let main_path = PathBuf::from(main);
		if !files.contains_key(&main_path) {
			return Err(err!("The project has no source for its main file {:?}.", main; Input, Missing));
		}
		res!(vfs::install(files));

		let (assembled, _refusals, _skip)	= res!(compile::assemble(&main_path, || Ok(fonts.clone())));
		let rendered						= res!(compile::author_and_run(assembled));

		// Keep the resolved ledger and heading table for a later section-rail query.
		self.last_ledger	= Some(rendered.out.ledger.clone());
		self.last_heads		= rendered.heads.clone();
		Ok(rendered)
	}

	fn run_inner(&mut self, project: &JsValue, main: &str, mode: Mode) -> Outcome<Product> {
		let compile::Rendered { mut out, heads, geom: _ } = res!(self.assemble_and_run(project, main));

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

/// `{ version, order: string[], changed: [{ id, svg }], reset }` -- the changed-only delta. Ids (page
/// content hashes) are decimal strings, since a JavaScript number holds only 53 bits exactly and would
/// silently corrupt a 64-bit hash; the consumer treats them as opaque keys.
fn ok_delta(d: &delta::PageDelta) -> JsValue {
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
	obj.into()
}

/// `{ error: String }` -- the shape every method returns on failure, so a caller never sees a throw.
fn err_obj(msg: &str) -> JsValue {
	let obj = js_sys::Object::new();
	let _ = js_sys::Reflect::set(&obj, &JsValue::from_str("error"), &JsValue::from_str(msg));
	obj.into()
}
