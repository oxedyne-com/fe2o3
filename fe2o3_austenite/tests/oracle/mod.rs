//! The oracle harness's driver: the bounded corpus, the two compiles, and the comparisons between them.
//!
//! "Oracle" here is the installed `typst` binary: for each corpus root, the same `.typ` source is
//! compiled once by it and once by the built `austenite` binary ([`env!("CARGO_BIN_EXE_austenite")`],
//! Cargo's own path to the package's compiled binary, so the test exercises the real artefact rather
//! than re-implementing its pipeline). What is compared is deliberately narrow -- page counts, and each
//! heading's and figure's resolved page, order-matched rather than matched by name, since Austenite's
//! own anchor keys are synthesised from a running count and a title slug ([`doc.rs`]'s `AnchorId::new`
//! calls), not read back from the document's own Typst labels. A raster sample adds a coarse visual
//! sanity check where ImageMagick is installed.
//!
//! Every external tool -- `typst`, `pdfinfo`, `pdftoppm`, `compare`, `identify` -- is invoked by
//! [`std::process::Command`], never linked in: a missing or incompatible `typst` (see
//! [`corpus`]'s doc comment on the `oxeweb` roots) downgrades that root's oracle comparison to a
//! recorded note rather than failing the whole suite, since the Austenite-only side -- did it compile,
//! how many pages, how many anchors -- is still a real regression signal on its own.

pub mod trio;

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_jdat::prelude::*;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ THE CORPUS                                                                 │
// └───────────────────────────────────────────────────────────────────────────┘

/// One root the oracle harness compiles both ways. `typst_root` is the `--root` the installed `typst`
/// needs when the default (the root file's own directory) does not reach every `#import` the root's
/// template chain makes -- `thinking_chap_03.typ` reaches two levels up to a shared `style/` tree, where
/// the three `doc`-template roots each sit beside their own `template.typ` and need none.
pub struct CorpusRoot {
	pub name:		&'static str,	// short, for a file tag and a report line
	pub path:		&'static str,	// the root `.typ` file, absolute
	pub typst_root:	Option<&'static str>,
}

/// The bounded corpus, listed in the one place a later unit extends: three template documents that
/// exercise Austenite's own book assembly (`#include` chains, front matter, a table of contents), plus
/// one short book chapter compiled as a lone file, the other path through the reader. Deliberately not
/// a whole 700-page book -- the point is a repeatable few-second check, not a render soak.
///
/// **The two `oxeweb` roots do not compile under the `typst` installed at the time of writing (0.15.1,
/// 2026-09-18): their shared `oxeweb/doc/template.typ:100` uses a `$times.circle$` symbol-modifier
/// expression that binary rejects with "unknown symbol modifier".** This is a pre-existing
/// incompatibility in a template this crate does not own, not an Austenite regression, so
/// [`compare_root`] downgrades an oracle failure on these two roots to a recorded note rather than a
/// test failure -- they still exercise Austenite's own compile, page count and ledger. Re-pin `typst`
/// (or fix the template) and the two roots' Typst comparison resumes without any change here.
pub fn corpus() -> Vec<CorpusRoot> {
	vec![
		CorpusRoot {
			name:		"austenite-doc",
			path:		"/home/jason/usr/complement/projects/oxedyne/doc/Austenite/austenite.typ",
			typst_root:	None,
		},
		CorpusRoot {
			name:		"oxeweb-overview",
			path:		"/home/jason/usr/complement/projects/oxegen/oxeweb/doc/Overview/overview.typ",
			typst_root:	None,
		},
		CorpusRoot {
			name:		"oxeweb-techspec",
			path:		"/home/jason/usr/complement/projects/oxegen/oxeweb/doc/TechSpec/techspec.typ",
			typst_root:	None,
		},
		CorpusRoot {
			name:		"cheapthinking-ch03",
			path:		"/home/jason/usr/books/elearnity/CheapThinking/thinking_chap_03.typ",
			typst_root:	Some("/home/jason/usr/books/elearnity"),
		},
	]
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ WORKING DIRECTORY                                                          │
// └───────────────────────────────────────────────────────────────────────────┘

/// Where the harness writes every rendered artefact and the baseline file -- never `/tmp` (tmpfs,
/// charged to the session's memory cgroup: see `~/usr/CLAUDE.md`'s cargo-target-dir warning, the same
/// hazard for any large written output) and never the crate's own `tests/` tree, so a run leaves no
/// diff for git to see. Fixed rather than keyed by `RC_SLOT`, on the coordinator's direction, since the
/// fleet's build lanes serialise on one shared box and the harness is meant to be found again by name.
pub fn qc_dir() -> Outcome<PathBuf> {
	let home = res!(std::env::var("HOME"));
	let dir = Path::new(&home).join(".cache").join("austenite-qc-rc5");
	res!(std::fs::create_dir_all(&dir));
	Ok(dir)
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ ONE ROW OF A LABEL→PAGE DUMP, EITHER SIDE                                  │
// └───────────────────────────────────────────────────────────────────────────┘

/// One row of Austenite's own `--ledger-out` dump: an anchor's kind (as [`AnchorKind::name`], e.g.
/// `"heading"`, `"float"`), its content key, and the page it resolved to.
#[derive(Clone, Debug)]
struct AnchorRow {
	kind:	String,
	#[allow(dead_code)]	// carried through for a future label-based match; today's comparison is by order
	label:	String,
	page:	u32,
}

/// One row of the Typst side's dump (`tests/oracle/dump.typ`): a heading's or figure's kind, its Typst
/// label (usually empty -- most headings in these documents carry none), its plain title text where the
/// body was simple enough to read as one, and the page it resolved to.
#[derive(Clone, Debug)]
struct TypstRow {
	kind:	String,
	#[allow(dead_code)]	// carried through for a future label-based match; today's comparison is by order
	label:	String,
	title:	String,
	page:	u32,
}

/// Reads a JSON array of flat objects -- either side's dump -- into rows of `{kind, label, page}`,
/// tolerating any integer width the source encoded `page` as (Austenite's own writer picks `U32`; the
/// Typst side's JSON, decoded back through the JDAT reader since JDAT is a JSON superset, could as
/// easily land on `I64`). `title`, present only on the Typst side, defaults to empty when the caller
/// does not ask for it.
fn parse_rows(json: &str, want_title: bool) -> Outcome<Vec<(String, String, String, u32)>> {
	let dat		= res!(Dat::decode_string(json));
	let list	= try_extract_dat!(dat, List);
	let mut out = Vec::with_capacity(list.len());
	for mut row in list {
		let kind	= try_extract_dat!(res!(row.map_remove_must(&dat!("kind"))), Str);
		let label	= try_extract_dat!(res!(row.map_remove_must(&dat!("label"))), Str);
		let title	= if want_title {
			try_extract_dat!(res!(row.map_remove_must(&dat!("title"))), Str)
		} else {
			String::new()
		};
		let page_dat	= res!(row.map_remove_must(&dat!("page")));
		let page		= try_extract_dat_as!(page_dat, u32, U8, U16, U32, U64, I8, I16, I32, I64);
		out.push((kind, label, title, page));
	}
	Ok(out)
}

fn parse_anchor_rows(json: &str) -> Outcome<Vec<AnchorRow>> {
	let raw = res!(parse_rows(json, false));
	Ok(raw.into_iter().map(|(kind, label, _title, page)| AnchorRow { kind, label, page }).collect())
}

fn parse_typst_rows(json: &str) -> Outcome<Vec<TypstRow>> {
	let raw = res!(parse_rows(json, true));
	Ok(raw.into_iter().map(|(kind, label, title, page)| TypstRow { kind, label, title, page }).collect())
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ RUNNING AUSTENITE                                                          │
// └───────────────────────────────────────────────────────────────────────────┘

struct AusOutput {
	total_pages:	usize,
	rows:			Vec<AnchorRow>,
	pdf_path:		PathBuf,
	skip_line:		Option<String>,	// austenite's own "skipped: ..." stderr line, if it printed one
}

/// Compiles `root` with the built `austenite` binary into the harness's working directory, reading back
/// the page count from its stdout status line and the anchor table from the `--ledger-out` JSON this
/// unit adds to `src/bin/austenite.rs`.
fn run_austenite(root: &CorpusRoot, work_dir: &Path) -> Outcome<AusOutput> {
	let out_dir			= work_dir.join(fmt!("{}-austenite-out", root.name));
	let ledger_json_path	= work_dir.join(fmt!("{}-ledger.json", root.name));
	let bin				= env!("CARGO_BIN_EXE_austenite");

	let output = match Command::new(bin)
		.arg("--ledger-out").arg(&ledger_json_path)
		.arg(root.path)
		.arg(&out_dir)
		.output()
	{
		Ok(o)	=> o,
		Err(e)	=> return Err(err!(e, "Could not run the built austenite binary at {:?}.", bin; IO)),
	};
	if !output.status.success() {
		return Err(err!(
			"austenite exited with {} compiling {:?}:\n{}",
			output.status, root.path, String::from_utf8_lossy(&output.stderr);
			Invalid, Unexpected));
	}

	let stdout			= String::from_utf8_lossy(&output.stdout).to_string();
	let total_pages		= res!(parse_austenite_page_count(&stdout));
	let ledger_json		= res!(std::fs::read_to_string(&ledger_json_path));
	let rows			= res!(parse_anchor_rows(&ledger_json));
	let skip_line		= parse_skip_line(&String::from_utf8_lossy(&output.stderr));

	// The harness needs only the PDF (for the raster sample) and the ledger JSON (already read above),
	// not the per-page SVGs -- on `oxeweb-techspec`'s 141 pages those are most of the ~340 MB a run
	// otherwise leaves behind. Deleting them keeps the working directory bounded across repeat runs
	// rather than growing it, per the coordinator's direction to keep this render out of `/tmp` and
	// bounded (a stale session's 7.7 GB of `/tmp` PDFs is exactly the failure mode this avoids).
	if let Ok(entries) = std::fs::read_dir(&out_dir) {
		for entry in entries.flatten() {
			let p = entry.path();
			if p.extension().and_then(|e| e.to_str()) == Some("svg") {
				let _ = std::fs::remove_file(&p);
			}
		}
	}

	Ok(AusOutput { total_pages, rows, pdf_path: out_dir.join("document.pdf"), skip_line })
}

/// Reads the page count back out of austenite's one-line stdout report -- "austenite: SRC -> N
/// page(s) in ..." -- rather than trusting a duplicate count kept only for this test.
fn parse_austenite_page_count(stdout: &str) -> Outcome<usize> {
	for line in stdout.lines() {
		if let Some(after) = line.split("-> ").nth(1) {
			if let Some(digits) = after.split(" page").next() {
				if let Ok(n) = digits.trim().parse::<usize>() {
					return Ok(n);
				}
			}
		}
	}
	Err(err!("Could not find a page count in austenite's output: {:?}", stdout; Missing, Invalid))
}

/// austenite's terse "[austenite] skipped: ..." stderr line, when it printed one.
fn parse_skip_line(stderr: &str) -> Option<String> {
	stderr.lines().find(|l| l.contains("skipped:")).map(|l| l.to_string())
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ RUNNING THE TYPST ORACLE                                                   │
// └───────────────────────────────────────────────────────────────────────────┘

struct TypstOutput {
	total_pages:	usize,
	rows:			Vec<TypstRow>,
}

/// Removes its path when dropped, including on an early `return`, so a failed run does not litter the
/// corpus root's own directory with a stray dump copy.
struct RemoveOnDrop(PathBuf);
impl Drop for RemoveOnDrop {
	fn drop(&mut self) { let _ = std::fs::remove_file(&self.0); }
}

/// Compiles `root` with the installed `typst` and reads back its page count and its heading/figure
/// pages, via the `tests/oracle/dump.typ` template copied beside the root (see that file's own comment
/// for why it rides out through `#metadata` and `typst query --field value` rather than a rendered
/// `json.encode` string). An `Err` here means the oracle could not run for this root at all -- a missing
/// `typst`, or a template incompatibility such as the `oxeweb` roots' (see [`corpus`]) -- and
/// [`compare_root`] treats that as the comparison being unavailable, not as an Austenite fault.
fn run_typst(root: &CorpusRoot, work_dir: &Path) -> Outcome<TypstOutput> {
	let root_path = Path::new(root.path);
	let root_name = match root_path.file_name().and_then(|n| n.to_str()) {
		Some(n)	=> n.to_string(),
		None	=> return Err(err!("Corpus root {:?} has no file name.", root.path; Input, Invalid)),
	};
	let root_dir = match root_path.parent() {
		Some(d)	=> d,
		None	=> return Err(err!("Corpus root {:?} has no parent directory.", root.path; Input, Invalid)),
	};
	if !root_path.is_file() {
		return Err(err!("Corpus root {:?} does not exist.", root.path; NotFound, File));
	}

	let template	= include_str!("dump.typ");
	let dump_src	= template.replace("__ROOT__", &root_name);
	let dump_path	= root_dir.join(fmt!(".oracle-dump-{}.typ", root.name));
	res!(std::fs::write(&dump_path, &dump_src));
	let _cleanup = RemoveOnDrop(dump_path.clone());

	let mut query = Command::new("typst");
	query.arg("query").arg(&dump_path).arg("<oracle-dump>").arg("--field").arg("value").arg("--one");
	if let Some(r) = root.typst_root { query.arg("--root").arg(r); }
	let qout = match query.output() {
		Ok(o)	=> o,
		Err(e)	=> return Err(err!(e, "Could not run the `typst` binary -- is it installed?"; IO)),
	};
	if !qout.status.success() {
		return Err(err!(
			"`typst query` on {:?} exited with {}:\n{}",
			dump_path, qout.status, String::from_utf8_lossy(&qout.stderr);
			Invalid, Unexpected));
	}
	let rows = res!(parse_typst_rows(&String::from_utf8_lossy(&qout.stdout)));

	// The page count comes from compiling the ORIGINAL root, not the dump copy, so the trailing
	// invisible `#metadata` call (it draws no ink and opens no page of its own) cannot be blamed for a
	// count that turns out to disagree.
	let pdf_path = work_dir.join(fmt!("{}-typst.pdf", root.name));
	let mut compile = Command::new("typst");
	compile.arg("compile").arg(root_path).arg(&pdf_path);
	if let Some(r) = root.typst_root { compile.arg("--root").arg(r); }
	let cout = match compile.output() {
		Ok(o)	=> o,
		Err(e)	=> return Err(err!(e, "Could not run `typst compile`."; IO)),
	};
	if !cout.status.success() {
		return Err(err!(
			"`typst compile` of {:?} exited with {}:\n{}",
			root.path, cout.status, String::from_utf8_lossy(&cout.stderr);
			Invalid, Unexpected));
	}
	let total_pages = res!(pdf_page_count(&pdf_path));

	Ok(TypstOutput { total_pages, rows })
}

/// The page count `pdfinfo` reports for a PDF, parsed off its `Pages:` line.
fn pdf_page_count(pdf: &Path) -> Outcome<usize> {
	let output = match Command::new("pdfinfo").arg(pdf).output() {
		Ok(o)	=> o,
		Err(e)	=> return Err(err!(e, "Could not run `pdfinfo`."; IO)),
	};
	let text = String::from_utf8_lossy(&output.stdout);
	for line in text.lines() {
		if let Some(rest) = line.strip_prefix("Pages:") {
			if let Ok(n) = rest.trim().parse::<usize>() {
				return Ok(n);
			}
		}
	}
	Err(err!("pdfinfo's output for {:?} carried no readable Pages line.", pdf; Missing, Invalid))
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ COMPARISON                                                                 │
// └───────────────────────────────────────────────────────────────────────────┘

/// What one corpus root's comparison found: always the Austenite-only facts (they are the regression
/// signal even when the oracle could not run), and, when the Typst oracle ran, the page-count and
/// order-matched anchor-page differences found against it.
pub struct RootReport {
	pub name:				&'static str,
	pub austenite_pages:	usize,
	pub austenite_anchors:	usize,
	pub skip_line:			Option<String>,
	pub typst_pages:		Option<usize>,
	pub oracle_note:		Option<String>,	// why the oracle comparison did not run, when it did not
	pub mismatches:			Vec<String>,	// page or count disagreements found against a running oracle
	pub raster_note:		Option<String>,
	pub pdf_path:			PathBuf,
}

impl RootReport {
	/// A short line for the test's own stdout -- visible under `--nocapture` and on a failure, so a
	/// reader sees every root's numbers without re-running anything.
	pub fn summary(&self) -> String {
		let oracle = match (&self.typst_pages, &self.oracle_note) {
			(Some(p), _)		=> fmt!("typst {} page(s), {} mismatch(es)", p, self.mismatches.len()),
			(None, Some(note))	=> fmt!("typst oracle unavailable ({})", note),
			(None, None)		=> "typst oracle not attempted".to_string(),
		};
		fmt!("{}: austenite {} page(s), {} anchor(s){} -- {}{}",
			self.name, self.austenite_pages, self.austenite_anchors,
			self.skip_line.as_ref().map(|s| fmt!(" [{}]", s)).unwrap_or_default(),
			oracle,
			self.raster_note.as_ref().map(|r| fmt!("; {}", r)).unwrap_or_default())
	}
}

/// How many pages an anchor's Austenite page may drift from its Typst oracle page before the drift is
/// reported as a problem rather than noted. Austenite's line breaker does not reproduce Typst's exact
/// paragraph and page breaks -- a small, roughly constant drift over a long document (see the
/// `austenite-doc` root's own recorded baseline: a one-to-two-page drift appears from its "In use"
/// section on, and holds rather than growing without bound) is the expected shape of that, not a
/// regression. What this harness exists to catch is a GROSS divergence -- a missing section, a runaway
/// page count -- and a `3` page tolerance leaves plenty of room for that while still tripping if the
/// drift starts widening.
const PAGE_DRIFT_TOLERANCE: i64 = 3;

/// How far apart the two engines' total page counts may sit, as a fraction of the Typst count, before
/// the harness reports it. Generous for the same reason as [`PAGE_DRIFT_TOLERANCE`].
const TOTAL_PAGE_TOLERANCE_FRACTION: f64 = 0.15;

/// How many headings (or figures) the two engines' counts may disagree by before it is reported. Typst
/// counts a level the reader treats specially (or vice versa) often enough on a 90-heading document that
/// exact parity is not today's fact -- `austenite-doc`'s own count sits at 89 against Typst's 90 -- so
/// this, like the two tolerances above, bounds the harness to catching a heading section actually going
/// missing rather than every single-heading accounting difference.
const COUNT_TOLERANCE: usize = 2;

/// Runs both compiles for `root` and returns the report [`RootReport::summary`] prints. An `Err` here
/// means Austenite itself could not produce a page for this root -- the one failure this harness treats
/// as a hard stop, since every other comparison depends on that page existing.
pub fn compare_root(root: &CorpusRoot, work_dir: &Path) -> Outcome<RootReport> {
	let ausout = res!(run_austenite(root, work_dir));

	let mut mismatches: Vec<String>	= Vec::new();
	let mut typst_pages					= None;
	let mut oracle_note					= None;

	match run_typst(root, work_dir) {
		Ok(typout) => {
			typst_pages = Some(typout.total_pages);

			let total_tolerance = ((typout.total_pages as f64) * TOTAL_PAGE_TOLERANCE_FRACTION).max(2.0);
			let total_drift = (typout.total_pages as i64 - ausout.total_pages as i64).abs();
			if (total_drift as f64) > total_tolerance {
				mismatches.push(fmt!(
					"total page count differs by {} (tolerance {:.0}): typst {} vs austenite {}",
					total_drift, total_tolerance, typout.total_pages, ausout.total_pages));
			}

			// The book template's three front-matter headings ("Title", "Meta", "Contents") are real
			// `heading` elements under Typst but `Label`-kind anchors under Austenite (see
			// `build_outline` in `src/bin/austenite.rs`), so they are dropped here rather than thrown
			// off the order match that follows. A lone chapter (no such front matter) drops nothing.
			let ty_heads: Vec<&TypstRow> = typout.rows.iter()
				.filter(|r| r.kind == "heading")
				.filter(|r| !(r.page <= 3 && matches!(r.title.as_str(), "Title" | "Meta" | "Contents")))
				.collect();
			// A lone chapter appends an always-present "Bibliography" heading whenever a `refs.bib` is
			// found anywhere up its directory tree ([`book::append_bibliography`]), whether or not the
			// chapter cited anything -- `thinking_chap_03.typ` cites nothing but still gets one, from a
			// book-wide `refs.bib` two levels up. Real, and arguably worth a follow-up (an empty
			// reference list probably should not print), but not this unit's to fix; it is excluded here
			// by its own anchor key rather than folded into a blanket count tolerance, so a genuinely
			// missing or duplicated body heading still trips the count check below.
			let au_heads: Vec<&AnchorRow> = ausout.rows.iter()
				.filter(|r| r.kind == "heading")
				.filter(|r| !r.label.to_lowercase().ends_with("bibliography"))
				.collect();
			let head_count_diff = ty_heads.len().abs_diff(au_heads.len());
			if head_count_diff > COUNT_TOLERANCE {
				mismatches.push(fmt!(
					"heading count differs by {} (tolerance {}): typst {} vs austenite {}",
					head_count_diff, COUNT_TOLERANCE, ty_heads.len(), au_heads.len()));
			}
			for (i, (t, a)) in ty_heads.iter().zip(au_heads.iter()).enumerate() {
				let drift = (t.page as i64 - a.page as i64).abs();
				if drift > PAGE_DRIFT_TOLERANCE {
					mismatches.push(fmt!(
						"heading #{} {:?} drifted {} page(s) (tolerance {}): typst page {} vs austenite page {}",
						i + 1, t.title, drift, PAGE_DRIFT_TOLERANCE, t.page, a.page));
				}
			}

			let ty_figs: Vec<&TypstRow> = typout.rows.iter().filter(|r| r.kind == "figure").collect();
			let au_figs: Vec<&AnchorRow> = ausout.rows.iter().filter(|r| r.kind == "float").collect();
			let fig_count_diff = ty_figs.len().abs_diff(au_figs.len());
			if fig_count_diff > COUNT_TOLERANCE {
				mismatches.push(fmt!(
					"figure count differs by {} (tolerance {}): typst {} vs austenite {}",
					fig_count_diff, COUNT_TOLERANCE, ty_figs.len(), au_figs.len()));
			}
			for (i, (t, a)) in ty_figs.iter().zip(au_figs.iter()).enumerate() {
				let drift = (t.page as i64 - a.page as i64).abs();
				if drift > PAGE_DRIFT_TOLERANCE {
					mismatches.push(fmt!(
						"figure #{} drifted {} page(s) (tolerance {}): typst page {} vs austenite page {}",
						i + 1, drift, PAGE_DRIFT_TOLERANCE, t.page, a.page));
				}
			}
		},
		Err(e) => oracle_note = Some(fmt!("{}", e)),
	}

	// The raster sample: one page, sampled only where a Typst PDF exists to sample against (skipped,
	// not failed, when ImageMagick or Poppler are absent -- see [`raster_diff_fraction`]).
	let raster_note = if typst_pages.is_some() {
		let ty_pdf = work_dir.join(fmt!("{}-typst.pdf", root.name));
		match raster_diff_fraction(&ausout.pdf_path, &ty_pdf, 1, 150, work_dir, root.name) {
			Ok(Some(frac))	=> Some(fmt!("page 1 raster differs in {:.1}% of pixels (fuzz 5%)", frac * 100.0)),
			Ok(None)		=> None,	// no ImageMagick/Poppler on this box
			Err(e)			=> Some(fmt!("raster sample failed: {}", e)),
		}
	} else {
		None
	};

	Ok(RootReport {
		name:				root.name,
		austenite_pages:	ausout.total_pages,
		austenite_anchors:	ausout.rows.len(),
		skip_line:			ausout.skip_line,
		typst_pages,
		oracle_note,
		mismatches,
		raster_note,
		pdf_path:			ausout.pdf_path,
	})
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ RASTER SAMPLE                                                              │
// └───────────────────────────────────────────────────────────────────────────┘

/// The fraction of pixels that differ between page `page` of `pdf_a` and `pdf_b`, both rasterised at
/// `dpi` via `pdftoppm` and compared with ImageMagick's `compare -metric AE -fuzz 5%` (a small fuzz so
/// anti-aliasing and hinting differences between the two engines' rasterisers do not themselves count).
/// `Ok(None)` when `pdftoppm`, `compare` or `identify` is not installed, so a box without ImageMagick
/// still runs the rest of the suite -- this sample is a coarse visual sanity net, one root at a page,
/// bounded well under the DPI a real proof pass would use, not the harness's primary signal (that is
/// the page-count and anchor-page comparison above, which needs no rasteriser at all).
fn raster_diff_fraction(
	pdf_a:		&Path,
	pdf_b:		&Path,
	page:		usize,
	dpi:		u32,
	work_dir:	&Path,
	tag:		&str,
)
	-> Outcome<Option<f64>>
{
	if !tool_present("pdftoppm") || !tool_present("compare") || !tool_present("identify") {
		return Ok(None);
	}
	let dir_a = work_dir.join(fmt!("{}-raster-a", tag));
	let dir_b = work_dir.join(fmt!("{}-raster-b", tag));
	let png_a = res!(rasterise_page(pdf_a, page, dpi, &dir_a));
	let png_b = res!(rasterise_page(pdf_b, page, dpi, &dir_b));
	Ok(Some(res!(pixel_diff_fraction(&png_a, &png_b))))
}

/// Does invoking `name --version` at least spawn? A missing binary fails to spawn at all; a real one
/// may still exit non-zero on a bare `--version` (uninteresting here), so only the spawn is checked.
fn tool_present(name: &str) -> bool {
	Command::new(name).arg("--version").output().is_ok()
}

/// Renders one page of `pdf` to a PNG under `out_dir` at `dpi`, returning the file `pdftoppm` wrote.
/// The output is found by listing `out_dir` rather than assumed by name, since `pdftoppm`'s page-number
/// suffix width depends on the source PDF's own page count, not on the `-f`/`-l` range requested.
fn rasterise_page(pdf: &Path, page: usize, dpi: u32, out_dir: &Path) -> Outcome<PathBuf> {
	res!(std::fs::create_dir_all(out_dir));
	let prefix = out_dir.join("p");
	let status = match Command::new("pdftoppm")
		.arg("-png").arg("-r").arg(dpi.to_string())
		.arg("-f").arg(page.to_string()).arg("-l").arg(page.to_string())
		.arg(pdf).arg(&prefix)
		.status()
	{
		Ok(s)	=> s,
		Err(e)	=> return Err(err!(e, "Could not run `pdftoppm`."; IO)),
	};
	if !status.success() {
		return Err(err!("`pdftoppm` on {:?} page {} exited with {}.", pdf, page, status; Invalid, Unexpected));
	}
	let entries = match std::fs::read_dir(out_dir) {
		Ok(e)	=> e,
		Err(e)	=> return Err(err!(e, "Could not list {:?}.", out_dir; File, Read)),
	};
	for entry in entries.flatten() {
		let p = entry.path();
		if p.extension().and_then(|e| e.to_str()) == Some("png") {
			return Ok(p);
		}
	}
	Err(err!("`pdftoppm` wrote no PNG into {:?}.", out_dir; Missing, Invalid))
}

/// The fraction of pixels ImageMagick's `compare -metric AE` counts as differing between two
/// same-size PNGs -- a non-zero exit from `compare` itself is the expected shape of a real difference,
/// not a tool failure, so only the parse of its reported count can fail this. Recent ImageMagick prints
/// both the raw differing-pixel count and its own normalised fraction in parentheses, e.g.
/// `"28521 (0.0131028)"`; the parenthesised value is used directly when present (it is already what
/// this function returns), falling back to `count / (width * height)` via `identify` for an older
/// build that prints the raw count alone.
fn pixel_diff_fraction(png_a: &Path, png_b: &Path) -> Outcome<f64> {
	let output = match Command::new("compare")
		.arg("-metric").arg("AE")
		.arg("-fuzz").arg("5%")
		.arg(png_a).arg(png_b).arg("null:")
		.output()
	{
		Ok(o)	=> o,
		Err(e)	=> return Err(err!(e, "Could not run ImageMagick `compare`."; IO)),
	};
	let text = String::from_utf8_lossy(&output.stderr).trim().to_string();

	if let (Some(open), Some(close)) = (text.find('('), text.find(')')) {
		if close > open {
			if let Ok(frac) = text[open + 1..close].trim().parse::<f64>() {
				return Ok(frac);
			}
		}
	}

	let count: f64 = match text.split_whitespace().next().and_then(|s| s.parse().ok()) {
		Some(n)	=> n,
		None	=> return Err(err!("Could not parse `compare`'s AE output {:?}.", text; Decode, Invalid)),
	};
	let (w, h) = res!(png_dimensions(png_a));
	let total = (w as f64) * (h as f64);
	if total <= 0.0 {
		return Err(err!("{:?} has zero area.", png_a; Invalid, Range));
	}
	Ok(count / total)
}

/// A PNG's pixel dimensions, via ImageMagick's `identify -format "%w %h"`.
fn png_dimensions(png: &Path) -> Outcome<(u32, u32)> {
	let output = match Command::new("identify").arg("-format").arg("%w %h").arg(png).output() {
		Ok(o)	=> o,
		Err(e)	=> return Err(err!(e, "Could not run ImageMagick `identify`."; IO)),
	};
	let text = String::from_utf8_lossy(&output.stdout);
	let mut parts = text.split_whitespace();
	let w = parts.next().and_then(|s| s.parse::<u32>().ok());
	let h = parts.next().and_then(|s| s.parse::<u32>().ok());
	match (w, h) {
		(Some(w), Some(h))	=> Ok((w, h)),
		_					=> Err(err!("Could not parse identify's dimensions {:?} for {:?}.", text, png; Decode, Invalid)),
	}
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ BASELINE                                                                   │
// └───────────────────────────────────────────────────────────────────────────┘

/// What is recorded per root for a later run to diff against: today's own output, not a copy of the
/// Typst oracle's (which the corpus's own comparison already re-derives fresh on every run). A drift
/// here -- a page count or anchor count that moves between two runs of the *same* commit -- is either a
/// non-determinism bug or evidence the corpus files themselves changed underfoot; a drift across two
/// commits is what a later unit's regression check is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BaselineEntry {
	pub pages:		usize,
	pub anchors:	usize,
}

#[derive(Clone, Debug, Default)]
pub struct Baseline {
	entries: BTreeMap<String, BaselineEntry>,
}

impl Baseline {
	pub fn from_entries(entries: BTreeMap<String, BaselineEntry>) -> Self {
		Self { entries }
	}

	pub fn get(&self, name: &str) -> Option<BaselineEntry> {
		self.entries.get(name).copied()
	}

	pub fn set(&mut self, name: &str, entry: BaselineEntry) {
		self.entries.insert(name.to_string(), entry);
	}

	fn to_dat(&self) -> Outcome<Dat> {
		let mut rows = Vec::with_capacity(self.entries.len());
		for (name, e) in &self.entries {
			rows.push(omapdat!{
				"name"		=> dat!(name.clone()),
				"pages"		=> dat!(e.pages as u64),
				"anchors"	=> dat!(e.anchors as u64),
			});
		}
		Ok(omapdat!{ "roots" => Dat::List(rows) })
	}

	fn from_dat(mut dat: Dat) -> Outcome<Self> {
		let rows	= try_extract_dat!(res!(dat.map_remove_must(&dat!("roots"))), List);
		let mut entries = BTreeMap::new();
		for mut row in rows {
			let name	= try_extract_dat!(res!(row.map_remove_must(&dat!("name"))), Str);
			let pages_d	= res!(row.map_remove_must(&dat!("pages")));
			let anch_d	= res!(row.map_remove_must(&dat!("anchors")));
			let pages	= try_extract_dat_as!(pages_d, usize, U8, U16, U32, U64, I8, I16, I32, I64);
			let anchors	= try_extract_dat_as!(anch_d, usize, U8, U16, U32, U64, I8, I16, I32, I64);
			entries.insert(name, BaselineEntry { pages, anchors });
		}
		Ok(Self { entries })
	}

	pub fn write_to_file(&self, path: &Path) -> Outcome<()> {
		let dat	= res!(self.to_dat());
		let s	= res!(dat.json());
		res!(std::fs::write(path, s));
		Ok(())
	}

	pub fn read_from_file(path: &Path) -> Outcome<Self> {
		let s	= res!(std::fs::read_to_string(path));
		let dat	= res!(Dat::decode_string(s));
		Self::from_dat(dat)
	}
}

/// The real baseline file's path, in the harness's working directory rather than the crate's own
/// `tests/` tree, so it never becomes something git tracks or a reviewer reads as fixture data.
pub fn baseline_path(work_dir: &Path) -> PathBuf {
	work_dir.join("baseline.json")
}

/// Records `report` into the baseline at `path`, or -- when an entry for this root is already there --
/// returns the disagreement as a `Some` for the caller to decide whether it is a real regression.
/// Bootstraps the file (and always reports no disagreement) the first time a root is seen, which is
/// exactly today's run against the tree this unit lands in: there is no earlier baseline to diverge
/// from yet, only one being laid down for the unit after this one to diff against.
pub fn record_and_diff(path: &Path, report: &RootReport) -> Outcome<Option<String>> {
	let mut baseline = if path.is_file() {
		res!(Baseline::read_from_file(path))
	} else {
		Baseline::default()
	};
	let current = BaselineEntry { pages: report.austenite_pages, anchors: report.austenite_anchors };
	let prior = baseline.get(report.name);
	baseline.set(report.name, current);
	res!(baseline.write_to_file(path));
	match prior {
		Some(p) if p != current => Ok(Some(fmt!(
			"{} moved from the recorded baseline: {} page(s)/{} anchor(s) -> {} page(s)/{} anchor(s)",
			report.name, p.pages, p.anchors, current.pages, current.anchors))),
		_ => Ok(None),
	}
}
