//! Retiring old and orphaned entries from the Ozone path namespace (`ns`), R1f3.
//!
//! A key-range delete takes the entries of one directory whose names fall in a half-open range,
//! values and subtrees alike; a TTL sweep, on a fake clock, deletes the journals older than seven
//! days and keeps the younger, a directory aged by the newest entry under it; and the orphan
//! sweep deletes a value no index reaches while a live value and another space's orphan stay.
//!
//! Each test opens its own store under `~/.cache/lane-daimond-runner/r1f3-db/`.
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
	data::core::RestSchemesInput,
	comm::response::Wait,
	ns::{
		self,
		Ns,
	},
	test::{
		hooks::{
			self,
			NsTear,
		},
		setup::{
			self,
			Uid,
			UID_LEN,
		},
	},
};

use std::{
	path::PathBuf,
	sync::Arc,
};


type Db = O3db<{ UID_LEN }, Uid, (), HashScheme, HashScheme, ChecksumScheme>;
type TestNs = Ns<{ UID_LEN }, Uid, (), HashScheme, HashScheme, ChecksumScheme>;

const DAY_MS: u64 = 24 * 60 * 60 * 1000;
const NOW_MS: u64 = 1_800_000_000_000;	// the fake clock: no test reads the wall clock

fn fresh_dir(name: &str) -> Outcome<PathBuf> {
	let home = match std::env::var_os("HOME") {
		Some(h)	=> PathBuf::from(h),
		None	=> return Err(err!("HOME is not set."; Test, Missing)),
	};
	let dir = home.join(".cache/lane-daimond-runner/r1f3-db").join(name);
	let _ = std::fs::remove_dir_all(&dir);
	res!(std::fs::create_dir_all(&dir));
	Ok(res!(dir.canonicalize()))
}

fn open(db_root: &PathBuf) -> Outcome<Arc<Db>> {
	let schms_input = RestSchemesInput::new(
		None::<()>,
		None::<HashScheme>,
		None::<HashScheme>,
		Some(ChecksumScheme::new_crc32()),
	);
	let mut cfg = res!(setup::default_cfg());
	cfg.zone_overrides = Default::default();
	Ok(Arc::new(res!(setup::start_db(db_root.clone(), Some(cfg), schms_input, None, false, true))))
}

fn wrap(outcome: Outcome<()>) -> Outcome<()> {
	log_finish_wait!();
	outcome
}

fn names(ns: &TestNs, dir: &str) -> Outcome<Vec<String>> {
	match res!(ns.list(dir)) {
		Some(entries)	=> Ok(entries.into_iter().map(|(n, _)| n).collect()),
		None			=> Ok(Vec::new()),
	}
}

#[test]
fn ns_sweep_key_range_delete() -> Outcome<()> {
	log_set_level!("warn");
	wrap(key_range())
}

fn key_range() -> Outcome<()> {
	let root = res!(fresh_dir("range"));
	let db = res!(open(&root));
	let ns = res!(TestNs::new(db.clone(), Uid::default(), "t"));
	res!(ns.put("j/a", b"a"));
	res!(ns.put("j/b", b"b"));
	res!(ns.put("j/c/x", b"x"));
	res!(ns.put("j/c/y", b"y"));
	res!(ns.put("j/d", b"d"));
	res!(ns.put("k/b", b"other dir"));
	// [b, d): b, and c with its two values.
	assert_eq!(res!(ns.delete_range("j", "b", "d")), 3);
	assert_eq!(res!(names(&ns, "j")), vec!["a".to_string(), "d".to_string()]);
	assert_eq!(res!(ns.get("j/c/x")), None);
	assert_eq!(res!(ns.stat("j/c")), None);
	assert_eq!(res!(ns.get("k/b")), Some(b"other dir".to_vec()));
	// An empty range, and a range over a missing directory, delete nothing.
	assert_eq!(res!(ns.delete_range("j", "d", "d")), 0);
	assert_eq!(res!(ns.delete_range("none", "a", "z")), 0);
	// The range never reaches the remains of what it deleted.
	assert_eq!(res!(ns::orphans(&ns, Wait::default())), ns::Orphans::default());
	res!(db.close());
	Ok(())
}

#[test]
fn ns_sweep_ttl_on_a_fake_clock() -> Outcome<()> {
	log_set_level!("warn");
	wrap(ttl())
}

fn ttl() -> Outcome<()> {
	let root = res!(fresh_dir("ttl"));
	let db = res!(open(&root));
	let ns = res!(TestNs::new(db.clone(), Uid::default(), "runner"));
	let week = 7 * DAY_MS;
	res!(ns.put_at("journals/old", b"o", NOW_MS - 8 * DAY_MS));
	res!(ns.put_at("journals/edge", b"e", NOW_MS - week));		// exactly 7 days: expired
	res!(ns.put_at("journals/young", b"y", NOW_MS - 6 * DAY_MS));
	res!(ns.put_at("journals/future", b"f", NOW_MS + DAY_MS));	// a clock step back: kept
	// A run journal as a directory: made 9 days ago, its last round written 1 day ago, so kept.
	res!(ns.put_at("journals/run1/r0", b"0", NOW_MS - 9 * DAY_MS));
	res!(ns.put_at("journals/run1/r1", b"1", NOW_MS - DAY_MS));
	// A directory wholly older than 7 days goes, with everything under it.
	res!(ns.put_at("journals/run2/r0", b"0", NOW_MS - 10 * DAY_MS));
	res!(ns.put_at("journals/run2/r1", b"1", NOW_MS - 8 * DAY_MS));
	assert_eq!(res!(ns.sweep_expired("journals", week, NOW_MS)), 4);
	let mut left = res!(names(&ns, "journals"));
	left.sort();
	assert_eq!(left, vec!["future".to_string(), "run1".to_string(), "young".to_string()]);
	assert_eq!(res!(ns.get("journals/run2/r1")), None);
	assert_eq!(res!(ns.get("journals/run1/r0")), Some(b"0".to_vec()));
	// A second sweep at the same instant finds nothing; `young` goes the moment it turns 7 days.
	assert_eq!(res!(ns.sweep_expired("journals", week, NOW_MS)), 0);
	assert_eq!(res!(ns.sweep_expired("journals", week, NOW_MS + DAY_MS - 1)), 0);
	assert_eq!(res!(ns.sweep_expired("journals", week, NOW_MS + DAY_MS)), 1);
	assert_eq!(res!(ns.get("journals/young")), None);
	assert_eq!(res!(ns::orphans(&ns, Wait::default())), ns::Orphans::default());
	res!(db.close());
	Ok(())
}

#[test]
fn ns_sweep_deletes_an_orphan_value() -> Outcome<()> {
	log_set_level!("warn");
	wrap(orphan())
}

fn orphan() -> Outcome<()> {
	let root = res!(fresh_dir("orphan"));
	let db = res!(open(&root));
	let ns = res!(TestNs::new(db.clone(), Uid::default(), "t"));
	res!(ns.put("o/kept", b"k"));
	hooks::set_ns_tear(NsTear::Top(0));
	res!(ns.put("o/lost", b"l"));		// a value, no entry
	let other = res!(TestNs::new(db.clone(), Uid::default(), "u"));
	hooks::set_ns_tear(NsTear::Top(0));
	res!(other.put("elsewhere", b"e"));	// another space's orphan, not this sweep's
	let swept = res!(ns::sweep_orphans(&ns, Wait::default()));
	assert_eq!(swept.values, vec!["o/lost".to_string()]);
	assert!(swept.dirs.is_empty());
	assert!(!res!(db.api().contains_wait(&ns.value_key("o/lost"), None)));
	assert_eq!(res!(ns.get("o/kept")), Some(b"k".to_vec()));
	assert_eq!(res!(ns::orphans(&ns, Wait::default())), ns::Orphans::default());
	assert_eq!(res!(ns::orphans(&other, Wait::default())).values, vec!["elsewhere".to_string()]);
	// A put at the swept key makes it live again, and the next sweep leaves it.
	res!(ns.put("o/lost", b"back"));
	assert_eq!(res!(ns::sweep_orphans(&ns, Wait::default())), ns::Orphans::default());
	assert_eq!(res!(ns.get("o/lost")), Some(b"back".to_vec()));
	res!(db.close());
	Ok(())
}
