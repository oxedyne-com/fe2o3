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
	cells_for,
	cells_in,
	estimate_for,
	grown,
	reconcile,
	sketch_bytes,
	Diff,
	Fallback,
	GROW_CELLS,
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

	/// The guess at the difference, and what it rests on is which of the other
	/// peer's heads this log does not hold.
	///
	/// **A head of theirs we hold is the whole answer, not a term in it.** A log
	/// is causally closed and its frontier dominates it, so holding every head of
	/// theirs means holding everything they hold: the difference is then exactly
	/// the two logs' difference in length and nothing stands for branches that are
	/// not there. That is the pull, the clone and the fetch after somebody else
	/// pushed, and it is the common case.
	///
	/// A head of theirs we do not hold says the divergence is two-sided, and a
	/// length delta then measures nothing: two logs that have each written since
	/// they last spoke can be the same length and share almost nothing. So each
	/// frontier's heads are taken to stand for [`FANOUT`] operations apiece and the
	/// delta rides on top of that, being evidence that one of the two tails is
	/// longer than the other.
	///
	/// It is still a guess, and a guess that is low costs a round trip rather than
	/// a history: [`crate::sync::sketch::grown`] is what a stalled decode reaches
	/// for before the walk.
	///
	/// Where the guess comes to as much as the smaller log, sketching is pointless
	/// -- a sketch sized for the whole history costs more than the history -- and
	/// the walk is both cheaper and exact. That covers the clone case, where one
	/// log is empty, and the case of two logs that share nothing.
	///
	/// A guess of nothing means one log holds what the other does, which the walk
	/// settles in one message; a sketch of nothing would be a table nobody needs.
	///
	/// Both peers may call this and neither has to: the modes need not agree,
	/// since a session answers whatever it is given.
	///
	/// # Arguments
	///
	/// * `unknown` - Heads of the other peer's frontier this log does not hold. A
	///   caller who has their count and not their names has to pass that count,
	///   which is the reading that assumes the worst.
	pub fn between(
		here_len:	usize,	// operations this peer holds
		here_heads:	usize,	// heads of this peer's frontier
		there_len:	usize,	// operations the other peer holds
		unknown:	usize,	// heads of theirs this peer does not hold
	)
		-> Self
	{
		let delta = here_len.abs_diff(there_len);
		let spread = match unknown {
			0	=> delta,
			_	=> delta + FANOUT * (here_heads + unknown),
		};
		if spread == 0 || spread >= here_len.min(there_len).max(1) {
			Self::Walk
		} else {
			Self::sketch(spread)
		}
	}
}


/// Whether this end takes part in growing a sketch.
///
/// Three behaviours and one switch, because they are one mechanism: a stalled
/// decode is answered with a larger table, an arriving table wider than the last
/// one sent is answered by opening again at it, and the widest shape met is
/// carried into the session after ([`Session::sizing`]). Refusing turns off all
/// three, which is a peer built before any of it -- and that is the second thing
/// this is for, since a carrier's older-peer behaviour can then be exercised
/// against a real session rather than reasoned about.
///
/// **What it is set FROM is a fact about the other end: whether it re-opens.** A
/// grown table is a question, and the answer to it is the peer opening again at
/// the new shape; a peer that does not takes the grown table, answers what it
/// owes, and is never given what it is owed, because the end that grew said
/// nothing else and the exchange has nowhere to go. So an end that cannot tell
/// refuses, and the walk answers instead, which every peer there has ever been
/// understands.
///
/// The two ends need not agree, and in the ordinary case they do not: a client
/// grows freely against a carrier that opens afresh on every request -- and so
/// always answers an opening -- while that carrier grows only against a client
/// that told it what it speaks.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Growth {
	// A stalled decode is answered with a larger table, a wider table with a
	// wider answer, and the width reached is carried into the next session.
	Allowed,
	// None of the three.  Both what a peer that opens once must be answered with,
	// and what a build from before any of this does.
	Refused,
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
	// A sketch could not be decoded and a larger one was answered with, rather
	// than the walk.  Nothing was handed over on this turn: the answer is the
	// bigger table alone, and what is owed crosses once a table decodes.
	Grew {
		cells: usize,	// cells the table this end answered with declares
	},
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
	known:		BTreeSet<OpId>,		// operations the far end has shown it holds
	sent:		usize,				// operations handed over
	absorbed:	usize,				// operations absorbed
	offered:	usize,				// cells of the widest table this end has sent
	widest:		usize,				// cells of the widest table met, sent or received
	growth:		Growth,				// whether a stalled decode may be answered with a larger table
}

impl Session {

	pub fn new(mode: Mode) -> Self {
		Self::knowing(mode, BTreeSet::new())
	}

	/// A session that starts knowing the far end already holds `known`.
	///
	/// The one thing a session boundary would otherwise throw away. No message says
	/// "you handed me this": a frontier reports what its own end holds, and a head
	/// nobody here has seen subtracts nothing, so a fresh session works out what it
	/// owes from the frontier alone and offers back everything the sessions before
	/// it took in. A carrier that runs several sessions -- which a bounded reply
	/// makes it do -- hands one session's [`Session::known`] to the next, and the
	/// re-offer stops.
	///
	/// It is a proof and not a claim. Every identifier in it is one this end watched
	/// arrive, or one the carrier watched leave and be taken; nothing in it rests on
	/// what a peer says about itself, which is what makes it safe to subtract.
	pub fn knowing(mode: Mode, known: BTreeSet<OpId>) -> Self {
		Self {
			mode,
			opened:		false,
			told:		false,
			heard:		false,
			fell_back:	None,
			known,
			sent:		0,
			absorbed:	0,
			offered:	0,
			widest:		0,
			growth:		Growth::Allowed,
		}
	}

	/// A session that takes no part in growing a sketch.
	///
	/// For a caller that knows what the far end speaks, which over a carrier is
	/// the carrier's business and not this module's. See [`Growth`].
	pub fn with_growth(mut self, growth: Growth) -> Self {
		self.growth = growth;
		self
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

	/// The mode a session following this one should open in.
	///
	/// The other thing a session boundary would otherwise throw away, and it is
	/// [`Session::knowing`]'s companion. A table that had to be grown to before
	/// anything decoded is the shape that works, and a carrier running several
	/// sessions -- which a bounded reply makes it do -- would otherwise open each
	/// of them at the estimate the first one was handed, stall, and grow again.
	///
	/// A walk stays a walk. It was chosen because the difference was judged to be
	/// as large as the history, and nothing a sketch did in passing changes that.
	/// So does a session that refuses growth, which is one that behaves as a build
	/// from before any of this: it opens every session at the estimate it was
	/// handed.
	pub fn sizing(&self) -> Mode {
		if self.growth == Growth::Refused {
			return self.mode;
		}
		match self.mode {
			Mode::Walk => Mode::Walk,
			Mode::Sketch { estimate, seed } => {
				// At the same ceiling growth climbs to, and for the same reason: a
				// table crosses in one message and nothing cuts one up, so a shape
				// carried into the next session is a shape that has to fit the body
				// this end posts.  A peer may open wider than this end would have
				// chosen -- MAX_CELLS is what a reader will allocate, not what a
				// carrier will take -- and that is where an uncapped carry would put
				// an oversized request on the wire.
				let want = self.widest.min(GROW_CELLS);
				match want > cells_for(estimate) {
					true	=> Mode::Sketch { estimate: estimate_for(want), seed },
					false	=> self.mode,
				}
			},
		}
	}

	/// What the far end has shown it holds: what this session was told when it
	/// opened, and every operation that has arrived in a [`Message::Send`] since.
	///
	/// Not what it was sent. Whether a batch reached the far end is a question about
	/// the carrier and not about the session, so a carrier that knows its request
	/// was taken puts those identifiers in itself.
	pub fn known(&self) -> &BTreeSet<OpId> {
		&self.known
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
			Mode::Sketch { estimate, seed } => {
				self.offered = self.offered.max(cells_for(estimate));
				self.widest = self.widest.max(self.offered);
				Ok(Message::sketch(
					log.frontier(),
					res!(sketch_bytes(log, estimate, seed)),
					log.len() as u64,
				))
			},
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
			Message::Hello { heads } => self.answer(log, &heads, None, 0),
			Message::Sketch { heads, cells, count } =>
				self.answer(log, &heads, Some(&cells), count as usize),
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
					// Written down whether or not it is news here. An operation a peer
					// hands over is an operation that peer holds, and that is the whole of
					// what a later session needs to stop offering it back.
					self.known.insert(id);
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
			// A piece of an operation is not an operation, and a session that
			// tried to place one would be placing something nobody signed. Putting
			// the pieces back together is the carrier's work -- `Parts` in
			// `sync::msg` -- and a session is handed the result.
			Message::Part { id, seq, total, .. } => Err(err!(
				"A session was handed piece {} of the {} the operation {} was cut \
				into. A piece is a transport's business: `sync::msg::Parts` puts a \
				run of them back into the send it was cut from, and that is what a \
				session receives.", seq, total, id;
			Invalid, Input, Mismatch)),
			// How far the far end was carried into what this log last owed it.
			// Written down as operations it holds, because that is what it is: the
			// prefix is subtracted by the same walk that subtracts a head, and a
			// carrier that keeps nothing between sessions is thereby told where to
			// carry on from without being told what it already sent.
			Message::Resume { at } => {
				self.resume(log, &at);
				Ok(self.turn(Vec::new(), None))
			},
			// A replacement names operations the receiver already holds and asks
			// for the form they are held in to change, which is a fact about a
			// store and not about a set of operations. A session reconciles sets,
			// so it neither sends this nor answers it; a carrier acts on it once
			// the session it arrived beside has finished.
			Message::Forgotten { entries } => Err(err!(
				"A session was handed {} record{} to write in place of operations it \
				already holds. What a forget takes out of a store is the carrier's \
				work -- `Message::replacements` is what hands them over -- and a \
				session places operations and never rewrites one.",
				entries.len(), if entries.len() == 1 { "" } else { "s" };
			Invalid, Input, Mismatch)),
		}
	}

	/// Answers an opening: what we owe, and our own opening if we have not made
	/// one -- or if the one we made was narrower than the table we are answering.
	fn answer(&mut self, log: &OpLog, heads: &[OpId], cells: Option<&[u8]>, theirs: usize)
		-> Outcome<Turn>
	{
		// The shape the other end sketched under, read out of the table's header
		// rather than out of the table.
		let arriving = match cells {
			Some(bytes)	=> res!(cells_in(bytes)),
			None		=> 0,
		};
		self.widest = self.widest.max(arriving);
		let taken = match cells {
			Some(bytes)	=> Some(res!(reconcile(log, bytes))),
			None		=> None,
		};
		// A stall is answered with a larger table before it is answered with the
		// walk. What the walk would say here is the whole log: a peer whose head
		// this end does not hold has nothing subtracted by it, and handing over a
		// history to save a round trip is the trade the sketch exists to refuse.
		// Nothing is owed on this turn and nothing is said to be finished; the
		// bigger table goes on its own and what it decodes crosses next.
		if let Some(Diff::Undecodable(Fallback::Incomplete { .. })) = &taken {
			if let Some(turn) = res!(self.growing(log, theirs)) {
				return Ok(turn);
			}
		}
		// Never a narrower table than the one it answers. Both peers build under
		// the arriving shape, so a shape this end could decode is one the other
		// end can decode too, and answering it with less would stall over there
		// and cost the round trip this turn just saved.
		self.widen(arriving);
		let mut out = Vec::new();
		if !self.opened || (self.growth == Growth::Allowed && arriving > self.offered) {
			out.push(res!(self.open(log)));
		}
		// What we owe, and what we believe they hold, which is what the send set
		// is closed against.
		let mut fallback: Option<Fallback> = None;
		let (send, held) = match taken {
			Some(Diff::Decoded { local_only, .. }) => {
				// Everything we hold that is not ours alone, they hold too.
				let owed_set: BTreeSet<OpId> = local_only.iter().copied().collect();
				let held: BTreeSet<OpId> = log.iter()
					.map(|rec| rec.id())
					.filter(|id| !owed_set.contains(id))
					.collect();
				(local_only, held)
			},
			Some(Diff::Undecodable(reason)) => {
				fallback = Some(reason);
				if self.fell_back.is_none() {
					self.fell_back = Some(reason);
				}
				(owed(log, heads, &self.known), covered(log, heads, &self.known))
			},
			None => (owed(log, heads, &self.known), covered(log, heads, &self.known)),
		};
		let ids = close(log, &send, &held);
		if !ids.is_empty() {
			let entries = res!(entries_for(log, &ids));
			self.sent += entries.len();
			out.push(Message::Send { entries });
		}
		out.push(Message::Done);
		self.told = true;
		Ok(self.turn(out, fallback.map(Step::FellBack)))
	}

	/// Opens again with a table of twice the cells, where there is one worth
	/// having.
	///
	/// `None` is every reason not to, and each of them leaves the walk to answer,
	/// which it always can. The far end does not re-open, so a grown table would
	/// be the last thing said -- see [`Growth`], and it is the reason this is a
	/// knob at all. This end is walking anyway, so a sketch is not what it chose.
	/// The double would pass [`crate::sync::sketch::GROW_CELLS`], which is what a
	/// table has to cross in one message inside. Or the size it would climb to is
	/// one [`Mode::between`] would not have chosen in the first place: a table
	/// sized for as much as the smaller of the two logs costs more than that log,
	/// and growth is what buys time against the walk rather than a substitute for
	/// it.
	///
	/// Every growth at least doubles, so the climb is geometric and each of those
	/// three bounds is reached in a handful of turns. A session after this one
	/// opens at [`Session::sizing`], so a bounded exchange keeps the size it
	/// reached rather than starting again at the first estimate.
	///
	/// # Arguments
	///
	/// * `theirs` - Operations the other peer says it holds. It decides nothing
	///   but how far this climbs, and a peer that overstates it is bounded by this
	///   log's own length.
	fn growing(&mut self, log: &OpLog, theirs: usize)
		-> Outcome<Option<Turn>>
	{
		if self.growth == Growth::Refused {
			return Ok(None);
		}
		let seed = match self.mode {
			Mode::Sketch { seed, .. }	=> seed,
			Mode::Walk					=> return Ok(None),
		};
		let estimate = match grown(self.widest) {
			Some(e)	=> e,
			None	=> return Ok(None),
		};
		if estimate >= log.len().min(theirs).max(1) {
			return Ok(None);
		}
		self.mode = Mode::Sketch { estimate, seed };
		let msg = res!(self.open(log));
		Ok(Some(self.turn(vec![msg], Some(Step::Grew { cells: self.offered }))))
	}

	/// Raises this end's estimate so that the table it opens with is at least
	/// `cells` wide, up to [`crate::sync::sketch::GROW_CELLS`], which is what a
	/// carrier will take in one body.
	///
	/// A walk is left alone: it was chosen because the difference was judged to be
	/// as large as the history. So is a session that refuses growth, which is one
	/// that does not open twice.
	fn widen(&mut self, cells: usize) {
		if self.growth == Growth::Refused {
			return;
		}
		if let Mode::Sketch { estimate, seed } = self.mode {
			let want = cells.min(GROW_CELLS);
			if cells_for(estimate) < want {
				self.mode = Mode::Sketch { estimate: estimate_for(want), seed };
			}
		}
	}

	/// Writes down that the far end holds every operation of this log up to and
	/// including `at`, in the log's append order.
	///
	/// Sound because of what the far end can only have come by it: everything
	/// this end sent it went in append order, and everything before it that was
	/// not sent was subtracted as already held. An append-order prefix is closed
	/// downwards by the log's own append guard, so what is written down here is
	/// closed downwards too, which is what makes it safe to subtract.
	///
	/// An identifier this log does not hold says nothing and is dropped, exactly
	/// as a head nobody here has seen is.
	fn resume(&mut self, log: &OpLog, at: &OpId) {
		let end = match log.position(at) {
			Some(p)	=> p,
			None	=> return,
		};
		for rec in log.iter().take(end + 1) {
			self.known.insert(rec.id());
		}
	}

	/// A step made on this turn is reported ahead of anything else.
	fn turn(&self, send: Vec<Message>, step: Option<Step>) -> Turn {
		let step = match step {
			Some(said) => said,
			None => if self.is_converged() {
				Step::Converged
			} else {
				Step::NeedMore
			},
		};
		Turn { send, step }
	}
}
