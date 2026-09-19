//! The oracle harness -- reproduction as instrument.
//!
//! Before this unit, the Typst-vs-Austenite comparison lived only in a person's hands: compile both,
//! eyeball the PDFs, remember what to check next time. These tests make that repeatable: the bounded
//! corpus (see `tests/oracle/mod.rs`'s [`driver::corpus`]) is compiled with both engines and compared, a
//! fixed trio of fixtures guards three fixes already on `main`, and a baseline file under the harness's
//! own working directory (never `/tmp`, never the crate's `tests/` tree) lets a later run -- this
//! session's or a future one's -- diff against what this one found.
//!
//! The baseline now asserts, not just reports: austenite's own rendered PDF hash and the worst of three
//! sampled pages' raster diff against Typst are compared to what was recorded, and a run that finds
//! either moved fails -- unless `ORACLE_ACCEPT=1` is set in the environment, in which case the new
//! values are re-recorded and printed as an accepted change rather than silently passing. See
//! `tests/oracle/mod.rs`'s `record_and_diff` and `BaselineOutcome`.
//!
//! The reference for the crate-owned corpus roots is now checked in at `tests/oracle/expected.json`, and
//! the harness seeds a fresh (or cleared) cache from it: a pinned root can no longer bootstrap on whatever
//! renders, so a regression on a fresh box fails instead of self-blessing -- the milestone audit's first
//! finding. austenite-doc is deferred (its source is under active authoring), so it still bootstraps.
//!
//! `tests/oracle/mod.rs` (loaded below as `mod driver`, see its own comment for why) is the driver;
//! everything that shells out to `typst`, `pdfinfo`, `pdftoppm`, ImageMagick or `sha256sum` lives there,
//! so this file stays a short statement of what is being asserted and why.

// `#[path]` rather than a plain `mod oracle;`: the crate root of this integration test is itself
// named `oracle` (the file is `tests/oracle.rs`), and a plain `mod oracle;` pointing at the sibling
// `tests/oracle/mod.rs` collides with that -- rustc reports the same module found at both paths. Naming
// the module `driver` here sidesteps the collision; the directory stays `tests/oracle/`, matching the
// unit's own file layout.
#[path = "oracle/mod.rs"]
mod driver;

use driver::{
	BaselineOutcome,
	baseline_path,
	compare_root,
	corpus,
	qc_dir,
	record_and_diff,
	trio,
};

use oxedyne_fe2o3_core::prelude::*;

/// Compiles every corpus root with both `austenite` and the installed `typst`, and asserts:
///
/// - Austenite itself produced at least one page for every root (the one hard stop: every other
///   comparison depends on that page existing).
/// - Where the Typst oracle could run, its page count and its heading/figure pages agree with
///   Austenite's own ledger, order-matched (see `tests/oracle/mod.rs` for why order rather than name).
/// - The corpus's numbers, PDF hash and raster fraction do not drift against the recorded baseline
///   within this run -- a bootstrapping first run for a root always passes, laying the baseline a later
///   run diffs against; a later run that DOES drift fails unless `ORACLE_ACCEPT=1` is set, in which case
///   the drift is printed and the baseline is re-recorded rather than the run failing.
///
/// A root whose oracle comparison could not run at all (`oxeweb-techspec`, at the time of writing -- see
/// [`driver::corpus`]'s doc comment) is reported, not failed: the Austenite-only checks above still ran
/// for it.
#[test]
fn corpus_roots_compile_and_match_the_typst_oracle() -> Outcome<()> {
	let work_dir	= res!(qc_dir());
	let baseline	= baseline_path(&work_dir);
	let mut problems: Vec<String> = Vec::new();
	// `ORACLE_ACCEPT=1` gates every baseline drift this run finds, across every root -- an accepted
	// styling change is expected to move more than one root's PDF hash at once, so this is read once
	// rather than per root.
	let accept = std::env::var("ORACLE_ACCEPT").map(|v| v == "1").unwrap_or(false);

	for root in corpus() {
		let report = match compare_root(&root, &work_dir) {
			Ok(r)	=> r,
			Err(e)	=> {
				problems.push(fmt!("{}: austenite could not compile it at all: {}", root.name, e));
				continue;
			},
		};
		println!("[oracle] {}", report.summary());

		if let Some(note) = &report.oracle_note {
			println!("[oracle] {}: typst oracle unavailable -- {}", report.name, note);
		}
		// Each sampled page's own raster fraction, beyond the worst-of-three figure already folded into
		// `report.summary()` -- visible under `--nocapture` so a failure's root cause (WHICH page moved)
		// does not need a re-run with extra flags to see.
		for (page, pct) in &report.raster_samples {
			println!("[oracle] {}: page {} raster diff {:.2}%", report.name, page, pct);
		}
		for m in &report.mismatches {
			problems.push(fmt!("{}: {} (see {:?})", report.name, m, report.pdf_path));
		}
		match res!(record_and_diff(&baseline, &report, accept)) {
			BaselineOutcome::Bootstrapped	=> println!("[oracle] {}: baseline recorded (first run for this root)", report.name),
			BaselineOutcome::Unchanged		=> {},
			BaselineOutcome::Accepted(msg)	=> println!("[oracle] {}: ACCEPTED (ORACLE_ACCEPT=1), baseline re-recorded", msg),
			BaselineOutcome::Rejected(msg)	=> problems.push(msg),
		}
	}

	if !problems.is_empty() {
		return Err(err!("The oracle harness found {} problem(s):\n{}",
			problems.len(), problems.join("\n"); Test, Mismatch));
	}
	Ok(())
}

/// The three fixed no-regression fixtures -- see `tests/oracle/trio.rs` for what each guards.
#[test]
fn no_regression_trio_holds() -> Outcome<()> {
	let work_dir = res!(qc_dir());
	res!(trio::inline_maths_gallery_renders(&work_dir));
	res!(trio::display_equation_cases_render());
	res!(trio::pdf_stays_compact_and_extractable(&work_dir));
	Ok(())
}

/// The harness's own self-test: a [`driver::Baseline`] written to the working directory reads back
/// exactly what was written, including its PDF hash and raster fields, and a changed entry persists
/// through a rewrite -- the round-trip the real regression check above depends on.
#[test]
fn baseline_records_and_diffs_correctly() -> Outcome<()> {
	use driver::{Baseline, BaselineEntry};
	use std::collections::BTreeMap;

	let work_dir	= res!(qc_dir());
	let path		= work_dir.join("baseline-selftest.json");

	let entry = |pages: usize, hash: &str, raster: Option<f64>| BaselineEntry {
		pages, anchors: 7, pdf_sha256: hash.to_string(), raster_worst_pct: raster,
	};

	let mut entries = BTreeMap::new();
	entries.insert("fixture-root".to_string(), entry(3, "aaaa1111", Some(1.25)));
	let baseline = Baseline::from_entries(entries);
	res!(baseline.write_to_file(&path));

	let read_back = res!(Baseline::read_from_file(&path));
	if read_back.get("fixture-root") != Some(entry(3, "aaaa1111", Some(1.25))) {
		return Err(err!("A baseline did not round-trip through {:?}: got {:?}",
			path, read_back.get("fixture-root"); Test, Mismatch));
	}

	// `record_and_diff` works off a `RootReport`, which only `compare_root` builds (see
	// `oracle_accept_gates_and_rerecords_a_changed_baseline` below for that path); this self-test
	// exercises the underlying read-compare-write round trip directly on `Baseline`, including a root
	// whose raster field was never measured (`None`) -- a box with no ImageMagick's own shape.
	let mut moved = read_back;
	moved.set("fixture-root", entry(4, "bbbb2222", None));
	res!(moved.write_to_file(&path));
	let after = res!(Baseline::read_from_file(&path));
	if after.get("fixture-root") != Some(entry(4, "bbbb2222", None)) {
		return Err(err!("A changed baseline entry did not persist through {:?}.", path; Test, Mismatch));
	}

	let _ = std::fs::remove_file(&path);
	Ok(())
}

/// The accept-gate flow [`driver::record_and_diff`] implements, driven directly off two synthetic
/// [`driver::RootReport`]s (building a real one means compiling something, which the harness-level tests
/// above already cover): a first report bootstraps the baseline; a second report with a different PDF
/// hash and a moved raster fraction is REJECTED, unchanged on disk, when `accept` is `false`, and
/// ACCEPTED and re-recorded when `accept` is `true` -- the exact two branches item 1 of this unit's brief
/// asks be provable, not just asserted by inspection.
#[test]
fn oracle_accept_gates_and_rerecords_a_changed_baseline() -> Outcome<()> {
	use driver::{Baseline, BaselineOutcome, RootReport, record_and_diff};
	use std::path::PathBuf;

	let work_dir	= res!(qc_dir());
	let path		= work_dir.join("baseline-accept-selftest.json");
	let _ = std::fs::remove_file(&path);	// a stale file from an earlier aborted run must not leak in

	let report = |hash: &str, raster: Option<f64>| RootReport {
		name:				"accept-fixture-root",
		austenite_pages:	3,
		austenite_anchors:	5,
		pdf_sha256:			hash.to_string(),
		raster_samples:		Vec::new(),
		raster_worst_pct:	raster,
		skip_line:			None,
		typst_pages:		Some(3),
		oracle_note:		None,
		mismatches:			Vec::new(),
		raster_note:		None,
		pdf_path:			PathBuf::from("/nonexistent/accept-fixture.pdf"),	// never read by record_and_diff
	};

	// First run: nothing recorded yet, so this bootstraps regardless of `accept`.
	match res!(record_and_diff(&path, &report("hash-one", Some(2.0)), false)) {
		BaselineOutcome::Bootstrapped	=> {},
		_								=> return Err(err!("The first run for a new root was not a bootstrap."; Test, Mismatch)),
	}

	// Second run, same values: unchanged, whichever way `accept` is set.
	match res!(record_and_diff(&path, &report("hash-one", Some(2.0)), false)) {
		BaselineOutcome::Unchanged	=> {},
		_							=> return Err(err!("An unchanged report was not reported unchanged."; Test, Mismatch)),
	}

	// A changed hash and raster fraction, NOT accepted: rejected, and the baseline on disk stays at the
	// first run's values.
	match res!(record_and_diff(&path, &report("hash-two", Some(9.0)), false)) {
		BaselineOutcome::Rejected(msg) => {
			if !msg.contains("sha256") || !msg.contains("raster") {
				return Err(err!("A rejected drift's message named neither hash nor raster: {:?}", msg; Test, Mismatch));
			}
		},
		_ => return Err(err!("A changed, unaccepted report was not rejected."; Test, Mismatch)),
	}
	let baseline = res!(Baseline::read_from_file(&path));
	let still_first = baseline.get("accept-fixture-root");
	if still_first.as_ref().map(|e| e.pdf_sha256.as_str()) != Some("hash-one") {
		return Err(err!("A rejected drift's baseline was rewritten anyway: {:?}", still_first; Test, Mismatch));
	}

	// The same changed report, NOW accepted: re-recorded, and the baseline on disk moves to it.
	match res!(record_and_diff(&path, &report("hash-two", Some(9.0)), true)) {
		BaselineOutcome::Accepted(msg) => {
			if !msg.contains("sha256") || !msg.contains("raster") {
				return Err(err!("An accepted drift's message named neither hash nor raster: {:?}", msg; Test, Mismatch));
			}
		},
		_ => return Err(err!("A changed, accepted report was not accepted."; Test, Mismatch)),
	}
	let baseline = res!(Baseline::read_from_file(&path));
	let now_second = baseline.get("accept-fixture-root");
	if now_second.as_ref().map(|e| e.pdf_sha256.as_str()) != Some("hash-two") {
		return Err(err!("An accepted drift's baseline was not re-recorded: {:?}", now_second; Test, Mismatch));
	}

	let _ = std::fs::remove_file(&path);
	Ok(())
}

/// The in-tree pin: a root listed in `tests/oracle/expected.json` seeds its reference from that tracked
/// file when the cache has no entry, so on a fresh (or cleared) cache a matching render is `Unchanged` and
/// a regressed one is `Rejected` -- never the always-pass `Bootstrapped`. A root absent from the file
/// (austenite-doc, under active authoring) still bootstraps. This is the milestone audit's first finding
/// fixed: a fresh box can no longer silently re-baseline on whatever it renders.
#[test]
fn expected_json_pin_blocks_bootstrap_for_crate_owned_roots() -> Outcome<()> {
	use driver::{BaselineOutcome, RootReport, record_and_diff};
	use std::path::PathBuf;

	let work_dir	= res!(qc_dir());
	let path		= work_dir.join("baseline-expected-selftest.json");

	let report = |name: &'static str, hash: &str| RootReport {
		name,
		austenite_pages:	1,
		austenite_anchors:	4,
		pdf_sha256:			hash.to_string(),
		raster_samples:		Vec::new(),
		raster_worst_pct:	None,
		skip_line:			None,
		typst_pages:		Some(1),
		oracle_note:		None,
		mismatches:			Vec::new(),
		raster_note:		None,
		pdf_path:			PathBuf::from("/nonexistent/expected-selftest.pdf"),
	};

	// styling-fixture is pinned; this is exactly its hash in tests/oracle/expected.json.
	const FIXTURE_HASH: &str = "d035bd3c146435ef5571d26708f274b94dc412e9914317bbc98624c5db45d53f";

	// A matching render on a fresh cache is Unchanged (the reference came from expected.json), not Bootstrapped.
	let _ = std::fs::remove_file(&path);
	match res!(record_and_diff(&path, &report("styling-fixture", FIXTURE_HASH), false)) {
		BaselineOutcome::Unchanged	=> {},
		_							=> return Err(err!(
			"a pinned root matching expected.json was not Unchanged -- it bootstrapped or rejected instead";
			Test, Mismatch)),
	}

	// A regressed render on a fresh cache is Rejected, not Bootstrapped: the pin blocks self-blessing.
	let _ = std::fs::remove_file(&path);
	match res!(record_and_diff(&path, &report("styling-fixture", "deadbeefdeadbeef"), false)) {
		BaselineOutcome::Rejected(_)	=> {},
		_							=> return Err(err!(
			"a regressed pinned root was not Rejected -- the in-tree pin did not block the bootstrap";
			Test, Mismatch)),
	}

	// A root absent from expected.json (austenite-doc's shape) still bootstraps.
	let _ = std::fs::remove_file(&path);
	match res!(record_and_diff(&path, &report("not-a-pinned-root", "abc123"), false)) {
		BaselineOutcome::Bootstrapped	=> {},
		_							=> return Err(err!("a non-pinned root did not bootstrap"; Test, Mismatch)),
	}

	// Q6: a cache that self-blessed a WRONG hash for a pinned root does not govern it -- the tracked
	// expected.json is authoritative even when a cache entry exists, so the correct render is still
	// Unchanged (matched against expected, not the poisoned cache) rather than the cache masking a drift.
	{
		use driver::{Baseline, BaselineEntry};
		use std::collections::BTreeMap;
		let mut seeded = BTreeMap::new();
		seeded.insert("styling-fixture".to_string(), BaselineEntry {
			pages:				1,
			anchors:			4,
			pdf_sha256:			"cache-self-blessed-wrong-hash".to_string(),
			raster_worst_pct:	None,
		});
		res!(Baseline::from_entries(seeded).write_to_file(&path));
	}
	match res!(record_and_diff(&path, &report("styling-fixture", FIXTURE_HASH), false)) {
		BaselineOutcome::Unchanged	=> {},
		_							=> return Err(err!(
			"a pinned root was governed by a poisoned cache instead of expected.json";
			Test, Mismatch)),
	}

	let _ = std::fs::remove_file(&path);
	Ok(())
}
