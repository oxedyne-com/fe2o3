//! Reading and writing whole documents: the verification order of `SPEC.md` §2.
//!
//! A document is verified before it is parsed, and content that fails is never parsed at all. The
//! header says what the file is, the envelope says what the tree region should hash to and who
//! vouches for that hash, and only a tree region whose bytes hash to what the author signed is
//! handed to a decoder. Steps 1 to 5 touch no content, so a caller may run them and stop, which is
//! what [`verify_only`] is for.
//!
//! [`write`] is the inverse, and refuses to sign what [`read`] would reject: the tree is validated
//! against its schema, encoded canonically (§3), hashed, and the hash signed, before a byte of it
//! reaches a file.

use crate::{
	canon,
	card::Card,
	envelope::{
		self,
		Envelope,
	},
	index,
	kinds::Schema,
	limit,
	post::Post,
	validate,
	HEADER_LEN,
	SCHEMA_CARD,
	SCHEMA_POST,
};

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_crypto::sign::SignatureScheme;
use oxedyne_fe2o3_hash::hash::HashScheme;
use oxedyne_fe2o3_iop_crypto::{
	keys::KeyManager,
	sign::Signer,
};
use oxedyne_fe2o3_iop_hash::api::Hasher;
use oxedyne_fe2o3_jdat::prelude::*;

/// A verified document: holding one *is* holding a document whose header, envelope, hash, signature,
/// canonical encoding and schema all checked out, because [`read`] is the only way to obtain one.
///
/// The guarantee is the type's, not the caller's. The fields are private and there is no public
/// constructor, so a `Doc` cannot be minted by a cache, a test helper, an application host or a
/// refactor: every one of them must go through [`read`], which verifies. Anything downstream that
/// renders, indexes, stores or attributes a `Doc` may therefore say so without qualification, and
/// nothing here may be weakened without taking that claim away from all of them at once.
///
/// The fields are read-only, for the same reason. Handing out `&mut` to the tree would let a
/// verified envelope be paired with a tree its author never signed, which is the same hole reached
/// by a different door.
///
/// A `Doc` is read from a file, and cannot be made any other way:
///
/// ```no_run
/// use oxedyne_fe2o3_core::prelude::*;
///
/// fn show(bytes: &[u8]) -> Outcome<()> {
///     let doc = res!(oxedyne_fe2o3_sbj::doc::read(bytes)); // The only route.
///     let _tree = doc.tree();
///     let _author = &doc.env().author;
///     Ok(())
/// }
/// ```
///
/// An unverified one cannot be assembled out of its parts (E0451, the fields are private):
///
/// ```compile_fail
/// use oxedyne_fe2o3_jdat::prelude::Dat;
/// use oxedyne_fe2o3_sbj::{doc::Doc, envelope::Envelope};
///
/// fn forge(env: Envelope, tree: Dat) -> Doc {
///     Doc { env, tree }
/// }
/// ```
///
/// Nor can a verified one have its tree swapped for one nobody signed (E0616, the same):
///
/// ```compile_fail
/// use oxedyne_fe2o3_jdat::prelude::Dat;
/// use oxedyne_fe2o3_sbj::doc::Doc;
///
/// fn tamper(doc: &mut Doc, tree: Dat) {
///     doc.tree = tree;
/// }
/// ```
#[derive(Clone, Debug)]
pub struct Doc {
	/// The signed envelope.
	env:	Envelope,
	/// The decoded node tree.
	tree:	Dat,
}

impl Doc {

	/// The envelope the author signed: the schema, the author, the schemes, the time, the address.
	pub fn env(&self) -> &Envelope {
		&self.env
	}

	/// The node tree, which hashes to the address the envelope carries.
	pub fn tree(&self) -> &Dat {
		&self.tree
	}

	/// Takes the tree, for a caller that owns the document and wants only what is in it.
	pub fn into_tree(self) -> Dat {
		self.tree
	}

	/// Takes the document apart, for a caller that owns it and wants both halves.
	///
	/// The parts carry no guarantee once separated, which is why they are only ever handed out to a
	/// caller that already held the whole: an `Envelope` and a `Dat` cannot be made into a `Doc`.
	pub fn into_parts(self) -> (Envelope, Dat) {
		(self.env, self.tree)
	}
}

/// What the payload region of an artefact holds, and the one place a schema name chooses a
/// validator.
///
/// The container carries any schema (§1.2), and the schemas it carries are no longer one shape. An
/// oxeweb document is a tree of typed nodes, so it is validated by walking that tree against the
/// vocabulary its schema admits; a post and a card are flat canonical maps whose whole validity is
/// their own field rules, and the node vocabulary has nothing to say about either. Putting them
/// through [`validate::validate`] would mean teaching a tree walker two schemas with no tree in
/// them, and teaching [`Schema`] — whose stated job is to fix a vocabulary of node kinds and a
/// vocabulary of style properties — two members that have neither.
///
/// An enum instead, so that the dispatch is exhaustive: a sixth schema cannot be added without the
/// compiler naming every place that must learn about it. The variant carries the schema for a tree,
/// where three names share one shape, and fixes it for a post and a card, where the name and the
/// shape are the same fact.
#[derive(Clone, Debug, PartialEq)]
pub enum Payload {
	/// An oxeweb node tree, under whichever of the three `oxeweb/*` schemas admits its vocabulary.
	Tree {
		/// The schema the envelope declares.
		schema:	Schema,
		/// The node tree.
		tree:	Dat,
	},
	/// A `daimond/post/0` message.
	Post(Post),
	/// A `daimond/card/0` identity card.
	Card(Card),
}

impl Payload {

	/// The schema name the envelope declares for this payload.
	pub fn schema(&self) -> &'static str {
		match self {
			Self::Tree { schema, .. }	=> schema.name(),
			Self::Post(_)	=> SCHEMA_POST,
			Self::Card(_)	=> SCHEMA_CARD,
		}
	}

	/// Validates this payload against its own rules, then encodes it canonically.
	///
	/// Nothing this crate would refuse to read is ever given a signature and an address, which for
	/// a tree means the schema walk of §4 and for a record means its own field rules. Each arm
	/// validates before it encodes.
	pub fn encode(&self) -> Outcome<Vec<u8>> {
		let bytes = match self {
			Self::Tree { schema, tree }	=> res!(encode_tree(tree, schema.name())),
			Self::Post(p)	=> res!(p.encode()),
			Self::Card(c)	=> res!(c.encode()),
		};
		// Checked here as well as inside the arms that check it, so that the container's own limit
		// is not something a payload kind added later can be written without meeting.
		if bytes.len() > limit::TREE_BYTES {
			return Err(err!(
				"The {} payload encodes to {} bytes, exceeding the limit of {} bytes (SPEC.md §5).",
				self.schema(), bytes.len(), limit::TREE_BYTES;
			Invalid, Input, TooBig, LimitReached));
		}
		Ok(bytes)
	}

	/// Decodes and validates a payload region under the schema the envelope declared.
	///
	/// The bytes must already have been hashed and the hash found to be the one the author signed,
	/// which is why this is not public: the only caller is [`read_artefact`], and reaching it any
	/// other way would be parsing content nobody vouched for.
	fn decode(
		schema:	&str,
		bytes:	&[u8],
	)
		-> Outcome<Self>
	{
		match schema {
			SCHEMA_POST	=> Ok(Self::Post(res!(Post::decode(bytes)))),
			SCHEMA_CARD	=> Ok(Self::Card(res!(Card::decode(bytes)))),
			other	=> {
				let schema = res!(Schema::from_name(other));
				// `canon::decode` enforces the depth limit as it descends, and checks every rule of
				// §3 that survives a decode, so `canon::check` is not repeated here.
				let tree = res!(canon::decode(bytes));
				res!(validate::validate(&tree, schema.name()));
				Ok(Self::Tree {
					schema,
					tree,
				})
			},
		}
	}
}

/// A verified artefact: holding one *is* holding a file whose header, envelope, hash, signature,
/// canonical encoding and schema all checked out, because [`read_artefact`] is the only way to
/// obtain one.
///
/// The same guarantee [`Doc`] carries, over the whole set of schemas rather than the three that are
/// node trees, and for the same reason: the fields are private, there is no public constructor, and
/// no `&mut` is handed out, so a verified envelope cannot be paired with a payload its author never
/// signed.
#[derive(Clone, Debug)]
pub struct Artefact {
	/// The signed envelope.
	env:	Envelope,
	/// The decoded, validated payload.
	payload:	Payload,
}

impl Artefact {

	/// The envelope the author signed: the schema, the author, the schemes, the time, the address.
	pub fn env(&self) -> &Envelope {
		&self.env
	}

	/// The payload, which hashes to the address the envelope carries.
	pub fn payload(&self) -> &Payload {
		&self.payload
	}

	/// Takes the payload, for a caller that owns the artefact and wants only what is in it.
	pub fn into_payload(self) -> Payload {
		self.payload
	}

	/// Takes the artefact apart, for a caller that owns it and wants both halves.
	///
	/// The parts carry no guarantee once separated, which is why they are only ever handed out to a
	/// caller that already held the whole.
	pub fn into_parts(self) -> (Envelope, Payload) {
		(self.env, self.payload)
	}
}

/// The regions of a file, located by the header and the envelope but not yet trusted.
///
/// The tree region is exactly the `tree_len` bytes the envelope declares. Whatever follows it is
/// the optional index of §1.4, which lies outside the hash, is derived from the tree, and is never
/// trusted, so nothing here reads it.
#[derive(Clone, Copy, Debug)]
struct Regions<'a> {
	/// The tree region, exactly as long as the envelope declares.
	tree:	&'a [u8],
	/// Whatever trails the tree region: the optional index, or nothing.
	rest:	&'a [u8],
}

/// Reads a document: header, envelope, hash, signature, decode, validate, in that order.
///
/// The tree is decoded only once its bytes have been hashed and the hash found to be the one the
/// author signed. Decoding enforces the depth limit of §5 as it descends, and rejects bytes that
/// are not the canonical encoding of the tree they decode to (§3). The decoded tree is then
/// validated against the schema the envelope declares.
pub fn read(buf: &[u8]) -> Outcome<Doc> {
	let (env, tree_bytes) = res!(verify(buf));
	// Steps 6 and 7. `canon::decode` enforces the depth limit during decoding, and checks the tree
	// against every rule of §3 that survives a decode, which is what `canon::check` does, so the
	// check is not repeated here.
	let tree = res!(canon::decode(tree_bytes));
	res!(validate::validate(&tree, &env.schema));
	Ok(Doc {
		env,
		tree,
	})
}

/// Verifies a document without decoding its tree: steps 1 to 5 of §2, which touch no content.
pub fn verify_only(buf: &[u8]) -> Outcome<Envelope> {
	let (env, _) = res!(verify(buf));
	Ok(env)
}

/// Verifies a document and returns its envelope and the tree region the envelope vouches for.
///
/// The bytes returned have been hashed with the scheme the envelope names, found to hash to what
/// the envelope declares, and that declaration found to have been signed by the author. They have
/// not been decoded, and nothing yet knows whether they are a tree at all.
pub fn verify<'a>(buf: &'a [u8]) -> Outcome<(Envelope, &'a [u8])> {

	// Steps 1 to 3.
	let (env, regions) = res!(locate(buf));

	// Step 4: the tree region hashes to what the envelope declares.
	let hash = res!(hash_tree(env.hash_scheme, regions.tree));
	if hash != env.hash {
		return Err(err!(
			"The {} byte tree region hashes to {}, but the envelope declares the hash {}. The \
			hash is the document's address, so a tree that does not hash to it is not this \
			document.", regions.tree.len(), hex(&hash), hex(&env.hash);
		Invalid, Input, Mismatch));
	}

	// Step 5: the author signed that hash. The width is checked first, because a signature of the
	// wrong width is not a failed verification but a malformed field, and the signing crate answers
	// one with an error carrying nothing about this format in it.
	let verifier = res!(verifier(env.sig_scheme, &env.author));
	res!(check_sig_len(env.sig_scheme, env.sig.len()));
	let input = env.signing_input();
	if !res!(verifier.verify(&input, &env.sig)) {
		return Err(err!(
			"The signature in the envelope is not a signature by the author {} over the signing \
			input of this document (SPEC.md §1.3): schema '{}', scheme ids {:#010X} and {:#010X}, \
			time {}, hash {}.",
			hex(&env.author), env.schema, env.sig_scheme, env.hash_scheme, env.time,
			hex(&env.hash);
		Invalid, Input, Security));
	}

	Ok((env, regions.tree))
}

/// Returns the region trailing the tree, which holds the optional index of §1.4, if any.
///
/// The bytes are derived data lying outside the hash, and are not trusted by anything here: what
/// they say is checked against the tree by `index::check` before it is believed.
pub fn index_region<'a>(buf: &'a [u8]) -> Outcome<&'a [u8]> {
	let (_, regions) = res!(locate(buf));
	Ok(regions.rest)
}

/// Reads an artefact of any schema this build carries: header, envelope, hash, signature, then the
/// payload's own decoder and validator.
///
/// [`read`] is this for the three `oxeweb/*` schemas, and returns a [`Doc`] because a caller that
/// asked for a document wants a tree rather than a match. A caller that will take whatever the file
/// turns out to be asks here.
pub fn read_artefact(buf: &[u8]) -> Outcome<Artefact> {
	let (env, payload_bytes) = res!(verify(buf));
	// Steps 6 and 7, dispatched on the schema the author signed. Nothing here runs until the bytes
	// have hashed to the address in the envelope and that address has been found to be signed.
	let payload = res!(Payload::decode(&env.schema, payload_bytes));
	Ok(Artefact {
		env,
		payload,
	})
}

/// Writes an artefact of any schema this build carries: validate, canonical encode, hash, sign,
/// assemble.
///
/// The payload is validated against its own rules first, so that nothing this crate would refuse to
/// read is ever given a signature and an address.
pub fn write_artefact(
	payload:	&Payload,
	signer:	&SignatureScheme,
	time:	u64,
)
	-> Outcome<Vec<u8>>
{
	let bytes = res!(payload.encode());
	let env = res!(seal(&bytes, payload.schema(), signer, time));
	assemble(&env, &bytes)
}

/// Writes a document: validate, canonical encode, hash, sign, assemble.
///
/// The tree is validated against the schema first, so that nothing this crate would refuse to read
/// is ever given a signature and an address. The schema must be one of the three that are node
/// trees; a post or a card is written by [`write_artefact`], which takes the payload rather than a
/// tree because neither is one.
pub fn write(
	tree:	&Dat,
	schema:	&str,
	signer:	&SignatureScheme,
	time:	u64,
)
	-> Outcome<Vec<u8>>
{
	// Written out rather than routed through `write_artefact`, which would have to be handed an
	// owned tree: a `Dat` clone recurses as deep as the tree goes, which is the cost `validate`
	// walks by reference to avoid. Each step below is the same function `write_artefact` calls.
	let tree_bytes = res!(encode_tree(tree, schema));
	let env = res!(seal(&tree_bytes, schema, signer, time));
	assemble(&env, &tree_bytes)
}

/// Writes a document, and appends the optional index of §1.4.
///
/// The index lies outside the hash, so the document has the same address, and is the same document,
/// whether it is written with an index or without one.
pub fn write_with_index(
	tree:	&Dat,
	schema:	&str,
	signer:	&SignatureScheme,
	time:	u64,
)
	-> Outcome<Vec<u8>>
{
	let tree_bytes = res!(encode_tree(tree, schema));
	let env = res!(seal(&tree_bytes, schema, signer, time));
	let mut buf = res!(assemble(&env, &tree_bytes));
	buf.extend_from_slice(&res!(index::build(&tree_bytes)));
	Ok(buf)
}

/// Hashes a tree region with the scheme the envelope names.
pub fn hash_tree(
	scheme:	u32,
	bytes:	&[u8],
)
	-> Outcome<Vec<u8>>
{
	let hasher = res!(hasher(scheme));
	Ok(hasher.hash(&[bytes], [0u8; 0]).as_vec())
}

/// Locates the regions of a file: steps 1 to 3 of §2.
///
/// The header is checked, the envelope decoded, and the tree region measured against the bytes
/// available. A tree region shorter than the envelope declares is a rejection rather than a
/// truncation, and one longer than the limit of §5 is refused before a byte of it is read.
fn locate<'a>(buf: &'a [u8]) -> Outcome<(Envelope, Regions<'a>)> {

	// Step 1: the header. Magic, major version, and the envelope length, which is checked against
	// the limit of §5 before it is believed.
	let hdr = res!(envelope::read_header(buf));
	let env_end = HEADER_LEN + hdr.env_len as usize;
	if buf.len() < env_end {
		return Err(err!(
			"The header declares an envelope of {} bytes, which would end at byte {}, but the \
			file is {} bytes.", hdr.env_len, env_end, buf.len();
		Invalid, Input, Decode));
	}

	// Step 2: the envelope, whose every key must be present and correctly typed.
	let env = res!(Envelope::decode(&buf[HEADER_LEN..env_end]));

	// Step 3: the tree region, against the limit and against the bytes there are.
	let tree_len = try_into!(usize, env.tree_len);
	if tree_len > limit::TREE_BYTES {
		return Err(err!(
			"The envelope declares a tree region of {} bytes, exceeding the limit of {} bytes \
			(SPEC.md §5). The limit is enforced before decoding, so an envelope claiming a tree \
			larger than this is never believed.", tree_len, limit::TREE_BYTES;
		Invalid, Input, TooBig, LimitReached));
	}
	let avail = buf.len() - env_end;
	if avail < tree_len {
		return Err(err!(
			"The envelope declares a tree region of {} bytes, but only {} bytes follow the \
			envelope. A tree region shorter than declared is a rejection, not a truncation \
			(SPEC.md §2).", tree_len, avail;
		Invalid, Input, Decode));
	}
	let tree_end = env_end + tree_len;

	Ok((env, Regions {
		tree:	&buf[env_end..tree_end],
		rest:	&buf[tree_end..],
	}))
}

/// Validates a tree against its schema, then encodes it canonically, refusing one too large for §5.
fn encode_tree(
	tree:	&Dat,
	schema:	&str,
)
	-> Outcome<Vec<u8>>
{
	res!(validate::validate(tree, schema));
	let bytes = res!(canon::encode(tree));
	if bytes.len() > limit::TREE_BYTES {
		return Err(err!(
			"The tree encodes to {} bytes, exceeding the limit of {} bytes (SPEC.md §5).",
			bytes.len(), limit::TREE_BYTES;
		Invalid, Input, TooBig, LimitReached));
	}
	Ok(bytes)
}

/// Builds the envelope for an encoded payload, up to but not including the signature.
///
/// The half of sealing that holds no key material, so that a signer living somewhere this code
/// cannot reach — a browser's non-extractable `CryptoKey`, a hardware token — can still produce an
/// artefact. The caller names the author's public key, takes [`Envelope::signing_input`] away,
/// signs it wherever the secret is, puts the signature in `sig`, and hands the envelope to
/// [`assemble`]. The secret never crosses this boundary in either direction.
///
/// The returned envelope carries an empty `sig` and is not yet a sealed envelope. `assemble` will
/// happily write one with an empty signature, and [`read`] will refuse it, which is the correct
/// order: an unsigned artefact is a rejection at step 5 and not a special case anywhere earlier.
pub fn envelope_for(
	payload_bytes:	&[u8],
	schema:	&str,
	author:	&[u8],
	time:	u64,
)
	-> Outcome<Envelope>
{
	// The author key is checked at its width here rather than at verification time, because a key
	// of the wrong width names no signer and the artefact it would produce is unreadable by
	// everybody including its writer.
	if author.len() != SignatureScheme::ED25519_PK_LEN {
		return Err(err!(
			"The v0 envelope names the Ed25519 signature scheme, whose public key is {} bytes, \
			but the author key supplied is {} bytes.",
			SignatureScheme::ED25519_PK_LEN, author.len();
		Invalid, Input, Mismatch));
	}
	let hash_scheme = envelope::HASH_SCHEME_SHA3_256;
	Ok(Envelope {
		schema:	schema.to_string(),
		author:	author.to_vec(),
		sig_scheme:	envelope::SIG_SCHEME_ED25519,
		hash_scheme,
		time,
		hash:	res!(hash_tree(hash_scheme, payload_bytes)),
		sig:	Vec::new(),
		tree_len:	try_into!(u64, payload_bytes.len()),
	})
}

/// Builds the envelope for an encoded payload: hash the bytes, then sign the hash.
///
/// Signing the hash rather than the payload is what binds the artefact's permanent address to its
/// author, and the schema and the scheme ids go into the signing input so that neither can be
/// re-labelled afterwards.
pub fn seal(
	payload_bytes:	&[u8],
	schema:		&str,
	signer:		&SignatureScheme,
	time:		u64,
)
	-> Outcome<Envelope>
{
	// Refused before anything is hashed: a signer whose scheme v0 cannot name in an envelope must
	// not produce bytes at all.
	let sig_scheme = res!(sig_scheme_id(signer));
	let author = match res!(signer.get_public_key()) {
		Some(pk) => pk.to_vec(),
		None => return Err(err!(
			"The signer holds no public key, so there is no author to name in the envelope.";
		Missing, Configuration)),
	};
	let mut env = res!(envelope_for(payload_bytes, schema, &author, time));
	env.sig_scheme = sig_scheme;
	env.sig = res!(signer.sign(&env.signing_input()));
	Ok(env)
}

/// Assembles a file: header, envelope, payload region.
///
/// The envelope's signature is written as it stands and is not checked here, since this is the
/// half of writing that a caller signing elsewhere reaches after [`envelope_for`]. What makes an
/// artefact sound is that [`read`] accepts it, and nothing else.
pub fn assemble(
	env:		&Envelope,
	tree_bytes:	&[u8],
)
	-> Outcome<Vec<u8>>
{
	let env_bytes = res!(env.encode());
	let mut buf = res!(envelope::write_header(env_bytes.len()));
	buf.reserve(env_bytes.len() + tree_bytes.len());
	buf.extend_from_slice(&env_bytes);
	buf.extend_from_slice(tree_bytes);
	Ok(buf)
}

/// The scheme id a signer signs under, refusing one v0 cannot name in an envelope.
fn sig_scheme_id(signer: &SignatureScheme) -> Outcome<u32> {
	match signer {
		SignatureScheme::Ed25519(..) => Ok(envelope::SIG_SCHEME_ED25519),
		other => Err(err!(
			"The v0 envelope names the signature scheme Ed25519 only, but the signer is a {:?}.",
			other;
		Invalid, Input, Unimplemented)),
	}
}

/// The hash scheme a scheme id names, refusing an id this version does not implement.
///
/// A signature scheme may be replaced freely, since a signature is checked once and discarded, but
/// a hash scheme may not, because the hash is the address.
fn hasher(scheme: u32) -> Outcome<HashScheme> {
	match scheme {
		envelope::HASH_SCHEME_SHA3_256 => Ok(HashScheme::new_sha3_256()),
		_ => Err(err!(
			"The envelope names the hash scheme {:#010X}, which this version does not implement. \
			v0 hashes with SHA3-256, whose scheme id is {:#010X}.",
			scheme, envelope::HASH_SCHEME_SHA3_256;
		Invalid, Input, Unimplemented)),
	}
}

/// Checks a signature's width against the scheme that wrote it.
///
/// An unsigned envelope is the case that matters: `envelope_for` builds one with an empty `sig` so
/// that a caller signing elsewhere has something to fill in, and an artefact assembled around one
/// before the signature arrives must be refused here, saying so, rather than deep inside a
/// signature library that has never heard of this format.
fn check_sig_len(
	scheme:	u32,
	len:	usize,
)
	-> Outcome<()>
{
	let want = match scheme {
		envelope::SIG_SCHEME_ED25519 => envelope::SIG_LEN_ED25519,
		// Any other scheme id was already refused by `verifier`, which runs first.
		_ => return Ok(()),
	};
	if len != want {
		return Err(err!(
			"The envelope carries a signature of {} bytes, but the scheme it names writes \
			signatures of {} bytes. {}",
			len, want,
			if len == 0 {
				"The signature is empty, so this artefact was assembled before it was signed."
			} else {
				"A signature of the wrong width is a malformed field, not a failed check."
			};
		Invalid, Input, Mismatch));
	}
	Ok(())
}

/// The verifier for a scheme id and an author's public key, refusing an id v0 does not implement.
fn verifier(
	scheme:	u32,
	author:	&[u8],
)
	-> Outcome<SignatureScheme>
{
	match scheme {
		envelope::SIG_SCHEME_ED25519 => {
			if author.len() != SignatureScheme::ED25519_PK_LEN {
				return Err(err!(
					"The envelope names the Ed25519 signature scheme, whose public key is {} \
					bytes, but the author key it carries is {} bytes.",
					SignatureScheme::ED25519_PK_LEN, author.len();
				Invalid, Input, Mismatch));
			}
			Ok(res!(SignatureScheme::empty_ed25519().set_public_key(Some(author))))
		},
		_ => Err(err!(
			"The envelope names the signature scheme {:#010X}, which this version does not \
			implement. v0 signs with Ed25519, whose scheme id is {:#010X}.",
			scheme, envelope::SIG_SCHEME_ED25519;
		Invalid, Input, Unimplemented)),
	}
}

/// Renders bytes as hexadecimal, for an error message that must name what it rejected.
fn hex(byts: &[u8]) -> String {
	let mut s = String::new();
	for b in byts {
		s.push_str(&fmt!("{:02x}", b));
	}
	s
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::{
		kinds::NodeKind,
		MAGIC,
		SCHEMA_DOC,
	};

	use oxedyne_fe2o3_jdat::usr::UsrKindId;

	/// A fixed authoring time, so that a document written twice is written the same.
	const TIME: u64 = 1_752_000_000_000;

	/// Wraps a payload as a node of the given kind.
	fn node(kind: NodeKind, payload: Dat) -> Dat {
		Dat::Usr(
			UsrKindId::new(kind.code(), Some(kind.label()), None),
			Some(Box::new(payload)),
		)
	}

	/// A text run, whose payload is a bare string.
	fn text(s: &str) -> Dat {
		node(NodeKind::Text, dat!(s))
	}

	/// A small but complete document: a heading, a paragraph, and emphasis inside it.
	fn sample_tree() -> Dat {
		node(NodeKind::Doc, mapdat!{
			"title" => dat!("Style without a cascade"),
			"lang" => dat!("en"),
			"children" => Dat::List(vec![
				node(NodeKind::Heading, mapdat!{
					"level" => dat!(2u8),
					"children" => Dat::List(vec![text("Style without a cascade")]),
				}),
				node(NodeKind::Para, mapdat!{
					"children" => Dat::List(vec![
						text("A paragraph, and some "),
						node(NodeKind::Emph, mapdat!{
							"strong" => dat!(true),
							"children" => Dat::List(vec![text("emphasis")]),
						}),
					]),
				}),
			]),
		})
	}

	/// A file carrying the sample document, signed by a fresh key.
	fn sample_file() -> Outcome<(SignatureScheme, Vec<u8>)> {
		let signer = SignatureScheme::new_ed25519();
		let buf = res!(write(&sample_tree(), SCHEMA_DOC, &signer, TIME));
		Ok((signer, buf))
	}

	/// The offset at which the tree region of a file starts.
	fn tree_start(buf: &[u8]) -> Outcome<usize> {
		let hdr = res!(envelope::read_header(buf));
		Ok(HEADER_LEN + hdr.env_len as usize)
	}

	/// Asserts that reading fails, and that the message says why.
	fn rejects(buf: &[u8], what: &str, says: &str) -> Outcome<()> {
		match read(buf) {
			Ok(_) => Err(err!(
				"Expected a rejection of {}, but the document was read.", what;
			Test, Invalid)),
			Err(e) => {
				let msg = fmt!("{}", e);
				assert!(msg.contains(says),
					"The rejection of {} should say '{}', but says: {}", what, says, msg);
				Ok(())
			},
		}
	}

	#[test]
	fn test_signed_round_trip_00() -> Outcome<()> {
		let (signer, buf) = res!(sample_file());
		let doc = res!(read(&buf));
		assert_eq!(doc.tree, sample_tree(), "The tree did not survive a signed round trip.");
		assert_eq!(doc.env.schema, SCHEMA_DOC);
		assert_eq!(doc.env.time, TIME);
		assert_eq!(doc.env.sig_scheme, envelope::SIG_SCHEME_ED25519);
		assert_eq!(doc.env.hash_scheme, envelope::HASH_SCHEME_SHA3_256);
		match res!(signer.get_public_key()) {
			Some(pk) => assert_eq!(&doc.env.author[..], pk, "The author is not the signer."),
			None => return Err(err!("The signer holds no public key."; Test, Invalid)),
		}
		// The envelope's hash is the hash of the tree region, and the region is what it declares.
		let start = res!(tree_start(&buf));
		let tree_bytes = &buf[start..];
		assert_eq!(doc.env.tree_len as usize, tree_bytes.len());
		assert_eq!(doc.env.hash, res!(hash_tree(doc.env.hash_scheme, tree_bytes)));
		// Writing the same tree twice writes the same bytes: one document, one address.
		let again = res!(write(&sample_tree(), SCHEMA_DOC, &signer, TIME));
		assert_eq!(again, buf, "A document written twice is not the same document.");
		Ok(())
	}

	#[test]
	fn test_verify_only_never_decodes_01() -> Outcome<()> {
		let (signer, buf) = res!(sample_file());
		let env = res!(verify_only(&buf));
		assert_eq!(env, res!(read(&buf)).env);

		// A tree region that is signed but is not a tree at all passes every step that touches no
		// content, and fails the moment one does. This is the whole point of the ordering.
		let rubbish = vec![0xFF; 64];
		let env = res!(seal(&rubbish, SCHEMA_DOC, &signer, TIME));
		let buf = res!(assemble(&env, &rubbish));
		res!(verify_only(&buf));
		assert!(read(&buf).is_err(), "Rubbish under a good signature was decoded as a tree.");
		Ok(())
	}

	#[test]
	fn test_corrupt_tree_byte_02() -> Outcome<()> {
		let (_, buf) = res!(sample_file());
		let start = res!(tree_start(&buf));
		// Every byte of the tree region, flipped one at a time, is caught by the hash.
		for i in start..buf.len() {
			let mut bad = buf.clone();
			bad[i] ^= 0x01;
			res!(rejects(&bad, "a corrupted tree byte", "hashes to"));
			assert!(verify_only(&bad).is_err(), "A corrupted tree byte survived verification.");
		}
		Ok(())
	}

	#[test]
	fn test_corrupt_signature_03() -> Outcome<()> {
		let (signer, buf) = res!(sample_file());
		let start = res!(tree_start(&buf));
		let tree_bytes = buf[start..].to_vec();
		let mut env = res!(seal(&tree_bytes, SCHEMA_DOC, &signer, TIME));
		env.sig[0] ^= 0x01;
		let bad = res!(assemble(&env, &tree_bytes));
		res!(rejects(&bad, "a corrupted signature", "not a signature by the author"));
		Ok(())
	}

	#[test]
	fn test_signature_by_another_key_04() -> Outcome<()> {
		let (signer, buf) = res!(sample_file());
		let start = res!(tree_start(&buf));
		let tree_bytes = buf[start..].to_vec();
		// The document is signed correctly, but by a key that is not the author it names.
		let other = SignatureScheme::new_ed25519();
		let mut env = res!(seal(&tree_bytes, SCHEMA_DOC, &other, TIME));
		env.author = match res!(signer.get_public_key()) {
			Some(pk) => pk.to_vec(),
			None => return Err(err!("The signer holds no public key."; Test, Invalid)),
		};
		let bad = res!(assemble(&env, &tree_bytes));
		res!(rejects(&bad, "a signature by another key", "not a signature by the author"));
		// And the reverse: the right signature, an author who did not make it.
		let mut env = res!(seal(&tree_bytes, SCHEMA_DOC, &signer, TIME));
		env.author = match res!(other.get_public_key()) {
			Some(pk) => pk.to_vec(),
			None => return Err(err!("The signer holds no public key."; Test, Invalid)),
		};
		let bad = res!(assemble(&env, &tree_bytes));
		res!(rejects(&bad, "an author who did not sign", "not a signature by the author"));
		Ok(())
	}

	#[test]
	fn test_truncated_tree_05() -> Outcome<()> {
		let (_, buf) = res!(sample_file());
		// One byte short of the tree region the envelope declares.
		let short = &buf[..buf.len() - 1];
		res!(rejects(short, "a truncated tree", "shorter than declared"));
		// And truncated to nothing at all.
		let start = res!(tree_start(&buf));
		res!(rejects(&buf[..start], "an absent tree", "shorter than declared"));
		// A file truncated inside its envelope, and inside its header.
		res!(rejects(&buf[..start - 1], "a truncated envelope", "envelope"));
		assert!(read(&buf[..4]).is_err(), "A file shorter than its header was read.");
		Ok(())
	}

	#[test]
	fn test_tree_longer_than_tree_len_06() -> Outcome<()> {
		let (signer, buf) = res!(sample_file());
		let start = res!(tree_start(&buf));
		let tree_bytes = buf[start..].to_vec();
		let short = tree_bytes.len() - 1;

		// The strongest form: the tree is one byte longer than `tree_len`, and the author has
		// hashed and signed the shortened region, so the hash and the signature both pass. The
		// declared region holds a tree cut off one byte from its end, and the decoder says so.
		let mut env = res!(seal(&tree_bytes[..short], SCHEMA_DOC, &signer, TIME));
		env.tree_len = short as u64;
		let bad = res!(assemble(&env, &tree_bytes));
		res!(verify_only(&bad)); // Steps 1 to 5 pass: the bytes are what the author signed.
		assert!(read(&bad).is_err(), "A tree longer than tree_len was decoded.");

		// The plain form: `tree_len` understates the tree by one byte, and the hash covers the tree
		// the author meant. The hash of the declared region is not the hash the envelope carries.
		let mut env = res!(seal(&tree_bytes, SCHEMA_DOC, &signer, TIME));
		env.tree_len = short as u64;
		env.sig = res!(signer.sign(&env.signing_input()));
		let bad = res!(assemble(&env, &tree_bytes));
		res!(rejects(&bad, "a tree longer than tree_len", "hashes to"));

		// A byte appended to the tree region without the envelope being told is a trailing byte,
		// which is where an index would sit. It is outside the hash, so the document reads, and it
		// is the same document at the same address.
		let mut padded = buf.clone();
		padded.push(0x00);
		let doc = res!(read(&padded));
		assert_eq!(doc.env, res!(read(&buf)).env, "A trailing byte changed the document.");
		assert_eq!(res!(index_region(&padded)), &[0x00][..]);
		Ok(())
	}

	#[test]
	fn test_tree_shorter_than_tree_len_07() -> Outcome<()> {
		let (signer, buf) = res!(sample_file());
		let start = res!(tree_start(&buf));
		let mut tree_bytes = buf[start..].to_vec();
		// The tree region carries the tree and one byte more, all of it hashed and signed. The
		// canonical encoding of a tree is the tree and nothing else (§3).
		tree_bytes.push(0x00);
		let env = res!(seal(&tree_bytes, SCHEMA_DOC, &signer, TIME));
		let bad = res!(assemble(&env, &tree_bytes));
		res!(verify_only(&bad));
		res!(rejects(&bad, "a byte trailing the tree inside the tree region", "canonical"));
		Ok(())
	}

	#[test]
	fn test_unknown_magic_08() -> Outcome<()> {
		let (_, buf) = res!(sample_file());
		for i in 0..MAGIC.len() {
			let mut bad = buf.clone();
			bad[i] ^= 0xFF;
			res!(rejects(&bad, "an unknown magic", "Not an SBJ file"));
		}
		// An empty file, and a file that is valid BDAT but not an SBJ file at all.
		assert!(read(&[]).is_err(), "An empty file was read.");
		let bdat = res!(dat!("not a document").to_bytes(Vec::new()));
		assert!(read(&bdat).is_err(), "A bare daticle was read as a document.");
		Ok(())
	}

	#[test]
	fn test_unknown_major_version_09() -> Outcome<()> {
		let (_, buf) = res!(sample_file());
		let mut bad = buf.clone();
		bad[5] = 1; // Major version 1.
		res!(rejects(&bad, "an unknown major version", "not implemented here"));
		let mut bad = buf.clone();
		bad[4] = 0xFF; // Major version 65280 and up.
		res!(rejects(&bad, "an unknown major version", "not implemented here"));
		Ok(())
	}

	#[test]
	fn test_unknown_schemes_10() -> Outcome<()> {
		let (signer, buf) = res!(sample_file());
		let start = res!(tree_start(&buf));
		let tree_bytes = buf[start..].to_vec();

		// A hash scheme this version does not implement is refused before the tree is hashed.
		let mut env = res!(seal(&tree_bytes, SCHEMA_DOC, &signer, TIME));
		env.hash_scheme = 0xDEAD_BEEF;
		env.sig = res!(signer.sign(&env.signing_input()));
		let bad = res!(assemble(&env, &tree_bytes));
		res!(rejects(&bad, "an unknown hash scheme", "does not implement"));

		// And a signature scheme, before the signature is checked.
		let mut env = res!(seal(&tree_bytes, SCHEMA_DOC, &signer, TIME));
		env.sig_scheme = 0xDEAD_BEEF;
		env.sig = res!(signer.sign(&env.signing_input()));
		let bad = res!(assemble(&env, &tree_bytes));
		res!(rejects(&bad, "an unknown signature scheme", "does not implement"));
		Ok(())
	}

	#[test]
	fn test_tree_region_limit_11() -> Outcome<()> {
		let (signer, buf) = res!(sample_file());
		let start = res!(tree_start(&buf));
		let tree_bytes = buf[start..].to_vec();
		// A tree region larger than the limit is refused on the envelope's word alone, without the
		// bytes ever being supplied, let alone read.
		let mut env = res!(seal(&tree_bytes, SCHEMA_DOC, &signer, TIME));
		env.tree_len = (limit::TREE_BYTES + 1) as u64;
		env.sig = res!(signer.sign(&env.signing_input()));
		let bad = res!(assemble(&env, &tree_bytes));
		res!(rejects(&bad, "a tree region over the limit", "exceeding the limit"));
		Ok(())
	}

	#[test]
	fn test_foreign_schema_12() -> Outcome<()> {
		// The container carries any schema, but this build validates one, so a payload declaring
		// another is rejected rather than read as though it were a document.
		let (signer, buf) = res!(sample_file());
		let start = res!(tree_start(&buf));
		let tree_bytes = buf[start..].to_vec();
		let env = res!(seal(&tree_bytes, "oxeweb/cmd/0", &signer, TIME));
		let bad = res!(assemble(&env, &tree_bytes));
		res!(verify_only(&bad)); // The envelope is sound; it is the payload that is foreign.
		res!(rejects(&bad, "a foreign schema", "oxeweb/cmd/0"));
		// And write refuses to sign a tree it cannot validate.
		assert!(write(&sample_tree(), "oxeweb/cmd/0", &signer, TIME).is_err(),
			"A tree was signed under a schema this build cannot validate.");
		Ok(())
	}

	#[test]
	fn test_write_refuses_an_invalid_tree_13() -> Outcome<()> {
		let signer = SignatureScheme::new_ed25519();
		// A heading level outside 1..=6 never reaches a file, so it never gets an address.
		let tree = node(NodeKind::Doc, mapdat!{
			"title" => dat!("T"),
			"lang" => dat!("en"),
			"children" => Dat::List(vec![
				node(NodeKind::Heading, mapdat!{
					"level" => dat!(7u8),
					"children" => Dat::List(vec![text("Too deep")]),
				}),
			]),
		});
		match write(&tree, SCHEMA_DOC, &signer, TIME) {
			Ok(_) => return Err(err!("A heading of level 7 was signed."; Test, Invalid)),
			Err(e) => {
				let msg = fmt!("{}", e);
				assert!(msg.contains("1..=6"), "The refusal should name the range: {}", msg);
			},
		}
		// And a tree that is not canonical: an empty children list has two encodings.
		let tree = node(NodeKind::Doc, mapdat!{
			"title" => dat!("T"),
			"lang" => dat!("en"),
			"children" => Dat::List(vec![
				node(NodeKind::Para, mapdat!{
					"children" => Dat::List(Vec::new()),
				}),
			]),
		});
		assert!(write(&tree, SCHEMA_DOC, &signer, TIME).is_err(),
			"A non-canonical tree was signed.");
		Ok(())
	}

	/// A chain of boxes `depth` nodes deep, the doc at its head and the deepest box childless.
	fn chain(depth: usize) -> Dat {
		let mut inner = node(NodeKind::Boxx, mapdat!{});
		for _ in 0..(depth - 2) {
			inner = node(NodeKind::Boxx, mapdat!{
				"children" => Dat::List(vec![inner]),
			});
		}
		node(NodeKind::Doc, mapdat!{
			"title" => dat!("A deep document"),
			"lang" => dat!("en"),
			"children" => Dat::List(vec![inner]),
		})
	}

	/// A document at the depth limit, and one past it.
	fn nesting_at_the_limit_and_past_it() -> Outcome<()> {
		let signer = SignatureScheme::new_ed25519();

		// At the limit, a document is written and read like any other.
		let buf = res!(write(&chain(limit::DEPTH), SCHEMA_DOC, &signer, TIME));
		let doc = res!(read(&buf));
		assert_eq!(doc.tree, chain(limit::DEPTH), "A tree at the depth limit did not survive.");

		// One past it, nothing is signed, and a file assembled by hand around such a tree is
		// refused. The bytes are what the author signed, so the refusal comes from the reading of
		// the tree, which is where the depth limit belongs: the tree is never validated, because
		// it is never built.
		let deep = chain(limit::DEPTH + 1);
		assert!(write(&deep, SCHEMA_DOC, &signer, TIME).is_err(),
			"A tree past the depth limit was signed.");
		let tree_bytes = res!(deep.to_bytes(Vec::new()));
		let env = res!(seal(&tree_bytes, SCHEMA_DOC, &signer, TIME));
		let bad = res!(assemble(&env, &tree_bytes));
		res!(verify_only(&bad));
		res!(rejects(&bad, "a tree past the depth limit", "depth"));
		Ok(())
	}

	#[test]
	fn test_nesting_at_the_limit_and_past_it_15() -> Outcome<()> {
		// A node costs three daticle levels, so a document at the node depth limit of 256 nests
		// daticles 770 deep, and a recursive decoder spends a frame on each. A release build
		// spends about half a kilobyte a level, so the deepest legal document costs it some 400
		// KiB, well inside the two megabytes a thread is given by default. A debug build spends
		// about eight times that, since it gives every arm of a match its own slot in the frame,
		// so the test runs on a thread with a stack that can hold the deepest document the format
		// permits. The limit is the format's, and it does not move to suit a test.
		let thread = match std::thread::Builder::new()
			.name("sbj_depth".to_string())
			.stack_size(16 * 1024 * 1024)
			.spawn(nesting_at_the_limit_and_past_it)
		{
			Ok(thread) => thread,
			Err(e) => return Err(err!(e,
				"Could not spawn the thread the deepest legal document is read on.";
			Test, Init)),
		};
		match thread.join() {
			Ok(outcome) => outcome,
			Err(_) => Err(err!(
				"The thread reading the deepest legal document did not return.";
			Test, Panic)),
		}
	}

	#[test]
	fn test_a_doc_is_a_verified_document_16() -> Outcome<()> {
		// The claim the type makes is that holding a `Doc` means the document verified. That a `Doc`
		// cannot be constructed any other way is a fact about compilation, not about execution, so it
		// is asserted by the `compile_fail` doctests on `Doc` rather than here: no test that runs can
		// observe code that does not compile. What runs here is the other half of the claim: that
		// what `read` hands back has in fact been verified, checked from the outside, against the
		// document's own bytes rather than against anything the reader remembers.
		let (signer, buf) = res!(sample_file());
		let doc = res!(read(&buf));

		// The tree is the tree the envelope vouches for: it re-encodes to the region that was hashed,
		// and that region hashes to the address the envelope carries.
		let tree_bytes = res!(canon::encode(doc.tree()));
		assert_eq!(doc.env().tree_len as usize, tree_bytes.len(),
			"The tree of a Doc is not the length the envelope declares.");
		assert_eq!(doc.env().hash, res!(hash_tree(doc.env().hash_scheme, &tree_bytes)),
			"The tree of a Doc does not hash to the address of its envelope.");

		// And the author signed that address.
		let verifier = res!(verifier(doc.env().sig_scheme, &doc.env().author));
		assert!(res!(verifier.verify(&doc.env().signing_input(), &doc.env().sig)),
			"The envelope of a Doc carries a signature the author did not make.");
		match res!(signer.get_public_key()) {
			Some(pk) => assert_eq!(&doc.env().author[..], pk, "The author is not the signer."),
			None => return Err(err!("The signer holds no public key."; Test, Invalid)),
		}

		// The accessors hand out no way to undo any of that: `env` and `tree` borrow, and the `into_`
		// pair consumes the document rather than opening it.
		let (env, tree) = res!(read(&buf)).into_parts();
		assert_eq!(env, *doc.env());
		assert_eq!(tree, *doc.tree());
		assert_eq!(res!(read(&buf)).into_tree(), *doc.tree());

		// Every document that fails at any step fails to become a `Doc` at all, so there is no state
		// in which one exists and its verification did not happen.
		let start = res!(tree_start(&buf));
		let mut bad = buf.clone();
		bad[start] ^= 0x01; // A tree byte the author did not sign.
		res!(rejects(&bad, "a tampered tree", "hashes to"));
		let tree_bytes = buf[start..].to_vec();
		let mut env = res!(seal(&tree_bytes, SCHEMA_DOC, &signer, TIME));
		env.sig[0] ^= 0x01; // A signature the author did not make.
		let bad = res!(assemble(&env, &tree_bytes));
		res!(rejects(&bad, "a tampered signature", "not a signature by the author"));
		Ok(())
	}

	#[test]
	fn test_index_is_outside_the_document_14() -> Outcome<()> {
		let signer = SignatureScheme::new_ed25519();
		let plain = res!(write(&sample_tree(), SCHEMA_DOC, &signer, TIME));
		let indexed = res!(write_with_index(&sample_tree(), SCHEMA_DOC, &signer, TIME));
		assert!(indexed.len() > plain.len(), "The index added nothing.");

		// Two files carrying the same tree are the same document at the same address, whether or
		// not either carries an index.
		let a = res!(read(&plain));
		let b = res!(read(&indexed));
		assert_eq!(a.env, b.env, "The index changed the document's envelope.");
		assert_eq!(a.tree, b.tree, "The index changed the document's tree.");
		assert_eq!(a.env.hash, b.env.hash, "The index changed the document's address.");

		// The index that was appended describes the tree it was built from.
		assert_eq!(res!(index_region(&plain)).len(), 0);
		let idx = res!(index::parse(res!(index_region(&indexed))));
		let (_, tree_bytes) = res!(verify(&indexed));
		res!(index::check(tree_bytes, &idx));
		// doc, heading, the heading's text, para, its text, the emph, and the emph's text.
		assert_eq!(idx.len(), 7);
		Ok(())
	}

	/// A post, for the artefact tests.
	fn sample_post() -> Post {
		Post {
			body:	fmt!("The crop is in, and the second field can wait."),
			to:	vec![0xA1; crate::post::limit::KEY_BYTES],
			nonce:	vec![0xB2; crate::post::limit::NONCE_BYTES],
			reply_to:	None,
			refs:	Vec::new(),
		}
	}

	/// A whole file carrying a post can be written and read back.
	///
	/// This is the gap [`Payload`] closes. Before it, [`write`] was the only writer and it routes
	/// every schema through the node-tree validator, which admits the three `oxeweb/*` names alone,
	/// so no path in the crate could produce or read a file carrying a message.
	#[test]
	fn test_a_post_is_a_whole_artefact_17() -> Outcome<()> {
		let signer = SignatureScheme::new_ed25519();
		let post = sample_post();
		let buf = res!(write_artefact(&Payload::Post(post.clone()), &signer, TIME));
		let back = res!(read_artefact(&buf));
		assert_eq!(back.env().schema, SCHEMA_POST);
		assert_eq!(*back.payload(), Payload::Post(post));

		// The verification order is the container's, not the payload's: a payload byte the author
		// did not sign fails at the hash, before a decoder sees it.
		let start = res!(tree_start(&buf));
		let mut bad = buf.clone();
		bad[start] ^= 0x01;
		match read_artefact(&bad) {
			Ok(_) => return Err(err!("A tampered post was read."; Test, Invalid)),
			Err(e) => assert!(fmt!("{}", e).contains("hashes to")),
		}
		Ok(())
	}

	/// A whole file carrying a card can be written and read back.
	#[test]
	fn test_a_card_is_a_whole_artefact_18() -> Outcome<()> {
		let signer = SignatureScheme::new_ed25519();
		let card = Card {
			label:	fmt!("Jason"),
			enc:	vec![0xE1; crate::card::limit::KEY_BYTES],
			role:	crate::card::Role::Root,
			prev:	None,
		};
		let buf = res!(write_artefact(&Payload::Card(card.clone()), &signer, TIME));
		let back = res!(read_artefact(&buf));
		assert_eq!(back.env().schema, SCHEMA_CARD);
		assert_eq!(*back.payload(), Payload::Card(card));
		Ok(())
	}

	/// A payload cannot be re-labelled into another schema after signing.
	///
	/// The attack the length prefix of §1.3 closes, run over the two schemas that made it possible
	/// to attempt: a post and a card are both flat maps, so the bytes of one can be handed to the
	/// other's decoder, and only the envelope says which it is. Re-labelling the envelope is what
	/// the signature refuses.
	#[test]
	fn test_a_payload_cannot_be_relabelled_19() -> Outcome<()> {
		let signer = SignatureScheme::new_ed25519();
		let bytes = res!(sample_post().encode());
		let env = res!(seal(&bytes, SCHEMA_POST, &signer, TIME));
		res!(read_artefact(&res!(assemble(&env, &bytes))));

		// The same bytes, the same hash, the same signature, one word changed in the envelope.
		let mut relabelled = env.clone();
		relabelled.schema = SCHEMA_CARD.to_string();
		match read_artefact(&res!(assemble(&relabelled, &bytes))) {
			Ok(_) => Err(err!(
				"A post was read as a card, so the schema is not inside the signature.";
			Test, Invalid)),
			Err(e) => {
				assert!(fmt!("{}", e).contains("not a signature by the author"));
				Ok(())
			},
		}
	}

	/// A caller may build an envelope, sign it elsewhere, and assemble the artefact, without the
	/// secret key ever reaching this crate.
	///
	/// The seam the browser uses: `envelope_for` holds no key material, `signing_input` says what
	/// to sign, and `assemble` takes the signature back. The signer here stands in for the one that
	/// lives outside.
	#[test]
	fn test_signing_happens_elsewhere_20() -> Outcome<()> {
		let signer = SignatureScheme::new_ed25519();
		let author = match res!(signer.get_public_key()) {
			Some(pk) => pk.to_vec(),
			None => return Err(err!("A fresh signer holds no public key."; Test, Bug)),
		};
		let bytes = res!(sample_post().encode());
		let mut env = res!(envelope_for(&bytes, SCHEMA_POST, &author, TIME));

		// Unsigned, and refused as such rather than by a special case.
		assert!(env.sig.is_empty());
		match read_artefact(&res!(assemble(&env, &bytes))) {
			Ok(_) => return Err(err!("An unsigned artefact was read."; Test, Invalid)),
			Err(e) => assert!(fmt!("{}", e).contains("assembled before it was signed")),
		}

		env.sig = res!(signer.sign(&env.signing_input()));
		let back = res!(read_artefact(&res!(assemble(&env, &bytes))));
		assert_eq!(back.env().schema, SCHEMA_POST);

		// The same artefact the all-in-one path writes, byte for byte.
		assert_eq!(
			res!(assemble(&env, &bytes)),
			res!(write_artefact(&Payload::Post(sample_post()), &signer, TIME)),
		);
		Ok(())
	}

	/// An author key of the wrong width is refused before an artefact is built around it.
	#[test]
	fn test_an_author_key_has_one_width_21() -> Outcome<()> {
		let bytes = res!(sample_post().encode());
		match envelope_for(&bytes, SCHEMA_POST, &[0u8; 31], TIME) {
			Ok(_) => Err(err!("A 31 byte author key was accepted."; Test, Invalid)),
			Err(e) => {
				assert!(fmt!("{}", e).contains("31 bytes"));
				Ok(())
			},
		}
	}
}
