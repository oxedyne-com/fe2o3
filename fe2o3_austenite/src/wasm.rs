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

use crate::compile::{
	self,
	Diagnostic,
	Report,
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
	/// stands beside the PDF. `needs` lists the packages an import asked for that were not supplied.
	#[wasm_bindgen(js_name = compileProject)]
	pub fn compile_project(&mut self, project: &JsValue) -> JsValue {
		match self.inst.compile_pdf(&project_of(project)) {
			Ok(made)	=> ok_pdf(&made),
			Err(fail)	=> err_obj(&fail),
		}
	}

	/// The fast live-view path: compiles a project to per-page SVG (already glyph outlines), `{ svg:
	/// string[], pages, diagnostics, skipped, needs }` on success or `{ error, diagnostics, skipped, needs
	/// }` otherwise, `strict` as for [`Self::compile_project`].
	#[wasm_bindgen(js_name = compileProjectVector)]
	pub fn compile_project_vector(&mut self, project: &JsValue) -> JsValue {
		match self.inst.compile_svg(&project_of(project)) {
			Ok(made)	=> ok_svg(&made),
			Err(fail)	=> err_obj(&fail),
		}
	}

	/// The live-view path Daimond consumes: compiles a project to a changed-only page delta, returning
	/// `{ version, order: string[], changed: [{ id, svg }], reset, pages, diagnostics, skipped, needs }`
	/// on success or `{ error, diagnostics, skipped, needs }` otherwise, `strict` as for
	/// [`Self::compile_project`]. The project carries `known: string[]`, the ids the consumer still holds
	/// in its own SVG cache; only the pages whose id is not among them carry their SVG in `changed`. A
	/// consumer that has cleared its cache sends `known: []` and gets a full resend (`reset: true`). The
	/// `version` is this instance's tick, stepped by each delta and left where it was by a refusal. See
	/// [`crate::delta`] for the shape. Ids are opaque decimal strings, since a JavaScript number cannot
	/// hold every 64-bit hash exactly.
	#[wasm_bindgen(js_name = compileProjectDelta)]
	pub fn compile_project_delta(&mut self, project: &JsValue) -> JsValue {
		match self.inst.compile_delta(&project_of(project)) {
			Ok(made)	=> ok_delta(&made),
			Err(fail)	=> err_obj(&fail),
		}
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
	let mut p = Project {
		main:		string_field(v, "main").unwrap_or_default(),
		strict:		bool_field(v, "strict"),
		known:		parse_known(v),
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

/// `{ pdf: Uint8Array, pages, diagnostics, skipped, needs }`. The array is allocated at its final length
/// and the chunks are copied in one by one, so the file never exists twice in the wasm heap.
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
	obj.into()
}

/// `{ svg: string[], pages, diagnostics, skipped, needs }`.
fn ok_svg(made: &Made<Vec<String>>) -> JsValue {
	let obj = js_sys::Object::new();
	set(&obj, "svg", &strings_js(&made.product));
	set_report(&obj, &made.report, &made.needs);
	obj.into()
}

/// `{ version, order: string[], changed: [{ id, svg }], reset, pages, diagnostics, skipped, needs }`. Ids
/// (page content hashes) are decimal strings, since a JavaScript number holds only 53 bits exactly and
/// would silently corrupt a 64-bit hash; the consumer treats them as opaque keys.
fn ok_delta(made: &Made<crate::delta::PageDelta>) -> JsValue {
	let d = &made.product;
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
	set_report(&obj, &made.report, &made.needs);
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
