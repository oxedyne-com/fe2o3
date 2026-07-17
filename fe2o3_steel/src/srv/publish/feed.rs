//! The posts as a feed, for a reader that subscribes rather than visits.
//!
//! # Atom, not RSS
//!
//! Because of the dates. RSS's `pubDate` is an RFC 822 date -- `Thu, 17 Jul 2026 00:00:00 GMT` --
//! and that leading day name is a calendar calculation: to emit it, this would have to work out which
//! day of the week a date fell on. Atom's `updated` is ISO 8601, `2026-07-17T00:00:00Z`, which a post
//! named `2026-07-17-on-rent.md` is already most of the way to.
//!
//! So RSS would mean owning a calendar here, or taking a dependency for one field. Atom means neither,
//! and every reader worth having reads it.
//!
//! # A date without a time
//!
//! A post is dated by its file to the day, and a day is not an instant. Every post is therefore
//! published at midnight UTC on the day it names. That is a fiction, but a stable one: it does not
//! drift, it does not depend on where the server is, and re-serving a feed never reorders it.

use crate::srv::publish::{
	Post,
	PublishConfig,
};

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_net::http::{
	fields::{
		HeaderFieldValue,
		HeaderName,
	},
	msg::HttpMessage,
};
use oxedyne_fe2o3_text::doc::html::{
	escape_attr,
	escape_text,
};


/// The instant an undated post, or an empty feed, claims.
///
/// A feed must say when it was last updated, and one with nothing in it has never been. The epoch says
/// that plainly and never looks like a real time that happens to be wrong.
const EPOCH: &str = "1970-01-01T00:00:00Z";


/// Serves the feed.
pub fn serve(cfg: &PublishConfig, posts: &[Post], id: &str) -> Outcome<HttpMessage> {
	let self_url = cfg.url_of(&cfg.feed_path());
	let index_url = cfg.url_of(&cfg.path);

	let mut s = String::new();
	s.push_str("<?xml version=\"1.0\" encoding=\"utf-8\"?>\n");
	s.push_str("<feed xmlns=\"http://www.w3.org/2005/Atom\">\n");

	s.push_str("  <title>");
	escape_text(&mut s, &cfg.title);
	s.push_str("</title>\n");

	s.push_str("  <id>");
	escape_text(&mut s, &index_url);
	s.push_str("</id>\n");

	s.push_str("  <link rel=\"alternate\" type=\"text/html\" href=\"");
	escape_attr(&mut s, &index_url);
	s.push_str("\"/>\n");
	s.push_str("  <link rel=\"self\" type=\"application/atom+xml\" href=\"");
	escape_attr(&mut s, &self_url);
	s.push_str("\"/>\n");

	// The feed is as new as its newest post, which is the first, the list being newest first.
	let newest = posts.first()
		.and_then(|p| p.date.as_ref())
		.map(|d| instant(d))
		.unwrap_or_else(|| EPOCH.to_string());
	s.push_str("  <updated>");
	s.push_str(&newest);
	s.push_str("</updated>\n");

	if !cfg.site_name.is_empty() {
		s.push_str("  <author><name>");
		escape_text(&mut s, &cfg.site_name);
		s.push_str("</name></author>\n");
	}

	for p in posts {
		let url = cfg.url_of(&cfg.path_of(&p.slug));
		s.push_str("  <entry>\n    <title>");
		escape_text(&mut s, &p.title);
		s.push_str("</title>\n    <id>");
		escape_text(&mut s, &url);
		s.push_str("</id>\n    <link rel=\"alternate\" type=\"text/html\" href=\"");
		escape_attr(&mut s, &url);
		s.push_str("\"/>\n    <updated>");
		s.push_str(&p.date.as_ref().map(|d| instant(d)).unwrap_or_else(|| EPOCH.to_string()));
		s.push_str("</updated>\n");
		if !p.excerpt.is_empty() {
			s.push_str("    <summary>");
			escape_text(&mut s, &p.excerpt);
			s.push_str("</summary>\n");
		}
		// The whole post travels with the entry, so a reader who subscribed can read without coming
		// back. Escaped rather than wrapped in CDATA: CDATA cannot carry `]]>` and prose can.
		s.push_str("    <content type=\"html\">");
		escape_text(&mut s, &p.html);
		s.push_str("</content>\n  </entry>\n");
	}

	s.push_str("</feed>\n");

	info!("{}: publish: feed, {} entries", id, posts.len());

	let mut resp = HttpMessage::ok_respond_with_text(s);
	resp = resp.with_field(
		HeaderName::ContentType,
		HeaderFieldValue::Generic(fmt!("application/atom+xml; charset=utf-8")),
	);
	Ok(resp)
}

/// A date the file named, as the instant the feed claims for it.
///
/// The shape was checked when the name was split, so a date here is ten characters of the right kind.
/// Anything else is passed through as the epoch rather than emitted malformed: a feed that will not
/// parse is worse than one that admits it does not know.
fn instant(date: &str) -> String {
	if date.len() != 10 {
		return EPOCH.to_string();
	}
	let mut s = date.to_string();
	s.push_str("T00:00:00Z");
	s
}

#[cfg(test)]
mod tests {
	use super::*;

	/// A date becomes the instant its day began, in UTC, whoever is asking.
	#[test]
	fn test_a_date_becomes_midnight_utc_00() -> Outcome<()> {
		assert_eq!(instant("2026-07-17"), "2026-07-17T00:00:00Z");
		Ok(())
	}

	/// Anything that is not a date says the epoch rather than producing a feed that will not parse.
	#[test]
	fn test_a_non_date_says_the_epoch_01() -> Outcome<()> {
		assert_eq!(instant("whenever"), EPOCH);
		assert_eq!(instant(""), EPOCH);
		Ok(())
	}
}
