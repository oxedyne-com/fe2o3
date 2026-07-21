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
	Post,
	PostState,
	PublishConfig,
	dest::{
		Delivery,
		DeliveryState,
		Destination,
		Rendition,
	},
	store,
	subscribe,
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
	string::dec::DecoderConfig,
	usr::{
		UsrKind,
		UsrKindCode,
		UsrKindId,
	},
};
use oxedyne_fe2o3_datime::{
	constant::DayOfWeek,
	format::rfc9557::Rfc9557Format,
	time::{
		CalClock,
		CalClockZone,
	},
};
use oxedyne_fe2o3_net::{
	dkim::DkimSigner,
	http::{
		client::https_request,
		header::{
			HttpHeadline,
			HttpMethod,
		},
		msg::HttpMessage,
	},
	smtp::client::{
		OutboundClient,
		is_permanent,
	},
};
use oxedyne_fe2o3_text::doc::html::{
	escape_attr,
	escape_text,
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


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ EMAIL: the site's own DKIM sender                                         │
// └───────────────────────────────────────────────────────────────────────────┘

/// The site's own outbound mail: the SMTP client that reaches a recipient's MX, and the DKIM
/// identities that sign what it sends.
///
/// "Own the send." Built once at start-up from the server's mail configuration and threaded into the
/// publish path the way the outbound TLS client is, so the newsletter and its confirmation are signed
/// and delivered by exactly the machinery the mail server and the operator alerter already use --
/// [`OutboundClient::deliver`] straight to the recipient's MX, each message signed by every configured
/// [`DkimSigner`] as [`crate::srv::alert`] and the mail handler both do it.
///
/// Cheap to clone: the client and the signers are shared behind `Arc`s.
#[derive(Clone)]
pub struct MailSender {
	/// The SMTP client, dialling each recipient's MX directly.
	client:		Arc<OutboundClient>,
	/// The DKIM identities every message is signed with. Empty means unsigned, which still delivers --
	/// a key that will not sign is skipped, not fatal, as everywhere else the domain signs its mail.
	dkim:		Vec<Arc<DkimSigner>>,
	/// The address the newsletter is from where a site's `publish` block names none, derived from the
	/// mail configuration's signing domain, e.g. `news@<domain>`.
	default_from:	String,
}

impl std::fmt::Debug for MailSender {
	/// Written by hand because [`OutboundClient`] is not `Debug`; the sending identity is the part worth
	/// a log line.
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("MailSender")
			.field("default_from", &self.default_from)
			.field("dkim", &self.dkim.len())
			.finish()
	}
}

impl MailSender {

	/// Builds a sender from an EHLO hostname, the DKIM identities to sign with, and the default
	/// newsletter From address.
	///
	/// The signers and the From are the caller's -- built from the server's mail configuration -- so this
	/// invents no key and reads no config: it is the same pattern the alerter follows.
	pub fn new(
		ehlo_host:	String,
		dkim:		Vec<Arc<DkimSigner>>,
		default_from:	String,
	)
		-> Outcome<Self>
	{
		let client = res!(OutboundClient::with_system_roots(ehlo_host));
		Ok(Self {
			client:	Arc::new(client),
			dkim,
			default_from,
		})
	}

	/// The address the newsletter is from where a site names none.
	pub fn default_from(&self) -> &str {
		&self.default_from
	}

	/// Signs a message with every configured DKIM identity and delivers it to one recipient's MX.
	///
	/// The one door every piece of newsletter mail goes through, the confirmation included. Each signer
	/// prepends its own `DKIM-Signature`; a key that will not sign is skipped with a warning rather than
	/// failing the send, since an unsigned message that arrives beats a signed one that does not.
	/// Returns the remote's queue id.
	async fn deliver_signed(
		&self,
		from:	&str,
		to:	&str,
		msg:	&str,
	)
		-> Outcome<String>
	{
		let mut bytes = msg.as_bytes().to_vec();
		let now = match SystemTime::now().duration_since(UNIX_EPOCH) {
			Ok(d)	=> d.as_secs(),
			Err(_)	=> 0,
		};
		for signer in &self.dkim {
			match signer.sign(&bytes, &[], now) {
				Ok(b)	=> bytes = b,
				Err(e)	=> warn!("publish: signing newsletter mail with the {} key for selector \
					'{}' failed; sending it unsigned: {}",
					signer.algorithm(), signer.selector(), e),
			}
		}
		let rcpt = [to.to_string()];
		self.client.deliver(from, &rcpt, &bytes).await
	}

	/// Sends the double opt-in confirmation to a pending subscriber.
	///
	/// One plain-text message carrying the confirm link and nothing else: it is not the newsletter, and
	/// an address that never asked for it should get as little as possible.
	pub async fn send_confirmation(
		&self,
		from:		&str,
		to:		&str,
		confirm_url:	&str,
		site_name:	&str,
	)
		-> Outcome<String>
	{
		let msg = build_confirmation_email(from, to, confirm_url, site_name);
		self.deliver_signed(from, to, &msg).await
	}
}

/// The From address a newsletter is sent with, the site's own where it names one and the sender's
/// derived default otherwise.
///
/// A method on the config so the resolution lives in one place: the `publish` block's
/// `newsletter_from` wins, and an empty one falls back to `news@<mail-domain>`, which is aligned with
/// the DKIM signing domain so the signature authenticates.
impl PublishConfig {
	/// The newsletter's From, resolved against the mail sender's default.
	pub fn newsletter_from(&self, sender: &MailSender) -> String {
		if self.newsletter_from.trim().is_empty() {
			sender.default_from().to_string()
		} else {
			self.newsletter_from.clone()
		}
	}
}

/// What one newsletter send did: how many it was sent to, and how each attempt ended.
///
/// The tally a send returns and a [`SendEntry`] is built from. `attempted` is the confirmed send set at
/// the moment of the send; `sent` the deliveries the receiving server accepted; `failed` the transient
/// failures, which stay on the list to try again; `suppressed` the permanent ones, whose addresses were
/// marked [`SubState::Bounced`](super::subscribe::SubState::Bounced) and will not be sent to again.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SendReport {
	/// How many confirmed subscribers the send set held.
	pub attempted:	usize,
	/// How many the receiving server accepted.
	pub sent:	usize,
	/// How many failed transiently, and stay on the list to retry.
	pub failed:	usize,
	/// How many failed permanently and were suppressed as bounced.
	pub suppressed:	usize,
}

/// Sends a live post to every confirmed subscriber, best-effort, one message each.
///
/// The [`Destination::Email`](super::dest::Destination::Email) delivery, but not through the per-remote
/// retry queue: a newsletter is a fan-out to many addresses with no single permalink to return, so it
/// is its own path rather than a [`Delivery`] on the post. Each subscriber gets a message carrying
/// their own unsubscribe link -- built from their token, so the person who clicks it removes themselves
/// and nobody else -- and delivery is per recipient, since [`OutboundClient::deliver`] is one MX per
/// call. A recipient the send fails for is logged (redacted) and counted; the send does not stop.
///
/// A **permanent** failure -- a 5xx, an unknown mailbox, told apart by [`is_permanent`] -- suppresses
/// the address: it is marked [`SubState::Bounced`](super::subscribe::SubState::Bounced) so no later send
/// reaches it. A **transient** failure is merely counted, and the address stays confirmed for the next
/// send. Returns the [`SendReport`] the caller records as history.
pub async fn send_newsletter<
	const UIDL: usize,
	UID:	NumIdDat<UIDL>,
	ENC:	Encrypter,
	KH:	Hasher,
	DB:	Database<UIDL, UID, ENC, KH>,
>(
	sender:	&MailSender,
	db:	&(Arc<RwLock<DB>>, UID),
	cfg:	&PublishConfig,
	from:	&str,
	slug:	&str,
	id:	&str,
)
	-> Outcome<SendReport>
{
	let rec = match res!(store::get(db, slug)) {
		Some(r)	=> r,
		None	=> return Err(err!(
			"publish: there is no post '{}' to send to subscribers.", slug;
			Invalid, Input, Missing)),
	};
	// A draft is served to nobody, and mailed to nobody: a newsletter is a publication, and a post not
	// published is not one.
	if rec.state != PostState::Live {
		return Err(err!(
			"publish: '{}' is a draft; a draft is sent to no subscriber.", slug;
			Invalid, Input));
	}
	let post = res!(rec.render());
	let subs = res!(subscribe::confirmed(db, id));
	let online = cfg.url_of(&cfg.path_of(slug));

	let mut report = SendReport { attempted: subs.len(), ..Default::default() };
	for sub in &subs {
		let unsub = cfg.url_of(&cfg.unsubscribe_path(&sub.token));
		let msg = build_newsletter_email(from, &sub.email, &post, &online, &unsub, &cfg.site_name);
		match sender.deliver_signed(from, &sub.email, &msg).await {
			Ok(qid)	=> {
				report.sent += 1;
				debug!("{}: publish: newsletter '{}' to {} ({})",
					id, slug, subscribe::redact(&sub.email), qid);
			}
			// A permanent failure suppresses the address so no future send reaches it; a transient one is
			// counted and the address stays confirmed. The suppression is best-effort: if the mark itself
			// will not write, the send still finishes and logs, rather than fail the whole run.
			Err(e) if is_permanent(&e)	=> {
				report.suppressed += 1;
				warn!("{}: publish: newsletter '{}' to {} failed permanently; suppressing: {}",
					id, slug, subscribe::redact(&sub.email), e);
				if let Err(e2) = subscribe::mark_bounced(db, &sub.email, id) {
					warn!("{}: publish: could not suppress {}: {}",
						id, subscribe::redact(&sub.email), e2);
				}
			}
			Err(e)	=> {
				report.failed += 1;
				warn!("{}: publish: newsletter '{}' to {} failed: {}",
					id, slug, subscribe::redact(&sub.email), e);
			}
		}
	}
	info!("{}: publish: newsletter '{}' sent to {} of {} confirmed subscriber(s), {} failed, {} suppressed",
		id, slug, report.sent, report.attempted, report.failed, report.suppressed);
	Ok(report)
}

/// Sends a live post to one address only: the operator's own, to see what a subscriber would get.
///
/// A test, not a send: it touches no subscriber state, marks nothing bounced whatever the delivery does,
/// and writes no history. The one recipient need not be a subscriber, so the unsubscribe link carries a
/// throwaway token that matches nobody -- the message is well-formed and its link is harmless. The build
/// is [`send_newsletter`]'s own, so the test is the newsletter, not an approximation of it.
pub async fn send_test<
	const UIDL: usize,
	UID:	NumIdDat<UIDL>,
	ENC:	Encrypter,
	KH:	Hasher,
	DB:	Database<UIDL, UID, ENC, KH>,
>(
	sender:	&MailSender,
	db:	&(Arc<RwLock<DB>>, UID),
	cfg:	&PublishConfig,
	from:	&str,
	slug:	&str,
	to:	&str,
	id:	&str,
)
	-> Outcome<()>
{
	let to = subscribe::normalise_email(to);
	if !subscribe::valid_email(&to) {
		return Err(err!(
			"publish: {} is not a shape an address takes.", subscribe::redact(&to);
			Invalid, Input));
	}
	let rec = match res!(store::get(db, slug)) {
		Some(r)	=> r,
		None	=> return Err(err!(
			"publish: there is no post '{}' to test-send.", slug;
			Invalid, Input, Missing)),
	};
	if rec.state != PostState::Live {
		return Err(err!(
			"publish: '{}' is a draft; a test sends the live post a subscriber would get.", slug;
			Invalid, Input));
	}
	let post = res!(rec.render());
	let online = cfg.url_of(&cfg.path_of(slug));
	// A throwaway token: the link is well-formed but names no subscriber, so a test recipient who follows
	// it lands on the bad-token page and nobody is unsubscribed.
	let unsub = cfg.url_of(&cfg.unsubscribe_path(&subscribe::mint_token()));
	let msg = build_newsletter_email(from, &to, &post, &online, &unsub, &cfg.site_name);
	let qid = res!(sender.deliver_signed(from, &to, &msg).await);
	info!("{}: publish: test of '{}' sent to {} ({})", id, slug, subscribe::redact(&to), qid);
	Ok(())
}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ SEND HISTORY                                                               │
// └───────────────────────────────────────────────────────────────────────────┘

/// The key the append-only list of newsletter sends lives under.
pub const SENDS_KEY: &str = "publish/sends";

/// One newsletter send, as the history keeps it.
///
/// A record of a send that happened: which post, when, and how the attempts ended. Written once per real
/// send -- never for a test -- and only appended to, so the history is the site's own log of what it
/// mailed and to how many.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SendEntry {
	/// The post that was sent.
	pub slug:	String,
	/// When it was sent, as an ISO timestamp.
	pub at:		String,
	/// How many confirmed subscribers the send set held.
	pub attempted:	usize,
	/// How many the receiving servers accepted.
	pub sent:	usize,
	/// How many failed transiently.
	pub failed:	usize,
	/// How many failed permanently and were suppressed.
	pub suppressed:	usize,
}

impl SendEntry {

	/// A history entry from a send's slug, the moment, and its [`SendReport`].
	pub fn of(slug: &str, at: &str, report: &SendReport) -> Self {
		Self {
			slug:		slug.to_string(),
			at:		at.to_string(),
			attempted:	report.attempted,
			sent:		report.sent,
			failed:		report.failed,
			suppressed:	report.suppressed,
		}
	}

	/// The entry as a daticle.
	pub fn to_dat(&self) -> Dat {
		let mut m = DaticleMap::new();
		m.insert(dat!("slug"),		dat!(self.slug.clone()));
		m.insert(dat!("at"),		dat!(self.at.clone()));
		m.insert(dat!("attempted"),	Dat::U64(self.attempted as u64));
		m.insert(dat!("sent"),		Dat::U64(self.sent as u64));
		m.insert(dat!("failed"),	Dat::U64(self.failed as u64));
		m.insert(dat!("suppressed"),	Dat::U64(self.suppressed as u64));
		Dat::Map(m)
	}

	/// The entry from a daticle, leniently: a missing count reads as zero and a missing string as empty,
	/// so a record a later version wrote or an older one half-filled still lists rather than fails the
	/// whole history.
	pub fn from_dat(d: &Dat) -> Self {
		let m = match d {
			Dat::Map(m)	=> m,
			_		=> return Self::default(),
		};
		let get_str = |key: &str| -> String {
			match m.get(&dat!(key)) {
				Some(Dat::Str(s))	=> s.clone(),
				_			=> String::new(),
			}
		};
		Self {
			slug:		get_str("slug"),
			at:		get_str("at"),
			attempted:	as_usize(m.get(&dat!("attempted"))),
			sent:		as_usize(m.get(&dat!("sent"))),
			failed:		as_usize(m.get(&dat!("failed"))),
			suppressed:	as_usize(m.get(&dat!("suppressed"))),
		}
	}
}

/// A `usize` out of a daticle that may hold an integer under any of the widths jdat writes one as, or
/// zero where there is no readable number.
fn as_usize(d: Option<&Dat>) -> usize {
	match d {
		Some(Dat::U64(n))	=> *n as usize,
		Some(Dat::U32(n))	=> *n as usize,
		Some(Dat::U16(n))	=> *n as usize,
		Some(Dat::U8(n))	=> *n as usize,
		_			=> 0,
	}
}

/// Appends one send to the history, reading the list and writing it back with the entry on the end.
///
/// Index-driven, no scan: the whole history is one list under [`SENDS_KEY`], read, pushed to, and
/// written -- the same shape the subscriber index takes. Newest is last on disk; [`send_history`] hands
/// it back newest first.
pub fn record_send<
	const UIDL: usize,
	UID:	NumIdDat<UIDL>,
	ENC:	Encrypter,
	KH:	Hasher,
	DB:	Database<UIDL, UID, ENC, KH>,
>(
	db:	&(Arc<RwLock<DB>>, UID),
	entry:	&SendEntry,
)
	-> Outcome<()>
{
	let (db_arc, user) = db;
	let mut items = res!(sends_list(db));
	items.push(entry.to_dat());
	let list = Dat::List(items);
	let guard = lock_read!(db_arc);
	res!(guard.insert(dat!(SENDS_KEY), list, *user, None));
	Ok(())
}

/// The raw history list, or an empty one where nothing has been sent.
fn sends_list<
	const UIDL: usize,
	UID:	NumIdDat<UIDL>,
	ENC:	Encrypter,
	KH:	Hasher,
	DB:	Database<UIDL, UID, ENC, KH>,
>(
	db:	&(Arc<RwLock<DB>>, UID),
)
	-> Outcome<Vec<Dat>>
{
	let (db_arc, _) = db;
	let guard = lock_read!(db_arc);
	let val = match res!(guard.get(&dat!(SENDS_KEY), None)) {
		Some((v, _))	=> v,
		// No history is a site that has sent nothing, not an error -- the empty log it never wrote.
		None		=> return Ok(Vec::new()),
	};
	match &val {
		Dat::List(items)	=> Ok(items.clone()),
		Dat::Vek(vek)		=> Ok(vek.as_slice().to_vec()),
		_			=> Err(err!(
			"publish: the send history must be a list, not {:?}.", val.kind();
			Invalid, Input, Mismatch)),
	}
}

/// Every recorded send, most recent first.
///
/// What the subscribers page draws its history table from. The list is stored oldest-first, as it was
/// appended, and reversed here so the newest send heads the table.
pub fn send_history<
	const UIDL: usize,
	UID:	NumIdDat<UIDL>,
	ENC:	Encrypter,
	KH:	Hasher,
	DB:	Database<UIDL, UID, ENC, KH>,
>(
	db:	&(Arc<RwLock<DB>>, UID),
)
	-> Outcome<Vec<SendEntry>>
{
	let mut out: Vec<SendEntry> = res!(sends_list(db)).iter().map(SendEntry::from_dat).collect();
	out.reverse();
	Ok(out)
}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ EMAIL: building the messages                                              │
// └───────────────────────────────────────────────────────────────────────────┘

/// An RFC 5322 confirmation message: plain text, the confirm link, and a line saying why it arrived.
///
/// Pure over its strings, so what a subscriber is sent can be tested without a socket. The body is
/// deliberately spare -- an address that never opted in gets a link and an explanation, no more.
fn build_confirmation_email(from: &str, to: &str, confirm_url: &str, site_name: &str) -> String {
	let who = if site_name.trim().is_empty() {
		fmt!("this site")
	} else {
		site_name.to_string()
	};
	let subject = fmt!("Confirm your subscription to {}", who);
	let body = fmt!(
		"Someone -- probably you -- asked to subscribe this address to {who}.\r\n\
		\r\n\
		To confirm and start receiving posts, follow this link:\r\n\
		\r\n\
		{url}\r\n\
		\r\n\
		If it was not you, ignore this message: without the link followed, this address \
		receives nothing further.\r\n",
		who = who,
		url = confirm_url,
	);
	let date = rfc5322_date_now();
	fmt!(
		"From: {from}\r\n\
		To: {to}\r\n\
		Subject: {subject}\r\n\
		Date: {date}\r\n\
		MIME-Version: 1.0\r\n\
		Auto-Submitted: auto-generated\r\n\
		Content-Type: text/plain; charset=utf-8\r\n\
		\r\n\
		{body}",
		from = from, to = to, subject = subject, date = date, body = body,
	)
}

/// An RFC 5322 newsletter message: `multipart/alternative`, the post as HTML and as plain text, each
/// with a footer that carries the unsubscribe link -- as [CAN-SPAM] and the mailbox providers both
/// expect of bulk mail.
///
/// The HTML part is the post's own rendering, the same HTML a reader gets on the site, wrapped in a
/// minimal document and followed by the footer. The plain-text part is the title, the opening, and the
/// two links, for a reader whose client shows text. The Subject is the post's own title.
fn build_newsletter_email(
	from:		&str,
	to:		&str,
	post:		&Post,
	online_url:	&str,
	unsub_url:	&str,
	site_name:	&str,
) -> String {
	let boundary = fmt!("=_steel_{}", Rand::generate_random_string(24,
		"abcdefghijklmnopqrstuvwxyz0123456789"));
	let date = rfc5322_date_now();

	// The plain-text alternative: title, opening, and the two links, wrapped where a client wants text.
	let text = fmt!(
		"{title}\r\n\r\n{excerpt}\r\n\r\nRead it online:\r\n{online}\r\n\r\n\
		--\r\nYou are receiving this because you confirmed a subscription{site}.\r\n\
		Unsubscribe: {unsub}\r\n",
		title	= post.title,
		excerpt	= post.excerpt,
		online	= online_url,
		site	= if site_name.trim().is_empty() { String::new() } else { fmt!(" to {}", site_name) },
		unsub	= unsub_url,
	);

	// The HTML alternative: the post's own rendering, then a footer with the same links, everything the
	// site did not itself render escaped where it lands in markup.
	let mut html = String::new();
	html.push_str("<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n");
	html.push_str("<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n");
	html.push_str("<title>");
	escape_text(&mut html, &post.title);
	html.push_str("</title>\n</head>\n<body>\n<article>\n");
	// The prose was escaped where it was rendered; it is HTML by the time it reaches here.
	html.push_str(&post.html);
	html.push_str("\n</article>\n<hr>\n<footer>\n<p><a href=\"");
	escape_attr(&mut html, online_url);
	html.push_str("\">Read it online</a></p>\n<p>You are receiving this because you confirmed a \
		subscription");
	if !site_name.trim().is_empty() {
		html.push_str(" to ");
		escape_text(&mut html, site_name);
	}
	html.push_str(". <a href=\"");
	escape_attr(&mut html, unsub_url);
	html.push_str("\">Unsubscribe</a>.</p>\n</footer>\n</body>\n</html>\n");

	fmt!(
		"From: {from}\r\n\
		To: {to}\r\n\
		Subject: {subject}\r\n\
		Date: {date}\r\n\
		MIME-Version: 1.0\r\n\
		List-Unsubscribe: <{unsub}>\r\n\
		Content-Type: multipart/alternative; boundary=\"{boundary}\"\r\n\
		\r\n\
		--{boundary}\r\n\
		Content-Type: text/plain; charset=utf-8\r\n\
		\r\n\
		{text}\r\n\
		--{boundary}\r\n\
		Content-Type: text/html; charset=utf-8\r\n\
		\r\n\
		{html}\r\n\
		--{boundary}--\r\n",
		from = from, to = to, subject = post.title, date = date, unsub = unsub_url,
		boundary = boundary, text = text, html = html,
	)
}

/// The current moment as an RFC 5322 date, e.g. `Fri, 18 Jul 2026 10:00:00 +0000`.
///
/// Built from [`CalClock`] -- the fe2o3 calendar does the civil arithmetic -- with only the label
/// arrays here. A clock that will not read yields the epoch's date rather than failing a send.
fn rfc5322_date_now() -> String {
	let secs = match SystemTime::now().duration_since(UNIX_EPOCH) {
		Ok(d)	=> d.as_secs() as i64,
		Err(_)	=> 0,
	};
	match rfc5322_date(secs) {
		Ok(s)	=> s,
		Err(_)	=> fmt!("Thu, 01 Jan 1970 00:00:00 +0000"),
	}
}

/// A unix second as an RFC 5322 date in UTC.
fn rfc5322_date(unix_secs: i64) -> Outcome<String> {
	let cc = res!(CalClock::from_unix_timestamp_seconds(unix_secs, CalClockZone::utc()));
	let dow = match cc.day_of_week() {
		DayOfWeek::Monday	=> "Mon",
		DayOfWeek::Tuesday	=> "Tue",
		DayOfWeek::Wednesday	=> "Wed",
		DayOfWeek::Thursday	=> "Thu",
		DayOfWeek::Friday	=> "Fri",
		DayOfWeek::Saturday	=> "Sat",
		DayOfWeek::Sunday	=> "Sun",
	};
	let months = [
		"Jan", "Feb", "Mar", "Apr", "May", "Jun",
		"Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
	];
	let mi = (cc.month().max(1).min(12) - 1) as usize;
	Ok(fmt!(
		"{dow}, {day:02} {mon} {year:04} {h:02}:{m:02}:{s:02} +0000",
		dow = dow, day = cc.day(), mon = months[mi], year = cc.year(),
		h = cc.hour(), m = cc.minute(), s = cc.second(),
	))
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

	/// A stand-in post for the mail-building tests.
	fn a_post() -> Post {
		Post {
			slug:		fmt!("on-rent"),
			title:		fmt!("On rent"),
			author:		String::new(),
			categories:	Vec::new(),
			date:		None,
			excerpt:	fmt!("An opening sentence."),
			words:		3,
			html:		fmt!("<h1>On rent</h1>\n<p>An opening sentence.</p>\n"),
			also_on:	Vec::new(),
			tags:		Vec::new(),
		}
	}

	/// The confirmation is plain text, names why it arrived, and carries the confirm link and nothing
	/// that would act on the reader's behalf beyond it.
	#[test]
	fn test_a_confirmation_carries_its_link_14() -> Outcome<()> {
		let msg = build_confirmation_email(
			"news@x.test", "me@example.com", "https://x.test/posts/confirm?token=abc", "README");
		assert!(msg.contains("From: news@x.test"), "got: {}", msg);
		assert!(msg.contains("To: me@example.com"), "got: {}", msg);
		assert!(msg.contains("Subject: Confirm your subscription to README"), "got: {}", msg);
		assert!(msg.contains("https://x.test/posts/confirm?token=abc"), "no confirm link: {}", msg);
		assert!(msg.contains("text/plain; charset=utf-8"), "not plain text: {}", msg);
		assert!(msg.contains("Date: "), "no date header: {}", msg);
		Ok(())
	}

	/// The newsletter is multipart, carries the post's own HTML, its title as the subject, and the
	/// unsubscribe link in the body and the `List-Unsubscribe` header both.
	#[test]
	fn test_a_newsletter_carries_the_post_and_unsub_15() -> Outcome<()> {
		let post = a_post();
		let msg = build_newsletter_email(
			"README <news@x.test>", "me@example.com", &post,
			"https://x.test/posts/on-rent", "https://x.test/posts/unsubscribe?token=zzz", "README");
		assert!(msg.contains("Subject: On rent"), "the subject is not the title: {}", msg);
		assert!(msg.contains("multipart/alternative"), "not multipart: {}", msg);
		assert!(msg.contains("text/plain; charset=utf-8"), "no text part: {}", msg);
		assert!(msg.contains("text/html; charset=utf-8"), "no html part: {}", msg);
		assert!(msg.contains("<h1>On rent</h1>"), "the post's HTML is not in the message: {}", msg);
		assert!(msg.contains("https://x.test/posts/unsubscribe?token=zzz"), "no unsubscribe link: {}", msg);
		assert!(msg.contains("List-Unsubscribe: <https://x.test/posts/unsubscribe?token=zzz>"),
			"no List-Unsubscribe header: {}", msg);
		Ok(())
	}

	/// A unix second becomes an RFC 5322 date the epoch pins, ending in a UTC offset.
	#[test]
	fn test_a_unix_second_becomes_an_rfc5322_date_16() -> Outcome<()> {
		// 2026-07-18T10:00:00Z is 1_784_368_800.
		let s = res!(rfc5322_date(1_784_368_800));
		assert!(s.contains("18 Jul 2026"), "got: {}", s);
		assert!(s.contains("10:00:00"), "got: {}", s);
		assert!(s.ends_with("+0000"), "got: {}", s);
		// The epoch itself was a Thursday.
		let s = res!(rfc5322_date(0));
		assert!(s.starts_with("Thu, 01 Jan 1970"), "got: {}", s);
		Ok(())
	}

	/// The newsletter From is the site's own where it names one, and the sender's default otherwise.
	#[test]
	fn test_the_newsletter_from_resolves_17() -> Outcome<()> {
		let sender = res!(MailSender::new(
			"mail.x.test".to_string(), Vec::new(), "news@x.test".to_string()));
		let named = PublishConfig { newsletter_from: fmt!("README <hi@x.test>"), ..Default::default() };
		assert_eq!(named.newsletter_from(&sender), "README <hi@x.test>");
		let unnamed = PublishConfig { newsletter_from: String::new(), ..Default::default() };
		assert_eq!(unnamed.newsletter_from(&sender), "news@x.test");
		Ok(())
	}

	/// A send report becomes a history entry that carries every count, and the entry survives the trip
	/// through its daticle.
	#[test]
	fn test_a_send_entry_round_trips_18() -> Outcome<()> {
		let report = SendReport { attempted: 10, sent: 7, failed: 2, suppressed: 1 };
		let entry = SendEntry::of("on-rent", "2026-07-18T10:00:00Z", &report);
		assert_eq!(entry.slug, "on-rent");
		assert_eq!(entry.at, "2026-07-18T10:00:00Z");
		assert_eq!(entry.attempted, 10);
		assert_eq!(entry.sent, 7);
		assert_eq!(entry.failed, 2);
		assert_eq!(entry.suppressed, 1);

		let back = SendEntry::from_dat(&entry.to_dat());
		assert_eq!(back, entry);

		// A record missing a count reads that count as zero rather than failing the whole history.
		let mut sparse = DaticleMap::new();
		sparse.insert(dat!("slug"), dat!("x"));
		let back = SendEntry::from_dat(&Dat::Map(sparse));
		assert_eq!(back.slug, "x");
		assert_eq!(back.attempted, 0);
		assert_eq!(back.sent, 0);
		Ok(())
	}
}
