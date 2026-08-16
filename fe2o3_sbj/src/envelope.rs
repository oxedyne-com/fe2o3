//! The SBJ header and envelope. See `SPEC.md` §1.1 to §1.3.
//!
//! The header is eight fixed bytes that say what the file is and how far the envelope reaches. The
//! envelope is a BDAT-encoded map naming the payload schema, the author, the schemes used, the time,
//! the hash of the tree region and the signature over that hash. Nothing here touches content: a
//! caller may read the header, decode the envelope, check the hash and check the signature, and stop.

use crate::{
	HEADER_LEN,
	MAGIC,
	VERSION_MAJOR,
	limit,
};

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_jdat::{
	prelude::*,
	bdat::DecodeLimits,
};

/// The Namex id of the Ed25519 signature scheme, as declared by `SignatureScheme` in `fe2o3_crypto`
/// (base64 `9UQvATp4Zbv8IbWOivdhiQnex+ELo7sxOr8ntEZphMc=`).
pub const NAMEX_ED25519: [u8; 32] = [
	0xF5, 0x44, 0x2F, 0x01, 0x3A, 0x78, 0x65, 0xBB,
	0xFC, 0x21, 0xB5, 0x8E, 0x8A, 0xF7, 0x61, 0x89,
	0x09, 0xDE, 0xC7, 0xE1, 0x0B, 0xA3, 0xBB, 0x31,
	0x3A, 0xBF, 0x27, 0xB4, 0x46, 0x69, 0x84, 0xC7,
];

/// The Namex id of the SHA3-256 hash scheme, as declared by `HashScheme` in `fe2o3_hash`
/// (base64 `VybbHNWeNXeTqTrXj66TzZScbSTsEFVy0W79QnbroFA=`).
pub const NAMEX_SHA3_256: [u8; 32] = [
	0x57, 0x26, 0xDB, 0x1C, 0xD5, 0x9E, 0x35, 0x77,
	0x93, 0xA9, 0x3A, 0xD7, 0x8F, 0xAE, 0x93, 0xCD,
	0x94, 0x9C, 0x6D, 0x24, 0xEC, 0x10, 0x55, 0x72,
	0xD1, 0x6E, 0xFD, 0x42, 0x76, 0xEB, 0xA0, 0x50,
];

/// Derives a v0 scheme id from a Namex id: the leading four bytes, big-endian.
///
/// A Namex id is 32 bytes and the envelope carries a `u32`, so the id on the wire is a prefix of the
/// global name rather than a new numbering of our own. The prefix is stable, since a Namex id never
/// changes, and it is the same value on every machine that reads the byte string the same way.
pub const fn scheme_id(namex: [u8; 32]) -> u32 {
	u32::from_be_bytes([namex[0], namex[1], namex[2], namex[3]])
}

/// The v0 signature scheme id, Ed25519. See `scheme_id`.
pub const SIG_SCHEME_ED25519: u32 = scheme_id(NAMEX_ED25519);

/// The width of an Ed25519 signature, which is what a v0 envelope's `sig` carries.
///
/// Here rather than taken from the signing crate, which publishes its key widths and not this one.
/// The envelope names the scheme, so the envelope knows the width that scheme writes, and a `sig`
/// of any other width is refused with a message about this format before the signing crate is asked
/// to make sense of it.
pub const SIG_LEN_ED25519: usize = 64;

/// The v0 hash scheme id, SHA3-256. See `scheme_id`.
pub const HASH_SCHEME_SHA3_256: u32 = scheme_id(NAMEX_SHA3_256);

/// Envelope key naming the payload schema.
pub const KEY_SCHEMA:	&'static str = "schema";
/// Envelope key carrying the author's public key.
pub const KEY_AUTHOR:	&'static str = "author";
/// Envelope key carrying the signature scheme id.
pub const KEY_SIG_SCHEME:	&'static str = "sig_scheme";
/// Envelope key carrying the hash scheme id.
pub const KEY_HASH_SCHEME:	&'static str = "hash_scheme";
/// Envelope key carrying the authoring time, in Unix milliseconds.
pub const KEY_TIME:	&'static str = "time";
/// Envelope key carrying the hash of the tree region.
pub const KEY_HASH:	&'static str = "hash";
/// Envelope key carrying the signature over the signing input.
pub const KEY_SIG:	&'static str = "sig";
/// Envelope key carrying the length of the tree region, in bytes.
pub const KEY_TREE_LEN:	&'static str = "tree_len";

/// The daticle nesting depth an envelope reaches: the map itself, then the scalar under each key.
///
/// The envelope is the first thing a reader decodes, and every byte of it came from somewhere else,
/// so it is decoded under a limit rather than on trust. Nothing in it nests, so the limit is two.
pub const ENVELOPE_DAT_DEPTH: usize = 2;

/// Every key the envelope map must carry, and no others.
pub const KEYS: [&'static str; 8] = [
	KEY_SCHEMA,
	KEY_AUTHOR,
	KEY_SIG_SCHEME,
	KEY_HASH_SCHEME,
	KEY_TIME,
	KEY_HASH,
	KEY_SIG,
	KEY_TREE_LEN,
];

/// The fixed 8-byte header: magic, major version, envelope length.
#[derive(Clone, Copy, Debug)]
pub struct Header {
	/// Format major version.
	pub major:	u16,
	/// Length of the envelope region, in bytes.
	pub env_len:	u16,
}

/// Reads and checks the fixed header.
pub fn read_header(buf: &[u8]) -> Outcome<Header> {
	if buf.len() < HEADER_LEN {
		return Err(err!(
			"An SBJ header is {} bytes, but only {} {} available.",
			HEADER_LEN, buf.len(), if buf.len() == 1 { "is" } else { "are" };
		Invalid, Input, Decode));
	}
	if buf[0..4] != MAGIC {
		return Err(err!(
			"Not an SBJ file: expected the magic {}, found {}.",
			fmt_magic(&MAGIC), fmt_magic(&buf[0..4]);
		Invalid, Input, Decode));
	}
	let major = u16::from_be_bytes([buf[4], buf[5]]);
	if major != VERSION_MAJOR {
		return Err(err!(
			"SBJ format major version {} is not implemented here, which reads version {}.",
			major, VERSION_MAJOR;
		Invalid, Input, Unimplemented));
	}
	let env_len = u16::from_be_bytes([buf[6], buf[7]]);
	if env_len == 0 {
		return Err(err!(
			"The header declares an envelope of zero bytes, which cannot hold an envelope map.";
		Invalid, Input, Decode));
	}
	if (env_len as usize) > limit::ENVELOPE_BYTES {
		return Err(err!(
			"The header declares an envelope of {} bytes, exceeding the limit of {} bytes.",
			env_len, limit::ENVELOPE_BYTES;
		Invalid, Input, LimitReached));
	}
	Ok(Header {
		major,
		env_len,
	})
}

/// Writes the fixed header for an envelope of the given length.
pub fn write_header(env_len: usize) -> Outcome<Vec<u8>> {
	if env_len == 0 {
		return Err(err!(
			"An envelope of zero bytes cannot hold an envelope map.";
		Invalid, Input, Encode));
	}
	if env_len > limit::ENVELOPE_BYTES {
		return Err(err!(
			"The envelope is {} bytes, exceeding the limit of {} bytes.",
			env_len, limit::ENVELOPE_BYTES;
		Invalid, Input, LimitReached));
	}
	let env_len = env_len as u16; // Safe: the limit is well below `u16::MAX`.
	let mut buf = Vec::with_capacity(HEADER_LEN);
	buf.extend_from_slice(&MAGIC);
	buf.extend_from_slice(&VERSION_MAJOR.to_be_bytes());
	buf.extend_from_slice(&env_len.to_be_bytes());
	Ok(buf)
}

/// Renders four magic bytes as hexadecimal, for an error message.
fn fmt_magic(byts: &[u8]) -> String {
	let mut s = String::new();
	for (i, b) in byts.iter().enumerate() {
		if i > 0 {
			s.push(' ');
		}
		s.push_str(&fmt!("{:02X}", b));
	}
	s
}

/// The signed envelope. Every field is required.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Envelope {
	/// Schema of the payload, e.g. `oxeweb/doc/0`.
	pub schema:	String,
	/// The author's public key.
	pub author:	Vec<u8>,
	/// Namex id of the signature scheme.
	pub sig_scheme:	u32,
	/// Namex id of the hash scheme.
	pub hash_scheme:	u32,
	/// Unix milliseconds.
	pub time:	u64,
	/// Hash of the tree region.
	pub hash:	Vec<u8>,
	/// Signature over the signing input.
	pub sig:	Vec<u8>,
	/// Length of the tree region, in bytes.
	pub tree_len:	u64,
}

impl Envelope {

	/// Encodes the envelope as a canonical daticle map.
	pub fn to_dat(&self) -> Outcome<Dat> {
		let mut map = DaticleMap::new();
		map.insert(dat!(KEY_SCHEMA),	Dat::Str(self.schema.clone()));
		map.insert(dat!(KEY_AUTHOR),	Dat::BU8(self.author.clone()));
		map.insert(dat!(KEY_SIG_SCHEME),	Dat::U32(self.sig_scheme));
		map.insert(dat!(KEY_HASH_SCHEME),	Dat::U32(self.hash_scheme));
		map.insert(dat!(KEY_TIME),	Dat::U64(self.time));
		map.insert(dat!(KEY_HASH),	Dat::BU8(self.hash.clone()));
		map.insert(dat!(KEY_SIG),	Dat::BU8(self.sig.clone()));
		map.insert(dat!(KEY_TREE_LEN),	Dat::C64(self.tree_len));
		Ok(Dat::Map(map))
	}

	/// Decodes an envelope from a daticle map, checking every required key.
	pub fn from_dat(d: &Dat) -> Outcome<Self> {
		let map = match d {
			Dat::Map(map) => map,
			_ => return Err(err!(
				"The envelope must be a {:?}, found a {:?}.", Kind::Map, d.kind();
			Invalid, Input, Mismatch)),
		};
		for key in map.keys() {
			let name = match key {
				Dat::Str(s) => s.clone(),
				_ => return Err(err!(
					"Envelope map keys must be of kind {:?}, found a {:?}.",
					Kind::Str, key.kind();
				Invalid, Input, Mismatch)),
			};
			if !KEYS.contains(&name.as_str()) {
				return Err(err!(
					"The envelope carries the unknown key \"{}\". The v0 envelope carries \
					exactly these keys: {:?}.", name, KEYS;
				Invalid, Input, Excessive));
			}
		}
		Ok(Self {
			schema:	res!(get_str(map, KEY_SCHEMA)),
			author:	res!(get_bu8(map, KEY_AUTHOR)),
			sig_scheme:	res!(get_u32(map, KEY_SIG_SCHEME)),
			hash_scheme:	res!(get_u32(map, KEY_HASH_SCHEME)),
			time:	res!(get_u64(map, KEY_TIME)),
			hash:	res!(get_bu8(map, KEY_HASH)),
			sig:	res!(get_bu8(map, KEY_SIG)),
			tree_len:	res!(get_c64(map, KEY_TREE_LEN)),
		})
	}

	/// Encodes the envelope to canonical BDAT bytes.
	pub fn encode(&self) -> Outcome<Vec<u8>> {
		let d = res!(self.to_dat());
		let buf = res!(d.to_bytes(Vec::new()));
		if buf.len() > limit::ENVELOPE_BYTES {
			return Err(err!(
				"The encoded envelope is {} bytes, exceeding the limit of {} bytes.",
				buf.len(), limit::ENVELOPE_BYTES;
			Invalid, Input, LimitReached));
		}
		Ok(buf)
	}

	/// Decodes an envelope from BDAT bytes, which must be consumed exactly.
	///
	/// The bytes are untrusted, so they are decoded under a depth limit: a header claiming a 4 KiB
	/// envelope should not be believed for free, and neither should the bytes it points at, which
	/// could otherwise describe a value nested deeply enough to exhaust the stack of a recursive
	/// decoder before a single key had been looked at.
	pub fn decode(buf: &[u8]) -> Outcome<Self> {
		if buf.len() > limit::ENVELOPE_BYTES {
			return Err(err!(
				"The envelope region is {} bytes, exceeding the limit of {} bytes.",
				buf.len(), limit::ENVELOPE_BYTES;
			Invalid, Input, LimitReached));
		}
		let lims = DecodeLimits::new(ENVELOPE_DAT_DEPTH, limit::ENVELOPE_BYTES);
		let (d, n) = res!(Dat::from_bytes_limited(buf, &lims));
		if n != buf.len() {
			return Err(err!(
				"The envelope map occupies {} of the {} bytes of the envelope region, \
				leaving {} trailing bytes.", n, buf.len(), buf.len() - n;
			Invalid, Input, Decode));
		}
		// SPEC.md §1.2: the envelope obeys the §3 canonical rules, like everything the hash reaches.
		// The envelope is not itself hashed, but it is what the hash and signature are read from, so
		// a second encoding of the same fields must not decode to the same envelope. Re-encoding and
		// comparing byte-for-byte, as the tree path does, refuses a duplicate key that a decoding map
		// would silently collapse, a non-minimal length, an ordmap, or any other non-canonical form.
		let re = res!(d.to_bytes(Vec::new()));
		if re != buf {
			return Err(err!(
				"The envelope is not in canonical form: it re-encodes to {} bytes against the {} \
				bytes supplied, so it carries a duplicate key, a non-minimal encoding, or a \
				non-canonical map. See SPEC.md §1.2 and §3.", re.len(), buf.len();
			Invalid, Input, Decode));
		}
		Self::from_dat(&d)
	}

	/// The bytes a signature covers. See `SPEC.md` §1.3.
	///
	/// The schema and the scheme ids are included so that a signed payload cannot be re-labelled as
	/// a different schema, nor claimed to have been addressed by a weaker hash function.
	///
	/// The schema is preceded by its length, because it is variable-length and it is not the last
	/// field. Without that, `schema` and `hash` are two variable-length fields separated only by
	/// fixed-width ones, and a byte at the boundary can be read as belonging to either: two
	/// envelopes sharing no field value can share a preimage, and so a signature. `hash` needs no
	/// prefix, being last, since its extent is whatever remains.
	pub fn signing_input(&self) -> Vec<u8> {
		let schema = self.schema.as_bytes();
		let mut buf = Vec::with_capacity(
			4 + schema.len() + 4 + 4 + 8 + self.hash.len()
		);
		// Saturating rather than wrapping: a schema longer than u32 cannot occur, since the whole
		// envelope is capped at 4 KiB by `limit::ENVELOPE_BYTES`, but a length that silently wrapped
		// would reintroduce the ambiguity this prefix exists to remove.
		buf.extend_from_slice(&(schema.len() as u64).min(u32::MAX as u64).to_be_bytes()[4..]);
		buf.extend_from_slice(schema);
		buf.extend_from_slice(&self.sig_scheme.to_be_bytes());
		buf.extend_from_slice(&self.hash_scheme.to_be_bytes());
		buf.extend_from_slice(&self.time.to_be_bytes());
		buf.extend_from_slice(&self.hash);
		buf
	}
}

/// Returns the value for a required envelope key, or an error naming the missing key.
fn get<'a>(map: &'a DaticleMap, key: &str) -> Outcome<&'a Dat> {
	match map.get(&dat!(key)) {
		Some(d) => Ok(d),
		None => Err(err!(
			"The envelope is missing the required key \"{}\".", key;
		Invalid, Input, Missing)),
	}
}

/// The error raised when a key carries the wrong kind of daticle.
fn wrong_kind(key: &str, expected: Kind, found: Kind) -> Error<ErrTag> {
	err!(
		"The envelope key \"{}\" must carry a daticle of kind {:?}, found a {:?}.",
		key, expected, found;
	Invalid, Input, Mismatch)
}

/// Reads a required `str` key.
fn get_str(map: &DaticleMap, key: &str) -> Outcome<String> {
	match res!(get(map, key)) {
		Dat::Str(s) => Ok(s.clone()),
		d => Err(wrong_kind(key, Kind::Str, d.kind())),
	}
}

/// Reads a required `bu8` key.
fn get_bu8(map: &DaticleMap, key: &str) -> Outcome<Vec<u8>> {
	match res!(get(map, key)) {
		Dat::BU8(v) => Ok(v.clone()),
		d => Err(wrong_kind(key, Kind::BU8, d.kind())),
	}
}

/// Reads a required `u32` key.
fn get_u32(map: &DaticleMap, key: &str) -> Outcome<u32> {
	match res!(get(map, key)) {
		Dat::U32(n) => Ok(*n),
		d => Err(wrong_kind(key, Kind::U32, d.kind())),
	}
}

/// Reads a required `u64` key.
fn get_u64(map: &DaticleMap, key: &str) -> Outcome<u64> {
	match res!(get(map, key)) {
		Dat::U64(n) => Ok(*n),
		d => Err(wrong_kind(key, Kind::U64, d.kind())),
	}
}

/// Reads a required `c64` key.
fn get_c64(map: &DaticleMap, key: &str) -> Outcome<u64> {
	match res!(get(map, key)) {
		Dat::C64(n) => Ok(*n),
		d => Err(wrong_kind(key, Kind::C64, d.kind())),
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::SCHEMA_DOC;

	/// A plausible envelope, with fixed contents.
	fn sample() -> Envelope {
		Envelope {
			schema:	SCHEMA_DOC.to_string(),
			author:	vec![0xAA; 32],
			sig_scheme:	SIG_SCHEME_ED25519,
			hash_scheme:	HASH_SCHEME_SHA3_256,
			time:	1_752_000_000_000,
			hash:	vec![0xBB; 32],
			sig:	vec![0xCC; 64],
			tree_len:	4096,
		}
	}

	#[test]
	fn test_scheme_ids() -> Outcome<()> {
		// The ids are the leading four bytes of the Namex ids that `fe2o3_crypto` and `fe2o3_hash`
		// declare, read big-endian.
		assert_eq!(SIG_SCHEME_ED25519, 0xF544_2F01);
		assert_eq!(HASH_SCHEME_SHA3_256, 0x5726_DB1C);
		assert_eq!(scheme_id(NAMEX_ED25519), SIG_SCHEME_ED25519);
		assert_eq!(scheme_id(NAMEX_SHA3_256), HASH_SCHEME_SHA3_256);
		Ok(())
	}

	#[test]
	fn test_header_round_trip() -> Outcome<()> {
		let buf = res!(write_header(1234));
		assert_eq!(buf.len(), HEADER_LEN);
		assert_eq!(&buf[0..4], &MAGIC[..]);
		let hdr = res!(read_header(&buf));
		assert_eq!(hdr.major, VERSION_MAJOR);
		assert_eq!(hdr.env_len, 1234);
		// Trailing bytes are ignored: the header is the first eight.
		let mut long = buf.clone();
		long.extend_from_slice(&[0x00; 16]);
		let hdr = res!(read_header(&long));
		assert_eq!(hdr.env_len, 1234);
		Ok(())
	}

	#[test]
	fn test_header_bad_magic() -> Outcome<()> {
		let mut buf = res!(write_header(64));
		buf[1] = b'X';
		assert!(read_header(&buf).is_err());
		Ok(())
	}

	#[test]
	fn test_header_bad_version() -> Outcome<()> {
		let mut buf = res!(write_header(64));
		buf[5] = 1; // Major version 1.
		match read_header(&buf) {
			Ok(hdr) => return Err(err!(
				"Expected a rejection of major version 1, decoded {:?}.", hdr;
			Test, Invalid)),
			Err(e) => {
				let msg = fmt!("{}", e);
				assert!(msg.contains("1"), "Error should name the version found: {}", msg);
			},
		}
		Ok(())
	}

	#[test]
	fn test_header_too_short() -> Outcome<()> {
		let buf = res!(write_header(64));
		assert!(read_header(&buf[0..7]).is_err());
		Ok(())
	}

	#[test]
	fn test_header_envelope_limit() -> Outcome<()> {
		// The writer refuses an over-large envelope.
		assert!(write_header(limit::ENVELOPE_BYTES + 1).is_err());
		assert!(write_header(0).is_err());
		// The reader refuses a header claiming one, without believing it.
		let mut buf = res!(write_header(limit::ENVELOPE_BYTES));
		assert!(read_header(&buf).is_ok());
		let over = (limit::ENVELOPE_BYTES + 1) as u16;
		buf[6] = (over >> 8) as u8;
		buf[7] = (over & 0xFF) as u8;
		assert!(read_header(&buf).is_err());
		Ok(())
	}

	#[test]
	fn test_envelope_round_trip() -> Outcome<()> {
		let env = sample();
		let buf = res!(env.encode());
		let dec = res!(Envelope::decode(&buf));
		assert_eq!(dec, env);
		// And through the daticle alone.
		let d = res!(env.to_dat());
		assert_eq!(res!(Envelope::from_dat(&d)), env);
		// The encoding is deterministic.
		assert_eq!(res!(dec.encode()), buf);
		Ok(())
	}

	#[test]
	fn test_envelope_missing_key() -> Outcome<()> {
		for key in KEYS {
			let mut map = match res!(sample().to_dat()) {
				Dat::Map(map) => map,
				d => return Err(err!(
					"Expected a map, found a {:?}.", d.kind();
				Test, Invalid)),
			};
			map.remove(&dat!(key));
			match Envelope::from_dat(&Dat::Map(map)) {
				Ok(_) => return Err(err!(
					"Expected a rejection of an envelope missing the key \"{}\".", key;
				Test, Invalid)),
				Err(e) => {
					let msg = fmt!("{}", e);
					assert!(msg.contains(key), "Error should name the key \"{}\": {}", key, msg);
				},
			}
		}
		Ok(())
	}

	#[test]
	fn test_envelope_wrong_typed_key() -> Outcome<()> {
		// A `time` promoted to `u128`, and a `tree_len` written as a `u64` rather than a `c64`, are
		// both rejections: the schema fixes the width.
		let cases: [(&str, Dat); 4] = [
			(KEY_TIME,	Dat::U128(1)),
			(KEY_TREE_LEN,	Dat::U64(4096)),
			(KEY_SCHEMA,	Dat::BU8(vec![1, 2, 3])),
			(KEY_SIG_SCHEME,	Dat::U16(1)),
		];
		for (key, val) in cases {
			let mut map = match res!(sample().to_dat()) {
				Dat::Map(map) => map,
				d => return Err(err!(
					"Expected a map, found a {:?}.", d.kind();
				Test, Invalid)),
			};
			map.insert(dat!(key), val.clone());
			match Envelope::from_dat(&Dat::Map(map)) {
				Ok(_) => return Err(err!(
					"Expected a rejection of the key \"{}\" carrying a {:?}.", key, val.kind();
				Test, Invalid)),
				Err(e) => {
					let msg = fmt!("{}", e);
					assert!(msg.contains(key), "Error should name the key \"{}\": {}", key, msg);
				},
			}
		}
		Ok(())
	}

	#[test]
	fn test_envelope_unknown_key() -> Outcome<()> {
		let mut map = match res!(sample().to_dat()) {
			Dat::Map(map) => map,
			d => return Err(err!(
				"Expected a map, found a {:?}.", d.kind();
			Test, Invalid)),
		};
		map.insert(dat!("extra"), dat!(1u8));
		match Envelope::from_dat(&Dat::Map(map)) {
			Ok(_) => Err(err!(
				"Expected a rejection of an envelope carrying an unknown key.";
			Test, Invalid)),
			Err(e) => {
				let msg = fmt!("{}", e);
				assert!(msg.contains("extra"), "Error should name the key: {}", msg);
				Ok(())
			},
		}
	}

	#[test]
	fn test_envelope_not_a_map() -> Outcome<()> {
		assert!(Envelope::from_dat(&dat!("oxeweb/doc/0")).is_err());
		assert!(Envelope::from_dat(&Dat::List(Vec::new())).is_err());
		Ok(())
	}

	#[test]
	fn test_envelope_trailing_bytes() -> Outcome<()> {
		let mut buf = res!(sample().encode());
		buf.push(0x00);
		assert!(Envelope::decode(&buf).is_err());
		Ok(())
	}

	#[test]
	fn test_envelope_nesting_is_bounded() -> Outcome<()> {
		// An envelope region is the first untrusted thing a reader decodes, and a few hundred bytes
		// of it can describe a value nested hundreds deep. The depth limit refuses it as it
		// descends, rather than after it has descended.
		let mut buf = Vec::new();
		for _ in 0..512 {
			let mut lvl = vec![Dat::LIST_CODE];
			lvl = res!(Dat::C64(buf.len() as u64).to_bytes(lvl));
			lvl.extend_from_slice(&buf);
			buf = lvl;
		}
		assert!(buf.len() <= limit::ENVELOPE_BYTES, "The nest outgrew the envelope limit.");
		match Envelope::decode(&buf) {
			Ok(_) => Err(err!(
				"A deeply nested envelope region was accepted.";
			Test, Invalid)),
			Err(e) => {
				let msg = fmt!("{}", e);
				assert!(msg.contains("nesting depth"),
					"The rejection should name the depth limit, but says: {}", msg);
				Ok(())
			},
		}
	}

	#[test]
	fn test_envelope_non_canonical_duplicate_key() -> Outcome<()> {
		// A decoding BTreeMap silently collapses a duplicate key, so hand-built envelope bytes
		// carrying "time" twice would decode to a valid-looking envelope. SPEC.md §1.2 forbids it,
		// and the re-encode-and-compare gate in decode catches it: the bytes decode to eight
		// entries, which re-encode to fewer bytes than were supplied.
		let env = sample();
		let map = match res!(env.to_dat()) {
			Dat::Map(map) => map,
			d => return Err(err!("Expected a map, found a {:?}.", d.kind(); Test, Invalid)),
		};
		let mut inner = Vec::new();
		for (k, v) in &map {
			inner = res!(k.to_bytes(inner));
			inner = res!(v.to_bytes(inner));
		}
		// Append the "time" pair a second time.
		let dup_key = dat!(KEY_TIME);
		let dup_val = match map.get(&dup_key) {
			Some(v) => v.clone(),
			None => return Err(err!("The sample envelope carries a time key."; Test, Bug)),
		};
		inner = res!(dup_key.to_bytes(inner));
		inner = res!(dup_val.to_bytes(inner));
		let mut bytes = vec![Dat::MAP_CODE];
		bytes = res!(Dat::C64(inner.len() as u64).to_bytes(bytes));
		bytes.extend_from_slice(&inner);
		match Envelope::decode(&bytes) {
			Ok(_) => Err(err!(
				"A non-canonical envelope with a duplicate key was accepted."; Test, Invalid)),
			Err(_) => Ok(()),
		}
	}

	#[test]
	fn test_signing_input() -> Outcome<()> {
		let env = sample();
		let input = env.signing_input();
		let schema = SCHEMA_DOC.as_bytes();
		assert_eq!(input.len(), 4 + schema.len() + 4 + 4 + 8 + 32);
		let mut i = 0;
		assert_eq!(&input[i..i + 4], &(schema.len() as u32).to_be_bytes()[..]);
		i += 4;
		assert_eq!(&input[i..i + schema.len()], schema);
		i += schema.len();
		assert_eq!(&input[i..i + 4], &SIG_SCHEME_ED25519.to_be_bytes()[..]);
		i += 4;
		assert_eq!(&input[i..i + 4], &HASH_SCHEME_SHA3_256.to_be_bytes()[..]);
		i += 4;
		assert_eq!(&input[i..i + 8], &env.time.to_be_bytes()[..]);
		i += 8;
		assert_eq!(&input[i..], &env.hash[..]);
		// Re-labelling the schema, or the scheme, changes what was signed.
		let mut other = env.clone();
		other.schema = "oxeweb/cmd/0".to_string();
		assert_ne!(other.signing_input(), input);
		let mut other = env.clone();
		other.hash_scheme = SIG_SCHEME_ED25519;
		assert_ne!(other.signing_input(), input);
		// A schema of a DIFFERENT length, which the two checks above cannot reach: both re-label
		// `oxeweb/doc/0` to a string of the same width, so neither would notice a preimage that
		// could be split two ways. See `test_the_preimage_cannot_be_split_two_ways`.
		let mut other = env.clone();
		other.schema = fmt!("oxeweb/administrative-command/0");
		assert_ne!(other.signing_input(), input);
		Ok(())
	}

	/// Two envelopes agreeing on no field must not share a signing input.
	///
	/// Written from the collision the unprefixed preimage admitted. With `schema` variable-length
	/// and not the last field, the byte at its boundary can be read as the end of the schema or as
	/// the first byte of the fixed-width run after it, and the same shift at the far end is
	/// absorbed by `hash`, which is variable-length too. The two values below produced identical
	/// bytes before the length prefix existed.
	///
	/// Removing the prefix from `signing_input` turns this test red, which is the only reason to
	/// believe it is testing anything.
	#[test]
	fn test_the_preimage_cannot_be_split_two_ways() -> Outcome<()> {
		let a = Envelope {
			schema:	fmt!("a"),
			author:	vec![0xAA; 32],
			sig_scheme:	0x0102_0304,
			hash_scheme:	0x0506_0708,
			time:	0x090A_0B0C_0D0E_0F10,
			hash:	vec![0x11; 32],
			sig:	Vec::new(),
			tree_len:	0,
		};
		let b = Envelope {
			schema:	fmt!("a\u{1}"),		// the same byte, read as schema rather than as scheme
			author:	vec![0xAA; 32],
			sig_scheme:	0x0203_0405,
			hash_scheme:	0x0607_0809,
			time:	0x0A0B_0C0D_0E0F_1011,
			hash:	vec![0x11; 31],
			sig:	Vec::new(),
			tree_len:	0,
		};
		// Not one field in common.
		assert_ne!(a.schema,	b.schema);
		assert_ne!(a.sig_scheme,	b.sig_scheme);
		assert_ne!(a.hash_scheme,	b.hash_scheme);
		assert_ne!(a.time,	b.time);
		assert_ne!(a.hash,	b.hash);
		// The old, unprefixed reading of both is the same 49 bytes. Asserted rather than assumed:
		// if the collision ever stops holding, this test must say so rather than pass by comparing
		// two things that were never confusable in the first place.
		let unprefixed = |e: &Envelope| -> Vec<u8> {
			let mut v = Vec::new();
			v.extend_from_slice(e.schema.as_bytes());
			v.extend_from_slice(&e.sig_scheme.to_be_bytes());
			v.extend_from_slice(&e.hash_scheme.to_be_bytes());
			v.extend_from_slice(&e.time.to_be_bytes());
			v.extend_from_slice(&e.hash);
			v
		};
		assert_eq!(unprefixed(&a), unprefixed(&b),
			"The collision this test is built on no longer holds, so it proves nothing.");
		assert_eq!(unprefixed(&a).len(), 49);
		// With the length in front of the schema, the two readings are different bytes.
		assert_ne!(a.signing_input(), b.signing_input());
		Ok(())
	}
}
