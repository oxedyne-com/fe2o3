// The structure oracle harness (U12): each synthetic fixture under `tests/fixtures/eval/<area>/` is
// run through the host's `typst` 0.15.x and through Austenite's evaluator, and the two are compared
// at four levels -- values, positions, realised structure and layout (see `corpus.rs` for the
// levels and the fixture directives). Conformance is reported as numbers, per area and level, so
// "as faithfully as it can" is measurable; `EVAL_ORACLE_STRICT` turns any shortfall in the named
// areas into a failure, which is how a unit gates its own area and how U11 gates the whole.
//
// Environment:
//
//     EVAL_ORACLE_AREA=a,b        only these areas
//     EVAL_ORACLE_FIXTURE=s       only fixtures whose `area/name` contains `s`
//     EVAL_ORACLE_STRICT=a,b|all  every compared level must pass in these areas
//     EVAL_ORACLE_TIMEOUT=secs    per fixture on Austenite's side (default 60)
//     EVAL_ORACLE_SKIP=1          skip, explicitly, when no typst 0.15.x exists
//     EVAL_ORACLE_TYPST=path      the typst binary (default `typst` on the path)
//     EVAL_ORACLE_CAP=3G|none     the memory cap each typst run is given
//
// A panic or an unbounded run on Austenite's side, and a fixture Typst itself rejects, fail the
// suite whatever the strictness: the first two are engine faults, the last a corpus fault.

pub mod austenite;
pub mod corpus;
pub mod json;
pub mod layout;
pub mod markup;
pub mod oracle;
pub mod structure;

use austenite::{
	ProbeSource,
	RunEnd,
	Want,
};
use corpus::Fixture;
use json::J;
use oracle::Oracle;

use oxedyne_fe2o3_core::prelude::*;

use std::time::Duration;

pub const POS_TOL: f64 = 0.5;	// points, the design's level-2 tolerance

/// One located element: `func` is the element's name as `repr(it.func())` prints it.
#[derive(Clone, Debug)]
pub struct PosRow {
	pub func:	String,
	pub page:	u32,
	pub x:		f64,
	pub y:		f64,
}

#[derive(Clone, Debug)]
pub enum Verdict {
	Pass,
	Fail(Vec<String>),
	NotApplicable(String),	// nothing to compare: no probes, or Typst's HTML export drops the construct
	NotRequested,
}

impl Verdict {
	pub fn is_pass(&self) -> bool { matches!(self, Verdict::Pass) }

	pub fn counts(&self) -> bool { matches!(self, Verdict::Pass | Verdict::Fail(_)) }

	fn tag(&self) -> &'static str {
		match self {
			Verdict::Pass				=> "pass",
			Verdict::Fail(_)			=> "FAIL",
			Verdict::NotApplicable(_)	=> "n/a",
			Verdict::NotRequested		=> "-",
		}
	}
}

#[derive(Clone, Debug)]
pub struct FixtureReport {
	pub id:			String,
	pub area:		String,
	pub levels:		[Verdict; 4],
	pub fault:		Option<String>,	// panic, timeout or an oracle rejection: fails the suite always
	pub notes:		Vec<String>,	// Austenite's evaluation and layout errors, and its diagnostics
	pub probe_src:	Option<ProbeSource>,
}

pub fn timeout() -> Duration {
	let s = std::env::var("EVAL_ORACLE_TIMEOUT").ok().and_then(|v| v.parse::<u64>().ok()).unwrap_or(60);
	Duration::from_secs(s.max(1))
}

/// Compares one fixture at every level it asks for.
pub fn check(fx: &Fixture, oracle: &Oracle, limit: Duration) -> Outcome<FixtureReport> {
	let mut rep = FixtureReport {
		id:			fx.id(),
		area:		fx.area.clone(),
		levels:		[Verdict::NotRequested, Verdict::NotRequested, Verdict::NotRequested, Verdict::NotRequested],
		fault:		None,
		notes:		Vec::new(),
		probe_src:	None,
	};
	if let Some(f) = corpus::provenance_fault(fx) {
		rep.fault = Some(f);
		return Ok(rep);
	}
	if let Err(e) = res!(oracle.compiles(fx)) {
		rep.fault = Some(fmt!("typst rejects the fixture: {}", e));
		return Ok(rep);
	}

	// The oracle first: what it has nothing to say about, Austenite is not asked.
	let o1 = if fx.wants(1) { Some(res!(oracle.probes(fx))) } else { None };
	let o2 = if fx.wants(2) { Some(res!(oracle.positions(fx))) } else { None };
	let o3 = if fx.wants(3) { Some(res!(oracle.html(fx))) } else { None };
	let o4 = if fx.wants(4) { Some(res!(oracle.layout(fx))) } else { None };
	let mut want = Want { levels: [false; 4] };
	match &o1 {
		Some(Ok(v)) if v.is_empty()	=> rep.levels[0] = Verdict::NotApplicable("no <probe> values".to_string()),
		Some(Ok(_))					=> want.levels[0] = true,
		Some(Err(e))				=> rep.fault = Some(fmt!("typst cannot answer the probe query: {}", e)),
		None						=> (),
	}
	match &o2 {
		Some(Ok(v)) if v.is_empty()	=> rep.levels[1] = Verdict::NotApplicable("no located elements".to_string()),
		Some(Ok(_))					=> want.levels[1] = true,
		Some(Err(e))				=> rep.fault = Some(fmt!("typst cannot answer the position query: {}", e)),
		None						=> (),
	}
	let lossy: Vec<String> = match &o3 {
		Some(Ok(h))	=> h.ignored.iter().filter(|n| !structure::HARMLESS.contains(&n.as_str())).cloned().collect(),
		_			=> Vec::new(),
	};
	match &o3 {
		Some(Ok(_)) if !lossy.is_empty() => rep.levels[2] = Verdict::NotApplicable(fmt!(
			"typst's HTML export drops {}", lossy.join(", "))),
		Some(Ok(_))					=> want.levels[2] = true,
		Some(Err(e))				=> rep.levels[2] = Verdict::NotApplicable(fmt!("typst's HTML export fails: {}", e)),
		None						=> (),
	}
	match &o4 {
		Some(Ok(_))					=> want.levels[3] = true,
		Some(Err(e))				=> rep.fault = Some(fmt!("typst cannot lay the fixture out: {}", e)),
		None						=> (),
	}
	if rep.fault.is_some() || !want.levels.iter().any(|w| *w) {
		return Ok(rep);
	}

	let out = match austenite::run(fx, want, limit) {
		RunEnd::Done(out)	=> out,
		RunEnd::Panic(p)	=> {
			rep.fault = Some(fmt!("austenite panicked: {}", p));
			return Ok(rep);
		}
		RunEnd::Timeout(s)	=> {
			rep.fault = Some(fmt!("austenite did not finish within {}s", s));
			return Ok(rep);
		}
	};
	if let Some(e) = &out.layout {
		rep.notes.push(fmt!("layout: {}", e));
	}
	rep.notes.extend(out.diags.iter().map(|d| fmt!("diagnostic: {}", d)));

	let failed = |why: &str| Verdict::Fail(vec![why.to_string()]);
	let eval_failed = out.eval.as_ref().map(|e| fmt!("evaluation failed: {}", json::clip(e)));

	if want.levels[0] {
		rep.levels[0] = match (&o1, &out.probes, &eval_failed) {
			(_, _, Some(e))	=> failed(e),
			(Some(Ok(w)), Some(Ok((g, src))), _) => {
				rep.probe_src = Some(*src);
				let mut d = Vec::new();
				json::diff("probes", &J::Arr(w.clone()), &J::Arr(g.clone()), &mut d);
				if d.is_empty() { Verdict::Pass } else { Verdict::Fail(d) }
			}
			(_, Some(Err(e)), _)	=> failed(e),
			_						=> failed("no probe values"),
		};
	}
	if want.levels[1] {
		rep.levels[1] = match (&o2, &out.positions, &eval_failed) {
			(_, _, Some(e))	=> failed(e),
			(Some(Ok(w)), Some(Ok(g)), _) => {
				let d = compare_positions(w, g);
				if d.is_empty() { Verdict::Pass } else { Verdict::Fail(d) }
			}
			(_, Some(Err(e)), _)	=> failed(e),
			_						=> failed("no positions"),
		};
	}
	if want.levels[2] {
		rep.levels[2] = match (&o3, &out.skeleton, &eval_failed) {
			(_, _, Some(e))	=> failed(e),
			(Some(Ok(w)), Some(Ok(g)), _) => {
				let mut d = Vec::new();
				structure::compare(&w.skeleton, g, &mut d);
				if d.is_empty() { Verdict::Pass } else { Verdict::Fail(d) }
			}
			(_, Some(Err(e)), _)	=> failed(e),
			_						=> failed("no realised structure"),
		};
	}
	if want.levels[3] {
		rep.levels[3] = match (&o4, &out.pages, &eval_failed) {
			(_, _, Some(e))	=> failed(e),
			(Some(Ok(w)), Some(Ok(g)), _) => {
				let mut d = Vec::new();
				layout::compare(w, g, &mut d);
				if d.is_empty() { Verdict::Pass } else { Verdict::Fail(d) }
			}
			(_, Some(Err(e)), _)	=> failed(e),
			_						=> failed("no pages"),
		};
	}
	Ok(rep)
}

/// Level-2 differences: the same elements in the same order, on the same page, within [`POS_TOL`].
pub fn compare_positions(want: &[PosRow], got: &[PosRow]) -> Vec<String> {
	let mut out = Vec::new();
	if want.len() != got.len() {
		out.push(fmt!("typst locates {} element(s), austenite {}: typst [{}], austenite [{}]",
			want.len(), got.len(),
			want.iter().map(|r| r.func.as_str()).collect::<Vec<_>>().join(", "),
			got.iter().map(|r| r.func.as_str()).collect::<Vec<_>>().join(", ")));
	}
	for (i, (w, g)) in want.iter().zip(got.iter()).enumerate() {
		if w.func != g.func {
			out.push(fmt!("#{}: typst {}, austenite {}", i, w.func, g.func));
			continue;
		}
		if w.page != g.page {
			out.push(fmt!("#{} {}: page typst {}, austenite {}", i, w.func, w.page, g.page));
			continue;
		}
		if !((w.x - g.x).abs() <= POS_TOL && (w.y - g.y).abs() <= POS_TOL) {
			out.push(fmt!("#{} {} on page {}: typst ({:.2}, {:.2})pt, austenite ({:.2}, {:.2})pt",
				i, w.func, w.page, w.x, w.y, g.x, g.y));
		}
	}
	out.truncate(20);
	out
}

// Scoreboard

/// Which areas must pass every compared level: `EVAL_ORACLE_STRICT=all` or a list.
pub fn strict(area: &str) -> bool {
	match std::env::var("EVAL_ORACLE_STRICT") {
		Ok(v) if v.trim() == "all" || v.trim() == "1"	=> true,
		Ok(v)	=> v.split(',').any(|a| a.trim() == area),
		Err(_)	=> false,
	}
}

pub struct Board {
	pub reports:	Vec<FixtureReport>,
}

impl Board {
	/// Passes and compared fixtures for one level, over the reports `pick` admits.
	pub fn score<F: Fn(&FixtureReport) -> bool>(&self, level: usize, pick: F) -> (usize, usize) {
		let mut pass = 0;
		let mut n = 0;
		for r in self.reports.iter().filter(|r| pick(r)) {
			let v = &r.levels[level];
			if v.counts() {
				n += 1;
				if v.is_pass() {
					pass += 1;
				}
			}
		}
		(pass, n)
	}

	pub fn areas(&self) -> Vec<String> {
		let mut a: Vec<String> = self.reports.iter().map(|r| r.area.clone()).collect();
		a.dedup();
		a
	}

	pub fn print(&self) {
		println!("[eval-oracle] {:<36} {:>5} {:>5} {:>5} {:>5}  notes", "fixture", "L1", "L2", "L3", "L4");
		for r in &self.reports {
			let src = match r.probe_src {
				Some(ProbeSource::Content)	=> " (L1 read from evaluated content)",
				_							=> "",
			};
			println!("[eval-oracle] {:<36} {:>5} {:>5} {:>5} {:>5}  {}{}", r.id,
				r.levels[0].tag(), r.levels[1].tag(), r.levels[2].tag(), r.levels[3].tag(),
				r.fault.clone().unwrap_or_default(), src);
			for (i, v) in r.levels.iter().enumerate() {
				match v {
					Verdict::Fail(d)			=> for m in d.iter().take(6) {
						println!("[eval-oracle]     L{}: {}", i + 1, m);
					},
					Verdict::NotApplicable(w)	=> println!("[eval-oracle]     L{} n/a: {}", i + 1, w),
					_							=> (),
				}
			}
			for n in r.notes.iter().take(4) {
				println!("[eval-oracle]     {}", json::clip(n));
			}
		}
		for area in self.areas() {
			let cells: Vec<String> = (0..4).map(|l| {
				let (p, n) = self.score(l, |r| r.area == area);
				fmt!("L{} {}/{}", l + 1, p, n)
			}).collect();
			println!("[eval-oracle] area {:<16} {}{}", area, cells.join("  "),
				if strict(&area) { "  (strict)" } else { "" });
		}
		let cells: Vec<String> = (0..4).map(|l| {
			let (p, n) = self.score(l, |_| true);
			fmt!("L{} {}/{}", l + 1, p, n)
		}).collect();
		println!("[eval-oracle] conformance        {}", cells.join("  "));
	}

	/// The machine-readable scoreboard, for the orchestrator and the U11 gate.
	pub fn to_json(&self) -> J {
		let fixtures = self.reports.iter().map(|r| {
			let levels = r.levels.iter().map(|v| match v {
				Verdict::Fail(d)			=> J::Obj(vec![
					("verdict".to_string(), J::str("fail")),
					("why".to_string(), J::Arr(d.iter().map(|m| J::Str(m.clone())).collect())),
				]),
				Verdict::NotApplicable(w)	=> J::Obj(vec![
					("verdict".to_string(), J::str("n/a")),
					("why".to_string(), J::Arr(vec![J::Str(w.clone())])),
				]),
				other						=> J::Obj(vec![("verdict".to_string(), J::str(other.tag()))]),
			}).collect();
			J::Obj(vec![
				("id".to_string(), J::Str(r.id.clone())),
				("levels".to_string(), J::Arr(levels)),
				("fault".to_string(), r.fault.clone().map(J::Str).unwrap_or(J::Null)),
				("notes".to_string(), J::Arr(r.notes.iter().map(|n| J::Str(n.clone())).collect())),
			])
		}).collect();
		let totals = (0..4).map(|l| {
			let (p, n) = self.score(l, |_| true);
			J::Obj(vec![("pass".to_string(), J::Int(p as i64)), ("compared".to_string(), J::Int(n as i64))])
		}).collect();
		J::Obj(vec![("totals".to_string(), J::Arr(totals)), ("fixtures".to_string(), J::Arr(fixtures))])
	}

	/// Why the run fails: every fault, and every shortfall in a strict area.
	pub fn failures(&self) -> Vec<String> {
		let mut out = Vec::new();
		for r in &self.reports {
			if let Some(f) = &r.fault {
				out.push(fmt!("{}: {}", r.id, f));
			}
			if strict(&r.area) {
				for (i, v) in r.levels.iter().enumerate() {
					if let Verdict::Fail(d) = v {
						out.push(fmt!("{} L{}: {}", r.id, i + 1, d.first().cloned().unwrap_or_default()));
					}
				}
			}
		}
		out
	}
}
