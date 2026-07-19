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
	Post,
	PublishConfig,
	date_text,
	normalise_tag,
};

#[cfg(test)]
use crate::srv::publish::{
	PostKind,
	Source,
};

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_net::http::{
	fields::{
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
	cfg:	&PublishConfig,
	posts:	&[Post],
	path:	&str,
	query:	&str,
	id:	&str,
)
	-> Outcome<HttpMessage>
{
	if path == cfg.path {
		return index(cfg, posts, query, id);
	}
	if path == cfg.feed_path() {
		return super::feed::serve(cfg, posts, id);
	}
	if path == cfg.json_path() {
		return super::json::serve(cfg, posts, id);
	}
	// Everything else under the prefix names a post. The slug is what a reader put in a URL, so it is
	// checked before it is used: a name is letters, digits, a dash or an underscore.
	let slug = &path[cfg.path.len() + 1..];
	if !is_slug(slug) {
		info!("{}: publish: '{}' is not a name a post may wear", id, slug);
		return Ok(not_found(cfg));
	}
	match posts.iter().find(|p| p.slug == slug) {
		Some(post)	=> post_page(cfg, post),
		None		=> {
			info!("{}: publish: no post '{}'", id, slug);
			Ok(not_found(cfg))
		}
	}
}

/// Whether a string is a name a post may wear.
fn is_slug(s: &str) -> bool {
	!s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// The index: every post, newest first, as a link and its opening.
///
/// A `?tag=` in the query narrows it to the posts carrying that tag, so a tag link on a card or a
/// post is never a dead end. A narrowed index names the tag it is filtered by, so a reader knows
/// they are looking at a slice rather than the whole.
fn index(cfg: &PublishConfig, posts: &[Post], query: &str, id: &str) -> Outcome<HttpMessage> {
	// The facet the index is narrowed to, normalised the way a stored tag is, so `?tag=Rust` finds
	// the posts tagged `rust`.
	let filter = tag_filter(query);
	let shown: Vec<&Post> = match &filter {
		Some(t)	=> posts.iter().filter(|p| p.tags.iter().any(|x| x == t)).collect(),
		None	=> posts.iter().collect(),
	};

	let mut body = String::new();
	body.push_str("<header class=\"aside-index-head\"><h1>");
	escape_text(&mut body, &cfg.title);
	body.push_str("</h1>");
	if let Some(t) = &filter {
		body.push_str("<p class=\"aside-index-tag\">Tagged <span class=\"tag\">");
		escape_text(&mut body, t);
		body.push_str("</span></p>");
	}
	body.push_str("</header>\n<ul class=\"aside-index\">\n");
	for p in &shown {
		body.push_str("<li class=\"aside-index-item\">");
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

	if shown.is_empty() {
		body.push_str("<p class=\"aside-empty\">Nothing here yet.</p>\n");
	}

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

	info!("{}: publish: index, {} posts", id, shown.len());

	let head = Head {
		title:		cfg.title.clone(),
		description:	String::new(),
		url:		cfg.url_of(&cfg.path),
		kind:		"website",
		date:		None,
	};
	Ok(html_response(HttpStatus::OK, &page(cfg, &head, &body, None)))
}

/// The tag a `?tag=` query narrows the index to, normalised as a stored tag is.
///
/// Read from the raw query with no percent-decoding: a tag is `[a-z0-9-]`, which carries nothing a
/// query string would encode, so a value that needed decoding is a value no post is tagged with and
/// narrows to nothing -- which is the right answer to a tag that does not exist.
fn tag_filter(query: &str) -> Option<String> {
	for pair in query.split('&') {
		let mut kv = pair.splitn(2, '=');
		let k = kv.next().unwrap_or("");
		let v = kv.next().unwrap_or("");
		if k == "tag" {
			let t = normalise_tag(v);
			if t.is_empty() {
				return None;
			}
			return Some(t);
		}
	}
	None
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

/// One post.
fn post_page(cfg: &PublishConfig, post: &Post) -> Outcome<HttpMessage> {
	let mut body = String::new();
	body.push_str("<article class=\"aside\">\n");
	if let Some(d) = &post.date {
		body.push_str("<div class=\"aside-date\"><time datetime=\"");
		escape_attr(&mut body, d);
		body.push_str("\">");
		escape_text(&mut body, &date_text(d));
		body.push_str("</time></div>\n");
	}
	// The prose was escaped where it was rendered.
	body.push_str(&post.html);
	// The tags, in the article's footer, each a link back to the index narrowed to that tag. Omitted
	// entirely for a post with none.
	body.push_str(&tags_list(cfg, post));
	body.push_str("</article>\n");

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
			newsletter_from:	String::new(),
		}
	}

	fn post() -> Post {
		Post {
			slug:		fmt!("on-rent"),
			title:		fmt!("On rent"),
			kind:		PostKind::Note,
			date:		Some(fmt!("2026-07-17")),
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
		let resp = res!(post_page(&cfg(), &post()));
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
		let resp = res!(post_page(&cfg(), &p));
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
		let resp = res!(post_page(&cfg(), &p));
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
		let resp = res!(index(&cfg(), &posts, "", "test"));
		let body = String::from_utf8_lossy(&resp.body).to_string();
		assert_eq!(status_of(&resp), Some(HttpStatus::OK));
		assert!(body.contains(r#"<a href="/asides/on-rent">On rent</a>"#), "got: {}", body);
		assert!(body.contains("An opening sentence."), "no excerpt: {}", body);
		Ok(())
	}

	/// An index with nothing in it says so, rather than being a blank page that looks broken.
	#[test]
	fn test_an_empty_index_says_so_04() -> Outcome<()> {
		let resp = res!(index(&cfg(), &[], "", "test"));
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
			let resp = res!(handle_get(&cfg(), &posts, &path, "", "test"));
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
		let resp = res!(post_page(&cfg(), &p));
		let body = String::from_utf8_lossy(&resp.body).to_string();
		assert!(body.contains(r#"<ul class="post-tags">"#), "no tag list: {}", body);
		assert!(body.contains(r#"<a class="tag" href="/asides?tag=rust">rust</a>"#), "got: {}", body);
		assert!(body.contains(r#"<a class="tag" href="/asides?tag=web">web</a>"#), "got: {}", body);

		// A post with no tags has no empty shell.
		let resp = res!(post_page(&cfg(), &post()));
		let body = String::from_utf8_lossy(&resp.body).to_string();
		assert!(!body.contains("post-tags"), "an untagged post drew a tag element: {}", body);
		Ok(())
	}

	/// A `?tag=` narrows the index to the posts wearing that tag, and names the tag it narrowed by.
	#[test]
	fn test_the_index_filters_by_tag_07() -> Outcome<()> {
		let mut a = post();
		a.slug = fmt!("tagged");
		a.title = fmt!("Tagged");
		a.tags = vec![fmt!("rust")];
		let mut b = post();
		b.slug = fmt!("untagged");
		b.title = fmt!("Untagged");
		let posts = vec![a, b];

		let resp = res!(index(&cfg(), &posts, "tag=rust", "test"));
		let body = String::from_utf8_lossy(&resp.body).to_string();
		assert!(body.contains(r#"<a href="/asides/tagged">Tagged</a>"#), "the tagged post is missing: {}", body);
		assert!(!body.contains(">Untagged</a>"), "an untagged post survived the filter: {}", body);
		assert!(body.contains(r#"<span class="tag">rust</span>"#), "the filter is not named: {}", body);

		// A `?tag=` naming no post's tag narrows to nothing and says so.
		let resp = res!(index(&cfg(), &posts, "tag=nonesuch", "test"));
		let body = String::from_utf8_lossy(&resp.body).to_string();
		assert!(body.contains("Nothing here yet."), "an empty filter did not say so: {}", body);
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
}
