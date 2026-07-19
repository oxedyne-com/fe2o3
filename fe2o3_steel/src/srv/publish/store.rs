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
	dest::{
		Delivery,
		DeliveryState,
	},
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

use std::collections::BTreeMap;
use std::sync::{
	Arc,
	RwLock,
};


/// The key every post's key begins with.
pub const KEY_PREFIX: &str = "publish/post/";

/// The key the list of slugs lives under.
pub const INDEX_KEY: &str = "publish/index";

/// The key every post's read tally begins with.
///
/// One key per post rather than one map for the site: a read touches only the post that was read, so
/// two posts being read at once do not contend, and a tally cannot take the whole site's counts with
/// it when it goes wrong.
pub const READS_PREFIX: &str = "publish/reads/";

/// The key the list of database-granted site admins lives under.
///
/// The companion to the config's [`site_admins`](crate::srv::cfg::VhostConfig::site_admins): the
/// admins a site grants itself from the browser, kept apart from the operator's failsafe list so the
/// two can be reasoned about separately. The functions in the ADMINS section below are its only
/// writers.
pub const ADMINS_KEY: &str = "publish/admins";


/// A post's read-tally key.
fn reads_key_of(slug: &str) -> Dat {
	let mut s = String::from(READS_PREFIX);
	s.push_str(slug);
	dat!(s)
}

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
	/// Where this post has been sent besides the site's own pages, one entry per remote. Empty for a
	/// post that lives only at home, which every post read from a directory does.
	pub deliveries:	Vec<Delivery>,
	/// The tags the author gave it, normalised and deduped, in first-seen order. Empty for an
	/// untagged post, which is what every record written before tags were a field reads as.
	pub tags:	Vec<String>,
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
		// A post sent nowhere carries no deliveries key, for the same reason it carries no empty date:
		// an absent key and an empty list say the same thing, and one of them is enough.
		if !self.deliveries.is_empty() {
			let list = Dat::List(self.deliveries.iter().map(|d| d.to_dat()).collect());
			m.insert(dat!("deliveries"), list);
		}
		// A post with no tags carries no tags key, for the same reason it carries no empty deliveries:
		// an absent key and an empty list say the same thing, and one of them is enough.
		if !self.tags.is_empty() {
			let list = Dat::List(self.tags.iter().map(|t| dat!(t.clone())).collect());
			m.insert(dat!("tags"), list);
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
		// Deliveries are a list of maps, not a string, so they are read apart from the string fields
		// above rather than in that loop. A list and a vek are both written as a list and both mean one,
		// as everywhere else in this grammar. A delivery to a destination this version does not know is
		// dropped, not fatal: a later version naming a new remote must not make its posts unreadable.
		let items = match m.get(&dat!("deliveries")) {
			Some(Dat::List(items))	=> items.as_slice(),
			Some(Dat::Vek(vek))	=> vek.as_slice(),
			_			=> &[],
		};
		for item in items {
			if let Some(d) = Delivery::from_dat(item) {
				out.deliveries.push(d);
			}
		}
		// Tags are a list of strings, read apart from the string fields above like the deliveries. An
		// absent key is a post with no tags, which every record written before tags were a field is. A
		// tag is taken as stored here; the composer is where one is normalised and checked.
		let tags = match m.get(&dat!("tags")) {
			Some(Dat::List(items))	=> items.as_slice(),
			Some(Dat::Vek(vek))	=> vek.as_slice(),
			_			=> &[],
		};
		for item in tags {
			if let Dat::Str(s) = item {
				out.tags.push(s.clone());
			}
		}
		Ok(out)
	}

	/// The record as a reader gets it, "also on …" links and all.
	pub fn render(&self) -> Outcome<Post> {
		let mut post = res!(render_source(
			&self.source, self.slug.clone(), self.date.clone(), self.kind, self.markup));
		// The remotes the post actually reached, with the address it landed at, for the backlinks. Only
		// a sent delivery has a permalink; a queued or failed one has nowhere to point.
		post.also_on = self.deliveries.iter().filter_map(|d| match &d.state {
			DeliveryState::Sent { permalink, .. } if !permalink.is_empty()	=>
				Some((d.dest, permalink.clone())),
			_								=> None,
		}).collect();
		// The tags travel with the rendered post, so every read surface can show them without knowing a
		// record from a directory.
		post.tags = self.tags.clone();
		Ok(post)
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

/// Every tag any post carries, drafts included, sorted and deduped.
///
/// The site's accumulating vocabulary: a tag is offered as soon as one post wears it, so the
/// composer's palette grows as the site does, and a draft counts -- a tag is available the moment it
/// is first used.
///
/// Built from [`list_records`], which is the index read and a read per post the composer already
/// pays, rather than [`Database::scan`], which is the expensive thing the index exists to avoid.
pub fn all_tags<
	const UIDL: usize,
	UID:	NumIdDat<UIDL>,
	ENC:	Encrypter,
	KH:	Hasher,
	DB:	Database<UIDL, UID, ENC, KH>,
>(
	db:	&(Arc<RwLock<DB>>, UID),
	id:	&str,
)
	-> Outcome<Vec<String>>
{
	let recs = res!(list_records(db, id));
	let mut out: Vec<String> = Vec::new();
	for rec in &recs {
		for t in &rec.tags {
			if !out.iter().any(|s| s == t) {
				out.push(t.clone());
			}
		}
	}
	out.sort();
	Ok(out)
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

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ READS                                                                     │
// └───────────────────────────────────────────────────────────────────────────┘

/// Reads a post's tally.
///
/// An absent key is a post nobody has read yet, which is nought rather than an error -- the same
/// reasoning [`index`] takes for a store that has published nothing.
pub fn reads_get<
	const UIDL: usize,
	UID:	NumIdDat<UIDL>,
	ENC:	Encrypter,
	KH:	Hasher,
	DB:	Database<UIDL, UID, ENC, KH>,
>(
	db:	&(Arc<RwLock<DB>>, UID),
	slug:	&str,
)
	-> Outcome<u64>
{
	let (db_arc, _) = db;
	let guard = lock_read!(db_arc);
	match res!(guard.get(&reads_key_of(slug), None)) {
		Some((Dat::U64(n), _))	=> Ok(n),
		Some((v, _))		=> Err(err!(
			"publish: a read tally must be a count, not {:?}.", v.kind();
			Invalid, Input, Mismatch)),
		None			=> Ok(0),
	}
}

/// Adds one to a post's tally and answers the new total.
///
/// Read-add-write rather than an atomic increment, because the store offers no increment. Two reads
/// landing together can therefore lose one of the two. That is accepted deliberately: this counts
/// roughly how many people read a post, a question that does not become better answered by a lock
/// held across the render path of every request. It is not billing.
pub fn reads_bump<
	const UIDL: usize,
	UID:	NumIdDat<UIDL>,
	ENC:	Encrypter,
	KH:	Hasher,
	DB:	Database<UIDL, UID, ENC, KH>,
>(
	db:	&(Arc<RwLock<DB>>, UID),
	slug:	&str,
)
	-> Outcome<u64>
{
	let now = res!(reads_get(db, slug)).saturating_add(1);
	let (db_arc, user) = db;
	let guard = lock_read!(db_arc);
	res!(guard.insert(reads_key_of(slug), dat!(now), *user, None));
	Ok(now)
}

/// Every post's tally, by slug.
///
/// What the reports page aggregates. A tally whose post has since been deleted is still returned --
/// the caller knows which slugs it published and can decide whether a count without a post is worth
/// showing; throwing it away here would be this function deciding that on their behalf.
pub fn reads_all<
	const UIDL: usize,
	UID:	NumIdDat<UIDL>,
	ENC:	Encrypter,
	KH:	Hasher,
	DB:	Database<UIDL, UID, ENC, KH>,
>(
	db:	&(Arc<RwLock<DB>>, UID),
	id:	&str,
)
	-> Outcome<BTreeMap<String, u64>>
{
	let (db_arc, _) = db;
	// The scan selects keys and the reads fetch values, which is not an optimisation to undo: scan v1
	// answers `Dat::Empty` for every value whatever `include_values` asks for, and says so in a log
	// line rather than an error. Asking it for values yields a tally of nothing, silently. This is the
	// same shape `rebuild_index` takes, for the same reason.
	let found = {
		let guard = lock_read!(db_arc);
		let mut opts = ScanOpts::default();
		opts.prefix = Some(dat!(READS_PREFIX));
		opts.include_values = false;
		res!(guard.scan(&opts, None))
	};
	let mut out = BTreeMap::new();
	for (k, _, _) in &found {
		let s = match k {
			Dat::Str(s)	=> s,
			_		=> continue,
		};
		let slug = match s.strip_prefix(READS_PREFIX) {
			Some(slug)	=> slug,
			None		=> continue,
		};
		// A tally that will not read is a bug elsewhere, and losing the whole page over one bad key
		// would be the wrong trade. It is logged and passed over.
		match reads_get(db, slug) {
			Ok(n)	=> { out.insert(slug.to_string(), n); }
			Err(e)	=> debug!(
				"{}: publish: the read tally for '{}' will not read: {}", id, slug, e),
		}
	}
	Ok(out)
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ ADMINS                                                                    │
// └───────────────────────────────────────────────────────────────────────────┘

/// The database-granted admins: every member id-hash the console has added.
///
/// The read that lets a site bootstrap its own administration without a config edit. An absent key is
/// a site that has granted none -- which every site is until the first admin claims it -- and reads as
/// the empty list, not an error, on the same reasoning [`index`] takes for a store that has published
/// nothing.
pub fn admins_get<
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
	let val = match res!(guard.get(&dat!(ADMINS_KEY), None)) {
		Some((v, _))	=> v,
		// No key is a site that has granted no admins from the browser, which is not an error: its
		// database admin list is the empty one it never wrote.
		None		=> return Ok(Vec::new()),
	};
	let items = match &val {
		Dat::List(items)	=> items.clone(),
		Dat::Vek(vek)		=> vek.as_slice().to_vec(),
		_			=> return Err(err!(
			"publish: the admin list must be a list, not {:?}.", val.kind();
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

/// Adds an id-hash to the database admin list, once.
///
/// Idempotent: a hash the list already holds is left as it is, so granting the same admin twice grants
/// them once. The caller owns validating the hash's shape; this stores what it is given.
pub fn admins_add<
	const UIDL: usize,
	UID:	NumIdDat<UIDL>,
	ENC:	Encrypter,
	KH:	Hasher,
	DB:	Database<UIDL, UID, ENC, KH>,
>(
	db:	&(Arc<RwLock<DB>>, UID),
	id:	&str,
	hash:	&str,
)
	-> Outcome<()>
{
	let mut hashes = res!(admins_get(db, id));
	if !hashes.iter().any(|h| h == hash) {
		hashes.push(hash.to_string());
		res!(put_admins(db, &hashes));
		debug!("{}: publish: granted site admin to '{}'", id, hash);
	}
	Ok(())
}

/// Removes an id-hash from the database admin list.
///
/// Only the database list: a hash the operator has pinned in config is not here to remove and stays an
/// admin regardless, which is the point of the config list being the failsafe. Removing a hash the
/// list does not hold is a no-op, not an error.
pub fn admins_remove<
	const UIDL: usize,
	UID:	NumIdDat<UIDL>,
	ENC:	Encrypter,
	KH:	Hasher,
	DB:	Database<UIDL, UID, ENC, KH>,
>(
	db:	&(Arc<RwLock<DB>>, UID),
	id:	&str,
	hash:	&str,
)
	-> Outcome<()>
{
	let hashes = res!(admins_get(db, id));
	let kept: Vec<String> = hashes.into_iter().filter(|h| h != hash).collect();
	res!(put_admins(db, &kept));
	debug!("{}: publish: revoked site admin from '{}'", id, hash);
	Ok(())
}

/// Writes the database admin list.
///
/// Private, and the only writer of [`ADMINS_KEY`] besides the two above that go through it: the list is
/// derived from nothing, so it is written whole where it changes and nowhere else.
fn put_admins<
	const UIDL: usize,
	UID:	NumIdDat<UIDL>,
	ENC:	Encrypter,
	KH:	Hasher,
	DB:	Database<UIDL, UID, ENC, KH>,
>(
	db:	&(Arc<RwLock<DB>>, UID),
	hashes:	&[String],
)
	-> Outcome<()>
{
	let (db_arc, user) = db;
	let list = Dat::List(hashes.iter().map(|s| dat!(s.clone())).collect());
	let guard = lock_read!(db_arc);
	res!(guard.insert(dat!(ADMINS_KEY), list, *user, None));
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
			// A file has been sent nowhere: a directory is prose, not a record of where it went.
			deliveries:	Vec::new(),
			// A directory carries no tags: there is no front matter to put them in, and a filename is
			// the slug and the date, nothing else.
			tags:	Vec::new(),
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
	use crate::srv::publish::dest::{
		Delivery,
		DeliveryState,
		Destination,
		Rendition,
	};

	/// A record survives the trip through a daticle, including a date it does not have and the
	/// deliveries it does.
	#[test]
	fn test_a_record_round_trips_00() -> Outcome<()> {
		let rec = Record {
			slug:	fmt!("on-rent"),
			kind:	PostKind::Essay,
			state:	PostState::Draft,
			markup:	Markup::Djot,
			date:	Some(fmt!("2026-07-17")),
			source:	fmt!("# On rent\n\nWords.\n"),
			deliveries:	vec![
				Delivery {
					dest:		Destination::Mastodon,
					rendition:	Rendition { text: fmt!("On rent https://x"), auto: false },
					state:		DeliveryState::Sent {
						at:		fmt!("2026-07-18T10:00:00Z"),
						permalink:	fmt!("https://m.example/1"),
					},
				},
				Delivery::new(Destination::Bluesky, Rendition::default()),
			],
			tags:	vec![fmt!("rust"), fmt!("web")],
		};
		let back = res!(Record::from_dat(&rec.to_dat()));
		assert_eq!(back, rec);

		let undated = Record { date: None, ..rec };
		let back = res!(Record::from_dat(&undated.to_dat()));
		assert_eq!(back, undated);
		assert_eq!(back.date, None);
		Ok(())
	}

	/// An untagged post writes no tags key, and a record with no tags key reads as untagged -- the
	/// absent key and the empty list saying the one thing.
	#[test]
	fn test_tags_follow_the_empty_list_idiom_05() -> Outcome<()> {
		let rec = Record {
			slug:	fmt!("on-rent"),
			source:	fmt!("Words."),
			tags:	Vec::new(),
			..Default::default()
		};
		// No tags, no key.
		if let Dat::Map(m) = rec.to_dat() {
			assert!(m.get(&dat!("tags")).is_none(), "an untagged post wrote a tags key");
		}
		// A record with no tags key reads as untagged.
		let mut m = DaticleMap::new();
		m.insert(dat!("slug"),		dat!("on-rent"));
		m.insert(dat!("source"),	dat!("Words."));
		let back = res!(Record::from_dat(&Dat::Map(m)));
		assert!(back.tags.is_empty(), "a record with no tags key read as tagged");

		// Tags given survive the trip, in order.
		let tagged = Record { tags: vec![fmt!("rust"), fmt!("web")], ..rec };
		let back = res!(Record::from_dat(&tagged.to_dat()));
		assert_eq!(back.tags, vec![fmt!("rust"), fmt!("web")]);
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
