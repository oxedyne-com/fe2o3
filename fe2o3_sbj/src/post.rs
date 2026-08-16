//! `daimond/post/0` — a signed message payload.
//!
//! The second schema this container carries, and the first that is not a node tree. An oxeweb
//! document is a tree because a document is one; a message is a record with five fields, so its
//! payload is a single canonical map and the whole of §4 does not apply to it.
//!
//! Everything the sender means is inside the payload, which is what the envelope's `hash` covers
//! and therefore what the signature commits to. There is deliberately no `from` field: the author
//! is the envelope's `author`, so there is no second place to say who wrote it and no spoofable
//! name to disagree with the key. For the same reason there is no `created` field here — a
//! timestamp is the envelope's, advisory, and a payload that carried its own would be asserting a
//! clock nobody can check.
//!
//! The canonical rules of `SPEC.md` §3 apply unchanged, and are what makes one message one
//! address: fixed widths, a `Dat::Map` rather than an `OrdMap`, lowercase ASCII keys, absent
//! optional fields omitted rather than encoded as `none`, and [`decode`] re-encoding what it
//! decoded and demanding the same bytes back.
//!
//! `body` is a `BU32` and never a `BU8`. A `BU8` carries a single length byte and so truncates
//! silently past 255 bytes, which for a message body is a defect that would only appear once
//! somebody wrote a long one.

use crate::{
	canon,
	limit as sbj_limit,
};

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_jdat::{
	prelude::*,
	bdat::DecodeLimits,
};


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ KEYS                                                                      │
// └───────────────────────────────────────────────────────────────────────────┘

/// The message text.
pub const KEY_BODY:	&'static str = "body";
/// Per-message randomness, so that two identical messages are two addresses.
pub const KEY_NONCE:	&'static str = "nonce";
/// References, at most [`limit::REFS`] of them.
pub const KEY_REFS:	&'static str = "refs";
/// The address this message answers.
pub const KEY_REPLY_TO:	&'static str = "reply_to";
/// The recipient's public key.
pub const KEY_TO:	&'static str = "to";

/// A reference's own description, drawn only when resolution fails.
pub const KEY_FALLBACK:	&'static str = "fallback";
/// A reference's referent: a map with exactly one entry, whose key selects the kind.
pub const KEY_TARGET:	&'static str = "target";

/// A proposal on a forge repository.
pub const REF_PROPOSAL:	&'static str = "proposal";
/// A release build.
pub const REF_BUILD:	&'static str = "build";
/// A panel in the reader's own client.
pub const REF_PANEL:	&'static str = "panel";
/// A page of the in-app guide.
pub const REF_GUIDE:	&'static str = "guide";

/// A proposal's account.
pub const KEY_ACCOUNT:	&'static str = "account";
/// A proposal's repository.
pub const KEY_REPO:	&'static str = "repo";
/// A proposal's number.
pub const KEY_NUMBER:	&'static str = "number";
/// A build's identifier.
pub const KEY_ID:	&'static str = "id";
/// A panel's name.
pub const KEY_NAME:	&'static str = "name";
/// A guide page.
pub const KEY_PAGE:	&'static str = "page";
/// An anchor within a guide page.
pub const KEY_ANCHOR:	&'static str = "anchor";


/// Limits this schema enforces. Every one is a rejection, never a truncation.
pub mod limit {
	/// The most a message body may carry, in bytes of UTF-8.
	///
	/// A message is prose a person reads in a panel, not a document; anything longer wants to be a
	/// document, which this container already has a schema for. The number is revisable on
	/// evidence, as `SPEC.md` §5's are; what is fixed is that there is one, since a body with no
	/// ceiling is a body that sets the relay's storage.
	pub const BODY_BYTES:	usize = 8 * 1024;
	/// The most references one message may carry.
	///
	/// Four, because a reference is resolved lazily by the reader and each resolution is metered
	/// against that reader's own allowance. A message that could carry fifty would let a sender
	/// spend a stranger's quota by being opened.
	pub const REFS:	usize = 4;
	/// The exact width of the per-message nonce.
	pub const NONCE_BYTES:	usize = 16;
	/// The exact width of a public key.
	pub const KEY_BYTES:	usize = 32;
	/// The exact width of an address, matching the v0 hash scheme's digest.
	pub const ADDR_BYTES:	usize = 32;
	/// The most a reference's fallback description may carry, in bytes of UTF-8.
	pub const FALLBACK_BYTES:	usize = 256;
	/// The most any single reference field may carry, in bytes of UTF-8.
	pub const FIELD_BYTES:	usize = 128;
	/// Decoding depth for a payload of this schema.
	///
	/// A post is a flat record holding one list of maps, so three levels is already more than its
	/// shape can reach. It is far below `SPEC.md` §5's tree limit because nothing here recurses,
	/// and a limit set to what the shape needs refuses a nested value before it is looked at.
	pub const DEPTH:	usize = 8;
}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ REFERENCES                                                                │
// └───────────────────────────────────────────────────────────────────────────┘

/// What a reference points at.
///
/// An enum rather than a string and a bag of fields, because the four referents have four identity
/// schemes: a proposal is named by three parts, a build by one opaque id, a panel by a name the
/// reader's own client resolves, and a guide page by a page and an optional anchor. A single
/// "target string" would put the parsing in the renderer, which is where a malformed reference
/// becomes a drawing bug rather than a rejection.
///
/// All four are **public anchors**: globally named, resolvable by anybody holding a session, and
/// safe to send. Private, device-local pointers — a chat, a workspace file — are deliberately
/// absent, because the other party cannot dereference one and an interface must never draw a
/// pressable chip that will always fail.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
	/// A proposal on a forge repository.
	Proposal {
		/// The owning account.
		account:	String,
		/// The repository.
		repo:	String,
		/// The proposal number.
		number:	u32,
	},
	/// A release build, by the identifier a release stamp carries.
	Build {
		/// The build identifier.
		id:	String,
	},
	/// A panel in the reader's own client. A surface, not an object.
	Panel {
		/// The panel's name.
		name:	String,
	},
	/// A page of the in-app guide, and optionally an anchor within it.
	Guide {
		/// The page.
		page:	String,
		/// An anchor within the page.
		anchor:	Option<String>,
	},
}

impl Target {
	/// The key that selects this kind in the encoded form.
	pub fn key(&self) -> &'static str {
		match self {
			Self::Proposal { .. }	=> REF_PROPOSAL,
			Self::Build { .. }	=> REF_BUILD,
			Self::Panel { .. }	=> REF_PANEL,
			Self::Guide { .. }	=> REF_GUIDE,
		}
	}
}

/// One reference: what it points at, and what to say when that cannot be resolved.
///
/// The wire carries the referent and a fallback description, and **never a rendered title**. A
/// sender-supplied title is a lie waiting to happen, since a proposal can be renamed or closed
/// after the message is signed, and it is an injection surface besides — arbitrary sender text
/// drawn as though it were a forge record. The reader resolves the referent itself and draws the
/// fallback only on failure, as plain text, framed as the sender's own description of it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reference {
	/// What is pointed at.
	pub target:	Target,
	/// The sender's description, drawn only when resolution fails.
	pub fallback:	String,
}

impl Reference {
	/// Encodes this reference as a canonical daticle.
	pub fn to_dat(&self) -> Outcome<Dat> {
		let mut inner = DaticleMap::new();
		match &self.target {
			Target::Proposal { account, repo, number } => {
				inner.insert(dat!(KEY_ACCOUNT),	Dat::Str(account.clone()));
				inner.insert(dat!(KEY_NUMBER),	Dat::U32(*number));
				inner.insert(dat!(KEY_REPO),	Dat::Str(repo.clone()));
			},
			Target::Build { id } => {
				inner.insert(dat!(KEY_ID),	Dat::Str(id.clone()));
			},
			Target::Panel { name } => {
				inner.insert(dat!(KEY_NAME),	Dat::Str(name.clone()));
			},
			Target::Guide { page, anchor } => {
				// An absent anchor is OMITTED, never encoded as `none`: SPEC.md §3 rule 4, so that
				// one reference has one encoding.
				if let Some(a) = anchor {
					inner.insert(dat!(KEY_ANCHOR),	Dat::Str(a.clone()));
				}
				inner.insert(dat!(KEY_PAGE),	Dat::Str(page.clone()));
			},
		}
		let mut target = DaticleMap::new();
		target.insert(dat!(self.target.key()), Dat::Map(inner));

		let mut map = DaticleMap::new();
		map.insert(dat!(KEY_FALLBACK),	Dat::Str(self.fallback.clone()));
		map.insert(dat!(KEY_TARGET),	Dat::Map(target));
		Ok(Dat::Map(map))
	}

	/// Reads a reference, refusing anything this schema does not admit.
	pub fn from_dat(d: &Dat) -> Outcome<Self> {
		let map = match d {
			Dat::Map(m) => m,
			other => return Err(err!(
				"A reference must be a Dat::Map, found a {:?}.", other.kind();
			Invalid, Input, Mismatch)),
		};
		if map.len() != 2 {
			return Err(err!(
				"A reference carries exactly the keys \"{}\" and \"{}\", found {} keys.",
				KEY_FALLBACK, KEY_TARGET, map.len();
			Invalid, Input));
		}
		let fallback = res!(get_str(map, KEY_FALLBACK));
		res!(check_text(&fallback, KEY_FALLBACK, limit::FALLBACK_BYTES));

		let target = match res!(get(map, KEY_TARGET)) {
			Dat::Map(m) => m,
			other => return Err(err!(
				"A reference's \"{}\" must be a Dat::Map, found a {:?}.",
				KEY_TARGET, other.kind();
			Invalid, Input, Mismatch)),
		};
		// Exactly one entry, whose key selects the kind — the same rule, and for the same reason,
		// as a link address in SPEC.md §4.3. A map with none is a reference to nothing; a map with
		// two is a reference the reader would have to choose between.
		if target.len() != 1 {
			return Err(err!(
				"A reference's \"{}\" is a map with exactly one entry, whose key names the kind. \
				Found {} entries.", KEY_TARGET, target.len();
			Invalid, Input));
		}
		let (kind_key, body) = match target.iter().next() {
			Some((k, v)) => (k, v),
			None => return Err(err!(
				"A reference's \"{}\" is empty.", KEY_TARGET;
			Invalid, Input, Missing)),
		};
		let kind = match kind_key {
			Dat::Str(s) => s.clone(),
			other => return Err(err!(
				"A reference kind must be named by a string, found a {:?}.", other.kind();
			Invalid, Input, Mismatch)),
		};
		let inner = match body {
			Dat::Map(m) => m,
			other => return Err(err!(
				"The reference kind \"{}\" must carry a Dat::Map, found a {:?}.",
				kind, other.kind();
			Invalid, Input, Mismatch)),
		};

		let target = match kind.as_str() {
			REF_PROPOSAL => {
				res!(exact_keys(inner, &[KEY_ACCOUNT, KEY_NUMBER, KEY_REPO], REF_PROPOSAL));
				let account = res!(get_str(inner, KEY_ACCOUNT));
				let repo = res!(get_str(inner, KEY_REPO));
				res!(check_text(&account, KEY_ACCOUNT, limit::FIELD_BYTES));
				res!(check_text(&repo, KEY_REPO, limit::FIELD_BYTES));
				Target::Proposal {
					account,
					repo,
					number: res!(get_u32(inner, KEY_NUMBER)),
				}
			},
			REF_BUILD => {
				res!(exact_keys(inner, &[KEY_ID], REF_BUILD));
				let id = res!(get_str(inner, KEY_ID));
				res!(check_text(&id, KEY_ID, limit::FIELD_BYTES));
				Target::Build { id }
			},
			REF_PANEL => {
				res!(exact_keys(inner, &[KEY_NAME], REF_PANEL));
				let name = res!(get_str(inner, KEY_NAME));
				res!(check_text(&name, KEY_NAME, limit::FIELD_BYTES));
				Target::Panel { name }
			},
			REF_GUIDE => {
				// The anchor is optional, so the key set is checked against both admissible shapes
				// rather than one.
				let allowed: &[&str] = if inner.contains_key(&dat!(KEY_ANCHOR)) {
					&[KEY_ANCHOR, KEY_PAGE]
				} else {
					&[KEY_PAGE]
				};
				res!(exact_keys(inner, allowed, REF_GUIDE));
				let page = res!(get_str(inner, KEY_PAGE));
				res!(check_text(&page, KEY_PAGE, limit::FIELD_BYTES));
				let anchor = match inner.get(&dat!(KEY_ANCHOR)) {
					Some(Dat::Str(a)) => {
						res!(check_text(a, KEY_ANCHOR, limit::FIELD_BYTES));
						Some(a.clone())
					},
					Some(other) => return Err(err!(
						"A guide reference's \"{}\" must be a string, found a {:?}.",
						KEY_ANCHOR, other.kind();
					Invalid, Input, Mismatch)),
					None => None,
				};
				Target::Guide { page, anchor }
			},
			other => return Err(err!(
				"\"{}\" is not a reference kind this schema admits. The four are \"{}\", \"{}\", \
				\"{}\" and \"{}\". A private, device-local pointer is deliberately not among them: \
				the other party cannot resolve one, and a chip that will always fail must not be \
				drawn.", other, REF_PROPOSAL, REF_BUILD, REF_PANEL, REF_GUIDE;
			Invalid, Input, Unknown)),
		};
		Ok(Self { target, fallback })
	}
}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ THE POST                                                                  │
// └───────────────────────────────────────────────────────────────────────────┘

/// A `daimond/post/0` payload.
///
/// Every field here is inside the tree region, so every field is covered by the envelope's `hash`
/// and therefore by its signature. A relay handling this artefact can add nothing to it, remove
/// nothing from it, and rewrite nothing in it without the signature ceasing to verify.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Post {
	/// The message text. Prose, not markup.
	pub body:	String,
	/// The recipient's public key.
	pub to:	Vec<u8>,
	/// Per-message randomness.
	///
	/// Signed, so that a replay cannot mint a fresh one, and present so that two identical bodies
	/// sent to one recipient are two distinct addresses rather than one message that appears to
	/// have been sent once.
	pub nonce:	Vec<u8>,
	/// The address this message answers, if it answers one.
	pub reply_to:	Option<Vec<u8>>,
	/// References, at most [`limit::REFS`].
	pub refs:	Vec<Reference>,
}

impl Post {
	/// Encodes this post as a canonical daticle.
	pub fn to_dat(&self) -> Outcome<Dat> {
		let mut map = DaticleMap::new();
		map.insert(dat!(KEY_BODY),	Dat::BU32(self.body.as_bytes().to_vec()));
		map.insert(dat!(KEY_NONCE),	Dat::BU8(self.nonce.clone()));
		// An empty list and an absent one would be two encodings of one message, and so two
		// addresses: SPEC.md §3 rules 4 and 8. Omitted when empty, and `from_dat` refuses a list
		// that is present and empty.
		if !self.refs.is_empty() {
			let mut list = Vec::with_capacity(self.refs.len());
			for r in &self.refs {
				list.push(res!(r.to_dat()));
			}
			map.insert(dat!(KEY_REFS), Dat::List(list));
		}
		if let Some(a) = &self.reply_to {
			map.insert(dat!(KEY_REPLY_TO), Dat::BU8(a.clone()));
		}
		map.insert(dat!(KEY_TO),	Dat::BU8(self.to.clone()));
		Ok(Dat::Map(map))
	}

	/// Reads a post from a daticle, enforcing every rule this schema declares.
	pub fn from_dat(d: &Dat) -> Outcome<Self> {
		let map = match d {
			Dat::Map(m) => m,
			Dat::OrdMap(_) => return Err(err!(
				"SPEC.md §3 rule 2: a post payload is a Dat::Map, never a Dat::OrdMap. An OrdMap \
				follows the author's typing rather than the keys, so the same message would have \
				as many addresses as there are orders to write it in.";
			Invalid, Input, Mismatch)),
			other => return Err(err!(
				"A post payload must be a Dat::Map, found a {:?}.", other.kind();
			Invalid, Input, Mismatch)),
		};
		// The key set is checked whole, both ways: a missing key is a message that does not say
		// what it must, and an unknown key is a field somebody signed that no reader will draw.
		let allowed: Vec<&str> = {
			let mut v = vec![KEY_BODY, KEY_NONCE, KEY_TO];
			if map.contains_key(&dat!(KEY_REFS)) { v.push(KEY_REFS); }
			if map.contains_key(&dat!(KEY_REPLY_TO)) { v.push(KEY_REPLY_TO); }
			v
		};
		res!(exact_keys(map, &allowed, "post"));

		let body_bytes = match res!(get(map, KEY_BODY)) {
			Dat::BU32(b) => b.clone(),
			Dat::BU8(_) | Dat::BU16(_) | Dat::BU64(_) => return Err(err!(
				"The post key \"{}\" must be a BU32. A narrower byte string truncates a long \
				message silently, and a wider one is a second encoding of the same value.",
				KEY_BODY;
			Invalid, Input, Mismatch)),
			other => return Err(err!(
				"The post key \"{}\" must be a BU32, found a {:?}.", KEY_BODY, other.kind();
			Invalid, Input, Mismatch)),
		};
		if body_bytes.len() > limit::BODY_BYTES {
			return Err(err!(
				"The message body is {} bytes, exceeding the limit of {}. It is refused rather \
				than truncated: half a message is not a shorter message.",
				body_bytes.len(), limit::BODY_BYTES;
			Invalid, Input, LimitReached));
		}
		let body = match String::from_utf8(body_bytes) {
			Ok(s) => s,
			Err(e) => return Err(err!(
				"The message body is not valid UTF-8: {}.", e;
			Invalid, Input, Decode)),
		};
		// The body is carried as bytes, so `canon`'s string rules do not reach it and this schema
		// applies them itself. Without that a message could hold two encodings of one text and so
		// two addresses, which is exactly what SPEC.md §3 rule 5 exists to prevent.
		res!(canon::check_string(&body));

		let to = res!(get_bytes(map, KEY_TO, limit::KEY_BYTES));
		let nonce = res!(get_bytes(map, KEY_NONCE, limit::NONCE_BYTES));

		let reply_to = match map.get(&dat!(KEY_REPLY_TO)) {
			Some(_) => Some(res!(get_bytes(map, KEY_REPLY_TO, limit::ADDR_BYTES))),
			None => None,
		};

		let refs = match map.get(&dat!(KEY_REFS)) {
			Some(Dat::List(items)) => {
				if items.is_empty() {
					return Err(err!(
						"SPEC.md §3 rule 8: the post carries an empty \"{}\" list. A thing a \
						reader would draw identically whether present or absent gives one message \
						two encodings, and so two addresses. Omit the key.", KEY_REFS;
					Invalid, Input));
				}
				if items.len() > limit::REFS {
					return Err(err!(
						"The post carries {} references, exceeding the limit of {}. Each is \
						resolved lazily against the READER's own metered allowance, so a message \
						that could carry many would spend a stranger's quota by being opened.",
						items.len(), limit::REFS;
					Invalid, Input, LimitReached));
				}
				let mut out = Vec::with_capacity(items.len());
				for (i, item) in items.iter().enumerate() {
					out.push(res!(Reference::from_dat(item).map_err(|e| err!(e,
						"Reference {} of {} is not one this schema admits.", i, items.len();
					Invalid, Input))));
				}
				out
			},
			Some(Dat::Vek(_)) => return Err(err!(
				"SPEC.md §3 rule 7: \"{}\" is a Dat::List, never a Dat::Vek, even where every \
				element shares a kind.", KEY_REFS;
			Invalid, Input, Mismatch)),
			Some(other) => return Err(err!(
				"The post key \"{}\" must be a list, found a {:?}.", KEY_REFS, other.kind();
			Invalid, Input, Mismatch)),
			None => Vec::new(),
		};

		Ok(Self { body, to, nonce, reply_to, refs })
	}

	/// Encodes this post to the canonical bytes that become the tree region.
	pub fn encode(&self) -> Outcome<Vec<u8>> {
		let d = res!(self.to_dat());
		// Read straight back, so that a post which cannot be decoded can never be signed. Signing
		// bytes no reader will accept produces an artefact that is valid to its author and refused
		// by everybody else, which is the worst of the failures available here.
		res!(Self::from_dat(&d));
		let bytes = res!(d.to_bytes(Vec::new()));
		if bytes.len() > sbj_limit::TREE_BYTES {
			return Err(err!(
				"The encoded post is {} bytes, exceeding the tree region limit of {}.",
				bytes.len(), sbj_limit::TREE_BYTES;
			Invalid, Input, LimitReached));
		}
		Ok(bytes)
	}

	/// Decodes a post from the bytes of a tree region, which must be consumed exactly.
	///
	/// The bytes are re-encoded and compared with what came in, which is what enforces the
	/// byte-level rules a decoded value can no longer show: a duplicate key collapses into one
	/// entry when BDAT builds its map, and a length written in more bytes than it needs decodes to
	/// the same number. Both survive only in the bytes.
	pub fn decode(buf: &[u8]) -> Outcome<Self> {
		let lims = DecodeLimits::new(limit::DEPTH, sbj_limit::TREE_BYTES);
		let (d, n) = res!(Dat::from_bytes_limited(buf, &lims));
		if n != buf.len() {
			return Err(err!(
				"The post payload occupies {} of the {} bytes supplied, leaving {} trailing.",
				n, buf.len(), buf.len() - n;
			Invalid, Input, Decode));
		}
		let re = res!(d.to_bytes(Vec::new()));
		if re != buf {
			return Err(err!(
				"The post payload is not in canonical form: it re-encodes to {} bytes against the \
				{} supplied, so it carries a duplicate key, a non-minimal length, or a \
				non-canonical map. See SPEC.md §3.", re.len(), buf.len();
			Invalid, Input, Decode));
		}
		Self::from_dat(&d)
	}
}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ FIELD READERS                                                             │
// └───────────────────────────────────────────────────────────────────────────┘

/// Returns a required key's value, or an error naming the key that is missing.
fn get<'a>(map: &'a DaticleMap, key: &str) -> Outcome<&'a Dat> {
	match map.get(&dat!(key)) {
		Some(d) => Ok(d),
		None => Err(err!(
			"The post is missing the required key \"{}\".", key;
		Invalid, Input, Missing)),
	}
}

/// Reads a required string key.
fn get_str(map: &DaticleMap, key: &str) -> Outcome<String> {
	match res!(get(map, key)) {
		Dat::Str(s) => Ok(s.clone()),
		other => Err(err!(
			"The key \"{}\" must carry a string, found a {:?}.", key, other.kind();
		Invalid, Input, Mismatch)),
	}
}

/// Reads a required `u32` key, refusing any other width.
fn get_u32(map: &DaticleMap, key: &str) -> Outcome<u32> {
	match res!(get(map, key)) {
		Dat::U32(n) => Ok(*n),
		other => Err(err!(
			"SPEC.md §3 rule 6: the key \"{}\" is declared a u32 and must be encoded as exactly \
			that width, found a {:?}. A promoted or demoted integer gives one message two \
			encodings.", key, other.kind();
		Invalid, Input, Mismatch)),
	}
}

/// Reads a required `BU8` key of an exact width.
///
/// The width is exact rather than bounded because every one of these is a key, a nonce or an
/// address, and each has one size. A short one is not a smaller key; it is a different thing.
fn get_bytes(map: &DaticleMap, key: &str, width: usize) -> Outcome<Vec<u8>> {
	let b = match res!(get(map, key)) {
		Dat::BU8(b) => b.clone(),
		other => return Err(err!(
			"The key \"{}\" must carry a BU8, found a {:?}.", key, other.kind();
		Invalid, Input, Mismatch)),
	};
	if b.len() != width {
		return Err(err!(
			"The key \"{}\" carries {} bytes and must carry exactly {}.", key, b.len(), width;
		Invalid, Input, Mismatch));
	}
	Ok(b)
}

/// Checks a string field against the canonical text rules and a byte ceiling.
fn check_text(s: &str, key: &str, max: usize) -> Outcome<()> {
	if s.len() > max {
		return Err(err!(
			"The field \"{}\" is {} bytes, exceeding the limit of {}.", key, s.len(), max;
		Invalid, Input, LimitReached));
	}
	res!(canon::check_string(s));
	Ok(())
}

/// Requires a map to carry exactly the named keys — no more, and no fewer.
///
/// Both directions, because they catch different faults. A missing key is a message that does not
/// say something it must. An unknown key is a field the sender signed and no reader will ever
/// draw, which is worse than useless: it is covered by the signature, so it looks like meaning.
fn exact_keys(map: &DaticleMap, allowed: &[&str], what: &str) -> Outcome<()> {
	for k in allowed {
		if !map.contains_key(&dat!(*k)) {
			return Err(err!(
				"The {} is missing the required key \"{}\".", what, k;
			Invalid, Input, Missing));
		}
	}
	for k in map.keys() {
		let name = match k {
			Dat::Str(s) => s.clone(),
			other => return Err(err!(
				"SPEC.md §3 rule 3: a map key must be a string, found a {:?}.", other.kind();
			Invalid, Input, Mismatch)),
		};
		res!(canon::check_key_string(&name));
		if !allowed.iter().any(|a| *a == name.as_str()) {
			return Err(err!(
				"The {} carries the key \"{}\", which this schema does not admit. The admitted \
				keys are: {}.", what, name, allowed.join(", ");
			Invalid, Input, Unknown));
		}
	}
	Ok(())
}


#[cfg(test)]
mod tests {
	use super::*;

	/// A plausible post, with fixed contents.
	fn sample() -> Post {
		Post {
			body:	fmt!("The crop is in, and the second field can wait."),
			to:	vec![0xA1; limit::KEY_BYTES],
			nonce:	vec![0xB2; limit::NONCE_BYTES],
			reply_to:	None,
			refs:	Vec::new(),
		}
	}

	/// Every reference kind, once.
	fn every_ref() -> Vec<Reference> {
		vec![
			Reference {
				target: Target::Proposal {
					account:	fmt!("oxedyne"),
					repo:	fmt!("daimond"),
					number:	17,
				},
				fallback: fmt!("the proposal about the panel showing nothing when signed out"),
			},
			Reference {
				target: Target::Build { id: fmt!("f9f68b75c73b") },
				fallback: fmt!("the build this was fixed in"),
			},
			Reference {
				target: Target::Panel { name: fmt!("spend") },
				fallback: fmt!("the Spending panel"),
			},
			Reference {
				target: Target::Guide {
					page:	fmt!("improve"),
					anchor:	Some(fmt!("voices")),
				},
				fallback: fmt!("the guide section on voices"),
			},
		]
	}

	#[test]
	fn test_round_trip_minimal() -> Outcome<()> {
		let p = sample();
		let bytes = res!(p.encode());
		let back = res!(Post::decode(&bytes));
		assert_eq!(p, back);
		Ok(())
	}

	#[test]
	fn test_round_trip_every_field() -> Outcome<()> {
		let mut p = sample();
		p.reply_to = Some(vec![0xC3; limit::ADDR_BYTES]);
		p.refs = every_ref();
		let bytes = res!(p.encode());
		let back = res!(Post::decode(&bytes));
		assert_eq!(p, back);
		assert_eq!(back.refs.len(), 4);
		Ok(())
	}

	/// A guide reference without an anchor must not encode the absence as `none`.
	#[test]
	fn test_guide_anchor_is_omitted_not_none() -> Outcome<()> {
		let mut p = sample();
		p.refs = vec![Reference {
			target: Target::Guide { page: fmt!("improve"), anchor: None },
			fallback: fmt!("the guide"),
		}];
		let bytes = res!(p.encode());
		let back = res!(Post::decode(&bytes));
		assert_eq!(p, back);
		// The two shapes must be different bytes, or the anchor is not carrying meaning.
		let mut q = p.clone();
		q.refs = vec![Reference {
			target: Target::Guide { page: fmt!("improve"), anchor: Some(fmt!("voices")) },
			fallback: fmt!("the guide"),
		}];
		assert_ne!(res!(q.encode()), bytes);
		Ok(())
	}

	#[test]
	fn test_trailing_bytes_refused() -> Outcome<()> {
		let mut bytes = res!(sample().encode());
		bytes.push(0x00);
		match Post::decode(&bytes) {
			Ok(_) => Err(err!("A payload with a trailing byte was accepted."; Test, Invalid)),
			Err(_) => Ok(()),
		}
	}

	/// The body is a BU32 because a BU8 truncates past 255 bytes.
	#[test]
	fn test_body_must_be_bu32() -> Outcome<()> {
		let mut map = DaticleMap::new();
		map.insert(dat!(KEY_BODY),	Dat::BU8(b"short enough to fit".to_vec()));
		map.insert(dat!(KEY_NONCE),	Dat::BU8(vec![0xB2; limit::NONCE_BYTES]));
		map.insert(dat!(KEY_TO),	Dat::BU8(vec![0xA1; limit::KEY_BYTES]));
		match Post::from_dat(&Dat::Map(map)) {
			Ok(_) => Err(err!("A body encoded as a BU8 was accepted."; Test, Invalid)),
			Err(_) => Ok(()),
		}
	}

	/// A body longer than the ceiling is refused, not truncated.
	#[test]
	fn test_body_over_the_limit_refused() -> Outcome<()> {
		let mut p = sample();
		p.body = "a".repeat(limit::BODY_BYTES + 1);
		match p.encode() {
			Ok(_) => Err(err!("An oversized body was accepted."; Test, Invalid)),
			Err(_) => Ok(()),
		}
	}

	/// A body at exactly the ceiling is accepted, so the limit is a boundary and not a scare.
	#[test]
	fn test_body_at_the_limit_accepted() -> Outcome<()> {
		let mut p = sample();
		p.body = "a".repeat(limit::BODY_BYTES);
		let bytes = res!(p.encode());
		assert_eq!(res!(Post::decode(&bytes)).body.len(), limit::BODY_BYTES);
		Ok(())
	}

	/// Text that displays identically must encode identically, or one message has two addresses.
	#[test]
	fn test_body_not_nfc_refused() -> Outcome<()> {
		let mut p = sample();
		p.body = fmt!("cafe\u{0301}");		// e + combining acute, not the composed form
		match p.encode() {
			Ok(_) => Err(err!("A body that is not in NFC was accepted."; Test, Invalid)),
			Err(_) => Ok(()),
		}
	}

	#[test]
	fn test_body_control_character_refused() -> Outcome<()> {
		let mut p = sample();
		p.body = fmt!("before\u{0}after");
		match p.encode() {
			Ok(_) => Err(err!("A body carrying a NUL was accepted."; Test, Invalid)),
			Err(_) => Ok(()),
		}
	}

	/// A carriage return is refused so that one line ending has one encoding.
	#[test]
	fn test_body_carriage_return_refused() -> Outcome<()> {
		let mut p = sample();
		p.body = fmt!("one\r\ntwo");
		match p.encode() {
			Ok(_) => Err(err!("A body carrying a carriage return was accepted."; Test, Invalid)),
			Err(_) => Ok(()),
		}
	}

	/// A tab and a newline are ordinary text and must survive.
	#[test]
	fn test_body_tab_and_newline_accepted() -> Outcome<()> {
		let mut p = sample();
		p.body = fmt!("one\ttwo\nthree");
		let bytes = res!(p.encode());
		assert_eq!(res!(Post::decode(&bytes)).body, p.body);
		Ok(())
	}

	#[test]
	fn test_nonce_must_be_exact_width() -> Outcome<()> {
		let mut p = sample();
		p.nonce = vec![0xB2; limit::NONCE_BYTES - 1];
		match p.encode() {
			Ok(_) => Err(err!("A short nonce was accepted."; Test, Invalid)),
			Err(_) => Ok(()),
		}
	}

	#[test]
	fn test_recipient_key_must_be_exact_width() -> Outcome<()> {
		let mut p = sample();
		p.to = vec![0xA1; limit::KEY_BYTES + 1];
		match p.encode() {
			Ok(_) => Err(err!("An overlong recipient key was accepted."; Test, Invalid)),
			Err(_) => Ok(()),
		}
	}

	/// An empty list and an absent one would be two encodings of one message.
	#[test]
	fn test_empty_refs_list_refused() -> Outcome<()> {
		let mut map = DaticleMap::new();
		map.insert(dat!(KEY_BODY),	Dat::BU32(b"hello".to_vec()));
		map.insert(dat!(KEY_NONCE),	Dat::BU8(vec![0xB2; limit::NONCE_BYTES]));
		map.insert(dat!(KEY_REFS),	Dat::List(Vec::new()));
		map.insert(dat!(KEY_TO),	Dat::BU8(vec![0xA1; limit::KEY_BYTES]));
		match Post::from_dat(&Dat::Map(map)) {
			Ok(_) => Err(err!("An empty refs list was accepted."; Test, Invalid)),
			Err(_) => Ok(()),
		}
	}

	/// Omitting refs entirely is the correct encoding, and it must work.
	#[test]
	fn test_absent_refs_accepted() -> Outcome<()> {
		let bytes = res!(sample().encode());
		assert!(res!(Post::decode(&bytes)).refs.is_empty());
		Ok(())
	}

	#[test]
	fn test_five_references_refused() -> Outcome<()> {
		let mut p = sample();
		p.refs = every_ref();
		p.refs.push(Reference {
			target: Target::Panel { name: fmt!("work") },
			fallback: fmt!("one too many"),
		});
		match p.encode() {
			Ok(_) => Err(err!("Five references were accepted."; Test, Invalid)),
			Err(_) => Ok(()),
		}
	}

	#[test]
	fn test_four_references_accepted() -> Outcome<()> {
		let mut p = sample();
		p.refs = every_ref();
		assert_eq!(p.refs.len(), limit::REFS);
		let bytes = res!(p.encode());
		assert_eq!(res!(Post::decode(&bytes)).refs.len(), limit::REFS);
		Ok(())
	}

	/// A private, device-local pointer is refused by name rather than drawn as a dead chip.
	#[test]
	fn test_unknown_reference_kind_refused() -> Outcome<()> {
		let mut inner = DaticleMap::new();
		inner.insert(dat!("id"), Dat::Str(fmt!("chat-14")));
		let mut target = DaticleMap::new();
		target.insert(dat!("chat"), Dat::Map(inner));
		let mut r = DaticleMap::new();
		r.insert(dat!(KEY_FALLBACK),	Dat::Str(fmt!("that chat")));
		r.insert(dat!(KEY_TARGET),	Dat::Map(target));
		match Reference::from_dat(&Dat::Map(r)) {
			Ok(_) => Err(err!("A chat reference was accepted."; Test, Invalid)),
			Err(_) => Ok(()),
		}
	}

	/// A target naming two kinds is a reference the reader would have to choose between.
	#[test]
	fn test_target_with_two_entries_refused() -> Outcome<()> {
		let mut one = DaticleMap::new();
		one.insert(dat!(KEY_NAME), Dat::Str(fmt!("spend")));
		let mut two = DaticleMap::new();
		two.insert(dat!(KEY_ID), Dat::Str(fmt!("f9f68b75c73b")));
		let mut target = DaticleMap::new();
		target.insert(dat!(REF_PANEL),	Dat::Map(one));
		target.insert(dat!(REF_BUILD),	Dat::Map(two));
		let mut r = DaticleMap::new();
		r.insert(dat!(KEY_FALLBACK),	Dat::Str(fmt!("either of those")));
		r.insert(dat!(KEY_TARGET),	Dat::Map(target));
		match Reference::from_dat(&Dat::Map(r)) {
			Ok(_) => Err(err!("A target naming two kinds was accepted."; Test, Invalid)),
			Err(_) => Ok(()),
		}
	}

	/// A sender-supplied title has nowhere to go: an unadmitted key is refused.
	#[test]
	fn test_sender_supplied_title_refused() -> Outcome<()> {
		let mut inner = DaticleMap::new();
		inner.insert(dat!(KEY_ID),	Dat::Str(fmt!("f9f68b75c73b")));
		inner.insert(dat!("title"),	Dat::Str(fmt!("Fixed everything, click here")));
		let mut target = DaticleMap::new();
		target.insert(dat!(REF_BUILD), Dat::Map(inner));
		let mut r = DaticleMap::new();
		r.insert(dat!(KEY_FALLBACK),	Dat::Str(fmt!("a build")));
		r.insert(dat!(KEY_TARGET),	Dat::Map(target));
		match Reference::from_dat(&Dat::Map(r)) {
			Ok(_) => Err(err!("A sender-supplied title was accepted."; Test, Invalid)),
			Err(_) => Ok(()),
		}
	}

	#[test]
	fn test_unknown_post_key_refused() -> Outcome<()> {
		let mut map = DaticleMap::new();
		map.insert(dat!(KEY_BODY),	Dat::BU32(b"hello".to_vec()));
		map.insert(dat!(KEY_NONCE),	Dat::BU8(vec![0xB2; limit::NONCE_BYTES]));
		map.insert(dat!(KEY_TO),	Dat::BU8(vec![0xA1; limit::KEY_BYTES]));
		map.insert(dat!("from"),	Dat::Str(fmt!("somebody else")));
		match Post::from_dat(&Dat::Map(map)) {
			Ok(_) => Err(err!("A post carrying a `from` field was accepted."; Test, Invalid)),
			Err(_) => Ok(()),
		}
	}

	#[test]
	fn test_missing_key_refused() -> Outcome<()> {
		let mut map = DaticleMap::new();
		map.insert(dat!(KEY_BODY),	Dat::BU32(b"hello".to_vec()));
		map.insert(dat!(KEY_TO),	Dat::BU8(vec![0xA1; limit::KEY_BYTES]));
		match Post::from_dat(&Dat::Map(map)) {
			Ok(_) => Err(err!("A post with no nonce was accepted."; Test, Invalid)),
			Err(_) => Ok(()),
		}
	}

	#[test]
	fn test_ordmap_refused() -> Outcome<()> {
		let ord = oxedyne_fe2o3_jdat::map::create_dat_ordmap(vec![
			(dat!(KEY_BODY),	Dat::BU32(b"hello".to_vec())),
			(dat!(KEY_NONCE),	Dat::BU8(vec![0xB2; limit::NONCE_BYTES])),
			(dat!(KEY_TO),	Dat::BU8(vec![0xA1; limit::KEY_BYTES])),
		]);
		match Post::from_dat(&ord) {
			Ok(_) => Err(err!("An OrdMap payload was accepted."; Test, Invalid)),
			Err(_) => Ok(()),
		}
	}

	/// The proposal number is declared a u32 and must be encoded as exactly that width.
	#[test]
	fn test_proposal_number_width_is_fixed() -> Outcome<()> {
		let mut inner = DaticleMap::new();
		inner.insert(dat!(KEY_ACCOUNT),	Dat::Str(fmt!("oxedyne")));
		inner.insert(dat!(KEY_NUMBER),	Dat::U16(17));		// declared u32
		inner.insert(dat!(KEY_REPO),	Dat::Str(fmt!("daimond")));
		let mut target = DaticleMap::new();
		target.insert(dat!(REF_PROPOSAL), Dat::Map(inner));
		let mut r = DaticleMap::new();
		r.insert(dat!(KEY_FALLBACK),	Dat::Str(fmt!("a proposal")));
		r.insert(dat!(KEY_TARGET),	Dat::Map(target));
		match Reference::from_dat(&Dat::Map(r)) {
			Ok(_) => Err(err!("A demoted integer width was accepted."; Test, Invalid)),
			Err(_) => Ok(()),
		}
	}

	/// Two identical bodies to one recipient are two addresses, because the nonce is signed.
	#[test]
	fn test_the_nonce_separates_identical_messages() -> Outcome<()> {
		let a = sample();
		let mut b = sample();
		b.nonce = vec![0xB3; limit::NONCE_BYTES];
		assert_eq!(a.body, b.body);
		assert_ne!(res!(a.encode()), res!(b.encode()));
		Ok(())
	}
}
