//! Integration tests for the Kademlia routing-table primitive.
#![cfg(feature = "dist")]

use oxedyne_fe2o3_core::prelude::*;

use oxedyne_fe2o3_o3db_sync::kademlia::{
	contact::Contact,
	id::{
		Distance,
		ID_BITS,
		ID_LEN,
		NodeId,
	},
	kmap::{
		InsertOutcome,
		KMap,
	},
	table::RoutingTable,
};

use std::net::SocketAddr;


/// Helper -- builds a NodeId with a single bit set at `bit` (counted from
/// the LSB).
fn id_with_bit(bit: usize) -> NodeId {
	let mut bytes = [0u8; ID_LEN];
	let byte_from_msb = ID_LEN - 1 - bit / 8;
	let bit_in_byte = bit % 8;
	bytes[byte_from_msb] = 1u8 << bit_in_byte;
	NodeId::from_bytes(bytes)
}

/// Helper -- builds a NodeId from a u64 suffix in the low bytes.
fn id_from_u64(suffix: u64) -> NodeId {
	let mut bytes = [0u8; ID_LEN];
	bytes[ID_LEN - 8 ..].copy_from_slice(&suffix.to_be_bytes());
	NodeId::from_bytes(bytes)
}

/// Helper -- socket address for contact construction.
fn loopback(port: u16) -> SocketAddr {
	let s = format!("127.0.0.1:{}", port);
	s.parse().expect("test loopback addr parses")
}


#[test]
fn xor_distance_is_self_inverse() -> Outcome<()> {
	let a = id_from_u64(0xdead_beef_cafe_babe);
	let b = id_from_u64(0x0123_4567_89ab_cdef);
	let d1 = a.distance(&b);
	let d2 = b.distance(&a);
	assert_eq!(d1, d2);
	Ok(())
}

#[test]
fn xor_distance_to_self_is_zero() -> Outcome<()> {
	let a = id_from_u64(42);
	let d = a.distance(&a);
	assert!(d.is_zero());
	assert_eq!(d.bucket_index(), None);
	assert_eq!(a.bucket_index(&a), None);
	Ok(())
}

#[test]
fn bucket_index_matches_bit_position() -> Outcome<()> {
	let me = NodeId::from_bytes([0u8; ID_LEN]);
	for bit in 0..ID_BITS {
		let peer = id_with_bit(bit);
		let idx = res!(me.bucket_index(&peer).ok_or_else(|| err!(
			"bit-exact distance unexpectedly zero for bit {}", bit;
		Bug, Unexpected)));
		assert_eq!(idx, bit,
			"bit {} expected bucket {}, got {}", bit, bit, idx);
	}
	Ok(())
}

#[test]
fn distance_ord_is_byte_wise_unsigned() -> Outcome<()> {
	let mut low = [0u8; ID_LEN];
	low[ID_LEN - 1] = 1;
	let mut high = [0u8; ID_LEN];
	high[0] = 1;
	let d_low = Distance(low);
	let d_high = Distance(high);
	assert!(d_low < d_high);
	Ok(())
}

#[test]
fn kmap_inserts_then_refreshes() -> Outcome<()> {
	let mut map = res!(KMap::new(3));
	let id_a = id_from_u64(1);
	let id_b = id_from_u64(2);

	let c_a = Contact::new(id_a, vec![loopback(1)]);
	let c_b = Contact::new(id_b, vec![loopback(2)]);

	assert!(matches!(map.insert(c_a.clone()), InsertOutcome::Inserted));
	assert!(matches!(map.insert(c_b.clone()), InsertOutcome::Inserted));
	assert!(matches!(map.insert(c_a.clone()), InsertOutcome::Refreshed));
	// After refresh, `a` is MRU (front), `b` is LRU (back).
	let ordered: Vec<_> = map.iter().map(|c| c.node_id).collect();
	assert_eq!(ordered, vec![id_a, id_b]);
	Ok(())
}

#[test]
fn kmap_full_surfaces_lru_candidate() -> Outcome<()> {
	let mut map = res!(KMap::new(2));
	let id_a = id_from_u64(1);
	let id_b = id_from_u64(2);
	let id_c = id_from_u64(3);

	assert!(matches!(
		map.insert(Contact::new(id_a, vec![loopback(1)])),
		InsertOutcome::Inserted));
	assert!(matches!(
		map.insert(Contact::new(id_b, vec![loopback(2)])),
		InsertOutcome::Inserted));

	let outcome = map.insert(Contact::new(id_c, vec![loopback(3)]));
	let (candidate_id, pending_id) = match outcome {
		InsertOutcome::Full { candidate, pending } => (candidate.node_id, pending.node_id),
		other => panic!("expected Full outcome, got {:?}", other),
	};
	assert_eq!(candidate_id, id_a, "LRU after two inserts is `a`");
	assert_eq!(pending_id, id_c);
	Ok(())
}

#[test]
fn kmap_keep_lru_retains_on_live_probe() -> Outcome<()> {
	let mut map = res!(KMap::new(2));
	let id_a = id_from_u64(1);
	let id_b = id_from_u64(2);
	let _ = map.insert(Contact::new(id_a, vec![loopback(1)]));
	let _ = map.insert(Contact::new(id_b, vec![loopback(2)]));
	map.keep_lru(100);
	// `a` was LRU; touching moves it to MRU.
	let ordered: Vec<_> = map.iter().map(|c| c.node_id).collect();
	assert_eq!(ordered, vec![id_a, id_b]);
	let refreshed = res!(map.get(&id_a).ok_or_else(|| err!(
		"`a` missing after keep_lru"; Bug, Unexpected)));
	assert_eq!(refreshed.last_seen, 100);
	Ok(())
}

#[test]
fn kmap_evict_and_insert_drops_dead_lru() -> Outcome<()> {
	let mut map = res!(KMap::new(2));
	let id_a = id_from_u64(1);
	let id_b = id_from_u64(2);
	let id_c = id_from_u64(3);
	let _ = map.insert(Contact::new(id_a, vec![loopback(1)]));
	let _ = map.insert(Contact::new(id_b, vec![loopback(2)]));

	let evicted = map.evict_and_insert(Contact::new(id_c, vec![loopback(3)]));
	let evicted = res!(evicted.ok_or_else(|| err!(
		"eviction did not return a prior contact"; Bug, Unexpected)));
	assert_eq!(evicted.node_id, id_a);
	let ordered: Vec<_> = map.iter().map(|c| c.node_id).collect();
	assert_eq!(ordered, vec![id_c, id_b]);
	Ok(())
}

#[test]
fn routing_table_rejects_self_insertion() -> Outcome<()> {
	let me = id_from_u64(42);
	let mut table = res!(RoutingTable::new(me, 20));
	let outcome = res!(table.insert(Contact::new(me, vec![loopback(1)])));
	assert!(outcome.is_none());
	assert!(table.is_empty());
	Ok(())
}

#[test]
fn routing_table_routes_by_bucket() -> Outcome<()> {
	let me = NodeId::from_bytes([0u8; ID_LEN]);
	let mut table = res!(RoutingTable::new(me, 20));
	// Insert three peers into three distinct buckets.
	for bit in [3usize, 100, 255] {
		let peer_id = id_with_bit(bit);
		let outcome = res!(table.insert(
			Contact::new(peer_id, vec![loopback(bit as u16 + 1)])));
		assert!(outcome.is_none(),
			"bit {} expected trivial insert, got {:?}", bit, outcome);
	}
	assert_eq!(table.len(), 3);
	Ok(())
}

#[test]
fn k_closest_returns_ascending_distance() -> Outcome<()> {
	let me = NodeId::from_bytes([0u8; ID_LEN]);
	let mut table = res!(RoutingTable::new(me, 20));
	// Sprinkle a handful of peers across buckets.
	for bit in [1usize, 2, 4, 8, 16, 32, 64, 128] {
		let peer = id_with_bit(bit);
		let _ = res!(table.insert(Contact::new(peer, vec![loopback(bit as u16 + 1)])));
	}
	// Target is a single bit at position 3 -- closest match is bit-2 (distance
	// differs in bits 2 and 3, XOR = 0x0C), then bit-4 (XOR = 0x18), etc.
	let target = id_with_bit(3);
	let closest = table.k_closest(&target, 3);
	assert_eq!(closest.len(), 3);
	// Verify ascending distance.
	let mut prev = closest[0].node_id.distance(&target);
	for c in &closest[1..] {
		let d = c.node_id.distance(&target);
		assert!(prev <= d, "k_closest must return ascending distance");
		prev = d;
	}
	Ok(())
}

#[test]
fn k_closest_caps_at_want() -> Outcome<()> {
	let me = NodeId::from_bytes([0u8; ID_LEN]);
	let mut table = res!(RoutingTable::new(me, 20));
	for suffix in 1u64..=10 {
		let peer = id_from_u64(suffix);
		let _ = res!(table.insert(
			Contact::new(peer, vec![loopback(suffix as u16)])));
	}
	let target = id_from_u64(5);
	assert_eq!(table.k_closest(&target, 0).len(), 0);
	assert_eq!(table.k_closest(&target, 3).len(), 3);
	assert_eq!(table.k_closest(&target, 100).len(), 10);
	Ok(())
}

#[test]
fn two_peer_mutual_discovery() -> Outcome<()> {
	// A tiny in-process two-peer scenario: each learns about the other and
	// a k_closest lookup for the remote's id returns the remote contact.
	let id_a = id_from_u64(0xaaaa_aaaa_aaaa_aaaa);
	let id_b = id_from_u64(0xbbbb_bbbb_bbbb_bbbb);

	let mut table_a = res!(RoutingTable::new(id_a, 20));
	let mut table_b = res!(RoutingTable::new(id_b, 20));

	let _ = res!(table_a.insert(Contact::new(id_b, vec![loopback(60001)])));
	let _ = res!(table_b.insert(Contact::new(id_a, vec![loopback(60000)])));

	let from_a = table_a.k_closest(&id_b, 1);
	let from_b = table_b.k_closest(&id_a, 1);
	assert_eq!(from_a.len(), 1);
	assert_eq!(from_a[0].node_id, id_b);
	assert_eq!(from_b.len(), 1);
	assert_eq!(from_b[0].node_id, id_a);
	Ok(())
}
