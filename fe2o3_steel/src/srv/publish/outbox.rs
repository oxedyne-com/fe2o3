//! The paced outbound queue: what a site sends, held in its own store and let out at a set rate.
//!
//! A sign-up used to wait on SMTP inside the request, so the pace of a site's mail was whatever the
//! visitors made it, and a script could spend the host's sending reputation as fast as it could post
//! a form. Now the request writes a queue entry and answers. A drainer task takes entries off the
//! queue and sends them at no more than the host's hourly ceiling, and a ceiling of 0 sends nothing
//! at all, which is how a site is moved live with its mail tap shut.
//!
//! # Where the queue lives
//!
//! In the vhost's own Ozone store, one lane for each [`Kind`]. A lane is a head counter, a tail
//! counter and an entry at each sequence number between them, so the queue survives a restart and
//! holds its place. Head and tail move only inside [`store::exclusive`]. **No token is stored in an
//! entry.** At send time the drainer reads the subscriber afresh and sends to nobody who has left
//! the state the entry was queued for, so the link a message carries is always the current one.
//!
//! # Where the ceiling lives
//!
//! In one [`Pacer`] shared by every drainer on the host, because the address and the domain carry
//! one reputation, not one for each site. The pacer spaces sends evenly (no bursts), and it is held
//! in memory, so a restart can add at most one send to the schedule.
//!
//! # The two lanes, and the alert
//!
//! A confirmation waits in the `c` lane and the newsletter in the `n` lane, and the drainer empties
//! `c` first, so a sign-up is never kept behind a newsletter. A [`Watch`] reads the queue after each
//! pass and tells the operator once when the oldest entry has waited past `outbox_alert_secs`, which a
//! held ceiling and a deep queue both cause, and once more when the queue has drained.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use crate::srv::{
	alert::{
		AlertEvent,
		Alerter,
	},
	publish::{
		PostState,
		PublishConfig,
		send::{
			self,
			Fate,
		},
		store::{
			self,
			Edit,
		},
		subscribe::{
			self,
			SubState,
		},
	},
};

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_iop_crypto::enc::Encrypter;
use oxedyne_fe2o3_iop_db::api::Database;
use oxedyne_fe2o3_iop_hash::api::Hasher;
use oxedyne_fe2o3_jdat::{
	prelude::*,
	id::NumIdDat,
};
use oxedyne_fe2o3_mail::outbound::retry_after;
use oxedyne_fe2o3_net::smtp::client::is_permanent;

use std::{
	future::Future,
	sync::{
		Arc,
		Mutex,
		RwLock,
	},
	time::Duration,
};

use tokio::sync::Notify;


pub const PREFIX: &str = "publish/outbox/";

// The ceiling a mail block takes where it names none. Chosen to be slow enough that a young
// domain is never mistaken for a spammer, and fast enough that one confirmation is never kept
// waiting more than a minute.
pub const HOURLY_DEFAULT: u32 = 100;

// How many times an entry is tried before it is given up and dropped.
pub const TRIES_MAX: u32 = 5;

// The longest the drainer sleeps with nothing to wake it, in seconds. The queue is also polled at
// this interval, so an entry written by another process is found within it.
pub const POLL_SECS: u64 = 30;

// Entries written under one write guard. A newsletter to thousands is pushed in chunks so that
// reads on the same store are not kept waiting for the whole run.
const PUSH_CHUNK: usize = 64;

const HOUR_MS: u64 = 3_600_000;

// The lanes, in the order they are drained.
const KINDS: [Kind; 2] = [Kind::Confirm, Kind::News];

const DAY_SECS: u64 = 86_400;

// How many places from the head of a lane to look for an entry that can be read, when the head
// itself cannot be. A queue is not walked from end to end to find its age.
const HEAD_LOOK: u64 = 16;


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ PACER                                                                     │
// └───────────────────────────────────────────────────────────────────────────┘

/// What the pacer says about sending now.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Pace {
	Held,		// the ceiling is 0: nothing leaves
	Wait(u64),	// the next slot is this many milliseconds away
	Go,		// this send holds the slot
}

/// The host's outbound ceiling, shared by every drainer.
///
/// Sends are spaced evenly: the gap is an hour divided by the ceiling, rounded up, so the ceiling
/// is never exceeded in any hour. The next slot is held in memory only.
#[derive(Debug)]
pub struct Pacer {
	hourly:	u32,		// sends an hour; 0 holds everything
	next:	Mutex<u64>,	// the earliest millisecond the next send may leave
	wake:	Notify,		// signalled when something is queued
}

impl Pacer {

	pub fn new(hourly: u32) -> Self {
		Self { hourly, next: Mutex::new(0), wake: Notify::new() }
	}

	pub fn hourly(&self) -> u32 {
		self.hourly
	}

	/// Is the ceiling 0, so that nothing leaves?
	pub fn is_held(&self) -> bool {
		self.hourly == 0
	}

	/// The milliseconds between two sends at this ceiling; 0 where the ceiling is 0.
	pub fn gap_ms(&self) -> u64 {
		if self.hourly == 0 {
			return 0;
		}
		let h = self.hourly as u64;
		(HOUR_MS + h - 1) / h
	}

	/// Asks for the slot at `now_ms`. A `Go` has taken it: whatever the send does, the next one
	/// waits a gap. The slot is taken before the send, so a send that fails still counts.
	pub fn claim(&self, now_ms: u64) -> Outcome<Pace> {
		if self.hourly == 0 {
			return Ok(Pace::Held);
		}
		let mut next = lock_mutex!(self.next);
		if now_ms < *next {
			return Ok(Pace::Wait(*next - now_ms));
		}
		*next = now_ms.saturating_add(self.gap_ms());
		Ok(Pace::Go)
	}

	/// Wakes every drainer that is idle, after something has been queued.
	pub fn wake(&self) {
		self.wake.notify_waiters();
	}
}

/// The unix millisecond now, or 0 where the clock is before the epoch.
pub fn now_ms() -> u64 {
	std::time::SystemTime::now()
		.duration_since(std::time::UNIX_EPOCH)
		.map(|d| d.as_millis() as u64)
		.unwrap_or(0)
}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ COURIER                                                                   │
// └───────────────────────────────────────────────────────────────────────────┘

/// What the drainer sends through: the host's signing sender in service, and a recorder in a test.
pub trait Courier {

	/// The From address a site that names none sends from.
	fn default_from(&self) -> &str;

	/// Delivers one finished message to one recipient and answers the remote's queue id.
	fn deliver(&self, from: &str, to: &str, msg: &str)
		-> impl Future<Output = Outcome<String>> + Send;
}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ ENTRY                                                                     │
// └───────────────────────────────────────────────────────────────────────────┘

/// The lane an entry waits in.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Kind {
	Confirm,	// a double opt-in confirmation, drained first
	News,		// one subscriber's copy of a post
}

impl Kind {

	pub fn as_str(&self) -> &'static str {
		match self {
			Self::Confirm	=> "confirm",
			Self::News	=> "news",
		}
	}

	pub fn of(s: &str) -> Option<Self> {
		match s {
			"confirm"	=> Some(Self::Confirm),
			"news"		=> Some(Self::News),
			_		=> None,
		}
	}

	fn lane(&self) -> &'static str {
		match self {
			Self::Confirm	=> "c",
			Self::News	=> "n",
		}
	}
}

/// One queued send. It names who and what, never a token.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Entry {
	pub kind:	Kind,
	pub email:	String,		// normalised
	pub slug:	String,		// the post, for a newsletter entry; empty otherwise
	pub send:	u64,		// the place of the send in the history, for a newsletter entry; 0 otherwise
	pub enqueued:	u64,		// unix seconds the entry was first queued, kept through retries
	pub tries:	u32,		// attempts that have failed
	pub next_try:	u64,		// unix seconds before which the entry is not tried
}

impl Entry {

	/// A fresh entry, due at once.
	pub fn new(kind: Kind, email: &str, slug: &str, now: u64) -> Self {
		Self {
			kind,
			email:		email.to_string(),
			slug:		slug.to_string(),
			send:		0,
			enqueued:	now,
			tries:		0,
			next_try:	now,
		}
	}

	/// This entry, counted on the history entry at `index`.
	pub fn for_send(mut self, index: u64) -> Self {
		self.send = index;
		self
	}

	pub fn to_dat(&self) -> Dat {
		let mut m = DaticleMap::new();
		m.insert(dat!("kind"),		dat!(self.kind.as_str().to_string()));
		m.insert(dat!("email"),		dat!(self.email.clone()));
		if !self.slug.is_empty() {
			m.insert(dat!("slug"),	dat!(self.slug.clone()));
			m.insert(dat!("send"),	dat!(self.send));
		}
		m.insert(dat!("enqueued"),	dat!(self.enqueued));
		m.insert(dat!("tries"),		dat!(self.tries as u64));
		m.insert(dat!("next_try"),	dat!(self.next_try));
		Dat::Map(m)
	}

	pub fn from_dat(d: &Dat) -> Outcome<Self> {
		let m = match d {
			Dat::Map(m)	=> m,
			_		=> return Err(err!(
				"publish: an outbox entry must be a map, not {:?}.", d.kind();
				Invalid, Input, Mismatch)),
		};
		let text = |key: &str| -> String {
			match m.get(&dat!(key)) {
				Some(Dat::Str(s))	=> s.clone(),
				_			=> String::new(),
			}
		};
		let kind = res!(Kind::of(&text("kind")).ok_or_else(|| err!(
			"publish: an outbox entry names the kind '{}', which is not one.", text("kind");
			Invalid, Input, Mismatch)));
		let email = text("email");
		if email.is_empty() {
			return Err(err!("publish: an outbox entry names no email."; Invalid, Input, Missing));
		}
		Ok(Self {
			kind,
			email,
			slug:		text("slug"),
			send:		number(m.get(&dat!("send"))),
			enqueued:	number(m.get(&dat!("enqueued"))),
			tries:		number(m.get(&dat!("tries"))).min(u32::MAX as u64) as u32,
			next_try:	number(m.get(&dat!("next_try"))),
		})
	}
}

// A stored count, however narrowly the decoder happened to type it. An absent or odd value is 0.
fn number(d: Option<&Dat>) -> u64 {
	match d {
		Some(Dat::U64(x))	=> *x,
		Some(Dat::U32(x))	=> *x as u64,
		Some(Dat::U16(x))	=> *x as u64,
		Some(Dat::U8(x))	=> *x as u64,
		_			=> 0,
	}
}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ QUEUE                                                                     │
// └───────────────────────────────────────────────────────────────────────────┘

fn head_key(kind: Kind) -> Dat {
	dat!(fmt!("{}{}/head", PREFIX, kind.lane()))
}

fn tail_key(kind: Kind) -> Dat {
	dat!(fmt!("{}{}/tail", PREFIX, kind.lane()))
}

fn entry_key(kind: Kind, seq: u64) -> Dat {
	dat!(fmt!("{}{}/{}", PREFIX, kind.lane(), seq))
}

// The counter at a key, on a database already locked. An absent one is 0.
fn counter_in<
	const UIDL: usize,
	UID:	NumIdDat<UIDL>,
	ENC:	Encrypter,
	KH:	Hasher,
	DB:	Database<UIDL, UID, ENC, KH>,
>(
	dbr:	&DB,
	key:	&Dat,
)
	-> Outcome<u64>
{
	match res!(dbr.get(key, None)) {
		Some((v, _))	=> Ok(number(Some(&v))),
		None		=> Ok(0),
	}
}

// The entry at a sequence number, on a database already locked. A key never written, or deleted, is
// `None`.
fn entry_in<
	const UIDL: usize,
	UID:	NumIdDat<UIDL>,
	ENC:	Encrypter,
	KH:	Hasher,
	DB:	Database<UIDL, UID, ENC, KH>,
>(
	dbr:	&DB,
	kind:	Kind,
	seq:	u64,
)
	-> Outcome<Option<Entry>>
{
	match res!(dbr.get(&entry_key(kind, seq), None)) {
		Some((v, _))	=> Ok(Some(res!(Entry::from_dat(&v)))),
		None		=> Ok(None),
	}
}

// Writes an entry at the tail and moves the tail past it, on a database already write-locked. The
// entry is written first, so a stop between the two leaves an entry nobody names and not a name
// with no entry.
fn push_in<
	const UIDL: usize,
	UID:	NumIdDat<UIDL>,
	ENC:	Encrypter,
	KH:	Hasher,
	DB:	Database<UIDL, UID, ENC, KH>,
>(
	dbr:	&DB,
	user:	UID,
	e:	&Entry,
)
	-> Outcome<()>
{
	store::edit_in(dbr, user, &tail_key(e.kind), |old| -> Outcome<(Edit, ())> {
		let t = number(old.as_ref());
		res!(dbr.insert(entry_key(e.kind, t), e.to_dat(), user, None));
		Ok((Edit::Set(dat!(t + 1)), ()))
	})
}

// Deletes the entry at `seq` and, where it is the head, moves the head past it.
fn pop_in<
	const UIDL: usize,
	UID:	NumIdDat<UIDL>,
	ENC:	Encrypter,
	KH:	Hasher,
	DB:	Database<UIDL, UID, ENC, KH>,
>(
	dbr:	&DB,
	user:	UID,
	kind:	Kind,
	seq:	u64,
)
	-> Outcome<()>
{
	res!(dbr.delete(&entry_key(kind, seq), user, None));
	store::edit_in(dbr, user, &head_key(kind), |old| -> Outcome<(Edit, ())> {
		if number(old.as_ref()) == seq {
			Ok((Edit::Set(dat!(seq + 1)), ()))
		} else {
			Ok((Edit::Keep, ()))
		}
	})
}

/// Queues entries at the tail of their lanes, in order. Answers how many were queued.
pub fn push<
	const UIDL: usize,
	UID:	NumIdDat<UIDL>,
	ENC:	Encrypter,
	KH:	Hasher,
	DB:	Database<UIDL, UID, ENC, KH>,
>(
	db:		&(Arc<RwLock<DB>>, UID),
	entries:	&[Entry],
)
	-> Outcome<usize>
{
	for chunk in entries.chunks(PUSH_CHUNK) {
		res!(store::exclusive(db, |dbr, user| -> Outcome<()> {
			for e in chunk {
				res!(push_in(dbr, user, e));
			}
			Ok(())
		}));
	}
	Ok(entries.len())
}

/// How many entries wait in a lane.
pub fn queued<
	const UIDL: usize,
	UID:	NumIdDat<UIDL>,
	ENC:	Encrypter,
	KH:	Hasher,
	DB:	Database<UIDL, UID, ENC, KH>,
>(
	db:	&(Arc<RwLock<DB>>, UID),
	kind:	Kind,
)
	-> Outcome<u64>
{
	let (db_arc, _) = db;
	let guard = lock_read!(db_arc);
	let head = res!(counter_in(&*guard, &head_key(kind)));
	let tail = res!(counter_in(&*guard, &tail_key(kind)));
	Ok(tail.saturating_sub(head))
}

/// What a lane holds that can be sent now.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Take {
	Empty,			// nothing queued
	Later(u64),		// everything is backing off; the soonest is due at this unix second
	Due(u64, Entry),	// the entry at this sequence number, still queued until [`done`]
}

/// Finds the first entry in a lane that is due at `now`.
///
/// An entry backing off from a failure is moved to the tail, not waited for, so one address whose
/// mail host is down never holds up the rest. Each is moved at most once in a call. A key that
/// holds nothing, or nothing readable, is passed over, since a queue that stops at a bad entry
/// would never move again.
pub fn take<
	const UIDL: usize,
	UID:	NumIdDat<UIDL>,
	ENC:	Encrypter,
	KH:	Hasher,
	DB:	Database<UIDL, UID, ENC, KH>,
>(
	db:	&(Arc<RwLock<DB>>, UID),
	kind:	Kind,
	now:	u64,
)
	-> Outcome<Take>
{
	store::exclusive(db, |dbr, user| -> Outcome<Take> {
		let mut head = res!(counter_in(dbr, &head_key(kind)));
		let tail = res!(counter_in(dbr, &tail_key(kind)));
		let mut left = tail.saturating_sub(head);
		let mut soonest: Option<u64> = None;
		let mut out = Take::Empty;
		let start = head;
		while left > 0 {
			left -= 1;
			let e = match entry_in(dbr, kind, head) {
				Ok(Some(e))	=> e,
				Ok(None)	=> {
					head += 1;
					continue;
				}
				Err(e)		=> {
					warn!("publish: dropping an unreadable outbox entry {}/{}: {}",
						kind.lane(), head, e);
					res!(dbr.delete(&entry_key(kind, head), user, None));
					head += 1;
					continue;
				}
			};
			if e.next_try <= now {
				out = Take::Due(head, e);
				break;
			}
			soonest = Some(match soonest {
				Some(s)	=> s.min(e.next_try),
				None	=> e.next_try,
			});
			res!(push_in(dbr, user, &e));
			res!(dbr.delete(&entry_key(kind, head), user, None));
			head += 1;
		}
		if head != start {
			res!(dbr.insert(head_key(kind), dat!(head), user, None));
		}
		if let (Take::Empty, Some(s)) = (&out, soonest) {
			out = Take::Later(s);
		}
		Ok(out)
	})
}

/// Removes the entry [`take`] handed out, once it has been dealt with.
pub fn done<
	const UIDL: usize,
	UID:	NumIdDat<UIDL>,
	ENC:	Encrypter,
	KH:	Hasher,
	DB:	Database<UIDL, UID, ENC, KH>,
>(
	db:	&(Arc<RwLock<DB>>, UID),
	kind:	Kind,
	seq:	u64,
)
	-> Outcome<()>
{
	store::exclusive(db, |dbr, user| pop_in(dbr, user, kind, seq))
}

/// Queues an entry again, changed, and removes the one it replaces. The new one is written first,
/// so a stop between the two sends twice at worst and never loses a message.
pub fn retry<
	const UIDL: usize,
	UID:	NumIdDat<UIDL>,
	ENC:	Encrypter,
	KH:	Hasher,
	DB:	Database<UIDL, UID, ENC, KH>,
>(
	db:	&(Arc<RwLock<DB>>, UID),
	seq:	u64,
	next:	&Entry,
)
	-> Outcome<()>
{
	store::exclusive(db, |dbr, user| -> Outcome<()> {
		res!(push_in(dbr, user, next));
		pop_in(dbr, user, next.kind, seq)
	})
}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ DRAINER                                                                   │
// └───────────────────────────────────────────────────────────────────────────┘

/// What one pass of the drainer did.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Step {
	Sent,		// a message was accepted by the far end; call again at once
	Worked,		// an entry was skipped, refused, retried or given up; call again at once
	Wait(u64),	// the next slot is this many milliseconds away
	Later(u64),	// every entry is backing off; the soonest is this many seconds away
	Idle,		// nothing is queued
	Held,		// the ceiling is 0
}

/// Takes the next due entry, in lane order, and sends it if the pacer allows.
///
/// `now_ms` is the clock, so a test can run an hour in an instant. The pacer is asked only after the
/// entry has proved still worth sending, so a skipped entry uses no slot. Nothing here holds the
/// database lock across an `.await`, and the self-locking `subscribe` calls are made outside any
/// [`store::exclusive`].
pub async fn step<
	const UIDL: usize,
	UID:	NumIdDat<UIDL>,
	ENC:	Encrypter,
	KH:	Hasher,
	DB:	Database<UIDL, UID, ENC, KH>,
	C:	Courier,
>(
	db:	&(Arc<RwLock<DB>>, UID),
	cfg:	&PublishConfig,
	courier: &C,
	pacer:	&Pacer,
	id:	&str,
	now_ms:	u64,
)
	-> Outcome<Step>
{
	if pacer.is_held() {
		return Ok(Step::Held);
	}
	let now = now_ms / 1000;
	let mut soonest: Option<u64> = None;
	let mut found: Option<(u64, Entry)> = None;
	for kind in KINDS {
		match res!(take(db, kind, now)) {
			Take::Due(seq, e)	=> {
				found = Some((seq, e));
				break;
			}
			Take::Later(t)		=> {
				soonest = Some(soonest.map_or(t, |s| s.min(t)));
			}
			Take::Empty		=> {}
		}
	}
	let (seq, entry) = match found {
		Some(f)	=> f,
		None	=> return Ok(match soonest {
			Some(t)	=> Step::Later(t.saturating_sub(now)),
			None	=> Step::Idle,
		}),
	};
	match entry.kind {
		Kind::Confirm	=> confirm_step(db, cfg, courier, pacer, id, now_ms, seq, entry).await,
		Kind::News	=> news_step(db, cfg, courier, pacer, id, now_ms, seq, entry).await,
	}
}

async fn confirm_step<
	const UIDL: usize,
	UID:	NumIdDat<UIDL>,
	ENC:	Encrypter,
	KH:	Hasher,
	DB:	Database<UIDL, UID, ENC, KH>,
	C:	Courier,
>(
	db:	&(Arc<RwLock<DB>>, UID),
	cfg:	&PublishConfig,
	courier: &C,
	pacer:	&Pacer,
	id:	&str,
	now_ms:	u64,
	seq:	u64,
	entry:	Entry,
)
	-> Outcome<Step>
{
	let now = now_ms / 1000;
	// Re-read at send time: an address that has confirmed, left or bounced since it was queued is
	// owed nothing, and the token in the link is whatever the record holds now.
	let sub = match res!(subscribe::get(db, &entry.email)) {
		Some(s) if s.state == SubState::Pending	=> s,
		_					=> {
			debug!("{}: publish: confirmation to {} skipped, no longer pending",
				id, subscribe::redact(&entry.email));
			res!(done(db, entry.kind, seq));
			return Ok(Step::Worked);
		}
	};
	match res!(pacer.claim(now_ms)) {
		Pace::Held	=> return Ok(Step::Held),
		Pace::Wait(ms)	=> return Ok(Step::Wait(ms)),
		Pace::Go	=> {}
	}
	let from = cfg.from_or(courier.default_from());
	let url = cfg.url_of(&cfg.confirm_path(&sub.token));
	let msg = send::build_confirmation_email(&from, &sub.email, &url, &cfg.site_name);
	let mut step = Step::Worked;
	match courier.deliver(&from, &sub.email, &msg).await {
		Ok(_)	=> {
			step = Step::Sent;
			info!("{}: publish: confirmation sent to {}", id, subscribe::redact(&sub.email));
			// Gone first, so a failure after the send can never send it twice.
			res!(done(db, entry.kind, seq));
			let at = send::iso_of(now as i64).unwrap_or_default();
			if let Err(e) = subscribe::mark_sent(db, &sub.email, &at) {
				warn!("{}: publish: could not record the send to {}: {}",
					id, subscribe::redact(&sub.email), e);
			}
		}
		// A permanent failure means the address does not exist; suppress it so no later sign-up
		// mails a mailbox the remote has refused.
		Err(e) if is_permanent(&e)	=> {
			warn!("{}: publish: confirmation to {} failed permanently; suppressing: {}",
				id, subscribe::redact(&sub.email), e);
			res!(done(db, entry.kind, seq));
			if let Err(e2) = subscribe::mark_bounced(db, &sub.email, id) {
				warn!("{}: publish: could not suppress {}: {}",
					id, subscribe::redact(&sub.email), e2);
			}
		}
		Err(e)	=> {
			let tries = entry.tries + 1;
			if tries >= TRIES_MAX {
				warn!("{}: publish: confirmation to {} given up after {} tries: {}",
					id, subscribe::redact(&sub.email), tries, e);
				res!(done(db, entry.kind, seq));
			} else {
				warn!("{}: publish: confirmation to {} did not send, try {}: {}",
					id, subscribe::redact(&sub.email), tries, e);
				let next = Entry {
					tries,
					next_try:	now + retry_after(tries),
					..entry
				};
				res!(retry(db, seq, &next));
			}
		}
	}
	Ok(step)
}

// Counts how a newsletter message ended on the send it belongs to. A history that will not take the
// count does not undo the send, so it is logged and the drainer carries on.
fn tally<
	const UIDL: usize,
	UID:	NumIdDat<UIDL>,
	ENC:	Encrypter,
	KH:	Hasher,
	DB:	Database<UIDL, UID, ENC, KH>,
>(
	db:	&(Arc<RwLock<DB>>, UID),
	entry:	&Entry,
	fate:	Fate,
	id:	&str,
) {
	if let Err(e) = send::tally_send(db, entry.send as usize, fate) {
		warn!("{}: publish: could not count a newsletter message as {:?}: {}", id, fate, e);
	}
}

// A newsletter message that is no longer owed: the post is no longer live, or the subscriber has gone.
// It leaves the queue and the send set, and uses no slot.
fn left<
	const UIDL: usize,
	UID:	NumIdDat<UIDL>,
	ENC:	Encrypter,
	KH:	Hasher,
	DB:	Database<UIDL, UID, ENC, KH>,
>(
	db:	&(Arc<RwLock<DB>>, UID),
	entry:	&Entry,
	seq:	u64,
	id:	&str,
)
	-> Outcome<Step>
{
	debug!("{}: publish: newsletter '{}' to {} skipped, no longer owed",
		id, entry.slug, subscribe::redact(&entry.email));
	res!(done(db, entry.kind, seq));
	tally(db, entry, Fate::Left, id);
	Ok(Step::Worked)
}

async fn news_step<
	const UIDL: usize,
	UID:	NumIdDat<UIDL>,
	ENC:	Encrypter,
	KH:	Hasher,
	DB:	Database<UIDL, UID, ENC, KH>,
	C:	Courier,
>(
	db:	&(Arc<RwLock<DB>>, UID),
	cfg:	&PublishConfig,
	courier: &C,
	pacer:	&Pacer,
	id:	&str,
	now_ms:	u64,
	seq:	u64,
	entry:	Entry,
)
	-> Outcome<Step>
{
	let now = now_ms / 1000;
	// Re-read at send time: a post unpublished, or a subscriber who has left, since the entry was
	// queued is owed nothing, and the unsubscribe link carries the token the record holds now.
	let rec = match res!(store::get(db, &entry.slug)) {
		Some(r) if r.state == PostState::Live	=> r,
		_					=> return left(db, &entry, seq, id),
	};
	let sub = match res!(subscribe::get(db, &entry.email)) {
		Some(s) if s.state == SubState::Confirmed	=> s,
		_						=> return left(db, &entry, seq, id),
	};
	// Rendered before the slot is claimed, so a post that will not render costs no slot.
	let post = match rec.render() {
		Ok(p)	=> p,
		Err(e)	=> {
			warn!("{}: publish: newsletter '{}' will not render, so {} is not sent it: {}",
				id, entry.slug, subscribe::redact(&entry.email), e);
			res!(done(db, entry.kind, seq));
			tally(db, &entry, Fate::Failed, id);
			return Ok(Step::Worked);
		}
	};
	match res!(pacer.claim(now_ms)) {
		Pace::Held	=> return Ok(Step::Held),
		Pace::Wait(ms)	=> return Ok(Step::Wait(ms)),
		Pace::Go	=> {}
	}
	let from = cfg.from_or(courier.default_from());
	let online = cfg.url_of(&cfg.path_of(&entry.slug));
	let unsub = cfg.url_of(&cfg.unsubscribe_path(&sub.token));
	let msg = send::build_newsletter_email(&from, &sub.email, &post, &online, &unsub, &cfg.site_name);
	let mut step = Step::Worked;
	match courier.deliver(&from, &sub.email, &msg).await {
		Ok(qid)	=> {
			step = Step::Sent;
			debug!("{}: publish: newsletter '{}' to {} ({})",
				id, entry.slug, subscribe::redact(&sub.email), qid);
			// Gone first, so a failure after the send can never send it twice.
			res!(done(db, entry.kind, seq));
			tally(db, &entry, Fate::Sent, id);
		}
		// A permanent failure suppresses the address so no future send reaches it.
		Err(e) if is_permanent(&e)	=> {
			warn!("{}: publish: newsletter '{}' to {} failed permanently; suppressing: {}",
				id, entry.slug, subscribe::redact(&sub.email), e);
			res!(done(db, entry.kind, seq));
			tally(db, &entry, Fate::Suppressed, id);
			if let Err(e2) = subscribe::mark_bounced(db, &sub.email, id) {
				warn!("{}: publish: could not suppress {}: {}",
					id, subscribe::redact(&sub.email), e2);
			}
		}
		Err(e)	=> {
			let tries = entry.tries + 1;
			if tries >= TRIES_MAX {
				warn!("{}: publish: newsletter '{}' to {} given up after {} tries: {}",
					id, entry.slug, subscribe::redact(&sub.email), tries, e);
				res!(done(db, entry.kind, seq));
				tally(db, &entry, Fate::Failed, id);
			} else {
				warn!("{}: publish: newsletter '{}' to {} did not send, try {}: {}",
					id, entry.slug, subscribe::redact(&sub.email), tries, e);
				let next = Entry {
					tries,
					next_try:	now + retry_after(tries),
					..entry
				};
				res!(retry(db, seq, &next));
			}
		}
	}
	Ok(step)
}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ WATCH                                                                     │
// └───────────────────────────────────────────────────────────────────────────┘

/// What the queue holds, as the alert reads it.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Stats {
	pub queued:	u64,		// entries in every lane, including those backing off
	pub oldest:	Option<u64>,	// unix second the oldest lane head was first queued; None if empty
}

/// Reads the queue's depth and the age of what waits longest at the head of a lane.
///
/// An entry retried goes to the tail but keeps the time it was first queued, so a lane's head is its
/// oldest entry except while one is backing off, which [`retry_after`] keeps well inside any alert
/// threshold worth setting.
pub fn stats<
	const UIDL: usize,
	UID:	NumIdDat<UIDL>,
	ENC:	Encrypter,
	KH:	Hasher,
	DB:	Database<UIDL, UID, ENC, KH>,
>(
	db:	&(Arc<RwLock<DB>>, UID),
)
	-> Outcome<Stats>
{
	let (db_arc, _) = db;
	let guard = lock_read!(db_arc);
	let mut out = Stats::default();
	for kind in KINDS {
		let head = res!(counter_in(&*guard, &head_key(kind)));
		let tail = res!(counter_in(&*guard, &tail_key(kind)));
		if tail <= head {
			continue;
		}
		out.queued += tail - head;
		let end = tail.min(head.saturating_add(HEAD_LOOK));
		for at in head..end {
			if let Ok(Some(e)) = entry_in(&*guard, kind, at) {
				out.oldest = Some(out.oldest.map_or(e.enqueued, |o| o.min(e.enqueued)));
				break;
			}
		}
	}
	Ok(out)
}

/// Tells one backlog episode apart from the next, so the operator hears of each once.
///
/// An episode begins when the oldest entry has waited longer than the threshold, is told again each
/// day it lasts, and ends when the queue is back under it. It lives in memory, so a restart during a
/// backlog begins a new episode and tells it again.
#[derive(Clone, Debug, Default)]
pub struct Watch {
	since:	Option<u64>,	// unix second the episode was first told
	told:	u64,		// unix second of the latest telling
	sent:	u64,		// messages sent since the episode began
}

impl Watch {

	/// Counts a message sent, if an episode is open.
	pub fn sent(&mut self) {
		if self.since.is_some() {
			self.sent += 1;
		}
	}

	/// What, if anything, the operator is owed at `now`, given what the queue holds. A `limit` of 0
	/// turns the alert off.
	pub fn note(&mut self, site: &str, st: &Stats, hourly: u32, limit: u64, now: u64)
		-> Option<AlertEvent>
	{
		let waited = st.oldest.map_or(0, |o| now.saturating_sub(o));
		let over = limit > 0 && st.queued > 0 && waited > limit;
		match (over, self.since) {
			(true, None)		=> {
				self.since = Some(now);
				self.told = now;
				self.sent = 0;
				Some(AlertEvent::OutboxBacklog {
					site:		site.to_string(),
					queued:		st.queued,
					oldest_secs:	waited,
					hourly,
				})
			}
			(true, Some(_))		=> {
				if now.saturating_sub(self.told) < DAY_SECS {
					return None;
				}
				self.told = now;
				Some(AlertEvent::OutboxBacklog {
					site:		site.to_string(),
					queued:		st.queued,
					oldest_secs:	waited,
					hourly,
				})
			}
			(false, Some(since))	=> {
				let sent = self.sent;
				*self = Self::default();
				Some(AlertEvent::OutboxCleared {
					site:		site.to_string(),
					were_secs:	now.saturating_sub(since),
					sent,
				})
			}
			(false, None)		=> None,
		}
	}
}

/// One pass of the drainer, then a look at the queue: the step it took and the alert it owes.
pub async fn tick<
	const UIDL: usize,
	UID:	NumIdDat<UIDL>,
	ENC:	Encrypter,
	KH:	Hasher,
	DB:	Database<UIDL, UID, ENC, KH>,
	C:	Courier,
>(
	db:	&(Arc<RwLock<DB>>, UID),
	cfg:	&PublishConfig,
	courier: &C,
	pacer:	&Pacer,
	watch:	&mut Watch,
	id:	&str,
	now_ms:	u64,
)
	-> Outcome<(Step, Option<AlertEvent>)>
{
	let stepped = res!(step(db, cfg, courier, pacer, id, now_ms).await);
	if stepped == Step::Sent {
		watch.sent();
	}
	let st = res!(stats(db));
	let owed = watch.note(id, &st, pacer.hourly(), cfg.outbox_alert_secs, now_ms / 1000);
	Ok((stepped, owed))
}

// Tells the operator, or says in the log that nobody can be told.
fn tell(alerter: &Option<Alerter>, id: &str, event: AlertEvent) {
	match alerter {
		Some(a)	=> a.raise(event),
		None	=> warn!("{}: publish: {} (no alerter is configured, so nobody is told)",
			id, event.subject(id)),
	}
}

/// Drains a vhost's queue for as long as the server runs.
///
/// Sleeps until the next slot, or until something is queued, or for [`POLL_SECS`], whichever is
/// first, and tells the operator through `alerter` what the [`Watch`] finds. A pass that fails is logged and tried again after the poll interval, never in a tight
/// loop.
pub async fn run<
	const UIDL: usize,
	UID:	NumIdDat<UIDL>,
	ENC:	Encrypter,
	KH:	Hasher,
	DB:	Database<UIDL, UID, ENC, KH>,
	C:	Courier,
>(
	db:	(Arc<RwLock<DB>>, UID),
	cfg:	Arc<PublishConfig>,
	courier: Arc<C>,
	pacer:	Arc<Pacer>,
	alerter: Option<Alerter>,
	id:	String,
) {
	info!("{}: publish: the outbox drainer is running at {} an hour", id, pacer.hourly());
	let mut watch = Watch::default();
	loop {
		// Registered before the pass, so a push during it is not missed.
		let woken = pacer.wake.notified();
		tokio::pin!(woken);
		woken.as_mut().enable();
		let nap_ms = match tick(&db, &cfg, &*courier, &pacer, &mut watch, &id, now_ms()).await {
			Ok((stepped, owed))	=> {
				if let Some(event) = owed {
					tell(&alerter, &id, event);
				}
				match stepped {
					Step::Sent | Step::Worked	=> continue,
					Step::Wait(ms)			=> ms,
					Step::Later(s)			=> s.min(POLL_SECS).saturating_mul(1000),
					Step::Idle | Step::Held		=> POLL_SECS * 1000,
				}
			}
			Err(e)			=> {
				warn!("{}: publish: the outbox drainer failed: {}", id, e);
				POLL_SECS * 1000
			}
		};
		tokio::select! {
			_ = &mut woken	=> {},
			_ = tokio::time::sleep(Duration::from_millis(nap_ms.max(1))) => {},
		}
	}
}
