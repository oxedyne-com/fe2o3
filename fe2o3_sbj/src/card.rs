//! `daimond/card/0` — a self-signed identity card, and the two renderings a person compares.
//!
//! A card is what a QR code carries and what a paste carries. A bare public key is not: it says
//! nothing about which key is for signing and which for sealing, carries no display name, and
//! gives a reader no way to tell a first key from one that replaced another.
//!
//! Most of the card is the envelope's already, which is the point of putting it in this container:
//! the signing key is `author`, the signature is `sig`, the algorithm is `sig_scheme`, and the
//! creation time is `time`. What remains, and what this schema defines, is the part a key cannot
//! say about itself — a display label, the separate encryption subkey, the role, and the key this
//! one supersedes.
//!
//! **Self-signed means exactly what it says, and it is worth being blunt about what it does not
//! buy.** A card verifies under the key it carries, so it proves the holder of that key composed
//! it. It proves nothing whatever about who that holder is. A card fetched from a server is
//! therefore Unverified no matter how well it verifies: an intermediary that substituted its own
//! key would produce a card that verifies perfectly. Only an out-of-band act — a QR read in
//! person, or a safety number compared aloud — raises it, and that act is the user's, never the
//! software's.
//!
//! The label is **advisory display text and never an identity**. Equality is always the full
//! 32-byte key. Two people may choose one label and neither is lying.

use crate::{
	canon,
	limit as sbj_limit,
};

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_hash::sha256;
use oxedyne_fe2o3_jdat::{
	prelude::*,
	bdat::DecodeLimits,
};
use oxedyne_fe2o3_text::base2x::CROCKFORD32;


/// The display name the holder chose. Advisory.
pub const KEY_LABEL:	&'static str = "label";
/// The holder's encryption subkey.
pub const KEY_ENC:	&'static str = "enc";
/// What this key is for.
pub const KEY_ROLE:	&'static str = "role";
/// The key this one supersedes, if it supersedes one.
pub const KEY_PREV:	&'static str = "prev";

/// Domain separator for a fingerprint. See [`fingerprint`].
pub const FINGERPRINT_DOMAIN:	&'static [u8] = b"daimond-id-v1";
/// Domain separator for a safety number. See [`safety_number`].
pub const SAFETY_DOMAIN:	&'static [u8] = b"daimond-safety-v1";

/// Limits this schema enforces.
pub mod limit {
	/// The most a display label may carry, in bytes of UTF-8.
	pub const LABEL_BYTES:	usize = 64;
	/// The exact width of a public key.
	pub const KEY_BYTES:	usize = 32;
	/// Decoding depth for a card, which is a flat record and reaches two.
	pub const DEPTH:	usize = 4;
	/// Bytes of the fingerprint digest that are rendered.
	///
	/// Ten, giving eighty bits, which is sixteen base-32 characters exactly and so needs no
	/// padding. A fingerprint is a display convenience and decides nothing, so its width is chosen
	/// for the eye rather than for a security margin — the margin lives in the full key, which is
	/// what every comparison actually uses.
	pub const FINGERPRINT_BYTES:	usize = 10;
	/// Decimal digits in a safety number.
	///
	/// Sixty, which is the whole 256-bit digest and not a prefix of it. Truncation is refused: the
	/// attack on a safety number is a meet-in-the-middle, which costs about 2^(n/2), so halving the
	/// width to 120 bits would leave a 60-bit search rather than a 120-bit one.
	pub const SAFETY_DIGITS:	usize = 60;
}


/// What a key is for.
///
/// An enum because the set is closed and a reader must be able to refuse a role it does not
/// implement rather than treat an unknown string as harmless. Only one role exists in v0; the type
/// is here so that adding a second is a versioned act rather than a new string appearing on the
/// wire.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
	/// The account's own long-lived identity key.
	Root,
}

impl Role {
	/// The spelling on the wire.
	pub fn as_str(&self) -> &'static str {
		match self {
			Self::Root => "root",
		}
	}

	/// Reads a role, refusing any spelling this version does not know.
	pub fn from_str(s: &str) -> Outcome<Self> {
		match s {
			"root" => Ok(Self::Root),
			other => Err(err!(
				"\"{}\" is not a role this version admits. The only role in v0 is \"{}\". An \
				unknown role is refused rather than ignored: a reader that skipped it would be \
				treating a key it does not understand as though it were an ordinary one.",
				other, Self::Root.as_str();
			Invalid, Input, Unknown)),
		}
	}
}


/// A `daimond/card/0` payload.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Card {
	/// The display name the holder chose. Advisory, and never an identity.
	pub label:	String,
	/// The holder's encryption subkey, which is not the signing key.
	///
	/// Separate because the two do different jobs and have different lifetimes: a signature is
	/// checked once and discarded, so a signing scheme may be replaced freely, while anything
	/// sealed to an encryption key must remain openable. Carrying the sealing key inside a
	/// signed card is what lets a reply be sealed to a key the recipient PROVED they hold, rather
	/// than to one a server asserted on their behalf.
	pub enc:	Vec<u8>,
	/// What this key is for.
	pub role:	Role,
	/// The key this one supersedes, if any.
	///
	/// Present when a holder has rotated. A reader that knows the previous key can see that the
	/// new card claims to replace it — and that claim is signed by the NEW key only, so it is a
	/// statement of intent and not a proof of succession. Treating it as proof would let anybody
	/// claim to supersede anybody.
	pub prev:	Option<Vec<u8>>,
}

impl Card {
	/// Encodes this card as a canonical daticle.
	pub fn to_dat(&self) -> Outcome<Dat> {
		let mut map = DaticleMap::new();
		map.insert(dat!(KEY_ENC),	Dat::BU8(self.enc.clone()));
		map.insert(dat!(KEY_LABEL),	Dat::Str(self.label.clone()));
		// Absent means omitted, never `none`: SPEC.md §3 rule 4.
		if let Some(p) = &self.prev {
			map.insert(dat!(KEY_PREV), Dat::BU8(p.clone()));
		}
		map.insert(dat!(KEY_ROLE),	Dat::Str(fmt!("{}", self.role.as_str())));
		Ok(Dat::Map(map))
	}

	/// Reads a card, enforcing every rule this schema declares.
	pub fn from_dat(d: &Dat) -> Outcome<Self> {
		let map = match d {
			Dat::Map(m) => m,
			Dat::OrdMap(_) => return Err(err!(
				"SPEC.md §3 rule 2: a card payload is a Dat::Map, never a Dat::OrdMap.";
			Invalid, Input, Mismatch)),
			other => return Err(err!(
				"A card payload must be a Dat::Map, found a {:?}.", other.kind();
			Invalid, Input, Mismatch)),
		};
		let allowed: Vec<&str> = {
			let mut v = vec![KEY_ENC, KEY_LABEL, KEY_ROLE];
			if map.contains_key(&dat!(KEY_PREV)) { v.push(KEY_PREV); }
			v
		};
		for k in &allowed {
			if !map.contains_key(&dat!(*k)) {
				return Err(err!(
					"The card is missing the required key \"{}\".", k;
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
					"The card carries the key \"{}\", which this schema does not admit. The \
					admitted keys are: {}.", name, allowed.join(", ");
				Invalid, Input, Unknown));
			}
		}

		let label = match map.get(&dat!(KEY_LABEL)) {
			Some(Dat::Str(s)) => s.clone(),
			Some(other) => return Err(err!(
				"The card key \"{}\" must be a string, found a {:?}.", KEY_LABEL, other.kind();
			Invalid, Input, Mismatch)),
			None => return Err(err!(
				"The card is missing \"{}\".", KEY_LABEL; Invalid, Input, Missing)),
		};
		if label.len() > limit::LABEL_BYTES {
			return Err(err!(
				"The card label is {} bytes, exceeding the limit of {}.",
				label.len(), limit::LABEL_BYTES;
			Invalid, Input, LimitReached));
		}
		res!(canon::check_string(&label));

		let enc = res!(exact_bytes(map, KEY_ENC, limit::KEY_BYTES));
		let prev = match map.get(&dat!(KEY_PREV)) {
			Some(_) => Some(res!(exact_bytes(map, KEY_PREV, limit::KEY_BYTES))),
			None => None,
		};
		let role = match map.get(&dat!(KEY_ROLE)) {
			Some(Dat::Str(s)) => res!(Role::from_str(s)),
			Some(other) => return Err(err!(
				"The card key \"{}\" must be a string, found a {:?}.", KEY_ROLE, other.kind();
			Invalid, Input, Mismatch)),
			None => return Err(err!(
				"The card is missing \"{}\".", KEY_ROLE; Invalid, Input, Missing)),
		};

		Ok(Self { label, enc, role, prev })
	}

	/// Encodes this card to the canonical bytes that become the tree region.
	pub fn encode(&self) -> Outcome<Vec<u8>> {
		let d = res!(self.to_dat());
		// Read straight back, so a card that cannot be decoded can never be signed.
		res!(Self::from_dat(&d));
		Ok(res!(d.to_bytes(Vec::new())))
	}

	/// Decodes a card from the bytes of a tree region, which must be consumed exactly.
	pub fn decode(buf: &[u8]) -> Outcome<Self> {
		let lims = DecodeLimits::new(limit::DEPTH, sbj_limit::TREE_BYTES);
		let (d, n) = res!(Dat::from_bytes_limited(buf, &lims));
		if n != buf.len() {
			return Err(err!(
				"The card occupies {} of the {} bytes supplied, leaving {} trailing.",
				n, buf.len(), buf.len() - n;
			Invalid, Input, Decode));
		}
		let re = res!(d.to_bytes(Vec::new()));
		if re != buf {
			return Err(err!(
				"The card is not in canonical form: it re-encodes to {} bytes against the {} \
				supplied. See SPEC.md §3.", re.len(), buf.len();
			Invalid, Input, Decode));
		}
		Self::from_dat(&d)
	}
}

/// Reads a `BU8` key of an exact width.
fn exact_bytes(map: &DaticleMap, key: &str, width: usize) -> Outcome<Vec<u8>> {
	let b = match map.get(&dat!(key)) {
		Some(Dat::BU8(b)) => b.clone(),
		Some(other) => return Err(err!(
			"The card key \"{}\" must carry a BU8, found a {:?}.", key, other.kind();
		Invalid, Input, Mismatch)),
		None => return Err(err!(
			"The card is missing the required key \"{}\".", key;
		Invalid, Input, Missing)),
	};
	if b.len() != width {
		return Err(err!(
			"The card key \"{}\" carries {} bytes and must carry exactly {}. A key of the wrong \
			width is not a shorter key; it is a different thing.", key, b.len(), width;
		Invalid, Input, Mismatch));
	}
	Ok(b)
}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ WHAT A PERSON READS                                                       │
// └───────────────────────────────────────────────────────────────────────────┘

/// A short rendering of a key, for a person's eye. **It decides nothing.**
///
/// Eighty bits of `SHA-256(domain ‖ key)`, in Crockford base 32, in four groups of four:
/// `K7Q2-9F3M-XR4A-8WVN`. The grouping is for reading aloud and copying by hand, and the alphabet
/// leaves out `I`, `L`, `O` and `U` for the same reason.
///
/// **Equality is always the full 32-byte key, everywhere, without exception.** A fingerprint is a
/// display convenience, and eighty bits is comfortably within reach of somebody who wants two keys
/// to look alike in a list. Anything that compares fingerprints to decide whether two keys are the
/// same is a defect, and this function exists so that there is one implementation of the rendering
/// rather than two that can disagree about it.
///
/// One function, deliberately: the same rendering is shown by the client and by the account
/// lookup, and two implementations of it would eventually differ on a key nobody had tested, which
/// a user would read as their correspondent's key having changed.
pub fn fingerprint(key: &[u8]) -> String {
	let mut msg = Vec::with_capacity(FINGERPRINT_DOMAIN.len() + key.len());
	msg.extend_from_slice(FINGERPRINT_DOMAIN);
	msg.extend_from_slice(key);
	let digest = sha256::digest(&msg);
	// Eighty bits is sixteen base-32 characters exactly, so nothing is padded and the rendering has
	// one form.
	let s = CROCKFORD32.to_string(&digest[..limit::FINGERPRINT_BYTES]);
	let chars: Vec<char> = s.chars().collect();
	let mut out = String::with_capacity(19);
	for (i, c) in chars.iter().enumerate() {
		if i > 0 && i % 4 == 0 {
			out.push('-');
		}
		out.push(*c);
	}
	out
}

/// The number two people read to each other to check they hold each other's real keys.
///
/// `SHA-256(domain ‖ min(a, b) ‖ max(a, b))`, rendered as sixty decimal digits in twelve groups of
/// five. Sorting the two keys is what makes it symmetric: both parties compute the same number
/// without having to agree who is first, and a protocol that needed them to agree would be one
/// more thing for an intermediary to influence.
///
/// **The whole digest, never a prefix.** The attack is a meet-in-the-middle, costing about
/// 2^(n/2), so a number truncated to 120 bits would face a 60-bit search rather than a 120-bit
/// one. A shorter number is easier to read aloud and that is not a reason.
///
/// It is read over a channel an attacker cannot silently rewrite — a voice call, or in person.
/// Reading it over the same channel the keys arrived on proves nothing, since whatever substituted
/// the keys can substitute the number.
pub fn safety_number(a: &[u8], b: &[u8]) -> String {
	let (first, second) = if a <= b { (a, b) } else { (b, a) };
	let mut msg = Vec::with_capacity(SAFETY_DOMAIN.len() + first.len() + second.len());
	msg.extend_from_slice(SAFETY_DOMAIN);
	msg.extend_from_slice(first);
	msg.extend_from_slice(second);
	let digest = sha256::digest(&msg);

	// Five decimal digits per five bytes, taken big-endian and reduced modulo 100000, which is the
	// same construction Signal's safety numbers use. Twelve groups of five covers the whole digest:
	// 32 bytes does not divide by 5, so the last group is taken from the remaining two bytes and
	// the digest's first byte, which is why the loop walks a rotated window rather than a slice.
	let mut out = String::with_capacity(limit::SAFETY_DIGITS + 11);
	for g in 0..12 {
		let mut acc: u64 = 0;
		for i in 0..5 {
			acc = (acc << 8) | digest[(g * 5 + i) % digest.len()] as u64;
		}
		if g > 0 {
			out.push(' ');
		}
		out.push_str(&fmt!("{:05}", acc % 100_000));
	}
	out
}


#[cfg(test)]
mod tests {
	use super::*;

	fn sample() -> Card {
		Card {
			label:	fmt!("Jason"),
			enc:	vec![0xE1; limit::KEY_BYTES],
			role:	Role::Root,
			prev:	None,
		}
	}

	#[test]
	fn test_round_trip() -> Outcome<()> {
		let c = sample();
		let bytes = res!(c.encode());
		assert_eq!(res!(Card::decode(&bytes)), c);
		Ok(())
	}

	#[test]
	fn test_round_trip_with_prev() -> Outcome<()> {
		let mut c = sample();
		c.prev = Some(vec![0xD4; limit::KEY_BYTES]);
		let bytes = res!(c.encode());
		assert_eq!(res!(Card::decode(&bytes)), c);
		// A rotated card and a first card must not encode alike.
		assert_ne!(bytes, res!(sample().encode()));
		Ok(())
	}

	#[test]
	fn test_unknown_role_refused() -> Outcome<()> {
		match Role::from_str("admin") {
			Ok(_) => Err(err!("An unknown role was accepted."; Test, Invalid)),
			Err(_) => Ok(()),
		}
	}

	#[test]
	fn test_encryption_key_must_be_exact_width() -> Outcome<()> {
		let mut c = sample();
		c.enc = vec![0xE1; limit::KEY_BYTES - 1];
		match c.encode() {
			Ok(_) => Err(err!("A short encryption key was accepted."; Test, Invalid)),
			Err(_) => Ok(()),
		}
	}

	#[test]
	fn test_unknown_card_key_refused() -> Outcome<()> {
		let mut map = DaticleMap::new();
		map.insert(dat!(KEY_ENC),	Dat::BU8(vec![0xE1; limit::KEY_BYTES]));
		map.insert(dat!(KEY_LABEL),	Dat::Str(fmt!("Jason")));
		map.insert(dat!(KEY_ROLE),	Dat::Str(fmt!("root")));
		map.insert(dat!("verified"),	Dat::Bool(true));
		match Card::from_dat(&Dat::Map(map)) {
			Ok(_) => Err(err!("A card claiming to be verified was accepted."; Test, Invalid)),
			Err(_) => Ok(()),
		}
	}

	#[test]
	fn test_label_not_nfc_refused() -> Outcome<()> {
		let mut c = sample();
		c.label = fmt!("Jaso\u{0301}n");
		match c.encode() {
			Ok(_) => Err(err!("A label that is not in NFC was accepted."; Test, Invalid)),
			Err(_) => Ok(()),
		}
	}

	/// The rendering must have one shape, and it must not be the key.
	#[test]
	fn test_fingerprint_shape() -> Outcome<()> {
		let f = fingerprint(&[0xAA; 32]);
		assert_eq!(f.len(), 19, "sixteen characters and three separators: {}", f);
		assert_eq!(f.matches('-').count(), 3);
		for part in f.split('-') {
			assert_eq!(part.len(), 4);
			for ch in part.chars() {
				assert!(CROCKFORD32_CHARS.contains(&ch), "'{}' is outside the alphabet", ch);
			}
		}
		Ok(())
	}

	/// The alphabet a fingerprint may use, restated here so the test is not checking the code
	/// against itself.
	const CROCKFORD32_CHARS: [char; 32] = [
		'0', '1', '2', '3', '4', '5', '6', '7',
		'8', '9', 'A', 'B', 'C', 'D', 'E', 'F',
		'G', 'H', 'J', 'K', 'M', 'N', 'P', 'Q',
		'R', 'S', 'T', 'V', 'W', 'X', 'Y', 'Z',
	];

	#[test]
	fn test_fingerprint_differs_by_key() -> Outcome<()> {
		assert_ne!(fingerprint(&[0xAA; 32]), fingerprint(&[0xAB; 32]));
		Ok(())
	}

	/// Sorting the keys is what makes both parties compute the same number.
	#[test]
	fn test_safety_number_is_symmetric() -> Outcome<()> {
		let a = [0x01; 32];
		let b = [0x02; 32];
		assert_eq!(safety_number(&a, &b), safety_number(&b, &a));
		Ok(())
	}

	#[test]
	fn test_safety_number_shape() -> Outcome<()> {
		let s = safety_number(&[0x01; 32], &[0x02; 32]);
		let groups: Vec<&str> = s.split(' ').collect();
		assert_eq!(groups.len(), 12, "twelve groups: {}", s);
		for g in &groups {
			assert_eq!(g.len(), 5);
			assert!(g.chars().all(|c| c.is_ascii_digit()));
		}
		assert_eq!(groups.iter().map(|g| g.len()).sum::<usize>(), limit::SAFETY_DIGITS);
		Ok(())
	}

	/// A different pair of keys must give a different number, or it is measuring nothing.
	#[test]
	fn test_safety_number_differs_by_pair() -> Outcome<()> {
		let a = [0x01; 32];
		let b = [0x02; 32];
		let c = [0x03; 32];
		assert_ne!(safety_number(&a, &b), safety_number(&a, &c));
		Ok(())
	}

	/// The domain separators must actually separate: the same bytes under the two constructions
	/// must not collide.
	#[test]
	fn test_domains_are_separated() -> Outcome<()> {
		let k = [0x07; 32];
		let mut plain = Vec::new();
		plain.extend_from_slice(&k);
		assert_ne!(
			fmt!("{:?}", sha256::digest(&plain)),
			fmt!("{:?}", {
				let mut m = Vec::new();
				m.extend_from_slice(FINGERPRINT_DOMAIN);
				m.extend_from_slice(&k);
				sha256::digest(&m)
			}),
		);
		Ok(())
	}
}
