//! HTTP caching for static responses: entity tags, conditional requests and
//! cache directives.
//!
//! A server that emits no validators can never answer `304 Not Modified`, so it
//! re-sends every byte of every asset on every request, however little has
//! changed. A server that emits no cache directives leaves the browser to guess
//! how long it may keep a document, and a browser guessing about an application
//! shell will eventually serve a stale one. This module supplies both halves.

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_net::http::{
    fields::{
        HeaderFields,
        HeaderFieldValue,
        HeaderName,
    },
    msg::HttpMessage,
    status::HttpStatus,
};

use std::{
    fs::Metadata,
    time::UNIX_EPOCH,
};


/// Entity tag for a static file, derived from its modification time and size.
///
/// The pair is what the filesystem already knows, and it changes whenever the
/// file does. A digest of the contents would be a stronger tag, but computing one
/// means reading the whole file on every conditional request, which is precisely
/// the work the tag exists to avoid.
pub fn entity_tag(meta: &Metadata) -> Outcome<String> {
    let modified = res!(meta.modified());
    let secs = match modified.duration_since(UNIX_EPOCH) {
        Ok(dur) => dur.as_secs(),
        Err(_)  => 0, // A file dated before the epoch is not stale, merely odd.
    };
    Ok(fmt!("\"{:x}-{:x}\"", secs, meta.len()))
}

/// Does the client already hold this exact entity?
///
/// Per RFC 9110 §13.1.2 an `If-None-Match` listing the current tag, or `*`, means
/// the copy in hand is current and the body must not be sent again. A weak tag is
/// accepted against its strong twin, since this comparison is about identity, not
/// byte-for-byte equivalence.
pub fn is_current(req: &HeaderFields, etag: &str) -> bool {
    match req.get_one(&HeaderName::IfNoneMatch) {
        Some(val) => fmt!("{}", val)
            .split(',')
            .map(|given| given.trim())
            .any(|given|
                given == "*"
                || given == etag
                || given.strip_prefix("W/").map_or(false, |given| given == etag)
            ),
        None => false,
    }
}

/// Cache directive for a static response.
///
/// An entry document is always revalidated, because a deploy that changes it is
/// invisible to anyone still holding the old one. Every other asset may be held
/// for `max_age_secs`, which an operator should raise above zero only when the
/// filenames carry a content hash, since a cached asset under a stable name
/// survives the deploy that replaced it. The default of zero revalidates
/// everything, which the entity tag makes cheap.
pub fn cache_control(content_type: &str, max_age_secs: u32) -> String {
    if is_document(content_type) || max_age_secs == 0 {
        fmt!("no-cache")
    } else {
        fmt!("public, max-age={}", max_age_secs)
    }
}

/// Is this an entry document, rather than an asset it refers to?
pub fn is_document(content_type: &str) -> bool {
    content_type.contains("text/html")
}

/// A `304 Not Modified`: the validators and directives, and no body.
pub fn not_modified(etag: String, directive: String) -> Outcome<HttpMessage> {
    Ok(HttpMessage::new_response(HttpStatus::NotModified)
        .with_field(HeaderName::ETag, res!(HeaderFieldValue::new(
            &HeaderName::ETag, &etag)))
        .with_field(HeaderName::CacheControl, res!(HeaderFieldValue::new(
            &HeaderName::CacheControl, &directive))))
}


#[cfg(test)]
mod tests {
    use super::*;

    fn headers_with(name: HeaderName, value: &str) -> Outcome<HeaderFields> {
        let mut fields = HeaderFields::default();
        fields.insert(name.clone(), res!(HeaderFieldValue::new(&name, value)), None);
        Ok(fields)
    }

    #[test]
    fn if_none_match_recognises_the_current_tag() -> Outcome<()> {
        let fields = res!(headers_with(HeaderName::IfNoneMatch, "\"abc-10\""));
        assert!(is_current(&fields, "\"abc-10\""));
        assert!(!is_current(&fields, "\"abc-11\""));
        Ok(())
    }

    #[test]
    fn if_none_match_accepts_a_list_a_wildcard_and_a_weak_tag() -> Outcome<()> {
        let listed = res!(headers_with(
            HeaderName::IfNoneMatch, "\"other\", \"abc-10\""));
        assert!(is_current(&listed, "\"abc-10\""));

        let wildcard = res!(headers_with(HeaderName::IfNoneMatch, "*"));
        assert!(is_current(&wildcard, "\"abc-10\""));

        let weak = res!(headers_with(HeaderName::IfNoneMatch, "W/\"abc-10\""));
        assert!(is_current(&weak, "\"abc-10\""));
        Ok(())
    }

    #[test]
    fn a_request_without_the_header_is_never_current() -> Outcome<()> {
        let fields = HeaderFields::default();
        assert!(!is_current(&fields, "\"abc-10\""));
        Ok(())
    }

    #[test]
    fn a_document_always_revalidates_however_long_the_max_age() {
        assert_eq!(cache_control("text/html; charset=utf-8", 31536000), "no-cache");
        assert_eq!(cache_control("text/html", 0), "no-cache");
    }

    #[test]
    fn an_asset_is_held_only_when_the_operator_asks_for_it() {
        assert_eq!(cache_control("application/wasm", 0), "no-cache");
        assert_eq!(cache_control("application/wasm", 3600), "public, max-age=3600");
    }
}
