//! HTTP caching for static responses: entity tags, conditional requests and
//! cache directives.
//!
//! A server that emits no validators can never answer `304 Not Modified`, so it
//! re-sends every byte of every asset on every request, however little has
//! changed. A server that emits no cache directives leaves the browser to guess
//! how long it may keep a document, and a browser guessing about an application
//! shell will eventually serve a stale one. This module supplies both halves.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_net::http::{
    encoding,
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
    path::Path,
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
/// Three cases, in order.
///
/// An entry document is always revalidated, because a deploy that changes it is
/// invisible to anyone still holding the old one. That holds whatever the
/// filename says, since a document is the thing a reader has bookmarked.
///
/// An asset whose filename carries a content hash may be held for
/// `fingerprint_max_age_secs` and marked `immutable` (RFC 8246): the name is a
/// promise that the bytes under it cannot change, so revalidating it can only
/// ever confirm what the client already has. `immutable` is what stops a browser
/// asking again on a manual reload.
///
/// Every other asset may be held for `max_age_secs`, which an operator should
/// raise above zero only when the filenames carry a content hash, since a cached
/// asset under a stable name survives the deploy that replaced it. The default
/// of zero revalidates everything, which the entity tag makes cheap.
pub fn cache_control(
    content_type:   &str,
    path:           &Path,
    max_age_secs:   u32,
    fingerprint_secs: u32,
)
    -> String
{
    if is_document(content_type) {
        return fmt!("no-cache");
    }
    if fingerprint_secs > 0 && is_fingerprinted(path) {
        return fmt!("public, max-age={}, immutable", fingerprint_secs);
    }
    if max_age_secs == 0 {
        fmt!("no-cache")
    } else {
        fmt!("public, max-age={}", max_age_secs)
    }
}

/// Is this an entry document, rather than an asset it refers to?
pub fn is_document(content_type: &str) -> bool {
    content_type.contains("text/html")
}

/// Does this filename carry a content hash?
///
/// Every build tool that fingerprints its output puts a run of hex in the name
/// -- `app.4f3a9c21.js`, `main-8ab19c7e.css`, `module_1f2e3d4c_bg.wasm` -- and
/// the point of doing so is that a changed file gets a different name. That is
/// what makes a year-long `max-age` safe, and nothing else does.
///
/// The test is deliberately narrow, because a false positive means a browser
/// holding a stale file for a year:
///
/// - the run is at least eight characters, which is the shortest hash any of
///   these tools emits;
/// - every character is a hex digit, so words are not mistaken for hashes;
/// - at least one is a numeral and at least one a letter, so neither a run of
///   letters that happens to be hex (`deadbeef`, `facecafe`, and every English
///   word spellable in `a`--`f`) nor a run of numerals that is plainly a date or
///   an identifier (`20260728`) is mistaken for a hash;
/// - and it stands as its own segment, delimited by `.`, `-` or `_`, so a hash
///   is never read out of the middle of a longer word.
///
/// A real hash trips all four almost always: an eight-character hex digest
/// misses only when it happens to be all letters or all numerals, which is about
/// one name in forty, and the miss costs a revalidation rather than a stale
/// file.
pub fn is_fingerprinted(path: &Path) -> bool {
    let name = match path.file_name().and_then(|n| n.to_str()) {
        Some(n) => n,
        None    => return false,
    };
    name.split(|c| c == '.' || c == '-' || c == '_').any(|seg|
        seg.len() >= 8
        && seg.bytes().all(|b| b.is_ascii_hexdigit())
        && seg.bytes().any(|b| b.is_ascii_digit())
        && seg.bytes().any(|b| b.is_ascii_alphabetic())
    )
}

/// Stamp a response the server generated, so no store may serve it unasked.
///
/// A generated response describes the site at the instant it was asked for, and
/// the next thing an author writes changes it. Carrying no directive and no
/// validator, it is not merely uncached -- RFC 9111 §4.2.2 lets a store invent a
/// freshness lifetime for it, and a browser will then redraw a page from a copy
/// taken before the post existed. That is the stale index an author has to force
/// a refresh to get past, and forcing a refresh is not something a reader will
/// think to do. `no-cache` keeps the store and forbids the guess: the response
/// may be held, and may never be used without asking first.
///
/// A response that already says how long it may be held keeps what it said. This
/// is a default for the responses that say nothing, not an override -- and
/// `Cache-Control` is a list field, so appending a second directive would leave
/// both in force rather than replacing the first.
pub fn generated(resp: HttpMessage) -> HttpMessage {
    if resp.header.fields.get_one(&HeaderName::CacheControl).is_some() {
        return resp;
    }
    resp.with_field(
        HeaderName::CacheControl,
        HeaderFieldValue::Generic(fmt!("no-cache")),
    )
}

/// Fails unless a response forbids a store from serving it unasked.
///
/// The invariant [`generated`] exists to keep, in one place so that the tests of every surface that
/// must hold it say the same thing -- and so a refactor that drops one of those calls fails a test
/// rather than going out. Six surfaces hold it and one of them was tested; that is how five of them
/// came to be able to break quietly.
#[cfg(test)]
pub fn assert_not_held(resp: &HttpMessage, what: &str) {
    match resp.header.fields.get_one(&HeaderName::CacheControl) {
        Some(val) => {
            let directive = fmt!("{}", val).to_ascii_lowercase();
            assert!(directive.contains("no-cache") || directive.contains("no-store"),
                "{} may be served from a store unasked: '{}'", what, directive);
        }
        None => panic!(
            "{} carried no cache directive, so a store is free to invent a lifetime for it", what),
    }
}

/// A `304 Not Modified`: the validators and directives, and no body.
///
/// `varies_by_encoding` says whether the representation is one the server would
/// have offered a content coding for. It has to be said here as much as on a
/// `200`: RFC 9111 §4.3.4 has a cache update its stored response from the fields
/// of the `304`, so a `Vary` omitted here would undo the one stored with the
/// body, and the cache would go back to serving one encoding to everybody.
pub fn not_modified(
    etag:               String,
    directive:          String,
    varies_by_encoding: bool,
)
    -> Outcome<HttpMessage>
{
    let mut msg = HttpMessage::new_response(HttpStatus::NotModified)
        .with_field(HeaderName::ETag, res!(HeaderFieldValue::new(
            &HeaderName::ETag, &etag)))
        .with_field(HeaderName::CacheControl, res!(HeaderFieldValue::new(
            &HeaderName::CacheControl, &directive)));
    if varies_by_encoding {
        encoding::mark_varying(&mut msg);
    }
    Ok(msg)
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

    const YEAR: u32 = 31_536_000;

    fn at(name: &str) -> std::path::PathBuf {
        std::path::PathBuf::from("/srv/www").join(name)
    }

    #[test]
    fn a_document_always_revalidates_however_long_the_max_age() {
        assert_eq!(
            cache_control("text/html; charset=utf-8", &at("index.html"), YEAR, YEAR),
            "no-cache");
        assert_eq!(cache_control("text/html", &at("index.html"), 0, YEAR), "no-cache");
        // Even one whose own name carries a hash: a document is the thing a
        // reader has bookmarked, and a deploy that changes it must be seen.
        assert_eq!(
            cache_control("text/html", &at("page.4f3a9c21.html"), 0, YEAR),
            "no-cache");
    }

    /// A name that carries a content hash is a promise the bytes cannot change
    /// under it, which is the only thing that makes a year safe.
    #[test]
    fn a_hashed_name_is_held_and_never_revalidated() {
        assert_eq!(
            cache_control("application/wasm", &at("module_1f2e3d4c_bg.wasm"), 0, YEAR),
            fmt!("public, max-age={}, immutable", YEAR));
        // The operator can switch the whole treatment off.
        assert_eq!(
            cache_control("application/wasm", &at("module_1f2e3d4c_bg.wasm"), 0, 0),
            "no-cache");
    }

    /// Narrow on purpose: a false positive means a browser holding a stale file
    /// for a year.
    #[test]
    fn only_a_name_that_really_carries_a_hash_is_read_as_one() {
        for name in [
            "app.4f3a9c21.js",
            "main-8ab19c7e.css",
            "module_1f2e3d4c_bg.wasm",
            "sha-2c26b46b68ffc68ff99b453c1d30413413422d706483bfa0f98a5e886266e7ae.bin",
        ] {
            assert!(is_fingerprinted(&at(name)), "{} carries a hash", name);
        }
        for name in [
            "index.html",
            "explayna_bg.wasm",
            "app.js",
            "style.css",
            // Hex, but all letters: an English word, not a digest.
            "deadbeef.js",
            "facecafe.css",
            // All numerals: a date or an identifier, not a digest.
            "20260728.json",
            "post-20260728.html",
            // Too short to be any tool's output.
            "app.4f3a9c.js",
            // Hash-shaped, but buried in a longer word rather than its own segment.
            "prefix4f3a9c21suffix.js",
        ] {
            assert!(!is_fingerprinted(&at(name)), "{} does not carry a hash", name);
        }
    }

    /// A generated response says so, rather than leaving a store to guess a lifetime for it.
    #[test]
    fn a_generated_response_is_never_served_unasked() -> Outcome<()> {
        let resp = generated(HttpMessage::new_response(HttpStatus::OK));
        let held = res!(resp.header.fields.get_one(&HeaderName::CacheControl).ok_or_else(||
            err!("A generated response carried no cache directive."; Missing)));
        assert_eq!(fmt!("{}", held), "no-cache");
        Ok(())
    }

    /// `Cache-Control` is a list field, so a second stamp would leave both directives in force.
    #[test]
    fn stamping_twice_leaves_one_directive() -> Outcome<()> {
        let resp = generated(generated(HttpMessage::new_response(HttpStatus::OK)));
        let all = res!(resp.header.fields.get_list(&HeaderName::CacheControl).ok_or_else(||
            err!("A generated response carried no cache directive."; Missing)));
        assert_eq!(all.len(), 1, "The directive was repeated rather than replaced.");
        Ok(())
    }

    /// A response that has said how long it may be held is not overruled by the default.
    #[test]
    fn an_explicit_directive_survives_the_default() -> Outcome<()> {
        let held = HttpMessage::new_response(HttpStatus::OK)
            .with_field(
                HeaderName::CacheControl,
                HeaderFieldValue::Generic(fmt!("public, max-age=86400")),
            );
        let resp = generated(held);
        let all = res!(resp.header.fields.get_list(&HeaderName::CacheControl).ok_or_else(||
            err!("The directive went missing."; Missing)));
        assert_eq!(all.len(), 1);
        assert_eq!(fmt!("{}", all[0]), "public, max-age=86400");
        Ok(())
    }

    #[test]
    fn an_asset_is_held_only_when_the_operator_asks_for_it() {
        assert_eq!(cache_control("application/wasm", &at("app_bg.wasm"), 0, YEAR),
            "no-cache");
        assert_eq!(cache_control("application/wasm", &at("app_bg.wasm"), 3600, YEAR),
            "public, max-age=3600");
    }

    /// A `304` restates the fields a cache stores, so one that dropped `Vary`
    /// would send the cache back to serving one encoding to everybody.
    #[test]
    fn a_not_modified_repeats_what_the_response_varies_by() -> Outcome<()> {
        let varying = res!(not_modified(
            fmt!("\"abc-10-gzip\""), fmt!("no-cache"), true));
        let held = res!(varying.header.fields.get_one(&HeaderName::Vary).ok_or_else(||
            err!("The 304 did not say what it varies by."; Missing)));
        assert_eq!(fmt!("{}", held).to_ascii_lowercase(), "accept-encoding");

        let fixed = res!(not_modified(fmt!("\"abc-10\""), fmt!("no-cache"), false));
        assert!(fixed.header.fields.get_one(&HeaderName::Vary).is_none(),
            "a representation with only one form does not vary");
        Ok(())
    }
}
