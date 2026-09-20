//! Mapping an annotation's lifecycle onto the Ore operation vocabulary.
//!
//! The operation log already carries a threaded-discussion vocabulary -- a proposal, replies to it,
//! amendments of it, and a settlement of it -- and an annotation thread is exactly that shape. So no
//! new operation kind is minted: an annotation is a proposal whose body is the annotation itself, and
//! its later life is the operations that already speak about a proposal by name.
//!
//! - **create** -> [`Op::Proposal`], body the annotation.
//! - **reply**  -> [`Op::Said`] naming the opening proposal.
//! - **edit**   -> [`Op::Amended`] naming it, body the new annotation.
//! - **resolve**/  **withdraw** -> [`Op::Settled`] naming it, `Done` or `Declined`.
//!
//! Each is wrapped in a record with a header (identity and parents) by its author and sealed by
//! [`super::sign`]; this module supplies only the operation payloads and the decoding back out.

use oxedyne_fe2o3_austenite::emit::pearl::Annotation;
use oxedyne_fe2o3_ore::{
	id::OpId,
	op::{
		Op,
		Settled,
	},
};

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_jdat::prelude::*;

/// The forge voice annotation operations are written under, so the log can tell a Pearlite annotation
/// thread from any other proposal it might come to carry.
pub const VOICE: &str = "pearlite";

/// Encodes an annotation as the body bytes an operation carries: its [`ToDat`] form in canonical
/// binary daticle, the same encoding the store and the envelope speak.
fn annotation_body(ann: &Annotation) -> Outcome<Vec<u8>> {
	Ok(res!(res!(ann.to_dat()).to_bytes(Vec::new())))
}

/// Decodes an annotation from an operation body's bytes, the inverse of what [`create`] and [`edit`]
/// store.
pub fn annotation_from_body(body: &[u8]) -> Outcome<Annotation> {
	let (dat, _) = res!(Dat::from_bytes(body));
	Annotation::from_dat(dat)
}

/// A new annotation opens a thread: an [`Op::Proposal`] whose body is the annotation and whose title
/// is the block it hangs on, so a reader listing proposals sees what each is anchored to without
/// decoding the body.
pub fn create(ann: &Annotation, time: u64) -> Outcome<Op> {
	Ok(Op::Proposal {
		title:	ann.anchor.clone(),
		body:	res!(annotation_body(ann)),
		voice:	VOICE.to_string(),
		time,
	})
}

/// A reply to an annotation thread: an [`Op::Said`] naming the opening proposal `on`.
pub fn reply(on: OpId, text: &str, time: u64) -> Op {
	Op::Said {
		on,
		text:	text.as_bytes().to_vec(),
		voice:	VOICE.to_string(),
		time,
	}
}

/// An edit restates the annotation: an [`Op::Amended`] naming the opening proposal `on`, whose body is
/// the new annotation. The fold takes the latest amendment in time order.
pub fn edit(on: OpId, ann: &Annotation, time: u64) -> Outcome<Op> {
	Ok(Op::Amended {
		on,
		title:	ann.anchor.clone(),
		body:	res!(annotation_body(ann)),
		voice:	VOICE.to_string(),
		time,
	})
}

/// Resolving an annotation marks its thread done: an [`Op::Settled`] in the `Done` state. The
/// annotation stays in a fold -- it is resolved, not gone.
pub fn resolve(on: OpId, mark: Option<OpId>, time: u64) -> Op {
	Op::Settled { on, state: Settled::Done, mark, time }
}

/// Withdrawing an annotation declines its thread: an [`Op::Settled`] in the `Declined` state, which a
/// fold reads as a removal.
pub fn withdraw(on: OpId, mark: Option<OpId>, time: u64) -> Op {
	Op::Settled { on, state: Settled::Declined, mark, time }
}

/// The author's clock reading an operation carries, for the operations that carry one; the fold orders
/// by it. An operation with no time of its own -- none of the annotation operations -- reads as zero.
pub fn time_of(op: &Op) -> u64 {
	match op {
		Op::Proposal { time, .. }	=> *time,
		Op::Said { time, .. }		=> *time,
		Op::Settled { time, .. }	=> *time,
		Op::Amended { time, .. }	=> *time,
		_							=> 0,
	}
}
