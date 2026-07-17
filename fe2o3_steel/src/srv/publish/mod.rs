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

pub mod feed;
pub mod json;
pub mod page;

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_jdat::prelude::*;
use oxedyne_fe2o3_text::doc::{
	Block,
	Doc,
	html,
	markdown,
	text_of,
};

use std::{
	fs,
	path::Path,
};


/// Directory holding the posts, where the config names no other.
pub const DIR_DEFAULT: &str = "./www/public/content/posts";

/// URL prefix the posts are served under, where the config names no other.
pub const PATH_DEFAULT: &str = "/posts";

/// The extension a post wears.
const EXT: &str = "md";

/// How much of a post's opening stands in for it in a card and a feed.
const EXCERPT_LEN: usize = 200;


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
	/// Directory holding the Markdown, absolute or relative to the app root.
	pub dir:	String,
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

		Ok(Self {
			path,
			dir:		res!(get_str("dir", DIR_DEFAULT)),
			title:		res!(get_str("title", "Posts")),
			site_name:	res!(get_str("site_name", "")),
			base_url,
			css,
		})
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
}


/// One post.
#[derive(Clone, Debug)]
pub struct Post {
	/// The post's name in a URL, taken from its file.
	pub slug:	String,
	/// The post's own most prominent heading, or its slug where it has none.
	pub title:	String,
	/// The date its file names, where its file names one.
	pub date:	Option<String>,
	/// The opening of the prose, flattened, for a card and a feed.
	pub excerpt:	String,
	/// The prose, rendered.
	pub html:	String,
}


/// Reads every post in a directory, newest first.
///
/// A file that will not read or will not parse is passed over with a complaint in the log rather than
/// failing the lot: one broken post should not take the others off the page, and the log is where its
/// author will look. The directory itself failing is a different thing, and is an error -- an empty
/// shelf looks like the truth and is not.
pub fn read_all(dir: &str, id: &str) -> Outcome<Vec<Post>> {
	let entries = res!(fs::read_dir(Path::new(dir)), IO, File);

	let mut posts = Vec::new();
	for entry in entries {
		let entry = res!(entry, IO, File);
		let path = entry.path();
		if path.extension().map(|e| e != EXT).unwrap_or(true) {
			continue;
		}
		let stem = match path.file_stem().and_then(|s| s.to_str()) {
			Some(s)	=> s,
			None	=> {
				warn!("{}: posts: skipping a file whose name is not text: {:?}", id, path);
				continue;
			}
		};
		let src = match fs::read_to_string(&path) {
			Ok(src)	=> src,
			Err(e)	=> {
				warn!("{}: posts: skipping '{}': {}", id, stem, e);
				continue;
			}
		};
		let doc = match markdown::parse(&src) {
			Ok(doc)	=> doc,
			Err(e)	=> {
				warn!("{}: posts: skipping '{}', which will not read as Markdown: {}", id, stem, e);
				continue;
			}
		};
		let (date, slug) = split_date(stem);
		let title = doc.top_heading().unwrap_or_else(|| slug.clone());
		posts.push(Post {
			slug,
			title,
			date,
			excerpt:	excerpt_of(&doc),
			html:		html::render(&doc),
		});
	}

	// Newest first, and among posts of one date, or of none, by slug. The date descending and the slug
	// ascending are compared in opposite directions, so they are compared apart.
	posts.sort_by(|a, b| b.date.cmp(&a.date).then_with(|| a.slug.cmp(&b.slug)));
	Ok(posts)
}

/// Reads one post by slug, where it exists.
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
}
