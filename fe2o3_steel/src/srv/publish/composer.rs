//! Writing the prose a site publishes.
//!
//! # Where this lives, and why it is not under the prose's own prefix
//!
//! Under `/admin`, with the rest of the dashboard, because that is the only place the dashboard's
//! session can be presented. The cookie `/admin/login` issues is `Path=/admin` and
//! `SameSite=Strict`, so a browser sends it to `/admin` and nowhere else. A composer at
//! `/asides/admin` would be handed no cookie at all, and would refuse its own author every time.
//!
//! This is not hypothetical. The import route was first built at `{path}/import`, gated on the
//! dashboard's session, and could never have authorised anyone: the gate was right and the cookie
//! was never going to arrive. It answered 404 to its author exactly as it would to a stranger, which
//! is what a correct gate looks like from the outside and why nothing about it looked wrong.
//!
//! # Who
//!
//! The dashboard's session, the same one `/admin` issues. This module runs inside the server, beside
//! the wallet and the admin list, so the identity that unsealed the server is already here and is
//! the right one: publishing is something the operator of a site does, and on a personal site the
//! operator is the author.
//!
//! # What a form may say
//!
//! A slug and a date arrive from a browser, which means they arrive from anywhere. Both are checked
//! ([`valid_slug`], [`valid_date`]) before either reaches a key: a slug is pasted into
//! `publish/post/<slug>` and into a URL, and a form's word for one is not a reason to trust it.
//!
//! The prose is not checked, because there is nothing to check it against -- it is Markdown, and
//! Markdown that will not parse is refused by the parser at the point it is rendered. It is escaped
//! on the way back out, like everything else here.
//!
//! # Why there is no title field
//!
//! A post's title is its own most prominent heading, as it is for a post read from a directory. A
//! field would be a second place to say it and therefore a second place for it to be wrong.

use crate::srv::{
	admin::{
		assets::{
			html_escape,
			render_layout,
		},
		handler::{
			extract_form_field,
			extract_principal,
			redirect_to_login,
		},
		state::AdminState,
	},
	publish::{
		PostKind,
		PostState,
		PublishConfig,
		Source,
		store::{
			self,
			Record,
		},
		valid_date,
		valid_slug,
	},
};

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_iop_crypto::enc::Encrypter;
use oxedyne_fe2o3_iop_db::api::Database;
use oxedyne_fe2o3_iop_hash::api::Hasher;
use oxedyne_fe2o3_jdat::id::NumIdDat;
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


/// The composer's root.
pub const PATH_ROOT: &str = "/admin/publish";

/// The editor, for one post or for a new one.
pub const PATH_EDIT: &str = "/admin/publish/edit";

/// A draft as a reader would get it, if it were not a draft.
pub const PATH_PREVIEW: &str = "/admin/publish/preview";

/// Where the editor posts.
pub const PATH_SAVE: &str = "/admin/publish/save";

/// Where a deletion posts.
pub const PATH_DELETE: &str = "/admin/publish/delete";

/// Where an import of the directory posts.
pub const PATH_IMPORT: &str = "/admin/publish/import";


/// Whether a path belongs to the composer.
pub fn owns(path: &str) -> bool {
	path == PATH_ROOT
		|| (path.starts_with(PATH_ROOT)
			&& path.as_bytes().get(PATH_ROOT.len()) == Some(&b'/'))
}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ GET                                                                       │
// └───────────────────────────────────────────────────────────────────────────┘

/// Serves the composer's pages.
pub async fn handle_get<
	const UIDL: usize,
	UID:	NumIdDat<UIDL>,
	ENC:	Encrypter,
	KH:	Hasher,
	DB:	Database<UIDL, UID, ENC, KH>,
>(
	cfg:		Option<&PublishConfig>,
	state:		&AdminState,
	db:		Option<&(Arc<RwLock<DB>>, UID)>,
	request_path:	&str,
	query:		&str,
	headers:	&Arc<HeaderFields>,
	id:		&str,
)
	-> Outcome<HttpMessage>
{
	debug!("{}: publish: composer GET {}", id, request_path);

	// The gate first, before the page says anything about what this vhost publishes.
	let principal = match extract_principal(state, headers) {
		Some(p)	=> p,
		None	=> return Ok(redirect_to_login()),
	};

	// The path and the query arrive apart, so nothing here splits one out of the other. The caller
	// hands over `HttpLocator::query`, which is the raw substring with no leading `?`.
	let path = request_path;

	// A vhost that publishes nothing has no composer, and says so rather than offering an editor
	// that would have nowhere to write.
	let cfg = match cfg {
		Some(c)	=> c,
		None	=> return Ok(page(&principal, path, "Publish", &notice(
			"This vhost publishes nothing. Give it a <code>publish</code> block to change that.",
		))),
	};

	match path {
		PATH_ROOT	=> handle_list(cfg, &principal, db, id),
		PATH_EDIT	=> handle_edit(cfg, &principal, db, query, id),
		PATH_PREVIEW	=> handle_preview(cfg, &principal, db, query, id),
		_		=> Ok(HttpMessage::respond_with_text(
			HttpStatus::NotFound,
			"Not found.",
		)),
	}
}

/// The list of posts, drafts and all.
fn handle_list<
	const UIDL: usize,
	UID:	NumIdDat<UIDL>,
	ENC:	Encrypter,
	KH:	Hasher,
	DB:	Database<UIDL, UID, ENC, KH>,
>(
	cfg:		&PublishConfig,
	principal:	&AdminPrincipalRef,
	db:		Option<&(Arc<RwLock<DB>>, UID)>,
	id:		&str,
)
	-> Outcome<HttpMessage>
{
	let mut body = String::new();

	body.push_str(&fmt!(
		"<h1>{title}</h1>\n<p class=\"muted\">Served at <a href=\"{path}\">{path}</a>.</p>\n",
		title	= html_escape(&cfg.title),
		path	= html_escape(&cfg.path),
	));

	// A directory-backed vhost has no composer, and the reason is worth stating rather than leaving
	// an author to wonder why the editor will not save: the files are the posts, so an editor here
	// would be writing into a store nothing serves.
	if cfg.source != Source::Store {
		body.push_str(&notice(&fmt!(
			"This site serves its posts from the directory <code>{dir}</code>, so they are edited \
			by editing those files. To write them here instead, set <code>source</code> to \
			<code>\"store\"</code> in this vhost's <code>publish</code> block, then import what the \
			directory holds.",
			dir = html_escape(&cfg.dir),
		)));
		body.push_str(&import_form(&cfg.dir));
		return Ok(page(principal, PATH_ROOT, "Publish", &body));
	}

	let db = match db {
		Some(db)	=> db,
		None		=> {
			body.push_str(&notice(
				"This vhost keeps its posts in its database, and has no database configured. Set \
				<code>db_dir_rel</code> on the vhost.",
			));
			return Ok(page(principal, PATH_ROOT, "Publish", &body));
		}
	};

	let recs = match store::list_records(db, id) {
		Ok(r)	=> r,
		Err(e)	=> {
			error!(e, "{}: publish: cannot list the posts", id);
			body.push_str(&notice("The posts could not be listed. The log says why."));
			return Ok(page(principal, PATH_ROOT, "Publish", &body));
		}
	};

	body.push_str(&fmt!(
		"<p><a class=\"steel-button\" href=\"{edit}\">Write a new post</a></p>\n",
		edit = PATH_EDIT,
	));

	if recs.is_empty() {
		body.push_str(&notice("Nothing written yet."));
		body.push_str(&import_form(&cfg.dir));
		return Ok(page(principal, PATH_ROOT, "Publish", &body));
	}

	body.push_str("<table class=\"steel-table\">\n<thead><tr>\
		<th>Post</th><th>Kind</th><th>State</th><th>Date</th><th></th>\
		</tr></thead>\n<tbody>\n");
	for rec in &recs {
		let slug = html_escape(&rec.slug);
		// The title is the prose's own heading, so it costs a parse to know. A list of ten posts is
		// ten parses; a stored title would be a second place for it to be wrong. Where the prose
		// will not parse the slug stands in, and the state cell says the post is broken.
		let (title, broken) = match rec.render() {
			Ok(p)	=> (html_escape(&p.title), false),
			Err(e)	=> {
				warn!("{}: publish: '{}' will not render: {}", id, rec.slug, e);
				(html_escape(&rec.slug), true)
			}
		};
		let state = if broken {
			fmt!("<span class=\"tag tag-err\">will not render</span>")
		} else {
			match rec.state {
				PostState::Live		=> fmt!("<span class=\"tag tag-ok\">live</span>"),
				PostState::Draft	=> fmt!("<span class=\"tag\">draft</span>"),
			}
		};
		body.push_str(&fmt!(
			"<tr>\
			<td><a href=\"{edit}?slug={slug}\">{title}</a><br><span class=\"muted\">{slug}</span></td>\
			<td>{kind}</td>\
			<td>{state}</td>\
			<td>{date}</td>\
			<td><a href=\"{preview}?slug={slug}\">Preview</a></td>\
			</tr>\n",
			edit	= PATH_EDIT,
			preview	= PATH_PREVIEW,
			slug	= slug,
			title	= title,
			kind	= rec.kind.as_str(),
			state	= state,
			date	= html_escape(rec.date.as_deref().unwrap_or("--")),
		));
	}
	body.push_str("</tbody>\n</table>\n");
	body.push_str(&import_form(&cfg.dir));

	Ok(page(principal, PATH_ROOT, "Publish", &body))
}

/// The editor, for a post that exists or one that does not yet.
fn handle_edit<
	const UIDL: usize,
	UID:	NumIdDat<UIDL>,
	ENC:	Encrypter,
	KH:	Hasher,
	DB:	Database<UIDL, UID, ENC, KH>,
>(
	cfg:		&PublishConfig,
	principal:	&AdminPrincipalRef,
	db:		Option<&(Arc<RwLock<DB>>, UID)>,
	query:		&str,
	id:		&str,
)
	-> Outcome<HttpMessage>
{
	if cfg.source != Source::Store {
		return Ok(page(principal, PATH_ROOT, "Publish", &notice(
			"This site serves its posts from a directory, so there is nothing here to edit them \
			with.",
		)));
	}

	let slug = query_field(query, "slug");

	// No slug is a new post, which is the editor with nothing in it.
	let rec = match &slug {
		None		=> None,
		Some(slug)	=> {
			let db = match db {
				Some(db)	=> db,
				None		=> return Ok(page(principal, PATH_ROOT, "Publish", &notice(
					"This vhost has no database configured.",
				))),
			};
			match store::get(db, slug) {
				Ok(Some(r))	=> Some(r),
				Ok(None)	=> return Ok(page(principal, PATH_ROOT, "Publish", &notice(
					"There is no post by that name.",
				))),
				Err(e)		=> {
					error!(e, "{}: publish: cannot read '{}'", id, slug);
					return Ok(page(principal, PATH_ROOT, "Publish", &notice(
						"That post could not be read. The log says why.",
					)));
				}
			}
		}
	};

	let (heading, existing) = match &rec {
		Some(_)	=> ("Edit a post", true),
		None	=> ("Write a new post", false),
	};
	let r = rec.unwrap_or_default();

	let mut body = fmt!("<h1>{}</h1>\n", heading);

	body.push_str(&fmt!(
		"<form class=\"steel-form\" method=\"POST\" action=\"{save}\">\n\
		<input type=\"hidden\" name=\"was\" value=\"{was}\">\n\
		<div class=\"row\">\n\
			<div>\n\
				<label for=\"slug\">Name in the URL</label>\n\
				<input type=\"text\" id=\"slug\" name=\"slug\" value=\"{slug}\" \
					placeholder=\"on-rent\" required>\n\
			</div>\n\
			<div>\n\
				<label for=\"date\">Date</label>\n\
				<input type=\"text\" id=\"date\" name=\"date\" value=\"{date}\" \
					placeholder=\"2026-07-17\">\n\
			</div>\n\
			<div>\n\
				<label for=\"kind\">Kind</label>\n\
				<select id=\"kind\" name=\"kind\">\n\
					<option value=\"note\"{note_sel}>Note</option>\n\
					<option value=\"essay\"{essay_sel}>Essay</option>\n\
				</select>\n\
			</div>\n\
			<div>\n\
				<label for=\"state\">State</label>\n\
				<select id=\"state\" name=\"state\">\n\
					<option value=\"draft\"{draft_sel}>Draft</option>\n\
					<option value=\"live\"{live_sel}>Live</option>\n\
				</select>\n\
			</div>\n\
		</div>\n\
		<label for=\"source\">Markdown</label>\n\
		<textarea id=\"source\" name=\"source\" rows=\"24\" spellcheck=\"true\" \
			placeholder=\"# The title goes here, as the first heading\">{source}</textarea>\n\
		<p class=\"muted\">The title is the post's own most prominent heading. There is no title \
			field because there is no second place to say it.</p>\n\
		<button type=\"submit\">Save</button>\n\
		<a class=\"steel-button steel-button-quiet\" href=\"{root}\">Cancel</a>\n\
		</form>\n",
		save		= PATH_SAVE,
		root		= PATH_ROOT,
		// What the post was called on the way in, so a renamed slug can take the old record with it
		// rather than leave a copy behind.
		was		= html_escape(&r.slug),
		slug		= html_escape(&r.slug),
		date		= html_escape(r.date.as_deref().unwrap_or("")),
		note_sel	= selected(r.kind == PostKind::Note),
		essay_sel	= selected(r.kind == PostKind::Essay),
		draft_sel	= selected(r.state == PostState::Draft),
		live_sel	= selected(r.state == PostState::Live),
		source		= html_escape(&r.source),
	));

	if existing {
		body.push_str(&fmt!(
			"<form class=\"steel-form\" method=\"POST\" action=\"{del}\" \
			onsubmit=\"return confirm('Delete this post? There is no undo.')\">\n\
			<input type=\"hidden\" name=\"slug\" value=\"{slug}\">\n\
			<button type=\"submit\" class=\"danger\">Delete</button>\n\
			</form>\n",
			del	= PATH_DELETE,
			slug	= html_escape(&r.slug),
		));
	}

	Ok(page(principal, PATH_ROOT, "Publish", &body))
}

/// A post as a reader would get it, whether or not a reader can.
///
/// The point of it: a draft is served to nobody, so its author cannot see it by visiting it. Here
/// the same rendering runs behind the gate.
fn handle_preview<
	const UIDL: usize,
	UID:	NumIdDat<UIDL>,
	ENC:	Encrypter,
	KH:	Hasher,
	DB:	Database<UIDL, UID, ENC, KH>,
>(
	cfg:		&PublishConfig,
	principal:	&AdminPrincipalRef,
	db:		Option<&(Arc<RwLock<DB>>, UID)>,
	query:		&str,
	id:		&str,
)
	-> Outcome<HttpMessage>
{
	let slug = match query_field(query, "slug") {
		Some(s)	=> s,
		None	=> return Ok(page(principal, PATH_ROOT, "Publish", &notice(
			"No post was named.",
		))),
	};

	if cfg.source != Source::Store {
		return Ok(page(principal, PATH_ROOT, "Publish", &notice(
			"This site serves its posts from a directory, so every post it has is already \
			readable.",
		)));
	}

	let db = match db {
		Some(db)	=> db,
		None		=> return Ok(page(principal, PATH_ROOT, "Publish", &notice(
			"This vhost has no database configured.",
		))),
	};

	let rec = match store::get(db, &slug) {
		Ok(Some(r))	=> r,
		Ok(None)	=> return Ok(page(principal, PATH_ROOT, "Publish", &notice(
			"There is no post by that name.",
		))),
		Err(e)		=> {
			error!(e, "{}: publish: cannot read '{}'", id, slug);
			return Ok(page(principal, PATH_ROOT, "Publish", &notice(
				"That post could not be read. The log says why.",
			)));
		}
	};

	// The prose is the author's own, and it is rendered by the same renderer that serves it. It is
	// not escaped, because rendered Markdown is HTML and escaping it would show the reader the tags.
	// Nothing else on this page comes from the record without escaping.
	let post = match rec.render() {
		Ok(p)	=> p,
		Err(e)	=> {
			warn!("{}: publish: '{}' will not render: {}", id, slug, e);
			return Ok(page(principal, PATH_ROOT, "Publish", &notice(
				"That post will not render as Markdown. The log says where it goes wrong.",
			)));
		}
	};

	let body = fmt!(
		"<p class=\"muted\"><a href=\"{edit}?slug={slug}\">&larr; Back to the editor</a> \
		&middot; {state} &middot; this is the page a reader gets.</p>\n\
		<article class=\"steel-prose\">{html}</article>\n",
		edit	= PATH_EDIT,
		slug	= html_escape(&slug),
		state	= match rec.state {
			PostState::Live		=> "live",
			PostState::Draft	=> "a draft, so served to nobody",
		},
		html	= post.html,
	);

	Ok(page(principal, PATH_ROOT, "Preview", &body))
}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ POST                                                                      │
// └───────────────────────────────────────────────────────────────────────────┘

/// Serves the composer's writes.
///
/// Returns `None` for a path this module does not write to, so the caller can carry on down its own
/// routing rather than have every unrecognised POST under `/admin` become an error here.
pub async fn handle_post<
	const UIDL: usize,
	UID:	NumIdDat<UIDL>,
	ENC:	Encrypter,
	KH:	Hasher,
	DB:	Database<UIDL, UID, ENC, KH>,
>(
	cfg:		Option<&PublishConfig>,
	state:		&AdminState,
	db:		Option<&(Arc<RwLock<DB>>, UID)>,
	request_path:	&str,
	headers:	&Arc<HeaderFields>,
	body:		&[u8],
	id:		&str,
)
	-> Outcome<Option<HttpMessage>>
{
	if request_path != PATH_SAVE && request_path != PATH_DELETE && request_path != PATH_IMPORT {
		return Ok(None);
	}

	// The gate, before the body is read and before the response says whether the route does anything
	// at all. An unauthenticated caller learns nothing here it did not already know.
	let principal = match extract_principal(state, headers) {
		Some(p)	=> p,
		None	=> {
			warn!("{}: publish: an unauthenticated caller tried to write", id);
			return Ok(Some(HttpMessage::respond_with_text(
				HttpStatus::NotFound,
				"Not found.",
			)));
		}
	};

	let cfg = match cfg {
		Some(c)	=> c,
		None	=> return Ok(Some(back_with("this vhost publishes nothing"))),
	};
	if cfg.source != Source::Store {
		return Ok(Some(back_with(
			"this vhost serves its posts from a directory, so there is nothing to write into; set \
			'source' to 'store' first",
		)));
	}
	let db = match db {
		Some(db)	=> db,
		None		=> return Ok(Some(back_with("this vhost has no database configured"))),
	};

	let resp = match request_path {
		PATH_SAVE	=> res!(do_save(db, body, &principal.name, id)),
		PATH_DELETE	=> res!(do_delete(db, body, &principal.name, id)),
		PATH_IMPORT	=> res!(do_import(cfg, db, &principal.name, id)),
		// Unreachable: the guard above names the same three paths.
		_		=> return Ok(None),
	};
	Ok(Some(resp))
}

/// Writes a post.
fn do_save<
	const UIDL: usize,
	UID:	NumIdDat<UIDL>,
	ENC:	Encrypter,
	KH:	Hasher,
	DB:	Database<UIDL, UID, ENC, KH>,
>(
	db:	&(Arc<RwLock<DB>>, UID),
	body:	&[u8],
	who:	&str,
	id:	&str,
)
	-> Outcome<HttpMessage>
{
	let slug = extract_form_field(body, "slug").unwrap_or_default();
	let slug = slug.trim().to_string();
	if !valid_slug(&slug) {
		return Ok(back_with(
			"a post's name may hold letters, digits, hyphens and underscores, and nothing else",
		));
	}

	let date = extract_form_field(body, "date").unwrap_or_default();
	let date = date.trim().to_string();
	if !valid_date(&date) {
		return Ok(back_with("a date is written 2026-07-17, or is left empty"));
	}

	let source = extract_form_field(body, "source").unwrap_or_default();
	if source.trim().is_empty() {
		return Ok(back_with("a post with no prose in it is not a post"));
	}

	let rec = Record {
		slug:	slug.clone(),
		kind:	PostKind::of(&extract_form_field(body, "kind").unwrap_or_default()),
		state:	PostState::of(&extract_form_field(body, "state").unwrap_or_default()),
		date:	if date.is_empty() { None } else { Some(date) },
		source,
	};

	res!(store::put(db, &rec, id));

	// A renamed post is a new key, and the old one would otherwise stay behind: served, indexed, and
	// a second copy of prose the author believes they moved. The old name is what the editor was
	// opened with, not what the form now says, which is why the form carries both.
	if let Some(was) = extract_form_field(body, "was") {
		let was = was.trim();
		if !was.is_empty() && was != slug && valid_slug(was) {
			match store::delete(db, was, id) {
				Ok(_)	=> info!("{}: publish: '{}' renamed '{}' to '{}'", id, who, was, slug),
				// The new post is written; the old one is still there. Say so rather than fail
				// the save, which would lose the edit as well.
				Err(e)	=> warn!(
					"{}: publish: '{}' was renamed to '{}' and the old one would not delete: {}",
					id, was, slug, e),
			}
		}
	}

	info!("{}: publish: '{}' saved '{}' ({})", id, who, rec.slug, rec.state.as_str());
	Ok(back())
}

/// Deletes a post.
fn do_delete<
	const UIDL: usize,
	UID:	NumIdDat<UIDL>,
	ENC:	Encrypter,
	KH:	Hasher,
	DB:	Database<UIDL, UID, ENC, KH>,
>(
	db:	&(Arc<RwLock<DB>>, UID),
	body:	&[u8],
	who:	&str,
	id:	&str,
)
	-> Outcome<HttpMessage>
{
	let slug = match extract_form_field(body, "slug") {
		Some(s)	=> s,
		None	=> return Ok(back_with("no post was named")),
	};
	if !valid_slug(&slug) {
		return Ok(back_with("that is not a post's name"));
	}
	let existed = res!(store::delete(db, &slug, id));
	if existed {
		info!("{}: publish: '{}' deleted '{}'", id, who, slug);
	}
	Ok(back())
}

/// Reads the directory into the store.
fn do_import<
	const UIDL: usize,
	UID:	NumIdDat<UIDL>,
	ENC:	Encrypter,
	KH:	Hasher,
	DB:	Database<UIDL, UID, ENC, KH>,
>(
	cfg:	&PublishConfig,
	db:	&(Arc<RwLock<DB>>, UID),
	who:	&str,
	id:	&str,
)
	-> Outcome<HttpMessage>
{
	let n = match store::import_dir(db, &cfg.dir, id) {
		Ok(n)	=> n,
		Err(e)	=> {
			error!(e, "{}: publish: import from '{}' failed", id, cfg.dir);
			return Ok(back_with("the directory could not be read; the log says why"));
		}
	};
	info!("{}: publish: '{}' imported {} posts from '{}'", id, who, n, cfg.dir);
	Ok(back())
}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ HELPERS                                                                   │
// └───────────────────────────────────────────────────────────────────────────┘

/// The dashboard's principal, named once so the renderers below do not each repeat the path.
type AdminPrincipalRef = crate::srv::admin::AdminPrincipal;

/// A composer page in the dashboard's chrome.
fn page(
	principal:	&AdminPrincipalRef,
	current:	&str,
	title:		&str,
	body:		&str,
)
	-> HttpMessage
{
	let html = render_layout(title, current, principal, body, "");
	HttpMessage::new_response(HttpStatus::OK)
		.with_field(
			HeaderName::ContentType,
			HeaderFieldValue::Generic("text/html; charset=utf-8".to_string()),
		)
		.with_body(html.into_bytes())
}

/// Back to the list, having done the thing.
///
/// A redirect rather than a rendered page, so a reload does not offer to write the post again.
fn back() -> HttpMessage {
	HttpMessage::new_response(HttpStatus::SeeOther)
		.with_field(
			HeaderName::Location,
			HeaderFieldValue::Generic(PATH_ROOT.to_string()),
		)
}

/// Back to the list, having not.
///
/// The reason rides in the query string, which is where a redirect can carry one.
fn back_with(why: &str) -> HttpMessage {
	HttpMessage::new_response(HttpStatus::SeeOther)
		.with_field(
			HeaderName::Location,
			HeaderFieldValue::Generic(fmt!("{}?said={}", PATH_ROOT, url_encode(why))),
		)
}

/// A thing the page says.
fn notice(html: &str) -> String {
	fmt!("<p class=\"notice\">{}</p>\n", html)
}

/// The button that reads the directory in.
fn import_form(dir: &str) -> String {
	fmt!(
		"<form class=\"steel-form\" method=\"POST\" action=\"{import}\">\n\
		<p class=\"muted\">Import the Markdown in <code>{dir}</code>. A post the store already holds \
		is overwritten, so importing twice is importing once.</p>\n\
		<button type=\"submit\">Import the directory</button>\n\
		</form>\n",
		import	= PATH_IMPORT,
		dir	= html_escape(dir),
	)
}

/// `selected`, where it is.
fn selected(yes: bool) -> &'static str {
	if yes { " selected" } else { "" }
}

/// One field out of a raw query substring, which has no leading `?`.
fn query_field(query: &str, key: &str) -> Option<String> {
	if query.is_empty() {
		return None;
	}
	for pair in query.split('&') {
		let mut kv = pair.splitn(2, '=');
		let k = kv.next()?;
		let v = kv.next().unwrap_or("");
		if k == key {
			let val = url_decode(v);
			if val.is_empty() {
				return None;
			}
			return Some(val);
		}
	}
	None
}

/// Percent-encode a string for a query parameter, per RFC 3986 section 2.3.
fn url_encode(s: &str) -> String {
	let mut out = String::with_capacity(s.len());
	for b in s.as_bytes().iter() {
		match *b {
			b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9'
			| b'-' | b'_' | b'.' | b'~'	=> out.push(*b as char),
			other				=> out.push_str(&fmt!("%{:02X}", other)),
		}
	}
	out
}

/// Decode a percent-encoded value. `+` is a space, `%XX` is a byte, and an escape that is not one
/// passes through as what it was.
fn url_decode(s: &str) -> String {
	let bytes = s.as_bytes();
	let mut out = Vec::with_capacity(bytes.len());
	let mut i = 0;
	while i < bytes.len() {
		match bytes[i] {
			b'+' => {
				out.push(b' ');
				i += 1;
			}
			b'%' if i + 2 < bytes.len() => {
				match (hex_nibble(bytes[i + 1]), hex_nibble(bytes[i + 2])) {
					(Some(hi), Some(lo)) => {
						out.push((hi << 4) | lo);
						i += 3;
					}
					_ => {
						out.push(bytes[i]);
						i += 1;
					}
				}
			}
			b => {
				out.push(b);
				i += 1;
			}
		}
	}
	String::from_utf8_lossy(&out).into_owned()
}

/// One hex digit's value.
fn hex_nibble(b: u8) -> Option<u8> {
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

	use crate::srv::publish::SLUG_MAX;

	/// The composer answers for its own prefix and for nothing that merely starts like it.
	#[test]
	fn test_the_composer_owns_its_prefix_00() -> Outcome<()> {
		assert!(owns("/admin/publish"));
		assert!(owns("/admin/publish/edit"));
		assert!(owns("/admin/publish/save"));
		// The dashboard's own pages, and a page that shares the first letters, are not the
		// composer's.
		assert!(!owns("/admin"));
		assert!(!owns("/admin/publisher"));
		assert!(!owns("/admin/database"));
		assert!(!owns("/asides"));
		Ok(())
	}

	/// A slug a form invented does not reach past its own key.
	#[test]
	fn test_a_slug_cannot_leave_its_key_01() -> Outcome<()> {
		assert!(valid_slug("on-rent"));
		assert!(valid_slug("a_note_2"));
		assert!(valid_slug("2026"));
		// The ones that matter: a slash names another key, a dot pair climbs, a space and a quote
		// arrive somewhere as something else.
		assert!(!valid_slug(""));
		assert!(!valid_slug("../../etc/passwd"));
		assert!(!valid_slug("publish/index"));
		assert!(!valid_slug("on rent"));
		assert!(!valid_slug("on\"rent"));
		assert!(!valid_slug("on<rent"));
		assert!(!valid_slug(&"a".repeat(SLUG_MAX + 1)));
		Ok(())
	}

	/// A date is the shape the feed needs, or it is nothing.
	#[test]
	fn test_a_date_is_shaped_or_absent_02() -> Outcome<()> {
		assert!(valid_date(""));
		assert!(valid_date("2026-07-17"));
		assert!(!valid_date("17/07/2026"));
		assert!(!valid_date("2026-7-7"));
		assert!(!valid_date("yesterday"));
		assert!(!valid_date("2026-07-17T09:00:00Z"));
		Ok(())
	}

	/// A query string gives up one field, and says nothing where it has nothing.
	///
	/// The query arrives as its own raw substring, with no leading `?`, because a request's path and
	/// query are parsed apart. Reading it out of the path instead finds nothing, every time, and
	/// silently -- which is exactly what the dashboard's own database page did.
	#[test]
	fn test_a_query_field_is_read_03() -> Outcome<()> {
		assert_eq!(query_field("slug=on-rent", "slug"), Some(fmt!("on-rent")));
		assert_eq!(query_field("a=1&slug=on-rent&b=2", "slug"), Some(fmt!("on-rent")));
		assert_eq!(query_field("slug=a%20b", "slug"), Some(fmt!("a b")));
		assert_eq!(query_field("other=1", "slug"), None);
		assert_eq!(query_field("", "slug"), None);
		// An empty value is not a value: `?slug=` names no post, and the editor opens blank rather
		// than looking for a post called nothing.
		assert_eq!(query_field("slug=", "slug"), None);
		Ok(())
	}

	/// What a redirect says survives being said in a URL.
	#[test]
	fn test_a_reason_survives_the_redirect_04() -> Outcome<()> {
		let enc = url_encode("a post with no prose in it is not a post");
		assert!(!enc.contains(' '));
		assert_eq!(url_decode(&enc), "a post with no prose in it is not a post");
		Ok(())
	}
}
