//! Length-prefixed BDAT frames over a tokio byte stream, for two processes that talk over a pipe or
//! a socket.
//!
//! A frame is a four-byte big-endian length followed by exactly that many bytes of one BDAT value.
//! BDAT is already self-delimiting, but a reader of a stream it did not write needs to know how
//! much it is being asked to hold *before* it holds it, so the length comes first and is checked
//! against a ceiling the caller chooses.
//!
//! The rules the reader keeps, for bytes it did not create:
//!
//! * **A frame over the ceiling is refused before it is read.**  The declared length is compared
//!   with the ceiling as soon as the prefix arrives, so a prefix promising four gigabytes costs
//!   four bytes, no buffer is sized from it and no body byte is waited for.
//! * **Partial frames never surface.**  A read returns a whole decoded value or a fault.  A stream
//!   that ends between frames is a clean end (`None`); one that ends inside a prefix or a body is
//!   [`PipeFault::ShortPrefix`] or [`PipeFault::Truncated`], never a shorter value.
//! * **A frame is one value.**  A body that does not decode, or that holds bytes after its value,
//!   is [`PipeFault::Undecodable`].  Nesting is held to a depth limit as well as the byte ceiling.
//!
//! The writer refuses a value whose encoding exceeds its ceiling before writing a byte, so a
//! refused frame leaves the stream exactly as it was and the next frame still lines up.
//!
//! The faults are named and recoverable from the error with [`PipeFault::of`], because a caller
//! deciding whether to drop a connection needs to know which thing went wrong.
//!
//! Neither read nor write is cancel-safe: dropping a pending read part-way through a frame leaves
//! the stream misaligned, so a caller selecting over a read keeps the reader's future alive rather
//! than restarting it.  This module sits beside, and follows the shape of, the Hand's native
//! messaging codec; it differs in carrying BDAT rather than JSON text, and in a big-endian prefix,
//! since both ends are ours rather than Chrome's.
//!
//! Enabled by the `pipe` feature, which is off by default so that a wasm or synchronous user of
//! this crate pulls in no tokio.

use crate::{
    bdat::DecodeLimits,
    daticle::Dat,
};

use oxedyne_fe2o3_core::prelude::*;

use tokio::io::{
    AsyncRead,
    AsyncReadExt,
    AsyncWrite,
    AsyncWriteExt,
};


/// The width of the length prefix, in bytes.
pub const LEN_PREFIX: usize = 4;

/// The largest ceiling a frame can have, set by the width of the prefix.
pub const LEN_MAX: usize = u32::MAX as usize;

/// The named ways a frame fails to be read or written.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PipeFault {
    /// The stream ended part-way through a length prefix.
    ShortPrefix,
    /// The stream ended before the declared number of body bytes arrived.
    Truncated,
    /// The frame declared, or would need, more bytes than the ceiling.
    TooBig,
    /// The body was not exactly one BDAT value within the limits.
    Undecodable,
    /// The stream could not be read or written at all.
    Io,
}

impl PipeFault {

    /// Every fault, so a search over them is exhaustive by construction.
    pub const ALL: [Self; 5] = [
        Self::ShortPrefix,
        Self::Truncated,
        Self::TooBig,
        Self::Undecodable,
        Self::Io,
    ];

    /// The stable identifier that begins every message this fault raises.
    pub fn name(&self) -> &'static str {
        match self {
            Self::ShortPrefix   => "pipe.short-prefix",
            Self::Truncated     => "pipe.truncated",
            Self::TooBig        => "pipe.too-big",
            Self::Undecodable   => "pipe.undecodable",
            Self::Io            => "pipe.io",
        }
    }

    /// Builds the error this fault raises, with the detail that identifies the case.
    ///
    /// # Arguments
    /// * `detail` - What was wrong, in a sentence a developer reading a journal can use.
    pub fn raise(&self, detail: &str) -> Error<ErrTag> {
        match self {
            Self::ShortPrefix   => err!("{}: {}", self.name(), detail; Input, Decode, Missing),
            Self::Truncated     => err!("{}: {}", self.name(), detail; Input, Decode, Missing),
            Self::TooBig        => err!("{}: {}", self.name(), detail; Input, TooBig),
            Self::Undecodable   => err!("{}: {}", self.name(), detail; Input, Decode, Invalid),
            Self::Io            => err!("{}: {}", self.name(), detail; IO, Wire),
        }
    }

    /// The fault an error carries, where it carries one, looking through any layers wrapped
    /// around it.
    ///
    /// # Arguments
    /// * `e` - The error to inspect.
    pub fn of(e: &Error<ErrTag>) -> Option<Self> {
        for msg in e.msgs() {
            for f in Self::ALL {
                if msg.starts_with(f.name()) {
                    return Some(f);
                }
            }
        }
        None
    }
}

/// Reads whole frames from a byte stream, refusing any over its ceiling.
#[derive(Debug)]
pub struct FrameReader<R> {
    r:      R,
    max:    usize,
    depth:  usize,
}

impl<R: AsyncRead + Unpin> FrameReader<R> {

    /// A reader of frames up to `max` bytes of body, with the default nesting limit.
    ///
    /// A ceiling over [`LEN_MAX`] is held to [`LEN_MAX`], since no prefix can declare more.
    ///
    /// # Arguments
    /// * `r` - The stream.
    /// * `max` - The largest body, in bytes, the reader will accept.
    pub fn new(r: R, max: usize) -> Self {
        Self {
            r,
            max:    std::cmp::min(max, LEN_MAX),
            depth:  DecodeLimits::DEFAULT_MAX_DEPTH,
        }
    }

    /// The same reader with a different nesting limit for decoded values.
    ///
    /// # Arguments
    /// * `depth` - The greatest nesting depth, with the root value at depth 1.
    pub fn with_max_depth(mut self, depth: usize) -> Self {
        self.depth = depth;
        self
    }

    /// The body ceiling in force.
    pub fn max(&self) -> usize { self.max }

    /// Gives back the stream.
    pub fn into_inner(self) -> R { self.r }

    /// Reads the next whole frame's body without decoding it, for a relay that forwards frames.
    ///
    /// Returns `None` at a clean end between frames.
    pub async fn read_bytes(&mut self) -> Outcome<Option<Vec<u8>>> {
        let mut pre = [0u8; LEN_PREFIX];
        let got = res!(read_upto(&mut self.r, &mut pre).await);
        if got == 0 {
            return Ok(None);
        }
        if got < LEN_PREFIX {
            return Err(PipeFault::ShortPrefix.raise(&fmt!(
                "The stream ended {} bytes into a {}-byte length prefix.", got, LEN_PREFIX)));
        }
        let n = u32::from_be_bytes(pre) as usize;
        if n > self.max {
            return Err(PipeFault::TooBig.raise(&fmt!(
                "The prefix declares {} bytes and the ceiling is {}. Nothing more was read.",
                n, self.max)));
        }
        let mut body = vec![0u8; n];
        let got = res!(read_upto(&mut self.r, &mut body).await);
        if got < n {
            return Err(PipeFault::Truncated.raise(&fmt!(
                "The prefix declares {} bytes and the stream ended after {}.", n, got)));
        }
        Ok(Some(body))
    }

    /// Reads and decodes the next whole frame.
    ///
    /// Returns `None` at a clean end between frames.
    pub async fn read(&mut self) -> Outcome<Option<Dat>> {
        let body = match res!(self.read_bytes().await) {
            Some(b) => b,
            None    => return Ok(None),
        };
        let lims = DecodeLimits::new(self.depth, self.max);
        match Dat::from_bytes_limited(&body, &lims) {
            Ok((d, used)) if used == body.len() => Ok(Some(d)),
            Ok((_, used)) => Err(PipeFault::Undecodable.raise(&fmt!(
                "The frame holds {} bytes after its value, which ends at byte {} of {}.",
                body.len() - used, used, body.len()))),
            Err(e) => Err(PipeFault::Undecodable.raise(&fmt!(
                "The {}-byte frame is not one BDAT value: {}", body.len(), e))),
        }
    }
}

/// Writes whole frames to a byte stream, refusing any over its ceiling before writing a byte.
#[derive(Debug)]
pub struct FrameWriter<W> {
    w:      W,
    max:    usize,
}

impl<W: AsyncWrite + Unpin> FrameWriter<W> {

    /// A writer of frames up to `max` bytes of body.
    ///
    /// A ceiling over [`LEN_MAX`] is held to [`LEN_MAX`], since no prefix can declare more.
    ///
    /// # Arguments
    /// * `w` - The stream.
    /// * `max` - The largest body, in bytes, the writer will send.
    pub fn new(w: W, max: usize) -> Self {
        Self {
            w,
            max: std::cmp::min(max, LEN_MAX),
        }
    }

    /// The body ceiling in force.
    pub fn max(&self) -> usize { self.max }

    /// Gives back the stream.
    pub fn into_inner(self) -> W { self.w }

    /// Encodes a value and writes it as one frame, then flushes.
    ///
    /// # Arguments
    /// * `dat` - The value.
    pub async fn write(&mut self, dat: &Dat) -> Outcome<()> {
        let body = match dat.as_bytes() {
            Ok(b)   => b,
            Err(e)  => return Err(PipeFault::Undecodable.raise(&fmt!(
                "The value could not be encoded: {}", e))),
        };
        self.write_bytes(&body).await
    }

    /// Writes an already encoded body as one frame, then flushes, for a relay that forwards
    /// frames.
    ///
    /// # Arguments
    /// * `body` - The body, one BDAT value.
    pub async fn write_bytes(&mut self, body: &[u8]) -> Outcome<()> {
        if body.len() > self.max {
            return Err(PipeFault::TooBig.raise(&fmt!(
                "The frame would hold {} bytes and the ceiling is {}. Nothing was written.",
                body.len(), self.max)));
        }
        // One buffer, so a frame goes out in as few writes as the stream allows.
        let mut whole = Vec::with_capacity(LEN_PREFIX + body.len());
        whole.extend_from_slice(&(body.len() as u32).to_be_bytes());
        whole.extend_from_slice(body);
        if let Err(e) = self.w.write_all(&whole).await {
            return Err(PipeFault::Io.raise(&fmt!("The frame could not be written: {}.", e)));
        }
        match self.w.flush().await {
            Ok(())  => Ok(()),
            Err(e)  => Err(PipeFault::Io.raise(&fmt!("The frame could not be flushed: {}.", e))),
        }
    }
}

/// Fills a buffer, returning how much of it arrived before the stream ended.
///
/// `read_exact` cannot tell a clean end from a short one, and that difference is what a reader
/// needs: nothing at all means the other end has gone, and part of a prefix means something is
/// wrong.
///
/// # Arguments
/// * `r` - The stream.
/// * `buf` - The buffer to fill.
async fn read_upto<R: AsyncRead + Unpin>(r: &mut R, buf: &mut [u8]) -> Outcome<usize> {
    let mut n = 0;
    while n < buf.len() {
        match r.read(&mut buf[n..]).await {
            Ok(0) => break,
            Ok(k) => n += k,
            Err(ref e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(PipeFault::Io.raise(&fmt!(
                "The stream could not be read: {}.", e))),
        }
    }
    Ok(n)
}
