//! `#align(alignment)[body]`, an own-line `#text(args)[body]` and `#set heading(numbering: none)` on the
//! reader path (`austenite` without `--eval`). Before 2026-10-11 the first two were skipped, their words
//! dropped, and a chapter still drew its giant number under `numbering: none`.

use oxedyne_fe2o3_austenite::compile;
use oxedyne_fe2o3_austenite::compile::Rendered;
use oxedyne_fe2o3_austenite::fonts;
use oxedyne_fe2o3_austenite::page::PlacedKind;
use oxedyne_fe2o3_austenite::vfs;

use oxedyne_fe2o3_core::prelude::*;

use std::collections::BTreeMap;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{
	Arc,
	Mutex,
	MutexGuard,
};

// The source map is a process global, so the tests take turns.
static VFS_TURN: Mutex<()> = Mutex::new(());

fn turn() -> MutexGuard<'static, ()> {
	VFS_TURN.lock().unwrap_or_else(|p| p.into_inner())
}

const TOL: f64 = 0.05;	// points, rounding slack only

const PARA: &str = "The quick brown fox jumps over the lazy dog while the patient reader waits for the \
	paragraph to wrap across several lines, so that the page fills and the last line falls short.";

fn render(src: &str) -> Outcome<Rendered> {
	let _turn	= turn();
	let mut map: HashMap<PathBuf, Vec<u8>> = HashMap::new();
	map.insert(PathBuf::from("/aligntext/main.typ"), src.as_bytes().to_vec());
	res!(vfs::install(map));
	let fonts	= Arc::new(res!(fonts::libertinus()));
	let run = || -> Outcome<Rendered> {
		let assembled = res!(compile::assemble(&PathBuf::from("/aligntext/main.typ"), || Ok(fonts.clone())));
		compile::author_and_run(assembled)
	};
	let out = run();
	res!(vfs::clear());
	out
}

/// The first page's body lines, top to bottom, each as (left edge, right edge, its words), measured
/// from the content block's left edge.
fn lines(r: &Rendered) -> Vec<(f64, f64, String)> {
	let left	= r.geom.content_left().to_pt();
	let bottom	= r.geom.content_top() + r.geom.content_height();
	let mut by_y: BTreeMap<i32, (f64, f64, String)> = BTreeMap::new();
	if let Some(page) = r.out.pages.first() {
		for p in &page.frame.placed {
			if let PlacedKind::Text(t) = &p.kind {
				if p.y >= bottom || p.y + p.dims.height < r.geom.content_top() {
					continue;
				}
				let (x0, x1) = (p.x.to_pt() - left, (p.x + p.dims.width).to_pt() - left);
				let e = by_y.entry(p.y.raw()).or_insert((f64::MAX, f64::MIN, String::new()));
				if x0 < e.0 { e.0 = x0; }
				if x1 > e.1 { e.1 = x1; }
				e.2.push_str(t.source());
			}
		}
	}
	by_y.into_values().collect()
}

fn measure(r: &Rendered) -> f64 { r.geom.content_width().to_pt() }

#[test]
fn a_centred_block_centres_its_short_lines_and_not_its_justified_ones() -> Outcome<()> {
	let r = res!(render(&fmt!("#set par(justify: true)\n\n#align(center)[\n  {}\n]\n", PARA)));
	let ls = lines(&r);
	let w = measure(&r);
	if ls.len() < 2 {
		return Err(err!("The centred paragraph set {} lines, wanting at least two.", ls.len(); Test, Mismatch));
	}
	for (i, (x0, x1, words)) in ls[..ls.len() - 1].iter().enumerate() {
		if x0.abs() > TOL || (x1 - w).abs() > TOL {
			return Err(err!("Justified line {} ({:?}) runs {}..{}pt of a {}pt measure.", i, words, x0, x1, w; Test, Mismatch));
		}
	}
	let (x0, x1, words) = &ls[ls.len() - 1];
	if *x0 < 1.0 || ((w - x1) - x0).abs() > TOL {
		return Err(err!("The short last line ({:?}) runs {}..{}pt of a {}pt measure, not centred.", words, x0, x1, w; Test, Mismatch));
	}
	Ok(())
}

#[test]
fn a_right_aligned_block_sets_every_ragged_line_at_the_right_edge() -> Outcome<()> {
	let r = res!(render(&fmt!("#set par(justify: false)\n\n#align(right)[\n  {}\n]\n", PARA)));
	let ls = lines(&r);
	let w = measure(&r);
	if ls.len() < 2 {
		return Err(err!("The right-aligned paragraph set {} lines, wanting at least two.", ls.len(); Test, Mismatch));
	}
	for (x0, x1, words) in &ls {
		if (x1 - w).abs() > TOL || *x0 < 1.0 {
			return Err(err!("Line {:?} runs {}..{}pt of a {}pt measure, not flush right.", words, x0, x1, w; Test, Mismatch));
		}
	}
	Ok(())
}

#[test]
fn an_own_line_text_call_sets_its_body_bold_at_its_size() -> Outcome<()> {
	let r = res!(render("#text(size: 26pt, weight: \"bold\")[Xylophone]\n\nAfter.\n"));
	let page = res!(r.out.pages.first().ok_or_else(|| err!("No page was set."; Test, Missing)));
	let mut found = false;
	for p in &page.frame.placed {
		if let PlacedKind::Text(t) = &p.kind {
			if t.source().contains("Xylophone") {
				found = true;
				let info = res!(t.face_info());
				if (t.size() as f64 - 26.0).abs() > TOL || info.weight < 600 {
					return Err(err!("Xylophone was set at {}pt in weight {}, not bold at 26pt.", t.size(), info.weight; Test, Mismatch));
				}
			}
			if t.source().contains("After") && (t.size() as f64 - 26.0).abs() < 1.0 {
				return Err(err!("The text call's size leaked into the next paragraph."; Test, Mismatch));
			}
		}
	}
	if !found {
		return Err(err!("The text call's body was not set."; Test, Missing));
	}
	Ok(())
}

/// How many marks the first page draws above its first heading, a chapter's display numeral among them.
fn marks_above_heading(r: &Rendered) -> usize {
	let Some(page) = r.out.pages.first() else { return 0; };
	let head = page.frame.placed.iter().find_map(|p| match &p.kind {
		PlacedKind::Text(t) if t.source() == "Alpha"	=> Some(p.y),
		_											=> None,
	});
	match head {
		Some(y) => page.frame.placed.iter().filter(|p| p.y < y && p.y >= r.geom.content_top()).count(),
		None	=> 0,
	}
}

#[test]
fn heading_numbering_none_draws_no_number() -> Outcome<()> {
	let r = res!(render(&fmt!("#set heading(numbering: none)\n\n= Alpha\n\n{}\n\n== Beta\n\n{}\n", PARA, PARA)));
	let n = marks_above_heading(&r);
	if n != 0 {
		return Err(err!("Under numbering: none the page still drew {} marks above the heading.", n; Test, Mismatch));
	}
	Ok(())
}

#[test]
fn without_the_set_a_chapter_keeps_its_number() -> Outcome<()> {
	let r = res!(render(&fmt!("= Alpha\n\n{}\n", PARA)));
	if marks_above_heading(&r) == 0 {
		return Err(err!("With no heading set the chapter drew no number."; Test, Missing));
	}
	Ok(())
}

