//! BDAT, the binary encoding of JDAT, standing to JDAT as BSON does to JSON.
//!
//! Every value begins with a kind byte, and every list, map, tuple or vek declares the byte length
//! of its contents, so a subtree occupies a contiguous, self-delimiting run of bytes that a reader
//! can step over with a single seek rather than decoding it.
//!
//! BDAT is a serialisation rather than a container. The bytes begin with the kind byte of whatever
//! value was handed to the encoder, with no magic number, no version field, no index and no
//! signature, and the same logical value may encode differently, since 42 may be held as a `u8` or
//! an `i32`. An application wanting a file format supplies those things itself, declaring a version,
//! fixing one legal encoding so that a hash of the bytes is a stable identity, and wrapping the
//! result in whatever envelope its trust model needs.

pub mod core;
pub mod count;
pub mod dec;
pub mod enc;
pub mod limits;
pub mod load;

pub use limits::DecodeLimits;
