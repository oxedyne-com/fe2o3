//! [`O3dbCas`] keeps a chunk of any length whole.
//!
//! A chunk was written as `Dat::BU8`, whose length field is one byte, so a chunk of more than
//! 255 bytes was stored with its length cut modulo 256 and came back as a shorter chunk. The
//! address check on `put` passed, since it runs on the bytes before they are stored, so nothing
//! noticed.
//! These tests put chunks either side of each width's boundary and read them back, in the session
//! that wrote them and after a close and a reopen of the same root.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_hash::{
	csum::ChecksumScheme,
	hash::HashScheme,
};
use oxedyne_fe2o3_o3db_sync::{
	O3db,
	cas::{
		Cas,
		Chunk,
		ContentId,
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


type Db = O3db<{ UID_LEN }, Uid, (), HashScheme, HashScheme, ChecksumScheme>;
type TestCas = O3dbCas<{ UID_LEN }, Uid, (), HashScheme, HashScheme, ChecksumScheme>;

// Lengths either side of the one-byte and two-byte length fields, and one that needs four bytes.
const LENS: [usize; 11] = [1, 100, 254, 255, 256, 300, 1000, 65_534, 65_535, 65_536, 70_000];

fn open(db_root: &PathBuf, wipe: bool) -> Outcome<Db> {
	let schms_input = RestSchemesInput::new(
		None::<()>,
		None::<HashScheme>,
		None::<HashScheme>,
		Some(ChecksumScheme::new_crc32()),
	);
	let mut cfg = res!(setup::default_cfg());
	// Every zone in this test's own directory, not the shared container the default names.
	cfg.zone_overrides = Default::default();
	setup::start_db(db_root.clone(), Some(cfg), schms_input, None, false, wipe)
}

fn bytes(n: usize, seed: u8) -> Vec<u8> {
	(0..n).map(|i| ((i * 31 + seed as usize) % 251) as u8).collect()
}

fn check_all(cas: &TestCas, when: &str) -> Outcome<()> {
	for (k, n) in LENS.iter().enumerate() {
		let want = bytes(*n, k as u8);
		let id = ContentId::of(&want);
		let got = match res!(cas.get(&id)) {
			Some(b)	=> b,
			None	=> return Err(err!(
				"The chunk of {} bytes put earlier is absent {}.", n, when;
			Test, Missing)),
		};
		if got.len() != want.len() {
			return Err(err!(
				"A chunk of {} bytes came back as {} bytes {}.", want.len(), got.len(), when;
			Test, Mismatch, Size));
		}
		if got != want {
			return Err(err!(
				"A chunk of {} bytes came back with other bytes {}.", n, when;
			Test, Mismatch, Data));
		}
		if !res!(cas.has(&id)) {
			return Err(err!("has() denies the chunk of {} bytes {}.", n, when; Test, Mismatch));
		}
	}
	Ok(())
}

#[test]
fn chunks_of_every_width_round_trip() -> Outcome<()> {
	log_set_level!("warn");
	let outcome = run();
	log_finish_wait!();
	outcome
}

fn run() -> Outcome<()> {
	let db_dir = PathBuf::from("./test_db_cas_o3db");
	let _ = std::fs::remove_dir_all(&db_dir);
	res!(std::fs::create_dir_all(&db_dir));
	let db_root = res!(db_dir.canonicalize());

	{
		let db = Arc::new(res!(open(&db_root, true)));
		let cas = TestCas::new(db.clone(), Uid::default());
		for (k, n) in LENS.iter().enumerate() {
			res!(cas.put(&Chunk::new(bytes(*n, k as u8))));
		}
		res!(check_all(&cas, "in the session that wrote it"));
		let ids = res!(cas.ids());
		if ids.len() != LENS.len() {
			return Err(err!(
				"ids() lists {} chunks after {} were put.", ids.len(), LENS.len();
			Test, Mismatch, Size));
		}
		res!(db.close());
	}

	{
		let db = Arc::new(res!(open(&db_root, false)));
		let cas = TestCas::new(db.clone(), Uid::default());
		res!(check_all(&cas, "after a close and a reopen"));
		res!(db.close());
	}
	Ok(())
}
