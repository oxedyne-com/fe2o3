//! Folding a document's operation stream into its annotation set, and materialising that set into a
//! local `.prl`.
//!
//! # Determinism
//!
//! Two replicas that hold the same operations must produce the same annotation set in the same order,
//! however those operations arrived. So the fold does not walk the log in append order: it orders every
//! operation by the author's clock reading and then, for a tie, by operation identifier -- a total
//! order that is the same on every replica because it depends only on the operations themselves, not on
//! which was received first. An annotation's place in the result is the place of the proposal that
//! opened it.
//!
//! # What the fold does with each operation
//!
//! - A proposal opens an annotation, keyed by its own identifier.
//! - An amendment on that proposal replaces the annotation; processed in time order, the last one wins.
//! - A settlement in the `Declined` state withdraws the annotation, so it drops out of the set; any
//!   other state leaves it in, resolved but present.
//! - A reply (`Said`) is threaded discussion, not a standalone annotation, so it does not add to the
//!   materialised set here; carrying replies into a threaded view is the next increment's concern.

use crate::collab::op;

use oxedyne_fe2o3_austenite::emit::pearl::{
	Annotation,
	PearlDoc,
};
use oxedyne_fe2o3_ore::{
	id::OpId,
	log::OpLog,
	op::{
		Op,
		Settled,
	},
};

use oxedyne_fe2o3_core::prelude::*;

use std::collections::BTreeMap;

/// Folds a document's operation log into its annotation set, in deterministic order.
///
/// The log is the whole of a document's edit stream, however assembled -- see
/// [`super::hub`] for loading one out of the store. The result is the annotations that stand once
/// every amendment and withdrawal has been applied, ordered by their opening proposal's `(time, id)`.
pub fn fold(log: &OpLog) -> Outcome<Vec<Annotation>> {
	// Every operation, ordered by the author's clock and then by identifier, so the fold is the same
	// on every replica whatever order the log was assembled in.
	let mut recs: Vec<_> = log.iter().collect();
	recs.sort_by(|a, b| {
		op::time_of(&a.op).cmp(&op::time_of(&b.op))
			.then_with(|| a.head.id().cmp(&b.head.id()))
	});

	// Annotations keyed by their opening proposal's order key, so iterating the map yields them in
	// `(time, id)` order; and, beside it, the order key each proposal identifier maps to, so an
	// amendment or a settlement can find the annotation it acts on.
	let mut anns:	BTreeMap<(u64, OpId), Annotation>	= BTreeMap::new();
	let mut key_of:	BTreeMap<OpId, (u64, OpId)>			= BTreeMap::new();

	for rec in recs {
		match &rec.op {
			Op::Proposal { body, .. } => {
				let key = (op::time_of(&rec.op), rec.head.id());
				key_of.insert(rec.head.id(), key);
				anns.insert(key, res!(op::annotation_from_body(body)));
			},
			Op::Amended { on, body, .. } => {
				// An amendment on a proposal this fold has not seen names content outside the log; it
				// is left for a later, causally complete fold rather than guessed at here.
				if let Some(key) = key_of.get(on) {
					anns.insert(*key, res!(op::annotation_from_body(body)));
				}
			},
			Op::Settled { on, state, .. } => {
				if let Some(key) = key_of.get(on) {
					if matches!(state, Settled::Declined) {
						anns.remove(key);
					}
				}
			},
			// A reply or any non-annotation operation adds nothing to the materialised set.
			_ => {},
		}
	}

	Ok(anns.into_values().collect())
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
