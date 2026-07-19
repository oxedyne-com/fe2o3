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
		send::{
			self,
			MailSender,
		},
		comment,
		store::{
			self,
			Record,
		},
		subscribe,
		date_text,
		normalise_date,
		parse_tags,
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

use std::{
	collections::BTreeMap,
	sync::{
		Arc,
		RwLock,
	},
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

/// The site's tag vocabulary as JSON, for the composer's palette.
pub const PATH_TAGS_JSON: &str = "/manage/tags.json";

/// A site's destination settings as JSON: the public fields and whether a secret is set, never the
/// secret itself.
pub const PATH_CREDS_JSON: &str = "/manage/creds.json";

/// Where the destination settings form posts a remote's credentials.
pub const PATH_CREDS: &str = "/manage/creds";

/// The destinations page: the remotes a post can be sent on to, and the credentials for each.
pub const PATH_DESTS: &str = "/manage/destinations";

/// The moderation queue: what has been said and what is waiting on a decision.
pub const PATH_COMMENTS: &str = "/manage/comments";

/// Where the queue's approve, spam, remove, erase and block actions post.
pub const PATH_COMMENTS_ACTION: &str = "/manage/comments/action";

/// The moderation queue as JSON, for an app that draws it itself.
pub const PATH_COMMENTS_JSON: &str = "/manage/comments.json";

/// The subscriber list as JSON, for an app that draws the list itself.
pub const PATH_SUBS_JSON: &str = "/manage/subscribers.json";

/// The reports as JSON, for an app that draws them itself.
pub const PATH_REPORTS_JSON: &str = "/manage/reports.json";

/// The subscribers page: the newsletter's list, its count, and where a post is sent to it.
pub const PATH_SUBS: &str = "/manage/subscribers";

/// The subscriber list as CSV, for the site to keep the list it owns.
pub const PATH_SUBS_CSV: &str = "/manage/subscribers.csv";

/// The reports page: what the list is made of, and what has been sent to it.
pub const PATH_REPORTS: &str = "/manage/reports";

/// The bucket a report counts a record under when it carries no month to file it by.
const UNDATED: &str = "unknown";

/// Where the "send to subscribers" form posts a live post's slug.
pub const PATH_NEWSLETTER: &str = "/manage/newsletter";

/// Where the per-subscriber unsubscribe and remove forms post.
///
/// One endpoint, two actions: an `action` of `unsubscribe` sets the address [`unsubscribed`], and one of
/// `delete` erases it outright. The target is the `email` field, exactly as the admin sees it in the
/// list, mirroring the admin-management page's own `action` form.
pub const PATH_SUBS_ACTION: &str = "/manage/subscribers/action";

/// Where the "send a test" form posts a slug and a single recipient.
pub const PATH_NEWSLETTER_TEST: &str = "/manage/newsletter/test";


/// Whether a path is one this module writes to.
pub fn writes(path: &str) -> bool {
	path == PATH_SAVE
		|| path == PATH_DELETE
		|| path == PATH_IMPORT
		|| path == PATH_CREDS
		|| path == PATH_NEWSLETTER
		|| path == PATH_SUBS_ACTION
		|| path == PATH_COMMENTS_ACTION
		|| path == PATH_NEWSLETTER_TEST
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
		PATH_SUBS	=> subscribers_page(cfg, theme, admin, csrf, db, query, id),
		PATH_SUBS_CSV	=> subscribers_csv(db, id),
		PATH_REPORTS	=> reports_page(theme, admin, db, id),
		PATH_DESTS	=> destinations_page(cfg, theme, admin, csrf, db, query, id),
		PATH_LIST_JSON	=> list_json(cfg, db, id),
		PATH_POST_JSON	=> post_json(cfg, db, query, id),
		PATH_TAGS_JSON	=> tags_json(cfg, db, id),
		PATH_CREDS_JSON	=> creds_json(cfg, db, id),
		PATH_SUBS_JSON	=> subs_json(db, id),
		PATH_COMMENTS	=> comments_page(theme, admin, csrf, db, query, id),
		PATH_COMMENTS_JSON	=> comments_json(db, query, id),
		PATH_REPORTS_JSON	=> reports_json(db, id),
		_		=> Ok(HttpMessage::respond_with_text(
			HttpStatus::NotFound,
			"Not found.",
		)),
	}
}

/// The subscribers page: the newsletter's list, its count, an export, and a way to send a live post to
/// the list.
///
/// The home of "own the list, own the send": the confirmed count is the reach, the table is the list,
/// the CSV is the copy the site keeps, and the send form picks a live post and mails it to every
/// confirmed address. A directory-backed site keeps its posts in files rather than the store, so it can
/// still hold subscribers but has no store post to send; the send form is offered only where the store
/// is the source.
fn subscribers_page<
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
	body.push_str("<h1>Subscribers</h1>\n");

	if let Some(said) = query_field(query, "said") {
		body.push_str(&notice(&html_escape(&said)));
	}

	let db = match db {
		Some(db)	=> db,
		None		=> {
			body.push_str(&notice(
				"This site keeps its subscribers in its database, and has no database configured. Set \
				<code>db_dir_rel</code> on the vhost.",
			));
			return Ok(page(theme, admin, "Subscribers", &body));
		}
	};

	let subs = match subscribe::list(db, id) {
		Ok(s)	=> s,
		Err(e)	=> {
			error!(e, "{}: console: cannot list the subscribers", id);
			body.push_str(&notice("The subscribers could not be listed. The log says why."));
			return Ok(page(theme, admin, "Subscribers", &body));
		}
	};
	let confirmed = subs.iter().filter(|s| s.state == subscribe::SubState::Confirmed).count();
	let pending = subs.iter().filter(|s| s.state == subscribe::SubState::Pending).count();
	let unsubbed = subs.iter().filter(|s| s.state == subscribe::SubState::Unsubscribed).count();
	let bounced = subs.iter().filter(|s| s.state == subscribe::SubState::Bounced).count();

	// The counts, said once and briefly. Which states receive a post is a rule of the system, not
	// news about this list, and repeating it on every visit is how a page stops being read.
	body.push_str(&fmt!(
		"<p class=\"mc-muted\">{counts} &middot; <a href=\"{csv}\">Export CSV</a></p>\n",
		counts	= if subs.is_empty() {
			fmt!("No subscribers yet")
		} else {
			fmt!("{} confirmed &middot; {} pending &middot; {} unsubscribed &middot; {} bounced",
				confirmed, pending, unsubbed, bounced)
		},
		csv	= PATH_SUBS_CSV,
	));

	// The send form and the test-send form, where the store is the source and there is a live post to
	// send. The test needs only a live post, not a confirmed subscriber, so it is offered even where the
	// send set is empty.
	// The list is what this page is about, so it comes first and the sending follows it. An empty
	// list says so once, in the line above, and does not repeat itself in a box of its own.
	if subs.is_empty() {
		body.push_str(&send_section(cfg, csrf, db, confirmed, id));
		return Ok(page(theme, admin, "Subscribers", &body));
	}

	// A list that outgrows a screen needs the same two things the posts do: a way to look for one
	// address, and a way not to render ten thousand rows into one page.
	let q = query_field(query, "q").unwrap_or_default();
	let want = query_field(query, "state").unwrap_or_default();
	let needle = q.to_lowercase();
	let shown: Vec<&subscribe::Subscriber> = subs.iter()
		.filter(|s| want.is_empty() || s.state.as_str() == want)
		.filter(|s| needle.is_empty() || s.email.to_lowercase().contains(&needle))
		.collect();

	body.push_str(&subs_filter(&q, &want, shown.len(), subs.len()));

	if shown.is_empty() {
		body.push_str(&notice("No subscriber matches that."));
		body.push_str(&send_section(cfg, csrf, db, confirmed, id));
		return Ok(page(theme, admin, "Subscribers", &body));
	}

	let page_at = query_field(query, "page").and_then(|p| p.parse::<usize>().ok()).unwrap_or(1).max(1);
	let pages = shown.len().div_ceil(PAGE_SIZE).max(1);
	let page_at = page_at.min(pages);
	let from = (page_at - 1) * PAGE_SIZE;
	let upto = (from + PAGE_SIZE).min(shown.len());

	body.push_str("<table class=\"mc-table\">\n<thead><tr>\
		<th>Address</th><th>State</th><th>Since</th><th></th>\
		</tr></thead>\n<tbody>\n");
	for sub in &shown[from..upto] {
		let state = match sub.state {
			subscribe::SubState::Confirmed	=> fmt!("<span class=\"mc-tag mc-tag-live\">confirmed</span>"),
			subscribe::SubState::Pending	=> fmt!("<span class=\"mc-tag\">pending</span>"),
			subscribe::SubState::Unsubscribed	=>
				fmt!("<span class=\"mc-tag mc-tag-err\">unsubscribed</span>"),
			subscribe::SubState::Bounced	=>
				fmt!("<span class=\"mc-tag mc-tag-err\">bounced</span>"),
		};
		body.push_str(&fmt!(
			"<tr><td>{email}</td><td>{state}</td><td>{since}</td><td>{actions}</td></tr>\n",
			email	= html_escape(&sub.email),
			state	= state,
			since	= html_escape(sub.created.as_deref().unwrap_or("--")),
			actions	= subscriber_actions(csrf, sub),
		));
	}
	body.push_str("</tbody>\n</table>\n");
	body.push_str(&pager(PATH_SUBS, &q, &want, page_at, pages));
	body.push_str(&send_section(cfg, csrf, db, confirmed, id));

	Ok(page(theme, admin, "Subscribers", &body))
}

/// The per-subscriber actions: unsubscribe where they still receive mail, and erase, always.
///
/// Two small CSRF-protected forms posting to [`PATH_SUBS_ACTION`] with the address as their target. The
/// unsubscribe is offered only where it would change something -- an address already unsubscribed or
/// bounced is past it -- while the erase is offered on every row, since a record in any state can be a
/// thing a person has asked be forgotten.
fn subscriber_actions(csrf: &str, sub: &subscribe::Subscriber) -> String {
	let mut s = String::new();
	s.push_str("<div class=\"mc-actions\">");
	let receiving = matches!(
		sub.state,
		subscribe::SubState::Confirmed | subscribe::SubState::Pending);
	if receiving {
		s.push_str(&fmt!(
			"<form method=\"POST\" action=\"{act}\" style=\"display:inline\">\
			<input type=\"hidden\" name=\"csrf\" value=\"{csrf}\">\
			<input type=\"hidden\" name=\"action\" value=\"unsubscribe\">\
			<input type=\"hidden\" name=\"email\" value=\"{email}\">\
			<button type=\"submit\" class=\"mc-ico\" title=\"Unsubscribe\" \
			aria-label=\"Unsubscribe\">{close}</button>\
			</form>",
			act	= PATH_SUBS_ACTION,
			csrf	= html_escape(csrf),
			email	= html_escape(&sub.email),
			close	= icon("close"),
		));
	}
	s.push_str(&fmt!(
		"<form method=\"POST\" action=\"{act}\" style=\"display:inline\" \
		onsubmit=\"return confirm('Erase {email} for good? There is no undo.')\">\
		<input type=\"hidden\" name=\"csrf\" value=\"{csrf}\">\
		<input type=\"hidden\" name=\"action\" value=\"delete\">\
		<input type=\"hidden\" name=\"email\" value=\"{email}\">\
		<button type=\"submit\" class=\"mc-ico mc-ico-danger\" title=\"Erase\" \
		aria-label=\"Erase\">{trash}</button>\
		</form>",
		act	= PATH_SUBS_ACTION,
		csrf	= html_escape(csrf),
		email	= html_escape(&sub.email),
		trash	= icon("trash"),
	));
	s.push_str("</div>");
	s
}

/// Everything to do with sending, under one heading and below the list.
///
/// The list is the page's subject and the sending is what is done with it, so the sending follows
/// it rather than sitting on top of it. Grouped, because a send form, a test form and a history
/// loose on a page read as three unrelated things.
fn send_section<
	const UIDL: usize,
	UID:	NumIdDat<UIDL>,
	ENC:	Encrypter,
	KH:	Hasher,
	DB:	Database<UIDL, UID, ENC, KH>,
>(
	cfg:		&PublishConfig,
	csrf:		&str,
	db:		&(Arc<RwLock<DB>>, UID),
	confirmed:	usize,
	id:		&str,
)
	-> String
{
	let mut s = String::new();
	s.push_str("<h2>Send a post</h2>\n<div class=\"mc-send\">\n");

	if cfg.source == Source::Store {
		s.push_str(&send_form(cfg, csrf, db, confirmed, id));
		s.push_str(&test_form(csrf, db, id));
	} else {
		s.push_str(&notice(
			"This site serves its posts from a directory, so a post is not in the database to send. \
			Move to the store to mail a post to subscribers.",
		));
	}

	s.push_str("</div>\n");

	// A read that fails costs the table, not the page, so it logs and carries on.
	match send::send_history(db) {
		Ok(hist)	=> s.push_str(&history_table(&hist)),
		Err(e)		=> {
			warn!("{}: console: cannot read the send history: {}", id, e);
			s.push_str(&notice("The send history could not be read. The log says why."));
		}
	}
	s
}

/// The search-and-filter row over the subscriber list, by address and by state.
fn subs_filter(q: &str, want: &str, showing: usize, total: usize) -> String {
	let count = if showing == total {
		fmt!("{} subscribers", total)
	} else {
		fmt!("{} of {} subscribers", showing, total)
	};
	fmt!(
		"<form class=\"mc-filter mc-form\" method=\"GET\" action=\"{subs}\">\n\
		<div class=\"mc-f-text\"><label for=\"q\">Search</label>\
		<input type=\"text\" id=\"q\" name=\"q\" value=\"{q}\" placeholder=\"address\"></div>\n\
		<div class=\"mc-f-sel\"><label for=\"state\">State</label>\
		<select id=\"state\" name=\"state\">\
		<option value=\"\"{any}>Any</option>\
		<option value=\"confirmed\"{conf}>Confirmed</option>\
		<option value=\"pending\"{pend}>Pending</option>\
		<option value=\"unsubscribed\"{unsub}>Unsubscribed</option>\
		<option value=\"bounced\"{bounce}>Bounced</option>\
		</select></div>\n\
		<button type=\"submit\" class=\"mc-btn mc-btn-quiet\">Filter</button>\n\
		<span class=\"mc-muted\" style=\"margin:0 0 0 auto\">{count}</span>\n\
		</form>\n",
		subs	= PATH_SUBS,
		q	= html_escape(q),
		any	= selected(want.is_empty()),
		conf	= selected(want == "confirmed"),
		pend	= selected(want == "pending"),
		unsub	= selected(want == "unsubscribed"),
		bounce	= selected(want == "bounced"),
		count	= count,
	)
}

/// The send-history table: post, when, and how each send's attempts ended, most recent first.
///
/// Says nothing where nothing has been sent, so a site that has not yet mailed a post shows no empty
/// table. Every value is the site's own record, and the slug is escaped where it lands in markup.
fn history_table(hist: &[send::SendEntry]) -> String {
	if hist.is_empty() {
		return String::new();
	}
	let mut s = String::new();
	s.push_str("<h2>Send history</h2>\n");
	s.push_str("<table class=\"mc-table\">\n<thead><tr>\
		<th>Post</th><th>When</th><th>Attempted</th><th>Sent</th><th>Failed</th><th>Suppressed</th>\
		</tr></thead>\n<tbody>\n");
	for e in hist {
		s.push_str(&fmt!(
			"<tr><td><span class=\"mc-slug\">{slug}</span></td><td>{at}</td>\
			<td>{attempted}</td><td>{sent}</td><td>{failed}</td><td>{suppressed}</td></tr>\n",
			slug		= html_escape(&e.slug),
			at		= html_escape(&e.at),
			attempted	= e.attempted,
			sent		= e.sent,
			failed		= e.failed,
			suppressed	= e.suppressed,
		));
	}
	s.push_str("</tbody>\n</table>\n");
	s
}

/// The reports page: what the list is made of, how it grew, and what has been sent to it.
///
/// Two questions, answered from what the site already records: who is on the list, and what happened
/// to the posts mailed to it. Both are aggregations over the subscriber store and the send history --
/// nothing here is measured for the purpose, and nothing is asked of a reader. There is deliberately
/// no open or click tracking: an open pixel and a rewritten link are surveillance of a person who
/// only asked to be sent some prose, and the site does not do it.
///
/// The honest ceiling, stated on the page as well as here: a subscriber records the moment it signed
/// up and nothing else, so growth is knowable and cohort behaviour is not. Where a rate would be a
/// guess, a share of the list as it stands is given instead, and said to be that.
fn reports_page<
	const UIDL: usize,
	UID:	NumIdDat<UIDL>,
	ENC:	Encrypter,
	KH:	Hasher,
	DB:	Database<UIDL, UID, ENC, KH>,
>(
	theme:	&Theme,
	admin:	&SiteAdmin,
	db:	Option<&(Arc<RwLock<DB>>, UID)>,
	id:	&str,
)
	-> Outcome<HttpMessage>
{
	let mut body = String::new();
	body.push_str("<h1>Reports</h1>\n");

	let db = match db {
		Some(db)	=> db,
		None		=> {
			body.push_str(&notice(
				"This site keeps its subscribers in its database, and has no database configured. Set \
				<code>db_dir_rel</code> on the vhost.",
			));
			return Ok(page(theme, admin, "Reports", &body));
		}
	};

	// The list. A read that fails costs its half of the page, not the page, so the send half still
	// renders.
	match subscribe::list(db, id) {
		Ok(subs)	=> body.push_str(&list_report(&subs)),
		Err(e)		=> {
			error!(e, "{}: console: cannot list the subscribers for the report", id);
			body.push_str(&notice("The subscribers could not be listed. The log says why."));
		}
	}

	// The sends.
	match send::send_history(db) {
		Ok(hist)	=> body.push_str(&send_report(&hist)),
		Err(e)		=> {
			warn!("{}: console: cannot read the send history for the report: {}", id, e);
			body.push_str(&notice("The send history could not be read. The log says why."));
		}
	}

	// The reads. Two reads that must both land, so a failure in either costs this section alone.
	match (store::reads_all(db, id), store::list_records(db, id)) {
		(Ok(reads), Ok(recs))	=> body.push_str(&reads_report(&reads, &recs)),
		(Err(e), _)		=> {
			warn!("{}: console: cannot read the read tallies for the report: {}", id, e);
			body.push_str(&notice("The read counts could not be read. The log says why."));
		}
		(_, Err(e))		=> {
			warn!("{}: console: cannot list the posts for the read report: {}", id, e);
			body.push_str(&notice("The posts could not be listed. The log says why."));
		}
	}

	Ok(page(theme, admin, "Reports", &body))
}

/// The reads half of the report: how often each post has been read, most-read first.
///
/// What is counted, said on the page rather than left to be inferred: a request that served the post
/// to somebody who was neither carrying a management session nor an obvious machine. What is *not*
/// counted is the more important half -- nothing identifies a reader, so this is a tally of readings
/// and never of people, and it cannot answer "how many different readers" because it never knew.
///
/// A tally whose post no longer exists is folded into one line rather than listed. The count is real
/// and dropping it silently would make the total disagree with the rows; naming each deleted slug
/// would be a list of things the reader cannot act on.
fn reads_report(reads: &BTreeMap<String, u64>, recs: &[Record]) -> String {
	let mut s = String::new();
	s.push_str("<h2>Reads</h2>\n");

	if reads.is_empty() {
		s.push_str(&notice(
			"Nothing has been read yet. A read is counted when a post is served to somebody who is \
			neither signed in to manage the site nor an obvious machine.",
		));
		return s;
	}

	// The rows a reader can act on: a live post and its tally, most-read first. A post nobody has
	// read yet is shown at nought rather than omitted -- "which of my posts is unread" is exactly
	// the question this page should answer.
	let mut rows: Vec<(&str, String, u64)> = Vec::new();
	for rec in recs {
		let title = match rec.render() {
			Ok(p)	=> p.title,
			Err(_)	=> rec.slug.clone(),
		};
		rows.push((&rec.slug, title, reads.get(&rec.slug).copied().unwrap_or(0)));
	}
	rows.sort_by(|a, b| b.2.cmp(&a.2).then_with(|| a.1.cmp(&b.1)));

	let total: u64 = reads.values().sum();
	let live: u64 = rows.iter().map(|r| r.2).sum();
	let gone = total.saturating_sub(live);
	let read_posts = rows.iter().filter(|r| r.2 > 0).count();

	s.push_str(&stat_cards(&[
		("Reads",	fmt!("{}", total),	"posts served, all time"),
		("Posts read",	fmt!("{}/{}", read_posts, rows.len()),	"have been read at least once"),
	]));

	s.push_str("<table class=\"mc-table\">\n<thead><tr>\
		<th>Post</th><th>Reads</th><th>Share</th>\
		</tr></thead>\n<tbody>\n");
	for (slug, title, n) in &rows {
		s.push_str(&fmt!(
			"<tr><td>{title}<br><span class=\"mc-slug\">{slug}</span></td>\
			<td>{n}</td><td>{share}</td></tr>\n",
			title	= html_escape(title),
			slug	= html_escape(slug),
			n	= n,
			share	= if live == 0 { fmt!("&mdash;") } else { pct(*n as usize, live as usize) },
		));
	}
	s.push_str("</tbody>\n</table>\n");

	if gone > 0 {
		s.push_str(&fmt!(
			"<p class=\"mc-muted\">{} {} counted against posts that have since been deleted.</p>\n",
			gone,
			if gone == 1 { "read was" } else { "reads were" },
		));
	}

	// The ceiling, on the page, for the same reason the list report states its own: an absent figure
	// reads as an oversight unless it is named as a decision.
	s.push_str(&notice(
		"A read is a reading, not a reader: nothing identifies who asked, so one person returning \
		twice counts twice. There is no open or click tracking anywhere on this site.",
	));
	s
}

/// The list half of the report: the states as they stand, the shares they make, and growth by month.
fn list_report(subs: &[subscribe::Subscriber]) -> String {
	let mut s = String::new();
	s.push_str("<h2>The list</h2>\n");

	if subs.is_empty() {
		s.push_str(&notice("Nobody has subscribed yet, so there is nothing to report."));
		return s;
	}

	let confirmed = subs.iter().filter(|x| x.state == subscribe::SubState::Confirmed).count();
	let pending = subs.iter().filter(|x| x.state == subscribe::SubState::Pending).count();
	let unsubbed = subs.iter().filter(|x| x.state == subscribe::SubState::Unsubscribed).count();
	let bounced = subs.iter().filter(|x| x.state == subscribe::SubState::Bounced).count();
	let total = subs.len();

	s.push_str(&stat_cards(&[
		("Reach", fmt!("{}", confirmed), "confirmed, and receiving"),
		("Awaiting", fmt!("{}", pending), "signed up, not yet confirmed"),
		("Left", fmt!("{}", unsubbed), "unsubscribed"),
		("Suppressed", fmt!("{}", bounced), "bounced, never retried"),
	]));

	s.push_str(&fmt!(
		"<p class=\"mc-muted\">{total} addresses on record. {conf_pct} of them are confirmed and \
		{pend_pct} are still to confirm. Of those who confirmed, {churn} have since unsubscribed.</p>\n",
		total		= total,
		conf_pct	= pct(confirmed, total),
		pend_pct	= pct(pending, total),
		churn		= pct(unsubbed, confirmed + unsubbed),
	));

	s.push_str(&notice(
		"These are shares of the list as it stands, not rates over time. A subscriber records when it \
		signed up and nothing else -- there is no confirmed-on or unsubscribed-on date -- so an address \
		that confirmed and later left counts only in <em>left</em>, and the confirmed share therefore \
		understates how many ever confirmed.",
	));

	s.push_str(&month_table(
		"Signed up by month",
		"Sign-ups",
		&by_month(subs.iter().map(|x| x.created.as_deref().unwrap_or(""))),
	));
	s
}

/// The send half of the report: the totals across every send, the rate they make, and the per-post
/// and per-month rollups.
fn send_report(hist: &[send::SendEntry]) -> String {
	let mut s = String::new();
	s.push_str("<h2>Newsletter sends</h2>\n");

	if hist.is_empty() {
		s.push_str(&notice("No post has been mailed to the list yet, so there is nothing to report."));
		return s;
	}

	let attempted: usize = hist.iter().map(|e| e.attempted).sum();
	let sent: usize = hist.iter().map(|e| e.sent).sum();
	let failed: usize = hist.iter().map(|e| e.failed).sum();
	let suppressed: usize = hist.iter().map(|e| e.suppressed).sum();

	s.push_str(&stat_cards(&[
		("Sends", fmt!("{}", hist.len()), "posts mailed to the list"),
		("Accepted", fmt!("{}", sent), "taken by a receiving server"),
		("Delivery", pct(sent, attempted), "of every address attempted"),
		("Suppressed", fmt!("{}", suppressed), "hard failures, now off the list"),
	]));

	s.push_str(&fmt!(
		"<p class=\"mc-muted\">{attempted} addresses attempted across {sends} sends: {sent} accepted, \
		{failed} failed for now and will be tried on the next send, {suppressed} refused for good and \
		suppressed.</p>\n",
		attempted	= attempted,
		sends		= hist.len(),
		sent		= sent,
		failed		= failed,
		suppressed	= suppressed,
	));

	s.push_str(&notice(
		"<em>Accepted</em> is what a receiving server took, which is not the same as what a person read. \
		Whether a message was opened, and whether a link in it was followed, are deliberately not \
		recorded.",
	));

	// Per post, most attempted first: which post reached the most people.
	let mut per_post: BTreeMap<&str, (usize, usize, usize, usize, usize)> = BTreeMap::new();
	for e in hist {
		let row = per_post.entry(e.slug.as_str()).or_insert((0, 0, 0, 0, 0));
		row.0 += 1;
		row.1 += e.attempted;
		row.2 += e.sent;
		row.3 += e.failed;
		row.4 += e.suppressed;
	}
	let mut rows: Vec<(&str, (usize, usize, usize, usize, usize))> = per_post.into_iter().collect();
	rows.sort_by(|a, b| b.1.1.cmp(&a.1.1));

	s.push_str("<h3>By post</h3>\n");
	s.push_str("<table class=\"mc-table\">\n<thead><tr>\
		<th>Post</th><th>Sends</th><th>Attempted</th><th>Accepted</th><th>Delivery</th>\
		<th>Suppressed</th>\
		</tr></thead>\n<tbody>\n");
	for (slug, (sends, att, ok, _fail, supp)) in &rows {
		s.push_str(&fmt!(
			"<tr><td><span class=\"mc-slug\">{slug}</span></td><td>{sends}</td><td>{att}</td>\
			<td>{ok}</td><td>{rate}</td><td>{supp}</td></tr>\n",
			slug	= html_escape(slug),
			sends	= sends,
			att	= att,
			ok	= ok,
			rate	= pct(*ok, *att),
			supp	= supp,
		));
	}
	s.push_str("</tbody>\n</table>\n");

	s.push_str(&month_table(
		"Sends by month",
		"Sends",
		&by_month(hist.iter().map(|e| e.at.as_str())),
	));
	s
}

/// A row of headline numbers: a big figure, what it counts, and a word on what it means.
///
/// Four at most read well on a phone, which is the width this is built for.
fn stat_cards(cards: &[(&str, String, &str)]) -> String {
	let mut s = String::new();
	s.push_str("<div class=\"mc-stats\">\n");
	for (key, value, note) in cards {
		s.push_str(&fmt!(
			"<div class=\"mc-stat\"><div class=\"mc-stat-n\">{value}</div>\
			<div class=\"mc-stat-k\">{key}</div><div class=\"mc-stat-note\">{note}</div></div>\n",
			value	= html_escape(value),
			key	= html_escape(key),
			note	= html_escape(note),
		));
	}
	s.push_str("</div>\n");
	s
}

/// Counts by calendar month, newest first, from a run of ISO timestamps.
///
/// A timestamp this cannot read a month from is counted under `unknown` rather than dropped: a
/// subscriber that predates the sign-up date being recorded is still a subscriber, and a total that
/// quietly disagreed with the list above it would be worse than an honest bucket.
fn by_month<'a, I: Iterator<Item = &'a str>>(stamps: I) -> Vec<(String, usize)> {
	let mut months: BTreeMap<String, usize> = BTreeMap::new();
	for stamp in stamps {
		let key = if stamp.len() >= 7 && stamp.is_char_boundary(7) {
			stamp[..7].to_string()
		} else {
			fmt!("{}", UNDATED)
		};
		*months.entry(key).or_insert(0) += 1;
	}
	// Newest month first -- and `unknown` last whatever it sorts as, since it is not a month and would
	// otherwise sit above every real one on a plain descending sort.
	let mut out: Vec<(String, usize)> = months.into_iter().collect();
	out.sort_by(|a, b| {
		let (a_odd, b_odd) = (a.0 == UNDATED, b.0 == UNDATED);
		a_odd.cmp(&b_odd).then_with(|| b.0.cmp(&a.0))
	});
	out
}

/// A month-by-month table with a bar for the shape of it, the widest month full width.
fn month_table(heading: &str, unit: &str, months: &[(String, usize)]) -> String {
	if months.is_empty() {
		return String::new();
	}
	let peak = months.iter().map(|(_, n)| *n).max().unwrap_or(0);
	let mut s = String::new();
	s.push_str(&fmt!("<h3>{}</h3>\n", html_escape(heading)));
	s.push_str(&fmt!(
		"<table class=\"mc-table\">\n<thead><tr><th>Month</th><th>{}</th><th></th></tr></thead>\n\
		<tbody>\n",
		html_escape(unit),
	));
	for (month, n) in months {
		// The bar is decoration over the number beside it, so a zero peak simply draws nothing.
		let width = if peak > 0 { (n * 100) / peak } else { 0 };
		s.push_str(&fmt!(
			"<tr><td>{month}</td><td>{n}</td>\
			<td><div class=\"mc-bar\"><div class=\"mc-bar-fill\" style=\"width:{width}%\"></div></div></td>\
			</tr>\n",
			month	= html_escape(month),
			n	= n,
			width	= width,
		));
	}
	s.push_str("</tbody>\n</table>\n");
	s
}

/// How many rows a list page shows before it pages.
const PAGE_SIZE: usize = 20;

/// The search-and-filter row over a list of posts.
///
/// Says how many of how many are being shown, because a filter that silently hides things is how a
/// person concludes their work has been lost. Submits by GET, so a filtered list is a link.
fn list_filter(q: &str, want: &str, showing: usize, total: usize) -> String {
	let count = if showing == total {
		fmt!("{} posts", total)
	} else {
		fmt!("{} of {} posts", showing, total)
	};
	fmt!(
		"<form class=\"mc-filter mc-form\" method=\"GET\" action=\"{root}\">\n\
		<div class=\"mc-f-text\"><label for=\"q\">Search</label>\
		<input type=\"text\" id=\"q\" name=\"q\" value=\"{q}\" placeholder=\"title or name\"></div>\n\
		<div class=\"mc-f-sel\"><label for=\"state\">State</label>\
		<select id=\"state\" name=\"state\">\
		<option value=\"\"{any}>Any</option>\
		<option value=\"draft\"{draft}>Draft</option>\
		<option value=\"live\"{live}>Live</option>\
		</select></div>\n\
		<button type=\"submit\" class=\"mc-btn mc-btn-quiet\">Filter</button>\n\
		<span class=\"mc-muted\" style=\"margin:0 0 0 auto\">{count}</span>\n\
		</form>\n",
		root	= PATH_ROOT,
		q	= html_escape(q),
		any	= selected(want.is_empty()),
		draft	= selected(want == "draft"),
		live	= selected(want == "live"),
		count	= count,
	)
}

/// Previous and next over a paged list, and where in it the reader is.
///
/// Nothing at all where everything fits on one page: a pager under a list of four is furniture that
/// says only that there is no more.
fn pager(path: &str, q: &str, want: &str, at: usize, pages: usize) -> String {
	if pages <= 1 {
		return String::new();
	}
	let carry = fmt!("&q={}&state={}", url_encode(q), url_encode(want));
	let mut s = String::from("<div class=\"mc-pager\">");
	if at > 1 {
		s.push_str(&fmt!("<a href=\"{}?page={}{}\">Previous</a>", path, at - 1, carry));
	}
	s.push_str(&fmt!("<span class=\"mc-pager-at\">Page {} of {}</span>", at, pages));
	if at < pages {
		s.push_str(&fmt!("<a href=\"{}?page={}{}\">Next</a>", path, at + 1, carry));
	}
	s.push_str("</div>\n");
	s
}

/// A post's row actions: read it as a reader would, and delete it.
///
/// Icons, because a row is a place for a verb and not a sentence, and because two words per row
/// across twenty rows is a wall of text where the eye wants the titles. Deleting asks first.
fn post_actions(csrf: &str, slug: &str) -> String {
	fmt!(
		"<div class=\"mc-actions\">\
		<a class=\"mc-ico\" href=\"{preview}?slug={slug}\" title=\"Preview as a reader\" \
		aria-label=\"Preview as a reader\">{eye}</a>\
		<form method=\"POST\" action=\"{del}\" style=\"display:inline\" \
		onsubmit=\"return confirm('Delete &quot;{slug}&quot;? There is no undo.')\">\
		<input type=\"hidden\" name=\"csrf\" value=\"{csrf}\">\
		<input type=\"hidden\" name=\"slug\" value=\"{slug}\">\
		<button type=\"submit\" class=\"mc-ico mc-ico-danger\" title=\"Delete\" \
		aria-label=\"Delete\">{trash}</button>\
		</form>\
		</div>",
		preview	= PATH_PREVIEW,
		del	= PATH_DELETE,
		csrf	= html_escape(csrf),
		slug	= html_escape(slug),
		eye	= icon("eye"),
		trash	= icon("trash"),
	)
}

/// The live preview: the text as it will read, beside the box it is typed in.
///
/// The server renders it, over [`PATH_RENDER`], because there is one tested parser and it is in
/// Rust. That costs a round trip, so it is debounced rather than run per keystroke -- and the delay
/// is why the pane says nothing at all until the first render lands, rather than flashing empty.
/// Changing the markup select re-renders too: the same source is a different document in Djot.
///
/// A failed render shows its complaint in the pane. Prose that will not parse is a thing the author
/// wants to see immediately, and it is the one message the preview exists to deliver.
fn preview_script(csrf: &str) -> String {
	fmt!(
		"<script>\n\
		(function () {{\n\
		\tvar src = document.getElementById('source');\n\
		\tvar out = document.getElementById('mc-preview');\n\
		\tvar mk = document.getElementById('markup');\n\
		\tif (!src || !out) return;\n\
		\tvar timer = null;\n\
		\tfunction draw() {{\n\
		\t\tvar body = 'csrf={csrf}&markup=' + encodeURIComponent(mk ? mk.value : 'markdown')\n\
		\t\t\t+ '&source=' + encodeURIComponent(src.value);\n\
		\t\tfetch('{render}', {{ method: 'POST', credentials: 'same-origin',\n\
		\t\t\theaders: {{ 'Content-Type': 'application/x-www-form-urlencoded' }}, body: body }})\n\
		\t\t\t.then(function (r) {{ return r.json(); }})\n\
		\t\t\t.then(function (d) {{\n\
		\t\t\t\tif (d && typeof d.html === 'string') out.innerHTML = d.html;\n\
		\t\t\t\telse if (d && d.error) out.textContent = d.error;\n\
		\t\t\t}})\n\
		\t\t\t.catch(function () {{}});\n\
		\t}}\n\
		\tfunction soon() {{ clearTimeout(timer); timer = setTimeout(draw, 400); }}\n\
		\tsrc.addEventListener('input', soon);\n\
		\tif (mk) mk.addEventListener('change', draw);\n\
		\tdraw();\n\
		}})();\n\
		</script>\n",
		csrf	= html_escape(csrf),
		render	= PATH_RENDER,
	)
}

/// An inline SVG icon, drawn in the current text colour at the size of the control holding it.
///
/// Inline rather than a file: the console is one response with no asset it can be separated from,
/// and a stylesheet that reaches for an image is a stylesheet that can arrive without one. Unknown
/// names give nothing, so a typo shows as a bare button rather than a broken glyph.
/// The close icon, for the page shell, which draws the way out of the console itself.
pub fn icon_close() -> &'static str {
	icon("close")
}

fn icon(name: &str) -> &'static str {
	match name {
		"close"	=> "<svg viewBox=\"0 0 16 16\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"1.6\" \
			stroke-linecap=\"round\" aria-hidden=\"true\"><path d=\"M4 4l8 8M12 4l-8 8\"/></svg>",
		"trash"	=> "<svg viewBox=\"0 0 16 16\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"1.4\" \
			stroke-linecap=\"round\" stroke-linejoin=\"round\" aria-hidden=\"true\">\
			<path d=\"M2.5 4h11M6 4V2.5h4V4M4 4l.7 9.5h6.6L12 4M6.5 6.5v5M9.5 6.5v5\"/></svg>",
		"eye"	=> "<svg viewBox=\"0 0 16 16\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"1.4\" \
			stroke-linecap=\"round\" stroke-linejoin=\"round\" aria-hidden=\"true\">\
			<path d=\"M1 8s2.6-4.5 7-4.5S15 8 15 8s-2.6 4.5-7 4.5S1 8 1 8z\"/>\
			<circle cx=\"8\" cy=\"8\" r=\"1.9\"/></svg>",
		_	=> "",
	}
}

/// A percentage of a total, to one decimal place, or a dash where the total is zero.
///
/// Nothing out of nothing is not zero per cent, and printing it as such would invent a fact.
fn pct(n: usize, d: usize) -> String {
	if d == 0 {
		return fmt!("--");
	}
	fmt!("{:.1}%", (n as f64 * 100.0) / d as f64)
}

/// The "send a post to subscribers" form: a live post picked from a select, and the send.
///
/// Offered with a count of who will receive it, so the operator sends with their eyes open. Where no
/// post is live, or nobody is confirmed, it says so instead of offering a button that would do nothing.
fn send_form<
	const UIDL: usize,
	UID:	NumIdDat<UIDL>,
	ENC:	Encrypter,
	KH:	Hasher,
	DB:	Database<UIDL, UID, ENC, KH>,
>(
	_cfg:	&PublishConfig,
	csrf:	&str,
	db:	&(Arc<RwLock<DB>>, UID),
	confirmed:	usize,
	id:	&str,
)
	-> String
{
	// The live posts, the only ones a newsletter may carry: a draft is sent to nobody.
	let live: Vec<Record> = match store::list_records(db, id) {
		Ok(recs)	=> recs.into_iter().filter(|r| r.state == PostState::Live).collect(),
		Err(e)		=> {
			warn!("{}: console: cannot list posts for the send form: {}", id, e);
			Vec::new()
		}
	};
	if live.is_empty() {
		return notice("No post is live to send. Publish a post first, then send it here.");
	}
	if confirmed == 0 {
		return notice("No confirmed subscribers to send to yet.");
	}

	let mut opts = String::new();
	for rec in &live {
		let title = match rec.render() {
			Ok(p)	=> p.title,
			Err(_)	=> rec.slug.clone(),
		};
		opts.push_str(&fmt!(
			"<option value=\"{slug}\">{title}</option>\n",
			slug	= html_escape(&rec.slug),
			title	= html_escape(&title),
		));
	}

	fmt!(
		"<form class=\"mc-form\" method=\"POST\" action=\"{send}\" \
		onsubmit=\"return confirm('Send this post to {n} confirmed subscriber(s)? There is no undo.')\">\n\
		<input type=\"hidden\" name=\"csrf\" value=\"{csrf}\">\n\
		<label for=\"mc-send-slug\">Send a post to {n} subscriber(s)</label>\n\
		<select id=\"mc-send-slug\" name=\"slug\">\n{opts}</select>\n\
		<div class=\"mc-actions\">\n\
		<button type=\"submit\" class=\"mc-btn\">Send to subscribers</button>\n\
		</div>\n\
		</form>\n",
		send	= PATH_NEWSLETTER,
		csrf	= html_escape(csrf),
		n	= confirmed,
		opts	= opts,
	)
}

/// The "send a test" form: a live post, an address to send it to, and the send.
///
/// The operator's own preview -- it mails the chosen post to one address and touches nothing: no
/// subscriber, no state, no history. Offered wherever there is a live post, since a test needs no
/// confirmed subscriber. Says so where there is none, rather than a select with nothing to pick.
fn test_form<
	const UIDL: usize,
	UID:	NumIdDat<UIDL>,
	ENC:	Encrypter,
	KH:	Hasher,
	DB:	Database<UIDL, UID, ENC, KH>,
>(
	csrf:	&str,
	db:	&(Arc<RwLock<DB>>, UID),
	id:	&str,
)
	-> String
{
	// The live posts, the only ones the test offers, since a test is a preview of what a subscriber gets
	// and a subscriber only ever gets a live post.
	let live: Vec<Record> = match store::list_records(db, id) {
		Ok(recs)	=> recs.into_iter().filter(|r| r.state == PostState::Live).collect(),
		Err(e)		=> {
			warn!("{}: console: cannot list posts for the test form: {}", id, e);
			Vec::new()
		}
	};
	if live.is_empty() {
		return String::new();
	}

	let mut opts = String::new();
	for rec in &live {
		let title = match rec.render() {
			Ok(p)	=> p.title,
			Err(_)	=> rec.slug.clone(),
		};
		opts.push_str(&fmt!(
			"<option value=\"{slug}\">{title}</option>\n",
			slug	= html_escape(&rec.slug),
			title	= html_escape(&title),
		));
	}

	fmt!(
		"<form class=\"mc-form\" method=\"POST\" action=\"{test}\">\n\
		<input type=\"hidden\" name=\"csrf\" value=\"{csrf}\">\n\
		<label for=\"mc-test-slug\">Send a test to one address</label>\n\
		<select id=\"mc-test-slug\" name=\"slug\">\n{opts}</select>\n\
		<input type=\"text\" id=\"mc-test-to\" name=\"test_to\" placeholder=\"you@example.com\" \
		autocomplete=\"off\" spellcheck=\"false\">\n\
		<div class=\"mc-actions\">\n\
		<button type=\"submit\" class=\"mc-btn mc-btn-quiet\">Send test</button>\n\
		</div>\n\
		</form>\n",
		test	= PATH_NEWSLETTER_TEST,
		csrf	= html_escape(csrf),
		opts	= opts,
	)
}

/// The subscriber list as a CSV download.
fn subscribers_csv<
	const UIDL: usize,
	UID:	NumIdDat<UIDL>,
	ENC:	Encrypter,
	KH:	Hasher,
	DB:	Database<UIDL, UID, ENC, KH>,
>(
	db:	Option<&(Arc<RwLock<DB>>, UID)>,
	id:	&str,
)
	-> Outcome<HttpMessage>
{
	let db = match db {
		Some(db)	=> db,
		None		=> return Ok(HttpMessage::respond_with_text(
			HttpStatus::NotFound, "Not found.")),
	};
	let csv = res!(subscribe::export(db, id));
	let mut resp = HttpMessage::ok_respond_with_text(csv);
	resp = resp.with_field(
		HeaderName::ContentType,
		HeaderFieldValue::Generic(fmt!("text/csv; charset=utf-8")),
	);
	Ok(resp)
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

	// The one thing this page is for, at the top right where the eye lands after the heading.
	body.push_str(&fmt!(
		"<div class=\"mc-head-row\"><h1>Posts</h1>\
		<div class=\"mc-actions\"><a class=\"mc-btn\" href=\"{edit}\">Write</a></div></div>\n\
		<p class=\"mc-muted\">Served at <a href=\"{path}\">{path}</a>.</p>\n",
		edit = PATH_EDIT,
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

	if recs.is_empty() {
		body.push_str(&notice("Nothing written yet."));
		body.push_str(&import_form(csrf, &cfg.dir));
		return Ok(page(theme, admin, "Posts", &body));
	}

	// What the reader of this page asked to see. A site with three posts needs none of this; a site
	// with three hundred is unusable without it, and the same page has to serve both.
	let q = query_field(query, "q").unwrap_or_default();
	let want = query_field(query, "state").unwrap_or_default();
	let needle = q.to_lowercase();

	// Titles cost a parse, so each record is rendered once here and the result carried: the filter
	// wants the title, the row wants the title, and parsing twice for one row would be paying twice.
	let mut rows: Vec<(&Record, String, bool)> = Vec::new();
	for rec in &recs {
		let (title, broken) = match rec.render() {
			Ok(p)	=> (p.title, false),
			Err(e)	=> {
				warn!("{}: console: '{}' will not render: {}", id, rec.slug, e);
				(rec.slug.clone(), true)
			}
		};
		let matches_state = match want.as_str() {
			"draft"	=> rec.state == PostState::Draft,
			"live"	=> rec.state == PostState::Live,
			_	=> true,
		};
		let matches_text = needle.is_empty()
			|| title.to_lowercase().contains(&needle)
			|| rec.slug.to_lowercase().contains(&needle);
		if matches_state && matches_text {
			rows.push((rec, title, broken));
		}
	}

	body.push_str(&list_filter(&q, &want, rows.len(), recs.len()));

	if rows.is_empty() {
		body.push_str(&notice("No post matches that."));
		return Ok(page(theme, admin, "Posts", &body));
	}

	// One page of them. Slicing after the filter, so a search reaches the whole site and not just
	// whatever happened to be on the page being looked at.
	let page_at = query_field(query, "page").and_then(|p| p.parse::<usize>().ok()).unwrap_or(1).max(1);
	let pages = rows.len().div_ceil(PAGE_SIZE).max(1);
	let page_at = page_at.min(pages);
	let from = (page_at - 1) * PAGE_SIZE;
	let upto = (from + PAGE_SIZE).min(rows.len());

	body.push_str("<table class=\"mc-table\">\n<thead><tr>\
		<th>Post</th><th>Kind</th><th>State</th><th>Date</th><th></th>\
		</tr></thead>\n<tbody>\n");
	for (rec, title, broken) in &rows[from..upto] {
		let rec = *rec;
		let broken = *broken;
		let title = html_escape(title);
		let slug = html_escape(&rec.slug);
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
			<td>{actions}</td>\
			</tr>\n",
			edit	= PATH_EDIT,
			slug	= slug,
			title	= title,
			kind	= rec.kind.as_str(),
			state	= state,
			date	= html_escape(&rec.date.as_deref().map(date_text)
				.unwrap_or_else(|| fmt!("--"))),
			actions	= post_actions(csrf, &rec.slug),
		));
	}
	body.push_str("</tbody>\n</table>\n");
	body.push_str(&pager(PATH_ROOT, &q, &want, page_at, pages));
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

	let heading = match &rec {
		Some(_)	=> "Edit a post",
		None	=> "Write a new post",
	};
	let r = rec.unwrap_or_default();

	// The site's accumulating vocabulary, for the click-to-add palette. A read the composer already
	// pays for the list; a failure to read it costs the palette, not the editor, so it logs and
	// carries on with an empty one rather than refusing the page.
	let palette = match db {
		Some(db)	=> match store::all_tags(db, id) {
			Ok(t)	=> t,
			Err(e)	=> {
				warn!("{}: console: cannot list the tag vocabulary: {}", id, e);
				Vec::new()
			}
		},
		None		=> Vec::new(),
	};

	// The title row: what this is, and the way out. The way out is the close, not a Cancel button --
	// leaving is not an action of the same weight as saving, and should not look like one.
	let mut body = fmt!(
		"<div class=\"mc-head-row\"><h1>{heading}</h1>\
		<a class=\"mc-close\" href=\"{root}\" title=\"Close\" aria-label=\"Close\">{close}</a></div>\n",
		heading	= heading,
		root	= PATH_ROOT,
		close	= icon("close"),
	);

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
		{tags_block}\
		<div class=\"mc-split\">\n\
			<div class=\"mc-pane\">\n\
				<label for=\"source\">Text</label>\n\
				<textarea id=\"source\" name=\"source\" rows=\"24\" spellcheck=\"true\" \
					placeholder=\"# The title goes here, as the first heading\">{source}</textarea>\n\
			</div>\n\
			<div class=\"mc-pane\">\n\
				<label for=\"mc-preview\">Preview</label>\n\
				<div class=\"mc-preview\" id=\"mc-preview\"></div>\n\
			</div>\n\
		</div>\n\
		<div class=\"mc-actions\">\n\
			<button type=\"submit\" class=\"mc-btn\">Save</button>\n\
		</div>\n\
		</form>\n",
		save		= PATH_SAVE,
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
		// A whole block, pre-built, so the inline script's braces never reach the format string.
		tags_block	= tags_field(&r.tags, &palette),
	));

	// Deleting is not an editing action: it belongs beside the post in the list, where a person is
	// choosing between posts, not in the editor, where a person is working on one. The editor's only
	// verb is Save.
	body.push_str(&preview_script(csrf));

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

	// The state is worth saying, because a draft looks identical here and is served to nobody. It is
	// a badge, as everywhere else a state is shown, rather than a sentence explaining itself.
	let body = fmt!(
		"<div class=\"mc-head-row\"><h1>Preview {state}</h1>\
		<a class=\"mc-close\" href=\"{edit}?slug={slug}\" title=\"Back to the editor\" \
		aria-label=\"Back to the editor\">{close}</a></div>\n\
		<article class=\"mc-prose aside\">{html}</article>\n",
		edit	= PATH_EDIT,
		slug	= html_escape(&slug),
		close	= icon("close"),
		state	= match rec.state {
			PostState::Live		=> fmt!("<span class=\"mc-tag mc-tag-live\">live</span>"),
			PostState::Draft	=> fmt!("<span class=\"mc-tag\">draft</span>"),
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
	// The post's tags, so the editor fills its chips from the record it is editing. Always an array,
	// empty for an untagged post, so the front-end need not ask whether the key is there.
	m.insert(dat!("tags"),
		Dat::List(rec.tags.iter().map(|t| dat!(t.clone())).collect()));
	Ok(json_body(&res!(Dat::Map(m).encode_string_with_config(&EncoderConfig::<(), ()>::json(None)))))
}

/// The site's tag vocabulary as JSON: every tag any post wears, sorted.
///
/// Feeds the composer's palette, so a tag is offered as soon as one post uses it. Gated as every
/// console read is -- the gate ran before this -- so a non-admin never reaches it.
fn tags_json<
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
	// A directory-backed site keeps no tags, so its vocabulary is empty rather than an error.
	if cfg.source != Source::Store {
		return Ok(json_body("{\"tags\":[]}"));
	}
	let db = match db {
		Some(db)	=> db,
		None		=> return Ok(json_body("{\"tags\":[]}")),
	};
	let tags = res!(store::all_tags(db, id));
	let list = Dat::List(tags.iter().map(|t| dat!(t.clone())).collect());
	let body = create_dat_ordmap(vec![(dat!("tags"), list)]);
	Ok(json_body(&res!(body.encode_string_with_config(&EncoderConfig::<(), ()>::json(None)))))
}


/// The destinations page: the remotes this site can send a post on to, and what each needs to do it.
///
/// The server-rendered twin of the app's Destinations panel, and the only one a site without the app
/// has. Every secret here is write-only, exactly as it is over JSON: a stored secret comes back as the
/// word that one is held and never as its value, and a field left blank keeps what is stored, so a
/// handle can be corrected without re-typing a password. A remote the config file also provides is
/// named as such, because a site whose credentials come from `{env:}` or `{file:}` should not be told
/// its destination is unset.
fn destinations_page<
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
	body.push_str("<h1>Destinations</h1>\n");

	if let Some(said) = query_field(query, "said") {
		body.push_str(&notice(&html_escape(&said)));
	}

	let db = match db {
		Some(db)	=> db,
		None		=> {
			body.push_str(&notice(
				"This site keeps its destination credentials in its database, and has no database \
				configured. Set <code>db_dir_rel</code> on the vhost.",
			));
			return Ok(page(theme, admin, "Destinations", &body));
		}
	};

	// A read that fails costs the forms, not the page: without knowing what is stored, a form cannot
	// honestly say whether a secret is held, and a form that guesses is worse than none.
	let stored = match send::get_creds(db) {
		Ok(c)	=> c,
		Err(e)	=> {
			error!(e, "{}: console: cannot read the destination credentials", id);
			body.push_str(&notice("The destination settings could not be read. The log says why."));
			return Ok(page(theme, admin, "Destinations", &body));
		}
	};

	body.push_str(&fmt!(
		"<p class=\"mc-muted\">A post you save can be sent on to these. A secret is stored encrypted \
		and never shown again &mdash; leave a secret field blank to keep the one held.</p>\n",
	));

	// Mastodon: an instance to post to, and a token to post with.
	body.push_str(&dest_panel(
		"Mastodon",
		"mastodon",
		csrf,
		stored.mastodon.is_some(),
		cfg.creds.mastodon.is_some(),
		&fmt!(
			"<div class=\"mc-f-text\"><label for=\"base_url\">Instance URL</label>\
			<input type=\"text\" id=\"base_url\" name=\"base_url\" value=\"{url}\" \
			placeholder=\"https://mastodon.social\"></div>\n\
			<div class=\"mc-f-text\"><label for=\"token\">Access token</label>\
			<input type=\"password\" id=\"token\" name=\"token\" autocomplete=\"new-password\" \
			placeholder=\"{hint}\"></div>\n",
			url	= html_escape(&stored.mastodon.as_ref().map(|c| c.base_url.clone()).unwrap_or_default()),
			hint	= if stored.mastodon.is_some() { "kept" } else { "required" },
		),
	));

	// Bluesky: a handle, an app password, and a host that almost always wants its default.
	body.push_str(&dest_panel(
		"Bluesky",
		"bluesky",
		csrf,
		stored.bluesky.is_some(),
		cfg.creds.bluesky.is_some(),
		&fmt!(
			"<div class=\"mc-f-text\"><label for=\"handle\">Handle</label>\
			<input type=\"text\" id=\"handle\" name=\"handle\" value=\"{handle}\" \
			placeholder=\"you.bsky.social\"></div>\n\
			<div class=\"mc-f-text\"><label for=\"host\">Host</label>\
			<input type=\"text\" id=\"host\" name=\"host\" value=\"{host}\" \
			placeholder=\"{default}\"></div>\n\
			<div class=\"mc-f-text\"><label for=\"app_password\">App password</label>\
			<input type=\"password\" id=\"app_password\" name=\"app_password\" \
			autocomplete=\"new-password\" placeholder=\"{hint}\"></div>\n",
			handle	= html_escape(&stored.bluesky.as_ref().map(|c| c.handle.clone()).unwrap_or_default()),
			host	= html_escape(&stored.bluesky.as_ref().map(|c| c.host.clone()).unwrap_or_default()),
			default	= send::BLUESKY_HOST_DEFAULT,
			hint	= if stored.bluesky.is_some() { "kept" } else { "required" },
		),
	));

	Ok(page(theme, admin, "Destinations", &body))
}

/// One remote's settings: its fields, whether it is set, and the ways to set or clear it.
///
/// Clearing is a second form rather than a checkbox in the first, so that saving cannot clear by
/// accident, and it is offered only where something is stored to clear.
fn dest_panel(
	title:		&str,
	dest:		&str,
	csrf:		&str,
	is_set:		bool,
	in_config:	bool,
	fields:		&str,
)
	-> String
{
	let mut s = String::new();
	s.push_str(&fmt!("<h2>{}</h2>\n", html_escape(title)));

	// What the site knows about this remote, said before the form asks anything of it.
	let state = match (is_set, in_config) {
		(true, true)	=> "Set here, and also in the configuration file.",
		(true, false)	=> "Set.",
		(false, true)	=> "Set in the configuration file, not here.",
		(false, false)	=> "Not set.",
	};
	s.push_str(&fmt!("<p class=\"mc-muted\">{}</p>\n", state));

	s.push_str(&fmt!(
		"<form class=\"mc-form mc-settings\" method=\"POST\" action=\"{creds}\">\n\
		<input type=\"hidden\" name=\"csrf\" value=\"{csrf}\">\n\
		<input type=\"hidden\" name=\"dest\" value=\"{dest}\">\n\
		{fields}\
		<button type=\"submit\" class=\"mc-btn\">Save</button>\n\
		</form>\n",
		creds	= PATH_CREDS,
		csrf	= html_escape(csrf),
		dest	= html_escape(dest),
		fields	= fields,
	));

	if is_set {
		s.push_str(&fmt!(
			"<form class=\"mc-form mc-settings\" method=\"POST\" action=\"{creds}\" \
			onsubmit=\"return confirm('Clear the {title} credentials?')\">\n\
			<input type=\"hidden\" name=\"csrf\" value=\"{csrf}\">\n\
			<input type=\"hidden\" name=\"dest\" value=\"{dest}\">\n\
			<input type=\"hidden\" name=\"clear\" value=\"1\">\n\
			<button type=\"submit\" class=\"mc-btn mc-btn-danger\">Clear</button>\n\
			</form>\n",
			creds	= PATH_CREDS,
			csrf	= html_escape(csrf),
			dest	= html_escape(dest),
			title	= html_escape(title),
		));
	}
	s
}

/// The subscriber list as JSON.
///
/// The same data the subscribers page renders, for an app that would rather draw it in its own idiom
/// than open a page of the server's. The addresses are the site's own list and the session asking has
/// already been established as a site admin, so they are given in full -- this is the same admin
/// reading the same list, in a different surface.
fn subs_json<
	const UIDL: usize,
	UID:	NumIdDat<UIDL>,
	ENC:	Encrypter,
	KH:	Hasher,
	DB:	Database<UIDL, UID, ENC, KH>,
>(
	db:	Option<&(Arc<RwLock<DB>>, UID)>,
	id:	&str,
)
	-> Outcome<HttpMessage>
{
	let db = match db {
		Some(db)	=> db,
		None		=> return Ok(json_error("this site has no database configured")),
	};
	let subs = res!(subscribe::list(db, id));

	let count = |s: subscribe::SubState| subs.iter().filter(|x| x.state == s).count();
	let mut counts = DaticleMap::new();
	counts.insert(dat!("confirmed"),	dat!(count(subscribe::SubState::Confirmed) as u64));
	counts.insert(dat!("pending"),		dat!(count(subscribe::SubState::Pending) as u64));
	counts.insert(dat!("unsubscribed"),	dat!(count(subscribe::SubState::Unsubscribed) as u64));
	counts.insert(dat!("bounced"),		dat!(count(subscribe::SubState::Bounced) as u64));
	counts.insert(dat!("total"),		dat!(subs.len() as u64));

	let mut items = Vec::new();
	for s in &subs {
		let mut m = DaticleMap::new();
		m.insert(dat!("email"),	dat!(s.email.clone()));
		m.insert(dat!("state"),	dat!(s.state.as_str().to_string()));
		m.insert(dat!("since"),	dat!(s.created.clone().unwrap_or_default()));
		items.push(Dat::Map(m));
	}

	let body = create_dat_ordmap(vec![
		(dat!("counts"),	Dat::Map(counts)),
		(dat!("subscribers"),	Dat::List(items)),
	]);
	Ok(json_body(&res!(body.encode_string_with_config(&EncoderConfig::<(), ()>::json(None)))))
}

/// The reports as JSON.
///
/// The three halves the reports page renders -- the list, the sends and the reads -- as data rather
/// than as a table, so an app can draw them in its own idiom. The ceilings the page states in prose
/// are not repeated here: they are properties of the data that the drawing surface must state, and a
/// caller that omits them is showing figures without their caveats. Both surfaces in this tree do.
fn reports_json<
	const UIDL: usize,
	UID:	NumIdDat<UIDL>,
	ENC:	Encrypter,
	KH:	Hasher,
	DB:	Database<UIDL, UID, ENC, KH>,
>(
	db:	Option<&(Arc<RwLock<DB>>, UID)>,
	id:	&str,
)
	-> Outcome<HttpMessage>
{
	let db = match db {
		Some(db)	=> db,
		None		=> return Ok(json_error("this site has no database configured")),
	};

	// The list.
	let subs = res!(subscribe::list(db, id));
	let count = |s: subscribe::SubState| subs.iter().filter(|x| x.state == s).count();
	let mut list = DaticleMap::new();
	list.insert(dat!("confirmed"),		dat!(count(subscribe::SubState::Confirmed) as u64));
	list.insert(dat!("pending"),		dat!(count(subscribe::SubState::Pending) as u64));
	list.insert(dat!("unsubscribed"),	dat!(count(subscribe::SubState::Unsubscribed) as u64));
	list.insert(dat!("bounced"),		dat!(count(subscribe::SubState::Bounced) as u64));
	list.insert(dat!("total"),		dat!(subs.len() as u64));
	list.insert(dat!("by_month"),		months_dat(&by_month(subs.iter().map(|x| x.created.as_deref().unwrap_or("")))));

	// The sends.
	let hist = res!(send::send_history(db));
	let sent: usize = hist.iter().map(|h| h.sent).sum();
	let failed: usize = hist.iter().map(|h| h.failed).sum();
	let mut sends = DaticleMap::new();
	sends.insert(dat!("sends"),	dat!(hist.len() as u64));
	sends.insert(dat!("accepted"),	dat!(sent as u64));
	sends.insert(dat!("failed"),	dat!(failed as u64));
	sends.insert(dat!("suppressed"),	dat!(hist.iter().map(|e| e.suppressed).sum::<usize>() as u64));
	sends.insert(dat!("by_month"),	months_dat(&by_month(hist.iter().map(|e| e.at.as_str()))));

	// The reads.
	let reads = res!(store::reads_all(db, id));
	let recs = res!(store::list_records(db, id));
	let mut rows = Vec::new();
	let mut live_total: u64 = 0;
	let mut read_posts = 0usize;
	for rec in &recs {
		let title = match rec.render() {
			Ok(p)	=> p.title,
			Err(_)	=> rec.slug.clone(),
		};
		let n = reads.get(&rec.slug).copied().unwrap_or(0);
		live_total = live_total.saturating_add(n);
		if n > 0 {
			read_posts += 1;
		}
		let mut m = DaticleMap::new();
		m.insert(dat!("slug"),	dat!(rec.slug.clone()));
		m.insert(dat!("title"),	dat!(title));
		m.insert(dat!("reads"),	dat!(n));
		rows.push((n, Dat::Map(m)));
	}
	rows.sort_by(|a, b| b.0.cmp(&a.0));
	let total: u64 = reads.values().sum();
	let mut reads_m = DaticleMap::new();
	reads_m.insert(dat!("total"),		dat!(total));
	reads_m.insert(dat!("posts"),		dat!(recs.len() as u64));
	reads_m.insert(dat!("posts_read"),	dat!(read_posts as u64));
	// Reads counted against posts that no longer exist. Real, and named apart rather than folded in,
	// so a caller's rows and its total agree.
	reads_m.insert(dat!("deleted"),		dat!(total.saturating_sub(live_total)));
	reads_m.insert(dat!("rows"),		Dat::List(rows.into_iter().map(|(_, d)| d).collect()));

	let body = create_dat_ordmap(vec![
		(dat!("list"),	Dat::Map(list)),
		(dat!("sends"),	Dat::Map(sends)),
		(dat!("reads"),	Dat::Map(reads_m)),
	]);
	Ok(json_body(&res!(body.encode_string_with_config(&EncoderConfig::<(), ()>::json(None)))))
}

/// Counts by month as a list of maps, for the JSON reports.
fn months_dat(months: &[(String, usize)]) -> Dat {
	Dat::List(months.iter().map(|(m, n)| {
		let mut e = DaticleMap::new();
		e.insert(dat!("month"),	dat!(m.clone()));
		e.insert(dat!("n"),	dat!(*n as u64));
		Dat::Map(e)
	}).collect())
}

/// A site's destination settings as JSON, for the settings form.
///
/// Each remote's public fields -- an instance URL, a handle -- and whether its secret is set, and
/// **never the secret**. The secret is write-only: it goes in through [`do_creds`] and does not come
/// back out, so a session that should not have it cannot read it here. `in_config` says the config file
/// also provides the remote, so the form can show it is set even where the store holds nothing.
fn creds_json<
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
	let db = match db {
		Some(d)	=> d,
		None	=> return Ok(json_error("this site has no database configured")),
	};
	debug!("{}: console: GET creds.json", id);
	let stored = res!(send::get_creds(db));

	let mut mm = DaticleMap::new();
	mm.insert(dat!("base_url"),
		dat!(stored.mastodon.as_ref().map(|c| c.base_url.clone()).unwrap_or_default()));
	mm.insert(dat!("secret_set"),	Dat::Bool(stored.mastodon.is_some()));
	mm.insert(dat!("in_config"),	Dat::Bool(cfg.creds.mastodon.is_some()));

	let mut bm = DaticleMap::new();
	bm.insert(dat!("host"),
		dat!(stored.bluesky.as_ref().map(|c| c.host.clone()).unwrap_or_default()));
	bm.insert(dat!("handle"),
		dat!(stored.bluesky.as_ref().map(|c| c.handle.clone()).unwrap_or_default()));
	bm.insert(dat!("secret_set"),	Dat::Bool(stored.bluesky.is_some()));
	bm.insert(dat!("in_config"),	Dat::Bool(cfg.creds.bluesky.is_some()));

	let mut m = DaticleMap::new();
	m.insert(dat!("mastodon"),	Dat::Map(mm));
	m.insert(dat!("bluesky"),	Dat::Map(bm));
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
	mail:		&Option<Arc<MailSender>>,
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
	// before the switch or the switch empties the site. Setting a destination's credentials does not
	// either: a remote is a remote whatever the posts are served from.
	// A subscriber lives in the database whatever the posts are served from, so unsubscribing or erasing
	// one does not wait on the store being the source -- only the writes that touch a post do.
	if cfg.source != Source::Store
		&& request_path != PATH_IMPORT
		&& request_path != PATH_CREDS
		&& request_path != PATH_SUBS_ACTION
	{
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
		PATH_CREDS	=> do_creds(db, body, &admin.username, json, id),
		PATH_NEWSLETTER	=> do_newsletter(cfg, db, mail, body, &admin.username, json, id).await,
		PATH_NEWSLETTER_TEST	=> do_test_send(cfg, db, mail, body, &admin.username, json, id).await,
		PATH_SUBS_ACTION	=> do_subs_action(db, body, &admin.username, json, id),
		PATH_COMMENTS_ACTION	=> do_comment_action(db, body, &admin.username, json, id),
		// Unreachable: `writes` names the same paths.
		_		=> Ok(back(json)),
	}
}

/// Sends a live post to every confirmed subscriber.
///
/// The console side of "own the send": it reads the slug the send form named, checks the post is live,
/// and hands off to [`send::send_newsletter`], which signs and delivers a message per confirmed
/// subscriber straight to their MX. Where mail is not configured on the host, it says so rather than
/// pretending to send. The reason -- how many went, how many failed, or why none could -- rides back in
/// the redirect the way every other console write's does.
async fn do_newsletter<
	const UIDL: usize,
	UID:	NumIdDat<UIDL>,
	ENC:	Encrypter,
	KH:	Hasher,
	DB:	Database<UIDL, UID, ENC, KH>,
>(
	cfg:	&PublishConfig,
	db:	&(Arc<RwLock<DB>>, UID),
	mail:	&Option<Arc<MailSender>>,
	body:	&[u8],
	who:	&str,
	json:	bool,
	id:	&str,
)
	-> Outcome<HttpMessage>
{
	let sender = match mail {
		Some(m)	=> m,
		None	=> return Ok(subs_back_with(
			"email is not set up on this host, so there is nowhere to send from", json)),
	};
	if cfg.base_url.is_empty() {
		return Ok(subs_back_with(
			"this site has no base_url, so a post's online link and the unsubscribe link cannot be built",
			json));
	}
	let slug = super::form_field(body, "slug").unwrap_or_default();
	let slug = slug.trim().to_string();
	if !valid_slug(&slug) {
		return Ok(subs_back_with("that is not a post's name", json));
	}
	let from = cfg.newsletter_from(sender);
	match send::send_newsletter(sender, db, cfg, &from, &slug, id).await {
		Ok(report)	=> {
			info!("{}: console: '{}' sent newsletter '{}' ({} sent, {} failed, {} suppressed)",
				id, who, slug, report.sent, report.failed, report.suppressed);
			// One history entry per real send, stamped with the moment the way the mail's own Date header
			// is. A history that will not write does not fail the send -- the mail has gone -- so it logs
			// and carries on.
			let at = send::iso_now().unwrap_or_default();
			let entry = send::SendEntry::of(&slug, &at, &report);
			if let Err(e) = send::record_send(db, &entry) {
				warn!("{}: console: '{}' sent newsletter '{}' but the history would not record it: {}",
					id, who, slug, e);
			}
			Ok(subs_back_with(
				&fmt!("newsletter '{}' sent to {} subscriber(s), {} failed, {} suppressed",
					slug, report.sent, report.failed, report.suppressed),
				json))
		}
		Err(e)			=> {
			warn!("{}: console: '{}' newsletter '{}' failed: {}", id, who, slug, e);
			Ok(subs_back_with("the newsletter could not be sent; the log says why", json))
		}
	}
}

/// Sends a live post to a single address, to preview what a subscriber gets.
///
/// The console side of the test-send: it reads the slug and the `test_to` address the form named and
/// hands off to [`send::send_test`], which delivers one message and touches no subscriber state and no
/// history. Where mail is not set up on the host, or the site has no origin to build the post's links
/// from, it says so rather than pretend to send.
async fn do_test_send<
	const UIDL: usize,
	UID:	NumIdDat<UIDL>,
	ENC:	Encrypter,
	KH:	Hasher,
	DB:	Database<UIDL, UID, ENC, KH>,
>(
	cfg:	&PublishConfig,
	db:	&(Arc<RwLock<DB>>, UID),
	mail:	&Option<Arc<MailSender>>,
	body:	&[u8],
	who:	&str,
	json:	bool,
	id:	&str,
)
	-> Outcome<HttpMessage>
{
	let sender = match mail {
		Some(m)	=> m,
		None	=> return Ok(subs_back_with(
			"email is not set up on this host, so there is nowhere to send from", json)),
	};
	if cfg.base_url.is_empty() {
		return Ok(subs_back_with(
			"this site has no base_url, so a post's online link cannot be built", json));
	}
	let slug = super::form_field(body, "slug").unwrap_or_default();
	let slug = slug.trim().to_string();
	if !valid_slug(&slug) {
		return Ok(subs_back_with("that is not a post's name", json));
	}
	let to = super::form_field(body, "test_to").unwrap_or_default();
	if to.trim().is_empty() {
		return Ok(subs_back_with("type an address to send the test to", json));
	}
	let from = cfg.newsletter_from(sender);
	match send::send_test(sender, db, cfg, &from, &slug, &to, id).await {
		Ok(())	=> {
			info!("{}: console: '{}' test-sent '{}'", id, who, slug);
			// The address is not echoed back: the reply lands on a page anyone at the console can read, and
			// the operator knows where they sent it.
			Ok(subs_back_with(&fmt!("a test of '{}' was sent", slug), json))
		}
		Err(e)	=> {
			warn!("{}: console: '{}' test-send of '{}' failed: {}", id, who, slug, e);
			Ok(subs_back_with("the test could not be sent; the log says why", json))
		}
	}
}

/// Unsubscribes or erases one subscriber, by the address the admin named.
///
/// Two actions on one endpoint, told apart by the `action` field: `unsubscribe` sets the address
/// [`unsubscribed`](subscribe::SubState::Unsubscribed), keeping the record so a re-subscribe opts in
/// afresh; `delete` erases it outright, a GDPR removal that leaves nothing behind. Both name the target
/// in the `email` field. CSRF is checked upstream, as for every console write.
fn do_subs_action<
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
	let email = super::form_field(body, "email").unwrap_or_default();
	if email.trim().is_empty() {
		return Ok(subs_back_with("no subscriber was named", json));
	}
	let action = super::form_field(body, "action").unwrap_or_default();
	match action.as_str() {
		"unsubscribe"	=> {
			let found = res!(subscribe::unsubscribe_email(db, &email, id));
			info!("{}: console: '{}' unsubscribed a subscriber (found: {})", id, who, found);
			Ok(subs_back_with(
				if found { "the subscriber was unsubscribed" } else { "no such subscriber" }, json))
		}
		"delete"	=> {
			let existed = res!(subscribe::remove(db, &email, id));
			info!("{}: console: '{}' erased a subscriber (existed: {})", id, who, existed);
			Ok(subs_back_with(
				if existed { "the subscriber was erased" } else { "no such subscriber" }, json))
		}
		other	=> Ok(subs_back_with(&fmt!("'{}' is not an action here", other), json)),
	}
}

/// The answer to a newsletter write, carrying the reason back to the subscribers page rather than the
/// posts list, since that is where the operator sent it from.
fn subs_back_with(why: &str, json: bool) -> HttpMessage {
	if json {
		json_error(why)
	} else {
		redirect(&fmt!("{}?said={}", PATH_SUBS, url_encode(why)))
	}
}

/// The answer to a destination write, landing back on the destinations page rather than the post list.
///
/// The app posts the same endpoint with `json` set and is unaffected: it wants a yes it can act on, not
/// a page. Only the form has somewhere to be returned to, and it is the page it was posted from.
fn dests_back(json: bool) -> HttpMessage {
	if json {
		json_body("{\"ok\":true}")
	} else {
		redirect(PATH_DESTS)
	}
}

/// The answer to a destination write that did not go through, carrying the reason back to its own page.
fn dests_back_with(why: &str, json: bool) -> HttpMessage {
	if json {
		json_error(why)
	} else {
		redirect(&fmt!("{}?said={}", PATH_DESTS, url_encode(why)))
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

	// The comma-separated tags field, split and normalised and deduped. An invalid tag is dropped in
	// silence, on the same footing as a slug's small alphabet, so a stray character does not fail the
	// save. Empty or whitespace is no tags.
	let tags = parse_tags(&super::form_field(body, "tags").unwrap_or_default());

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
		tags,
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

/// Sets or clears a remote's credentials, from the settings form.
///
/// Write-only. The secret arrives, is stored, and is never sent back; the log line names the remote and
/// whether it was set or cleared, never the value, on the same footing as a login passphrase. An empty
/// secret field with a secret already stored keeps the stored one -- so a handle can be changed without
/// re-typing a password -- but with none stored the secret is required, since a remote with no secret
/// cannot be reached. The whole set is read, the one remote changed, and the set written back.
fn do_creds<
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
	let dest = super::form_field(body, "dest").unwrap_or_default();
	let clear = super::form_field(body, "clear").as_deref() == Some("1");
	let mut stored = res!(send::get_creds(db));

	match dest.as_str() {
		"mastodon"	=> {
			if clear {
				stored.mastodon = None;
			} else {
				let base_url = super::form_field(body, "base_url").unwrap_or_default().trim().to_string();
				if base_url.is_empty() {
					return Ok(dests_back_with("the Mastodon instance URL is required", json));
				}
				// The token is kept where the form left it blank and one is already stored, so a public
				// field can be edited without re-entering the secret; it is required where none is held.
				let token = match super::form_field(body, "token") {
					Some(t) if !t.trim().is_empty()	=> t,
					_				=> match &stored.mastodon {
						Some(c)	=> c.token.clone(),
						None	=> return Ok(dests_back_with(
							"the Mastodon access token is required", json)),
					},
				};
				stored.mastodon = Some(send::MastodonCreds { base_url, token });
			}
		}
		"bluesky"	=> {
			if clear {
				stored.bluesky = None;
			} else {
				let handle = super::form_field(body, "handle").unwrap_or_default().trim().to_string();
				if handle.is_empty() {
					return Ok(dests_back_with("the Bluesky handle is required", json));
				}
				let host = {
					let h = super::form_field(body, "host").unwrap_or_default().trim().to_string();
					if h.is_empty() { send::BLUESKY_HOST_DEFAULT.to_string() } else { h }
				};
				let app_password = match super::form_field(body, "app_password") {
					Some(p) if !p.trim().is_empty()	=> p,
					_				=> match &stored.bluesky {
						Some(c)	=> c.app_password.clone(),
						None	=> return Ok(dests_back_with(
							"the Bluesky app password is required", json)),
					},
				};
				stored.bluesky = Some(send::BlueskyCreds { host, handle, app_password });
			}
		}
		other	=> return Ok(dests_back_with(
			&fmt!("'{}' is not a destination this site can set", other), json)),
	}

	res!(send::put_creds(db, &stored));
	// No secret in this line, deliberately: it is what a journal keeps, and a journal is read by anyone
	// who can read the host.
	info!("{}: console: '{}' {} {} credentials",
		id, who, if clear { "cleared" } else { "set" }, dest);
	Ok(dests_back(json))
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
	// Importing reads a directory of Markdown on the server into the store. It is the migration
	// path off `source: dir`, and once a site has made that move it is a control that can only
	// overwrite what the site now writes here. So it appears where there is something to import
	// and nowhere else, rather than sitting under every list explaining itself for ever.
	if !dir_has_files(dir) {
		return String::new();
	}
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

/// Whether a directory holds anything an import would read.
///
/// A directory that cannot be read is answered `false` rather than an error: the question is only
/// ever asked to decide whether to offer a control, and a site with no such directory is the
/// ordinary case, not a fault.
fn dir_has_files(dir: &str) -> bool {
	match std::fs::read_dir(dir) {
		Ok(entries)	=> entries.flatten().any(|e| match e.file_type() {
			Ok(t)	=> t.is_file(),
			Err(_)	=> false,
		}),
		Err(_)		=> false,
	}
}

/// `selected`, where it is.
fn selected(yes: bool) -> &'static str {
	if yes { " selected" } else { "" }
}

/// The tags field: a text input that is the source of truth, the current tags as removable chips,
/// and the site's vocabulary as a click-to-add palette.
///
/// The text input holds the tags comma-joined and is what the form submits, so the field saves with
/// no script at all -- a person types `rust, web` and it is stored. The chips and the palette are a
/// progressive enhancement: [`TAG_SCRIPT`] wires the close buttons and the palette to the input and
/// keeps them in step, and does nothing where scripting is off, leaving the plain text field.
///
/// Built as one string rather than through the form's `fmt!`, so the script's braces are data here
/// and never reach a format string.
fn tags_field(tags: &[String], palette: &[String]) -> String {
	let mut s = String::new();
	s.push_str("<label for=\"tags\">Tags</label>\n");
	s.push_str("<input type=\"text\" id=\"tags\" name=\"tags\" value=\"");
	s.push_str(&html_escape(&tags.join(", ")));
	s.push_str("\" placeholder=\"rust, web\">\n");

	// The current tags as chips. Server-rendered so they show without a script; the script replaces
	// them with wired-up ones where it runs.
	s.push_str("<div class=\"tag-chips\" id=\"tag-chips\">");
	for t in tags {
		let e = html_escape(t);
		s.push_str(&fmt!(
			"<span class=\"tag-chip\">{tag}<button type=\"button\" class=\"tag-chip-close\" \
			aria-label=\"Remove {tag}\">×</button></span>",
			tag = e,
		));
	}
	s.push_str("</div>\n");

	// The vocabulary as a palette, where there is one. A click copies a tag into the set.
	if !palette.is_empty() {
		s.push_str("<div class=\"tag-palette\" id=\"tag-palette\">");
		for t in palette {
			s.push_str(&fmt!(
				"<button type=\"button\" class=\"tag-palette-chip\">{tag}</button>",
				tag = html_escape(t),
			));
		}
		s.push_str("</div>\n");
	}

	s.push_str(TAG_SCRIPT);
	s
}

/// The composer's tag script: chips from the input, the input from the chips, and the palette adding
/// to both.
///
/// Reads the comma-joined `tags` input as the source of truth, renders a removable chip per tag,
/// removes on a chip's close, and copies a palette chip in on a click, keeping the input in step so
/// the form submits what the chips show. It touches nothing where it does not run, so the plain text
/// field still saves.
const TAG_SCRIPT: &str = "<script>\n\
(function(){\n\
  var input=document.getElementById('tags');\n\
  var chips=document.getElementById('tag-chips');\n\
  var palette=document.getElementById('tag-palette');\n\
  if(!input||!chips){return;}\n\
  function norm(t){return t.trim().toLowerCase();}\n\
  function list(){return input.value.split(',').map(norm).filter(function(t){return t.length>0;});}\n\
  function uniq(a){var o=[];a.forEach(function(t){if(o.indexOf(t)<0){o.push(t);}});return o;}\n\
  function set(a){input.value=uniq(a).join(', ');render();}\n\
  function add(t){t=norm(t);if(!t){return;}var a=list();if(a.indexOf(t)<0){a.push(t);set(a);}}\n\
  function remove(t){set(list().filter(function(x){return x!==t;}));}\n\
  function render(){\n\
    chips.innerHTML='';\n\
    uniq(list()).forEach(function(t){\n\
      var s=document.createElement('span');s.className='tag-chip';s.textContent=t;\n\
      var b=document.createElement('button');b.type='button';b.className='tag-chip-close';\n\
      b.setAttribute('aria-label','Remove '+t);b.textContent='\\u00d7';\n\
      b.addEventListener('click',function(){remove(t);});\n\
      s.appendChild(b);chips.appendChild(s);\n\
    });\n\
  }\n\
  if(palette){palette.addEventListener('click',function(e){\n\
    var b=e.target.closest('.tag-palette-chip');if(!b){return;}\n\
    e.preventDefault();add(b.textContent);\n\
  });}\n\
  input.addEventListener('change',render);\n\
  render();\n\
})();\n\
</script>\n";

/// One field out of a raw query substring, which has no leading `?`.
fn query_field(query: &str, key: &str) -> Option<String> {
	if query.is_empty() {
		return None;
	}
	for pair in query.split('&') {
		let mut kv = pair.splitn(2, '=');
		let k = ok!(kv.next());
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

	/// The console writes to its mutation paths and reads the rest.
	#[test]
	fn test_writes_are_the_mutations_00() -> Outcome<()> {
		assert!(writes("/manage/save"));
		assert!(writes("/manage/delete"));
		assert!(writes("/manage/import"));
		assert!(writes("/manage/creds"));
		assert!(writes("/manage/newsletter"));
		assert!(writes("/manage/newsletter/test"));
		assert!(writes("/manage/subscribers/action"));
		assert!(!writes("/manage"));
		assert!(!writes("/manage/edit"));
		assert!(!writes("/manage/preview"));
		assert!(!writes("/manage/subscribers"));
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

	/// The tags field emits the source-of-truth input, a removable chip per current tag, and a
	/// palette chip per word of the vocabulary, under the class names the front-ends target.
	#[test]
	fn test_the_tags_field_emits_its_chips_03() -> Outcome<()> {
		let cur = vec![fmt!("rust"), fmt!("web")];
		let pal = vec![fmt!("rust"), fmt!("web"), fmt!("ozone")];
		let s = tags_field(&cur, &pal);
		// The input is the source of truth, comma-joined.
		assert!(s.contains(r#"<input type="text" id="tags" name="tags" value="rust, web""#),
			"got: {}", s);
		// A removable chip per current tag.
		assert!(s.contains(r#"<span class="tag-chip">rust<button type="button" class="tag-chip-close""#),
			"got: {}", s);
		// The palette carries the vocabulary as click-to-add chips.
		assert!(s.contains(r#"<div class="tag-palette" id="tag-palette">"#), "got: {}", s);
		assert!(s.contains(r#"<button type="button" class="tag-palette-chip">ozone</button>"#),
			"got: {}", s);
		// The enhancement script is present.
		assert!(s.contains("<script>"), "no script: {}", s);
		Ok(())
	}

	/// A field with no tags and no vocabulary is still a working input, with an empty chip container
	/// and no palette.
	#[test]
	fn test_the_tags_field_is_empty_gracefully_04() -> Outcome<()> {
		let s = tags_field(&[], &[]);
		assert!(s.contains(r#"value="""#), "got: {}", s);
		assert!(s.contains(r#"<div class="tag-chips" id="tag-chips"></div>"#), "got: {}", s);
		// No palette div (the script still references the id, so the div, not the word, is the check).
		assert!(!s.contains(r#"<div class="tag-palette""#), "an empty vocabulary drew a palette: {}", s);
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

	/// A subscriber in a given state, signed up at a given moment, for the report tests.
	fn sub_at(email: &str, state: subscribe::SubState, created: Option<&str>) -> subscribe::Subscriber {
		subscribe::Subscriber {
			email:		fmt!("{}", email),
			state:		state,
			token:		fmt!("t0000000000000000000000000000000"),
			created:	created.map(|c| fmt!("{}", c)),
		}
	}

	/// A send of a post, for the report tests.
	fn sent_at(slug: &str, at: &str, attempted: usize, sent: usize, failed: usize, suppressed: usize)
		-> send::SendEntry
	{
		send::SendEntry {
			slug:		fmt!("{}", slug),
			at:		fmt!("{}", at),
			attempted:	attempted,
			sent:		sent,
			failed:		failed,
			suppressed:	suppressed,
		}
	}

	/// Nothing out of nothing is not zero per cent, and a share is given to one decimal place.
	#[test]
	fn test_a_share_of_nothing_is_a_dash_05() -> Outcome<()> {
		assert_eq!(pct(0, 0), fmt!("--"));
		assert_eq!(pct(7, 0), fmt!("--"));
		assert_eq!(pct(1, 3), fmt!("33.3%"));
		assert_eq!(pct(3, 3), fmt!("100.0%"));
		assert_eq!(pct(0, 4), fmt!("0.0%"));
		Ok(())
	}

	/// Months group by their first seven characters, newest first, and a stamp with no month in it is
	/// counted under `unknown` rather than dropped.
	#[test]
	fn test_months_group_newest_first_06() -> Outcome<()> {
		let stamps = vec![
			"2026-07-19T08:00:00Z",
			"2026-07-01T00:00:00Z",
			"2026-06-30T23:59:59Z",
			"",
			"nope",
		];
		let months = by_month(stamps.into_iter());
		assert_eq!(months.len(), 3);
		assert_eq!(months[0], (fmt!("2026-07"), 2));
		assert_eq!(months[1], (fmt!("2026-06"), 1));
		assert_eq!(months[2], (fmt!("unknown"), 2));
		// Nothing is lost: the buckets total what went in.
		let total: usize = months.iter().map(|(_, n)| *n).sum();
		assert_eq!(total, 5);
		Ok(())
	}

	/// An empty month run draws no table at all, rather than an empty one.
	#[test]
	fn test_no_months_draw_no_table_07() -> Outcome<()> {
		assert_eq!(month_table("Signed up by month", "Sign-ups", &[]), fmt!(""));
		let one = vec![(fmt!("2026-07"), 3), (fmt!("2026-06"), 1)];
		let html = month_table("Signed up by month", "Sign-ups", &one);
		assert!(html.contains("Signed up by month"));
		// The peak month fills the bar and the lesser one is scaled against it.
		assert!(html.contains("width:100%"));
		assert!(html.contains("width:33%"));
		Ok(())
	}

	/// The list report counts each state, and says so where there is nobody to count.
	#[test]
	fn test_the_list_report_counts_the_states_08() -> Outcome<()> {
		let empty = list_report(&[]);
		assert!(empty.contains("Nobody has subscribed yet"));
		assert!(!empty.contains("mc-stat-n"));

		let subs = vec![
			sub_at("a@example.com", subscribe::SubState::Confirmed, Some("2026-07-19T08:00:00Z")),
			sub_at("b@example.com", subscribe::SubState::Confirmed, Some("2026-06-02T08:00:00Z")),
			sub_at("c@example.com", subscribe::SubState::Pending, Some("2026-07-18T08:00:00Z")),
			sub_at("d@example.com", subscribe::SubState::Unsubscribed, Some("2026-05-01T08:00:00Z")),
			sub_at("e@example.com", subscribe::SubState::Bounced, None),
		];
		let html = list_report(&subs);
		assert!(html.contains("5 addresses on record"));
		// Two of five confirmed, one of five pending, and one of the three who confirmed has left.
		assert!(html.contains("40.0%"));
		assert!(html.contains("20.0%"));
		assert!(html.contains("33.3%"));
		// The undated subscriber still appears, under `unknown`.
		assert!(html.contains("unknown"));
		// The ceiling of the data is stated on the page, not only in the source.
		assert!(html.contains("shares of the list as it stands"));
		Ok(())
	}

	/// The send report totals every send, rolls up by post, and never claims an open or a click.
	#[test]
	fn test_the_send_report_rolls_up_by_post_09() -> Outcome<()> {
		let empty = send_report(&[]);
		assert!(empty.contains("No post has been mailed"));

		let hist = vec![
			sent_at("on-rent", "2026-07-19T08:00:00Z", 10, 8, 1, 1),
			sent_at("on-rent", "2026-07-18T08:00:00Z", 4, 4, 0, 0),
			sent_at("on-time", "2026-06-02T08:00:00Z", 6, 3, 3, 0),
		];
		let html = send_report(&hist);
		// Three sends, twenty attempts, fifteen accepted.
		assert!(html.contains("20 addresses attempted across 3 sends"));
		assert!(html.contains("75.0%"));
		// The two sends of one post are one row carrying both.
		assert!(html.contains("on-rent"));
		assert!(html.contains("on-time"));
		assert!(html.contains("<td>14</td>"));
		// The privacy floor is stated where an operator would look for an open rate.
		assert!(html.contains("deliberately not"));
		Ok(())
	}

	/// The reports page is a read: it is not a write, and not a POST.
	#[test]
	fn test_the_reports_page_is_a_read_10() -> Outcome<()> {
		assert!(!writes(PATH_REPORTS));
		assert!(!posts(PATH_REPORTS));
		Ok(())
	}

	/// The destinations page is a read; the credentials endpoint it posts to is the write.
	#[test]
	fn test_the_destinations_page_is_a_read_11() -> Outcome<()> {
		assert!(!writes(PATH_DESTS));
		assert!(!posts(PATH_DESTS));
		assert!(writes(PATH_CREDS));
		Ok(())
	}

	/// A panel names the remote it sets, carries the token, and offers a clear only where something
	/// is stored to clear.
	#[test]
	fn test_a_destination_panel_offers_a_clear_only_when_set_12() -> Outcome<()> {
		let unset = dest_panel("Mastodon", "mastodon", "tok", false, false, "");
		assert!(unset.contains("name=\"dest\" value=\"mastodon\""));
		assert!(unset.contains("name=\"csrf\" value=\"tok\""));
		assert!(unset.contains("Not set."));
		// Nothing is held, so there is nothing to clear and no button to do it.
		assert!(!unset.contains("value=\"1\""));
		assert!(!unset.contains("mc-btn-danger"));

		let set = dest_panel("Bluesky", "bluesky", "tok", true, false, "");
		assert!(set.contains("Set."));
		assert!(set.contains("name=\"clear\" value=\"1\""));
		assert!(set.contains("mc-btn-danger"));
		Ok(())
	}

	/// An unread site says so, and says what a read is rather than showing an empty table.
	#[test]
	fn test_the_reads_report_says_when_nothing_is_read_14() -> Outcome<()> {
		let s = reads_report(&BTreeMap::new(), &[]);
		assert!(s.contains("Nothing has been read yet"));
		assert!(!s.contains("<table"));
		Ok(())
	}

	/// Posts are listed most-read first, an unread post is shown at nought rather than dropped, and
	/// the page states the ceiling on what a tally means.
	#[test]
	fn test_the_reads_report_ranks_by_reads_15() -> Outcome<()> {
		let mut recs = Vec::new();
		for slug in ["quiet", "popular", "middling"] {
			let mut r = Record::default();
			r.slug = fmt!("{}", slug);
			r.source = fmt!("# {}\n\nprose.\n", slug);
			recs.push(r);
		}
		let mut reads = BTreeMap::new();
		reads.insert(fmt!("popular"), 90u64);
		reads.insert(fmt!("middling"), 10u64);

		let s = reads_report(&reads, &recs);
		let at = |n: &str| s.find(n).unwrap_or(usize::MAX);
		// Most-read first, and the unread post is present rather than omitted.
		assert!(at("popular") < at("middling"), "the most-read post comes first");
		assert!(at("middling") < at("quiet"), "an unread post sorts last, and is still shown");
		assert!(s.contains("100"), "the whole of the reads belongs to the two that were read");
		// One of three posts is unread, so two have been read.
		assert!(s.contains("2/3"));
		assert!(s.contains("a reading, not a reader"));
		Ok(())
	}

	/// A tally whose post is gone is counted in the total and named once, not lost and not listed.
	#[test]
	fn test_the_reads_report_accounts_for_a_deleted_post_16() -> Outcome<()> {
		let mut rec = Record::default();
		rec.slug = fmt!("here");
		rec.source = fmt!("# here\n\nprose.\n");
		let mut reads = BTreeMap::new();
		reads.insert(fmt!("here"), 4u64);
		reads.insert(fmt!("deleted-long-ago"), 3u64);

		let s = reads_report(&reads, &[rec]);
		assert!(s.contains("3 reads were counted against posts that have since been deleted"));
		// Named as a total, not listed as a row anybody could act on.
		assert!(!s.contains("deleted-long-ago"));
		Ok(())
	}

	/// A remote the config file provides is named as such, so a site keyed from `{env:}` or `{file:}`
	/// is not told its destination is unset.
	#[test]
	fn test_a_destination_panel_names_a_config_credential_13() -> Outcome<()> {
		assert!(dest_panel("Mastodon", "mastodon", "t", false, true, "")
			.contains("Set in the configuration file, not here."));
		assert!(dest_panel("Mastodon", "mastodon", "t", true, true, "")
			.contains("Set here, and also in the configuration file."));
		Ok(())
	}
}



// ┌───────────────────────────────────────────────────────────────────────────┐
// │ COMMENTS                                                                  │
// └───────────────────────────────────────────────────────────────────────────┘

/// The moderation queue.
///
/// What is waiting first, because that is the reason to open this page. Everything else is reachable
/// by the filter, since a decision already made is worth being able to revisit -- especially a wrong
/// one, which is the whole reason spam is kept rather than dropped.
fn comments_page<
	const UIDL: usize,
	UID:	NumIdDat<UIDL>,
	ENC:	Encrypter,
	KH:	Hasher,
	DB:	Database<UIDL, UID, ENC, KH>,
>(
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
	body.push_str("<h1>Comments</h1>\n");

	if let Some(said) = query_field(query, "said") {
		body.push_str(&notice(&html_escape(&said)));
	}

	let db = match db {
		Some(db)	=> db,
		None		=> {
			body.push_str(&notice(
				"This site keeps its comments in its database, and has no database configured. Set \
				<code>db_dir_rel</code> on the vhost.",
			));
			return Ok(page(theme, admin, "Comments", &body));
		}
	};

	let all = match comment::queue(db, None, id) {
		Ok(v)	=> v,
		Err(e)	=> {
			error!(e, "{}: console: cannot read the comment queue", id);
			body.push_str(&notice("The comments could not be read. The log says why."));
			return Ok(page(theme, admin, "Comments", &body));
		}
	};

	let count = |s: comment::CommentState| all.iter().filter(|c| c.state == s).count();
	let waiting = count(comment::CommentState::Pending);
	body.push_str(&fmt!(
		"<p class=\"mc-muted\">{} waiting &middot; {} published &middot; {} spam &middot; {} removed</p>\n",
		waiting,
		count(comment::CommentState::Approved),
		count(comment::CommentState::Spam),
		count(comment::CommentState::Removed),
	));

	// Waiting is the default view, because it is the only one that needs anybody.
	let want = query_field(query, "state").unwrap_or_else(|| fmt!("pending"));
	body.push_str(&comments_filter(&want, all.len()));

	let shown: Vec<&comment::Comment> = all.iter()
		.filter(|c| want == "any" || c.state.as_str() == want)
		.collect();

	if shown.is_empty() {
		body.push_str(&notice(if want == "pending" {
			"Nothing is waiting. Comments appear here when somebody who is not yet known writes one."
		} else {
			"No comment is in that state."
		}));
		return Ok(page(theme, admin, "Comments", &body));
	}

	for c in &shown {
		body.push_str(&comment_card(c, csrf));
	}

	Ok(page(theme, admin, "Comments", &body))
}

/// One comment in the queue, with what can be done to it.
///
/// The prose is shown **rendered**, through the same policy a reader's page uses, because a decision
/// about what to publish should be made looking at what would be published. The reason it is here is
/// shown too: a moderator deciding blind is a moderator guessing.
fn comment_card(c: &comment::Comment, csrf: &str) -> String {
	let mut s = String::new();
	s.push_str("<div class=\"mc-comment\">\n");

	s.push_str(&fmt!(
		"<div class=\"mc-comment-by\"><strong>{who}</strong> on <a href=\"{post}\">{slug}</a> \
		<span class=\"mc-muted\">{when}</span> {tag}</div>\n",
		who	= html_escape(c.author.display_name()),
		post	= html_escape(&fmt!("{}?slug={}", PATH_PREVIEW, c.slug)),
		slug	= html_escape(&c.slug),
		when	= html_escape(&c.created[..10.min(c.created.len())]),
		tag	= state_tag(c.state),
	));

	if let Some(r) = &c.reason {
		s.push_str(&fmt!("<p class=\"mc-muted mc-comment-why\">{}</p>\n", html_escape(r)));
	}

	s.push_str("<div class=\"mc-comment-body mc-prose\">");
	match c.render() {
		Ok(html)	=> s.push_str(&html),
		Err(_)		=> s.push_str(&fmt!("<p>{}</p>", html_escape(&c.body))),
	}
	s.push_str("</div>\n");

	s.push_str("<div class=\"mc-comment-acts\">\n");
	for (action, label, class, confirm) in [
		("approve",	"Approve",	"mc-btn mc-btn-quiet",	""),
		("spam",	"Spam",		"mc-btn mc-btn-quiet",	""),
		("remove",	"Remove",	"mc-btn mc-btn-quiet",	""),
		("erase",	"Erase",	"mc-btn mc-btn-danger",
			"Erase this comment entirely? This cannot be undone."),
	] {
		s.push_str(&fmt!(
			"<form method=\"POST\" action=\"{path}\" class=\"mc-inline\"{on}>\
			<input type=\"hidden\" name=\"csrf\" value=\"{csrf}\">\
			<input type=\"hidden\" name=\"slug\" value=\"{slug}\">\
			<input type=\"hidden\" name=\"id\" value=\"{id}\">\
			<input type=\"hidden\" name=\"action\" value=\"{action}\">\
			<button type=\"submit\" class=\"{class}\">{label}</button></form>\n",
			path	= PATH_COMMENTS_ACTION,
			on	= if confirm.is_empty() { String::new() }
					else { fmt!(" onsubmit=\"return confirm('{}')\"", confirm) },
			csrf	= html_escape(csrf),
			slug	= html_escape(&c.slug),
			id	= html_escape(&c.id),
			action	= action,
			class	= class,
			label	= label,
		));
	}
	// Blocking needs somebody to block: an anonymous comment has no handle to attach it to.
	if c.author.handle().is_some() {
		s.push_str(&fmt!(
			"<form method=\"POST\" action=\"{path}\" class=\"mc-inline\" \
			onsubmit=\"return confirm('Block this commenter? Their comments will go straight to spam.')\">\
			<input type=\"hidden\" name=\"csrf\" value=\"{csrf}\">\
			<input type=\"hidden\" name=\"slug\" value=\"{slug}\">\
			<input type=\"hidden\" name=\"id\" value=\"{id}\">\
			<input type=\"hidden\" name=\"action\" value=\"block\">\
			<button type=\"submit\" class=\"mc-btn mc-btn-danger\">Block</button></form>\n",
			path	= PATH_COMMENTS_ACTION,
			csrf	= html_escape(csrf),
			slug	= html_escape(&c.slug),
			id	= html_escape(&c.id),
		));
	}
	s.push_str("</div>\n</div>\n");
	s
}

/// The tag a comment's state wears in the queue.
fn state_tag(state: comment::CommentState) -> String {
	let (cls, word) = match state {
		comment::CommentState::Pending	=> ("mc-tag",		"waiting"),
		comment::CommentState::Approved	=> ("mc-tag mc-tag-live",	"published"),
		comment::CommentState::Spam	=> ("mc-tag mc-tag-err",	"spam"),
		comment::CommentState::Removed	=> ("mc-tag",		"removed"),
	};
	fmt!("<span class=\"{}\">{}</span>", cls, word)
}

/// The state filter over the queue.
fn comments_filter(want: &str, total: usize) -> String {
	fmt!(
		"<form class=\"mc-filter mc-form\" method=\"GET\" action=\"{path}\">\n\
		<div class=\"mc-f-sel\"><label for=\"state\">Showing</label>\
		<select id=\"state\" name=\"state\">\
		<option value=\"pending\"{p}>Waiting</option>\
		<option value=\"approved\"{a}>Published</option>\
		<option value=\"spam\"{s}>Spam</option>\
		<option value=\"removed\"{r}>Removed</option>\
		<option value=\"any\"{n}>Everything</option>\
		</select></div>\n\
		<button type=\"submit\" class=\"mc-btn mc-btn-quiet\">Filter</button>\n\
		<span class=\"mc-muted\" style=\"margin:0 0 0 auto\">{total} in all</span>\n\
		</form>\n",
		path	= PATH_COMMENTS,
		p	= selected(want == "pending"),
		a	= selected(want == "approved"),
		s	= selected(want == "spam"),
		r	= selected(want == "removed"),
		n	= selected(want == "any"),
		total	= total,
	)
}

/// The queue as JSON, for an app that draws it itself.
fn comments_json<
	const UIDL: usize,
	UID:	NumIdDat<UIDL>,
	ENC:	Encrypter,
	KH:	Hasher,
	DB:	Database<UIDL, UID, ENC, KH>,
>(
	db:	Option<&(Arc<RwLock<DB>>, UID)>,
	query:	&str,
	id:	&str,
)
	-> Outcome<HttpMessage>
{
	let db = match db {
		Some(db)	=> db,
		None		=> return Ok(json_error("this site has no database configured")),
	};
	let want = query_field(query, "state").unwrap_or_else(|| fmt!("pending"));
	let all = res!(comment::queue(db, None, id));

	let count = |s: comment::CommentState| all.iter().filter(|c| c.state == s).count();
	let mut counts = DaticleMap::new();
	counts.insert(dat!("pending"),	dat!(count(comment::CommentState::Pending) as u64));
	counts.insert(dat!("approved"),	dat!(count(comment::CommentState::Approved) as u64));
	counts.insert(dat!("spam"),	dat!(count(comment::CommentState::Spam) as u64));
	counts.insert(dat!("removed"),	dat!(count(comment::CommentState::Removed) as u64));

	let mut items = Vec::new();
	for c in all.iter().filter(|c| want == "any" || c.state.as_str() == want) {
		let mut m = DaticleMap::new();
		m.insert(dat!("id"),		dat!(c.id.clone()));
		m.insert(dat!("slug"),		dat!(c.slug.clone()));
		m.insert(dat!("who"),		dat!(c.author.display_name().to_string()));
		m.insert(dat!("when"),		dat!(c.created.clone()));
		m.insert(dat!("state"),		dat!(c.state.as_str().to_string()));
		m.insert(dat!("body"),		dat!(c.body.clone()));
		m.insert(dat!("html"),		dat!(c.render().unwrap_or_default()));
		m.insert(dat!("blockable"),	Dat::Bool(c.author.handle().is_some()));
		// The reason a moderator gave, which is for the moderator. **Never the address**: it is not
		// in this map and must not be added to it.
		if let Some(r) = &c.reason {
			m.insert(dat!("reason"), dat!(r.clone()));
		}
		items.push(Dat::Map(m));
	}

	let body = create_dat_ordmap(vec![
		(dat!("counts"),	Dat::Map(counts)),
		(dat!("comments"),	Dat::List(items)),
	]);
	Ok(json_body(&res!(body.encode_string_with_config(&EncoderConfig::<(), ()>::json(None)))))
}

/// Approves, bins, removes, erases a comment, or blocks its author.
fn do_comment_action<
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
	let slug = super::form_field(body, "slug").unwrap_or_default();
	let cid = super::form_field(body, "id").unwrap_or_default();
	let action = super::form_field(body, "action").unwrap_or_default();
	if slug.is_empty() || cid.is_empty() {
		return Ok(comments_back_with("no comment was named", json));
	}

	let done = match action.as_str() {
		"approve"	=> res!(comment::set_state(
			db, &slug, &cid, comment::CommentState::Approved, None)),
		"spam"		=> res!(comment::set_state(
			db, &slug, &cid, comment::CommentState::Spam, Some(fmt!("marked by {}", who)))),
		"remove"	=> res!(comment::set_state(
			db, &slug, &cid, comment::CommentState::Removed, Some(fmt!("taken down by {}", who)))),
		"erase"		=> res!(comment::erase(db, &slug, &cid)),
		"block"		=> {
			// Blocking bins what is in hand as well as what comes next: leaving this one published
			// while blocking its author would be a decision that half applied.
			let c = match res!(comment::get(db, &slug, &cid)) {
				Some(c)	=> c,
				None	=> return Ok(comments_back_with("that comment is not there", json)),
			};
			match c.author.handle() {
				Some(h)	=> {
					res!(comment::set_blocked(db, &h, true, &c.created));
					res!(comment::set_state(db, &slug, &cid,
						comment::CommentState::Spam, Some(fmt!("blocked by {}", who))))
				}
				None	=> return Ok(comments_back_with(
					"that commenter gave nothing to recognise them by, so they cannot be blocked",
					json)),
			}
		}
		other		=> return Ok(comments_back_with(
			&fmt!("'{}' is not an action here", other), json)),
	};

	if !done {
		return Ok(comments_back_with("that comment is not there", json));
	}
	info!("{}: console: '{}' {} comment '{}/{}'", id, who, action, slug, cid);
	Ok(comments_back(json))
}

/// The answer to a moderation write, landing back on the queue.
fn comments_back(json: bool) -> HttpMessage {
	if json {
		json_body("{\"ok\":true}")
	} else {
		redirect(PATH_COMMENTS)
	}
}

/// The answer to a moderation write that did not go through.
fn comments_back_with(why: &str, json: bool) -> HttpMessage {
	if json {
		json_error(why)
	} else {
		redirect(&fmt!("{}?said={}", PATH_COMMENTS, url_encode(why)))
	}
}
