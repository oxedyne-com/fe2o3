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
//! A post dated only to the day is published at midnight UTC on the day it names. A day is not an
//! instant and the feed must claim one, so this is a fiction -- but a stable one: it does not drift,
//! it does not depend on where the server is, and re-serving a feed never reorders it.
//!
//! A post dated to the minute is published at that minute, and needs no fiction. Both are read as
//! UTC, because a post carries no zone and inventing one from where the server happens to be would
//! make the same post's feed entry move when the server did.

use crate::srv::cache;
use crate::srv::publish::{
	DATE_LEN,
	Post,
	PublishConfig,
	STAMP_LEN,
};

#[cfg(test)]
use crate::srv::publish::valid_date;

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
		// One category per tag, so a reader's feed reader can file the entry by the same tags the site
		// shows. The term is escaped for an attribute, since a tag reaches the feed as the store kept it.
		for t in &p.tags {
			s.push_str("    <category term=\"");
			escape_attr(&mut s, t);
			s.push_str("\"/>\n");
		}
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
	// A feed reader polls this on a schedule; a store answering from a copy would defeat the poll.
	Ok(cache::generated(resp))
}

/// A post's date, as the instant the feed claims for it.
///
/// A date naming a day becomes midnight UTC on it; a date naming a minute becomes that minute. Both
/// are already ISO, so both are a suffix away from RFC 3339 and neither needs a calendar.
///
/// Anything else is passed through as the epoch rather than emitted malformed: a feed that will not
/// parse is worse than one that admits it does not know. **That fallback is silent**, which is why
/// the length is tested against the shapes [`valid_date`] admits rather than against a bare `10` --
/// a date the store accepts and the feed quietly dates to 1970 is the kind of wrong nobody sees
/// until a reader's feed reader has already sorted it to the bottom for a year.
fn instant(date: &str) -> String {
	match date.len() {
		DATE_LEN	=> fmt!("{}T00:00:00Z", date),
		STAMP_LEN	=> fmt!("{}:00Z", date),
		_		=> EPOCH.to_string(),
	}
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

	/// A date naming a minute is that minute, and needs no fiction about when the day began.
	#[test]
	fn test_a_minute_is_the_minute_02() -> Outcome<()> {
		assert_eq!(instant("2026-07-17T14:30"), "2026-07-17T14:30:00Z");
		assert_eq!(instant("2026-07-17T00:00"), "2026-07-17T00:00:00Z");
		Ok(())
	}

	/// Every shape the store accepts is a shape the feed dates properly.
	///
	/// The pairing that matters: [`valid_date`] decides what may be stored and this decides what a
	/// reader's feed reader is told, and the two agreeing is not automatic. A date the store took
	/// and the feed dated to 1970 would sort to the bottom of every reader in the world and say
	/// nothing about it here.
	#[test]
	fn test_the_feed_dates_everything_the_store_takes_03() -> Outcome<()> {
		for d in ["2026-07-17", "2026-07-17T14:30"] {
			assert!(valid_date(d), "the store would refuse {}", d);
			assert_ne!(instant(d), EPOCH, "the store takes {} and the feed dates it to 1970", d);
			assert!(instant(d).ends_with('Z'), "{} did not become an instant", d);
		}
		Ok(())
	}

	/// Anything that is not a date says the epoch rather than producing a feed that will not parse.
	#[test]
	fn test_a_non_date_says_the_epoch_01() -> Outcome<()> {
		assert_eq!(instant("whenever"), EPOCH);
		assert_eq!(instant(""), EPOCH);
		// Ten characters of the wrong kind still reach the suffix, and the calendar is nobody's
		// business here -- the shape is all this claims to know.
		assert_eq!(instant("2026-02-31"), "2026-02-31T00:00:00Z");
		Ok(())
	}
}
