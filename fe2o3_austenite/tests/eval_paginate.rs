//! Paginated text against the `typst` 0.15.1 oracle at level 4 (U6b-S1): page count, page sizes, and every
//! line's text and baseline, for documents whose text runs across pages -- a paragraph, a deferred float,
//! `pagebreak(to: "odd")`, a breakable block and `columns(2)[..]`.
//!
//! Austenite's side paginates a page at a time, as the pipeline does: each body is placed, its text runs
//! read, and the page dropped. A fixture whose first lines say `// needs: <unit>` depends on a unit not yet
//! merged: while Austenite cannot lay it out it is reported as pending, and once it can it is held to the
//! oracle like any other.

#![allow(dead_code)]

#[path = "eval_oracle/mod.rs"]
mod harness;

use harness::corpus::{
	Expect,
	Fixture,
};
use harness::layout::{
	compare,
	APage,
	ARun,
};
use harness::oracle::Oracle;

use oxedyne_fe2o3_austenite::driver::{
	self,
	Nowhere,
};
use oxedyne_fe2o3_austenite::eval::styles::StyleChain;
use oxedyne_fe2o3_austenite::eval::{
	eval_source,
	Engine,
	World,
};
use oxedyne_fe2o3_austenite::flow;
use oxedyne_fe2o3_austenite::page::PlacedKind;

use oxedyne_fe2o3_core::prelude::*;

use std::path::{
	Path,
	PathBuf,
};

const AREA: &str = "paginate";

fn dir() -> PathBuf {
	Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/eval").join(AREA)
}

fn fixtures() -> Outcome<Vec<(Fixture, Vec<String>)>> {
	let root = dir();
	let mut files: Vec<PathBuf> = match std::fs::read_dir(&root) {
		Ok(rd)	=> rd.filter_map(|e| e.ok().map(|e| e.path())).collect(),
		Err(e)	=> return Err(err!("Cannot read {}: {}", root.display(), e; IO, File, Read)),
	};
	files.sort();
	let mut out = Vec::new();
	for path in files {
		let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
		if !name.ends_with(".typ") || name.starts_with('_') {
			continue;
		}
		let text = res!(std::fs::read_to_string(&path).map_err(|e| err!("Cannot read {}: {}", path.display(), e; IO, File, Read)));
		let needs = text.lines()
			.take_while(|l| l.starts_with("//"))
			.filter_map(|l| l.strip_prefix("// needs:"))
			.flat_map(|l| l.split_whitespace().map(|s| s.to_string()).collect::<Vec<_>>())
			.collect();
		let fx = Fixture {
			area:	AREA.to_string(),
			name:	name.trim_end_matches(".typ").to_string(),
			path,
			root:	root.clone(),
			text,
			levels:	[false, false, false, true],
			expect:	Expect::Accepts,
		};
		out.push((fx, needs));
	}
	Ok(out)
}

/// Austenite's pages, read as they stream: each page's text runs are kept, the page itself dropped.
fn austenite(fx: &Fixture) -> std::result::Result<Vec<APage>, String> {
	let mut world = World::new(fx.root.clone());
	let id = match world.load(&fx.path) {
		Ok(id)	=> id,
		Err(e)	=> return Err(fmt!("load: {}", e)),
	};
	let mut engine = Engine::new(world);
	let module = match eval_source(&mut engine, id) {
		Ok(m)	=> m,
		Err(e)	=> return Err(fmt!("eval: {}", last(&e))),
	};
	let mut paginator = flow::paginate(&module.content, &StyleChain::root());
	let mut pages = Vec::new();
	loop {
		let body = match paginator.next_page(&mut engine) {
			Ok(Some(b))	=> b,
			Ok(None)	=> break,
			Err(e)		=> return Err(fmt!("flow: {}", last(&e))),
		};
		let page = match driver::place_page(body, &mut Nowhere) {
			Ok(p)	=> p,
			Err(e)	=> return Err(fmt!("driver: {}", last(&e))),
		};
		pages.push(APage {
			width:	page.geom.width.to_pt(),
			height:	page.geom.height.to_pt(),
			runs:	page.frame.placed.iter().filter_map(|pl| match &pl.kind {
				PlacedKind::Text(st) => Some(ARun {
					text:	st.source().to_string(),
					x0:		pl.x.to_pt(),
					x1:		(pl.x + pl.dims.width).to_pt(),
					base:	(pl.y + pl.dims.height).to_pt(),
				}),
				_ => None,
			}).collect(),
		});
	}
	Ok(pages)
}

fn last(e: &Error<ErrTag>) -> String {
	match e.msgs().into_iter().last() {
		Some(m)	=> m,
		None	=> fmt!("{}", e),
	}
}

#[test]
fn paginated_text_matches_the_typst_oracle() -> Outcome<()> {
	let oracle = match res!(Oracle::find()) {
		Some(o)	=> o,
		None	=> return Ok(()),
	};
	let only = std::env::var("EVAL_PAGINATE_FIXTURE").ok();
	let (mut compared, mut pending, mut failures) = (0usize, Vec::new(), Vec::new());
	for (fx, needs) in res!(fixtures()) {
		if let Some(o) = &only {
			if !fx.name.contains(o.as_str()) {
				continue;
			}
		}
		let want = match res!(oracle.layout(&fx)) {
			Ok(w)	=> w,
			Err(e)	=> {
				failures.push(fmt!("{}: typst: {}", fx.id(), e));
				continue;
			},
		};
		let got = match austenite(&fx) {
			Ok(g)	=> g,
			Err(e)	=> {
				if needs.is_empty() {
					failures.push(fmt!("{}: {}", fx.id(), e));
				} else {
					pending.push(fmt!("{} (needs {}): {}", fx.id(), needs.join(", "), e));
				}
				continue;
			},
		};
		let mut diffs = Vec::new();
		compare(&want, &got, &mut diffs);
		if diffs.is_empty() {
			compared += 1;
			println!("ok      {} ({} pages)", fx.id(), want.len());
		} else {
			failures.push(fmt!("{}:\n    {}", fx.id(), diffs.join("\n    ")));
			if std::env::var("EVAL_PAGINATE_DUMP").is_ok() {
				for (i, p) in want.iter().enumerate() {
					for l in &p.lines {
						println!("typst     p{} x {:7.2} base {:7.2} {:?}", i + 1, l.x0, l.base, l.text);
					}
				}
				for (i, p) in got.iter().enumerate() {
					for r in &p.runs {
						println!("austenite p{} x {:7.2} base {:7.2} {:?}", i + 1, r.x0, r.base, r.text);
					}
				}
			}
		}
	}
	for p in &pending {
		println!("pending {}", p);
	}
	for f in &failures {
		println!("FAIL    {}", f);
	}
	println!("{} fixtures compared at level 4; {} pending; {} failing", compared, pending.len(), failures.len());
	assert!(failures.is_empty(), "{} fixture(s) differ from typst", failures.len());
	Ok(())
}
