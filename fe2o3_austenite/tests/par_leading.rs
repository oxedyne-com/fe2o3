//! `#set par(leading: ..)` on the reader path (`austenite` without `--eval`) keeps Typst's meaning: the
//! gap added between line boxes, in `em` or `pt`, so the baseline pitch is the calibrated line box plus
//! that gap. Before 2026-10-11 an `em` leading was refused and a `pt` one was taken as the pitch itself.

use oxedyne_fe2o3_austenite::compile;
use oxedyne_fe2o3_austenite::fonts;
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

// A quarter point, in scaled points: slack for rounding only.
const TOL: f64 = 0.25;

const BODY: &str = "The quick brown fox jumps over the lazy dog while the patient reader waits for the \
	paragraph to wrap across several lines, so that the distance between one baseline and the next can be \
	measured as many times as a reader might wish, without a heading or a list to disturb the count. \
	Another sentence follows to make quite sure that the paragraph runs to five or six lines at the least.";

/// The most common gap, in points, between consecutive distinct baselines of `head` + [`BODY`] compiled
/// through the reader path. The body is one paragraph, so every gap is a line pitch.
fn body_pitch(head: &str) -> Outcome<f64> {
	let _turn	= turn();
	let src		= fmt!("{}\n\n{}\n", head, BODY);
	let mut map: HashMap<PathBuf, Vec<u8>> = HashMap::new();
	map.insert(PathBuf::from("/leading/main.typ"), src.into_bytes());
	res!(vfs::install(map));
	let fonts	= Arc::new(res!(fonts::libertinus()));
	let run = || -> Outcome<Vec<i32>> {
		let assembled	= res!(compile::assemble(&PathBuf::from("/leading/main.typ"), || Ok(fonts.clone())));
		let rendered	= res!(compile::author_and_run(assembled));
		let mut bases: Vec<i32> = Vec::new();
		for page in rendered.out.pages.iter().take(1) {
			for placed in &page.frame.placed {
				if let PlacedKind::Text(_) = &placed.kind {
					let base = placed.y.raw() + placed.dims.height.raw();
					if !bases.contains(&base) {
						bases.push(base);
					}
				}
			}
		}
		Ok(bases)
	};
	let out = run();
	res!(vfs::clear());
	let mut bases = res!(out);
	bases.sort();
	if bases.len() < 4 {
		return Err(err!("Expected at least four body lines, found {}.", bases.len(); Invalid, Output));
	}
	let mut gaps: Vec<i32> = bases.windows(2).map(|w| w[1] - w[0]).collect();
	gaps.sort();
	let mut best = (gaps[0], 0usize);
	for g in &gaps {
		let n = gaps.iter().filter(|x| *x == g).count();
		if n > best.1 {
			best = (*g, n);
		}
	}
	Ok(best.0 as f64 / 65536.0)
}

fn line_box_pt(body_pt: f64) -> f64 {
	Theme::default().calibration.line_box_em * body_pt
}

#[test]
fn unset_leading_keeps_the_theme_default_pitch() -> Outcome<()> {
	let pitch	= res!(body_pitch(""));
	let want	= Theme::default().text.leading.to_pt();
	if (pitch - want).abs() > TOL {
		return Err(err!("Unset leading set a {:.3}pt pitch, the theme default is {:.3}pt.", pitch, want; Invalid, Output));
	}
	Ok(())
}

#[test]
fn em_leading_is_the_gap_between_line_boxes() -> Outcome<()> {
	let pitch	= res!(body_pitch("#set par(leading: 0.9em)"));
	let want	= line_box_pt(11.0) + 0.9 * 11.0;
	if (pitch - want).abs() > TOL {
		return Err(err!("A 0.9em leading set a {:.3}pt pitch, want {:.3}pt.", pitch, want; Invalid, Output));
	}
	// The em follows the text size in force, wherever the `#set text` sits in the batch.
	let pitch	= res!(body_pitch("#set par(leading: 0.65em)\n#set text(size: 14pt)"));
	let want	= line_box_pt(14.0) + 0.65 * 14.0;
	if (pitch - want).abs() > TOL {
		return Err(err!("A 0.65em leading at 14pt set a {:.3}pt pitch, want {:.3}pt.", pitch, want; Invalid, Output));
	}
	Ok(())
}

#[test]
fn pt_leading_is_the_gap_between_line_boxes() -> Outcome<()> {
	// Above the reader's floor: a pitch never drops below the face's nominal ascender + descender (~1.14em),
	// so a gap under ~0.48em does not tighten further there, unlike Typst.
	let pitch	= res!(body_pitch("#set par(leading: 8pt)"));
	let want	= line_box_pt(11.0) + 8.0;
	if (pitch - want).abs() > TOL {
		return Err(err!("An 8pt leading set a {:.3}pt pitch, want {:.3}pt.", pitch, want; Invalid, Output));
	}
	Ok(())
}
