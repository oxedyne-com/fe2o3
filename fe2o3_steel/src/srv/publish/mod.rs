//! Publishing the prose a site holds.
//!
//! A directory of Markdown becomes posts a site serves under a name of its own: real pages at real
//! URLs, a feed, and a JSON list for a page that would rather render them itself.
//!
//! # Why real pages matter here
//!
//! A post that exists only inside a page's JavaScript cannot be linked to, cannot be found, and
//! unfurls to nothing when it is pasted anywhere. Self-hosting prose in order to be read, and then
//! serving it in a form only a browser running scripts can see, gives up the thing it was for. So the
//! canonical form of a post is a page: a URL, HTML in the first response, and the tags a card is built
//! from.
//!
//! [`json`] serves the same posts for a page that wants them inline. It is the convenience;
//! [`page`] is the point.
//!
//! # What a file says
//!
//! A file names itself. `2026-07-17-on-rent.md` is the post `on-rent`, dated `2026-07-17`; a name
//! without a leading date is a post without one. The title is the document's own most prominent
//! heading, and the slug where it has no heading -- so a post says its title once, in the prose, and
//! nowhere else.
//!
//! There is no front matter, deliberately. A metadata block is a second little language to learn, to
//! parse and to get wrong, and everything above is already in the file or its name.
//!
//! # Where the posts live
//!
//! In a directory, for now, and in the vhost's database later. This module sits in the server process
//! and is handed the database already, so the move is a store behind [`read_all`] rather than a
//! rearrangement. A directory of Markdown is not a stand-in meanwhile: it is a real way to write, and
//! the file is the source either way.

pub mod comment;
pub mod dest;
pub mod feed;
pub mod json;
pub mod page;
pub mod send;
pub mod store;
pub mod subscribe;

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_datime::time::{
	CalClock,
	CalClockZone,
};
use oxedyne_fe2o3_iop_crypto::enc::Encrypter;
use oxedyne_fe2o3_iop_db::api::Database;
use oxedyne_fe2o3_iop_hash::api::Hasher;
use oxedyne_fe2o3_jdat::{
	prelude::*,
	id::NumIdDat,
};
use oxedyne_fe2o3_text::doc::{
	Block,
	Doc,
	djot,
	html,
	markdown,
	text_of,
};

use std::{
	fs,
	path::Path,
	sync::{
		Arc,
		RwLock,
	},
};


/// Directory holding the posts, where the config names no other.
pub const DIR_DEFAULT: &str = "./www/public/content/posts";

/// URL prefix the posts are served under, where the config names no other.
pub const PATH_DEFAULT: &str = "/posts";

/// The categories a site starts with where its config names none: a small, thematic taxonomy of the
/// kind a general blog keeps, the defined counterpart to the free-form tags. An operator narrows,
/// renames or empties this in one place. In config order, which is the order the filter draws them.
pub const CATEGORIES_DEFAULT: [&str; 6] =
	["Personal", "Technical", "Ideas", "Reviews", "Projects", "Announcements"];

/// The extension a post wears.
const EXT: &str = "md";

/// How much of a post's opening stands in for it in a card and a feed.
const EXCERPT_LEN: usize = 200;

/// Words a minute, for a post's reading time.
///
/// Two hundred is the low end of the range measured for silent reading of English prose, so the
/// estimate errs towards telling a reader a piece is longer than they will find it.
pub const READ_WPM: usize = 200;

/// How long a post of the given length takes to read, in whole minutes.
///
/// The one definition of reading time, so the badge a page shows and the slider a filter offers count
/// the same way. Rounded up, and never below one: "0 min read" tells a reader nothing they wanted to
/// know, and a slider whose floor is zero has a dead notch.
pub fn read_mins(words: usize) -> usize {
	words.div_ceil(READ_WPM).max(1)
}


/// Where a vhost's posts are kept.
///
/// Both produce the same [`Post`], so nothing downstream of the read knows which it was.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Source {
	/// A directory of Markdown. What a text editor writes, and where prose that already exists is.
	#[default]
	Dir,
	/// The vhost's database. What the composer writes, and the only source a draft can live in --
	/// a directory on a server holds no drafts, since putting one there would publish it.
	Store,
}

impl Source {

	/// The source a word names.
	pub fn of(s: &str) -> Outcome<Self> {
		match s {
			"dir"	=> Ok(Self::Dir),
			"store"	=> Ok(Self::Store),
			// Not a lenient default: a site that meant `store` and typed `stor` would silently serve a
			// directory instead, and discover it by noticing the wrong prose on its own front page.
			_	=> Err(err!(
				"PublishConfig: 'source' must be 'dir' or 'store', not '{}'.", s;
				Invalid, Input)),
		}
	}
}

/// A vhost's published prose: where the posts are, where they are served, and what the site calls
/// them.
///
/// Absent from a vhost, the vhost publishes nothing and none of these paths are served. Every field
/// has a default, so a config saying `"publish": {}` publishes an empty directory rather than failing
/// to load -- the shape of a config is not the place to discover a typo in a path.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PublishConfig {
	/// URL prefix the posts are served under, without a trailing slash.
	pub path:	String,
	/// Directory holding the Markdown, absolute or relative to the app root. Read when the source is
	/// a directory, and the place an import reads from when it is the store.
	pub dir:	String,
	/// Where the posts are kept.
	pub source:	Source,
	/// What the site calls its posts, as the index's heading and the feed's name.
	pub title:	String,
	/// The site's own name, for the card a shared link makes.
	pub site_name:	String,
	/// The site's canonical origin, e.g. `https://example.com`, without a trailing slash.
	///
	/// Needed rather than derived from the request, because a card's URLs and a feed's must be
	/// absolute, and a `Host` header is the client's word for where it thinks it is.
	pub base_url:	String,
	/// Stylesheets a page links, in order. A page carries no styling of its own: what prose should
	/// look like is the site's business, not the server's.
	pub css:	Vec<String>,
	/// The remotes this site is configured to post to, and the credentials to reach them. Empty where
	/// the site publishes only to its own pages, which is the default.
	pub creds:	send::DestCreds,
	/// The least seconds between two comments from one sender. `0` turns the interval off.
	///
	/// **Operational policy, not a constant**, because the thing being counted is an address and an
	/// address is not a person: a household, an office and a university share one. A site whose
	/// readers are behind shared addresses wants this low or off; one being flooded wants it high.
	/// It also does nothing useful where the server sits behind a proxy that does not pass the
	/// client's address through, since then every reader is one address.
	pub comment_rate_secs:		u64,
	/// How many comments one sender may leave in an hour. `0` turns the count off.
	pub comment_rate_hourly:	u32,
	/// Whether this site takes comments on its posts.
	///
	/// **Off unless a site asks for it.** A comment endpoint is an unauthenticated public write, and
	/// turning one on for every site that happens to publish prose -- which is what a default of `true`
	/// would do -- is not a decision this module gets to make on an operator's behalf. A `publish` block
	/// that names nothing takes no comments and serves no form.
	pub comments:	bool,
	/// The address the newsletter is sent from, e.g. `README <news@oxedyne.com>`. Empty falls back to
	/// the mail configuration's derived default (`news@<mail-domain>`), which is aligned with the DKIM
	/// signing domain so the signature authenticates. A `#[optional]` field: a `publish` block that
	/// names none still loads, and the newsletter takes the default.
	pub newsletter_from:	String,
	/// The categories a post may sit in: the site's defined taxonomy, the checkbox counterpart to the
	/// free-form tags. Drawn as the filter's category row and offered in the composer. An optional
	/// field: a `publish` block naming none takes [`CATEGORIES_DEFAULT`], so a site gets a sensible set
	/// without configuring one, and an operator narrows or renames it in one place.
	pub categories:	Vec<String>,
	/// The author a post carries when its own source names none -- chiefly a directory post, which has
	/// no front matter to hold one. Empty leaves such a post unattributed. A member's site-login
	/// username. An optional field.
	pub default_author:	String,
}

impl PublishConfig {

	/// Parses a vhost's `publish` block.
	pub fn from_datmap(m: &DaticleMap) -> Outcome<Self> {
		let get_str = |key: &str, default: &str| -> Outcome<String> {
			match m.get(&dat!(key)) {
				Some(Dat::Str(s))	=> Ok(s.clone()),
				None			=> Ok(default.to_string()),
				_			=> Err(err!(
					"PublishConfig: '{}' must be a string.", key;
					Invalid, Input, Mismatch)),
			}
		};

		// A count, however narrowly the grammar happened to type it. A bare `0` is not a `u64` to
		// the decoder -- it is the smallest thing that holds it -- so a match on one variant refuses
		// exactly the value an operator is most likely to write.
		let get_count = |key: &str, default: u64| -> Outcome<u64> {
			match m.get(&dat!(key)) {
				None			=> Ok(default),
				Some(Dat::U8(n))	=> Ok(*n as u64),
				Some(Dat::U16(n))	=> Ok(*n as u64),
				Some(Dat::U32(n))	=> Ok(*n as u64),
				Some(Dat::U64(n))	=> Ok(*n),
				Some(Dat::I8(n)) if *n >= 0	=> Ok(*n as u64),
				Some(Dat::I16(n)) if *n >= 0	=> Ok(*n as u64),
				Some(Dat::I32(n)) if *n >= 0	=> Ok(*n as u64),
				Some(Dat::I64(n)) if *n >= 0	=> Ok(*n as u64),
				_			=> Err(err!(
					"PublishConfig: '{}' must be a count of zero or more.", key;
					Invalid, Input, Mismatch)),
			}
		};

		let mut path = res!(get_str("path", PATH_DEFAULT));
		// A trailing slash would make every route below double it, and a prefix that is not rooted
		// would match nothing. Correct both rather than serve something subtly wrong.
		while path.ends_with('/') {
			path.pop();
		}
		if !path.starts_with('/') {
			path.insert(0, '/');
		}

		let mut base_url = res!(get_str("base_url", ""));
		while base_url.ends_with('/') {
			base_url.pop();
		}

		// A list and a vek are both written as a list of strings and both mean one, so both are read.
		// The rest of this config grammar accepts either, and a stylesheet list is no place to
		// discover that it does not.
		let strings = |items: &[Dat]| -> Outcome<Vec<String>> {
			let mut out = Vec::new();
			for item in items {
				match item {
					Dat::Str(s)	=> out.push(s.clone()),
					_		=> return Err(err!(
						"PublishConfig: every 'css' entry must be a string.";
						Invalid, Input, Mismatch)),
				}
			}
			Ok(out)
		};
		let css = match m.get(&dat!("css")) {
			Some(Dat::List(list))	=> res!(strings(list)),
			Some(Dat::Vek(vek))	=> res!(strings(vek.as_slice())),
			None			=> Vec::new(),
			_			=> return Err(err!(
				"PublishConfig: 'css' must be a list of strings.";
				Invalid, Input, Mismatch)),
		};

		let source = match m.get(&dat!("source")) {
			Some(Dat::Str(s))	=> res!(Source::of(s)),
			None			=> Source::default(),
			_			=> return Err(err!(
				"PublishConfig: 'source' must be a string.";
				Invalid, Input, Mismatch)),
		};

		let creds = match m.get(&dat!("destinations")) {
			Some(Dat::Map(dm))	=> res!(send::DestCreds::from_datmap(dm)),
			None			=> send::DestCreds::default(),
			_			=> return Err(err!(
				"PublishConfig: 'destinations' must be a map.";
				Invalid, Input, Mismatch)),
		};

		Ok(Self {
			path,
			dir:		res!(get_str("dir", DIR_DEFAULT)),
			source,
			title:		res!(get_str("title", "Posts")),
			site_name:	res!(get_str("site_name", "")),
			base_url,
			css,
			creds,
			// A site that names no From takes the mail default; the field is optional, so an existing
			// `publish` block that predates the newsletter still loads.
			comment_rate_secs:	res!(get_count("comment_rate_secs", 30)),
			comment_rate_hourly:	res!(get_count("comment_rate_hourly", 10)) as u32,
			comments:		match m.get(&dat!("comments")) {
				Some(Dat::Bool(b))	=> *b,
				None			=> false,
				_			=> return Err(err!(
					"PublishConfig: 'comments' must be true or false.";
					Invalid, Input, Mismatch)),
			},
			newsletter_from:	res!(get_str("newsletter_from", "")),
			// The taxonomy, or the built-in set where a site names none. A site that wants no categories
			// at all writes an empty list, which is distinct from naming none: the first is a deliberate
			// nothing, the second takes the default.
			categories:	{
				let cats = match m.get(&dat!("categories")) {
					Some(Dat::List(list))	=> res!(strings(list)),
					Some(Dat::Vek(vek))	=> res!(strings(vek.as_slice())),
					None			=> CATEGORIES_DEFAULT.iter().map(|c| c.to_string()).collect(),
					_			=> return Err(err!(
						"PublishConfig: 'categories' must be a list of strings.";
						Invalid, Input, Mismatch)),
				};
				// A category is joined into a comma-separated field in the composer and in the filter's
				// data attribute, so a comma inside a category name would split into two. A space is
				// fine, and common ("Book reviews"); a comma is refused here rather than left to corrupt
				// the field silently at a distance.
				for c in &cats {
					if c.contains(',') {
						return Err(err!(
							"PublishConfig: a category name may not contain a comma: '{}'.", c;
							Invalid, Input));
					}
				}
				cats
			},
			default_author:	res!(get_str("default_author", "")),
		})
	}

	/// Resolves every secret reference the config carries against the app root.
	///
	/// A destination's token or app password is written as an `{env:}` or `{file:}` reference, never in
	/// the clear, and is resolved once at startup -- the same treatment the SMTP submission password
	/// gets, and for the same reason: a secret in the config file is a secret in every backup of it.
	pub fn resolve_secrets(&mut self, root: &std::path::Path) -> Outcome<()> {
		res!(self.creds.resolve_secrets(root));
		Ok(())
	}

	/// Whether a request path belongs to the published prose.
	///
	/// The prefix and what sits under it, and nothing that merely begins with the same letters: a
	/// site publishing at `/asides` has not thereby claimed `/asides-are-great`.
	pub fn owns(&self, path: &str) -> bool {
		path == self.path
			|| (path.starts_with(&self.path)
				&& path.as_bytes().get(self.path.len()) == Some(&b'/'))
	}

	/// The absolute URL of a path under this site.
	pub fn url_of(&self, path: &str) -> String {
		let mut s = self.base_url.clone();
		s.push_str(path);
		s
	}

	/// The URL path of a post.
	pub fn path_of(&self, slug: &str) -> String {
		let mut s = self.path.clone();
		s.push('/');
		s.push_str(slug);
		s
	}

	/// The URL path of the feed.
	pub fn feed_path(&self) -> String {
		let mut s = self.path.clone();
		s.push_str("/feed.xml");
		s
	}

	/// The URL path of the JSON list.
	pub fn json_path(&self) -> String {
		let mut s = self.path.clone();
		s.push_str("/index.json");
		s
	}

	/// The URL path a sign-up posts to, and the themed form is served at.
	pub fn subscribe_path(&self) -> String {
		let mut s = self.path.clone();
		s.push_str("/subscribe");
		s
	}

	/// The URL path an edit to a comment is posted to.
	pub fn comment_edit_path(&self, slug: &str) -> String {
		let mut s = self.path.clone();
		s.push('/');
		s.push_str(slug);
		s.push_str("/comment/edit");
		s
	}

	/// The slug an edit path names, where it names one.
	pub fn comment_edit_slug<'a>(&self, path: &'a str) -> Option<&'a str> {
		let rest = path.strip_prefix(&self.path)?.strip_prefix('/')?;
		let slug = rest.strip_suffix("/comment/edit")?;
		if slug.is_empty() || !valid_slug(slug) {
			return None;
		}
		Some(slug)
	}

	/// The URL path a comment preview is asked for.
	pub fn comment_preview_path(&self, slug: &str) -> String {
		let mut s = self.path.clone();
		s.push('/');
		s.push_str(slug);
		s.push_str("/comment/preview");
		s
	}

	/// The slug a preview path names, where it names one.
	pub fn comment_preview_slug<'a>(&self, path: &'a str) -> Option<&'a str> {
		let rest = path.strip_prefix(&self.path)?.strip_prefix('/')?;
		let slug = rest.strip_suffix("/comment/preview")?;
		if slug.is_empty() || !valid_slug(slug) {
			return None;
		}
		Some(slug)
	}

	/// The URL path the comment form's script is served at.
	///
	/// A file rather than an inline block, so a site can run a Content-Security-Policy without
	/// `unsafe-inline`. An inline script forces every page that carries it to allow inline scripts,
	/// which switches off the one layer that would contain a mistake in the render policy -- and the
	/// same untrusted prose is rendered into the admin console, where a mistake would be worst.
	pub fn comment_js_path(&self) -> String {
		let mut s = self.path.clone();
		s.push_str("/comments.js");
		s
	}

	/// The URL path the index filter's script is served at. A file rather than an inline block, so a
	/// site can run a Content-Security-Policy that forbids inline script and still get the filter.
	pub fn filter_js_path(&self) -> String {
		let mut s = self.path.clone();
		s.push_str("/filter.js");
		s
	}

	/// The URL prefix a member's uploaded picture is served under.
	pub fn avatar_prefix(&self) -> String {
		let mut s = self.path.clone();
		s.push_str("/avatar/");
		s
	}

	/// The URL a member's uploaded picture is served at.
	///
	/// The path a profile stores once a member uploads one, so a byline points at this module rather
	/// than at a file somewhere on disk that a deploy could take away.
	pub fn avatar_path(&self, username: &str) -> String {
		let mut s = self.avatar_prefix();
		s.push_str(username);
		s
	}

	/// The URL path a comment on a post is posted to.
	///
	/// Under the post's own path rather than a shared endpoint, so which post is being commented on is
	/// carried by the URL and cannot be swapped for another in the body.
	pub fn comment_path(&self, slug: &str) -> String {
		let mut s = self.path.clone();
		s.push('/');
		s.push_str(slug);
		s.push_str("/comment");
		s
	}

	/// The slug a comment-posting path names, where it names one.
	pub fn comment_slug<'a>(&self, path: &'a str) -> Option<&'a str> {
		let rest = path.strip_prefix(&self.path)?.strip_prefix('/')?;
		let slug = rest.strip_suffix("/comment")?;
		if slug.is_empty() || !valid_slug(slug) {
			return None;
		}
		Some(slug)
	}

	/// The URL path a confirmation link points at, carrying the subscriber's token.
	pub fn confirm_path(&self, token: &str) -> String {
		let mut s = self.path.clone();
		s.push_str("/confirm?token=");
		s.push_str(token);
		s
	}

	/// The URL path an unsubscribe link points at, carrying the subscriber's token.
	pub fn unsubscribe_path(&self, token: &str) -> String {
		let mut s = self.path.clone();
		s.push_str("/unsubscribe?token=");
		s.push_str(token);
		s
	}

	/// Whether a request path is one of the subscription endpoints -- the sign-up, the confirm, or the
	/// unsubscribe -- so the reader dispatch can hand it to [`subscribe`] before it reads the posts.
	///
	/// The bare path only: the query, where the token rides, is matched apart. So `{path}/confirm` is
	/// this whether or not it carries a token, and the handler answers a missing one with the same
	/// bad-token page a wrong one gets.
	pub fn subscription_of(&self, path: &str) -> Option<Subscription> {
		if path == self.subscribe_path() {
			Some(Subscription::Subscribe)
		} else if path == self.confirm_bare_path() {
			Some(Subscription::Confirm)
		} else if path == self.unsubscribe_bare_path() {
			Some(Subscription::Unsubscribe)
		} else {
			None
		}
	}

	/// The confirm endpoint's path without its query.
	fn confirm_bare_path(&self) -> String {
		let mut s = self.path.clone();
		s.push_str("/confirm");
		s
	}

	/// The unsubscribe endpoint's path without its query.
	fn unsubscribe_bare_path(&self) -> String {
		let mut s = self.path.clone();
		s.push_str("/unsubscribe");
		s
	}

}

/// Which subscription endpoint a request named.
///
/// A small enum rather than three string comparisons at the call site, so the reader dispatch reads as
/// a match and a new endpoint is a new arm.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Subscription {
	/// `{path}/subscribe`: the sign-up form (GET) and where it posts (POST).
	Subscribe,
	/// `{path}/confirm`: a confirmation link followed.
	Confirm,
	/// `{path}/unsubscribe`: an unsubscribe link followed.
	Unsubscribe,
}


/// The longest a slug may be.
///
/// A key and a URL both hold one, and neither has a natural limit worth relying on. The number is
/// arbitrary; having one is not.
pub const SLUG_MAX: usize = 128;

/// Whether a word may be a post's name.
///
/// A slug is not decoration: it is pasted into a database key (`publish/post/<slug>`) and into a
/// URL, so a form's idea of one cannot be taken at its word. A slug carrying a slash would reach
/// past its own key and name a different post's, or a different thing entirely; one carrying a dot
/// pair would do the same to a path; one carrying a space or a quote would arrive somewhere as
/// something other than what was typed.
///
/// So the rule is a small alphabet rather than a list of what to reject: letters, digits, hyphen and
/// underscore. Anything a list of forbidden characters missed would be allowed by default, and the
/// thing about that mistake is that it does not announce itself.
pub fn valid_slug(s: &str) -> bool {
	!s.is_empty()
		&& s.len() <= SLUG_MAX
		&& s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// The marks in a user-agent that name a thing which is not a reader.
///
/// A substring list is a poor way to identify a browser and an adequate way to discard the obvious
/// machines, which is all this is for. It will miss a crawler that lies, and that is tolerable: a
/// tally is a rough shape, not a headcount, and a bot pretending to be Chrome will not be caught by
/// any list at all.
const BOT_MARKS: &[&str] = &[
	"bot", "crawl", "spider", "slurp", "archiver", "curl", "wget", "python-requests",
	"headlesschrome", "facebookexternalhit", "embedly", "preview", "monitor", "uptime",
	"scrapy", "feedfetcher", "pingdom", "lighthouse", "http-client",
];

/// Whether a user-agent names something that is not a person reading.
///
/// Lowercased before the comparison, because a user-agent's capitalisation is the sender's choice.
/// An absent or empty user-agent counts as a bot: every real browser sends one, and a request with
/// none is a script that did not bother.
pub fn looks_automated(ua: Option<&str>) -> bool {
	let ua = match ua {
		Some(s) if !s.trim().is_empty()	=> s.to_lowercase(),
		_				=> return true,
	};
	BOT_MARKS.iter().any(|m| ua.contains(m))
}

/// Whether a request for a post should add one to its tally.
///
/// Three exclusions, and each is here for a reason worth keeping:
///
/// - **A request carrying a management session is the author.** A count that climbs while its author
///   re-reads their own draft measures the author's attention, not a reader's, and is worse than no
///   count because it looks like one.
/// - **A request from an obvious machine is not a read.** See [`looks_automated`].
/// - **A `HEAD` asked for no prose.** It is how a monitor checks the site is up and how a chat client
///   fetches a link preview, several times an hour and forever. Counted, the tally would measure the
///   monitor.
///
/// There is deliberately nothing here about *who* the reader is: no identifier is derived, stored or
/// compared, so two reads by one person count twice and the site never learns they were one person.
/// That is the trade this counter makes on purpose -- it is a tally of readings, not of readers.
pub fn counts_as_read(has_manage_session: bool, user_agent: Option<&str>, head_only: bool) -> bool {
	!head_only && !has_manage_session && !looks_automated(user_agent)
}

/// The longest a tag may be, once normalised.
///
/// A tag is a facet in a URL and a word on a card, neither with a natural limit worth relying on.
/// The number is arbitrary; having one is not.
pub const TAG_MAX: usize = 32;

/// Whether a word may be a tag, once normalised.
///
/// A small alphabet rather than a reject-list, on the same reasoning as [`valid_slug`]: lowercase
/// letters, digits and the hyphen, and nothing else. A tag is pasted into a query (`?tag=rust`) and
/// shown to a reader, so a space, a slash or a capital in one would be a tag that reaches somewhere
/// as something other than it looks.
///
/// The check is against the normalised form, so `valid_tag` normalises first and a caller may pass
/// what a person typed: `Rust` normalises to `rust` and passes, `a b` normalises to `a b` and does
/// not -- a space is dropped, not guessed into a hyphen.
pub fn valid_tag(s: &str) -> bool {
	let t = normalise_tag(s);
	!t.is_empty()
		&& t.len() <= TAG_MAX
		&& t.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// A tag as the store keeps it, from a tag as a person typed it.
///
/// Trimmed and lowercased, so `Rust` and ` rust ` are the one tag `rust`. A space is left as a
/// space, deliberately: [`valid_tag`] then drops such a tag rather than this guessing that a space
/// meant a hyphen and minting a tag the author did not type.
pub fn normalise_tag(s: &str) -> String {
	s.trim().to_lowercase()
}

/// The tags a record keeps, from the comma-separated field a form said.
///
/// Split on commas, each normalised and validated, the invalid dropped in silence -- a small
/// alphabet, not a reject-list, as [`valid_tag`] is -- and deduped keeping first appearance. Empty
/// or whitespace gives no tags.
pub fn parse_tags(s: &str) -> Vec<String> {
	let mut out: Vec<String> = Vec::new();
	for part in s.split(',') {
		let t = normalise_tag(part);
		if !valid_tag(&t) {
			continue;
		}
		if !out.iter().any(|x| x == &t) {
			out.push(t);
		}
	}
	out
}

/// The length of a date that names a day.
pub const DATE_LEN: usize = 10;

/// The length of a date that names a minute.
pub const STAMP_LEN: usize = 16;

/// Whether a word may be a post's date.
///
/// `YYYY-MM-DD`, the shape [`split_date`] reads out of a filename -- or `YYYY-MM-DDTHH:MM`, which is
/// the same day with a minute on it. Both are ISO 8601, which is what the feed and `<time>` need:
/// Atom's dates are ISO, and a date that is not one reaches a reader's feed reader as a malformed
/// entry rather than as an error anyone here would see. Empty is allowed -- a post without a date is
/// a post, and says so by carrying none.
///
/// # Why a minute, when a filename only ever said a day
///
/// Because a day is not an order. Posts sort by date, and two posts of one day fall back to sorting
/// by slug -- alphabetically, which is to say arbitrarily, and not at all by which was written
/// first. A directory could not say more than the day, since the date was in the filename and there
/// is no front matter to put a time in. A record can: its date is a field.
///
/// It matters most where most of the writing is. A note is the thing an author writes most, and
/// several notes in a day is the ordinary case for the form, so the day-only date was weakest
/// exactly where the module expects the traffic.
///
/// A space is accepted where the `T` goes, because that is how a person writes a date;
/// [`normalise_date`] takes it at the door so one shape reaches the store.
///
/// # What is not checked
///
/// The calendar. `2026-02-31` passes, and so does `2026-07-17T99:99`. Refusing either means owning a
/// calendar and a clock, which is the dependency this module does not have and the reason the feed
/// is Atom rather than RSS. A date that is shaped right and means nothing is the author's typo to
/// see, and it is visible -- it is printed on the post.
pub fn valid_date(s: &str) -> bool {
	if s.is_empty() {
		return true;
	}
	let b = s.as_bytes();
	if b.len() != DATE_LEN && b.len() != STAMP_LEN {
		return false;
	}
	let day = b[..DATE_LEN].iter().enumerate().all(|(i, c)| {
		match i {
			4 | 7	=> *c == b'-',
			_	=> c.is_ascii_digit(),
		}
	});
	if !day || b.len() == DATE_LEN {
		return day;
	}
	// `T14:30`, or ` 14:30` from a person who wrote it the way people do.
	(b[10] == b'T' || b[10] == b' ')
		&& b[11].is_ascii_digit()
		&& b[12].is_ascii_digit()
		&& b[13] == b':'
		&& b[14].is_ascii_digit()
		&& b[15].is_ascii_digit()
}

/// Today, as a post writes a date: `2026-07-22`, in UTC.
///
/// What an author who gave no date meant. A post reaches a feed, and Atom requires every entry to
/// say when it was updated -- so an undated post is not a post without a date on the page, it is a
/// post the feed has to invent one for, and the invention was the epoch. That sorts the piece below
/// everything written since 1970 in every reader in the world, silently. Dating it on the way in
/// costs the author nothing (the field is filled in for them, and they may change it) and is the
/// only version of the story where nothing downstream has to guess.
///
/// [`CalClock`] does the civil arithmetic, per the fe2o3 calendar. A clock that will not read gives
/// nothing rather than a wrong day: the caller then keeps the post undated, which is the behaviour
/// that has always been there.
pub fn today() -> Option<String> {
	let secs = match std::time::SystemTime::now()
		.duration_since(std::time::UNIX_EPOCH)
	{
		Ok(d)	=> d.as_secs() as i64,
		Err(_)	=> return None,
	};
	let cc = match CalClock::from_unix_timestamp_seconds(secs, CalClockZone::utc()) {
		Ok(cc)	=> cc,
		Err(_)	=> return None,
	};
	Some(fmt!("{:04}-{:02}-{:02}", cc.year(), cc.month(), cc.day()))
}

/// The date a record keeps, from the date a form said.
///
/// One shape in the store, so nothing downstream has to know there were two. ISO puts a `T` between
/// the day and the hour and people put a space, so the space is taken here rather than handled
/// everywhere after here.
pub fn normalise_date(s: &str) -> String {
	let s = s.trim();
	if s.len() == STAMP_LEN && s.as_bytes()[10] == b' ' {
		let mut out = s.to_string();
		out.replace_range(10..11, "T");
		return out;
	}
	s.to_string()
}

/// The date a person reads, from the date a record keeps.
///
/// The stored form is ISO, so a post dated to the minute carries a `T` in the middle of it. That is
/// for a machine. A reader gets a space, and the `T` form stays in the `datetime` attribute beside
/// it, which is the whole point of `<time>` having both.
pub fn date_text(date: &str) -> String {
	date.replacen('T', " ", 1)
}


/// The syntax a post is written in.
///
/// Both read into the same document tree, so a post's kind of markup is a fact about how it was
/// typed and nothing a reader ever sees: the page, the feed and the excerpt are made from the tree,
/// which knows neither. What Djot buys an author over Markdown is the power to name a box (`:::`) and
/// a style (`{.class}`) in the prose itself, which Markdown has no syntax for. A post says which it
/// is so the two can sit side by side in one store, each read by its own front-end.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Markup {
	/// Markdown -- the form most prose is already written in, and the default.
	#[default]
	Markdown,
	/// Djot -- for prose that wants to name a box or a style.
	Djot,
}

impl Markup {

	/// The word a record stores.
	pub fn as_str(&self) -> &'static str {
		match self {
			Self::Markdown	=> "markdown",
			Self::Djot	=> "djot",
		}
	}

	/// The markup a word names. An unknown word is Markdown, the safe default: it is what nearly every
	/// post is, and a record this version cannot place should read as the ordinary thing rather than
	/// the exception.
	pub fn of(s: &str) -> Self {
		match s {
			"djot"	=> Self::Djot,
			_	=> Self::Markdown,
		}
	}
}

/// Whether a post is anybody's business but its author's.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum PostState {
	/// Written, not published. Served to nobody.
	#[default]
	Draft,
	/// Published.
	Live,
}

impl PostState {

	/// The word a record stores.
	pub fn as_str(&self) -> &'static str {
		match self {
			Self::Draft	=> "draft",
			Self::Live	=> "live",
		}
	}

	/// The state a word names. **An unknown word is a draft**, deliberately: a record this version
	/// cannot make sense of should not thereby become published. The safe reading of a state nobody
	/// understands is that it is not ready.
	pub fn of(s: &str) -> Self {
		match s {
			"live"	=> Self::Live,
			_	=> Self::Draft,
		}
	}
}

/// An author, as a reader is shown one: the login username a post stores, resolved to a display name,
/// an avatar and a public handle through the member's profile.
///
/// What the filter draws a face from, and what a post's byline reads. Built by
/// [`store::resolve_authors`] from a username and its profile, so the page layer never touches the
/// database to draw an author.
///
/// # The username never reaches a page
///
/// It is the SHA-256 of the member's passphrase. Anything public derived from it -- the whole hash, a
/// prefix of it, a hash of it -- is a verifier a guess can be tested against, offline and unwatched,
/// and a page is read by anyone. So [`handle`](Author::handle) is what is drawn and matched on, and
/// `username` stays on the server side of the resolution: it is here only to pair an author with the
/// posts that name them.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Author {
	/// The login username a post stores. Matched against a post's `author` **on the server**, and
	/// never rendered, emitted or put in a path. See the note above.
	pub username:	String,
	/// The name this author wears in public: the profile's handle, or a name made for the page where
	/// the member has no profile yet. What a page matches a post to an author by.
	pub handle:	String,
	/// The name a reader sees. The profile's name, or `Anonymous` where the profile named none --
	/// never the username, which is not a name and is not for showing.
	pub name:	String,
	/// A path or URL to the avatar image. Empty draws an initial from the name instead.
	pub avatar:	String,
	/// What the author says they write about. Shown above the posts and under a byline; empty shows
	/// nothing, rather than an empty box where a description would be.
	pub bio:	String,
}

impl Author {

	/// An author from a username and the profile it resolved to, applying the fallbacks: an unnamed
	/// member is `Anonymous`, an avatarless one lets the reader draw an initial, and one who has never
	/// saved a profile takes the handle the caller made for the page.
	///
	/// `spare_handle` is used only where the profile holds none -- a member who has written a post but
	/// never opened their profile. It must not be derived from the username; callers pass a position
	/// on the page, which identifies an author within one rendering and says nothing anywhere else.
	pub fn from_profile(username: &str, profile: &store::Profile, spare_handle: &str) -> Self {
		Self {
			username:	username.to_string(),
			handle:		if profile.handle.is_empty() {
				spare_handle.to_string()
			} else {
				profile.handle.clone()
			},
			name:		if profile.name.is_empty() {
				fmt!("Anonymous")
			} else {
				profile.name.clone()
			},
			avatar:		profile.avatar.clone(),
			bio:		profile.bio.clone(),
		}
	}

	/// The initial a reader draws where the author has no avatar: the first character of the display
	/// name, upper-cased, or `?` where even that is empty.
	pub fn initial(&self) -> String {
		self.name.chars().next()
			.map(|c| c.to_uppercase().to_string())
			.unwrap_or_else(|| "?".to_string())
	}
}


/// One post, as a reader gets it.
#[derive(Clone, Debug)]
pub struct Post {
	/// The post's name in a URL.
	pub slug:	String,
	/// The post's own most prominent heading, or its slug where it has none.
	pub title:	String,
	/// The member who wrote it, by their site-login username. Empty where none is named. Resolved to a
	/// display name and avatar through the member's profile at the point it is drawn.
	pub author:	String,
	/// The categories the post sits in, from the site's configured set. Empty for an uncategorised
	/// post. The free-form counterpart is [`tags`](Post::tags).
	pub categories:	Vec<String>,
	/// The date it was given, where it was given one.
	pub date:	Option<String>,
	/// The opening of the prose, flattened, for a card and a feed.
	pub excerpt:	String,
	/// The prose, rendered.
	pub html:	String,
	/// How many words the prose runs to, for the reading time shown above it. Counted from the tree
	/// rather than the rendered HTML, so no tag or attribute is mistaken for a word.
	pub words:	usize,
	/// Where else the post has been published, as a destination and the permalink it landed at. Drawn
	/// on the page as "also on …", so a reader can follow the post to where the conversation is. Empty
	/// for a post read from a directory, which records no deliveries, and for one sent nowhere.
	pub also_on:	Vec<(dest::Destination, String)>,
	/// The tags the post carries, normalised. Drawn on the page and the card as tag links, one per
	/// entry in the feed. Empty for a post with none, and for one read from a directory.
	pub tags:	Vec<String>,
}


/// Reads every post in a directory, newest first.
///
/// A file that will not read or will not parse is passed over with a complaint in the log rather than
/// failing the lot: one broken post should not take the others off the page, and the log is where its
/// author will look. The directory itself failing is a different thing, and is an error -- an empty
/// shelf looks like the truth and is not.
pub fn read_all(dir: &str, id: &str) -> Outcome<Vec<Post>> {
	let sources = res!(read_sources(dir, id));

	let mut posts = Vec::new();
	for (stem, source) in sources {
		let (date, slug) = split_date(&stem);
		// A file on disk is not a draft, or it would not be on the disk of a server: everything in a
		// directory is live. A directory of files is Markdown: it is the form prose already exists in,
		// and a file on disk carries no field to name an author, a category or another markup.
		match render_source(&source, slug, date, Markup::Markdown) {
			Ok(p)	=> posts.push(p),
			Err(e)	=> warn!(
				"{}: posts: skipping '{}', which will not read as Markdown: {}", id, stem, e),
		}
	}

	// Newest first, and among posts of one date, or of none, by slug. The date descending and the slug
	// ascending are compared in opposite directions, so they are compared apart.
	posts.sort_by(|a, b| b.date.cmp(&a.date).then_with(|| a.slug.cmp(&b.slug)));
	Ok(posts)
}

/// Every post a vhost publishes, newest first, from wherever it keeps them.
///
/// The one place the source is chosen, and the only part of this module that touches the database. So
/// the genericity the database drags along -- five type parameters, threaded from the web handler --
/// stops here, and everything downstream is a plain function over a slice of posts.
///
/// A store-backed vhost with no database is a misconfiguration rather than an empty site, and says so:
/// silently serving nothing would look exactly like a site that has published nothing.
pub fn read<
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
	-> Outcome<Vec<Post>>
{
	match cfg.source {
		Source::Dir	=> read_all(&cfg.dir, id),
		Source::Store	=> match db {
			Some(db)	=> store::list(db, id),
			None		=> Err(err!(
				"publish: this vhost keeps its posts in the store and has no database \
				configured; give the vhost a 'db_dir_rel' or set 'source' to 'dir'.";
				Invalid, Input, Missing)),
		},
	}
}

/// Every Markdown file in a directory, as its stem and its text.
///
/// A file that will not read is passed over with a complaint in the log rather than failing the lot:
/// one broken post should not take the others off the page, and the log is where its author will look.
/// The directory itself failing is a different thing, and is an error -- an empty shelf looks like the
/// truth and is not.
pub fn read_sources(dir: &str, id: &str) -> Outcome<Vec<(String, String)>> {
	let entries = res!(fs::read_dir(Path::new(dir)), IO, File);

	let mut out = Vec::new();
	for entry in entries {
		let entry = res!(entry, IO, File);
		let path = entry.path();
		if path.extension().map(|e| e != EXT).unwrap_or(true) {
			continue;
		}
		let stem = match path.file_stem().and_then(|s| s.to_str()) {
			Some(s)	=> s.to_string(),
			None	=> {
				warn!("{}: posts: skipping a file whose name is not text: {:?}", id, path);
				continue;
			}
		};
		match fs::read_to_string(&path) {
			Ok(src)	=> out.push((stem, src)),
			Err(e)	=> warn!("{}: posts: skipping '{}': {}", id, stem, e),
		}
	}
	Ok(out)
}

/// Markdown, as a reader gets it.
///
/// The one place prose becomes a [`Post`], so a post from a directory and a post from the store are
/// the same post, made the same way. The title is the document's own most prominent heading, and the
/// slug where it has none.
pub fn render_source(
	source:	&str,
	slug:	String,
	date:	Option<String>,
	markup:	Markup,
)
	-> Outcome<Post>
{
	let doc = res!(parse_markup(source, markup));
	let title = doc.top_heading().unwrap_or_else(|| slug.clone());
	Ok(Post {
		slug,
		title,
		// A post made from source alone names no author or categories: those are a record's fields,
		// which the store threads in where it has one, as it does the tags and deliveries below.
		author:		String::new(),
		categories:	Vec::new(),
		date,
		excerpt:	excerpt_of(&doc),
		words:		doc.word_count(),
		html:		html::render(&doc),
		also_on:	Vec::new(),
		tags:		Vec::new(),
	})
}

/// Reads source in the syntax a post names, into the tree.
///
/// The one place either front-end is chosen. Both produce the same tree, so every caller past this
/// knows nothing of which was read.
pub fn parse_markup(source: &str, markup: Markup) -> Outcome<Doc> {
	match markup {
		Markup::Markdown	=> markdown::parse(source),
		Markup::Djot		=> djot::parse(source),
	}
}

/// The HTML of a run of source, for a preview of prose not yet saved.
///
/// The same parse and the same render a published post goes through, over source straight from an
/// editor rather than from the store -- which is the whole of what a live preview is: the page a
/// reader would get, shown to the author as they type, so the box a Djot `:::` makes is seen where it
/// will land and not guessed at.
pub fn render_html(source: &str, markup: Markup) -> Outcome<String> {
	Ok(html::render(&res!(parse_markup(source, markup))))
}

/// Reads one post by slug from a directory, where it exists.
///
/// The slug is what a reader put in a URL, so it is checked before it is allowed near a path: a name
/// is letters, digits, a dash or an underscore, which leaves nothing to climb out of the directory
/// with. A post is found by its slug whatever date its file wears, since the date is not in the URL.
pub fn read_one(dir: &str, slug: &str, id: &str) -> Outcome<Option<Post>> {
	if slug.is_empty()
		|| !slug.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
	{
		return Ok(None);
	}
	let posts = res!(read_all(dir, id));
	Ok(posts.into_iter().find(|p| p.slug == slug))
}

/// The opening of a document, flattened to its words, as a card and a feed want it.
///
/// The first paragraph, cut at a word boundary. Not the first heading: that is the title, and a card
/// saying its own title twice says nothing.
fn excerpt_of(doc: &Doc) -> String {
	let mut s = String::new();
	for block in &doc.blocks {
		if let Block::Para(content) = block {
			s = text_of(content);
			break;
		}
	}
	if s.chars().count() <= EXCERPT_LEN {
		return s;
	}
	// Cut at the last space before the limit, so a card ends on a word rather than mid-syllable.
	let cut = s.char_indices()
		.take(EXCERPT_LEN)
		.filter(|(_, c)| c.is_whitespace())
		.map(|(i, _)| i)
		.last()
		.unwrap_or_else(|| s.char_indices().nth(EXCERPT_LEN).map(|(i, _)| i).unwrap_or(s.len()));
	let mut out = s[..cut].trim_end().to_string();
	out.push('…');
	out
}

/// Splits a leading `YYYY-MM-DD-` from a file's stem, giving the date it names and the slug that is
/// left. A stem that does not begin with a date is all slug.
///
/// The shape is checked rather than the value: a date is ten characters, digits where digits belong
/// and dashes where dashes belong, followed by a dash. `2026-13-45` passes, and is a date this does not
/// have to understand -- it sorts, which is all that is asked of it here.
pub fn split_date(stem: &str) -> (Option<String>, String) {
	let b = stem.as_bytes();
	if b.len() < 11 || b[10] != b'-' {
		return (None, stem.to_string());
	}
	let shaped = b[..10].iter().enumerate().all(|(i, c)| {
		match i {
			4 | 7	=> *c == b'-',
			_	=> c.is_ascii_digit(),
		}
	});
	if !shaped {
		return (None, stem.to_string());
	}
	(Some(stem[..10].to_string()), stem[11..].to_string())
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_a_dated_name_splits_into_a_date_and_a_slug_00() -> Outcome<()> {
		assert_eq!(
			split_date("2026-07-17-on-rent"),
			(Some("2026-07-17".to_string()), "on-rent".to_string()),
		);
		Ok(())
	}

	/// A name that does not begin with a date is all slug, and says nothing about when it was written.
	#[test]
	fn test_an_undated_name_is_all_slug_01() -> Outcome<()> {
		assert_eq!(split_date("on-rent"), (None, "on-rent".to_string()));
		// Shaped like a date but not punctuated like one.
		assert_eq!(split_date("2026_07_17-on-rent"), (None, "2026_07_17-on-rent".to_string()));
		// A date with nothing after it is a name, not a date and an empty slug.
		assert_eq!(split_date("2026-07-17"), (None, "2026-07-17".to_string()));
		Ok(())
	}

	/// The shape is what is checked. A date this cannot make sense of still sorts, and sorting is all
	/// that is asked of it.
	#[test]
	fn test_a_date_is_checked_for_shape_not_sense_02() -> Outcome<()> {
		assert_eq!(
			split_date("2026-13-45-impossible"),
			(Some("2026-13-45".to_string()), "impossible".to_string()),
		);
		Ok(())
	}

	/// The prefix and what sits under it, and nothing that merely starts the same way.
	#[test]
	fn test_a_vhost_owns_its_prefix_and_no_more_03() -> Outcome<()> {
		let cfg = PublishConfig { path: fmt!("/asides"), ..Default::default() };
		assert!(cfg.owns("/asides"));
		assert!(cfg.owns("/asides/on-rent"));
		assert!(cfg.owns("/asides/feed.xml"));
		assert!(!cfg.owns("/asides-are-great"));
		assert!(!cfg.owns("/aside"));
		assert!(!cfg.owns("/"));
		Ok(())
	}

	/// A path is rooted and has no trailing slash, whatever the config said, so every route built from
	/// it is the route it looks like.
	#[test]
	fn test_a_path_is_tidied_04() -> Outcome<()> {
		let m = mapdat!{ "path" => dat!("asides/") }.get_map().unwrap();
		let cfg = res!(PublishConfig::from_datmap(&m));
		assert_eq!(cfg.path, "/asides");
		assert_eq!(cfg.feed_path(), "/asides/feed.xml");
		assert_eq!(cfg.path_of("on-rent"), "/asides/on-rent");
		Ok(())
	}

	/// A tag is validated against its normalised form, so what a person types is judged by what the
	/// store would keep: case and surrounding space do not decide it, a space inside does.
	#[test]
	fn test_a_tag_is_a_small_alphabet_06() -> Outcome<()> {
		assert!(valid_tag("rust"));
		assert!(valid_tag("Rust"));		// normalised to `rust`
		assert!(valid_tag("  web  "));		// trimmed
		assert!(valid_tag("web-dev"));
		assert!(valid_tag("c99"));
		assert!(!valid_tag(""));
		assert!(!valid_tag("   "));
		assert!(!valid_tag("a b"));		// a space is dropped, not hyphenated
		assert!(!valid_tag("a/b"));
		assert!(!valid_tag("café"));
		assert!(!valid_tag(&"x".repeat(TAG_MAX + 1)));
		Ok(())
	}

	/// The comma field splits, normalises, drops the invalid in silence, and dedupes keeping first
	/// appearance.
	#[test]
	fn test_tags_parse_from_a_comma_field_07() -> Outcome<()> {
		assert_eq!(parse_tags("rust, web"), vec![fmt!("rust"), fmt!("web")]);
		assert_eq!(parse_tags("Rust, RUST, rust"), vec![fmt!("rust")]);
		assert_eq!(parse_tags(" web , , rust "), vec![fmt!("web"), fmt!("rust")]);
		// The invalid drop out and the valid stay.
		assert_eq!(parse_tags("rust, a b, web"), vec![fmt!("rust"), fmt!("web")]);
		assert!(parse_tags("").is_empty());
		assert!(parse_tags("  ,  ").is_empty());
		Ok(())
	}

	/// A slug that could climb out of the directory finds nothing, and finds it before it touches a
	/// path.
	#[test]
	fn test_a_slug_cannot_climb_out_05() -> Outcome<()> {
		for bad in ["../../etc/passwd", "..", "a/b", "a.b", ""] {
			assert!(
				res!(read_one("/nonexistent-directory", bad, "test")).is_none(),
				"'{}' was not refused", bad,
			);
		}
		Ok(())
	}

	/// The obvious machines are discarded, whatever case they announce themselves in, and a request
	/// with no user-agent at all is one of them.
	#[test]
	fn test_a_machine_is_not_a_reader_08() -> Outcome<()> {
		for ua in [
			"Googlebot/2.1 (+http://www.google.com/bot.html)",
			"Mozilla/5.0 (compatible; bingbot/2.0)",
			"curl/8.5.0",
			"python-requests/2.31.0",
			"facebookexternalhit/1.1",
			"Mozilla/5.0 HeadlessChrome/120.0.0.0",
		] {
			assert!(looks_automated(Some(ua)), "'{}' was taken for a reader", ua);
		}
		// No user-agent, and an empty one, are scripts that did not bother.
		assert!(looks_automated(None));
		assert!(looks_automated(Some("   ")));

		// A real browser is a reader.
		for ua in [
			"Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) \
			 Chrome/120.0.0.0 Safari/537.36",
			"Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X) AppleWebKit/605.1.15",
		] {
			assert!(!looks_automated(Some(ua)), "'{}' was taken for a machine", ua);
		}
		Ok(())
	}

	/// The author reading their own post is not a read, whatever they are browsing with.
	#[test]
	fn test_the_author_is_not_a_reader_09() -> Outcome<()> {
		let browser = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 Chrome/120.0.0.0";
		// A management session is the author, so it never counts.
		assert!(!counts_as_read(true, Some(browser), false));
		// The same request without one is a reader.
		assert!(counts_as_read(false, Some(browser), false));
		// And a machine is not a reader either way.
		assert!(!counts_as_read(false, Some("Googlebot/2.1"), false));
		assert!(!counts_as_read(true, Some("Googlebot/2.1"), false));
		// A HEAD asked for no prose, so it read none -- however browser-like it looks. This is
		// the uptime monitor that would otherwise be the site's most devoted reader.
		assert!(!counts_as_read(false, Some(browser), true));
		Ok(())
	}

	/// Comments are off unless a site asks, and a config that says nothing still loads.
	#[test]
	fn test_comments_are_off_unless_asked_10() -> Outcome<()> {
		// A publish block naming nothing: loads, and takes no comments.
		let m = DaticleMap::new();
		let cfg = res!(PublishConfig::from_datmap(&m));
		assert!(!cfg.comments, "a site that said nothing was given a public write endpoint");

		// One that asks gets them.
		let mut m = DaticleMap::new();
		m.insert(dat!("comments"), Dat::Bool(true));
		assert!(res!(PublishConfig::from_datmap(&m)).comments);

		// And one that says something else is refused rather than guessed at.
		let mut m = DaticleMap::new();
		m.insert(dat!("comments"), dat!("yes".to_string()));
		assert!(PublishConfig::from_datmap(&m).is_err(), "a non-boolean 'comments' was accepted");
		Ok(())
	}

	/// The categories default where none are named, take a named list, and refuse a comma inside a
	/// name -- the character that joins them in the composer and the filter, which a name may not hold.
	#[test]
	fn test_categories_default_and_reject_commas_12() -> Outcome<()> {
		// Named none: the built-in set.
		let cfg = res!(PublishConfig::from_datmap(&DaticleMap::new()));
		assert_eq!(cfg.categories, CATEGORIES_DEFAULT.iter().map(|c| c.to_string()).collect::<Vec<_>>());

		// A multi-word name is fine: a space is not the delimiter.
		let mut m = DaticleMap::new();
		m.insert(dat!("categories"), Dat::List(vec![dat!("Book reviews".to_string()),
			dat!("Personal".to_string())]));
		let cfg = res!(PublishConfig::from_datmap(&m));
		assert_eq!(cfg.categories, vec![fmt!("Book reviews"), fmt!("Personal")]);

		// A comma in a name is refused, since it would split the joined field in two.
		let mut m = DaticleMap::new();
		m.insert(dat!("categories"), Dat::List(vec![dat!("Reviews, essays".to_string())]));
		assert!(PublishConfig::from_datmap(&m).is_err(), "a comma in a category name was accepted");
		Ok(())
	}

	/// The comment path names its post, and only a real slug.
	#[test]
	fn test_a_comment_path_names_its_post_11() -> Outcome<()> {
		let mut m = DaticleMap::new();
		m.insert(dat!("path"), dat!("/posts".to_string()));
		let cfg = res!(PublishConfig::from_datmap(&m));

		assert_eq!(cfg.comment_path("a-post"), "/posts/a-post/comment");
		assert_eq!(cfg.comment_slug("/posts/a-post/comment"), Some("a-post"));

		// Not comment paths at all.
		assert_eq!(cfg.comment_slug("/posts/a-post"), None);
		assert_eq!(cfg.comment_slug("/posts/comment"), None);
		assert_eq!(cfg.comment_slug("/elsewhere/a/comment"), None);
		// A slug that is not a name a post may wear reaches no lookup.
		assert_eq!(cfg.comment_slug("/posts/../../etc/comment"), None);
		assert_eq!(cfg.comment_slug("/posts//comment"), None);
		Ok(())
	}

	/// Today is a date the store will take and the feed will date an entry by. A day the composer
	/// offers and the save falls back to is worth nothing if it fails the module's own check.
	#[test]
	fn test_today_is_a_date_a_post_may_wear_13() -> Outcome<()> {
		let d = res!(today().ok_or_else(|| err!("The clock would not read."; Missing)));
		assert_eq!(d.len(), DATE_LEN, "today is not a day-only date: {}", d);
		assert!(valid_date(&d), "the store would refuse today: {}", d);
		// Not the epoch, which is the thing the fallback exists to stop the feed emitting.
		assert!(!d.starts_with("1970-"), "the clock read as the epoch: {}", d);
		// Shaped as the store keeps it, so it round-trips without normalisation.
		assert_eq!(normalise_date(&d), d);
		Ok(())
	}
}
