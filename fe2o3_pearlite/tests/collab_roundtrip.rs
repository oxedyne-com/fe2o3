//! The collaboration backend end to end: two different authors each seal an annotation operation with
//! a real P-256 key, both are stored in an o3db hub under the document's identity, and a scan then a
//! fold reproduces both annotations in deterministic order and materialises them into a local `.prl`.
//! Both signatures verify, a tampered envelope does not, the displayed author comes from the signer by
//! way of the document's keyring rather than from the body, and a `.prl` written without a doc_id still
//! reads -- the back-compatibility the additive header field promises.

use oxedyne_fe2o3_austenite::{
	emit::pearl::{
		Annotation,
		AnnotationKind,
		PearlBuilder,
		PearlDoc,
	},
	ir::{
		Dims,
		DrawOp,
		Graphic,
		Sp,
	},
	ledger::Ledger,
	page::{
		Frame,
		Page,
		PageGeometry,
		Placed,
		PlacedKind,
	},
};
use oxedyne_fe2o3_core::{
	prelude::*,
	rand::Rand,
};
use oxedyne_fe2o3_crypto::enc::EncryptionScheme;
use oxedyne_fe2o3_graphics::{
	colour::Rgba,
	path::{
		Bounds,
		Path,
	},
};
use oxedyne_fe2o3_hash::{
	csum::ChecksumScheme,
	hash::HashScheme,
};
use oxedyne_fe2o3_jdat::prelude::*;
use oxedyne_fe2o3_net::ecdsa::P256KeyPair;
use oxedyne_fe2o3_o3db_sync::{
	data::core::RestSchemesInput,
	test::setup,
};
use oxedyne_fe2o3_ore::{
	envelope::Envelope,
	id::OpId,
	op::Record,
};
use oxedyne_fe2o3_pearlite::collab::{
	fold,
	hub::{
		Hub,
		O3dbHub,
	},
	op,
	replica_of,
	sign,
	DocId,
	Keyring,
};

use std::sync::Arc;


/// A one-graphic page whose block hash is distinct per page number, so a two-page document has two
/// real anchors to attach annotations to.
fn dot_page(number: u32, geom: PageGeometry) -> Outcome<Page> {
	let fill	= res!(Path::rect(Bounds::new(0.0, 0.0, 20.0, 20.0)));
	let graphic	= Graphic::new(
		vec![DrawOp::Fill { path: fill, colour: Rgba::BLACK }],
		Dims::new(Sp::from_pt(20.0), Sp::from_pt(20.0), Sp::ZERO));
	let mut frame = Frame::new();
	frame.push(Placed::new(
		Sp::from_pt(40.0), Sp::from_pt(40.0), graphic.dims, PlacedKind::Graphic(Arc::new(graphic))));
	Ok(Page::new(number, geom, frame))
}

/// Seals one author's creation of an annotation: an Ore proposal, identified under the replica the
/// author's own key derives, signed with a real P-256 key in the encodings a browser's WebCrypto key
/// uses, into an envelope the hub can store.
fn sealed_create(
	doc:		&DocId,
	key:		&P256KeyPair,
	counter:	u64,
	ann:		&Annotation,
	time:		u64,
)
	-> Outcome<Envelope>
{
	let op		= res!(op::create(doc, ann, time));
	let replica	= replica_of(&key.public_key(), doc);
	let rec		= Record::root(OpId::new(replica, counter), op);
	let sig		= res!(key.sign(&res!(sign::signing_bytes(&rec))));
	sign::seal(&rec, key.public_key(), sig, doc)
}

#[test]
fn collaboration_backend_round_trips_two_authors_through_the_hub() -> Outcome<()> {
	let geom = PageGeometry::a4();

	// A two-page document with a stable identity, and its two real block hashes.
	let mut builder = res!(PearlBuilder::new(&Ledger::new(), geom)).with_doc_id("prl-collab-test");
	res!(builder.add_page(&res!(dot_page(1, geom))));
	res!(builder.add_page(&res!(dot_page(2, geom))));
	let prl_text = res!(builder.to_string());

	let doc		= res!(PearlDoc::from_string(prl_text.clone()));
	let doc_id	= DocId::new(res!(res!(doc.doc_id()).ok_or_else(|| err!(
		"the test document was built with a doc_id but reports none"; Test, Missing))));
	let hashes	= res!(doc.block_hashes());
	assert_eq!(hashes.len(), 2, "a two-page document has two block hashes to anchor to");

	// Two different authors, each annotating a different page.
	let key_a = res!(P256KeyPair::generate());
	let key_b = res!(P256KeyPair::generate());
	assert_ne!(key_a.public_key(), key_b.public_key(), "the two authors are distinct signers");

	// The bodies claim friendly author strings, but the fold takes the author from the signer via the
	// keyring, so what the body says is only advisory.
	let ann_a = Annotation::new(
		hashes[0].as_str(), AnnotationKind::Highlight, "author A on page one", "whoever-a-claims",
		"2026-09-20T10:00:00Z");
	let ann_b = Annotation::new(
		hashes[1].as_str(), AnnotationKind::Note, "author B on page two", "whoever-b-claims",
		"2026-09-20T11:00:00Z");

	let mut keyring = Keyring::new();
	keyring.insert(&key_a.public_key(), "author-a");
	keyring.insert(&key_b.public_key(), "author-b");

	// Author A's operation carries the earlier clock reading, so a deterministic fold must place it
	// first whatever order the store hands the two back in.
	let env_a = res!(sealed_create(&doc_id, &key_a, 1, &ann_a, 1000));
	let env_b = res!(sealed_create(&doc_id, &key_b, 1, &ann_b, 1001));

	// A signature presented against the wrong author's key is refused at the seal, so nothing
	// unattributable is ever built.
	let rec_a	= Record::root(
		OpId::new(replica_of(&key_a.public_key(), &doc_id), 1), res!(op::create(&doc_id, &ann_a, 1000)));
	let cross_sig = res!(key_a.sign(&res!(sign::signing_bytes(&rec_a))));
	assert!(sign::seal(&rec_a, key_b.public_key(), cross_sig, &doc_id).is_err(),
		"a record signed by A but presented with B's public key must not seal");

	// Start a throwaway o3db instance under a process-unique root, with every zone inside it.
	let root	= std::env::temp_dir().join(fmt!("pearlite_collab_hub_{}", std::process::id()));
	let db_root	= root.join("db");
	res!(std::fs::create_dir_all(&db_root));

	let mut enckey = [0u8; 32];
	Rand::fill_u8(&mut enckey);
	let aes = res!(EncryptionScheme::new_aes_256_gcm_with_key(&enckey[..]));
	let crc = ChecksumScheme::new_crc32();
	let schms_input = RestSchemesInput::new(
		Some(aes.clone()), None::<HashScheme>, None::<HashScheme>, Some(crc.clone()));

	let mut cfg = res!(setup::default_cfg());
	cfg.zone_overrides			= DaticleMap::new();	// keep every zone under db_root
	cfg.data_file_max_bytes		= 1_000_000;
	cfg.rest_chunk_threshold	= 500_000;
	cfg.cache_size_limit_bytes	= 10_000_000;

	let user = setup::Uid::default();
	let db = res!(setup::start_db(db_root.clone(), Some(cfg), schms_input, None, true, true));

	// Store both operations -- the hub verifies each and takes its identity from the record -- then
	// read the whole log back.
	let hub = O3dbHub::new(&db, user);
	let id_a = res!(hub.put(&doc_id, &env_a));
	let id_b = res!(hub.put(&doc_id, &env_b));
	assert_ne!(id_a, id_b, "the two authors' operations have distinct identities");

	let loaded = res!(hub.scan(&doc_id));
	req!(2, loaded.len(), "both operations are in the document's log");

	// Every stored envelope verifies, and opening it yields its record bound to its signer.
	let mut opened = Vec::new();
	for (_id, env) in &loaded {
		assert!(res!(sign::verify(env)), "a stored envelope must verify against its own key");
		opened.push(res!(sign::open(env, &doc_id)));
	}

	// Fold the log: both annotations materialise, in (time, id) order -- A before B -- and the author
	// is the keyring's name for the signer, not the string the body carried.
	let report = fold::fold(&opened, &doc_id, &keyring);
	assert!(report.skipped.is_empty(), "no operation is skipped in the honest case");
	let anns = report.annotations;
	req!(2, anns.len(), "two proposals fold to two annotations");
	assert_eq!(anns[0].author, "author-a", "the earlier operation folds first, attributed by key");
	assert_eq!(anns[1].author, "author-b", "the later operation folds second, attributed by key");
	assert_ne!(anns[0].author, "whoever-a-claims", "the body's author string does not win");
	assert_eq!(anns[0].anchor, hashes[0]);
	assert_eq!(anns[1].anchor, hashes[1]);

	// Materialise into a fresh local copy of the .prl.
	let mut local = res!(PearlDoc::from_string(prl_text.clone()));
	assert!(res!(local.annotations()).is_empty(), "the local copy opens with no annotations");
	req!(2, res!(fold::materialise(&mut local, &anns)));
	let got = res!(local.annotations());
	req!(2, got.len(), "both annotations are attached to the local .prl");
	assert_eq!(got[0].payload, "author A on page one");
	assert_eq!(got[1].payload, "author B on page two");

	// Shut the database down and clear its files before the checks that do not need it.
	res!(db.shutdown());
	let _ = std::fs::remove_dir_all(&root);

	// A tampered envelope does not verify: the signature is bound to the payload and the key.
	let mut bad_sig = env_a.signature().to_vec();
	bad_sig[10] ^= 0x01;
	let tampered_sig = Envelope::new(env_a.payload().to_vec(), env_a.signer().to_vec(), bad_sig);
	assert!(!res!(sign::verify(&tampered_sig)), "a tampered signature must not verify");
	assert!(sign::open(&tampered_sig, &doc_id).is_err(), "opening a tampered envelope must error");

	let mut bad_payload = env_a.payload().to_vec();
	bad_payload[0] ^= 0x01;
	let tampered_payload = Envelope::new(bad_payload, env_a.signer().to_vec(), env_a.signature().to_vec());
	assert!(!res!(sign::verify(&tampered_payload)), "a tampered payload must not verify");

	// Back-compatibility: a checked-in .prl written before the doc_id field reads, and reports none.
	let sample = concat!(
		env!("CARGO_MANIFEST_DIR"), "/../fe2o3_austenite/web/pearl-reader/samples/demo.prl");
	let old = res!(PearlDoc::read_file(sample));
	assert_eq!(res!(old.doc_id()), None, "a .prl written without a doc_id reports none");
	assert!(res!(old.page_count()) >= 1, "and it still reads as a document");

	Ok(())
}
