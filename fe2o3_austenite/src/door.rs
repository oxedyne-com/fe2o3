//! The door: every compile the wasm surface offers, as plain Rust, so that a native test drives exactly what
//! the browser calls. [`crate::wasm`] holds only the translation between JavaScript values and these types.
//!
//! The evaluator serves every door, the live view's changed-only delta included. A project's files are sealed
//! into the source map as the browser has them, so nothing the project does not hold resolves from the real
//! filesystem, whatever the host. The source map, the image base directory and the package store are process
//! globals, so a call takes one turn at a time. Columns on this surface count UTF-16 code units, as JavaScript
//! counts them, and start at 1.

use crate::caches::{
	Budgets,
	Caches,
	Counters,
};
use crate::compile::{
	Cols,
	Diagnostic,
	Report,
	Session,
	Severity,
};
use crate::delta::{
	self,
	Changed,
};
use crate::diag::DiagnosticKind;
use crate::emit::sinks::{
	Chunks,
	DeltaSink,
	PdfSink,
	VectorSink,
};
use crate::eval::{
	Engine,
	World,
};
use crate::eval::eval::{
	eval_string,
	EvalMode,
};
use crate::eval::fixpoint::PageSink;
use crate::eval::lib::data;
use crate::eval::package::{
	self,
	PackageSpec,
};
use crate::eval::scope::Scope;
use crate::eval::select;
use crate::eval::value::Value;
use crate::flow::text::FontStore;
use crate::fonts::FontBook;
use crate::syntax::Span;
use crate::vfs;

use oxedyne_fe2o3_core::prelude::*;

use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{
	Hash,
	Hasher,
};
use std::path::{
	Path,
	PathBuf,
};
use std::sync::{
	Arc,
	Mutex,
	MutexGuard,
};

// One call at a time: the source map, the image base directory and the package store are process globals.
static TURN: Mutex<()> = Mutex::new(());

/// Takes this call's turn; a turn a panicked call poisoned is still a turn.
fn turn() -> MutexGuard<'static, ()> {
	TURN.lock().unwrap_or_else(|p| p.into_inner())
}

/// The path a project's main file takes when it names none.
pub const MAIN: &str = "/main.typ";

/// The loop iterations a compile may spend, per layout pass, before the door refuses it. The host's own
/// thread cannot stop a compile that has begun, so the door bounds the loops of one unless the host says
/// otherwise with [`Instance::set_loop_budget`]. The command line has no such bound, as Typst has none.
pub const LOOP_BUDGET: u64 = 10_000_000;

/// What a call is given: the project's files by path in its own shadow file system, as Daimond sends them.
#[derive(Clone, Debug, Default)]
pub struct Project {
	pub main:		String,					// empty for `/main.typ`
	pub sources:	Vec<(String, String)>,	// path, text
	pub assets:		Vec<(String, Vec<u8>)>,
	pub fonts:		Vec<(String, Vec<u8>)>,	// the name is not read: the face declares its own family
	pub strict:		bool,
	pub known:		Vec<u64>,				// the delta's page ids the consumer already holds
}

impl Project {
	/// A one-file project, `/main.typ`, not strict.
	pub fn single(source: &str) -> Self {
		Self { sources: vec![(MAIN.to_string(), source.to_string())], ..Self::default() }
	}

	fn main_path(&self) -> PathBuf {
		PathBuf::from(if self.main.is_empty() { MAIN } else { self.main.as_str() })
	}
}

/// A compile that produced its artefact: the artefact, every site not set as written, the packages the
/// project asked for that nobody supplied, and what the session's caches did.
pub struct Made<T> {
	pub product:	T,
	pub report:		Report,
	pub needs:		Vec<String>,
	pub counters:	Counters,
}

/// A compile that did not: the error's own site first, then the sites that follow it.
#[derive(Debug)]
pub struct Failure {
	pub head:		Diagnostic,
	pub rest:		Vec<Diagnostic>,
	pub pages:		Option<usize>,	// set when the compile got as far as laying pages out
	pub skipped:	Option<String>,
	pub needs:		Vec<String>,
}

impl Failure {
	/// An error with no report behind it, placed at the site it was raised with, else at `0:0` in `main`.
	fn of(e: &Error<ErrTag>, main: &Path) -> Self {
		Self {
			head:		Diagnostic::from_error(e, main),
			rest:		Vec::new(),
			pages:		None,
			skipped:	None,
			needs:		Vec::new(),
		}
	}

	/// A fault of the engine's own, at `0:0` in `main`.
	fn internal(main: &Path, message: &str) -> Self {
		Self {
			head:		Diagnostic {
				file:		main.display().to_string(),
				line:		0,
				col:		0,
				message:	fmt!("internal: {}", message),
				severity:	Severity::Error,
				kind:		DiagnosticKind::Internal,
				hint:		None,
			},
			rest:		Vec::new(),
			pages:		None,
			skipped:	None,
			needs:		Vec::new(),
		}
	}

	/// A strict refusal: its head restates the first site, then every site follows.
	fn refused(head: Diagnostic, report: &Report, needs: Vec<String>) -> Self {
		Self {
			head,
			rest:		report.diagnostics.clone(),
			pages:		Some(report.pages),
			skipped:	report.skipped.clone(),
			needs,
		}
	}
}

/// One element a query found.
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
	pub kind:	String,				// the element's function: `heading`, `figure`, `metadata`
	pub label:	Option<String>,
	pub page:	Option<u32>,		// 1-based, from the introspector
	pub value:	Option<String>,		// JSON text of the field asked for, none where the element has none
	pub title:	Option<String>,		// a heading's plain text
	pub level:	Option<i64>,		// a heading's depth
}

/// A long-lived compiler: a [`Session`] holding the embedded faces parsed once and the introspector the
/// next compile starts from, whether the last compile answers queries, and the delta's version tick.
pub struct Instance {
	session:	Session,
	fonts:		Vec<u64>,					// the fingerprints of the project fonts the session's store holds
	good:		bool,						// the last compile laid pages without a refusal, so queries answer
	version:	u32,						// the delta's tick
	budget:		Option<u64>,				// the loop budget of a compile, none for no bound
}

// A font file's fingerprint, to tell a project's fonts unchanged from one call to the next.
fn fingerprint(bytes: &[u8]) -> u64 {
	let mut h = DefaultHasher::new();
	bytes.hash(&mut h);
	h.finish()
}

impl Default for Instance {
	fn default() -> Self { Self::new() }
}

impl Instance {
	/// Parses the embedded faces once. A face that will not parse is reported by the first compile, never
	/// here, so a constructor cannot fail.
	pub fn new() -> Self {
		let fonts = match FontBook::embedded() {
			Ok(book)	=> FontStore::with_base(Arc::new(book)),
			Err(_)		=> FontStore::default(),
		};
		Self {
			session:	Session::new(fonts),
			fonts:		Vec::new(),
			good:		false,
			version:	0,
			budget:		Some(LOOP_BUDGET),
		}
	}

	/// Sets the loop iterations a compile may spend, per layout pass: a number, or `None` for no bound, by
	/// which the host declares it can stop a compile itself. It is the host's option, held here and never
	/// read from a project, which is document data and may arrive by sync.
	pub fn set_loop_budget(&mut self, budget: Option<u64>) {
		self.budget = budget;
	}

	/// The loop budget in force: [`LOOP_BUDGET`] until the host sets another.
	pub fn loop_budget(&self) -> Option<u64> { self.budget }

	/// Sets the bytes each of the session's caches may hold, from the next compile. Like the loop budget it
	/// is the host's option and never a project's: a lower one slows a compile and changes nothing it makes.
	/// It outlasts a panic, which drops the session's caches and keeps the budgets.
	pub fn set_cache_budget(&mut self, budgets: Budgets) {
		self.session.set_budgets(budgets);
	}

	/// The cache budgets in force: [`Budgets::default`] until the host sets others.
	pub fn cache_budget(&self) -> Budgets { self.session.budgets() }

	/// The session's caches, for the counters of the last compile and the configuration they were made under.
	pub fn caches(&self) -> &Caches { self.session.caches() }

	/// Compiles to PDF through the evaluator, cold, as an explicit output is: the file never depends on the
	/// compiles before it. The chunks are the file; a host copies them out one by one.
	pub fn compile_pdf(&mut self, p: &Project) -> Result<Made<Chunks>, Failure> {
		let sink = match PdfSink::new() {
			Ok(s)	=> s,
			Err(e)	=> return Err(Failure::of(&e, &p.main_path())),
		};
		self.run(p, sink, true, |s| s.into_output())
	}

	/// Compiles to one SVG document per page through the evaluator, from the kept introspector.
	pub fn compile_svg(&mut self, p: &Project) -> Result<Made<Vec<String>>, Failure> {
		self.run(p, VectorSink::default(), false, |s| s.into_pages())
	}

	fn run<S, T, F>(&mut self, p: &Project, mut sink: S, cold: bool, take: F) -> Result<Made<T>, Failure>
	where
		S: PageSink,
		F: FnOnce(S) -> Option<T>,
	{
		let _turn	= turn();
		let budget	= self.budget;
		let budgets	= self.session.budgets();	// read before the compile lends its caches, budgets included, to the engine
		let main	= p.main_path();
		// A failed compile answers no query, so the answer goes before the compile begins.
		self.good	= false;
		let base = match self.session.fonts().base() {
			Some(b)	=> b.clone(),
			None	=> return Err(Failure::internal(&main, "the embedded font set could not be built")),
		};
		if let Err(e) = install(p, &main) {
			let _ = vfs::clear();
			return Err(Failure::of(&e, &main));
		}
		// The store is kept while the project's fonts are the same, with its parsed faces and shaping cache,
		// and made afresh over the embedded faces when they are not, so that no project's fonts reach another's.
		let held: Vec<u64> = p.fonts.iter().map(|(_, bytes)| fingerprint(bytes)).collect();
		if held != self.fonts {
			let mut fonts = FontStore::with_base(base.clone());
			for (_, bytes) in &p.fonts {
				fonts.add_bytes(bytes.clone());
			}
			self.session.set_fonts(fonts);
			self.fonts = held;
		}
		let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
			self.session.compile(&main, Path::new("/"), &mut sink, None, budget, cold)
		}));
		let _ = vfs::clear();
		let done = match caught {
			Ok(Ok(d))	=> d,
			Ok(Err(e))	=> return Err(Failure::of(&e, &main)),
			Err(_)		=> {
				// A panic leaves the session in a state nobody has examined, its caches included, so the
				// whole session goes and the next compile begins from the embedded faces and the budgets.
				// The project's fonts are added again then.
				self.session	= Session::with_budgets(FontStore::with_base(base), budgets);
				self.fonts		= Vec::new();
				return Err(Failure::internal(&main,
					"the compiler panicked while compiling, so no document was produced"));
			},
		};
		let needs		= done.needs();
		let report		= done.report_in(Cols::Utf16);
		let counters	= done.counters;
		if let Err(e) = &done.laid {
			// The first error is the one the fixpoint stopped on; the sites after it follow.
			let mut rest = report.diagnostics.clone();
			let head = match done.engine.diags.iter().position(|d| d.is_error()) {
				Some(i)	=> rest.remove(i),
				None	=> Diagnostic::from_error(e, &main),
			};
			return Err(Failure { head, rest, pages: None, skipped: report.skipped.clone(), needs });
		}
		if p.strict {
			if let Some(head) = report.strict_failure(&main) {
				return Err(Failure::refused(head, &report, needs));
			}
		}
		self.good = true;
		match take(sink) {
			Some(product)	=> Ok(Made { product, report, needs, counters }),
			None			=> Err(Failure::internal(&main, "the fixpoint ended with no output")),
		}
	}

	/// The families a compile of `p` can set by name, sorted: the embedded faces and the family every face
	/// of every project font declares. It is read from the font book a compile resolves `font:` against, so
	/// a family is listed exactly when a compile finds it.
	pub fn font_families(&self, p: Option<&Project>) -> Vec<String> {
		let base = match self.session.fonts().base() {
			Some(b)	=> b.clone(),
			None	=> return Vec::new(),
		};
		let mut store = FontStore::with_base(base);
		if let Some(p) = p {
			for (_, bytes) in &p.fonts {
				store.add_bytes(bytes.clone());
			}
		}
		match store.book() {
			Ok(book)	=> book.family_names(),
			Err(_)		=> Vec::new(),
		}
	}

	/// What `selector`, a Typst selector expression, finds in the last good compile, as `typst query` finds
	/// it. `field` names the field each row's `value` carries, `value` where empty. It answers `None`, never
	/// a wrong answer, when nothing has compiled, the selector does not evaluate, or a field cannot be
	/// written as Typst writes it.
	pub fn query(&self, selector: &str, field: &str) -> Option<Vec<Row>> {
		let intro = match (self.good, self.session.intro()) {
			(true, Some(i))	=> i.clone(),
			_				=> return None,
		};
		let _turn = turn();
		// A fresh engine over the kept introspector evaluates the selector as `typst query` does.
		let mut engine = Engine::new(World::new(PathBuf::from("/")));
		engine.intro = intro.clone();
		let span = Span::detached();
		let found = match eval_string(&mut engine, selector, EvalMode::Code, Scope::default(), span) {
			Ok(v)	=> v,
			Err(_)	=> return None,
		};
		let sel = match select::cast(&mut engine, span, found) {
			Ok(s)	=> s,
			Err(_)	=> return None,
		};
		let hits = match intro.query(&sel) {
			Ok(h)	=> h,
			Err(_)	=> return None,
		};
		let want = if field.is_empty() { "value" } else { field };
		let mut rows = Vec::with_capacity(hits.len());
		for c in hits {
			let kind = match c.kind() {
				Some(k)	=> k.name().to_string(),
				None	=> return None,
			};
			// A field that cannot be written as Typst writes it spoils the whole answer, not just its row.
			let value = match c.field(want) {
				Some(v)	=> match data::plain_json(v) {
					Some(j)	=> Some(j),
					None	=> return None,
				},
				None	=> None,
			};
			let (title, level) = if kind == "heading" {
				let title = match c.field("body") {
					Some(Value::Content(b))	=> b.plain_text(),
					_						=> String::new(),
				};
				let level = match c.field("level") {
					Some(Value::Int(l))	=> *l,
					_					=> 1,
				};
				(Some(title), Some(level))
			} else {
				(None, None)
			};
			rows.push(Row {
				kind,
				label:	c.label().map(|l| l.as_str().to_string()),
				page:	c.location().and_then(|l| intro.page(l)),
				value,
				title,
				level,
			});
		}
		Some(rows)
	}

	/// The changed-only page delta through the evaluator, the live view's path, warm: it starts from the
	/// introspector of the compile before it. `known` is the page ids the
	/// consumer holds; the version is this instance's tick, stepped by a compile that produced a delta and
	/// left where it was by one that did not, a strict refusal included, so a consumer that dispatches
	/// compiles without awaiting each can discard a stale return by its version.
	pub fn compile_delta(&mut self, p: &Project) -> Result<Made<delta::PageDelta>, Failure> {
		self.compile_delta_into(p, Vec::new()).map(|m| Made {
			product:	delta::PageDelta::of(m.product.0, m.product.1),
			report:		m.report,
			needs:		m.needs,
			counters:	m.counters,
		})
	}

	/// As [`compile_delta`](Self::compile_delta), each changed page's SVG handed to `out` the moment it is
	/// drawn, so the compile holds none of them: the browser door pushes each into a JavaScript array.
	pub fn compile_delta_into<C: Changed>(&mut self, p: &Project, out: C)
		-> Result<Made<(delta::Head, C)>, Failure>
	{
		let made = self.run(p, DeltaSink::new(&p.known, self.version, out), false, |s| s.into_delta());
		if let Ok(m) = &made {
			self.version = m.product.0.version;
		}
		made
	}
}

/// Hands a package's files to the engine, `spec` as `@namespace/name:version`; the count of files held.
pub fn supply_package(spec: &str, files: Vec<(String, Vec<u8>)>) -> Outcome<usize> {
	let _turn = turn();
	let spec = res!(package::parse_spec(spec));
	package::supply(&spec, files)
}

/// Every package supplied, as `@namespace/name:version`, in order.
pub fn packages() -> Outcome<Vec<String>> {
	let _turn = turn();
	Ok(res!(package::supplied()).iter().map(|s| s.to_string()).collect())
}

/// Withdraws a supplied package; false when none was held under `spec`.
pub fn withdraw_package(spec: &str) -> Outcome<bool> {
	let _turn = turn();
	let spec: PackageSpec = res!(package::parse_spec(spec));
	package::withdraw(&spec)
}

/// Seals the project's sources and assets into the source map. A project that does not hold its main
/// file is an error before any compile.
fn install(p: &Project, main: &Path) -> Outcome<()> {
	let mut files: HashMap<PathBuf, Vec<u8>> = HashMap::new();
	for (path, text) in &p.sources {
		files.insert(PathBuf::from(path), text.clone().into_bytes());
	}
	for (path, bytes) in &p.assets {
		files.insert(PathBuf::from(path), bytes.clone());
	}
	if !files.keys().any(|k| k == main) {
		return Err(err!("The project has no source for its main file {:?}.", main; Input, Missing, File));
	}
	vfs::install_sealed(files)
}
