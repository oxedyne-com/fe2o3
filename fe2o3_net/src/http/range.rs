//! Byte ranges: the `Range` request field, and the `206`, `416` and
//! `Accept-Ranges` answers to it (RFC 9110 §14).
//!
//! A player asked to jump to the middle of a two hour recording does not want the
//! first hour and a half of it, and will not wait for them. It asks for the bytes
//! around the point it was sent to, and it asks again as it plays. A server that
//! cannot answer such a request either sends the whole representation every time
//! the viewer moves the scrubber, or -- what browsers actually do -- refuses to
//! offer seeking at all.
//!
//! # One range, never several
//!
//! The `Range` grammar admits a list, and a server may answer a list with a
//! `multipart/byteranges` body. This module recognises such a request and declines
//! to answer it that way: [`RangeRequest::Multiple`] is served as the whole
//! representation with a plain `200`, which RFC 9110 §14.2 permits ("a server MAY
//! ignore the Range header field"). Nothing that matters here asks for several
//! ranges at once -- media players, download managers and browsers all ask for one
//! window at a time -- and a multipart body costs a boundary generator, a second
//! framing to get wrong, and a client population that would rather have the file.
//!
//! # A field that cannot fail
//!
//! RFC 9110 §14.2 requires an unsatisfiable *unit* and a malformed field alike to
//! be ignored rather than rejected, so parsing yields [`RangeRequest::Ignored`]
//! instead of an error: a client that garbles its `Range` gets the representation,
//! not a refusal. Only a well-formed byte range that falls entirely outside the
//! representation earns a `416`, which is a statement about the representation
//! rather than about the syntax.

use crate::http::{
    fields::{
        HeaderFieldCategory,
        HeaderFields,
        HeaderFieldValue,
        HeaderName,
    },
    msg::HttpMessage,
    status::HttpStatus,
};

use oxedyne_fe2o3_core::prelude::*;


/// The only range unit anyone implements (RFC 9110 §14.1).
pub const BYTES_UNIT: &str = "bytes";

/// The value a resource that can be asked for in byte ranges advertises.
pub const ACCEPT_RANGES_BYTES: &str = "bytes";


/// One byte-range specifier, as written in a `Range` field and before it has met
/// the representation it names (RFC 9110 §14.1.1).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ByteRangeSpec {
    /// `bytes=s-e`: from `s` to `e`, both inclusive.
    FromTo(u64, u64),
    /// `bytes=s-`: from `s` to the last byte.
    From(u64),
    /// `bytes=-n`: the last `n` bytes.
    Suffix(u64),
}

/// What a `Range` field asked for.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RangeRequest {
    /// One window, which is what every media player and download manager sends.
    Single(ByteRangeSpec),
    /// Several windows at once, answered with the whole representation. See the
    /// module documentation for why this is not a `multipart/byteranges` body.
    Multiple,
    /// A unit this server does not implement, or a field that did not parse. The
    /// field is ignored and the whole representation is sent (RFC 9110 §14.2).
    Ignored,
}

impl RangeRequest {

    /// Parse the value of a `Range` field, e.g. `bytes=0-499`.
    ///
    /// Never fails: anything this server cannot honour is [`Self::Ignored`], which
    /// the caller serves as the whole representation.
    pub fn parse(value: &str) -> Self {
        let trimmed = value.trim();
        let (unit, list) = match trimmed.split_once('=') {
            Some(pair) => pair,
            None       => return Self::Ignored,
        };
        // The unit is a case-insensitive token, and only `bytes` is implemented.
        if !unit.trim().eq_ignore_ascii_case(BYTES_UNIT) {
            return Self::Ignored;
        }

        let mut specs = Vec::new();
        for part in list.split(',') {
            let part = part.trim();
            if part.is_empty() {
                // `bytes=0-1,,4-5` is malformed, and the whole field goes with it.
                return Self::Ignored;
            }
            match Self::parse_spec(part) {
                Some(spec) => specs.push(spec),
                None       => return Self::Ignored,
            }
        }

        match specs.len() {
            0 => Self::Ignored,
            1 => Self::Single(specs[0]),
            _ => Self::Multiple,
        }
    }

    /// Parse one `first-last`, `first-` or `-suffix` specifier.
    fn parse_spec(part: &str) -> Option<ByteRangeSpec> {
        let (first, last) = match part.split_once('-') {
            Some(pair) => pair,
            None       => return None, // No dash at all is not a range.
        };
        let first = first.trim();
        let last = last.trim();

        if first.is_empty() {
            // `-n`, the last n bytes. `bytes=-` names nothing.
            if last.is_empty() {
                return None;
            }
            return match last.parse::<u64>() {
                Ok(n)  => Some(ByteRangeSpec::Suffix(n)),
                Err(_) => None,
            };
        }

        let start = match first.parse::<u64>() {
            Ok(n)  => n,
            Err(_) => return None,
        };

        if last.is_empty() {
            return Some(ByteRangeSpec::From(start));
        }

        match last.parse::<u64>() {
            // A last position before the first is not a range at all, and the
            // grammar of RFC 9110 §14.1.1 forbids it.
            Ok(end) if end >= start => Some(ByteRangeSpec::FromTo(start, end)),
            _ => None,
        }
    }

    /// Read the `Range` field out of a request's header fields, if it carries one.
    ///
    /// The field is held as a `Generic` value rather than encapsulated, because a
    /// malformed one must be ignored and not turned into a read error that fails
    /// the whole message.
    pub fn from_fields(fields: &HeaderFields) -> Option<Self> {
        fields.get_one(&HeaderName::Range)
            .map(|val| Self::parse(&fmt!("{}", val)))
    }

    /// Resolve against a representation of `total` bytes.
    pub fn resolve(&self, total: u64) -> RangeOutcome {
        match self {
            Self::Single(spec)  => spec.resolve(total),
            Self::Multiple      => RangeOutcome::Whole,
            Self::Ignored       => RangeOutcome::Whole,
        }
    }
}

impl ByteRangeSpec {

    /// Resolve to a concrete window of a representation of `total` bytes
    /// (RFC 9110 §14.1.2).
    ///
    /// A zero-length representation satisfies no range whatsoever, an end past the
    /// last byte is clamped to it, and a suffix longer than the representation is
    /// the whole of it.
    pub fn resolve(&self, total: u64) -> RangeOutcome {
        if total == 0 {
            return RangeOutcome::NotSatisfiable;
        }
        let last = total - 1;
        match *self {
            Self::FromTo(start, end) => {
                if start > last {
                    RangeOutcome::NotSatisfiable
                } else {
                    RangeOutcome::Partial(ByteWindow {
                        start,
                        end: end.min(last),
                        total,
                    })
                }
            }
            Self::From(start) => {
                if start > last {
                    RangeOutcome::NotSatisfiable
                } else {
                    RangeOutcome::Partial(ByteWindow { start, end: last, total })
                }
            }
            Self::Suffix(n) => {
                // `bytes=-0` asks for the last nothing, which no representation
                // holds (RFC 9110 §14.1.2).
                if n == 0 {
                    RangeOutcome::NotSatisfiable
                } else {
                    RangeOutcome::Partial(ByteWindow {
                        start: total.saturating_sub(n),
                        end: last,
                        total,
                    })
                }
            }
        }
    }
}

/// A concrete window of a representation, with both ends inclusive as they are on
/// the wire.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ByteWindow {
    /// First byte sent.
    pub start:  u64,
    /// Last byte sent, inclusive.
    pub end:    u64,
    /// Length of the whole representation the window was cut from.
    pub total:  u64,
}

impl ByteWindow {

    /// How many bytes the window holds, which is the `Content-Length` of the
    /// answer.
    pub fn len(&self) -> u64 {
        self.end - self.start + 1
    }

    /// Whether the window covers the whole representation.
    pub fn is_whole(&self) -> bool {
        self.start == 0 && self.len() == self.total
    }

    /// The `Content-Range` field value naming this window, `bytes s-e/total`.
    pub fn content_range(&self) -> String {
        fmt!("{} {}-{}/{}", BYTES_UNIT, self.start, self.end, self.total)
    }
}

/// What answering a `Range` field comes to, once the representation is known.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RangeOutcome {
    /// Send the whole representation, `200`.
    Whole,
    /// Send this window, `206`, with a `Content-Range` naming it.
    Partial(ByteWindow),
    /// Send `416` with a `Content-Range` of `bytes */total`.
    NotSatisfiable,
}

/// Read a request's `Range` field and resolve it against a representation of
/// `total` bytes, in one call.
///
/// A request carrying no `Range` at all gets [`RangeOutcome::Whole`], as does one
/// whose field this server declines to honour.
pub fn resolve(fields: &HeaderFields, total: u64) -> RangeOutcome {
    match RangeRequest::from_fields(fields) {
        Some(req) => req.resolve(total),
        None      => RangeOutcome::Whole,
    }
}

/// The `Accept-Ranges: bytes` a resource advertises when it can be asked for in
/// windows.
///
/// A browser will not offer a scrubber on a video the server has not said this
/// about, however well the server would in fact answer the request.
pub fn accept_ranges() -> HeaderFieldValue {
    HeaderFieldValue::Generic(ACCEPT_RANGES_BYTES.to_string())
}

/// Stamp a response as answerable in byte ranges.
pub fn with_accept_ranges(msg: HttpMessage) -> HttpMessage {
    msg.with_field_with_order(
        HeaderName::AcceptRanges,
        accept_ranges(),
        Some(HeaderFieldCategory::Response as u16),
    )
}

/// The `Content-Range` field a `206` carries, naming the window sent.
pub fn content_range_field(window: &ByteWindow) -> HeaderFieldValue {
    HeaderFieldValue::Generic(window.content_range())
}

/// The `Content-Range` field a `416` carries, naming only the length the client's
/// range missed (RFC 9110 §14.4).
pub fn unsatisfied_range_field(total: u64) -> HeaderFieldValue {
    HeaderFieldValue::Generic(fmt!("{} */{}", BYTES_UNIT, total))
}

/// A `416 Range Not Satisfiable`, telling the client how long the representation
/// actually is so its next request can be a sensible one.
pub fn not_satisfiable(total: u64) -> HttpMessage {
    with_accept_ranges(
        HttpMessage::new_response(HttpStatus::RangeNotSatisfiable)
            .with_field_with_order(
                HeaderName::ContentRange,
                unsatisfied_range_field(total),
                Some(HeaderFieldCategory::Entity as u16),
            )
    )
}


#[cfg(test)]
mod tests {
    use super::*;

    fn single(spec: ByteRangeSpec) -> RangeRequest {
        RangeRequest::Single(spec)
    }

    // ┌───────────────────────────────────────────────────────────────────────┐
    // │ PARSING                                                               │
    // └───────────────────────────────────────────────────────────────────────┘

    #[test]
    fn test_the_three_shapes_of_a_byte_range() {
        assert_eq!(RangeRequest::parse("bytes=0-499"), single(ByteRangeSpec::FromTo(0, 499)));
        assert_eq!(RangeRequest::parse("bytes=500-"),  single(ByteRangeSpec::From(500)));
        assert_eq!(RangeRequest::parse("bytes=-500"),  single(ByteRangeSpec::Suffix(500)));
    }

    #[test]
    fn test_a_single_byte_is_a_range() {
        assert_eq!(RangeRequest::parse("bytes=0-0"), single(ByteRangeSpec::FromTo(0, 0)));
    }

    /// The unit is a case-insensitive token, and the field tolerates whitespace
    /// around what it names.
    #[test]
    fn test_the_unit_and_the_spacing_are_forgiving() {
        assert_eq!(RangeRequest::parse("BYTES=0-9"),   single(ByteRangeSpec::FromTo(0, 9)));
        assert_eq!(RangeRequest::parse(" bytes = 0 - 9 "), single(ByteRangeSpec::FromTo(0, 9)));
    }

    /// Only `bytes` is implemented, and RFC 9110 §14.2 says an unknown unit is
    /// ignored rather than refused -- so the client gets the file.
    #[test]
    fn test_a_unit_that_is_not_bytes_is_ignored() {
        assert_eq!(RangeRequest::parse("items=0-9"),    RangeRequest::Ignored);
        assert_eq!(RangeRequest::parse("seconds=0-9"),  RangeRequest::Ignored);
    }

    #[test]
    fn test_a_field_that_does_not_parse_is_ignored_not_refused() {
        for bad in [
            "",                 // Nothing at all.
            "bytes",            // No `=`.
            "bytes=",           // No specifier.
            "bytes=-",          // Neither end.
            "bytes=abc-def",    // Not numbers.
            "bytes=1x-2",       // Not quite numbers.
            "bytes=99-10",      // Last before first, which the grammar forbids.
            "bytes=0-1,,4-5",   // An empty specifier in the list.
        ] {
            assert_eq!(RangeRequest::parse(bad), RangeRequest::Ignored,
                "{:?} should have been ignored", bad);
        }
    }

    /// Several ranges are recognised, and answered with the whole representation.
    #[test]
    fn test_several_ranges_are_recognised_and_answered_whole() {
        assert_eq!(RangeRequest::parse("bytes=0-49,100-149"), RangeRequest::Multiple);
        assert_eq!(RangeRequest::parse("bytes=0-49,100-149").resolve(1000),
            RangeOutcome::Whole);
    }

    // ┌───────────────────────────────────────────────────────────────────────┐
    // │ RESOLUTION                                                            │
    // └───────────────────────────────────────────────────────────────────────┘

    #[test]
    fn test_a_window_inside_the_file() {
        assert_eq!(
            RangeRequest::parse("bytes=0-99").resolve(1000),
            RangeOutcome::Partial(ByteWindow { start: 0, end: 99, total: 1000 }),
        );
    }

    #[test]
    fn test_an_open_ended_range_runs_to_the_last_byte() {
        assert_eq!(
            RangeRequest::parse("bytes=100-").resolve(1000),
            RangeOutcome::Partial(ByteWindow { start: 100, end: 999, total: 1000 }),
        );
    }

    #[test]
    fn test_a_suffix_takes_the_last_bytes() {
        assert_eq!(
            RangeRequest::parse("bytes=-50").resolve(1000),
            RangeOutcome::Partial(ByteWindow { start: 950, end: 999, total: 1000 }),
        );
    }

    /// An end past the last byte is clamped rather than refused: the client asked
    /// for more than there is, and gets what there is.
    #[test]
    fn test_an_end_past_the_last_byte_is_clamped() {
        assert_eq!(
            RangeRequest::parse("bytes=900-99999").resolve(1000),
            RangeOutcome::Partial(ByteWindow { start: 900, end: 999, total: 1000 }),
        );
    }

    /// A start past the last byte is a genuine `416`: there is nothing there to
    /// clamp to.
    #[test]
    fn test_a_start_past_the_last_byte_is_not_satisfiable() {
        assert_eq!(RangeRequest::parse("bytes=1000-").resolve(1000),
            RangeOutcome::NotSatisfiable);
        assert_eq!(RangeRequest::parse("bytes=1000-2000").resolve(1000),
            RangeOutcome::NotSatisfiable);
        assert_eq!(RangeRequest::parse("bytes=999999999-").resolve(1000),
            RangeOutcome::NotSatisfiable);
    }

    /// The last byte is the last satisfiable start, and asking for it yields one
    /// byte rather than nothing.
    #[test]
    fn test_the_last_byte_is_still_satisfiable() {
        assert_eq!(
            RangeRequest::parse("bytes=999-").resolve(1000),
            RangeOutcome::Partial(ByteWindow { start: 999, end: 999, total: 1000 }),
        );
    }

    /// A suffix longer than the representation is the whole of it, not an error.
    #[test]
    fn test_a_suffix_longer_than_the_file_is_the_whole_file() {
        let outcome = RangeRequest::parse("bytes=-5000").resolve(1000);
        assert_eq!(outcome,
            RangeOutcome::Partial(ByteWindow { start: 0, end: 999, total: 1000 }));
        match outcome {
            RangeOutcome::Partial(w) => assert!(w.is_whole()),
            _ => panic!("the whole file was not recognised as whole"),
        }
    }

    /// `bytes=-0` asks for the last nothing, which no representation holds.
    #[test]
    fn test_a_zero_length_suffix_is_not_satisfiable() {
        assert_eq!(RangeRequest::parse("bytes=-0").resolve(1000),
            RangeOutcome::NotSatisfiable);
    }

    /// A zero-length representation satisfies no range at all, including the ones
    /// that name byte zero -- there is no byte zero.
    #[test]
    fn test_an_empty_file_satisfies_nothing() {
        for asked in ["bytes=0-", "bytes=0-0", "bytes=-1", "bytes=0-99"] {
            assert_eq!(RangeRequest::parse(asked).resolve(0),
                RangeOutcome::NotSatisfiable, "{:?} against an empty file", asked);
        }
    }

    /// A one-byte file is the smallest thing a range can actually name.
    #[test]
    fn test_a_one_byte_file() {
        assert_eq!(
            RangeRequest::parse("bytes=0-").resolve(1),
            RangeOutcome::Partial(ByteWindow { start: 0, end: 0, total: 1 }),
        );
        assert_eq!(RangeRequest::parse("bytes=1-").resolve(1),
            RangeOutcome::NotSatisfiable);
    }

    /// A window's length counts both ends, and an off-by-one here is a truncated
    /// video that plays to within a frame of the end and stops.
    #[test]
    fn test_a_window_counts_both_of_its_ends() {
        assert_eq!(ByteWindow { start: 0, end: 99, total: 1000 }.len(), 100);
        assert_eq!(ByteWindow { start: 0, end: 0, total: 1000 }.len(), 1);
        assert_eq!(ByteWindow { start: 950, end: 999, total: 1000 }.len(), 50);
    }

    // ┌───────────────────────────────────────────────────────────────────────┐
    // │ THE ANSWER ON THE WIRE                                                │
    // └───────────────────────────────────────────────────────────────────────┘

    #[test]
    fn test_the_content_range_of_a_window() {
        assert_eq!(
            ByteWindow { start: 0, end: 99, total: 1000 }.content_range(),
            "bytes 0-99/1000",
        );
    }

    /// A `416` names the length the client's range missed, and nothing else.
    #[test]
    fn test_a_refusal_states_the_length() -> Outcome<()> {
        let msg = not_satisfiable(1000);
        let held = res!(msg.header.fields.get_one(&HeaderName::ContentRange)
            .ok_or_else(|| err!("A 416 carried no Content-Range."; Missing)));
        assert_eq!(fmt!("{}", held), "bytes */1000");
        let ranges = res!(msg.header.fields.get_one(&HeaderName::AcceptRanges)
            .ok_or_else(|| err!("A 416 did not say what it does accept."; Missing)));
        assert_eq!(fmt!("{}", ranges), "bytes");
        Ok(())
    }

    /// The whole answer as bytes, because a field this server emits by hand is a
    /// field it can render wrongly.
    #[test]
    fn test_a_refusal_on_the_wire() -> Outcome<()> {
        let wire = not_satisfiable(1000).header.as_vec();
        let text = String::from_utf8_lossy(&wire).to_string();
        assert!(text.starts_with("HTTP/1.1 416 Range Not Satisfiable\r\n"),
            "unexpected status line: {:?}", text);
        assert!(text.contains("content-range: bytes */1000\r\n"),
            "unexpected fields: {:?}", text);
        assert!(text.contains("accept-ranges: bytes\r\n"),
            "unexpected fields: {:?}", text);
        Ok(())
    }

    /// A request with no `Range` at all asks for the whole thing.
    #[test]
    fn test_no_range_field_means_the_whole_representation() {
        assert_eq!(resolve(&HeaderFields::default(), 1000), RangeOutcome::Whole);
        assert_eq!(RangeRequest::from_fields(&HeaderFields::default()), None);
    }

    #[test]
    fn test_a_range_field_is_read_off_the_request() -> Outcome<()> {
        let mut fields = HeaderFields::default();
        fields.insert(
            HeaderName::Range,
            res!(HeaderFieldValue::new(&HeaderName::Range, "bytes=10-19")),
            None,
        );
        assert_eq!(
            resolve(&fields, 1000),
            RangeOutcome::Partial(ByteWindow { start: 10, end: 19, total: 1000 }),
        );
        Ok(())
    }
}
