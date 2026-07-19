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

use crate::srv::publish::{
	PublishConfig,
	send::{
		self,
		MailSender,
	},
	page,
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
	http::msg::HttpMessage,
	smtp::client::is_permanent,
};

use std::sync::{
	Arc,
	RwLock,
};


/// The key every subscriber's key begins with.
pub const KEY_PREFIX: &str = "publish/subscriber/";

/// The key the list of subscriber emails lives under.
pub const INDEX_KEY: &str = "publish/subscribers";

/// The longest an address the form will take may be.
///
/// A generous ceiling: the number is arbitrary, having one -- so a form cannot hand the store an
/// unbounded key -- is not.
pub const EMAIL_MAX: usize = 254;

/// How many characters an opt-in token carries.
///
/// Drawn from [`TOKEN_ALPHABET`], so 32 characters of a 36-symbol alphabet is a little over 165 bits:
/// far past guessing. The token is the only thing that confirms or unsubscribes an address, so it is
/// the one field here that must be unguessable.
pub const TOKEN_LEN: usize = 32;

/// The alphabet an opt-in token is drawn from: lowercase letters and digits.
///
/// Deliberately URL-safe and needing no encoding, so the token sits in a `?token=` query and in a
/// database key as itself, exactly as a slug's small alphabet does.
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
	/// Signed up, sent a confirmation link, not yet confirmed. Receives nothing but that one link.
	#[default]
	Pending,
	/// Followed the confirmation link. The one state that receives the newsletter.
	Confirmed,
	/// Asked to stop. Kept as a record, so re-subscribing opts in afresh rather than resurrecting.
	Unsubscribed,
	/// Suppressed after a permanent delivery failure -- a 5xx, an unknown mailbox. Kept as a record and
	/// never sent to again: a re-subscribe does not resurrect it, since the address bounced for a reason
	/// no opt-in changes.
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
	/// The address, normalised: trimmed and lowercased, so one address is one row.
	pub email:	String,
	/// Where they are in the double opt-in.
	pub state:	SubState,
	/// The unguessable token that confirms or unsubscribes them. Minted fresh on each sign-up.
	pub token:	String,
	/// When they signed up, as an ISO timestamp, where it is known.
	pub created:	Option<String>,
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
		Dat::Map(m)
	}

	/// The subscriber from a daticle.
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
		let created = match m.get(&dat!("created")) {
			Some(Dat::Str(s))	=> Some(s.clone()),
			_			=> None,
		};
		Ok(Self {
			email,
			state:	SubState::of(&get_str("state")),
			token:	get_str("token"),
			created,
		})
	}
}


/// A subscriber's key.
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
/// carries a dot and is not a bare label, no whitespace, and within [`EMAIL_MAX`]. The point is to
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
	true
}

/// A fresh, unguessable opt-in token.
pub fn mint_token() -> String {
	Rand::generate_random_string(TOKEN_LEN, TOKEN_ALPHABET)
}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ STORE                                                                     │
// └───────────────────────────────────────────────────────────────────────────┘

/// Reads one subscriber by address.
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
	match res!(guard.get(&key_of(email), None)) {
		Some((val, _))	=> Ok(Some(res!(Subscriber::from_dat(&val)))),
		None		=> Ok(None),
	}
}

/// Writes a subscriber, adding it to the index if it is new.
fn put<
	const UIDL: usize,
	UID:	NumIdDat<UIDL>,
	ENC:	Encrypter,
	KH:	Hasher,
	DB:	Database<UIDL, UID, ENC, KH>,
>(
	db:	&(Arc<RwLock<DB>>, UID),
	sub:	&Subscriber,
)
	-> Outcome<()>
{
	let (db_arc, user) = db;
	{
		let guard = lock_read!(db_arc);
		res!(guard.insert(key_of(&sub.email), sub.to_dat(), *user, None));
	}
	let mut emails = res!(index(db));
	if !emails.iter().any(|e| e == &sub.email) {
		emails.push(sub.email.clone());
		res!(put_index(db, &emails));
	}
	Ok(())
}

/// The index: every subscriber's address.
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
	let val = match res!(guard.get(&dat!(INDEX_KEY), None)) {
		Some((v, _))	=> v,
		// No index is a list nobody has subscribed to, not an error -- the empty list it never wrote.
		None		=> return Ok(Vec::new()),
	};
	let items = match &val {
		Dat::List(items)	=> items.clone(),
		Dat::Vek(vek)		=> vek.as_slice().to_vec(),
		_			=> return Err(err!(
			"publish: the subscriber index must be a list, not {:?}.", val.kind();
			Invalid, Input, Mismatch)),
	};
	let mut out = Vec::new();
	for item in &items {
		if let Dat::Str(s) = item {
			out.push(s.clone());
		}
	}
	Ok(out)
}

/// Writes the index.
fn put_index<
	const UIDL: usize,
	UID:	NumIdDat<UIDL>,
	ENC:	Encrypter,
	KH:	Hasher,
	DB:	Database<UIDL, UID, ENC, KH>,
>(
	db:	&(Arc<RwLock<DB>>, UID),
	emails:	&[String],
)
	-> Outcome<()>
{
	let (db_arc, user) = db;
	let list = Dat::List(emails.iter().map(|e| dat!(e.clone())).collect());
	let guard = lock_read!(db_arc);
	res!(guard.insert(dat!(INDEX_KEY), list, *user, None));
	Ok(())
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
/// - An address already **pending** is re-issued a fresh token and re-sent -- the earlier link may be
///   lost -- and `Some(subscriber)` is returned.
/// - An address already **confirmed** is left exactly as it is and `None` is returned: it is on the
///   list, and re-confirming it would be a second welcome to someone who never left.
/// - An address **bounced** is left suppressed and `None` is returned: a permanent failure marked it,
///   and a re-subscribe must not resurrect an address the mail server said does not exist.
///
/// The caller answers the same page whichever it gets, so the form never reveals which case it was.
pub fn add_pending<
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
	let email = normalise_email(email);
	if !valid_email(&email) {
		return Err(err!(
			"publish: {} is not a shape an address takes.", redact(&email);
			Invalid, Input));
	}
	// An address already confirmed is on the list; do not welcome it twice. A bounced address is
	// suppressed and stays so -- a re-subscribe does not undo a permanent failure. Neither leaks that it
	// is known, since the caller shows the same page whether `Some` or `None` comes back.
	if let Some(existing) = res!(get(db, &email)) {
		match existing.state {
			SubState::Confirmed | SubState::Bounced	=> return Ok(None),
			_					=> {}
		}
	}
	let sub = Subscriber {
		email:		email.clone(),
		state:		SubState::Pending,
		token:		mint_token(),
		created:	send::iso_now().ok(),
	};
	res!(put(db, &sub));
	Ok(Some(sub))
}

/// What a confirmation link found when it was followed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConfirmOutcome {
	/// A pending subscriber was promoted to confirmed: the newsletter now reaches them.
	Confirmed,
	/// The token named a subscriber already confirmed. The safe, idempotent answer to a link followed
	/// twice: they are on the list, said so, and nothing changed.
	Already,
	/// The token named nobody: it is malformed, expired by a re-subscribe that minted a new one, or
	/// never existed.
	Unknown,
}

/// Promotes a pending subscriber to confirmed, by their token.
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
	let mut sub = match res!(find_by_token(db, token, id)) {
		Some(s)	=> s,
		None	=> return Ok(ConfirmOutcome::Unknown),
	};
	match sub.state {
		SubState::Confirmed	=> Ok(ConfirmOutcome::Already),
		_			=> {
			sub.state = SubState::Confirmed;
			res!(put(db, &sub));
			info!("{}: publish: {} confirmed their subscription", id, redact(&sub.email));
			Ok(ConfirmOutcome::Confirmed)
		}
	}
}

/// What an unsubscribe link found when it was followed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UnsubOutcome {
	/// A subscriber was set unsubscribed, or was already, so no more mail reaches them either way.
	Done,
	/// The token named nobody.
	Unknown,
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
	let mut sub = match res!(find_by_token(db, token, id)) {
		Some(s)	=> s,
		None	=> return Ok(UnsubOutcome::Unknown),
	};
	if sub.state != SubState::Unsubscribed {
		sub.state = SubState::Unsubscribed;
		res!(put(db, &sub));
		info!("{}: publish: {} unsubscribed", id, redact(&sub.email));
	}
	Ok(UnsubOutcome::Done)
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
	let mut sub = match res!(get(db, &email)) {
		Some(s)	=> s,
		None	=> return Ok(false),
	};
	if sub.state != SubState::Unsubscribed {
		sub.state = SubState::Unsubscribed;
		res!(put(db, &sub));
		info!("{}: publish: {} unsubscribed by an admin", id, redact(&sub.email));
	}
	Ok(true)
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
	let mut sub = match res!(get(db, &email)) {
		Some(s)	=> s,
		None	=> return Ok(false),
	};
	if sub.state != SubState::Bounced {
		sub.state = SubState::Bounced;
		res!(put(db, &sub));
		warn!("{}: publish: {} suppressed after a permanent delivery failure", id, redact(&sub.email));
	}
	Ok(true)
}

/// Erases a subscriber outright: the record and its place in the index both, by their address.
///
/// A GDPR erasure, distinct from [`unsubscribe_email`]: an unsubscribe keeps the record so a re-subscribe
/// opts in afresh, whereas this leaves nothing behind -- no state, no token, no row in the count. Mirrors
/// [`super::store::delete`]: the key is deleted and the address filtered out of the index, so a listing
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
	// Whether the address was really there, read by key so a tombstone reads as absent -- unlike the
	// database's own `delete`, which marks a key for deletion and reports success even for one already
	// gone. So a repeat erase honestly says there was nothing to erase.
	let existed = res!(get(db, &email)).is_some();
	let (db_arc, user) = db;
	{
		let guard = lock_read!(db_arc);
		res!(guard.delete(&key_of(&email), *user, None));
	}
	let emails = res!(index(db));
	let kept: Vec<String> = emails.into_iter().filter(|e| e != &email).collect();
	res!(put_index(db, &kept));
	if existed {
		info!("{}: publish: {} erased from the list by an admin", id, redact(&email));
	}
	Ok(existed)
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

/// Records a pending sign-up and sends the confirmation, for a `POST {path}/subscribe`.
///
/// Always answers the same "check your inbox" page, whether the address was new, pending or already
/// confirmed, so nothing here reveals whether an address is on the list. Where mail is not configured,
/// or the site has no canonical origin to build an absolute confirmation link from, it says the
/// newsletter is not set up rather than storing a pending subscriber it can never confirm.
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
	body:	&[u8],
	id:	&str,
)
	-> Outcome<HttpMessage>
{
	let db = match db {
		Some(db)	=> db,
		None		=> return Ok(page::subscribe_unavailable_page(cfg)),
	};
	// The newsletter needs a sender to post the confirmation, and an absolute origin to build the link
	// it carries. Missing either, the honest answer is that signup is not available -- not a pending row
	// that will wait for a confirmation nothing can send.
	let sender = match mail {
		Some(m)	=> m,
		None	=> return Ok(page::subscribe_unavailable_page(cfg)),
	};
	if cfg.base_url.is_empty() {
		warn!("{}: publish: a subscribe arrived but the site has no base_url for a confirm link", id);
		return Ok(page::subscribe_unavailable_page(cfg));
	}

	let email = crate::srv::console::form_field(body, "email").unwrap_or_default();
	let email = normalise_email(&email);
	// A plainly malformed address is told so on its own page: that reveals nothing about the list, only
	// about what was typed.
	if !valid_email(&email) {
		return Ok(page::subscribe_invalid_page(cfg));
	}

	match res!(add_pending(db, &email)) {
		// New or pending: send the confirmation. A send that fails is logged, and the reader still gets
		// the same page -- retrying the form re-sends, and saying "we could not email you" would leak
		// that the address was actionable.
		Some(sub)	=> {
			let url = cfg.url_of(&cfg.confirm_path(&sub.token));
			let from = cfg.newsletter_from(sender);
			match sender.send_confirmation(&from, &sub.email, &url, &cfg.site_name).await {
				Ok(_)	=> info!("{}: publish: confirmation sent to {}", id, redact(&sub.email)),
				// A permanent failure means the address does not exist; suppress it so a retry of the form
				// does not keep mailing a mailbox the server has refused. A transient failure is left to be
				// retried by the form, exactly as before.
				Err(e) if is_permanent(&e)	=> {
					warn!("{}: publish: confirmation to {} failed permanently; suppressing: {}",
						id, redact(&sub.email), e);
					if let Err(e2) = mark_bounced(db, &sub.email, id) {
						warn!("{}: publish: could not suppress {}: {}", id, redact(&sub.email), e2);
					}
				}
				Err(e)	=> warn!("{}: publish: confirmation to {} did not send: {}",
					id, redact(&sub.email), e),
			}
		}
		// Already confirmed: send nothing, and answer identically.
		None		=> debug!("{}: publish: subscribe for an address already on the list", id),
	}

	Ok(page::subscribe_sent_page(cfg))
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

	/// A subscriber survives the trip through a daticle, with and without a sign-up time.
	#[test]
	fn test_a_subscriber_round_trips_01() -> Outcome<()> {
		let sub = Subscriber {
			email:		fmt!("me@example.com"),
			state:		SubState::Confirmed,
			token:		fmt!("abc123"),
			created:	Some(fmt!("2026-07-18T10:00:00Z")),
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

	/// A CSV field carrying a comma or a quote is quoted, so an address never splits a row.
	#[test]
	fn test_a_csv_field_is_quoted_when_it_must_be_08() -> Outcome<()> {
		assert_eq!(csv_field("me@example.com"), "me@example.com");
		assert_eq!(csv_field("a,b@example.com"), "\"a,b@example.com\"");
		assert_eq!(csv_field("a\"b"), "\"a\"\"b\"");
		Ok(())
	}
}
