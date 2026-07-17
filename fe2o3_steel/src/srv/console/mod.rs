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

use crate::srv::{
	admin::assets::html_escape,
	publish::PublishConfig,
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
		HeaderFieldValue,
		HeaderFields,
		HeaderName,
	},
	msg::HttpMessage,
	status::HttpStatus,
};

use std::sync::{
	Arc,
	RwLock,
};


/// The console's root.
pub const PATH_ROOT: &str = "/manage";

/// Whether the signed-in member may reach the console, as JSON, for the site's own chrome to ask
/// before it offers a way in. A read, so it is a GET, and it answers for anyone -- signed in or not,
/// admin or not -- rather than turning a non-admin away, because a page asking "should I show the
/// door" is not itself the door.
pub const PATH_STATUS: &str = "/manage/status";


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

/// The member the request belongs to, if they are a site admin.
///
/// Authentication and then the operator's list: a signed-in member whose username is on it. The
/// authorisation, in one place, over [`member_username`]'s identity.
pub fn site_admin<
	const UIDL: usize,
	UID:	NumIdDat<UIDL>,
	ENC:	Encrypter,
	KH:	Hasher,
	DB:	Database<UIDL, UID, ENC, KH>,
>(
	site_admins:	&[String],
	db:		Option<&(Arc<RwLock<DB>>, UID)>,
	headers:	&Arc<HeaderFields>,
)
	-> Outcome<Option<SiteAdmin>>
{
	// A site with no admins has no console, and the question stops here rather than reaching for a
	// database it has no reason to.
	if site_admins.is_empty() {
		return Ok(None);
	}
	match res!(member_username(db, headers)) {
		Some(username) if site_admins.iter().any(|a| a == &username)
			=> Ok(Some(SiteAdmin { username })),
		_	=> Ok(None),
	}
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
	publish:	Option<&PublishConfig>,
	db:		Option<&(Arc<RwLock<DB>>, UID)>,
	request_path:	&str,
	query:		&str,
	headers:	&Arc<HeaderFields>,
	id:		&str,
)
	-> Outcome<HttpMessage>
{
	let admin = res!(site_admin(site_admins, db, headers));

	// The status probe answers everyone, so the site's chrome can ask whether to show the way in
	// without being redirected. It is the one console path a non-admin may read. An admin also gets
	// the CSRF token here, since the app that draws its own management surface needs it to write and
	// cannot read the session cookie to derive it.
	if request_path == PATH_STATUS {
		let csrf = match (&admin, headers.get_session_id()) {
			(Some(_), Some(sid))	=> Some(csrf_token(&sid)),
			_			=> None,
		};
		return Ok(status_json(admin.is_some(), csrf.as_deref()));
	}

	let admin = match admin {
		Some(a)	=> a,
		None	=> {
			// Not an admin. A signed-in member is one the operator has not listed: tell them their
			// id, so they can ask to be added, and where the front door is. This is the bootstrap
			// too -- the first admin reads their own id here and hands it to the operator. A visitor
			// who is not signed in has nothing to add and is sent home.
			return match res!(member_username(db, headers)) {
				Some(username)	=> Ok(not_yet_admin(&Theme::of(publish), &username)),
				None		=> Ok(redirect(&home_of(publish))),
			};
		}
	};

	// The token every form on the pages below carries, so the write it makes proves it came from a
	// page the session rendered. Derived from the session the cookie names; the same for every form
	// in a session.
	let csrf = match headers.get_session_id() {
		Some(s)	=> csrf_token(&s),
		None	=> return Ok(redirect(&home_of(publish))),
	};

	let theme = Theme::of(publish);
	publish::handle_get(publish, &theme, &admin, &csrf, db, request_path, query, id)
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
	publish:	Option<&PublishConfig>,
	db:		Option<&(Arc<RwLock<DB>>, UID)>,
	request_path:	&str,
	headers:	&Arc<HeaderFields>,
	body:		&[u8],
	id:		&str,
)
	-> Outcome<Option<HttpMessage>>
{
	if !publish::posts(request_path) {
		return Ok(None);
	}

	let admin = match res!(site_admin(site_admins, db, headers)) {
		Some(a)	=> a,
		None	=> {
			warn!("{}: console: a caller who is not a site admin tried to write", id);
			return Ok(Some(redirect(&home_of(publish))));
		}
	};

	// The cross-site guard. The member cookie is `SameSite=Lax`, so a cross-site POST does not carry
	// it and never reaches an authenticated state at all -- this is the belt to that braces, and the
	// thing that still holds if the cookie's policy is ever loosened. The token is a value only a
	// page that held the session could have been given, checked against the session the cookie names.
	let sid = match headers.get_session_id() {
		Some(s)	=> s,
		None	=> return Ok(Some(redirect(&home_of(publish)))),
	};
	// Whether the caller is the site's own front-end asking over fetch, which wants a plain JSON
	// answer, or a browser posting a form, which wants a redirect. The app says so with its Accept.
	let json = wants_json(headers);

	let sent = form_field(body, "csrf").unwrap_or_default();
	if !csrf_ok(&sid, &sent) {
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

	let resp = res!(publish::handle_post(publish, &admin, db, request_path, body, json, id));
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
	s.push_str(&fmt!("<a href=\"{}\">View site</a>", html_escape(&theme.home)));
	s.push_str(&fmt!("<span class=\"mc-who\">{}…</span>", html_escape(&admin.username[..8.min(admin.username.len())])));
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
/// Their own id, and what to do with it. It is the bootstrap made visible: the first admin is a
/// member the operator has not yet listed, and this is where they read the id to hand over. A page,
/// not a redirect, because there is something here for them to read and act on -- which a member sent
/// silently home would never find.
fn not_yet_admin(theme: &Theme, username: &str) -> HttpMessage {
	let body = fmt!(
		"<h1>Not your site to manage — yet</h1>\n\
		<p class=\"mc-muted\">You are signed in, but you are not one of this site's administrators. \
		If you should be, give the operator this id and ask to be added to the site's \
		<code>site_admins</code>:</p>\n\
		<p class=\"mc-notice\"><code>{id}</code></p>\n\
		<p class=\"mc-muted\">It is not a secret; it is the public name of your account, and knowing \
		it does not let anyone sign in as you. <a href=\"{home}\">Back to the site.</a></p>\n",
		id	= html_escape(username),
		home	= html_escape(&theme.home),
	);
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
.mc-body{margin:0;background:var(--body-bg,#14181d);color:var(--body-color,#e6e6e6);\
font-family:var(--font-ui,var(--font-body,system-ui,sans-serif));line-height:1.5;}\
.mc-head{border-bottom:1px solid var(--aside-rule-color,#333c47);}\
.mc-head-in{max-width:52rem;margin:0 auto;padding:0.9rem 1.2rem;display:flex;\
align-items:baseline;justify-content:space-between;gap:1rem;flex-wrap:wrap;}\
.mc-brand{font-weight:600;font-size:1.05rem;}\
.mc-brand-sub{color:var(--aside-date-color,#8a97a6);font-weight:400;font-size:0.8rem;\
text-transform:uppercase;letter-spacing:0.08em;}\
.mc-nav{display:flex;align-items:center;gap:1.1rem;font-size:0.9rem;}\
.mc-nav a{color:var(--aside-link-color,#7fb0e0);text-decoration:none;}\
.mc-nav a:hover{text-decoration:underline;}\
.mc-who{color:var(--aside-date-color,#8a97a6);font-family:var(--font-mono,monospace);font-size:0.8rem;}\
.mc-main{max-width:52rem;margin:0 auto;padding:1.4rem 1.2rem 4rem;}\
.mc-main h1{font-size:1.5rem;margin:0 0 0.3rem;}\
.mc-muted{color:var(--aside-date-color,#8a97a6);font-size:0.9rem;margin:0 0 1.4rem;}\
.mc-notice{border:1px solid var(--aside-rule-color,#333c47);border-radius:6px;\
padding:0.8rem 1rem;margin:0 0 1.2rem;}\
.mc-notice code{font-family:var(--font-mono,monospace);font-size:0.85em;}\
.mc-btn,button.mc-btn{display:inline-block;font:inherit;font-size:0.9rem;cursor:pointer;\
padding:0.5rem 0.9rem;border-radius:6px;border:1px solid var(--aside-link-color,#7fb0e0);\
background:var(--aside-link-color,#7fb0e0);color:var(--body-bg,#14181d);text-decoration:none;}\
.mc-btn:hover{opacity:0.9;text-decoration:none;}\
.mc-btn-quiet{background:transparent;color:var(--aside-link-color,#7fb0e0);}\
.mc-btn-danger{background:transparent;border-color:#c0554e;color:#d9776f;}\
table.mc-table{width:100%;border-collapse:collapse;margin:0.4rem 0 1.6rem;font-size:0.92rem;}\
.mc-table th{text-align:left;font-size:0.75rem;text-transform:uppercase;letter-spacing:0.06em;\
color:var(--aside-date-color,#8a97a6);border-bottom:1px solid var(--aside-rule-color,#333c47);\
padding:0.4rem 0.6rem;}\
.mc-table td{border-bottom:1px solid var(--aside-rule-color,#333c47);padding:0.55rem 0.6rem;\
vertical-align:top;}\
.mc-table a{color:var(--aside-link-color,#7fb0e0);text-decoration:none;}\
.mc-table a:hover{text-decoration:underline;}\
.mc-slug{color:var(--aside-date-color,#8a97a6);font-family:var(--font-mono,monospace);font-size:0.8rem;}\
.mc-tag{display:inline-block;font-size:0.72rem;text-transform:uppercase;letter-spacing:0.05em;\
padding:0.1rem 0.45rem;border-radius:4px;border:1px solid var(--aside-rule-color,#333c47);\
color:var(--aside-date-color,#8a97a6);}\
.mc-tag-live{border-color:#4f8f57;color:#7bc084;}\
.mc-tag-err{border-color:#c0554e;color:#d9776f;}\
.mc-form label{display:block;font-size:0.8rem;text-transform:uppercase;letter-spacing:0.05em;\
color:var(--aside-date-color,#8a97a6);margin:1rem 0 0.3rem;}\
.mc-form input[type=text],.mc-form select,.mc-form textarea{width:100%;box-sizing:border-box;\
font:inherit;background:var(--input-bg,#0e1216);color:var(--input-color,inherit);\
border:1px solid var(--aside-rule-color,#333c47);border-radius:6px;padding:0.5rem 0.6rem;}\
.mc-form textarea{min-height:22rem;font-family:var(--font-mono,monospace);font-size:0.9rem;\
line-height:1.5;resize:vertical;}\
.mc-row{display:flex;gap:1rem;flex-wrap:wrap;}\
.mc-row>div{flex:1 1 8rem;}\
.mc-actions{margin-top:1.2rem;display:flex;gap:0.7rem;align-items:center;flex-wrap:wrap;}\
.mc-prose{border:1px solid var(--aside-rule-color,#333c47);border-radius:6px;padding:1.2rem 1.4rem;\
margin-top:0.8rem;}\
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
fn status_json(admin: bool, csrf: Option<&str>) -> HttpMessage {
	let body = match csrf {
		Some(t)	=> fmt!("{{\"admin\":{},\"csrf\":\"{}\"}}", admin, t),
		None	=> fmt!("{{\"admin\":{}}}", admin),
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
}
