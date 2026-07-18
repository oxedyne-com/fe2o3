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
	id:	&str,
)
	-> Outcome<HttpMessage>
{
	if path == cfg.path {
		return index(cfg, posts, id);
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
fn index(cfg: &PublishConfig, posts: &[Post], id: &str) -> Outcome<HttpMessage> {
	let mut body = String::new();
	body.push_str("<header class=\"aside-index-head\"><h1>");
	escape_text(&mut body, &cfg.title);
	body.push_str("</h1></header>\n<ul class=\"aside-index\">\n");
	for p in posts {
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
		body.push_str("</li>\n");
	}
	body.push_str("</ul>\n");

	if posts.is_empty() {
		body.push_str("<p class=\"aside-empty\">Nothing here yet.</p>\n");
	}

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
	body.push_str("</article>\n");

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
		let resp = res!(index(&cfg(), &posts, "test"));
		let body = String::from_utf8_lossy(&resp.body).to_string();
		assert_eq!(status_of(&resp), Some(HttpStatus::OK));
		assert!(body.contains(r#"<a href="/asides/on-rent">On rent</a>"#), "got: {}", body);
		assert!(body.contains("An opening sentence."), "no excerpt: {}", body);
		Ok(())
	}

	/// An index with nothing in it says so, rather than being a blank page that looks broken.
	#[test]
	fn test_an_empty_index_says_so_04() -> Outcome<()> {
		let resp = res!(index(&cfg(), &[], "test"));
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
			let resp = res!(handle_get(&cfg(), &posts, &path, "test"));
			assert_eq!(status_of(&resp), Some(HttpStatus::NotFound), "'{}' was not refused", bad);
		}
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
