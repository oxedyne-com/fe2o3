//! `#set page(..)` on the reader path (`austenite` without `--eval`): `paper`, `width`, `height` and
//! `margin` lay the document's own geometry over the base, a one-sided margin is not mirrored, and
//! `numbering` sets Typst's page number or none. With no `par` set, the leading and paragraph spacing
//! are Typst's 0.65em and 1.2em. Before 2026-10-11 a `#set page` was refused and left the page at A4.

use oxedyne_fe2o3_austenite::compile;
use oxedyne_fe2o3_austenite::compile::Rendered;
use oxedyne_fe2o3_austenite::fonts;
use oxedyne_fe2o3_austenite::ir::Sp;
use oxedyne_fe2o3_austenite::page::PageGeometry;
use oxedyne_fe2o3_austenite::page::PlacedKind;
use oxedyne_fe2o3_austenite::theme::Theme;
use oxedyne_fe2o3_austenite::vfs;

use oxedyne_fe2o3_core::prelude::*;

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

const CM: f64 = 72.0 / 2.54;	// points in a centimetre
const TOL: f64 = 0.05;			// points, rounding slack only

const PARA: &str = "The quick brown fox jumps over the lazy dog while the patient reader waits for the \
	paragraph to wrap across several lines, so that the page fills and the next one begins.";

/// `head` followed by `paras` paragraphs, compiled through the reader path.
fn render(head: &str, paras: usize) -> Outcome<Rendered> {
	let _turn	= turn();
	let mut src	= fmt!("{}\n\n", head);
	for _ in 0..paras {
		src.push_str(PARA);
		src.push_str("\n\n");
	}
	let mut map: HashMap<PathBuf, Vec<u8>> = HashMap::new();
	map.insert(PathBuf::from("/pagesetup/main.typ"), src.into_bytes());
	res!(vfs::install(map));
	let fonts	= Arc::new(res!(fonts::libertinus()));
	let run = || -> Outcome<Rendered> {
		let assembled = res!(compile::assemble(&PathBuf::from("/pagesetup/main.typ"), || Ok(fonts.clone())));
		compile::author_and_run(assembled)
	};
	let out = run();
	res!(vfs::clear());
	out
}

fn near(a: Sp, want_pt: f64) -> bool {
	(a.to_pt() - want_pt).abs() < TOL
}

/// The texts set below the text block of each page, in page order.
fn foot_texts(r: &Rendered) -> Vec<Vec<String>> {
	let bottom = r.geom.content_top() + r.geom.content_height();
	r.out.pages.iter().map(|page| {
		page.frame.placed.iter().filter_map(|p| match &p.kind {
			PlacedKind::Text(t) if p.y >= bottom	=> Some(t.source().to_string()),
			_										=> None,
		}).collect()
	}).collect()
}

#[test]
fn margin_and_paper_change_the_geometry() -> Outcome<()> {
	let r = res!(render("#set page(margin: 2.2cm)", 1));
	let g = r.geom;
	for side in [g.inside, g.outside, g.top, g.bottom] {
		if !near(side, 2.2 * CM) {
			return Err(err!("A 2.2cm margin set a side of {}pt.", side.to_pt(); Test, Mismatch));
		}
	}
	if g.width != PageGeometry::a4().width {
		return Err(err!("A margin alone moved the page width to {}pt.", g.width.to_pt(); Test, Mismatch));
	}
	let r = res!(render("#set page(paper: \"a5\")", 1));
	if !near(r.geom.width, 148.0 / 25.4 * 72.0) || !near(r.geom.height, 210.0 / 25.4 * 72.0) {
		return Err(err!("Paper a5 set {}pt by {}pt.", r.geom.width.to_pt(), r.geom.height.to_pt(); Test, Mismatch));
	}
	let r = res!(render("#set page(paper: \"a5\", width: 100mm)", 1));
	if !near(r.geom.width, 100.0 / 25.4 * 72.0) || !near(r.geom.height, 210.0 / 25.4 * 72.0) {
		return Err(err!("A width over a5 set {}pt by {}pt.", r.geom.width.to_pt(), r.geom.height.to_pt(); Test, Mismatch));
	}
	Ok(())
}

#[test]
fn margin_dictionary_sides_beat_axes_beat_rest() -> Outcome<()> {
	let r = res!(render("#set page(margin: (rest: 1cm, x: 2cm, left: 3cm))", 1));
	let g = r.geom;
	let want = [(g.inside, 3.0), (g.outside, 2.0), (g.top, 1.0), (g.bottom, 1.0)];
	for (side, cm) in want {
		if !near(side, cm * CM) {
			return Err(err!("A margin dictionary set {}pt where {}cm was wanted.", side.to_pt(), cm; Test, Mismatch));
		}
	}
	Ok(())
}

#[test]
fn one_sided_margins_are_not_mirrored() -> Outcome<()> {
	let r = res!(render("#set page(paper: \"a6\", margin: (left: 3cm, right: 1cm))", 30));
	if r.out.pages.len() < 2 {
		return Err(err!("Expected at least two pages, found {}.", r.out.pages.len(); Invalid, Output));
	}
	for page in r.out.pages.iter().take(2) {
		let left = page.frame.placed.iter()
			.filter(|p| matches!(p.kind, PlacedKind::Text(_)))
			.map(|p| p.x)
			.min();
		match left {
			Some(x) if near(x, 3.0 * CM)	=> {},
			other							=> return Err(err!(
				"Page {} sets its text from {:?}, not the 3cm left margin.", page.number, other.map(|x| x.to_pt());
				Test, Mismatch)),
		}
	}
	Ok(())
}

#[test]
fn numbering_pattern_numbers_every_page() -> Outcome<()> {
	let r = res!(render("#set page(paper: \"a6\", numbering: \"1\")", 30));
	let feet = foot_texts(&r);
	if feet.len() < 2 {
		return Err(err!("Expected at least two pages, found {}.", feet.len(); Invalid, Output));
	}
	for (i, foot) in feet.iter().enumerate() {
		if foot != &vec![fmt!("{}", i + 1)] {
			return Err(err!("Page {} carries {:?} at its foot.", i + 1, foot; Test, Mismatch));
		}
	}
	// The number's cap top stands three-tenths of the bottom margin below the text block.
	let g		= r.geom;
	let page	= &r.out.pages[0];
	let bottom	= g.content_top() + g.content_height();
	let num = page.frame.placed.iter().find(|p| p.y >= bottom);
	match num {
		Some(p) if p.y < bottom + Sp::from_pt(0.3 * g.bottom.to_pt())	=> {},
		other	=> return Err(err!("The page number stands at {:?}.", other.map(|p| p.y.to_pt()); Test, Mismatch)),
	}
	Ok(())
}

#[test]
fn numbering_none_numbers_no_page() -> Outcome<()> {
	let r = res!(render("#set page(paper: \"a6\", numbering: none)", 30));
	let feet = foot_texts(&r);
	if feet.len() < 2 || feet.iter().any(|f| !f.is_empty()) {
		return Err(err!("numbering: none left {:?} at the feet.", feet; Test, Mismatch));
	}
	Ok(())
}

#[test]
fn no_set_page_keeps_the_geometry() -> Outcome<()> {
	let r = res!(render("#set text(size: 11pt)", 1));
	if r.geom != PageGeometry::a4() {
		return Err(err!("A file with no #set page took {:?}.", r.geom; Test, Mismatch));
	}
	Ok(())
}

#[test]
fn defaults_are_typst_leading_and_spacing() -> Outcome<()> {
	let t		= Theme::default();
	let gap		= res!(t.text.leading_gap.ok_or_else(|| err!("No default leading gap."; Test, Missing)));
	if gap.em != 0.65 || gap.abs != Sp::ZERO {
		return Err(err!("The default leading gap is {:?}, not 0.65em.", gap; Test, Mismatch));
	}
	if t.par.skip_em != Some(1.2) || !near(t.par.skip, 1.2 * 11.0) {
		return Err(err!("The default paragraph spacing is {:?} ({}pt), not 1.2em.", t.par.skip_em, t.par.skip.to_pt();
			Test, Mismatch));
	}
	// Both follow a body size set without a `par`.
	let mut big = Theme::default();
	let mut patch = oxedyne_fe2o3_austenite::theme::ThemePatch::default();
	patch.text.body_size = Some(Sp::from_pt(20.0));
	big.apply(&patch);
	if !near(big.par.skip, 24.0) || !near(big.text.leading, (t.calibration.line_box_em + 0.65) * 20.0) {
		return Err(err!("At 20pt the spacing is {}pt and the pitch {}pt.", big.par.skip.to_pt(), big.text.leading.to_pt();
			Test, Mismatch));
	}
	Ok(())
}
