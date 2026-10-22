#![cfg(feature = "dist")]
//! Integration tests for the content hash behind [`RecordDigest::content`].
//!
//! Anti-entropy compares these digests to decide what to repair, so two peers must
//! digest the same bytes alike and different bytes apart, whichever [`Storage`]
//! adapter each runs. The known answers below were computed outside Rust with
//! python3's `hashlib.sha3_256` over `len as u64 big-endian ‖ value`.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_crypto::enc::EncryptionScheme;
use oxedyne_fe2o3_hash::{
	csum::ChecksumScheme,
	hash::HashScheme,
};
use oxedyne_fe2o3_jdat::prelude::*;
use oxedyne_fe2o3_o3db_sync::{
	data::core::RestSchemesInput,
	dist::{
		o3db_storage::O3dbStorage,
		record::{
			Record,
			RecordDigest,
			RecordId,
			content_hash,
		},
		resolve::LastVersionWins,
		storage::{
			MemoryStorage,
			Storage,
		},
	},
	test::setup,
};

use std::{
	path::Path,
	sync::Arc,
	thread,
	time::Duration,
};


fn hex(bytes: &[u8]) -> String {
	let mut s = String::with_capacity(bytes.len() * 2);
	for b in bytes {
		s.push_str(&fmt!("{:02x}", b));
	}
	s
}

fn rid(b: u8) -> RecordId {
	RecordId::from_bytes([b; 32])
}

/// The digest `storage` gives the one record it holds in the table "t".
fn digest_of<S: Storage>(storage: &S) -> Outcome<RecordDigest> {
	let mut d = res!(storage.digests("t"));
	assert_eq!(d.len(), 1);
	Ok(d.remove(0))
}

/// A `MemoryStorage` holding one record with this value at id 1 of table "t".
fn mem_with(value: &[u8]) -> Outcome<MemoryStorage> {
	let storage = MemoryStorage::new();
	res!(storage.put(&Record::new(rid(1), "t", value.to_vec())));
	Ok(storage)
}

#[test]
fn content_hash_known_answer() {
	// SHA3-256 of eight zero bytes, the length of the empty value.
	assert_eq!(
		hex(&content_hash(b"")),
		"48dda5bbe9171a6656206ec56c595c5834b6cf38c5fe71bcb44fe43833aee9df",
	);
	// Seven zero bytes, 0x01 (the length), then 0xab.
	assert_eq!(
		hex(&content_hash(&[0xAB])),
		"5489666bad674de9ffe884577300c6e0edabad022917e889d5fc21c9364a8b0b",
	);
	// Seven zero bytes, 0x02, then 0xab 0x00.
	assert_eq!(
		hex(&content_hash(&[0xAB, 0x00])),
		"daf62553bc0a272a7491fcb69de7b547a95ce6ea4d6f5d6fdd687cfd7d46e1dd",
	);
}

#[test]
fn trailing_zero_values_digest_apart() -> Outcome<()> {
	// A value and the same value with a trailing zero byte.
	let a = res!(digest_of(&res!(mem_with(&[0xAB]))));
	let b = res!(digest_of(&res!(mem_with(&[0xAB, 0x00]))));
	assert_ne!(a.content, b.content);
	// One zero byte and an empty payload under a zero version, which is eight zero bytes.
	let c = res!(digest_of(&res!(mem_with(&[0]))));
	let d = res!(digest_of(&res!(mem_with(&LastVersionWins::value(0, b"")))));
	assert_ne!(c.content, d.content);
	Ok(())
}

#[test]
fn memory_storage_digests_with_the_protocol_hash() -> Outcome<()> {
	// What the storage reports is the protocol hash of the stored value, not a copy of it.
	let values: [&[u8]; 4] = [b"", &[0xAB], &[0xAB, 0x00], b"a longer value than one chunk of eight bytes"];
	for value in values {
		let d = res!(digest_of(&res!(mem_with(value))));
		assert_eq!(d.id, rid(1));
		assert_eq!(d.content, content_hash(value));
	}
	Ok(())
}

#[test]
fn o3db_storage_digests_with_the_protocol_hash() -> Outcome<()> {
	let db_root = res!(Path::new("./test_db_dist_content")
		.canonicalize()
		.or_else(|_| -> std::io::Result<_> {
			ok!(std::fs::create_dir_all("./test_db_dist_content"));
			Path::new("./test_db_dist_content").canonicalize()
		}));
	let enckey = [0x44u8; 32];
	let aes_gcm = res!(EncryptionScheme::new_aes_256_gcm_with_key(&enckey[..]));
	let crc32 = ChecksumScheme::new_crc32();
	let user = setup::Uid::default();
	let schms_input = RestSchemesInput::new(
		Some(aes_gcm.clone()),
		None::<HashScheme>,
		None::<HashScheme>,
		Some(crc32.clone()),
	);
	let mut cfg = res!(setup::default_cfg());
	cfg.num_zones				= 2;
	cfg.num_cbots_per_zone		= 2;
	cfg.num_igbots_per_zone		= 2;
	cfg.data_file_max_bytes		= 200_000;
	cfg.zone_overrides = mapdat!{
		1u16 => mapdat!{ "dir" => "", "max_size" => 1_000_000u64 },
	}.get_map().unwrap();
	let db = res!(setup::start_db(db_root.clone(), Some(cfg), schms_input, None, true, true));
	thread::sleep(Duration::from_secs(1));
	let db = Arc::new(db);
	let o3 = O3dbStorage::new(Arc::clone(&db), user);
	let mem = MemoryStorage::new();

	// Two values that differ only by a trailing zero, and one the old hash read as the empty value.
	let values: [(u8, &[u8]); 3] = [(1, &[0xAB]), (2, &[0xAB, 0x00]), (3, &[0, 0, 0])];
	for (n, value) in values {
		let record = Record::new(rid(n), "t", value.to_vec());
		res!(o3.put(&record));
		res!(mem.put(&record));
	}
	thread::sleep(Duration::from_millis(500));

	let held = res!(o3.digests("t"));
	assert_eq!(held.len(), 3);
	for d in &held {
		let value = values.iter().find(|(n, _)| rid(*n) == d.id).map(|(_, v)| *v);
		let value = res!(value.ok_or_else(|| err!("Unexpected id {} in the O3db digests.", hex(d.id.as_bytes()); Test, Mismatch)));
		assert_eq!(d.content, content_hash(value));
	}
	// A mixed cluster of one adapter of each kind must digest the same records alike.
	assert_eq!(held, res!(mem.digests("t")));
	assert_ne!(held[0].content, held[1].content);
	Ok(())
}
