//! The posts in the vhost's database.
//!
//! The alternative to [a directory](super::read_all), and where posts end up once anything other than
//! a text editor writes them. Same [`Post`] out either way, so nothing downstream knows which it is.
//!
//! # The source is what is kept
//!
//! A record holds the Markdown an author wrote, not the HTML a reader gets. The renderer improves --
//! it gained tables this morning -- and a stored rendering would be a photograph of what the renderer
//! used to do. Rendering on read costs a parse per request and is always right; caching it is an
//! optimisation to make when a profile asks for one, not before.
//!
//! # An index, because scanning is not free
//!
//! [`Database::scan`] is documented as O(database size), and a vhost's database is not only posts --
//! it is sessions, users, whatever else the app keeps. Walking all of it to list ten asides would tie
//! the cost of a page to how busy the site has been.
//!
//! So the slugs live in one record under [`INDEX_KEY`], and a post under its own key. Listing is a
//! read of the index and a read per post; nothing scans. The index is derived, so it can be rebuilt
//! from a scan when it has to be ([`rebuild_index`]), but that is a repair rather than a code path
//! anything normal takes.

use crate::srv::publish::{
	Post,
	Markup,
	PostKind,
	PostState,
	render_source,
	split_date,
};

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_iop_crypto::enc::Encrypter;
use oxedyne_fe2o3_iop_db::api::{
	Database,
	ScanOpts,
};
use oxedyne_fe2o3_iop_hash::api::Hasher;
use oxedyne_fe2o3_jdat::{
	prelude::*,
	id::NumIdDat,
};

use std::sync::{
	Arc,
	RwLock,
};


/// The key every post's key begins with.
pub const KEY_PREFIX: &str = "publish/post/";

/// The key the list of slugs lives under.
pub const INDEX_KEY: &str = "publish/index";


/// A post's key.
fn key_of(slug: &str) -> Dat {
	let mut s = String::from(KEY_PREFIX);
	s.push_str(slug);
	dat!(s)
}

/// What a post is, on the way in and out of the database.
///
/// Separate from [`Post`], which is the rendered view a reader gets. This is what an author wrote and
/// what is kept; that is what is made of it.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Record {
	/// The post's name in a URL.
	pub slug:	String,
	/// Long or short.
	pub kind:	PostKind,
	/// Draft or live.
	pub state:	PostState,
	/// Markdown or Djot.
	pub markup:	Markup,
	/// The date the author gave it, where they gave one.
	pub date:	Option<String>,
	/// The prose, as written, in whatever markup [`markup`](Record::markup) names.
	pub source:	String,
}

impl Record {

	/// The record as a daticle.
	///
	/// A plain map, not an ordered one: a record is a set of named fields and nothing depends on the
	/// order they were written in. A `BTreeMap` sorts them by name, so one record encodes one way.
	pub fn to_dat(&self) -> Dat {
		let mut m = DaticleMap::new();
		m.insert(dat!("slug"),		dat!(self.slug.clone()));
		m.insert(dat!("kind"),		dat!(self.kind.as_str().to_string()));
		m.insert(dat!("state"),		dat!(self.state.as_str().to_string()));
		m.insert(dat!("markup"),	dat!(self.markup.as_str().to_string()));
		m.insert(dat!("source"),	dat!(self.source.clone()));
		// A post without a date carries no date key. A key saying nothing is a second way to say
		// nothing, and two ways to say one thing is one too many.
		if let Some(d) = &self.date {
			m.insert(dat!("date"), dat!(d.clone()));
		}
		Dat::Map(m)
	}

	/// The record from a daticle.
	pub fn from_dat(d: &Dat) -> Outcome<Self> {
		let m = match d {
			Dat::Map(m)	=> m,
			_		=> return Err(err!(
				"publish: a post record must be a map, not {:?}.", d.kind();
				Invalid, Input, Mismatch)),
		};
		let mut out = Self::default();
		let mut date = None;
		for (k, v) in m.iter() {
			let key = match k {
				Dat::Str(s)	=> s.clone(),
				_		=> continue,
			};
			let val = match v {
				Dat::Str(s)	=> s.clone(),
				_		=> continue,
			};
			match key.as_str() {
				"slug"		=> out.slug = val,
				"kind"		=> out.kind = PostKind::of(&val),
				"state"		=> out.state = PostState::of(&val),
				// A record written before markup was a field carries none, and reads as Markdown --
				// which is what every such post was. The default falls out of `Markup::of`.
				"markup"	=> out.markup = Markup::of(&val),
				"source"	=> out.source = val,
				"date"		=> date = Some(val),
				// An unknown field is a field a later version wrote. Ignore it rather than refuse the
				// record: a reader that cannot read forwards makes every addition a migration.
				_		=> {},
			}
		}
		if out.slug.is_empty() {
			return Err(err!(
				"publish: a post record names no slug.";
				Invalid, Input, Missing));
		}
		out.date = date;
		Ok(out)
	}

	/// The record as a reader gets it.
	pub fn render(&self) -> Outcome<Post> {
		render_source(&self.source, self.slug.clone(), self.date.clone(), self.kind, self.markup)
	}
}


/// Writes a post, adding it to the index if it is new.
pub fn put<
	const UIDL: usize,
	UID:	NumIdDat<UIDL>,
	ENC:	Encrypter,
	KH:	Hasher,
	DB:	Database<UIDL, UID, ENC, KH>,
>(
	db:	&(Arc<RwLock<DB>>, UID),
	rec:	&Record,
	id:	&str,
)
	-> Outcome<()>
{
	let (db_arc, user) = db;
	{
		let guard = lock_read!(db_arc);
		res!(guard.insert(key_of(&rec.slug), rec.to_dat(), *user, None));
	}
	let mut slugs = res!(index(db, id));
	if !slugs.iter().any(|s| s == &rec.slug) {
		slugs.push(rec.slug.clone());
		res!(put_index(db, &slugs));
	}
	debug!("{}: publish: wrote '{}'", id, rec.slug);
	Ok(())
}

/// Reads one post's record.
pub fn get<
	const UIDL: usize,
	UID:	NumIdDat<UIDL>,
	ENC:	Encrypter,
	KH:	Hasher,
	DB:	Database<UIDL, UID, ENC, KH>,
>(
	db:	&(Arc<RwLock<DB>>, UID),
	slug:	&str,
)
	-> Outcome<Option<Record>>
{
	let (db_arc, _) = db;
	let guard = lock_read!(db_arc);
	match res!(guard.get(&key_of(slug), None)) {
		Some((val, _))	=> Ok(Some(res!(Record::from_dat(&val)))),
		None		=> Ok(None),
	}
}

/// Deletes a post and takes it out of the index.
pub fn delete<
	const UIDL: usize,
	UID:	NumIdDat<UIDL>,
	ENC:	Encrypter,
	KH:	Hasher,
	DB:	Database<UIDL, UID, ENC, KH>,
>(
	db:	&(Arc<RwLock<DB>>, UID),
	slug:	&str,
	id:	&str,
)
	-> Outcome<bool>
{
	let (db_arc, user) = db;
	let existed = {
		let guard = lock_read!(db_arc);
		res!(guard.delete(&key_of(slug), *user, None))
	};
	let slugs = res!(index(db, id));
	let kept: Vec<String> = slugs.into_iter().filter(|s| s != slug).collect();
	res!(put_index(db, &kept));
	Ok(existed)
}

/// Every record the store holds, whatever its state, newest first.
///
/// What an author gets. [`list`] is what a reader gets and so passes over drafts, which is exactly
/// what the author of a draft must be able to see: a composer that could not show the thing not yet
/// published would be showing everything except the work in progress.
///
/// A record the index names but the database does not hold is passed over with a complaint, rather
/// than failing the lot: one bad post should not take the others off the page.
pub fn list_records<
	const UIDL: usize,
	UID:	NumIdDat<UIDL>,
	ENC:	Encrypter,
	KH:	Hasher,
	DB:	Database<UIDL, UID, ENC, KH>,
>(
	db:	&(Arc<RwLock<DB>>, UID),
	id:	&str,
)
	-> Outcome<Vec<Record>>
{
	let slugs = res!(index(db, id));
	let mut recs = Vec::new();
	for slug in &slugs {
		match get(db, slug) {
			Ok(Some(r))	=> recs.push(r),
			Ok(None)	=> {
				// The index names a post the database does not hold. Derived data disagreeing with
				// what it was derived from is worth saying out loud.
				warn!("{}: publish: the index names '{}', which is not there", id, slug);
			}
			Err(e)		=> {
				warn!("{}: publish: skipping '{}': {}", id, slug, e);
			}
		}
	}
	// Newest first, and among posts of one date, or of none, by slug. The date descending and the
	// slug ascending are compared in opposite directions, so they are compared apart.
	recs.sort_by(|a, b| b.date.cmp(&a.date).then_with(|| a.slug.cmp(&b.slug)));
	Ok(recs)
}

/// Every live post, newest first, rendered.
///
/// A record that will not render is passed over with a complaint in the log rather than failing the
/// lot, on the same reasoning a directory's unreadable file is: one bad post should not take the
/// others off the page. The order is [`list_records`]'s.
pub fn list<
	const UIDL: usize,
	UID:	NumIdDat<UIDL>,
	ENC:	Encrypter,
	KH:	Hasher,
	DB:	Database<UIDL, UID, ENC, KH>,
>(
	db:	&(Arc<RwLock<DB>>, UID),
	id:	&str,
)
	-> Outcome<Vec<Post>>
{
	let recs = res!(list_records(db, id));
	let mut posts = Vec::new();
	for rec in &recs {
		if rec.state != PostState::Live {
			continue;
		}
		match rec.render() {
			Ok(p)	=> posts.push(p),
			Err(e)	=> warn!("{}: publish: '{}' will not render: {}", id, rec.slug, e),
		}
	}
	Ok(posts)
}

/// The index: every slug the store holds.
fn index<
	const UIDL: usize,
	UID:	NumIdDat<UIDL>,
	ENC:	Encrypter,
	KH:	Hasher,
	DB:	Database<UIDL, UID, ENC, KH>,
>(
	db:	&(Arc<RwLock<DB>>, UID),
	_id:	&str,
)
	-> Outcome<Vec<String>>
{
	let (db_arc, _) = db;
	let guard = lock_read!(db_arc);
	let val = match res!(guard.get(&dat!(INDEX_KEY), None)) {
		Some((v, _))	=> v,
		// No index is an empty store, not an error: a site that has published nothing is a site, and
		// its index is the empty list it never wrote.
		None		=> return Ok(Vec::new()),
	};
	let items = match &val {
		Dat::List(items)	=> items.clone(),
		Dat::Vek(vek)		=> vek.as_slice().to_vec(),
		_			=> return Err(err!(
			"publish: the index must be a list, not {:?}.", val.kind();
			Invalid, Input, Mismatch)),
	};
	let mut out = Vec::new();
	for item in &items {
		if let Dat::Str(s) = item {
			out.push(s.clone());
		}
	}
	Ok(out)
}

/// Writes the index.
fn put_index<
	const UIDL: usize,
	UID:	NumIdDat<UIDL>,
	ENC:	Encrypter,
	KH:	Hasher,
	DB:	Database<UIDL, UID, ENC, KH>,
>(
	db:	&(Arc<RwLock<DB>>, UID),
	slugs:	&[String],
)
	-> Outcome<()>
{
	let (db_arc, user) = db;
	let list = Dat::List(slugs.iter().map(|s| dat!(s.clone())).collect());
	let guard = lock_read!(db_arc);
	res!(guard.insert(dat!(INDEX_KEY), list, *user, None));
	Ok(())
}

/// Rebuilds the index from what the database actually holds.
///
/// The repair. Scans, which is the expensive thing the index exists to avoid, so this is for putting
/// the index right after something has gone wrong with it -- not for serving a page.
///
/// # A scan is not a list of what is there
///
/// [`Database::delete`] "deletes the given key ... **or at least marks it for deletion**", and a
/// marked key still comes back from a scan. So every key the scan offers is confirmed with a read,
/// and one that does not read back is one that is gone.
///
/// Without that read this would resurrect every post ever deleted, the next time anything repaired
/// the index, silently and long after the deletion. The extra read per key is the price of the
/// repair being a repair.
pub fn rebuild_index<
	const UIDL: usize,
	UID:	NumIdDat<UIDL>,
	ENC:	Encrypter,
	KH:	Hasher,
	DB:	Database<UIDL, UID, ENC, KH>,
>(
	db:	&(Arc<RwLock<DB>>, UID),
	id:	&str,
)
	-> Outcome<usize>
{
	let (db_arc, _) = db;
	let found = {
		let guard = lock_read!(db_arc);
		let mut opts = ScanOpts::default();
		opts.prefix = Some(dat!(KEY_PREFIX));
		opts.include_values = false;
		res!(guard.scan(&opts, None))
	};
	let mut slugs = Vec::new();
	let mut marked = 0;
	for (k, _, _) in &found {
		let s = match k {
			Dat::Str(s)	=> s,
			_		=> continue,
		};
		let slug = match s.strip_prefix(KEY_PREFIX) {
			Some(slug)	=> slug,
			None		=> continue,
		};
		// The scan said the key is there; the read says whether it still means anything.
		match res!(get(db, slug)) {
			Some(_)	=> slugs.push(slug.to_string()),
			None	=> marked += 1,
		}
	}
	slugs.sort();
	let n = slugs.len();
	res!(put_index(db, &slugs));
	if marked > 0 {
		debug!("{}: publish: the scan offered {} deleted keys, which were not taken", id, marked);
	}
	info!("{}: publish: index rebuilt, {} posts", id, n);
	Ok(n)
}

/// Reads a directory of Markdown into the store.
///
/// How prose that already exists gets in, and the reason a directory stays a first-class way to write
/// even once the store is the source: a file is still where most prose starts.
///
/// A slug the store already holds is overwritten, so importing twice is importing once. That makes the
/// import safe to repeat, which is what anyone will do with it.
pub fn import_dir<
	const UIDL: usize,
	UID:	NumIdDat<UIDL>,
	ENC:	Encrypter,
	KH:	Hasher,
	DB:	Database<UIDL, UID, ENC, KH>,
>(
	db:	&(Arc<RwLock<DB>>, UID),
	dir:	&str,
	id:	&str,
)
	-> Outcome<usize>
{
	let sources = res!(super::read_sources(dir, id));
	let mut n = 0;
	for (stem, source) in sources {
		let (date, slug) = split_date(&stem);
		let rec = Record {
			slug,
			// Everything a directory holds is live: a file on disk is not a draft, or it would not be
			// on the disk of a server.
			kind:	PostKind::Note,
			state:	PostState::Live,
			markup:	Markup::Markdown,
			date,
			source,
		};
		res!(put(db, &rec, id));
		n += 1;
	}
	info!("{}: publish: imported {} posts from '{}'", id, n, dir);
	Ok(n)
}

#[cfg(test)]
mod tests {
	use super::*;

	/// A record survives the trip through a daticle, including a date it does not have.
	#[test]
	fn test_a_record_round_trips_00() -> Outcome<()> {
		let rec = Record {
			slug:	fmt!("on-rent"),
			kind:	PostKind::Essay,
			state:	PostState::Draft,
			markup:	Markup::Djot,
			date:	Some(fmt!("2026-07-17")),
			source:	fmt!("# On rent\n\nWords.\n"),
		};
		let back = res!(Record::from_dat(&rec.to_dat()));
		assert_eq!(back, rec);

		let undated = Record { date: None, ..rec };
		let back = res!(Record::from_dat(&undated.to_dat()));
		assert_eq!(back, undated);
		assert_eq!(back.date, None);
		Ok(())
	}

	/// A field a later version wrote does not stop this one reading the record.
	#[test]
	fn test_an_unknown_field_is_ignored_01() -> Outcome<()> {
		let mut m = DaticleMap::new();
		m.insert(dat!("slug"),		dat!("on-rent"));
		m.insert(dat!("kind"),		dat!("note"));
		m.insert(dat!("state"),		dat!("live"));
		m.insert(dat!("source"),	dat!("Words."));
		m.insert(dat!("mood"),		dat!("wistful"));
		let rec = res!(Record::from_dat(&Dat::Map(m)));
		assert_eq!(rec.slug, "on-rent");
		assert_eq!(rec.source, "Words.");
		assert_eq!(rec.state, PostState::Live);
		// A record written before markup was a field carries no markup key, and reads as Markdown --
		// which every such post was.
		assert_eq!(rec.markup, Markup::Markdown);
		Ok(())
	}

	/// A state this version cannot read is a draft, not a publication. The safe reading of a word
	/// nobody understands is that the post is not ready.
	#[test]
	fn test_an_unreadable_state_is_a_draft_04() -> Outcome<()> {
		let mut m = DaticleMap::new();
		m.insert(dat!("slug"),		dat!("on-rent"));
		m.insert(dat!("state"),		dat!("scheduled-for-tuesday"));
		m.insert(dat!("source"),	dat!("Words."));
		let rec = res!(Record::from_dat(&Dat::Map(m)));
		assert_eq!(rec.state, PostState::Draft);
		Ok(())
	}

	/// A record with no slug is not a record: nothing could address it.
	#[test]
	fn test_a_record_without_a_slug_is_refused_02() -> Outcome<()> {
		let d = create_dat_ordmap(vec![(dat!("source"), dat!("Words."))]);
		assert!(Record::from_dat(&d).is_err());
		Ok(())
	}

	/// A key is the prefix and the slug, so a scan for the prefix finds posts and nothing else.
	#[test]
	fn test_a_key_is_prefixed_03() -> Outcome<()> {
		assert_eq!(key_of("on-rent"), dat!("publish/post/on-rent"));
		Ok(())
	}
}
