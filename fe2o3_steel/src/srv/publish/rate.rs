//! A windowed counter for the acts a stranger can repeat.
//!
//! One row of three numbers, `[last, count, window_start]`, under a key the caller chooses. It limits
//! two things at once: how soon after the last act the next may come (`interval_secs`), and how many
//! may land within a span (`max` in `span_secs`). The comment limiter, the sign-up limiter and the
//! per-recipient confirmation limit are all this counter with different keys and windows.
//!
//! The row is read, changed and written back under [`store::update`](super::store::update)'s write
//! guard. With only a read guard, thirty-two requests arriving together each read the same count and
//! every one of them was let through.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use crate::srv::publish::store::{
	self,
	Edit,
};

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_iop_crypto::enc::Encrypter;
use oxedyne_fe2o3_iop_db::api::{
	Database,
	ScanOpts,
};
use oxedyne_fe2o3_iop_hash::api::Hasher;
use oxedyne_fe2o3_jdat::{
	prelude::*,
	id::NumIdDat,
};

use std::sync::{
	Arc,
	RwLock,
};


// The span the comment and sign-up limiters have always counted over.
pub const HOUR_SECS: u64 = 3600;

// Rows judged under one write guard when expiring, so a sweep never holds the writers for long.
const EXPIRE_CHUNK: usize = 256;

/// How often one key may act. A bound of 0 is that bound switched off.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Window {
	pub interval_secs:	u64,	// the shortest gap between two acts
	pub max:		u32,	// the most acts within one span
	pub span_secs:		u64,	// how long a count runs before it starts again
}

impl Window {

	/// A window that counts over an hour, as the comment and sign-up limiters do.
	pub fn hourly(interval_secs: u64, max: u32) -> Self {
		Self { interval_secs, max, span_secs: HOUR_SECS }
	}

	/// Are both bounds off, so that the key may always act and nothing need be read?
	pub fn is_off(&self) -> bool {
		self.interval_secs == 0 && self.max == 0
	}
}

/// Seconds since the epoch, or 0 where the clock is before it.
pub fn now_secs() -> u64 {
	std::time::SystemTime::now()
		.duration_since(std::time::UNIX_EPOCH)
		.map(|d| d.as_secs())
		.unwrap_or(0)
}

/// Whether the key may act now, and the record of its having done so.
///
/// When both bounds are off nothing is read and nothing is written. A refusal writes nothing either,
/// so a flood of refused requests does not become a flood of writes.
pub fn allow<
	const UIDL: usize,
	UID:	NumIdDat<UIDL>,
	ENC:	Encrypter,
	KH:	Hasher,
	DB:	Database<UIDL, UID, ENC, KH>,
>(
	db:	&(Arc<RwLock<DB>>, UID),
	key:	&str,
	w:	&Window,
)
	-> Outcome<bool>
{
	allow_at(db, key, w, now_secs())
}

/// As [`allow`], at a time the caller names.
pub fn allow_at<
	const UIDL: usize,
	UID:	NumIdDat<UIDL>,
	ENC:	Encrypter,
	KH:	Hasher,
	DB:	Database<UIDL, UID, ENC, KH>,
>(
	db:	&(Arc<RwLock<DB>>, UID),
	key:	&str,
	w:	&Window,
	now:	u64,
)
	-> Outcome<bool>
{
	if w.is_off() {
		return Ok(true);
	}
	store::exclusive(db, |dbr, user| allow_in(dbr, user, key, w, now))
}

/// As [`allow_at`], on a database already write-locked by [`store::exclusive`].
///
/// For a caller that must count and change something else in the same step, as the per-recipient
/// confirmation limit does beside the subscriber record. Inside the guard it must be this and never
/// [`allow`]: a std `RwLock` is not re-entrant, and `allow` would wait on the guard its own caller
/// holds.
pub fn allow_in<
	const UIDL: usize,
	UID:	NumIdDat<UIDL>,
	ENC:	Encrypter,
	KH:	Hasher,
	DB:	Database<UIDL, UID, ENC, KH>,
>(
	dbr:	&DB,
	user:	UID,
	key:	&str,
	w:	&Window,
	now:	u64,
)
	-> Outcome<bool>
{
	if w.is_off() {
		return Ok(true);
	}
	store::edit_in(dbr, user, &dat!(key.to_string()), |old| -> Outcome<(Edit, bool)> {
		Ok(step(old, w, now))
	})
}

/// Would one more act be allowed at `now`? Reads the row and writes nothing, on a database already locked.
///
/// For a caller that counts only when the act has happened, as the per-recipient limit does: the
/// check comes before the act and [`allow_at`] counts it after.
pub fn permits_in<
	const UIDL: usize,
	UID:	NumIdDat<UIDL>,
	ENC:	Encrypter,
	KH:	Hasher,
	DB:	Database<UIDL, UID, ENC, KH>,
>(
	dbr:	&DB,
	key:	&str,
	w:	&Window,
	now:	u64,
)
	-> Outcome<bool>
{
	if w.is_off() {
		return Ok(true);
	}
	let old = match res!(dbr.get(&dat!(key.to_string()), None)) {
		Some((v, _))	=> Some(v),
		None		=> None,
	};
	Ok(step(old, w, now).1)
}

/// The count a stored row holds, and what one more act at `now` does to it.
///
/// A new window resets the count. The window runs from the act that opened it, not rolling: a rolling
/// window needs every timestamp kept, and this needs three numbers. A row of any other shape reads as
/// no row, so a store holding something else at the key starts the count afresh.
pub fn step(old: Option<Dat>, w: &Window, now: u64) -> (Edit, bool) {
	let (last, count, start) = read_row(&old);
	let (count, start) = if now.saturating_sub(start) >= w.span_secs {
		(0, now)
	} else {
		(count, start)
	};
	if (w.interval_secs > 0 && now.saturating_sub(last) < w.interval_secs)
		|| (w.max > 0 && count >= w.max)
	{
		return (Edit::Keep, false);
	}
	let row = Dat::List(vec![dat!(now), dat!((count as u64) + 1), dat!(start)]);
	(Edit::Set(row), true)
}


// The three numbers a stored row holds: the last act, the count, and where the window began. A row
// of any other shape, or none, reads as all zeroes.
fn read_row(old: &Option<Dat>) -> (u64, u32, u64) {
	match old {
		Some(Dat::List(v)) if v.len() == 3	=> {
			let n = |i: usize| match v.get(i) {
				Some(Dat::U64(x))	=> *x,
				Some(Dat::U32(x))	=> *x as u64,
				Some(Dat::U16(x))	=> *x as u64,
				Some(Dat::U8(x))	=> *x as u64,
				_			=> 0,
			};
			(n(0), n(1).min(u32::MAX as u64) as u32, n(2))
		},
		_					=> (0, 0, 0),
	}
}

/// Has a stored row outlived its window, so that it no longer limits anything?
///
/// It has once the window it counted in and the interval after the last act have both passed, which
/// is a time `span_secs + interval_secs` after the later of the last act and the window's start. Past
/// that, [`step`] would open a new window and count from nothing, so deleting the row changes no
/// answer. A row of any other shape is spent already, since `step` reads it as no row at all.
pub fn spent(old: &Option<Dat>, w: &Window, now: u64) -> bool {
	let (last, _, start) = read_row(old);
	now.saturating_sub(last.max(start)) >= w.span_secs.saturating_add(w.interval_secs)
}

/// Deletes every row under the given prefixes that has outlived that prefix's window, and says how
/// many went.
///
/// The keys are found by one `scan`, for the longest prefix the sets share, and that scan **walks
/// every index file of the store**: what it costs is the size of the store however few rows match,
/// and the database holds it to the request deadline rather than letting it stall. It is for a
/// background task, never a request, and it is made once for all the sets so a store is not walked
/// for each. The scan holds the read guard and no more; each row is then judged again under the
/// write guard, in chunks, so a row a request has counted since the scan is kept, and no write waits
/// on the whole sweep. A scan that cannot finish in time is an error for the caller to log and try
/// again, and nothing is lost by it: a spent row only occupies space.
pub fn expire_rows<
	const UIDL: usize,
	UID:	NumIdDat<UIDL>,
	ENC:	Encrypter,
	KH:	Hasher,
	DB:	Database<UIDL, UID, ENC, KH>,
>(
	db:	&(Arc<RwLock<DB>>, UID),
	sets:	&[(&str, Window)],
	now:	u64,
)
	-> Outcome<usize>
{
	let (db_arc, _) = db;
	let mut shared: Option<&str> = None;
	for (prefix, _) in sets {
		shared = Some(match shared {
			None		=> *prefix,
			Some(sofar)	=> {
				let n: usize = sofar.chars().zip(prefix.chars()).take_while(|(a, b)| a == b)
					.map(|(a, _)| a.len_utf8()).sum();
				&sofar[..n]
			},
		});
	}
	let shared = match shared {
		Some(p)	=> p,
		None	=> return Ok(0),
	};
	let found = {
		let guard = lock_read!(db_arc);
		res!(guard.scan(&ScanOpts::with_str_prefix(shared), None))
	};
	let mut rows: Vec<(Dat, Window)> = Vec::new();
	for (key, _, _) in found {
		let w = match &key {
			Dat::Str(k)	=> sets.iter().find(|(p, _)| k.starts_with(p)).map(|(_, w)| *w),
			_		=> None,
		};
		if let Some(w) = w {
			rows.push((key, w));
		}
	}
	let mut gone = 0;
	for chunk in rows.chunks(EXPIRE_CHUNK) {
		gone += res!(store::exclusive(db, |dbr, user| -> Outcome<usize> {
			let mut n = 0;
			for (key, w) in chunk {
				// A key the scan listed that the database reads as absent is a deleted row: nothing to
				// delete, and `edit_in` leaves it alone.
				let cleared = res!(store::edit_in(dbr, user, key, |old| -> Outcome<(Edit, bool)> {
					if old.is_some() && spent(&old, w, now) {
						return Ok((Edit::Clear, true));
					}
					Ok((Edit::Keep, false))
				}));
				if cleared {
					n += 1;
				}
			}
			Ok(n)
		}));
	}
	Ok(gone)
}

#[cfg(test)]
mod tests {
	use super::*;

	const T0: u64 = 1_800_000_000;

	// The row a step wrote, as the three numbers it holds.
	fn row_of(e: &Edit) -> Option<Dat> {
		match e {
			Edit::Set(d)	=> Some(d.clone()),
			_		=> None,
		}
	}

	/// Acts up to the maximum are allowed within a span, and the next is refused with no write.
	#[test]
	fn test_a_count_stops_at_the_maximum_00() -> Outcome<()> {
		let w = Window { interval_secs: 0, max: 3, span_secs: 100 };
		let mut row = None;
		for i in 0..3u64 {
			let (e, ok) = step(row, &w, T0 + i);
			assert!(ok, "act {} was refused", i);
			row = row_of(&e);
			assert!(row.is_some());
		}
		let (e, ok) = step(row.clone(), &w, T0 + 10);
		assert!(!ok, "the fourth act was allowed");
		assert!(matches!(e, Edit::Keep), "a refusal wrote");
		Ok(())
	}

	/// The span is the caller's: a count over a day is still full after an hour, and fresh after a day.
	#[test]
	fn test_the_span_is_a_parameter_01() -> Outcome<()> {
		let day = Window { interval_secs: 0, max: 1, span_secs: 86_400 };
		let hour = Window::hourly(0, 1);
		let (e, ok) = step(None, &day, T0);
		assert!(ok);
		let row = row_of(&e);
		// An hour and a second on: the hourly window has turned over and the daily one has not.
		assert!(!step(row.clone(), &day, T0 + 3_601).1, "a day's count was reset after an hour");
		let (e, ok) = step(None, &hour, T0);
		assert!(ok);
		assert!(step(row_of(&e), &hour, T0 + 3_601).1, "an hour's count was not reset");
		// And the daily one turns over after the day.
		assert!(step(row, &day, T0 + 86_400).1, "a day's count was not reset after a day");
		Ok(())
	}

	/// The interval refuses an act that follows the last too soon, whatever the count.
	#[test]
	fn test_the_interval_refuses_a_quick_repeat_02() -> Outcome<()> {
		let w = Window::hourly(60, 0);
		let (e, ok) = step(None, &w, 5_000);
		assert!(ok);
		let row = row_of(&e);
		assert!(!step(row.clone(), &w, 5_030).1);
		assert!(step(row, &w, 5_060).1);
		Ok(())
	}

	/// A row written before this module existed reads the same: three unsigned numbers.
	#[test]
	fn test_an_existing_row_still_reads_03() -> Outcome<()> {
		let old = Dat::List(vec![dat!(1_000u64), dat!(5u64), dat!(900u64)]);
		let w = Window::hourly(0, 5);
		let (e, ok) = step(Some(old), &w, 1_100);
		assert!(!ok, "a full count was forgotten");
		assert!(matches!(e, Edit::Keep));
		// A row of some other shape is no row.
		let (_, ok) = step(Some(dat!("junk")), &w, 1_100);
		assert!(ok);
		Ok(())
	}
}
