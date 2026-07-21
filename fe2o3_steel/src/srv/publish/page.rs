//! The posts as pages: a URL each, HTML in the first response, and the tags a card is built from.
//!
//! This is what publishing means here. A reader arrives at a post's own URL and the prose is in the
//! response that answers it -- no script has to run, nothing has to be fetched, and a crawler, a
//! reader-mode, a feed reader and a chat window that unfurls a link all see the same thing a person
//! does.
//!
//! # A page names its own look and holds none of it
//!
//! The markup here is structural: an article, a heading, a date, a navigation. Every rule about what
//! those look like comes from the stylesheets the site named in its config. A server that shipped a
//! font would be deciding something that is not its to decide, and a site that could not restyle its
//! own prose would not really own it.

use crate::srv::publish::{
	Author,
	Post,
	PublishConfig,
	comment::{
		DEPTH_MAX,
		POW_BITS,
		Thread,
	},
	date_text,
	read_mins,
};

#[cfg(test)]
use crate::srv::publish::Source;

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_net::http::{
	fields::{
		HeaderFields,
		HeaderFieldValue,
		HeaderName,
	},
	msg::HttpMessage,
	status::HttpStatus,
};
use oxedyne_fe2o3_text::doc::html::{
	escape_attr,
	escape_text,
};


/// Serves a request that belongs to the published prose.
///
/// The caller has already established that the path is this module's, so anything unrecognised under
/// the prefix is a post that does not exist.
pub fn handle_get(
	cfg:		&PublishConfig,
	posts:		&[Post],
	authors:	&[Author],
	path:		&str,
	query:		&str,
	comments:	Option<&CommentsView>,
	id:		&str,
)
	-> Outcome<HttpMessage>
{
	if path == cfg.path {
		return index(cfg, posts, authors, query, id);
	}
	if path == cfg.feed_path() {
		return super::feed::serve(cfg, posts, id);
	}
	if path == cfg.json_path() {
		return super::json::serve(cfg, posts, authors, id);
	}
	if path == cfg.comment_js_path() {
		return Ok(comment_js());
	}
	if path == cfg.filter_js_path() {
		return Ok(filter_js());
	}
	// Everything else under the prefix names a post. The slug is what a reader put in a URL, so it is
	// checked before it is used: a name is letters, digits, a dash or an underscore.
	let slug = &path[cfg.path.len() + 1..];
	if !is_slug(slug) {
		info!("{}: publish: '{}' is not a name a post may wear", id, slug);
		return Ok(not_found(cfg));
	}
	match posts.iter().find(|p| p.slug == slug) {
		Some(post)	=> post_page(cfg, post,
			authors.iter().find(|a| a.username == post.author), comments),
		None		=> {
			info!("{}: publish: no post '{}'", id, slug);
			Ok(not_found(cfg))
		}
	}
}

/// The post a request path names, where it names one that exists.
///
/// The renderers take a slice of posts and touch no database, so the read tally -- which is a write --
/// cannot be kept here. This is the half of that decision which is pure: given the same path the
/// renderer was given, it says whether a post was served and which. The caller, which still holds the
/// database, does the counting.
///
/// The index, the feed and the JSON are not posts and answer `None`, so a reader browsing the index
/// does not add to the tally of everything on it.
pub fn served_post<'a>(cfg: &PublishConfig, posts: &'a [Post], path: &str) -> Option<&'a Post> {
	if path == cfg.path || path == cfg.feed_path() || path == cfg.json_path() {
		return None;
	}
	// The same slicing the renderer does, and the same guard: a path that is not under the prefix
	// with room for a name is not a post.
	if path.len() < cfg.path.len() + 2 {
		return None;
	}
	let slug = &path[cfg.path.len() + 1..];
	if !is_slug(slug) {
		return None;
	}
	posts.iter().find(|p| p.slug == slug)
}

/// Whether a string is a name a post may wear.
fn is_slug(s: &str) -> bool {
	!s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// The index: every post, newest first, above a filter that narrows them in the reader's browser.
///
/// The whole list is rendered, each item carrying its author, tags, categories and reading time as
/// data attributes; the filter shows and hides items against those. So a reader with no JavaScript
/// gets every post, and a reader with it gets the filter, over the same markup -- the filter is an
/// enhancement of the list, never the thing that fetches it.
///
/// `authors` are the distinct authors the posts name, resolved to a face, drawn as the filter's
/// author row. A `?tag=` in the query is read by the script, not here, so a tag link lands on the
/// index with that tag alone selected; without the script the whole list stands, tag and all.
fn index(
	cfg:		&PublishConfig,
	posts:		&[Post],
	authors:	&[Author],
	_query:		&str,
	id:		&str,
)
	-> Outcome<HttpMessage>
{
	let mut body = String::new();
	body.push_str("<header class=\"aside-index-head\"><h1>");
	escape_text(&mut body, &cfg.title);
	body.push_str("</h1></header>\n");

	// What the site is about, in the words of whoever writes it, above everything else on the page.
	body.push_str(&about_block(authors));

	// The filter: the reader's own instrument for narrowing the list. Rendered before the list so it
	// is the first thing to hand, and wired by the served script; without the script it does nothing
	// and the whole list stands, which is the point of building it as an enhancement.
	body.push_str(&filter_shell(cfg, posts, authors));

	body.push_str("<ul class=\"aside-index\" id=\"aside-index-list\">\n");
	for p in posts {
		// The item carries what the filter matches on, so the script reads the list rather than a second
		// copy of it: the author's username, the tags and categories space-joined, the reading time, and
		// a lower-cased haystack of the title and opening for the search box.
		body.push_str("<li class=\"aside-index-item\" data-author=\"");
		escape_attr(&mut body, &p.author);
		body.push_str("\" data-tags=\"");
		// Tags are `[a-z0-9-]`, so a space joins them safely. Categories are free config strings that
		// may hold a space, so they are joined on a comma the config forbids inside a category, and the
		// script splits on the same.
		escape_attr(&mut body, &p.tags.join(" "));
		body.push_str("\" data-categories=\"");
		escape_attr(&mut body, &p.categories.join(","));
		body.push_str("\" data-read-mins=\"");
		body.push_str(&fmt!("{}", read_mins(p.words)));
		body.push_str("\" data-search=\"");
		escape_attr(&mut body, &fmt!("{} {}", p.title, p.excerpt).to_lowercase());
		body.push_str("\">");
		if let Some(d) = &p.date {
			// The attribute is the stored ISO form and the text is the readable one, which is what
			// `<time>` has two of them for: a post dated to the minute would otherwise show a reader
			// the `T` in the middle of its own date.
			body.push_str("<div class=\"aside-date\"><time datetime=\"");
			escape_attr(&mut body, d);
			body.push_str("\">");
			escape_text(&mut body, &date_text(d));
			body.push_str("</time></div>");
		}
		// Who wrote it, where more than one person writes here. On a blog of one it would be the same
		// name under every title, which tells a reader choosing between them nothing.
		if authors.len() > 1 {
			if let Some(a) = authors.iter().find(|a| a.username == p.author) {
				body.push_str("<div class=\"aside-byline\">");
				body.push_str(&author_face(a));
				body.push_str("<span class=\"aside-byline-name\">");
				escape_text(&mut body, &a.name);
				body.push_str("</span></div>");
			}
		}
		body.push_str("<h2><a href=\"");
		escape_attr(&mut body, &cfg.path_of(&p.slug));
		body.push_str("\">");
		escape_text(&mut body, &p.title);
		body.push_str("</a></h2>");
		if !p.excerpt.is_empty() {
			body.push_str("<p class=\"aside-excerpt\">");
			escape_text(&mut body, &p.excerpt);
			body.push_str("</p>");
		}
		body.push_str(&tags_list(cfg, p));
		body.push_str("</li>\n");
	}
	body.push_str("</ul>\n");

	// Where the filter lands a reader on nothing, the script shows this line; the server shows it only
	// when the site itself is empty. Both say the same thing, so the reader is never left at a blank.
	body.push_str("<p class=\"aside-empty\" id=\"aside-empty\"");
	if !posts.is_empty() {
		body.push_str(" hidden");
	}
	body.push_str(">Nothing here yet.</p>\n");

	// A newsletter sign-up beneath the list, so a reader subscribes in place rather than hunting for
	// it. It posts to the same endpoint as the standalone page; where mail is not configured that
	// endpoint answers "not available", so the form is safe to show unconditionally.
	body.push_str("<section class=\"aside-subscribe-inline\">\n<h2>Subscribe</h2>\n");
	body.push_str("<p>New posts by email. Confirm once, unsubscribe from any message.</p>\n");
	body.push_str("<form class=\"aside-subscribe\" method=\"post\" action=\"");
	escape_attr(&mut body, &cfg.subscribe_path());
	body.push_str("\">\n<input type=\"email\" name=\"email\" id=\"aside-subscribe-email\" \
		placeholder=\"you@example.com\" autocomplete=\"email\" aria-label=\"Email\" required>\n");
	body.push_str("<button type=\"submit\" class=\"aside-subscribe-btn\">Subscribe</button>\n");
	body.push_str("</form>\n</section>\n");

	// The script that wires the filter. Referenced rather than inlined, on the same reasoning as the
	// comment script: a site may run a Content-Security-Policy that forbids inline script, and the
	// filter is an enhancement -- `defer`, since it only reads the list already in the page.
	body.push_str("<script defer src=\"");
	escape_attr(&mut body, &cfg.filter_js_path());
	body.push_str("\"></script>\n");

	info!("{}: publish: index, {} posts", id, posts.len());

	let head = Head {
		title:		cfg.title.clone(),
		description:	String::new(),
		url:		cfg.url_of(&cfg.path),
		kind:		"website",
		date:		None,
	};
	Ok(html_response(HttpStatus::OK, &page(cfg, &head, &body, None)))
}

/// The filter above the index: the controls a reader narrows the list with.
///
/// Rendered whole and static; the served script wires it. Every control starts in the state that
/// shows every post, so the page a reader lands on is the whole list and the filter only ever takes
/// away: no author pressed, every category checked, every tag in the selected box under `Includes`,
/// and the reading-time slider spanning the full range. The vocabulary and the range are read from
/// the posts, so the filter offers exactly what the list holds and nothing it does not.
fn filter_shell(cfg: &PublishConfig, posts: &[Post], authors: &[Author]) -> String {
	// The tag vocabulary: every tag any shown post wears, sorted, deduped. What the two chip boxes are
	// filled from -- the selected box by default, since the default is to hide nothing.
	let mut tags: Vec<&str> = Vec::new();
	for p in posts {
		for t in &p.tags {
			if !tags.iter().any(|x| *x == t.as_str()) {
				tags.push(t.as_str());
			}
		}
	}
	tags.sort_unstable();

	// The reading-time range across the posts, the slider's bounds. A site whose posts run one to nine
	// minutes gets a one-to-nine slider, not a dead one-to-sixty. Equal bounds (one post, or all of a
	// length) leave a slider with nothing to drag, which the script hides.
	let mins: Vec<usize> = posts.iter().map(|p| read_mins(p.words)).collect();
	let rt_lo = mins.iter().copied().min().unwrap_or(1);
	let rt_hi = mins.iter().copied().max().unwrap_or(1);

	let mut s = String::from("<section class=\"aside-filter\" aria-label=\"Filter posts\">\n");

	// The search box, over the title and opening of each post.
	s.push_str("<input type=\"search\" class=\"aside-filter-search\" id=\"aside-filter-search\" \
		placeholder=\"Search posts\" aria-label=\"Search posts\" autocomplete=\"off\">\n");

	// The authors, each a face that toggles the list to that author. None pressed is every author, so
	// the row starts showing all. Only those with a post in the list are offered: `authors` also holds
	// whoever else may write here, for the description above, and a face that narrows the list to
	// nothing is a control that can only disappoint.
	let authors: Vec<&Author> = authors.iter()
		.filter(|a| posts.iter().any(|p| p.author == a.username))
		.collect();
	if !authors.is_empty() {
		s.push_str("<div class=\"aside-filter-authors\" id=\"aside-filter-authors\" \
			aria-label=\"Filter by author\">\n");
		for a in authors {
			s.push_str("<button type=\"button\" class=\"aside-author\" data-author=\"");
			escape_attr(&mut s, &a.username);
			s.push_str("\" title=\"");
			escape_attr(&mut s, &a.name);
			s.push_str("\" aria-pressed=\"false\">");
			s.push_str(&author_face(a));
			s.push_str("<span class=\"aside-author-name\">");
			escape_text(&mut s, &a.name);
			s.push_str("</span></button>\n");
		}
		s.push_str("</div>\n");
	}

	// The tag machinery: the mode, then the two boxes. Shown only where the site has tags at all.
	if !tags.is_empty() {
		// Includes / Only / Excludes, over the selected set. `Includes` is the default, being the one
		// that with every tag selected still shows every tagged post.
		s.push_str("<div class=\"aside-filter-mode\" id=\"aside-filter-mode\" role=\"radiogroup\" \
			aria-label=\"Tag match\">\n");
		for (val, label, on) in [("includes", "Includes", true), ("only", "Only", false),
			("excludes", "Excludes", false)]
		{
			s.push_str("<label class=\"aside-mode\"><input type=\"radio\" name=\"aside-mode\" value=\"");
			s.push_str(val);
			s.push('"');
			if on {
				s.push_str(" checked");
			}
			s.push('>');
			s.push_str(label);
			s.push_str("</label>\n");
		}
		s.push_str("</div>\n");

		// The two boxes. The selected box holds every tag by default, each with a closer that moves it to
		// the source box; the source box starts empty and its chips carry no closer -- a click, or a
		// drag, moves a chip either way. The labels name which is which, since the two look alike.
		s.push_str("<div class=\"aside-filter-tags\">\n");
		s.push_str("<div class=\"aside-tagbox\">\n<span class=\"aside-tagbox-lbl\">Selected</span>\n\
			<div class=\"aside-chips aside-chips-selected\" id=\"aside-tags-selected\" \
			data-box=\"selected\" role=\"list\">\n");
		for t in &tags {
			s.push_str("<button type=\"button\" class=\"aside-chip\" draggable=\"true\" data-tag=\"");
			escape_attr(&mut s, t);
			s.push_str("\" role=\"listitem\">");
			escape_text(&mut s, t);
			s.push_str(" <span class=\"aside-chip-x\" aria-hidden=\"true\">\u{00d7}</span></button>\n");
		}
		s.push_str("</div>\n</div>\n");
		s.push_str("<div class=\"aside-tagbox\">\n<span class=\"aside-tagbox-lbl\">Available</span>\n\
			<div class=\"aside-chips aside-chips-source\" id=\"aside-tags-source\" \
			data-box=\"source\" role=\"list\"></div>\n</div>\n");
		s.push_str("</div>\n");
	}

	// The categories, a checkbox each, every one checked so the default hides nothing. A post with no
	// category the script always passes, on the same footing as an untagged post under the tag filter.
	if !cfg.categories.is_empty() {
		s.push_str("<div class=\"aside-filter-cats\" id=\"aside-filter-cats\" aria-label=\"Categories\">\n");
		for c in &cfg.categories {
			s.push_str("<label class=\"aside-cat\"><input type=\"checkbox\" checked value=\"");
			escape_attr(&mut s, c);
			s.push_str("\">");
			escape_text(&mut s, c);
			s.push_str("</label>\n");
		}
		s.push_str("</div>\n");
	}

	// The reading-time slider: two thumbs over the range the posts span, so a reader keeps the short
	// ones, the long ones, or a band between. Hidden by the script where every post reads alike, since
	// a slider that cannot move is furniture. The read-out beside it the script keeps current.
	if rt_hi > rt_lo {
		s.push_str(&fmt!(
			"<div class=\"aside-filter-time\" id=\"aside-filter-time\" data-lo=\"{lo}\" data-hi=\"{hi}\">\n\
			<span class=\"aside-time-lbl\">Reading time</span>\n\
			<div class=\"aside-time-track\">\n\
			<input type=\"range\" class=\"aside-time-min\" id=\"aside-time-min\" \
				min=\"{lo}\" max=\"{hi}\" value=\"{lo}\" step=\"1\" aria-label=\"Least minutes\">\n\
			<input type=\"range\" class=\"aside-time-max\" id=\"aside-time-max\" \
				min=\"{lo}\" max=\"{hi}\" value=\"{hi}\" step=\"1\" aria-label=\"Most minutes\">\n\
			</div>\n\
			<span class=\"aside-time-out\" id=\"aside-time-out\">{lo}\u{2013}{hi} min</span>\n\
			</div>\n",
			lo = rt_lo, hi = rt_hi));
	}

	s.push_str("</section>\n");
	s
}

/// An author's picture, or the initial drawn in its place.
///
/// One definition, so a byline, the note under a post and the filter's author row all draw the same
/// face. An avatar the author uploaded is served by this module; one they gave as a URL is fetched
/// from wherever they said.
fn author_face(a: &Author) -> String {
	let mut s = String::new();
	if a.avatar.is_empty() {
		s.push_str("<span class=\"aside-author-initial\" aria-hidden=\"true\">");
		escape_text(&mut s, &a.initial());
		s.push_str("</span>");
	} else {
		s.push_str("<img class=\"aside-author-pic\" alt=\"\" src=\"");
		escape_attr(&mut s, &a.avatar);
		s.push_str("\">");
	}
	s
}

/// What the site is about, above the posts: each author's own description of what they write.
///
/// The first thing a reader meets, because a stranger landing on a list of titles has no way to tell
/// what the blog is for. Where one person writes the blog, their description *is* the blog's, which is
/// why nothing here is configured separately -- an author who writes what they are about has said what
/// the site is about, and there is no second place for the two to disagree.
///
/// Nothing at all where no author has written one, rather than an empty panel.
fn about_block(authors: &[Author]) -> String {
	if !authors.iter().any(|a| !a.bio.is_empty()) {
		return String::new();
	}
	let mut s = String::from("<section class=\"aside-about\" aria-label=\"About\">\n");
	for a in authors.iter().filter(|a| !a.bio.is_empty()) {
		s.push_str("<div class=\"aside-about-who\">");
		s.push_str(&author_face(a));
		s.push_str("<div class=\"aside-about-body\">");
		// The name is drawn only where more than one person writes here: on a blog of one, the name
		// is on every post already, and a heading repeating it says nothing.
		if authors.len() > 1 {
			s.push_str("<span class=\"aside-about-name\">");
			escape_text(&mut s, &a.name);
			s.push_str("</span>");
		}
		s.push_str("<p class=\"aside-about-bio\">");
		escape_text(&mut s, &a.bio);
		s.push_str("</p></div></div>\n");
	}
	s.push_str("</section>\n");
	s
}

/// A post's tags as a list of links, each narrowing the index to that tag.
///
/// Nothing at all for a post with no tags, so the element is never an empty shell. The link is the
/// index with the tag as a facet; the tag is `[a-z0-9-]`, so it needs no encoding to sit in a query.
fn tags_list(cfg: &PublishConfig, post: &Post) -> String {
	if post.tags.is_empty() {
		return String::new();
	}
	let mut s = String::from("<ul class=\"post-tags\">");
	for t in &post.tags {
		s.push_str("<li><a class=\"tag\" href=\"");
		escape_attr(&mut s, &cfg.path);
		s.push_str("?tag=");
		escape_attr(&mut s, t);
		s.push_str("\">");
		escape_text(&mut s, t);
		s.push_str("</a></li>");
	}
	s.push_str("</ul>");
	s
}

/// The reading time above a post, as its label. The minutes are [`read_mins`], the site's one
/// definition, so this badge and the filter's slider count alike.
fn read_time(words: usize) -> String {
	fmt!("{} min read", read_mins(words))
}

/// One post.
fn post_page(
	cfg:		&PublishConfig,
	post:		&Post,
	author:		Option<&Author>,
	comments:	Option<&CommentsView>,
)
	-> Outcome<HttpMessage>
{
	let mut body = String::new();
	body.push_str("<article class=\"aside\">\n");
	// Who wrote it, before the prose: on a blog more than one person writes, the byline is part of
	// reading the piece rather than a credit to find afterwards.
	if let Some(a) = author {
		body.push_str("<div class=\"aside-byline\">");
		body.push_str(&author_face(a));
		body.push_str("<span class=\"aside-byline-name\">");
		escape_text(&mut body, &a.name);
		body.push_str("</span></div>\n");
	}
	// The date, and beside it how long the piece takes to read. A reader deciding whether to start
	// wants both, and wants them before the prose rather than after it.
	if post.date.is_some() || post.words > 0 {
		body.push_str("<div class=\"aside-date\">");
		if let Some(d) = &post.date {
			body.push_str("<time datetime=\"");
			escape_attr(&mut body, d);
			body.push_str("\">");
			escape_text(&mut body, &date_text(d));
			body.push_str("</time>");
		}
		if post.words > 0 {
			body.push_str("<span class=\"aside-read\">");
			escape_text(&mut body, &read_time(post.words));
			body.push_str("</span>");
		}
		body.push_str("</div>\n");
	}
	// The prose was escaped where it was rendered.
	body.push_str(&post.html);
	// The tags, in the article's footer, each a link back to the index narrowed to that tag. Omitted
	// entirely for a post with none.
	body.push_str(&tags_list(cfg, post));
	body.push_str("</article>\n");

	// Who wrote it, in their own words, for the reader who has just finished and wants to know whose
	// piece it was. Only where they have written a description: an empty one draws nothing rather
	// than a box with a name in it.
	if let Some(a) = author.filter(|a| !a.bio.is_empty()) {
		body.push_str("<aside class=\"aside-author-note\">");
		body.push_str(&author_face(a));
		body.push_str("<div class=\"aside-author-note-body\"><span class=\"aside-author-note-name\">");
		escape_text(&mut body, &a.name);
		body.push_str("</span><p>");
		escape_text(&mut body, &a.bio);
		body.push_str("</p></div></aside>\n");
	}

	// Where the post also lives, and where the conversation about it may be. `nofollow`, since these
	// are the site's own syndicated copies and not endorsements to pass rank to, and a new tab, since a
	// reader following one has not finished with the page they are on.
	if !post.also_on.is_empty() {
		body.push_str("<nav class=\"aside-also\"><span class=\"aside-also-lbl\">Also on</span>");
		for (dest, url) in &post.also_on {
			body.push_str(" <a class=\"aside-also-link\" rel=\"nofollow noopener\" target=\"_blank\" href=\"");
			escape_attr(&mut body, url);
			body.push_str("\">");
			escape_text(&mut body, dest.capability().name);
			body.push_str("</a>");
		}
		body.push_str("</nav>\n");
	}

	// The conversation, where the caller read one. A page rendered without it is a page for a site
	// that takes no comments, which is a configuration rather than a failure.
	if let Some(view) = comments {
		body.push_str(&comments_section(cfg, post, view));
	}

	let head = Head {
		title:		post.title.clone(),
		description:	post.excerpt.clone(),
		url:		cfg.url_of(&cfg.path_of(&post.slug)),
		kind:		"article",
		date:		post.date.clone(),
	};
	Ok(html_response(HttpStatus::OK, &page(cfg, &head, &body, Some(post))))
}

/// A post that is not there.
///
/// Served as a page rather than a bare line, because a reader who mistyped a URL, or followed a link
/// to a post that has been taken down, is still a reader and should land somewhere with a way on.
fn not_found(cfg: &PublishConfig) -> HttpMessage {
	let mut body = String::new();
	body.push_str("<article class=\"aside\"><h1>Not here</h1><p>There is no such piece. <a href=\"");
	escape_attr(&mut body, &cfg.path);
	body.push_str("\">");
	escape_text(&mut body, &cfg.title);
	body.push_str("</a> has the rest.</p></article>\n");

	let head = Head {
		title:		fmt!("Not here"),
		description:	String::new(),
		url:		cfg.url_of(&cfg.path),
		kind:		"website",
		date:		None,
	};
	html_response(HttpStatus::NotFound, &page(cfg, &head, &body, None))
}


/// What a page says about itself.
struct Head {
	/// The page's own title, before the site's name is added.
	title:		String,
	/// A sentence standing in for the page, in a card and in search.
	description:	String,
	/// The page's canonical absolute URL.
	url:		String,
	/// `article` for a post, `website` for the index.
	kind:		&'static str,
	/// The date, for a post that has one.
	date:		Option<String>,
}

/// Wraps a body in the document a browser reads.
fn page(cfg: &PublishConfig, head: &Head, body: &str, post: Option<&Post>) -> String {
	let mut s = String::new();
	s.push_str("<!DOCTYPE html>\n<html lang=\"en\">\n<head>\n");
	s.push_str("<meta charset=\"utf-8\">\n");
	s.push_str("<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n");

	// The tab, and the card's fallback title.
	s.push_str("<title>");
	escape_text(&mut s, &head.title);
	if !cfg.site_name.is_empty() && head.title != cfg.site_name {
		s.push_str(" — ");
		escape_text(&mut s, &cfg.site_name);
	}
	s.push_str("</title>\n");

	if !head.description.is_empty() {
		s.push_str("<meta name=\"description\" content=\"");
		escape_attr(&mut s, &head.description);
		s.push_str("\">\n");
	}

	// Canonical, so a post shared with a query string on it is still one post.
	if !cfg.base_url.is_empty() {
		s.push_str("<link rel=\"canonical\" href=\"");
		escape_attr(&mut s, &head.url);
		s.push_str("\">\n");
	}

	s.push_str("<link rel=\"alternate\" type=\"application/atom+xml\" title=\"");
	escape_attr(&mut s, &cfg.title);
	s.push_str("\" href=\"");
	escape_attr(&mut s, &cfg.feed_path());
	s.push_str("\">\n");

	// The card a link makes when it is pasted somewhere.
	meta_prop(&mut s, "og:type", head.kind);
	meta_prop(&mut s, "og:title", &head.title);
	if !head.description.is_empty() {
		meta_prop(&mut s, "og:description", &head.description);
	}
	if !cfg.base_url.is_empty() {
		meta_prop(&mut s, "og:url", &head.url);
	}
	if !cfg.site_name.is_empty() {
		meta_prop(&mut s, "og:site_name", &cfg.site_name);
	}
	if let Some(d) = &head.date {
		meta_prop(&mut s, "article:published_time", d);
	}
	// No image, so a card with no picture is the summary rather than a large empty frame.
	meta_name(&mut s, "twitter:card", "summary");

	for href in &cfg.css {
		s.push_str("<link rel=\"stylesheet\" href=\"");
		escape_attr(&mut s, href);
		s.push_str("\">\n");
	}

	if let Some(post) = post {
		s.push_str(&json_ld(cfg, post));
	}

	s.push_str("</head>\n<body class=\"aside-body\">\n<main class=\"aside-page\">\n");
	s.push_str("<nav class=\"aside-nav\"><a href=\"");
	escape_attr(&mut s, &cfg.path);
	s.push_str("\">");
	escape_text(&mut s, &cfg.title);
	s.push_str("</a></nav>\n");
	s.push_str(body);
	s.push_str("</main>\n</body>\n</html>\n");
	s
}

/// What a search engine reads instead of guessing.
fn json_ld(cfg: &PublishConfig, post: &Post) -> String {
	// Built by hand rather than through an encoder, because the values are escaped for a script
	// element rather than for JSON alone: a title containing `</script>` would otherwise end the
	// block and everything after it would be markup.
	let mut s = String::new();
	s.push_str("<script type=\"application/ld+json\">\n{\n");
	s.push_str("  \"@context\": \"https://schema.org\",\n  \"@type\": \"BlogPosting\",\n");
	s.push_str("  \"headline\": ");
	json_str(&mut s, &post.title);
	s.push_str(",\n");
	if !post.excerpt.is_empty() {
		s.push_str("  \"description\": ");
		json_str(&mut s, &post.excerpt);
		s.push_str(",\n");
	}
	if let Some(d) = &post.date {
		s.push_str("  \"datePublished\": ");
		json_str(&mut s, d);
		s.push_str(",\n");
	}
	if !cfg.base_url.is_empty() {
		s.push_str("  \"url\": ");
		json_str(&mut s, &cfg.url_of(&cfg.path_of(&post.slug)));
		s.push_str(",\n");
	}
	s.push_str("  \"mainEntityOfPage\": true\n}\n</script>\n");
	s
}

/// Writes a JSON string that is also safe inside a `script` element.
///
/// `<` is escaped as `<`, which JSON reads as `<` and an HTML parser cannot read as the start of
/// a tag. That is what stops a title containing `</script>` from closing the block it sits in.
fn json_str(out: &mut String, s: &str) {
	out.push('"');
	for c in s.chars() {
		match c {
			'"'		=> out.push_str("\\\""),
			'\\'		=> out.push_str("\\\\"),
			'\n'		=> out.push_str("\\n"),
			'\r'		=> out.push_str("\\r"),
			'\t'		=> out.push_str("\\t"),
			'<'		=> out.push_str("\\u003c"),
			'>'		=> out.push_str("\\u003e"),
			'&'		=> out.push_str("\\u0026"),
			c if (c as u32) < 0x20 => {
				out.push_str("\\u00");
				let b = c as u8;
				out.push(char::from_digit((b >> 4) as u32, 16).unwrap_or('0'));
				out.push(char::from_digit((b & 0xf) as u32, 16).unwrap_or('0'));
			}
			c		=> out.push(c),
		}
	}
	out.push('"');
}

/// A `<meta property=...>`, as Open Graph wants.
fn meta_prop(out: &mut String, prop: &str, content: &str) {
	out.push_str("<meta property=\"");
	out.push_str(prop);
	out.push_str("\" content=\"");
	escape_attr(out, content);
	out.push_str("\">\n");
}

/// A `<meta name=...>`, as everything else wants.
fn meta_name(out: &mut String, name: &str, content: &str) {
	out.push_str("<meta name=\"");
	out.push_str(name);
	out.push_str("\" content=\"");
	escape_attr(out, content);
	out.push_str("\">\n");
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ THE NEWSLETTER'S PUBLIC PAGES                                             │
// └───────────────────────────────────────────────────────────────────────────┘

/// The themed sign-up form, served at `GET {path}/subscribe`.
///
/// A working, script-free form the site can link to directly, and the shape the site's own inline form
/// should mirror: a `POST` to the same path with one field, `email`. The classes and ids below are the
/// contract the front-end is built against.
pub fn subscribe_form_page(cfg: &PublishConfig) -> HttpMessage {
	let mut body = String::new();
	body.push_str("<article class=\"aside aside-subscribe-page\">\n<h1>Subscribe</h1>\n");
	body.push_str("<p>Get new posts by email. Confirm once, and unsubscribe from any message.</p>\n");
	body.push_str("<form class=\"aside-subscribe\" id=\"aside-subscribe-form\" method=\"post\" action=\"");
	escape_attr(&mut body, &cfg.subscribe_path());
	body.push_str("\">\n<label for=\"aside-subscribe-email\">Email</label>\n");
	body.push_str("<input type=\"email\" name=\"email\" id=\"aside-subscribe-email\" \
		placeholder=\"you@example.com\" autocomplete=\"email\" required>\n");
	body.push_str("<button type=\"submit\" class=\"aside-subscribe-btn\">Subscribe</button>\n");
	body.push_str("</form>\n</article>\n");
	subscribe_page(cfg, "Subscribe", &body, HttpStatus::OK)
}

/// The "check your inbox" answer to a sign-up, served whether the address was new, pending or already
/// confirmed -- so the form is never an oracle for whether an address is on the list.
pub fn subscribe_sent_page(cfg: &PublishConfig) -> HttpMessage {
	let body = subscribe_result(
		"Check your inbox",
		"If that address can receive mail, a confirmation link is on its way. Follow it to start \
		receiving posts. Nothing arrives until you do.",
	);
	subscribe_page(cfg, "Check your inbox", &body, HttpStatus::OK)
}

/// The answer to a confirmation link followed: the address is now on the list.
///
/// The same page whether the link was fresh or followed a second time, so a double-click is not an
/// error to a reader who did nothing wrong.
pub fn subscribe_confirmed_page(cfg: &PublishConfig) -> HttpMessage {
	let body = subscribe_result(
		"You are subscribed",
		"Your subscription is confirmed. New posts will arrive by email, and every one carries a link \
		to unsubscribe.",
	);
	subscribe_page(cfg, "Subscribed", &body, HttpStatus::OK)
}

/// The answer to an unsubscribe link followed: no more mail reaches this address.
pub fn subscribe_unsubscribed_page(cfg: &PublishConfig) -> HttpMessage {
	let body = subscribe_result(
		"Unsubscribed",
		"You will receive no further posts at this address. You are welcome back any time from the \
		subscribe page.",
	);
	subscribe_page(cfg, "Unsubscribed", &body, HttpStatus::OK)
}

/// The answer to a token that names nobody: malformed, already spent by a re-subscribe, or never real.
pub fn subscribe_bad_token_page(cfg: &PublishConfig) -> HttpMessage {
	let body = subscribe_result(
		"This link did not work",
		"That link is not one we recognise -- it may be old, or already used. Try subscribing again \
		if you meant to.",
	);
	subscribe_page(cfg, "Link not recognised", &body, HttpStatus::NotFound)
}

/// The answer to an address the form will not take: it is not a shape an address wears.
pub fn subscribe_invalid_page(cfg: &PublishConfig) -> HttpMessage {
	let body = subscribe_result(
		"That does not look like an email",
		"Check the address and try again. It should look like you@example.com.",
	);
	subscribe_page(cfg, "Check the address", &body, HttpStatus::OK)
}

/// The honest answer where mail is not configured on this host, or the site has no origin to build a
/// confirmation link from: signup is not available, rather than a pending row that can never confirm.
pub fn subscribe_unavailable_page(cfg: &PublishConfig) -> HttpMessage {
	let body = subscribe_result(
		"Signups are not available yet",
		"Email subscriptions are not set up on this site at the moment. Nothing has been recorded.",
	);
	subscribe_page(cfg, "Not available", &body, HttpStatus::OK)
}

/// A titled paragraph, the body every subscription-result page shares.
fn subscribe_result(heading: &str, para: &str) -> String {
	let mut s = String::from("<article class=\"aside aside-subscribe-result\">\n<h1>");
	escape_text(&mut s, heading);
	s.push_str("</h1>\n<p>");
	escape_text(&mut s, para);
	s.push_str("</p>\n</article>\n");
	s
}

/// Wraps a subscription page's body in the reader's own chrome, so the site's skin applies.
///
/// The same [`page`] wrapper the posts use, so a subscribe page is styled by the site's stylesheets
/// exactly as a post is, with no card metadata -- these are not shareable articles.
fn subscribe_page(cfg: &PublishConfig, title: &str, body: &str, status: HttpStatus) -> HttpMessage {
	let head = Head {
		title:		title.to_string(),
		description:	String::new(),
		url:		cfg.url_of(&cfg.subscribe_path()),
		kind:		"website",
		date:		None,
	};
	html_response(status, &page(cfg, &head, body, None))
}

/// An HTML response with the type and status a browser expects.
fn html_response(status: HttpStatus, body: &str) -> HttpMessage {
	let mut resp = HttpMessage::respond_with_text(status, body);
	resp = resp.with_field(
		HeaderName::ContentType,
		HeaderFieldValue::Generic(fmt!("text/html; charset=utf-8")),
	);
	resp
}

#[cfg(test)]
mod tests {
	use super::*;

	use oxedyne_fe2o3_net::http::header::HttpHeadline;

	/// The status a response carries.
	fn status_of(resp: &HttpMessage) -> Option<HttpStatus> {
		match &resp.header.headline {
			HttpHeadline::Response { status }	=> Some(status.clone()),
			_					=> None,
		}
	}

	fn cfg() -> PublishConfig {
		PublishConfig {
			path:		fmt!("/asides"),
			dir:		fmt!("/nonexistent"),
			source:		Source::Dir,
			title:		fmt!("Asides"),
			site_name:	fmt!("Elearnity"),
			base_url:	fmt!("https://example.com"),
			css:		vec![fmt!("/css/a.css")],
			creds:		Default::default(),
			comments:		true,
		comment_rate_secs:	0,
		comment_rate_hourly:	0,
		newsletter_from:	String::new(),
		categories:	vec![fmt!("Personal"), fmt!("Technical")],
		default_author:	String::new(),
		}
	}

	fn post() -> Post {
		Post {
			slug:		fmt!("on-rent"),
			title:		fmt!("On rent"),
			author:		fmt!("jason"),
			categories:	vec![fmt!("Personal")],
			date:		Some(fmt!("2026-07-17")),
			words:		420,
			excerpt:	fmt!("An opening sentence."),
			html:		fmt!("<h1>On rent</h1>\n<p>An opening sentence.</p>\n"),
			also_on:	Vec::new(),
			tags:		Vec::new(),
		}
	}

	/// A post's page carries the tags a card is built from, an absolute canonical URL, and the prose
	/// itself in the response rather than a promise of it.
	#[test]
	fn test_a_post_page_carries_its_card_00() -> Outcome<()> {
		let resp = res!(post_page(&cfg(), &post(), None, None));
		let body = String::from_utf8_lossy(&resp.body).to_string();
		assert!(body.contains("<title>On rent — Elearnity</title>"), "got: {}", body);
		assert!(body.contains(r#"<meta property="og:type" content="article">"#), "got: {}", body);
		assert!(body.contains(r#"<meta property="og:title" content="On rent">"#), "got: {}", body);
		assert!(body.contains(r#"<meta property="og:url" content="https://example.com/asides/on-rent">"#),
			"got: {}", body);
		assert!(body.contains(r#"<link rel="canonical" href="https://example.com/asides/on-rent">"#),
			"got: {}", body);
		assert!(body.contains(r#"<link rel="stylesheet" href="/css/a.css">"#), "got: {}", body);
		assert!(body.contains("<p>An opening sentence.</p>"), "the prose is not in the page: {}", body);
		assert!(body.contains(r#"<time datetime="2026-07-17">"#), "got: {}", body);
		// A post sent nowhere carries no "also on" nav.
		assert!(!body.contains("aside-also"), "an unsent post should have no backfeed: {}", body);
		Ok(())
	}

	/// A post that has been syndicated carries an "also on" backlink to each remote it reached, as a
	/// nofollow link that opens away from the page.
	#[test]
	fn test_a_syndicated_post_backlinks_02() -> Outcome<()> {
		use crate::srv::publish::dest::Destination;
		let mut p = post();
		p.also_on = vec![
			(Destination::Mastodon, fmt!("https://mastodon.social/@me/1")),
			(Destination::Bluesky, fmt!("https://bsky.app/profile/did:plc:x/post/3k")),
		];
		let resp = res!(post_page(&cfg(), &p, None, None));
		let body = String::from_utf8_lossy(&resp.body).to_string();
		assert!(body.contains("Also on"), "no backfeed label: {}", body);
		assert!(body.contains(r#"href="https://mastodon.social/@me/1""#), "no Mastodon link: {}", body);
		assert!(body.contains(">Mastodon</a>"), "no Mastodon name: {}", body);
		assert!(body.contains(">Bluesky</a>"), "no Bluesky name: {}", body);
		assert!(body.contains(r#"rel="nofollow noopener""#), "backlinks should be nofollow: {}", body);
		Ok(())
	}

	/// A title that would close the block it sits in does not close it. `</script>` in prose is a
	/// title an author may plausibly write, and the escape must survive the trip into JSON-LD.
	#[test]
	fn test_a_hostile_title_cannot_break_out_01() -> Outcome<()> {
		let mut p = post();
		p.title = fmt!(r#"</script><img src=x onerror=alert(1)>"#);
		p.excerpt = fmt!(r#"a " quote and an <b>"#);
		let resp = res!(post_page(&cfg(), &p, None, None));
		let body = String::from_utf8_lossy(&resp.body).to_string();
		// The JSON-LD block ends exactly once, where it should.
		assert_eq!(body.matches("</script>").count(), 1, "script block broken out of: {}", body);
		assert!(body.contains(r#"</script>"#), "title not escaped for JSON: {}", body);
		// And nothing reached an attribute unescaped.
		assert!(!body.contains(r#"content="a " quote"#), "attribute broken out of: {}", body);
		assert!(body.contains("&quot;"), "got: {}", body);
		Ok(())
	}

	/// The index links every post and says what each one opens with.
	#[test]
	fn test_the_index_links_its_posts_02() -> Outcome<()> {
		let posts = vec![post()];
		let resp = res!(index(&cfg(), &posts, &[], "", "test"));
		let body = String::from_utf8_lossy(&resp.body).to_string();
		assert_eq!(status_of(&resp), Some(HttpStatus::OK));
		assert!(body.contains(r#"<a href="/asides/on-rent">On rent</a>"#), "got: {}", body);
		assert!(body.contains("An opening sentence."), "no excerpt: {}", body);
		// The list item carries the facts the filter matches on.
		assert!(body.contains(r#"data-read-mins="3""#), "no reading-time datum: {}", body);
		assert!(body.contains(r#"data-categories="Personal""#), "no category datum: {}", body);
		// And the filter's own script is linked, once.
		assert_eq!(body.matches("/asides/filter.js").count(), 1, "filter script not linked once: {}", body);
		Ok(())
	}

	/// An index with nothing in it says so, rather than being a blank page that looks broken.
	#[test]
	fn test_an_empty_index_says_so_04() -> Outcome<()> {
		let resp = res!(index(&cfg(), &[], &[], "", "test"));
		let body = String::from_utf8_lossy(&resp.body).to_string();
		assert!(body.contains("Nothing here yet."), "got: {}", body);
		Ok(())
	}

	/// A slug that could climb out of a directory never reaches one: it is not a name a post may wear,
	/// so the lookup refuses it before anything else looks at it.
	#[test]
	fn test_a_hostile_slug_is_refused_05() -> Outcome<()> {
		let posts = vec![post()];
		for bad in ["../../etc/passwd", "..", "a.b", "a%2Fb"] {
			let path = fmt!("/asides/{}", bad);
			let resp = res!(handle_get(&cfg(), &posts, &[], &path, "", None, "test"));
			assert_eq!(status_of(&resp), Some(HttpStatus::NotFound), "'{}' was not refused", bad);
		}
		Ok(())
	}

	/// A tagged post carries its tags as `.tag` links to the filtered index, and an untagged one
	/// carries no `.post-tags` element at all.
	#[test]
	fn test_a_post_page_carries_its_tags_06() -> Outcome<()> {
		let mut p = post();
		p.tags = vec![fmt!("rust"), fmt!("web")];
		let resp = res!(post_page(&cfg(), &p, None, None));
		let body = String::from_utf8_lossy(&resp.body).to_string();
		assert!(body.contains(r#"<ul class="post-tags">"#), "no tag list: {}", body);
		assert!(body.contains(r#"<a class="tag" href="/asides?tag=rust">rust</a>"#), "got: {}", body);
		assert!(body.contains(r#"<a class="tag" href="/asides?tag=web">web</a>"#), "got: {}", body);

		// A post with no tags has no empty shell.
		let resp = res!(post_page(&cfg(), &post(), None, None));
		let body = String::from_utf8_lossy(&resp.body).to_string();
		assert!(!body.contains("post-tags"), "an untagged post drew a tag element: {}", body);
		Ok(())
	}

	/// The index renders the whole list and the filter above it: every post is present, whatever its
	/// tags, and the filter offers the vocabulary the posts hold. Narrowing is the reader's, in the
	/// browser, so the server draws the tools and all the posts and never a slice.
	#[test]
	fn test_the_index_renders_the_filter_07() -> Outcome<()> {
		let mut a = post();
		a.slug = fmt!("tagged");
		a.title = fmt!("Tagged");
		a.tags = vec![fmt!("rust")];
		let mut b = post();
		b.slug = fmt!("untagged");
		b.title = fmt!("Untagged");
		b.tags = Vec::new();
		let posts = vec![a, b];

		let author = Author {
			username:	fmt!("jason"),
			name:		fmt!("Jason"),
			avatar:		String::new(),
			bio:		fmt!("Notes on rent, housing and what follows."),
		};
		let resp = res!(index(&cfg(), &posts, &[author], "", "test"));
		let body = String::from_utf8_lossy(&resp.body).to_string();

		// Both posts are on the page: the server narrows nothing.
		assert!(body.contains(">Tagged</a>"), "the tagged post is missing: {}", body);
		assert!(body.contains(">Untagged</a>"), "the untagged post is missing: {}", body);
		// The filter is there: the search box, the mode radios, the two chip boxes, and the tag from the
		// posts sits in the selected box by default.
		assert!(body.contains(r#"id="aside-filter-search""#), "no search box: {}", body);
		assert!(body.contains(r#"value="includes" checked"#), "Includes is not the default mode: {}", body);
		assert!(body.contains(r#"id="aside-tags-selected""#), "no selected box: {}", body);
		assert!(body.contains(r#"data-tag="rust""#), "the tag is not a chip: {}", body);
		// A multi-word category rides in data-categories comma-joined, so the filter's comma-split keeps
		// it whole rather than tearing "Big Ideas" into "Big" and "Ideas".
		let mut c = post();
		c.slug = fmt!("multi");
		c.categories = vec![fmt!("Big Ideas"), fmt!("Personal")];
		let resp = res!(index(&cfg(), &[c], &[], "", "test"));
		let cbody = String::from_utf8_lossy(&resp.body).to_string();
		assert!(cbody.contains(r#"data-categories="Big Ideas,Personal""#),
			"multi-word category not comma-joined: {}", cbody);

		// The author drew a face with an initial, since the fixture set no avatar.
		assert!(body.contains(r#"data-author="jason""#), "no author face: {}", body);
		assert!(body.contains("aside-author-initial"), "no drawn initial for an avatarless author: {}", body);
		// The category checkboxes are drawn from the config, checked.
		assert!(body.contains(r#"<label class="aside-cat"><input type="checkbox" checked value="Personal">"#),
			"no category checkbox: {}", body);
		Ok(())
	}

	/// What the site is about stands above the posts, in the words of whoever writes it. On a blog of
	/// one, the description carries no name over it, since the name is on every post already; where
	/// more than one person writes, each description is attributed.
	#[test]
	fn test_the_index_says_what_the_site_is_about_13() -> Outcome<()> {
		let one = Author {
			username:	fmt!("jason"),
			name:		fmt!("Jason"),
			avatar:		String::new(),
			bio:		fmt!("Notes on rent and what follows."),
		};
		let resp = res!(index(&cfg(), &[post()], std::slice::from_ref(&one), "", "test"));
		let body = String::from_utf8_lossy(&resp.body).to_string();
		assert!(body.contains("aside-about"), "no description above the posts: {}", body);
		assert!(body.contains("Notes on rent and what follows."), "the description is not shown: {}", body);
		assert!(!body.contains("aside-about-name"), "a lone author was named over their own line: {}", body);
		// It is above the filter, which is above the list: what the site is about is met first.
		let about = body.find("aside-about").unwrap_or(usize::MAX);
		let filter = body.find("aside-filter").unwrap_or(usize::MAX);
		let list = body.find("aside-index-list").unwrap_or(usize::MAX);
		assert!(about < filter && filter < list, "the page is out of order: {}", body);

		// A second author, and each description carries a name.
		let two = Author {
			username:	fmt!("ada"),
			name:		fmt!("Ada"),
			avatar:		fmt!("/asides/avatar/ada"),
			bio:		fmt!("Writes about machines."),
		};
		let resp = res!(index(&cfg(), &[post()], &[one, two], "", "test"));
		let body = String::from_utf8_lossy(&resp.body).to_string();
		assert!(body.contains("aside-about-name"), "two authors and no names: {}", body);
		assert!(body.contains("Writes about machines."), "the second description is missing: {}", body);
		assert!(body.contains(r#"<img class="aside-author-pic" alt="" src="/asides/avatar/ada">"#),
			"an uploaded picture is not drawn: {}", body);

		// A blog whose first post is not written yet still says what it will be about: the description
		// stands on an empty index, and the filter offers no face, there being nothing to narrow.
		let one = Author {
			username:	fmt!("jason"),
			name:		fmt!("Jason"),
			avatar:		String::new(),
			bio:		fmt!("Notes on rent and what follows."),
		};
		let resp = res!(index(&cfg(), &[], std::slice::from_ref(&one), "", "test"));
		let body = String::from_utf8_lossy(&resp.body).to_string();
		assert!(body.contains("Notes on rent and what follows."),
			"an empty blog said nothing about itself: {}", body);
		assert!(!body.contains("aside-filter-authors"),
			"a face was offered for an author with no posts: {}", body);

		// An author who has written no description draws no panel at all, rather than an empty one.
		let bare = Author {
			username:	fmt!("mel"),
			name:		fmt!("Mel"),
			avatar:		String::new(),
			bio:		String::new(),
		};
		let resp = res!(index(&cfg(), &[post()], &[bare], "", "test"));
		let body = String::from_utf8_lossy(&resp.body).to_string();
		assert!(!body.contains("aside-about"), "an empty description drew a panel: {}", body);
		Ok(())
	}

	/// A post carries a byline above the prose and, where its author has written a description, a note
	/// about them beneath it. A post whose author is unknown carries neither rather than a blank.
	#[test]
	fn test_a_post_says_who_wrote_it_14() -> Outcome<()> {
		let a = Author {
			username:	fmt!("jason"),
			name:		fmt!("Jason Hoogland"),
			avatar:		String::new(),
			bio:		fmt!("Notes on rent."),
		};
		let resp = res!(post_page(&cfg(), &post(), Some(&a), None));
		let body = String::from_utf8_lossy(&resp.body).to_string();
		assert!(body.contains("aside-byline"), "no byline: {}", body);
		assert!(body.contains(r#"<span class="aside-byline-name">Jason Hoogland</span>"#),
			"the byline names nobody: {}", body);
		assert!(body.contains("aside-author-note"), "no note about the author: {}", body);
		assert!(body.contains("Notes on rent."), "the description is not under the post: {}", body);
		// The byline is above the prose and the note below it.
		let byline = body.find("aside-byline").unwrap_or(usize::MAX);
		let prose = body.find("<h1>On rent</h1>").unwrap_or(usize::MAX);
		let note = body.find("aside-author-note").unwrap_or(usize::MAX);
		assert!(byline < prose && prose < note, "the post is out of order: {}", body);

		// An author with nothing written about them keeps the byline and drops the note.
		let quiet = Author { bio: String::new(), ..a.clone() };
		let resp = res!(post_page(&cfg(), &post(), Some(&quiet), None));
		let body = String::from_utf8_lossy(&resp.body).to_string();
		assert!(body.contains("aside-byline"), "the byline went with the description: {}", body);
		assert!(!body.contains("aside-author-note"), "an empty description drew a note: {}", body);

		// A post whose author could not be resolved carries neither.
		let resp = res!(post_page(&cfg(), &post(), None, None));
		let body = String::from_utf8_lossy(&resp.body).to_string();
		assert!(!body.contains("aside-byline"), "an unattributed post drew a byline: {}", body);
		Ok(())
	}

	/// A name a reader could put in a byline is escaped everywhere it is drawn, since a display name is
	/// whatever a member typed into their own profile.
	#[test]
	fn test_a_hostile_profile_cannot_break_out_15() -> Outcome<()> {
		let a = Author {
			username:	fmt!("jason"),
			name:		fmt!(r#"<img src=x onerror=alert(1)>"#),
			avatar:		fmt!(r#""onload="alert(1)"#),
			bio:		fmt!(r#"</p><script>alert(1)</script>"#),
		};
		let resp = res!(index(&cfg(), &[post()], std::slice::from_ref(&a), "", "test"));
		let body = String::from_utf8_lossy(&resp.body).to_string();
		assert!(!body.contains("<img src=x"), "a name reached the page as markup: {}", body);
		assert!(!body.contains("<script>alert(1)</script>"), "a description reached the page as markup: {}",
			body);
		assert!(!body.contains(r#"src=""onload="#), "an avatar broke out of its attribute: {}", body);

		let resp = res!(post_page(&cfg(), &post(), Some(&a), None));
		let body = String::from_utf8_lossy(&resp.body).to_string();
		assert!(!body.contains("<img src=x"), "a name reached the post as markup: {}", body);
		assert!(!body.contains("<script>alert(1)</script>"), "a description reached the post as markup: {}",
			body);
		Ok(())
	}

	/// A member's uploaded picture is served from this module, under the site's own prefix.
	#[test]
	fn test_a_picture_has_a_path_of_its_own_16() -> Outcome<()> {
		let c = cfg();
		assert_eq!(c.avatar_path("abc123"), "/asides/avatar/abc123");
		assert_eq!(c.avatar_prefix(), "/asides/avatar/");
		// It is not a post, so a request for one is never counted as a read.
		assert!(served_post(&c, &[post()], &c.avatar_path("abc123")).is_none(),
			"a picture counted as a post");
		Ok(())
	}


	/// A post says how long it takes to read, beside its date and above the prose, so a reader deciding
	/// whether to start is told before rather than after.
	#[test]
	fn test_a_post_says_how_long_it_takes_to_read_09() -> Outcome<()> {
		let resp = res!(post_page(&cfg(), &post(), None, None));
		let body = String::from_utf8_lossy(&resp.body).to_string();
		// 420 words at 200 a minute rounds up to 3.
		assert!(body.contains(r#"<span class="aside-read">3 min read</span>"#), "got: {}", body);

		// A piece shorter than a minute is still a minute, since "0 min read" tells a reader nothing.
		let mut p = post();
		p.words = 12;
		let resp = res!(post_page(&cfg(), &p, None, None));
		let body = String::from_utf8_lossy(&resp.body).to_string();
		assert!(body.contains("1 min read"), "a short post lost its minute: {}", body);

		// A post whose words were never counted says nothing rather than "0 min read".
		p.words = 0;
		let resp = res!(post_page(&cfg(), &p, None, None));
		let body = String::from_utf8_lossy(&resp.body).to_string();
		assert!(!body.contains("aside-read"), "an uncounted post claimed a reading time: {}", body);
		assert!(body.contains(r#"<time datetime="2026-07-17">"#), "the date went with it: {}", body);
		Ok(())
	}

	/// A post that is not there is still a page, with a way back to the ones that are.
	#[test]
	fn test_a_missing_post_is_a_page_03() -> Outcome<()> {
		let resp = not_found(&cfg());
		let body = String::from_utf8_lossy(&resp.body).to_string();
		assert_eq!(status_of(&resp), Some(HttpStatus::NotFound));
		assert!(body.contains(r#"<a href="/asides">Asides</a>"#), "no way back: {}", body);
		Ok(())
	}

	/// Only a post that was actually served is a post that was read.
	///
	/// The index, the feed and the JSON are not posts: counting a browse of the index as a read of
	/// everything listed on it would make the tally meaningless in exactly the direction that
	/// flatters.
	#[test]
	fn test_only_a_served_post_is_counted_08() -> Outcome<()> {
		let mut a = post();
		a.slug = fmt!("here");
		let posts = vec![a];
		let c = cfg();

		assert_eq!(served_post(&c, &posts, "/asides/here").map(|p| p.slug.as_str()), Some("here"));

		// Not posts.
		assert!(served_post(&c, &posts, "/asides").is_none(), "the index counted as a read");
		assert!(served_post(&c, &posts, &c.feed_path()).is_none(), "the feed counted as a read");
		assert!(served_post(&c, &posts, &c.json_path()).is_none(), "the JSON counted as a read");

		// A post that does not exist was not served, and a path that is not a name never reaches a
		// lookup -- the same guard the renderer applies.
		assert!(served_post(&c, &posts, "/asides/nonesuch").is_none());
		assert!(served_post(&c, &posts, "/asides/../../etc/passwd").is_none());
		assert!(served_post(&c, &posts, "/asides/").is_none());
		Ok(())
	}
}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ COMMENTS                                                                  │
// └───────────────────────────────────────────────────────────────────────────┘

/// What a post's page needs to draw its conversation.
///
/// Assembled by the caller, which holds the database; the rendering here takes data and touches
/// nothing, on the same terms as the rest of this module.
pub struct CommentsView {
	/// The approved comments, threaded and in the ranker's order.
	pub threads:	Vec<Thread>,
	/// How many comments that is, at every depth.
	pub count:	usize,
	/// Which page of the conversation is shown, from one.
	pub page:	usize,
	/// How many pages there are.
	pub pages:	usize,
	/// Which order the reader asked for.
	pub order:	&'static str,
	/// The post's own path, for the links the pager and the ordering build.
	pub path:	String,
	/// The challenge a sender's proof must answer.
	pub challenge:	String,
	/// What the last attempt said, where the reader has just made one.
	pub said:	Option<String>,
	/// Whether the site is taking comments at all.
	pub open:	bool,
	/// The comment this reader may still correct, and the token proving they wrote it.
	pub editable:	Option<(String, String)>,
}

/// The conversation below a post, and the form to join it.
pub fn comments_section(cfg: &PublishConfig, post: &Post, view: &CommentsView) -> String {
	let mut s = String::new();
	s.push_str("<section class=\"comments\" id=\"comments\">\n");
	s.push_str(&fmt!("<h2 class=\"comments-h\">{}</h2>\n", match view.count {
		0	=> fmt!("No comments yet"),
		1	=> fmt!("One comment"),
		n	=> fmt!("{} comments", n),
	}));

	// What the last attempt said, where there was one. Shown at the top, because it is the answer to
	// something the reader just did and they should not have to hunt for it.
	if let Some(said) = &view.said {
		s.push_str("<p class=\"comments-said\">");
		escape_text(&mut s, said);
		s.push_str("</p>\n");
	}

	if !view.threads.is_empty() {
		// The ordering, where there is more than one comment to order.
		if view.count > 1 {
			s.push_str(&comments_order(view));
		}
		s.push_str("<ol class=\"comment-list\">\n");
		for t in &view.threads {
			s.push_str(&thread_item(t, 0, &view.path, &view.editable));
		}
		s.push_str("</ol>\n");
		s.push_str(&comments_pager(view));
	}

	if view.open {
		s.push_str(&comment_form(cfg, post, view, None));
	} else {
		// Said once, whether or not there is a conversation above it: a reader looking for the form
		// should learn why it is not there rather than assume the page is broken.
		s.push_str("<p class=\"comments-shut\">Comments are closed on this post.</p>\n");
	}

	s.push_str("</section>\n");
	s
}

/// The order the conversation is read in.
///
/// Two links rather than a form, so it works with nothing running and a reader can share the URL of
/// the view they are looking at.
fn comments_order(view: &CommentsView) -> String {
	let mut s = String::from("<nav class=\"comment-order\">");
	for (i, (key, label)) in [("oldest", "Oldest first"), ("newest", "Newest first")]
		.iter().enumerate()
	{
		// A separator in the markup, not only in a stylesheet. This module leaves the look to the
		// site, but a site that has not styled this yet should still read as two choices rather than
		// as one run-together word -- which is what it did.
		if i > 0 {
			s.push_str(" <span class=\"comment-order-sep\">&middot;</span> ");
		}
		if view.order == *key {
			s.push_str(&fmt!("<span class=\"comment-order-on\">{}</span>", label));
		} else {
			s.push_str("<a href=\"");
			escape_attr(&mut s, &fmt!("{}?order={}#comments", view.path, key));
			s.push_str("\">");
			s.push_str(label);
			s.push_str("</a>");
		}
	}
	s.push_str("</nav>\n");
	s
}

/// The pager beneath a long conversation.
fn comments_pager(view: &CommentsView) -> String {
	if view.pages <= 1 {
		return String::new();
	}
	let link = |p: usize, label: &str, s: &mut String| {
		s.push_str("<a href=\"");
		escape_attr(s, &fmt!("{}?order={}&cpage={}#comments", view.path, view.order, p));
		s.push_str("\">");
		s.push_str(label);
		s.push_str("</a>");
	};
	let mut s = String::from("<nav class=\"comment-pager\">");
	if view.page > 1 {
		link(view.page - 1, "Earlier comments", &mut s);
	}
	s.push_str(&fmt!("<span class=\"comment-pager-at\">Page {} of {}</span>", view.page, view.pages));
	if view.page < view.pages {
		link(view.page + 1, "More comments", &mut s);
	}
	s.push_str("</nav>\n");
	s
}

/// One comment and its replies.
fn thread_item(
	t:		&Thread,
	depth:		usize,
	path:		&str,
	editable:	&Option<(String, String)>,
)
	-> String
{
	let mut s = String::new();
	s.push_str(&fmt!("<li class=\"comment\" id=\"c-{}\">\n", esc_id(&t.comment.id)));

	s.push_str("<div class=\"comment-by\">");
	s.push_str("<span class=\"comment-who\">");
	escape_text(&mut s, t.comment.author.display_name());
	s.push_str("</span>");
	// A commenter may call themselves anything, including the name of the person whose site this
	// is. Nothing can stop them typing it, so the site says which comments it wrote instead: an
	// absent mark is the claim, not the name. Only an approved comment can carry it, and only where
	// the site's own admin wrote it.
	if t.comment.by_site_author {
		s.push_str(" <span class=\"comment-author-mark\" title=\"Written by the author of this site\">author</span>");
	}
	if !t.comment.created.is_empty() {
		s.push_str(" <time class=\"comment-when\" datetime=\"");
		escape_attr(&mut s, &t.comment.created);
		s.push_str("\">");
		escape_text(&mut s, &t.comment.created[..10.min(t.comment.created.len())]);
		s.push_str("</time>");
	}
	s.push_str("</div>\n");

	// The prose, already brought within the policy by `Comment::render`. A comment whose source will
	// not parse shows as its own words rather than vanishing: it is still what somebody said.
	s.push_str("<div class=\"comment-body\">");
	match t.comment.render() {
		Ok(html)	=> s.push_str(&html),
		Err(_)		=> {
			s.push_str("<p>");
			escape_text(&mut s, &t.comment.body);
			s.push_str("</p>");
		}
	}
	s.push_str("</div>\n");

	// A reply link, down to the depth the module threads. Below that a reader replies to the parent,
	// which is where the flattened comment already sits.
	if depth + 1 < DEPTH_MAX {
		s.push_str(&fmt!(
			"<a class=\"comment-reply\" href=\"#comment-form\" data-reply-to=\"{id}\" \
			data-reply-name=\"{who}\">Reply</a>\n",
			id	= esc_id(&t.comment.id),
			who	= {
				let mut a = String::new();
				escape_attr(&mut a, t.comment.author.display_name());
				a
			},
		));
	}

	// The author's own way to correct what they just wrote, shown only to whoever holds the token
	// for this comment and only while the window stands.
	if let Some((cid, token)) = editable {
		if *cid == t.comment.id {
			s.push_str(&edit_form(path, &t.comment, token));
		}
	}

	if !t.replies.is_empty() {
		s.push_str("<ol class=\"comment-replies\">\n");
		for r in &t.replies {
			s.push_str(&thread_item(r, depth + 1, path, editable));
		}
		s.push_str("</ol>\n");
	}
	s.push_str("</li>\n");
	s
}

/// The form a comment's own author corrects it with.
fn edit_form(path: &str, c: &crate::srv::publish::comment::Comment, token: &str) -> String {
	let mut s = String::new();
	s.push_str("<details class=\"comment-edit\"><summary>Correct this</summary>\n");
	s.push_str("<form method=\"POST\" action=\"");
	escape_attr(&mut s, &fmt!("{}/comment/edit", path));
	s.push_str("\">\n");
	s.push_str("<input type=\"hidden\" name=\"id\" value=\"");
	escape_attr(&mut s, &c.id);
	s.push_str("\">\n<input type=\"hidden\" name=\"token\" value=\"");
	escape_attr(&mut s, token);
	s.push_str("\">\n<textarea name=\"body\" rows=\"5\">");
	escape_text(&mut s, &c.body);
	s.push_str("</textarea>\n");
	s.push_str("<p class=\"comment-note\">A comment that has already been published goes back to \
		the author to look at again.</p>\n");
	s.push_str("<button type=\"submit\">Save the correction</button>\n</form>\n</details>\n");
	s
}

/// The form for writing one.
///
/// Three things a reader does not see and one they do. The honeypot is a field a person cannot fill
/// because it is not shown, so anything in it was put there by something filling every field it
/// found. The challenge and the nonce are the proof: the browser spends about a second finding a
/// nonce, which costs a reader nothing they notice and costs a machine posting ten thousand comments
/// ten thousand seconds. The parent is which comment is being answered.
fn comment_form(cfg: &PublishConfig, post: &Post, view: &CommentsView, parent: Option<&str>) -> String {
	let mut s = String::new();
	s.push_str("<form class=\"comment-form\" id=\"comment-form\" method=\"POST\" action=\"");
	escape_attr(&mut s, &cfg.comment_path(&post.slug));
	s.push_str("\">\n");
	s.push_str("<h3 class=\"comment-form-h\">Leave a comment</h3>\n");

	// Where a reply is being written, said plainly, with a way out of it.
	s.push_str("<p class=\"comment-replying\" id=\"comment-replying\" hidden>Replying to \
		<span id=\"comment-replying-who\"></span> \
		<a href=\"#comment-form\" id=\"comment-reply-cancel\">(cancel)</a></p>\n");
	// Escaped like every other value here. The sole caller passes None today, which is exactly why
	// this was missed -- and `parent` is attacker-supplied, so the moment somebody wires it up an
	// unescaped value is an attribute breakout.
	s.push_str("<input type=\"hidden\" name=\"parent\" id=\"comment-parent\" value=\"");
	escape_attr(&mut s, parent.unwrap_or(""));
	s.push_str("\">\n");
	s.push_str("<input type=\"hidden\" name=\"challenge\" id=\"comment-challenge\" value=\"");
	escape_attr(&mut s, &view.challenge);
	s.push_str("\">\n");
	s.push_str("<input type=\"hidden\" name=\"nonce\" id=\"comment-nonce\" value=\"\">\n");
	s.push_str(&fmt!("<input type=\"hidden\" name=\"bits\" value=\"{}\">\n", POW_BITS));

	// The honeypot. Hidden from a person by every means at once -- off-screen, no tab stop, told to
	// assistive technology that it is not for them -- and left in the markup for anything that reads
	// the markup rather than the page.
	// The hiding is an inline style and not a class, deliberately. This module does not own the site's
	// stylesheet -- a site brings its own -- so a class here is a rule that may never exist, and a
	// honeypot a reader can see is a field they will fill in and have their comment silently refused
	// for. Measured in a browser: with only a class, it rendered as an ordinary visible input.
	s.push_str("<div class=\"comment-hp\" aria-hidden=\"true\" \
		style=\"position:absolute;left:-9999px;width:1px;height:1px;overflow:hidden\">\
		<label for=\"comment-website\">Website</label>\
		<input type=\"text\" id=\"comment-website\" name=\"website\" tabindex=\"-1\" \
		autocomplete=\"off\"></div>\n");

	s.push_str("<div class=\"comment-fields\">\n");
	s.push_str("<label class=\"comment-lbl\" for=\"comment-name\">Name\
		<input type=\"text\" id=\"comment-name\" name=\"name\" maxlength=\"64\" required></label>\n");
	// The address is optional, and what it is for is said where it is asked for rather than in a
	// policy page nobody opens.
	s.push_str("<label class=\"comment-lbl\" for=\"comment-email\">Email \
		<span class=\"comment-hint\">optional, never shown or shared</span>\
		<input type=\"email\" id=\"comment-email\" name=\"email\" autocomplete=\"email\"></label>\n");
	s.push_str("</div>\n");

	s.push_str(&fmt!("<label class=\"comment-lbl\" for=\"comment-body\">Comment\
		<textarea id=\"comment-body\" name=\"body\" rows=\"6\" maxlength=\"{}\" required></textarea>\
		</label>\n", crate::srv::publish::comment::BODY_MAX));
	s.push_str("<p class=\"comment-note\">Markdown works. Links are kept, images and scripts are not. \
		A first comment waits for the author to see it.</p>\n");

	s.push_str("<button type=\"button\" class=\"comment-preview-btn\" id=\"comment-preview-btn\" \
		data-to=\"");
	escape_attr(&mut s, &cfg.comment_preview_path(&post.slug));
	s.push_str("\">Preview</button>\n");
	s.push_str("<div class=\"comment-preview\" id=\"comment-preview\" hidden></div>\n");
	s.push_str("<button type=\"submit\" class=\"comment-send\" id=\"comment-send\">Post comment</button>\n");
	s.push_str("<span class=\"comment-working\" id=\"comment-working\" hidden>Working…</span>\n");
	s.push_str("</form>\n");
	// Referenced, not inlined: see `comment_js_path`. `defer` because it only wires handlers, and a
	// form that works without it is the point -- a reader with no scripting posts a comment with no
	// proof, and the server holds it rather than refusing it.
	s.push_str("<script defer src=\"");
	escape_attr(&mut s, &cfg.comment_js_path());
	s.push_str("\"></script>\n");
	s
}

/// A rendered preview, or the reason there is none.
///
/// HTML rather than JSON: what comes back is dropped straight into the page, and wrapping a fragment
/// in JSON only to unwrap it is a step that buys nothing.
pub fn comment_preview(html: Option<String>) -> HttpMessage {
	let body = html.unwrap_or_else(|| fmt!(
		"<p class=\"comment-preview-none\">Nothing to preview yet, or you have previewed very \
		recently.</p>"));
	let mut resp = HttpMessage::ok_respond_with_text(body);
	resp = resp.with_field(
		HeaderName::ContentType,
		HeaderFieldValue::Generic(fmt!("text/html; charset=utf-8")),
	);
	resp
}

/// Serves the comment form's script.
///
/// Cached hard: it is the same bytes for every post on every site, and it changes only when this
/// server does.
pub fn comment_js() -> HttpMessage {
	let mut resp = HttpMessage::ok_respond_with_text(COMMENT_JS.to_string());
	resp = resp.with_field(
		HeaderName::ContentType,
		HeaderFieldValue::Generic(fmt!("text/javascript; charset=utf-8")),
	);
	resp = resp.with_field(
		HeaderName::CacheControl,
		HeaderFieldValue::Generic(fmt!("public, max-age=86400")),
	);
	resp
}

/// The index filter's script, served as a file. Static, cacheable, and CSP-friendly.
pub fn filter_js() -> HttpMessage {
	let mut resp = HttpMessage::ok_respond_with_text(FILTER_JS.to_string());
	resp = resp.with_field(
		HeaderName::ContentType,
		HeaderFieldValue::Generic(fmt!("text/javascript; charset=utf-8")),
	);
	resp = resp.with_field(
		HeaderName::CacheControl,
		HeaderFieldValue::Generic(fmt!("public, max-age=86400")),
	);
	resp
}

/// An id, reduced to what may sit in one.
///
/// A comment's name is minted from a small alphabet so this changes nothing in practice; it is here
/// so that a record written by hand, or by a later version with a wider alphabet, cannot put anything
/// into an `id` attribute or a fragment that does not belong there.
fn esc_id(s: &str) -> String {
	s.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').collect()
}

/// The script the form needs: the proof, and the reply wiring.
///
/// Vanilla, inline, and small enough to read. It uses the browser's own SHA-256 -- the one strong
/// digest every browser implements natively -- rather than carrying an implementation of its own,
/// which is why the server verifies the proof with the same.
///
/// **The form works without it**, which is the point of doing the proof on submit rather than
/// gating the fields: a reader with no scripting posts a comment with no nonce, and the server holds
/// it for a person instead of refusing it. The proof buys a queue that is not full of machines; it is
/// not a condition of being heard.
const COMMENT_JS: &str = r#"(function () {
	var form = document.getElementById('comment-form');
	if (!form || !window.crypto || !window.crypto.subtle) return;

	/* Replying: which comment, said plainly, and a way back out. */
	var parent = document.getElementById('comment-parent');
	var banner = document.getElementById('comment-replying');
	var who = document.getElementById('comment-replying-who');
	document.querySelectorAll('.comment-reply').forEach(function (a) {
		a.addEventListener('click', function () {
			parent.value = a.getAttribute('data-reply-to') || '';
			who.textContent = a.getAttribute('data-reply-name') || '';
			banner.hidden = false;
		});
	});
	var cancel = document.getElementById('comment-reply-cancel');
	if (cancel) cancel.addEventListener('click', function (ev) {
		ev.preventDefault();
		parent.value = '';
		banner.hidden = true;
	});

	/* Preview: ask the server for the same rendering a reader would get, since the
	   parser that matters is the one in Rust and there is not a second one here. */
	var pv = document.getElementById('comment-preview-btn');
	var pvOut = document.getElementById('comment-preview');
	if (pv && pvOut) pv.addEventListener('click', function () {
		var src = document.getElementById('comment-body').value;
		if (!src.trim()) return;
		var data = new URLSearchParams();
		data.set('body', src);
		pv.disabled = true;
		fetch(pv.getAttribute('data-to'), {
			method: 'POST',
			credentials: 'same-origin',
			headers: { 'Content-Type': 'application/x-www-form-urlencoded' },
			body: data.toString(),
		}).then(function (r) { return r.text(); })
		  .then(function (html) { pvOut.innerHTML = html; pvOut.hidden = false; })
		  .catch(function () { /* a preview that will not come is not an error worth a dialog */ })
		  .then(function () { pv.disabled = false; });
	});

	/* The proof. Count leading zero bits of SHA-256(challenge + nonce) until the
	   width is met. Done on submit so a reader who never comments never pays. */
	function zeros(buf) {
		var b = new Uint8Array(buf), n = 0;
		for (var i = 0; i < b.length; i++) {
			if (b[i] === 0) { n += 8; continue; }
			var v = b[i], c = 0;
			while ((v & 0x80) === 0) { c++; v = (v << 1) & 0xff; }
			return n + c;
		}
		return n;
	}

	var enc = new TextEncoder();
	form.addEventListener('submit', function (ev) {
		if (form.dataset.proved === '1') return;      /* already done; let it go */
		ev.preventDefault();
		var challenge = document.getElementById('comment-challenge').value;
		var bits = parseInt(form.querySelector('input[name=bits]').value, 10) || 0;
		var send = document.getElementById('comment-send');
		var working = document.getElementById('comment-working');
		send.disabled = true;
		if (working) working.hidden = false;

		var n = 0;
		function attempt() {
			/* A slice at a time, yielding between, so the page never locks up. */
			var deadline = Date.now() + 60;
			function step() {
				if (Date.now() > deadline) { setTimeout(attempt, 0); return; }
				crypto.subtle.digest('SHA-256', enc.encode(challenge + n)).then(function (d) {
					if (zeros(d) >= bits) {
						document.getElementById('comment-nonce').value = String(n);
						form.dataset.proved = '1';
						form.submit();
						return;
					}
					n++;
					step();
				});
			}
			step();
		}
		attempt();
	});
})();
"#;

/// The index filter. Reads the post list already in the page and shows or hides each item against the
/// controls above it: a search box, the author faces, the tag mode and its two boxes, the category
/// checkboxes and the reading-time slider. It renders nothing and fetches nothing -- every post is in
/// the markup, and this only ever narrows what is seen.
const FILTER_JS: &str = r##"(function () {
	"use strict";
	var list = document.getElementById("aside-index-list");
	if (!list) { return; }
	var items = Array.prototype.slice.call(list.querySelectorAll(".aside-index-item"));
	var empty = document.getElementById("aside-empty");

	// Each item's filterable facts, read once from its data attributes.
	var rows = items.map(function (li) {
		var tags = (li.getAttribute("data-tags") || "").split(" ").filter(Boolean);
		var cats = (li.getAttribute("data-categories") || "").split(",").filter(Boolean);
		return {
			el:     li,
			author: li.getAttribute("data-author") || "",
			tags:   tags,
			cats:   cats,
			mins:   parseInt(li.getAttribute("data-read-mins") || "0", 10),
			text:   li.getAttribute("data-search") || ""
		};
	});

	// The filter's state. Every field starts in the value that hides nothing.
	var search = "";
	var authors = {};          // pressed authors; empty means every author
	var mode = "includes";
	var selected = {};         // tags in the selected box
	var cats = {};             // checked categories
	var offered = {};          // every category the filter offers, checked or not
	var tmin = -Infinity, tmax = Infinity;

	function keys(o) { var k = []; for (var x in o) { if (o[x]) { k.push(x); } } return k; }
	function any(o) { for (var x in o) { if (o[x]) { return true; } } return false; }

	// Whether one item passes every control at once.
	function passes(r) {
		if (search && r.text.indexOf(search) === -1) { return false; }
		if (any(authors) && !authors[r.author]) { return false; }
		// Categories: the filter constrains only against the categories it offers. A post is hidden only
		// when at least one of its categories is offered and every offered one it has is unchecked. A
		// post with no category, or one whose categories the filter does not offer at all -- an operator
		// dropped the category from the config after the post used it -- is left alone, never vanished.
		if (r.cats.length) {
			var constrained = false, ok = false;
			for (var i = 0; i < r.cats.length; i++) {
				if (offered[r.cats[i]]) { constrained = true; if (cats[r.cats[i]]) { ok = true; break; } }
			}
			if (constrained && !ok) { return false; }
		}
		// Tags: an untagged post always passes; an empty selected box imposes nothing.
		if (r.tags.length && any(selected)) {
			var inter = 0, outside = 0;
			for (var j = 0; j < r.tags.length; j++) {
				if (selected[r.tags[j]]) { inter++; } else { outside++; }
			}
			if (mode === "includes" && inter === 0) { return false; }
			if (mode === "only" && outside > 0) { return false; }
			if (mode === "excludes" && inter > 0) { return false; }
		}
		if (r.mins < tmin || r.mins > tmax) { return false; }
		return true;
	}

	function apply() {
		var shown = 0;
		for (var i = 0; i < rows.length; i++) {
			var ok = passes(rows[i]);
			rows[i].el.hidden = !ok;
			if (ok) { shown++; }
		}
		if (empty) { empty.hidden = shown !== 0; }
	}

	// The search box.
	var box = document.getElementById("aside-filter-search");
	if (box) {
		box.addEventListener("input", function () {
			search = box.value.trim().toLowerCase();
			apply();
		});
	}

	// The author faces: press to narrow to that author, press again to release. None pressed is all.
	var authorRow = document.getElementById("aside-filter-authors");
	if (authorRow) {
		authorRow.addEventListener("click", function (e) {
			var b = e.target.closest(".aside-author");
			if (!b) { return; }
			var u = b.getAttribute("data-author");
			authors[u] = !authors[u];
			b.setAttribute("aria-pressed", authors[u] ? "true" : "false");
			apply();
		});
	}

	// The tag mode.
	var modeRow = document.getElementById("aside-filter-mode");
	if (modeRow) {
		modeRow.addEventListener("change", function (e) {
			if (e.target.name === "aside-mode") { mode = e.target.value; apply(); }
		});
	}

	// The two chip boxes. A chip lives in one box; clicking it, or dragging it, sends it to the other.
	var selBox = document.getElementById("aside-tags-selected");
	var srcBox = document.getElementById("aside-tags-source");

	function refreshSelected() {
		selected = {};
		if (selBox) {
			selBox.querySelectorAll(".aside-chip").forEach(function (c) {
				selected[c.getAttribute("data-tag")] = true;
			});
		}
	}
	refreshSelected();

	// A chip carries its closer only in the selected box; moving it re-dresses it for its new home.
	function dress(chip, inSelected) {
		var x = chip.querySelector(".aside-chip-x");
		if (inSelected && !x) {
			chip.insertAdjacentHTML("beforeend",
				' <span class="aside-chip-x" aria-hidden="true">×</span>');
		} else if (!inSelected && x) {
			x.parentNode.removeChild(x);
		}
	}

	function move(chip, toSource) {
		if (!selBox || !srcBox) { return; }
		var dest = toSource ? srcBox : selBox;
		dest.appendChild(chip);
		dress(chip, !toSource);
		refreshSelected();
		apply();
	}

	function wireBox(boxEl, toSource) {
		if (!boxEl) { return; }
		boxEl.addEventListener("click", function (e) {
			var chip = e.target.closest(".aside-chip");
			if (chip) { move(chip, toSource); }
		});
		// Drop target: a chip dragged here lands here, whichever box it came from.
		boxEl.addEventListener("dragover", function (e) { e.preventDefault(); boxEl.classList.add("aside-drop"); });
		boxEl.addEventListener("dragleave", function () { boxEl.classList.remove("aside-drop"); });
		boxEl.addEventListener("drop", function (e) {
			e.preventDefault();
			boxEl.classList.remove("aside-drop");
			var tag = e.dataTransfer.getData("text/plain");
			var chip = document.querySelector('.aside-chip[data-tag="' + (window.CSS && CSS.escape ? CSS.escape(tag) : tag) + '"]');
			if (chip && chip.parentNode !== boxEl) {
				boxEl.appendChild(chip);
				dress(chip, boxEl === selBox);
				refreshSelected();
				apply();
			}
		});
	}
	wireBox(selBox, true);
	wireBox(srcBox, false);

	// A tag link lands here as `?tag=x`. Honour it by leaving only that tag in the selected box and
	// sending every other tag to the source, so the reader arrives on that tag as the tag chips say.
	(function () {
		var m = /[?&]tag=([a-z0-9-]+)/.exec(location.search);
		if (!m || !selBox || !srcBox) { return; }
		var want = m[1];
		Array.prototype.slice.call(selBox.querySelectorAll(".aside-chip")).forEach(function (chip) {
			if (chip.getAttribute("data-tag") !== want) { srcBox.appendChild(chip); dress(chip, false); }
		});
		refreshSelected();
	})();

	// Dragging a chip carries its tag; both boxes read it on drop.
	document.addEventListener("dragstart", function (e) {
		var chip = e.target.closest && e.target.closest(".aside-chip");
		if (chip && e.dataTransfer) {
			e.dataTransfer.setData("text/plain", chip.getAttribute("data-tag"));
			e.dataTransfer.effectAllowed = "move";
		}
	});

	// The category checkboxes.
	var catRow = document.getElementById("aside-filter-cats");
	if (catRow) {
		catRow.querySelectorAll('input[type="checkbox"]').forEach(function (cb) {
			cats[cb.value] = cb.checked;
			offered[cb.value] = true;
		});
		catRow.addEventListener("change", function (e) {
			if (e.target.type === "checkbox") { cats[e.target.value] = e.target.checked; apply(); }
		});
	}

	// The reading-time slider: two thumbs that may not cross. The read-out follows them.
	var timeWrap = document.getElementById("aside-filter-time");
	var tminEl = document.getElementById("aside-time-min");
	var tmaxEl = document.getElementById("aside-time-max");
	var tout = document.getElementById("aside-time-out");
	if (timeWrap && tminEl && tmaxEl) {
		tmin = parseInt(tminEl.value, 10);
		tmax = parseInt(tmaxEl.value, 10);
		var syncTime = function () {
			var lo = parseInt(tminEl.value, 10);
			var hi = parseInt(tmaxEl.value, 10);
			if (lo > hi) {
				// Whichever thumb crossed the other is pushed back to meet it.
				if (this === tminEl) { hi = lo; tmaxEl.value = hi; }
				else { lo = hi; tminEl.value = lo; }
			}
			tmin = lo; tmax = hi;
			if (tout) { tout.textContent = lo + "–" + hi + " min"; }
			apply();
		};
		tminEl.addEventListener("input", syncTime);
		tmaxEl.addEventListener("input", syncTime);
	}

	apply();
})();
"##;

/// A query value made of the few characters this module's own links use.
///
/// No percent-decoding: every value read with this is one the page itself wrote, from a small
/// alphabet, so anything needing decoding is something else and reads as absent -- which is the
/// right answer to a value no link of ours produces.
pub fn query_word(query: &str, key: &str) -> Option<String> {
	for pair in query.split('&') {
		let mut kv = pair.splitn(2, '=');
		if kv.next() == Some(key) {
			let v = kv.next().unwrap_or("");
			if v.is_empty() || !v.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
				return None;
			}
			return Some(v.to_string());
		}
	}
	None
}

/// What the last comment attempt said, read out of the query a redirect landed with.
///
/// **A code, not a sentence.** The query is a thing anybody can put in a link and send to somebody
/// else, so carrying the message text in it would let a stranger make this site say whatever they
/// liked above its own comment form -- "your payment failed", say, over a plausible-looking URL. The
/// redirect carries a word this function knows, and the words themselves live here. A code this does
/// not know says nothing at all.
pub fn said_of(query: &str) -> Option<String> {
	for pair in query.split('&') {
		let mut kv = pair.splitn(2, '=');
		if kv.next() == Some("said") {
			return match kv.next().unwrap_or("") {
				"published"	=> Some(fmt!("Thank you — your comment is below.")),
				"held"		=> Some(fmt!(
					"Thank you — your comment has been sent to the author for review.")),
				"shut"		=> Some(fmt!("Comments are not open on this site.")),
				"edited"	=> Some(fmt!(
					"Your comment has been changed. A comment that was already published goes \
					back to the author to look at again.")),
				"noedit"	=> Some(fmt!(
					"That comment could not be changed. The few minutes for correcting it may \
					have passed.")),
				_		=> None,
			};
		}
	}
	None
}

/// The answer to a posted comment: back to the post, carrying what to tell the reader.
///
/// A redirect rather than a rendered page, so a reload does not post the comment again -- the same
/// reasoning the console's writes take. The fragment puts the reader at the conversation rather than
/// at the top of prose they have just read.
pub fn comment_posted(cfg: &PublishConfig, slug: &str, said: &str) -> HttpMessage {
	// `said` is one of the codes `said_of` knows, never a sentence. See its documentation.
	let to = fmt!("{}?said={}#comments", cfg.path_of(slug), percent_encode(said));
	let mut resp = HttpMessage::new_response(HttpStatus::SeeOther);
	resp = resp.with_field(HeaderName::Location, HeaderFieldValue::Generic(to));
	resp
}

/// The comment a request's cookie claims to have written, and the token it offers.
///
/// Read only; whether the token is any good is the caller's to check, since only the caller holds
/// the site's secret.
pub fn edit_claim(headers: &std::sync::Arc<HeaderFields>) -> Option<(String, String)> {
	use oxedyne_fe2o3_net::http::fields::HeaderFieldValue as V;
	if let Some(V::Cookie(cookies)) = headers.get_one(&HeaderName::Cookie) {
		for c in cookies {
			if c.key == "comment_edit" {
				let (id, token) = c.val.split_once('.')?;
				if id.is_empty() || token.is_empty() {
					return None;
				}
				return Some((id.to_string(), token.to_string()));
			}
		}
	}
	None
}

/// Attaches the token that lets a comment's author correct it.
///
/// A cookie, because it is the only thing a browser will carry back on its own and the alternative
/// is a token in a URL that lands in history, in a referrer and in anything the reader pastes. It
/// expires with the window, is `HttpOnly` so no script reads it, and names one comment.
pub fn with_edit_cookie(resp: HttpMessage, id: &str, token: &str) -> HttpMessage {
	let value = fmt!(
		"comment_edit={}.{}; Path=/; Max-Age={}; HttpOnly; SameSite=Lax",
		id, token, crate::srv::publish::comment::EDIT_WINDOW_SECS,
	);
	resp.with_field(HeaderName::SetCookie, HeaderFieldValue::Generic(value))
}

/// Percent-encodes what a redirect carries.
///
/// Only what has to be: a query value's own delimiters, and the characters a browser would otherwise
/// treat as structure. Everything else is left legible, since this lands in a URL a reader may see.
fn percent_encode(s: &str) -> String {
	let mut out = String::with_capacity(s.len());
	for b in s.bytes() {
		match b {
			b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~'
				=> out.push(b as char),
			b' '	=> out.push('+'),
			_	=> out.push_str(&fmt!("%{:02X}", b)),
		}
	}
	out
}

/// Reverses [`percent_encode`].
fn percent_decode(s: &str) -> String {
	let bytes = s.as_bytes();
	let mut out = Vec::with_capacity(bytes.len());
	let mut i = 0;
	while i < bytes.len() {
		match bytes[i] {
			b'+' => { out.push(b' '); i += 1; }
			b'%' if i + 2 < bytes.len() => {
				let hi = (bytes[i + 1] as char).to_digit(16);
				let lo = (bytes[i + 2] as char).to_digit(16);
				match (hi, lo) {
					(Some(h), Some(l))	=> { out.push((h * 16 + l) as u8); i += 3; }
					// Not a pair of hex digits, so not an escape: the byte stands as itself.
					_			=> { out.push(bytes[i]); i += 1; }
				}
			}
			b => { out.push(b); i += 1; }
		}
	}
	String::from_utf8_lossy(&out).to_string()
}
