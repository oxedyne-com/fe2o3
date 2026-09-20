//! `austenite` -- compile a Typst document to a set of pages.
//!
//! Reads a Typst root, follows its `#include` chain through the [`book`](oxedyne_fe2o3_austenite::book)
//! assembler (or, for a lone file, straight through the [`lang`](oxedyne_fe2o3_austenite::lang) reader),
//! authors the block stream through the block layer, runs the two-pass driver to a fixed point, decorates
//! each page with a running head and a folio, and writes every page as SVG alongside the resolved ledger
//! and a single PDF of the whole run.
//!
//! A construct the reader cannot yet set -- a `#show` rule, a `#columns` wrapper, an unknown `#func` --
//! is passed over rather than failing the compile, and the lone-file path reports the tally on one terse
//! line so a dropped construct is visible.
//!
//! Usage: `austenite <SOURCE.typ> [OUTPUT_DIR]` (default output `austenite-out`), or
//! `austenite --watch <SOURCE.typ> [OUTPUT_DIR]` to recompile on every change to the root, its includes,
//! its `config.typ`, or its assets.

use oxedyne_fe2o3_austenite::{
	compile,
	emit::{
		self,
		svg,
	},
	ir::DrawOp,
	ledger::Ledger,
	lang,
	memo::Memo,
	page::{
		Frame,
		Page,
		PlacedKind,
	},
	watch,
};

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_graphics::pdf::PdfPage;
use oxedyne_fe2o3_jdat::prelude::*;

use std::fs::File;
use std::io::BufWriter;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

/// An estimate of the extra memory rendering this page holds in flight, in bytes -- dominated by the
/// figure rasters, which [`emit::pdf::render_page`] copies out of the shared frame into the page's own
/// straight-RGB (and, when translucent, grey) buffers. A text page estimates near zero; a full-page
/// illustration estimates several megabytes. The chunker sums this across a forming chunk and closes it
/// before the sum would breach the memory budget, so an illustration-dense book self-limits its window
/// while a text book packs a chunk full. The glyph outlines are not counted: a chunk holds them until the
/// writer serialises its pages, but they were shown to stay flat to a wide window, and text is now stored
/// once per distinct glyph rather than baked per occurrence, so the held bytes are smaller than before.
fn page_hold_estimate(page: &Page) -> usize {
	// Rough bytes-per-unit for the SVG text and PDF content a page's ink expands to while it is in flight.
	// A glyph becomes an outline of a couple of dozen path operators in each of the two serialisations; a
	// figure's own path is written op for op; a raster is copied sample by sample. The constants are
	// deliberately generous -- the estimate gates concurrency, so over-counting only narrows a chunk.
	const PER_GLYPH:	usize = 800;	// one glyph's outline, in both serialisations (measured)
	const PER_SEG:		usize = 320;	// one figure path segment, across every live buffer (measured)
	const PER_SAMPLE:	usize = 8;		// RGB copy plus soft mask, with headroom

	let mut bytes = 0usize;
	for placed in &page.frame.placed {
		match &placed.kind {
			PlacedKind::Text(shaped) => {
				bytes += shaped.run().glyphs.len() * PER_GLYPH;
			},
			PlacedKind::Graphic(g) => {
				for op in &g.ops {
					match op {
						DrawOp::Fill { path, .. }	=> bytes += path.segs().len() * PER_SEG,
						DrawOp::Stroke { path, .. }	=> bytes += path.segs().len() * PER_SEG,
						DrawOp::Image { image, .. }	=> bytes += image.width * image.height * PER_SAMPLE,
					}
				}
			},
			_ => {},
		}
	}
	bytes
}

/// One page's PDF draw list, built off the writer's thread. The worker fetches the page's glyph outlines
/// (warming the shared cache) and writes the page's SVG straight to its own file, since an SVG page owes
/// nothing to page order. The content stream itself is serialised on the writer's thread, in page order,
/// where a glyph's Type-3 code is assigned deterministically; the draw list travels back for that.
struct Prepared {
	pdf:		PdfPage,
}

/// The result of a compile, for the caller to report: the page count, the number of driver passes to
/// the fixed point, the count of anchors in the resolved ledger, and the terse skip line (or `None`
/// when nothing was skipped).
struct CompileStats {
	pages:		usize,
	passes:		u32,
	anchors:	usize,
	skip_line:	Option<String>,
	refusals:	lang::Refusals,	// every refused site, for `--explain`; the terse `skip_line` stays the default
}

/// Renders one page to both artefacts, the pure work a chunk runs across the cores. The SVG is written
/// to its file here and dropped; the PDF draw list is built here -- fetching each glyph's outline, the
/// bulk of the cost -- and returned for the ordered writer to serialise and frame in page order.
fn render_page_pair(page: &Page, out_dir: &str, write_svg: bool) -> Outcome<Prepared> {
	// When the memo drives the run (the `--watch` loop), the SVG is emitted sequentially through the memo
	// in a pre-pass, so the parallel worker here builds only the PDF; a one-shot compile writes both.
	if write_svg {
		let svg		= res!(svg::render_page(page));
		let path	= fmt!("{}/page-{:03}.svg", out_dir, page.number);
		res!(std::fs::write(&path, &svg));
		drop(svg);
	}

	let pdf			= res!(emit::pdf::render_page(page));
	Ok(Prepared { pdf })
}

/// The detailed report `--explain` prints: every refused site, one per line, as `file:line:col: <class>:
/// skipped <name>` with the source line beneath it and a `^` caret under the column the span starts at.
/// Each referenced file is read at most once, cached by path, and a file that has since moved or gone
/// (a rare race, not the common case) yields a one-line note in its place rather than failing the whole
/// report -- `--explain` is a diagnostic, and a diagnostic that can fail is a worse tool than one that
/// degrades. Sites are printed in the order the reader met them, which is document order within a file
/// and file order (root first, then each `#include` as it is read) across a whole book.
fn explain_refusals(refusals: &lang::Refusals) -> String {
	let mut cache: std::collections::HashMap<String, Option<String>> = std::collections::HashMap::new();
	let mut out = String::new();
	for r in refusals.sites() {
		let text = cache.entry(r.file.clone())
			.or_insert_with(|| std::fs::read_to_string(&r.file).ok());
		match text {
			Some(src) => {
				let (line_no, col, line_text) = lang::line_col_of(src, r.span.start);
				out.push_str(&fmt!("{}:{}:{}: {}: skipped {}\n", r.file, line_no, col, r.class.label(), r.name));
				out.push_str(line_text);
				out.push('\n');
				for _ in 1..col { out.push(' '); }
				out.push_str("^\n");
			},
			None => {
				out.push_str(&fmt!("{}: {}: skipped {} (source no longer readable for a caret)\n",
					r.file, r.class.label(), r.name));
			},
		}
	}
	out
}

/// Each ledger anchor's kind, label (its content key) and resolved page, as a small JSON array -- the
/// oracle harness's other half, compared against a Typst `query` dump of the same document's headings
/// and figures. Kept separate from [`Ledger::to_file`]'s full jdat dump, which also carries the
/// `reserved`/`realised` widths a Typst comparison has no equivalent for.
fn ledger_dump_json(ledger: &Ledger) -> Outcome<String> {
	let mut rows = Vec::with_capacity(ledger.len());
	for a in ledger.anchors() {
		rows.push(omapdat!{
			"kind"	=> dat!(a.id.kind.name()),
			"label"	=> dat!(a.id.key.clone()),
			"page"	=> dat!(a.pos.page),
			// The x of the anchor's left from the page's left edge, in whole points, so the oracle can tell
			// which column an index slot landed in and gate that the two-column index really uses two.
			"x"		=> dat!(a.pos.x.to_pt().round() as i64),
			// The y of the anchor's top from the page's top edge, in whole points, so the oracle can tell a
			// top float from a foot one and check a float landed on the same side as Typst.
			"y"		=> dat!(a.pos.y.to_pt().round() as i64),
		});
	}
	Dat::List(rows).json()
}

/// Compiles the Typst root at `source` into `out_dir`, writing every page's SVG, the resolved ledger,
/// and one PDF of the whole run. `ledger_out`, when given, also writes the terse kind/label/page JSON
/// dump ([`ledger_dump_json`]) the oracle harness compares against a Typst `query`. Returns the counts
/// and the terse skip line for the caller to report; prints nothing itself save the phase profile when
/// `AUS_PROFILE` is set.
fn compile(
	source:		&str,
	out_dir:	&str,
	pearl:		bool,
	ledger_out:	Option<&str>,
	mut memo:	Option<&mut Memo>,
)
	-> Outcome<CompileStats>
{
	// Phase timing, gated on AUS_PROFILE so a normal run is untouched. Each phase reports its wall time
	// to stderr, leaving stdout (and every emitted byte) exactly as it was.
	let prof = std::env::var("AUS_PROFILE").is_ok();
	let mark = |label: &str, t: std::time::Instant| {
		if prof {
			eprintln!("[profile] {:<22} {:>8.1} ms", label, t.elapsed().as_secs_f64() * 1000.0);
		}
	};
	let t_all = std::time::Instant::now();

	// Assemble the document -- a book or doc root through the whole-book assembler, a lone file through the
	// reader -- and then author, run, decorate and mirror-shift it. Both stages live in `compile`, shared
	// verbatim with the wasm surface so the two cannot drift. The lone-file path builds the embedded
	// Libertinus through the thunk, only when it is in fact a lone file.
	let t_parse = std::time::Instant::now();
	let (assembled, refusals, skip_line) = res!(compile::assemble(
		std::path::Path::new(source),
		|| Ok(Arc::new(res!(oxedyne_fe2o3_austenite::fonts::libertinus()))),
	));
	mark("parse+lower+fonts", t_parse);

	let t_author = std::time::Instant::now();
	let compile::Rendered { mut out, heads, geom } = res!(compile::author_and_run_memo(assembled, memo.as_deref_mut()));
	mark("author+run+decorate", t_author);

	res!(std::fs::create_dir_all(out_dir));

	// The incremental SVG emit, when a memo drives the run (the `--watch` loop): each page's body is
	// content-hashed and an unedited page reuses its rendered SVG, so an edit re-renders only the pages the
	// cascade reaches. It runs sequentially, ahead of the PDF pass below, because the memo is a single
	// shared cache; that is exactly the live-view latency this increment targets. The SVG bytes are
	// identical to the parallel path's, so the emitted files do not depend on which path wrote them.
	let svg_in_worker = memo.is_none();
	if let Some(m) = memo.as_deref_mut() {
		let t_svg = std::time::Instant::now();
		for page in &out.pages {
			let svg		= res!(svg::render_page_memo(page, m));
			let path	= fmt!("{}/page-{:03}.svg", out_dir, page.number);
			res!(std::fs::write(&path, &svg));
		}
		mark("emit(svg, memo)", t_svg);
	}

	// The ledger is small and independent of the pages, so it is written first and out of the way.
	let ledger_path = fmt!("{}/ledger.jdat", out_dir);
	res!(out.ledger.to_file(&ledger_path));
	if let Some(path) = ledger_out {
		let json = res!(ledger_dump_json(&out.ledger));
		res!(std::fs::write(path, json));
	}

	// Pearl, when asked: a content-addressed `.prl` accumulated across the streaming emit loop, each page
	// folded in before its frame is dropped, so it streams exactly as the SVG and PDF arms do.
	let mut pearl_builder = if pearl {
		Some(res!(emit::pearl::PearlBuilder::new(&out.ledger, geom)))
	} else {
		None
	};

	// Emit each page and drop its frame before the next. Both writers are streaming: the SVG is one file
	// per page, and the PDF is written object by object into the file as each page is composed, never
	// accumulated. Holding a bounded window of pages' glyph outlines -- rather than every page's at once,
	// as a buffered whole-document PDF would -- is what keeps a whole-book compile flat in memory.
	//
	// Almost the whole cost of a compile is here: turning each glyph into a filled outline and serialising
	// it, page after page. That work is a pure function of the placed frame and is independent between
	// pages, so a chunk of pages is rendered across the cores at once. The order is preserved exactly: a
	// chunk's results are written to the SVG files and folded into the single PDF stream in page order,
	// so the bytes are identical to a sequential emit -- only the wall time differs. The PDF's object
	// numbering and its running `/ID` hash stay strictly sequential in the writer, on this thread.
	let t_emit	= std::time::Instant::now();
	let mut t_render_ms	= 0.0f64;	// wall spent in the parallel render stage
	let mut t_write_ms	= 0.0f64;	// wall spent writing results out in order
	let pdf_file	= res!(File::create(fmt!("{}/document.pdf", out_dir)));
	let outline		= compile::build_outline(&heads, &out.ledger);
	let mut pdf		= res!(emit::pdf::open_document_with_outline(
		BufWriter::new(pdf_file), out.pages.len(), outline));

	// Emit is by far the costliest phase and is embarrassingly parallel: each page's outline transforms
	// and serialisation are a pure function of its frame, independent of every other page. But a rendered
	// page is large -- its glyph outlines and figures expand to megabytes of SVG and PDF operators -- so
	// holding several at once regresses peak memory, which the engine keeps to a page-at-a-time budget.
	// For an illustration-dense book the per-page ink is heavy enough that even a pair of pages can breach
	// that budget, so parallelism there is not free.
	//
	// The default therefore opens the window to eight pages (capped at the core count), which brings a
	// text book to roughly Typst's own wall time while peak memory stays a few hundred megabytes -- far
	// under Typst's gigabytes -- and the shared glyph-outline cache speeds every path besides. The ink
	// budget below keeps the window honest: AUS_EMIT_BUDGET_MB caps the estimated page ink in flight (see
	// [`page_hold_estimate`]), so a run of heavy figure pages closes its chunk early and never all
	// coincide; a chunk always holds at least one page, so a page heavier than the budget still renders --
	// alone. A caller wanting the strict page-at-a-time floor sets AUS_EMIT_WINDOW=1.
	let cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1);
	let width = std::env::var("AUS_EMIT_WINDOW")
		.ok()
		.and_then(|s| s.parse::<usize>().ok())
		.filter(|n| *n >= 1)
		.unwrap_or(8)
		.min(cores);
	let budget = std::env::var("AUS_EMIT_BUDGET_MB")
		.ok()
		.and_then(|s| s.parse::<usize>().ok())
		.unwrap_or(8)
		.saturating_mul(1024 * 1024);

	if prof {
		let ests: Vec<usize> = out.pages.iter().map(page_hold_estimate).collect();
		let sum: usize = ests.iter().sum();
		let max = ests.iter().copied().max().unwrap_or(0);
		eprintln!("[profile]   est/page max {:.2} MB, mean {:.2} MB",
			max as f64 / 1048576.0, sum as f64 / 1048576.0 / out.pages.len().max(1) as f64);
	}

	let total = out.pages.len();
	let mut start = 0usize;
	while start < total {
		// Grow the chunk to the page-count width, but stop early once the page ink in flight would exceed
		// the memory budget -- keeping at least the one page so a heavy page still renders.
		let mut end		= start;
		let mut held	= 0usize;
		while end < total && end - start < width {
			let cost = page_hold_estimate(&out.pages[end]);
			if end > start && held + cost > budget {
				break;
			}
			held += cost;
			end += 1;
		}
		let slice	= &out.pages[start..end];

		// Render this chunk's pages in parallel: each worker builds its page's SVG string, its PDF draw
		// list, and that list serialised to content-stream bytes -- all pure, all independent.
		let tr = std::time::Instant::now();
		let out_ref = out_dir;
		let rendered: Vec<Outcome<Prepared>> = std::thread::scope(|scope| {
			let handles: Vec<_> = slice.iter()
				.map(|page| scope.spawn(move || render_page_pair(page, out_ref, svg_in_worker)))
				.collect();
			handles.into_iter()
				.map(|h| match h.join() {
					Ok(r)	=> r,
					Err(_)	=> Err(err!("A page-render worker thread panicked."; Bug, Thread)),
				})
				.collect()
		});
		if prof { t_render_ms += tr.elapsed().as_secs_f64() * 1000.0; }

		// Fold the chunk into the PDF stream in page order (its `/ID` hashes page by page). The SVG files
		// were already written by the workers. Then free each page's frame, holding no chunk beyond this.
		let tw = std::time::Instant::now();
		for prep in rendered {
			let prep	= res!(prep);
			res!(emit::pdf::write_built_page(&mut pdf, &prep.pdf));
		}
		// Fold this chunk's pages into the Pearl document before their frames are freed below.
		if let Some(pb) = pearl_builder.as_mut() {
			for page in &out.pages[start..end] {
				res!(pb.add_page(page));
			}
		}
		for page in &mut out.pages[start..end] {
			page.frame = Frame::new();
		}
		if prof { t_write_ms += tw.elapsed().as_secs_f64() * 1000.0; }

		start = end;
	}
	res!(pdf.finish());
	if let Some(pb) = pearl_builder {
		res!(pb.to_file(fmt!("{}/document.prl", out_dir)));
	}
	mark("emit(svg+pdf)", t_emit);
	if prof {
		eprintln!("[profile]   render (parallel){:>8.1} ms", t_render_ms);
		eprintln!("[profile]   write (in order) {:>8.1} ms", t_write_ms);
		eprintln!("[profile]   width/budgetMB    {:>8}", width);
		eprintln!("[profile] {:<22} {:>8.1} ms", "TOTAL", t_all.elapsed().as_secs_f64() * 1000.0);
	}

	// Close the memo generation now the pages are emitted, dropping entries untouched for two compiles.
	if let Some(m) = memo.as_deref_mut() {
		m.sweep();
	}

	Ok(CompileStats {
		pages:		out.pages.len(),
		passes:		out.passes,
		anchors:	out.ledger.len(),
		skip_line,
		refusals,
	})
}

/// The first double-quoted run in a slice, its contents without the quotes. Mirrors the book
/// assembler's include parsing so the watch set follows exactly the files a compile reads.
fn first_quoted(s: &str) -> Option<String> {
	let open	= match s.find('"') {
		Some(i)	=> i,
		None	=> return None,
	};
	let rest	= &s[open + 1..];
	let close	= match rest.find('"') {
		Some(i)	=> i,
		None	=> return None,
	};
	Some(rest[..close].to_string())
}

/// Adds every file under `dir`, recursively, to `set`, plus `dir` itself so an asset added or removed
/// changes the watched snapshot. Bounded in depth and count so a large font tree cannot make a tick
/// expensive; the cap is generous for a book's assets.
fn collect_files(dir: &std::path::Path, set: &mut Vec<PathBuf>, depth: usize) {
	const MAX_DEPTH:	usize = 6;
	const MAX_FILES:	usize = 4000;

	if depth > MAX_DEPTH || set.len() > MAX_FILES {
		return;
	}
	set.push(dir.to_path_buf());
	let entries = match std::fs::read_dir(dir) {
		Ok(e)	=> e,
		Err(_)	=> return,
	};
	for entry in entries.flatten() {
		let path = entry.path();
		if path.is_dir() {
			collect_files(&path, set, depth + 1);
		} else {
			set.push(path);
		}
		if set.len() > MAX_FILES {
			return;
		}
	}
}

/// The set of files a compile of `source` depends on, for the watch to poll: the root itself, its
/// `config.typ`, each file it `#include`s, and the assets trees a book resolves against (beside the
/// root and one level up, per the book assembler). Recomputed each tick, so a newly added include or
/// asset is watched without a restart.
fn watch_set(source: &str) -> Vec<PathBuf> {
	let mut set: Vec<PathBuf> = Vec::new();
	let src_path = PathBuf::from(source);
	set.push(src_path.clone());

	let root_dir = src_path.parent()
		.map(|p| p.to_path_buf())
		.unwrap_or_else(|| PathBuf::from("."));
	set.push(root_dir.join("config.typ"));

	// Read the root fresh so an include added mid-session joins the watch; a read failure just leaves the
	// include set as it was on the previous tick.
	if let Ok(src) = std::fs::read_to_string(&src_path) {
		for line in src.lines() {
			let t = line.trim_start();
			if let Some(rest) = t.strip_prefix("#include") {
				if let Some(rel) = first_quoted(rest) {
					set.push(root_dir.join(rel));
				}
			}
		}
	}

	// The assets tree sits beside the root and, for a book, one level up at the project root. Watch both,
	// recursively, so an edited figure or image triggers a rebuild, not only an added or removed file.
	collect_files(&root_dir.join("assets"), &mut set, 0);
	if let Some(project_dir) = root_dir.parent() {
		collect_files(&project_dir.join("assets"), &mut set, 0);
	}
	set
}

/// Prints one terse status line for a compile that produced `stats` of `source` into `out_dir`, taking
/// `elapsed` wall: the source, the page count, the wall in seconds, and the skip line folded on where
/// there is one.
fn print_status(source: &str, out_dir: &str, stats: &CompileStats, elapsed: Duration) {
	let mut line = fmt!("[austenite] {} -> {} page(s), {:.2}s -> {}/",
		source, stats.pages, elapsed.as_secs_f64(), out_dir);
	if let Some(skip) = &stats.skip_line {
		line.push_str("; ");
		line.push_str(skip);
	}
	println!("{}", line);
}

fn main() -> Outcome<()> {
	// Flags may precede or follow the paths; only `--watch` (`-w`), `--pearl`, `--ledger-out <path>` and
	// `--explain` are recognised, everything else is a positional argument in order: the source root,
	// then the optional output directory.
	let mut watching	= false;
	let mut pearl		= false;
	let mut explain		= false;
	let mut ledger_out:	Option<String>	= None;
	let mut pos:	Vec<String>	= Vec::new();
	let mut args = std::env::args().skip(1);
	while let Some(a) = args.next() {
		match a.as_str() {
			"--watch" | "-w"	=> watching = true,
			"--pearl"			=> pearl = true,
			"--explain"			=> explain = true,
			"--ledger-out"		=> {
				ledger_out = Some(match args.next() {
					Some(p)	=> p,
					None	=> return Err(err!(
						"--ledger-out needs a path argument."; Input, Invalid, Missing)),
				});
			},
			_					=> pos.push(a),
		}
	}
	let source = match pos.first() {
		Some(s)	=> s.clone(),
		None	=> return Err(err!(
			"Usage: austenite [--watch] [--pearl] [--explain] [--ledger-out PATH] <SOURCE.typ> [OUTPUT_DIR]";
			Input, Invalid, Missing)),
	};
	let out_dir = match pos.get(1) {
		Some(s)	=> s.clone(),
		None	=> "austenite-out".to_string(),
	};

	if watching {
		// Poll interval: brisk enough to feel live, cheap enough to leave the cores to the compile.
		let interval	= Duration::from_millis(400);
		let src_files	= source.clone();		// the file-set closure borrows this
		let src_build	= source.clone();		// the build closure owns this
		let out			= out_dir.clone();
		let ledger_out_w	= ledger_out.clone();	// the build closure owns this
		println!("[austenite] watching {} -> {}/ (Ctrl-C to stop)", source, out_dir);
		// One memo lives across every rebuild of the watch, so an edit re-authors and re-emits only the
		// blocks and pages it actually changes -- the incremental live-view loop the wasm swap needs.
		let mut memo = Memo::new();
		return watch::run(
			move || watch_set(&src_files),
			move || {
				let t = std::time::Instant::now();
				match compile(&src_build, &out, pearl, ledger_out_w.as_deref(), Some(&mut memo)) {
					Ok(stats)	=> {
						// The skip line is folded into the status line, so the rebuild is one line.
						print_status(&src_build, &out, &stats, t.elapsed());
						Ok(())
					},
					Err(e)		=> Err(e),
				}
			},
			interval,
		);
	}

	let t = std::time::Instant::now();
	let stats = res!(compile(&source, &out_dir, pearl, ledger_out.as_deref(), None));
	if explain {
		print!("{}", explain_refusals(&stats.refusals));
	} else if let Some(skip) = &stats.skip_line {
		eprintln!("[austenite] {}", skip);
	}
	println!(
		"austenite: {} -> {} page(s) in {} pass(es); {} anchor(s) in the ledger; {:.2}s; written to {}/",
		source, stats.pages, stats.passes, stats.anchors, t.elapsed().as_secs_f64(), out_dir);
	Ok(())
}

#[cfg(test)]
mod tests {
	use super::*;
	use oxedyne_fe2o3_austenite::ir::Span;
	use oxedyne_fe2o3_austenite::lang::{Refusal, RefusalClass, Refusals};

	/// `--explain`'s report for a site whose file can no longer be read (moved, deleted -- a rare race,
	/// not the common case) degrades to a one-line note naming the class and construct, rather than
	/// failing the whole report or panicking.
	#[test]
	fn explain_degrades_when_the_file_cannot_be_read() {
		let refusals = Refusals::from_sites(vec![Refusal {
			name:	"#query".to_string(),
			span:	Span::new(0, 0),
			class:	RefusalClass::Introspective,
			file:	"/nonexistent/path/for/an/austenite/explain/test.typ".to_string(),
		}]);
		let report = explain_refusals(&refusals);
		assert!(report.contains("introspective"), "class label missing: {:?}", report);
		assert!(report.contains("#query"), "construct name missing: {:?}", report);
		assert!(report.contains("no longer readable"), "no degraded-file note: {:?}", report);
	}

	/// `--explain`'s report for a readable file names it, its 1-based line and column, the refusal's
	/// class and name, the source line itself, and a caret under the column the span starts at -- the
	/// full shape a reader relies on to jump straight to the site.
	#[test]
	fn explain_reports_file_line_col_class_name_and_a_caret() -> Outcome<()> {
		let dir = std::env::temp_dir().join(fmt!("austenite-explain-test-{}", std::process::id()));
		res!(std::fs::create_dir_all(&dir));
		let path = dir.join("fixture.typ");
		let src = "= Heading\n\n#context[whatever]\n";
		res!(std::fs::write(&path, src));

		let offset = res!(src.find("#context")
			.ok_or_else(|| err!("Fixture text lost its own marker."; Bug, Missing))) as u32;
		let refusals = Refusals::from_sites(vec![Refusal {
			name:	"#context".to_string(),
			span:	Span::new(offset, offset + "#context[whatever]".len() as u32),
			class:	RefusalClass::Introspective,
			file:	path.display().to_string(),
		}]);
		let report = explain_refusals(&refusals);

		let _ = std::fs::remove_file(&path);
		let _ = std::fs::remove_dir(&dir);

		let expected_head = fmt!("{}:3:1: introspective: skipped #context", path.display());
		assert!(report.starts_with(&expected_head), "report was {:?}", report);
		let lines: Vec<&str> = report.lines().collect();
		assert_eq!(lines.get(1), Some(&"#context[whatever]"), "source line missing: {:?}", report);
		assert_eq!(lines.get(2), Some(&"^"), "caret missing or misplaced: {:?}", report);
		Ok(())
	}
}
