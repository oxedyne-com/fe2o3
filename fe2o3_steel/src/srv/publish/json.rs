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

use crate::srv::publish::{
	Post,
	PublishConfig,
	date_text,
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
pub fn serve(cfg: &PublishConfig, posts: &[Post], id: &str) -> Outcome<HttpMessage> {
	let list = posts.iter()
		.map(|p| {
			let mut fields = vec![
				(dat!("slug"),		dat!(p.slug.clone())),
				(dat!("title"),		dat!(p.title.clone())),
				// Which the post is, because a page showing these cannot tell otherwise, and the
				// difference is the whole reason the two kinds exist. Without it a passing thought
				// and an essay arrive identical and get shown identically -- which is precisely the
				// furniture a note is defined by not wearing.
				(dat!("kind"),		dat!(p.kind.as_str().to_string())),
				(dat!("url"),		dat!(cfg.path_of(&p.slug))),
				(dat!("excerpt"),	dat!(p.excerpt.clone())),
				(dat!("html"),		dat!(p.html.clone())),
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

	let body_dat = create_dat_ordmap(vec![
		(dat!("posts"), Dat::List(list)),
	]);
	let json_cfg = EncoderConfig::<(), ()>::json(None);
	let body_json = res!(body_dat.encode_string_with_config(&json_cfg));

	info!("{}: publish: json, {} posts", id, posts.len());

	let mut resp = HttpMessage::ok_respond_with_text(body_json);
	resp = resp.with_field(
		HeaderName::ContentType,
		HeaderFieldValue::Generic(fmt!("application/json")),
	);
	Ok(resp)
}
