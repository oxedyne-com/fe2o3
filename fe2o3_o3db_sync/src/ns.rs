//! A path namespace over Ozone: byte values under `/`-separated paths, and one index record per
//! directory, `{name → (is_dir, size, mtime_ms)}`, so a directory lists without a `scan`.
//!
//! - **Keys.** A value sits at `ns:{space}:v:{path}`, a directory's index at `ns:{space}:d:{dir}`,
//!   the root's at `ns:{space}:d:`. A directory exists while its index record does.
//! - **Order.** An index record exists only while its parent lists it. `put` writes the value,
//!   then the index records top-down (a parent it made first), so a crash leaves at worst a
//!   parent listing a directory whose record is not there yet; that entry reads as absent and the
//!   next `put` through it writes the record. `delete` drops a value's entry before the value, and
//!   a directory's record before its entry. `delete_tree` deletes the values, then the records
//!   deepest first, then the entry. `rename` copies, links the copy top-down, then deletes the
//!   original as `delete_tree` does.
//! - **Reads trust the index only so far.** `stat` and `list` ignore an entry whose value (or, for
//!   a directory, whose index record) is absent, and `put` replaces such a dead entry.
//! - **One mutation lane.** Every mutation holds the handle's FIFO lane ([`O3db::ns_lane`]) across
//!   its reads, checks and writes, including the parent index's read-modify-write, so a second
//!   `Ns` over the same handle cannot interleave with it. Reads take no lane. `put_if` and
//!   `delete_if` run their `pred` under it, so a mutation called from inside `pred` is refused.
//! - **Orphans.** A crash leaves values (and, from older stores, directory records) that no live
//!   chain reaches; [`orphans`] finds them by walking the store, for a background sweep only.
//!
//! Meant for a single caller thread (the lane makes other threads safe, not fast). Nothing on a
//! request path may scan; only [`orphans`] does, from `sweep`.
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
	thread::{
		self,
		ThreadId,
	},
	time::{
		SystemTime,
		UNIX_EPOCH,
	},
};


/// A FIFO ticket lock: holders are served in the order they asked. It is not re-entrant, so a
/// thread that asks again while holding it is refused rather than left waiting on itself.
#[derive(Debug, Default)]
pub struct Lane {
	turn:	Mutex<Turn>,
	moved:	Condvar,
}

#[derive(Debug, Default)]
struct Turn {
	next:		u64,				// the next ticket to hand out
	serving:	u64,				// the ticket being served
	holder:		Option<ThreadId>,	// the thread being served
}

/// The lane, held until dropped.
#[derive(Debug)]
pub(crate) struct LaneHold<'a> {
	lane: &'a Lane,
}

impl Lane {
	pub(crate) fn hold(&self) -> Outcome<LaneHold<'_>> {
		let me = thread::current().id();
		let mut turn = lock_mutex!(self.turn, "Taking a ticket for the ns mutation lane.");
		if turn.holder == Some(me) {
			return Err(err!(
				"This thread already holds the ns mutation lane, so a mutation called from inside \
				another (a put_if or delete_if predicate, say) is refused rather than left waiting \
				on itself.";
			Lock, Conflict));
		}
		let mine = turn.next;
		turn.next = turn.next.wrapping_add(1);
		while turn.serving != mine {
			turn = match self.moved.wait(turn) {
				Ok(t)	=> t,
				Err(_)	=> return Err(err!(
					"The ns mutation lane was poisoned while waiting for ticket {}.", mine;
				Lock, Poisoned)),
			};
		}
		turn.holder = Some(me);
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
		turn.holder = None;
		turn.serving = turn.serving.wrapping_add(1);
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

/// What [`orphans`] found: values and directory index records that no chain of live entries
/// from the root reaches.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Orphans {
	pub values:	Vec<String>,	// paths
	pub dirs:	Vec<String>,	// paths
}

pub use crate::sweep::ns_orphans as orphans;

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
	pub fn space(&self) -> &str { &self.space }

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
		self.put_held(path, bytes, mtime_ms, usize::MAX)
	}

	/// Puts `bytes` at `path` only if `pred` passes on its current entry (`None` when absent),
	/// all under the lane. Whether it was put.
	pub fn put_if<F>(&self, path: &str, bytes: &[u8], pred: F) -> Outcome<bool>
		where F: FnOnce(Option<&Entry>) -> bool
	{
		res!(check_path(path, false));
		let _hold = res!(self.db.ns_lane().hold());
		let now = res!(self.stat(path));
		if !pred(now.as_ref()) {
			return Ok(false);
		}
		res!(self.put_held(path, bytes, now_ms(), usize::MAX));
		Ok(true)
	}

	/// Deletes the value at `path`, or the empty directory there. Whether an entry was listed.
	pub fn delete(&self, path: &str) -> Outcome<bool> {
		res!(check_path(path, false));
		let _hold = res!(self.db.ns_lane().hold());
		self.delete_held(path)
	}

	/// `delete` only if `pred` passes on the current entry (`None` when absent), all under the
	/// lane. Whether `pred` passed.
	pub fn delete_if<F>(&self, path: &str, pred: F) -> Outcome<bool>
		where F: FnOnce(Option<&Entry>) -> bool
	{
		res!(check_path(path, false));
		let _hold = res!(self.db.ns_lane().hold());
		let now = res!(self.stat(path));
		if !pred(now.as_ref()) {
			return Ok(false);
		}
		res!(self.delete_held(path));
		Ok(true)
	}

	/// Deletes `path` and everything under it, dead entries' remains included: values first, then
	/// index records deepest first, then the parent's entry, so a crash leaves only dead entries.
	/// The number of values gone.
	pub fn delete_tree(&self, path: &str) -> Outcome<usize> {
		res!(check_path(path, false));
		let _hold = res!(self.db.ns_lane().hold());
		self.delete_tree_held(path)
	}

	/// Moves the value or tree at `from` to `to`, which must be absent and not inside `from`.
	/// The values are copied, `to` is linked and its index records written top-down, then `from`
	/// goes as `delete_tree` does. A crash leaves `from` whole, or `to` whole, or both.
	pub fn rename(&self, from: &str, to: &str) -> Outcome<()> {
		res!(check_path(from, false));
		res!(check_path(to, false));
		if to == from || to.starts_with(&fmt!("{}/", from)) {
			return Err(err!(
				"The ns path '{}' cannot be renamed to '{}', which is itself or inside it.", from, to;
			Invalid, Input, Path));
		}
		let _hold = res!(self.db.ns_lane().hold());
		let top = match res!(self.stat(from)) {
			Some(e)	=> e,
			None	=> return Err(err!("The ns path '{}' is absent, so not renamed.", from;
				Missing, Path)),
		};
		if res!(self.stat(to)).is_some() {
			return Err(err!("The ns path '{}' already exists, so '{}' is not renamed onto it.",
				to, from; Exists, Path));
		}
		if !top.is_dir {
			res!(self.copy_value(from, to));
			res!(self.link_held(to, top));
			res!(self.delete_held(from));
			return Ok(());
		}
		// Live entries only. The values go in unreachable; each index record is planned parent first.
		let mut plan: Vec<(String, Index)> = Vec::new();
		let mut todo = vec![String::new()];
		while let Some(rel) = todo.pop() {
			let live = match res!(self.list(&Self::join(from, &rel))) {
				Some(live)	=> live,
				None		=> Vec::new(),
			};
			let mut idx = Index::new();
			for (name, e) in live {
				let sub = Self::join(&rel, &name);
				if e.is_dir {
					todo.push(sub);
				} else {
					res!(self.copy_value(&Self::join(from, &sub), &Self::join(to, &sub)));
				}
				idx.insert(name, e);
			}
			plan.push((rel, idx));
		}
		res!(self.link_held(to, top));
		for (rel, idx) in plan {
			res!(self.write_index(&Self::join(to, &rel), &idx));
		}
		res!(self.delete_tree_held(from));
		Ok(())
	}

	#[doc(hidden)]
	/// A test's crash: `put` that stops after the value and `index_writes` index writes.
	pub fn put_torn(&self, path: &str, bytes: &[u8], index_writes: usize) -> Outcome<()> {
		res!(check_path(path, false));
		let _hold = res!(self.db.ns_lane().hold());
		self.put_held(path, bytes, now_ms(), index_writes)
	}

	// ── Lane-held bodies ─────────────────────────────────────────────────────────────────────

	fn put_held(&self, path: &str, bytes: &[u8], mtime_ms: u64, index_writes: usize)
		-> Outcome<()>
	{
		let writes = res!(self.plan_link(path,
			Entry { is_dir: false, size: bytes.len() as u64, mtime_ms }));
		let resp = res!(self.db.api().store(
			self.value_key(path), Dat::wrap_dat(bytes.to_vec()), self.user));
		res!(resp.recv_store_ack());
		// Top-down: a crash leaves a parent listing a directory whose index is not yet there,
		// which reads as absent, and the next put through it lists it again.
		for (dir, idx) in writes.into_iter().rev().take(index_writes) {
			res!(self.write_index(&dir, &idx));
		}
		Ok(())
	}

	// Lists `path` as `entry` in its parent, making parents, top-down.
	fn link_held(&self, path: &str, entry: Entry) -> Outcome<()> {
		let writes = res!(self.plan_link(path, entry));
		for (dir, idx) in writes.into_iter().rev() {
			res!(self.write_index(&dir, &idx));
		}
		Ok(())
	}

	// The index writes, leaf first, that list `path` as `entry` and make its parents. Planned
	// before anything is written, so a conflict leaves nothing behind. A dead entry (whose value
	// or index is absent) is no conflict: it is replaced.
	fn plan_link(&self, path: &str, entry: Entry) -> Outcome<Vec<(String, Index)>> {
		let mut writes: Vec<(String, Index)> = Vec::new();
		let mut child = path.to_string();
		let mut entry = entry;
		let mtime_ms = entry.mtime_ms;
		loop {
			let (dir, name) = split(&child);
			let (dir, name) = (dir.to_string(), name.to_string());
			let read = res!(self.read_index(&dir));
			let existed = read.is_some();
			let mut idx = match read {
				Some(idx)	=> idx,
				None		=> Index::new(),
			};
			if let Some(old) = idx.get(&name) {
				let live = res!(self.db.api().contains_wait(&self.backing_key(&child, old), None));
				if live && old.is_dir != entry.is_dir {
					return Err(err!(
						"The ns path '{}' is a {}, so '{}' cannot be put.", child,
						if old.is_dir { "directory" } else { "value" }, path;
					Conflict, Path));
				}
				// A parent already listed: nothing above changes.
				if old.is_dir && entry.is_dir && existed {
					break;
				}
			}
			idx.insert(name, entry);
			writes.push((dir.clone(), idx));
			if existed || dir.is_empty() {
				break;
			}
			child = dir;
			entry = Entry { is_dir: true, size: 0, mtime_ms };
		}
		Ok(writes)
	}

	fn delete_held(&self, path: &str) -> Outcome<bool> {
		let (dir, name) = split(path);
		let mut idx = match res!(self.read_index(dir)) {
			Some(idx)	=> idx,
			None		=> Index::new(),
		};
		match idx.remove(name) {
			Some(e) if e.is_dir => {
				// Only live entries count: a dead one (left by a crash) does not keep it.
				if let Some(inner) = res!(self.list(path)) {
					if !inner.is_empty() {
						return Err(err!(
							"The ns directory '{}' holds {} entries, so it is not deleted.",
							path, inner.len();
						Conflict, Path));
					}
				}
				res!(self.remove_key(&self.index_key(path)));
				res!(self.write_index(dir, &idx));
				Ok(true)
			},
			Some(_) => {
				res!(self.write_index(dir, &idx));
				res!(self.remove_key(&self.value_key(path)));
				Ok(true)
			},
			None => {
				res!(self.remove_key(&self.value_key(path)));
				Ok(false)
			},
		}
	}

	fn delete_tree_held(&self, path: &str) -> Outcome<usize> {
		let (values, dirs) = res!(self.tree_keys(path));
		for key in values.iter().chain(dirs.iter().rev()) {
			res!(self.remove_key(key));
		}
		let (dir, name) = split(path);
		if let Some(mut idx) = res!(self.read_index(dir)) {
			if idx.remove(name).is_some() {
				res!(self.write_index(dir, &idx));
			}
		}
		Ok(values.len())
	}

	// Every value key and directory index key at and under `path`, dead entries included; the
	// directories parent first.
	fn tree_keys(&self, path: &str) -> Outcome<(Vec<Dat>, Vec<Dat>)> {
		let mut values = vec![self.value_key(path)];
		let mut dirs = Vec::new();
		let mut todo = vec![path.to_string()];
		while let Some(dir) = todo.pop() {
			let idx = match res!(self.read_index(&dir)) {
				Some(idx)	=> idx,
				None		=> continue,
			};
			for (name, e) in idx {
				let sub = Self::join(&dir, &name);
				if e.is_dir {
					todo.push(sub);
				} else {
					values.push(self.value_key(&sub));
				}
			}
			dirs.push(self.index_key(&dir));
		}
		// Only what is there: the count is of values gone.
		let held = res!(self.db.api().contains_many_wait(&values, None));
		let values = values.into_iter().enumerate()
			.filter(|(i, _)| held.get(*i) == Some(&true))
			.map(|(_, k)| k)
			.collect();
		Ok((values, dirs))
	}

	fn copy_value(&self, from: &str, to: &str) -> Outcome<()> {
		let bytes = match res!(self.get(from)) {
			Some(b)	=> b,
			None	=> return Err(err!("The ns value at '{}' went while being copied.", from;
				Missing, Path)),
		};
		let resp = res!(self.db.api().store(self.value_key(to), Dat::wrap_dat(bytes), self.user));
		res!(resp.recv_store_ack());
		Ok(())
	}

	fn remove_key(&self, key: &Dat) -> Outcome<()> {
		if res!(self.db.api().contains_wait(key, None)) {
			let resp = self.db.api().responder();
			res!(self.db.api().delete_using_responder(key, self.user, None, resp.clone()));
			res!(resp.recv_delete_ack());
		}
		Ok(())
	}

	// ── Orphans ──────────────────────────────────────────────────────────────────────────────

	/// Of the keys a walk of the store found, those of this space no live chain reaches, judged
	/// under the lane so that no put is part way through.
	pub(crate) fn confirm_orphans(&self, keys: Vec<Dat>) -> Outcome<Orphans> {
		let vpre = fmt!("ns:{}:v:", self.space);
		let dpre = fmt!("ns:{}:d:", self.space);
		let _hold = res!(self.db.ns_lane().hold());
		let mut seen: BTreeMap<String, Option<Index>> = BTreeMap::new();
		let mut out = Orphans::default();
		for key in keys {
			let s = match &key {
				Dat::Str(s)	=> s,
				_			=> continue,
			};
			if let Some(path) = s.strip_prefix(&vpre) {
				if res!(self.db.api().contains_wait(&key, None))
					&& !res!(self.linked(path, false, &mut seen))
				{
					out.values.push(path.to_string());
				}
			} else if let Some(dir) = s.strip_prefix(&dpre) {
				if !dir.is_empty()
					&& res!(self.db.api().contains_wait(&key, None))
					&& !res!(self.linked(dir, true, &mut seen))
				{
					out.dirs.push(dir.to_string());
				}
			}
		}
		Ok(out)
	}

	// Whether every directory from the root down lists the next name, as the right kind.
	fn linked(&self, path: &str, is_dir: bool, seen: &mut BTreeMap<String, Option<Index>>)
		-> Outcome<bool>
	{
		if check_path(path, false).is_err() {
			return Ok(false);
		}
		let parts: Vec<&str> = path.split('/').collect();
		let mut dir = String::new();
		for (i, name) in parts.iter().enumerate() {
			if !seen.contains_key(&dir) {
				let idx = res!(self.read_index(&dir));
				seen.insert(dir.clone(), idx);
			}
			let want_dir = i + 1 < parts.len() || is_dir;
			match seen.get(&dir) {
				Some(Some(idx)) => match idx.get(*name) {
					Some(e) if e.is_dir == want_dir	=> (),
					_								=> return Ok(false),
				},
				_ => return Ok(false),
			}
			dir = Self::join(&dir, name);
		}
		Ok(true)
	}

	// ── Index records ────────────────────────────────────────────────────────────────────────

	fn join(dir: &str, name: &str) -> String {
		match (dir.is_empty(), name.is_empty()) {
			(true, _)		=> name.to_string(),
			(false, true)	=> dir.to_string(),
			(false, false)	=> fmt!("{}/{}", dir, name),
		}
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
