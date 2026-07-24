//! The posts as JSON, for a page that would rather render them itself.
//!
//! The convenience, not the point. A post's canonical form is a [page](super::page): a URL, HTML in
//! the first response, and the tags a card is built from. This exists so an app that is already a
//! running page can show its posts inline without a navigation, and it hands over prose that was
//! rendered on the way out, so there is one renderer rather than one per client.
//!
//! Served from the same prefix as everything else here, because the prefix is the module's and a post
//! is a post however it is asked for. A slug cannot collide with this: `index.json` is not a name a
//! slug may wear, punctuation not being allowed in one.

use crate::srv::cache;
use crate::srv::publish::{
	Author,
	Post,
	PublishConfig,
	date_text,
	read_mins,
};

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


/// Serves the posts as JSON: rendered, newest first.
///
/// Beside the posts go the two things a page cannot work out from them: the authors they name,
/// resolved to a face, and the categories the site offers. A post carries its author as a login
/// username, which is a hash and shows a reader nothing, and it carries only the categories it wears
/// rather than the ones it could have worn -- so a client drawing its own filter would otherwise
/// offer a taxonomy narrower than the site's and a row of hashes for faces.
pub fn serve(
	cfg:		&PublishConfig,
	posts:		&[Post],
	authors:	&[Author],
	id:		&str,
)
	-> Outcome<HttpMessage>
{
	// A post's author, by the handle the page shows, resolved through the authors this request already
	// read. The stored username is the SHA-256 of a passphrase and never leaves the server.
	let handle_of = |username: &str| -> String {
		authors.iter().find(|a| a.username == username)
			.map(|a| a.handle.clone())
			.unwrap_or_default()
	};
	let list = posts.iter()
		.map(|p| {
			let mut fields = vec![
				(dat!("slug"),		dat!(p.slug.clone())),
				(dat!("title"),		dat!(p.title.clone())),
				// The author, by the public handle the faces below carry, so a page can group posts
				// under one. Empty where none is named, or where the author could not be resolved,
				// which draws as no author rather than as a missing one.
				(dat!("author"),	dat!(handle_of(&p.author))),
				(dat!("url"),		dat!(cfg.path_of(&p.slug))),
				(dat!("excerpt"),	dat!(p.excerpt.clone())),
				(dat!("html"),		dat!(p.html.clone())),
				// Reading time in whole minutes, so a filter can offer a min/max slider without recounting
				// words in the client. The figure the post's own badge shows, from the one definition.
				(dat!("read_mins"),	dat!(read_mins(p.words) as u64)),
				// The tags and categories, always present as arrays so a page reading this need not ask
				// whether the key is there -- an empty post carries the empty list, the same thing said
				// once. Each is a plain string the store already normalised.
				(dat!("tags"),		Dat::List(p.tags.iter().map(|t| dat!(t.clone())).collect())),
				(dat!("categories"),	Dat::List(p.categories.iter().map(|c| dat!(c.clone())).collect())),
			];
			// A post without a date carries no date key, rather than a key saying nothing. The reader
			// asks whether the post has one; it should not also have to ask what a date of nothing means.
			if let Some(d) = &p.date {
				fields.push((dat!("date"), dat!(d.clone())));
				// The same instant, said the way a person says it, so a page showing this does not
				// have to know that the stored form is ISO.
				fields.push((dat!("date_text"), dat!(date_text(d))));
			}
			create_dat_ordmap(fields)
		})
		.collect::<Vec<_>>();

	// The authors, each with the name and avatar a reader sees, keyed by the username a post stores.
	// A client matches a post to a face on that username, as the server's own filter does.
	let faces = authors.iter()
		.map(|a| create_dat_ordmap(vec![
			(dat!("handle"),	dat!(a.handle.clone())),
			(dat!("name"),		dat!(a.name.clone())),
			(dat!("avatar"),	dat!(a.avatar.clone())),
			// What the author writes about, which a page drawing its own reader shows above the
			// posts the way the index page does.
			(dat!("bio"),		dat!(a.bio.clone())),
			// The letter drawn where there is no avatar, worked out once here rather than in every
			// client that would have to know the same fallback rule.
			(dat!("initial"),	dat!(a.initial())),
		]))
		.collect::<Vec<_>>();

	let body_dat = create_dat_ordmap(vec![
		(dat!("posts"), Dat::List(list)),
		(dat!("authors"), Dat::List(faces)),
		// The site's whole category vocabulary, in the order the config gives, so a client's checkboxes
		// stand in that order and offer what the composer offers.
		(dat!("categories"), Dat::List(cfg.categories.iter().map(|c| dat!(c.clone())).collect())),
	]);
	let json_cfg = EncoderConfig::<(), ()>::json(None);
	let body_json = res!(body_dat.encode_string_with_config(&json_cfg));

	info!("{}: publish: json, {} posts", id, posts.len());

	let mut resp = HttpMessage::ok_respond_with_text(body_json);
	resp = resp.with_field(
		HeaderName::ContentType,
		HeaderFieldValue::Generic(fmt!("application/json")),
	);
	// An app drawing its own stream from this must see a publication at once, and will not
	// think to force a refresh.
	Ok(cache::generated(resp))
}


#[cfg(test)]
mod tests {
	use super::*;

	use crate::srv::publish::Source;

	fn cfg() -> PublishConfig {
		PublishConfig {
			path:			fmt!("/asides"),
			dir:			fmt!("/nonexistent"),
			source:			Source::Dir,
			title:			fmt!("Asides"),
			site_name:		fmt!("Elearnity"),
			base_url:		fmt!("https://example.com"),
			css:			vec![],
			creds:			Default::default(),
			comments:		true,
			comment_rate_secs:	0,
			comment_rate_hourly:	0,
			newsletter_from:	String::new(),
			categories:		vec![fmt!("Personal"), fmt!("Big Ideas")],
			default_author:		String::new(),
			logo:			String::new(),
			home:			String::new(),
		}
	}

	fn post() -> Post {
		Post {
			slug:		fmt!("on-rent"),
			title:		fmt!("On rent"),
			author:		fmt!("9f3ac1"),
			categories:	vec![fmt!("Big Ideas")],
			date:		Some(fmt!("2026-07-17")),
			words:		420,
			excerpt:	fmt!("An opening sentence."),
			html:		fmt!("<p>An opening sentence.</p>\n"),
			also_on:	Vec::new(),
			tags:		vec![fmt!("rent")],
		}
	}

	/// The feed a page draws itself from carries the two things it cannot work out from the posts: the
	/// faces behind the usernames, and the whole category vocabulary the site offers.
	#[test]
	fn test_the_json_carries_faces_and_a_taxonomy_00() -> Outcome<()> {
		let authors = vec![Author {
			username:	fmt!("9f3ac1"),
			handle:		fmt!("qv7m2ab9dz"),
			name:		fmt!("Jason"),
			avatar:		String::new(),
			bio:		fmt!("Notes on rent."),
		}];
		let resp = res!(serve(&cfg(), &[post()], &authors, "test"));
		let body = String::from_utf8_lossy(&resp.body).to_string();
		// The public handle, and nowhere the login username, which is the SHA-256 of a passphrase.
		assert!(body.contains(r#""handle": "qv7m2ab9dz""#), "no author handle: {}", body);
		assert!(!body.contains("9f3ac1"), "the login username reached the feed: {}", body);
		assert!(body.contains(r#""author": "qv7m2ab9dz""#), "a post is not keyed to its author: {}", body);
		assert!(body.contains(r#""name": "Jason""#), "no author name: {}", body);
		// The letter a client draws where an author has no picture, settled here.
		assert!(body.contains(r#""initial": "J""#), "no drawn initial: {}", body);
		// What the author says they write about, which a client shows above the posts.
		assert!(body.contains(r#""bio": "Notes on rent.""#), "no description: {}", body);
		// Every category the site offers, not only the one the post wears.
		assert!(body.contains(r#""Personal""#), "an unworn category is still offered: {}", body);
		assert!(body.contains(r#""Big Ideas""#), "no category with a space: {}", body);
		assert!(body.contains(r#""read_mins": 3"#), "no reading time: {}", body);
		Ok(())
	}

	/// The feed says it may not be served from a store unasked. Without that an app redraws its
	/// stream from a copy taken before the post was published, and only a forced refresh gets past it.
	#[test]
	fn test_the_json_is_never_served_from_a_store_unasked_02() -> Outcome<()> {
		let resp = res!(serve(&cfg(), &[post()], &[], "test"));
		let held = res!(resp.header.fields.get_one(&HeaderName::CacheControl).ok_or_else(||
			err!("The feed carried no cache directive, so a store is free to guess one."; Missing)));
		assert_eq!(fmt!("{}", held), "no-cache");
		Ok(())
	}

	/// A site with no authors and no posts still answers the three keys, each an empty list, so a page
	/// reading it never has to ask whether a key is there.
	#[test]
	fn test_an_empty_site_still_answers_in_shape_01() -> Outcome<()> {
		let resp = res!(serve(&cfg(), &[], &[], "test"));
		let body = String::from_utf8_lossy(&resp.body).to_string();
		assert!(body.contains(r#""posts": []"#), "no empty post list: {}", body);
		assert!(body.contains(r#""authors": []"#), "no empty author list: {}", body);
		Ok(())
	}
}
