//! Machinery shared by the fixture generator and the conformance suite.
//!
//! The generator (`examples/gen_fixtures.rs`) writes `fixtures/`, and the suite
//! (`tests/conformance.rs`) reads it. Both need the same node builders, the same JDAT text codec,
//! the same fixed key, and the same declaration formats, so both take them from here rather than
//! from each other, and neither takes an expected error message from the implementation it is
//! testing.
//!
//! The container is assembled here out of the crate's public API alone: `Envelope` says what an
//! envelope is, `doc::hash_tree` hashes a region, a `Signer` signs the hash, and `write_header`
//! writes the eight bytes in front. A rejection fixture must therefore carry a sound signature over
//! whatever is wrong with it, so that the rejection can only have come from the rule the fixture
//! breaks.

#![allow(dead_code)] // Each of the two callers uses a part of this.

use oxedyne_fe2o3_sbj::{
	canon,
	doc,
	envelope::{
		self,
		Envelope,
	},
	kinds::{
		NodeKind,
		ReservedKind,
	},
	limit,
	text,
	HEADER_LEN,
	SCHEMA_DOC,
};

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_crypto::sign::SignatureScheme;
use oxedyne_fe2o3_iop_crypto::{
	keys::KeyManager,
	sign::Signer,
};
use oxedyne_fe2o3_jdat::{
	prelude::*,
	string::{
		dec::DecoderConfig,
		enc::EncoderConfig,
	},
	usr::{
		UsrKind,
		UsrKindCode,
		UsrKindId,
		UsrKinds,
	},
};

use std::{
	collections::BTreeMap,
	fs,
	path::{
		Path,
		PathBuf,
	},
};

/// The authoring time every fixture is signed at, so that a fixture written twice is the same file.
pub const TIME: u64 = 1_752_000_000_000;

/// The stack a thread is given before it reads a document at the depth limit.
///
/// This was once 512 MiB, because the JDAT *text* decoder a fixture's `doc.jdat` goes through spent
/// some 158 KB of stack on every level of a debug build, and the deepest legal fixture would not
/// read in less. The decoder has since been given frame-splitting and costs about 2.4 KB a level.
///
/// The arithmetic now: a node costs about five text levels, and `SPEC.md` §5 permits 64, so the
/// deepest legal document needs roughly 320 levels, or some 800 KB. Eight mebibytes leaves a
/// tenfold margin. The limit is the format's, and it does not move to suit a test, so the test
/// moves -- but it no longer has to move nearly so far.
pub const STACK_BYTES: usize = 8 * 1024 * 1024;

/// The file holding the fixed key the fixtures are signed with.
pub const KEY_FILE:	&'static str = "key.jdat";
/// The file explaining the fixture directory to a reader.
pub const README_FILE:	&'static str = "README.md";
/// The document, in JDAT text form: the source of truth for a fixture.
pub const DOC_JDAT:	&'static str = "doc.jdat";
/// The signed binary artefact.
pub const DOC_SBJ:	&'static str = "doc.sbj";
/// The expectations of an acceptance fixture.
pub const META_JDAT:	&'static str = "meta.jdat";
/// The declared failure of a rejection fixture.
pub const REJECT_JDAT:	&'static str = "reject.jdat";

/// Every v0 node kind, in code order.
pub const KINDS: [NodeKind; 13] = [
	NodeKind::Doc,
	NodeKind::Section,
	NodeKind::Para,
	NodeKind::Heading,
	NodeKind::List,
	NodeKind::Item,
	NodeKind::Boxx,
	NodeKind::Image,
	NodeKind::Text,
	NodeKind::Emph,
	NodeKind::Link,
	NodeKind::Code,
	NodeKind::Quote,
];

/// A kind code the v0 vocabulary does not know, for the fixture that carries an unknown node.
pub const ALIEN_CODE: u16 = 99;

/// The label the alien kind carries in the JDAT text form.
pub const ALIEN_LABEL: &'static str = "sbj_alien";

/// The reserved kinds of `SPEC.md` §4.2, which `oxeweb/doc/0` admits nowhere.
///
/// They are the engine's own: an editable text field, and a pane an application paints. A document
/// carrying one is refused, fallback or no fallback, and the fixtures that carry them are what says
/// so.
pub const RESERVED: [ReservedKind; 2] = [ReservedKind::Edit, ReservedKind::Surface];

/// The user kind registry the JDAT text codec needs to read and write node kinds.
pub type Ukinds = UsrKinds<BTreeMap<UsrKindCode, UsrKind>, BTreeMap<String, UsrKindId>>;

/// The label a node kind carries in the JDAT text form of a tree.
///
/// Two of the v0 kind labels, `box` and `list`, are also JDAT's own kind labels, so a node written
/// as `(box|{..})` would read back as a `Dat::Box` and a node written as `(list|[..])` as a
/// `Dat::List`. Every node label is therefore prefixed in the text form. Nothing of this reaches the
/// wire: BDAT carries the `u16` kind code and no label at all, and `UsrKindId` compares by code.
pub fn text_label(kind: NodeKind) -> String {
	fmt!("sbj_{}", kind.label())
}

/// The user kind registry: one entry per v0 node kind, one for the kind that is not one, and one for
/// each kind the document schema reserves.
///
/// The alien kind and the reserved kinds are registered so that the fixtures carrying them can still
/// be read and written as text. The registry is the fixture suite's, not the format's: what SBJ
/// admits in a document is `NodeKind`, and it refuses code 99 for want of a fallback and the codes 14
/// and 15 whatever they carry.
pub fn ukinds() -> Outcome<Ukinds> {
	let mut uks = UsrKinds::new(BTreeMap::new(), BTreeMap::new());
	for kind in KINDS {
		res!(uks.add(ukid(kind)));
	}
	res!(uks.add(UsrKindId::new(ALIEN_CODE, Some(ALIEN_LABEL), Some(Kind::Map))));
	for kind in RESERVED {
		res!(uks.add(reserved_ukid(kind)));
	}
	Ok(uks)
}

/// The user kind id of a reserved kind, under the `sbj_k<code>` label the text form gives a kind the
/// document vocabulary does not admit.
pub fn reserved_ukid(kind: ReservedKind) -> UsrKindId {
	UsrKindId::new(kind.code(), Some(&text::unknown_label(kind.code())), Some(Kind::Map))
}

/// The user kind id of a node kind, declaring the kind of payload it carries.
///
/// The payload kind is declared because the JDAT text decoder reads a user kind that declares one
/// and drops the payload of one that does not. Nothing of it reaches the wire, where BDAT writes the
/// `u16` code and nothing else, and `UsrKindId` compares by code, so a tree built here is the tree a
/// decoder builds.
pub fn ukid(kind: NodeKind) -> UsrKindId {
	let payload = if kind.payload_is_str() {
		Kind::Str
	} else {
		Kind::Map
	};
	UsrKindId::new(kind.code(), Some(&text_label(kind)), Some(payload))
}

/// A node of a kind the v0 vocabulary does not know.
pub fn alien(payload: Dat) -> Dat {
	Dat::Usr(
		UsrKindId::new(ALIEN_CODE, Some(ALIEN_LABEL), Some(Kind::Map)),
		Some(Box::new(payload)),
	)
}

/// A node of a kind the document schema reserves (§4.2), which no document may carry.
pub fn reserved(kind: ReservedKind, payload: Dat) -> Dat {
	Dat::Usr(reserved_ukid(kind), Some(Box::new(payload)))
}

/// Writes a tree in JDAT text form, the form a fixture's `doc.jdat` carries.
///
/// Every kindicle is written out, including the ones JDAT would infer, so that the text says what
/// the bytes say and nothing is left to a reader's guess: a `u8` reads as a `u8`, a list as a list,
/// and a map as a map. It is what §3 asks of the bytes, asked of the text.
pub fn to_jdat(dat: &Dat) -> Outcome<String> {
	let cfg = EncoderConfig::jdat_full_to_lines(Some(res!(ukinds())), "    ");
	let mut s = res!(dat.encode_string_with_config(&cfg));
	s.push('\n');
	Ok(s)
}

/// Reads a tree from JDAT text form.
///
/// The crate's own text codec does the reading, under the limits of `SPEC.md` §5 rather than the
/// decoder's defaults, since a document that nests to the ceiling §5 sets must read and one that
/// nests past it must be refused for that reason. The alien kind is declared, since a label a
/// document invents for a kind outside the vocabulary is declared rather than guessed.
pub fn from_jdat(s: &str) -> Outcome<Dat> {
	text::decode(s, &[
		text::KindDecl {
			label:	ALIEN_LABEL.to_string(),
			code:	ALIEN_CODE,
		},
	])
}

/// Writes a plain daticle, such as a `meta.jdat`, in JDAT text form.
pub fn to_jdat_plain(dat: &Dat) -> Outcome<String> {
	let cfg = EncoderConfig::<
		BTreeMap<UsrKindCode, UsrKind>,
		BTreeMap<String, UsrKindId>,
	>::jdat_to_lines(None, "    ");
	let mut s = res!(dat.encode_string_with_config(&cfg));
	s.push('\n');
	Ok(s)
}

/// Reads a plain daticle, such as a `meta.jdat`.
pub fn from_jdat_plain(s: &str) -> Outcome<Dat> {
	let cfg = DecoderConfig::<
		BTreeMap<UsrKindCode, UsrKind>,
		BTreeMap<String, UsrKindId>,
	>::jdat(None);
	Dat::decode_string_with_config(s, &cfg)
}

/// Wraps a payload as a node of the given kind.
pub fn node(kind: NodeKind, payload: Dat) -> Dat {
	Dat::Usr(ukid(kind), Some(Box::new(payload)))
}

/// A text run, the one node whose payload is a bare string.
pub fn text(s: &str) -> Dat {
	node(NodeKind::Text, Dat::Str(s.to_string()))
}

/// A payload map, from string keys.
pub fn map(kv: Vec<(&str, Dat)>) -> Dat {
	create_dat_map(
		kv.into_iter().map(|(k, v)| (Dat::Str(k.to_string()), v)).collect()
	)
}

/// The fixed key pair a fixture is signed with, and the one it is not signed with.
///
/// Both are committed beside the fixtures, since a freshly generated key would give every artefact
/// a new signature on every run, and a fixture that changes every run is not a fixture.
#[derive(Clone, Debug)]
pub struct Keys {
	/// The author of every fixture.
	pub author:	KeyPair,
	/// A key that is not the author, for the fixture signed by the wrong hand.
	pub impostor:	KeyPair,
}

/// An Ed25519 key pair, held as raw bytes.
#[derive(Clone, Debug)]
pub struct KeyPair {
	/// The public key, which an envelope names as its author.
	pub pk:	Vec<u8>,
	/// The secret key. This is a test key, published on purpose, and signs nothing else.
	pub sk:	Vec<u8>,
}

impl KeyPair {

	/// A signer holding this pair.
	pub fn signer(&self) -> Outcome<SignatureScheme> {
		Ok(res!(SignatureScheme::empty_ed25519().clone_with_keys(Some(&self.pk), Some(&self.sk))))
	}

	/// This pair as a daticle, for the committed key file.
	pub fn to_dat(&self) -> Dat {
		map(vec![
			("pk",	Dat::BU8(self.pk.clone())),
			("sk",	Dat::BU8(self.sk.clone())),
		])
	}

	/// Reads a pair from the committed key file.
	pub fn from_dat(d: &Dat) -> Outcome<Self> {
		Ok(Self {
			pk:	res!(get_bytes(d, "pk")),
			sk:	res!(get_bytes(d, "sk")),
		})
	}
}

impl Keys {

	/// Generates a fresh pair of key pairs. Called once, when the key file is first written.
	pub fn generate() -> Outcome<Self> {
		Ok(Self {
			author:	res!(fresh()),
			impostor:	res!(fresh()),
		})
	}

	/// The keys as a daticle, for the committed key file.
	pub fn to_dat(&self) -> Dat {
		map(vec![
			("scheme",	Dat::Str("ed25519".to_string())),
			("author",	self.author.to_dat()),
			("impostor",	self.impostor.to_dat()),
		])
	}

	/// Reads the keys from the committed key file.
	pub fn from_dat(d: &Dat) -> Outcome<Self> {
		let scheme = res!(get_str(d, "scheme"));
		if scheme != "ed25519" {
			return Err(err!(
				"The fixture key file names the signature scheme '{}'; v0 signs with Ed25519.",
				scheme;
			Invalid, Input));
		}
		Ok(Self {
			author:	res!(KeyPair::from_dat(&res!(get(d, "author")))),
			impostor:	res!(KeyPair::from_dat(&res!(get(d, "impostor")))),
		})
	}

	/// Reads the committed key file.
	pub fn load(root: &Path) -> Outcome<Self> {
		let path = root.join(KEY_FILE);
		let s = res!(fs::read_to_string(&path), IO, File);
		Self::from_dat(&res!(from_jdat_plain(&s)))
	}

	/// Writes the key file.
	pub fn save(&self, root: &Path) -> Outcome<()> {
		let path = root.join(KEY_FILE);
		res!(fs::write(&path, res!(to_jdat_plain(&self.to_dat()))), IO, File);
		Ok(())
	}
}

/// A fresh Ed25519 key pair, in raw bytes.
fn fresh() -> Outcome<KeyPair> {
	let signer = SignatureScheme::new_ed25519();
	let pk = match res!(signer.get_public_key()) {
		Some(pk) => pk.to_vec(),
		None => return Err(err!("A fresh Ed25519 signer holds no public key."; Bug, Missing)),
	};
	let sk = match res!(signer.get_secret_key()) {
		Some(sk) => sk.to_vec(),
		None => return Err(err!("A fresh Ed25519 signer holds no secret key."; Bug, Missing)),
	};
	Ok(KeyPair {
		pk,
		sk,
	})
}

/// Builds the envelope for a tree region: hash the bytes, then sign the hash (`SPEC.md` §1.3).
///
/// This is the writer's half of the format, assembled from the crate's public API rather than
/// borrowed from its private one, so that a fixture is a second opinion about what a file is.
pub fn seal(
	tree_bytes:	&[u8],
	schema:		&str,
	signer:		&SignatureScheme,
	time:		u64,
)
	-> Outcome<Envelope>
{
	let author = match res!(signer.get_public_key()) {
		Some(pk) => pk.to_vec(),
		None => return Err(err!("The signer holds no public key."; Missing, Configuration)),
	};
	let hash_scheme = envelope::HASH_SCHEME_SHA3_256;
	let mut env = Envelope {
		schema:	schema.to_string(),
		author,
		sig_scheme:	envelope::SIG_SCHEME_ED25519,
		hash_scheme,
		time,
		hash:	res!(doc::hash_tree(hash_scheme, tree_bytes)),
		sig:	Vec::new(),
		tree_len:	try_into!(u64, tree_bytes.len()),
	};
	res!(resign(&mut env, signer));
	Ok(env)
}

/// Signs the envelope's signing input afresh, after the envelope has been meddled with.
pub fn resign(
	env:	&mut Envelope,
	signer:	&SignatureScheme,
)
	-> Outcome<()>
{
	env.sig = res!(signer.sign(&env.signing_input()));
	Ok(())
}

/// Assembles a file: header, envelope, tree region.
pub fn assemble(
	env:		&Envelope,
	tree_bytes:	&[u8],
)
	-> Outcome<Vec<u8>>
{
	let env_bytes = res!(env.encode());
	let mut buf = res!(envelope::write_header(env_bytes.len()));
	buf.extend_from_slice(&env_bytes);
	buf.extend_from_slice(tree_bytes);
	Ok(buf)
}

/// Assembles a file around an envelope map that is not an envelope, such as one missing a key.
pub fn assemble_raw(
	env_dat:	&Dat,
	tree_bytes:	&[u8],
)
	-> Outcome<Vec<u8>>
{
	let env_bytes = res!(env_dat.to_bytes(Vec::new()));
	let mut buf = res!(envelope::write_header(env_bytes.len()));
	buf.extend_from_slice(&env_bytes);
	buf.extend_from_slice(tree_bytes);
	Ok(buf)
}

/// The offset at which the tree region of a file starts.
pub fn tree_start(buf: &[u8]) -> Outcome<usize> {
	let hdr = res!(envelope::read_header(buf));
	Ok(HEADER_LEN + hdr.env_len as usize)
}

/// The first byte at which two byte strings differ, or `None` if one is a prefix of the other.
pub fn first_diff(a: &[u8], b: &[u8]) -> Option<usize> {
	a.iter().zip(b.iter()).position(|(x, y)| x != y)
}

/// The step of `SPEC.md` §2 at which a rejection fixture is refused.
///
/// The distinction that matters is between the steps that touch no content, which a caller may run
/// and stop, and the steps that decode. A fixture declares which one refused it, and the suite holds
/// the implementation to it: a document refused at `Decode` or `Validate` must pass verification
/// first, or verification is not doing its job, and a document refused before that must never be
/// decoded at all.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
	/// Step 1: the magic and the major version.
	Header,
	/// Step 2: the envelope map and its keys.
	Envelope,
	/// Step 3: the tree region, against the limit and against the bytes there are.
	Region,
	/// Step 4: the tree region hashes to what the envelope declares.
	Hash,
	/// Step 5: the author signed that hash.
	Sig,
	/// Step 6: the tree decodes, canonically, within the depth limit.
	Decode,
	/// Step 7: the tree obeys its schema and the remaining limits.
	Validate,
}

impl Stage {

	/// The label a `reject.jdat` names this step by.
	pub fn label(&self) -> &'static str {
		match self {
			Self::Header	=> "header",
			Self::Envelope	=> "envelope",
			Self::Region	=> "region",
			Self::Hash	=> "hash",
			Self::Sig	=> "sig",
			Self::Decode	=> "decode",
			Self::Validate	=> "validate",
		}
	}

	/// The step a label names.
	pub fn from_label(s: &str) -> Outcome<Self> {
		match s {
			"header"	=> Ok(Self::Header),
			"envelope"	=> Ok(Self::Envelope),
			"region"	=> Ok(Self::Region),
			"hash"	=> Ok(Self::Hash),
			"sig"	=> Ok(Self::Sig),
			"decode"	=> Ok(Self::Decode),
			"validate"	=> Ok(Self::Validate),
			_	=> Err(err!(
				"'{}' names no step of the verification order of SPEC.md §2.", s;
			Invalid, Input)),
		}
	}

	/// Whether steps 1 to 5 pass, so that the document is verified and only its content is wrong.
	pub fn verifies(&self) -> bool {
		matches!(self, Self::Decode | Self::Validate)
	}
}

/// What an acceptance fixture expects: the address of the document, and its shape.
#[derive(Clone, Debug)]
pub struct Meta {
	/// The schema the envelope declares.
	pub schema:	String,
	/// The authoring time, in Unix milliseconds.
	pub time:	u64,
	/// The hash of the tree region, which is the document's address.
	pub hash:	Vec<u8>,
	/// The length of the tree region, in bytes.
	pub tree_len:	u64,
	/// The number of nodes.
	pub nodes:	u64,
	/// The greatest nesting depth, the root alone being a depth of 1.
	pub depth:	u64,
	/// Whether the file carries the optional index of §1.4.
	pub index:	bool,
	/// What the fixture is for.
	pub note:	String,
}

impl Meta {

	/// The expectations as a daticle.
	pub fn to_dat(&self) -> Dat {
		map(vec![
			("schema",	Dat::Str(self.schema.clone())),
			("time",	Dat::U64(self.time)),
			("hash",	Dat::BU8(self.hash.clone())),
			("tree_len",	Dat::U64(self.tree_len)),
			("nodes",	Dat::U64(self.nodes)),
			("depth",	Dat::U64(self.depth)),
			("index",	Dat::Bool(self.index)),
			("note",	Dat::Str(self.note.clone())),
		])
	}

	/// Reads the expectations of a fixture.
	pub fn from_dat(d: &Dat) -> Outcome<Self> {
		Ok(Self {
			schema:	res!(get_str(d, "schema")),
			time:	res!(get_u64(d, "time")),
			hash:	res!(get_bytes(d, "hash")),
			tree_len:	res!(get_u64(d, "tree_len")),
			nodes:	res!(get_u64(d, "nodes")),
			depth:	res!(get_u64(d, "depth")),
			index:	res!(get_bool(d, "index")),
			note:	res!(get_str(d, "note")),
		})
	}
}

/// What a rejection fixture expects: the rule broken, and where.
///
/// "It was rejected" is not the claim. The claim is that it was rejected for this reason, at this
/// step, naming this node or this byte, which is what stops a reader passing every rejection fixture
/// by refusing everything.
#[derive(Clone, Debug)]
pub struct Reject {
	/// The step of §2 that must refuse it.
	pub stage:	Stage,
	/// The rule broken, as `SPEC.md` writes it.
	pub rule:	String,
	/// What the rejection must say. `SPEC.md` §6: "Invalid document" is not an error message.
	pub says:	String,
	/// The node the rejection must name, if the rule is a node's.
	pub node:	Option<u64>,
	/// The byte the rejection must name, if the rule is a byte's.
	pub offset:	Option<u64>,
	/// What is wrong with the file.
	pub note:	String,
}

impl Reject {

	/// The declaration as a daticle.
	pub fn to_dat(&self) -> Dat {
		let mut kv = vec![
			("stage",	Dat::Str(self.stage.label().to_string())),
			("rule",	Dat::Str(self.rule.clone())),
			("says",	Dat::Str(self.says.clone())),
		];
		if let Some(id) = self.node {
			kv.push(("node", Dat::U64(id)));
		}
		if let Some(off) = self.offset {
			kv.push(("offset", Dat::U64(off)));
		}
		kv.push(("note", Dat::Str(self.note.clone())));
		map(kv)
	}

	/// Reads the declaration of a fixture.
	pub fn from_dat(d: &Dat) -> Outcome<Self> {
		Ok(Self {
			stage:	res!(Stage::from_label(&res!(get_str(d, "stage")))),
			rule:	res!(get_str(d, "rule")),
			says:	res!(get_str(d, "says")),
			node:	res!(get_u64_opt(d, "node")),
			offset:	res!(get_u64_opt(d, "offset")),
			note:	res!(get_str(d, "note")),
		})
	}
}

/// The value under a key of a daticle map.
fn get(d: &Dat, key: &str) -> Outcome<Dat> {
	match d {
		Dat::Map(m) => match m.get(&dat!(key)) {
			Some(v) => Ok(v.clone()),
			None => Err(err!(
				"The declaration is missing the required key '{}'.", key;
			Invalid, Input, Missing)),
		},
		d => Err(err!(
			"A fixture declaration is a map, found a {:?}.", d.kind();
		Invalid, Input)),
	}
}

/// The value under an optional key of a daticle map.
fn get_opt(d: &Dat, key: &str) -> Outcome<Option<Dat>> {
	match d {
		Dat::Map(m) => Ok(m.get(&dat!(key)).cloned()),
		d => Err(err!(
			"A fixture declaration is a map, found a {:?}.", d.kind();
		Invalid, Input)),
	}
}

/// A required string key.
fn get_str(d: &Dat, key: &str) -> Outcome<String> {
	match res!(get(d, key)) {
		Dat::Str(s) => Ok(s),
		v => Err(err!(
			"The key '{}' carries a {:?}, but a str was expected.", key, v.kind();
		Invalid, Input, Mismatch)),
	}
}

/// A required unsigned integer key.
fn get_u64(d: &Dat, key: &str) -> Outcome<u64> {
	match res!(get(d, key)) {
		Dat::U64(n) => Ok(n),
		v => Err(err!(
			"The key '{}' carries a {:?}, but a u64 was expected.", key, v.kind();
		Invalid, Input, Mismatch)),
	}
}

/// An optional unsigned integer key.
fn get_u64_opt(d: &Dat, key: &str) -> Outcome<Option<u64>> {
	match res!(get_opt(d, key)) {
		None => Ok(None),
		Some(Dat::U64(n)) => Ok(Some(n)),
		Some(v) => Err(err!(
			"The key '{}' carries a {:?}, but a u64 was expected.", key, v.kind();
		Invalid, Input, Mismatch)),
	}
}

/// A required boolean key.
fn get_bool(d: &Dat, key: &str) -> Outcome<bool> {
	match res!(get(d, key)) {
		Dat::Bool(b) => Ok(b),
		v => Err(err!(
			"The key '{}' carries a {:?}, but a bool was expected.", key, v.kind();
		Invalid, Input, Mismatch)),
	}
}

/// A required byte string key.
fn get_bytes(d: &Dat, key: &str) -> Outcome<Vec<u8>> {
	match res!(get(d, key)) {
		Dat::BU8(v) => Ok(v),
		v => Err(err!(
			"The key '{}' carries a {:?}, but a bu8 was expected.", key, v.kind();
		Invalid, Input, Mismatch)),
	}
}

/// The fixture directory.
pub fn fixtures_dir() -> PathBuf {
	PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures")
}

/// Reads a file whole.
pub fn read_bytes(path: &Path) -> Outcome<Vec<u8>> {
	match fs::read(path) {
		Ok(byts) => Ok(byts),
		Err(e) => Err(err!(e,
			"Could not read {}.", path.display();
		IO, File)),
	}
}

/// Reads a text file whole.
pub fn read_text(path: &Path) -> Outcome<String> {
	match fs::read_to_string(path) {
		Ok(s) => Ok(s),
		Err(e) => Err(err!(e,
			"Could not read {}.", path.display();
		IO, File)),
	}
}

/// Writes a file whole.
pub fn write_bytes(path: &Path, byts: &[u8]) -> Outcome<()> {
	match fs::write(path, byts) {
		Ok(()) => Ok(()),
		Err(e) => Err(err!(e,
			"Could not write {}.", path.display();
		IO, File)),
	}
}

/// The trees the fixtures are built from, and the ones they are built to break.
pub mod tree {
	use super::*;

	/// A document with nothing in it.
	pub fn empty() -> Dat {
		node(NodeKind::Doc, map(vec![
			("title",	Dat::Str("An empty document".to_string())),
			("lang",	Dat::Str("en".to_string())),
		]))
	}

	/// A document of one paragraph.
	pub fn one_para() -> Dat {
		doc_of(vec![
			node(NodeKind::Para, map(vec![
				("children", Dat::List(vec![
					text("The web gave that up in 1993 and spent thirty years paying for it."),
				])),
			])),
		])
	}

	/// A document using every v0 node kind once, an optional field present, another absent, and a
	/// node carrying no children at all.
	///
	/// The thirteen kinds are the doc itself, a heading, a section, a paragraph, text, emphasis, a
	/// link addressed by name, code, a quote, a list, an item, a box, and an image. The link carries
	/// the typed address form of §4.3, a single-entry map, and the image references its content by a
	/// `b32` content hash, both of which the old bare-string and short-byte forms are not.
	pub fn every_kind() -> Dat {
		node(NodeKind::Doc, map(vec![
			("title",	Dat::Str("Style without a cascade".to_string())),
			("lang",	Dat::Str("en".to_string())),
			("children", Dat::List(vec![
				node(NodeKind::Heading, map(vec![
					("level",	Dat::U8(2)),
					("children",	Dat::List(vec![text("Style without a cascade")])),
				])),
				node(NodeKind::Section, map(vec![
					("title",	Dat::Str("A section".to_string())),
					("children", Dat::List(vec![
						node(NodeKind::Para, map(vec![
							("children", Dat::List(vec![
								text("A run\twith a tab\nand a newline, "),
								node(NodeKind::Emph, map(vec![
									("strong",	Dat::Bool(true)),
									("children",	Dat::List(vec![text("loud")])),
								])),
								node(NodeKind::Link, map(vec![
									("to",	map(vec![
										("name",	Dat::Str("news.cricket".to_string())),
									])),
									("children",	Dat::List(vec![text("and a link")])),
								])),
							])),
						])),
						// A paragraph with no children omits the key entirely (§3 rule 4).
						node(NodeKind::Para, map(vec![])),
						// A preserved run of source, whose text is a field rather than children.
						node(NodeKind::Code, map(vec![
							("lang",	Dat::Str("rust".to_string())),
							("text",	Dat::Str("fn main() {}".to_string())),
						])),
						// A block quotation, carrying flow content and an optional citation.
						node(NodeKind::Quote, map(vec![
							("cite",	Dat::Str("A. Author".to_string())),
							("children", Dat::List(vec![
								node(NodeKind::Para, map(vec![
									("children", Dat::List(vec![text("A quoted line.")])),
								])),
							])),
						])),
						node(NodeKind::List, map(vec![
							("ordered",	Dat::Bool(false)),
							("children", Dat::List(vec![
								node(NodeKind::Item, map(vec![
									("children", Dat::List(vec![
										node(NodeKind::Para, map(vec![
											("children", Dat::List(vec![text("An item.")])),
										])),
									])),
								])),
							])),
						])),
						// A box with its optional style absent, around an image with both of its
						// optional fields present and a b32 content hash.
						node(NodeKind::Boxx, map(vec![
							("children", Dat::List(vec![
								node(NodeKind::Image, map(vec![
									("hash",	Dat::from([0x01u8; 32])),
									("alt",	Dat::Str("A diagram of a tree".to_string())),
									("w",	Dat::U32(640)),
									("h",	Dat::U32(480)),
								])),
							])),
						])),
					])),
				])),
			])),
		]))
	}

	/// A document exercising the style table of §4.4: an inherited property and a self-only one.
	///
	/// The table defines two styles. `callout` carries the self-only `bg` and `pad`, and `lede`
	/// carries the inherited `size`; a box names the first and a paragraph within it names the second,
	/// so that both a style that inherits and one that does not are declared, named, and resolved.
	pub fn styled() -> Dat {
		node(NodeKind::Doc, map(vec![
			("title",	Dat::Str("Style without a cascade".to_string())),
			("lang",	Dat::Str("en".to_string())),
			("styles", map(vec![
				("callout", map(vec![
					("bg",	Dat::Str("muted".to_string())),	// Self-only (§4.4).
					("pad",	Dat::U8(3)),			// Self-only.
				])),
				("lede", map(vec![
					("size",	Dat::I8(1)),			// Inherited.
				])),
			])),
			("children", Dat::List(vec![
				node(NodeKind::Boxx, map(vec![
					("style",	Dat::Str("callout".to_string())),
					("children", Dat::List(vec![
						node(NodeKind::Para, map(vec![
							("style",	Dat::Str("lede".to_string())),
							("children", Dat::List(vec![
								text("A lede paragraph in a callout box."),
							])),
						])),
					])),
				])),
			])),
		]))
	}

	/// A document where a box names an alignment, and the paragraphs inside it do not (§4.4).
	///
	/// `align` is self-only, so it aligns the lines of the node that named it and of nothing within
	/// it. The box has no lines of its own, so this document must read exactly as it would with no
	/// style at all. It is here because the property inherits in CSS and does not in the format, so a
	/// reader built on a browser will carry the box's alignment down into its paragraphs unless it is
	/// stopped, and then one document says two things.
	pub fn align_is_local() -> Dat {
		node(NodeKind::Doc, map(vec![
			("title",	Dat::Str("An alignment that stays where it is put".to_string())),
			("lang",	Dat::Str("en".to_string())),
			("styles", map(vec![
				("flush", map(vec![
					("align",	Dat::Str("justify".to_string())),	// Self-only (§4.4).
				])),
			])),
			("children", Dat::List(vec![
				node(NodeKind::Boxx, map(vec![
					("style",	Dat::Str("flush".to_string())),
					("children", Dat::List(vec![
						node(NodeKind::Para, map(vec![
							("children", Dat::List(vec![
								// Long enough to take several lines, since a promise about how lines
								// are aligned cannot be tested on a document with one line.
								text("The oxeweb replaces the web's cascade with locality, so that a \
									style error cannot escape the node that made it, and no rule \
									reaches across a document to touch what it never named."),
							])),
						])),
					])),
				])),
			])),
		]))
	}

	/// A document whose one paragraph carries a link addressed by NAMES name (§4.3).
	pub fn link_by_name() -> Dat {
		doc_of(vec![
			node(NodeKind::Para, map(vec![
				("children", Dat::List(vec![
					node(NodeKind::Link, map(vec![
						("to",	map(vec![
							("name",	Dat::Str("news.cricket".to_string())),
						])),
						("children",	Dat::List(vec![text("the cricket news")])),
					])),
				])),
			])),
		])
	}

	/// A document whose one paragraph carries a link addressed by content hash (§4.3).
	pub fn link_by_hash() -> Dat {
		doc_of(vec![
			node(NodeKind::Para, map(vec![
				("children", Dat::List(vec![
					node(NodeKind::Link, map(vec![
						("to",	map(vec![
							("hash",	Dat::from([0x9fu8; 32])),
						])),
						("children",	Dat::List(vec![text("a document by address")])),
					])),
				])),
			])),
		])
	}

	/// A document whose one child is a kind this version does not know, carrying a valid fallback.
	///
	/// The fallback is a list of known nodes that stand in for the unknown kind (§4.5), and the
	/// unknown node also carries an uninterpreted field, which a reader that knew the kind would use
	/// and which this one holds only to the canonical encoding rules of §3. A reader that does not
	/// know the kind renders and validates the fallback, so the whole document is accepted.
	pub fn unknown_fallback() -> Dat {
		node(NodeKind::Doc, map(vec![
			("title",	Dat::Str("A document with an unknown kind".to_string())),
			("lang",	Dat::Str("en".to_string())),
			("children", Dat::List(vec![
				alien(map(vec![
					("fallback", Dat::List(vec![
						node(NodeKind::List, map(vec![
							("ordered",	Dat::Bool(false)),
							("children", Dat::List(vec![
								node(NodeKind::Item, map(vec![
									("children", Dat::List(vec![
										node(NodeKind::Para, map(vec![
											("children", Dat::List(vec![
												text("Q1 revenue: 1.2M"),
											])),
										])),
									])),
								])),
								node(NodeKind::Item, map(vec![
									("children", Dat::List(vec![
										node(NodeKind::Para, map(vec![
											("children", Dat::List(vec![
												text("Q2 revenue: 1.5M"),
											])),
										])),
									])),
								])),
							])),
						])),
					])),
					// A field only a reader that knows the kind interprets.
					("rows",	Dat::Str("held to §3, not read as a document".to_string())),
				])),
			])),
		]))
	}

	/// A document whose deepest node sits at `depth`, counting the root as 1.
	///
	/// The chain is boxes, since a box takes flow content and is therefore the cheapest way to nest.
	pub fn chain(depth: usize) -> Outcome<Dat> {
		if depth < 2 {
			return Err(err!(
				"A chain of depth {} has no room for the doc at its head.", depth;
			Invalid, Input));
		}
		let mut inner = node(NodeKind::Boxx, map(vec![]));
		for _ in 0..(depth - 2) {
			inner = node(NodeKind::Boxx, map(vec![
				("children", Dat::List(vec![inner])),
			]));
		}
		Ok(doc_of(vec![inner]))
	}

	/// A document whose canonical encoding is exactly `target` bytes.
	///
	/// The tree is one paragraph of one text run, padded until the bytes come out to the byte the
	/// caller asked for, since the size limit of §5 is a limit on the encoded region and a fixture
	/// at the limit must land on it exactly.
	pub fn sized(target: usize) -> Outcome<Dat> {
		// The encoding grows by one byte for each character of the pad, except where the length in
		// front of a compound needs another byte, so search for the longest pad that fits and then
		// walk up from it.
		let mut lo = 0;
		let mut hi = target;
		while lo < hi {
			let mid = lo + (hi - lo + 1) / 2;
			if res!(sized_len(mid)) <= target {
				lo = mid;
			} else {
				hi = mid - 1;
			}
		}
		for n in lo..=(lo + 8) {
			if res!(sized_len(n)) == target {
				return Ok(padded(n));
			}
		}
		Err(err!(
			"No padding gives a tree of exactly {} bytes; the nearest is {} bytes.",
			target, res!(sized_len(lo));
		Invalid, Input, Bug))
	}

	/// The encoded length of the padded document with a pad of `n` characters.
	fn sized_len(n: usize) -> Outcome<usize> {
		Ok(res!(padded(n).to_bytes(Vec::new())).len())
	}

	/// The padded document, with a pad of `n` characters of prose.
	fn padded(n: usize) -> Dat {
		doc_of(vec![
			node(NodeKind::Para, map(vec![
				("children", Dat::List(vec![text(&prose(n))])),
			])),
		])
	}

	/// `n` characters of ASCII prose, so that a character is a byte.
	fn prose(n: usize) -> String {
		const LINE: &'static str =
			"The hash is the address, and the address is the hash. ";
		let mut s = String::with_capacity(n + LINE.len());
		while s.len() < n {
			s.push_str(LINE);
		}
		s.truncate(n);
		s
	}

	/// A document whose children are the given flow nodes.
	pub fn doc_of(kids: Vec<Dat>) -> Dat {
		node(NodeKind::Doc, map(vec![
			("title",	Dat::Str("A document".to_string())),
			("lang",	Dat::Str("en".to_string())),
			("children",	Dat::List(kids)),
		]))
	}

	/// A document whose one child is a heading carrying the given payload, which is node 1.
	pub fn doc_with_heading(payload: Dat) -> Dat {
		node(NodeKind::Doc, map(vec![
			("title",	Dat::Str("A document".to_string())),
			("lang",	Dat::Str("en".to_string())),
			("children", Dat::List(vec![
				Dat::Usr(ukid(NodeKind::Heading), Some(Box::new(payload))),
			])),
		]))
	}
}

/// The schema every fixture but one declares.
pub fn schema() -> &'static str {
	SCHEMA_DOC
}

/// The node depth limit, quoted where a fixture needs it.
pub fn depth_limit() -> usize {
	limit::DEPTH
}

/// The tree region size limit, quoted where a fixture needs it.
pub fn tree_limit() -> usize {
	limit::TREE_BYTES
}

/// Encodes a tree canonically, refusing one that is not canonical.
pub fn encode(tree: &Dat) -> Outcome<Vec<u8>> {
	canon::encode(tree)
}

/// Encodes a tree without checking it, which is how a fixture that breaks a rule is written.
pub fn encode_unchecked(tree: &Dat) -> Outcome<Vec<u8>> {
	Ok(res!(tree.to_bytes(Vec::new())))
}
