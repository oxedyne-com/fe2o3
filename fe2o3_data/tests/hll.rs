//! Integration tests for the HyperLogLog primitive.

use oxedyne_fe2o3_core::prelude::*;

use oxedyne_fe2o3_data::hll::{
	HyperLogLog,
	P_DEFAULT,
	P_MAX,
	P_MIN,
};


/// A deterministic, uniformly distributed 64-bit hash suitable for synthetic
/// cardinality tests. Uses xoshiro256++ mixing; not cryptographic.
fn hash_u64(mut x: u64) -> u64 {
	x = x.wrapping_add(0x9e37_79b9_7f4a_7c15);
	let mut z = x;
	z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
	z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
	z ^ (z >> 31)
}


#[test]
fn rejects_out_of_range_precision() -> Outcome<()> {
	assert!(HyperLogLog::new(P_MIN - 1).is_err());
	assert!(HyperLogLog::new(P_MAX + 1).is_err());
	assert!(HyperLogLog::new(0).is_err());
	assert!(HyperLogLog::new(100).is_err());
	Ok(())
}

#[test]
fn new_allocates_correct_register_count() -> Outcome<()> {
	for p in P_MIN..=P_MAX {
		let s = res!(HyperLogLog::new(p));
		let m = 1usize << p;
		assert_eq!(s.m(), m, "precision {}: expected {} registers", p, m);
		assert_eq!(s.precision(), p);
		assert_eq!(s.as_bytes().len(), m);
		assert!(s.as_bytes().iter().all(|&b| b == 0));
	}
	Ok(())
}

#[test]
fn default_precision_is_sixteen_kibibytes() -> Outcome<()> {
	let s = res!(HyperLogLog::new(P_DEFAULT));
	assert_eq!(s.as_bytes().len(), 16 * 1024);
	Ok(())
}

#[test]
fn empty_sketch_estimate_is_zero() -> Outcome<()> {
	let s = res!(HyperLogLog::new(P_DEFAULT));
	// Linear counting kicks in: m * ln(m/m) = m * 0 = 0.
	assert_eq!(s.estimate_rounded(), 0);
	Ok(())
}

#[test]
fn single_element_estimate_near_one() -> Outcome<()> {
	let mut s = res!(HyperLogLog::new(P_DEFAULT));
	s.add_hash(hash_u64(1));
	// Linear counting: m * ln(m / (m-1)) for a single filled register.
	// At m = 16384 this gives ≈ 1.0 (the ln(m/(m-1)) factor ≈ 1/(m-1)).
	let est = s.estimate();
	assert!(est > 0.5 && est < 2.0, "estimate = {}", est);
	Ok(())
}

#[test]
fn estimate_accurate_at_medium_cardinality() -> Outcome<()> {
	let mut s = res!(HyperLogLog::new(P_DEFAULT));
	// 10 000 distinct elements; at p=14 the theoretical standard error is
	// ~0.8%. Allow 5% tolerance for safety on a single run.
	let true_n = 10_000u64;
	for i in 0..true_n {
		s.add_hash(hash_u64(i));
	}
	let est = s.estimate();
	let err = (est - true_n as f64).abs() / true_n as f64;
	assert!(err < 0.05,
		"estimate {} vs true {} differs by {:.2}%",
		est, true_n, err * 100.0);
	Ok(())
}

#[test]
fn estimate_accurate_at_large_cardinality() -> Outcome<()> {
	let mut s = res!(HyperLogLog::new(P_DEFAULT));
	// 100 000 distinct elements; raw HLL regime.
	let true_n = 100_000u64;
	for i in 0..true_n {
		s.add_hash(hash_u64(i));
	}
	let est = s.estimate();
	let err = (est - true_n as f64).abs() / true_n as f64;
	assert!(err < 0.05,
		"estimate {} vs true {} differs by {:.2}%",
		est, true_n, err * 100.0);
	Ok(())
}

#[test]
fn duplicate_inserts_do_not_change_estimate() -> Outcome<()> {
	let mut s = res!(HyperLogLog::new(P_DEFAULT));
	for i in 0..1_000u64 {
		s.add_hash(hash_u64(i));
	}
	let before = s.estimate();
	// Re-insert every element; register values are already at their max so
	// the sketch should not move.
	for _ in 0..5 {
		for i in 0..1_000u64 {
			s.add_hash(hash_u64(i));
		}
	}
	let after = s.estimate();
	assert!((before - after).abs() < 1e-9,
		"duplicate inserts changed estimate: before {}, after {}",
		before, after);
	Ok(())
}

#[test]
fn merge_union_approximates_cardinality_of_union() -> Outcome<()> {
	let mut a = res!(HyperLogLog::new(P_DEFAULT));
	let mut b = res!(HyperLogLog::new(P_DEFAULT));
	// a = {0..5000}, b = {2500..7500}, union = 7500 distinct.
	for i in 0..5_000u64 {
		a.add_hash(hash_u64(i));
	}
	for i in 2_500u64..7_500 {
		b.add_hash(hash_u64(i));
	}
	res!(a.merge(&b));
	let est = a.estimate();
	let true_union = 7_500.0f64;
	let err = (est - true_union).abs() / true_union;
	assert!(err < 0.05,
		"merged estimate {} vs true union {} differs by {:.2}%",
		est, true_union, err * 100.0);
	Ok(())
}

#[test]
fn merge_is_idempotent() -> Outcome<()> {
	let mut a = res!(HyperLogLog::new(P_DEFAULT));
	let b = {
		let mut b = res!(HyperLogLog::new(P_DEFAULT));
		for i in 0..1_000u64 {
			b.add_hash(hash_u64(i));
		}
		b
	};
	res!(a.merge(&b));
	let once: Vec<u8> = a.as_bytes().to_vec();
	res!(a.merge(&b));
	assert_eq!(once, a.as_bytes(),
		"merging the same sketch twice changed registers");
	Ok(())
}

#[test]
fn merge_rejects_mismatched_precision() -> Outcome<()> {
	let mut a = res!(HyperLogLog::new(10));
	let b = res!(HyperLogLog::new(12));
	assert!(a.merge(&b).is_err());
	Ok(())
}

#[test]
fn from_bytes_roundtrips() -> Outcome<()> {
	let mut src = res!(HyperLogLog::new(P_DEFAULT));
	for i in 0..500u64 {
		src.add_hash(hash_u64(i));
	}
	let bytes = src.as_bytes().to_vec();
	let copy = res!(HyperLogLog::from_bytes(P_DEFAULT, &bytes));
	assert_eq!(copy.as_bytes(), src.as_bytes());
	let a = src.estimate();
	let b = copy.estimate();
	assert!((a - b).abs() < 1e-9);
	Ok(())
}

#[test]
fn from_bytes_validates_length() -> Outcome<()> {
	let bytes = vec![0u8; 10];
	assert!(HyperLogLog::from_bytes(P_DEFAULT, &bytes).is_err());
	Ok(())
}

#[test]
fn clear_zeroes_registers() -> Outcome<()> {
	let mut s = res!(HyperLogLog::new(P_DEFAULT));
	for i in 0..100u64 {
		s.add_hash(hash_u64(i));
	}
	assert!(s.estimate() > 50.0);
	s.clear();
	assert_eq!(s.estimate_rounded(), 0);
	assert!(s.as_bytes().iter().all(|&b| b == 0));
	Ok(())
}

#[test]
fn register_values_within_theoretical_bound() -> Outcome<()> {
	let mut s = res!(HyperLogLog::new(P_DEFAULT));
	// The per-register cap at p is (64 - p) + 1 = 51 here.
	for i in 0..1_000_000u64 {
		s.add_hash(hash_u64(i));
	}
	let max_rho = 64u8 - P_DEFAULT + 1;
	assert!(s.as_bytes().iter().all(|&r| r <= max_rho),
		"register exceeded theoretical max {}", max_rho);
	Ok(())
}
