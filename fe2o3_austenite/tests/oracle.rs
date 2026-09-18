//! The oracle harness -- reproduction as instrument.
//!
//! Before this unit, the Typst-vs-Austenite comparison lived only in a person's hands: compile both,
//! eyeball the PDFs, remember what to check next time. These three tests make that repeatable: the
//! bounded corpus (see `tests/oracle/mod.rs`'s [`driver::corpus`]) is compiled with both engines and
//! compared, a fixed trio of fixtures guards three fixes already on `main`, and a baseline file under
//! the harness's own working directory (never `/tmp`, never the crate's `tests/` tree) lets a later run
//! -- this session's or a future one's -- diff against what this one found.
//!
//! `tests/oracle/mod.rs` (loaded below as `mod driver`, see its own comment for why) is the driver;
//! everything that shells out to `typst`, `pdfinfo`, `pdftoppm` or ImageMagick lives there, so this
//! file stays a short statement of what is being asserted and why.

// `#[path]` rather than a plain `mod oracle;`: the crate root of this integration test is itself
// named `oracle` (the file is `tests/oracle.rs`), and a plain `mod oracle;` pointing at the sibling
// `tests/oracle/mod.rs` collides with that -- rustc reports the same module found at both paths. Naming
// the module `driver` here sidesteps the collision; the directory stays `tests/oracle/`, matching the
// unit's own file layout.
#[path = "oracle/mod.rs"]
mod driver;

use driver::{
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
/// - The corpus's numbers do not drift against the recorded baseline within this run -- a bootstrapping
///   first run for a root always passes, laying the baseline a later unit diffs against.
///
/// A root whose oracle comparison could not run at all (the two `oxeweb` roots, at the time of writing
/// -- see [`driver::corpus`]'s doc comment) is reported, not failed: the Austenite-only checks above
/// still ran for it.
#[test]
fn corpus_roots_compile_and_match_the_typst_oracle() -> Outcome<()> {
	let work_dir	= res!(qc_dir());
	let baseline	= baseline_path(&work_dir);
	let mut problems: Vec<String> = Vec::new();

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
		for m in &report.mismatches {
			problems.push(fmt!("{}: {} (see {:?})", report.name, m, report.pdf_path));
		}
		if let Some(drift) = res!(record_and_diff(&baseline, &report)) {
			problems.push(drift);
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
/// exactly what was written, and [`driver::record_and_diff`] reports no drift the first time a root is
/// seen but does report one when a second write disagrees with the first -- the two behaviours the real
/// regression check above depends on.
#[test]
fn baseline_records_and_diffs_correctly() -> Outcome<()> {
	use driver::{Baseline, BaselineEntry};
	use std::collections::BTreeMap;

	let work_dir	= res!(qc_dir());
	let path		= work_dir.join("baseline-selftest.json");

	let mut entries = BTreeMap::new();
	entries.insert("fixture-root".to_string(), BaselineEntry { pages: 3, anchors: 7 });
	let baseline = Baseline::from_entries(entries);
	res!(baseline.write_to_file(&path));

	let read_back = res!(Baseline::read_from_file(&path));
	if read_back.get("fixture-root") != Some(BaselineEntry { pages: 3, anchors: 7 }) {
		return Err(err!("A baseline did not round-trip through {:?}: got {:?}",
			path, read_back.get("fixture-root"); Test, Mismatch));
	}

	// `record_and_diff` works off a `RootReport`, which only `compare_root` builds; the self-test
	// exercises the same read-compare-write path directly on `Baseline`, since building a real
	// `RootReport` here would mean compiling something.
	let mut moved = read_back;
	moved.set("fixture-root", BaselineEntry { pages: 4, anchors: 7 });
	res!(moved.write_to_file(&path));
	let after = res!(Baseline::read_from_file(&path));
	if after.get("fixture-root") != Some(BaselineEntry { pages: 4, anchors: 7 }) {
		return Err(err!("A changed baseline entry did not persist through {:?}.", path; Test, Mismatch));
	}

	let _ = std::fs::remove_file(&path);
	Ok(())
}
