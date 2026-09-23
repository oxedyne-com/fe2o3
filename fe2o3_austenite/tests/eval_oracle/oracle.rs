// The external oracle: the host's `typst` 0.15.x, run as a process, never linked. Every run is
// capped with `systemd-run --user --scope -p MemoryMax=<cap>` where systemd-run exists
// (`EVAL_ORACLE_CAP`, default 3G, `none` to run uncapped); a missing or wrongly versioned binary is
// an error unless `EVAL_ORACLE_SKIP=1` says, explicitly, to skip -- so absence never reads as green.
//
// Outputs are cached under the test binary's target directory (never `/tmp`, never the source
// tree), keyed by the typst version and every byte of the fixture's area, so an edit to a fixture or
// a helper it imports re-runs the oracle and nothing else does. Expected values are always the
// oracle's own output; nothing Austenite produced is ever written here.

use crate::harness::corpus::{
	area_bytes,
	Fixture,
};
use crate::harness::json::{
	self,
	J,
};
use crate::harness::layout::{
	self,
	OPage,
};
use crate::harness::structure::{
	self,
	Sk,
};
use crate::harness::PosRow;

use oxedyne_fe2o3_core::prelude::*;

use std::path::{
	Path,
	PathBuf,
};
use std::process::Command;

pub const PINNED: &str = "typst 0.15.";

pub const PROBES_EXPR: &str = "query(<probe>).map(it => it.value)";
pub const POSITIONS_EXPR: &str = "query(selector(heading).or(figure, math.equation, footnote, <probe>))\
	.map(it => (func: repr(it.func()), pos: it.location().position()))";

/// What the oracle said about one fixture at one level: its answer, or the reason it has none.
pub type Said<T> = std::result::Result<T, String>;

#[derive(Clone, Debug)]
pub struct Oracle {
	pub bin:		String,
	pub version:	String,
	cap:			Option<String>,
	pub work:		PathBuf,
}

pub struct Html {
	pub skeleton:	Vec<Sk>,
	pub ignored:	Vec<String>,	// elements the HTML export dropped
}

impl Oracle {
	/// The oracle, `Ok(None)` only when `EVAL_ORACLE_SKIP=1` is set and no usable `typst` exists.
	pub fn find() -> Outcome<Option<Oracle>> {
		let skip = std::env::var("EVAL_ORACLE_SKIP").map(|v| v == "1").unwrap_or(false);
		let bin = std::env::var("EVAL_ORACLE_TYPST").unwrap_or_else(|_| "typst".to_string());
		let version = match Command::new(&bin).arg("--version").output() {
			Ok(o) if o.status.success()	=> String::from_utf8_lossy(&o.stdout).trim().to_string(),
			Ok(o)	=> {
				if skip {
					return Ok(None);
				}
				return Err(err!("`{} --version` failed: {}", bin, String::from_utf8_lossy(&o.stderr).trim(); IO, Missing));
			}
			Err(e)	=> {
				if skip {
					return Ok(None);
				}
				return Err(err!("The typst oracle `{}` cannot be run ({}). Install typst 0.15.x, point \
					EVAL_ORACLE_TYPST at it, or set EVAL_ORACLE_SKIP=1 to skip the oracle explicitly.", bin, e; IO, Missing));
			}
		};
		if !version.starts_with(PINNED) {
			if skip {
				return Ok(None);
			}
			return Err(err!("The oracle is pinned to {}x; `{}` reports {:?}.", PINNED, bin, version; Invalid, Mismatch));
		}
		let cap = match std::env::var("EVAL_ORACLE_CAP") {
			Ok(v) if v == "none"	=> None,
			Ok(v) if !v.is_empty()	=> Some(v),
			_						=> Some("3G".to_string()),
		};
		let cap = match cap {
			Some(c) if Command::new("systemd-run").arg("--version").output().map(|o| o.status.success()).unwrap_or(false) => Some(c),
			_ => None,
		};
		let work = res!(work_dir());
		Ok(Some(Oracle { bin, version, cap, work }))
	}

	fn command(&self, args: &[String], cwd: &Path) -> Command {
		let mut c = match &self.cap {
			Some(cap) => {
				let mut c = Command::new("systemd-run");
				c.args(["--user", "--scope", "--quiet", "-p"]);
				c.arg(fmt!("MemoryMax={}", cap));
				c.arg("--slice=claude-rc.slice");
				c.arg(&self.bin);
				c
			}
			None => Command::new(&self.bin),
		};
		c.args(args);
		c.current_dir(cwd);
		c
	}

	/// Runs typst; `Ok(Err(stderr))` when typst itself reported failure.
	fn run(&self, args: &[String], cwd: &Path) -> Outcome<Said<(String, String)>> {
		let out = match self.command(args, cwd).output() {
			Ok(o)	=> o,
			Err(e)	=> return Err(err!("Could not run the typst oracle: {}", e; IO)),
		};
		let stdout = String::from_utf8_lossy(&out.stdout).to_string();
		let stderr = String::from_utf8_lossy(&out.stderr).to_string();
		if out.status.success() {
			Ok(Ok((stdout, stderr)))
		} else {
			Ok(Err(first_error(&stderr)))
		}
	}

	/// The fixture's cache directory, created on first use.
	pub fn dir(&self, fx: &Fixture) -> Outcome<PathBuf> {
		let files = res!(area_bytes(&fx.root));
		let mut h = Fnv::new();
		h.feed(self.version.as_bytes());
		h.feed(fx.name.as_bytes());
		for (rel, bytes) in &files {
			h.feed(rel.as_bytes());
			h.feed(&[0]);
			h.feed(bytes);
		}
		let d = self.work.join(&fx.area).join(fmt!("{}-{:016x}", fx.name, h.0));
		res!(std::fs::create_dir_all(&d).map_err(|e| err!("Cannot create {}: {}", d.display(), e; IO, File, Write)));
		Ok(d)
	}

	/// A cached text product: `<what>.out` holds a success, `<what>.err` a typst failure.
	fn cached<F>(&self, fx: &Fixture, what: &str, make: F) -> Outcome<Said<String>>
		where F: FnOnce(&Oracle, &Path) -> Outcome<Said<String>>
	{
		let d = res!(self.dir(fx));
		let ok = d.join(fmt!("{}.out", what));
		let bad = d.join(fmt!("{}.err", what));
		// Tests run in parallel and share fixtures, so one of them makes a product at a time.
		let _guard = lock_mutex!(ORACLE_LOCK);
		if let Ok(s) = std::fs::read_to_string(&ok) {
			return Ok(Ok(s));
		}
		if let Ok(s) = std::fs::read_to_string(&bad) {
			return Ok(Err(s));
		}
		let said = res!(make(self, &d));
		let (path, text) = match &said {
			Ok(s)	=> (ok, s.clone()),
			Err(s)	=> (bad, s.clone()),
		};
		res!(write_whole(&path, text.as_bytes()));
		Ok(said)
	}

	fn eval(&self, fx: &Fixture, what: &str, expr: &str) -> Outcome<Said<J>> {
		let said = res!(self.cached(fx, what, |o, _| {
			let args = vec![
				"eval".to_string(), expr.to_string(),
				"--in".to_string(), fx.path.display().to_string(),
				"--root".to_string(), fx.root.display().to_string(),
			];
			Ok(res!(o.run(&args, &fx.root)).map(|(out, _)| out))
		}));
		match said {
			Ok(text)	=> Ok(Ok(res!(json::parse(&text)))),
			Err(e)		=> Ok(Err(e)),
		}
	}

	/// Does the fixture compile without error? `Ok(Err(message))` names the first error.
	pub fn compiles(&self, fx: &Fixture) -> Outcome<Said<()>> {
		let pdf = res!(self.pdf(fx));
		Ok(pdf.map(|_| ()))
	}

	fn pdf(&self, fx: &Fixture) -> Outcome<Said<PathBuf>> {
		let said = res!(self.cached(fx, "pdf", |o, d| {
			let pdf = d.join("typst.pdf");
			let args = vec![
				"compile".to_string(), fx.path.display().to_string(), pdf.display().to_string(),
				"--root".to_string(), fx.root.display().to_string(),
			];
			Ok(res!(o.run(&args, &fx.root)).map(|_| pdf.display().to_string()))
		}));
		Ok(said.map(PathBuf::from))
	}

	/// Level 1: every `<probe>` metadata value, in document order.
	pub fn probes(&self, fx: &Fixture) -> Outcome<Said<Vec<J>>> {
		match res!(self.eval(fx, "probes", PROBES_EXPR)) {
			Ok(J::Arr(v))	=> Ok(Ok(v)),
			Ok(other)		=> Ok(Err(fmt!("the probe query returned {}", other.render()))),
			Err(e)			=> Ok(Err(e)),
		}
	}

	/// Level 2: headings, figures, equations, footnotes and probes with their positions.
	pub fn positions(&self, fx: &Fixture) -> Outcome<Said<Vec<PosRow>>> {
		let v = match res!(self.eval(fx, "positions", POSITIONS_EXPR)) {
			Ok(J::Arr(v))	=> v,
			Ok(other)		=> return Ok(Err(fmt!("the position query returned {}", other.render()))),
			Err(e)			=> return Ok(Err(e)),
		};
		let mut rows = Vec::new();
		for r in v {
			let func = r.get("func").and_then(|f| f.as_str()).unwrap_or("").to_string();
			let pos = match r.get("pos") {
				Some(p)	=> p,
				None	=> return Ok(Err(fmt!("a position row has no `pos`: {}", r.render()))),
			};
			rows.push(PosRow {
				func,
				page:	pos.get("page").and_then(|p| p.as_i64()).unwrap_or(0) as u32,
				x:		pt(pos.get("x")),
				y:		pt(pos.get("y")),
			});
		}
		Ok(Ok(rows))
	}

	/// Level 3: the HTML export's skeleton and the elements it ignored.
	pub fn html(&self, fx: &Fixture) -> Outcome<Said<Html>> {
		let said = res!(self.cached(fx, "html", |o, d| {
			let out = d.join("typst.html");
			let args = vec![
				"compile".to_string(), "--features".to_string(), "html".to_string(),
				"--format".to_string(), "html".to_string(),
				fx.path.display().to_string(), out.display().to_string(),
				"--root".to_string(), fx.root.display().to_string(),
			];
			match res!(o.run(&args, &fx.root)) {
				Ok((_, stderr))	=> {
					let html = res!(std::fs::read_to_string(&out).map_err(|e| err!(
						"Cannot read {}: {}", out.display(), e; IO, File, Read)));
					// The ignored-element warnings ride along, one per line, ahead of a marker.
					let mut s = String::new();
					for n in structure::ignored_elements(&stderr) {
						s.push_str(&n);
						s.push('\n');
					}
					s.push_str("\u{0}\n");
					s.push_str(&html);
					Ok(Ok(s))
				}
				Err(e)	=> Ok(Err(e)),
			}
		}));
		let text = match said {
			Ok(t)	=> t,
			Err(e)	=> return Ok(Err(e)),
		};
		let (head, html) = match text.split_once("\u{0}\n") {
			Some(p)	=> p,
			None	=> ("", text.as_str()),
		};
		let ignored = head.lines().filter(|l| !l.is_empty()).map(|l| l.to_string()).collect();
		Ok(Ok(Html { skeleton: structure::from_html(html), ignored }))
	}

	/// Level 4: pages with their lines and baselines.
	pub fn layout(&self, fx: &Fixture) -> Outcome<Said<Vec<OPage>>> {
		let pdf = match res!(self.pdf(fx)) {
			Ok(p)	=> p,
			Err(e)	=> return Ok(Err(e)),
		};
		let d = res!(self.dir(fx));
		let bbox = d.join("typst.bbox.html");
		let _guard = lock_mutex!(ORACLE_LOCK);
		if !bbox.exists() {
			let st = Command::new("pdftotext").arg("-bbox-layout").arg(&pdf).arg(&bbox).output();
			match st {
				Ok(o) if o.status.success()	=> (),
				Ok(o)	=> return Err(err!("pdftotext failed on {}: {}", pdf.display(),
					String::from_utf8_lossy(&o.stderr).trim(); IO)),
				Err(e)	=> return Err(err!("pdftotext (poppler-utils) is needed for level 4: {}", e; IO, Missing)),
			}
		}
		let xml = res!(std::fs::read_to_string(&bbox).map_err(|e| err!("Cannot read {}: {}", bbox.display(), e; IO, File, Read)));
		let mut pages = layout::read_bbox(&xml);
		let svg1 = d.join("typst-1.svg");
		if !svg1.exists() {
			let pattern = d.join("typst-{p}.svg");
			let args = vec![
				"compile".to_string(), fx.path.display().to_string(), pattern.display().to_string(),
				"--root".to_string(), fx.root.display().to_string(),
			];
			if let Err(e) = res!(self.run(&args, &fx.root)) {
				return Ok(Err(e));
			}
		}
		for (i, p) in pages.iter_mut().enumerate() {
			let svg = d.join(fmt!("typst-{}.svg", i + 1));
			let text = res!(std::fs::read_to_string(&svg).map_err(|e| err!("Cannot read {}: {}", svg.display(), e; IO, File, Read)));
			layout::attach_baselines(p, &layout::glyph_origins(&text));
		}
		Ok(Ok(pages))
	}
}

fn pt(v: Option<&J>) -> f64 {
	match v {
		Some(J::Str(s))		=> s.trim_end_matches("pt").parse::<f64>().unwrap_or(f64::NAN),
		Some(J::Float(f))	=> *f,
		Some(J::Int(i))		=> *i as f64,
		_					=> f64::NAN,
	}
}

/// The first `error:` line of typst's diagnostics, with its location line when there is one.
fn first_error(stderr: &str) -> String {
	let mut lines = stderr.lines();
	while let Some(l) = lines.next() {
		if l.starts_with("error:") {
			let loc = lines.next().map(|n| n.trim().to_string()).unwrap_or_default();
			return fmt!("{} {}", l.trim(), loc);
		}
	}
	stderr.trim().lines().take(3).collect::<Vec<_>>().join(" | ")
}

static ORACLE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Writes beside the target and renames over it, so a reader never sees half a file.
fn write_whole(path: &Path, bytes: &[u8]) -> Outcome<()> {
	let tmp = path.with_extension(fmt!("part{}", std::process::id()));
	res!(std::fs::write(&tmp, bytes).map_err(|e| err!("Cannot write {}: {}", tmp.display(), e; IO, File, Write)));
	res!(std::fs::rename(&tmp, path).map_err(|e| err!("Cannot rename {}: {}", tmp.display(), e; IO, File, Write)));
	Ok(())
}

/// `<target>/<profile>/eval_oracle`, beside the test binary that is running.
pub fn work_dir() -> Outcome<PathBuf> {
	let exe = res!(std::env::current_exe().map_err(|e| err!("Cannot locate the test binary: {}", e; IO)));
	// `<target>/<profile>/deps/<binary>`
	let profile = match exe.parent().and_then(|deps| deps.parent()) {
		Some(p)	=> p.to_path_buf(),
		None	=> return Err(err!("The test binary {} has no target directory above it", exe.display(); Missing)),
	};
	let d = profile.join("eval_oracle");
	res!(std::fs::create_dir_all(&d).map_err(|e| err!("Cannot create {}: {}", d.display(), e; IO, File, Write)));
	Ok(d)
}

pub struct Fnv(pub u64);

impl Fnv {
	pub fn new() -> Self { Fnv(0xcbf2_9ce4_8422_2325) }

	pub fn feed(&mut self, b: &[u8]) {
		for x in b {
			self.0 ^= *x as u64;
			self.0 = self.0.wrapping_mul(0x0000_0100_0000_01b3);
		}
	}
}
