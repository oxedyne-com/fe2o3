//! How much a work needed AI, declared in a mark a reader can see.
//!
//! A voluntary declaration, not provenance and not detection: it asserts nothing a machine could
//! check, and is worth exactly what the reader thinks the declarer's word is worth. What the module
//! does is put the claim where a person meets the work -- beside a post, beside a book, in the site's
//! own footer -- and link it to the scheme that defines the words.
//!
//! # The ladder
//!
//! Five rungs, from *AI was unnecessary* to *the human was unnecessary*. The question each one
//! answers is how much the work **needed** a model, not what share of it a model produced: a share is
//! a count nobody performs, and the answer that matters to a reader is whether the work could have
//! existed without the machine.
//!
//! # No scheme is named here
//!
//! The words are ordinary English and the ladder is a public one, but the site that defines them, and
//! the artwork that draws them, are a particular scheme's. Both are configuration
//! ([`DeclareConfig::url`] and [`DeclareConfig::marks`]), so this engine serves a site declaring
//! under any such scheme, and a site that configures none draws nothing at all.
//!
//! # The size rule is structural
//!
//! A level is read by counting the pins around the mark, and the count stops being possible as the
//! mark shrinks -- at half size two neighbouring levels are the same picture, and a mark too small to
//! read does not *look* broken, which is what makes it worse than no mark. So a mark drawn below
//! [`MARK_MIN_PX`] carries its declaration in words beside it, and [`Size::alone`] will not return a
//! wordless mark below that size however small a caller asks for.

use crate::srv::cache;

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_jdat::prelude::*;
use oxedyne_fe2o3_jdat::string::enc::EncoderConfig;
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

use std::collections::BTreeMap;


/// The smallest a mark may be drawn without its words, in CSS pixels.
///
/// Below this the pins are no longer countable, so the level is no longer readable, so the words come
/// with it. See the module note: this is the one number in the scheme that a renderer must not treat
/// as advice.
pub const MARK_MIN_PX: u32 = 40;

/// The file extension the artwork is kept in. The marks are line drawings, and a line drawing that
/// has to sit at a byline and on a poster is a vector.
const MARK_EXT: &str = ".svg";


/// How much a work needed AI.
///
/// The ladder turns on questions a declarer can answer honestly about their own work: was any used at
/// all; could **you** have done it; does subtracting the AI leave a lesser work or none at all; was a
/// person needed to produce it rather than to direct it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Level {
	/// None was used.
	No,
	/// It helped, but the declarer could have done this without it.
	Some,
	/// Parts of this could not have been done without it.
	With,
	/// Without AI there would be no work at all.
	Mostly,
	/// No person was needed to produce it.
	Entirely,
}

impl Level {

	/// Every rung, in order, weakest claim on AI first. What a chooser offers and what a test walks.
	pub const ALL: [Self; 5] = [Self::No, Self::Some, Self::With, Self::Mostly, Self::Entirely];

	/// The word a record stores and a URL carries.
	///
	/// **These never change.** They are printed into the scheme's badge codes, so a slug is a
	/// permanent name rather than a spelling this module is free to tidy.
	pub fn slug(&self) -> &'static str {
		match self {
			Self::No	=> "no-ai",
			Self::Some	=> "some-ai",
			Self::With	=> "with-ai",
			Self::Mostly	=> "mostly-ai",
			Self::Entirely	=> "entirely-ai",
		}
	}

	/// The declaration in words, as a reader is shown it.
	///
	/// Note the word order of the fourth rung: a work is made **mostly with** AI, not made with mostly
	/// AI. The first says how much the making leaned on the machine, which is the claim; the second
	/// says something about the machine, which is not.
	pub fn words(&self) -> &'static str {
		match self {
			Self::No	=> "Made with no AI",
			Self::Some	=> "Made with some AI",
			Self::With	=> "Made with AI",
			Self::Mostly	=> "Made mostly with AI",
			Self::Entirely	=> "Made with AI entirely",
		}
	}

	/// The rung a slug names, or nothing where it names none.
	///
	/// **An unknown word is not a level**, deliberately: every other reading would put a declaration on
	/// a work whose author did not make one, and a declaration nobody made is the one output this
	/// module must never produce. Undeclared is a state, not a failure.
	pub fn of(s: &str) -> Option<Self> {
		Self::ALL.iter().find(|l| l.slug() == s).copied()
	}
}

/// What kind of work is being declared for.
///
/// A work is often several at once -- a book has a cover and a text, a film has a picture and a score
/// -- and the honest answer can differ between them, so the mark says which part it speaks for. The
/// exception is [`Whole`](Self::Whole), for a work that sits at one level throughout.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Medium {
	/// The work entire, at one level. The scheme's umbrella mark.
	Whole,
	/// Written prose: a post, a chapter, a document.
	Doc,
	/// Software.
	Code,
	/// A still picture.
	Image,
	/// Sound.
	Audio,
	/// Moving picture.
	Video,
}

impl Medium {

	/// Every kind, the whole work first. What a config chooser offers and what a test walks.
	pub const ALL: [Self; 6] =
		[Self::Whole, Self::Doc, Self::Code, Self::Image, Self::Audio, Self::Video];

	/// The word a config names it by and a URL carries. The whole work carries none: its declaration
	/// is about the work rather than a part of it, and a URL saying `/with-ai` is that claim exactly.
	pub fn slug(&self) -> &'static str {
		match self {
			Self::Whole	=> "",
			Self::Doc	=> "doc",
			Self::Code	=> "code",
			Self::Image	=> "image",
			Self::Audio	=> "audio",
			Self::Video	=> "video",
		}
	}

	/// The kind a word names, or nothing where it names none. An empty word is the whole work.
	pub fn of(s: &str) -> Option<Self> {
		Self::ALL.iter().find(|m| m.slug() == s).copied()
	}
}

/// A declaration: a rung of the ladder, and the part of the work it speaks for.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Declaration {
	/// How much this needed AI.
	pub level:	Level,
	/// Which part of the work says so.
	pub medium:	Medium,
}

impl Declaration {

	/// A declaration of a level about a kind of work.
	pub fn new(level: Level, medium: Medium) -> Self {
		Self { level, medium }
	}

	/// The path under the scheme's site that defines this declaration, e.g. `/with-ai/doc`.
	///
	/// The web counterpart of the code printed on a badge, and the same address: a reader who wants to
	/// know what a mark means follows it to the page that says so, whether they scanned it or clicked
	/// it.
	pub fn path(&self) -> String {
		match self.medium {
			Medium::Whole	=> fmt!("/{}", self.level.slug()),
			_		=> fmt!("/{}/{}", self.level.slug(), self.medium.slug()),
		}
	}

	/// The artwork's file name, without a directory, e.g. `doc-with-ai.svg`.
	///
	/// Built from the two permanent slugs rather than from the artwork's own file names, which belong
	/// to whoever drew it and have been renamed at least once. A site ships the thirty marks under
	/// this rule and the rule is the whole of the contract.
	pub fn mark_file(&self) -> String {
		match self.medium {
			Medium::Whole	=> fmt!("{}{}", self.level.slug(), MARK_EXT),
			_		=> fmt!("{}-{}{}", self.medium.slug(), self.level.slug(), MARK_EXT),
		}
	}
}

/// A named thing on the site that carries a declaration of its own.
///
/// Posts hold their level in their own record, because a post is written here. This is for everything
/// else a site shows -- a book, a project, a product -- which is authored somewhere else and needs
/// only somewhere to keep the one field, and a place for an admin to set it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Declarable {
	/// What the site calls it, in the store and in the JSON a page reads. Never shown.
	pub key:	String,
	/// What an admin sees when choosing its level.
	pub name:	String,
	/// The kind of work it is, which decides which mark it wears.
	pub medium:	Medium,
}

/// A site's declaration settings: the scheme it speaks, where its artwork is, what the site says
/// about itself, and what else on the site may be declared for.
///
/// Absent from a config, every field is empty and [`is_on`](Self::is_on) is false, which draws no mark
/// anywhere. A site declares when it says where the scheme lives and where the artwork is, and not
/// before -- a mark whose artwork 404s is worse than no mark, and a mark linking nowhere explains
/// nothing.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DeclareConfig {
	/// The scheme's own site, e.g. `https://example.org`, without a trailing slash. What a mark links
	/// to, with the declaration's own path after it.
	pub url:	String,
	/// Where the artwork is served from, e.g. `/assets/marks`, without a trailing slash.
	pub marks:	String,
	/// What the site says about itself, drawn in its footer. Nothing where the site declares nothing
	/// about itself, which is not the same as declaring that it used none.
	pub site:	Option<Declaration>,
	/// The things on this site an admin may set a level for, in the order they are offered.
	pub items:	Vec<Declarable>,
}

impl DeclareConfig {

	/// Whether this site declares at all.
	pub fn is_on(&self) -> bool {
		!self.url.is_empty() && !self.marks.is_empty()
	}

	/// The declarable of a given key, where the site has one.
	pub fn item(&self, key: &str) -> Option<&Declarable> {
		self.items.iter().find(|i| i.key == key)
	}

	/// Reads the block a config writes.
	///
	/// ```text
	/// "declare": {
	///   "url":   "https://example.org",
	///   "marks": "/assets/marks",
	///   "site":  "with-ai/code",
	///   "items": [
	///     { "key": "widget", "name": "The Widget", "medium": "doc" }
	///   ]
	/// }
	/// ```
	///
	/// A level or a medium this module does not know is an error rather than a silent default: a
	/// mistyped rung would otherwise publish a claim the operator did not make.
	pub fn from_datmap(m: &DaticleMap) -> Outcome<Self> {
		let get_str = |key: &str| -> Outcome<String> {
			match m.get(&dat!(key)) {
				Some(Dat::Str(s))	=> Ok(s.clone()),
				None			=> Ok(String::new()),
				_			=> Err(err!(
					"DeclareConfig: '{}' must be a string.", key;
					Invalid, Input, Mismatch)),
			}
		};

		// Both are prefixes something is appended to, so a trailing slash would double the one in the
		// path that follows.
		let mut url = res!(get_str("url"));
		while url.ends_with('/') {
			url.pop();
		}
		let mut marks = res!(get_str("marks"));
		while marks.ends_with('/') {
			marks.pop();
		}

		let site_str = res!(get_str("site"));
		let site = if site_str.is_empty() {
			None
		} else {
			Some(res!(parse_declaration(&site_str)))
		};

		let items = match m.get(&dat!("items")) {
			Some(Dat::List(list))	=> res!(declarables(list)),
			Some(Dat::Vek(vek))	=> res!(declarables(vek.as_slice())),
			None			=> Vec::new(),
			_			=> return Err(err!(
				"DeclareConfig: 'items' must be a list of maps.";
				Invalid, Input, Mismatch)),
		};

		Ok(Self { url, marks, site, items })
	}
}

/// A declaration written as one word, `<level>` or `<level>/<medium>`.
///
/// One field rather than two, because the two are never usefully set apart: a level with no medium is
/// a declaration about the whole work, which the grammar says by leaving the medium off.
fn parse_declaration(s: &str) -> Outcome<Declaration> {
	let (level_str, medium_str) = match s.split_once('/') {
		Some((l, m))	=> (l, m),
		None		=> (s, ""),
	};
	let level = res!(Level::of(level_str).ok_or_else(|| err!(
		"DeclareConfig: '{}' is not a declaration level. The levels are {}.",
		level_str, slug_list(); Invalid, Input)));
	let medium = res!(Medium::of(medium_str).ok_or_else(|| err!(
		"DeclareConfig: '{}' is not a kind of work.", medium_str; Invalid, Input)));
	Ok(Declaration { level, medium })
}

/// Every level's slug, for the error a mistyped one raises. An operator who typed the wrong word is
/// owed the list of right ones.
fn slug_list() -> String {
	Level::ALL.iter().map(|l| l.slug()).collect::<Vec<_>>().join(", ")
}

/// The declarable things in a config list.
fn declarables(items: &[Dat]) -> Outcome<Vec<Declarable>> {
	let mut out = Vec::new();
	for item in items {
		let m = match item {
			Dat::Map(m)	=> m,
			_		=> return Err(err!(
				"DeclareConfig: every 'items' entry must be a map.";
				Invalid, Input, Mismatch)),
		};
		let field = |key: &str| -> Outcome<String> {
			match m.get(&dat!(key)) {
				Some(Dat::Str(s))	=> Ok(s.clone()),
				None			=> Ok(String::new()),
				_			=> Err(err!(
					"DeclareConfig: an item's '{}' must be a string.", key;
					Invalid, Input, Mismatch)),
			}
		};
		let key = res!(field("key"));
		if key.is_empty() {
			return Err(err!(
				"DeclareConfig: every declarable item needs a 'key', which is how its level is \
				stored and how a page asks for it."; Invalid, Input, Missing));
		}
		let name = res!(field("name"));
		let medium_str = res!(field("medium"));
		let medium = res!(Medium::of(&medium_str).ok_or_else(|| err!(
			"DeclareConfig: '{}' is not a kind of work, for item '{}'.", medium_str, key;
			Invalid, Input)));
		out.push(Declarable {
			// A name nobody set reads as the key, which is at least a word an admin recognises.
			name:	if name.is_empty() { key.clone() } else { name },
			key,
			medium,
		});
	}
	Ok(out)
}

/// How a mark is set on the page.
///
/// The two are not interchangeable and the difference is not taste: see [`MARK_MIN_PX`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Size {
	/// The mark by itself, at the given size in CSS pixels, which is at least [`MARK_MIN_PX`].
	Alone(u32),
	/// The mark at text size with its declaration in words beside it.
	WithWords,
}

impl Size {

	/// A mark alone at the given size, **or with its words where that size is too small to read**.
	///
	/// The rule made structural: a caller asking for a wordless mark at 16 px does not get one, it
	/// gets a legible declaration. There is no way to spell the illegible arrangement.
	pub fn alone(px: u32) -> Self {
		if px < MARK_MIN_PX {
			Self::WithWords
		} else {
			Self::Alone(px)
		}
	}
}

/// The mark for a declaration, as HTML: a link to the scheme, wearing the artwork.
///
/// Empty where the site declares nothing ([`DeclareConfig::is_on`]), so every caller can ask
/// unconditionally and a site that has not configured the scheme simply has no marks.
///
/// # Why the artwork is a mask and not a picture
///
/// The mark has to sit on a dark site and a light one and read as the site's own furniture, and the
/// artwork is one set of files drawn in black. So it is used as a **mask** over `currentColor`: the
/// shape comes from the file, the colour from whatever the surrounding text is. One asset, every
/// palette, and no per-site copy of the artwork to keep in step.
///
/// The mask carries no meaning to a reader who cannot see it, so the accessible name is on the link,
/// and the shape itself is hidden from assistive technology rather than announced as an image with no
/// description.
pub fn mark_html(cfg: &DeclareConfig, decl: Declaration, size: Size, class: &str) -> String {
	if !cfg.is_on() {
		return String::new();
	}
	let mut s = String::new();
	s.push_str("<a class=\"ai-mark");
	match size {
		Size::Alone(_)	=> s.push_str(" ai-mark-alone"),
		Size::WithWords	=> s.push_str(" ai-mark-inline"),
	}
	if !class.is_empty() {
		s.push(' ');
		escape_attr(&mut s, class);
	}
	s.push_str("\" href=\"");
	escape_attr(&mut s, &fmt!("{}{}", cfg.url, decl.path()));
	// A reader following a mark has not finished with the page they were reading, and the scheme is a
	// third-party site: a new tab, and no window handle back to this one.
	s.push_str("\" target=\"_blank\" rel=\"noopener\" title=\"");
	escape_attr(&mut s, decl.level.words());
	s.push_str("\" aria-label=\"");
	escape_attr(&mut s, decl.level.words());
	s.push_str("\">");

	// The shape. Its size rides in a custom property rather than a class per size, because the sizes
	// are a placement decision and the stylesheet should not have to grow a class for each one.
	s.push_str("<span class=\"ai-mark-ink\" aria-hidden=\"true\" style=\"");
	if let Size::Alone(px) = size {
		s.push_str(&fmt!("--ai-mark-size:{}px;", px));
	}
	let url = fmt!("{}/{}", cfg.marks, decl.mark_file());
	s.push_str(&fmt!(
		"-webkit-mask-image:url('{0}');mask-image:url('{0}')", css_url(&url)));
	s.push_str("\"></span>");

	// The words, where the mark is too small to be read without them.
	if size == Size::WithWords {
		s.push_str("<span class=\"ai-mark-words\">");
		escape_text(&mut s, decl.level.words());
		s.push_str("</span>");
	}
	s.push_str("</a>");
	s
}

/// A URL fit to sit inside a CSS `url('…')` in a style attribute.
///
/// Two escapes, not one: the attribute is escaped on the way out by the caller's `escape_attr`, but
/// the CSS string inside it has its own quoting, and a value carrying a quote or a backslash would
/// otherwise close the string and leave the rest as declarations. The paths this takes are built from
/// config, and config is not a trusted source of syntax.
fn css_url(url: &str) -> String {
	let mut out = String::new();
	for c in url.chars() {
		match c {
			'\\' | '\'' | '"'	=> {
				out.push('\\');
				out.push(c);
			},
			// A newline would end the declaration; a parenthesis would end the `url()`.
			'\n' | '\r' | '(' | ')'	=> {},
			_			=> out.push(c),
		}
	}
	// The attribute's own escaping happens where this is written into one.
	out.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// Serves the site's declarations as JSON, for a page that draws its own.
///
/// Everything resolved -- the words, the artwork's URL, the link -- rather than the two slugs and a
/// rule to apply to them. There is one rule for how a mark is built and it lives here; a client
/// reimplementing it is a second rule that will drift from this one.
///
/// An item the admin has not set a level for carries no `level` key at all, the empty idiom the rest
/// of this module keeps: absent and undeclared are the same thing said once.
pub fn serve_json(
	cfg:	&DeclareConfig,
	levels:	&BTreeMap<String, Level>,
	id:	&str,
)
	-> Outcome<HttpMessage>
{
	let resolved = |d: Declaration| -> Vec<(Dat, Dat)> {
		vec![
			(dat!("level"),		dat!(d.level.slug().to_string())),
			(dat!("medium"),	dat!(d.medium.slug().to_string())),
			(dat!("words"),		dat!(d.level.words().to_string())),
			(dat!("mark"),		dat!(fmt!("{}/{}", cfg.marks, d.mark_file()))),
			(dat!("href"),		dat!(fmt!("{}{}", cfg.url, d.path()))),
		]
	};

	let mut fields = vec![
		(dat!("url"),	dat!(cfg.url.clone())),
		(dat!("marks"),	dat!(cfg.marks.clone())),
	];
	// What the site says about itself. Absent where it says nothing.
	if let Some(d) = cfg.site {
		fields.push((dat!("site"), create_dat_ordmap(resolved(d))));
	}
	let items = cfg.items.iter()
		.map(|item| {
			let mut f = vec![
				(dat!("key"),		dat!(item.key.clone())),
				(dat!("name"),		dat!(item.name.clone())),
				(dat!("medium"),	dat!(item.medium.slug().to_string())),
			];
			if let Some(level) = levels.get(&item.key) {
				f.extend(resolved(Declaration::new(*level, item.medium)));
			}
			create_dat_ordmap(f)
		})
		.collect::<Vec<_>>();
	fields.push((dat!("items"), Dat::List(items)));

	let json_cfg = EncoderConfig::<(), ()>::json(None);
	let body_json = res!(create_dat_ordmap(fields).encode_string_with_config(&json_cfg));

	info!("{}: publish: declarations, {} item(s)", id, cfg.items.len());

	let mut resp = HttpMessage::ok_respond_with_text(body_json);
	resp = resp.with_field(
		HeaderName::ContentType,
		HeaderFieldValue::Generic(fmt!("application/json")),
	);
	// A level changed in the console must show on the site at once. A page holding yesterday's copy
	// would draw a declaration its author has since corrected, which is the one staleness that matters
	// here.
	Ok(cache::generated(resp))
}


#[cfg(test)]
mod tests {
	use super::*;

	fn cfg() -> DeclareConfig {
		DeclareConfig {
			url:	fmt!("https://example.org"),
			marks:	fmt!("/assets/marks"),
			site:	Some(Declaration::new(Level::With, Medium::Code)),
			items:	vec![Declarable {
				key:	fmt!("widget"),
				name:	fmt!("The Widget"),
				medium:	Medium::Doc,
			}],
		}
	}

	/// The slugs are printed into badge codes and cannot be revised. Pinned here so a tidy-up of the
	/// spelling fails a test rather than breaking every code already in the world.
	#[test]
	fn test_the_slugs_are_permanent_00() -> Outcome<()> {
		let got = Level::ALL.iter().map(|l| l.slug()).collect::<Vec<_>>();
		assert_eq!(got, vec!["no-ai", "some-ai", "with-ai", "mostly-ai", "entirely-ai"]);
		// Round trip: every slug names back the rung it came from, and nothing else does.
		for l in Level::ALL {
			assert_eq!(Level::of(l.slug()), Some(l), "'{}' did not read back", l.slug());
		}
		assert_eq!(Level::of("mostly"), None, "a partial word named a level");
		assert_eq!(Level::of(""), None, "an empty word named a level");
		Ok(())
	}

	/// The fourth rung says a work was made *mostly with* AI. The other word order says something
	/// about the machine rather than about the making, and is not the claim.
	#[test]
	fn test_the_fourth_rung_keeps_its_word_order_01() -> Outcome<()> {
		assert_eq!(Level::Mostly.words(), "Made mostly with AI");
		Ok(())
	}

	/// A declaration addresses the scheme's page for it, and wears the artwork named by the same two
	/// slugs. The whole work carries no medium in either.
	#[test]
	fn test_a_declaration_addresses_its_page_and_its_artwork_02() -> Outcome<()> {
		let d = Declaration::new(Level::With, Medium::Doc);
		assert_eq!(d.path(), "/with-ai/doc");
		assert_eq!(d.mark_file(), "doc-with-ai.svg");
		let w = Declaration::new(Level::Entirely, Medium::Whole);
		assert_eq!(w.path(), "/entirely-ai");
		assert_eq!(w.mark_file(), "entirely-ai.svg");
		Ok(())
	}

	/// A wordless mark below the countable size is not a thing this module can be asked to draw. The
	/// caller asking for one gets the legible arrangement instead.
	#[test]
	fn test_a_small_mark_cannot_lose_its_words_03() -> Outcome<()> {
		assert_eq!(Size::alone(MARK_MIN_PX), Size::Alone(MARK_MIN_PX));
		assert_eq!(Size::alone(MARK_MIN_PX + 8), Size::Alone(MARK_MIN_PX + 8));
		assert_eq!(Size::alone(MARK_MIN_PX - 1), Size::WithWords, "a mark went wordless below the floor");
		assert_eq!(Size::alone(16), Size::WithWords);

		// And the words really are drawn in that arrangement, not merely chosen.
		let html = mark_html(&cfg(), Declaration::new(Level::Some, Medium::Doc), Size::alone(16), "");
		assert!(html.contains("Made with some AI"), "the small mark carried no words: {}", html);
		assert!(html.contains("ai-mark-inline"), "the small mark is not the inline arrangement: {}", html);
		Ok(())
	}

	/// The mark links to the scheme's page for exactly the declaration it draws, wears the artwork as
	/// a mask so it takes the site's own colour, and says in words what it is to a reader who cannot
	/// see it.
	#[test]
	fn test_the_mark_links_and_names_itself_04() -> Outcome<()> {
		let html = mark_html(&cfg(), Declaration::new(Level::Mostly, Medium::Code), Size::alone(44), "foot-mark");
		assert!(html.contains("href=\"https://example.org/mostly-ai/code\""), "wrong link: {}", html);
		assert!(html.contains("mask-image:url('/assets/marks/code-mostly-ai.svg')"), "wrong artwork: {}", html);
		assert!(html.contains("-webkit-mask-image"), "no mask for the older engine: {}", html);
		assert!(html.contains("aria-label=\"Made mostly with AI\""), "no accessible name: {}", html);
		assert!(html.contains("--ai-mark-size:44px"), "the size did not reach the mark: {}", html);
		assert!(html.contains("foot-mark"), "the caller's class was dropped: {}", html);
		// The shape says nothing to a reader who cannot see it, and must not be announced twice.
		assert!(html.contains("aria-hidden=\"true\""), "the shape is not hidden from assistive tech: {}", html);
		Ok(())
	}

	/// A site that has not configured the scheme draws nothing at all, rather than a mark whose
	/// artwork is missing and whose link goes nowhere.
	#[test]
	fn test_an_unconfigured_site_draws_no_mark_05() -> Outcome<()> {
		let off = DeclareConfig::default();
		assert!(!off.is_on());
		let html = mark_html(&off, Declaration::new(Level::No, Medium::Doc), Size::alone(44), "");
		assert!(html.is_empty(), "an unconfigured site drew a mark: {}", html);
		// Half-configured is still off: artwork with no scheme explains nothing, and a scheme with no
		// artwork draws nothing.
		let half = DeclareConfig { url: fmt!("https://example.org"), ..Default::default() };
		assert!(!half.is_on(), "a site with no artwork declared anyway");
		Ok(())
	}

	/// The config block reads the site's own declaration and its declarable things, and refuses a rung
	/// it does not know rather than quietly picking one.
	#[test]
	fn test_the_config_block_reads_and_refuses_06() -> Outcome<()> {
		let m = mapdat!{
			"url"	=> "https://example.org/",
			"marks"	=> "/assets/marks/",
			"site"	=> "with-ai/code",
			"items"	=> listdat![
				mapdat!{ "key" => "widget", "name" => "The Widget", "medium" => "doc" },
				mapdat!{ "key" => "engine", "medium" => "code" },
			],
		}.get_map().unwrap();
		let c = res!(DeclareConfig::from_datmap(&m));
		// The trailing slashes are gone, or every path built from them would double one.
		assert_eq!(c.url, "https://example.org");
		assert_eq!(c.marks, "/assets/marks");
		assert_eq!(c.site, Some(Declaration::new(Level::With, Medium::Code)));
		assert_eq!(c.items.len(), 2);
		assert_eq!(c.items[0].name, "The Widget");
		// An item with no name of its own is offered under its key, not under nothing.
		assert_eq!(c.items[1].name, "engine");
		assert_eq!(c.items[1].medium, Medium::Code);

		let bad = mapdat!{
			"url"	=> "https://example.org",
			"marks"	=> "/assets/marks",
			"site"	=> "quite-a-lot-of-ai",
		}.get_map().unwrap();
		assert!(DeclareConfig::from_datmap(&bad).is_err(), "a level nobody defined was accepted");
		Ok(())
	}

	/// The JSON hands a client the finished mark rather than the parts, and says nothing at all about
	/// an item whose level nobody has set.
	#[test]
	fn test_the_json_resolves_a_mark_and_omits_an_undeclared_one_07() -> Outcome<()> {
		let mut levels = BTreeMap::new();
		levels.insert(fmt!("widget"), Level::Some);
		let resp = res!(serve_json(&cfg(), &levels, "test"));
		let body = String::from_utf8_lossy(&resp.body).to_string();
		assert!(body.contains(r#""mark": "/assets/marks/doc-some-ai.svg""#), "no artwork URL: {}", body);
		assert!(body.contains(r#""href": "https://example.org/some-ai/doc""#), "no link: {}", body);
		assert!(body.contains(r#""words": "Made with some AI""#), "no words: {}", body);
		// The site's own declaration rides alongside, so a footer needs one fetch and not two.
		assert!(body.contains(r#""/assets/marks/code-with-ai.svg""#), "no site mark: {}", body);

		// The same site with nothing declared about itself, so the only `level` a body could carry
		// would be the item's own -- and there is none to carry.
		let bare = DeclareConfig { site: None, ..cfg() };
		let resp = res!(serve_json(&bare, &BTreeMap::new(), "test"));
		let body = String::from_utf8_lossy(&resp.body).to_string();
		assert!(body.contains(r#""key": "widget""#), "the declarable itself went missing: {}", body);
		assert!(!body.contains(r#""level""#), "an unset item was given a level: {}", body);
		Ok(())
	}

	/// A path from config cannot break out of the CSS string it is written into.
	#[test]
	fn test_an_artwork_path_cannot_escape_its_css_string_08() -> Outcome<()> {
		let c = DeclareConfig {
			marks: fmt!("/a'); background:url('http://elsewhere/x"),
			..cfg()
		};
		let html = mark_html(&c, Declaration::new(Level::No, Medium::Doc), Size::alone(44), "");
		assert!(!html.contains("background:url('http"), "a config path opened a second declaration: {}", html);
		Ok(())
	}
}
