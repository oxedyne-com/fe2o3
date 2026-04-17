//! Integration test for the O3db-backed [`Storage`] adapter.
//!
//! Boots a fresh [`O3db`] instance in a per-test directory, wraps it in
//! [`O3dbStorage`], and exercises the full [`Storage`] contract
//! (`put`, `get`, `delete`, `digests`) plus a few edge cases. The test
//! runs only with `--features dist`.

#![cfg(feature = "dist")]

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
			RecordId,
		},
		storage::Storage,
	},
	test::setup,
};

use std::{
	path::Path,
	sync::Arc,
	thread,
	time::Duration,
};


#[test]
fn o3db_storage_round_trip() -> Outcome<()> {

	let db_root = res!(Path::new("./test_db_dist_o3db_storage")
		.canonicalize()
		.or_else(|_| -> std::io::Result<_> {
			std::fs::create_dir_all("./test_db_dist_o3db_storage")?;
			Path::new("./test_db_dist_o3db_storage").canonicalize()
		}));

	let enckey = [0x33u8; 32];
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
	// Keep zone directories inside the per-test db root so this test
	// cannot inherit or contaminate state from sibling tests.
	cfg.zone_overrides = mapdat!{
		1u16 => mapdat!{ "dir" => "", "max_size" => 1_000_000u64 },
	}.get_map().unwrap();

	let db = res!(setup::start_db(
		db_root.clone(),
		Some(cfg),
		schms_input,
		None,
		true,	// gc on
		true,	// wipe pre-existing
	));

	thread::sleep(Duration::from_secs(1));

	let db = Arc::new(db);
	let storage = O3dbStorage::new(Arc::clone(&db), user);

	// 1. Put three records across two tables.
	let id_a = RecordId::from_bytes([0x01; 32]);
	let rec_a = Record::new(id_a, "identity", b"alpha".to_vec());
	res!(storage.put(&rec_a));

	let id_b = RecordId::from_bytes([0x02; 32]);
	let rec_b = Record::new(id_b, "identity", b"beta".to_vec());
	res!(storage.put(&rec_b));

	let id_c = RecordId::from_bytes([0x03; 32]);
	let rec_c = Record::new(id_c, "escrow", b"gamma-with-a-longer-payload".to_vec());
	res!(storage.put(&rec_c));

	thread::sleep(Duration::from_millis(500));

	// 2. Fetch each record back and verify round-trip equality.
	let fetched_a = res!(storage.get("identity", &id_a));
	assert_eq!(fetched_a.as_ref(), Some(&rec_a),
		"identity record A did not round-trip");

	let fetched_b = res!(storage.get("identity", &id_b));
	assert_eq!(fetched_b.as_ref(), Some(&rec_b),
		"identity record B did not round-trip");

	let fetched_c = res!(storage.get("escrow", &id_c));
	assert_eq!(fetched_c.as_ref(), Some(&rec_c),
		"escrow record C did not round-trip");

	// 3. A missing id returns None rather than erroring.
	let missing_id = RecordId::from_bytes([0x99; 32]);
	let fetched_missing = res!(storage.get("identity", &missing_id));
	assert!(fetched_missing.is_none(),
		"missing record unexpectedly returned something");

	// 4. digests("identity") enumerates exactly A and B.
	let digests_identity = res!(storage.digests("identity"));
	assert_eq!(digests_identity.len(), 2,
		"expected 2 identity digests, got {}", digests_identity.len());
	let mut identity_ids: Vec<_> = digests_identity.iter()
		.map(|d| d.id)
		.collect();
	identity_ids.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
	assert_eq!(identity_ids, vec![id_a, id_b]);

	// 5. digests("escrow") enumerates only C.
	let digests_escrow = res!(storage.digests("escrow"));
	assert_eq!(digests_escrow.len(), 1);
	assert_eq!(digests_escrow[0].id, id_c);

	// 6. Content hashes differ between records with different payloads.
	let digest_a = digests_identity.iter().find(|d| d.id == id_a)
		.expect("digest for A");
	let digest_b = digests_identity.iter().find(|d| d.id == id_b)
		.expect("digest for B");
	assert_ne!(digest_a.content, digest_b.content,
		"different payloads should hash differently");

	// 7. delete reports `true` for a present record and `false` for a
	//    subsequent attempt on the same key.
	let was_present = res!(storage.delete("identity", &id_a));
	assert!(was_present, "first delete of A should return true");

	// Give the delete tombstone time to land in cache / on disk.
	thread::sleep(Duration::from_secs(2));

	let after_delete = res!(storage.get("identity", &id_a));
	assert!(after_delete.is_none(),
		"A should be gone after delete, but got {:?}", after_delete);

	let was_present_again = res!(storage.delete("identity", &id_a));
	assert!(!was_present_again,
		"second delete of A should return false");

	// 8. digests("identity") is now just B.
	thread::sleep(Duration::from_millis(500));
	let digests_identity_after = res!(storage.digests("identity"));
	assert_eq!(digests_identity_after.len(), 1);
	assert_eq!(digests_identity_after[0].id, id_b);

	// 9. Graceful shutdown. Drop the adapter so the Arc refcount drops
	//    back to one, then reclaim the db and shut it down. Skipping
	//    shutdown leaves bot threads spinning and pollutes sibling
	//    tests' log streams.
	drop(storage);
	let db = match Arc::try_unwrap(db) {
		Ok(db) => db,
		Err(_) => return Err(err!(
			"Could not reclaim db from Arc: extra references outlived storage.";
			Bug)),
	};
	res!(db.shutdown());

	Ok(())
}
