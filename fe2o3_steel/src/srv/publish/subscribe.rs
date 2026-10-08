//! The newsletter's subscribers, in the vhost's own database.
//!
//! "Own the list, own the send." A subscriber is a row in the site's Ozone database, not a record in
//! a third party's, and the mail that reaches them is signed and sent by this host. There is no
//! provider between the site and its readers, and no list that leaves with one.
//!
//! # Double opt-in, because an address is not a consent
//!
//! Anyone can type anyone's address into a form. So a fresh sign-up is [`SubState::Pending`] and
//! receives one thing only -- a confirmation link -- and is promoted to [`SubState::Confirmed`], the
//! state that receives the newsletter, only when that link is followed. An address that never confirms
//! never hears from the site again, which is the difference between a subscriber and a stranger whose
//! address someone knew.
//!
//! # No enumeration, and no scans
//!
//! Subscribing is idempotent and says the same thing whether or not the address was already known: the
//! endpoint answers one "check your inbox" page either way, so the form is not an oracle for whether an
//! address is on the list. And the reads mirror [`super::store`]: the emails live in one index under
//! [`INDEX_KEY`], a subscriber under its own key, and nothing walks the whole database -- a token is
//! matched by reading the index and a record per entry, the same cost a listing already pays.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use crate::srv::publish::{
	PublishConfig,
	json,
	outbox::{
		self,
		Entry,
		Kind,
	},
	send::{
		self,
		MailSender,
	},
	page,
	rate::{
		self,
		Window,
	},
	store::{
		self,
		Edit,
	},
};

use oxedyne_fe2o3_core::{
	prelude::*,
	rand::Rand,
};
use oxedyne_fe2o3_iop_crypto::enc::Encrypter;
use oxedyne_fe2o3_iop_db::api::Database;
use oxedyne_fe2o3_iop_hash::api::Hasher;
use oxedyne_fe2o3_jdat::{
	prelude::*,
	id::NumIdDat,
};
use oxedyne_fe2o3_net::{
	http::{
		fields::HeaderFields,
		msg::HttpMessage,
		status::HttpStatus,
	},
};

use std::{
	collections::HashSet,
	sync::{
		Arc,
		RwLock,
	},
};


pub const KEY_PREFIX: &str = "publish/subscriber/";

pub const INDEX_KEY: &str = "publish/subscribers";

// The longest an address the form will take may be. A generous ceiling: the number is arbitrary,
// having one -- so a form cannot hand the store an unbounded key -- is not.
pub const EMAIL_MAX: usize = 254;

// How many characters an opt-in token carries. Drawn from TOKEN_ALPHABET, so 32 characters of a
// 36-symbol alphabet is a little over 165 bits: far past guessing. The token is the only thing
// that confirms or unsubscribes an address, so it is the one field here that must be unguessable.
pub const TOKEN_LEN: usize = 32;

// Deliberately URL-safe and needing no encoding, so the token sits in a `?token=` query and in a
// database key as itself, exactly as a slug's small alphabet does.
const TOKEN_ALPHABET: &str = "abcdefghijklmnopqrstuvwxyz0123456789";


/// Where a subscriber has got to in the double opt-in.
///
/// The state a piece of mail is gated on: only [`Confirmed`](Self::Confirmed) receives the newsletter.
/// [`Pending`](Self::Pending) has been sent a confirmation and not yet followed it;
/// [`Unsubscribed`](Self::Unsubscribed) has asked to stop and is kept, not deleted, so a later
/// re-subscribe is a fresh opt-in rather than a silent resurrection; [`Bounced`](Self::Bounced) is
/// suppressed -- a permanent delivery failure marked it, and nothing, not even a re-subscribe, sends to
/// it again.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum SubState {
	#[default]
	Pending,	// signed up and sent a confirmation link; receives nothing but that one link
	Confirmed,	// followed the link; the one state that receives the newsletter
	Unsubscribed,	// asked to stop; kept as a record, so re-subscribing opts in afresh
	// Suppressed after a permanent delivery failure -- a 5xx, an unknown mailbox. Kept as a record
	// and never sent to again: a re-subscribe does not resurrect it, since the address bounced for
	// a reason no opt-in changes.
	Bounced,
}

impl SubState {

	/// The word a record stores.
	pub fn as_str(&self) -> &'static str {
		match self {
			Self::Pending		=> "pending",
			Self::Confirmed		=> "confirmed",
			Self::Unsubscribed	=> "unsubscribed",
			Self::Bounced		=> "bounced",
		}
	}

	/// The state a word names. **An unknown word is pending**, the safe reading: a state this version
	/// cannot place must not thereby be treated as confirmed and sent mail, so it falls to the state
	/// that receives none.
	pub fn of(s: &str) -> Self {
		match s {
			"confirmed"	=> Self::Confirmed,
			"unsubscribed"	=> Self::Unsubscribed,
			"bounced"	=> Self::Bounced,
			_		=> Self::Pending,
		}
	}
}


/// One subscriber, as the store keeps them.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Subscriber {
	pub email:	String,		// normalised: trimmed and lowercased, so one address is one row
	pub state:	SubState,	// where they are in the double opt-in
	pub token:	String,		// unguessable, and minted fresh on each sign-up
	pub created:	Option<String>,	// ISO timestamp, where it is known
	// ISO timestamp of the last confirmation the outbox actually sent. A pending record held while
	// the outbound ceiling is 0 has none, and the expiry clock reads this, not `created`.
	pub sent:	Option<String>,
}

impl Subscriber {

	/// The subscriber as a daticle.
	///
	/// A plain map, not an ordered one, on the same reasoning as a post record: a subscriber is a set of
	/// named fields and nothing depends on their written order.
	pub fn to_dat(&self) -> Dat {
		let mut m = DaticleMap::new();
		m.insert(dat!("email"),	dat!(self.email.clone()));
		m.insert(dat!("state"),	dat!(self.state.as_str().to_string()));
		m.insert(dat!("token"),	dat!(self.token.clone()));
		// A subscriber with no known sign-up time carries no key for it, on the same footing an undated
		// post takes: an absent key and an empty value say the one thing.
		if let Some(c) = &self.created {
			m.insert(dat!("created"), dat!(c.clone()));
		}
		if let Some(t) = &self.sent {
			m.insert(dat!("sent"), dat!(t.clone()));
		}
		Dat::Map(m)
	}

	pub fn from_dat(d: &Dat) -> Outcome<Self> {
		let m = match d {
			Dat::Map(m)	=> m,
			_		=> return Err(err!(
				"publish: a subscriber record must be a map, not {:?}.", d.kind();
				Invalid, Input, Mismatch)),
		};
		let get_str = |key: &str| -> String {
			match m.get(&dat!(key)) {
				Some(Dat::Str(s))	=> s.clone(),
				_			=> String::new(),
			}
		};
		let email = get_str("email");
		if email.is_empty() {
			return Err(err!(
				"publish: a subscriber record names no email.";
				Invalid, Input, Missing));
		}
		let stamp = |key: &str| -> Option<String> {
			match m.get(&dat!(key)) {
				Some(Dat::Str(s))	=> Some(s.clone()),
				_			=> None,
			}
		};
		Ok(Self {
			email,
			state:		SubState::of(&get_str("state")),
			token:		get_str("token"),
			created:	stamp("created"),
			sent:		stamp("sent"),
		})
	}
}


fn key_of(email: &str) -> Dat {
	let mut s = String::from(KEY_PREFIX);
	s.push_str(email);
	dat!(s)
}

/// An address as the store keeps it, from an address as a person typed it: trimmed and lowercased.
///
/// One shape in the store, so an address typed `Me@Example.COM ` and one typed `me@example.com` are
/// the one subscriber and cannot both be on the list.
pub fn normalise_email(s: &str) -> String {
	s.trim().to_lowercase()
}

/// Whether a normalised address is one the form will take.
///
/// A shape check, not a delivery guarantee: exactly one `@`, a non-empty local part, a domain that
/// carries a dot, is not a bare label and is not an address literal, no whitespace, and within
/// [`EMAIL_MAX`]. The point is to
/// refuse what is plainly not an address before it reaches a key and a piece of mail -- the true test
/// of an address is whether the confirmation to it is ever followed, which is the whole reason for
/// double opt-in.
pub fn valid_email(s: &str) -> bool {
	if s.is_empty() || s.len() > EMAIL_MAX {
		return false;
	}
	if s.chars().any(|c| c.is_whitespace()) {
		return false;
	}
	let mut parts = s.split('@');
	let local = match parts.next() {
		Some(l)	=> l,
		None	=> return false,
	};
	let domain = match parts.next() {
		Some(d)	=> d,
		None	=> return false,
	};
	// A second `@` means more than two parts, so the iterator is not yet exhausted.
	if parts.next().is_some() {
		return false;
	}
	if local.is_empty() || domain.is_empty() {
		return false;
	}
	// A domain is at least `a.b`: a dot with something either side, and not at an edge.
	if !domain.contains('.') || domain.starts_with('.') || domain.ends_with('.') {
		return false;
	}
	// An address literal (`[127.0.0.1]`) names a machine, not a mailbox, and a form has no business
	// aiming this host's mail at one. Nor does a bare IPv4 form (`127.0.0.1`), which no domain is:
	// a top-level domain is never all digits.
	if domain.starts_with('[') {
		return false;
	}
	if domain.rsplit('.').next().map_or(true, |tld| tld.chars().all(|c| c.is_ascii_digit())) {
		return false;
	}
	true
}

/// A fresh, unguessable opt-in token.
pub fn mint_token() -> String {
	Rand::generate_random_string(TOKEN_LEN, TOKEN_ALPHABET)
}

// The name of the field no person fills in. `website`, because that is what a form-filler expects
// to find on a form, and filling it is the tell. The form must place it out of view without
// `display: none` or `hidden`, which the better form-fillers skip.
pub const TRAP_FIELD: &str = "website";

pub const RATE_PREFIX: &str = "publish/subscribe-rate/";	// apart from the comment counter

pub const TO_PREFIX: &str = "publish/subscribe-to/";	// the counter of confirmations to one address

/// Whether a submission filled in the field no person sees.
///
/// Whitespace is not a fill: a browser that helpfully trims or a proxy that pads should not cost a
/// reader their sign-up.
pub fn trapped(value: &str) -> bool {
	!value.trim().is_empty()
}

/// A salted, one-way rendering of where a sign-up came from.
///
/// The same trade [`super::comment::from_hash`] makes, with its own domain separator so one
/// counter's values are not the other's: enough to recognise a repeat, not enough to reconstruct an
/// address. The store holds no readable record of who signed up from where.
fn from_hash(addr: &str, salt: &[u8]) -> String {
	super::comment::hash_with(addr, b"subscribe-from", salt)
}

/// A salted, one-way rendering of the address a confirmation goes to.
///
/// The counter of confirmations to an address lives under this and not on the subscriber record, so
/// that expiring an unconfirmed record does not hand the limit back, and so that the store keeps no
/// readable address for someone who never consented. Its own domain separator keeps it apart from
/// [`from_hash`].
fn to_hash(email: &str, salt: &[u8]) -> String {
	super::comment::hash_with(email, b"subscribe-to", salt)
}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ STORE                                                                     │
// └───────────────────────────────────────────────────────────────────────────┘

pub fn get<
	const UIDL: usize,
	UID:	NumIdDat<UIDL>,
	ENC:	Encrypter,
	KH:	Hasher,
	DB:	Database<UIDL, UID, ENC, KH>,
>(
	db:	&(Arc<RwLock<DB>>, UID),
	email:	&str,
)
	-> Outcome<Option<Subscriber>>
{
	let (db_arc, _) = db;
	let guard = lock_read!(db_arc);
	get_in(&*guard, email)
}

// As `get`, on a database already locked.
fn get_in<
	const UIDL: usize,
	UID:	NumIdDat<UIDL>,
	ENC:	Encrypter,
	KH:	Hasher,
	DB:	Database<UIDL, UID, ENC, KH>,
>(
	dbr:	&DB,
	email:	&str,
)
	-> Outcome<Option<Subscriber>>
{
	match res!(dbr.get(&key_of(email), None)) {
		Some((val, _))	=> Ok(Some(res!(Subscriber::from_dat(&val)))),
		None		=> Ok(None),
	}
}

/// Writes a subscriber, adding it to the index if it is new, on a database already write-locked.
///
/// The record and the index entry go in under one write guard. With a read guard two sign-ups each
/// read the index, added their own address and wrote it back over the other's, and the loser had a
/// record the list never named.
fn put_in<
	const UIDL: usize,
	UID:	NumIdDat<UIDL>,
	ENC:	Encrypter,
	KH:	Hasher,
	DB:	Database<UIDL, UID, ENC, KH>,
>(
	dbr:	&DB,
	user:	UID,
	sub:	&Subscriber,
)
	-> Outcome<()>
{
	res!(dbr.insert(key_of(&sub.email), sub.to_dat(), user, None));
	store::edit_in(dbr, user, &dat!(INDEX_KEY), |old| -> Outcome<(Edit, ())> {
		let mut emails = res!(store::names_of(old, "subscriber index"));
		if emails.iter().any(|e| e == &sub.email) {
			return Ok((Edit::Keep, ()));
		}
		emails.push(sub.email.clone());
		Ok((Edit::Set(store::names_dat(&emails)), ()))
	})
}

// Reads a subscriber, lets `f` make of it what it will, and writes that back, all under one write
// guard. `f` answers the subscriber to write, if any, and what the caller wants to know. `None` where
// the store holds no such address.
fn amend<
	const UIDL: usize,
	UID:	NumIdDat<UIDL>,
	ENC:	Encrypter,
	KH:	Hasher,
	DB:	Database<UIDL, UID, ENC, KH>,
	R,
	F:	FnOnce(Subscriber) -> Outcome<(Option<Subscriber>, R)>,
>(
	db:	&(Arc<RwLock<DB>>, UID),
	email:	&str,
	f:	F,
)
	-> Outcome<Option<R>>
{
	store::exclusive(db, |dbr, user| -> Outcome<Option<R>> {
		let cur = match res!(get_in(dbr, email)) {
			Some(s)	=> s,
			None	=> return Ok(None),
		};
		let (next, out) = res!(f(cur));
		if let Some(n) = next {
			res!(put_in(dbr, user, &n));
		}
		Ok(Some(out))
	})
}

fn index<
	const UIDL: usize,
	UID:	NumIdDat<UIDL>,
	ENC:	Encrypter,
	KH:	Hasher,
	DB:	Database<UIDL, UID, ENC, KH>,
>(
	db:	&(Arc<RwLock<DB>>, UID),
)
	-> Outcome<Vec<String>>
{
	let (db_arc, _) = db;
	let guard = lock_read!(db_arc);
	// No index is a list nobody has subscribed to, not an error -- the empty list it never wrote.
	let old = match res!(guard.get(&dat!(INDEX_KEY), None)) {
		Some((v, _))	=> Some(v),
		None		=> None,
	};
	store::names_of(old, "subscriber index")
}

/// Every subscriber the store holds, in index order.
///
/// A record the index names but the database does not hold is passed over with a complaint, rather
/// than failing the lot, on the same reasoning [`super::store::list_records`] takes.
pub fn list<
	const UIDL: usize,
	UID:	NumIdDat<UIDL>,
	ENC:	Encrypter,
	KH:	Hasher,
	DB:	Database<UIDL, UID, ENC, KH>,
>(
	db:	&(Arc<RwLock<DB>>, UID),
	id:	&str,
)
	-> Outcome<Vec<Subscriber>>
{
	let emails = res!(index(db));
	let mut out = Vec::new();
	for email in &emails {
		match get(db, email) {
			Ok(Some(s))	=> out.push(s),
			Ok(None)	=> warn!(
				"{}: publish: the subscriber index names {}, which is not there", id, redact(email)),
			Err(e)		=> warn!("{}: publish: skipping subscriber {}: {}", id, redact(email), e),
		}
	}
	Ok(out)
}

/// How many subscribers the store holds, whatever their state.
pub fn count<
	const UIDL: usize,
	UID:	NumIdDat<UIDL>,
	ENC:	Encrypter,
	KH:	Hasher,
	DB:	Database<UIDL, UID, ENC, KH>,
>(
	db:	&(Arc<RwLock<DB>>, UID),
	id:	&str,
)
	-> Outcome<usize>
{
	Ok(res!(list(db, id)).len())
}

/// The send set: every confirmed subscriber, the only ones a newsletter reaches.
///
/// Whole subscribers rather than bare addresses, because each carries the token the newsletter's own
/// unsubscribe link is built from -- one link per recipient, so the person who clicks it removes
/// themselves and nobody else.
pub fn confirmed<
	const UIDL: usize,
	UID:	NumIdDat<UIDL>,
	ENC:	Encrypter,
	KH:	Hasher,
	DB:	Database<UIDL, UID, ENC, KH>,
>(
	db:	&(Arc<RwLock<DB>>, UID),
	id:	&str,
)
	-> Outcome<Vec<Subscriber>>
{
	Ok(res!(list(db, id)).into_iter().filter(|s| s.state == SubState::Confirmed).collect())
}

/// The subscriber list as CSV: address, state, sign-up time.
///
/// The list the site owns, in the form anything reads -- a spreadsheet, another tool, a backup. The
/// header names the columns; a field carrying a comma or a quote is quoted, so an address never splits
/// a row.
pub fn export<
	const UIDL: usize,
	UID:	NumIdDat<UIDL>,
	ENC:	Encrypter,
	KH:	Hasher,
	DB:	Database<UIDL, UID, ENC, KH>,
>(
	db:	&(Arc<RwLock<DB>>, UID),
	id:	&str,
)
	-> Outcome<String>
{
	let subs = res!(list(db, id));
	let mut out = String::from("email,state,created\n");
	for s in &subs {
		out.push_str(&csv_field(&s.email));
		out.push(',');
		out.push_str(s.state.as_str());
		out.push(',');
		out.push_str(&csv_field(s.created.as_deref().unwrap_or("")));
		out.push('\n');
	}
	Ok(out)
}

/// A CSV field, quoted where it carries a comma, a quote or a newline.
fn csv_field(s: &str) -> String {
	if s.contains(',') || s.contains('"') || s.contains('\n') {
		fmt!("\"{}\"", s.replace('"', "\"\""))
	} else {
		s.to_string()
	}
}

/// The subscriber a token names, by reading the index and a record per entry.
///
/// Index-driven, like every read here: no scan. The list is a newsletter's, not a social network's, so
/// a read per entry to match a token is a cost worth its simplicity.
fn find_by_token<
	const UIDL: usize,
	UID:	NumIdDat<UIDL>,
	ENC:	Encrypter,
	KH:	Hasher,
	DB:	Database<UIDL, UID, ENC, KH>,
>(
	db:	&(Arc<RwLock<DB>>, UID),
	token:	&str,
	id:	&str,
)
	-> Outcome<Option<Subscriber>>
{
	if token.is_empty() {
		return Ok(None);
	}
	for sub in res!(list(db, id)) {
		if sub.token == token {
			return Ok(Some(sub));
		}
	}
	Ok(None)
}

/// Records a pending sign-up and says whether a confirmation should be sent.
///
/// Idempotent, and deliberately not an oracle:
///
/// - A **new** or previously **unsubscribed** address is written [`Pending`](SubState::Pending) with a
///   fresh token, and `Some(subscriber)` is returned: send them a confirmation.
/// - An address already **pending** keeps its token, since the link already sent to it must go on
///   working, and `Some(subscriber)` is returned: send the confirmation again.
/// - An address already **confirmed** is left exactly as it is and `None` is returned: it is on the
///   list, and re-confirming it would be a second welcome to someone who never left.
/// - An address **bounced** is left suppressed and `None` is returned: a permanent failure marked it,
///   and a re-subscribe must not resurrect an address the mail server said does not exist.
///
/// Whatever the address's state, a confirmation is asked for only if `w` allows one more to it at
/// `now`; otherwise `None` is returned and nothing is written to the subscriber. The counter is only
/// read here. It counts confirmations **sent**, and [`count_sent`] moves it when one has left, so a
/// sign-up that waits behind a hold or a backlog, or is repeated, spends nothing. This is the
/// drainer's call, never a request's: the request queues the address and answers, so its reply does
/// not depend on which of these cases the address is.
pub fn add_pending<
	const UIDL: usize,
	UID:	NumIdDat<UIDL>,
	ENC:	Encrypter,
	KH:	Hasher,
	DB:	Database<UIDL, UID, ENC, KH>,
>(
	db:	&(Arc<RwLock<DB>>, UID),
	email:	&str,
	w:	&Window,
	now:	u64,
)
	-> Outcome<Option<Subscriber>>
{
	add_pending_from(db, email, w, now, None)
}

/// As [`add_pending`] for a sign-up that came off the queue at `from`, a lane and a sequence number.
///
/// The sign-up is applied only if that entry is still queued, checked under the guard that writes the
/// record. The drainer takes an entry and acts on it a moment later, and an erasure in between has
/// removed the entry and the record; without the check the drainer would make the record again. Where
/// the entry is gone, `None` is returned and nothing is written.
pub fn add_pending_from<
	const UIDL: usize,
	UID:	NumIdDat<UIDL>,
	ENC:	Encrypter,
	KH:	Hasher,
	DB:	Database<UIDL, UID, ENC, KH>,
>(
	db:	&(Arc<RwLock<DB>>, UID),
	email:	&str,
	w:	&Window,
	now:	u64,
	from:	Option<(Kind, u64)>,
)
	-> Outcome<Option<Subscriber>>
{
	let email = normalise_email(email);
	if !valid_email(&email) {
		return Err(err!(
			"publish: {} is not a shape an address takes.", redact(&email);
			Invalid, Input));
	}
	// The secret is read before the write guard is taken. `site_secret` takes the lock itself, and a
	// std `RwLock` is not re-entrant.
	let salt = res!(super::comment::site_secret(db));
	let rkey = fmt!("{}{}", TO_PREFIX, to_hash(&email, &salt));
	// The reads of what is there and of the counter, and the writes that replace them, are one step
	// under the write guard.
	store::exclusive(db, |dbr, user| -> Outcome<Option<Subscriber>> {
		if let Some((kind, seq)) = from {
			if !res!(outbox::queued_at_in(dbr, kind, seq)) {
				return Ok(None);
			}
		}
		let existing = res!(get_in(dbr, &email));
		let allowed = res!(rate::permits_in(dbr, &rkey, w, now));
		// A confirmed address is on the list; do not welcome it twice. A bounced address is
		// suppressed and stays so.
		let kept = match existing {
			Some(s)	=> match s.state {
				SubState::Confirmed | SubState::Bounced	=> return Ok(None),
				SubState::Pending			=> Some(s),
				SubState::Unsubscribed			=> None,
			},
			None	=> None,
		};
		if !allowed {
			return Ok(None);
		}
		if let Some(s) = kept {
			return Ok(Some(s));
		}
		let sub = Subscriber {
			email:		email.clone(),
			state:		SubState::Pending,
			token:		mint_token(),
			created:	send::iso_now().ok(),
			sent:		None,
		};
		res!(put_in(dbr, user, &sub));
		Ok(Some(sub))
	})
}

/// Counts a confirmation sent to an address, in the window `w`, at `now`.
///
/// The limit of confirmations to one address is spent here, when the mail has left, and checked in
/// [`add_pending`] before it goes.
pub fn count_sent<
	const UIDL: usize,
	UID:	NumIdDat<UIDL>,
	ENC:	Encrypter,
	KH:	Hasher,
	DB:	Database<UIDL, UID, ENC, KH>,
>(
	db:	&(Arc<RwLock<DB>>, UID),
	email:	&str,
	w:	&Window,
	now:	u64,
)
	-> Outcome<()>
{
	let email = normalise_email(email);
	let salt = res!(super::comment::site_secret(db));
	res!(rate::allow_at(db, &fmt!("{}{}", TO_PREFIX, to_hash(&email, &salt)), w, now));
	Ok(())
}

/// What a confirmation link found when it was followed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConfirmOutcome {
	Confirmed,	// promoted from pending: the newsletter now reaches them
	// The token named a subscriber already confirmed. The safe, idempotent answer to a link
	// followed twice: they are on the list, said so, and nothing changed.
	Already,
	// The token named nobody: it is malformed, expired by a re-subscribe that minted a new one, or
	// never existed.
	Unknown,
}

pub fn confirm<
	const UIDL: usize,
	UID:	NumIdDat<UIDL>,
	ENC:	Encrypter,
	KH:	Hasher,
	DB:	Database<UIDL, UID, ENC, KH>,
>(
	db:	&(Arc<RwLock<DB>>, UID),
	token:	&str,
	id:	&str,
)
	-> Outcome<ConfirmOutcome>
{
	let found = match res!(find_by_token(db, token, id)) {
		Some(s)	=> s,
		None	=> return Ok(ConfirmOutcome::Unknown),
	};
	// The record is read again under the write guard: a re-subscribe may have minted a new token, or an
	// unsubscribe changed the state, between the search and now.
	let out = res!(amend(db, &found.email, |mut sub| -> Outcome<(Option<Subscriber>, ConfirmOutcome)> {
		if sub.token != token {
			return Ok((None, ConfirmOutcome::Unknown));
		}
		match sub.state {
			SubState::Confirmed	=> Ok((None, ConfirmOutcome::Already)),
			_			=> {
				sub.state = SubState::Confirmed;
				Ok((Some(sub), ConfirmOutcome::Confirmed))
			}
		}
	}));
	let out = out.unwrap_or(ConfirmOutcome::Unknown);
	if out == ConfirmOutcome::Confirmed {
		info!("{}: publish: {} confirmed their subscription", id, redact(&found.email));
	}
	Ok(out)
}

/// What an unsubscribe link found when it was followed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UnsubOutcome {
	Done,		// set unsubscribed, or already was, so no more mail reaches them either way
	Unknown,	// the token named nobody
}

/// Sets a subscriber unsubscribed, by their token.
///
/// The record is kept, not deleted: a later re-subscribe is a fresh opt-in through
/// [`add_pending`], not a silent return to a list they asked to leave.
pub fn unsubscribe<
	const UIDL: usize,
	UID:	NumIdDat<UIDL>,
	ENC:	Encrypter,
	KH:	Hasher,
	DB:	Database<UIDL, UID, ENC, KH>,
>(
	db:	&(Arc<RwLock<DB>>, UID),
	token:	&str,
	id:	&str,
)
	-> Outcome<UnsubOutcome>
{
	let found = match res!(find_by_token(db, token, id)) {
		Some(s)	=> s,
		None	=> return Ok(UnsubOutcome::Unknown),
	};
	// `Some(true)` where this call changed the state, `Some(false)` where it already was, `None` where
	// the token no longer names the record.
	let changed = res!(amend(db, &found.email, |mut sub| -> Outcome<(Option<Subscriber>, Option<bool>)> {
		if sub.token != token {
			return Ok((None, None));
		}
		if sub.state == SubState::Unsubscribed {
			return Ok((None, Some(false)));
		}
		sub.state = SubState::Unsubscribed;
		Ok((Some(sub), Some(true)))
	}));
	match changed.flatten() {
		Some(true)	=> {
			info!("{}: publish: {} unsubscribed", id, redact(&found.email));
			Ok(UnsubOutcome::Done)
		},
		Some(false)	=> Ok(UnsubOutcome::Done),
		None		=> Ok(UnsubOutcome::Unknown),
	}
}

/// Sets a subscriber unsubscribed, by their address, for the admin console.
///
/// The address-keyed twin of [`unsubscribe`], which the public link uses by token. The admin acts on the
/// address they see in the list, not a token, so this reads the record by its key. The record is kept,
/// not deleted -- an admin who means to erase calls [`remove`]. `false` where the store holds no such
/// address, so the caller can say the subscriber was not there rather than claim an unsubscribe that
/// changed nothing.
pub fn unsubscribe_email<
	const UIDL: usize,
	UID:	NumIdDat<UIDL>,
	ENC:	Encrypter,
	KH:	Hasher,
	DB:	Database<UIDL, UID, ENC, KH>,
>(
	db:	&(Arc<RwLock<DB>>, UID),
	email:	&str,
	id:	&str,
)
	-> Outcome<bool>
{
	let email = normalise_email(email);
	let changed = res!(amend(db, &email, |mut sub| -> Outcome<(Option<Subscriber>, bool)> {
		if sub.state == SubState::Unsubscribed {
			return Ok((None, false));
		}
		sub.state = SubState::Unsubscribed;
		Ok((Some(sub), true))
	}));
	match changed {
		Some(true)	=> {
			info!("{}: publish: {} unsubscribed by an admin", id, redact(&email));
			Ok(true)
		},
		Some(false)	=> Ok(true),
		None		=> Ok(false),
	}
}

/// Suppresses a subscriber after a permanent delivery failure, by their address.
///
/// The send set is built from [`SubState::Confirmed`] alone, so a bounced address leaves it at once and
/// is never mailed again -- not by the newsletter, and not by a re-subscribe, since [`add_pending`]
/// keeps a bounced record suppressed. The record is kept so the suppression is durable and countable;
/// only a permanent failure calls this, never a transient one. `false` where the store holds no such
/// address, and a no-op where it is already bounced.
pub fn mark_bounced<
	const UIDL: usize,
	UID:	NumIdDat<UIDL>,
	ENC:	Encrypter,
	KH:	Hasher,
	DB:	Database<UIDL, UID, ENC, KH>,
>(
	db:	&(Arc<RwLock<DB>>, UID),
	email:	&str,
	id:	&str,
)
	-> Outcome<bool>
{
	let email = normalise_email(email);
	let changed = res!(amend(db, &email, |mut sub| -> Outcome<(Option<Subscriber>, bool)> {
		if sub.state == SubState::Bounced {
			return Ok((None, false));
		}
		sub.state = SubState::Bounced;
		Ok((Some(sub), true))
	}));
	match changed {
		Some(true)	=> {
			warn!("{}: publish: {} suppressed after a permanent delivery failure", id, redact(&email));
			Ok(true)
		},
		Some(false)	=> Ok(true),
		None		=> Ok(false),
	}
}

/// Records that a confirmation was sent to a pending subscriber, at an ISO time the caller names.
///
/// Only a [`SubState::Pending`] record takes it; a subscriber who has confirmed or left since the
/// send is left as it is. `true` where the time was written.
pub fn mark_sent<
	const UIDL: usize,
	UID:	NumIdDat<UIDL>,
	ENC:	Encrypter,
	KH:	Hasher,
	DB:	Database<UIDL, UID, ENC, KH>,
>(
	db:	&(Arc<RwLock<DB>>, UID),
	email:	&str,
	at:	&str,
)
	-> Outcome<bool>
{
	let email = normalise_email(email);
	let done = res!(amend(db, &email, |mut sub| -> Outcome<(Option<Subscriber>, bool)> {
		if sub.state != SubState::Pending {
			return Ok((None, false));
		}
		sub.sent = Some(at.to_string());
		Ok((Some(sub), true))
	}));
	Ok(done.unwrap_or(false))
}

/// Erases a subscriber outright: the record and its place in the index both, by their address.
///
/// A GDPR erasure, distinct from [`unsubscribe_email`]: an unsubscribe keeps the record so a re-subscribe
/// opts in afresh, whereas this leaves nothing behind -- no state, no token, no row in the count, no
/// message queued to the address. Mirrors [`super::store::delete`]: the key is deleted and the address filtered out of the index, so a listing
/// does not name what is gone. `true` where an address was there to erase.
pub fn remove<
	const UIDL: usize,
	UID:	NumIdDat<UIDL>,
	ENC:	Encrypter,
	KH:	Hasher,
	DB:	Database<UIDL, UID, ENC, KH>,
>(
	db:	&(Arc<RwLock<DB>>, UID),
	email:	&str,
	id:	&str,
)
	-> Outcome<bool>
{
	let email = normalise_email(email);
	let existed = res!(store::exclusive(db, |dbr, user| -> Outcome<bool> {
		// Whether the address was really there, read by key so a tombstone reads as absent -- unlike the
		// database's own `delete`, which marks a key for deletion and reports success even for one already
		// gone. So a repeat erase honestly says there was nothing to erase.
		let existed = res!(get_in(dbr, &email)).is_some();
		res!(dbr.delete(&key_of(&email), user, None));
		res!(unlist_in(dbr, user, &HashSet::from([email.as_str()])));
		// What is queued for the address goes under the same guard, so the drainer cannot make the
		// record again from a sign-up that waits, or mail the address a copy that waits.
		let queued = res!(outbox::purge_in(dbr, user, &email));
		if queued > 0 {
			info!("{}: publish: {} queued message(s) to {} removed with the erasure",
				id, queued, redact(&email));
		}
		Ok(existed)
	}));
	if existed {
		info!("{}: publish: {} erased from the list by an admin", id, redact(&email));
	}
	Ok(existed)
}

// Takes the addresses in `gone` out of the index, on a database already write-locked. The index is
// written back once, and not at all where it names none of them.
fn unlist_in<
	const UIDL: usize,
	UID:	NumIdDat<UIDL>,
	ENC:	Encrypter,
	KH:	Hasher,
	DB:	Database<UIDL, UID, ENC, KH>,
>(
	dbr:	&DB,
	user:	UID,
	gone:	&HashSet<&str>,
)
	-> Outcome<()>
{
	store::edit_in(dbr, user, &dat!(INDEX_KEY), |old| -> Outcome<(Edit, ())> {
		let emails = res!(store::names_of(old, "subscriber index"));
		let kept: Vec<String> = emails.iter().filter(|e| !gone.contains(e.as_str())).cloned().collect();
		if kept.len() == emails.len() {
			return Ok((Edit::Keep, ()));
		}
		Ok((Edit::Set(store::names_dat(&kept)), ()))
	})
}

// Has a pending subscriber waited `age` seconds since the last confirmation was sent to it?
//
// The clock is `sent`. A record never sent anything (one from before `sent` existed, or whose
// confirmation was given up) lapses from `created`, but only while the host is `sending`: held behind
// a ceiling of 0 nothing has been sent to anyone, and an address kept for the hold must outlive it. A
// `sent` that will not read is no clock, since an age that cannot be shown is not one to delete on.
// Only a pending record can lapse.
fn lapsed(sub: &Subscriber, age: u64, now: u64, sending: bool) -> bool {
	if sub.state != SubState::Pending {
		return false;
	}
	let parse = super::comment::parse_stamp_secs;
	let clock = match (&sub.sent, &sub.created) {
		(Some(s), _)			=> parse(s),
		(None, Some(c)) if sending	=> parse(c),
		_				=> None,
	};
	clock.map_or(false, |t| now.saturating_sub(t) >= age)
}

// Subscribers judged under one write guard when expiring. The index is written back once for each
// chunk, so this also sets how often a long list is rewritten.
const EXPIRE_CHUNK: usize = 256;

/// Deletes the pending subscribers whose last confirmation was sent `days` or more ago, and says how
/// many went. A `days` of 0 deletes nothing. A record never sent one lapses from its creation, while
/// `sending` says the host's ceiling is above 0.
///
/// Each goes whole, the record and its place in the index. Only [`SubState::Pending`] lapses: a
/// confirmed, unsubscribed or bounced record is kept, the last because its suppression is the
/// reason it exists. The counter of confirmations to the address lives apart, under a hash, so a
/// lapsed address that signs up again does not find its limit handed back.
///
/// The index is walked and no scan is made, so the cost is the size of the list. Every record is
/// read again under the write guard before it is deleted, so one that has confirmed, left or been
/// sent another confirmation since the walk is kept.
pub fn expire_pending<
	const UIDL: usize,
	UID:	NumIdDat<UIDL>,
	ENC:	Encrypter,
	KH:	Hasher,
	DB:	Database<UIDL, UID, ENC, KH>,
>(
	db:	&(Arc<RwLock<DB>>, UID),
	days:	u64,
	now:	u64,
	id:	&str,
	sending: bool,
)
	-> Outcome<usize>
{
	if days == 0 {
		return Ok(0);
	}
	let age = days.saturating_mul(86_400);
	let mut due = Vec::new();
	for email in res!(index(db)) {
		match get(db, &email) {
			Ok(Some(s)) if lapsed(&s, age, now, sending)	=> due.push(email),
			Ok(_)					=> {},
			Err(e)					=> warn!(
				"{}: publish: skipping subscriber {} while expiring: {}", id, redact(&email), e),
		}
	}
	let mut gone = 0;
	for chunk in due.chunks(EXPIRE_CHUNK) {
		gone += res!(store::exclusive(db, |dbr, user| -> Outcome<usize> {
			let mut done: HashSet<&str> = HashSet::new();
			for email in chunk {
				match res!(get_in(dbr, email)) {
					Some(s) if lapsed(&s, age, now, sending)	=> {
						res!(dbr.delete(&key_of(email), user, None));
						done.insert(email.as_str());
					},
					_				=> {},
				}
			}
			if !done.is_empty() {
				res!(unlist_in(dbr, user, &done));
			}
			Ok(done.len())
		}));
	}
	if gone > 0 {
		info!("{}: publish: {} sign-up(s) expired unconfirmed after {} days", id, gone, days);
	}
	Ok(gone)
}

/// An address with its local part masked, for a log line.
///
/// The domain is kept -- it is useful and not private -- and the local part is reduced to its first
/// character, so a log is a record of what happened without being a copy of the list.
pub fn redact(email: &str) -> String {
	match email.split_once('@') {
		Some((local, domain))	=> {
			let first = local.chars().next().unwrap_or('?');
			fmt!("{}***@{}", first, domain)
		}
		None			=> fmt!("***"),
	}
}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ THE PUBLIC ENDPOINTS                                                       │
// └───────────────────────────────────────────────────────────────────────────┘

/// The themed sign-up form, for a `GET {path}/subscribe`.
///
/// A working, script-free form the site can link to directly, and the shape the site's own inline form
/// should mirror: a `POST` to the same path with one field, `email`.
pub fn subscribe_form(cfg: &PublishConfig) -> HttpMessage {
	page::subscribe_form_page(cfg)
}

/// What a sign-up is told, as a page or, for a caller asking for JSON, as `{"said", "message"}`.
///
/// `Sent` is every outcome that looks like success -- a new address, a pending one, a confirmed one,
/// a trapped fill and a sender over its limit -- so it is no oracle for the list. A site may map
/// `said` to its own words and ignore `message`, which is Steel's generic English.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Said {
	Sent,
	Invalid,
	Unavailable,
}

impl Said {

	fn as_str(&self) -> &'static str {
		match self {
			Self::Sent		=> "sent",
			Self::Invalid		=> "invalid",
			Self::Unavailable	=> "unavailable",
		}
	}

	fn text(&self) -> &'static str {
		match self {
			Self::Sent		=> page::SENT_TEXT,
			Self::Invalid		=> page::INVALID_TEXT,
			Self::Unavailable	=> page::UNAVAILABLE_TEXT,
		}
	}

	// The same answer in the form the caller asked for. All three are a 200, as the pages always were.
	fn answer(self, cfg: &PublishConfig, headers: &HeaderFields) -> Outcome<HttpMessage> {
		if headers.wants_json() {
			return json::said(HttpStatus::OK, self.as_str(), self.text());
		}
		Ok(match self {
			Self::Sent		=> page::subscribe_sent_page(cfg),
			Self::Invalid		=> page::subscribe_invalid_page(cfg),
			Self::Unavailable	=> page::subscribe_unavailable_page(cfg),
		})
	}
}

/// Records a pending sign-up and queues the confirmation, for a `POST {path}/subscribe`.
///
/// Always answers the same "check your inbox" page, whether the address was new, pending or already
/// confirmed, so nothing here reveals whether an address is on the list. A caller whose `Accept`
/// asks for JSON gets `{"said", "message"}` instead, with the same property. Where mail is not configured,
/// or the site has no canonical origin to build an absolute confirmation link from, it says the
/// newsletter is not set up rather than storing a pending subscriber it can never confirm.
///
/// # What stands between a stranger and this host's outbound mail
///
/// This endpoint is unauthenticated and its effect is a piece of mail to an address the sender
/// chose. Left bare it is a mail-bombing tool wearing the site's own domain, and the cost is not
/// the disk -- it is the sending reputation every later confirmation depends on. So, before
/// anything is stored or sent:
///
/// - **The field no person fills in.** [`TRAP_FIELD`] filled means a machine filled it. The
///   submission is dropped and answered with the ordinary page, since telling a bot it was spotted
///   only teaches it which field to leave alone.
/// - **A limit per sender**, keyed on a salted hash of where the request came from and counted
///   apart from the comment limiter. Over it, the same page again: a form that says "you are doing
///   that too often" is a form that tells a script exactly what it has found.
/// - **A limit per address**, in [`add_pending`], counted whoever asks: a script rotating its
///   sources still cannot bury one mailbox. Over it the page is the same again.
///
/// Double opt-in is the further layer and the one already here: an address that never confirms hears
/// nothing further, so the worst a flood achieves is a few messages per address rather than a
/// correspondence.
pub async fn handle_subscribe<
	const UIDL: usize,
	UID:	NumIdDat<UIDL>,
	ENC:	Encrypter,
	KH:	Hasher,
	DB:	Database<UIDL, UID, ENC, KH>,
>(
	cfg:	&PublishConfig,
	db:	Option<&(Arc<RwLock<DB>>, UID)>,
	mail:	&Option<Arc<MailSender>>,
	hdrs:	&HeaderFields,
	body:	&[u8],
	from:	Option<&str>,
	id:	&str,
)
	-> Outcome<HttpMessage>
{
	let db = match db {
		Some(db)	=> db,
		None		=> return Said::Unavailable.answer(cfg, hdrs),
	};
	// The newsletter needs a sender to post the confirmation, and an absolute origin to build the link
	// it carries. Missing either, the honest answer is that signup is not available -- not a pending row
	// that will wait for a confirmation nothing can send.
	let sender = match mail {
		Some(m)	=> m,
		None	=> return Said::Unavailable.answer(cfg, hdrs),
	};
	if cfg.base_url.is_empty() {
		warn!("{}: publish: a subscribe arrived but the site has no base_url for a confirm link", id);
		return Said::Unavailable.answer(cfg, hdrs);
	}

	// The trap, read before the address: a filled one means nothing else about this submission is
	// worth the reads it would cost.
	let trap = crate::srv::console::form_field(body, TRAP_FIELD).unwrap_or_default();
	if trapped(&trap) {
		info!("{}: publish: a sign-up filled the field no person sees; dropped", id);
		return Said::Sent.answer(cfg, hdrs);
	}

	// What this sender is allowed. A refusal costs one read and writes no subscriber, which is why
	// it comes before the store and the mail. A request with no address behind it -- which should
	// not happen, since the caller supplies one -- is not limited here.
	if let Some(addr) = from {
		let salt = res!(crate::srv::publish::comment::site_secret(db));
		let hashed = from_hash(addr, &salt);
		let w = Window::hourly(cfg.subscribe_rate_secs, cfg.subscribe_rate_hourly);
		if !res!(rate::allow(db, &fmt!("{}{}", RATE_PREFIX, hashed), &w)) {
			info!("{}: publish: a sender is signing up faster than this site allows", id);
			return Said::Sent.answer(cfg, hdrs);
		}
	}

	let email = crate::srv::console::form_field(body, "email").unwrap_or_default();
	let email = normalise_email(&email);
	// A plainly malformed address is told so on its own page: that reveals nothing about the list, only
	// about what was typed.
	if !valid_email(&email) {
		return Said::Invalid.answer(cfg, hdrs);
	}

	queue_signup(cfg, db, sender, &email, id);
	Said::Sent.answer(cfg, hdrs)
}

// The whole of a sign-up that the request does: one entry on the queue, the same small write for every
// valid address, whatever state it is in. The drainer applies the sign-up (see [`add_pending`]), judges
// the limits and sends, so the reply carries no trace of whether the address was new, pending,
// confirmed or over its limit. A push that fails is logged, and the reader still gets the same page --
// retrying the form queues it again, and saying "we could not email you" would leak that the address
// was actionable. The queue is capped (`outbox_confirm_max`): past the cap the sign-up is dropped
// with a warning and the reply is the same page, so a flood fills no more than the cap and nobody
// can tell by the reply whether theirs was queued.
fn queue_signup<
	const UIDL: usize,
	UID:	NumIdDat<UIDL>,
	ENC:	Encrypter,
	KH:	Hasher,
	DB:	Database<UIDL, UID, ENC, KH>,
>(
	cfg:	&PublishConfig,
	db:	&(Arc<RwLock<DB>>, UID),
	sender:	&Arc<MailSender>,
	email:	&str,
	id:	&str,
) {
	let entry = Entry::new(Kind::Confirm, email, "", rate::now_secs());
	match outbox::push_capped(db, &entry, cfg.outbox_confirm_max) {
		Ok(true)	=> {
			debug!("{}: publish: sign-up of {} queued", id, redact(email));
			sender.pacer().wake();
		}
		Ok(false)	=> warn!("{}: publish: the confirmation queue is full at {}, so a sign-up of {} is dropped",
			id, cfg.outbox_confirm_max, redact(email)),
		Err(e)		=> warn!("{}: publish: sign-up of {} could not be queued: {}", id, redact(email), e),
	}
}

/// Confirms a pending subscriber, for a `GET {path}/confirm?token=...`.
pub fn handle_confirm<
	const UIDL: usize,
	UID:	NumIdDat<UIDL>,
	ENC:	Encrypter,
	KH:	Hasher,
	DB:	Database<UIDL, UID, ENC, KH>,
>(
	cfg:	&PublishConfig,
	db:	Option<&(Arc<RwLock<DB>>, UID)>,
	query:	&str,
	id:	&str,
)
	-> Outcome<HttpMessage>
{
	let db = match db {
		Some(db)	=> db,
		None		=> return Ok(page::subscribe_unavailable_page(cfg)),
	};
	let token = token_of(query);
	match res!(confirm(db, &token, id)) {
		ConfirmOutcome::Confirmed	=> Ok(page::subscribe_confirmed_page(cfg)),
		ConfirmOutcome::Already		=> Ok(page::subscribe_confirmed_page(cfg)),
		ConfirmOutcome::Unknown		=> Ok(page::subscribe_bad_token_page(cfg)),
	}
}

/// Unsubscribes a subscriber, for a `GET {path}/unsubscribe?token=...`.
///
/// A `GET` for a click from an email, which is where an unsubscribe link is followed. It removes and
/// says so idempotently -- a token followed twice lands on the same page.
pub fn handle_unsubscribe<
	const UIDL: usize,
	UID:	NumIdDat<UIDL>,
	ENC:	Encrypter,
	KH:	Hasher,
	DB:	Database<UIDL, UID, ENC, KH>,
>(
	cfg:	&PublishConfig,
	db:	Option<&(Arc<RwLock<DB>>, UID)>,
	query:	&str,
	id:	&str,
)
	-> Outcome<HttpMessage>
{
	let db = match db {
		Some(db)	=> db,
		None		=> return Ok(page::subscribe_unavailable_page(cfg)),
	};
	let token = token_of(query);
	match res!(unsubscribe(db, &token, id)) {
		UnsubOutcome::Done	=> Ok(page::subscribe_unsubscribed_page(cfg)),
		UnsubOutcome::Unknown	=> Ok(page::subscribe_bad_token_page(cfg)),
	}
}

/// The `token=` value out of a raw query substring.
///
/// A token is [`TOKEN_ALPHABET`] -- lowercase letters and digits -- so a value carrying anything a
/// query would percent-encode is a value no token wears, and matches nobody. Read with no decoding, so
/// a `%2e` reaching here stays `%2e` and finds nothing, which is the right answer to a token that does
/// not exist.
fn token_of(query: &str) -> String {
	for pair in query.split('&') {
		let mut kv = pair.splitn(2, '=');
		let k = kv.next().unwrap_or("");
		let v = kv.next().unwrap_or("");
		if k == "token" {
			return v.to_string();
		}
	}
	String::new()
}


#[cfg(test)]
mod tests {
	use super::*;

	/// An address is trimmed and lowercased to one shape, and shape-checked against the obvious wrongs.
	#[test]
	fn test_an_address_is_normalised_and_checked_00() -> Outcome<()> {
		assert_eq!(normalise_email("  Me@Example.COM "), "me@example.com");
		assert!(valid_email("me@example.com"));
		assert!(valid_email("a.b+tag@sub.example.co.uk"));
		assert!(!valid_email(""));
		assert!(!valid_email("no-at-sign"));
		assert!(!valid_email("two@@example.com"));
		assert!(!valid_email("@example.com"));
		assert!(!valid_email("me@"));
		assert!(!valid_email("me@localhost"));		// no dot in the domain
		assert!(!valid_email("me@.com"));
		assert!(!valid_email("me@example."));
		assert!(!valid_email("has space@example.com"));
		assert!(!valid_email(&fmt!("{}@example.com", "x".repeat(EMAIL_MAX))));
		Ok(())
	}

	/// An address literal names no mailbox a stranger should be able to aim this host's mail at.
	#[test]
	fn test_an_address_literal_is_refused_13() -> Outcome<()> {
		assert!(!valid_email("user@[127.0.0.1]"));
		assert!(!valid_email("user@[::1]"));
		assert!(!valid_email("user@127.0.0.1"));	// a bare IPv4 form: no top-level domain is all digits
		assert!(valid_email("user@mail.example.com"));
		assert!(valid_email("user@example.co2"));	// a digit in the last label is still a name
		Ok(())
	}

	/// A subscriber survives the trip through a daticle, with and without a sign-up time.
	#[test]
	fn test_a_subscriber_round_trips_01() -> Outcome<()> {
		let sub = Subscriber {
			email:		fmt!("me@example.com"),
			state:		SubState::Confirmed,
			token:		fmt!("abc123"),
			created:	Some(fmt!("2026-07-18T10:00:00Z")),
			sent:		Some(fmt!("2026-07-18T10:05:00Z")),
		};
		let back = res!(Subscriber::from_dat(&sub.to_dat()));
		assert_eq!(back, sub);

		let undated = Subscriber { created: None, ..sub };
		let back = res!(Subscriber::from_dat(&undated.to_dat()));
		assert_eq!(back, undated);
		assert_eq!(back.created, None);
		Ok(())
	}

	/// A state this version cannot read is pending -- the state that receives no mail -- not confirmed.
	#[test]
	fn test_an_unreadable_state_is_pending_02() -> Outcome<()> {
		assert_eq!(SubState::of("confirmed"), SubState::Confirmed);
		assert_eq!(SubState::of("unsubscribed"), SubState::Unsubscribed);
		assert_eq!(SubState::of("bounced"), SubState::Bounced);
		assert_eq!(SubState::of("something-new"), SubState::Pending);
		Ok(())
	}

	/// A bounced subscriber survives the trip through a daticle, keeping the suppressed state.
	#[test]
	fn test_a_bounced_subscriber_round_trips_09() -> Outcome<()> {
		assert_eq!(SubState::Bounced.as_str(), "bounced");
		let sub = Subscriber {
			email:		fmt!("gone@example.com"),
			state:		SubState::Bounced,
			token:		fmt!("tok"),
			created:	Some(fmt!("2026-07-18T10:00:00Z")),
			sent:		None,
		};
		let back = res!(Subscriber::from_dat(&sub.to_dat()));
		assert_eq!(back, sub);
		assert_eq!(back.state, SubState::Bounced);
		Ok(())
	}

	/// A record with no email is not a subscriber: nothing could address it or key it.
	#[test]
	fn test_a_subscriber_without_an_email_is_refused_03() -> Outcome<()> {
		let d = create_dat_ordmap(vec![(dat!("state"), dat!("confirmed"))]);
		assert!(Subscriber::from_dat(&d).is_err());
		Ok(())
	}

	/// A key is the prefix and the address, so a token's read is index-driven and never a scan.
	#[test]
	fn test_a_key_is_prefixed_04() -> Outcome<()> {
		assert_eq!(key_of("me@example.com"), dat!("publish/subscriber/me@example.com"));
		Ok(())
	}

	/// A token is minted from the small alphabet, at the stated length, and two are not the same.
	#[test]
	fn test_a_token_is_unguessable_shaped_05() -> Outcome<()> {
		let t = mint_token();
		assert_eq!(t.len(), TOKEN_LEN);
		assert!(t.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit()));
		assert_ne!(mint_token(), mint_token(), "two tokens collided");
		Ok(())
	}

	/// The `token=` field is read raw from the query, and a value that would need decoding is taken as
	/// itself -- which matches no token.
	#[test]
	fn test_a_token_is_read_from_the_query_06() -> Outcome<()> {
		assert_eq!(token_of("token=abc123"), "abc123");
		assert_eq!(token_of("a=1&token=xyz"), "xyz");
		assert_eq!(token_of("token="), "");
		assert_eq!(token_of(""), "");
		Ok(())
	}

	/// An address is redacted to its first character and domain for a log, never kept whole there.
	#[test]
	fn test_an_address_is_redacted_for_the_log_07() -> Outcome<()> {
		assert_eq!(redact("jason@oxedyne.com"), "j***@oxedyne.com");
		assert_eq!(redact("not-an-address"), "***");
		Ok(())
	}

	/// The field no person sees catches a fill and forgives whitespace.
	///
	/// Whitespace matters: a browser or a proxy that pads the value must not cost a reader their
	/// sign-up, and a bot that writes anything at all must lose theirs.
	#[test]
	fn test_the_trap_catches_a_fill_10() -> Outcome<()> {
		assert!(!trapped(""));
		assert!(!trapped("   "));
		assert!(!trapped("\t\n"));
		assert!(trapped("http://example.com"));
		assert!(trapped("x"));
		assert!(trapped("  x  "));
		Ok(())
	}

	/// The sign-up limit is policy with a limiting default: a block that names none still limits.
	///
	/// The default matters more than the number. This endpoint sends mail to an address a stranger
	/// chose, so the failure of omission -- a `publish` block written before these fields existed,
	/// loading with no limit at all -- is the one worth designing against.
	#[test]
	fn test_the_signup_limit_defaults_to_limiting_11() -> Outcome<()> {
		use crate::srv::publish::PublishConfig;

		// A block that names nothing about sign-ups still limits them.
		let cfg = res!(PublishConfig::from_datmap(&DaticleMap::new()));
		assert_eq!(cfg.subscribe_rate_secs, 60);
		assert_eq!(cfg.subscribe_rate_hourly, 5);

		// A site that names its own numbers is taken at its word. Written as bare counts, which the
		// grammar types as narrowly as it can -- the shape an operator actually writes.
		let mut m = DaticleMap::new();
		m.insert(dat!("subscribe_rate_secs"), dat!(120u8));
		m.insert(dat!("subscribe_rate_hourly"), dat!(2u8));
		let cfg = res!(PublishConfig::from_datmap(&m));
		assert_eq!(cfg.subscribe_rate_secs, 120);
		assert_eq!(cfg.subscribe_rate_hourly, 2);

		// Off is a decision a site may take, and is distinct from naming nothing.
		let mut off = DaticleMap::new();
		off.insert(dat!("subscribe_rate_secs"), dat!(0u8));
		off.insert(dat!("subscribe_rate_hourly"), dat!(0u8));
		let cfg = res!(PublishConfig::from_datmap(&off));
		assert_eq!(cfg.subscribe_rate_secs, 0);
		assert_eq!(cfg.subscribe_rate_hourly, 0);
		Ok(())
	}

	/// One address hashes to different values for the sign-up counter and the comment counter.
	///
	/// The separation is the point: a value taken from one counter must not be a lookup key for the
	/// other, or the two features become one another's oracle.
	#[test]
	fn test_the_two_counters_do_not_share_a_hash_12() -> Outcome<()> {
		let salt = b"a-site-secret";
		let mine = from_hash("203.0.113.7", salt);
		let theirs = super::super::comment::from_hash("203.0.113.7", salt);
		assert_ne!(mine, theirs);
		// Stable for one address, or a limiter counts every request as a new sender.
		assert_eq!(mine, from_hash("203.0.113.7", salt));
		// And separated by salt, so a value means nothing on another site.
		assert_ne!(mine, from_hash("203.0.113.7", b"another-site"));
		Ok(())
	}

	/// A CSV field carrying a comma or a quote is quoted, so an address never splits a row.
	#[test]
	fn test_a_csv_field_is_quoted_when_it_must_be_08() -> Outcome<()> {
		assert_eq!(csv_field("me@example.com"), "me@example.com");
		assert_eq!(csv_field("a,b@example.com"), "\"a,b@example.com\"");
		assert_eq!(csv_field("a\"b"), "\"a\"\"b\"");
		Ok(())
	}
}
