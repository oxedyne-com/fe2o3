//! Managing a site's posts, from within the site.
//!
//! The post half of the console: a list of what the site has written, an editor, a preview of a
//! draft, and a way to read a directory of Markdown in. The operations themselves live in
//! [`crate::srv::publish::store`] and are shared with the reader-facing pages; this is the surface
//! over them, dressed in the site's own look and reached only by a site admin.
//!
//! It was once the composer, mounted in the operator's dashboard at `/admin/publish` and gated on the
//! operator's session. That put a site's content behind the key to the whole host, which was the
//! wrong tier: writing a post is a site's business, not the server's. It moved here, behind the
//! site's own admins, and left the dashboard for the server's own concerns.
//!
//! # What a form may say
//!
//! A slug and a date arrive from a browser, which means they arrive from anywhere. Both are checked
//! ([`valid_slug`], [`valid_date`]) before either reaches a key: a slug is pasted into
//! `publish/post/<slug>` and into a URL, and a form's word for one is not a reason to trust it. The
//! prose is not checked -- it is Markdown, and Markdown that will not parse is refused by the parser
//! where it is rendered -- but it is escaped everywhere it is shown except the preview, which is the
//! rendered HTML a reader would get.

use crate::srv::{
	console::{
		SiteAdmin,
		Theme,
		page,
		redirect,
	},
	publish::{
		Markup,
		PostKind,
		PostState,
		PublishConfig,
		Source,
		dest::{
			DeliveryState,
			Destination,
		},
		send,
		store::{
			self,
			Record,
		},
		date_text,
		normalise_date,
		render_source,
		valid_date,
		valid_slug,
	},
};

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_iop_crypto::enc::Encrypter;
use oxedyne_fe2o3_iop_db::api::Database;
use oxedyne_fe2o3_iop_hash::api::Hasher;
use oxedyne_fe2o3_jdat::{
	prelude::*,
	id::NumIdDat,
	string::enc::EncoderConfig,
};
use oxedyne_fe2o3_net::http::{
	fields::{
		HeaderFieldValue,
		HeaderName,
	},
	msg::HttpMessage,
	status::HttpStatus,
};

use tokio_rustls::rustls::ClientConfig;

use std::sync::{
	Arc,
	RwLock,
};

use super::html_escape;


/// The console's root, where the posts are listed.
pub const PATH_ROOT: &str = "/manage";

/// The editor, for one post or for a new one.
pub const PATH_EDIT: &str = "/manage/edit";

/// A draft as a reader would get it, if it were not a draft.
pub const PATH_PREVIEW: &str = "/manage/preview";

/// Where the editor posts.
pub const PATH_SAVE: &str = "/manage/save";

/// Where a deletion posts.
pub const PATH_DELETE: &str = "/manage/delete";

/// Where an import of the directory posts.
pub const PATH_IMPORT: &str = "/manage/import";

/// Where the editor posts source to see it rendered, for a live preview.
pub const PATH_RENDER: &str = "/manage/render";

/// The posts as JSON, every state, for a front-end that renders its own list.
pub const PATH_LIST_JSON: &str = "/manage/list.json";

/// One post's source and rendering as JSON, for a front-end that edits it in place.
pub const PATH_POST_JSON: &str = "/manage/post.json";


/// Whether a path is one this module writes to.
pub fn writes(path: &str) -> bool {
	path == PATH_SAVE || path == PATH_DELETE || path == PATH_IMPORT
}

/// Whether a path is a POST this module answers.
///
/// The writes, and the render -- which is a POST because an editor's whole draft is too much for a
/// query string, but changes nothing: it reads source and hands back HTML. It is gated and
/// token-checked with the writes all the same, so the server is not a rendering service for anyone
/// who asks.
pub fn posts(path: &str) -> bool {
	writes(path) || path == PATH_RENDER
}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ GET                                                                       │
// └───────────────────────────────────────────────────────────────────────────┘

/// Serves the console's post pages. The gate ran before this: `admin` is a proven site admin.
pub fn handle_get<
	const UIDL: usize,
	UID:	NumIdDat<UIDL>,
	ENC:	Encrypter,
	KH:	Hasher,
	DB:	Database<UIDL, UID, ENC, KH>,
>(
	cfg:		Option<&PublishConfig>,
	theme:		&Theme,
	admin:		&SiteAdmin,
	csrf:		&str,
	db:		Option<&(Arc<RwLock<DB>>, UID)>,
	request_path:	&str,
	query:		&str,
	id:		&str,
)
	-> Outcome<HttpMessage>
{
	debug!("{}: console: GET {}", id, request_path);

	let cfg = match cfg {
		Some(c)	=> c,
		None	=> return Ok(page(theme, admin, "Manage", &notice(
			"This site publishes nothing. Give it a <code>publish</code> block to manage posts here.",
		))),
	};

	match request_path {
		PATH_ROOT	=> handle_list(cfg, theme, admin, csrf, db, query, id),
		PATH_EDIT	=> handle_edit(cfg, theme, admin, csrf, db, query, id),
		PATH_PREVIEW	=> handle_preview(cfg, theme, admin, db, query, id),
		PATH_LIST_JSON	=> list_json(cfg, db, id),
		PATH_POST_JSON	=> post_json(cfg, db, query, id),
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
	cfg:	&PublishConfig,
	theme:	&Theme,
	admin:	&SiteAdmin,
	csrf:	&str,
	db:	Option<&(Arc<RwLock<DB>>, UID)>,
	query:	&str,
	id:	&str,
)
	-> Outcome<HttpMessage>
{
	let mut body = String::new();

	body.push_str(&fmt!(
		"<h1>Posts</h1>\n<p class=\"mc-muted\">Served at <a href=\"{path}\">{path}</a>.</p>\n",
		path = html_escape(&cfg.path),
	));

	// A write that could not go through said why, in the query it was redirected with. Shown here,
	// where the writer lands, rather than swallowed -- the composer this grew from redirected with
	// the reason and then never showed it.
	if let Some(said) = query_field(query, "said") {
		body.push_str(&notice(&html_escape(&said)));
	}

	// A directory-backed site has nothing to edit here: the files are the posts. Say what to do
	// rather than leave the editor refusing to save with no reason.
	if cfg.source != Source::Store {
		body.push_str(&notice(&fmt!(
			"This site serves its posts from the directory <code>{dir}</code>, so they are edited by \
			editing those files, and there is nothing to write here yet. To move it into the database \
			and write here instead: import first, while the directory is still being served, then set \
			<code>source</code> to <code>\"store\"</code> in this site's <code>publish</code> block \
			and restart. That order keeps the site up; the other empties it until the import runs.",
			dir = html_escape(&cfg.dir),
		)));
		body.push_str(&import_form(csrf, &cfg.dir));
		return Ok(page(theme, admin, "Posts", &body));
	}

	let db = match db {
		Some(db)	=> db,
		None		=> {
			body.push_str(&notice(
				"This site keeps its posts in its database, and has no database configured. Set \
				<code>db_dir_rel</code> on the vhost.",
			));
			return Ok(page(theme, admin, "Posts", &body));
		}
	};

	let recs = match store::list_records(db, id) {
		Ok(r)	=> r,
		Err(e)	=> {
			error!(e, "{}: console: cannot list the posts", id);
			body.push_str(&notice("The posts could not be listed. The log says why."));
			return Ok(page(theme, admin, "Posts", &body));
		}
	};

	body.push_str(&fmt!(
		"<p><a class=\"mc-btn\" href=\"{edit}\">Write a new post</a></p>\n",
		edit = PATH_EDIT,
	));

	if recs.is_empty() {
		body.push_str(&notice("Nothing written yet."));
		body.push_str(&import_form(csrf, &cfg.dir));
		return Ok(page(theme, admin, "Posts", &body));
	}

	body.push_str("<table class=\"mc-table\">\n<thead><tr>\
		<th>Post</th><th>Kind</th><th>State</th><th>Date</th><th></th>\
		</tr></thead>\n<tbody>\n");
	for rec in &recs {
		let slug = html_escape(&rec.slug);
		// The title is the prose's own heading, so it costs a parse to know. Where the prose will not
		// parse, the slug stands in and the state cell says the post is broken.
		let (title, broken) = match rec.render() {
			Ok(p)	=> (html_escape(&p.title), false),
			Err(e)	=> {
				warn!("{}: console: '{}' will not render: {}", id, rec.slug, e);
				(html_escape(&rec.slug), true)
			}
		};
		let state = if broken {
			fmt!("<span class=\"mc-tag mc-tag-err\">will not render</span>")
		} else {
			match rec.state {
				PostState::Live		=> fmt!("<span class=\"mc-tag mc-tag-live\">live</span>"),
				PostState::Draft	=> fmt!("<span class=\"mc-tag\">draft</span>"),
			}
		};
		body.push_str(&fmt!(
			"<tr>\
			<td><a href=\"{edit}?slug={slug}\">{title}</a><br><span class=\"mc-slug\">{slug}</span></td>\
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
			date	= html_escape(&rec.date.as_deref().map(date_text)
				.unwrap_or_else(|| fmt!("--"))),
		));
	}
	body.push_str("</tbody>\n</table>\n");
	body.push_str(&import_form(csrf, &cfg.dir));

	Ok(page(theme, admin, "Posts", &body))
}

/// The editor, for a post that exists or one that does not yet.
fn handle_edit<
	const UIDL: usize,
	UID:	NumIdDat<UIDL>,
	ENC:	Encrypter,
	KH:	Hasher,
	DB:	Database<UIDL, UID, ENC, KH>,
>(
	cfg:	&PublishConfig,
	theme:	&Theme,
	admin:	&SiteAdmin,
	csrf:	&str,
	db:	Option<&(Arc<RwLock<DB>>, UID)>,
	query:	&str,
	id:	&str,
)
	-> Outcome<HttpMessage>
{
	if cfg.source != Source::Store {
		return Ok(page(theme, admin, "Posts", &notice(
			"This site serves its posts from a directory, so there is nothing here to edit them with.",
		)));
	}

	let slug = query_field(query, "slug");

	// No slug is a new post, which is the editor with nothing in it.
	let rec = match &slug {
		None		=> None,
		Some(slug)	=> {
			let db = match db {
				Some(db)	=> db,
				None		=> return Ok(page(theme, admin, "Posts", &notice(
					"This site has no database configured.",
				))),
			};
			match store::get(db, slug) {
				Ok(Some(r))	=> Some(r),
				Ok(None)	=> return Ok(page(theme, admin, "Posts", &notice(
					"There is no post by that name.",
				))),
				Err(e)		=> {
					error!(e, "{}: console: cannot read '{}'", id, slug);
					return Ok(page(theme, admin, "Posts", &notice(
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
		"<form class=\"mc-form\" method=\"POST\" action=\"{save}\">\n\
		<input type=\"hidden\" name=\"csrf\" value=\"{csrf}\">\n\
		<input type=\"hidden\" name=\"was\" value=\"{was}\">\n\
		<div class=\"mc-row\">\n\
			<div>\n\
				<label for=\"slug\">Name in the URL</label>\n\
				<input type=\"text\" id=\"slug\" name=\"slug\" value=\"{slug}\" \
					placeholder=\"on-rent\" required>\n\
			</div>\n\
			<div>\n\
				<label for=\"date\">Date</label>\n\
				<input type=\"text\" id=\"date\" name=\"date\" value=\"{date}\" \
					placeholder=\"2026-07-17 14:30\">\n\
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
			<div>\n\
				<label for=\"markup\">Written in</label>\n\
				<select id=\"markup\" name=\"markup\">\n\
					<option value=\"markdown\"{md_sel}>Markdown</option>\n\
					<option value=\"djot\"{djot_sel}>Djot</option>\n\
				</select>\n\
			</div>\n\
		</div>\n\
		<label for=\"source\">Text</label>\n\
		<textarea id=\"source\" name=\"source\" rows=\"24\" spellcheck=\"true\" \
			placeholder=\"# The title goes here, as the first heading\">{source}</textarea>\n\
		<p class=\"mc-muted\">The title is the post's own most prominent heading. There is no title \
			field because there is no second place to say it. A note shows whole on the site; an essay \
			shows as a card to open. Djot can name a box (<code>:::</code>) and a style \
			(<code>{{.class}}</code>) that Markdown cannot.</p>\n\
		<div class=\"mc-actions\">\n\
			<button type=\"submit\" class=\"mc-btn\">Save</button>\n\
			<a class=\"mc-btn mc-btn-quiet\" href=\"{root}\">Cancel</a>\n\
		</div>\n\
		</form>\n",
		save		= PATH_SAVE,
		root		= PATH_ROOT,
		csrf		= html_escape(csrf),
		// What the post was called on the way in, so a renamed slug takes the old record with it.
		was		= html_escape(&r.slug),
		slug		= html_escape(&r.slug),
		// The readable form in the box: a person edits what a person reads, and the `T` goes back in
		// at the door on the way to the store.
		date		= html_escape(&r.date.as_deref().map(date_text).unwrap_or_default()),
		note_sel	= selected(r.kind == PostKind::Note),
		essay_sel	= selected(r.kind == PostKind::Essay),
		draft_sel	= selected(r.state == PostState::Draft),
		live_sel	= selected(r.state == PostState::Live),
		md_sel		= selected(r.markup == Markup::Markdown),
		djot_sel	= selected(r.markup == Markup::Djot),
		source		= html_escape(&r.source),
	));

	if existing {
		body.push_str(&fmt!(
			"<form class=\"mc-form\" method=\"POST\" action=\"{del}\" \
			onsubmit=\"return confirm('Delete this post? There is no undo.')\">\n\
			<input type=\"hidden\" name=\"csrf\" value=\"{csrf}\">\n\
			<input type=\"hidden\" name=\"slug\" value=\"{slug}\">\n\
			<div class=\"mc-actions\"><button type=\"submit\" class=\"mc-btn mc-btn-danger\">Delete\
			</button></div>\n\
			</form>\n",
			del	= PATH_DELETE,
			csrf	= html_escape(csrf),
			slug	= html_escape(&r.slug),
		));
	}

	Ok(page(theme, admin, "Edit", &body))
}

/// A post as a reader would get it, whether or not a reader can.
///
/// A draft is served to nobody, so its author cannot see it by visiting it. Here the same rendering
/// runs behind the gate.
fn handle_preview<
	const UIDL: usize,
	UID:	NumIdDat<UIDL>,
	ENC:	Encrypter,
	KH:	Hasher,
	DB:	Database<UIDL, UID, ENC, KH>,
>(
	cfg:	&PublishConfig,
	theme:	&Theme,
	admin:	&SiteAdmin,
	db:	Option<&(Arc<RwLock<DB>>, UID)>,
	query:	&str,
	id:	&str,
)
	-> Outcome<HttpMessage>
{
	let slug = match query_field(query, "slug") {
		Some(s)	=> s,
		None	=> return Ok(page(theme, admin, "Posts", &notice("No post was named."))),
	};

	if cfg.source != Source::Store {
		return Ok(page(theme, admin, "Posts", &notice(
			"This site serves its posts from a directory, so every post it has is already readable.",
		)));
	}

	let db = match db {
		Some(db)	=> db,
		None		=> return Ok(page(theme, admin, "Posts", &notice(
			"This site has no database configured.",
		))),
	};

	let rec = match store::get(db, &slug) {
		Ok(Some(r))	=> r,
		Ok(None)	=> return Ok(page(theme, admin, "Posts", &notice(
			"There is no post by that name.",
		))),
		Err(e)		=> {
			error!(e, "{}: console: cannot read '{}'", id, slug);
			return Ok(page(theme, admin, "Posts", &notice(
				"That post could not be read. The log says why.",
			)));
		}
	};

	// The prose is the author's own, rendered by the same renderer that serves it. It is not escaped,
	// because rendered Markdown is HTML and escaping it would show the reader the tags. Everything
	// else on this page is escaped.
	let post = match rec.render() {
		Ok(p)	=> p,
		Err(e)	=> {
			warn!("{}: console: '{}' will not render: {}", id, slug, e);
			return Ok(page(theme, admin, "Posts", &notice(
				"That post will not render as Markdown. The log says where it goes wrong.",
			)));
		}
	};

	let body = fmt!(
		"<p class=\"mc-muted\"><a href=\"{edit}?slug={slug}\">&larr; Back to the editor</a> \
		&middot; {state} &middot; this is the page a reader gets.</p>\n\
		<article class=\"mc-prose aside\">{html}</article>\n",
		edit	= PATH_EDIT,
		slug	= html_escape(&slug),
		state	= match rec.state {
			PostState::Live		=> "live",
			PostState::Draft	=> "a draft, so served to nobody",
		},
		html	= post.html,
	);

	Ok(page(theme, admin, "Preview", &body))
}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ JSON, for a front-end that renders its own management surface              │
// └───────────────────────────────────────────────────────────────────────────┘

/// Every post the store holds, each state, as JSON.
///
/// The same list as the page, for a caller that draws its own: the app's Manage tab renders this in
/// the site's shell rather than send the operator to a page of its own. The reader's `index.json` is
/// the live posts only; this is the author's, so it carries the drafts too, and each post's state.
fn list_json<
	const UIDL: usize,
	UID:	NumIdDat<UIDL>,
	ENC:	Encrypter,
	KH:	Hasher,
	DB:	Database<UIDL, UID, ENC, KH>,
>(
	cfg:	&PublishConfig,
	db:	Option<&(Arc<RwLock<DB>>, UID)>,
	id:	&str,
)
	-> Outcome<HttpMessage>
{
	if cfg.source != Source::Store {
		return Ok(json_body(&fmt!("{{\"posts\":[],\"source\":\"dir\"}}")));
	}
	let db = match db {
		Some(db)	=> db,
		None		=> return Ok(json_body("{\"posts\":[]}")),
	};
	let recs = res!(store::list_records(db, id));
	let mut items = Vec::new();
	for rec in &recs {
		// The title is the prose's own heading; where it will not parse, the slug stands in and the
		// state says the post is broken, exactly as the page does it.
		let (title, broken) = match rec.render() {
			Ok(p)	=> (p.title, false),
			Err(_)	=> (rec.slug.clone(), true),
		};
		let mut m = DaticleMap::new();
		m.insert(dat!("slug"),		dat!(rec.slug.clone()));
		m.insert(dat!("title"),		dat!(title));
		m.insert(dat!("kind"),		dat!(rec.kind.as_str().to_string()));
		m.insert(dat!("markup"),	dat!(rec.markup.as_str().to_string()));
		m.insert(dat!("state"),		dat!(rec.state.as_str().to_string()));
		m.insert(dat!("broken"),	Dat::Bool(broken));
		if let Some(d) = &rec.date {
			m.insert(dat!("date"),		dat!(d.clone()));
			m.insert(dat!("date_text"),	dat!(date_text(d)));
		}
		items.push(Dat::Map(m));
	}
	let body = create_dat_ordmap(vec![(dat!("posts"), Dat::List(items))]);
	Ok(json_body(&res!(body.encode_string_with_config(&EncoderConfig::<(), ()>::json(None)))))
}

/// One post's source and rendering, as JSON.
///
/// What the editor loads to fill its fields, and what a preview shows: the Markdown as written, the
/// kind, the state, the date in the readable form a person edits, and the HTML a reader would get.
fn post_json<
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
	let slug = match query_field(query, "slug") {
		Some(s)	=> s,
		None	=> return Ok(json_error("no post was named")),
	};
	if cfg.source != Source::Store {
		return Ok(json_error("this site serves its posts from a directory"));
	}
	let db = match db {
		Some(db)	=> db,
		None		=> return Ok(json_error("this site has no database configured")),
	};
	let rec = match store::get(db, &slug) {
		Ok(Some(r))	=> r,
		Ok(None)	=> return Ok(json_error("there is no post by that name")),
		Err(e)		=> {
			error!(e, "{}: console: cannot read '{}'", id, slug);
			return Ok(json_error("that post could not be read"));
		}
	};
	// The rendered HTML for a preview, where the prose parses; where it does not, the empty string
	// and a flag, so the editor can say so rather than show nothing and seem to have lost the post.
	let (html, broken) = match rec.render() {
		Ok(p)	=> (p.html, false),
		Err(_)	=> (String::new(), true),
	};
	let mut m = DaticleMap::new();
	m.insert(dat!("slug"),		dat!(rec.slug.clone()));
	m.insert(dat!("source"),	dat!(rec.source.clone()));
	m.insert(dat!("kind"),		dat!(rec.kind.as_str().to_string()));
	m.insert(dat!("markup"),	dat!(rec.markup.as_str().to_string()));
	m.insert(dat!("state"),		dat!(rec.state.as_str().to_string()));
	m.insert(dat!("html"),		dat!(html));
	m.insert(dat!("broken"),	Dat::Bool(broken));
	// The readable form in the field; the `T` goes back in at save.
	m.insert(dat!("date"),		dat!(rec.date.as_deref().map(date_text).unwrap_or_default()));
	// Where the post has already been sent, so the composer's picker shows those destinations ticked
	// and their state -- and so re-saving does not silently drop a remote the post has already reached.
	let dlist: Vec<Dat> = rec.deliveries.iter().map(|d| {
		let (state, permalink) = match &d.state {
			DeliveryState::Queued			=> ("queued", String::new()),
			DeliveryState::Sent { permalink, .. }	=> ("sent", permalink.clone()),
			DeliveryState::Failed { .. }		=> ("failed", String::new()),
		};
		let mut dm = DaticleMap::new();
		dm.insert(dat!("dest"),		dat!(d.dest.as_str().to_string()));
		dm.insert(dat!("state"),	dat!(state.to_string()));
		if !permalink.is_empty() {
			dm.insert(dat!("permalink"), dat!(permalink));
		}
		Dat::Map(dm)
	}).collect();
	m.insert(dat!("deliveries"),	Dat::List(dlist));
	Ok(json_body(&res!(Dat::Map(m).encode_string_with_config(&EncoderConfig::<(), ()>::json(None)))))
}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ POST                                                                      │
// └───────────────────────────────────────────────────────────────────────────┘

/// Serves the console's writes. The gate and the CSRF check ran before this.
pub async fn handle_post<
	const UIDL: usize,
	UID:	NumIdDat<UIDL>,
	ENC:	Encrypter,
	KH:	Hasher,
	DB:	Database<UIDL, UID, ENC, KH>,
>(
	cfg:		Option<&PublishConfig>,
	admin:		&SiteAdmin,
	db:		Option<&(Arc<RwLock<DB>>, UID)>,
	tls_client:	&Option<Arc<ClientConfig>>,
	request_path:	&str,
	body:		&[u8],
	json:		bool,
	id:		&str,
)
	-> Outcome<HttpMessage>
{
	// A render touches no store and needs no config: it reads the source in the body and hands back
	// its HTML. It answers before the store checks below, since none of them bear on it.
	if request_path == PATH_RENDER {
		return do_render(body);
	}

	let cfg = match cfg {
		Some(c)	=> c,
		None	=> return Ok(back_with("this site publishes nothing", json)),
	};
	// Editing what is not served would be writing into the dark, so the editor waits for the store to
	// be the source. Importing does not: it is how a site gets from one to the other, and must run
	// before the switch or the switch empties the site.
	if cfg.source != Source::Store && request_path != PATH_IMPORT {
		return Ok(back_with(
			"this site serves its posts from a directory, so there is nothing to write into; set \
			'source' to 'store' first",
			json,
		));
	}
	let db = match db {
		Some(db)	=> db,
		None		=> return Ok(back_with("this site has no database configured", json)),
	};

	match request_path {
		PATH_SAVE	=> do_save(cfg, db, tls_client, body, &admin.username, json, id).await,
		PATH_DELETE	=> do_delete(db, body, &admin.username, json, id),
		PATH_IMPORT	=> do_import(cfg, db, &admin.username, json, id),
		// Unreachable: `writes` names the same three paths.
		_		=> Ok(back(json)),
	}
}

/// Writes a post, and delivers it to the destinations the author ticked.
async fn do_save<
	const UIDL: usize,
	UID:	NumIdDat<UIDL>,
	ENC:	Encrypter,
	KH:	Hasher,
	DB:	Database<UIDL, UID, ENC, KH>,
>(
	cfg:		&PublishConfig,
	db:		&(Arc<RwLock<DB>>, UID),
	tls_client:	&Option<Arc<ClientConfig>>,
	body:		&[u8],
	who:		&str,
	json:		bool,
	id:		&str,
)
	-> Outcome<HttpMessage>
{
	let slug = super::form_field(body, "slug").unwrap_or_default();
	let slug = slug.trim().to_string();
	if !valid_slug(&slug) {
		return Ok(back_with(
			"a post's name may hold letters, digits, hyphens and underscores, and nothing else",
			json,
		));
	}

	let date = normalise_date(&super::form_field(body, "date").unwrap_or_default());
	if !valid_date(&date) {
		return Ok(back_with(
			"a date is written 2026-07-17, or 2026-07-17 14:30 to say when in the day, or is left empty",
			json,
		));
	}

	let source = super::form_field(body, "source").unwrap_or_default();
	if source.trim().is_empty() {
		return Ok(back_with("a post with no prose in it is not a post", json));
	}

	let kind = PostKind::of(&super::form_field(body, "kind").unwrap_or_default());
	let state = PostState::of(&super::form_field(body, "state").unwrap_or_default());
	let markup = Markup::of(&super::form_field(body, "markup").unwrap_or_default());
	let date = if date.is_empty() { None } else { Some(date) };

	// An edit must not lose where a post has already been sent. So the deliveries are carried forward
	// from the post as it stands -- under its old name where this save renames it, since the deliveries
	// move with the prose they belong to.
	let prior_slug = super::form_field(body, "was")
		.map(|s| s.trim().to_string())
		.filter(|s| !s.is_empty() && valid_slug(s))
		.unwrap_or_else(|| slug.clone());
	let carried = res!(store::get(db, &prior_slug))
		.map(|r| r.deliveries)
		.unwrap_or_default();

	// The credentials that actually apply: what the console has set, over what the config names. The
	// picker offers these and a delivery is sent with them, so the two never disagree about which
	// remotes a site can reach.
	let creds = res!(send::effective_creds(db, cfg));

	// The destinations the author ticked, kept to those the site actually has credentials for -- a
	// browser's word for a remote is not a reason to queue a post the site cannot send. A post is only
	// delivered once it is live: a draft goes nowhere, so ticking a destination on a draft queues
	// nothing until it is published.
	let chosen: Vec<Destination> = super::form_field(body, "destinations")
		.unwrap_or_default()
		.split(',')
		.filter_map(|w| Destination::of(w.trim()))
		.filter(|d| creds.has(*d))
		.collect();

	let deliveries = if state == PostState::Live && !chosen.is_empty() {
		// The title and canonical link a social rendition is derived from. The title is the post's own
		// heading, as everywhere else; the link is where the post will live on this site.
		let title = match render_source(&source, slug.clone(), date.clone(), kind, markup) {
			Ok(p)	=> p.title,
			Err(_)	=> slug.clone(),
		};
		let url = cfg.url_of(&cfg.path_of(&slug));
		send::queue_deliveries(&carried, &chosen, &title, &url)
	} else {
		carried
	};

	let rec = Record {
		slug:	slug.clone(),
		kind,
		state,
		markup,
		date,
		source,
		deliveries,
	};

	res!(store::put(db, &rec, id));

	// A renamed post is a new key; the old one would otherwise stay behind, served and indexed, a
	// second copy of prose the author believes they moved. The old name is what the editor was opened
	// with, not what the form now says, which is why the form carries both.
	if let Some(was) = super::form_field(body, "was") {
		let was = was.trim();
		if !was.is_empty() && was != slug && valid_slug(was) {
			match store::delete(db, was, id) {
				Ok(_)	=> info!("{}: console: '{}' renamed '{}' to '{}'", id, who, was, slug),
				Err(e)	=> warn!(
					"{}: console: '{}' was renamed to '{}' and the old one would not delete: {}",
					id, was, slug, e),
			}
		}
	}

	info!("{}: console: '{}' saved '{}' ({})", id, who, rec.slug, rec.state.as_str());

	// Deliver what is queued, now, while the request is in hand: the handler holds the outbound TLS
	// client and the database, which a save is the natural moment to use. A delivery that fails records
	// its failure on the post and does not fail the save -- the post is written either way, and the
	// send is best-effort with its own state to show for it. A site with no outbound TLS client cannot
	// reach a remote at all, and says so in the log rather than silently dropping the queue.
	if rec.state == PostState::Live && rec.deliveries.iter().any(|d| !d.state.is_terminal()) {
		match tls_client {
			Some(tls)	=> {
				match send::deliver_post(db, &creds, tls.clone(), &slug, id).await {
					Ok(n)	=> info!("{}: console: '{}' attempted {} deliver(y/ies)", id, slug, n),
					Err(e)	=> warn!("{}: console: '{}' delivery sweep failed: {}", id, slug, e),
				}
			}
			None	=> warn!(
				"{}: console: '{}' has queued deliveries but the server has no outbound TLS client",
				id, slug),
		}
	}

	Ok(back(json))
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
	json:	bool,
	id:	&str,
)
	-> Outcome<HttpMessage>
{
	let slug = match super::form_field(body, "slug") {
		Some(s)	=> s,
		None	=> return Ok(back_with("no post was named", json)),
	};
	if !valid_slug(&slug) {
		return Ok(back_with("that is not a post's name", json));
	}
	let existed = res!(store::delete(db, &slug, id));
	if existed {
		info!("{}: console: '{}' deleted '{}'", id, who, slug);
	}
	Ok(back(json))
}

/// Renders a run of source to HTML, for a live preview.
///
/// No store, no slug, no date: only the source and the syntax it is in. What comes back is the same
/// HTML a reader would get, so the box a `:::` makes and the class a `{...}` names are seen where
/// they will land. A source that will not parse answers with the reason, which the editor shows in
/// place of the preview rather than leaving the last good render on screen as though nothing were
/// wrong.
fn do_render(body: &[u8]) -> Outcome<HttpMessage> {
	let source = super::form_field(body, "source").unwrap_or_default();
	let markup = Markup::of(&super::form_field(body, "markup").unwrap_or_default());
	match crate::srv::publish::render_html(&source, markup) {
		Ok(html)	=> {
			let m = create_dat_ordmap(vec![(dat!("html"), dat!(html))]);
			Ok(json_body(&res!(m.encode_string_with_config(&EncoderConfig::<(), ()>::json(None)))))
		}
		Err(e)		=> Ok(json_error(&fmt!("that will not render: {}", e))),
	}
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
	json:	bool,
	id:	&str,
)
	-> Outcome<HttpMessage>
{
	let n = match store::import_dir(db, &cfg.dir, id) {
		Ok(n)	=> n,
		Err(e)	=> {
			error!(e, "{}: console: import from '{}' failed", id, cfg.dir);
			return Ok(back_with("the directory could not be read; the log says why", json));
		}
	};
	info!("{}: console: '{}' imported {} posts from '{}'", id, who, n, cfg.dir);
	Ok(back(json))
}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ HELPERS                                                                   │
// └───────────────────────────────────────────────────────────────────────────┘

/// The answer to a write that went through.
///
/// Two callers, two shapes. A form wants a redirect, so a reload does not write again; the app,
/// which asked for JSON, wants a plain yes it can act on without a page changing under it.
fn back(json: bool) -> HttpMessage {
	if json {
		json_body("{\"ok\":true}")
	} else {
		redirect(PATH_ROOT)
	}
}

/// The answer to a write that did not, carrying the reason -- in the query a form lands with, or in
/// the JSON the app reads.
fn back_with(why: &str, json: bool) -> HttpMessage {
	if json {
		json_error(why)
	} else {
		redirect(&fmt!("{}?said={}", PATH_ROOT, url_encode(why)))
	}
}

/// A JSON body, already encoded.
fn json_body(body: &str) -> HttpMessage {
	let mut resp = HttpMessage::ok_respond_with_text(body.to_string());
	resp = resp.with_field(
		HeaderName::ContentType,
		HeaderFieldValue::Generic(fmt!("application/json")),
	);
	resp
}

/// A JSON error a caller can read, its reason escaped for a string literal.
fn json_error(why: &str) -> HttpMessage {
	let m = create_dat_ordmap(vec![(dat!("error"), dat!(why.to_string()))]);
	match m.encode_string_with_config(&EncoderConfig::<(), ()>::json(None)) {
		Ok(j)	=> json_body(&j),
		// The error about the error. Say the plain thing rather than nothing.
		Err(_)	=> json_body("{\"error\":\"error\"}"),
	}
}

/// A thing the page says.
fn notice(html: &str) -> String {
	fmt!("<p class=\"mc-notice\">{}</p>\n", html)
}

/// The button that reads the directory in.
fn import_form(csrf: &str, dir: &str) -> String {
	fmt!(
		"<form class=\"mc-form\" method=\"POST\" action=\"{import}\">\n\
		<input type=\"hidden\" name=\"csrf\" value=\"{csrf}\">\n\
		<p class=\"mc-muted\">Import the Markdown in <code>{dir}</code>. A post the store already holds \
		is overwritten, so importing twice is importing once.</p>\n\
		<div class=\"mc-actions\"><button type=\"submit\" class=\"mc-btn mc-btn-quiet\">Import the \
		directory</button></div>\n\
		</form>\n",
		import	= PATH_IMPORT,
		csrf	= html_escape(csrf),
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

/// Decode a percent-encoded value: `+` is a space, `%XX` a byte, a bad escape itself.
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

	/// The console writes to these three paths and reads the rest.
	#[test]
	fn test_writes_are_the_three_mutations_00() -> Outcome<()> {
		assert!(writes("/manage/save"));
		assert!(writes("/manage/delete"));
		assert!(writes("/manage/import"));
		assert!(!writes("/manage"));
		assert!(!writes("/manage/edit"));
		assert!(!writes("/manage/preview"));
		Ok(())
	}

	/// A query field is read out of the raw substring, and an empty value names nothing.
	#[test]
	fn test_a_query_field_is_read_01() -> Outcome<()> {
		assert_eq!(query_field("slug=on-rent", "slug"), Some(fmt!("on-rent")));
		assert_eq!(query_field("a=1&slug=on-rent", "slug"), Some(fmt!("on-rent")));
		assert_eq!(query_field("slug=", "slug"), None);
		assert_eq!(query_field("", "slug"), None);
		Ok(())
	}

	/// A reason survives being carried in a redirect's query and read back out.
	#[test]
	fn test_a_reason_survives_the_redirect_02() -> Outcome<()> {
		let enc = url_encode("a post with no prose in it is not a post");
		assert!(!enc.contains(' '));
		assert_eq!(url_decode(&enc), "a post with no prose in it is not a post");
		Ok(())
	}
}
