//! Model elements and block bodies laid out against the `typst` 0.15.1 oracle at level 4 (U6b-S2): every
//! line's text, baseline and start, through the whole pipeline (the fixpoint, so an outline's page numbers,
//! a footnote's number, a reference's target and what a block, a list item or a cell holds when it is laid out
//! again are the converged ones).
//!
//! The fixtures live in the areas below, one theme to an area. Each must pass level 4 in full: a fixture that
//! is not yet right is not kept here, it is an `unsupported` warning in the sweep (`eval_model_sweep`) until it
//! is. A fixture marked `oracle: rejects` must fail with Typst's first error, message and position. The suite
//! fails when fewer than `MIN_COMPARED` fixtures were compared or `MIN_REJECTED` rejected, so a harness that
//! stops finding them cannot pass for nothing.

#![allow(dead_code)]

#[path = "eval_oracle/mod.rs"]
mod harness;

use harness::corpus::{
	self,
	Expect,
	Filter,
};
use harness::oracle::Oracle;
use harness::Verdict;

use oxedyne_fe2o3_core::prelude::*;

const AREAS:			&[&str]	= &["bodies", "lists", "notes", "refs", "locate"];
const MIN_COMPARED:		usize	= 55;
const MIN_REJECTED:		usize	= 3;

#[test]
fn model_layout_matches_the_typst_oracle_at_level_4() -> Outcome<()> {
	let oracle = match res!(Oracle::find()) {
		Some(o)	=> o,
		None	=> return Ok(()),
	};
	let limit = harness::timeout();
	let only = std::env::var("EVAL_ORACLE_FIXTURE").ok().filter(|s| !s.trim().is_empty());
	let (mut compared, mut rejected, mut failures) = (0usize, 0usize, Vec::new());
	for area in AREAS {
		for fx in res!(corpus::discover(&Filter::only(area))) {
			if let Some(o) = &only {
				if !fx.id().contains(o.as_str()) {
					continue;
				}
			}
			let rep = res!(harness::check(&fx, &oracle, limit));
			if let Some(f) = &rep.fault {
				failures.push(fmt!("{}: {}", rep.id, f));
				continue;
			}
			if fx.expect == Expect::Rejects {
				match &rep.rejection {
					Verdict::Pass		=> rejected += 1,
					other				=> failures.push(fmt!("{}: the first error differs from Typst's ({:?})", rep.id, other)),
				}
				continue;
			}
			match &rep.levels[3] {
				Verdict::Pass		=> compared += 1,
				Verdict::Fail(d)	=> {
					compared += 1;
					failures.push(fmt!("{}: L4: {}", rep.id, d.iter().take(4).cloned().collect::<Vec<_>>().join(" | ")));
				},
				other				=> failures.push(fmt!("{}: L4 was not compared ({:?})", rep.id, other)),
			}
		}
	}
	println!("[model-layout] {} fixture(s) compared at level 4, {} rejected as Typst rejects them, {} failure(s)",
		compared, rejected, failures.len());
	assert!(failures.is_empty(), "{} failure(s):\n{}", failures.len(), failures.join("\n"));
	if only.is_none() {
		assert!(compared >= MIN_COMPARED, "only {} fixture(s) compared; {} are expected", compared, MIN_COMPARED);
		assert!(rejected >= MIN_REJECTED, "only {} fixture(s) rejected; {} are expected", rejected, MIN_REJECTED);
	}
	Ok(())
}
