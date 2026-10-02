// The synthetic fixture corpus: `tests/fixtures/eval/<area>/<name>.typ`, one area per unit. A file
// whose name starts with `_` is a helper a fixture imports, not a fixture; so is anything in a
// subdirectory. The area directory is the project root on both sides, so a fixture reaches nothing
// outside its own area.
//
// A fixture may narrow what is compared with a directive among its leading comment lines:
//
//     // oracle: levels 1 2      compare values and positions only
//     // oracle: no-html         skip level 3 (Typst's HTML export cannot express the construct)
//     // oracle: rejects         Typst must reject it; Austenite must fail with the same first error
//     // oracle: none            not a document (a unit's case file), so not in the corpus
//
// Levels: 1 values (`#metadata(..) <probe>`), 2 positions of headings, figures, equations,
// footnotes and probes, 3 realised structure against Typst's HTML export, 4 layout (page count,
// line text and baselines).

use oxedyne_fe2o3_core::prelude::*;

use std::path::{
	Path,
	PathBuf,
};

#[derive(Clone, Debug)]
pub struct Fixture {
	pub area:	String,
	pub name:	String,
	pub path:	PathBuf,
	pub root:	PathBuf,	// the area directory: `--root` for typst, `World::root` for Austenite
	pub text:	String,
	pub levels:	[bool; 4],
	pub expect:	Expect,
}

/// What Typst is expected to do with a fixture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Expect {
	Accepts,
	Rejects,	// compared on the first error's message and position, not on any level
}

impl Fixture {
	pub fn id(&self) -> String { fmt!("{}/{}", self.area, self.name) }

	/// Does the fixture ask for level `n` (1-4)?
	pub fn wants(&self, n: usize) -> bool { n >= 1 && n <= 4 && self.levels[n - 1] }

	/// Does anything in it depend on introspection, so its probes can only be read after layout?
	pub fn contextual(&self) -> bool {
		self.text.contains("context") || self.text.contains("show") || self.text.contains("query")
	}
}

pub fn fixtures_dir() -> PathBuf {
	Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join("fixtures").join("eval")
}

/// Which fixtures a run covers: `EVAL_ORACLE_AREA=a,b` limits the areas, `EVAL_ORACLE_FIXTURE=s` the
/// fixtures whose `area/name` contains `s`.
#[derive(Clone, Debug, Default)]
pub struct Filter {
	pub areas:	Option<Vec<String>>,
	pub name:	Option<String>,
}

impl Filter {
	pub fn from_env() -> Self {
		let areas = std::env::var("EVAL_ORACLE_AREA").ok()
			.filter(|s| !s.trim().is_empty())
			.map(|s| s.split(',').map(|a| a.trim().to_string()).collect());
		let name = std::env::var("EVAL_ORACLE_FIXTURE").ok().filter(|s| !s.trim().is_empty());
		Self { areas, name }
	}

	pub fn only(area: &str) -> Self {
		Self { areas: Some(vec![area.to_string()]), name: None }
	}

	fn admits(&self, area: &str, id: &str) -> bool {
		if let Some(a) = &self.areas {
			if !a.iter().any(|x| x == area) {
				return false;
			}
		}
		match &self.name {
			Some(n)	=> id.contains(n.as_str()),
			None	=> true,
		}
	}
}

pub fn discover(filter: &Filter) -> Outcome<Vec<Fixture>> {
	let base = fixtures_dir();
	let mut areas: Vec<PathBuf> = Vec::new();
	let rd = res!(std::fs::read_dir(&base).map_err(|e| err!(
		"Cannot read the fixture corpus at {}: {}", base.display(), e; IO, File, Read)));
	for entry in rd {
		let entry = res!(entry.map_err(|e| err!("Cannot list {}: {}", base.display(), e; IO, File, Read)));
		let p = entry.path();
		if p.is_dir() {
			areas.push(p);
		}
	}
	areas.sort();
	let mut out = Vec::new();
	for dir in areas {
		let area = dir.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
		let mut files: Vec<PathBuf> = Vec::new();
		let rd = res!(std::fs::read_dir(&dir).map_err(|e| err!(
			"Cannot read the fixture area {}: {}", dir.display(), e; IO, File, Read)));
		for entry in rd {
			let entry = res!(entry.map_err(|e| err!("Cannot list {}: {}", dir.display(), e; IO, File, Read)));
			let p = entry.path();
			let fname = p.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
			if p.is_file() && fname.ends_with(".typ") && !fname.starts_with('_') {
				files.push(p);
			}
		}
		files.sort();
		for path in files {
			let name = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
			if !filter.admits(&area, &fmt!("{}/{}", area, name)) {
				continue;
			}
			let text = res!(std::fs::read_to_string(&path).map_err(|e| err!(
				"Cannot read fixture {}: {}", path.display(), e; IO, File, Read)));
			let (levels, expect) = match directives(&text) {
				Some(d)	=> d,
				None	=> continue,
			};
			out.push(Fixture { area: area.clone(), name, path, root: dir.clone(), text, levels, expect });
		}
	}
	Ok(out)
}

// The levels and expectation a fixture's leading directives ask for, or `None` for `oracle: none`.
fn directives(text: &str) -> Option<([bool; 4], Expect)> {
	let mut levels = [true; 4];
	let mut expect = Expect::Accepts;
	for line in text.lines() {
		let t = line.trim();
		if !t.starts_with("//") {
			if t.is_empty() {
				continue;
			}
			break;
		}
		let d = t.trim_start_matches('/').trim();
		let d = match d.strip_prefix("oracle:") {
			Some(d)	=> d.trim(),
			None	=> continue,
		};
		if let Some(list) = d.strip_prefix("levels") {
			levels = [false; 4];
			for n in list.split(|c: char| c == ',' || c.is_whitespace()) {
				if let Ok(n) = n.parse::<usize>() {
					if n >= 1 && n <= 4 {
						levels[n - 1] = true;
					}
				}
			}
		} else if d == "no-html" {
			levels[2] = false;
		} else if d == "rejects" {
			expect = Expect::Rejects;
		} else if d == "none" {
			return None;
		}
	}
	Some((levels, expect))
}

/// Why a fixture cannot be in the corpus, if it cannot: it must be synthetic, so it may not name a
/// book or reach outside its area.
pub fn provenance_fault(f: &Fixture) -> Option<String> {
	// A `../` is not a fault in itself: Typst, run with the area as its root, and the evaluator's own
	// resolver both refuse a path that leaves the root, so a fixture that climbs either stays inside or is
	// rejected by both, and the import corpus tests that refusal.
	for bad in ["usr/books", "/books/"] {
		if f.text.contains(bad) {
			return Some(fmt!("{} names `{}`: fixtures are synthetic and stay inside their area", f.id(), bad));
		}
	}
	None
}

/// Every file in a fixture's area, for the oracle cache key: a helper edit invalidates the area.
pub fn area_bytes(root: &Path) -> Outcome<Vec<(String, Vec<u8>)>> {
	let mut out = Vec::new();
	res!(walk(root, root, &mut out, 0));
	out.sort_by(|a, b| a.0.cmp(&b.0));
	Ok(out)
}

fn walk(base: &Path, dir: &Path, out: &mut Vec<(String, Vec<u8>)>, depth: usize) -> Outcome<()> {
	if depth > 8 {
		return Ok(());
	}
	let rd = res!(std::fs::read_dir(dir).map_err(|e| err!("Cannot list {}: {}", dir.display(), e; IO, File, Read)));
	for entry in rd {
		let entry = res!(entry.map_err(|e| err!("Cannot list {}: {}", dir.display(), e; IO, File, Read)));
		let p = entry.path();
		if p.is_dir() {
			res!(walk(base, &p, out, depth + 1));
		} else if p.is_file() {
			let rel = p.strip_prefix(base).map(|r| r.to_string_lossy().to_string()).unwrap_or_default();
			let bytes = res!(std::fs::read(&p).map_err(|e| err!("Cannot read {}: {}", p.display(), e; IO, File, Read)));
			out.push((rel, bytes));
		}
	}
	Ok(())
}
