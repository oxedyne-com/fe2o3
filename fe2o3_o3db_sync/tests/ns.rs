//! The path namespace over Ozone (`ns`): values under paths, with a per-directory index record.
//!
//! A round trip of `put`, `get`, `get_capped`, `stat`, `list` and `delete`; a listing that
//! survives a close and a reopen; a 4 MiB value, which the store chunks; an index entry whose value
//! is gone, which reads as absent; and 50 puts from 50 threads into one directory, which must
//! list 50 (without the mutation lane the directory's index read-modify-writes lose entries).
//!
//! D1z1b: a put torn after its first index write leaves no directory its parent does not list;
//! a mutation called from inside a `put_if` predicate returns instead of waiting on itself;
//! `delete` counts live entries only; racing `put_if`s have exactly one winner; `delete_tree`
//! and `rename` of a three-level tree; `orphans` lists a value no index reaches; and a second
//! `O3db::start` on one store is refused.
//!
//! D1z1c: a directory record that reached the disk while its parent's new entry did not (a power
//! loss reorders the two) is relinked by the next put through it; `rename` makes the copy durable
//! before it deletes a key of the original; and two spaces cannot share a key.
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
	comm::response::Wait,
	ns::{
		self,
		Capped,
		Ns,
	},
	test::{
		hooks::{
			self,
			NsStep,
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
	sync::{
		Arc,
		Barrier,
		mpsc,
	},
	thread,
	time::Duration,
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
	Ok(Arc::new(res!(start(db_root, wipe))))
}

fn start(db_root: &PathBuf, wipe: bool) -> Outcome<Db> {
	let schms_input = RestSchemesInput::new(
		None::<()>,
		None::<HashScheme>,
		None::<HashScheme>,
		Some(ChecksumScheme::new_crc32()),
	);
	let mut cfg = res!(setup::default_cfg());
	cfg.zone_overrides = Default::default();
	setup::start_db(db_root.clone(), Some(cfg), schms_input, None, false, wipe)
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
	let ns = res!(TestNs::new(db.clone(), Uid::default(), "t"));

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
fn ns_make_dirs_makes_folders_only() -> Outcome<()> {
	log_set_level!("warn");
	wrap(make_dirs())
}

fn make_dirs() -> Outcome<()> {
	let root = res!(fresh_dir("make_dirs"));
	let db = res!(open(&root, true));
	let ns = res!(TestNs::new(db.clone(), Uid::default(), "t"));

	// Every folder down to the path is made, each listing as an empty directory.
	res!(ns.make_dirs("a/b/c"));
	for p in ["a", "a/b", "a/b/c"] {
		match res!(ns.stat(p)) {
			Some(st)	=> assert!(st.is_dir, "{} is not a directory", p),
			None		=> return Err(err!("make_dirs did not make '{}'.", p; Test, Missing)),
		}
	}
	assert!(res!(names(&ns, "a/b/c")).is_empty());

	// Again, over what stands: nothing in it is lost.
	res!(ns.put("a/b/c/f", b"kept"));
	res!(ns.make_dirs("a/b/c"));
	res!(ns.make_dirs("a/b"));
	assert_eq!(res!(names(&ns, "a/b/c")), vec!["f".to_string()]);
	assert_eq!(res!(ns.get("a/b/c/f")), Some(b"kept".to_vec()));

	// A value in the way is a conflict, and nothing is made beneath it.
	res!(ns.put("v", b"x"));
	match ns.make_dirs("v/w") {
		Ok(())	=> return Err(err!("make_dirs made a folder under a value."; Test, Mismatch)),
		Err(e)	=> assert!(e.tags().contains(&ErrTag::Conflict), "{}", e),
	}
	assert!(ns.make_dirs("a/b/c/f").is_err());
	assert_eq!(res!(ns.get("v")), Some(b"x".to_vec()));
	assert!(ns.make_dirs("").is_err());

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
		let ns = res!(TestNs::new(db.clone(), Uid::default(), "t"));
		res!(ns.put("d/one", b"1"));
		res!(ns.put("d/two", b"22"));
		res!(ns.put("d/sub/three", b"333"));
		res!(db.close());
	}
	{
		let db = res!(open(&root, false));
		let ns = res!(TestNs::new(db.clone(), Uid::default(), "t"));
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
		let ns = res!(TestNs::new(db.clone(), Uid::default(), "t"));
		res!(ns.put("big/blob", &want));
		match res!(ns.get("big/blob")) {
			Some(got)	=> assert!(got == want, "4 MiB value came back as {} other bytes", got.len()),
			None		=> return Err(err!("The 4 MiB value is absent."; Test, Missing)),
		}
		res!(db.close());
	}
	{
		let db = res!(open(&root, false));
		let ns = res!(TestNs::new(db.clone(), Uid::default(), "t"));
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
	let ns = res!(TestNs::new(db.clone(), Uid::default(), "t"));
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
		let ns = res!(TestNs::new(db.clone(), Uid::default(), "t"));
		hands.push(thread::spawn(move || ns.put(&fmt!("dir/f{:02}", i), &[i as u8; 64])));
	}
	for h in hands {
		match h.join() {
			Ok(outcome)	=> res!(outcome),
			Err(_)		=> return Err(err!("A putting thread panicked."; Test, Thread, Panic)),
		}
	}
	let ns = res!(TestNs::new(db.clone(), Uid::default(), "t"));
	let listed = res!(names(&ns, "dir"));
	if listed.len() != RACERS {
		return Err(err!(
			"{} interleaved puts into one directory list {}.", RACERS, listed.len();
		Test, Mismatch, Size));
	}
	res!(db.close());
	Ok(())
}

// ── D1z1b ────────────────────────────────────────────────────────────────────────────────────

#[test]
fn ns_torn_put_leaves_no_unlisted_directory() -> Outcome<()> {
	log_set_level!("warn");
	wrap(torn_put())
}

fn torn_put() -> Outcome<()> {
	let root = res!(fresh_dir("torn_put"));
	let db = res!(open(&root, true));
	let ns = res!(TestNs::new(db.clone(), Uid::default(), "t"));
	res!(ns.put("a/x", b"x"));
	// The crash: the value of a/b/c and one of its two index writes are in.
	hooks::set_ns_tear(NsTear::Top(1));
	res!(ns.put("a/b/c", b"c"));
	// No directory exists that its parent does not list.
	if res!(ns.list("a/b")).is_some() && !res!(names(&ns, "a")).contains(&"b".to_string()) {
		return Err(err!("A torn put left a/b existing but unlisted in a."; Test, Mismatch));
	}
	assert_eq!(res!(names(&ns, "a")), vec!["x".to_string()]);
	assert!(res!(ns.stat("a/b/c")).is_none());
	// The next put through it heals the chain.
	res!(ns.put("a/b/d", b"d"));
	assert_eq!(res!(names(&ns, "a")), vec!["b".to_string(), "x".to_string()]);
	assert_eq!(res!(names(&ns, "a/b")), vec!["d".to_string()]);
	// A dead directory entry does not block a value put at its path.
	hooks::set_ns_tear(NsTear::Top(1));
	res!(ns.put("e/f/g", b"g"));
	res!(ns.put("e/f", b"now a value"));
	assert_eq!(res!(ns.get("e/f")), Some(b"now a value".to_vec()));
	res!(db.close());
	Ok(())
}

#[test]
fn ns_mutation_inside_a_predicate_returns() -> Outcome<()> {
	log_set_level!("warn");
	wrap(mutation_in_pred())
}

fn mutation_in_pred() -> Outcome<()> {
	let root = res!(fresh_dir("reentrant"));
	let db = res!(open(&root, true));
	let (tx, rx) = mpsc::channel();
	let ns = res!(TestNs::new(db.clone(), Uid::default(), "t"));
	let inner = res!(TestNs::new(db.clone(), Uid::default(), "t"));
	let _ = thread::spawn(move || {
		let mut nested: Option<bool> = None;
		let outer = ns.put_if("p/q", b"1", |_| {
			nested = Some(inner.put("p/r", b"2").is_err());
			true
		});
		let _ = tx.send((outer.is_ok(), nested));
	});
	match rx.recv_timeout(Duration::from_secs(20)) {
		Ok((true, Some(true)))	=> (),
		Ok(other)				=> return Err(err!(
			"A mutation inside a put_if predicate gave {:?}, not a refusal.", other; Test, Mismatch)),
		Err(_)					=> return Err(err!(
			"A mutation inside a put_if predicate did not return in 20 s."; Test, Timeout)),
	}
	let ns = res!(TestNs::new(db.clone(), Uid::default(), "t"));
	assert_eq!(res!(names(&ns, "p")), vec!["q".to_string()]);
	res!(db.close());
	Ok(())
}

#[test]
fn ns_delete_counts_live_entries_only() -> Outcome<()> {
	log_set_level!("warn");
	wrap(delete_live_only())
}

fn delete_live_only() -> Outcome<()> {
	let root = res!(fresh_dir("delete_live"));
	let db = res!(open(&root, true));
	let ns = res!(TestNs::new(db.clone(), Uid::default(), "t"));
	res!(ns.put("d/gone", b"g"));
	let resp = db.api().responder();
	res!(db.api().delete_using_responder(&ns.value_key("d/gone"), Uid::default(), None, resp.clone()));
	res!(resp.recv_delete_ack());
	assert!(res!(names(&ns, "d")).is_empty());
	match ns.delete("d") {
		Ok(true)	=> (),
		other		=> return Err(err!(
			"Deleting a directory with only a dead entry gave {:?}.", other; Test, Mismatch)),
	}
	assert!(res!(ns.list("d")).is_none());
	assert!(res!(names(&ns, "")).is_empty());
	res!(db.close());
	Ok(())
}

const CAS_RACERS: usize = 8;
const CAS_ROUNDS: usize = 5;

#[test]
fn ns_racing_put_if_has_one_winner() -> Outcome<()> {
	log_set_level!("warn");
	wrap(put_if_race())
}

fn put_if_race() -> Outcome<()> {
	let root = res!(fresh_dir("put_if_race"));
	let db = res!(open(&root, true));
	for round in 0..CAS_ROUNDS {
		let gate = Arc::new(Barrier::new(CAS_RACERS));
		let mut hands = Vec::with_capacity(CAS_RACERS);
		for i in 0..CAS_RACERS {
			let ns = res!(TestNs::new(db.clone(), Uid::default(), "t"));
			let gate = gate.clone();
			hands.push(thread::spawn(move || {
				gate.wait();
				ns.put_if(&fmt!("race/r{}", round), &[i as u8; 32], |now| now.is_none())
			}));
		}
		let mut wins = 0;
		for h in hands {
			match h.join() {
				Ok(outcome)	=> if res!(outcome) { wins += 1; },
				Err(_)		=> return Err(err!("A racing thread panicked."; Test, Thread, Panic)),
			}
		}
		if wins != 1 {
			return Err(err!(
				"Round {}: {} racing put_if calls on an absent path had {} winners.",
				round, CAS_RACERS, wins;
			Test, Mismatch));
		}
	}
	// delete_if passes or fails on the current entry.
	let ns = res!(TestNs::new(db.clone(), Uid::default(), "t"));
	assert!(!res!(ns.delete_if("race/r0", |now| now.map(|e| e.size) == Some(1))));
	assert!(res!(ns.stat("race/r0")).is_some());
	assert!(res!(ns.delete_if("race/r0", |now| now.map(|e| e.size) == Some(32))));
	assert!(res!(ns.stat("race/r0")).is_none());
	res!(db.close());
	Ok(())
}

#[test]
fn ns_tree_ops_on_three_levels() -> Outcome<()> {
	log_set_level!("warn");
	wrap(tree_ops())
}

fn tree_ops() -> Outcome<()> {
	let root = res!(fresh_dir("tree_ops"));
	let db = res!(open(&root, true));
	let ns = res!(TestNs::new(db.clone(), Uid::default(), "t"));
	for p in ["t/a", "t/u/b", "t/u/v/c", "t/u/v/d", "keep/k"] {
		res!(ns.put(p, p.as_bytes()));
	}

	// rename the tree, then a value.
	assert!(ns.rename("t", "t/inside").is_err());
	assert!(ns.rename("t", "keep").is_err());
	res!(ns.rename("t", "m/n"));
	assert!(res!(ns.list("t")).is_none());
	assert_eq!(res!(names(&ns, "")), vec!["keep".to_string(), "m".to_string()]);
	assert_eq!(res!(names(&ns, "m/n")), vec!["a".to_string(), "u".to_string()]);
	assert_eq!(res!(names(&ns, "m/n/u/v")), vec!["c".to_string(), "d".to_string()]);
	assert_eq!(res!(ns.get("m/n/u/v/d")), Some(b"t/u/v/d".to_vec()));
	assert_eq!(res!(ns.get("t/u/v/d")), None);
	res!(ns.rename("m/n/u/b", "keep/b2"));
	assert_eq!(res!(names(&ns, "keep")), vec!["b2".to_string(), "k".to_string()]);
	assert_eq!(res!(names(&ns, "m/n/u")), vec!["v".to_string()]);

	// delete_tree the rest.
	assert_eq!(res!(ns.delete_tree("m")), 3);
	assert!(res!(ns.list("m")).is_none());
	assert!(res!(ns.list("m/n/u/v")).is_none());
	assert_eq!(res!(ns.get("m/n/u/v/c")), None);
	assert_eq!(res!(names(&ns, "")), vec!["keep".to_string()]);
	assert_eq!(res!(ns::orphans(&ns, Wait::default())), ns::Orphans::default());
	res!(db.close());
	Ok(())
}

#[test]
fn ns_orphans_lists_a_value_with_no_index_entry() -> Outcome<()> {
	log_set_level!("warn");
	wrap(orphans())
}

fn orphans() -> Outcome<()> {
	let root = res!(fresh_dir("orphans"));
	let db = res!(open(&root, true));
	let ns = res!(TestNs::new(db.clone(), Uid::default(), "t"));
	res!(ns.put("o/kept", b"k"));
	hooks::set_ns_tear(NsTear::Top(0));
	res!(ns.put("o/lost", b"l"));		// a value, no entry
	hooks::set_ns_tear(NsTear::Top(1));
	res!(ns.put("o/p/q", b"q"));		// o lists p, p has no record
	let other = res!(TestNs::new(db.clone(), Uid::default(), "u"));
	hooks::set_ns_tear(NsTear::Top(0));
	res!(other.put("elsewhere", b"e"));	// another space's orphan
	let found = res!(ns::orphans(&ns, Wait::default()));
	let mut values = found.values.clone();
	values.sort();
	assert_eq!(values, vec!["o/lost".to_string(), "o/p/q".to_string()]);
	assert!(found.dirs.is_empty());
	res!(db.close());
	Ok(())
}

#[test]
fn ns_second_start_on_one_store_is_refused() -> Outcome<()> {
	log_set_level!("warn");
	wrap(second_start())
}

fn second_start() -> Outcome<()> {
	let root = res!(fresh_dir("second_start"));
	let db = res!(open(&root, true));
	match start(&root, false) {
		Err(_)		=> (),
		Ok(second)	=> {
			let _ = second.close();
			return Err(err!("A second O3db::start on one store was not refused."; Test, Mismatch));
		},
	}
	let ns = res!(TestNs::new(db.clone(), Uid::default(), "t"));
	res!(ns.put("still/here", b"yes"));
	assert_eq!(res!(ns.get("still/here")), Some(b"yes".to_vec()));
	res!(db.close());
	Ok(())
}

// ── D1z1c ────────────────────────────────────────────────────────────────────────────────────

#[test]
fn ns_put_heals_a_directory_record_its_parent_does_not_list() -> Outcome<()> {
	log_set_level!("warn");
	wrap(heals_unlisted())
}

fn heals_unlisted() -> Outcome<()> {
	let root = res!(fresh_dir("heals_unlisted"));
	let db = res!(open(&root, true));
	let ns = res!(TestNs::new(db.clone(), Uid::default(), "t"));
	res!(ns.put("a/x", b"x"));
	// The power loss: a/b's new record reached the disk, a's new entry for b did not.
	hooks::set_ns_tear(NsTear::Deep(1));
	res!(ns.put("a/b/c", b"c"));
	assert_eq!(res!(names(&ns, "a")), vec!["x".to_string()]);
	assert_eq!(res!(names(&ns, "a/b")), vec!["c".to_string()]);
	// The next put through it lists b in a again, whatever order the disk kept.
	res!(ns.put("a/b/d", b"d"));
	assert_eq!(res!(names(&ns, "a")), vec!["b".to_string(), "x".to_string()]);
	assert_eq!(res!(names(&ns, "a/b")), vec!["c".to_string(), "d".to_string()]);
	res!(db.close());
	Ok(())
}

#[test]
fn ns_rename_is_durable_before_it_deletes() -> Outcome<()> {
	log_set_level!("warn");
	wrap(rename_barrier())
}

fn rename_barrier() -> Outcome<()> {
	let root = res!(fresh_dir("rename_barrier"));
	let db = res!(open(&root, true));
	let ns = res!(TestNs::new(db.clone(), Uid::default(), "t"));
	res!(ns.put("r/one", b"1"));
	res!(ns.put("r/s/two", b"2"));
	res!(ns.put("v", b"value"));
	for (from, to) in [("r", "q"), ("v", "w")] {
		hooks::set_ns_trace();
		res!(ns.rename(from, to));
		let steps = hooks::ns_steps();
		let barrier = steps.iter().position(|s| *s == NsStep::Barrier);
		let delete = steps.iter().position(|s| *s == NsStep::Delete);
		match (barrier, delete) {
			(Some(b), Some(d)) if b < d	=> (),
			_ => return Err(err!(
				"rename of '{}' deleted before its copy was durable: {:?}.", from, steps;
			Test, Mismatch)),
		}
	}
	assert_eq!(res!(names(&ns, "q")), vec!["one".to_string(), "s".to_string()]);
	assert_eq!(res!(ns.get("w")), Some(b"value".to_vec()));
	res!(db.close());
	Ok(())
}

#[test]
fn ns_spaces_cannot_share_a_key() -> Outcome<()> {
	log_set_level!("warn");
	wrap(space_names())
}

fn space_names() -> Outcome<()> {
	let root = res!(fresh_dir("space_names"));
	let db = res!(open(&root, true));
	// "x" with "v:p" and "x:v" with "p" would both be ns:x:v:v:p, so a ':' is refused, as is "".
	let x = res!(TestNs::new(db.clone(), Uid::default(), "x"));
	assert!(TestNs::new(db.clone(), Uid::default(), "x:v").is_err());
	assert!(TestNs::new(db.clone(), Uid::default(), "").is_err());
	res!(x.put("v:p", b"x's"));
	assert_eq!(res!(x.get("v:p")), Some(b"x's".to_vec()));
	res!(db.close());
	Ok(())
}
