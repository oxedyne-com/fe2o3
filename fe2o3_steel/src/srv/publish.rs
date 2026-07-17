//! Publishing the prose a site holds, as the browser wants it.
//!
//! The first cut of the publish module: a directory of Markdown becomes a list of rendered posts,
//! newest first, for a site to show under whatever name it gives them. Prose reaches HTML through the
//! [document tree](oxedyne_fe2o3_text::doc), so a post is rendered by the same tree a signed document
//! is built from, and gains any front-end that tree gains.
//!
//! # Why a directory, and what replaces it
//!
//! Posts belong in the vhost's database, and will live there. They cannot yet: the database is held by
//! the server process, and an API handler runs in whichever process registered it -- for an app that
//! keeps its handlers in a separate binary, that is not the server. So the store waits on this module
//! sitting beside the database rather than beside the request.
//!
//! A directory of Markdown is not a stand-in for that. It is a real way to write, and prose written
//! this way is prose the store reads in later: the file is the source either way, and only what holds
//! it changes.
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

use crate::srv::{
	api::ApiHandler,
	cfg::ApiRoute,
};

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_jdat::prelude::*;
use oxedyne_fe2o3_jdat::string::enc::EncoderConfig;
use oxedyne_fe2o3_net::http::{
	fields::{
		HeaderFieldValue,
		HeaderFields,
		HeaderName,
	},
	header::HttpMethod,
	loc::HttpLocator,
	msg::HttpMessage,
	status::HttpStatus,
};
use oxedyne_fe2o3_text::doc::{
	html,
	markdown,
};

use std::{
	fs,
	future::Future,
	path::Path,
	pin::Pin,
	sync::Arc,
};

use tokio_rustls::rustls::ClientConfig;


/// Directory holding the posts, where a route's `dir` config names no other.
pub const DIR_DEFAULT: &str = "./www/public/content/posts";

/// The extension a post wears.
const EXT: &str = "md";


/// One post, as it reaches the browser.
struct Post {
	/// The post's name in a URL, taken from its file.
	slug:	String,
	/// The post's own most prominent heading, or its slug where it has none.
	title:	String,
	/// The date its file names, where its file names one.
	date:	Option<String>,
	/// The prose, rendered.
	html:	String,
}


/// Lists a site's posts, rendered, newest first.
///
/// Registered by an app under whatever name it likes, and pointed at a directory by the route's `dir`
/// config. An app that shows its posts under a name of its own gives them that name in its own config;
/// this names them nothing.
pub struct Posts;

impl ApiHandler for Posts {
	fn handle<'a>(
		&'a self,
		route:		&'a ApiRoute,
		_method:	HttpMethod,
		_loc:		&'a HttpLocator,
		_body:		&'a [u8],
		_req_headers:	&'a HeaderFields,
		_tls_client:	&'a Option<Arc<ClientConfig>>,
		id:		&'a str,
	)
		-> Pin<Box<dyn Future<Output = Outcome<HttpMessage>> + Send + 'a>>
	{
		Box::pin(handle_impl(route, id))
	}
}

async fn handle_impl(
	route:	&ApiRoute,
	id:	&str,
)
	-> Outcome<HttpMessage>
{
	let dir = match route.config.iter().find(|(k, _)| k == "dir") {
		Some((_, v))	=> v.as_str(),
		None		=> DIR_DEFAULT,
	};

	let posts = match read_all(dir, id) {
		Ok(posts) => posts,
		Err(e) => {
			// A directory that cannot be read is the site's mistake, not the reader's, and the reader
			// should be told plainly rather than shown an empty shelf that looks like the truth.
			warn!("{}: posts: cannot read '{}': {}", id, dir, e);
			return Ok(HttpMessage::respond_with_text(
				HttpStatus::InternalServerError,
				"the posts cannot be read",
			));
		}
	};

	info!("{}: posts: {} from '{}'", id, posts.len(), dir);

	let list = posts.iter()
		.map(|p| {
			let mut fields = vec![
				(dat!("slug"),	dat!(p.slug.clone())),
				(dat!("title"),	dat!(p.title.clone())),
				(dat!("html"),	dat!(p.html.clone())),
			];
			// A post without a date carries no date key, rather than a key saying nothing. The reader
			// asks whether the post has one; it should not also have to ask what a date of nothing means.
			if let Some(d) = &p.date {
				fields.push((dat!("date"), dat!(d.clone())));
			}
			create_dat_ordmap(fields)
		})
		.collect::<Vec<_>>();

	let body_dat = create_dat_ordmap(vec![
		(dat!("posts"), Dat::List(list)),
	]);
	let json_cfg = EncoderConfig::<(), ()>::json(None);
	let body_json = res!(body_dat.encode_string_with_config(&json_cfg));

	let mut resp = HttpMessage::ok_respond_with_text(body_json);
	resp = resp.with_field(
		HeaderName::ContentType,
		HeaderFieldValue::Generic(fmt!("application/json")),
	);
	Ok(resp)
}

/// Reads every post in a directory, newest first.
///
/// A file that will not read or will not parse is passed over with a complaint in the log rather than
/// failing the lot: one broken post should not take the others off the page, and the log is where its
/// author will look. The directory itself failing is a different thing, and is an error.
fn read_all(dir: &str, id: &str) -> Outcome<Vec<Post>> {
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
			html: html::render(&doc),
		});
	}

	// Newest first, and among posts of one date, or of none, by slug. Sorting by the date descending
	// and the slug ascending needs the two compared in opposite directions, so they are compared apart.
	posts.sort_by(|a, b| b.date.cmp(&a.date).then_with(|| a.slug.cmp(&b.slug)));
	Ok(posts)
}

/// Splits a leading `YYYY-MM-DD-` from a file's stem, giving the date it names and the slug that is
/// left. A stem that does not begin with a date is all slug.
///
/// The shape is checked rather than the value: a date is ten characters, digits where digits belong
/// and dashes where dashes belong, followed by a dash. `2026-13-45` passes, and is a date this does not
/// have to understand -- it sorts, which is all that is asked of it here.
fn split_date(stem: &str) -> (Option<String>, String) {
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
}
