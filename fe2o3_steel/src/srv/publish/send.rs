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

use crate::srv::publish::dest::Destination;

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_jdat::{
	prelude::*,
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
	sync::Arc,
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
	let rest = uri.strip_prefix("at://")?;
	let mut parts = rest.splitn(3, '/');
	let did = parts.next()?;
	let _collection = parts.next()?;
	let rkey = parts.next()?;
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
