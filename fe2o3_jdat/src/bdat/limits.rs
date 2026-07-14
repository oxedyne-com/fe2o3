//! Limits applied while decoding BDAT from an untrusted source.
//!
//! BDAT is self-delimiting but not self-limiting: a few bytes can describe a list nested a million
//! deep, and a decoder that trusts them will recurse until its stack is gone.  A reader that did
//! not create the bytes it is reading should therefore decode through
//! [`Dat::from_bytes_limited`](crate::Dat::from_bytes_limited), which refuses input that is too
//! long, and refuses to descend past a stated depth.

use oxedyne_fe2o3_core::prelude::*;


/// The bounds a BDAT decoder enforces on untrusted input.
///
/// `max_depth` counts nested values, with the root value at depth 1.  A scalar at the root reaches
/// depth 1, a list holding a scalar reaches depth 2, and a list holding a list holding a scalar
/// reaches depth 3.  `max_bytes` bounds the length of the buffer handed to the decoder, not the
/// length of the value decoded from it, since a value may be followed by bytes that are none of the
/// decoder's business.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DecodeLimits {
    /// Greatest nesting depth the decoder will descend to, with the root value at depth 1.
    pub max_depth: usize,
    /// Greatest length, in bytes, of a buffer the decoder will accept.
    pub max_bytes: usize,
}

impl Default for DecodeLimits {
    fn default() -> Self {
        Self {
            max_depth: Self::DEFAULT_MAX_DEPTH,
            max_bytes: Self::DEFAULT_MAX_BYTES,
        }
    }
}

impl DecodeLimits {

    /// Default nesting depth, deep enough for any document a human wrote and shallow enough to
    /// leave the stack intact.
    pub const DEFAULT_MAX_DEPTH: usize = 64;
    /// Default buffer length, in bytes.
    pub const DEFAULT_MAX_BYTES: usize = 64 * 1024 * 1024;

    /// No limits at all, as trusted by [`Dat::from_bytes`](crate::Dat::from_bytes), whose behaviour
    /// predates this type.
    pub const UNLIMITED: Self = Self {
        max_depth: usize::MAX,
        max_bytes: usize::MAX,
    };

    /// Creates limits with the given maximum depth and buffer length.
    pub fn new(
        max_depth:  usize,
        max_bytes:  usize,
    )
        -> Self
    {
        Self {
            max_depth,
            max_bytes,
        }
    }

    /// Returns the limits with the maximum depth replaced.
    pub fn with_max_depth(mut self, max_depth: usize) -> Self {
        self.max_depth = max_depth;
        self
    }

    /// Returns the limits with the maximum buffer length replaced.
    pub fn with_max_bytes(mut self, max_bytes: usize) -> Self {
        self.max_bytes = max_bytes;
        self
    }

    /// Rejects a buffer longer than the maximum length.
    pub fn check_len(&self, len: usize) -> Outcome<()> {
        if len > self.max_bytes {
            return Err(err!(
                "Decoding input of {} bytes at byte offset 0 exceeds the maximum of {} bytes.",
                len, self.max_bytes;
            Input, Invalid, Excessive, Size));
        }
        Ok(())
    }

    /// Rejects a value nested deeper than the maximum depth, naming the offset of its first byte.
    pub fn check_depth(
        &self,
        depth:  usize,
        pos:    usize,
    )
        -> Outcome<()>
    {
        if depth > self.max_depth {
            return Err(err!(
                "Decoding a value at nesting depth {} at byte offset {} exceeds the maximum \
                depth of {}.",
                depth, pos, self.max_depth;
            Input, Invalid, Excessive, Size));
        }
        Ok(())
    }

}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::prelude::*;

    use oxedyne_fe2o3_core::byte::{
        FromBytes,
        ToBytes,
    };

    /// Wraps a leaf in `levels` nested lists, so that the leaf sits at depth `levels + 1`.
    fn nest(levels: usize) -> Dat {
        let mut dat = Dat::Empty;
        for _ in 0..levels {
            dat = Dat::List(vec![dat]);
        }
        dat
    }

    /// Encodes `levels` nested lists around an empty leaf, from the inside out.
    ///
    /// The bytes are built rather than encoded, since the encoder recurses as the decoder does, and
    /// an attacker is under no obligation to use it.  This is the bomb: a few hundred kilobytes
    /// describing a nesting deep enough to exhaust the stack of whoever decodes it.
    fn nested_list_bytes(levels: usize) -> Outcome<Vec<u8>> {
        let mut buf = vec![Dat::EMPTY_CODE];
        for _ in 0..levels {
            let payload_len = buf.len();
            let mut outer = vec![Dat::LIST_CODE];
            outer = res!(Dat::C64(payload_len as u64).to_bytes(outer));
            outer.append(&mut buf);
            buf = outer;
        }
        Ok(buf)
    }

    #[test]
    fn test_depth_limit_accepts_at_the_limit() -> Outcome<()> {
        const LEVELS: usize = 16;
        let dat = nest(LEVELS);
        let buf = res!(dat.to_bytes(Vec::new()));
        // The leaf sits one deeper than the deepest list.
        let lims = DecodeLimits::default().with_max_depth(LEVELS + 1);
        let (dat2, n) = res!(Dat::from_bytes_limited(&buf, &lims));
        assert_eq!(dat, dat2);
        assert_eq!(n, buf.len());
        Ok(())
    }

    #[test]
    fn test_depth_limit_refuses_past_the_limit() -> Outcome<()> {
        const LEVELS: usize = 16;
        let buf = res!(nest(LEVELS).to_bytes(Vec::new()));
        let lims = DecodeLimits::default().with_max_depth(LEVELS);
        match Dat::from_bytes_limited(&buf, &lims) {
            Ok((dat, _)) => Err(err!(
                "Expected a depth limit error, but decoded {:?}.", dat;
            Test, Invalid)),
            Err(e) => {
                let msg = e.to_string();
                assert!(msg.contains("depth"), "Error should name the depth limit: {}", msg);
                assert!(msg.contains("offset"), "Error should name the byte offset: {}", msg);
                Ok(())
            }
        }
    }

    #[test]
    fn test_hand_built_bytes_decode_as_the_encoder_would() -> Outcome<()> {
        // The bomb builder must agree with the encoder, or it proves nothing.
        const LEVELS: usize = 12;
        let built = res!(nested_list_bytes(LEVELS));
        let encoded = res!(nest(LEVELS).to_bytes(Vec::new()));
        assert_eq!(built, encoded);
        Ok(())
    }

    #[test]
    fn test_depth_bomb_is_refused() -> Outcome<()> {
        // A hostile file: 100,000 nested lists, which would exhaust the stack of a decoder that
        // trusted it, in a buffer small enough to arrive over a socket without comment.
        let buf = res!(nested_list_bytes(100_000));
        match Dat::from_bytes_limited(&buf, &DecodeLimits::default()) {
            Ok(_) => Err(err!(
                "A nesting of 100,000 lists should be refused by the default limits.";
            Test, Invalid)),
            Err(e) => {
                let msg = e.to_string();
                assert!(msg.contains("depth"), "Error should name the depth limit: {}", msg);
                Ok(())
            }
        }
    }

    #[test]
    fn test_byte_limit_refuses_an_oversized_input() -> Outcome<()> {
        let buf = res!(dat!("a string long enough to overrun a tiny limit").to_bytes(Vec::new()));
        let lims = DecodeLimits::default().with_max_bytes(8);
        match Dat::from_bytes_limited(&buf, &lims) {
            Ok((dat, _)) => Err(err!(
                "Expected a byte limit error, but decoded {:?}.", dat;
            Test, Invalid)),
            Err(e) => {
                let msg = e.to_string();
                assert!(msg.contains("8 bytes"), "Error should name the byte limit: {}", msg);
                Ok(())
            }
        }
    }

    #[test]
    fn test_limited_round_trip_matches_unlimited() -> Outcome<()> {
        let dat = listdat![
            dat!("hello"),
            dat!(42u8),
            mapdat!{
                dat!("k") => listdat![dat!(1i32), dat!(2i32)],
                dat!("n") => Dat::Opt(Box::new(Some(dat!(7u16)))),
            },
            Dat::Box(Box::new(dat!(true))),
        ];
        let buf = res!(dat.to_bytes(Vec::new()));

        let (limited, n1) = res!(Dat::from_bytes_limited(&buf, &DecodeLimits::default()));
        let (plain, n2) = res!(Dat::from_bytes(&buf));

        assert_eq!(limited, dat);
        assert_eq!(plain, dat);
        assert_eq!(n1, buf.len());
        assert_eq!(n2, buf.len());
        Ok(())
    }

    #[test]
    fn test_trailing_bytes_are_left_alone() -> Outcome<()> {
        // A limited decode reads one value and reports its length, ignoring what follows.
        let mut buf = res!(dat!(42u8).to_bytes(Vec::new()));
        buf.extend_from_slice(&[0xff, 0xff, 0xff]);
        let (dat, n) = res!(Dat::from_bytes_limited(&buf, &DecodeLimits::default()));
        assert_eq!(dat, dat!(42u8));
        assert_eq!(n, 2);
        Ok(())
    }

    // The three cases below are truncated encodings that a hostile sender can hand to a decoder.
    // Each once indexed past the end of the buffer and panicked; the decoder must now refuse them
    // with an error, never abort the process.  A decoder that panics on any byte sequence is a
    // denial-of-service on every service that reads bytes it did not write.

    #[test]
    fn test_usr_truncated_before_option_code() -> Outcome<()> {
        // A usr daticle: kind byte, then the two-byte kind code, and nothing more.  The option code
        // the decoder must read next is off the end of the buffer.
        let buf = vec![Dat::USR_CODE, 0x00, 0x05];
        assert!(Dat::from_bytes_limited(&buf, &DecodeLimits::default()).is_err());
        Ok(())
    }

    #[test]
    fn test_abox_truncated_after_inner_value() -> Outcome<()> {
        // An abox: kind byte, an empty NoteConfig byte, an empty inner value, and nothing more.  The
        // trailing annotation length the decoder must read next is off the end of the buffer.
        let buf = vec![Dat::ABOX_CODE, Dat::EMPTY_CODE, Dat::EMPTY_CODE];
        assert!(Dat::from_bytes_limited(&buf, &DecodeLimits::default()).is_err());
        Ok(())
    }

    #[test]
    fn test_list_payload_length_near_usize_max() -> Outcome<()> {
        // A list whose declared payload length is an eight-byte c64 of all ones, close to
        // usize::MAX.  Adding it to the buffer position once overflowed usize; it must be refused as
        // more bytes than the buffer holds.
        let mut buf = vec![Dat::LIST_CODE, Dat::C64_CODE_START + 8];
        buf.extend_from_slice(&[0xff; 8]);
        assert!(Dat::from_bytes_limited(&buf, &DecodeLimits::default()).is_err());
        // The same encoding as a map, which shares the length arithmetic.
        let mut buf = vec![Dat::MAP_CODE, Dat::C64_CODE_START + 8];
        buf.extend_from_slice(&[0xff; 8]);
        assert!(Dat::from_bytes_limited(&buf, &DecodeLimits::default()).is_err());
        Ok(())
    }
}
