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
pub mod store;

use oxedyne_fe2o3_core::prelude::*;
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

/// The extension a post wears.
const EXT: &str = "md";

/// How much of a post's opening stands in for it in a card and a feed.
const EXCERPT_LEN: usize = 200;


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

		let source = match m.get(&dat!("source")) {
			Some(Dat::Str(s))	=> res!(Source::of(s)),
			None			=> Source::default(),
			_			=> return Err(err!(
				"PublishConfig: 'source' must be a string.";
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


/// What shape a post is.
///
/// The two are not a taxonomy for its own sake. A note is the thing an author writes most and an
/// essay the thing they write hardest, and a surface that made a passing thought wear an essay's
/// furniture -- a slug typed by hand, a cover, an excerpt -- would quietly stop them writing the
/// passing thought down. So the tree carries which it is, and each renders as what it is.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum PostKind {
	/// Short. The default, because most of what gets written is.
	#[default]
	Note,
	/// Long.
	Essay,
}

impl PostKind {

	/// The word a record stores.
	pub fn as_str(&self) -> &'static str {
		match self {
			Self::Note	=> "note",
			Self::Essay	=> "essay",
		}
	}

	/// The kind a word names. An unknown word is a note, that being the lesser claim: calling an essay
	/// a note under-dresses it, where the reverse would put furniture around a passing thought.
	pub fn of(s: &str) -> Self {
		match s {
			"essay"	=> Self::Essay,
			_	=> Self::Note,
		}
	}
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

/// One post, as a reader gets it.
#[derive(Clone, Debug)]
pub struct Post {
	/// The post's name in a URL.
	pub slug:	String,
	/// The post's own most prominent heading, or its slug where it has none.
	pub title:	String,
	/// Long or short.
	pub kind:	PostKind,
	/// The date it was given, where it was given one.
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
	let sources = res!(read_sources(dir, id));

	let mut posts = Vec::new();
	for (stem, source) in sources {
		let (date, slug) = split_date(&stem);
		// A file on disk is not a draft, or it would not be on the disk of a server, so a directory
		// holds no kind or state of its own: everything in one is a live note.
		// A directory of files is Markdown: it is the form prose already exists in, and a file on
		// disk carries no field to say otherwise.
		match render_source(&source, slug, date, PostKind::Note, Markup::Markdown) {
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
	kind:	PostKind,
	markup:	Markup,
)
	-> Outcome<Post>
{
	let doc = res!(parse_markup(source, markup));
	let title = doc.top_heading().unwrap_or_else(|| slug.clone());
	Ok(Post {
		slug,
		title,
		kind,
		date,
		excerpt:	excerpt_of(&doc),
		html:		html::render(&doc),
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
