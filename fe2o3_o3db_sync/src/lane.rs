//! A sequence lane: a bounded log under one key prefix in any [`Database`], each entry at its own
//! rising sequence number, read in pages above a cursor, trimmed from the bottom, with lazy expiry
//! and per-reader cursors whose least live value is the lane's `stable` point.
//!
//! Every method runs while the caller holds THAT LANE's lock, and [`Lane::new`] takes the
//! [`LaneGuard`] that proves it, obtainable only from [`LaneLocks::lock`]: the head, the floor and
//! the reader table are read, changed and written back, and two unserialised callers on one lane
//! would lose an update.  `read_after` writes too, since it deletes the expired entries it meets.
//! The lane's prefix is the guard's, so a guard for one lane cannot open another.  Only the lane
//! is serialised, not the store: the database is shared (`&DB`, its `insert`, `get` and `delete`
//! being `&self` and thread-safe), so lanes of different accounts and scopes run side by side
//! and a lane section never holds up the rest of the store.  Every key a lane touches is under
//! its own prefix, so two lanes never write the same key.
//!
//! Each request's hold on its lane is bounded by the caller's doors: an append batch by its
//! record count (the caller's), and a page by both bytes and `max_steps`, the slots (entries,
//! holes and expired entries alike) one [`Lane::read_after`] may visit before it answers `more`.
//! The lane reads values throughout and never relies on a key-only presence answer.
//!
//! A replay whose entry has gone (trimmed, removed or expired) appends anew; refusing a replayed
//! ephemeral record after its expiry is the caller's (Daimond's C8), not the lane's.
//!
//! Keys, under the caller's prefix `p`:
//!
//! | key | holds |
//! |---|---|
//! | `p/m` | `[head, floor]`: the last sequence handed out, and the highest trimmed |
//! | `p/e/<seq>` | `[exp_ms, id, value]`, `exp_ms` 0 for never, `id` empty for none |
//! | `p/i/<hex id>` | the sequence the entry with that id was appended at |
//! | `p/r` | the reader table, `{name: [cursor, live]}`, one value, bounded by the caller's readers |
//!
//! Built for Daimond's sync carrier (design (2), 16 Oct 2026, §7; the U2 delta of 10 Oct, §3.1,
//! row G1).  Steel's paced outbox keeps the same shape by hand.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_iop_crypto::enc::Encrypter;
use oxedyne_fe2o3_iop_db::api::Database;
use oxedyne_fe2o3_iop_hash::api::Hasher;
use oxedyne_fe2o3_jdat::{
	prelude::*,
	id::NumIdDat,
};

use std::{
	collections::hash_map::RandomState,
	hash::BuildHasher,
	marker::PhantomData,
	sync::{
		Mutex,
		MutexGuard,
	},
};


/// The per-lane locks: a fixed set of mutexes (stripes), each lane taking the one its prefix
/// hashes to under a per-process random key, so the set is bounded whatever the number of lanes
/// and nobody outside can aim two lanes at one stripe.  Two lanes on one stripe only take turns.
///
/// Hold at most one [`LaneGuard`] per thread: two lanes on one stripe would wait on each other.
#[derive(Debug)]
pub struct LaneLocks {
	stripes:	Box<[Mutex<()>]>,
	hasher:		RandomState,
}

impl Default for LaneLocks {
	fn default() -> Self {
		Self::new(Self::DEFAULT_STRIPES)
	}
}

impl LaneLocks {
	/// Enough stripes that unrelated lanes rarely meet, at one mutex each.
	pub const DEFAULT_STRIPES: usize = 256;

	/// `stripes` mutexes, at least one.
	pub fn new(stripes: usize) -> Self {
		let n = stripes.max(1);
		let mut v = Vec::with_capacity(n);
		for _ in 0..n {
			v.push(Mutex::new(()));
		}
		Self {
			stripes:	v.into_boxed_slice(),
			hasher:		RandomState::new(),
		}
	}

	/// Takes the lock of the lane under `prefix`, waiting while another holds it, and answers the
	/// guard [`Lane::new`] takes.  A poisoned stripe is taken as it is: a lane writes each entry
	/// before the head that counts it, so a section cut short by a panic leaves a lane that reads
	/// whole.
	pub fn lock<S: Into<String>>(&self, prefix: S) -> LaneGuard<'_> {
		let prefix = prefix.into();
		let i = self.stripe(&prefix);
		let held = match self.stripes[i].lock() {
			Ok(g)	=> g,
			Err(p)	=> p.into_inner(),
		};
		LaneGuard { _held: held, prefix }
	}

	/// As [`lock`](Self::lock), but `None` rather than waiting when the lane's stripe is held.
	pub fn try_lock<S: Into<String>>(&self, prefix: S) -> Option<LaneGuard<'_>> {
		let prefix = prefix.into();
		let i = self.stripe(&prefix);
		let held = match self.stripes[i].try_lock() {
			Ok(g)									=> g,
			Err(std::sync::TryLockError::Poisoned(p))	=> p.into_inner(),
			Err(std::sync::TryLockError::WouldBlock)	=> return None,
		};
		Some(LaneGuard { _held: held, prefix })
	}

	fn stripe(&self, prefix: &str) -> usize {
		(self.hasher.hash_one(prefix) % self.stripes.len() as u64) as usize
	}
}

/// The proof that the holder has the lock of the lane under [`prefix`](Self::prefix), from
/// [`LaneLocks::lock`] only; the lock is released when it drops.
#[derive(Debug)]
pub struct LaneGuard<'l> {
	_held:	MutexGuard<'l, ()>,
	prefix:	String,
}

impl LaneGuard<'_> {
	/// The prefix of the lane this guard holds.
	pub fn prefix(&self) -> &str {
		&self.prefix
	}
}


/// What [`Lane::append`] answers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Appended {
	pub seq:	u64,
	pub replay:	bool,	// the id was already held, so nothing was written
}

/// One entry as read.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
	pub seq:	u64,
	pub exp_ms:	Option<u64>,
	pub val:	Dat,
}

/// One answer of [`Lane::read_after`].
#[derive(Clone, Debug, PartialEq)]
pub struct Page {
	pub entries:	Vec<Entry>,
	pub through:	u64,	// every sequence up to this one has been answered or is a hole
	pub more:		bool,	// entries above `through` were left for the next page
	pub floor:		u64,	// the highest sequence trimmed when the page was read
	pub gap:		bool,	// `after` was below the floor, so trimmed entries were never answered
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReaderState {
	Live,
	Dormant,	// kept, but out of `stable` until it acknowledges again
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reader {
	pub name:	String,
	pub cursor:	u64,
	pub state:	ReaderState,
}

/// A lane over a shared database, under its lane's lock.  Made for one section and dropped with
/// it; it holds nothing the store does not.
#[derive(Debug)]
pub struct Lane<
	'a,
	const UIDL: usize,
	UID:	NumIdDat<UIDL>,
	ENC:	Encrypter,
	KH:		Hasher,
	DB:		Database<UIDL, UID, ENC, KH>,
> {
	db:		&'a DB,
	user:	UID,
	prefix:	&'a str,
	phantom:	PhantomData<(ENC, KH)>,
}

impl<
	'a,
	const UIDL: usize,
	UID:	NumIdDat<UIDL>,
	ENC:	Encrypter,
	KH:		Hasher,
	DB:		Database<UIDL, UID, ENC, KH>,
>
	Lane<'a, UIDL, UID, ENC, KH, DB>
{
	/// The guard is the proof of the lane's section, and names the lane: the lane keeps the borrow
	/// for its life, so the lock outlives every call on it.
	pub fn new(db: &'a DB, user: UID, guard: &'a LaneGuard<'_>) -> Self {
		Self {
			db,
			user,
			prefix:		guard.prefix(),
			phantom:	PhantomData,
		}
	}

	/// The last sequence handed out, 0 for a lane never appended to.
	pub fn head(&self) -> Outcome<u64> {
		Ok(res!(self.meta()).0)
	}

	/// The highest sequence trimmed; reads start above it.
	pub fn floor(&self) -> Outcome<u64> {
		Ok(res!(self.meta()).1)
	}

	/// The sequences above the floor, entries and holes alike: an upper bound on the entries held.
	pub fn depth(&self) -> Outcome<u64> {
		let (head, floor) = res!(self.meta());
		Ok(head - floor)
	}

	/// Appends `val` at the next sequence.  With an `id` the lane already holds, nothing is written
	/// and the held entry's sequence is answered, so a replayed append does not double.  Held means
	/// the entry the id record names still carries that id: after a crash between the id write and
	/// the head write, the next append takes that sequence, and the dangling record is not held.  The
	/// id is
	/// forgotten with its entry, so an append replayed after its entry was trimmed, removed or
	/// expired appends anew.  `exp_ms` is the millisecond the entry stops reading, on whatever clock
	/// the caller passes to [`read_after`](Self::read_after).
	pub fn append(&self, val: Dat, id: Option<&[u8]>, exp_ms: Option<u64>) -> Outcome<Appended> {
		if val.byte_len().is_none() {
			return Err(err!(
				"lane {}: a value whose encoded length cannot be measured (an arbitrary-precision \
				number) cannot be paged by bytes.", self.prefix;
			Invalid, Input));
		}
		if let Some(id) = id {
			if let Some(seq) = res!(self.id_seq(id)) {
				if let Some((_, Some(held), _)) = res!(self.entry_at(seq)) {
					if held.as_slice() == id {
						return Ok(Appended { seq, replay: true });
					}
				}
			}
		}
		let (head, floor) = res!(self.meta());
		let seq = match head.checked_add(1) {
			Some(s)	=> s,
			None	=> return Err(err!("lane {}: the sequence is spent.", self.prefix; Overflow)),
		};
		let stored = Dat::List(vec![
			Dat::U64(exp_ms.unwrap_or(0)),
			match id {
				Some(b)	=> Dat::bytdat(b.to_vec()),
				None	=> Dat::Empty,
			},
			val,
		]);
		// The entry before the head, so a reader never sees a head whose entry is not yet written.
		res!(self.db.insert(self.entry_key(seq), stored, self.user, None));
		if let Some(id) = id {
			res!(self.db.insert(self.id_key(id), Dat::U64(seq), self.user, None));
		}
		res!(self.put_meta(seq, floor));
		Ok(Appended { seq, replay: false })
	}

	/// Entries above `after` in sequence order, stopping before the one that would carry the page
	/// over `max_bytes` of encoded values (a first entry larger than that is still answered, so a
	/// reader is never stuck), or once `max_steps` slots have been visited, holes and expired
	/// entries counting as entries do; either way `more` is set and `through` is the last slot
	/// visited, so the next page goes on from there.  Holes are skipped.  An entry whose `exp_ms` is at or before `now_ms`
	/// reads absent and is deleted, for every reader, so `now_ms` must be the gateway's receipt
	/// clock, the one `exp_ms` was set on, never a device's.  A cursor below the floor answers from
	/// the floor with `gap` set: the entries between were trimmed and the reader must re-base.
	pub fn read_after(
		&self,
		after:		u64,
		max_bytes:	usize,
		max_steps:	usize,
		now_ms:		u64,
	)
		-> Outcome<Page>
	{
		self.read_after_measured(after, max_bytes, max_steps, now_ms, |v| v.byte_len())
	}

	/// As [`read_after`](Self::read_after), with each entry's size taken from `measure` rather
	/// than its encoded length, so a caller can page by what it will send (a wire form) rather
	/// than by what is stored.  `None` from `measure` is an error.
	pub fn read_after_measured<F: Fn(&Dat) -> Option<usize>>(
		&self,
		after:		u64,
		max_bytes:	usize,
		max_steps:	usize,
		now_ms:		u64,
		measure:	F,
	)
		-> Outcome<Page>
	{
		let (head, floor) = res!(self.meta());
		let mut through = after.max(floor).min(head);
		let mut entries = Vec::new();
		let mut used = 0usize;
		let mut more = false;
		let mut steps = 0usize;
		let mut seq = through;
		while seq < head {
			if steps >= max_steps {
				more = true;
				break;
			}
			steps += 1;
			seq += 1;
			let (exp_ms, id, val) = match res!(self.entry_at(seq)) {
				Some(e)	=> e,
				None	=> {
					through = seq;
					continue;
				},
			};
			if let Some(exp) = exp_ms {
				if exp <= now_ms {
					res!(self.drop_entry(seq, id.as_deref()));
					through = seq;
					continue;
				}
			}
			let size = match measure(&val) {
				Some(n)	=> n,
				None	=> return Err(err!(
					"lane {}: entry {} cannot be measured.", self.prefix, seq;
				Invalid, Input)),
			};
			if !entries.is_empty() && used.saturating_add(size) > max_bytes {
				more = true;
				break;
			}
			used = used.saturating_add(size);
			entries.push(Entry { seq, exp_ms, val });
			through = seq;
		}
		Ok(Page { entries, through, more, floor, gap: after < floor })
	}

	/// Deletes the entry at `seq`, leaving a hole reads skip.  Answers whether there was one.
	pub fn remove(&self, seq: u64) -> Outcome<bool> {
		match res!(self.entry_at(seq)) {
			Some((_, id, _))	=> {
				res!(self.drop_entry(seq, id.as_deref()));
				Ok(true)
			},
			None				=> Ok(false),
		}
	}

	/// Deletes every entry up to `seq` (never above the head) and raises the floor there.  Never
	/// lowers the floor.  Answers the floor.
	pub fn trim_through(&self, seq: u64) -> Outcome<u64> {
		let (head, floor) = res!(self.meta());
		let to = seq.min(head);
		if to <= floor {
			return Ok(floor);
		}
		for s in (floor + 1)..=to {
			if let Some((_, id, _)) = res!(self.entry_at(s)) {
				res!(self.drop_entry(s, id.as_deref()));
			}
		}
		res!(self.put_meta(head, to));
		Ok(to)
	}

	/// Records that `name` has read through `cursor` (held at the head), and marks it live.  A
	/// cursor may move back, for a reader starting over.  Writes only when the cursor or the state
	/// changes, so a reader acknowledging where it already stands costs no write.  Answers the
	/// cursor kept.
	pub fn ack(&self, name: &str, cursor: u64) -> Outcome<u64> {
		let head = res!(self.head());
		let cursor = cursor.min(head);
		let mut rs = res!(self.readers());
		match rs.iter_mut().find(|r| r.name == name) {
			Some(r)	=> {
				if r.cursor == cursor && r.state == ReaderState::Live {
					return Ok(cursor);
				}
				r.cursor = cursor;
				r.state = ReaderState::Live;
			},
			None	=> rs.push(Reader {
				name:	name.to_string(),
				cursor,
				state:	ReaderState::Live,
			}),
		}
		res!(self.put_readers(&rs));
		Ok(cursor)
	}

	/// Takes `name` out of `stable` until it acknowledges again, keeping its cursor.  Answers
	/// whether the lane knew it.
	pub fn set_dormant(&self, name: &str) -> Outcome<bool> {
		let mut rs = res!(self.readers());
		let found = match rs.iter_mut().find(|r| r.name == name) {
			Some(r)	=> {
				r.state = ReaderState::Dormant;
				true
			},
			None	=> false,
		};
		if found {
			res!(self.put_readers(&rs));
		}
		Ok(found)
	}

	/// Forgets `name` and its cursor.  Answers whether the lane knew it.
	pub fn forget(&self, name: &str) -> Outcome<bool> {
		let mut rs = res!(self.readers());
		let n = rs.len();
		rs.retain(|r| r.name != name);
		if rs.len() == n {
			return Ok(false);
		}
		res!(self.put_readers(&rs));
		Ok(true)
	}

	/// Every reader the lane knows, live and dormant, in name order.
	pub fn readers(&self) -> Outcome<Vec<Reader>> {
		let m = match res!(self.db.get(&self.readers_key(), None)) {
			Some((Dat::Map(m), _))	=> m,
			Some((d, _))			=> return Err(err!(
				"lane {}: the reader table is a {:?}, not a map.", self.prefix, d.kind();
			Invalid, Input, Mismatch)),
			None					=> return Ok(Vec::new()),
		};
		let mut rs = Vec::with_capacity(m.len());
		for (k, v) in m.iter() {
			let name = match k {
				Dat::Str(s)	=> s.clone(),
				_			=> return Err(err!(
					"lane {}: a reader is named by a {:?}, not a string.", self.prefix, k.kind();
				Invalid, Input, Mismatch)),
			};
			let (cursor, live) = match v {
				Dat::List(l) if l.len() == 2 => match (number(&l[0]), &l[1]) {
					(Some(c), Dat::Bool(b))	=> (c, *b),
					_						=> return Err(err!(
						"lane {}: reader '{}' holds {:?}.", self.prefix, name, v;
					Invalid, Input, Mismatch)),
				},
				_ => return Err(err!(
					"lane {}: reader '{}' holds {:?}.", self.prefix, name, v;
				Invalid, Input, Mismatch)),
			};
			rs.push(Reader {
				name,
				cursor,
				state: if live { ReaderState::Live } else { ReaderState::Dormant },
			});
		}
		rs.sort_by(|a, b| a.name.cmp(&b.name));
		Ok(rs)
	}

	/// The least cursor over live readers: every live reader has read through it.  The head when no
	/// reader is live.  Never above the head.
	pub fn stable(&self) -> Outcome<u64> {
		let head = res!(self.head());
		let least = res!(self.readers()).iter()
			.filter(|r| r.state == ReaderState::Live)
			.map(|r| r.cursor)
			.min();
		Ok(least.unwrap_or(head).min(head))
	}

	// ── Keys and stored forms ──────────────────────────────────────────────────

	fn meta_key(&self) -> Dat			{ dat!(fmt!("{}/m", self.prefix)) }
	fn readers_key(&self) -> Dat		{ dat!(fmt!("{}/r", self.prefix)) }
	fn entry_key(&self, seq: u64) -> Dat	{ dat!(fmt!("{}/e/{}", self.prefix, seq)) }

	fn id_key(&self, id: &[u8]) -> Dat {
		let mut s = String::with_capacity(self.prefix.len() + 3 + 2 * id.len());
		s.push_str(&self.prefix);
		s.push_str("/i/");
		for b in id {
			s.push_str(&fmt!("{:02x}", b));
		}
		dat!(s)
	}

	fn meta(&self) -> Outcome<(u64, u64)> {
		match res!(self.db.get(&self.meta_key(), None)) {
			None => Ok((0, 0)),
			Some((Dat::List(l), _)) if l.len() == 2 => match (number(&l[0]), number(&l[1])) {
				(Some(h), Some(f)) if f <= h => Ok((h, f)),
				_ => Err(err!("lane {}: the head record holds {:?}.", self.prefix, l;
					Invalid, Input, Mismatch)),
			},
			Some((d, _)) => Err(err!("lane {}: the head record holds {:?}.", self.prefix, d;
				Invalid, Input, Mismatch)),
		}
	}

	fn put_meta(&self, head: u64, floor: u64) -> Outcome<()> {
		let v = Dat::List(vec![Dat::U64(head), Dat::U64(floor)]);
		res!(self.db.insert(self.meta_key(), v, self.user, None));
		Ok(())
	}

	fn put_readers(&self, rs: &[Reader]) -> Outcome<()> {
		let mut m = DaticleMap::new();
		for r in rs {
			m.insert(
				Dat::Str(r.name.clone()),
				Dat::List(vec![Dat::U64(r.cursor), Dat::Bool(r.state == ReaderState::Live)]),
			);
		}
		res!(self.db.insert(self.readers_key(), Dat::Map(m), self.user, None));
		Ok(())
	}

	fn id_seq(&self, id: &[u8]) -> Outcome<Option<u64>> {
		match res!(self.db.get(&self.id_key(id), None)) {
			None			=> Ok(None),
			Some((d, _))	=> match number(&d) {
				Some(n)	=> Ok(Some(n)),
				None	=> Err(err!("lane {}: an id record holds {:?}.", self.prefix, d;
					Invalid, Input, Mismatch)),
			},
		}
	}

	// The stored entry at `seq` as (expiry, id, value), or `None` for a hole.
	fn entry_at(&self, seq: u64) -> Outcome<Option<(Option<u64>, Option<Vec<u8>>, Dat)>> {
		let l = match res!(self.db.get(&self.entry_key(seq), None)) {
			None						=> return Ok(None),
			Some((Dat::List(l), _))	if l.len() == 3 => l,
			Some((d, _))				=> return Err(err!(
				"lane {}: entry {} holds {:?}.", self.prefix, seq, d.kind();
			Invalid, Input, Mismatch)),
		};
		let mut it = l.into_iter();
		let (exp, id, val) = match (it.next(), it.next(), it.next()) {
			(Some(e), Some(i), Some(v))	=> (e, i, v),
			_							=> return Err(err!(
				"lane {}: entry {} is short.", self.prefix, seq;
			Invalid, Input, Mismatch)),
		};
		let exp = match number(&exp) {
			Some(0)	=> None,
			Some(n)	=> Some(n),
			None	=> return Err(err!("lane {}: entry {} has expiry {:?}.", self.prefix, seq, exp;
				Invalid, Input, Mismatch)),
		};
		let id = match id {
			Dat::Empty		=> None,
			Dat::BU8(b)		|
			Dat::BU16(b)	|
			Dat::BU32(b)	|
			Dat::BU64(b)	|
			Dat::BC64(b)	=> Some(b),
			d				=> return Err(err!("lane {}: entry {} has id {:?}.", self.prefix, seq, d;
				Invalid, Input, Mismatch)),
		};
		Ok(Some((exp, id, val)))
	}

	fn drop_entry(&self, seq: u64, id: Option<&[u8]>) -> Outcome<()> {
		if let Some(id) = id {
			// Only when it still names this entry; a later append may have taken the id.
			if res!(self.id_seq(id)) == Some(seq) {
				res!(self.db.delete(&self.id_key(id), self.user, None));
			}
		}
		res!(self.db.delete(&self.entry_key(seq), self.user, None));
		Ok(())
	}
}

// A stored count, however narrowly the decoder typed it.
fn number(d: &Dat) -> Option<u64> {
	match d {
		Dat::U64(x)	=> Some(*x),
		Dat::U32(x)	=> Some(*x as u64),
		Dat::U16(x)	=> Some(*x as u64),
		Dat::U8(x)	=> Some(*x as u64),
		Dat::C64(x)	=> Some(*x),
		_			=> None,
	}
}
