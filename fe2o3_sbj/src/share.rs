//! `daimond/share/0` — one person sending another a copy of something they own.
//!
//! A share **carries** what it sends. The files travel inside the payload, sealed to the
//! recipient, and what lands is theirs: their copy, in their workspace, under their own key. They
//! may change it, and the sender never sees the change; the sender may change theirs, and the
//! receiver never sees that either. There is no shared content key that outlives an edit, nothing
//! to revoke, and nobody's storage but the receiver's own. That is the whole design, and every
//! field below follows from it.
//!
//! It is a schema rather than a fifth [`crate::post::Target`] for the reason written beside
//! [`crate::SCHEMA_SHARE`], and the argument that settles it is the last one: a share must carry a
//! consent bit the signature covers, and a `Reference` carries exactly two keys and refuses a
//! third.
//!
//! # The consent bit
//!
//! Data travels freely. Code does not. A shared Diamond that carries a page is carrying **a
//! program written by another person**, and the receiver decides whether to run it — so the
//! artefact says, in the part the author signed, whether there is anything to decide. A flag a
//! relay could add or strip is not a consent flag, which is why [`KEY_CODE`] is inside the payload
//! and not in a wrapper around it.
//!
//! [`KEY_CODE`] is **required and always written**, never omitted when false. An omitted false
//! would be indistinguishable from a sender whose build had never heard of the field, and the one
//! thing a receiver must be able to tell apart is "they said there is no code" from "they did not
//! say".
//!
//! And the claim is **checked against the files**, both ways (see [`code_file`]). A payload
//! carrying a page under `code: false` is refused, so the bit cannot hide a program; a payload
//! claiming code and carrying none is refused too, so a sender cannot cry wolf and teach people to
//! wave the question away. The bit is not therefore redundant with the files, which is the obvious
//! objection to it: it is the SENDER's reading of [`CODE_SUFFIXES`], pinned at signing time, so a
//! later build that learns of a suffix this one does not know will disagree with an old artefact
//! rather than quietly decide for the receiver.
//!
//! # What a share may not carry
//!
//! Three paths are refused outright, and each is refused here rather than left to a client, so
//! that every implementation refuses the same things:
//!
//! - `.daimond/` — the meta, the append-only log, the link sidecar. The log is a record of what
//!   agents did in the SENDER's Diamond, and nobody sending a recipe means to send that.
//! - `versions/` — the sender's own history, which is theirs and which would multiply the size of
//!   the share by the length of it.
//! - `capp.json` — the delivery record, which says which bytes were delivered and at what template
//!   version. It is a record of a delivery that never happened to the receiver, and one doctored
//!   by the sender would pin the receiver's copy against every future template fix on THEIR
//!   machine, which they never chose. A copy that arrives without one is a case the receiving
//!   client already knows how to handle: it asks.
//!
//! The canonical rules of `SPEC.md` §3 apply unchanged. Two of them do real work here that they do
//! not do for a message: [`KEY_FILES`] is ordered by path and refuses a duplicate, since a set of
//! files written in two orders would be two addresses for one Diamond; and a path is refused
//! rather than normalised, since normalising is exactly how one file comes to have two spellings.

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

/// Whether the files include executable page code. See the module note.
pub const KEY_CODE:	&'static str = "code";
/// The files, ordered by path.
pub const KEY_FILES:	&'static str = "files";
/// The shared thing's display name.
pub const KEY_NAME:	&'static str = "name";
/// Per-share randomness, so that two identical shares are two addresses.
pub const KEY_NONCE:	&'static str = "nonce";
/// The sender's covering sentence, if they wrote one.
pub const KEY_NOTE:	&'static str = "note";
/// The recipient's public key.
pub const KEY_TO:	&'static str = "to";

/// One file's contents.
pub const KEY_BODY:	&'static str = "body";
/// One file's path, relative to the Diamond's own folder.
pub const KEY_PATH:	&'static str = "path";


/// The path prefixes and names a share may not carry, and why.
///
/// Checked as a prefix in the first two cases and as the whole path in the third, which is what
/// `capp.json` needs: a file called `capp.json` inside a folder of the receiver's own making is
/// ordinary data, and the delivery record is the one at the root.
pub const REFUSED_PREFIXES:	&[&'static str] = &[".daimond/", "versions/"];

/// The exact path a share may not carry. See [`REFUSED_PREFIXES`] for the prefixes.
pub const REFUSED_EXACT:	&'static str = "capp.json";

/// The suffixes that make a file code rather than data.
///
/// A closed set, matched case-insensitively on ASCII. It is closed for the same reason the icon
/// names of `SPEC.md` §4.2 are: a reader knows exactly which files it will hand to an engine, and
/// a set that grew by guessing would be a set that admitted the first thing nobody thought of.
///
/// Case-insensitively because a suffix check that is not is one `.HTML` away from being no check
/// at all, and the receiving side stores a file under the name it was sent under.
pub const CODE_SUFFIXES:	&[&'static str] = &[".htm", ".html", ".js", ".mjs", ".svg", ".wasm"];


/// Limits this schema enforces. Every one is a rejection, never a truncation.
pub mod limit {
	/// The most files one share may carry.
	///
	/// Sixty-four. A capp is a page, a memory, an index and a handful of seeded tables — under ten
	/// — and each file in a share is examined and written on arrival, so the number bounds what
	/// opening one costs. The figure is revisable on evidence, as `SPEC.md` §5's are; that there is
	/// one is not.
	pub const FILES:	usize = 64;
	/// The most all the file bodies together may carry, in bytes.
	///
	/// Two mebibytes, and the reason is the RECEIVER's, not the format's. A Daimond sync parcel
	/// carries at most six mebibytes across every Diamond an account holds, and a Diamond that does
	/// not fit is left out of the parcel entirely rather than trimmed. A share larger than a third
	/// of that budget is a share that would stop travelling between the receiver's own devices the
	/// day it arrived, which is a worse failure than being refused now.
	pub const TOTAL_BYTES:	usize = 2 * 1024 * 1024;
	/// The most one file's path may carry, in bytes of UTF-8.
	pub const PATH_BYTES:	usize = 256;
	/// The most the display name may carry, in bytes of UTF-8.
	pub const NAME_BYTES:	usize = 128;
	/// The most the covering note may carry, in bytes of UTF-8.
	///
	/// A sentence, not a letter. A letter is a `daimond/post/0` message, which carries eight
	/// kibibytes and is the thing built for prose; this is the line that says what the gift is.
	pub const NOTE_BYTES:	usize = 512;
	/// The exact width of the per-share nonce.
	pub const NONCE_BYTES:	usize = 16;
	/// The exact width of a public key.
	pub const KEY_BYTES:	usize = 32;
	/// Decoding depth for a payload of this schema.
	///
	/// A share is a flat record holding one list of flat maps, so four levels is its whole shape
	/// and eight is already past anything it can reach. Far below `SPEC.md` §5's tree limit because
	/// nothing here recurses, and a limit set to what the shape needs refuses a nested value before
	/// it is looked at.
	pub const DEPTH:	usize = 8;
}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ CODE                                                                      │
// └───────────────────────────────────────────────────────────────────────────┘

/// Whether a path names a file this version considers code.
///
/// The suffix and nothing else. What a file contains is not consulted, deliberately: a rule about
/// contents would be a rule a reader had to run over every byte of every share before it could say
/// whether there was a question to ask, and it would answer differently for the same file on two
/// builds. A suffix is a fact about the name, and the name is what the receiving side stores.
pub fn is_code_path(path: &str) -> bool {
	let lower = path.to_ascii_lowercase();
	CODE_SUFFIXES.iter().any(|s| lower.ends_with(s))
}

/// The first file in a list that is code, or `None` when none of them is.
///
/// The FIRST rather than a count, because the error names it: "this share says it carries no code
/// and carries `crystal.html`" is a sentence a person can act on, and "1 code file" is not.
pub fn code_file(files: &[File]) -> Option<&File> {
	files.iter().find(|f| is_code_path(&f.path))
}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ ONE FILE                                                                  │
// └───────────────────────────────────────────────────────────────────────────┘

/// One file of a share: where it goes, and what is in it.
///
/// The body is bytes and is held to no text rule, because a Diamond holds pictures as well as
/// prose and a canonical encoding of bytes is the bytes. The PATH is a string and is held to every
/// rule `SPEC.md` §3 has for one, since two spellings of one path would be two addresses for one
/// Diamond.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct File {
	/// Where the file goes, relative to the receiver's copy of the Diamond.
	pub path:	String,
	/// What is in it.
	pub body:	Vec<u8>,
}

impl File {
	/// Encodes this file as a canonical daticle.
	pub fn to_dat(&self) -> Outcome<Dat> {
		let mut map = DaticleMap::new();
		map.insert(dat!(KEY_BODY),	Dat::BU32(self.body.clone()));
		map.insert(dat!(KEY_PATH),	Dat::Str(self.path.clone()));
		Ok(Dat::Map(map))
	}

	/// Reads a file, refusing anything this schema does not admit.
	pub fn from_dat(d: &Dat) -> Outcome<Self> {
		let map = match d {
			Dat::Map(m) => m,
			other => return Err(err!(
				"A shared file must be a Dat::Map, found a {:?}.", other.kind();
			Invalid, Input, Mismatch)),
		};
		res!(exact_keys(map, &[KEY_BODY, KEY_PATH], "shared file"));

		let path = match res!(get(map, KEY_PATH)) {
			Dat::Str(s) => s.clone(),
			other => return Err(err!(
				"A shared file's \"{}\" must be a string, found a {:?}.", KEY_PATH, other.kind();
			Invalid, Input, Mismatch)),
		};
		res!(check_path(&path));

		let body = match res!(get(map, KEY_BODY)) {
			Dat::BU32(b) => b.clone(),
			Dat::BU8(_) | Dat::BU16(_) | Dat::BU64(_) => return Err(err!(
				"The shared file \"{}\" carries its contents in a byte string that is not a BU32. A \
				narrower one truncates silently past its width, and a wider one is a second \
				encoding of the same value.", path;
			Invalid, Input, Mismatch)),
			other => return Err(err!(
				"The shared file \"{}\" must carry its contents in a BU32, found a {:?}.",
				path, other.kind();
			Invalid, Input, Mismatch)),
		};
		Ok(Self { path, body })
	}
}

/// Checks a path against every rule this schema has for one.
///
/// **Refused rather than normalised**, which is where this parts company with the client-side
/// `safePath` it otherwise matches. That function is handed an untrusted request and drops an
/// empty or `.` segment on the way to a real file; this is deciding what a signed artefact means,
/// and there a path that needed tidying is a path with two spellings and so a Diamond with two
/// addresses. Every check below is a rejection.
pub fn check_path(path: &str) -> Outcome<()> {
	if path.is_empty() {
		return Err(err!(
			"A shared file carries an empty path."; Invalid, Input, Missing));
	}
	if path.len() > limit::PATH_BYTES {
		return Err(err!(
			"The shared path \"{}\" is {} bytes, exceeding the limit of {}.",
			path, path.len(), limit::PATH_BYTES;
		Invalid, Input, LimitReached));
	}
	// The §3 rule 5 string rules: UTF-8 already, and now NFC, no control characters, no carriage
	// return. A path spelled with a combining accent displays as the composed one and hashes
	// differently, which for a file name is two files that look like one.
	res!(canon::check_string(path));

	if path.contains('\\') {
		return Err(err!(
			"The shared path \"{}\" carries a backslash. A path is joined with \"/\" and nothing \
			else, so a backslash is either a separator this format does not have or a character in \
			a name that will not survive being written down.", path;
		Invalid, Input));
	}
	if path.starts_with('/') {
		return Err(err!(
			"The shared path \"{}\" is absolute. Every path in a share is relative to the \
			receiver's own copy of the Diamond, and an absolute one names a place on their machine \
			that the sender cannot know and must not reach.", path;
		Invalid, Input));
	}
	// A scheme, by the same rule `safePath` uses: a letter, then letters, digits, `+`, `.` or `-`,
	// then a colon. `c:/x` and `data:…` are both caught, and neither is a relative path.
	if let Some(colon) = path.find(':') {
		let head = &path[..colon];
		if !head.is_empty()
			&& head.starts_with(|c: char| c.is_ascii_alphabetic())
			&& head.chars().all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '.' || c == '-')
		{
			return Err(err!(
				"The shared path \"{}\" begins with what reads as a scheme, \"{}:\". A share \
				carries files, never locations.", path, head;
			Invalid, Input));
		}
	}
	for seg in path.split('/') {
		if seg.is_empty() {
			return Err(err!(
				"The shared path \"{}\" carries an empty segment. It is refused rather than \
				tidied: a path that needs tidying has two spellings, and two spellings of one file \
				are two addresses for one Diamond.", path;
			Invalid, Input));
		}
		if seg == "." || seg == ".." {
			return Err(err!(
				"The shared path \"{}\" carries a \"{}\" segment. A share reaches nothing outside \
				the Diamond it is a copy of, and a path that walks is refused rather than \
				resolved.", path, seg;
			Invalid, Input));
		}
	}

	for prefix in REFUSED_PREFIXES {
		if path.starts_with(prefix) {
			return Err(err!(
				"The shared path \"{}\" is under \"{}\", which a share may not carry. That folder \
				holds the SENDER's own record — the stamps the sync merge decides on, the link \
				sidecar, and the append-only log of what agents did in their copy. A person \
				sending a recipe does not mean to send that, and the receiver's copy is new: its \
				record starts empty because nothing has happened in it yet.", path, prefix;
			Invalid, Input));
		}
	}
	if path == REFUSED_EXACT {
		return Err(err!(
			"A share may not carry \"{}\". It is a DELIVERY record: it says which bytes were \
			delivered to that instance and at what template version, and it decides which files a \
			future template fix may replace. The receiver was not delivered to; they were given a \
			copy by a person. One carried across from somebody else's machine would pin their copy \
			against updates they never chose, and a doctored one would do it on purpose. A copy \
			with no record is a case the receiving client already knows: it asks.", REFUSED_EXACT;
		Invalid, Input));
	}
	Ok(())
}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ THE SHARE                                                                 │
// └───────────────────────────────────────────────────────────────────────────┘

/// A `daimond/share/0` payload.
///
/// Every field is inside the payload region, so every field is covered by the envelope's `hash`
/// and therefore by its signature. A relay handling this artefact can add nothing to it, remove
/// nothing from it, and rewrite nothing in it — including [`Share::code`] — without the signature
/// ceasing to verify.
///
/// There is no sender field and no timestamp, for the reasons `crate::post` gives: the author is
/// the envelope's `author`, and the time is the envelope's and advisory. There is also **no
/// identifier of the sender's Diamond**, which is particular to this schema. The receiver's copy
/// is a new Diamond with an identifier of their own making, so an identifier that travelled would
/// either be a field nobody read or a way for one person's share to land on top of another
/// person's Diamond.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Share {
	/// The shared thing's display name. Advisory, exactly as a card's label is.
	pub name:	String,
	/// The recipient's public key.
	pub to:	Vec<u8>,
	/// Per-share randomness, so two identical shares are two addresses.
	pub nonce:	Vec<u8>,
	/// The sender's covering sentence, if they wrote one.
	pub note:	Option<String>,
	/// The files, ordered by path and each path carried once.
	pub files:	Vec<File>,
	/// Whether the files include executable page code.
	///
	/// The sender's own claim, signed, and checked against the files both ways. See the module
	/// note for why it is here rather than derived, and why it is always written.
	pub code:	bool,
}

impl Share {

	/// Builds a share, putting the files in canonical order and stating the code claim for the
	/// caller.
	///
	/// The claim is computed here rather than taken as an argument because a caller who could
	/// supply it could supply the wrong one, and the only honest value at composition time is what
	/// the files say. [`Share::code`] remains a field, and remains signed, because it is the value
	/// THIS build computed and a later one may disagree with; what this constructor removes is the
	/// chance to disagree with it on purpose.
	pub fn new(
		name:	String,
		to:	Vec<u8>,
		nonce:	Vec<u8>,
		note:	Option<String>,
		files:	Vec<File>,
	)
		-> Self
	{
		let mut files = files;
		files.sort_by(|a, b| a.path.as_bytes().cmp(b.path.as_bytes()));
		let code = code_file(&files).is_some();
		Self { name, to, nonce, note, files, code }
	}

	/// Encodes this share as a canonical daticle.
	pub fn to_dat(&self) -> Outcome<Dat> {
		let mut map = DaticleMap::new();
		// Always written, never omitted when false. An omitted false and a sender whose build had
		// never heard of the field are the same bytes, and those are the two things a receiver must
		// be able to tell apart.
		map.insert(dat!(KEY_CODE),	Dat::Bool(self.code));
		let mut list = Vec::with_capacity(self.files.len());
		for f in &self.files {
			list.push(res!(f.to_dat()));
		}
		map.insert(dat!(KEY_FILES),	Dat::List(list));
		map.insert(dat!(KEY_NAME),	Dat::Str(self.name.clone()));
		map.insert(dat!(KEY_NONCE),	Dat::BU8(self.nonce.clone()));
		// An absent note is OMITTED, never encoded as `none` or as an empty string: SPEC.md §3
		// rules 4 and 8, so that one share has one encoding.
		if let Some(n) = &self.note {
			map.insert(dat!(KEY_NOTE), Dat::Str(n.clone()));
		}
		map.insert(dat!(KEY_TO),	Dat::BU8(self.to.clone()));
		Ok(Dat::Map(map))
	}

	/// Reads a share, enforcing every rule this schema declares.
	pub fn from_dat(d: &Dat) -> Outcome<Self> {
		let map = match d {
			Dat::Map(m) => m,
			Dat::OrdMap(_) => return Err(err!(
				"SPEC.md §3 rule 2: a share payload is a Dat::Map, never a Dat::OrdMap. An OrdMap \
				follows the author's typing rather than the keys, so the same share would have as \
				many addresses as there are orders to write it in.";
			Invalid, Input, Mismatch)),
			other => return Err(err!(
				"A share payload must be a Dat::Map, found a {:?}.", other.kind();
			Invalid, Input, Mismatch)),
		};
		let allowed: Vec<&str> = {
			let mut v = vec![KEY_CODE, KEY_FILES, KEY_NAME, KEY_NONCE, KEY_TO];
			if map.contains_key(&dat!(KEY_NOTE)) { v.push(KEY_NOTE); }
			v
		};
		res!(exact_keys(map, &allowed, "share"));

		let name = match res!(get(map, KEY_NAME)) {
			Dat::Str(s) => s.clone(),
			other => return Err(err!(
				"The share key \"{}\" must be a string, found a {:?}.", KEY_NAME, other.kind();
			Invalid, Input, Mismatch)),
		};
		res!(check_text(&name, KEY_NAME, limit::NAME_BYTES));

		let to = res!(get_bytes(map, KEY_TO, limit::KEY_BYTES));
		let nonce = res!(get_bytes(map, KEY_NONCE, limit::NONCE_BYTES));

		let note = match map.get(&dat!(KEY_NOTE)) {
			Some(Dat::Str(s)) => {
				if s.is_empty() {
					return Err(err!(
						"SPEC.md §3 rule 8: the share carries an empty \"{}\". A note a reader \
						would draw identically whether present or absent gives one share two \
						encodings, and so two addresses. Omit the key.", KEY_NOTE;
					Invalid, Input));
				}
				res!(check_text(s, KEY_NOTE, limit::NOTE_BYTES));
				Some(s.clone())
			},
			Some(other) => return Err(err!(
				"The share key \"{}\" must be a string, found a {:?}.", KEY_NOTE, other.kind();
			Invalid, Input, Mismatch)),
			None => None,
		};

		let code = match res!(get(map, KEY_CODE)) {
			Dat::Bool(b) => *b,
			other => return Err(err!(
				"The share key \"{}\" must be a bool, found a {:?}. It is the sender's signed \
				statement about whether this share carries a program, and a reader that could not \
				read it would be asking a person to consent to something nobody described.",
				KEY_CODE, other.kind();
			Invalid, Input, Mismatch)),
		};

		let files = match res!(get(map, KEY_FILES)) {
			Dat::List(items) => {
				if items.is_empty() {
					return Err(err!(
						"The share carries no files. A share is a copy of something, and a copy of \
						nothing is not a smaller share; it is not one.";
					Invalid, Input, Missing));
				}
				if items.len() > limit::FILES {
					return Err(err!(
						"The share carries {} files, exceeding the limit of {}. Each is examined \
						and written on the RECEIVER's machine when the share is opened.",
						items.len(), limit::FILES;
					Invalid, Input, LimitReached));
				}
				let mut out: Vec<File> = Vec::with_capacity(items.len());
				let mut total: usize = 0;
				for (i, item) in items.iter().enumerate() {
					let f = res!(File::from_dat(item).map_err(|e| err!(e,
						"File {} of {} is not one this schema admits.", i, items.len();
					Invalid, Input)));
					// Ordered by path, and each path once. A set of files written in two orders
					// would be two addresses for one Diamond, and the same file twice is a share
					// whose meaning depends on which entry the receiver writes last.
					if let Some(prev) = out.last() {
						if f.path.as_bytes() == prev.path.as_bytes() {
							return Err(err!(
								"The share carries the path \"{}\" twice. Which copy the receiver \
								ends up with would then depend on the order they were written in.",
								f.path;
							Invalid, Input));
						}
						if f.path.as_bytes() < prev.path.as_bytes() {
							return Err(err!(
								"The share's files are not in path order: \"{}\" follows \"{}\". \
								The order is fixed so that one set of files has one encoding, and \
								so one address; it is refused rather than sorted, because sorting \
								it would be accepting a second encoding and quietly rewriting it.",
								f.path, prev.path;
							Invalid, Input));
						}
					}
					total = total.saturating_add(f.body.len());
					out.push(f);
				}
				if total > limit::TOTAL_BYTES {
					return Err(err!(
						"The share's files carry {} bytes together, exceeding the limit of {}. It \
						is refused rather than trimmed: a share missing a file is not a smaller \
						share, and the ceiling is the receiver's sync budget rather than this \
						format's.", total, limit::TOTAL_BYTES;
					Invalid, Input, LimitReached));
				}
				out
			},
			Dat::Vek(_) => return Err(err!(
				"SPEC.md §3 rule 7: \"{}\" is a Dat::List, never a Dat::Vek, even where every \
				element shares a kind.", KEY_FILES;
			Invalid, Input, Mismatch)),
			other => return Err(err!(
				"The share key \"{}\" must be a list, found a {:?}.", KEY_FILES, other.kind();
			Invalid, Input, Mismatch)),
		};

		// The consent bit against the files, both ways. Neither direction is a formality: one stops
		// a program arriving under a claim that there is none, and the other stops a sender asking
		// for consent they do not need, which is how a person learns to wave the question away.
		match (code, code_file(&files)) {
			(false, Some(f)) => return Err(err!(
				"The share states that it carries no code, and carries \"{}\". The claim is the \
				sender's, it is signed, and it is what a receiver is asked to consent to before \
				anything runs, so a share that contradicts its own claim is refused rather than \
				corrected.", f.path;
			Invalid, Input, Mismatch)),
			(true, None) => return Err(err!(
				"The share states that it carries code, and carries none. It is refused rather \
				than accepted as harmless caution: a receiver asked to consent to a program that \
				is not there is a receiver being taught that the question does not mean anything.";
			Invalid, Input, Mismatch)),
			_ => {},
		}

		Ok(Self { name, to, nonce, note, files, code })
	}

	/// Encodes this share to the canonical bytes that become the payload region.
	pub fn encode(&self) -> Outcome<Vec<u8>> {
		let d = res!(self.to_dat());
		// Read straight back, so that a share which cannot be decoded can never be signed. Signing
		// bytes no reader will accept produces an artefact that is valid to its author and refused
		// by everybody else.
		res!(Self::from_dat(&d));
		let bytes = res!(d.to_bytes(Vec::new()));
		if bytes.len() > sbj_limit::TREE_BYTES {
			return Err(err!(
				"The encoded share is {} bytes, exceeding the payload region limit of {}.",
				bytes.len(), sbj_limit::TREE_BYTES;
			Invalid, Input, LimitReached));
		}
		Ok(bytes)
	}

	/// Decodes a share from the bytes of a payload region, which must be consumed exactly.
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
				"The share payload occupies {} of the {} bytes supplied, leaving {} trailing.",
				n, buf.len(), buf.len() - n;
			Invalid, Input, Decode));
		}
		let re = res!(d.to_bytes(Vec::new()));
		if re != buf {
			return Err(err!(
				"The share payload is not in canonical form: it re-encodes to {} bytes against the \
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
			"The share is missing the required key \"{}\".", key;
		Invalid, Input, Missing)),
	}
}

/// Reads a required `BU8` key of an exact width.
///
/// Exact rather than bounded because every one of these is a key or a nonce, and each has one
/// size. A short one is not a smaller key; it is a different thing.
fn get_bytes(map: &DaticleMap, key: &str, width: usize) -> Outcome<Vec<u8>> {
	let b = match res!(get(map, key)) {
		Dat::BU8(b) => b.clone(),
		other => return Err(err!(
			"The share key \"{}\" must carry a BU8, found a {:?}.", key, other.kind();
		Invalid, Input, Mismatch)),
	};
	if b.len() != width {
		return Err(err!(
			"The share key \"{}\" carries {} bytes and must carry exactly {}.", key, b.len(), width;
		Invalid, Input, Mismatch));
	}
	Ok(b)
}

/// Checks a string field against the canonical text rules and a byte ceiling.
fn check_text(s: &str, key: &str, max: usize) -> Outcome<()> {
	if s.len() > max {
		return Err(err!(
			"The share's \"{}\" is {} bytes, exceeding the limit of {}.", key, s.len(), max;
		Invalid, Input, LimitReached));
	}
	res!(canon::check_string(s));
	Ok(())
}

/// Requires a map to carry exactly the named keys — no more, and no fewer.
///
/// Both directions, because they catch different faults. A missing key is a share that does not
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

	/// A plausible share of data alone, with fixed contents.
	fn sample() -> Share {
		Share::new(
			fmt!("Sourdough"),
			vec![0xA1; limit::KEY_BYTES],
			vec![0xB2; limit::NONCE_BYTES],
			None,
			vec![
				File { path: fmt!("crystal.json"), body: b"{\"loaves\":3}".to_vec() },
				File { path: fmt!("bakes/2026.jsonl"), body: b"{\"day\":1}\n".to_vec() },
			],
		)
	}

	/// The same share, carrying a page.
	fn sample_capp() -> Share {
		let mut files = sample().files;
		files.push(File { path: fmt!("crystal.html"), body: b"<p>hello</p>".to_vec() });
		Share::new(
			fmt!("Life log"),
			vec![0xA1; limit::KEY_BYTES],
			vec![0xB2; limit::NONCE_BYTES],
			Some(fmt!("The food log we talked about.")),
			files,
		)
	}

	#[test]
	fn test_round_trip_data_only() -> Outcome<()> {
		let s = sample();
		assert!(!s.code, "A share of two data files claims to carry code.");
		let bytes = res!(s.encode());
		let back = res!(Share::decode(&bytes));
		assert_eq!(s, back);
		Ok(())
	}

	#[test]
	fn test_round_trip_with_a_capp() -> Outcome<()> {
		let s = sample_capp();
		assert!(s.code, "A share carrying crystal.html does not claim to carry code.");
		let bytes = res!(s.encode());
		let back = res!(Share::decode(&bytes));
		assert_eq!(s, back);
		assert!(back.code);
		Ok(())
	}

	/// The property the whole schema exists for: a program cannot travel under a claim of none.
	#[test]
	fn test_a_page_under_a_false_code_claim_is_refused() -> Outcome<()> {
		let mut s = sample_capp();
		s.code = false;			// the sender lies, or a build computes it differently
		match s.encode() {
			Ok(_) => Err(err!(
				"A share carrying a page under `code: false` was encoded, so a program could \
				arrive as data."; Test, Invalid)),
			Err(e) => {
				let msg = fmt!("{}", e);
				assert!(msg.contains("crystal.html"),
					"The refusal does not name the file that is code: {}", msg);
				Ok(())
			},
		}
	}

	/// And the other way, so the bit cannot be set for effect.
	#[test]
	fn test_a_code_claim_with_no_code_is_refused() -> Outcome<()> {
		let mut s = sample();
		s.code = true;
		match s.encode() {
			Ok(_) => Err(err!(
				"A share claiming code and carrying none was encoded."; Test, Invalid)),
			Err(_) => Ok(()),
		}
	}

	/// The claim survives the wire, which is the point of it being signed rather than derived.
	#[test]
	fn test_the_code_bit_is_in_the_bytes() -> Outcome<()> {
		let data = res!(sample().encode());
		let capp = res!(sample_capp().encode());
		assert_ne!(data, capp);
		// And a payload with the bit flipped is not a payload this schema reads.
		let mut m = match res!(sample().to_dat()) {
			Dat::Map(m) => m,
			other => return Err(err!(
				"A share encodes as a map, and this is a {:?}.", other.kind(); Test, Bug)),
		};
		m.insert(dat!(KEY_CODE), Dat::Bool(true));
		match Share::from_dat(&Dat::Map(m)) {
			Ok(_) => Err(err!(
				"A share whose code bit was flipped on the wire was read."; Test, Invalid)),
			Err(_) => Ok(()),
		}
	}

	/// A code suffix in capitals is still a code suffix.
	#[test]
	fn test_the_suffix_check_ignores_case() -> Outcome<()> {
		assert!(is_code_path("Crystal.HTML"));
		assert!(is_code_path("a/b/PAGE.Js"));
		assert!(!is_code_path("crystal.json"));
		assert!(!is_code_path("notes.html.md"));
		let s = Share::new(
			fmt!("Shouting"),
			vec![0xA1; limit::KEY_BYTES],
			vec![0xB2; limit::NONCE_BYTES],
			None,
			vec![File { path: fmt!("PAGE.HTML"), body: b"<p>x</p>".to_vec() }],
		);
		assert!(s.code, "A file called PAGE.HTML was not counted as code.");
		Ok(())
	}

	#[test]
	fn test_files_must_be_in_path_order() -> Outcome<()> {
		let s = sample();
		let mut m = match res!(s.to_dat()) {
			Dat::Map(m) => m,
			other => return Err(err!(
				"A share encodes as a map, and this is a {:?}.", other.kind(); Test, Bug)),
		};
		let mut list = Vec::new();
		for f in s.files.iter().rev() {
			list.push(res!(f.to_dat()));
		}
		m.insert(dat!(KEY_FILES), Dat::List(list));
		match Share::from_dat(&Dat::Map(m)) {
			Ok(_) => Err(err!(
				"Files out of path order were accepted, so one Diamond has as many addresses as \
				there are orders to list its files in."; Test, Invalid)),
			Err(_) => Ok(()),
		}
	}

	#[test]
	fn test_a_duplicate_path_is_refused() -> Outcome<()> {
		let s = Share::new(
			fmt!("Twice"),
			vec![0xA1; limit::KEY_BYTES],
			vec![0xB2; limit::NONCE_BYTES],
			None,
			vec![
				File { path: fmt!("a.json"), body: b"1".to_vec() },
				File { path: fmt!("a.json"), body: b"2".to_vec() },
			],
		);
		match s.encode() {
			Ok(_) => Err(err!("Two files at one path were accepted."; Test, Invalid)),
			Err(_) => Ok(()),
		}
	}

	/// The constructor puts the files in order, so a caller cannot mint a second address by
	/// listing them differently.
	#[test]
	fn test_new_orders_the_files() -> Outcome<()> {
		let a = Share::new(fmt!("N"), vec![0xA1; 32], vec![0xB2; 16], None, vec![
			File { path: fmt!("b.json"), body: b"2".to_vec() },
			File { path: fmt!("a.json"), body: b"1".to_vec() },
		]);
		let b = Share::new(fmt!("N"), vec![0xA1; 32], vec![0xB2; 16], None, vec![
			File { path: fmt!("a.json"), body: b"1".to_vec() },
			File { path: fmt!("b.json"), body: b"2".to_vec() },
		]);
		assert_eq!(res!(a.encode()), res!(b.encode()));
		Ok(())
	}

	/// Each of the three refused paths, refused, and each saying which rule it broke.
	#[test]
	fn test_the_three_refused_paths() -> Outcome<()> {
		for (path, says) in [
			(".daimond/log.jsonl",	".daimond/"),
			("versions/3/crystal.json",	"versions/"),
			("capp.json",	"capp.json"),
		] {
			match check_path(path) {
				Ok(()) => return Err(err!(
					"The path \"{}\" was accepted into a share.", path; Test, Invalid)),
				Err(e) => {
					let msg = fmt!("{}", e);
					assert!(msg.contains(says),
						"The refusal of \"{}\" does not name what it broke: {}", path, msg);
				},
			}
		}
		// And each is refused only where it means what it says: the delivery record is the one at
		// the root, and a folder of the receiver's own making may hold anything.
		res!(check_path("recipes/capp.json"));
		res!(check_path("notes/versions/old.md"));
		Ok(())
	}

	#[test]
	fn test_a_walking_path_is_refused() -> Outcome<()> {
		for path in ["../secrets.json", "a/../../b.json", "a/./b.json", "/etc/passwd",
			"a//b.json", "a\\b.json", "data:text/plain,x", "c:/notes.md"]
		{
			if check_path(path).is_ok() {
				return Err(err!(
					"The path \"{}\" was accepted into a share.", path; Test, Invalid));
			}
		}
		Ok(())
	}

	/// A path is refused rather than tidied, which is what makes one file one address.
	#[test]
	fn test_a_path_is_not_normalised() -> Outcome<()> {
		// `a/./b.json` and `a/b.json` would be the same file after tidying and are different bytes,
		// so accepting the first would give one Diamond two addresses.
		assert!(check_path("a/./b.json").is_err());
		res!(check_path("a/b.json"));
		Ok(())
	}

	#[test]
	fn test_an_empty_share_is_refused() -> Outcome<()> {
		let mut m = match res!(sample().to_dat()) {
			Dat::Map(m) => m,
			other => return Err(err!(
				"A share encodes as a map, and this is a {:?}.", other.kind(); Test, Bug)),
		};
		m.insert(dat!(KEY_FILES), Dat::List(Vec::new()));
		match Share::from_dat(&Dat::Map(m)) {
			Ok(_) => Err(err!("A share of no files was accepted."; Test, Invalid)),
			Err(_) => Ok(()),
		}
	}

	#[test]
	fn test_an_empty_note_is_refused() -> Outcome<()> {
		let mut s = sample();
		s.note = Some(String::new());
		match s.encode() {
			Ok(_) => Err(err!(
				"An empty note was accepted, so a share with nothing to say has two encodings.";
			Test, Invalid)),
			Err(_) => Ok(()),
		}
	}

	/// An absent note is omitted, and the two shapes are different bytes.
	#[test]
	fn test_the_note_is_omitted_not_none() -> Outcome<()> {
		let bare = res!(sample().encode());
		let mut s = sample();
		s.note = Some(fmt!("Here you are."));
		assert_ne!(res!(s.encode()), bare);
		assert_eq!(res!(Share::decode(&bare)).note, None);
		Ok(())
	}

	#[test]
	fn test_trailing_bytes_refused() -> Outcome<()> {
		let mut bytes = res!(sample().encode());
		bytes.push(0x00);
		match Share::decode(&bytes) {
			Ok(_) => Err(err!("A payload with a trailing byte was accepted."; Test, Invalid)),
			Err(_) => Ok(()),
		}
	}

	#[test]
	fn test_ordmap_refused() -> Outcome<()> {
		let ord = oxedyne_fe2o3_jdat::map::create_dat_ordmap(vec![
			(dat!(KEY_CODE),	Dat::Bool(false)),
			(dat!(KEY_NAME),	Dat::Str(fmt!("N"))),
		]);
		match Share::from_dat(&ord) {
			Ok(_) => Err(err!("An OrdMap payload was accepted."; Test, Invalid)),
			Err(_) => Ok(()),
		}
	}

	#[test]
	fn test_unknown_key_refused() -> Outcome<()> {
		let mut m = match res!(sample().to_dat()) {
			Dat::Map(m) => m,
			other => return Err(err!(
				"A share encodes as a map, and this is a {:?}.", other.kind(); Test, Bug)),
		};
		m.insert(dat!("from"), Dat::Str(fmt!("somebody else")));
		match Share::from_dat(&Dat::Map(m)) {
			Ok(_) => Err(err!("A share carrying a `from` field was accepted."; Test, Invalid)),
			Err(_) => Ok(()),
		}
	}

	/// The code bit is required, not optional-and-false-by-default.
	#[test]
	fn test_a_missing_code_bit_is_refused() -> Outcome<()> {
		let mut m = match res!(sample().to_dat()) {
			Dat::Map(m) => m,
			other => return Err(err!(
				"A share encodes as a map, and this is a {:?}.", other.kind(); Test, Bug)),
		};
		m.remove(&dat!(KEY_CODE));
		match Share::from_dat(&Dat::Map(m)) {
			Ok(_) => Err(err!(
				"A share with no code claim was read, so 'they said no' and 'they did not say' \
				are the same artefact."; Test, Invalid)),
			Err(_) => Ok(()),
		}
	}

	#[test]
	fn test_nonce_and_key_widths_are_exact() -> Outcome<()> {
		let mut s = sample();
		s.nonce = vec![0xB2; limit::NONCE_BYTES - 1];
		assert!(s.encode().is_err(), "A short nonce was accepted.");
		let mut s = sample();
		s.to = vec![0xA1; limit::KEY_BYTES + 1];
		assert!(s.encode().is_err(), "An overlong recipient key was accepted.");
		Ok(())
	}

	/// Two identical shares to one recipient are two addresses, because the nonce is signed.
	#[test]
	fn test_the_nonce_separates_identical_shares() -> Outcome<()> {
		let a = sample();
		let mut b = sample();
		b.nonce = vec![0xB3; limit::NONCE_BYTES];
		assert_eq!(a.files, b.files);
		assert_ne!(res!(a.encode()), res!(b.encode()));
		Ok(())
	}

	#[test]
	fn test_too_many_files_refused() -> Outcome<()> {
		let mut files = Vec::new();
		for i in 0..(limit::FILES + 1) {
			files.push(File { path: fmt!("f{:04}.json", i), body: b"{}".to_vec() });
		}
		let s = Share::new(fmt!("Many"), vec![0xA1; 32], vec![0xB2; 16], None, files);
		match s.encode() {
			Ok(_) => Err(err!("More files than the limit were accepted."; Test, Invalid)),
			Err(_) => Ok(()),
		}
	}

	/// The ceiling is a boundary and not a scare: exactly the limit is accepted.
	#[test]
	fn test_files_at_the_limit_accepted() -> Outcome<()> {
		let mut files = Vec::new();
		for i in 0..limit::FILES {
			files.push(File { path: fmt!("f{:04}.json", i), body: b"{}".to_vec() });
		}
		let s = Share::new(fmt!("Many"), vec![0xA1; 32], vec![0xB2; 16], None, files);
		let bytes = res!(s.encode());
		assert_eq!(res!(Share::decode(&bytes)).files.len(), limit::FILES);
		Ok(())
	}

	#[test]
	fn test_total_bytes_over_the_limit_refused() -> Outcome<()> {
		let s = Share::new(fmt!("Heavy"), vec![0xA1; 32], vec![0xB2; 16], None, vec![
			File { path: fmt!("a.bin"), body: vec![0u8; limit::TOTAL_BYTES / 2] },
			File { path: fmt!("b.bin"), body: vec![0u8; limit::TOTAL_BYTES / 2 + 1] },
		]);
		match s.encode() {
			Ok(_) => Err(err!("A share over the byte ceiling was accepted."; Test, Invalid)),
			Err(_) => Ok(()),
		}
	}

	/// A file body is BYTES and is held to no text rule: a Diamond holds pictures too.
	#[test]
	fn test_a_body_may_be_arbitrary_bytes() -> Outcome<()> {
		let s = Share::new(fmt!("Picture"), vec![0xA1; 32], vec![0xB2; 16], None, vec![
			File { path: fmt!("shot.png"), body: vec![0x89, 0x50, 0x4E, 0x47, 0x00, 0xFF] },
		]);
		let bytes = res!(s.encode());
		assert_eq!(res!(Share::decode(&bytes)).files[0].body, s.files[0].body);
		Ok(())
	}

	/// A path, unlike a body, is text and is held to §3 rule 5.
	#[test]
	fn test_a_path_must_be_nfc() -> Outcome<()> {
		assert!(check_path("cafe\u{0301}/notes.md").is_err(),
			"A path with a combining accent was accepted, so one file has two spellings.");
		res!(check_path("caf\u{e9}/notes.md"));
		Ok(())
	}
}
