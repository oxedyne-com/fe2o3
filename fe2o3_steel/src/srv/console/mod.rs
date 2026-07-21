//! The site console: a site administered from within itself.
//!
//! # Two kinds of administration, told apart
//!
//! Steel already has an operator dashboard at `/admin`. That is the *server's* -- the wallet, the
//! certificates, the traffic, the seal: the fuse box for the whole host, shared by every site on it.
//! Signing into it means proving the wallet passphrase, which is also what unseals the databases.
//!
//! This is a different thing. A *site's* administration -- writing its posts, and in time its
//! settings -- is the site's own concern, not the host's. The person who runs Elearnity should reach
//! it from Elearnity, in Elearnity's own look, without being sent to a panel that also runs every
//! other site and holds the keys to the machine.
//!
//! The two were conflated because the fast path put content behind the operator session: it already
//! existed and already held the master key the database needed. But by the time a request to write a
//! post arrives, the database is long unsealed -- that happened once, at boot. A site admin never
//! needs the wallet. So the tiers separate cleanly: the operator holds the host, a site admin holds a
//! site, and the only thing they share is that neither can work until the operator has unsealed at
//! boot.
//!
//! # Who a site admin is
//!
//! An ordinary member of the site whose username the operator has listed in the vhost's
//! [`site_admins`](crate::srv::cfg::VhostConfig::site_admins). There is no separate admin account, no
//! separate password, and no separate login: the site's own member login is the admin login, and the
//! authority is nothing but being on the list. A member signs in as they always do; if they are on
//! the list, the console opens.
//!
//! The list lives in config, not in the site's database, on purpose. The operator owns the host and
//! says who runs each site; a content bug -- the kind this codebase has found more than one of --
//! must not be able to mint an administrator. Authority is the operator's grant, held where the
//! database cannot reach it.
//!
//! # Why the member cookie reaches here and the operator cookie would not
//!
//! The operator's session cookie is `Path=/admin`, so a browser sends it to `/admin` and nowhere
//! else -- which is why the first cut of the composer was trapped under `/admin`. The member's
//! session cookie is `Path=/`, so it is already sent to `/manage` and every other site path. The
//! console lives where the credential that opens it is actually presented.

pub mod publish;
pub mod session;

use crate::srv::{
	admin::{
		assets::html_escape,
		auth::{
			self,
			LoginOutcome,
		},
		state::AdminState,
	},
	publish::{
		PublishConfig,
		send::MailSender,
		store,
	},
};

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_hash::hash::HashScheme;
use oxedyne_fe2o3_iop_crypto::enc::Encrypter;
use oxedyne_fe2o3_iop_db::api::Database;
use oxedyne_fe2o3_iop_hash::api::Hasher;
use oxedyne_fe2o3_jdat::{
	prelude::*,
	id::NumIdDat,
};
use oxedyne_fe2o3_net::http::{
	fields::{
		Cookie,
		HeaderFieldValue,
		HeaderFields,
		HeaderName,
		SameSite,
		SetCookieAttributes,
	},
	msg::HttpMessage,
	status::HttpStatus,
};

use tokio_rustls::rustls::ClientConfig;

use std::{
	collections::BTreeSet,
	net::SocketAddr,
	sync::{
		Arc,
		RwLock,
	},
};


/// The console's root.
pub const PATH_ROOT: &str = "/manage";

/// Whether the signed-in member may reach the console, as JSON, for the site's own chrome to ask
/// before it offers a way in. A read, so it is a GET, and it answers for anyone -- signed in or not,
/// admin or not -- rather than turning a non-admin away, because a page asking "should I show the
/// door" is not itself the door.
pub const PATH_STATUS: &str = "/manage/status";

/// Where a signed-in member posts to become the site's first admin.
///
/// The self-bootstrap: open only while the site has no admins at all, so a member can make themselves
/// the first one without a config edit and a restart, and closed the moment there is one, so it cannot
/// take a site that is already owned.
pub const PATH_CLAIM: &str = "/manage/claim";

/// The admin-management page, and where its add and remove forms post.
///
/// A GET lists the site's admins; a POST adds one by id-hash or removes a database-granted one. Both
/// are gated on an existing admin, so this is how a second admin is granted once the first has claimed.
pub const PATH_ADMINS: &str = "/manage/admins";

/// Where a passphrase sign-in posts, and where a `GET` renders the themed login
/// form.
///
/// The generic way in: a site owner types the operator's wallet passphrase --
/// the same one the `/admin` dashboard verifies -- and is given a site-admin
/// session ([`session`]) that opens this console and nothing else. No member
/// account, no separate password, no redirect to `/admin`.
pub const PATH_LOGIN: &str = "/manage/login";

/// Where a site-admin session is cleared.
pub const PATH_LOGOUT: &str = "/manage/logout";


/// A member the operator has entrusted with a site.
///
/// Nothing but a name, because that is all authority here is: the session proved who they are, and
/// the list said they may.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SiteAdmin {
	/// The member's username -- the site login's own identifier, which is the SHA-256 of their
	/// passphrase.
	pub username:	String,
}


/// Whether a path belongs to the console.
pub fn owns(path: &str) -> bool {
	path == PATH_ROOT
		|| (path.starts_with(PATH_ROOT)
			&& path.as_bytes().get(PATH_ROOT.len()) == Some(&b'/'))
}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ THE GATE                                                                  │
// └───────────────────────────────────────────────────────────────────────────┘

/// The member a request's session names, if it names one.
///
/// The authentication half, on its own, so the console can tell a signed-in member who is not an
/// admin from a visitor who is not signed in at all -- the first wants their id and a way to ask for
/// access, the second wants sending home. `Ok(None)` covers every anonymous case alike: no cookie, an
/// expired session, a session bound to nobody.
///
/// `Err` is kept apart from `None`: a poisoned lock or an unreadable database is a fault to log and
/// deny on, not a member quietly failing to be signed in.
pub fn member_username<
	const UIDL: usize,
	UID:	NumIdDat<UIDL>,
	ENC:	Encrypter,
	KH:	Hasher,
	DB:	Database<UIDL, UID, ENC, KH>,
>(
	db:		Option<&(Arc<RwLock<DB>>, UID)>,
	headers:	&Arc<HeaderFields>,
)
	-> Outcome<Option<String>>
{
	let sid = match headers.get_session_id() {
		Some(s)	=> s,
		None	=> return Ok(None),
	};
	let db = match db {
		Some(db)	=> db,
		None		=> return Ok(None),
	};
	let (db_arc, _) = db;

	// The session record the member's login wrote: `sess_meta:<sid>` -> `{ user }`. The same record
	// the WebSocket handler reads to answer `whoami`, read here over HTTP because the console is
	// pages, not sockets.
	let meta_key = Dat::Str(fmt!("sess_meta:{}", sid));
	let guard = lock_read!(db_arc);
	match res!(guard.get(&meta_key, None)) {
		Some((Dat::Map(m), _)) => match m.get(&dat!("user")) {
			Some(Dat::Str(u)) if !u.is_empty()	=> Ok(Some(u.clone())),
			// A session with no user is an anonymous one -- issued to everybody, authenticated to
			// nobody.
			_					=> Ok(None),
		},
		_ => Ok(None),
	}
}

/// The admin the request belongs to, by any of the ways one is proven.
///
/// Three paths, any one of which suffices, tried cheapest first:
///
/// 1. A **passphrase session**: a valid `manage_session` cookie ([`session`]),
///    minted when a site owner signed in at [`PATH_LOGIN`] with the operator's
///    wallet passphrase. The generic way in, needing no member account.
/// 2. A **listed member**: a signed-in member whose username the operator pinned
///    in the vhost's [`site_admins`](crate::srv::cfg::VhostConfig::site_admins).
/// 3. A **granted member**: a signed-in member the site's own database names,
///    the union computed in [`effective_admins`].
///
/// The passphrase path is checked first because it consults no database. The
/// member paths are unchanged, so a site that signs its admins in the way it
/// always did keeps working exactly as before.
pub fn site_admin<
	const UIDL: usize,
	UID:	NumIdDat<UIDL>,
	ENC:	Encrypter,
	KH:	Hasher,
	DB:	Database<UIDL, UID, ENC, KH>,
>(
	site_admins:	&[String],
	admin_state:	Option<&AdminState>,
	db:		Option<&(Arc<RwLock<DB>>, UID)>,
	headers:	&Arc<HeaderFields>,
)
	-> Outcome<Option<SiteAdmin>>
{
	// The passphrase session: proven by the wallet passphrase at login, carried
	// in the manage cookie, and granting this console alone.
	if let Some(state) = admin_state {
		if let Some(name) = session::authenticate(state, headers) {
			return Ok(Some(SiteAdmin { username: name }));
		}
	}

	let username = match res!(member_username(db, headers)) {
		Some(u)	=> u,
		// Not signed in as anyone, so an admin of nothing. The database is not consulted: there is no
		// name to look for.
		None	=> return Ok(None),
	};
	let admins = res!(effective_admins(site_admins, db));
	if admins.iter().any(|a| a == &username) {
		Ok(Some(SiteAdmin { username }))
	} else {
		Ok(None)
	}
}

/// The seed the console's CSRF token is derived from for this request.
///
/// A passphrase-authed admin holds no member session id, so the token cannot be
/// keyed on one. It is keyed instead on their `manage_session` cookie value,
/// which is stable for the life of the session, `HttpOnly` so no script reads
/// it, and `SameSite=Strict` so no cross-site request carries it -- the same
/// properties the member session id has, and the same guarantee the token
/// needs. A member falls through to their session id, exactly as before.
///
/// Both the issuing side ([`PATH_STATUS`]) and the checking side (the write
/// handlers) call this, so the seed they agree on is the same one.
fn csrf_seed(
	admin_state:	Option<&AdminState>,
	headers:	&Arc<HeaderFields>,
)
	-> Option<String>
{
	if let Some(state) = admin_state {
		if let Some(value) = session::cookie_value(headers) {
			if session::decode(state, &value).is_ok() {
				return Some(value);
			}
		}
	}
	headers.get_session_id()
}

/// The effective admin set: the config's failsafe list unioned with the database's granted one.
///
/// A member is an admin if either set names them, so the operator's config-pinned admins are an
/// override the database cannot touch, and the site's own grants sit alongside them. Reads the
/// database list where there is a database and takes it as empty where there is not.
fn effective_admins<
	const UIDL: usize,
	UID:	NumIdDat<UIDL>,
	ENC:	Encrypter,
	KH:	Hasher,
	DB:	Database<UIDL, UID, ENC, KH>,
>(
	site_admins:	&[String],
	db:		Option<&(Arc<RwLock<DB>>, UID)>,
)
	-> Outcome<Vec<String>>
{
	let db_admins = match db {
		Some(d)	=> res!(store::admins_get(d, "console")),
		None	=> Vec::new(),
	};
	Ok(union_admins(site_admins, &db_admins))
}

/// The union of the config admin list and the database one, config first, deduped.
///
/// Pure, so the union's rule -- config-pinned admins always present, database ones added where they do
/// not repeat one -- can be reasoned about and tested without a database.
pub fn union_admins(config: &[String], db_admins: &[String]) -> Vec<String> {
	let mut out: Vec<String> = config.to_vec();
	for h in db_admins {
		if !out.iter().any(|a| a == h) {
			out.push(h.clone());
		}
	}
	out
}

/// Whether a string is a member id-hash: 64 lowercase hexadecimal characters.
///
/// A username here is the SHA-256 of a passphrase, rendered lowercase hex, so a grant's word for one is
/// held to that shape before it reaches the admin list -- a browser's field is not a reason to store a
/// name nothing could ever match.
fn valid_id_hash(s: &str) -> bool {
	s.len() == 64 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ GET                                                                       │
// └───────────────────────────────────────────────────────────────────────────┘

/// Serves the console's pages.
pub async fn handle_get<
	const UIDL: usize,
	UID:	NumIdDat<UIDL>,
	ENC:	Encrypter,
	KH:	Hasher,
	DB:	Database<UIDL, UID, ENC, KH>,
>(
	site_admins:	&[String],
	admin_state:	Option<&AdminState>,
	publish:	Option<&PublishConfig>,
	db:		Option<&(Arc<RwLock<DB>>, UID)>,
	request_path:	&str,
	query:		&str,
	headers:	&Arc<HeaderFields>,
	id:		&str,
)
	-> Outcome<HttpMessage>
{
	// The login form, themed by the site's own look. Served to anyone: it is the
	// way in, not a thing behind the way in. An already-signed-in admin who lands
	// here is shown it too, harmlessly -- posting it merely refreshes their session.
	if request_path == PATH_LOGIN {
		return Ok(login_page(&Theme::of(publish), None));
	}

	let admin = res!(site_admin(site_admins, admin_state, db, headers));

	// The status probe answers everyone, so the site's chrome can ask whether to show the way in
	// without being redirected. It is the one console path a non-admin may read. An admin also gets
	// the CSRF token here, since the app that draws its own management surface needs it to write and
	// cannot read the session cookie to derive it.
	if request_path == PATH_STATUS {
		// Claimable: a signed-in member who is not yet an admin, on a site that has none at all. The
		// site's own chrome reads this to offer a "Become admin" button, so the first admin bootstraps
		// without ever seeing a config file.
		let claimable = if admin.is_some() {
			false
		} else {
			match res!(member_username(db, headers)) {
				Some(_)	=> res!(effective_admins(site_admins, db)).is_empty(),
				None	=> false,
			}
		};
		// An admin needs the CSRF token to write; a claimable member needs it to post the claim. Both
		// hold the seed it is derived from -- a passphrase admin's manage cookie, or a member's session
		// id -- so both are given it here, and nobody else is.
		let csrf = if admin.is_some() || claimable {
			csrf_seed(admin_state, headers).map(|seed| csrf_token(&seed))
		} else {
			None
		};
		// An admin is told which remotes the site can post to, so the composer draws a picker for those
		// and no others. A non-admin is told nothing of them. The set is the effective one -- what the
		// console has set laid over the config -- so a remote configured from the settings form appears
		// here at once.
		let offered = match (&admin, publish, db) {
			(Some(_), Some(p), Some(d))	=>
				res!(crate::srv::publish::send::effective_creds(d, p)).offered(),
			(Some(_), Some(p), None)	=> p.creds.offered(),
			_				=> Vec::new(),
		};
		let dests: Vec<&str> = offered.iter().map(|d| d.as_str()).collect();
		// The site's category taxonomy, so the composer draws a checkbox per category. Only an admin
		// composes, and only where the vhost publishes at all; a non-admin, or a site with no publish
		// block, is given the empty set.
		let cats: &[String] = match (&admin, publish) {
			(Some(_), Some(p))	=> &p.categories,
			_			=> &[],
		};
		return Ok(status_json(admin.is_some(), claimable, csrf.as_deref(), &dests, cats));
	}

	let admin = match admin {
		Some(a)	=> a,
		None	=> {
			// Not an admin. A signed-in member is one no set has yet named. If the site has no admins
			// at all, this is the bootstrap: they may claim it, and are shown the button that does.
			// Otherwise they are shown their id, to hand to an existing admin. A visitor who is not
			// signed in is shown the passphrase login: the generic way in, so any site with a console
			// offers a themed sign-in without a line of its own code.
			return match res!(member_username(db, headers)) {
				Some(username)	=> {
					let claimable = res!(effective_admins(site_admins, db)).is_empty();
					let csrf = match headers.get_session_id() {
						Some(s)	=> csrf_token(&s),
						None	=> String::new(),
					};
					Ok(not_yet_admin(&Theme::of(publish), &username, claimable, &csrf))
				}
				None		=> Ok(login_page(&Theme::of(publish), None)),
			};
		}
	};

	// The token every form on the pages below carries, so the write it makes proves it came from a
	// page the session rendered. Derived from the session's seed -- a passphrase admin's manage cookie
	// or a member's session id -- and the same for every form in a session.
	let csrf = match csrf_seed(admin_state, headers) {
		Some(s)	=> csrf_token(&s),
		None	=> return Ok(redirect(&home_of(publish))),
	};

	let theme = Theme::of(publish);

	// The admin-management page needs the config admin list to tell a pinned admin from a granted one,
	// which the post console does not carry, so it is answered here rather than passed down.
	if request_path == PATH_ADMINS {
		return admins_page(&theme, &admin, &csrf, site_admins, db, query, id);
	}

	publish::handle_get(publish, &theme, &admin, &csrf, site_admins, db, request_path, query, id)
}

/// The admin-management page: who administers the site, and the forms to grant and revoke.
///
/// Config-pinned admins are shown read-only -- the operator holds them, and the database cannot remove
/// what config asserts. Database-granted admins each carry a Remove button, and one form adds a new
/// admin by id-hash. Dressed in the site's own chrome, like every console page.
fn admins_page<
	const UIDL: usize,
	UID:	NumIdDat<UIDL>,
	ENC:	Encrypter,
	KH:	Hasher,
	DB:	Database<UIDL, UID, ENC, KH>,
>(
	theme:		&Theme,
	admin:		&SiteAdmin,
	csrf:		&str,
	site_admins:	&[String],
	db:		Option<&(Arc<RwLock<DB>>, UID)>,
	query:		&str,
	id:		&str,
)
	-> Outcome<HttpMessage>
{
	let db_admins = match db {
		Some(d)	=> res!(store::admins_get(d, id)),
		None	=> Vec::new(),
	};

	let mut body = String::new();
	body.push_str("<h1>Administrators</h1>\n");
	body.push_str(
		"<p class=\"mc-muted\">Who may manage this site. An administrator is a member, named by the \
		id of their account. Add one by pasting their id below; they will find it on this site's manage \
		page when they are signed in but not yet an administrator.</p>\n");

	// A grant or a refusal that redirected here said why in the query it landed with. Shown, rather
	// than swallowed, exactly as the posts list shows its own.
	if let Some(said) = said_field(query) {
		body.push_str(&fmt!("<p class=\"mc-notice\">{}</p>\n", html_escape(&said)));
	}

	body.push_str("<table class=\"mc-table\">\n<thead><tr>\
		<th>Administrator</th><th>Source</th><th></th>\
		</tr></thead>\n<tbody>\n");

	// The config-pinned admins first, read-only: the operator's grant, which the database cannot lift.
	for h in site_admins {
		body.push_str(&fmt!(
			"<tr><td><span class=\"mc-slug\">{id}</span></td>\
			<td><span class=\"mc-tag\">config</span></td>\
			<td></td></tr>\n",
			id = html_escape(h),
		));
	}

	// The database-granted admins, each with a Remove button. One that is also config-pinned is already
	// shown above and not repeated: it cannot be removed here, so a Remove button on it would lie.
	for h in &db_admins {
		if site_admins.iter().any(|a| a == h) {
			continue;
		}
		body.push_str(&fmt!(
			"<tr><td><span class=\"mc-slug\">{id}</span></td>\
			<td><span class=\"mc-tag mc-tag-live\">granted</span></td>\
			<td>\
			<form class=\"mc-admin-remove\" method=\"POST\" action=\"{admins}\" \
			onsubmit=\"return confirm('Remove this administrator?')\">\
			<input type=\"hidden\" name=\"csrf\" value=\"{csrf}\">\
			<input type=\"hidden\" name=\"action\" value=\"remove\">\
			<input type=\"hidden\" name=\"id\" value=\"{id}\">\
			<button type=\"submit\" class=\"mc-btn mc-btn-danger\">Remove</button>\
			</form>\
			</td></tr>\n",
			admins	= PATH_ADMINS,
			csrf	= html_escape(csrf),
			id	= html_escape(h),
		));
	}
	body.push_str("</tbody>\n</table>\n");

	// The add form: a member id, and the grant.
	body.push_str(&fmt!(
		"<form class=\"mc-form\" id=\"mc-admin-add\" method=\"POST\" action=\"{admins}\">\n\
		<input type=\"hidden\" name=\"csrf\" value=\"{csrf}\">\n\
		<input type=\"hidden\" name=\"action\" value=\"add\">\n\
		<label for=\"mc-admin-id\">Add an administrator by id</label>\n\
		<input type=\"text\" id=\"mc-admin-id\" name=\"id\" \
		placeholder=\"64 hexadecimal characters\" autocomplete=\"off\" spellcheck=\"false\">\n\
		<div class=\"mc-actions\">\n\
		<button type=\"submit\" class=\"mc-btn\" id=\"mc-admin-add-btn\">Add administrator</button>\n\
		</div>\n\
		</form>\n",
		admins	= PATH_ADMINS,
		csrf	= html_escape(csrf),
	));

	Ok(page(theme, admin, "Administrators", &body))
}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ POST                                                                      │
// └───────────────────────────────────────────────────────────────────────────┘

/// Serves the console's writes.
///
/// Returns `None` for a path the console does not write to, so the caller carries on down its own
/// routing rather than turning every unknown POST under `/manage` into an error here.
pub async fn handle_post<
	const UIDL: usize,
	UID:	NumIdDat<UIDL>,
	ENC:	Encrypter,
	KH:	Hasher,
	DB:	Database<UIDL, UID, ENC, KH>,
>(
	site_admins:	&[String],
	admin_state:	Option<&AdminState>,
	publish:	Option<&PublishConfig>,
	db:		Option<&(Arc<RwLock<DB>>, UID)>,
	tls_client:	&Option<Arc<ClientConfig>>,
	mail:		&Option<Arc<MailSender>>,
	request_path:	&str,
	headers:	&Arc<HeaderFields>,
	body:		&[u8],
	peer:		SocketAddr,
	id:		&str,
)
	-> Outcome<Option<HttpMessage>>
{
	// The passphrase sign-in and its matching sign-out. Answered first: they need
	// no session and gate on nothing, they establish and clear the session the
	// rest of the console gates on. The passphrase is verified against the wallet
	// and never logged.
	if request_path == PATH_LOGIN {
		return Ok(Some(do_login(admin_state, publish, headers, body, peer, id)));
	}
	if request_path == PATH_LOGOUT {
		return Ok(Some(do_logout(publish, headers)));
	}
	// The claim: a signed-in member becomes the first admin, gated not on being an admin -- there are
	// none yet -- but on the set being empty. Answered before the admin gate below, which it could not
	// pass and does not need to.
	if request_path == PATH_CLAIM {
		return Ok(Some(res!(do_claim(site_admins, db, headers, body, id))));
	}
	// The admin-management writes: an existing admin grants or revokes. Its own gate and CSRF check are
	// inside, since it needs the config list the shared path below does not carry.
	if request_path == PATH_ADMINS {
		return Ok(Some(res!(do_admins(site_admins, admin_state, publish, db, headers, body, id))));
	}

	if !publish::posts(request_path) {
		return Ok(None);
	}

	let admin = match res!(site_admin(site_admins, admin_state, db, headers)) {
		Some(a)	=> a,
		None	=> {
			warn!("{}: console: a caller who is not a site admin tried to write", id);
			return Ok(Some(redirect(&home_of(publish))));
		}
	};

	// The cross-site guard. A member cookie is `SameSite=Lax` and a manage cookie `SameSite=Strict`,
	// so a cross-site POST carries neither and never reaches an authenticated state at all -- this is
	// the belt to that braces, and the thing that still holds if a cookie's policy is ever loosened.
	// The token is a value only a page that held the session could have been given, checked against
	// the seed the session's cookie names.
	let seed = match csrf_seed(admin_state, headers) {
		Some(s)	=> s,
		None	=> return Ok(Some(redirect(&home_of(publish)))),
	};
	// Whether the caller is the site's own front-end asking over fetch, which wants a plain JSON
	// answer, or a browser posting a form, which wants a redirect. The app says so with its Accept.
	let json = wants_json(headers);

	let sent = form_field(body, "csrf").unwrap_or_default();
	if !csrf_ok(&seed, &sent) {
		warn!("{}: console: a write arrived without a good csrf token", id);
		return Ok(Some(if json {
			HttpMessage::new_response(HttpStatus::Forbidden)
				.with_field(
					HeaderName::ContentType,
					HeaderFieldValue::Generic("application/json".to_string()),
				)
				.with_body(fmt!("{{\"error\":\"stale session; reload\"}}").into_bytes())
		} else {
			redirect(PATH_ROOT)
		}));
	}

	let resp = res!(publish::handle_post(
		publish, &admin, site_admins, db, tls_client, mail, request_path, body, json, id,
	).await);
	Ok(Some(resp))
}

/// Whether the caller wants JSON rather than a page -- the site's own front-end, over fetch, asking
/// with `Accept: application/json`. A browser form carries no such Accept and gets a redirect.
fn wants_json(headers: &Arc<HeaderFields>) -> bool {
	match headers.get_one(&HeaderName::Accept) {
		Some(HeaderFieldValue::Generic(v))	=> v.contains("application/json"),
		_					=> false,
	}
}

/// The self-bootstrap: a signed-in member becomes the site's first admin.
///
/// It goes through only where every guard holds: the caller is a signed-in member, the form carries a
/// good CSRF token, and the effective admin set is empty. That last is the whole safety of it -- the
/// claim is open while the site is unowned and shut the instant it is owned, so it can make a first
/// admin but never displace one. On success the caller is now an admin and is sent to the console.
fn do_claim<
	const UIDL: usize,
	UID:	NumIdDat<UIDL>,
	ENC:	Encrypter,
	KH:	Hasher,
	DB:	Database<UIDL, UID, ENC, KH>,
>(
	site_admins:	&[String],
	db:		Option<&(Arc<RwLock<DB>>, UID)>,
	headers:	&Arc<HeaderFields>,
	body:		&[u8],
	id:		&str,
)
	-> Outcome<HttpMessage>
{
	let json = wants_json(headers);

	// A claim is a member's, so an anonymous caller has nothing to claim with.
	let username = match res!(member_username(db, headers)) {
		Some(u)	=> u,
		None	=> {
			warn!("{}: console: a claim arrived from nobody signed in", id);
			return Ok(claim_deny(json, "sign in first"));
		}
	};

	// The cross-site guard, as every console write has: the token proves the post came from a page that
	// held the session.
	let sid = match headers.get_session_id() {
		Some(s)	=> s,
		None	=> return Ok(claim_deny(json, "sign in first")),
	};
	let sent = form_field(body, "csrf").unwrap_or_default();
	if !csrf_ok(&sid, &sent) {
		warn!("{}: console: a claim arrived without a good csrf token", id);
		return Ok(csrf_deny(json));
	}

	// The gate the whole thing turns on: a claim is refused the moment the site has an admin, so it can
	// only ever mint the first.
	let admins = res!(effective_admins(site_admins, db));
	if !admins.is_empty() {
		warn!("{}: console: '{}' tried to claim a site that already has admins", id, username);
		return Ok(claim_deny(json, "this site already has an administrator"));
	}

	let d = match db {
		Some(d)	=> d,
		None	=> return Ok(claim_deny(json, "this site has no database configured")),
	};
	res!(store::admins_add(d, id, &username));
	info!("{}: console: '{}' claimed the site as its first administrator", id, username);

	Ok(if json {
		json_ok()
	} else {
		redirect(PATH_ROOT)
	})
}

/// The admin-management writes: an existing admin grants a new admin or revokes a granted one.
///
/// Gated on an existing admin and CSRF-checked, both here since it needs the config list to keep a
/// pinned admin from being revoked -- removing one from the database would leave it effective anyway,
/// so the refusal is the honest answer rather than a silent no-op. A grant validates the id-hash's
/// shape before it reaches the list; a revoke touches the database list alone.
fn do_admins<
	const UIDL: usize,
	UID:	NumIdDat<UIDL>,
	ENC:	Encrypter,
	KH:	Hasher,
	DB:	Database<UIDL, UID, ENC, KH>,
>(
	site_admins:	&[String],
	admin_state:	Option<&AdminState>,
	publish:	Option<&PublishConfig>,
	db:		Option<&(Arc<RwLock<DB>>, UID)>,
	headers:	&Arc<HeaderFields>,
	body:		&[u8],
	id:		&str,
)
	-> Outcome<HttpMessage>
{
	let json = wants_json(headers);

	let admin = match res!(site_admin(site_admins, admin_state, db, headers)) {
		Some(a)	=> a,
		None	=> {
			warn!("{}: console: a non-admin tried to manage the admin list", id);
			return Ok(redirect(&home_of(publish)));
		}
	};

	let seed = match csrf_seed(admin_state, headers) {
		Some(s)	=> s,
		None	=> return Ok(redirect(&home_of(publish))),
	};
	let sent = form_field(body, "csrf").unwrap_or_default();
	if !csrf_ok(&seed, &sent) {
		warn!("{}: console: an admin-list write arrived without a good csrf token", id);
		return Ok(csrf_deny(json));
	}

	let d = match db {
		Some(d)	=> d,
		None	=> return Ok(admins_deny(json, "this site has no database configured")),
	};

	let action = form_field(body, "action").unwrap_or_default();
	let hash = form_field(body, "id").unwrap_or_default().trim().to_string();

	match action.as_str() {
		"add"	=> {
			if !valid_id_hash(&hash) {
				return Ok(admins_deny(
					json, "an administrator's id is 64 lowercase hexadecimal characters"));
			}
			res!(store::admins_add(d, id, &hash));
			info!("{}: console: '{}' granted admin to '{}'", id, admin.username, hash);
		}
		"remove"	=> {
			// A config-pinned admin is the operator's, and stays effective whatever the database says;
			// removing it here would be a lie the union unpicks, so it is refused outright.
			if site_admins.iter().any(|a| a == &hash) {
				return Ok(admins_deny(
					json, "that administrator is pinned in the site's configuration and cannot be \
					removed here"));
			}
			res!(store::admins_remove(d, id, &hash));
			info!("{}: console: '{}' revoked admin from '{}'", id, admin.username, hash);
		}
		other	=> return Ok(admins_deny(json, &fmt!("'{}' is not an action here", other))),
	}

	Ok(if json {
		json_ok()
	} else {
		redirect(PATH_ADMINS)
	})
}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ PASSPHRASE SIGN-IN                                                        │
// └───────────────────────────────────────────────────────────────────────────┘

/// The passphrase sign-in: verify the operator's wallet passphrase and, on
/// success, issue a site-admin session for this console.
///
/// The passphrase is checked with the same [`auth::verify_passphrase`] the
/// `/admin` dashboard uses, so there is one credential and one check, not two
/// that could drift. It is never logged. Success answers a fetch caller with
/// `{"ok":true}` and a browser form with a 303 to the console, both carrying the
/// `Set-Cookie`; failure answers `{"ok":false,"error":...}` or re-renders the
/// themed form. The session it mints opens this console alone -- it is not, and
/// cannot become, an `/admin` operator session.
fn do_login(
	admin_state:	Option<&AdminState>,
	publish:	Option<&PublishConfig>,
	headers:	&Arc<HeaderFields>,
	body:		&[u8],
	peer:		SocketAddr,
	id:		&str,
)
	-> HttpMessage
{
	let theme = Theme::of(publish);
	let json = wants_json(headers);

	// A site whose vhost has no admin state configured cannot verify a wallet
	// passphrase, so it has no passphrase sign-in to offer.
	let state = match admin_state {
		Some(s)	=> s,
		None	=> {
			warn!("{}: console: a passphrase login arrived where no admin state is configured", id);
			return login_deny(&theme, json, "sign-in is not available on this site");
		}
	};

	let passphrase = match form_field(body, "passphrase") {
		Some(p)	=> p,
		None	=> return login_deny(&theme, json, "a passphrase is required"),
	};

	// The passphrase is proven against the wallet here. Whatever the outcome, it
	// is never written to a log line.
	let outcome = match auth::verify_passphrase(state, passphrase.as_bytes(), peer) {
		Ok(o)	=> o,
		Err(e)	=> {
			error!(e, "{}: console: structural error verifying a manage passphrase", id);
			return login_deny(&theme, json, "an internal error prevented sign-in");
		}
	};

	match outcome {
		LoginOutcome::Ok(principal) => {
			let value = match session::encode(state, &principal.name) {
				Ok(v)	=> v,
				Err(e)	=> {
					error!(e, "{}: console: could not encode a manage session", id);
					return login_deny(&theme, json, "sign-in succeeded but the session could not be issued");
				}
			};
			info!("{}: console: '{}' signed in to manage via the wallet passphrase", id, principal.name);
			let cookie = build_manage_cookie(value, false);
			if json {
				json_ok().set_cookie(cookie)
			} else {
				redirect(PATH_ROOT).set_cookie(cookie)
			}
		}
		LoginOutcome::BadCredentials => {
			warn!("{}: console: a passphrase login failed on credentials from {}", id, peer.ip());
			login_deny(&theme, json, "the passphrase was not accepted")
		}
		LoginOutcome::NoDashboardScope { name } => {
			// The passphrase unwrapped the wallet but the admin holds no dashboard
			// scope. The console mirrors the dashboard's own rule: an admin gated
			// out of the dashboard is gated out of the console.
			warn!("{}: console: '{}' authenticated but holds no dashboard scope", id, name);
			login_deny(&theme, json, "this account is not authorised to manage sites")
		}
	}
}

/// The sign-out: clear the site-admin session and send the caller on.
///
/// Stateless, like the operator logout: the session lives only in the cookie, so
/// evicting the cookie is the whole of it. A fetch caller gets `{"ok":true}`, a
/// browser a redirect to the site home.
fn do_logout(
	publish:	Option<&PublishConfig>,
	headers:	&Arc<HeaderFields>,
)
	-> HttpMessage
{
	let json = wants_json(headers);
	let cookie = build_manage_cookie(String::new(), true);
	if json {
		json_ok().set_cookie(cookie)
	} else {
		redirect(&home_of(publish)).set_cookie(cookie)
	}
}

/// A sign-in that did not go through: the reason for a fetch caller as
/// `{"ok":false,"error":...}`, or the themed form again for a browser.
///
/// The reasons are deliberately plain and do not distinguish a wrong passphrase
/// from an unauthorised admin -- the same discretion the dashboard login keeps,
/// so the response never says whether a given passphrase was close.
fn login_deny(theme: &Theme, json: bool, why: &str) -> HttpMessage {
	if json {
		HttpMessage::new_response(HttpStatus::OK)
			.with_field(
				HeaderName::ContentType,
				HeaderFieldValue::Generic("application/json".to_string()),
			)
			.with_body(fmt!("{{\"ok\":false,\"error\":\"{}\"}}", json_escape(why)).into_bytes())
	} else {
		login_page(theme, Some(why))
	}
}

/// Build the `Set-Cookie` value for the site-admin session under
/// [`session::MANAGE_COOKIE_NAME`].
///
/// `Path=/` so the browser sends it to `/manage` -- unlike the operator cookie's
/// `Path=/admin`, which never reaches here. `HttpOnly` so no script reads it,
/// `Secure` so it rides only TLS, and `SameSite=Strict` so no cross-site request
/// carries it. `clear` produces the `Max-Age=0` eviction the sign-out uses.
fn build_manage_cookie(value: String, clear: bool) -> Cookie {
	let mut attrs: BTreeSet<SetCookieAttributes> = BTreeSet::new();
	attrs.insert(SetCookieAttributes::Path("/".to_string()));
	attrs.insert(SetCookieAttributes::HttpOnly);
	attrs.insert(SetCookieAttributes::Secure);
	attrs.insert(SetCookieAttributes::SameSite(SameSite::Strict));
	if clear {
		attrs.insert(SetCookieAttributes::MaxAge(0));
	}
	Cookie {
		key:	session::MANAGE_COOKIE_NAME.to_string(),
		val:	value,
		attrs:	Some(attrs),
	}
}

/// The themed passphrase login page: one password field, posting to
/// [`PATH_LOGIN`], dressed in the site's own look.
///
/// Rendered for any visitor who reaches the console without an admin session, so
/// every site with a console has a sign-in for free, in its own skin, with no
/// app code. A bespoke front-end may replace it with a popup over the same
/// endpoint, but it is never required. No CSRF token: there is no session yet to
/// protect, and the credential is the operator's own passphrase.
fn login_page(theme: &Theme, error: Option<&str>) -> HttpMessage {
	let notice = match error {
		Some(msg)	=> fmt!("<p class=\"mc-notice mc-notice-err\">{}</p>\n", html_escape(msg)),
		None		=> String::new(),
	};
	let body = fmt!(
		"<h1>Manage this site</h1>\n\
		<p class=\"mc-muted\">Sign in with the site's management passphrase to write its posts and \
		settings.</p>\n\
		{notice}\
		<form class=\"mc-form\" id=\"mc-login\" method=\"POST\" action=\"{login}\">\n\
		<label for=\"mc-passphrase\">Passphrase</label>\n\
		<input type=\"password\" id=\"mc-passphrase\" name=\"passphrase\" autocomplete=\"current-password\" \
		autofocus required>\n\
		<div class=\"mc-actions\">\n\
		<button type=\"submit\" class=\"mc-btn\" id=\"mc-login-btn\">Sign in</button>\n\
		</div>\n\
		</form>\n",
		notice	= notice,
		login	= PATH_LOGIN,
	);

	// A bare themed page, without the admin nav: the visitor is not an admin yet,
	// so a nav to pages they cannot open would only mislead.
	let mut s = String::new();
	s.push_str("<!DOCTYPE html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n");
	s.push_str("<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n");
	s.push_str("<meta name=\"robots\" content=\"noindex\">\n<title>");
	if !theme.site_name.is_empty() {
		s.push_str(&html_escape(&theme.site_name));
		s.push_str(" — ");
	}
	s.push_str("manage</title>\n");
	for href in &theme.css {
		s.push_str("<link rel=\"stylesheet\" href=\"");
		s.push_str(&html_escape(href));
		s.push_str("\">\n");
	}
	s.push_str("<style>\n");
	s.push_str(CONSOLE_CSS);
	s.push_str("</style>\n</head>\n<body class=\"mc-body\">\n<main class=\"mc-main\">\n");
	s.push_str(&body);
	s.push_str("</main>\n</body>\n</html>\n");

	HttpMessage::new_response(HttpStatus::OK)
		.with_field(
			HeaderName::ContentType,
			HeaderFieldValue::Generic("text/html; charset=utf-8".to_string()),
		)
		.with_body(s.into_bytes())
}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ CHROME                                                                    │
// └───────────────────────────────────────────────────────────────────────────┘

/// What a console page needs to look like the site it belongs to.
///
/// Drawn from the site's publish block, because that is where a site already says its name and names
/// its stylesheets, and the console wearing the same look means the operator never leaves the site to
/// manage it. A site with a console but no publish block gets a plain page that still works -- the
/// look is a courtesy, the function is not.
pub struct Theme {
	/// The site's name, for the tab and the header.
	pub site_name:	String,
	/// The site's own stylesheets, linked so the console inherits its palette and fonts.
	pub css:	Vec<String>,
	/// Where "View site" goes.
	pub home:	String,
}

impl Theme {

	/// The theme a site's publish block gives its console.
	fn of(publish: Option<&PublishConfig>) -> Self {
		match publish {
			Some(p)	=> Self {
				site_name:	p.site_name.clone(),
				css:		p.css.clone(),
				home:		if p.base_url.is_empty() { fmt!("/") } else { p.base_url.clone() },
			},
			None	=> Self {
				site_name:	String::new(),
				css:		Vec::new(),
				home:		fmt!("/"),
			},
		}
	}
}

/// Where a turned-away visitor is sent.
fn home_of(publish: Option<&PublishConfig>) -> String {
	match publish {
		Some(p) if !p.base_url.is_empty()	=> p.base_url.clone(),
		_					=> fmt!("/"),
	}
}

/// Wraps a console body in the site's own look.
///
/// The site's stylesheets are linked for their custom properties -- colours, fonts -- and a small
/// sheet of the console's own, which consumes those properties where they are set and falls back
/// where they are not, dresses the forms and tables the site's own stylesheets never had a reason to.
/// So the console reads as part of the site without the site having authored a line of admin styling.
pub fn page(theme: &Theme, admin: &SiteAdmin, title: &str, body: &str) -> HttpMessage {
	let mut s = String::new();
	s.push_str("<!DOCTYPE html>\n<html lang=\"en\">\n<head>\n");
	s.push_str("<meta charset=\"utf-8\">\n");
	s.push_str("<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n");
	s.push_str("<meta name=\"robots\" content=\"noindex\">\n");

	s.push_str("<title>");
	s.push_str(&html_escape(title));
	if !theme.site_name.is_empty() {
		s.push_str(" — ");
		s.push_str(&html_escape(&theme.site_name));
		s.push_str(" manage");
	}
	s.push_str("</title>\n");

	for href in &theme.css {
		s.push_str("<link rel=\"stylesheet\" href=\"");
		s.push_str(&html_escape(href));
		s.push_str("\">\n");
	}
	s.push_str("<style>\n");
	s.push_str(CONSOLE_CSS);
	s.push_str("</style>\n");

	s.push_str("</head>\n<body class=\"mc-body\">\n");

	// The header: whose site, that this is the management of it, and the two ways out -- back to the
	// site, and sign out.
	s.push_str("<header class=\"mc-head\">\n<div class=\"mc-head-in\">\n");
	s.push_str("<div class=\"mc-brand\">");
	if !theme.site_name.is_empty() {
		s.push_str(&html_escape(&theme.site_name));
	} else {
		s.push_str("Manage");
	}
	s.push_str(" <span class=\"mc-brand-sub\">manage</span></div>\n");
	s.push_str("<nav class=\"mc-nav\">");
	s.push_str(&fmt!("<a href=\"{}\">Posts</a>", PATH_ROOT));
	s.push_str(&fmt!("<a href=\"{}\">Subscribers</a>", publish::PATH_SUBS));
	s.push_str(&fmt!("<a href=\"{}\">Reports</a>", publish::PATH_REPORTS));
	s.push_str(&fmt!("<a href=\"{}\">Comments</a>", publish::PATH_COMMENTS));
	s.push_str(&fmt!("<a href=\"{}\">Destinations</a>", publish::PATH_DESTS));
	s.push_str(&fmt!("<a href=\"{}\">Profile</a>", publish::PATH_PROFILE));
	s.push_str(&fmt!("<span class=\"mc-who\">{}…</span>", html_escape(&admin.username[..8.min(admin.username.len())])));
	// The way out of the console is a close, in the corner, as it is on every page within it --
	// rather than a link competing for attention with the pages themselves.
	s.push_str(&fmt!(
		"<a class=\"mc-close\" href=\"{home}\" title=\"Back to the site\" \
		aria-label=\"Back to the site\">{close}</a>",
		home	= html_escape(&theme.home),
		close	= publish::icon_close(),
	));
	s.push_str("</nav>\n");
	s.push_str("</div>\n</header>\n");

	s.push_str("<main class=\"mc-main\">\n");
	s.push_str(body);
	s.push_str("</main>\n</body>\n</html>\n");

	HttpMessage::new_response(HttpStatus::OK)
		.with_field(
			HeaderName::ContentType,
			HeaderFieldValue::Generic("text/html; charset=utf-8".to_string()),
		)
		.with_body(s.into_bytes())
}

/// What a signed-in member who is not an admin is shown.
///
/// Two pages in one, told apart by `claimable`. Where the site has no admins at all, this is the
/// bootstrap: the member may claim it, and is shown the button that makes them the first admin, POSTing
/// the claim with its CSRF token. Where the site already has admins, they are shown their own id, to
/// hand to an existing admin who can add them. A page, not a redirect, because there is something here
/// for them to read and act on -- which a member sent silently home would never find.
fn not_yet_admin(theme: &Theme, username: &str, claimable: bool, csrf: &str) -> HttpMessage {
	let body = if claimable {
		// No admins yet, so this member may make themselves the first. The button is the whole
		// bootstrap: no config edit, no restart, no operator.
		fmt!(
			"<h1>Claim this site</h1>\n\
			<p class=\"mc-muted\">No one administers this site yet. You are signed in, so you can \
			claim it and become its first administrator. From there you can add others.</p>\n\
			<form method=\"POST\" action=\"{claim}\">\n\
			<input type=\"hidden\" name=\"csrf\" value=\"{csrf}\">\n\
			<div class=\"mc-actions\">\n\
			<button type=\"submit\" class=\"mc-btn\" id=\"mc-claim\">Claim this site as admin</button>\n\
			</div>\n\
			</form>\n\
			<p class=\"mc-muted\">Your account's id is <code>{id}</code>. \
			<a href=\"{home}\">Back to the site.</a></p>\n",
			claim	= PATH_CLAIM,
			csrf	= html_escape(csrf),
			id	= html_escape(username),
			home	= html_escape(&theme.home),
		)
	} else {
		fmt!(
			"<h1>Not your site to manage — yet</h1>\n\
			<p class=\"mc-muted\">You are signed in, but you are not one of this site's administrators. \
			If you should be, give an existing administrator this id and ask to be added:</p>\n\
			<p class=\"mc-notice\"><code>{id}</code></p>\n\
			<p class=\"mc-muted\">It is not a secret; it is the public name of your account, and knowing \
			it does not let anyone sign in as you. <a href=\"{home}\">Back to the site.</a></p>\n",
			id	= html_escape(username),
			home	= html_escape(&theme.home),
		)
	};
	// A bare page in the same chrome, but without the admin nav: they are not one, so it would name
	// pages they cannot open.
	let mut s = String::new();
	s.push_str("<!DOCTYPE html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n");
	s.push_str("<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n");
	s.push_str("<meta name=\"robots\" content=\"noindex\">\n<title>Manage</title>\n");
	for href in &theme.css {
		s.push_str("<link rel=\"stylesheet\" href=\"");
		s.push_str(&html_escape(href));
		s.push_str("\">\n");
	}
	s.push_str("<style>\n");
	s.push_str(CONSOLE_CSS);
	s.push_str("</style>\n</head>\n<body class=\"mc-body\">\n<main class=\"mc-main\">\n");
	s.push_str(&body);
	s.push_str("</main>\n</body>\n</html>\n");

	HttpMessage::new_response(HttpStatus::Forbidden)
		.with_field(
			HeaderName::ContentType,
			HeaderFieldValue::Generic("text/html; charset=utf-8".to_string()),
		)
		.with_body(s.into_bytes())
}

/// The console's own styling, consuming the site's custom properties where they are set.
///
/// Every colour and font falls back to a neutral default, so a site that defines none still gets a
/// legible page; a site that defines the Elearnity-style tokens gets its own palette. Kept small and
/// inline: it is chrome for a handful of pages, not a stylesheet worth a request.
const CONSOLE_CSS: &str = "\
.mc-body{margin:0;background:var(--bg-primary,var(--body-bg,#14181d));color:var(--text-primary,var(--body-color,#e6e6e6));\
font-family:var(--font,var(--font-ui,var(--font-body,system-ui,sans-serif)));line-height:1.5;}\
.mc-head{border-bottom:1px solid var(--border,var(--aside-rule-color,#333c47));}\
.mc-head-in{max-width:80rem;margin:0 auto;padding:0.9rem 1.2rem;display:flex;\
align-items:baseline;justify-content:space-between;gap:1rem;flex-wrap:wrap;}\
.mc-brand{font-weight:600;font-size:1.05rem;}\
.mc-brand-sub{color:var(--text-secondary,var(--aside-date-color,#8a97a6));font-weight:400;font-size:0.8rem;\
text-transform:uppercase;letter-spacing:0.08em;}\
.mc-nav{display:flex;align-items:center;gap:1.1rem;font-size:0.9rem;}\
.mc-nav a{color:var(--accent,var(--aside-link-color,#7fb0e0));text-decoration:none;}\
.mc-nav a:hover{text-decoration:underline;}\
.mc-who{color:var(--text-secondary,var(--aside-date-color,#8a97a6));font-family:var(--font-mono,monospace);font-size:0.8rem;}\
/* A management screen is tables and side-by-side panes, not prose, so it takes the width it is \
   given. Running text inside it is held to a readable measure separately, below. */\
.mc-main{max-width:80rem;margin:0 auto;padding:1.4rem 1.2rem 4rem;}\
/* The whole heading scale, not just the first rung. Setting h1 alone leaves h2 and h3 to the \
   site's own stylesheet, whose scale is built for prose -- and a site whose h2 is larger than \
   the console's h1 inverts the hierarchy on every page that has a section in it. */\
.mc-main h1{font-size:1.5rem;margin:0 0 0.3rem;}\
.mc-main h2{font-size:1.15rem;margin:2rem 0 0.4rem;}\
.mc-main h3{font-size:0.95rem;margin:1.4rem 0 0.3rem;\
color:var(--text-secondary,var(--aside-date-color,#8a97a6));}\
.mc-main h1:first-child,.mc-main h2:first-child{margin-top:0;}\
.mc-muted{color:var(--text-secondary,var(--aside-date-color,#8a97a6));font-size:0.9rem;margin:0 0 1.4rem;}\
.mc-notice{border:1px solid var(--border,var(--aside-rule-color,#333c47));border-radius:6px;\
padding:0.8rem 1rem;margin:0 0 1.2rem;}\
.mc-notice code{font-family:var(--font-mono,monospace);font-size:0.85em;}\
.mc-notice-err{border-color:#c0554e;color:#d9776f;}\
.mc-btn,button.mc-btn{display:inline-block;font:inherit;font-size:0.9rem;cursor:pointer;\
padding:0.5rem 0.9rem;border-radius:6px;border:1px solid var(--accent,var(--aside-link-color,#7fb0e0));\
background:var(--accent,var(--aside-link-color,#7fb0e0));color:var(--bg-primary,var(--body-bg,#14181d));text-decoration:none;}\
.mc-btn:hover{opacity:0.9;text-decoration:none;}\
/* The modifiers name the element as well, because the base rule does. `button.mc-btn` outranks \
   a bare `.mc-btn-quiet`, so without this every quiet and every dangerous BUTTON -- erase, \
   unsubscribe, import, filter -- draws itself as the loud primary one, while the same class on \
   an <a> behaves. They looked like three different consoles. */\
.mc-btn-quiet,button.mc-btn-quiet{background:transparent;\
color:var(--accent,var(--aside-link-color,#7fb0e0));}\
.mc-btn-danger,button.mc-btn-danger{background:transparent;border-color:#c0554e;color:#d9776f;}\
table.mc-table{width:100%;border-collapse:collapse;margin:0.4rem 0 1.6rem;font-size:0.92rem;}\
.mc-table th{text-align:left;font-size:0.75rem;text-transform:uppercase;letter-spacing:0.06em;\
color:var(--text-secondary,var(--aside-date-color,#8a97a6));border-bottom:1px solid var(--border,var(--aside-rule-color,#333c47));\
padding:0.4rem 0.6rem;}\
.mc-table td{border-bottom:1px solid var(--border,var(--aside-rule-color,#333c47));padding:0.55rem 0.6rem;\
vertical-align:top;}\
.mc-table a{color:var(--accent,var(--aside-link-color,#7fb0e0));text-decoration:none;}\
.mc-table a:hover{text-decoration:underline;}\
.mc-slug{color:var(--text-secondary,var(--aside-date-color,#8a97a6));font-family:var(--font-mono,monospace);font-size:0.8rem;}\
.mc-tag{display:inline-block;font-size:0.72rem;text-transform:uppercase;letter-spacing:0.05em;\
padding:0.1rem 0.45rem;border-radius:4px;border:1px solid var(--border,var(--aside-rule-color,#333c47));\
color:var(--text-secondary,var(--aside-date-color,#8a97a6));}\
.mc-tag-live{border-color:#4f8f57;color:#7bc084;}\
.mc-tag-err{border-color:#c0554e;color:#d9776f;}\
.mc-stats{display:grid;grid-template-columns:repeat(auto-fit,minmax(9rem,1fr));gap:0.8rem;\
margin:0.6rem 0 1.4rem;}\
.mc-stat{border:1px solid var(--border,var(--aside-rule-color,#333c47));border-radius:6px;\
padding:0.8rem 0.9rem;}\
.mc-stat-n{font-size:1.7rem;line-height:1.1;color:var(--text,var(--aside-text-color,#d8dee6));}\
.mc-stat-k{font-size:0.75rem;text-transform:uppercase;letter-spacing:0.06em;margin-top:0.3rem;\
color:var(--accent,var(--aside-link-color,#7fb0e0));}\
.mc-stat-note{font-size:0.78rem;margin-top:0.2rem;\
color:var(--text-secondary,var(--aside-date-color,#8a97a6));}\
.mc-bar{background:var(--border,var(--aside-rule-color,#333c47));border-radius:3px;height:0.5rem;\
min-width:6rem;}\
.mc-bar-fill{background:var(--accent,var(--aside-link-color,#7fb0e0));border-radius:3px;height:100%;}\
/* A page's own title row: the heading on the left, the way out on the right. */\
.mc-head-row{display:flex;align-items:center;justify-content:space-between;gap:1rem;margin:0 0 0.6rem;}\
.mc-head-row h1{margin:0;}\
.mc-head-row .mc-actions{margin-top:0;}\
/* The close: an icon, not a word. Same corner on every page that can be left. */\
.mc-close{display:inline-flex;align-items:center;justify-content:center;width:1.8rem;height:1.8rem;\
border-radius:6px;color:var(--text-secondary,var(--aside-date-color,#8a97a6));text-decoration:none;\
border:1px solid transparent;}\
.mc-close:hover{color:var(--text,var(--aside-text-color,#d8dee6));\
border-color:var(--border,var(--aside-rule-color,#333c47));}\
.mc-close svg{width:1.15rem;height:1.15rem;display:block;}\
/* A row action: an icon button sized to the row, quiet until pointed at. */\
.mc-ico{display:inline-flex;align-items:center;justify-content:center;width:2rem;height:2rem;padding:0;\
background:transparent;border:1px solid transparent;border-radius:6px;cursor:pointer;\
color:var(--text-secondary,var(--aside-date-color,#8a97a6));text-decoration:none;}\
.mc-ico:hover{color:var(--text,var(--aside-text-color,#d8dee6));\
border-color:var(--border,var(--aside-rule-color,#333c47));}\
.mc-ico-danger:hover{color:#d9776f;border-color:#c0554e;}\
.mc-ico svg{width:1.05rem;height:1.05rem;display:block;}\
.mc-table .mc-actions{margin-top:0;gap:0.15rem;flex-wrap:nowrap;justify-content:flex-end;}\
/* The editor beside its preview: stacked on a narrow screen, side by side where there is room. \
   Equal columns, so neither the prose nor its rendering is the afterthought. */\
.mc-split{display:grid;grid-template-columns:1fr;gap:1rem;align-items:stretch;}\
@media (min-width:60rem){.mc-split{grid-template-columns:1fr 1fr;}}\
/* Both panes fill the row, so the prose and its rendering are the same height. Left to \
   themselves a textarea takes its rows attribute and the preview takes its content, and the \
   two sit side by side at visibly different sizes. */\
.mc-pane{min-width:0;display:flex;flex-direction:column;}\
.mc-pane label{margin-top:0;}\
.mc-pane textarea,.mc-pane .mc-preview{flex:1 1 auto;min-height:26rem;}\
.mc-preview{border:1px solid var(--border,var(--aside-rule-color,#333c47));border-radius:6px;\
padding:0.9rem 1rem;overflow:auto;\
background:var(--bg-secondary,var(--aside-bg,transparent));}\
.mc-preview>*:first-child{margin-top:0;}\
.mc-preview img{max-width:100%;height:auto;}\
/* Filter and pager: the two things a list needs once it stops fitting on a screen. */\
.mc-filter{display:flex;gap:0.6rem;align-items:flex-end;flex-wrap:wrap;margin:0 0 1rem;}\
.mc-filter label{margin:0 0 0.25rem;}\
/* The button stands on the same line as the boxes it acts on, so it is the same height as \
   them -- a padding-sized button beside a fixed-height input is a step in the row. */\
.mc-filter .mc-btn{height:2.4rem;padding-top:0;padding-bottom:0;display:inline-flex;align-items:center;}\
.mc-filter .mc-f-text{flex:1 1 14rem;}\
.mc-filter .mc-f-sel{flex:0 0 9rem;}\
.mc-pager{display:flex;gap:0.5rem;align-items:center;justify-content:flex-end;margin:0 0 1.5rem;\
font-size:0.85rem;color:var(--text-secondary,var(--aside-date-color,#8a97a6));}\
.mc-pager a{color:var(--accent,var(--aside-link-color,#7fb0e0));text-decoration:none;\
border:1px solid var(--border,var(--aside-rule-color,#333c47));border-radius:6px;padding:0.25rem 0.6rem;}\
.mc-pager a:hover{text-decoration:underline;}\
.mc-pager .mc-pager-at{padding:0.25rem 0.2rem;}\
/* A form is as wide as its longest field wants to be, which for an address is not the page. */\
.mc-send .mc-form,.mc-send .mc-notice{max-width:34rem;}\
.mc-form label{display:block;font-size:0.8rem;text-transform:uppercase;letter-spacing:0.05em;\
color:var(--text-secondary,var(--aside-date-color,#8a97a6));margin:1rem 0 0.3rem;}\
.mc-form input[type=text],.mc-form input[type=password],.mc-form input[type=email],.mc-form select,.mc-form textarea{width:100%;box-sizing:border-box;\
font:inherit;background:var(--bg-tertiary,var(--input-bg,#0e1216));color:var(--input-color,inherit);\
border:1px solid var(--border,var(--aside-rule-color,#333c47));border-radius:6px;padding:0.5rem 0.6rem;}\
/* One height for every control on a row. A select carries its own intrinsic height and a text \
input another, so a row of them steps up and down unless both are told the same number. */\
.mc-form input[type=text],.mc-form input[type=password],.mc-form input[type=email],.mc-form select{\
height:2.4rem;line-height:normal;padding-top:0;padding-bottom:0;}\
.mc-form textarea{min-height:22rem;font-family:var(--font-mono,monospace);font-size:0.9rem;\
line-height:1.5;resize:vertical;}\
.mc-row{display:flex;gap:1rem;flex-wrap:wrap;}\
.mc-row>div{flex:1 1 8rem;}\
.mc-actions{margin-top:1.2rem;display:flex;gap:0.7rem;align-items:center;flex-wrap:wrap;}\
/* Prose keeps a readable measure even where the page around it is wide. */\
.mc-prose{max-width:40rem;border:1px solid var(--border,var(--aside-rule-color,#333c47));border-radius:6px;padding:1.2rem 1.4rem;\
margin-top:0.8rem;}\
/* A settings form keeps a measure too. The page is 80rem because it holds tables and\
   side-by-side panes; a field for a handle or a host is not made better by being 80rem\
   of it, and a row of them that wide reads as unconsidered rather than spacious. */\
.mc-settings{max-width:34rem;}\
/* The moderation queue. A comment is shown rendered, framed, with its verbs beneath: a\
   decision about what to publish is made looking at what would be published. */\
.mc-comment{border:1px solid var(--border,var(--aside-rule-color,#333c47));border-radius:6px;\
padding:0.9rem 1.1rem;margin:0.8rem 0;}\
.mc-comment-by{font-size:0.92rem;margin-bottom:0.2rem;}\
/* The post a comment is on, in the console's own link colour: without this the anchor falls\
   through to the browser default blue, which no other link on any console page wears. */\
.mc-comment-by a{color:var(--accent,var(--aside-link-color,#7fb0e0));text-decoration:none;}\
.mc-comment-by a:hover{text-decoration:underline;}\
.mc-comment-why{font-size:0.85rem;margin:0.1rem 0 0.4rem;}\
.mc-comment-body{margin:0.5rem 0 0.7rem;}\
.mc-comment-acts{display:flex;gap:0.5rem;flex-wrap:wrap;align-items:center;}\
.mc-inline{display:inline;}\
/* The site's comments switch, above the queue: the state, the control, and what it will do. */\
.mc-switch{display:flex;align-items:center;gap:0.8rem;flex-wrap:wrap;margin:0.6rem 0 1.2rem;\
padding:0.8rem 1rem;border:1px solid var(--border,var(--aside-rule-color,#333c47));border-radius:6px;}\
.mc-switch-state{font-size:0.95rem;}\
.mc-tag{display:inline-block;font-size:0.72rem;text-transform:uppercase;letter-spacing:0.06em;\
padding:0.1rem 0.4rem;border-radius:4px;border:1px solid var(--border,#333c47);opacity:0.75;}\
.mc-tag-live{border-color:#4c9a6a;color:#7fc79b;opacity:1;}\
.mc-tag-err{border-color:#c0554e;color:#d9776f;opacity:1;}\
.mc-settings + .mc-settings{margin-top:0.6rem;}\
.mc-author{display:flex;align-items:flex-end;gap:0.4rem;font-size:0.85rem;\
color:var(--text-secondary,var(--aside-date-color,#8a97a6));}\
.mc-author-lbl{text-transform:uppercase;letter-spacing:0.05em;font-size:0.72rem;}\
.mc-author-name{color:var(--text-primary,var(--body-color,#e6e6e6));font-weight:600;}\
.mc-cats-field{margin:0.2rem 0 0.9rem;}\
.mc-cats{display:flex;flex-wrap:wrap;gap:0.4rem 1rem;margin:0.3rem 0 0;}\
.mc-cat{display:inline-flex;align-items:center;gap:0.35rem;font-size:0.85rem;text-transform:none;\
letter-spacing:0;cursor:pointer;color:var(--text-secondary,var(--aside-date-color,#8a97a6));}\
.mc-tags-field{margin:0.2rem 0 0.9rem;}\
.mc-tags-boxes{display:grid;grid-template-columns:1fr 1fr;gap:0.8rem;margin:0.3rem 0 0;}\
.mc-tagbox{min-width:0;}\
.mc-tagbox-lbl{display:block;font-size:0.72rem;text-transform:uppercase;letter-spacing:0.06em;\
margin:0 0 0.35rem;color:var(--text-secondary,var(--aside-date-color,#8a97a6));}\
.mc-tags-search{width:100%;box-sizing:border-box;font:inherit;font-size:0.85rem;\
padding:0.35rem 0.55rem;margin:0 0 0.4rem;border-radius:6px;\
border:1px solid var(--border,var(--aside-rule-color,#333c47));background:transparent;\
color:var(--text-primary,var(--body-color,#e6e6e6));}\
.mc-chips{display:flex;flex-wrap:wrap;gap:0.4rem;align-content:flex-start;min-height:2.6rem;\
padding:0.45rem;border-radius:6px;border:1px dashed var(--border,var(--aside-rule-color,#333c47));}\
.mc-chips.mc-drop{border-style:solid;border-color:var(--accent,var(--aside-link-color,#7fb0e0));}\
.mc-chip{display:inline-flex;align-items:center;gap:0.25rem;font:inherit;font-size:0.82rem;\
cursor:pointer;padding:0.14rem 0.55rem;border-radius:999px;user-select:none;\
border:1px solid var(--border,var(--aside-rule-color,#333c47));background:transparent;\
color:var(--text-primary,var(--body-color,#e6e6e6));}\
.mc-chip:hover{border-color:var(--accent,var(--aside-link-color,#7fb0e0));}\
.mc-chips-selected .mc-chip{background:var(--accent,var(--aside-link-color,#3b6ea5));\
border-color:var(--accent,var(--aside-link-color,#3b6ea5));color:#fff;}\
.mc-chip-x,.mc-chip-del{font-size:0.95rem;line-height:1;opacity:0.75;}\
.mc-chip-del{color:#e57373;}\
.mc-chip:hover .mc-chip-x,.mc-chip:hover .mc-chip-del{opacity:1;}\
.mc-avatar-row{margin:0 0 1rem;}\
.mc-avatar-pic,.mc-avatar-initial{width:4rem;height:4rem;border-radius:50%;object-fit:cover;\
display:inline-flex;align-items:center;justify-content:center;font-size:1.6rem;font-weight:600;\
color:#fff;background:var(--accent,var(--aside-link-color,#3b6ea5));}\
.mc-hint{font-size:0.8rem;color:var(--text-secondary,var(--aside-date-color,#8a97a6));margin:0.3rem 0 0;}\
@media (max-width:32rem){.mc-tags-boxes{grid-template-columns:1fr;}}\
";


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ HELPERS                                                                   │
// └───────────────────────────────────────────────────────────────────────────┘

/// The status answer: whether the asker may manage this site, and if so the token their writes need.
///
/// The token is safe to hand out here: it proves a request came from a page that holds the session,
/// and only a caller with the session cookie reaches this with `admin` true. A cross-site page has
/// neither the cookie (it is `SameSite=Lax`, unsent on a cross-site request) nor the reply (the
/// same-origin policy hides it), so it learns nothing.
fn status_json(
	admin:		bool,
	claimable:	bool,
	csrf:		Option<&str>,
	dests:		&[&str],
	categories:	&[String],
)
	-> HttpMessage
{
	// The destinations the site can offer, as a JSON array. The words are a fixed vocabulary
	// (`Destination::as_str`), so they need no escaping.
	let items: Vec<String> = dests.iter().map(|d| fmt!("\"{}\"", d)).collect();
	let dest_arr = fmt!("[{}]", items.join(","));
	// The categories, as a JSON array. Unlike the destinations these are free config strings, so each
	// is escaped for a JSON string literal -- a quote or a backslash in a category name must not break
	// the document.
	let cat_items: Vec<String> = categories.iter().map(|c| {
		let mut s = String::from("\"");
		for ch in c.chars() {
			match ch {
				'"'	=> s.push_str("\\\""),
				'\\'	=> s.push_str("\\\\"),
				c if (c as u32) < 0x20	=> s.push_str(&fmt!("\\u{:04x}", c as u32)),
				c	=> s.push(c),
			}
		}
		s.push('"');
		s
	}).collect();
	let cat_arr = fmt!("[{}]", cat_items.join(","));
	let body = match csrf {
		Some(t)	=> fmt!(
			"{{\"admin\":{},\"claimable\":{},\"csrf\":\"{}\",\"destinations\":{},\"categories\":{}}}",
			admin, claimable, t, dest_arr, cat_arr),
		None	=> fmt!(
			"{{\"admin\":{},\"claimable\":{},\"destinations\":{},\"categories\":{}}}",
			admin, claimable, dest_arr, cat_arr),
	};
	HttpMessage::new_response(HttpStatus::OK)
		.with_field(
			HeaderName::ContentType,
			HeaderFieldValue::Generic("application/json".to_string()),
		)
		.with_body(body.into_bytes())
}

/// A redirect, for turning a visitor away or sending them back after a write.
pub fn redirect(to: &str) -> HttpMessage {
	HttpMessage::new_response(HttpStatus::SeeOther)
		.with_field(
			HeaderName::Location,
			HeaderFieldValue::Generic(to.to_string()),
		)
}

/// A plain JSON yes, for a fetch caller whose write went through.
fn json_ok() -> HttpMessage {
	HttpMessage::new_response(HttpStatus::OK)
		.with_field(
			HeaderName::ContentType,
			HeaderFieldValue::Generic("application/json".to_string()),
		)
		.with_body("{\"ok\":true}".to_string().into_bytes())
}

/// A plain JSON error a fetch caller can read, its reason escaped for a string literal.
fn json_err(why: &str) -> HttpMessage {
	HttpMessage::new_response(HttpStatus::OK)
		.with_field(
			HeaderName::ContentType,
			HeaderFieldValue::Generic("application/json".to_string()),
		)
		.with_body(fmt!("{{\"error\":\"{}\"}}", json_escape(why)).into_bytes())
}

/// A CSRF refusal, in the shape the caller asked for: JSON for a fetch, a redirect home for a form.
///
/// The same answer the post console gives, so a stale session fails one way wherever it is presented.
fn csrf_deny(json: bool) -> HttpMessage {
	if json {
		HttpMessage::new_response(HttpStatus::Forbidden)
			.with_field(
				HeaderName::ContentType,
				HeaderFieldValue::Generic("application/json".to_string()),
			)
			.with_body("{\"error\":\"stale session; reload\"}".to_string().into_bytes())
	} else {
		redirect(PATH_ROOT)
	}
}

/// A claim that did not go through: the reason for a fetch caller, or the manage page again for a form,
/// where the member lands back on the not-yet-admin view.
fn claim_deny(json: bool, why: &str) -> HttpMessage {
	if json {
		json_err(why)
	} else {
		redirect(PATH_ROOT)
	}
}

/// An admin-management write that did not go through: the reason for a fetch caller, or the admins page
/// again for a form, carrying the reason in the query it lands with.
fn admins_deny(json: bool, why: &str) -> HttpMessage {
	if json {
		json_err(why)
	} else {
		redirect(&fmt!("{}?said={}", PATH_ADMINS, query_encode(why)))
	}
}

/// The `said` field out of a raw query substring, url-decoded, so the admins page can show why a write
/// was refused.
fn said_field(query: &str) -> Option<String> {
	for pair in query.split('&') {
		let mut kv = pair.splitn(2, '=');
		let k = match kv.next() {
			Some(k)	=> k,
			None	=> continue,
		};
		let v = kv.next().unwrap_or("");
		if k == "said" {
			let val = form_decode(v);
			if val.is_empty() {
				return None;
			}
			return Some(val);
		}
	}
	None
}

/// Escapes a string for a JSON string literal: the two characters that would break out of one.
///
/// Enough for the reasons put through it, which are prose and the odd id-hash, never a control
/// character.
fn json_escape(s: &str) -> String {
	s.replace('\\', "\\\\").replace('"', "\\\"")
}

/// Percent-encodes a string for a query parameter, per RFC 3986 section 2.3.
fn query_encode(s: &str) -> String {
	let mut out = String::with_capacity(s.len());
	for b in s.as_bytes() {
		match *b {
			b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9'
			| b'-' | b'_' | b'.' | b'~'	=> out.push(*b as char),
			other				=> out.push_str(&fmt!("%{:02X}", other)),
		}
	}
	out
}

/// The token a form must carry back, derived from the session it was rendered for.
///
/// A value only a page that held the session cookie could have been handed: the cookie is `HttpOnly`,
/// so no script reads the session id, and the token is a one-way function of it, so no one derives
/// the token without it. A forged cross-site POST has neither.
pub fn csrf_token(sid: &str) -> String {
	let h = HashScheme::new_sha3_256().hash(&[sid.as_bytes(), CSRF_DOMAIN], []);
	hex(&h.as_hashform().as_vec())
}

/// Whether a token a form sent back matches the session the cookie names.
///
/// A plain comparison, and it can be: the token is not a secret to keep from timing, it is a value an
/// honest client already holds and a forger cannot compute. What it proves is provenance, not
/// identity -- the cookie proves identity.
fn csrf_ok(sid: &str, sent: &str) -> bool {
	!sent.is_empty() && sent == csrf_token(sid)
}

/// The domain-separator for the CSRF hash, so the token can never be some other digest of the same
/// session put to a different use.
const CSRF_DOMAIN: &[u8] = b"steel-site-console-csrf-v1";

/// Lowercase hex of some bytes.
fn hex(bytes: &[u8]) -> String {
	let mut s = String::with_capacity(bytes.len() * 2);
	for b in bytes {
		s.push_str(&fmt!("{:02x}", b));
	}
	s
}

/// One field out of an `x-www-form-urlencoded` body.
///
/// The console's own reader, so the console does not lean on the operator dashboard's -- the two
/// tiers share as little as they can, and a form field parser is not worth coupling them over.
pub fn form_field(body: &[u8], key: &str) -> Option<String> {
	let s = match std::str::from_utf8(body) {
		Ok(s)	=> s,
		Err(_)	=> return None,
	};
	for pair in s.split('&') {
		let mut kv = pair.splitn(2, '=');
		let k = match kv.next() {
			Some(k)	=> k,
			None	=> continue,
		};
		let v = kv.next().unwrap_or("");
		if form_decode(k) == key {
			return Some(form_decode(v));
		}
	}
	None
}

/// Decode an `x-www-form-urlencoded` value: `+` is a space, `%XX` a byte, a bad escape itself.
fn form_decode(s: &str) -> String {
	let b = s.as_bytes();
	let mut out = Vec::with_capacity(b.len());
	let mut i = 0;
	while i < b.len() {
		match b[i] {
			b'+' => {
				out.push(b' ');
				i += 1;
			}
			b'%' if i + 2 < b.len() => {
				match (nibble(b[i + 1]), nibble(b[i + 2])) {
					(Some(hi), Some(lo)) => {
						out.push((hi << 4) | lo);
						i += 3;
					}
					_ => {
						out.push(b[i]);
						i += 1;
					}
				}
			}
			c => {
				out.push(c);
				i += 1;
			}
		}
	}
	String::from_utf8_lossy(&out).into_owned()
}

/// One hex digit's value.
fn nibble(b: u8) -> Option<u8> {
	match b {
		b'0'..=b'9'	=> Some(b - b'0'),
		b'a'..=b'f'	=> Some(b - b'a' + 10),
		b'A'..=b'F'	=> Some(b - b'A' + 10),
		_		=> None,
	}
}


#[cfg(test)]
mod tests {
	use super::*;

	/// The console answers for its own prefix and nothing that merely starts like it.
	#[test]
	fn test_owns_its_prefix_00() -> Outcome<()> {
		assert!(owns("/manage"));
		assert!(owns("/manage/edit"));
		assert!(owns("/manage/status"));
		assert!(!owns("/manageable"));
		assert!(!owns("/admin"));
		assert!(!owns("/"));
		Ok(())
	}

	/// A token is a function of the session, so it holds for that session and no other.
	#[test]
	fn test_a_token_is_bound_to_its_session_01() -> Outcome<()> {
		let a = csrf_token("session-aaa");
		let b = csrf_token("session-bbb");
		assert_ne!(a, b, "two sessions produced the same token");
		assert!(csrf_ok("session-aaa", &a));
		assert!(!csrf_ok("session-aaa", &b), "another session's token passed");
		assert!(!csrf_ok("session-aaa", ""), "an empty token passed");
		assert_eq!(a.len(), 64, "a sha3-256 token is 64 hex characters");
		Ok(())
	}

	/// A form field survives the shapes a browser sends it in.
	#[test]
	fn test_a_form_field_is_read_02() -> Outcome<()> {
		assert_eq!(form_field(b"slug=on-rent", "slug"), Some(fmt!("on-rent")));
		assert_eq!(form_field(b"a=1&slug=on-rent&b=2", "slug"), Some(fmt!("on-rent")));
		assert_eq!(form_field(b"slug=a%20b", "slug"), Some(fmt!("a b")));
		assert_eq!(form_field(b"date=2026-07-17+14%3A30", "date"), Some(fmt!("2026-07-17 14:30")));
		assert_eq!(form_field(b"other=1", "slug"), None);
		Ok(())
	}

	/// The effective admin set is the config list unioned with the database one: config first, database
	/// names that do not repeat one added, and no id twice.
	#[test]
	fn test_the_effective_admins_are_the_union_03() -> Outcome<()> {
		let cfg = vec![fmt!("aaa"), fmt!("bbb")];
		let db = vec![fmt!("bbb"), fmt!("ccc")];
		// Config first, then the database's new one, and the shared id once.
		assert_eq!(union_admins(&cfg, &db), vec![fmt!("aaa"), fmt!("bbb"), fmt!("ccc")]);
		// Either set alone is itself.
		assert_eq!(union_admins(&cfg, &[]), cfg);
		assert_eq!(union_admins(&[], &db), db);
		// Two empty sets is empty -- which is the claimable state.
		assert!(union_admins(&[], &[]).is_empty());
		Ok(())
	}

	/// An id-hash is 64 lowercase hex characters, and nothing else passes the gate a grant goes through.
	#[test]
	fn test_an_id_hash_is_sixty_four_lower_hex_04() -> Outcome<()> {
		let good = "0123456789abcdef".repeat(4);
		assert_eq!(good.len(), 64);
		assert!(valid_id_hash(&good));
		// Too short, too long.
		assert!(!valid_id_hash(&"ab".repeat(31)));		// 62
		assert!(!valid_id_hash(&"ab".repeat(33)));		// 66
		// Uppercase is not lowercase.
		assert!(!valid_id_hash(&"AB".repeat(32)));
		// A non-hex character in an otherwise sound length.
		let mut bad = "a".repeat(63);
		bad.push('z');
		assert!(!valid_id_hash(&bad));
		// Empty is not a hash.
		assert!(!valid_id_hash(""));
		Ok(())
	}

	// ┌───────────────────────────────────────────────────────────────────────────┐
	// │ PASSPHRASE SIGN-IN                                                        │
	// └───────────────────────────────────────────────────────────────────────────┘

	use oxedyne_fe2o3_crypto::keystore::{
		Wallet,
		DEFAULT_WALLET_KDF_NAME,
	};
	use oxedyne_fe2o3_net::http::header::HttpHeadline;
	use secrecy::ExposeSecret;

	/// The database type the gate is instantiated over. The gate never touches it
	/// on the passphrase path -- the manage session is proven before any database
	/// is consulted -- so the tests pass `None` and only need the type named.
	type TestDb = oxedyne_fe2o3_o3db_sync::O3db<
		{ crate::srv::id::UID_LEN },
		crate::srv::id::Uid,
		oxedyne_fe2o3_crypto::enc::EncryptionScheme,
		oxedyne_fe2o3_hash::hash::HashScheme,
		oxedyne_fe2o3_hash::hash::HashScheme,
		oxedyne_fe2o3_hash::csum::ChecksumScheme,
	>;

	/// An unsealed admin state around a fresh wallet whose one admin is
	/// `name`/`pass`, holding the wildcard scope a first admin gets.
	fn mkstate(name: &str, pass: &[u8]) -> Outcome<AdminState> {
		let (wallet, unlocked) = res!(Wallet::create_with_first_admin(
			oxedyne_fe2o3_jdat::map::DaticleMap::new(),
			name,
			pass,
			DEFAULT_WALLET_KDF_NAME,
		));
		let master = unlocked.master_key.expose_secret().clone();
		AdminState::new(
			Arc::new(RwLock::new(wallet)),
			std::path::PathBuf::from("./wallet.jdat"),
			Some(master),
			1,
			None,
			crate::srv::admin::traffic::TrafficRecorder::new_shared(0),
			crate::srv::admin::host_sampler::HostSampler::new_shared(),
			res!(crate::srv::admin::guard::new_shared()),
			res!(crate::srv::admin::guard::new_shared()),
			Vec::new(),
			None,
		)
	}

	/// The loopback peer the login path is told the attempt came from.
	fn peer() -> SocketAddr {
		SocketAddr::from(([127, 0, 0, 1], 0))
	}

	/// Empty request headers -- no session of any kind.
	fn no_headers() -> Arc<HeaderFields> {
		Arc::new(HeaderFields::default())
	}

	/// Request headers carrying one cookie, `key=val`.
	fn cookie_headers(key: &str, val: &str) -> Arc<HeaderFields> {
		let mut h = HeaderFields::default();
		h.insert(
			HeaderName::Cookie,
			HeaderFieldValue::Cookie(vec![Cookie {
				key:	key.to_string(),
				val:	val.to_string(),
				attrs:	None,
			}]),
			None,
		);
		Arc::new(h)
	}

	/// Request headers a fetch caller sends: a manage cookie and `Accept: json`.
	fn json_headers(manage: &str) -> Arc<HeaderFields> {
		let mut h = HeaderFields::default();
		h.insert(
			HeaderName::Accept,
			HeaderFieldValue::Generic("application/json".to_string()),
			None,
		);
		if !manage.is_empty() {
			h.insert(
				HeaderName::Cookie,
				HeaderFieldValue::Cookie(vec![Cookie {
					key:	session::MANAGE_COOKIE_NAME.to_string(),
					val:	manage.to_string(),
					attrs:	None,
				}]),
				None,
			);
		}
		Arc::new(h)
	}

	/// The gate over the passphrase path, with the DB type named and no database.
	fn gate(state: Option<&AdminState>, headers: &Arc<HeaderFields>) -> Outcome<Option<SiteAdmin>> {
		site_admin::<
			{ crate::srv::id::UID_LEN },
			crate::srv::id::Uid,
			oxedyne_fe2o3_crypto::enc::EncryptionScheme,
			oxedyne_fe2o3_hash::hash::HashScheme,
			TestDb,
		>(&[], state, None, headers)
	}

	/// The response's status, if it is a response.
	fn status_of(resp: &HttpMessage) -> Option<HttpStatus> {
		match &resp.header.headline {
			HttpHeadline::Response { status }	=> Some(*status),
			_					=> None,
		}
	}

	/// The `Set-Cookie` the response carries, if any.
	fn set_cookie_of(resp: &HttpMessage) -> Option<Cookie> {
		match resp.header.fields.get_one(&HeaderName::SetCookie) {
			Some(HeaderFieldValue::SetCookie(c))	=> Some(c.clone()),
			_					=> None,
		}
	}

	/// A valid passphrase mints a manage session the gate then accepts as an
	/// admin, and the cookie it rides in is scoped and hardened for the console.
	#[test]
	fn test_a_good_passphrase_is_a_gate_admin_05() -> Outcome<()> {
		let state = res!(mkstate("jason", b"correct horse"));

		// The form sign-in: a browser posting `passphrase=...`.
		let resp = do_login(Some(&state), None, &no_headers(), b"passphrase=correct+horse", peer(), "t");
		assert_eq!(status_of(&resp), Some(HttpStatus::SeeOther), "a form login redirects on success");

		// The cookie it set: named for the console, scoped to reach it, hardened.
		let cookie = match set_cookie_of(&resp) {
			Some(c)	=> c,
			None	=> return Err(err!("the login set no cookie"; Invalid)),
		};
		assert_eq!(cookie.key, session::MANAGE_COOKIE_NAME);
		let attrs = match cookie.attrs.clone() {
			Some(a)	=> a,
			None	=> return Err(err!("the manage cookie carried no attributes"; Invalid)),
		};
		assert!(attrs.contains(&SetCookieAttributes::Path("/".to_string())),
			"the manage cookie must be Path=/ to reach /manage");
		assert!(!attrs.contains(&SetCookieAttributes::Path("/admin".to_string())),
			"the manage cookie must not be scoped to the operator dashboard");
		assert!(attrs.contains(&SetCookieAttributes::HttpOnly));
		assert!(attrs.contains(&SetCookieAttributes::Secure));
		assert!(attrs.contains(&SetCookieAttributes::SameSite(SameSite::Strict)));

		// A request bearing that cookie is a site admin at the gate.
		let headers = cookie_headers(session::MANAGE_COOKIE_NAME, &cookie.val);
		let admin = match res!(gate(Some(&state), &headers)) {
			Some(a)	=> a,
			None	=> return Err(err!("the gate refused a valid manage session"; Invalid)),
		};
		assert_eq!(admin.username, fmt!("jason"), "the session named the wrong admin");
		Ok(())
	}

	/// A wrong passphrase mints no session: the login sets no cookie, and a
	/// request with none is no admin.
	#[test]
	fn test_a_bad_passphrase_is_no_session_06() -> Outcome<()> {
		let state = res!(mkstate("jason", b"correct horse"));

		let resp = do_login(Some(&state), None, &no_headers(), b"passphrase=wrong", peer(), "t");
		assert_eq!(status_of(&resp), Some(HttpStatus::OK), "a failed form login re-renders the form");
		assert!(set_cookie_of(&resp).is_none(), "a failed login must set no session cookie");
		let form = resp.body_as_string();
		assert!(form.contains("name=\"passphrase\""), "the re-rendered page is the login form");

		// The verify itself refuses it.
		match res!(auth::verify_passphrase(&state, b"wrong", peer())) {
			LoginOutcome::BadCredentials	=> {}
			other				=> return Err(err!(
				"a wrong passphrase did not yield BadCredentials: {:?}", other; Invalid)),
		}

		// And no cookie means no admin.
		assert!(res!(gate(Some(&state), &no_headers())).is_none(), "an unsigned request was an admin");
		Ok(())
	}

	/// A forged or garbage manage cookie is refused by the gate, not read as a
	/// session.
	#[test]
	fn test_a_forged_manage_cookie_is_rejected_07() -> Outcome<()> {
		let state = res!(mkstate("jason", b"correct horse"));

		for junk in ["not-a-cookie", "m1.zzzz", "m1.", ""] {
			let headers = cookie_headers(session::MANAGE_COOKIE_NAME, junk);
			assert!(res!(gate(Some(&state), &headers)).is_none(),
				"the gate accepted a forged manage cookie '{}'", junk);
		}

		// A real session with a flipped byte no longer authenticates.
		let good = res!(session::encode(&state, "jason"));
		let mut bytes = good.into_bytes();
		let idx = bytes.len() - 4;
		bytes[idx] ^= 0x01;
		let tampered = String::from_utf8_lossy(&bytes).into_owned();
		let headers = cookie_headers(session::MANAGE_COOKIE_NAME, &tampered);
		assert!(res!(gate(Some(&state), &headers)).is_none(), "a tampered manage cookie passed the gate");
		Ok(())
	}

	/// The fetch shapes the front-end is built against: `{"ok":true}` and a
	/// cookie on success, `{"ok":false,...}` and no cookie on failure.
	#[test]
	fn test_the_json_login_shapes_08() -> Outcome<()> {
		let state = res!(mkstate("jason", b"correct horse"));

		let ok = do_login(Some(&state), None, &json_headers(""), b"passphrase=correct+horse", peer(), "t");
		assert_eq!(status_of(&ok), Some(HttpStatus::OK));
		assert!(ok.body_as_string().contains("\"ok\":true"), "a JSON success is {{\"ok\":true}}");
		assert!(set_cookie_of(&ok).is_some(), "a JSON success still sets the session cookie");

		let bad = do_login(Some(&state), None, &json_headers(""), b"passphrase=wrong", peer(), "t");
		let body = bad.body_as_string();
		assert!(body.contains("\"ok\":false"), "a JSON failure is {{\"ok\":false,...}}");
		assert!(body.contains("\"error\":"), "a JSON failure carries an error string");
		assert!(set_cookie_of(&bad).is_none(), "a JSON failure sets no cookie");
		Ok(())
	}

	/// `/manage/status` reports `admin:true` and hands out a CSRF token for a
	/// passphrase-authed session, so the console's write forms work.
	#[tokio::test]
	async fn test_status_admin_and_csrf_for_a_passphrase_session_09() -> Outcome<()> {
		let state = res!(mkstate("jason", b"correct horse"));
		let value = res!(session::encode(&state, "jason"));
		let headers = cookie_headers(session::MANAGE_COOKIE_NAME, &value);

		let resp = res!(handle_get::<
			{ crate::srv::id::UID_LEN },
			crate::srv::id::Uid,
			oxedyne_fe2o3_crypto::enc::EncryptionScheme,
			oxedyne_fe2o3_hash::hash::HashScheme,
			TestDb,
		>(&[], Some(&state), None, None, PATH_STATUS, "", &headers, "t").await);

		let body = resp.body_as_string();
		assert!(body.contains("\"admin\":true"), "status did not report the passphrase admin: {}", body);
		assert!(body.contains("\"csrf\":\""), "status gave the passphrase admin no csrf token: {}", body);
		Ok(())
	}

	/// An unauthenticated `GET /manage` is answered with the themed passphrase
	/// login form -- 200 with a password field, not a redirect.
	#[tokio::test]
	async fn test_unauthenticated_get_manage_is_a_login_form_10() -> Outcome<()> {
		let resp = res!(handle_get::<
			{ crate::srv::id::UID_LEN },
			crate::srv::id::Uid,
			oxedyne_fe2o3_crypto::enc::EncryptionScheme,
			oxedyne_fe2o3_hash::hash::HashScheme,
			TestDb,
		>(&[], None, None, None, PATH_ROOT, "", &no_headers(), "t").await);

		assert_eq!(status_of(&resp), Some(HttpStatus::OK), "an unauthenticated /manage must be 200, not a redirect");
		assert!(resp.header.fields.get_one(&HeaderName::Location).is_none(), "it must not be a redirect");
		let body = resp.body_as_string();
		assert!(body.contains("type=\"password\""), "the login form has a password field");
		assert!(body.contains("name=\"passphrase\""), "the field is named passphrase");
		assert!(body.contains(&fmt!("action=\"{}\"", PATH_LOGIN)), "the form posts to /manage/login");
		Ok(())
	}

	/// A manage credential cannot open the operator dashboard: presented under the
	/// operator cookie name, the operator gate refuses it, so the two sessions
	/// stay strictly separate and the operator path is untouched.
	#[test]
	fn test_a_manage_session_is_not_an_operator_session_11() -> Outcome<()> {
		let state = res!(mkstate("jason", b"correct horse"));
		let value = res!(session::encode(&state, "jason"));

		// The two cookies are distinct names, so neither is ever sent where the
		// other is read.
		assert_ne!(session::MANAGE_COOKIE_NAME,
			crate::srv::admin::session::SESSION_COOKIE_NAME,
			"the manage and operator cookies must not share a name");

		// Even smuggled under the operator cookie name, a manage blob does not
		// decode as an operator principal: the operator gate returns None.
		let headers = cookie_headers(crate::srv::admin::session::SESSION_COOKIE_NAME, &value);
		assert!(crate::srv::admin::handler::extract_principal(&state, &headers).is_none(),
			"a manage credential was accepted by the operator dashboard");

		// And the manage cookie is scoped to the site, never to /admin.
		let cookie = build_manage_cookie(value, false);
		let attrs = match cookie.attrs {
			Some(a)	=> a,
			None	=> return Err(err!("no attributes"; Invalid)),
		};
		assert!(attrs.contains(&SetCookieAttributes::Path("/".to_string())));
		assert!(!attrs.contains(&SetCookieAttributes::Path("/admin".to_string())));
		Ok(())
	}
}
