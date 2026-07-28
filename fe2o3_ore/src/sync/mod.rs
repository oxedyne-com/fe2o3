//! Bringing two logs into agreement, as pure computation.
//!
//! Two replicas that have edited apart hold overlapping histories, and neither
//! knows what the other is missing. This module works that out and says what to
//! send. It is a state machine over typed messages: bytes in, bytes out, and
//! whatever carries them -- a socket, a file, a courier with a memory stick --
//! is the caller's business. Nothing here opens anything.
//!
//! Both peers run the same code. There is no client and no server, and no
//! message either side may send that the other may not; a relay that carries the
//! bytes learns nothing and decides nothing, which is what makes a server
//! optional rather than authoritative.
//!
//! # Closures, never subsets
//!
//! Everything sent is causally closed against what the receiver already holds:
//! what arrives, plus what was already there, names no parent nobody has. That
//! is not a courtesy. An operation set with a hole in it renders differently
//! from the same set once the hole is filled -- an anchor whose target is absent
//! has to be placed somewhere -- so a peer that absorbed an arbitrary subset
//! would show a state that never existed and was never authored. The receiver
//! checks the property on arrival rather than trusting it: [`arrival_gap`] names
//! the operation and the parent nobody holds, and a batch that fails is refused
//! whole.
//!
//! # Two modes, one message set
//!
//! - The **frontier walk** is the default and is correct at any divergence. The
//!   peers exchange their frontiers, and each sends every operation the other's
//!   frontier does not cover. It costs one round trip and never fails.
//! - **Sketch reconciliation** is the optimisation. Each peer sends an
//!   invertible Bloom lookup table over the names of the operations it holds;
//!   subtracting one from the other yields the difference directly, in bytes
//!   proportional to the difference rather than to the history. It is worth it
//!   when two large logs differ by a little, which is the steady state of a
//!   repository that syncs often.
//!
//! A sketch is sized from an estimate, and an estimate can be wrong. When the
//! peeling decoder stalls the difference is not half taken: the outcome says so
//! -- [`Step::FellBack`] -- and the walk answers instead, from the frontier the
//! sketch message carried for exactly that purpose. Nothing is guessed and no
//! round trip is lost.
//!
//! # Layout
//!
//! - [`msg`] is the message set, with a daticle form and a version-tagged byte
//!   form.
//! - [`walk`] computes what a peer at a given frontier is owed, and the closure
//!   checks that hold at both ends.
//! - [`sketch`] is the invertible Bloom lookup table over operation names: how a
//!   name is keyed, how the table is sized, and what a decode yields.
//! - [`session`] is the driver: feed it a message, take the messages it hands
//!   back, and read the outcome.

pub mod msg;
pub mod session;
pub mod sketch;
pub mod walk;

#[cfg(test)]
mod tests;

pub use msg::{
	Message,
	MAGIC,
	VERSION,
};
pub use session::{
	Mode,
	Session,
	Step,
	Turn,
};
pub use sketch::{
	Diff,
	Fallback,
};
pub use walk::{
	arrival_gap,
	covered,
	owed,
};
