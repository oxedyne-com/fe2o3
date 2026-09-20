//! The malicious-peer regression suite: for each shape the security review named, a test that proves
//! the attack fails. A signed operation is attacker-controlled input from an untrusted peer, and every
//! test below hands the fold or the seal exactly what a hostile peer would and asserts it is refused,
//! attributed to the real signer, or skipped -- never trusted, and never able to wedge the whole fold.
//!
//! These are pure-crypto unit tests: no store is stood up, because none of the shapes needs one. The
//! hub's own verify-and-bound ingest is exercised by `collab_roundtrip.rs`; here the concern is the
//! signing core and the fold.

use oxedyne_fe2o3_austenite::emit::pearl::{
	Annotation,
	AnnotationKind,
};
use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_jdat::prelude::*;
use oxedyne_fe2o3_net::ecdsa::P256KeyPair;
use oxedyne_fe2o3_ore::{
	id::OpId,
	op::{
		Op,
		Record,
		Settled,
	},
};
use oxedyne_fe2o3_pearlite::collab::{
	fold,
	op,
	replica_of,
	sign::{
		self,
		Opened,
	},
	DocId,
	Keyring,
};


/// An annotation on `anchor` claiming `author` in its body, for the given document.
fn annotation(anchor: &str, author: &str) -> Annotation {
	Annotation::new(anchor, AnnotationKind::Note, "the note text", author, "2026-09-20T12:00:00Z")
}

/// Seals `op` as a root record under the replica the signer's own key derives, the way an honest peer
/// would, and returns the envelope.
fn seal_honest(doc: &DocId, key: &P256KeyPair, counter: u64, op: Op)
	-> Outcome<oxedyne_fe2o3_ore::envelope::Envelope>
{
	let rec = Record::root(OpId::new(replica_of(&key.public_key(), doc), counter), op);
	let sig = res!(key.sign(&res!(sign::signing_bytes(&rec))));
	sign::seal(&rec, key.public_key(), sig, doc)
}

/// Opens a freshly sealed operation, for `doc`, into the verified, signer-bound form the fold consumes.
fn opened(doc: &DocId, key: &P256KeyPair, counter: u64, op: Op) -> Outcome<Opened> {
	sign::open(&res!(seal_honest(doc, key, counter, op)), doc)
}

/// A proposal opening an annotation stamped for `doc`.
fn proposal(doc: &DocId, key: &P256KeyPair, counter: u64, anchor: &str, author: &str, time: u64)
	-> Outcome<Opened>
{
	opened(doc, key, counter, res!(op::create(doc, &annotation(anchor, author), time)))
}


/// A forged author string in the body loses to the signer: the folded author is the keyring's name for
/// the signing key, never the string the body claimed.
#[test]
fn a_forged_author_string_is_ignored_and_the_signer_is_shown() -> Outcome<()> {
	let doc		= DocId::new("doc-forge");
	let attacker	= res!(P256KeyPair::generate());

	// The body claims to be from "alice"; the attacker holds their own key, not alice's.
	let op		= res!(proposal(&doc, &attacker, 1, "block-1", "alice", 100));

	// An empty keyring shows the fingerprint; either way it is derived from the signing key.
	let report	= fold::fold(&[op], &doc, &Keyring::new());
	req!(1, report.annotations.len());
	let shown	= &report.annotations[0].author;
	assert_ne!(shown, "alice", "the forged body author string must not be shown");
	assert_eq!(shown, &oxedyne_fe2o3_pearlite::collab::fingerprint(&attacker.public_key()),
		"an unknown signer is shown by its own fingerprint");
	Ok(())
}

/// A record claiming a replica identity its signer's key does not derive is refused at the seal, and a
/// hand-built envelope carrying one is refused at the open -- so id-squatting cannot occur.
#[test]
fn a_squatted_replica_id_is_refused_at_seal_and_open() -> Outcome<()> {
	let attacker	= res!(P256KeyPair::generate());
	let victim		= res!(P256KeyPair::generate());
	let doc		= DocId::new("doc-squat");
	let victim_rep	= replica_of(&victim.public_key(), &doc);

	// The attacker builds a record under the victim's replica and signs it with their own key.
	let op	= res!(op::create(&doc, &annotation("block-1", "victim"), 100));
	let rec	= Record::root(OpId::new(victim_rep, 1), op);
	let sig	= res!(attacker.sign(&res!(sign::signing_bytes(&rec))));

	// Sealing with the attacker's key is refused: the header's replica is not the one that key derives.
	assert!(sign::seal(&rec, attacker.public_key(), sig.clone(), &doc).is_err(),
		"a record whose replica is not the signer's must not seal");

	// And a hand-built envelope carrying the same claim is refused at the open, though its signature is
	// perfectly valid over its bytes.
	let payload	= res!(sign::signing_bytes(&rec));
	let forged	= oxedyne_fe2o3_ore::envelope::Envelope::new(payload, attacker.public_key(), sig);
	assert!(res!(sign::verify(&forged)), "the forged envelope's signature does hold over its bytes");
	assert!(sign::open(&forged, &doc).is_err(),
		"opening an envelope whose replica is not its signer's must be refused");
	Ok(())
}

/// A counter-poison operation -- a forged op at a huge counter under the victim's replica, which would
/// wedge every reader's log -- never reaches the fold, because it cannot be opened: its replica is not
/// the forger's.
#[test]
fn a_counter_poison_op_cannot_be_opened() -> Outcome<()> {
	let attacker	= res!(P256KeyPair::generate());
	let victim		= res!(P256KeyPair::generate());

	let doc	= DocId::new("doc-poison");
	let op	= res!(op::create(&doc, &annotation("block-1", "victim"), 100));
	// The poison: the victim's replica, a counter far past anything they will mint.
	let rec	= Record::root(OpId::new(replica_of(&victim.public_key(), &doc), 1_000_000_000), op);
	let sig	= res!(attacker.sign(&res!(sign::signing_bytes(&rec))));
	let env	= oxedyne_fe2o3_ore::envelope::Envelope::new(
		res!(sign::signing_bytes(&rec)), attacker.public_key(), sig);

	assert!(sign::open(&env, &doc).is_err(),
		"a counter-poison op forging the victim's replica is refused at the open and never folds");
	Ok(())
}

/// A deeply nested annotation body is refused by the bounded decoder rather than recursed into: the
/// decode terminates, and a validly-signed proposal carrying one is skipped, not fatal to the fold.
#[test]
fn a_deeply_nested_body_is_refused_and_skipped_not_fatal() -> Outcome<()> {
	let doc		= DocId::new("doc-deep");
	let good_key	= res!(P256KeyPair::generate());
	let bad_key	= res!(P256KeyPair::generate());

	// A body nested a thousand deep, far past the decode limit. The bytes are built from the inside
	// out rather than encoded, because the encoder recurses just as the decoder does and would itself
	// overflow building the bomb -- which a hostile peer is under no obligation to use.
	let mut deep = vec![Dat::EMPTY_CODE];
	for _ in 0..1_000 {
		let payload_len = deep.len();
		let mut outer = vec![Dat::LIST_CODE];
		outer = res!(Dat::C64(payload_len as u64).to_bytes(outer));
		outer.append(&mut deep);
		deep = outer;
	}

	// Decoding it directly terminates and errors rather than exhausting the stack.
	assert!(op::annotation_from_body(&deep).is_err(),
		"a deeply nested body is refused by the bounded decoder");

	// A validly-signed proposal carrying it is skipped; a good proposal beside it still folds.
	let bad	= res!(opened(&doc, &bad_key, 1, Op::Proposal {
		title:	"block-x".to_string(),
		body:	deep,
		voice:	op::VOICE.to_string(),
		time:	50,
	}));
	let good	= res!(proposal(&doc, &good_key, 1, "block-1", "author", 100));

	let report	= fold::fold(&[bad, good], &doc, &Keyring::new());
	req!(1, report.annotations.len(), "the good annotation folds despite the poisoned one");
	req!(1, report.skipped.len(), "the deeply nested body is reported as skipped");
	Ok(())
}

/// A validly-signed proposal whose body is not an annotation at all is skipped and reported, and does
/// not make the fold fail for the good annotations.
#[test]
fn a_malformed_signed_body_is_skipped_not_fatal() -> Outcome<()> {
	let doc		= DocId::new("doc-malformed");
	let good_key	= res!(P256KeyPair::generate());
	let bad_key	= res!(P256KeyPair::generate());

	// A well-formed daticle that is simply not an Annotation map.
	let not_ann	= res!(dat!("just a string, not an annotation").to_bytes(Vec::new()));
	let bad	= res!(opened(&doc, &bad_key, 1, Op::Proposal {
		title:	"block-x".to_string(),
		body:	not_ann,
		voice:	op::VOICE.to_string(),
		time:	50,
	}));
	let good	= res!(proposal(&doc, &good_key, 1, "block-1", "author", 100));

	let report	= fold::fold(&[bad, good], &doc, &Keyring::new());
	req!(1, report.annotations.len(), "the good annotation survives a malformed neighbour");
	req!(1, report.skipped.len(), "the malformed body is reported as skipped");
	Ok(())
}

/// An annotation stamped for one document and replayed onto another -- where the same block address
/// happens to exist -- is refused by the fold, which folds only bodies stamped for the document in hand.
#[test]
fn a_cross_document_replay_is_rejected() -> Outcome<()> {
	let doc_a	= DocId::new("doc-A");
	let doc_b	= DocId::new("doc-B");
	let key	= res!(P256KeyPair::generate());

	// Signed for document A, then folded under document B.
	let op	= res!(proposal(&doc_a, &key, 1, "shared-block", "author", 100));
	let report	= fold::fold(&[op], &doc_b, &Keyring::new());
	req!(0, report.annotations.len(), "an annotation stamped for another document does not fold");
	req!(1, report.skipped.len(), "and it is reported as skipped");
	Ok(())
}

/// An amendment from anyone but the proposal's own signer is refused, so a stranger cannot edit another
/// author's annotation. The original stands.
#[test]
fn a_foreign_amendment_is_rejected() -> Outcome<()> {
	let doc		= DocId::new("doc-amend");
	let owner	= res!(P256KeyPair::generate());
	let stranger	= res!(P256KeyPair::generate());

	let create	= res!(proposal(&doc, &owner, 1, "block-1", "owner", 100));
	let on		= create.record().id();

	// The stranger, with a perfectly valid key and signature of their own, amends the owner's proposal.
	let amend	= res!(opened(&doc, &stranger, 1, res!(op::edit(
		&doc, on, &annotation("block-1", "stranger's rewrite"), 200))));

	let report	= fold::fold(&[create, amend], &doc, &Keyring::new());
	req!(1, report.annotations.len());
	assert_eq!(report.annotations[0].payload, "the note text",
		"the stranger's amendment must not replace the owner's annotation body");
	req!(1, report.skipped.len(), "the foreign amendment is reported as skipped");
	Ok(())
}

/// A proposal its own author withdraws stays withdrawn: a later amendment, even from that same author,
/// cannot resurrect it. Tombstone sticks.
#[test]
fn a_withdrawn_proposal_cannot_be_resurrected_by_an_amendment() -> Outcome<()> {
	let doc		= DocId::new("doc-tomb");
	let owner	= res!(P256KeyPair::generate());

	let create	= res!(proposal(&doc, &owner, 1, "block-1", "owner", 100));
	let on		= create.record().id();

	// The author withdraws it, then -- later -- tries to amend it back into existence.
	let withdraw	= res!(opened(&doc, &owner, 2, Op::Settled {
		on,
		state:	Settled::Declined,
		mark:	None,
		time:	200,
	}));
	let amend	= res!(opened(&doc, &owner, 3, res!(op::edit(
		&doc, on, &annotation("block-1", "back from the dead"), 300))));
	let amend_id	= amend.record().id();

	let report	= fold::fold(&[create, withdraw, amend], &doc, &Keyring::new());
	req!(0, report.annotations.len(), "a withdrawn proposal stays withdrawn despite a later amendment");
	assert!(report.skipped.iter().any(|s| s.id == amend_id),
		"the resurrection attempt is reported as skipped");
	Ok(())
}

/// A validly-signed Settled (and a Said), authored by a key in one document, cannot be replayed into
/// another to withdraw or disturb that key's annotation there -- even though the signature holds and
/// the key is the same. The per-document replica binding refuses it at the open, and the fold skips it
/// belt-and-braces. This is the residual the earlier body-`doc_id` stamp missed: a Settled carries no
/// body to stamp, and its `on` identifier is not inert across documents, Ore minting counters per log.
#[test]
fn a_settled_or_said_from_another_document_cannot_be_replayed() -> Outcome<()> {
	let doc_a	= DocId::new("doc-A");
	let doc_b	= DocId::new("doc-B");
	let owner	= res!(P256KeyPair::generate());

	// The owner's genuine annotation lives in document B.
	let create_b	= res!(proposal(&doc_b, &owner, 1, "block-1", "owner", 100));
	let on_b		= create_b.record().id();

	// A settlement the owner validly signed in document A, naming that identifier. A relay peer of A
	// lifts it and tries to apply it to B, which -- before the per-document binding -- would have
	// declined and tombstoned the owner's B annotation, unrecoverable.
	let settled_a_env = res!(seal_honest(&doc_a, &owner, 2, Op::Settled {
		on:		on_b,
		state:	Settled::Declined,
		mark:	None,
		time:	200,
	}));
	// Replayed into B, it is refused at the open: its replica is the owner's under A, not under B.
	assert!(sign::open(&settled_a_env, &doc_b).is_err(),
		"a settlement authored in another document must not open here");

	// Belt and braces: opened legitimately for its own document A, then handed to a fold for document
	// B, it is skipped -- not applied -- so the victim's annotation stands.
	let settled_a	= res!(sign::open(&settled_a_env, &doc_a));
	let settled_id	= settled_a.record().id();
	let report		= fold::fold(&[create_b, settled_a], &doc_b, &Keyring::new());
	req!(1, report.annotations.len(), "the victim's annotation survives a cross-document settlement");
	assert!(report.skipped.iter().any(|s| s.id == settled_id),
		"the replayed settlement is reported as skipped, not applied");

	// The same holds for a reply, which also carries no body to stamp.
	let said_a_env = res!(seal_honest(&doc_a, &owner, 3, op::reply(on_b, "a reply from doc A", 210)));
	assert!(sign::open(&said_a_env, &doc_b).is_err(),
		"a reply authored in another document must not open here either");
	Ok(())
}
