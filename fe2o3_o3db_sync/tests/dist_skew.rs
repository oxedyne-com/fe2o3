#![cfg(feature = "dist")]
//! Tests for the per-peer clock skew of [`check_convergence`] (A1 QA finding 2-F5).
//!
//! With one clock for every peer, a rule that answers `Refuse` to a time it has not
//! yet reached passes the check, although it makes the converged state depend on
//! which peer a record landed on. `Convergence::with_skew_ms` gives each peer a
//! seeded offset, and every full round moves every clock on by `skew_ms`, so that
//! `Defer` until the time comes cures and `Refuse` is reported.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_o3db_sync::dist::{
	record::{
		Record,
		RecordId,
	},
	resolve::{
		Convergence,
		LastVersionWins,
		ReadView,
		ResolveCtx,
		Resolver,
		Verdict,
		check_convergence,
	},
};

use std::{
	collections::BTreeSet,
	sync::{
		Arc,
		Mutex,
	},
};


const NOW:	u64 = 1_000;

fn rid(b: u8) -> RecordId {
	let mut bytes = [0u8; 32];
	bytes[0] = b;
	RecordId::from_bytes(bytes)
}

/// `n` records on "identity", each stamped with this version.
fn stamped(n: u8, stamp: u64) -> Vec<Record> {
	(1..=n).map(|i| Record::new(rid(i), "identity", LastVersionWins::value(stamp, &[i]))).collect()
}

/// Last-version-wins, except that a record stamped after the clock is not yet valid.
#[derive(Clone)]
struct TimeGate {
	refuse:	bool,					// answer Refuse where a sound rule answers Defer
	seen:	Arc<Mutex<Vec<u64>>>,	// every clock this rule was asked with, in order
}

impl TimeGate {
	fn new(refuse: bool) -> Self {
		Self { refuse, seen: Arc::new(Mutex::new(Vec::new())) }
	}
}

impl Resolver for TimeGate {
	fn resolve<V: ReadView>(
		&self,
		ctx:		&ResolveCtx,
		view:		&V,
		table:		&str,
		id:			&RecordId,
		held:		Option<&[u8]>,
		incoming:	&[u8],
	)
		-> Outcome<Verdict>
	{
		{
			let mut seen = lock_mutex!(self.seen);
			seen.push(ctx.now_ms);
		}
		if let Some(stamp) = LastVersionWins::version(incoming) {
			if stamp > ctx.now_ms {
				return Ok(if self.refuse {
					Verdict::Refuse(fmt!("stamp {} is after the clock {}", stamp, ctx.now_ms))
				} else {
					Verdict::Defer
				});
			}
		}
		LastVersionWins.resolve(ctx, view, table, id, held, incoming)
	}
}

/// Runs the check and returns the clocks each peer's rule was asked with, one list
/// per peer per seed, in the order the harness built them (seed by seed, peer by peer).
fn run(cfg: &Convergence, records: &[Record], refuse: bool) -> Outcome<(Outcome<()>, Vec<Vec<u64>>)> {
	let mut gates: Vec<TimeGate> = Vec::new();
	let result = check_convergence(
		cfg,
		&["identity"],
		records,
		|_| {
			let g = TimeGate::new(refuse);
			gates.push(g.clone());
			g
		},
		|_, _, _| Ok(Vec::new()),
	);
	let mut clocks = Vec::new();
	for g in &gates {
		clocks.push(lock_mutex!(g.seen).clone());
	}
	Ok((result, clocks))
}

#[test]
fn without_skew_every_peer_reads_the_one_clock() -> Outcome<()> {
	let cfg = Convergence::new(3, 0..4, NOW, 8);
	let (result, clocks) = res!(run(&cfg, &stamped(6, 1), false));
	res!(result);
	assert_eq!(clocks.len(), 12);
	for log in &clocks {
		assert!(!log.is_empty());
		assert!(log.iter().all(|c| *c == NOW), "a clock moved with no skew: {:?}", log);
	}
	Ok(())
}

#[test]
fn skewed_clocks_differ_by_peer_but_are_seeded_and_never_run_back() -> Outcome<()> {
	let (step, rounds) = (500u64, 8usize);
	let cfg = Convergence::new(3, 0..8, NOW, rounds).with_skew_ms(step);
	let (result, clocks) = res!(run(&cfg, &stamped(8, 1), false));
	res!(result);
	let mut offsets = BTreeSet::new();
	for log in &clocks {
		assert!(!log.is_empty());
		// Each full round moves a clock on by `step`, so the remainder is the peer's own offset.
		let offset = (log[0] - NOW) % step;
		offsets.insert(offset);
		for w in log.windows(2) {
			assert!(w[0] <= w[1], "a clock ran back: {:?}", log);
		}
		for c in log {
			assert!(*c >= NOW);
			assert_eq!((*c - NOW) % step, offset, "a peer's offset changed: {:?}", log);
			assert!(*c <= NOW + step * (rounds as u64 + 1), "clock {} is out of range", c);
		}
	}
	assert!(offsets.len() > 1, "every peer has the same offset: {:?}", offsets);
	// The offset is drawn per peer, not per seed: peers of one seed differ in some seed.
	let mut mixed = false;
	for seed_peers in clocks.chunks(3) {
		let own: BTreeSet<u64> = seed_peers.iter().map(|log| (log[0] - NOW) % step).collect();
		mixed = mixed || own.len() > 1;
	}
	assert!(mixed, "no seed gave its peers different offsets");
	// The same seeds draw the same offsets.
	let (again, replay) = res!(run(&cfg, &stamped(8, 1), false));
	res!(again);
	assert_eq!(clocks, replay);
	Ok(())
}

#[test]
fn defer_until_the_clock_passes_converges_under_skew() -> Outcome<()> {
	// A stamp inside the spread of the clocks: some peers are behind it at first.
	let cfg = Convergence::new(3, 0..16, NOW, 8).with_skew_ms(400);
	let (result, _) = res!(run(&cfg, &stamped(6, NOW + 200), false));
	res!(result);
	Ok(())
}

#[test]
fn refuse_on_a_time_passes_one_clock_and_fails_under_skew() -> Outcome<()> {
	let records = stamped(6, NOW + 200);
	// One clock for every peer: all refuse alike, so the check cannot see the fault.
	let one_clock = Convergence::new(3, 0..16, NOW, 8);
	let (result, _) = res!(run(&one_clock, &records, true));
	res!(result);
	// Skewed: the state depends on which peer a record landed on, and the check says so.
	let skewed = Convergence::new(3, 0..16, NOW, 8).with_skew_ms(400);
	let (result, _) = res!(run(&skewed, &records, true));
	let text = match result {
		Ok(())	=> String::new(),
		Err(e)	=> fmt!("{}", e),
	};
	assert!(text.contains("differ"), "the skewed run was not reported as divergent: {}", text);
	Ok(())
}
