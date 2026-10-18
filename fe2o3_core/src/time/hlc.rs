//! A hybrid logical clock: a stamp that follows the wall clock when the wall clock moves, a counter
//! when it does not, and the larger of its own and a peer's reading when a message arrives, so that
//! a stamp taken after receiving another is always greater than it.
//!
//! The wall reading is a [`WallClock`] (on main a `ClockSource` is one, so a rig's driven clock drives
//! the stamp) plus an offset the caller measures against a reference clock, so that a device whose own
//! clock is wrong still stamps near the reference's time.  A peer's wall more than `delta` ahead of
//! the corrected wall is not believed: the merge keeps only its own counter step and answers a
//! [`ClockFault`] for the caller to raise.  Built for Daimond's records (design (2), 16 Oct 2026,
//! §4.1); Ore's Lamport stamp and Ozone's process-local stamp are its other candidate callers.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use crate::prelude::*;

use std::{
    fmt,
    str::FromStr,
    sync::Arc,
};


/// The default bound on how far a peer's wall may run ahead of ours and still be believed.
pub const DELTA_MS_DEFAULT: u64 = 60_000;

/// `Hlc::to_bytes` length: 8 wall, 4 counter, 8 node.
pub const ENCODED_LEN: usize = 20;

/// One stamp.  The derived order is `(wall_ms, ctr, node)`, field by field, which is the clock's
/// total order; `node` only breaks a tie between two nodes stamping the same `(wall_ms, ctr)`.
///
/// Widths: milliseconds since the Unix epoch in a `u64`; a `u32` counter, four billion stamps in
/// one millisecond before the clock moves its wall on by one (see [`HlcClock::now`]); a `u64` node,
/// room for a random id with no registry.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Hlc {
    pub wall_ms:	u64,
    pub ctr:		u32,
    pub node:		u64,
}

impl Hlc {
    pub fn new(wall_ms: u64, ctr: u32, node: u64) -> Self {
        Self { wall_ms, ctr, node }
    }

    /// Big-endian throughout, so the byte order of two encodings is the order of the stamps and
    /// an encoding can be a store key.
    pub fn to_bytes(&self) -> [u8; ENCODED_LEN] {
        let mut b = [0u8; ENCODED_LEN];
        b[0..8].copy_from_slice(&self.wall_ms.to_be_bytes());
        b[8..12].copy_from_slice(&self.ctr.to_be_bytes());
        b[12..20].copy_from_slice(&self.node.to_be_bytes());
        b
    }

    pub fn from_bytes(b: &[u8]) -> Outcome<Self> {
        if b.len() != ENCODED_LEN {
            return Err(err!(
                "An encoded Hlc is {} bytes, not {}.", ENCODED_LEN, b.len();
            Invalid, Input, Size));
        }
        let mut w = [0u8; 8];
        let mut c = [0u8; 4];
        let mut n = [0u8; 8];
        w.copy_from_slice(&b[0..8]);
        c.copy_from_slice(&b[8..12]);
        n.copy_from_slice(&b[12..20]);
        Ok(Self {
            wall_ms:	u64::from_be_bytes(w),
            ctr:		u32::from_be_bytes(c),
            node:		u64::from_be_bytes(n),
        })
    }
}

/// `<wall_ms>.<ctr>@<node as 16 hex digits>`, e.g. `1760659200000.3@00000000000000a1`.
impl fmt::Display for Hlc {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}.{}@{:016x}", self.wall_ms, self.ctr, self.node)
    }
}

impl FromStr for Hlc {
    type Err = Error<ErrTag>;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        let (time, node) = match s.trim().split_once('@') {
            Some(p) => p,
            None    => return Err(err!(
                "Hlc '{}' has no '@' before its node.", s;
            Invalid, Input, Missing)),
        };
        let (wall, ctr) = match time.split_once('.') {
            Some(p) => p,
            None    => return Err(err!(
                "Hlc '{}' has no '.' between its wall and counter.", s;
            Invalid, Input, Missing)),
        };
        let wall_ms = match wall.parse::<u64>() {
            Ok(n)   => n,
            Err(e)  => return Err(err!(e, "Hlc '{}': the wall '{}'.", s, wall; Invalid, Input)),
        };
        let ctr = match ctr.parse::<u32>() {
            Ok(n)   => n,
            Err(e)  => return Err(err!(e, "Hlc '{}': the counter '{}'.", s, ctr; Invalid, Input)),
        };
        let node = match u64::from_str_radix(node, 16) {
            Ok(n)   => n,
            Err(e)  => return Err(err!(e, "Hlc '{}': the node '{}'.", s, node; Invalid, Input)),
        };
        Ok(Self { wall_ms, ctr, node })
    }
}

/// Where an [`HlcClock`] reads its wall, in milliseconds since the Unix epoch.  This base has no
/// `ClockSource`, so the caller supplies its own wall.
pub trait WallClock {
    fn wall_ms(&self) -> Outcome<u64>;
}

impl<W: WallClock + ?Sized> WallClock for &W {
    fn wall_ms(&self) -> Outcome<u64> {
        (**self).wall_ms()
    }
}

impl<W: WallClock + ?Sized> WallClock for Arc<W> {
    fn wall_ms(&self) -> Outcome<u64> {
        (**self).wall_ms()
    }
}

/// A peer's stamp whose wall ran more than `delta_ms` ahead of the corrected local wall, and was
/// therefore not merged.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClockFault {
    pub remote:		Hlc,	// the stamp received
    pub local_ms:	u64,	// the corrected local wall at receipt
    pub ahead_ms:	u64,	// remote.wall_ms - local_ms
    pub delta_ms:	u64,	// the bound in force
}

impl fmt::Display for ClockFault {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f,
            "a peer stamp {} is {} ms ahead of the local wall {} (bound {} ms); its wall was ignored",
            self.remote, self.ahead_ms, self.local_ms, self.delta_ms)
    }
}

/// What [`HlcClock::recv`] answers: the stamp for the receipt, and a fault when the peer's wall was
/// not believed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Recv {
    pub stamp:	Hlc,
    pub fault:	Option<ClockFault>,
}

/// The clock of one node.  It takes `&mut self`; a caller sharing one between threads puts it
/// behind its own lock, so a stamp and the state it advances change together.
#[derive(Debug)]
pub struct HlcClock<W: WallClock> {
    wall:		W,
    node:		u64,
    offset_ms:	i64,	// reference clock - local wall, as last measured
    delta_ms:	u64,
    last:		Hlc,	// the greatest stamp handed out or merged
}

impl<W: WallClock> HlcClock<W> {
    pub fn new(wall: W, node: u64) -> Self {
        Self {
            wall,
            node,
            offset_ms:	0,
            delta_ms:	DELTA_MS_DEFAULT,
            last:		Hlc::new(0, 0, node),
        }
    }

    pub fn with_delta_ms(mut self, delta_ms: u64) -> Self {
        self.delta_ms = delta_ms;
        self
    }

    /// Resumes from a stamp kept across a restart, so the clock never hands out one at or below it.
    pub fn with_last(mut self, last: Hlc) -> Self {
        self.last = Hlc::new(last.wall_ms, last.ctr, self.node);
        self
    }

    /// Sets the measured offset of a reference clock from the local wall (reference - local), so
    /// that `local + offset` reads near the reference.  A smaller offset than before does not move
    /// stamps back; they run on the counter until the corrected wall passes the last one.
    pub fn set_offset_ms(&mut self, offset_ms: i64) {
        self.offset_ms = offset_ms;
    }

    pub fn offset_ms(&self) -> i64		{ self.offset_ms }
    pub fn delta_ms(&self) -> u64		{ self.delta_ms }
    pub fn node(&self) -> u64			{ self.node }
    pub fn last(&self) -> Hlc			{ self.last }

    /// The local wall plus the offset, held inside `0..=u64::MAX`.
    pub fn corrected_ms(&self) -> Outcome<u64> {
        let w = res!(self.wall.wall_ms());
        Ok(if self.offset_ms >= 0 {
            w.saturating_add(self.offset_ms as u64)
        } else {
            w.saturating_sub(self.offset_ms.unsigned_abs())
        })
    }

    /// A stamp greater than every stamp this clock has handed out or merged.  Its wall is the
    /// larger of the corrected wall and the last stamp's; when that is the last stamp's, the counter
    /// steps.  A counter at `u32::MAX` moves the wall on by one millisecond and restarts at zero
    /// (the wall bump), rather than failing; only a wall at `u64::MAX` is an error.
    pub fn now(&mut self) -> Outcome<Hlc> {
        let pt = res!(self.corrected_ms());
        let next = if pt > self.last.wall_ms {
            Hlc::new(pt, 0, self.node)
        } else {
            res!(self.step(self.last.wall_ms, self.last.ctr))
        };
        self.last = next;
        Ok(next)
    }

    /// Merges a peer's stamp and answers a stamp greater than both it and every local one.  When
    /// the peer's wall is more than `delta_ms` ahead of the corrected wall it is not believed: the
    /// stamp is what [`now`](Self::now) would give, so it can be below the peer's, and the fault is
    /// answered for the caller to raise.
    pub fn recv(&mut self, remote: &Hlc) -> Outcome<Recv> {
        let pt = res!(self.corrected_ms());
        let ahead = remote.wall_ms.saturating_sub(pt);
        if ahead > self.delta_ms {
            let stamp = res!(self.now());
            return Ok(Recv {
                stamp,
                fault: Some(ClockFault {
                    remote:		*remote,
                    local_ms:	pt,
                    ahead_ms:	ahead,
                    delta_ms:	self.delta_ms,
                }),
            });
        }
        let l = self.last;
        let wall = pt.max(l.wall_ms).max(remote.wall_ms);
        let next = if wall == l.wall_ms && wall == remote.wall_ms {
            res!(self.step(wall, l.ctr.max(remote.ctr)))
        } else if wall == l.wall_ms {
            res!(self.step(wall, l.ctr))
        } else if wall == remote.wall_ms {
            res!(self.step(wall, remote.ctr))
        } else {
            Hlc::new(wall, 0, self.node)
        };
        self.last = next;
        Ok(Recv { stamp: next, fault: None })
    }

    // The stamp after (wall, ctr): the counter's next value, or the wall bump when it is spent.
    fn step(&self, wall: u64, ctr: u32) -> Outcome<Hlc> {
        match ctr.checked_add(1) {
            Some(c) => Ok(Hlc::new(wall, c, self.node)),
            None    => match wall.checked_add(1) {
                Some(w) => Ok(Hlc::new(w, 0, self.node)),
                None    => Err(err!(
                    "The hybrid logical clock of node {:016x} is spent: wall {} and counter {}.",
                    self.node, wall, ctr;
                Overflow)),
            },
        }
    }
}
