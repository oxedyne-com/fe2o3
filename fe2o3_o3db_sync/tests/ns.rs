//! The path namespace over Ozone (`ns`): values under paths, with a per-directory index record.
//!
//! A round trip of `put`, `get`, `get_capped`, `stat`, `list` and `delete`; a listing that
//! survives a close and a reopen; a 4 MiB value, which the store chunks; an index entry whose value
//! is gone, which reads as absent; and 50 puts from 50 threads into one directory, which must
//! list 50 (without the mutation lane the directory's index read-modify-writes lose entries).
//!
//! Each test opens its own store under `~/.cache/lane-daimond-runner/rnd1z1-db/`.
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
	ns::{
		Capped,
		Ns,
	},
	test::setup::{
		self,
		Uid,
		UID_LEN,
	},
};

use std::{
	path::PathBuf,
	sync::Arc,
	thread,
};


type Db = O3db<{ UID_LEN }, Uid, (), HashScheme, HashScheme, ChecksumScheme>;
type TestNs = Ns<{ UID_LEN }, Uid, (), HashScheme, HashScheme, ChecksumScheme>;

fn fresh_dir(name: &str) -> Outcome<PathBuf> {
	let home = match std::env::var_os("HOME") {
		Some(h)	=> PathBuf::from(h),
		None	=> return Err(err!("HOME is not set."; Test, Missing)),
	};
	let dir = home.join(".cache/lane-daimond-runner/rnd1z1-db").join(name);
	let _ = std::fs::remove_dir_all(&dir);
	res!(std::fs::create_dir_all(&dir));
	Ok(res!(dir.canonicalize()))
}

fn open(db_root: &PathBuf, wipe: bool) -> Outcome<Arc<Db>> {
	let schms_input = RestSchemesInput::new(
		None::<()>,
		None::<HashScheme>,
		None::<HashScheme>,
		Some(ChecksumScheme::new_crc32()),
	);
	let mut cfg = res!(setup::default_cfg());
	cfg.zone_overrides = Default::default();
	Ok(Arc::new(res!(setup::start_db(db_root.clone(), Some(cfg), schms_input, None, false, wipe))))
}

fn wrap(outcome: Outcome<()>) -> Outcome<()> {
	log_finish_wait!();
	outcome
}

fn names(ns: &TestNs, dir: &str) -> Outcome<Vec<String>> {
	match res!(ns.list(dir)) {
		Some(entries)	=> Ok(entries.into_iter().map(|(n, _)| n).collect()),
		None			=> Err(err!("Directory '{}' lists as absent.", dir; Test, Missing)),
	}
}

#[test]
fn ns_round_trip() -> Outcome<()> {
	log_set_level!("warn");
	wrap(round_trip())
}

fn round_trip() -> Outcome<()> {
	let root = res!(fresh_dir("round_trip"));
	let db = res!(open(&root, true));
	let ns = TestNs::new(db.clone(), Uid::default(), "t");

	res!(ns.put("a/b/c.txt", b"hello"));
	res!(ns.put_at("a/d.txt", b"there!", 1_234));
	assert_eq!(res!(ns.get("a/b/c.txt")), Some(b"hello".to_vec()));
	assert_eq!(res!(ns.get("a/b/none")), None);

	let st = match res!(ns.stat("a/d.txt")) {
		Some(st)	=> st,
		None		=> return Err(err!("stat of a/d.txt is absent."; Test, Missing)),
	};
	assert!(!st.is_dir);
	assert_eq!(st.size, 6);
	assert_eq!(st.mtime_ms, 1_234);
	match res!(ns.stat("a/b")) {
		Some(st)	=> assert!(st.is_dir),
		None		=> return Err(err!("The parent a/b was not made."; Test, Missing)),
	}
	assert!(res!(ns.stat("")).is_some());
	assert_eq!(res!(names(&ns, "")), vec!["a".to_string()]);
	assert_eq!(res!(names(&ns, "a")), vec!["b".to_string(), "d.txt".to_string()]);
	assert_eq!(res!(names(&ns, "a/b")), vec!["c.txt".to_string()]);
	assert!(res!(ns.list("nowhere")).is_none());

	// Overwrite: the index follows the new size.
	res!(ns.put("a/b/c.txt", b"hello again"));
	match res!(ns.stat("a/b/c.txt")) {
		Some(st)	=> assert_eq!(st.size, 11),
		None		=> return Err(err!("stat after an overwrite is absent."; Test, Missing)),
	}

	match res!(ns.get_capped("a/b/c.txt", 11)) {
		Capped::Whole(b)	=> assert_eq!(b, b"hello again".to_vec()),
		other				=> return Err(err!("get_capped at the size gave {:?}.", other; Test, Mismatch)),
	}
	match res!(ns.get_capped("a/b/c.txt", 10)) {
		Capped::Over(n)	=> assert_eq!(n, 11),
		other			=> return Err(err!("get_capped under the size gave {:?}.", other; Test, Mismatch)),
	}
	match res!(ns.get_capped("a/zzz", 10)) {
		Capped::Absent	=> (),
		other			=> return Err(err!("get_capped of nothing gave {:?}.", other; Test, Mismatch)),
	}

	// A file and a directory may not share a path.
	assert!(ns.put("a/b", b"x").is_err());
	assert!(ns.put("a/d.txt/e", b"x").is_err());
	assert!(ns.put("a//e", b"x").is_err());
	assert!(ns.put("a/../e", b"x").is_err());

	// A non-empty directory is not deleted; files are, then the emptied directory.
	assert!(ns.delete("a/b").is_err());
	assert!(res!(ns.delete("a/b/c.txt")));
	assert!(!res!(ns.delete("a/b/c.txt")));
	assert_eq!(res!(ns.get("a/b/c.txt")), None);
	assert!(res!(ns.stat("a/b/c.txt")).is_none());
	assert!(res!(names(&ns, "a/b")).is_empty());
	assert!(res!(ns.delete("a/b")));
	assert!(res!(ns.list("a/b")).is_none());
	assert_eq!(res!(names(&ns, "a")), vec!["d.txt".to_string()]);

	res!(db.close());
	Ok(())
}

#[test]
fn ns_list_after_reopen() -> Outcome<()> {
	log_set_level!("warn");
	wrap(list_after_reopen())
}

fn list_after_reopen() -> Outcome<()> {
	let root = res!(fresh_dir("reopen"));
	{
		let db = res!(open(&root, true));
		let ns = TestNs::new(db.clone(), Uid::default(), "t");
		res!(ns.put("d/one", b"1"));
		res!(ns.put("d/two", b"22"));
		res!(ns.put("d/sub/three", b"333"));
		res!(db.close());
	}
	{
		let db = res!(open(&root, false));
		let ns = TestNs::new(db.clone(), Uid::default(), "t");
		assert_eq!(res!(names(&ns, "d")),
			vec!["one".to_string(), "sub".to_string(), "two".to_string()]);
		assert_eq!(res!(ns.get("d/sub/three")), Some(b"333".to_vec()));
		match res!(ns.stat("d/two")) {
			Some(st)	=> assert_eq!(st.size, 2),
			None		=> return Err(err!("d/two is absent after a reopen."; Test, Missing)),
		}
		res!(db.close());
	}
	Ok(())
}

#[test]
fn ns_four_mib_value_round_trips() -> Outcome<()> {
	log_set_level!("warn");
	wrap(four_mib())
}

fn four_mib() -> Outcome<()> {
	let root = res!(fresh_dir("four_mib"));
	let want: Vec<u8> = (0..4 * 1024 * 1024).map(|i: usize| ((i * 31 + 7) % 251) as u8).collect();
	{
		let db = res!(open(&root, true));
		let ns = TestNs::new(db.clone(), Uid::default(), "t");
		res!(ns.put("big/blob", &want));
		match res!(ns.get("big/blob")) {
			Some(got)	=> assert!(got == want, "4 MiB value came back as {} other bytes", got.len()),
			None		=> return Err(err!("The 4 MiB value is absent."; Test, Missing)),
		}
		res!(db.close());
	}
	{
		let db = res!(open(&root, false));
		let ns = TestNs::new(db.clone(), Uid::default(), "t");
		match res!(ns.get_capped("big/blob", 4 * 1024 * 1024)) {
			Capped::Whole(got)	=> assert!(got == want, "4 MiB value differs after a reopen"),
			other				=> return Err(err!(
				"The 4 MiB value after a reopen gave {:?}.", other; Test, Mismatch)),
		}
		res!(db.close());
	}
	Ok(())
}

#[test]
fn ns_index_entry_without_value_reads_absent() -> Outcome<()> {
	log_set_level!("warn");
	wrap(entry_without_value())
}

fn entry_without_value() -> Outcome<()> {
	let root = res!(fresh_dir("no_value"));
	let db = res!(open(&root, true));
	let ns = TestNs::new(db.clone(), Uid::default(), "t");
	res!(ns.put("d/kept", b"k"));
	res!(ns.put("d/lost", b"l"));
	// The value goes, its index entry stays: a crash between `delete`'s two writes, reversed.
	let key = ns.value_key("d/lost");
	let resp = db.api().responder();
	res!(db.api().delete_using_responder(&key, Uid::default(), None, resp.clone()));
	res!(resp.recv_delete_ack());

	assert!(res!(ns.stat("d/lost")).is_none());
	assert_eq!(res!(ns.get("d/lost")), None);
	assert_eq!(res!(names(&ns, "d")), vec!["kept".to_string()]);
	res!(db.close());
	Ok(())
}

const RACERS: usize = 50;

#[test]
fn ns_fifty_interleaved_puts_list_fifty() -> Outcome<()> {
	log_set_level!("warn");
	wrap(fifty_puts())
}

fn fifty_puts() -> Outcome<()> {
	let root = res!(fresh_dir("fifty"));
	let db = res!(open(&root, true));
	let mut hands = Vec::with_capacity(RACERS);
	for i in 0..RACERS {
		// A second `Ns` over the same handle shares its lane.
		let ns = TestNs::new(db.clone(), Uid::default(), "t");
		hands.push(thread::spawn(move || ns.put(&fmt!("dir/f{:02}", i), &[i as u8; 64])));
	}
	for h in hands {
		match h.join() {
			Ok(outcome)	=> res!(outcome),
			Err(_)		=> return Err(err!("A putting thread panicked."; Test, Thread, Panic)),
		}
	}
	let ns = TestNs::new(db.clone(), Uid::default(), "t");
	let listed = res!(names(&ns, "dir"));
	if listed.len() != RACERS {
		return Err(err!(
			"{} interleaved puts into one directory list {}.", RACERS, listed.len();
		Test, Mismatch, Size));
	}
	res!(db.close());
	Ok(())
}
