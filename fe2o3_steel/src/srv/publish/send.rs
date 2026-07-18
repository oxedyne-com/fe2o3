//! Sending a post to the remotes it is bound for.
//!
//! A [`Destination`](super::dest::Destination) names a remote; this module reaches it. Each sender
//! builds a request, makes one HTTPS call through [`https_request`], reads the reply, and returns the
//! permalink the remote gave back -- the address a backlink points at, and the proof the post landed.
//!
//! # Two that need no new machinery
//!
//! Mastodon takes a static bearer token and one POST. Bluesky takes an app password, exchanges it for
//! a session, and posts with the session's token. Neither needs an OAuth client, which is why they are
//! the first two wired: they exercise the whole delivery seam for free. Email waits on a subscriber
//! list that does not exist yet; X and Threads wait on OAuth.
//!
//! # Built pure, wrapped thin
//!
//! The request bodies and the reply parsing are pure functions over strings, tested without a socket.
//! The network wrappers around them are as thin as they can be, because what cannot be exercised in a
//! test against a live remote is exactly what a test cannot catch. What *can* be pinned -- the JSON a
//! remote is sent, the permalink pulled from what it returns -- is.

use crate::srv::publish::{
	dest::{
		Delivery,
		DeliveryState,
		Destination,
		Rendition,
	},
	store,
};

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_iop_crypto::enc::Encrypter;
use oxedyne_fe2o3_iop_db::api::Database;
use oxedyne_fe2o3_iop_hash::api::Hasher;
use oxedyne_fe2o3_jdat::{
	prelude::*,
	id::NumIdDat,
	string::dec::DecoderConfig,
	usr::{
		UsrKind,
		UsrKindCode,
		UsrKindId,
	},
};
use oxedyne_fe2o3_datime::{
	format::rfc9557::Rfc9557Format,
	time::{
		CalClock,
		CalClockZone,
	},
};
use oxedyne_fe2o3_net::http::{
	client::https_request,
	header::{
		HttpHeadline,
		HttpMethod,
	},
	msg::HttpMessage,
};

use std::{
	collections::BTreeMap,
	path::Path,
	sync::{
		Arc,
		RwLock,
	},
	time::{
		SystemTime,
		UNIX_EPOCH,
	},
};

use tokio_rustls::rustls::ClientConfig;


/// The default Bluesky host, where a site's config names none. The public PDS, which is what an app
/// password authenticates against unless a site runs its own.
pub const BLUESKY_HOST_DEFAULT: &str = "bsky.social";


/// A Mastodon account's credentials.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct MastodonCreds {
	/// The instance the account lives on, e.g. `https://mastodon.social`. The scheme is stripped to a
	/// host before it is dialled.
	pub base_url:	String,
	/// The access token, a static bearer. Resolved from a secret reference, never in the config in the
	/// clear.
	pub token:	String,
}

/// A Bluesky account's credentials.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct BlueskyCreds {
	/// The PDS host, e.g. `bsky.social`. Defaults to [`BLUESKY_HOST_DEFAULT`].
	pub host:		String,
	/// The account handle, e.g. `me.bsky.social`, which is the session identifier.
	pub handle:		String,
	/// An app password, not the account password: Bluesky issues these precisely so a third party holds
	/// one and it can be revoked on its own. Resolved from a secret reference.
	pub app_password:	String,
}

/// The remotes a site is configured to post to.
///
/// A destination the site has not configured is one it will not offer and cannot send to, whatever its
/// [`Capability`](super::dest::Capability) says in the abstract: a capability describes what a remote
/// *could* take, this describes which remotes *this* site actually reaches.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DestCreds {
	/// The Mastodon account, where the site has one.
	pub mastodon:	Option<MastodonCreds>,
	/// The Bluesky account, where the site has one.
	pub bluesky:	Option<BlueskyCreds>,
}

impl DestCreds {

	/// Whether the site has credentials for a destination, and so can offer it.
	pub fn has(&self, dest: Destination) -> bool {
		match dest {
			Destination::Mastodon	=> self.mastodon.is_some(),
			Destination::Bluesky	=> self.bluesky.is_some(),
			// The rest are not wired regardless of config.
			_			=> false,
		}
	}

	/// The destinations the site can offer, in a picker's order.
	pub fn offered(&self) -> Vec<Destination> {
		Destination::ALL.iter().copied().filter(|d| self.has(*d)).collect()
	}

	/// These credentials laid over `base`, taking precedence where both name a remote.
	///
	/// How the interactively-entered credentials (in the store) win over the ones in the config file
	/// while still falling back to config for a remote the console has not set. Per-remote, not
	/// all-or-nothing: a site may keep Mastodon in its config and set Bluesky from the console.
	pub fn overlay(self, base: DestCreds) -> DestCreds {
		DestCreds {
			mastodon:	self.mastodon.or(base.mastodon),
			bluesky:	self.bluesky.or(base.bluesky),
		}
	}

	/// The credentials as a daticle, for the store.
	pub fn to_dat(&self) -> Dat {
		let mut m = DaticleMap::new();
		if let Some(x) = &self.mastodon {
			m.insert(dat!("mastodon"), x.to_dat());
		}
		if let Some(x) = &self.bluesky {
			m.insert(dat!("bluesky"), x.to_dat());
		}
		Dat::Map(m)
	}

	/// The credentials from a daticle, leniently: a remote whose block will not read is dropped, not an
	/// error, since a settings page and a delivery must both survive a record a later version wrote or a
	/// half-written one. This is the store's reader; [`from_datmap`](Self::from_datmap) is the config's,
	/// and errors, because a broken *config* is an operator's mistake to be told about.
	pub fn from_dat(d: &Dat) -> Self {
		let m = match d {
			Dat::Map(m)	=> m,
			_		=> return Self::default(),
		};
		Self {
			mastodon:	m.get(&dat!("mastodon")).and_then(MastodonCreds::from_dat),
			bluesky:	m.get(&dat!("bluesky")).and_then(BlueskyCreds::from_dat),
		}
	}

	/// Parses a vhost's `destinations` block: a map of per-remote credential blocks. A remote the site
	/// has no block for is a remote it does not post to, and reads as `None` rather than an error.
	pub fn from_datmap(m: &DaticleMap) -> Outcome<Self> {
		let mastodon = match m.get(&dat!("mastodon")) {
			Some(Dat::Map(mm))	=> Some(res!(MastodonCreds::from_datmap(mm))),
			None			=> None,
			_			=> return Err(err!(
				"publish: 'destinations.mastodon' must be a map.";
				Invalid, Input, Mismatch)),
		};
		let bluesky = match m.get(&dat!("bluesky")) {
			Some(Dat::Map(bm))	=> Some(res!(BlueskyCreds::from_datmap(bm))),
			None			=> None,
			_			=> return Err(err!(
				"publish: 'destinations.bluesky' must be a map.";
				Invalid, Input, Mismatch)),
		};
		Ok(Self { mastodon, bluesky })
	}

	/// Resolves every credential's `{env:}`/`{file:}` secret reference against the app root, so a token
	/// is never in the config in the clear.
	pub fn resolve_secrets(&mut self, root: &Path) -> Outcome<()> {
		if let Some(m) = &mut self.mastodon {
			res!(m.resolve_secrets(root));
		}
		if let Some(b) = &mut self.bluesky {
			res!(b.resolve_secrets(root));
		}
		Ok(())
	}
}

impl MastodonCreds {

	/// Parses a `destinations.mastodon` block. Both fields are required where the block is present: an
	/// account named without an instance or without a token is one no post can reach, and saying so at
	/// load beats a delivery failing at send with the same cause.
	fn from_datmap(m: &DaticleMap) -> Outcome<Self> {
		Ok(Self {
			base_url:	res!(cred_str(m, "base_url", "destinations.mastodon")),
			token:		res!(cred_str(m, "token", "destinations.mastodon")),
		})
	}

	/// Resolves the token's secret reference. The instance URL is public and taken as written.
	fn resolve_secrets(&mut self, root: &Path) -> Outcome<()> {
		self.token = res!(crate::srv::cfg::ApiRoute::resolve_file_refs(&self.token, root));
		Ok(())
	}

	/// The credentials as a daticle, for the store.
	fn to_dat(&self) -> Dat {
		let mut m = DaticleMap::new();
		m.insert(dat!("base_url"),	dat!(self.base_url.clone()));
		m.insert(dat!("token"),		dat!(self.token.clone()));
		Dat::Map(m)
	}

	/// The credentials from a stored daticle, or nothing where either required field is missing.
	fn from_dat(d: &Dat) -> Option<Self> {
		let m = ok!(as_map(d));
		Some(Self {
			base_url:	ok!(nonempty(m, "base_url")),
			token:		ok!(nonempty(m, "token")),
		})
	}
}

impl BlueskyCreds {

	/// Parses a `destinations.bluesky` block. Handle and app password are required; the host defaults to
	/// [`BLUESKY_HOST_DEFAULT`], since most accounts live on the public PDS.
	fn from_datmap(m: &DaticleMap) -> Outcome<Self> {
		let host = match m.get(&dat!("host")) {
			Some(Dat::Str(s)) if !s.trim().is_empty()	=> s.trim().to_string(),
			_						=> BLUESKY_HOST_DEFAULT.to_string(),
		};
		Ok(Self {
			host,
			handle:		res!(cred_str(m, "handle", "destinations.bluesky")),
			app_password:	res!(cred_str(m, "app_password", "destinations.bluesky")),
		})
	}

	/// Resolves the app password's secret reference. The handle and host are public and taken as
	/// written.
	fn resolve_secrets(&mut self, root: &Path) -> Outcome<()> {
		self.app_password = res!(crate::srv::cfg::ApiRoute::resolve_file_refs(&self.app_password, root));
		Ok(())
	}

	/// The credentials as a daticle, for the store.
	fn to_dat(&self) -> Dat {
		let mut m = DaticleMap::new();
		m.insert(dat!("host"),		dat!(self.host.clone()));
		m.insert(dat!("handle"),	dat!(self.handle.clone()));
		m.insert(dat!("app_password"),	dat!(self.app_password.clone()));
		Dat::Map(m)
	}

	/// The credentials from a stored daticle, or nothing where the handle or password is missing. The
	/// host defaults, as it does from config.
	fn from_dat(d: &Dat) -> Option<Self> {
		let m = ok!(as_map(d));
		let host = nonempty(m, "host").unwrap_or_else(|| BLUESKY_HOST_DEFAULT.to_string());
		Some(Self {
			host,
			handle:		ok!(nonempty(m, "handle")),
			app_password:	ok!(nonempty(m, "app_password")),
		})
	}
}

/// A daticle as a map, or nothing.
fn as_map(d: &Dat) -> Option<&DaticleMap> {
	match d {
		Dat::Map(m)	=> Some(m),
		_		=> None,
	}
}

/// A non-empty string field of a map, or nothing.
fn nonempty(m: &DaticleMap, key: &str) -> Option<String> {
	match m.get(&dat!(key)) {
		Some(Dat::Str(s)) if !s.trim().is_empty()	=> Some(s.clone()),
		_						=> None,
	}
}


/// The key a vhost's destination credentials live under in its store.
pub const CREDS_KEY: &str = "publish/creds";

/// The credentials a site has set from the console, from its store. An empty set where none are stored,
/// which is not an error: a site sets its remotes from the console or its config or neither.
pub fn get_creds<
	const UIDL: usize,
	UID:	NumIdDat<UIDL>,
	ENC:	Encrypter,
	KH:	Hasher,
	DB:	Database<UIDL, UID, ENC, KH>,
>(
	db:	&(Arc<RwLock<DB>>, UID),
)
	-> Outcome<DestCreds>
{
	let (db_arc, _) = db;
	let guard = lock_read!(db_arc);
	match res!(guard.get(&dat!(CREDS_KEY), None)) {
		Some((val, _))	=> Ok(DestCreds::from_dat(&val)),
		None		=> Ok(DestCreds::default()),
	}
}

/// Writes a site's console-set credentials to its store, where they are encrypted at rest under the
/// database's own scheme -- the same treatment its posts, sessions and users get, and the reason a
/// token entered here is not a token in a file in the clear.
pub fn put_creds<
	const UIDL: usize,
	UID:	NumIdDat<UIDL>,
	ENC:	Encrypter,
	KH:	Hasher,
	DB:	Database<UIDL, UID, ENC, KH>,
>(
	db:	&(Arc<RwLock<DB>>, UID),
	creds:	&DestCreds,
)
	-> Outcome<()>
{
	let (db_arc, user) = db;
	let guard = lock_read!(db_arc);
	res!(guard.insert(dat!(CREDS_KEY), creds.to_dat(), *user, None));
	Ok(())
}

/// The credentials that actually apply: what the console has set, laid over what the config names, so a
/// remote set interactively wins and one left to the config still works. This is what the picker offers
/// and what a delivery is sent with.
pub fn effective_creds<
	const UIDL: usize,
	UID:	NumIdDat<UIDL>,
	ENC:	Encrypter,
	KH:	Hasher,
	DB:	Database<UIDL, UID, ENC, KH>,
>(
	db:	&(Arc<RwLock<DB>>, UID),
	cfg:	&crate::srv::publish::PublishConfig,
)
	-> Outcome<DestCreds>
{
	Ok(res!(get_creds(db)).overlay(cfg.creds.clone()))
}

/// A required string field of a credential block, named for the block it is missing from.
fn cred_str(m: &DaticleMap, key: &str, block: &str) -> Outcome<String> {
	match m.get(&dat!(key)) {
		Some(Dat::Str(s)) if !s.trim().is_empty()	=> Ok(s.clone()),
		Some(Dat::Str(_)) | None			=> Err(err!(
			"publish: '{}.{}' is required and must be a non-empty string.", block, key;
			Invalid, Input, Missing)),
		_						=> Err(err!(
			"publish: '{}.{}' must be a string.", block, key;
			Invalid, Input, Mismatch)),
	}
}


/// Sends the rendition's words to a destination and returns the permalink the remote gave back.
///
/// The one door every send goes through. A destination the site has no credentials for, or one no
/// sender is written for, is an error and not a silent success: a delivery that reports itself sent
/// when nothing left the building is worse than one that fails honestly.
pub async fn deliver_one(
	dest:	Destination,
	creds:	&DestCreds,
	text:	&str,
	tls:	Arc<ClientConfig>,
)
	-> Outcome<String>
{
	match dest {
		Destination::Mastodon	=> match &creds.mastodon {
			Some(c)	=> mastodon(c, text, tls).await,
			None	=> Err(err!(
				"publish: this site has no Mastodon credentials configured.";
				Input, Missing)),
		},
		Destination::Bluesky	=> match &creds.bluesky {
			Some(c)	=> bluesky(c, text, tls).await,
			None	=> Err(err!(
				"publish: this site has no Bluesky credentials configured.";
				Input, Missing)),
		},
		other	=> Err(err!(
			"publish: no sender is wired for {}.", other.as_str();
			Input, Unknown)),
	}
}


/// Attempts every delivery of a post that is not yet done, and writes back what happened.
///
/// The outbox in one pass. It reads the post, walks its deliveries, and for each one still
/// [`open`](is_open) -- queued, or failed but not past retrying -- sends the rendition and records the
/// outcome: a permalink and the moment on success, the error and a bumped retry count on failure. The
/// record is written back once, at the end, with all the outcomes on it.
///
/// # Held under no lock
///
/// A network is slow and a database lock is not for holding across one. So the post is read, released,
/// sent over the wire, and only then written back. Two saves racing settle last-write-wins, which for a
/// delivery log is a cost worth its simplicity: the worst case re-sends a post, and the backfeed shows
/// it, where holding a lock across a remote that has stopped answering would wedge the site.
///
/// Returns how many deliveries were attempted -- zero where the post has none open, which is the
/// ordinary case for a post already sent everywhere it goes.
pub async fn deliver_post<
	const UIDL: usize,
	UID:	NumIdDat<UIDL>,
	ENC:	Encrypter,
	KH:	Hasher,
	DB:	Database<UIDL, UID, ENC, KH>,
>(
	db:	&(Arc<RwLock<DB>>, UID),
	creds:	&DestCreds,
	tls:	Arc<ClientConfig>,
	slug:	&str,
	id:	&str,
)
	-> Outcome<usize>
{
	let mut rec = match res!(store::get(db, slug)) {
		Some(r)	=> r,
		// A post that is not there has no deliveries to make, and is not an error: it may have been
		// deleted between the queueing and the sweep.
		None	=> return Ok(0),
	};

	let mut attempted = 0;
	for delivery in &mut rec.deliveries {
		if !is_open(&delivery.state) {
			continue;
		}
		attempted += 1;
		let retries = match &delivery.state {
			DeliveryState::Failed { retries, .. }	=> *retries,
			_					=> 0,
		};
		match deliver_one(delivery.dest, creds, &delivery.rendition.text, tls.clone()).await {
			Ok(permalink)	=> {
				let at = res!(iso_now());
				info!("{}: publish: '{}' delivered to {} at {}",
					id, slug, delivery.dest.as_str(), permalink);
				delivery.state = DeliveryState::Sent { at, permalink };
			}
			Err(e)	=> {
				let at = iso_now().unwrap_or_default();
				warn!("{}: publish: '{}' to {} failed (attempt {}): {}",
					id, slug, delivery.dest.as_str(), retries + 1, e);
				delivery.state = DeliveryState::Failed {
					at,
					err:		fmt!("{}", e),
					retries:	retries + 1,
				};
			}
		}
	}

	if attempted > 0 {
		res!(store::put(db, &rec, id));
	}
	Ok(attempted)
}

/// Whether a delivery still wants an attempt: queued, or failed but not yet past retrying. The
/// complement of [`DeliveryState::is_terminal`], named for the loop that reads it.
fn is_open(state: &DeliveryState) -> bool {
	!state.is_terminal()
}

/// Sets a post's deliveries to a fresh queue for the destinations named, keeping a hand-edited
/// rendition where one already exists for a destination and deriving a default where none does.
///
/// What the composer calls when an author ticks destinations and saves. It does not send -- it queues;
/// [`deliver_post`] sends. A destination dropped from the set loses its delivery, and one already sent
/// keeps it, so re-saving does not re-send what has gone: only an open or new delivery is queued.
pub fn queue_deliveries(
	existing:	&[Delivery],
	chosen:		&[Destination],
	title:		&str,
	url:		&str,
) -> Vec<Delivery> {
	let mut out = Vec::new();
	for &dest in chosen {
		let prior = existing.iter().find(|d| d.dest == dest);
		match prior {
			// Already sent, or already carrying a hand-written rendition: keep it as it stands. A
			// re-save must not re-derive over an edit, nor re-open a delivery that has landed.
			Some(d) if matches!(d.state, DeliveryState::Sent { .. }) || !d.rendition.auto	=> {
				out.push(d.clone());
			}
			// Known but still open with an automatic rendition: refresh the rendition (the title or link
			// may have changed) and leave it queued.
			Some(_)	=> {
				let rendition = Rendition::promo(title, url, dest.capability().max_chars);
				out.push(Delivery::new(dest, rendition));
			}
			// New: derive a default and queue it.
			None	=> {
				let rendition = Rendition::promo(title, url, dest.capability().max_chars);
				out.push(Delivery::new(dest, rendition));
			}
		}
	}
	out
}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ MASTODON                                                                  │
// └───────────────────────────────────────────────────────────────────────────┘

/// Posts to Mastodon and returns the status's URL.
///
/// One POST to `/api/v1/statuses`, a JSON body carrying the text, the token as a bearer. The reply is
/// the created status, and its `url` is the permalink.
pub async fn mastodon(
	creds:	&MastodonCreds,
	text:	&str,
	tls:	Arc<ClientConfig>,
)
	-> Outcome<String>
{
	let host = host_of(&creds.base_url);
	let body = res!(mastodon_status_body(text));
	let auth = fmt!("Bearer {}", creds.token);
	let headers = [
		("Authorization",	auth.as_str()),
		("Content-Type",	"application/json"),
		("Accept",		"application/json"),
	];
	let resp = res!(https_request(
		&host, 443, HttpMethod::POST, "/api/v1/statuses", &headers, body.as_bytes(), tls,
	).await);
	let code = status_code(&resp);
	let payload = resp.body_as_string().into_owned();
	if !is_success(code) {
		return Err(err!(
			"Mastodon at {} refused the post with {}: {}", host, code, payload;
			Network, Data));
	}
	let m = res!(parse_json(&payload));
	match json_str(&m, "url") {
		Some(u)	=> Ok(u),
		None	=> Err(err!(
			"Mastodon accepted the post but returned no url: {}", payload;
			Network, Data, Missing)),
	}
}

/// The JSON body for a Mastodon status: `{"status": "<text>"}`.
fn mastodon_status_body(text: &str) -> Outcome<String> {
	let mut m = DaticleMap::new();
	m.insert(dat!("status"), dat!(text.to_string()));
	Dat::Map(m).json()
}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ BLUESKY                                                                   │
// └───────────────────────────────────────────────────────────────────────────┘

/// Posts to Bluesky and returns a link to the post.
///
/// Two calls: `createSession` exchanges the app password for a session token and the account's DID,
/// then `createRecord` writes the post under that DID. The reply is an `at://` URI, which
/// [`at_uri_to_url`] turns into the `bsky.app` address a person can open.
pub async fn bluesky(
	creds:	&BlueskyCreds,
	text:	&str,
	tls:	Arc<ClientConfig>,
)
	-> Outcome<String>
{
	let host = if creds.host.trim().is_empty() {
		BLUESKY_HOST_DEFAULT.to_string()
	} else {
		creds.host.trim().to_string()
	};

	// 1. Exchange the app password for a session.
	let sess_body = res!(bluesky_session_body(&creds.handle, &creds.app_password));
	let headers = [
		("Content-Type",	"application/json"),
		("Accept",		"application/json"),
	];
	let resp = res!(https_request(
		&host, 443, HttpMethod::POST, "/xrpc/com.atproto.server.createSession",
		&headers, sess_body.as_bytes(), tls.clone(),
	).await);
	let code = status_code(&resp);
	let payload = resp.body_as_string().into_owned();
	if !is_success(code) {
		return Err(err!(
			"Bluesky at {} refused the session with {}: {}", host, code, payload;
			Network, Data));
	}
	let sm = res!(parse_json(&payload));
	let jwt = match json_str(&sm, "accessJwt") {
		Some(j)	=> j,
		None	=> return Err(err!(
			"Bluesky opened a session but returned no accessJwt: {}", payload;
			Network, Data, Missing)),
	};
	let did = match json_str(&sm, "did") {
		Some(d)	=> d,
		None	=> return Err(err!(
			"Bluesky opened a session but returned no did: {}", payload;
			Network, Data, Missing)),
	};

	// 2. Write the post under the account's DID.
	let created = res!(iso_now());
	let rec_body = res!(bluesky_record_body(&did, text, &created));
	let auth = fmt!("Bearer {}", jwt);
	let headers2 = [
		("Authorization",	auth.as_str()),
		("Content-Type",	"application/json"),
		("Accept",		"application/json"),
	];
	let resp2 = res!(https_request(
		&host, 443, HttpMethod::POST, "/xrpc/com.atproto.repo.createRecord",
		&headers2, rec_body.as_bytes(), tls,
	).await);
	let code2 = status_code(&resp2);
	let payload2 = resp2.body_as_string().into_owned();
	if !is_success(code2) {
		return Err(err!(
			"Bluesky at {} refused the post with {}: {}", host, code2, payload2;
			Network, Data));
	}
	let rm = res!(parse_json(&payload2));
	let uri = match json_str(&rm, "uri") {
		Some(u)	=> u,
		None	=> return Err(err!(
			"Bluesky accepted the post but returned no uri: {}", payload2;
			Network, Data, Missing)),
	};
	match at_uri_to_url(&uri) {
		Some(url)	=> Ok(url),
		// The post is up, but its address is not the shape this understands. Better to say so and keep
		// the at-uri than to claim a bsky.app link that may not resolve.
		None		=> Ok(uri),
	}
}

/// The JSON body for `createSession`: `{"identifier": "<handle>", "password": "<app password>"}`.
fn bluesky_session_body(handle: &str, app_password: &str) -> Outcome<String> {
	let mut m = DaticleMap::new();
	m.insert(dat!("identifier"),	dat!(handle.to_string()));
	m.insert(dat!("password"),	dat!(app_password.to_string()));
	Dat::Map(m).json()
}

/// The JSON body for `createRecord`: a feed post under the account's repo, stamped with the moment it
/// was written, which Bluesky requires and orders timelines by.
fn bluesky_record_body(did: &str, text: &str, created_at: &str) -> Outcome<String> {
	let mut record = DaticleMap::new();
	record.insert(dat!("$type"),		dat!("app.bsky.feed.post".to_string()));
	record.insert(dat!("text"),		dat!(text.to_string()));
	record.insert(dat!("createdAt"),	dat!(created_at.to_string()));

	let mut outer = DaticleMap::new();
	outer.insert(dat!("repo"),		dat!(did.to_string()));
	outer.insert(dat!("collection"),	dat!("app.bsky.feed.post".to_string()));
	outer.insert(dat!("record"),		Dat::Map(record));
	Dat::Map(outer).json()
}

/// A `bsky.app` link from the `at://` URI a write returns.
///
/// `at://<did>/app.bsky.feed.post/<rkey>` becomes
/// `https://bsky.app/profile/<did>/post/<rkey>`, which is the address a person opens. A URI that is
/// not that shape yields nothing, and the caller keeps the URI rather than inventing a link.
fn at_uri_to_url(uri: &str) -> Option<String> {
	let rest = ok!(uri.strip_prefix("at://"));
	let mut parts = rest.splitn(3, '/');
	let did = ok!(parts.next());
	let _collection = ok!(parts.next());
	let rkey = ok!(parts.next());
	if did.is_empty() || rkey.is_empty() {
		return None;
	}
	Some(fmt!("https://bsky.app/profile/{}/post/{}", did, rkey))
}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ SHARED HELPERS                                                            │
// └───────────────────────────────────────────────────────────────────────────┘

/// The host a base URL names, without its scheme or a trailing slash, for dialling.
fn host_of(base_url: &str) -> String {
	let s = base_url.trim();
	let s = s.strip_prefix("https://")
		.or_else(|| s.strip_prefix("http://"))
		.unwrap_or(s);
	s.trim_end_matches('/').to_string()
}

/// The numeric status of a response, or zero where the message is somehow not a response.
fn status_code(msg: &HttpMessage) -> u16 {
	match &msg.header.headline {
		HttpHeadline::Response { status }	=> *status as u16,
		_					=> 0,
	}
}

/// Whether a status is a 2xx.
fn is_success(code: u16) -> bool {
	(200..300).contains(&code)
}

/// A JSON object as a daticle map.
fn parse_json(text: &str) -> Outcome<DaticleMap> {
	let cfg: DecoderConfig<
		BTreeMap<UsrKindCode, UsrKind>,
		BTreeMap<String, UsrKindId>,
	> = DecoderConfig::json(None);
	let dat = res!(Dat::decode_string_with_config(text.to_string(), &cfg));
	match dat {
		Dat::Map(m)	=> Ok(m),
		other		=> Err(err!(
			"expected a JSON object, got {:?}", other.kind();
			Input, Decode, Mismatch)),
	}
}

/// A string field of a JSON object, where it holds one.
fn json_str(m: &DaticleMap, key: &str) -> Option<String> {
	match m.get(&dat!(key)) {
		Some(Dat::Str(s))	=> Some(s.clone()),
		_			=> None,
	}
}

/// The current moment as an RFC 3339 timestamp in UTC.
///
/// Unlike a post's date -- a day, which needs no clock and no calendar (the reason the feed is Atom) --
/// a delivery is an instant on a network, and the remote wants it stamped. So here the module does
/// reach for a clock and for [`CalClock`], which is the fe2o3 calendar and the right tool for turning
/// a unix second into a date a remote will accept.
pub fn iso_now() -> Outcome<String> {
	let secs = match SystemTime::now().duration_since(UNIX_EPOCH) {
		Ok(d)	=> d.as_secs() as i64,
		Err(_)	=> 0,
	};
	iso_of(secs)
}

/// A unix second as an RFC 3339 timestamp in UTC.
pub fn iso_of(unix_secs: i64) -> Outcome<String> {
	let cc = res!(CalClock::from_unix_timestamp_seconds(unix_secs, CalClockZone::utc()));
	cc.to_rfc9557_basic()
}


#[cfg(test)]
mod tests {
	use super::*;

	/// A base URL becomes a bare host, whatever scheme or trailing slash it wore.
	#[test]
	fn test_a_base_url_becomes_a_host_00() -> Outcome<()> {
		assert_eq!(host_of("https://mastodon.social"), "mastodon.social");
		assert_eq!(host_of("https://mastodon.social/"), "mastodon.social");
		assert_eq!(host_of("  http://example.test/  "), "example.test");
		assert_eq!(host_of("example.test"), "example.test");
		Ok(())
	}

	/// The Mastodon body carries the text under `status`, and reads back as that text.
	#[test]
	fn test_a_mastodon_body_carries_the_text_01() -> Outcome<()> {
		let body = res!(mastodon_status_body("On rent https://x/asides/on-rent"));
		let m = res!(parse_json(&body));
		assert_eq!(json_str(&m, "status").as_deref(), Some("On rent https://x/asides/on-rent"));
		Ok(())
	}

	/// The Bluesky session body names the handle and the password where Bluesky looks for them.
	#[test]
	fn test_a_bluesky_session_body_names_the_account_02() -> Outcome<()> {
		let body = res!(bluesky_session_body("me.bsky.social", "app-pw-1234"));
		let m = res!(parse_json(&body));
		assert_eq!(json_str(&m, "identifier").as_deref(), Some("me.bsky.social"));
		assert_eq!(json_str(&m, "password").as_deref(), Some("app-pw-1234"));
		Ok(())
	}

	/// The Bluesky record body is a feed post under the repo, with the text and a timestamp.
	#[test]
	fn test_a_bluesky_record_body_is_a_feed_post_03() -> Outcome<()> {
		let body = res!(bluesky_record_body("did:plc:abc", "On rent", "2026-07-18T10:00:00Z"));
		let m = res!(parse_json(&body));
		assert_eq!(json_str(&m, "repo").as_deref(), Some("did:plc:abc"));
		assert_eq!(json_str(&m, "collection").as_deref(), Some("app.bsky.feed.post"));
		let record = match m.get(&dat!("record")) {
			Some(Dat::Map(r))	=> r,
			other			=> return Err(err!(
				"the record must be a map, got {:?}", other; Test, Mismatch)),
		};
		assert_eq!(json_str(record, "text").as_deref(), Some("On rent"));
		assert_eq!(json_str(record, "$type").as_deref(), Some("app.bsky.feed.post"));
		assert_eq!(json_str(record, "createdAt").as_deref(), Some("2026-07-18T10:00:00Z"));
		Ok(())
	}

	/// An at-uri becomes a bsky.app link; a URI of the wrong shape becomes nothing.
	#[test]
	fn test_an_at_uri_becomes_a_link_04() -> Outcome<()> {
		assert_eq!(
			at_uri_to_url("at://did:plc:abc/app.bsky.feed.post/3krxy"),
			Some(fmt!("https://bsky.app/profile/did:plc:abc/post/3krxy")),
		);
		assert_eq!(at_uri_to_url("https://not-an-at-uri"), None);
		assert_eq!(at_uri_to_url("at://did:plc:abc/app.bsky.feed.post/"), None);
		Ok(())
	}

	/// A remote's reply is read back for the field a permalink lives in.
	#[test]
	fn test_a_permalink_is_read_from_a_reply_05() -> Outcome<()> {
		let mastodon_reply = r#"{"id":"1","url":"https://mastodon.social/@me/1","content":"x"}"#;
		let m = res!(parse_json(mastodon_reply));
		assert_eq!(json_str(&m, "url").as_deref(), Some("https://mastodon.social/@me/1"));

		let bluesky_reply = r#"{"uri":"at://did:plc:abc/app.bsky.feed.post/3k","cid":"bafy"}"#;
		let m = res!(parse_json(bluesky_reply));
		assert_eq!(json_str(&m, "uri").as_deref(), Some("at://did:plc:abc/app.bsky.feed.post/3k"));
		Ok(())
	}

	/// A unix second becomes an RFC 3339 UTC timestamp the epoch pins.
	#[test]
	fn test_a_unix_second_becomes_a_timestamp_06() -> Outcome<()> {
		// The epoch itself.
		let s = res!(iso_of(0));
		assert!(s.starts_with("1970-01-01T00:00:00"), "got: {}", s);
		// A known instant: 2026-07-18T10:00:00Z is 1_784_368_800.
		let s = res!(iso_of(1_784_368_800));
		assert!(s.starts_with("2026-07-18T10:00:00"), "got: {}", s);
		Ok(())
	}

	/// Queueing derives a default for a new destination and keeps a hand-edited or already-sent one.
	#[test]
	fn test_queueing_keeps_edits_and_sends_08() -> Outcome<()> {
		let sent = Delivery {
			dest:		Destination::Mastodon,
			rendition:	Rendition { text: fmt!("old"), auto: true },
			state:		DeliveryState::Sent { at: fmt!("t"), permalink: fmt!("https://m/1") },
		};
		let edited = Delivery {
			dest:		Destination::Bluesky,
			rendition:	Rendition { text: fmt!("my words"), auto: false },
			state:		DeliveryState::Queued,
		};
		let existing = [sent.clone(), edited.clone()];
		let out = queue_deliveries(
			&existing,
			&[Destination::Mastodon, Destination::Bluesky],
			"On rent",
			"https://x/asides/on-rent",
		);
		// The sent one is untouched -- not re-derived, not re-opened.
		let m = match out.iter().find(|d| d.dest == Destination::Mastodon) {
			Some(d)	=> d,
			None	=> return Err(err!("Mastodon delivery was dropped"; Test, Missing)),
		};
		assert_eq!(m.state, sent.state);
		assert_eq!(m.rendition.text, "old");
		// The hand-edited one keeps its words.
		let b = match out.iter().find(|d| d.dest == Destination::Bluesky) {
			Some(d)	=> d,
			None	=> return Err(err!("Bluesky delivery was dropped"; Test, Missing)),
		};
		assert_eq!(b.rendition.text, "my words");
		assert!(!b.rendition.auto);
		Ok(())
	}

	/// A destination dropped from the set loses its delivery; a new one gets a derived default, queued.
	#[test]
	fn test_queueing_drops_the_unchosen_and_adds_the_new_09() -> Outcome<()> {
		let existing = [Delivery::new(Destination::Mastodon, Rendition::default())];
		// Choose only Bluesky: Mastodon is dropped, Bluesky is new.
		let out = queue_deliveries(&existing, &[Destination::Bluesky], "On rent", "https://x/on-rent");
		assert_eq!(out.len(), 1);
		assert_eq!(out[0].dest, Destination::Bluesky);
		assert_eq!(out[0].state, DeliveryState::Queued);
		assert!(out[0].rendition.auto);
		assert!(out[0].rendition.text.ends_with("https://x/on-rent"));
		Ok(())
	}

	/// A destinations block parses into per-remote creds, defaulting the Bluesky host and reading a
	/// remote it does not name as absent.
	#[test]
	fn test_a_destinations_block_parses_10() -> Outcome<()> {
		let mut masto = DaticleMap::new();
		masto.insert(dat!("base_url"),		dat!("https://mastodon.social"));
		masto.insert(dat!("token"),		dat!("secret-token"));
		let mut bsky = DaticleMap::new();
		bsky.insert(dat!("handle"),		dat!("me.bsky.social"));
		bsky.insert(dat!("app_password"),	dat!("app-pw"));
		let mut dests = DaticleMap::new();
		dests.insert(dat!("mastodon"),	Dat::Map(masto));
		dests.insert(dat!("bluesky"),	Dat::Map(bsky));

		let creds = res!(DestCreds::from_datmap(&dests));
		let m = match &creds.mastodon {
			Some(m)	=> m,
			None	=> return Err(err!("mastodon creds did not parse"; Test, Missing)),
		};
		assert_eq!(m.base_url, "https://mastodon.social");
		assert_eq!(m.token, "secret-token");
		let b = match &creds.bluesky {
			Some(b)	=> b,
			None	=> return Err(err!("bluesky creds did not parse"; Test, Missing)),
		};
		assert_eq!(b.handle, "me.bsky.social");
		// The host defaulted, since the block named none.
		assert_eq!(b.host, BLUESKY_HOST_DEFAULT);
		assert_eq!(creds.offered(), vec![Destination::Mastodon, Destination::Bluesky]);
		Ok(())
	}

	/// Credentials survive the round-trip through the store's daticle, and a half-written remote is
	/// dropped rather than read as broken.
	#[test]
	fn test_creds_round_trip_through_the_store_12() -> Outcome<()> {
		let creds = DestCreds {
			mastodon:	Some(MastodonCreds {
				base_url:	fmt!("https://mastodon.social"),
				token:		fmt!("tok"),
			}),
			bluesky:	Some(BlueskyCreds {
				host:		fmt!("bsky.social"),
				handle:		fmt!("me.bsky.social"),
				app_password:	fmt!("pw"),
			}),
		};
		let back = DestCreds::from_dat(&creds.to_dat());
		assert_eq!(back, creds);

		// A Mastodon block with no token is not half-read; it is dropped.
		let mut mm = DaticleMap::new();
		mm.insert(dat!("base_url"), dat!("https://m"));
		let mut m = DaticleMap::new();
		m.insert(dat!("mastodon"), Dat::Map(mm));
		let back = DestCreds::from_dat(&Dat::Map(m));
		assert_eq!(back.mastodon, None);
		Ok(())
	}

	/// Console-set credentials win over config, per remote, and config fills the rest.
	#[test]
	fn test_console_creds_overlay_config_13() -> Outcome<()> {
		let config = DestCreds {
			mastodon:	Some(MastodonCreds { base_url: fmt!("https://cfg"), token: fmt!("cfg-tok") }),
			bluesky:	Some(BlueskyCreds {
				host: fmt!("bsky.social"), handle: fmt!("cfg"), app_password: fmt!("cfg-pw"),
			}),
		};
		// The console set only Mastodon.
		let console = DestCreds {
			mastodon:	Some(MastodonCreds { base_url: fmt!("https://con"), token: fmt!("con-tok") }),
			bluesky:	None,
		};
		let eff = console.overlay(config);
		// Mastodon is the console's; Bluesky falls back to config.
		let m = match &eff.mastodon { Some(m) => m, None => return Err(err!("no mastodon"; Test, Missing)) };
		assert_eq!(m.token, "con-tok");
		let b = match &eff.bluesky { Some(b) => b, None => return Err(err!("no bluesky"; Test, Missing)) };
		assert_eq!(b.handle, "cfg");
		Ok(())
	}

	/// A credential block missing a required field is refused at load, not left to fail at send.
	#[test]
	fn test_a_missing_credential_is_refused_at_load_11() -> Outcome<()> {
		let mut masto = DaticleMap::new();
		masto.insert(dat!("base_url"), dat!("https://mastodon.social"));
		// No token.
		let mut dests = DaticleMap::new();
		dests.insert(dat!("mastodon"), Dat::Map(masto));
		assert!(DestCreds::from_datmap(&dests).is_err());
		Ok(())
	}

	/// Only the wired destinations report themselves configured, and only when creds are present.
	#[test]
	fn test_creds_gate_which_destinations_are_offered_07() -> Outcome<()> {
		let creds = DestCreds {
			mastodon:	Some(MastodonCreds::default()),
			bluesky:	None,
		};
		assert!(creds.has(Destination::Mastodon));
		assert!(!creds.has(Destination::Bluesky));
		assert!(!creds.has(Destination::Email));
		assert!(!creds.has(Destination::X));
		Ok(())
	}
}
