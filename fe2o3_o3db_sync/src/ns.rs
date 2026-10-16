//! A path namespace over Ozone: byte values under `/`-separated paths, and one index record per
//! directory, `{name → (is_dir, size, mtime_ms)}`, so a directory lists without a `scan`.
//!
//! - **Keys.** A value sits at `ns:{space}:v:{path}`, a directory's index at `ns:{space}:d:{dir}`,
//!   the root's at `ns:{space}:d:`. A directory exists while its index record does.
//! - **Order.** `put` writes the value, then the parent's index, then any parent it made, upward.
//!   `delete` writes the index first, then deletes the value. A crash between the two leaves a
//!   value with no entry (unlisted, though `get` still reads it) or an entry with no value.
//! - **Reads trust the index only so far.** `stat` and `list` ignore an entry whose value (or, for
//!   a directory, whose index record) is absent.
//! - **One mutation lane.** Every mutation holds the handle's FIFO lane ([`O3db::ns_lane`]) across
//!   its reads, checks and writes, including the parent index's read-modify-write, so a second
//!   `Ns` over the same handle cannot interleave with it. Reads take no lane.
//!
//! Meant for a single caller thread (the lane makes other threads safe, not fast). Nothing here
//! scans, and nothing on a request path may.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use crate::O3db;

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_iop_crypto::enc::Encrypter;
use oxedyne_fe2o3_iop_hash::{
	api::Hasher,
	csum::Checksummer,
};
use oxedyne_fe2o3_jdat::{
	prelude::*,
	id::NumIdDat,
};

use std::{
	collections::BTreeMap,
	sync::{
		Arc,
		Condvar,
		Mutex,
	},
	time::{
		SystemTime,
		UNIX_EPOCH,
	},
};


/// A FIFO ticket lock: holders are served in the order they asked.
#[derive(Debug, Default)]
pub struct Lane {
	turn:	Mutex<(u64, u64)>,	// (next ticket, ticket being served)
	moved:	Condvar,
}

/// The lane, held until dropped.
#[derive(Debug)]
pub struct LaneHold<'a> {
	lane: &'a Lane,
}

impl Lane {
	pub fn hold(&self) -> Outcome<LaneHold<'_>> {
		let mut turn = lock_mutex!(self.turn, "Taking a ticket for the ns mutation lane.");
		let mine = turn.0;
		turn.0 = turn.0.wrapping_add(1);
		while turn.1 != mine {
			turn = match self.moved.wait(turn) {
				Ok(t)	=> t,
				Err(_)	=> return Err(err!(
					"The ns mutation lane was poisoned while waiting for ticket {}.", mine;
				Lock, Poisoned)),
			};
		}
		Ok(LaneHold { lane: self })
	}
}

impl Drop for LaneHold<'_> {
	fn drop(&mut self) {
		// A poisoned lock still passes the turn on, or every later holder would wait for ever.
		let mut turn = match self.lane.turn.lock() {
			Ok(t)	=> t,
			Err(p)	=> p.into_inner(),
		};
		turn.1 = turn.1.wrapping_add(1);
		self.lane.moved.notify_all();
	}
}

/// One index entry.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Entry {
	pub is_dir:		bool,
	pub size:		u64,	// bytes; 0 for a directory
	pub mtime_ms:	u64,	// ms since the Unix epoch
}

/// What `get_capped` found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Capped {
	Absent,
	Whole(Vec<u8>),
	Over(u64),	// the size, over the cap; the value is not read
}

type Index = BTreeMap<String, Entry>;

pub fn now_ms() -> u64 {
	match SystemTime::now().duration_since(UNIX_EPOCH) {
		Ok(d)	=> d.as_millis() as u64,
		Err(_)	=> 0,
	}
}

/// Splits a non-root path into its parent directory and its name.
fn split(path: &str) -> (&str, &str) {
	match path.rfind('/') {
		Some(i)	=> (&path[..i], &path[i + 1..]),
		None	=> ("", path),
	}
}

/// A path is `/`-separated names, none empty, `.` or `..`, with no NUL; `""` is the root.
fn check_path(path: &str, root_ok: bool) -> Outcome<()> {
	if path.is_empty() {
		if root_ok {
			return Ok(());
		}
		return Err(err!("An ns path must not be the root here."; Invalid, Input, Path));
	}
	for part in path.split('/') {
		if part.is_empty() || part == "." || part == ".." || part.contains('\0') {
			return Err(err!(
				"The ns path '{}' has an empty, '.', '..' or NUL-bearing part.", path;
			Invalid, Input, Path));
		}
	}
	Ok(())
}

/// A path namespace in one `space` of an [`O3db`].
pub struct Ns<
	const UIDL: usize,
	UID:	NumIdDat<UIDL> + 'static,
	ENC:	Encrypter + 'static,
	KH:		Hasher + 'static,
	PR:		Hasher + 'static,
	CS:		Checksummer + 'static,
> {
	db:		Arc<O3db<UIDL, UID, ENC, KH, PR, CS>>,
	user:	UID,	// stamped on every write
	space:	String,
}

impl<
	const UIDL: usize,
	UID:	NumIdDat<UIDL> + 'static,
	ENC:	Encrypter + 'static,
	KH:		Hasher + 'static,
	PR:		Hasher + 'static,
	CS:		Checksummer + 'static,
>
	Ns<UIDL, UID, ENC, KH, PR, CS>
{
	pub fn new(db: Arc<O3db<UIDL, UID, ENC, KH, PR, CS>>, user: UID, space: &str) -> Self {
		Self { db, user, space: space.to_string() }
	}

	pub fn db(&self) -> &Arc<O3db<UIDL, UID, ENC, KH, PR, CS>> { &self.db }

	pub fn value_key(&self, path: &str) -> Dat {
		Dat::Str(fmt!("ns:{}:v:{}", self.space, path))
	}

	pub fn index_key(&self, dir: &str) -> Dat {
		Dat::Str(fmt!("ns:{}:d:{}", self.space, dir))
	}

	// ── Reads ────────────────────────────────────────────────────────────────────────────────

	/// The value at `path`, as stored (an orphan value with no index entry included).
	pub fn get(&self, path: &str) -> Outcome<Option<Vec<u8>>> {
		res!(check_path(path, false));
		match res!(self.db.api().get_wait(&self.value_key(path), None)) {
			None			=> Ok(None),
			Some((dat, _))	=> Ok(Some(res!(Self::bytes_of(dat, path)))),
		}
	}

	/// The value at `path` if its indexed size is at most `cap`; a larger one is not read.
	pub fn get_capped(&self, path: &str, cap: u64) -> Outcome<Capped> {
		match res!(self.stat(path)) {
			None								=> return Ok(Capped::Absent),
			Some(e) if e.is_dir					=> return Err(err!(
				"The ns path '{}' is a directory, not a value.", path; Invalid, Input, Path)),
			Some(e) if e.size > cap				=> return Ok(Capped::Over(e.size)),
			Some(_)								=> (),
		}
		match res!(self.get(path)) {
			None								=> Ok(Capped::Absent),
			Some(b) if b.len() as u64 > cap		=> Ok(Capped::Over(b.len() as u64)),
			Some(b)								=> Ok(Capped::Whole(b)),
		}
	}

	/// The index entry for `path`, if it is there and what it names is too. The root is a directory.
	pub fn stat(&self, path: &str) -> Outcome<Option<Entry>> {
		res!(check_path(path, true));
		if path.is_empty() {
			return Ok(Some(Entry { is_dir: true, size: 0, mtime_ms: 0 }));
		}
		let (dir, name) = split(path);
		let entry = match res!(self.read_index(dir)) {
			None		=> return Ok(None),
			Some(idx)	=> match idx.get(name) {
				None	=> return Ok(None),
				Some(e)	=> *e,
			},
		};
		let key = self.backing_key(path, &entry);
		if res!(self.db.api().contains_wait(&key, None)) {
			Ok(Some(entry))
		} else {
			Ok(None)
		}
	}

	/// The live entries of directory `dir`, by name; `None` when it does not exist.
	pub fn list(&self, dir: &str) -> Outcome<Option<Vec<(String, Entry)>>> {
		res!(check_path(dir, true));
		let idx = match res!(self.read_index(dir)) {
			Some(idx)					=> idx,
			None if dir.is_empty()		=> return Ok(Some(Vec::new())),
			None						=> return Ok(None),
		};
		if idx.is_empty() {
			return Ok(Some(Vec::new()));
		}
		let mut keys = Vec::with_capacity(idx.len());
		for (name, e) in &idx {
			keys.push(self.backing_key(&Self::join(dir, name), e));
		}
		let held = res!(self.db.api().contains_many_wait(&keys, None));
		let mut out = Vec::with_capacity(idx.len());
		for (i, (name, e)) in idx.into_iter().enumerate() {
			if held.get(i) == Some(&true) {
				out.push((name, e));
			}
		}
		Ok(Some(out))
	}

	// ── Mutations, each under the lane ───────────────────────────────────────────────────────

	pub fn put(&self, path: &str, bytes: &[u8]) -> Outcome<()> {
		self.put_at(path, bytes, now_ms())
	}

	/// `put` with the entry's mtime given. Parents are made, stamped `mtime_ms`.
	pub fn put_at(&self, path: &str, bytes: &[u8], mtime_ms: u64) -> Outcome<()> {
		res!(check_path(path, false));
		let _hold = res!(self.db.ns_lane().hold());

		// Plan every index write before the value goes in, so a conflict leaves nothing behind.
		let mut writes: Vec<(String, Index)> = Vec::new();
		let mut child = path.to_string();
		let mut entry = Entry { is_dir: false, size: bytes.len() as u64, mtime_ms };
		loop {
			let (dir, name) = split(&child);
			let (dir, name) = (dir.to_string(), name.to_string());
			let read = res!(self.read_index(&dir));
			let existed = read.is_some();
			let mut idx = match read {
				Some(idx)	=> idx,
				None		=> Index::new(),
			};
			match idx.get(&name) {
				Some(old) if old.is_dir != entry.is_dir => return Err(err!(
					"The ns path '{}' is a {}, so '{}' cannot be put.", Self::join(&dir, &name),
					if old.is_dir { "directory" } else { "value" }, path;
				Conflict, Path)),
				// A parent already listed: nothing above changes.
				Some(_) if entry.is_dir && existed => break,
				_ => (),
			}
			idx.insert(name, entry);
			writes.push((dir.clone(), idx));
			if existed || dir.is_empty() {
				break;
			}
			child = dir;
			entry = Entry { is_dir: true, size: 0, mtime_ms };
		}

		let resp = res!(self.db.api().store(
			self.value_key(path), Dat::wrap_dat(bytes.to_vec()), self.user));
		res!(resp.recv_store_ack());
		for (dir, idx) in writes {
			res!(self.write_index(&dir, &idx));
		}
		Ok(())
	}

	/// Deletes the value at `path`, or the empty directory there. Whether an entry was listed.
	pub fn delete(&self, path: &str) -> Outcome<bool> {
		res!(check_path(path, false));
		let _hold = res!(self.db.ns_lane().hold());
		let (dir, name) = split(path);
		let mut idx = match res!(self.read_index(dir)) {
			Some(idx)	=> idx,
			None		=> Index::new(),
		};
		let listed = idx.remove(name);
		if let Some(e) = listed {
			if e.is_dir {
				if let Some(inner) = res!(self.read_index(path)) {
					if !inner.is_empty() {
						return Err(err!(
							"The ns directory '{}' holds {} entries, so it is not deleted.",
							path, inner.len();
						Conflict, Path));
					}
				}
			}
			res!(self.write_index(dir, &idx));
		}
		let key = match listed {
			Some(e)	=> self.backing_key(path, &e),
			None	=> self.value_key(path),
		};
		if res!(self.db.api().contains_wait(&key, None)) {
			let resp = self.db.api().responder();
			res!(self.db.api().delete_using_responder(&key, self.user, None, resp.clone()));
			res!(resp.recv_delete_ack());
		}
		Ok(listed.is_some())
	}

	// ── Index records ────────────────────────────────────────────────────────────────────────

	fn join(dir: &str, name: &str) -> String {
		if dir.is_empty() { name.to_string() } else { fmt!("{}/{}", dir, name) }
	}

	// What must be present for an entry to be live: a value, or a directory's index record.
	fn backing_key(&self, path: &str, e: &Entry) -> Dat {
		if e.is_dir { self.index_key(path) } else { self.value_key(path) }
	}

	fn bytes_of(dat: Dat, path: &str) -> Outcome<Vec<u8>> {
		match dat {
			Dat::BU8(b) | Dat::BU16(b) | Dat::BU32(b) | Dat::BU64(b) => Ok(b),
			other => Err(err!(
				"The ns value at '{}' is not bytes: {:?}.", path, other.kind();
			Decode, Unexpected)),
		}
	}

	fn read_index(&self, dir: &str) -> Outcome<Option<Index>> {
		let dat = match res!(self.db.api().get_wait(&self.index_key(dir), None)) {
			None			=> return Ok(None),
			Some((dat, _))	=> dat,
		};
		let map = match dat {
			Dat::Map(m)	=> m,
			other		=> return Err(err!(
				"The ns index of '{}' is not a map: {:?}.", dir, other.kind();
			Decode, Unexpected)),
		};
		let mut idx = Index::new();
		for (k, v) in map {
			let name = match k {
				Dat::Str(s)	=> s,
				other		=> return Err(err!(
					"The ns index of '{}' has a non-string name: {:?}.", dir, other.kind();
				Decode, Unexpected)),
			};
			let e = match v {
				Dat::Tup3(t) => match (&t[0], &t[1], &t[2]) {
					(Dat::Bool(d), Dat::U64(s), Dat::U64(m)) =>
						Entry { is_dir: *d, size: *s, mtime_ms: *m },
					_ => return Err(err!(
						"The ns index of '{}' has a malformed entry for '{}'.", dir, name;
					Decode, Unexpected)),
				},
				other => return Err(err!(
					"The ns index of '{}' has a {:?} entry for '{}'.", dir, other.kind(), name;
				Decode, Unexpected)),
			};
			idx.insert(name, e);
		}
		Ok(Some(idx))
	}

	fn write_index(&self, dir: &str, idx: &Index) -> Outcome<()> {
		let mut map = DaticleMap::new();
		for (name, e) in idx {
			map.insert(
				Dat::Str(name.clone()),
				Dat::Tup3(Box::new([Dat::Bool(e.is_dir), Dat::U64(e.size), Dat::U64(e.mtime_ms)])),
			);
		}
		let resp = res!(self.db.api().store(self.index_key(dir), Dat::Map(map), self.user));
		res!(resp.recv_store_ack());
		Ok(())
	}
}
