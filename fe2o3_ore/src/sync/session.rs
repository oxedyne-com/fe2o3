//! The driver: one message in, some messages out, and where things stand.
//!
//! A session holds no log and no connection. The caller passes the log it wants
//! brought up to date on each call, reads the messages the session hands back,
//! and puts them wherever messages go. Looping that over a pipe is the whole of
//! a sync:
//!
//! ```text
//! let mut s = Session::new(Mode::Walk);
//! send(res!(s.open(&log)));
//! while !s.is_converged() {
//!     let turn = res!(s.receive(&mut log, recv()));
//!     for msg in turn.send { send(msg); }
//! }
//! ```
//!
//! # Either peer may start
//!
//! A session that is handed an opening before it has made one answers with its
//! own, so a peer that was called upon needs no separate path: it constructs a
//! session and feeds it what arrived. Two peers that both open at once do not
//! open twice. There is no client and no server here, only two sessions running
//! the same code.
//!
//! # Convergence
//!
//! A session is converged when it has said everything it owes and heard the
//! other side say the same. That is a statement about the conversation. It means
//! the logs agree because the owed set was computed honestly at both ends, which
//! the closure check on arrival is what enforces: a peer that sends a batch with
//! a hole in it has its batch refused whole, and the session errs rather than
//! absorbing part of it.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use crate::id::OpId;
use crate::log::OpLog;
use crate::sync::msg::Message;
use crate::sync::sketch::{
	reconcile,
	sketch_bytes,
	Diff,
	Fallback,
	SEED,
};
use crate::sync::walk::{
	arrival_gap,
	close,
	covered,
	entries_for,
	owed,
};

use oxedyne_fe2o3_core::prelude::*;

use std::collections::BTreeSet;


/// How a session opens, and how it works out what it owes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Mode {
	// Exchange frontiers, and send everything the other's frontier does not
	// cover.  Correct at any divergence, and loose where both sides have written.
	Walk,
	// Exchange sketches, and send the difference.  Falls back to the walk, in the
	// same turn, where the sketch turns out to have been too small: an estimate
	// too low costs a fallback, one too high costs sketch bytes.  A receiver
	// adopts the seed it is sent, so the seed decides only what this peer opens
	// with, and SEED is the answer where the caller has no reason to prefer
	// another.
	Sketch {
		estimate:	usize,	// operations the two logs are guessed to differ by
		seed:		u64,	// the seed both peers' tables are built under
	},
}

// How many operations a head of a frontier is taken to stand for when the
// difference between two logs is being guessed at.  A log that has diverged
// carries more than one head, and each head is a branch somebody wrote since the
// two last spoke.  How much they wrote is exactly what nobody knows, so this is a
// guess with a fallback under it.
pub const FANOUT: usize = 8;

impl Mode {

	/// Under the usual seed.
	pub fn sketch(estimate: usize) -> Self {
		Self::Sketch { estimate, seed: SEED }
	}

	/// The guess at the difference is the two logs' difference in length, which
	/// is a lower bound on how far apart they are, plus [`FANOUT`] for every head
	/// either frontier carries, which is the part that stands for concurrent
	/// writing. Where the guess comes to as much as the smaller log, sketching is
	/// pointless -- a sketch sized for the whole history costs more than the
	/// history -- and the walk is both cheaper and exact. That covers the clone
	/// case, where one log is empty, and the case of two logs that share nothing.
	///
	/// A guess of nothing means two empty logs, which the walk settles in one
	/// message; a sketch of nothing would be a table nobody needs.
	///
	/// Both peers may call this and neither has to: the modes need not agree,
	/// since a session answers whatever it is given.
	pub fn between(
		here_len:		usize,	// operations this peer holds
		here_heads:		usize,	// heads of this peer's frontier
		there_len:		usize,	// operations the other peer holds
		there_heads:	usize,	// heads of the other peer's frontier
	)
		-> Self
	{
		let spread = here_len.abs_diff(there_len) + FANOUT * (here_heads + there_heads);
		if spread == 0 || spread >= here_len.min(there_len).max(1) {
			Self::Walk
		} else {
			Self::sketch(spread)
		}
	}
}


/// Where a session stands after taking a message.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Step {
	Converged,	// both sides have said everything they owe, so the logs agree
	NeedMore,	// more is expected from the other side
	// A sketch could not be decoded, so the walk answered instead.  The exchange
	// carries on and will converge; the reason is worth a caller's attention only
	// in that it says the estimate was low.
	FellBack(Fallback),
}


/// What a session hands back: what to send, and where things stand.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Turn {
	pub send:	Vec<Message>,	// to put on the wire, in order
	pub step:	Step,			// where the session stands now
}


/// One side of an exchange.
#[derive(Clone, Debug)]
pub struct Session {
	mode:		Mode,				// how this side opens and works out what it owes
	opened:		bool,				// an opening has been made
	told:		bool,				// everything owed has been sent
	heard:		bool,				// the other side has said it is finished
	fell_back:	Option<Fallback>,	// the first fallback, kept for the asking
	sent:		usize,				// operations handed over
	absorbed:	usize,				// operations absorbed
}

impl Session {

	pub fn new(mode: Mode) -> Self {
		Self {
			mode,
			opened:		false,
			told:		false,
			heard:		false,
			fell_back:	None,
			sent:		0,
			absorbed:	0,
		}
	}

	pub fn mode(&self) -> Mode {
		self.mode
	}

	/// Have both sides finished, which is when the logs agree?
	pub fn is_converged(&self) -> bool {
		self.told && self.heard
	}

	/// The first fallback, and sticky, so a caller can ask once at the end rather
	/// than watching every turn.
	pub fn fell_back(&self) -> Option<Fallback> {
		self.fell_back
	}

	pub fn ops_sent(&self) -> usize {
		self.sent
	}

	pub fn ops_absorbed(&self) -> usize {
		self.absorbed
	}

	/// A session that is fed an opening before it has made one opens on its own
	/// account, so calling this is a choice about who speaks first and not a
	/// requirement.
	pub fn open(&mut self, log: &OpLog)
		-> Outcome<Message>
	{
		self.opened = true;
		match self.mode {
			Mode::Walk => Ok(Message::hello(log.frontier())),
			Mode::Sketch { estimate, seed } => Ok(Message::sketch(
				log.frontier(),
				res!(sketch_bytes(log, estimate, seed)),
				log.len() as u64,
			)),
		}
	}

	/// A [`Message::Send`] is checked for causal closure against the log before
	/// anything is absorbed, and refused whole if it has a hole in it. Operations
	/// the log already holds, and repetitions within one batch, are dropped
	/// rather than refused: a peer that could not subtract one of our heads sends
	/// more than it needs to, and that is a cost rather than a fault.
	pub fn receive(&mut self, log: &mut OpLog, msg: Message)
		-> Outcome<Turn>
	{
		match msg {
			Message::Hello { heads } => self.answer(log, &heads, None),
			Message::Sketch { heads, cells, .. } => self.answer(log, &heads, Some(&cells)),
			Message::Send { entries } => {
				if let Some((id, parent)) = res!(arrival_gap(log, &entries)) {
					return Err(err!(
						"An arriving batch of {} operation{} names the parent {} of {}, \
						which neither the batch nor the log holds; a peer sends causal \
						closures and not subsets.", entries.len(),
						if entries.len() == 1 { "" } else { "s" }, parent, id;
					Invalid, Input, Missing, Order));
				}
				let mut batch = Vec::with_capacity(entries.len());
				let mut taken: BTreeSet<OpId> = BTreeSet::new();
				for entry in &entries {
					let rec = res!(entry.peek());
					let id = rec.id();
					if !log.contains(&id) && taken.insert(id) {
						batch.push(rec);
					}
				}
				let placed = batch.len();
				let left = res!(log.absorb(batch));
				self.absorbed += placed - left.len();
				if !left.is_empty() {
					return Err(err!(
						"An arriving batch closed causally and yet left {} operation{} \
						unplaced, starting at {}.", left.len(),
						if left.len() == 1 { "" } else { "s" }, left[0].id();
					Bug, Unreachable));
				}
				Ok(self.turn(Vec::new(), None))
			},
			Message::Done => {
				self.heard = true;
				Ok(self.turn(Vec::new(), None))
			},
		}
	}

	/// Answers an opening: what we owe, and our own opening if we have not made
	/// one.
	fn answer(&mut self, log: &OpLog, heads: &[OpId], cells: Option<&[u8]>)
		-> Outcome<Turn>
	{
		let mut out = Vec::new();
		if !self.opened {
			out.push(res!(self.open(log)));
		}
		// What we owe, and what we believe they hold, which is what the send set
		// is closed against.
		let mut fallback: Option<Fallback> = None;
		let (send, held) = match cells {
			Some(bytes) => match res!(reconcile(log, bytes)) {
				Diff::Decoded { local_only, .. } => {
					// Everything we hold that is not ours alone, they hold too.
					let owed_set: BTreeSet<OpId> = local_only.iter().copied().collect();
					let held: BTreeSet<OpId> = log.iter()
						.map(|rec| rec.id())
						.filter(|id| !owed_set.contains(id))
						.collect();
					(local_only, held)
				},
				Diff::Undecodable(reason) => {
					fallback = Some(reason);
					if self.fell_back.is_none() {
						self.fell_back = Some(reason);
					}
					(owed(log, heads), covered(log, heads))
				},
			},
			None => (owed(log, heads), covered(log, heads)),
		};
		let ids = close(log, &send, &held);
		if !ids.is_empty() {
			let entries = res!(entries_for(log, &ids));
			self.sent += entries.len();
			out.push(Message::Send { entries });
		}
		out.push(Message::Done);
		self.told = true;
		Ok(self.turn(out, fallback))
	}

	/// A fallback made on this turn is reported ahead of anything else.
	fn turn(&self, send: Vec<Message>, fallback: Option<Fallback>) -> Turn {
		let step = match fallback {
			Some(reason) => Step::FellBack(reason),
			None => if self.is_converged() {
				Step::Converged
			} else {
				Step::NeedMore
			},
		};
		Turn { send, step }
	}
}
