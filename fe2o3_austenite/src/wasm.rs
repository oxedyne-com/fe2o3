//! The wasm-bindgen compile surface, shaped to Daimond's Typst-wasm contract so the app can swap the Typst
//! compiler for Austenite without rewiring its callers.
//!
//! This file is JavaScript interop and nothing else: a project object becomes a [`door::Project`], a
//! [`door::Instance`] answers, and the answer becomes a JavaScript value. Every compile, strict decision,
//! position and query lives in [`crate::door`], where a native test drives it. Nothing here touches a
//! filesystem, a font service or a package registry: every source, asset and font a document needs is
//! injected as bytes, and a package is supplied by the host with `supplyPackage`.
//!
//! Every method resolves; none rejects or throws. A failure returns `{ error: "<file>:<line>:<col>:
//! <message>", diagnostics, skipped, needs }`, `0:0` where the cause could not be traced to a source
//! line, so a caller composes its diagnostics from a value it always receives. Columns are 1-based
//! UTF-16 code units. A project carrying `strict: true` turns a result that was not set as written into
//! such a failure, so a partial PDF never reads as success.
//!
//! **The font pack.** The browser build (`--no-default-features --features wasm`) embeds no fonts. The host
//! supplies the font pack, the twelve files of the crate's `fonts/` directory (`fonts::Embedded::BOOK`, each
//! named by `Embedded::file`), in the project's `fonts` as `[[name, bytes], ...]`, as it supplies any font. A
//! compile that reaches a pack family it was not given fails with a `missing_font` error and `needs` naming
//! `font:Libertinus Serif`, `font:New Computer Modern Math` or `font:DejaVu Sans Mono`. The first such
//! failure may name one family only, since an equation that cannot be set ends the compile before the text
//! asks, so a host answers any `font:` need by supplying the whole pack. A native build embeds the pack
//! (feature `embed-fonts`, on by default) and never asks.
//!
//! **Holding only the pages on screen.** `compileProjectDelta` keeps the laid-out pages of the compile it
//! answered (their frames, not their SVG), and `renderPage` draws any one of them on request, byte for byte
//! as the delta would have sent it. A consumer therefore holds the SVG of only the pages it shows: it sends
//! `known` as the ids it still holds and `want: [from, to]` as the span of `order` it shows, and the delta
//! sends the SVG of the changed pages inside that span alone, with the whole `order` as ever. A page that
//! scrolls into view and is not held is fetched with `renderPage(id)`. Beside `order` the delta sends `sizes`,
//! one `[w, h]` a page in the same order and in the units of the page's SVG `viewBox` (whole points), window
//! or none, so a page not yet fetched is laid out at its true size from the first frame. The kept pages are replaced by the
//! next successful delta and by nothing else, so a failed compile leaves the last good pages drawable.

use crate::caches::Counters;
use crate::compile::{
	self,
	Diagnostic,
	Report,
};
use crate::delta::{
	Changed,
	Head,
	PageAt,
	PageMiss,
	Window,
};
use crate::door::{
	self,
	Failure,
	Instance,
	Made,
	Project,
	Row,
};
use crate::eval::package;

use oxedyne_fe2o3_core::prelude::*;

use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;

/// A long-lived Austenite compiler for the browser: the embedded faces parsed once, and the last good
/// compile's introspector kept for [`DaimondTypst::query_project`].
#[wasm_bindgen]
pub struct DaimondTypst {
	inst:	Instance,
}

#[wasm_bindgen]
impl DaimondTypst {
	/// Builds a compiler instance, parsing the embedded faces once. Infallible by contract: a face that
	/// will not parse is reported at compile time, never thrown here.
	#[wasm_bindgen(constructor)]
	pub fn new() -> DaimondTypst {
		DaimondTypst { inst: Instance::new() }
	}

	/// Compiles a project to a single PDF: `{ pdf: Uint8Array, pages, diagnostics: [{ file, line, col,
	/// message, severity, kind, hint }], skipped: string | null, needs: string[] }` on success, `{ error,
	/// diagnostics, skipped, needs }` otherwise. `project` is `{ main, sources: [[path, text], ...],
	/// assets: [[path, bytes], ...], fonts: [[name, bytes], ...], strict?: bool }`; every entry is
	/// injected into the source map and nothing outside it is read. `diagnostics` lists every site not
	/// set as written, and `skipped` the constructs passed over among them. Under `strict` an error, a
	/// warning of a refusing kind, zero pages or a main that sets no content is returned as `{ error }`
	/// instead of a PDF; a `lint` or `limit` warning, or a document still moving after the fifth pass,
	/// stands beside the PDF. `needs` lists the packages an import asked for that were not supplied. A
	/// success also carries `counters`, as [`Self::set_cache_budget`] describes.
	#[wasm_bindgen(js_name = compileProject)]
	pub fn compile_project(&mut self, project: &JsValue) -> JsValue {
		match self.inst.compile_pdf(&project_of(project)) {
			Ok(made)	=> ok_pdf(&made),
			Err(fail)	=> err_obj(&fail),
		}
	}

	/// The fast live-view path: compiles a project to per-page SVG (already glyph outlines), `{ svg:
	/// string[], pages, diagnostics, skipped, needs }` on success or `{ error, diagnostics, skipped, needs
	/// }` otherwise, `strict` as for [`Self::compile_project`]. A success also carries `counters`.
	#[wasm_bindgen(js_name = compileProjectVector)]
	pub fn compile_project_vector(&mut self, project: &JsValue) -> JsValue {
		match self.inst.compile_svg(&project_of(project)) {
			Ok(made)	=> ok_svg(&made),
			Err(fail)	=> err_obj(&fail),
		}
	}

	/// The live-view path Daimond consumes: compiles a project to a changed-only page delta, returning
	/// `{ version, order: string[], sizes: [w, h][], changed: [{ id, svg }], reset, pages, diagnostics,
	/// skipped, needs }` on success or `{ error, diagnostics, skipped, needs }` otherwise, `strict` as for
	/// [`Self::compile_project`]. The project carries `known: string[]`, the ids the consumer still holds
	/// in its own SVG cache; only the pages whose id is not among them carry their SVG in `changed`. A
	/// consumer that has cleared its cache sends `known: []` and gets a full resend (`reset: true`). The
	/// `version` is this instance's tick, stepped by each delta and left where it was by a refusal. See
	/// [`crate::delta`] for the shape. Ids are opaque decimal strings, since a JavaScript number cannot
	/// hold every 64-bit hash exactly. A success also carries `counters`.
	///
	/// The project may carry `want: [from, to]`, whole numbers, the span `order.slice(from, to)` the consumer
	/// shows: then `changed` carries only the changed pages inside it, and the consumer fetches any other
	/// with [`Self::render_page`] when it comes into view. With no `want`, or one that is not two whole
	/// numbers in order, every changed page is sent, as before `want` existed.
	#[wasm_bindgen(js_name = compileProjectDelta)]
	pub fn compile_project_delta(&mut self, project: &JsValue) -> JsValue {
		match self.inst.compile_delta_into(&project_of(project), JsChanged::default()) {
			Ok(made)	=> ok_delta(&made),
			Err(fail)	=> err_obj(&fail),
		}
	}

	/// One page of the last delta, drawn from the pages it kept: `{ id, index, version, svg }`, the SVG
	/// byte for byte what the delta sends for that id. `page` is an id from `order` (a decimal string) or an
	/// index into `order` (a whole number from 0). The answer is `{ error, code, version }` when there is no
	/// such page: `code` is `"uncompiled"` before any delta, `"gone"` for an id the last delta's `order`
	/// does not hold, `"range"` for an index past its end, and `"input"` for a `page` of another kind;
	/// `version` is the last delta's, `0` before any. A fault in drawing gives `{ error, code: "internal",
	/// diagnostics, skipped, needs }`.
	#[wasm_bindgen(js_name = renderPage)]
	pub fn render_page(&self, page: &JsValue) -> JsValue {
		let version = self.inst.kept().map(|k| k.version()).unwrap_or(0);
		let at = if let Some(s) = page.as_string() {
			s.parse::<u64>().ok().map(PageAt::Id)
		} else {
			match page.as_f64() {
				Some(n) if n >= 0.0 && n.fract() == 0.0 && n <= 9_007_199_254_740_991.0	=> Some(PageAt::Index(n as usize)),
				_																		=> None,
			}
		};
		let obj = js_sys::Object::new();
		let miss = |obj: &js_sys::Object, code: &str, message: &str| {
			set(obj, "error", &JsValue::from_str(message));
			set(obj, "code", &JsValue::from_str(code));
			set(obj, "version", &JsValue::from_f64(version as f64));
		};
		let at = match at {
			Some(a)	=> a,
			None	=> {
				miss(&obj, "input", "a page is an id from order, as a decimal string, or an index into order from 0");
				return obj.into();
			},
		};
		match self.inst.page(at) {
			Ok(Ok((id, index, svg)))	=> {
				set(&obj, "id", &JsValue::from_str(&fmt!("{}", id)));
				set(&obj, "index", &JsValue::from_f64(index as f64));
				set(&obj, "version", &JsValue::from_f64(version as f64));
				set(&obj, "svg", &JsValue::from_str(&svg));
			},
			Ok(Err(m))					=> miss(&obj, m.code(), match m {
				PageMiss::Uncompiled	=> "no delta has been compiled",
				PageMiss::Gone			=> "the last delta has no page with that id",
				PageMiss::Range			=> "the index is past the last page of the last delta",
			}),
			Err(fail)					=> {
				let out = err_obj(&fail);
				let _ = js_sys::Reflect::set(&out, &JsValue::from_str("code"), &JsValue::from_str("internal"));
				return out;
			},
		}
		obj.into()
	}

	/// The font families a compile of `project` can set by name, as a sorted `string[]`: the embedded
	/// families and the family each face of each `project.fonts` file declares, one per face of a
	/// collection. The list is read from the font book `font:` resolves against, so a family is listed
	/// exactly when a compile finds it. `project` is optional.
	#[wasm_bindgen(js_name = fontFamilies)]
	pub fn font_families(&self, project: &JsValue) -> JsValue {
		let p = if project.is_undefined() || project.is_null() { None } else { Some(project_of(project)) };
		let arr = js_sys::Array::new();
		for f in &self.inst.font_families(p.as_ref()) {
			arr.push(&JsValue::from_str(f));
		}
		arr.into()
	}

	/// The engine's identity, `{ engine: "austenite", version, git, typst }`: the crate version, the commit
	/// it was built from (`<12 hex>`, `-dirty` when built from uncommitted changes, or `unknown`) and the
	/// Typst version the evaluator follows.
	#[wasm_bindgen(js_name = engineInfo)]
	pub fn engine_info(&self) -> JsValue {
		let obj = js_sys::Object::new();
		set(&obj, "engine", &JsValue::from_str("austenite"));
		set(&obj, "version", &JsValue::from_str(compile::engine_version()));
		set(&obj, "git", &JsValue::from_str(compile::engine_git_hash()));
		set(&obj, "typst", &JsValue::from_str(&package::version_text(package::TYPST_VERSION)));
		obj.into()
	}

	/// Sets the loop iterations a compile may spend for each layout pass, the host's bound on a compile it
	/// cannot stop: a whole number, or `null` for no bound, by which the host declares it can stop a
	/// compile itself. A new instance holds 10,000,000. Returns `{ loopBudget }`, the budget now in force,
	/// a number or `null`, or `{ error }` and no change for any other value. A project is document data
	/// and cannot set it.
	#[wasm_bindgen(js_name = setLoopBudget)]
	pub fn set_loop_budget(&mut self, budget: &JsValue) -> JsValue {
		let obj = js_sys::Object::new();
		let parsed = if budget.is_null() {
			Some(None)
		} else {
			match budget.as_f64() {
				Some(n) if n >= 0.0 && n.fract() == 0.0 && n <= 9_007_199_254_740_991.0	=> Some(Some(n as u64)),
				_																		=> None,
			}
		};
		match parsed {
			Some(b)	=> {
				self.inst.set_loop_budget(b);
				set(&obj, "loopBudget", &match b {
					Some(n)	=> JsValue::from_f64(n as f64),
					None	=> JsValue::NULL,
				});
			},
			None	=> set(&obj, "error", &JsValue::from_str("the loop budget is a whole number of iterations, or null")),
		}
		obj.into()
	}

	/// Sets the bytes each cache of the session may hold: `{ shapes, pars, ledger }`, each a whole number
	/// of bytes or left out to stand. A new instance holds 32 MiB of shaped runs and 16 MiB of paragraphs,
	/// and the ledger's is set with the ledger. The shaped-run cache at its budget evicts its least recently used
	/// result, and a cache swept by generation refuses a new entry and loses none, so a lower budget slows a
	/// compile and changes nothing it makes. Returns `{ cacheBudget: { shapes,
	/// pars, ledger } }`, the budgets now in force, or `{ error }` and no change, naming the first field
	/// that is not a whole number of bytes. A project is document data and cannot set it.
	///
	/// A successful compile carries `counters: { replayed, relaid, parHits, parMisses, shapes: { entries,
	/// bytes, budget, hits, misses, evictions } }`: the pages replayed from the ledger and laid again, the
	/// paragraph cache's hits and misses, and the shaped-run cache as the compile left it (its hits,
	/// misses and evictions are cumulative over the session's book). The page and paragraph counts are zero
	/// until those caches exist.
	#[wasm_bindgen(js_name = setCacheBudget)]
	pub fn set_cache_budget(&mut self, budgets: &JsValue) -> JsValue {
		let obj = js_sys::Object::new();
		let field = |name: &str| -> Option<f64> {
			let v = js_sys::Reflect::get(budgets, &JsValue::from_str(name)).unwrap_or(JsValue::UNDEFINED);
			if v.is_undefined() {
				None
			} else {
				Some(v.as_f64().unwrap_or(f64::NAN))
			}
		};
		match self.inst.cache_budget().with(field("shapes"), field("pars"), field("ledger")) {
			Ok(b)	=> {
				self.inst.set_cache_budget(b);
				let now = js_sys::Object::new();
				set(&now, "shapes",	&JsValue::from_f64(b.shapes as f64));
				set(&now, "pars",	&JsValue::from_f64(b.pars as f64));
				set(&now, "ledger",	&JsValue::from_f64(b.ledger as f64));
				set(&obj, "cacheBudget", &now);
			},
			Err(e)	=> set(&obj, "error", &JsValue::from_str(&fmt!("{}", e))),
		}
		obj.into()
	}

	/// Compiles a single source string to PDF, wrapping it as the project's `/main.typ`.
	#[wasm_bindgen]
	pub fn compile(&mut self, source: &str) -> JsValue {
		match self.inst.compile_pdf(&Project::single(source)) {
			Ok(made)	=> ok_pdf(&made),
			Err(fail)	=> err_obj(&fail),
		}
	}

	/// What a Typst selector finds in the last good compile, as a JSON array of rows `{ kind, label, page,
	/// value }`, a heading's also `title` and `level`. `selector` is evaluated as Typst code, as `typst
	/// query` evaluates it. `field` names the field each row's `value` carries, `value` when empty. `page`
	/// is 1-based; a heading's `title` is its plain text. `null` when nothing has compiled, the last
	/// compile failed, the selector does not evaluate, or a field cannot be written as Typst writes it.
	#[wasm_bindgen(js_name = queryProject)]
	pub fn query_project(&self, selector: &str, field: &str) -> JsValue {
		match self.inst.query(selector, field) {
			Some(rows)	=> rows_js(&rows),
			None		=> JsValue::NULL,
		}
	}

	/// Hands a package to the engine: `spec` is `@namespace/name:version` and `files` is `[[path,
	/// bytes | string], ...]`, paths relative to the package. Returns `{ spec, files }`, the count of
	/// files held, or `{ error }`. A version is immutable: other files for one already supplied are an
	/// error until it is withdrawn. Archives are not read: the host unpacks them.
	#[wasm_bindgen(js_name = supplyPackage)]
	pub fn supply_package(&mut self, spec: &str, files: &JsValue) -> JsValue {
		let mut held: Vec<(String, Vec<u8>)> = Vec::new();
		if let Ok(arr) = files.clone().dyn_into::<js_sys::Array>() {
			for entry in arr.iter() {
				if let Ok(pair) = entry.dyn_into::<js_sys::Array>() {
					if let Some(path) = pair.get(0).as_string() {
						let v = pair.get(1);
						let bytes = match v.as_string() {
							Some(text)	=> Some(text.into_bytes()),
							None		=> to_bytes(&v),
						};
						if let Some(b) = bytes {
							held.push((path, b));
						}
					}
				}
			}
		}
		let obj = js_sys::Object::new();
		match door::supply_package(spec, held) {
			Ok(n)	=> {
				set(&obj, "spec", &JsValue::from_str(spec));
				set(&obj, "files", &JsValue::from_f64(n as f64));
			},
			Err(e)	=> set(&obj, "error", &JsValue::from_str(&e.plain())),
		}
		obj.into()
	}

	/// The packages supplied, as a `string[]` of `@namespace/name:version`.
	#[wasm_bindgen]
	pub fn packages(&self) -> JsValue {
		let arr = js_sys::Array::new();
		if let Ok(list) = door::packages() {
			for s in &list {
				arr.push(&JsValue::from_str(s));
			}
		}
		arr.into()
	}

	/// Withdraws a supplied package: `{ spec, withdrawn }`, false when none was held, or `{ error }`.
	#[wasm_bindgen(js_name = withdrawPackage)]
	pub fn withdraw_package(&mut self, spec: &str) -> JsValue {
		let obj = js_sys::Object::new();
		match door::withdraw_package(spec) {
			Ok(b)	=> {
				set(&obj, "spec", &JsValue::from_str(spec));
				set(&obj, "withdrawn", &JsValue::from_bool(b));
			},
			Err(e)	=> set(&obj, "error", &JsValue::from_str(&e.plain())),
		}
		obj.into()
	}

	/// The compiler's current wasm linear-memory size, in megabytes: a heap-usage proxy for the app's
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

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ JS INTEROP                                                                 │
// └───────────────────────────────────────────────────────────────────────────┘

/// Reads a project object into the door's own type; a field that is absent or of the wrong shape is empty.
fn project_of(v: &JsValue) -> Project {
	// Only the fields a project has are read: its loop budget is the host's, set on the instance.
	let mut p = Project {
		main:		string_field(v, "main").unwrap_or_default(),
		strict:		bool_field(v, "strict"),
		known:		parse_known(v),
		want:		parse_want(v),
		..Project::default()
	};
	for (path, text) in text_pairs(v, "sources") {
		p.sources.push((path, text));
	}
	p.assets	= byte_pairs(v, "assets");
	p.fonts		= byte_pairs(v, "fonts");
	p
}

/// A `[[path, text], ...]` field.
fn text_pairs(project: &JsValue, key: &str) -> Vec<(String, String)> {
	let mut out = Vec::new();
	if let Some(arr) = array_field(project, key) {
		for entry in arr.iter() {
			if let Ok(pair) = entry.dyn_into::<js_sys::Array>() {
				if let (Some(path), Some(text)) = (pair.get(0).as_string(), pair.get(1).as_string()) {
					out.push((path, text));
				}
			}
		}
	}
	out
}

/// A `[[path, bytes], ...]` field, each value a `Uint8Array` or `ArrayBuffer`.
fn byte_pairs(project: &JsValue, key: &str) -> Vec<(String, Vec<u8>)> {
	let mut out = Vec::new();
	if let Some(arr) = array_field(project, key) {
		for entry in arr.iter() {
			if let Ok(pair) = entry.dyn_into::<js_sys::Array>() {
				if let Some(path) = pair.get(0).as_string() {
					if let Some(bytes) = to_bytes(&pair.get(1)) {
						out.push((path, bytes));
					}
				}
			}
		}
	}
	out
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
/// string, is skipped; an empty result forces a full reset, a resend over a blank.
fn parse_known(project: &JsValue) -> Vec<u64> {
	let mut ids = Vec::new();
	if let Some(arr) = array_field(project, "known") {
		for entry in arr.iter() {
			if let Some(s) = entry.as_string() {
				if let Ok(id) = s.parse::<u64>() {
					ids.push(id);
				}
			}
		}
	}
	ids
}

/// The span of `order` the consumer shows, from the project's `want: [from, to]`: two whole numbers,
/// `from` not past `to`. Anything else is no window, and every changed page is sent.
fn parse_want(project: &JsValue) -> Option<Window> {
	let arr = match array_field(project, "want") {
		Some(a) if a.length() == 2	=> a,
		_							=> return None,
	};
	let whole = |v: JsValue| match v.as_f64() {
		Some(n) if n >= 0.0 && n.fract() == 0.0 && n <= 9_007_199_254_740_991.0	=> Some(n as usize),
		_																		=> None,
	};
	match (whole(arr.get(0)), whole(arr.get(1))) {
		(Some(from), Some(to)) if from <= to	=> Some(Window { from, to }),
		_										=> None,
	}
}

/// Sets one property; a failed set on a fresh plain object cannot happen, so it is not reported.
fn set(obj: &js_sys::Object, key: &str, val: &JsValue) {
	let _ = js_sys::Reflect::set(obj, &JsValue::from_str(key), val);
}

fn strings_js(list: &[String]) -> js_sys::Array {
	let arr = js_sys::Array::new();
	for s in list {
		arr.push(&JsValue::from_str(s));
	}
	arr
}

/// Sets `pages`, `diagnostics`, `skipped` (string or `null`) and `needs`, the fields every compile result
/// carries.
fn set_report(obj: &js_sys::Object, rep: &Report, needs: &[String]) {
	set(obj, "pages", &JsValue::from_f64(rep.pages as f64));
	set(obj, "diagnostics", &diagnostics_array(&rep.diagnostics));
	set(obj, "skipped", &opt_str(rep.skipped.as_deref()));
	set(obj, "needs", &strings_js(needs));
}

/// Sets `counters`, what the session's caches did in the compile.
fn set_counters(obj: &js_sys::Object, c: &Counters) {
	let shapes = js_sys::Object::new();
	set(&shapes, "entries",		&JsValue::from_f64(c.shapes.entries as f64));
	set(&shapes, "bytes",		&JsValue::from_f64(c.shapes.bytes as f64));
	set(&shapes, "budget",		&JsValue::from_f64(c.shapes.budget as f64));
	set(&shapes, "hits",		&JsValue::from_f64(c.shapes.hits as f64));
	set(&shapes, "misses",		&JsValue::from_f64(c.shapes.misses as f64));
	set(&shapes, "evictions",	&JsValue::from_f64(c.shapes.evictions as f64));
	let out = js_sys::Object::new();
	set(&out, "replayed",	&JsValue::from_f64(c.replayed as f64));
	set(&out, "relaid",		&JsValue::from_f64(c.relaid as f64));
	set(&out, "parHits",	&JsValue::from_f64(c.par_hits as f64));
	set(&out, "parMisses",	&JsValue::from_f64(c.par_misses as f64));
	set(&out, "shapes",		&shapes);
	set(obj, "counters", &out);
}

fn opt_str(s: Option<&str>) -> JsValue {
	match s {
		Some(s)	=> JsValue::from_str(s),
		None	=> JsValue::NULL,
	}
}

fn diagnostics_array(diags: &[Diagnostic]) -> js_sys::Array {
	let arr = js_sys::Array::new();
	for d in diags {
		let entry = js_sys::Object::new();
		set(&entry, "file",		&JsValue::from_str(&d.file));
		set(&entry, "line",		&JsValue::from_f64(d.line as f64));
		set(&entry, "col",		&JsValue::from_f64(d.col as f64));
		set(&entry, "message",	&JsValue::from_str(&d.message));
		set(&entry, "severity",	&JsValue::from_str(d.severity.as_str()));
		set(&entry, "kind",		&JsValue::from_str(d.kind.as_str()));
		set(&entry, "hint",		&opt_str(d.hint.as_deref()));
		arr.push(&entry);
	}
	arr
}

/// `{ pdf: Uint8Array, pages, diagnostics, skipped, needs, counters }`. The array is allocated at its final
/// length and the chunks are copied in one by one, so the file never exists twice in the wasm heap.
fn ok_pdf(made: &Made<crate::emit::sinks::Chunks>) -> JsValue {
	let obj = js_sys::Object::new();
	let pdf = js_sys::Uint8Array::new_with_length(made.product.len() as u32);
	let mut at = 0u32;
	for chunk in made.product.chunks() {
		let end = at + chunk.len() as u32;
		pdf.subarray(at, end).copy_from(chunk);
		at = end;
	}
	set(&obj, "pdf", &pdf);
	set_report(&obj, &made.report, &made.needs);
	set_counters(&obj, &made.counters);
	obj.into()
}

/// `{ svg: string[], pages, diagnostics, skipped, needs, counters }`.
fn ok_svg(made: &Made<Vec<String>>) -> JsValue {
	let obj = js_sys::Object::new();
	set(&obj, "svg", &strings_js(&made.product));
	set_report(&obj, &made.report, &made.needs);
	set_counters(&obj, &made.counters);
	obj.into()
}

/// The delta's `changed` array, built as the pages are drawn: each page's SVG goes into a JavaScript string
/// the moment it is rendered and the Rust copy is dropped, so wasm memory never holds more than one page's SVG.
#[derive(Default)]
struct JsChanged {
	arr:	js_sys::Array,
}

impl Changed for JsChanged {
	fn take(&mut self, id: u64, svg: String) -> Outcome<()> {
		let entry = js_sys::Object::new();
		set(&entry, "id", &JsValue::from_str(&fmt!("{}", id)));
		set(&entry, "svg", &JsValue::from_str(&svg));
		self.arr.push(&entry);
		Ok(())
	}
}

/// `{ version, order: string[], sizes: [w, h][], changed: [{ id, svg }], reset, pages, diagnostics, skipped,
/// needs, counters }`.
/// Ids (page content hashes) are decimal strings, since a JavaScript number holds only 53 bits exactly and
/// would silently corrupt a 64-bit hash; the consumer treats them as opaque keys.
fn ok_delta(made: &Made<(Head, JsChanged)>) -> JsValue {
	let (head, changed) = &made.product;
	let obj = js_sys::Object::new();
	set(&obj, "version", &JsValue::from_f64(head.version as f64));
	let order = js_sys::Array::new();
	for id in &head.order {
		order.push(&JsValue::from_str(&fmt!("{}", id)));
	}
	set(&obj, "order", &order);
	let sizes = js_sys::Array::new();
	for (w, h) in &head.sizes {
		let pair = js_sys::Array::new();
		pair.push(&JsValue::from_f64(*w as f64));
		pair.push(&JsValue::from_f64(*h as f64));
		sizes.push(&pair);
	}
	set(&obj, "sizes", &sizes);
	set(&obj, "changed", &changed.arr);
	set(&obj, "reset", &JsValue::from_bool(head.reset));
	set_report(&obj, &made.report, &made.needs);
	set_counters(&obj, &made.counters);
	obj.into()
}

/// `{ error: "file:line:col: message", diagnostics, skipped, needs }`, and `pages` when the compile got as
/// far as laying pages out: the shape every compile returns on failure, so a caller never sees a throw.
/// `diagnostics` leads with the error's own entry, then every site the compile reached.
fn err_obj(fail: &Failure) -> JsValue {
	let obj = js_sys::Object::new();
	set(&obj, "error", &JsValue::from_str(&fmt!("{}", fail.head)));
	let mut diags = vec![fail.head.clone()];
	diags.extend(fail.rest.iter().cloned());
	if let Some(pages) = fail.pages {
		set(&obj, "pages", &JsValue::from_f64(pages as f64));
	}
	set(&obj, "diagnostics", &diagnostics_array(&diags));
	set(&obj, "skipped", &opt_str(fail.skipped.as_deref()));
	set(&obj, "needs", &strings_js(&fail.needs));
	obj.into()
}

/// The rows of a query as a JavaScript array. A row's `value` is parsed from its JSON text, `null` where the
/// element has no such field.
fn rows_js(rows: &[Row]) -> JsValue {
	let arr = js_sys::Array::new();
	for r in rows {
		let entry = js_sys::Object::new();
		set(&entry, "kind", &JsValue::from_str(&r.kind));
		set(&entry, "label", &opt_str(r.label.as_deref()));
		set(&entry, "page", &match r.page {
			Some(p)	=> JsValue::from_f64(p as f64),
			None	=> JsValue::NULL,
		});
		let value = match &r.value {
			Some(text)	=> js_sys::JSON::parse(text).unwrap_or(JsValue::NULL),
			None		=> JsValue::NULL,
		};
		set(&entry, "value", &value);
		if let Some(t) = &r.title {
			set(&entry, "title", &JsValue::from_str(t));
		}
		if let Some(l) = r.level {
			set(&entry, "level", &JsValue::from_f64(l as f64));
		}
		arr.push(&entry);
	}
	arr.into()
}
