//! The signing key an author holds, and the file it is kept in.
//!
//! A document's signature binds its address to its author, so an authoring tool needs a key, and a
//! key that changes between runs gives one document a new signature every time it is written. The
//! key is therefore a file, in the same JDAT text form the fixtures' `key.jdat` uses: a map naming
//! the signature scheme and carrying the public and secret keys as raw bytes.
//!
//! A key file may carry the pair directly, or hold several named pairs, as the fixture key does with
//! its `author` and its `impostor`. [`load`] takes the name of the entry to read, and reads the pair
//! at the top level when given none.

use crate::text;

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_crypto::sign::SignatureScheme;
use oxedyne_fe2o3_iop_crypto::{
	keys::KeyManager,
	sign::Signer,
};
use oxedyne_fe2o3_jdat::prelude::*;

use std::{
	fs,
	path::Path,
};

/// The name the key file gives the v0 signature scheme.
pub const SCHEME_ED25519: &'static str = "ed25519";

/// The key-file key naming the signature scheme.
pub const KEY_SCHEME:	&'static str = "scheme";
/// The key-file key carrying the public key.
pub const KEY_PK:	&'static str = "pk";
/// The key-file key carrying the secret key.
pub const KEY_SK:	&'static str = "sk";

/// An Ed25519 key pair, held as raw bytes.
#[derive(Clone, Debug)]
pub struct KeyPair {
	/// The public key, which an envelope names as its author.
	pub pk:	Vec<u8>,
	/// The secret key, which signs.
	pub sk:	Vec<u8>,
}

impl KeyPair {

	/// Generates a fresh Ed25519 key pair.
	pub fn generate() -> Outcome<Self> {
		let signer = SignatureScheme::new_ed25519();
		let pk = match res!(signer.get_public_key()) {
			Some(pk) => pk.to_vec(),
			None => return Err(err!(
				"A fresh Ed25519 signer holds no public key."; Bug, Missing)),
		};
		let sk = match res!(signer.get_secret_key()) {
			Some(sk) => sk.to_vec(),
			None => return Err(err!(
				"A fresh Ed25519 signer holds no secret key."; Bug, Missing)),
		};
		Ok(Self {
			pk,
			sk,
		})
	}

	/// A signer holding this pair, which is what `doc::write` signs an envelope with.
	pub fn signer(&self) -> Outcome<SignatureScheme> {
		Ok(res!(SignatureScheme::empty_ed25519().clone_with_keys(Some(&self.pk), Some(&self.sk))))
	}

	/// The pair as a daticle, the `pk` and `sk` of a key file.
	pub fn to_dat(&self) -> Dat {
		let mut map = DaticleMap::new();
		map.insert(dat!(KEY_PK),	Dat::BU8(self.pk.clone()));
		map.insert(dat!(KEY_SK),	Dat::BU8(self.sk.clone()));
		Dat::Map(map)
	}

	/// Reads a pair from the map that carries it, naming the file it came from.
	pub fn from_dat(
		d:	&Dat,
		path:	&Path,
	)
		-> Outcome<Self>
	{
		Ok(Self {
			pk:	res!(bytes(d, KEY_PK, path)),
			sk:	res!(bytes(d, KEY_SK, path)),
		})
	}
}

/// Signs a message with a pair, under the one scheme this version signs with.
///
/// A document's signature is over the envelope's signing input and is [`doc`](crate::doc)'s business.
/// This is for the OTHER things a holder of a key may need to put their name to -- a declaration about
/// the key itself, a statement carried beside a document rather than inside one -- so that they are
/// signed by the same scheme, and checked by the same code, as a document is. A second signing path
/// would be a second place for a scheme mismatch to hide.
pub fn sign(
	pair:	&KeyPair,
	msg:	&[u8],
)
	-> Outcome<Vec<u8>>
{
	let signer = res!(pair.signer());
	Ok(res!(signer.sign(msg)))
}

/// Whether a signature over a message is the one that public key would make.
///
/// The counterpart of [`sign`], and the same check `doc::verify` runs over an envelope: the key's
/// length is held to the scheme's before anything else, because a key of the wrong length is a
/// malformed input rather than a bad signature and the two want different words. A false answer is not
/// an error -- a signature that is not this key's is a fact, and the caller decides what to do about
/// it -- while a key that could not be read at all is.
pub fn verify(
	pk:	&[u8],
	msg:	&[u8],
	sig:	&[u8],
)
	-> Outcome<bool>
{
	if pk.len() != SignatureScheme::ED25519_PK_LEN {
		return Err(err!(
			"An {} public key is {} bytes, and this one is {}.",
			SCHEME_ED25519, SignatureScheme::ED25519_PK_LEN, pk.len();
		Invalid, Input, Mismatch));
	}
	let verifier = res!(SignatureScheme::empty_ed25519().set_public_key(Some(pk)));
	Ok(res!(verifier.verify(msg, sig)))
}

/// Reads a key file, taking the named entry if the file holds several pairs.
pub fn load(
	path:	&Path,
	entry:	Option<&str>,
)
	-> Outcome<KeyPair>
{
	let src = match fs::read_to_string(path) {
		Ok(src) => src,
		Err(e) => return Err(err!(e,
			"Could not read the key file {}.", path.display();
		IO, File)),
	};
	let d = match text::decode_plain(&src) {
		Ok(d) => d,
		Err(e) => return Err(err!(e,
			"The key file {} is not readable JDAT.", path.display();
		Invalid, Input, Decode)),
	};

	// The scheme is named, never assumed: a key file for a scheme this version does not sign with is
	// refused here rather than producing a signature nothing can check.
	let scheme = res!(string(&d, KEY_SCHEME, path));
	if scheme != SCHEME_ED25519 {
		return Err(err!(
			"The key file {} names the signature scheme '{}'; v0 signs with {}.",
			path.display(), scheme, SCHEME_ED25519;
		Invalid, Input, Unimplemented));
	}

	match entry {
		None => KeyPair::from_dat(&d, path),
		Some(name) => {
			let inner = match &d {
				Dat::Map(map) => match map.get(&dat!(name)) {
					Some(inner) => inner.clone(),
					None => return Err(err!(
						"The key file {} carries no entry '{}'. It holds: {}.",
						path.display(), name, entries(&d);
					Invalid, Input, Missing)),
				},
				d => return Err(err!(
					"The key file {} is a {:?}; a key file is a map.", path.display(), d.kind();
				Invalid, Input)),
			};
			KeyPair::from_dat(&inner, path)
		},
	}
}

/// Writes a key file, readable only by its owner where the platform says so.
pub fn save(
	pair:	&KeyPair,
	path:	&Path,
)
	-> Outcome<()>
{
	let mut map = DaticleMap::new();
	map.insert(dat!(KEY_SCHEME),	Dat::Str(SCHEME_ED25519.to_string()));
	map.insert(dat!(KEY_PK),	Dat::BU8(pair.pk.clone()));
	map.insert(dat!(KEY_SK),	Dat::BU8(pair.sk.clone()));
	let src = res!(text::encode_plain(&Dat::Map(map)));
	if let Some(dir) = path.parent() {
		if !dir.as_os_str().is_empty() {
			match fs::create_dir_all(dir) {
				Ok(()) => (),
				Err(e) => return Err(err!(e,
					"Could not make the directory {} for the key file.", dir.display();
				IO, File)),
			}
		}
	}
	match fs::write(path, &src) {
		Ok(()) => (),
		Err(e) => return Err(err!(e,
			"Could not write the key file {}.", path.display();
		IO, File)),
	}
	res!(restrict(path));
	Ok(())
}

/// Restricts a key file to its owner. A secret key readable by the machine is not a secret key.
#[cfg(unix)]
fn restrict(path: &Path) -> Outcome<()> {
	use std::os::unix::fs::PermissionsExt;
	match fs::set_permissions(path, fs::Permissions::from_mode(0o600)) {
		Ok(()) => Ok(()),
		Err(e) => Err(err!(e,
			"Could not restrict the key file {} to its owner.", path.display();
		IO, File)),
	}
}

/// Restricts a key file to its owner, where the platform offers no way to say so.
#[cfg(not(unix))]
fn restrict(_path: &Path) -> Outcome<()> {
	Ok(())
}

/// The keys a key file carries, listed for an error message that must name what it did not find.
fn entries(d: &Dat) -> String {
	let map = match d {
		Dat::Map(map) => map,
		_ => return fmt!("nothing"),
	};
	let mut s = String::new();
	for (k, _) in map {
		if let Dat::Str(name) = k {
			if !s.is_empty() {
				s.push_str(", ");
			}
			s.push_str(&fmt!("'{}'", name));
		}
	}
	if s.is_empty() {
		s.push_str("nothing");
	}
	s
}

/// The string under a key of a key file.
fn string(
	d:	&Dat,
	key:	&str,
	path:	&Path,
)
	-> Outcome<String>
{
	match res!(get(d, key, path)) {
		Dat::Str(s) => Ok(s.clone()),
		v => Err(err!(
			"The key file {} carries a {:?} under '{}', where a str belongs.",
			path.display(), v.kind(), key;
		Invalid, Input, Mismatch)),
	}
}

/// The raw bytes under a key of a key file.
fn bytes(
	d:	&Dat,
	key:	&str,
	path:	&Path,
)
	-> Outcome<Vec<u8>>
{
	match res!(get(d, key, path)) {
		Dat::BU8(v) => Ok(v.clone()),
		v => Err(err!(
			"The key file {} carries a {:?} under '{}', where a bu8 of raw key bytes belongs.",
			path.display(), v.kind(), key;
		Invalid, Input, Mismatch)),
	}
}

/// The value under a key of a key file, or an error naming the file and the key.
fn get<'a>(
	d:	&'a Dat,
	key:	&str,
	path:	&Path,
)
	-> Outcome<&'a Dat>
{
	match d {
		Dat::Map(map) => match map.get(&dat!(key)) {
			Some(v) => Ok(v),
			None => Err(err!(
				"The key file {} is missing the required key '{}'.", path.display(), key;
			Invalid, Input, Missing)),
		},
		d => Err(err!(
			"The key file {} holds a {:?}; a key file is a map carrying '{}', '{}' and '{}'.",
			path.display(), d.kind(), KEY_SCHEME, KEY_PK, KEY_SK;
		Invalid, Input)),
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	/// A key survives a trip through a file, and the pair that comes back signs as the one that went
	/// in.
	#[test]
	fn test_key_file_round_trip_00() -> Outcome<()> {
		let dir = std::env::temp_dir().join(fmt!("sbj_key_{}", std::process::id()));
		let path = dir.join("key.jdat");
		let pair = res!(KeyPair::generate());
		res!(save(&pair, &path));

		let read = res!(load(&path, None));
		assert_eq!(read.pk, pair.pk, "The public key did not survive the file.");
		assert_eq!(read.sk, pair.sk, "The secret key did not survive the file.");

		// The pair that came back is the pair that signs.
		let signer = res!(read.signer());
		let sig = res!(signer.sign(b"the hash is the address"));
		assert!(res!(signer.verify(b"the hash is the address", &sig)),
			"The key that came back did not sign.");

		match fs::remove_dir_all(&dir) {
			Ok(()) => (),
			Err(e) => return Err(err!(e, "Could not clean up {}.", dir.display(); IO, File)),
		}
		Ok(())
	}

	/// A key file that names a scheme this version does not sign with is refused, naming it.
	#[test]
	fn test_a_foreign_scheme_is_refused_01() -> Outcome<()> {
		let dir = std::env::temp_dir().join(fmt!("sbj_key_foreign_{}", std::process::id()));
		let path = dir.join("key.jdat");
		let pair = res!(KeyPair::generate());
		res!(save(&pair, &path));
		let src = res!(fs::read_to_string(&path), IO, File);
		let bad = src.replace(SCHEME_ED25519, "rsa");
		res!(fs::write(&path, &bad), IO, File);

		match load(&path, None) {
			Ok(_) => return Err(err!("A key file naming RSA was read."; Test, Invalid)),
			Err(e) => {
				let msg = fmt!("{}", e);
				assert!(msg.contains("rsa"), "The refusal should name the scheme: {}", msg);
			},
		}
		match fs::remove_dir_all(&dir) {
			Ok(()) => (),
			Err(e) => return Err(err!(e, "Could not clean up {}.", dir.display(); IO, File)),
		}
		Ok(())
	}
}
