//! Folding a document's operation stream into its annotation set, and materialising that set into a
//! local `.prl`.
//!
//! # What the fold is handed
//!
//! Verified, attributed operations -- [`Opened`] values, each a record together with the key that
//! signed it -- not a bare log. The signer is the operation's identity: the author a reader sees comes
//! from the key by way of the document's [`Keyring`], never from a field in the body, which any key
//! could fill with any name. An operation is folded under the document it was stamped for and no other.
//!
//! # Determinism
//!
//! Two replicas that hold the same operations must produce the same annotation set in the same order,
//! however those operations arrived. So the fold does not walk in arrival order: it orders every
//! operation by the author's clock reading and then, for a tie, by operation identifier -- a total
//! order that is the same on every replica because it depends only on the operations themselves. An
//! annotation's place in the result is the place of the proposal that opened it.
//!
//! # What the fold does with each operation
//!
//! - A proposal opens an annotation, keyed by its own identifier, attributed to its signer.
//! - An amendment on that proposal replaces the annotation, but only from the proposal's own signer,
//!   and only while the proposal has not been withdrawn; processed in time order, the last one wins.
//! - A settlement in the `Declined` state, from the proposal's own signer, withdraws the annotation and
//!   tombstones it, so that no later amendment can bring it back; any other state leaves it in.
//! - A reply (`Said`) is threaded discussion, not a standalone annotation, so it does not add here.
//!
//! # Poison-proof
//!
//! A stored operation is forever: an append-only log cannot forget one, so a single malformed but
//! validly-signed operation must never be able to make the whole fold fail for every reader. It cannot.
//! An operation the fold cannot use -- a body that does not decode, a body stamped for another document,
//! an amendment or settlement from someone other than the proposal's signer -- is skipped and reported,
//! never fatal. The fold always returns the annotations it could establish.

use crate::collab::{
	op,
	replica_of,
	sign::Opened,
	DocId,
	Keyring,
};

use oxedyne_fe2o3_austenite::emit::pearl::{
	Annotation,
	PearlDoc,
};
use oxedyne_fe2o3_ore::{
	id::OpId,
	op::{
		Op,
		Settled,
	},
};

use oxedyne_fe2o3_core::prelude::*;

use std::collections::{
	BTreeMap,
	BTreeSet,
};

/// One operation the fold could not use, and why.
///
/// A skip is not an error: the fold went on and produced its result without it. It is reported so a
/// caller can surface a malformed or unauthorised operation rather than have it vanish silently.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Skipped {
	pub id:		OpId,
	pub reason:	String,
}

/// The result of a fold: the annotations that stand, and the operations that were passed over.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct FoldReport {
	pub annotations:	Vec<Annotation>,
	pub skipped:		Vec<Skipped>,
}

/// Folds a document's verified operations into its annotation set, in deterministic order.
///
/// `opened` is every operation of the document, each already verified and bound to its signer by
/// [`super::sign::open`]; `doc` is the document being folded, against which each operation's stamped
/// document identity is checked; `keyring` names the signers for display. The fold never fails on a bad
/// operation: see the module header. The result carries the annotations that stand once every amendment
/// and withdrawal has been applied, ordered by their opening proposal's `(time, id)`, and beside them
/// the operations that were skipped.
pub fn fold(opened: &[Opened], doc: &DocId, keyring: &Keyring) -> FoldReport {
	// Every operation, ordered by the author's clock and then by identifier, so the fold is the same
	// on every replica whatever order the operations were assembled in.
	let mut recs: Vec<&Opened> = opened.iter().collect();
	recs.sort_by(|a, b| {
		op::time_of(&a.record().op).cmp(&op::time_of(&b.record().op))
			.then_with(|| a.record().id().cmp(&b.record().id()))
	});

	// Annotations keyed by their opening proposal's order key, so iterating the map yields them in
	// `(time, id)` order; beside it, the order key and the signing key each proposal identifier maps
	// to, so an amendment or a settlement can find the annotation it acts on and check it is acting on
	// its own author's proposal; and the proposals a withdrawal has tombstoned, which no amendment may
	// resurrect.
	let mut anns:		BTreeMap<(u64, OpId), Annotation>	= BTreeMap::new();
	let mut key_of:		BTreeMap<OpId, (u64, OpId)>			= BTreeMap::new();
	let mut signer_of:	BTreeMap<OpId, Vec<u8>>				= BTreeMap::new();
	let mut tombstoned:	BTreeSet<OpId>						= BTreeSet::new();
	let mut skipped:	Vec<Skipped>						= Vec::new();

	for opened in recs {
		let rec		= opened.record();
		let id		= rec.id();
		let signer	= opened.signer();
		// Belt and braces to the ingest check: an [`Opened`] is meant to have been opened for this
		// document, but the fold re-derives the binding rather than trust that it was, so an operation
		// from another document -- a settlement or reply lifted across, which carries no body stamp --
		// is skipped here even if it reached the set unopened for this document.
		if rec.id().replica != replica_of(signer, doc) {
			skipped.push(Skipped {
				id,
				reason: fmt!("its replica {} is not the one its signer derives under document {}; it \
					belongs to another document and is not folded here", rec.id().replica, doc),
			});
			continue;
		}
		match &rec.op {
			Op::Proposal { body, .. } => {
				let ann = match decode_for_doc(body, doc) {
					Ok(ann)		=> ann,
					Err(reason)	=> {
						skipped.push(Skipped { id, reason });
						continue;
					},
				};
				let key = (op::time_of(&rec.op), id);
				key_of.insert(id, key);
				signer_of.insert(id, signer.to_vec());
				anns.insert(key, attribute(ann, signer, keyring));
			},
			Op::Amended { on, body, .. } => {
				// An amendment on a proposal this fold has not seen names content outside what it
				// holds; it is left for a later, causally complete fold rather than guessed at.
				let key = match key_of.get(on) {
					Some(key)	=> *key,
					None		=> continue,
				};
				if tombstoned.contains(on) {
					skipped.push(Skipped {
						id,
						reason: fmt!("amends the withdrawn proposal {}, which a withdrawal tombstones \
							against resurrection", on),
					});
					continue;
				}
				// Only the proposal's own signer may restate it. An amendment from any other key is a
				// stranger editing someone else's annotation, and is refused.
				if signer_of.get(on).map(|s| s.as_slice()) != Some(signer) {
					skipped.push(Skipped {
						id,
						reason: fmt!("amends the proposal {}, whose signer it does not match; an \
							amendment is accepted only from the proposal's own signer", on),
					});
					continue;
				}
				let ann = match decode_for_doc(body, doc) {
					Ok(ann)		=> ann,
					Err(reason)	=> {
						skipped.push(Skipped { id, reason });
						continue;
					},
				};
				anns.insert(key, attribute(ann, signer, keyring));
			},
			Op::Settled { on, state, .. } => {
				let key = match key_of.get(on) {
					Some(key)	=> *key,
					None		=> continue,
				};
				// Only the proposal's own signer may settle it, for the reason an amendment is so
				// confined: a stranger does not get to withdraw or resolve another author's annotation.
				if signer_of.get(on).map(|s| s.as_slice()) != Some(signer) {
					skipped.push(Skipped {
						id,
						reason: fmt!("settles the proposal {}, whose signer it does not match; a \
							settlement is accepted only from the proposal's own signer", on),
					});
					continue;
				}
				if matches!(state, Settled::Declined) {
					anns.remove(&key);
					tombstoned.insert(*on);
				}
			},
			// A reply or any non-annotation operation adds nothing to the materialised set.
			_ => {},
		}
	}

	FoldReport {
		annotations:	anns.into_values().collect(),
		skipped,
	}
}

/// Decodes an annotation body under the collaboration decode bounds and checks it was stamped for
/// `doc`, returning a reason string on either failure so the fold can skip and report it.
///
/// A body stamped for another document, or for none, is a cross-document replay: a signed annotation
/// lifted from document A and appended to document B, where the same block address happens to exist.
/// Binding the fold to the stamped identity refuses it.
fn decode_for_doc(body: &[u8], doc: &DocId) -> std::result::Result<Annotation, String> {
	let ann = match op::annotation_from_body(body) {
		Ok(ann)	=> ann,
		Err(e)	=> return Err(fmt!("its body does not decode as an annotation: {}", e)),
	};
	match &ann.doc_id {
		Some(id) if id == doc.as_str()	=> Ok(ann),
		Some(id)						=> Err(fmt!(
			"its body is stamped for document {:?}, not {:?}; it will not be folded here", id, doc.as_str())),
		None							=> Err(fmt!(
			"its body carries no document identity, so it cannot be confirmed to belong to {:?}",
			doc.as_str())),
	}
}

/// Sets an annotation's displayed author from the key that signed it, by way of the keyring, so the
/// author a reader sees is the signer and never whatever the body claimed.
fn attribute(mut ann: Annotation, signer: &[u8], keyring: &Keyring) -> Annotation {
	ann.author = keyring.display(signer);
	ann
}

/// Attaches a folded annotation set to a decoded `.prl`, through
/// [`PearlDoc::add_annotation`](oxedyne_fe2o3_austenite::emit::pearl::PearlDoc::add_annotation).
///
/// An annotation whose anchor block is not in this particular document is skipped rather than
/// refused: a fold drawn from the shared stream may carry annotations on content a given local file
/// does not hold, and those simply have nothing to attach to here. The count returned is how many were
/// attached.
pub fn materialise(doc: &mut PearlDoc, anns: &[Annotation]) -> Outcome<usize> {
	let mut attached = 0;
	for ann in anns {
		if res!(doc.has_block(&ann.anchor)) {
			res!(doc.add_annotation(ann.clone()));
			attached += 1;
		}
	}
	Ok(attached)
}
