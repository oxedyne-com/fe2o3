//! Poll-based recompile for the `austenite --watch` mode, and the `watch` and `build` commands.
//!
//! There is no inotify on this fleet, so the watch samples a caller-supplied set of files on a fixed
//! interval and rebuilds whenever any of them changes. The file set is recomputed each tick from a
//! closure, so a chapter added to a book's `#include` list (or an asset dropped into its tree) is
//! picked up without restarting the watch. A file appearing or disappearing counts as a change, since
//! the snapshot keys on the paths that currently exist.
//!
//! [`Run`] is the loop of `austenite watch`, with a [`Run::tick`] that a test can drive; [`compile_pdf`]
//! is the one compile to a PDF that it, `austenite build` and `austenite --eval` share. The PDF is written
//! to a hidden temporary of this writer's own beside its place and renamed onto it, so a reader sees the
//! previous file or the next, never part of one, and a second writer of the same output never shares a file
//! with the first.

use crate::compile::Session;
use crate::diag;
use crate::emit::pdf::PdfOptions;
use crate::emit::sinks::PdfSink;
use crate::flow::text::FontStore;
use crate::settings::{
	self,
	Settings,
};
use crate::timings::{
	Phase,
	Timings,
};

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_core::file::{
	self,
	SaveMode,
};

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::io::{
	BufWriter,
	Write,
};
use std::path::{
	Path,
	PathBuf,
};
use std::time::{
	Duration,
	SystemTime,
};

// How far a file's stamp may fall behind the clock it was taken from, the kernel's tick.
const STAMP_LAG: Duration = Duration::from_millis(10);

/// Runs the poll loop: build once, then on every tick recompute the watched set via `files`, sample
/// their modification times, and call `build` whenever the sample differs from the last one -- an
/// mtime moved, or a watched file appeared or vanished. A build that fails returns its error, which is
/// printed here, and the loop carries on, so a transient compile error never stops the watch. Under
/// normal use this never returns; it ends only on an interrupt.
pub fn run<F, G>(mut files: G, mut build: F, interval: Duration) -> Outcome<()>
where
	F: FnMut() -> Outcome<()>,
	G: FnMut() -> Vec<PathBuf>,
{
	// Build once at the start so the watch shows a result immediately rather than on the first edit.
	if let Err(e) = build() {
		eprintln!("[austenite] {}", e);
	}
	let mut prev = snapshot(&files());
	loop {
		std::thread::sleep(interval);
		let now = snapshot(&files());
		if now != prev {
			prev = now;
			if let Err(e) = build() {
				eprintln!("[austenite] {}", e);
			}
		}
	}
}

// The snapshot to compare the next tick with, over the build's files: `before` for each path of the set
// the build began with, and `after` for a path it read for the first time, unless that was saved since the
// build `began`, when it gets the epoch so that the next tick sees a change. A path of the old set that did
// not exist before the build is left out, so that if it exists now the next tick sees a change; one that
// existed and has gone is kept, so that the next tick sees it missing.
fn carry(
	old:	&[PathBuf],
	files:	&[PathBuf],
	before:	&BTreeMap<PathBuf, SystemTime>,
	after:	&BTreeMap<PathBuf, SystemTime>,
	began:	SystemTime,
)
	-> BTreeMap<PathBuf, SystemTime>
{
	// A stamp can lag the clock it was taken from by a tick, so a file saved just before the build is
	// built again once, which costs a rebuild and not a missed save.
	let since = began.checked_sub(STAMP_LAG).unwrap_or(began);
	let known: BTreeSet<&PathBuf> = old.iter().collect();
	let mut out = BTreeMap::new();
	for path in files {
		if known.contains(path) {
			if let Some(t) = before.get(path) {
				out.insert(path.clone(), *t);
			}
		} else if let Some(t) = after.get(path) {
			out.insert(path.clone(), if *t >= since { SystemTime::UNIX_EPOCH } else { *t });
		}
	}
	out
}

/// Maps each path that currently exists and can be stat'd to its last-modified time. A path that
/// cannot be read is simply absent from the map, so its later appearance -- or the disappearance of one
/// present before -- changes the snapshot and triggers a rebuild.
fn snapshot(paths: &[PathBuf]) -> BTreeMap<PathBuf, SystemTime> {
	let mut m = BTreeMap::new();
	for p in paths {
		if let Ok(meta) = std::fs::metadata(p) {
			if let Ok(t) = meta.modified() {
				m.insert(p.clone(), t);
			}
		}
	}
	m
}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ ONE COMPILE TO A PDF                                                       │
// └───────────────────────────────────────────────────────────────────────────┘

/// What one compile to a PDF is asked to do.
#[derive(Clone, Debug)]
pub struct Spec {
	pub main:			PathBuf,
	pub root:			PathBuf,			// what a leading `/` resolves against
	pub out:			PathBuf,			// the finished PDF
	pub strict:			bool,
	pub diag_summary:	bool,
	pub timings:		Option<PathBuf>,	// the file the phase timings go to
	pub timings_fine:	bool,
	pub pdf:			PdfOptions,
}

/// What a compile reports of itself once its PDF is written.
#[derive(Clone, Debug)]
pub struct Report {
	pub pages:	u32,
	pub passes:	u32,
	pub warm:	bool,			// the first pass began from the session's kept introspector
	pub bytes:	usize,
	pub skip:	Option<String>,	// the terse `skipped:` line, when the caller folds it into its status line
	pub secs:	f64,
}

/// Compiles `spec.main` through the evaluator under `session` and writes the PDF to `spec.out`.
/// `cold` starts from no introspector, as a one-shot compile does, so that its bytes never depend on an
/// earlier compile; the watch compiles warm. The terse `skipped:` line is built from the diagnostics of kind
/// `unsupported`; under `strict` a refusal of the strict rule fails the compile as Daimond's does.
/// `diag_summary` adds one stderr line for each severity, kind and construct, counts only
/// ([`diag::summary_lines`]), and one for each error that names its call and types in Typst's own terms
/// ([`diag::error_lines`]), on a compile that succeeds and on one that fails. `spec.timings` writes each
/// phase's wall time, as JSON, after a compile that succeeds; the PDF is the same bytes either way.
///
/// `read` is filled with the files the evaluation asked for as soon as it has run, so it is there for a
/// compile that fails, and holds at least the source when even that could not be read. The PDF is written
/// whole to a hidden temporary of this call's own beside its place ([`file::save_atomic_with`]) and renamed
/// onto it, after every check, so a compile that fails leaves the last good one as it was, a reader never
/// meets half of one, and a failed write leaves nothing behind. An output where a directory or a symbolic link
/// stands, or that the compile has just read, is refused and left as it was. `fold` holds the `skipped:` line back in the
/// result for the caller to print, rather than writing it to the standard error.
pub fn compile_pdf(
	spec:		&Spec,
	session:	&mut Session,
	cold:		bool,
	fold:		bool,
	read:		&mut Vec<PathBuf>,
)
	-> Outcome<Report>
{
	let t		= std::time::Instant::now();
	let timings	= spec.timings.as_ref().map(|_| if spec.timings_fine { Timings::start_fine() } else { Timings::start() });
	read.push(spec.main.clone());
	crate::compile::supply_typst_package_cache();
	let mut sink = res!(PdfSink::with_options(spec.pdf.clone()));
	let mut done = res!(session.compile(&spec.main, &spec.root, &mut sink, timings, None, cold));
	*read = done.files_read();
	let report = done.report();
	let mut skip = None;
	if let Some(line) = &report.skipped {
		if fold {
			skip = Some(line.clone());
		} else {
			eprintln!("[austenite] {}", line);
		}
	}
	// Before the error a failed compile returns, so a run that stops still names what it passed over.
	if spec.diag_summary {
		for line in diag::summary_lines(&done.engine.diags) {
			eprintln!("{}", line);
		}
		for line in diag::error_lines(&done.engine.diags, &done.engine.world.sources) {
			eprintln!("{}", line);
		}
	}
	let laid = match &done.laid {
		Ok(l)	=> l,
		Err(e)	=> {
			for d in &done.engine.diags {
				eprintln!("{}", d.render(&done.engine.world.sources));
			}
			return Err(err!("{}", e.plain(); Input, Invalid));
		},
	};
	if spec.strict {
		if let Some(refusal) = report.strict_failure(&spec.main) {
			return Err(err!("{}", refusal.message; Input, Invalid));
		}
	}
	let out = match sink.output() {
		Some(o)	=> o,
		None	=> return Err(err!("The fixpoint ended without a finished PDF."; Bug)),
	};
	if let Some(t) = done.engine.timings.as_mut() {
		t.enter(Phase::Write);
	}
	// What stands at the output may have changed since the plan was settled, and only now is the read set known.
	res!(check_standing(&spec.out));
	res!(check_unread(&spec.out, read));
	if let Some(dir) = spec.out.parent().filter(|d| !d.as_os_str().is_empty()) {
		res!(std::fs::create_dir_all(dir));
	}
	res!(file::save_atomic_with(&spec.out, SaveMode::Keep, |f| {
		let mut w = BufWriter::new(f);
		res!(out.write_to(&mut w));
		res!(w.flush());
		Ok(())
	}));
	if let (Some(tm), Some(dest)) = (done.engine.timings.as_mut(), spec.timings.as_ref()) {
		tm.leave();
		if let Ok(book) = done.engine.fonts.book() {
			if let Ok(stats) = book.shape_stats() {
				tm.set_shape(stats);
			}
		}
		res!(std::fs::write(dest, tm.json(t.elapsed().as_nanos() as u64)));
	}
	Ok(Report {
		pages:	laid.pages,
		passes:	laid.passes,
		warm:	done.warm,
		bytes:	out.len(),
		skip,
		secs:	t.elapsed().as_secs_f64(),
	})
}

// Refuses an output that names no file, or the source itself, before anything compiles.
fn check_output(out: &Path, raw: &str, main: &Path) -> Outcome<()> {
	if raw.trim().is_empty() {
		return Err(err!("The setting 'output' is empty, so it names no file to write the PDF to."; Input, Invalid));
	}
	if out.file_name().is_none() {
		return Err(err!("The setting 'output' = \"{}\" names no file to write the PDF to.", raw; Input, Invalid));
	}
	res!(check_standing(out));
	if std::fs::canonicalize(out).ok().as_deref() == Some(main) {
		return Err(err!(
			"The setting 'output' names {}, the document's own source; the PDF would replace it, so it is refused.",
			out.display(); Input, Invalid));
	}
	Ok(())
}

// Refuses an output where a directory or a symbolic link stands: the swap renames over the name, so it would
// replace the directory or the link itself, not write into it or through it.
fn check_standing(out: &Path) -> Outcome<()> {
	if let Ok(m) = std::fs::symlink_metadata(out) {
		let what = if m.file_type().is_symlink() { "a symbolic link" } else if m.is_dir() { "a directory" } else { return Ok(()) };
		return Err(err!(
			"The setting 'output' names {}, where {} stands; the PDF would replace it, so it is refused.",
			out.display(), what; Input, Invalid));
	}
	Ok(())
}

// Refuses an output that the compile has just read, before the swap, which would replace that file.
fn check_unread(out: &Path, read: &[PathBuf]) -> Outcome<()> {
	let at = match std::fs::canonicalize(out) {
		Ok(p)	=> p,
		Err(_)	=> return Ok(()),	// nothing there to read
	};
	if read.iter().any(|r| std::fs::canonicalize(r).ok().as_ref() == Some(&at)) {
		return Err(err!(
			"The setting 'output' names {}, a file the compile reads; the PDF would replace it, so it is refused \
			and the file is left as it was.", out.display(); Input, Invalid));
	}
	Ok(())
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ THE COMMANDS                                                               │
// └───────────────────────────────────────────────────────────────────────────┘

/// What a `watch` or `build` command settled on: the settings, the file they came from, and the compile
/// they ask for.
#[derive(Clone, Debug)]
pub struct Plan {
	pub settings:	Settings,
	pub file:		Option<PathBuf>,
	pub spec:		Spec,
	pub warned:		Vec<String>,	// a file setting a `--set` made inapplicable, one line each
	base:			PathBuf,	// the directory the root, the document and the fonts are named from
	bare:			bool,		// made by `bare`: no settings file is read or watched
}

/// What `austenite --eval --watch` is given, in place of a settings file: the source, the directory the PDF
/// goes into as `document.pdf`, `--root`, each `--font-path`, `--strict`, `--diag-summary`, `--timings FILE`
/// and `--timings-fine`.
#[derive(Clone, Debug)]
pub struct Bare {
	pub source:			PathBuf,
	pub out_dir:		PathBuf,
	pub root:			Option<PathBuf>,	// default the source's directory
	pub fonts:			Vec<PathBuf>,
	pub strict:			bool,
	pub diag_summary:	bool,
	pub timings:		Option<PathBuf>,
	pub timings_fine:	bool,
}

impl Plan {
	/// Settles the run for `source`, or for the document the settings or the selection rule name when there
	/// is none. The settings file is the first `austenite.jdat` at or above the source's directory, else the
	/// working directory; `sets` are the `--set` arguments. A named source wins over the `document` setting.
	pub fn new(source: Option<&Path>, sets: &[String], timings: Option<&Path>) -> Outcome<Self> {
		let cwd = res!(std::env::current_dir());
		Self::at(&cwd, source, sets, timings)
	}

	/// As [`new`](Self::new), with `cwd` the working directory the settings file is looked for from when there
	/// is no source.
	pub fn at(cwd: &Path, source: Option<&Path>, sets: &[String], timings: Option<&Path>) -> Outcome<Self> {
		let cwd = cwd.to_path_buf();
		let start = match source {
			Some(s)	=> s.to_path_buf(),
			None	=> cwd.clone(),
		};
		let file = settings::find(&start);
		let settings = res!(Settings::resolve(file.as_deref(), sets));
		let base = match &file {
			Some(f)	=> f.parent().map(|d| d.to_path_buf()).unwrap_or_else(|| cwd.clone()),
			None	=> if start.is_dir() { start.clone() } else {
				start.parent().filter(|d| !d.as_os_str().is_empty()).map(|d| d.to_path_buf()).unwrap_or_else(|| cwd.clone())
			},
		};
		let main = match source {
			Some(s)	=> s.to_path_buf(),
			None	=> if settings.document.is_empty() { res!(settings::select_document(&base)) } else { base.join(&settings.document) },
		};
		let main = res!(std::fs::canonicalize(&main));
		Self::settle(settings, file, base, main, timings)
	}

	/// The plan of `austenite --eval --watch`, which looks for no settings file: every setting at its default
	/// but for the viewer and the figure rendering, which are off because the caller that wants either has its
	/// own, and the few that `bare` carries. A relative path of `bare` is taken from `cwd`.
	pub fn bare(cwd: &Path, bare: &Bare) -> Outcome<Self> {
		let main = res!(std::fs::canonicalize(cwd.join(&bare.source)));
		let mut settings = Settings::default();
		settings.view.open		= false;
		settings.figs.render	= false;
		settings.strict			= bare.strict;
		if bare.diag_summary {
			settings.diagnostics = "summary".to_string();
		}
		settings.fonts	= bare.fonts.iter().map(|f| cwd.join(f).display().to_string()).collect();
		settings.root	= match &bare.root {
			Some(r)	=> cwd.join(r).display().to_string(),
			None	=> main.parent().map(|d| d.display().to_string()).unwrap_or_default(),
		};
		settings.output	= cwd.join(&bare.out_dir).join("document.pdf").display().to_string();
		let mut plan = res!(Self::settle(settings, None, cwd.to_path_buf(), main, bare.timings.as_deref()));
		plan.spec.timings_fine	= bare.timings_fine;
		plan.bare				= true;
		Ok(plan)
	}

	// The plan for settings already read, a canonical `main`, and the `base` the root and the profiles are named from.
	fn settle(settings: Settings, file: Option<PathBuf>, base: PathBuf, main: PathBuf, timings: Option<&Path>) -> Outcome<Self> {
		let doc_dir = main.parent().map(|d| d.to_path_buf()).unwrap_or_else(|| base.clone());
		let warned = res!(settings.check_built(&doc_dir));
		for w in &warned {
			eprintln!("[austenite] {}", w);
		}
		let root = if settings.root.is_empty() { base.clone() } else { base.join(&settings.root) };
		let out = settings.output_path(&main);
		res!(check_output(&out, &settings.output, &main));
		let spec = Spec {
			out,
			main,
			root,
			strict:			settings.strict,
			diag_summary:	settings.diagnostics == "summary",
			timings:		timings.map(|t| t.to_path_buf()),
			timings_fine:	false,
			pdf:			res!(settings.pdf_options(&base)),
		};
		Ok(Self { settings, file, spec, warned, base, bare: false })
	}

	/// A session over the font directories the settings name, relative to the root.
	pub fn session(&self) -> Session {
		let mut fonts = FontStore::default();
		for f in &self.settings.fonts {
			fonts.add_dir(self.spec.root.join(f));
		}
		Session::new(fonts)
	}

	// Where the settings file is, or would be if made: beside the document root. None for a bare plan, which has none.
	fn settings_path(&self) -> Option<PathBuf> {
		if self.bare {
			return None;
		}
		Some(self.file.clone().unwrap_or_else(|| self.base.join(settings::FILE)))
	}
}

/// `austenite build`: one cold compile of `source`, or of the document the settings or the selection rule
/// name when there is none, under the settings.
pub fn build(source: Option<&Path>, sets: &[String], timings: Option<&Path>) -> Outcome<(Plan, Report)> {
	let cwd = res!(std::env::current_dir());
	build_at(&cwd, source, sets, timings)
}

/// As [`build`], with `cwd` the working directory the settings file is looked for from when there is no source.
pub fn build_at(cwd: &Path, source: Option<&Path>, sets: &[String], timings: Option<&Path>) -> Outcome<(Plan, Report)> {
	let plan = res!(Plan::at(cwd, source, sets, timings));
	let mut session = plan.session();
	let mut read = Vec::new();
	let report = res!(compile_pdf(&plan.spec, &mut session, true, false, &mut read));
	Ok((plan, report))
}

/// What a tick of the watch did.
#[derive(Clone, Debug)]
pub enum Tick {
	Idle,					// nothing watched had changed
	Built(Report),			// compiled, and the PDF swapped in
	Failed(String),			// a compile or a settings reload failed; the last good PDF is as it was
}

/// The loop of `austenite watch`: the files the last compile read, and the settings file, sampled on
/// each tick; a change compiles through one [`Session`], warm. A change to the settings file reloads them
/// and compiles cold once. The viewer opens after the first finished PDF and is not opened again.
pub struct Run {
	source:		Option<PathBuf>,
	sets:		Vec<String>,
	timings:	Option<PathBuf>,
	cold:		bool,			// every compile cold, as `--cold` asks
	plan:		Plan,
	session:	Session,
	set:		Vec<PathBuf>,	// the last compile's reads, and the settings file
	seen:		BTreeMap<PathBuf, SystemTime>,
	first:		bool,
	cold_next:	bool,			// the next compile starts cold, after a settings change
	viewed:		bool,
}

impl Run {
	pub fn new(source: Option<PathBuf>, sets: Vec<String>, cold: bool, timings: Option<PathBuf>) -> Outcome<Self> {
		let plan = res!(Plan::new(source.as_deref(), &sets, timings.as_deref()));
		Ok(Self::over(plan, source, sets, cold, timings))
	}

	/// The loop of `austenite --eval --watch`: the plan of [`Plan::bare`], which no settings file shapes or
	/// reloads.
	pub fn bare(cwd: &Path, bare: &Bare) -> Outcome<Self> {
		let plan = res!(Plan::bare(cwd, bare));
		Ok(Self::over(plan, None, Vec::new(), false, None))
	}

	fn over(plan: Plan, source: Option<PathBuf>, sets: Vec<String>, cold: bool, timings: Option<PathBuf>) -> Self {
		let session = plan.session();
		Self {
			source, sets, timings, cold, plan, session,
			set:		Vec::new(),
			seen:		BTreeMap::new(),
			first:		true,
			cold_next:	false,
			viewed:		false,
		}
	}

	pub fn plan(&self) -> &Plan { &self.plan }

	/// One sample of the watched set, and a compile if it moved (or this is the first tick).
	pub fn tick(&mut self) -> Outcome<Tick> {
		let before = snapshot(&self.set);
		if !self.first && before == self.seen {
			return Ok(Tick::Idle);
		}
		if let Some(sf) = self.plan.settings_path() {
			if !self.first && before.get(&sf) != self.seen.get(&sf) {
				match Plan::new(self.source.as_deref(), &self.sets, self.timings.as_deref()) {
					Ok(plan)	=> {
						self.session.set_fonts(plan.session().fonts().clone());
						self.plan = plan;
						self.cold_next = true;
					},
					Err(e)		=> {
						// The old settings stand; the file is seen as it is now, so the error is told once.
						match before.get(&sf) {
							Some(t)	=> { self.seen.insert(sf, *t); },
							None	=> { self.seen.remove(&sf); },
						}
						return Ok(Tick::Failed(e.plain()));
					},
				}
			}
		}
		let began = SystemTime::now();
		let cold = self.cold || !self.plan.settings.watch.warm || self.cold_next;
		self.cold_next = false;
		let mut read = Vec::new();
		let result = compile_pdf(&self.plan.spec, &mut self.session, cold, true, &mut read);
		let mut files = read;
		if let Some(sf) = self.plan.settings_path() {
			if !files.contains(&sf) {
				files.push(sf);
			}
		}
		self.seen = carry(&self.set, &files, &before, &snapshot(&files), began);
		self.set = files;
		self.first = false;
		match result {
			Ok(report)	=> {
				self.view();
				Ok(Tick::Built(report))
			},
			Err(e)		=> Ok(Tick::Failed(e.plain())),
		}
	}

	// Opens the viewer the first time a PDF is finished, in the terminal's process group so that its
	// interrupt reaches it, and never again.
	fn view(&mut self) {
		if self.viewed {
			return;
		}
		self.viewed = true;
		if let Some(app) = self.plan.settings.viewer() {
			let spawned = std::process::Command::new(&app)
				.arg(&self.plan.spec.out)
				.stdin(std::process::Stdio::null())
				.stdout(std::process::Stdio::null())
				.stderr(std::process::Stdio::null())
				.spawn();
			if let Err(e) = spawned {
				eprintln!("[austenite] the viewer '{}' did not start: {}", app, e);
			}
		}
	}

	/// The status line for a finished compile: pages, passes, warm or cold, and seconds.
	pub fn line(&self, r: &Report) -> String {
		let mut line = fmt!("[austenite] {} -> {} page(s), {:.2}s, {}, {} pass{} -> {}",
			self.plan.spec.main.display(), r.pages, r.secs, if r.warm { "warm" } else { "cold" },
			r.passes, if r.passes == 1 { "" } else { "es" }, self.plan.spec.out.display());
		if let Some(skip) = &r.skip {
			line.push_str("; ");
			line.push_str(skip);
		}
		line
	}

	/// Runs until interrupted, printing one line for each compile.
	pub fn run(mut self) -> Outcome<()> {
		println!("[austenite] watching {} -> {} (Ctrl-C to stop)",
			self.plan.spec.main.display(), self.plan.spec.out.display());
		loop {
			match res!(self.tick()) {
				Tick::Idle			=> (),
				Tick::Built(r)		=> println!("{}", self.line(&r)),
				Tick::Failed(why)	=> eprintln!("[austenite] {}", why),
			}
			std::thread::sleep(Duration::from_millis(self.plan.settings.watch.poll_ms));
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	use std::time::UNIX_EPOCH;

	fn at(s: u64) -> SystemTime { UNIX_EPOCH + Duration::from_secs(s) }

	fn p(name: &str) -> PathBuf { PathBuf::from(name) }

	fn snap(items: &[(&str, u64)]) -> BTreeMap<PathBuf, SystemTime> {
		items.iter().map(|(n, s)| (p(n), at(*s))).collect()
	}

	#[test]
	fn a_known_file_keeps_the_time_it_had_before_the_build() {
		// `a` was saved again while the build ran: the build may have read it either side of the save, so
		// the next tick must see it as changed.
		let out = carry(&[p("a")], &[p("a")], &snap(&[("a", 1)]), &snap(&[("a", 2)]), at(0));
		assert_eq!(out, snap(&[("a", 1)]));
	}

	#[test]
	fn a_file_first_read_by_the_build_takes_the_time_after_it() {
		let out = carry(&[p("a")], &[p("a"), p("b")], &snap(&[("a", 1)]), &snap(&[("a", 1), ("b", 5)]), at(9));
		assert_eq!(out, snap(&[("a", 1), ("b", 5)]));
	}

	#[test]
	fn a_file_first_read_and_saved_since_the_build_began_reads_as_changed() {
		// `b` carries a time after the start of the build (10 s), so whatever the build read of it is
		// stale: the epoch stands in, which no later sample equals.
		let out = carry(&[], &[p("a"), p("b")], &snap(&[]), &snap(&[("a", 5), ("b", 12)]), at(10));
		assert_eq!(out, snap(&[("a", 5), ("b", 0)]));
	}

	#[test]
	fn a_known_file_that_was_absent_and_has_arrived_reads_as_changed() {
		// Absent before the build, present after it: left out, so the next sample, which has it, differs.
		let out = carry(&[p("a")], &[p("a")], &snap(&[]), &snap(&[("a", 3)]), at(9));
		assert_eq!(out, snap(&[]));
	}

	#[test]
	fn a_known_file_that_has_gone_reads_as_changed() {
		let out = carry(&[p("a")], &[p("a")], &snap(&[("a", 1)]), &snap(&[]), at(9));
		assert_eq!(out, snap(&[("a", 1)]));
	}

	#[test]
	fn a_file_the_build_no_longer_reads_is_dropped() {
		let out = carry(&[p("a"), p("b")], &[p("a")], &snap(&[("a", 1), ("b", 2)]), &snap(&[("a", 1), ("b", 2)]), at(9));
		assert_eq!(out, snap(&[("a", 1)]));
	}
}
