//! Content codings: what a client will accept, what is worth encoding, and the
//! gzip stream itself.
//!
//! A response sent raw costs the reader every byte of it. Markup, script,
//! stylesheets and WebAssembly are all highly redundant, and a page built of
//! them typically weighs two to four times on the wire what it need weigh --
//! which is paid by whoever is on the slowest connection, every visit.
//!
//! Encoding one is only correct if three things hold together: the client said
//! it would accept the coding ([RFC 9110 §12.5.3]), the representation is not
//! already compressed, and every framing field describes the *encoded* body
//! rather than the original. The last is not a nicety -- a `Content-Length`
//! naming the wrong number desynchronises a kept-alive connection, and the
//! client waits for bytes that never come.
//!
//! [RFC 9110 §12.5.3]: https://www.rfc-editor.org/rfc/rfc9110#section-12.5.3

use crate::{
    http::{
        fields::{
            HeaderFields,
            HeaderFieldValue,
            HeaderName,
        },
        msg::HttpMessage,
    },
    media::MediaType,
};

use oxedyne_fe2o3_core::prelude::*;

use std::{
    io::Write,
    str::FromStr,
};

use flate2::{
    Compression,
    write::GzEncoder,
    read::GzDecoder,
};


/// Compression level used for the gzip stream.
///
/// Level 6 is zlib's own default and the knee of the curve. Measured over a
/// megabyte of base64-heavy markup, a WebAssembly module and a script bundle:
/// level 9 costs about half as much time again for two parts in a thousand more
/// saving, and level 1 runs in a third of the time but gives up something like a
/// sixth of the saving. The encoding is the reason the response is smaller, so
/// the saving is what is being bought.
const GZIP_LEVEL: u32 = 6;

/// The default below which a body is sent as it is.
///
/// A gzip member costs eighteen bytes of framing before it encodes anything, and
/// the round trip through the encoder and the client's decoder is not free
/// either. Under about a kilobyte the saving is noise, and on the smallest
/// bodies the encoded form is the larger of the two.
pub const MIN_BYTES_DEFAULT: usize = 1024;


/// A content coding this server can produce.
///
/// Only the two: `gzip` ([RFC 9110 §8.4.1.3], the format of [RFC 1952]) and no
/// coding at all. Naming a coding the encoder cannot actually emit would let
/// negotiation promise something the wire could not keep, so the enum is exactly
/// the set of things that can be sent.
///
/// [RFC 9110 §8.4.1.3]: https://www.rfc-editor.org/rfc/rfc9110#section-8.4.1.3
/// [RFC 1952]: https://www.rfc-editor.org/rfc/rfc1952
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ContentCoding {
    /// The representation as it stands, carrying no `Content-Encoding`.
    Identity,
    /// A gzip member, as `Content-Encoding: gzip`.
    Gzip,
}

impl ContentCoding {

    /// The token as it appears in `Accept-Encoding` and `Content-Encoding`.
    pub fn token(&self) -> &'static str {
        match self {
            Self::Identity  => "identity",
            Self::Gzip      => "gzip",
        }
    }

    /// Read a coding token, accepting the historic `x-gzip` spelling.
    ///
    /// RFC 9110 §8.4.1.3 records `x-gzip` as the name some senders still use for
    /// the same format, and a client that asks for it by that name is asking for
    /// gzip.
    pub fn from_token(token: &str) -> Option<Self> {
        match token.trim().to_ascii_lowercase().as_str() {
            "gzip" | "x-gzip"   => Some(Self::Gzip),
            "identity"          => Some(Self::Identity),
            _                   => None,
        }
    }

    /// Does this coding change the bytes on the wire?
    pub fn encodes(&self) -> bool {
        !matches!(self, Self::Identity)
    }
}


/// One entry of an `Accept-Encoding` field: a coding token and its weight.
///
/// The weight is held in thousandths, which is the full precision RFC 9110
/// §12.4.2 allows a qvalue (`0.000` to `1.000`), so the whole comparison is
/// integer arithmetic and no two weights ever compare equal by rounding.
#[derive(Clone, Debug, Eq, PartialEq)]
struct Preference {
    /// The token as written, lowercased; `*` is kept as itself.
    token:  String,
    /// Weight in thousandths, `0` meaning "not acceptable".
    weight: u16,
}

/// Split an `Accept-Encoding` field value into its entries.
///
/// A malformed weight is read as the absent one, per RFC 9110 §12.4.2: the
/// default when no weight is given is `q=1`, and a sender that writes rubbish
/// after the semicolon has still named the coding.
fn preferences(field: &str) -> Vec<Preference> {
    let mut out = Vec::new();
    for entry in field.split(',') {
        let entry = entry.trim();
        if entry.is_empty() {
            continue;
        }
        let mut parts = entry.split(';');
        let token = match parts.next() {
            Some(t) => t.trim().to_ascii_lowercase(),
            None    => continue,
        };
        if token.is_empty() {
            continue;
        }
        let mut weight = 1000u16;
        for param in parts {
            let param = param.trim();
            let rest = match param.strip_prefix("q=").or_else(|| param.strip_prefix("Q=")) {
                Some(rest)  => rest.trim(),
                None        => continue,
            };
            weight = qvalue(rest).unwrap_or(1000);
        }
        out.push(Preference { token, weight });
    }
    out
}

/// Read a qvalue into thousandths.
///
/// RFC 9110 §12.4.2 defines it as `0[.0-3 digits]` or `1[.0-3 zeroes]`, so the
/// scale is exactly a thousand and nothing outside `0.0 ..= 1.0` is a qvalue at
/// all.
fn qvalue(s: &str) -> Option<u16> {
    let (whole, frac) = match s.split_once('.') {
        Some((w, f))    => (w, f),
        None            => (s, ""),
    };
    let lead: u16 = match whole.trim() {
        "0" => 0,
        "1" => 1000,
        _   => return None,
    };
    if frac.is_empty() {
        return Some(lead);
    }
    if frac.len() > 3 || !frac.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    // Pad to three places so `.5` and `.500` weigh the same.
    let mut thousandths: u16 = 0;
    let mut scale = 100u16;
    for b in frac.bytes() {
        thousandths += ((b - b'0') as u16) * scale;
        scale /= 10;
    }
    match lead {
        0 => Some(thousandths),
        // `q=1.000` is the only legal form above one; anything else is not a
        // qvalue, and reading it as a weight would let a sender outrank the
        // scale's own ceiling.
        _ => if thousandths == 0 { Some(1000) } else { None },
    }
}

/// The weight a field value gives one coding.
///
/// A coding named outright takes its own weight. Otherwise `*` speaks for it,
/// per RFC 9110 §12.5.3 -- "the asterisk symbol matches any available content
/// coding not explicitly listed". A coding neither named nor covered by `*` is
/// not acceptable, which is the whole point of sending the field.
fn weight_of(prefs: &[Preference], coding: ContentCoding) -> u16 {
    let named = prefs.iter().find(|p|
        ContentCoding::from_token(&p.token) == Some(coding));
    if let Some(p) = named {
        return p.weight;
    }
    if let Some(p) = prefs.iter().find(|p| p.token == "*") {
        return p.weight;
    }
    match coding {
        // "If the representation has no content coding, then it is acceptable by
        // default unless specifically refused" -- RFC 9110 §12.5.3.
        ContentCoding::Identity => 1000,
        _                       => 0,
    }
}

/// Choose a coding from an `Accept-Encoding` field value.
///
/// Follows RFC 9110 §12.5.3:
///
/// - No field at all means the sender expressed no preference. The
///   specification permits any coding here, but this server sends none: a
///   request with no `Accept-Encoding` is very rarely a browser, and handing an
///   unrequested coding to a script or a proxy that never asked for one is how
///   an integration breaks for no gain.
/// - An empty field value means no coding is supported, so identity it is.
/// - `q=0` means not acceptable, for `identity` as much as for anything else.
/// - `*` speaks for every coding not named outright.
/// - Among acceptable codings the greatest weight wins, and a tie goes to gzip,
///   which is the server's own preference and the reason the negotiation is
///   being done.
pub fn negotiate(accept_encoding: Option<&str>) -> ContentCoding {
    let field = match accept_encoding {
        Some(f) => f,
        None    => return ContentCoding::Identity,
    };
    let prefs = preferences(field);
    if prefs.is_empty() {
        return ContentCoding::Identity;
    }
    let gzip = weight_of(&prefs, ContentCoding::Gzip);
    let identity = weight_of(&prefs, ContentCoding::Identity);
    if gzip > 0 && gzip >= identity {
        ContentCoding::Gzip
    } else {
        ContentCoding::Identity
    }
}

/// Read the `Accept-Encoding` field, if the request carried one.
pub fn accept_encoding(fields: &HeaderFields) -> Option<String> {
    fields.get_one(&HeaderName::AcceptEncoding).map(|val| fmt!("{}", val))
}

/// Is a body of this media type worth encoding?
///
/// The string is a `Content-Type` field value, so any parameters after the
/// media type (`; charset=utf-8`, a multipart boundary) are cut before it is
/// read.
///
/// Two cases are settled on the string before the media type is parsed at all.
/// Anything under `text/` is text by definition (RFC 2046 §4.1), whether or not
/// this crate models the subtype, so `text/markdown` and `text/calendar` are not
/// left out for want of an enum variant. And the several names for script --
/// `application/javascript` and its `x-` and `ecmascript` spellings, which a
/// proxied upstream may well use in place of `text/javascript` -- name a format
/// that halves under DEFLATE whichever way it is spelled.
///
/// Otherwise a type this crate cannot parse is left alone, which is the safe way
/// round: a missed saving costs bandwidth, a needless one costs the processor
/// and gains nothing.
pub fn is_compressible(content_type: &str) -> bool {
    let media = content_type.split(';').next().unwrap_or("").trim().to_ascii_lowercase();
    if media.starts_with("text/") {
        return true;
    }
    if matches!(media.as_str(),
        "application/javascript"
        | "application/x-javascript"
        | "application/ecmascript"
        | "application/x-ecmascript"
    ) {
        return true;
    }
    match MediaType::from_str(&media) {
        Ok(mt)  => mt.is_compressible(),
        Err(_)  => false,
    }
}

/// The whole rule: what coding should a response of this type and size carry?
///
/// Three things must hold at once, and the cheapest is asked first. A body under
/// `min_bytes` is sent as it is, since a gzip member costs eighteen bytes of
/// framing before it encodes anything. A type that is already compressed is sent
/// as it is. And whatever is left is offered only if the client said it would
/// take it.
pub fn choose(
    fields:         &HeaderFields,
    content_type:   &str,
    body_len:       usize,
    min_bytes:      usize,
)
    -> ContentCoding
{
    choose_for(
        accept_encoding(fields).as_deref(),
        content_type,
        body_len,
        min_bytes,
    )
}

/// [`choose`], for a caller that kept the `Accept-Encoding` field rather than
/// the request it came on.
///
/// The request is moved into the dispatch chain long before the response is
/// encoded, so the server holds the one field it will need and lets the rest go.
pub fn choose_for(
    accept_encoding:    Option<&str>,
    content_type:       &str,
    body_len:           usize,
    min_bytes:          usize,
)
    -> ContentCoding
{
    if body_len < min_bytes {
        return ContentCoding::Identity;
    }
    if !is_compressible(content_type) {
        return ContentCoding::Identity;
    }
    negotiate(accept_encoding)
}

/// Name the coding in an entity tag, so two encodings of one representation
/// never share a validator.
///
/// RFC 9110 §8.8.3 makes an entity tag the identity of a *representation*, and a
/// gzipped body is a different representation of the same resource. Handing both
/// the same tag is the classic caching fault: a client holding the encoded copy
/// sends the tag back on a request that accepts no coding, the server answers
/// `304`, and the client renders a gzip member as though it were markup.
///
/// The coding goes inside the quotes, leaving the tag a valid `entity-tag` and
/// keeping the weakness marker where it belongs.
pub fn tagged(etag: &str, coding: ContentCoding) -> String {
    if !coding.encodes() {
        return etag.to_string();
    }
    match etag.strip_suffix('"') {
        Some(head) => fmt!("{}-{}\"", head, coding.token()),
        // Not a quoted tag at all; leave it be rather than mint a malformed one.
        None => etag.to_string(),
    }
}

/// gzip a buffer, as [RFC 1952] defines the format.
///
/// [RFC 1952]: https://www.rfc-editor.org/rfc/rfc1952
pub fn gzip(data: &[u8]) -> Outcome<Vec<u8>> {
    let mut enc = GzEncoder::new(Vec::new(), Compression::new(GZIP_LEVEL));
    res!(enc.write_all(data), IO, Encode);
    Ok(res!(enc.finish(), IO, Encode))
}

/// Read a gzip member back, which is what a client does with one.
pub fn gunzip(data: &[u8]) -> Outcome<Vec<u8>> {
    use std::io::Read;
    let mut dec = GzDecoder::new(data);
    let mut out = Vec::new();
    res!(dec.read_to_end(&mut out), IO, Decode);
    Ok(out)
}

/// Say that the response varies by the coding asked for.
///
/// A shared cache keyed on the URL alone would hand a stored gzip body to the
/// next client along, coding or no coding. RFC 9111 §4.1 makes `Vary` the key,
/// and it is needed on *every* response whose type could have been encoded --
/// including the ones that were not, since those are exactly the copies a cache
/// would otherwise reuse for a client that does accept a coding.
pub fn mark_varying(msg: &mut HttpMessage) {
    let already = msg.header.fields.get_list(&HeaderName::Vary)
        .map_or(false, |vals| vals.iter().any(|v|
            fmt!("{}", v).to_ascii_lowercase().contains("accept-encoding")));
    if !already {
        msg.header.fields.insert(
            HeaderName::Vary,
            HeaderFieldValue::Generic(fmt!("accept-encoding")),
            None,
        );
    }
}

/// Would encoding this response be correct at all?
///
/// Independent of what the client will accept: some responses must not be
/// encoded whatever the request said.
///
/// - A body already carrying a `Content-Encoding` has been encoded by whoever
///   produced it, and a second coding would have to be declared as such and
///   undone in order.
/// - A chunked message frames itself, and RFC 9112 §6.1 forbids the
///   `Content-Length` an encoded body would need.
/// - A `206` answers a byte range of the *identity* representation. Encoding it
///   would make the range name bytes of something else entirely.
/// - A status with no body has nothing to encode; `304` in particular must carry
///   the validators of the representation it stands for, which
///   [`tagged`] has already named.
/// - A `HEAD` answer withholds its body at the wire, so coding one buys nothing
///   and costs everything: the whole body has to be materialised to be encoded --
///   a file window read off the disk included -- and then thrown away unsent. The
///   answer keeps the `Content-Length` of the identity representation, which is
///   what a `GET` accepting no coding would be told and what anyone asking how
///   big a thing is wants to know. RFC 9110 §9.3.2 asks for the fields the `GET`
///   would carry; it does not ask a server to do the `GET`'s work to find out.
pub fn is_encodable(msg: &HttpMessage) -> bool {
    use crate::http::{
        header::HttpHeadline,
        status::HttpStatus,
    };
    if msg.head_only {
        return false;
    }
    if msg.header.fields.get_one(&HeaderName::ContentEncoding).is_some() {
        return false;
    }
    if msg.header.fields.get_one(&HeaderName::TransferEncoding).is_some() {
        return false;
    }
    match msg.header.headline {
        HttpHeadline::Response { status } => !matches!(status,
            HttpStatus::PartialContent
            | HttpStatus::NotModified
            | HttpStatus::NoContent
        ),
        _ => false,
    }
}

/// Encode a response, leaving every framing field describing the encoded body.
///
/// The body is materialised first: a message whose body is named as a window of
/// a file has to be read before it can be encoded, and the window is then
/// dropped, since the bytes going out are no longer the bytes on disk.
/// `Content-Length` follows from [`HttpMessage::body_len`] when the message is
/// written, so it describes the encoded body by construction.
///
/// A body the encoder cannot shrink is returned as it stands. Sending the larger
/// of the two forms would be a strange thing to have gone to the trouble of, and
/// it happens on small or already-dense bodies that slipped past the earlier
/// tests.
///
/// `Vary` is set either way, because a cache must key on the coding whether or
/// not this particular response carried one.
pub async fn encode(
    mut msg:    HttpMessage,
    coding:     ContentCoding,
)
    -> Outcome<HttpMessage>
{
    mark_varying(&mut msg);
    if !coding.encodes() || !is_encodable(&msg) {
        return Ok(msg);
    }
    let plain = match msg.file.take() {
        Some(window)    => res!(window.read().await),
        None            => std::mem::take(&mut msg.body),
    };
    let encoded = match coding {
        ContentCoding::Gzip     => res!(gzip(&plain)),
        ContentCoding::Identity => plain.clone(),
    };
    if encoded.len() >= plain.len() {
        msg.body = plain;
        return Ok(msg);
    }
    msg.body = encoded;
    msg.header.fields.insert(
        HeaderName::ContentEncoding,
        HeaderFieldValue::Generic(fmt!("{}", coding.token())),
        None,
    );
    // The encoded body is a different representation, so it needs a validator of
    // its own. A client holding one must not be able to claim it holds the other.
    if let Some(val) = msg.header.fields.get_one(&HeaderName::ETag) {
        let renamed = tagged(&fmt!("{}", val), coding);
        msg.header.fields.insert(
            HeaderName::ETag,
            HeaderFieldValue::Generic(renamed),
            None,
        );
    }
    Ok(msg)
}


#[cfg(test)]
mod tests {
    use super::*;

    use crate::http::status::HttpStatus;

    /// RFC 9110 §12.5.3: a request that names no coding is offered none.
    #[test]
    fn a_request_that_asks_for_nothing_is_sent_as_it_is() {
        assert_eq!(negotiate(None), ContentCoding::Identity);
        assert_eq!(negotiate(Some("")), ContentCoding::Identity);
    }

    /// The field every browser actually sends.
    #[test]
    fn a_browser_asking_for_gzip_is_given_gzip() {
        assert_eq!(negotiate(Some("gzip, deflate, br")), ContentCoding::Gzip);
        assert_eq!(negotiate(Some("gzip")), ContentCoding::Gzip);
        assert_eq!(negotiate(Some("GZIP")), ContentCoding::Gzip);
        assert_eq!(negotiate(Some("x-gzip")), ContentCoding::Gzip);
    }

    /// A coding neither named nor covered by `*` is not acceptable, which is
    /// what sending the field is for.
    #[test]
    fn a_coding_that_was_not_asked_for_is_not_sent() {
        assert_eq!(negotiate(Some("deflate, br")), ContentCoding::Identity);
        assert_eq!(negotiate(Some("br;q=1.0")), ContentCoding::Identity);
    }

    /// RFC 9110 §12.5.3: `q=0` means not acceptable.
    #[test]
    fn a_zero_weight_refuses_the_coding() {
        assert_eq!(negotiate(Some("gzip;q=0")), ContentCoding::Identity);
        assert_eq!(negotiate(Some("gzip;q=0.000")), ContentCoding::Identity);
        assert_eq!(negotiate(Some("gzip;q=0, deflate")), ContentCoding::Identity);
    }

    /// The asterisk "matches any available content coding not explicitly
    /// listed" -- RFC 9110 §12.5.3.
    #[test]
    fn the_asterisk_speaks_for_a_coding_not_named() {
        assert_eq!(negotiate(Some("*")), ContentCoding::Gzip);
        assert_eq!(negotiate(Some("deflate, *")), ContentCoding::Gzip);
        // Named outright, the entry beats the wildcard.
        assert_eq!(negotiate(Some("*, gzip;q=0")), ContentCoding::Identity);
        // And the wildcard can refuse everything it is left to speak for.
        assert_eq!(negotiate(Some("*;q=0")), ContentCoding::Identity);
    }

    /// Identity is acceptable by default and refusable outright.
    #[test]
    fn identity_is_assumed_unless_it_is_refused() {
        // Refusing identity leaves gzip the only thing that can be sent.
        assert_eq!(negotiate(Some("gzip, identity;q=0")), ContentCoding::Gzip);
        // `*;q=0` refuses identity too, since identity is not named separately.
        assert_eq!(negotiate(Some("gzip, *;q=0")), ContentCoding::Gzip);
        // A more specific entry for identity overrides the wildcard.
        assert_eq!(negotiate(Some("*;q=0, identity")), ContentCoding::Identity);
    }

    /// The greatest weight wins; a tie is the server's to break.
    #[test]
    fn the_heavier_coding_wins_and_a_tie_goes_to_gzip() {
        assert_eq!(negotiate(Some("gzip;q=0.5, identity;q=1.0")), ContentCoding::Identity);
        assert_eq!(negotiate(Some("gzip;q=1.0, identity;q=0.5")), ContentCoding::Gzip);
        assert_eq!(negotiate(Some("gzip;q=1.0, identity;q=1.0")), ContentCoding::Gzip);
        // Thousandths, so the finest distinction the scale allows still decides.
        assert_eq!(negotiate(Some("gzip;q=0.501, identity;q=0.500")), ContentCoding::Gzip);
        assert_eq!(negotiate(Some("gzip;q=0.500, identity;q=0.501")), ContentCoding::Identity);
    }

    /// RFC 9110 §12.4.2 gives the qvalue three decimal places and a ceiling of
    /// one. Anything else is not a weight, and the entry keeps the default.
    #[test]
    fn a_qvalue_outside_the_scale_is_not_a_weight() {
        assert_eq!(qvalue("0"), Some(0));
        assert_eq!(qvalue("1"), Some(1000));
        assert_eq!(qvalue("0.5"), Some(500));
        assert_eq!(qvalue("0.05"), Some(50));
        assert_eq!(qvalue("0.005"), Some(5));
        assert_eq!(qvalue("1.000"), Some(1000));
        assert_eq!(qvalue("1.001"), None);
        assert_eq!(qvalue("2"), None);
        assert_eq!(qvalue("0.0001"), None);
        assert_eq!(qvalue("abc"), None);
        // A weight that is not a weight leaves the coding named and acceptable.
        assert_eq!(negotiate(Some("gzip;q=nonsense")), ContentCoding::Gzip);
    }

    /// Whitespace around the entries and their parameters is optional per the
    /// ABNF, so a field written either way means the same thing.
    #[test]
    fn the_spacing_of_the_field_does_not_change_its_meaning() {
        assert_eq!(negotiate(Some("gzip;q=0.9,identity;q=1.0")), ContentCoding::Identity);
        assert_eq!(negotiate(Some(" gzip ; q=0.9 , identity ; q=1.0 ")),
            ContentCoding::Identity);
    }

    /// The eligibility list, by media type.
    #[test]
    fn only_a_type_that_gains_by_it_is_encoded() {
        for ct in [
            "text/html; charset=utf-8",
            "text/css",
            "text/plain",
            "text/javascript; charset=utf-8",
            "application/json",
            "application/manifest+json",
            "application/xml",
            "application/problem+json",
            "image/svg+xml",
            "application/wasm",
            "font/ttf",
            // Text by definition, subtype modelled or not.
            "text/markdown",
            "text/calendar",
            "TEXT/HTML",
            // The other spellings of script, which a proxied upstream may use.
            "application/javascript",
            "application/x-javascript; charset=utf-8",
            "application/ecmascript",
        ] {
            assert!(is_compressible(ct), "{} should be encoded", ct);
        }
        for ct in [
            "image/png",
            "image/jpeg",
            "image/webp",
            "image/avif",
            "image/gif",
            "font/woff",
            "font/woff2",
            "audio/ogg",
            "audio/mpeg",
            "video/mp4",
            "video/webm",
            "application/zip",
            "application/zstd",
            "application/pdf",
            // Not a media type at all, so nothing is assumed about it.
            "",
            "nonsense",
        ] {
            assert!(!is_compressible(ct), "{} should be sent as it is", ct);
        }
    }

    /// A body too small to be worth the framing is sent as it is, whatever the
    /// request said.
    #[test]
    fn a_small_body_is_below_the_floor() -> Outcome<()> {
        let mut fields = HeaderFields::default();
        fields.insert(
            HeaderName::AcceptEncoding,
            res!(HeaderFieldValue::new(&HeaderName::AcceptEncoding, "gzip")),
            None,
        );
        assert_eq!(
            choose(&fields, "text/html", MIN_BYTES_DEFAULT - 1, MIN_BYTES_DEFAULT),
            ContentCoding::Identity);
        assert_eq!(
            choose(&fields, "text/html", MIN_BYTES_DEFAULT, MIN_BYTES_DEFAULT),
            ContentCoding::Gzip);
        assert_eq!(
            choose(&fields, "image/png", 1_000_000, MIN_BYTES_DEFAULT),
            ContentCoding::Identity);
        Ok(())
    }

    /// Two encodings of one representation must not share a validator.
    #[test]
    fn an_encoded_body_carries_a_tag_of_its_own() {
        assert_eq!(tagged("\"68a1-3b\"", ContentCoding::Gzip), "\"68a1-3b-gzip\"");
        assert_eq!(tagged("\"68a1-3b\"", ContentCoding::Identity), "\"68a1-3b\"");
        assert_eq!(tagged("W/\"68a1-3b\"", ContentCoding::Gzip), "W/\"68a1-3b-gzip\"");
        // Not a quoted tag; better left alone than turned into a malformed one.
        assert_eq!(tagged("68a1", ContentCoding::Gzip), "68a1");
    }

    /// A `HEAD` answer is not encoded, and keeps the length of the identity
    /// representation -- which is what a `GET` accepting no coding would be told,
    /// and what anyone asking how big a thing is wants to know.
    #[tokio::test]
    async fn a_head_answer_is_not_encoded() -> Outcome<()> {
        let body = "<p>a paragraph of markup</p>\n".repeat(500).into_bytes();
        let plain = body.len();
        let msg = HttpMessage::new_response(HttpStatus::OK)
            .with_field(
                HeaderName::ContentType,
                HeaderFieldValue::Generic(fmt!("text/html; charset=utf-8")),
            )
            .with_body(body)
            .head_only();
        assert!(!is_encodable(&msg), "a HEAD answer was offered to the encoder");
        let out = res!(encode(msg, ContentCoding::Gzip).await);
        assert_eq!(out.body_len(), plain, "a HEAD answer did not state the identity length");
        assert!(out.header.fields.get_one(&HeaderName::ContentEncoding).is_none(),
            "a HEAD answer named a coding it had not applied");
        // It still says the representation varies by coding: a store keyed on the
        // URL alone would otherwise hand this to the next client along.
        let vary = res!(out.header.fields.get_one(&HeaderName::Vary).ok_or_else(||
            err!("The HEAD answer did not say it varies by coding."; Missing)));
        assert!(fmt!("{}", vary).to_ascii_lowercase().contains("accept-encoding"),
            "got: {}", vary);
        Ok(())
    }

    /// The file a `HEAD` answer names is never opened, which is the cost the guard
    /// is there to save: encoding a window means reading it off the disk first, and
    /// a `HEAD` throws the result away unsent.
    #[tokio::test]
    async fn a_head_answer_leaves_its_file_unread() -> Outcome<()> {
        use crate::http::msg::FileWindow;
        // A window on a path that does not exist, so a run that reads it fails
        // rather than merely being slower than it should be.
        let msg = HttpMessage::new_response(HttpStatus::OK)
            .with_field(
                HeaderName::ContentType,
                HeaderFieldValue::Generic(fmt!("text/html; charset=utf-8")),
            )
            .with_file_window(FileWindow::new(
                std::path::PathBuf::from("/nonexistent/no-such-file.html"), 0, 4096))
            .head_only();
        let out = res!(encode(msg, ContentCoding::Gzip).await);
        assert_eq!(out.body_len(), 4096, "the window was not left as the body");
        Ok(())
    }

    /// The encoder's own output, read back by the decoder beside it. This says
    /// only that the pair agree; the test that the stream is really gzip is in
    /// `tests/`, against `gzip(1)`.
    #[test]
    fn a_gzip_member_round_trips() -> Outcome<()> {
        let plain = "the quick brown fox jumps over the lazy dog\n"
            .repeat(200).into_bytes();
        let encoded = res!(gzip(&plain));
        assert!(encoded.len() < plain.len() / 4,
            "{} bytes encoded to {}", plain.len(), encoded.len());
        // RFC 1952 §2.3.1: every member begins with the two magic bytes and the
        // compression method.
        assert_eq!(&encoded[..3], &[0x1f, 0x8b, 0x08]);
        assert_eq!(res!(gunzip(&encoded)), plain);
        Ok(())
    }
}
