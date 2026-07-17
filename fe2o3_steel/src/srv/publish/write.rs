//! The write side: what changes a site's prose, and who is allowed to.
//!
//! # Who
//!
//! The dashboard's session, the same one `/admin` issues. This module runs inside the server, beside
//! the wallet and the admin list, so the identity that unsealed the server is already here and is the
//! right one: publishing is something the operator of a site does, and on a personal site the operator
//! is the author.
//!
//! An earlier plan had this reusing the *app's* login and a role flag on the user record. That was
//! written when the module was going to live in the app's own process, where the dashboard's session
//! is not visible. It lives here now, so it uses what is here.
//!
//! # What
//!
//! Importing a directory, for now. The composer follows, and posts to the same place under the same
//! gate.

use crate::srv::{
	admin::{
		handler::extract_principal,
		state::AdminState,
	},
	publish::{
		PublishConfig,
		Source,
		store,
	},
};

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_iop_crypto::enc::Encrypter;
use oxedyne_fe2o3_iop_db::api::Database;
use oxedyne_fe2o3_iop_hash::api::Hasher;
use oxedyne_fe2o3_jdat::{
	prelude::*,
	id::NumIdDat,
	string::enc::EncoderConfig,
};
use oxedyne_fe2o3_net::http::{
	fields::{
		HeaderFieldValue,
		HeaderFields,
		HeaderName,
	},
	msg::HttpMessage,
	status::HttpStatus,
};

use std::sync::{
	Arc,
	RwLock,
};


/// Serves a `POST` that belongs to the published prose.
///
/// Returns `None` when the path is not one this module writes to, so the caller can carry on down its
/// own routing rather than have every unrecognised POST under the prefix become an error here.
pub async fn handle_post<
	const UIDL: usize,
	UID:	NumIdDat<UIDL>,
	ENC:	Encrypter,
	KH:	Hasher,
	DB:	Database<UIDL, UID, ENC, KH>,
>(
	cfg:		&PublishConfig,
	admin_state:	Option<&Arc<AdminState>>,
	db:		Option<&(Arc<RwLock<DB>>, UID)>,
	path:		&str,
	headers:	&Arc<HeaderFields>,
	id:		&str,
)
	-> Outcome<Option<HttpMessage>>
{
	if path != cfg.import_path() {
		return Ok(None);
	}

	// The gate. Refused before anything is read, and before the response says whether the route even
	// does anything: an unauthenticated caller learns nothing here it did not already know.
	let state = match admin_state {
		Some(s)	=> s,
		None	=> {
			// No dashboard configured means no way to be authorised, so this route cannot be used.
			// Say nothing about why.
			return Ok(Some(refused()));
		}
	};
	let principal = match extract_principal(state.as_ref(), headers) {
		Some(p)	=> p,
		None	=> {
			warn!("{}: publish: an unauthenticated caller tried to import", id);
			return Ok(Some(refused()));
		}
	};

	if cfg.source != Source::Store {
		return Ok(Some(json_error(
			HttpStatus::BadRequest,
			"this vhost serves its posts from a directory, so there is nothing to import them into; \
			set 'source' to 'store' first",
		)));
	}

	let db = match db {
		Some(db)	=> db,
		None		=> return Ok(Some(json_error(
			HttpStatus::InternalServerError,
			"this vhost has no database configured",
		))),
	};

	let n = match store::import_dir(db, &cfg.dir, id) {
		Ok(n)	=> n,
		Err(e)	=> {
			error!(e, "{}: publish: import from '{}' failed", id, cfg.dir);
			return Ok(Some(json_error(
				HttpStatus::InternalServerError,
				"the directory could not be read",
			)));
		}
	};

	info!("{}: publish: '{}' imported {} posts from '{}'", id, principal.name, n, cfg.dir);

	let body = create_dat_ordmap(vec![
		(dat!("imported"),	dat!(n as u64)),
		(dat!("from"),		dat!(cfg.dir.clone())),
	]);
	let json_cfg = EncoderConfig::<(), ()>::json(None);
	let json = res!(body.encode_string_with_config(&json_cfg));
	Ok(Some(json_response(HttpStatus::OK, json)))
}

/// The answer to a caller with no business here.
///
/// A 404 rather than a 401, and no `WWW-Authenticate`: the dashboard's session is a cookie a browser
/// already has or does not, so there is nothing to prompt for, and a 401 would confirm the route
/// exists to someone who should not know.
fn refused() -> HttpMessage {
	HttpMessage::respond_with_text(HttpStatus::NotFound, "Not found.")
}

/// An error a caller can read.
fn json_error(status: HttpStatus, msg: &str) -> HttpMessage {
	let body = create_dat_ordmap(vec![(dat!("error"), dat!(msg.to_string()))]);
	let json_cfg = EncoderConfig::<(), ()>::json(None);
	match body.encode_string_with_config(&json_cfg) {
		Ok(json)	=> json_response(status, json),
		// The error about the error. Say the plain thing rather than nothing.
		Err(_)		=> HttpMessage::respond_with_text(status, msg),
	}
}

/// A JSON response with the type a caller expects.
fn json_response(status: HttpStatus, body: String) -> HttpMessage {
	let mut resp = HttpMessage::respond_with_text(status, body);
	resp = resp.with_field(
		HeaderName::ContentType,
		HeaderFieldValue::Generic(fmt!("application/json")),
	);
	resp
}
