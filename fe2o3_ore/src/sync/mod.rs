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
//! # Layout
//!
//! - [`msg`] is the message set, with a daticle form and a version-tagged byte
//!   form.
//! - [`walk`] computes what a peer at a given frontier is owed, and the closure
//!   checks that hold at both ends.

pub mod msg;
pub mod walk;

pub use msg::{
	Message,
	MAGIC,
	VERSION,
};
pub use walk::{
	arrival_gap,
	covered,
	owed,
};
