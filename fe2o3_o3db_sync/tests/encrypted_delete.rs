//! A delete through [`O3dbCas`] or [`O3dbStorage`] on an encrypted store leaves the key absent.
//!
//! Both adapters wrote their tombstone with `store`, which encrypts it like any value.  The
//! stored record was then neither a tombstone's length nor its bytes, so the key index, which
//! recognises only the unencrypted tombstone `delete` writes, called the key present: `has`
//! answered true for a deleted chunk, a second `delete` answered `Ok(true)` and wrote another
//! tombstone, and a scan listed the deleted key (OZ-HAVE check, 2026-10-16).
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_crypto::enc::EncryptionScheme;
use oxedyne_fe2o3_hash::{
	csum::ChecksumScheme,
	hash::HashScheme,
};
use oxedyne_fe2o3_iop_db::api::ScanOpts;
use oxedyne_fe2o3_jdat::prelude::*;
use oxedyne_fe2o3_o3db_sync::{
	O3db,
	cas::{
		Cas,
		Chunk,
	},
	cas_o3db::O3dbCas,
	data::core::RestSchemesInput,
	test::setup::{
		self,
		Uid,
		UID_LEN,
	},
};

use std::{
	path::PathBuf,
	sync::Arc,
};


type Db = O3db<{ UID_LEN }, Uid, EncryptionScheme, HashScheme, HashScheme, ChecksumScheme>;
type TestCas = O3dbCas<{ UID_LEN }, Uid, EncryptionScheme, HashScheme, HashScheme, ChecksumScheme>;

fn root(name: &str) -> Outcome<PathBuf> {
	let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
	let _ = std::fs::remove_dir_all(&dir);
	res!(std::fs::create_dir_all(&dir));
	Ok(res!(dir.canonicalize()))
}

fn open(db_root: &PathBuf, wipe: bool) -> Outcome<Db> {
	let aes = res!(EncryptionScheme::new_aes_256_gcm_with_key(&[0x5au8; 32][..]));
	let schms_input = RestSchemesInput::new(
		Some(aes),
		None::<HashScheme>,
		None::<HashScheme>,
		Some(ChecksumScheme::new_crc32()),
	);
	let mut cfg = res!(setup::default_cfg());
	// Every zone in this test's own directory.
	cfg.zone_overrides = Default::default();
	setup::start_db(db_root.clone(), Some(cfg), schms_input, None, false, wipe)
}

fn check_gone(db: &Db, key: &Dat, prefix: &str, when: &str) -> Outcome<()> {
	if res!(db.api().contains_wait(key, None)) {
		return Err(err!("The deleted key {:?} reads present {}.", key, when; Test, Mismatch));
	}
	if res!(db.api().get_wait(key, None)).is_some() {
		return Err(err!("get finds the deleted key {:?} {}.", key, when; Test, Mismatch));
	}
	let listed = res!(db.api().scan(&ScanOpts::with_str_prefix(prefix.to_string()), None));
	if !listed.is_empty() {
		return Err(err!(
			"A scan lists {} keys under '{}' {}, after the only one was deleted.",
			listed.len(), prefix, when;
		Test, Mismatch, Size));
	}
	Ok(())
}

#[test]
fn cas_delete_on_an_encrypted_store_leaves_the_chunk_absent() -> Outcome<()> {
	log_set_level!("warn");
	let outcome = run_cas();
	log_finish_wait!();
	outcome
}

fn run_cas() -> Outcome<()> {
	let db_root = res!(root("test_db_encrypted_delete_cas"));
	let chunk = Chunk::new(b"a chunk deleted from an encrypted store".to_vec());
	let key = Dat::Str(fmt!("chunk:{}", chunk.id.to_hex()));
	{
		let db = Arc::new(res!(open(&db_root, true)));
		let cas = TestCas::new(db.clone(), Uid::default());
		res!(cas.put(&chunk));
		if !res!(cas.has(&chunk.id)) {
			return Err(err!("has() denies a chunk just put."; Test, Mismatch));
		}
		if !res!(cas.delete(&chunk.id)) {
			return Err(err!("The first delete says the chunk was not there."; Test, Mismatch));
		}
		if res!(cas.has(&chunk.id)) {
			return Err(err!("has() answers true for a deleted chunk."; Test, Mismatch));
		}
		if res!(cas.get(&chunk.id)).is_some() {
			return Err(err!("get() returns a deleted chunk."; Test, Mismatch));
		}
		if res!(cas.delete(&chunk.id)) {
			return Err(err!("A second delete answers Ok(true)."; Test, Mismatch));
		}
		let ids = res!(cas.ids());
		if !ids.is_empty() {
			return Err(err!("ids() lists {} chunks after the only one was deleted.", ids.len();
				Test, Mismatch, Size));
		}
		res!(check_gone(&db, &key, "chunk:", "in the session that deleted it"));
		res!(db.close());
	}
	{
		let db = Arc::new(res!(open(&db_root, false)));
		let cas = TestCas::new(db.clone(), Uid::default());
		if res!(cas.has(&chunk.id)) {
			return Err(err!("has() answers true for a deleted chunk after a reopen."; Test, Mismatch));
		}
		if res!(cas.delete(&chunk.id)) {
			return Err(err!("A delete after a reopen answers Ok(true)."; Test, Mismatch));
		}
		res!(check_gone(&db, &key, "chunk:", "after a reopen"));
		res!(db.close());
	}
	Ok(())
}

#[cfg(feature = "dist")]
#[test]
fn storage_delete_on_an_encrypted_store_leaves_the_record_absent() -> Outcome<()> {
	log_set_level!("warn");
	let outcome = run_storage();
	log_finish_wait!();
	outcome
}

#[cfg(feature = "dist")]
fn run_storage() -> Outcome<()> {
	use oxedyne_fe2o3_o3db_sync::dist::{
		o3db_storage::O3dbStorage,
		record::{
			Record,
			RecordId,
		},
		storage::Storage,
	};

	let db_root = res!(root("test_db_encrypted_delete_storage"));
	let id = RecordId::from_bytes([0xa5; 32]);
	let rec = Record::new(id, "ident", b"a record deleted from an encrypted store".to_vec());
	let key = Dat::Str(fmt!("ident:{}", "a5".repeat(32)));
	{
		let db = Arc::new(res!(open(&db_root, true)));
		let storage = O3dbStorage::new(db.clone(), Uid::default());
		res!(storage.put(&rec));
		if !res!(db.api().contains_wait(&key, None)) {
			return Err(err!("The record just put reads absent under {:?}.", key; Test, Mismatch));
		}
		if !res!(storage.delete("ident", &id)) {
			return Err(err!("The first delete says the record was not there."; Test, Mismatch));
		}
		if res!(storage.get("ident", &id)).is_some() {
			return Err(err!("get() returns a deleted record."; Test, Mismatch));
		}
		if res!(storage.delete("ident", &id)) {
			return Err(err!("A second delete answers Ok(true)."; Test, Mismatch));
		}
		let digests = res!(storage.digests("ident"));
		if !digests.is_empty() {
			return Err(err!("digests() lists {} records after the only one was deleted.",
				digests.len(); Test, Mismatch, Size));
		}
		res!(check_gone(&db, &key, "ident:", "in the session that deleted it"));
		res!(db.close());
	}
	{
		let db = Arc::new(res!(open(&db_root, false)));
		let storage = O3dbStorage::new(db.clone(), Uid::default());
		if res!(storage.delete("ident", &id)) {
			return Err(err!("A delete after a reopen answers Ok(true)."; Test, Mismatch));
		}
		res!(check_gone(&db, &key, "ident:", "after a reopen"));
		res!(db.close());
	}
	Ok(())
}
