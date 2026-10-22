//! `#v(..)` on the reader path (`austenite` without `--eval`, so `--explain` too) keeps Typst's meaning:
//! a strong space adds its amount to the block spacing, an `em` resolving against the text size in force,
//! and a weak one displaces the block spacing and collapses at the start of the flow. Before 2026-10-11 an
//! `em` or weak `#v` was refused and skipped.

use oxedyne_fe2o3_austenite::compile;
use oxedyne_fe2o3_austenite::fonts;
use oxedyne_fe2o3_austenite::page::PlacedKind;
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

const TOL: f64 = 0.05;	// points, rounding slack only
const HEAD: &str = "#set text(size: 11pt)\n#set par(spacing: 1.2em)\n";
const HEAD_14: &str = "#set text(size: 14pt)\n#set par(spacing: 1.2em)\n";
const SKIP: f64 = 13.2;	// the 1.2em block spacing at 11pt

/// The distinct baselines, in points, on the first page of [`HEAD`] + `body` compiled through the reader.
fn baselines(body: &str) -> Outcome<Vec<f64>> {
	baselines_under(HEAD, body)
}

fn baselines_under(head: &str, body: &str) -> Outcome<Vec<f64>> {
	let _turn	= turn();
	let src		= fmt!("{}\n{}\n", head, body);
	let mut map: HashMap<PathBuf, Vec<u8>> = HashMap::new();
	map.insert(PathBuf::from("/vspace/main.typ"), src.into_bytes());
	res!(vfs::install(map));
	let fonts	= Arc::new(res!(fonts::libertinus()));
	let run = || -> Outcome<Vec<i32>> {
		let assembled	= res!(compile::assemble(&PathBuf::from("/vspace/main.typ"), || Ok(fonts.clone())));
		if let Some(line) = assembled.refusals.report() {
			return Err(err!("The reader refused part of the source: {}", line; Invalid, Input));
		}
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
	Ok(bases.into_iter().map(|b| b as f64 / 65536.0).collect())
}

/// The baseline gap between two one-line paragraphs with `between` set on its own line between them.
fn gap(between: &str) -> Outcome<f64> {
	gap_under(HEAD, between)
}

fn gap_under(head: &str, between: &str) -> Outcome<f64> {
	let bases = res!(baselines_under(head, &fmt!("First paragraph.\n\n{}\n\nSecond paragraph.", between)));
	// A folio may follow at the page foot; the two paragraphs are the first two baselines.
	if bases.len() < 2 {
		return Err(err!("Expected two body baselines, found {:?}.", bases; Invalid, Output));
	}
	Ok(bases[1] - bases[0])
}

fn near(got: f64, want: f64, what: &str) -> Outcome<()> {
	if (got - want).abs() > TOL {
		return Err(err!("{}: {:.3}pt, expected {:.3}pt.", what, got, want; Invalid, Output));
	}
	Ok(())
}

#[test]
fn an_em_space_adds_one_em_of_the_text_size() -> Outcome<()> {
	let plain	= res!(gap(""));
	let spaced	= res!(gap("#v(1em)"));
	near(spaced - plain, 11.0, "#v(1em) at 11pt")
}

#[test]
fn a_point_space_adds_its_points() -> Outcome<()> {
	let plain	= res!(gap(""));
	let spaced	= res!(gap("#v(12pt)"));
	near(spaced - plain, 12.0, "#v(12pt)")
}

#[test]
fn an_em_space_follows_the_text_size() -> Outcome<()> {
	let plain	= res!(gap_under(HEAD_14, ""));
	let spaced	= res!(gap_under(HEAD_14, "#v(1.5em)"));
	near(spaced - plain, 21.0, "#v(1.5em) at 14pt")
}

#[test]
fn a_weak_space_displaces_the_block_spacing() -> Outcome<()> {
	let plain	= res!(gap(""));
	let weak	= res!(gap("#v(30pt, weak: true)"));
	res!(near(weak - plain, 30.0 - SKIP, "#v(30pt, weak: true)"));
	// Two weak spaces with nothing between collapse to the larger.
	let both	= res!(gap("#v(20pt, weak: true)\n#v(30pt, weak: true)"));
	near(both - plain, 30.0 - SKIP, "two weak spaces")
}

#[test]
fn a_weak_space_collapses_at_the_start_of_the_flow() -> Outcome<()> {
	let plain	= res!(baselines("First paragraph."));
	let weak	= res!(baselines("#v(30pt, weak: true)\n\nFirst paragraph."));
	if plain.is_empty() || weak.is_empty() {
		return Err(err!("A first baseline is missing."; Invalid, Output));
	}
	near(weak[0], plain[0], "a weak space at the flow's start")
}
