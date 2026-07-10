//! Integration tests for the OAM placement primitive.
#![cfg(feature = "dist")]

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_o3db_sync::kademlia::id::{
	Distance,
	NodeId,
};
use oxedyne_fe2o3_o3db_sync::oam::{
	config::OamConfig,
	placement,
	threshold::Threshold,
};


/// Deterministic pseudo-random 32-byte generator based on splitmix64.
///
/// Integration tests need a reproducible source of "random-looking" 256-bit
/// identifiers without depending on any particular RNG crate; splitmix64 is
/// compact and well-behaved for this.
struct Rng { state: u64 }

impl Rng {
	fn new(seed: u64) -> Self {
		Self { state: seed }
	}

	fn next_u64(&mut self) -> u64 {
		self.state = self.state.wrapping_add(0x9E3779B97F4A7C15);
		let mut z = self.state;
		z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
		z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
		z ^ (z >> 31)
	}

	fn next_id(&mut self) -> NodeId {
		let mut bytes = [0u8; 32];
		for i in 0..4 {
			let word = self.next_u64().to_le_bytes();
			bytes[i * 8..(i + 1) * 8].copy_from_slice(&word);
		}
		NodeId::from_bytes(bytes)
	}
}


fn node_id_from_u8(b: u8) -> NodeId {
	let mut bytes = [0u8; 32];
	bytes[31] = b;
	NodeId::from_bytes(bytes)
}


#[test]
fn threshold_none_excludes_everything() -> Outcome<()> {
	let t = Threshold::from_params(0, 100);
	assert!(matches!(t, Threshold::None));
	let p = node_id_from_u8(1);
	let h = node_id_from_u8(2);
	assert!(!placement::is_holder(&p, &h, &t));
	Ok(())
}

#[test]
fn threshold_all_includes_everything() -> Outcome<()> {
	let t = Threshold::from_params(20, 20);
	assert!(matches!(t, Threshold::All));
	let mut rng = Rng::new(0xa5a5a5);
	for _ in 0..50 {
		let p = rng.next_id();
		let h = rng.next_id();
		assert!(placement::is_holder(&p, &h, &t));
	}
	Ok(())
}

#[test]
fn threshold_half_network_top_bit_set() -> Outcome<()> {
	let t = Threshold::from_params(1, 2);
	let bytes = match &t {
		Threshold::Bounded(b) => b,
		_ => return Err(err!("expected Bounded threshold"; Bug, Mismatch)),
	};
	assert_eq!(bytes[0], 0x80);
	for b in &bytes[1..] {
		assert_eq!(*b, 0);
	}
	Ok(())
}

#[test]
fn threshold_quarter_network_second_bit_set() -> Outcome<()> {
	let t = Threshold::from_params(1, 4);
	let bytes = match &t {
		Threshold::Bounded(b) => b,
		_ => return Err(err!("expected Bounded threshold"; Bug, Mismatch)),
	};
	assert_eq!(bytes[0], 0x40);
	for b in &bytes[1..] {
		assert_eq!(*b, 0);
	}
	Ok(())
}

#[test]
fn config_rejects_zero_network_with_positive_replication() -> Outcome<()> {
	assert!(OamConfig::new(20, 0).is_err());
	Ok(())
}

#[test]
fn config_accepts_zero_replication_zero_network() -> Outcome<()> {
	let cfg = res!(OamConfig::new(0, 0));
	assert!(matches!(cfg.threshold(), Threshold::None));
	Ok(())
}

#[test]
fn config_default_replication_is_twenty() -> Outcome<()> {
	let cfg = res!(OamConfig::default_replication(1_000));
	assert_eq!(cfg.replication, 20);
	assert_eq!(cfg.network_size, 1_000);
	assert_eq!(cfg.expected_holders(), 20);
	Ok(())
}

#[test]
fn config_expected_holders_clamps_at_network() -> Outcome<()> {
	let cfg = res!(OamConfig::new(100, 10));
	assert_eq!(cfg.expected_holders(), 10);
	Ok(())
}

#[test]
fn is_holder_for_zero_distance_matches_threshold_positivity() -> Outcome<()> {
	// Equal peer id and record hash -> zero XOR distance. A Bounded threshold
	// of any positive value covers zero distance, so the peer always holds.
	let cfg = res!(OamConfig::new(1, 1_000));
	let t = cfg.threshold();
	assert!(matches!(t, Threshold::Bounded(_)));
	let same = node_id_from_u8(42);
	assert!(placement::is_holder(&same, &same, &t));
	Ok(())
}

#[test]
fn is_holder_is_deterministic() -> Outcome<()> {
	let cfg = res!(OamConfig::new(20, 500));
	let t = cfg.threshold();
	let mut rng = Rng::new(0x1234_5678);
	for _ in 0..1000 {
		let p = rng.next_id();
		let h = rng.next_id();
		let a = placement::is_holder(&p, &h, &t);
		let b = placement::is_holder(&p, &h, &t);
		assert_eq!(a, b);
	}
	Ok(())
}

#[test]
fn is_holder_is_symmetric_in_operands() -> Outcome<()> {
	// XOR is symmetric, so is_holder(p, h, t) == is_holder(h, p, t).
	let cfg = res!(OamConfig::new(20, 500));
	let t = cfg.threshold();
	let mut rng = Rng::new(0xdead_beef);
	for _ in 0..100 {
		let p = rng.next_id();
		let h = rng.next_id();
		assert_eq!(
			placement::is_holder(&p, &h, &t),
			placement::is_holder(&h, &p, &t),
		);
	}
	Ok(())
}

#[test]
fn uniform_sampling_converges_to_fraction() -> Outcome<()> {
	// For well-mixed ids and hashes, the fraction of holders should approach
	// n/N. Sample ten thousand random (peer, record) pairs at n=20, N=500 and
	// count holders; expected = 10 000 * 20 / 500 = 400, standard deviation
	// sqrt(400 * (1 - 20/500)) ~ 19.6. Accept a +/- 15% window (60 holders,
	// ~3 sigma) which is tight enough to catch bugs and loose enough not to
	// flake.
	let cfg = res!(OamConfig::new(20, 500));
	let t = cfg.threshold();
	let mut rng = Rng::new(0xaaaa_bbbb);
	let mut hits = 0usize;
	let trials = 10_000usize;
	for _ in 0..trials {
		let p = rng.next_id();
		let h = rng.next_id();
		if placement::is_holder(&p, &h, &t) {
			hits += 1;
		}
	}
	let expected = trials as f64 * cfg.replication as f64 / cfg.network_size as f64;
	let diff = (hits as f64 - expected).abs();
	assert!(
		diff / expected < 0.15,
		"uniform sampling off: hits={} expected={:.1} diff={:.1} (>15%)",
		hits, expected, diff,
	);
	Ok(())
}

#[test]
fn holders_filters_by_threshold() -> Outcome<()> {
	let cfg = res!(OamConfig::new(50, 500));
	let t = cfg.threshold();
	let mut rng = Rng::new(0xc0de_babe);
	let mut peers: Vec<NodeId> = (0..200).map(|_| rng.next_id()).collect();
	peers.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
	peers.dedup();

	let record = rng.next_id();
	let hs = placement::holders(&record, &peers, &t);
	// Every peer in hs must pass is_holder; every peer not in hs must fail.
	let picked: std::collections::HashSet<_> =
		hs.iter().map(|n| *n.as_bytes()).collect();
	for p in &peers {
		let expected = placement::is_holder(p, &record, &t);
		let actual = picked.contains(p.as_bytes());
		assert_eq!(
			expected, actual,
			"holders() disagreed with is_holder() for peer {}.", p,
		);
	}
	// The fraction should roughly match the configured ratio.
	let expected_count = peers.len() as f64 * 50.0 / 500.0;
	let diff = (hs.len() as f64 - expected_count).abs();
	assert!(
		diff / expected_count < 0.5,
		"holders() count off: got {} expected ~{:.1}",
		hs.len(), expected_count,
	);
	Ok(())
}

#[test]
fn holders_preserves_order() -> Outcome<()> {
	let cfg = res!(OamConfig::new(100, 200));
	let t = cfg.threshold();
	let mut rng = Rng::new(0x1111_2222);
	let peers: Vec<NodeId> = (0..50).map(|_| rng.next_id()).collect();
	let record = rng.next_id();
	let hs = placement::holders(&record, &peers, &t);
	// Scan peers in input order, collect those that pass; should match hs.
	let expected: Vec<&NodeId> = peers.iter()
		.filter(|p| placement::is_holder(p, &record, &t))
		.collect();
	assert_eq!(hs.len(), expected.len());
	for (a, b) in hs.iter().zip(expected.iter()) {
		assert_eq!(a.as_bytes(), b.as_bytes());
	}
	Ok(())
}

#[test]
fn holders_none_threshold_returns_empty() -> Outcome<()> {
	let t = Threshold::None;
	let mut rng = Rng::new(0);
	let peers: Vec<NodeId> = (0..10).map(|_| rng.next_id()).collect();
	let record = rng.next_id();
	assert!(placement::holders(&record, &peers, &t).is_empty());
	Ok(())
}

#[test]
fn holders_all_threshold_returns_all() -> Outcome<()> {
	let t = Threshold::All;
	let mut rng = Rng::new(0);
	let peers: Vec<NodeId> = (0..10).map(|_| rng.next_id()).collect();
	let record = rng.next_id();
	let hs = placement::holders(&record, &peers, &t);
	assert_eq!(hs.len(), peers.len());
	Ok(())
}

#[test]
fn closest_holders_orders_by_xor_distance() -> Outcome<()> {
	let mut rng = Rng::new(0xfeed_face);
	let peers: Vec<NodeId> = (0..100).map(|_| rng.next_id()).collect();
	let record = rng.next_id();
	let closest = placement::closest_holders(&record, &peers, 10);
	assert_eq!(closest.len(), 10);
	// Distances must be non-decreasing.
	let mut prev: Option<Distance> = None;
	for p in &closest {
		let d = p.distance(&record);
		if let Some(last) = prev {
			assert!(d >= last, "closest_holders not sorted by distance");
		}
		prev = Some(d);
	}
	// The first closest must be the global minimum across all peers.
	let global_min = peers.iter()
		.map(|p| p.distance(&record))
		.min();
	assert_eq!(Some(closest[0].distance(&record)), global_min);
	Ok(())
}

#[test]
fn closest_holders_returns_all_when_requested_exceeds_set() -> Outcome<()> {
	let mut rng = Rng::new(0x9876);
	let peers: Vec<NodeId> = (0..5).map(|_| rng.next_id()).collect();
	let record = rng.next_id();
	let closest = placement::closest_holders(&record, &peers, 100);
	assert_eq!(closest.len(), 5);
	Ok(())
}

#[test]
fn closest_holders_count_zero_is_empty() -> Outcome<()> {
	let mut rng = Rng::new(0);
	let peers: Vec<NodeId> = (0..5).map(|_| rng.next_id()).collect();
	let record = rng.next_id();
	assert!(placement::closest_holders(&record, &peers, 0).is_empty());
	Ok(())
}

#[test]
fn threshold_as_bytes_roundtrips_via_node_id() -> Outcome<()> {
	let t = Threshold::from_params(7, 1000);
	let bytes = res!(t.as_bytes().ok_or_else(|| err!(
		"Bounded threshold had no bytes"; Bug, Missing)));
	let nid = res!(t.as_node_id().ok_or_else(|| err!(
		"Bounded threshold had no NodeId view"; Bug, Missing)));
	assert_eq!(nid.as_bytes(), bytes);
	Ok(())
}

#[test]
fn monotone_in_replication_factor() -> Outcome<()> {
	// Larger n should yield a larger or equal threshold; a peer that holds at
	// n=k must still hold at n=k+1 (all else equal).
	let cfg_small = res!(OamConfig::new(10, 1000));
	let cfg_large = res!(OamConfig::new(30, 1000));
	let ts = cfg_small.threshold();
	let tl = cfg_large.threshold();
	let mut rng = Rng::new(0x1234);
	for _ in 0..500 {
		let p = rng.next_id();
		let h = rng.next_id();
		let s = placement::is_holder(&p, &h, &ts);
		let l = placement::is_holder(&p, &h, &tl);
		if s {
			assert!(l, "holder at n=10 but not at n=30");
		}
	}
	Ok(())
}
