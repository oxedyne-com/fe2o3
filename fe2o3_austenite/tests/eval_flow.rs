//! Block flow, floats, fractional spacing and pages against the `typst` 0.15.1 oracle (U6b).
//!
//! Each fixture in `tests/fixtures/eval/{flow,float,fr}` labels the elements it probes `<probe>`. Typst
//! reports each probe's page and position through `typst eval --in`, and its page count and page sizes
//! through an SVG compile; Austenite evaluates the fixture, lays its pages out through
//! `flow::paginate` and `Paginator::next_placed` a page at a time, and reads the same from the located elements
//! the pages report and the pages. Positions must agree within 0.5pt, page counts exactly, page sizes within 0.5pt.
//!
//! A fixture whose first lines say `// needs: <unit>` depends on a unit not yet merged. While Austenite
//! cannot evaluate it, it is reported as pending; the moment it evaluates, it is held to the oracle like
//! any other. The suite fails unless at least `MIN_COMPARED` fixtures were compared in full.

use oxedyne_fe2o3_austenite::driver::Recorder;
use oxedyne_fe2o3_austenite::eval::content::Content;
use oxedyne_fe2o3_austenite::eval::styles::StyleChain;
use oxedyne_fe2o3_austenite::eval::{
	eval_source,
	Engine,
	World,
};
use oxedyne_fe2o3_austenite::flow;
use oxedyne_fe2o3_austenite::ledger::Position;

use oxedyne_fe2o3_core::prelude::*;

use std::collections::HashMap;
use std::path::{
	Path,
	PathBuf,
};
use std::process::Command;

const TOL:				f64		= 0.5;	// points
const MIN_COMPARED:		usize	= 10;	// fixtures that must be compared in full
const AREAS:			&[&str]	= &["flow", "float", "fr", "flow_errors"];

fn root() -> PathBuf {
	Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/eval")
}

fn typst_bin() -> String {
	std::env::var("TYPST").unwrap_or_else(|_| "typst".to_string())
}

/// Runs the host `typst` under a 3G memory cap.
fn typst(args: &[&str]) -> std::result::Result<String, String> {
	let mut cmd = Command::new("systemd-run");
	cmd.args(["--user", "--scope", "--quiet", "-p", "MemoryMax=3G", "--slice=claude-rc.slice"]);
	cmd.arg(typst_bin());
	cmd.args(args);
	cmd.current_dir(root());
	let out = match cmd.output() {
		Ok(o)	=> o,
		Err(e)	=> return Err(fmt!("could not run typst: {}", e)),
	};
	if !out.status.success() {
		return Err(String::from_utf8_lossy(&out.stderr).to_string());
	}
	Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

#[derive(Clone, Copy, Debug)]
struct Pos {
	page:	u32,
	x:		f64,
	y:		f64,
}

#[derive(Debug)]
struct Laid {
	probes:	Vec<Pos>,
	pages:	Vec<(f64, f64)>,
}

/// Typst's side: probe positions from `typst eval`, page sizes from the SVG pages of a compile.
fn oracle(rel: &str) -> std::result::Result<Laid, String> {
	let expr = "query(<probe>).map(e => { let p = e.location().position(); (p.page, p.x.pt(), p.y.pt()) })";
	let json = match typst(&["eval", "--root", ".", "--in", rel, expr]) {
		Ok(j)	=> j,
		Err(e)	=> return Err(e),
	};
	let nums = numbers(&json);
	let mut probes = Vec::new();
	for c in nums.chunks(3) {
		if c.len() == 3 {
			probes.push(Pos { page: c[0] as u32, x: c[1], y: c[2] });
		}
	}
	let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("eval_flow").join(rel.replace('/', "_"));
	let _ = std::fs::remove_dir_all(&dir);
	if let Err(e) = std::fs::create_dir_all(&dir) {
		return Err(fmt!("could not make {}: {}", dir.display(), e));
	}
	let pattern = dir.join("p{0p}.svg");
	let pattern = pattern.to_string_lossy().to_string();
	if let Err(e) = typst(&["compile", "--root", ".", rel, &pattern]) {
		return Err(e);
	}
	let mut files: Vec<PathBuf> = match std::fs::read_dir(&dir) {
		Ok(rd)	=> rd.filter_map(|e| e.ok().map(|e| e.path())).collect(),
		Err(e)	=> return Err(fmt!("could not read {}: {}", dir.display(), e)),
	};
	files.sort();
	let mut pages = Vec::new();
	for f in files {
		let svg = std::fs::read_to_string(&f).unwrap_or_default();
		let w = attr(&svg, "width=\"").unwrap_or(0.0);
		let h = attr(&svg, "height=\"").unwrap_or(0.0);
		pages.push((w, h));
	}
	Ok(Laid { probes, pages })
}

/// Every number in a JSON text, in order.
fn numbers(s: &str) -> Vec<f64> {
	let mut out = Vec::new();
	let mut cur = String::new();
	for ch in s.chars() {
		if ch.is_ascii_digit() || ch == '.' || ch == '-' || ch == 'e' || ch == '+' {
			cur.push(ch);
		} else {
			if let Ok(v) = cur.parse::<f64>() {
				out.push(v);
			}
			cur.clear();
		}
	}
	if let Ok(v) = cur.parse::<f64>() {
		out.push(v);
	}
	out
}

/// The number after the first occurrence of `key` in an SVG's root element, in points.
fn attr(svg: &str, key: &str) -> Option<f64> {
	let i = match svg.find(key) {
		Some(i)	=> i + key.len(),
		None	=> return None,
	};
	let rest = &svg[i..];
	match rest.find(|c: char| !(c.is_ascii_digit() || c == '.')) {
		Some(end)	=> rest[..end].parse::<f64>().ok(),
		None		=> None,
	}
}

/// Typst's first error for a fixture it must reject.
fn oracle_error(rel: &str) -> std::result::Result<String, String> {
	let out = Path::new(env!("CARGO_TARGET_TMPDIR")).join("eval_flow_reject.pdf");
	let out = out.to_string_lossy().to_string();
	match typst(&["compile", "--root", ".", rel, &out]) {
		Ok(_)	=> Err("typst accepted a fixture marked `rejects`".to_string()),
		Err(e)	=> match e.lines().find_map(|l| l.strip_prefix("error: ")) {
			Some(m)	=> Ok(m.trim().to_string()),
			None	=> Err(fmt!("typst failed without an error line: {}", e)),
		},
	}
}

/// Austenite's first error for a fixture it must reject, as the innermost message.
fn austenite_error(path: &Path) -> std::result::Result<String, String> {
	match austenite(path) {
		Ok(_)	=> Err("austenite accepted it".to_string()),
		Err(e)	=> Ok(e.splitn(2, ": ").nth(1).unwrap_or("").trim().to_string()),
	}
}

/// Austenite's side: evaluate, lay the pages out, read the probes' located positions.
fn austenite(path: &Path) -> std::result::Result<Laid, String> {
	let mut world = World::new(root());
	let id = match world.load(path) {
		Ok(id)	=> id,
		Err(e)	=> return Err(fmt!("load: {}", e)),
	};
	let mut engine = Engine::new(world);
	let module = match eval_source(&mut engine, id) {
		Ok(m)	=> m,
		Err(e)	=> return Err(fmt!("eval: {}", last(&e))),
	};
	let mut paginator = flow::paginate(&module.content, &StyleChain::root());
	let mut order = Order::default();
	let mut pages = Vec::new();
	loop {
		let page = match paginator.next_placed(&mut engine, &mut order) {
			Ok(Some((p, _)))	=> p,
			Ok(None)			=> break,
			Err(e)				=> return Err(fmt!("flow: {}", last(&e))),
		};
		pages.push((page.geom.width.to_pt(), page.geom.height.to_pt()));
	}
	let probes = order.finish().iter()
		.filter(|(elem, _)| elem.label().map(|x| x.as_str() == "probe").unwrap_or(false))
		.map(|(_, pos)| Pos { page: pos.page, x: pos.x.to_pt(), y: pos.y.to_pt() })
		.collect();
	Ok(Laid { probes, pages })
}

/// Where a located element stands in document order: in the order the pages placed it, or, inside a frame
/// with a logical parent, just after that parent's mark with the others under it.
#[derive(Clone, Copy, Debug)]
enum Slot {
	Main(u64),
	Under(u64, u64),	// the parent's id, then the order among its children
}

/// The located elements of a document in Typst's document order: page order, except that a frame with a
/// parent (a float's material, a footnote's entry) is read where its parent stands, as Typst's introspector
/// reads a frame's parent. The test's stand-in for the introspector's builder.
#[derive(Default)]
struct Order {
	seq:		u64,
	marks:		HashMap<u64, Slot>,
	entries:	Vec<(Slot, Content, Position)>,
}

impl Order {
	fn slot(&mut self, parent: Option<u64>) -> Slot {
		self.seq += 1;
		match parent {
			Some(p)	=> Slot::Under(p, self.seq),
			None	=> Slot::Main(self.seq),
		}
	}

	fn key(&self, slot: Slot, depth: usize) -> Vec<u64> {
		match slot {
			Slot::Main(s)		=> vec![2 * s],
			Slot::Under(p, s)	=> match self.marks.get(&p) {
				// A parent chain deeper than any real nesting is cut off rather than followed round a cycle.
				Some(m) if depth < 64 => {
					let mut k = self.key(*m, depth + 1);
					k.push(1);
					k.push(s);
					k
				},
				_ => vec![2 * s],
			},
		}
	}

	fn finish(self) -> Vec<(Content, Position)> {
		let mut keyed: Vec<(Vec<u64>, Content, Position)> = Vec::with_capacity(self.entries.len());
		for (slot, elem, pos) in &self.entries {
			keyed.push((self.key(*slot, 0), elem.clone(), *pos));
		}
		keyed.sort_by(|a, b| a.0.cmp(&b.0));
		keyed.into_iter().map(|(_, e, p)| (e, p)).collect()
	}
}

impl Recorder for Order {
	fn record(&mut self, elem: &Content, pos: Position, parent: Option<u64>) {
		let slot = self.slot(parent);
		if let Some(loc) = elem.location() {
			// A footnote's entry is read after the footnote itself: the element's location is a parent id.
			self.marks.insert(loc.0, slot);
		}
		self.entries.push((slot, elem.clone(), pos));
	}

	fn mark(&mut self, id: u64) {
		let slot = self.slot(None);
		self.marks.insert(id, slot);
	}
}

fn last(e: &Error<ErrTag>) -> String {
	match e.msgs().into_iter().last() {
		Some(m)	=> m,
		None	=> fmt!("{}", e),
	}
}

/// The differences between the two sides, one line each; empty when they agree.
fn compare(want: &Laid, got: &Laid) -> Vec<String> {
	let mut out = Vec::new();
	if want.pages.len() != got.pages.len() {
		out.push(fmt!("pages: typst {} austenite {}", want.pages.len(), got.pages.len()));
	}
	for (i, (w, g)) in want.pages.iter().zip(got.pages.iter()).enumerate() {
		// Typst's SVG gives whole-point viewports when the size is whole; compare within the tolerance.
		if (w.0 - g.0).abs() > TOL.max(1.0) || (w.1 - g.1).abs() > TOL.max(1.0) {
			out.push(fmt!("page {} size: typst {:.2}x{:.2} austenite {:.2}x{:.2}", i + 1, w.0, w.1, g.0, g.1));
		}
	}
	if want.probes.len() != got.probes.len() {
		out.push(fmt!("probes: typst {} austenite {}", want.probes.len(), got.probes.len()));
	}
	for (i, (w, g)) in want.probes.iter().zip(got.probes.iter()).enumerate() {
		if w.page != g.page || (w.x - g.x).abs() > TOL || (w.y - g.y).abs() > TOL {
			out.push(fmt!("probe {}: typst p{} ({:.2}, {:.2}) austenite p{} ({:.2}, {:.2})",
				i + 1, w.page, w.x, w.y, g.page, g.x, g.y));
		}
	}
	out
}

struct Fixture {
	rel:		String,
	path:		PathBuf,
	needs:		Vec<String>,
	rejects:	bool,
}

fn fixtures() -> Vec<Fixture> {
	let mut out = Vec::new();
	for area in AREAS {
		let dir = root().join(area);
		let mut files: Vec<PathBuf> = match std::fs::read_dir(&dir) {
			Ok(rd)	=> rd.filter_map(|e| e.ok().map(|e| e.path())).collect(),
			Err(_)	=> continue,
		};
		files.sort();
		for f in files {
			let name = f.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
			if !name.ends_with(".typ") || name.starts_with('_') {
				continue;
			}
			let text = std::fs::read_to_string(&f).unwrap_or_default();
			let needs = text.lines()
				.take_while(|l| l.starts_with("//"))
				.filter_map(|l| l.strip_prefix("// needs:"))
				.flat_map(|l| l.split_whitespace().map(|s| s.to_string()).collect::<Vec<_>>())
				.collect();
			let rejects = text.lines().take_while(|l| l.starts_with("//")).any(|l| l.trim() == "// oracle: rejects");
			out.push(Fixture { rel: fmt!("{}/{}", area, name), path: f, needs, rejects });
		}
	}
	out
}

#[test]
fn flow_matches_the_typst_oracle() {
	let only = std::env::var("EVAL_FLOW_FIXTURE").ok();
	let mut compared	= 0usize;
	let mut probes		= 0usize;
	let mut failures	= Vec::new();
	let mut pending		= Vec::new();
	let mut refusals	= 0usize;
	for fx in fixtures() {
		let (rel, path, needs) = (fx.rel, fx.path, fx.needs);
		if let Some(o) = &only {
			if !rel.contains(o.as_str()) {
				continue;
			}
		}
		if fx.rejects {
			let want = match oracle_error(&rel) {
				Ok(m)	=> m,
				Err(e)	=> {
					failures.push(fmt!("{}: {}", rel, e));
					continue;
				},
			};
			match austenite_error(&path) {
				Ok(got) if got == want	=> {
					refusals += 1;
					println!("ok      {} (refused: {})", rel, want);
				},
				Ok(got)	=> failures.push(fmt!("{}:\n    typst:     {}\n    austenite: {}", rel, want, got)),
				Err(e)	=> failures.push(fmt!("{}: {}", rel, e)),
			}
			continue;
		}
		let want = match oracle(&rel) {
			Ok(w)	=> w,
			Err(e)	=> {
				failures.push(fmt!("{}: typst rejected the fixture: {}", rel, e.lines().next().unwrap_or("")));
				continue;
			},
		};
		let got = match austenite(&path) {
			Ok(g)	=> g,
			Err(e)	=> {
				if needs.is_empty() {
					failures.push(fmt!("{}: {}", rel, e));
				} else {
					pending.push(fmt!("{} (needs {}): {}", rel, needs.join(", "), e));
				}
				continue;
			},
		};
		let diffs = compare(&want, &got);
		if diffs.is_empty() {
			compared += 1;
			probes += want.probes.len();
			println!("ok      {} ({} pages, {} probes)", rel, want.pages.len(), want.probes.len());
		} else {
			failures.push(fmt!("{}:\n    {}", rel, diffs.join("\n    ")));
		}
	}
	for p in &pending {
		println!("pending {}", p);
	}
	for f in &failures {
		println!("FAIL    {}", f);
	}
	println!("{} fixtures compared in full, {} probes; {} refusals matched; {} pending; {} failing",
		compared, probes, refusals, pending.len(), failures.len());
	assert!(failures.is_empty(), "{} fixture(s) differ from typst", failures.len());
	if only.is_none() {
		assert!(compared >= MIN_COMPARED, "only {} fixtures were compared in full, fewer than {}", compared, MIN_COMPARED);
	}
}
