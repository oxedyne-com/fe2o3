//! Integration tests for the IBLT primitive.

use oxedyne_fe2o3_core::prelude::*;

use oxedyne_fe2o3_iblt::iblt::{
	DecodeOutcome,
	Iblt,
	IbltConfig,
};

use std::collections::BTreeSet;


fn key_only_cfg(num_cells: usize, num_hashes: usize) -> IbltConfig {
	IbltConfig {
		num_cells,
		num_hashes,
		key_len:	8,
		value_len:	0,
		seed:		0x0123_4567_89ab_cdef,
	}
}

fn key_bytes(x: u64) -> Vec<u8> {
	x.to_le_bytes().to_vec()
}


#[test]
fn rejects_zero_cells() -> Outcome<()> {
	let cfg = IbltConfig {
		num_cells: 0, num_hashes: 3, key_len: 8, value_len: 0, seed: 0,
	};
	assert!(Iblt::new(cfg).is_err());
	Ok(())
}

#[test]
fn rejects_zero_hashes() -> Outcome<()> {
	let cfg = IbltConfig {
		num_cells: 10, num_hashes: 0, key_len: 8, value_len: 0, seed: 0,
	};
	assert!(Iblt::new(cfg).is_err());
	Ok(())
}

#[test]
fn rejects_zero_key_len() -> Outcome<()> {
	let cfg = IbltConfig {
		num_cells: 10, num_hashes: 3, key_len: 0, value_len: 0, seed: 0,
	};
	assert!(Iblt::new(cfg).is_err());
	Ok(())
}

#[test]
fn rejects_num_hashes_above_num_cells() -> Outcome<()> {
	let cfg = IbltConfig {
		num_cells: 3, num_hashes: 4, key_len: 8, value_len: 0, seed: 0,
	};
	assert!(Iblt::new(cfg).is_err());
	Ok(())
}

#[test]
fn insert_length_mismatch_errors() -> Outcome<()> {
	let cfg = key_only_cfg(32, 3);
	let mut iblt = res!(Iblt::new(cfg));
	assert!(iblt.insert(&[0u8; 4], &[]).is_err());
	assert!(iblt.insert(&[0u8; 8], &[0u8; 1]).is_err());
	Ok(())
}

#[test]
fn empty_iblt_decodes_empty() -> Outcome<()> {
	let cfg = key_only_cfg(32, 3);
	let mut iblt = res!(Iblt::new(cfg));
	match res!(iblt.decode()) {
		DecodeOutcome::Complete { inserted, deleted } => {
			assert!(inserted.is_empty());
			assert!(deleted.is_empty());
		},
		other => panic!("expected Complete, got {:?}", other),
	}
	Ok(())
}

#[test]
fn insert_then_delete_restores_empty() -> Outcome<()> {
	let cfg = key_only_cfg(32, 3);
	let mut iblt = res!(Iblt::new(cfg));
	let k = key_bytes(42);
	res!(iblt.insert(&k, &[]));
	res!(iblt.delete(&k, &[]));
	assert!(iblt.is_empty());
	Ok(())
}

#[test]
fn self_subtract_zeroes_out() -> Outcome<()> {
	let cfg = key_only_cfg(64, 3);
	let mut a = res!(Iblt::new(cfg));
	for i in 1u64..=10 {
		res!(a.insert(&key_bytes(i), &[]));
	}
	let b = a.clone();
	res!(a.subtract(&b));
	assert!(a.is_empty());
	Ok(())
}

#[test]
fn symmetric_difference_recovers_cleanly() -> Outcome<()> {
	// Sizing: 19-key symmetric difference, k=3, so ~1.5 × 19 ≈ 29 cells
	// suffice. 80 gives ample headroom to avoid flaky tests.
	let cfg = key_only_cfg(80, 3);
	let mut a = res!(Iblt::new(cfg));
	let mut b = res!(Iblt::new(cfg));

	let a_set: BTreeSet<u64>	= (1u64..20).collect();
	let b_set: BTreeSet<u64>	= (10u64..30).collect();
	for &x in &a_set {
		res!(a.insert(&key_bytes(x), &[]));
	}
	for &x in &b_set {
		res!(b.insert(&key_bytes(x), &[]));
	}

	res!(a.subtract(&b));
	let outcome = res!(a.decode());
	let (inserted, deleted) = match outcome {
		DecodeOutcome::Complete { inserted, deleted } => (inserted, deleted),
		other => panic!("expected Complete, got {:?}", other),
	};

	let recovered_inserted: BTreeSet<u64> = inserted.iter()
		.map(|(k, _)| {
			let mut buf = [0u8; 8];
			buf.copy_from_slice(k);
			u64::from_le_bytes(buf)
		})
		.collect();
	let recovered_deleted: BTreeSet<u64> = deleted.iter()
		.map(|(k, _)| {
			let mut buf = [0u8; 8];
			buf.copy_from_slice(k);
			u64::from_le_bytes(buf)
		})
		.collect();

	let expected_a_only: BTreeSet<u64> = a_set.difference(&b_set).copied().collect();
	let expected_b_only: BTreeSet<u64> = b_set.difference(&a_set).copied().collect();
	assert_eq!(recovered_inserted, expected_a_only);
	assert_eq!(recovered_deleted, expected_b_only);
	Ok(())
}

#[test]
fn overload_returns_incomplete() -> Outcome<()> {
	// Difference of 50 keys against 20 cells is far above the peeling
	// threshold, so decoding must report Incomplete.
	let cfg = key_only_cfg(20, 3);
	let mut a = res!(Iblt::new(cfg));
	for i in 0u64..50 {
		res!(a.insert(&key_bytes(i), &[]));
	}
	let outcome = res!(a.decode());
	match outcome {
		DecodeOutcome::Incomplete { remaining_cells, .. } => {
			assert!(remaining_cells > 0);
		},
		DecodeOutcome::Complete { .. } => {
			panic!("expected Incomplete outcome on overloaded IBLT");
		},
	}
	Ok(())
}

#[test]
fn value_carrying_iblt_recovers_values() -> Outcome<()> {
	let cfg = IbltConfig {
		num_cells:	60,
		num_hashes:	3,
		key_len:	4,
		value_len:	4,
		seed:		0xdeadbeef,
	};
	let mut a = res!(Iblt::new(cfg));
	let pairs: Vec<(u32, u32)> = (1u32..=12).map(|i| (i, i * 100)).collect();
	for (k, v) in &pairs {
		res!(a.insert(&k.to_le_bytes(), &v.to_le_bytes()));
	}
	let b = res!(Iblt::new(cfg));
	res!(a.subtract(&b));
	let (inserted, deleted) = match res!(a.decode()) {
		DecodeOutcome::Complete { inserted, deleted } => (inserted, deleted),
		other => panic!("expected Complete, got {:?}", other),
	};
	assert!(deleted.is_empty());
	assert_eq!(inserted.len(), pairs.len());
	let recovered: BTreeSet<(u32, u32)> = inserted.iter()
		.map(|(k, v)| {
			let mut kb = [0u8; 4];
			kb.copy_from_slice(k);
			let mut vb = [0u8; 4];
			vb.copy_from_slice(v);
			(u32::from_le_bytes(kb), u32::from_le_bytes(vb))
		})
		.collect();
	let expected: BTreeSet<(u32, u32)> = pairs.iter().copied().collect();
	assert_eq!(recovered, expected);
	Ok(())
}

#[test]
fn serialisation_roundtrips() -> Outcome<()> {
	let cfg = key_only_cfg(32, 3);
	let mut a = res!(Iblt::new(cfg));
	for i in 1u64..10 {
		res!(a.insert(&key_bytes(i), &[]));
	}
	let bytes = a.to_bytes();
	let mut parsed = res!(Iblt::from_bytes(&bytes));
	assert_eq!(parsed.config(), cfg);
	// Decoding the roundtripped IBLT should recover the same set.
	let outcome = res!(parsed.decode());
	match outcome {
		DecodeOutcome::Complete { inserted, deleted } => {
			assert!(deleted.is_empty());
			assert_eq!(inserted.len(), 9);
			let recovered: BTreeSet<u64> = inserted.iter()
				.map(|(k, _)| {
					let mut buf = [0u8; 8];
					buf.copy_from_slice(k);
					u64::from_le_bytes(buf)
				})
				.collect();
			let expected: BTreeSet<u64> = (1u64..10).collect();
			assert_eq!(recovered, expected);
		},
		other => panic!("expected Complete, got {:?}", other),
	}
	Ok(())
}

#[test]
fn subtract_config_mismatch_errors() -> Outcome<()> {
	let a = res!(Iblt::new(key_only_cfg(32, 3)));
	let b = res!(Iblt::new(key_only_cfg(32, 4)));
	let mut a = a;
	assert!(a.subtract(&b).is_err());
	Ok(())
}

#[test]
fn decode_preserves_completeness_under_reorder() -> Outcome<()> {
	// Inserting the same set in two different orders must produce bitwise
	// identical IBLTs -- the structure is order-agnostic.
	let cfg = key_only_cfg(48, 3);
	let mut a = res!(Iblt::new(cfg));
	let mut b = res!(Iblt::new(cfg));
	for i in 1u64..=15 {
		res!(a.insert(&key_bytes(i), &[]));
	}
	for i in (1u64..=15).rev() {
		res!(b.insert(&key_bytes(i), &[]));
	}
	assert_eq!(a.to_bytes(), b.to_bytes());
	Ok(())
}
