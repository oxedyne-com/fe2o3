//! A request target read into an `HttpLocator` is written back byte for byte. A signed query
//! (sorted or not, with repeated keys, empty values, numbers or padded base64) must reach the
//! wire as it was signed, so the locator never rebuilds a query it was given (REL-1i: a
//! runner's wake was refused because the query went out as `"device"="..."&"ts"=(u32|...)`).

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_net::http::loc::HttpLocator;

const TARGETS: &[&str] = &[
    "/api/sync/ws?w=rn1a2b3c19a3f0c2d11&device=r2cfixturedevice&ts=1793352150&nonce=oysOzVAmnE4uvsXKi1O7SQ&sigv=2&sig=bvwVjtvw7rJqeAa_wBv2yZbGPLzH1KsasCwJ-_kiGsBxiOite0daGgsuNXSDqn7if6ob3_1P8tHJ3wXwDvXvDQ&caps=pong",
    "/p?b=2&a=1",               // Keys out of order.
    "/p?a=1&a=2",               // A repeated key.
    "/p?a=&b=x",                // An empty value.
    "/p?n=007&f=1.50&t=true",   // Values a daticle reader would re-render.
    "/p?sig=YWJj&pad=YQ==",     // Padded base64.
    "/p?have=s1%3A1&x=a+b",     // Percent escapes and a plus, as sent.
    "/p?q=a?b",                 // A `?` inside the query.
    "/p?a=1#sec",               // A fragment.
    "/p",                       // No query.
];

#[test]
fn test_a_locator_writes_its_target_back_byte_for_byte() -> Outcome<()> {
    for t in TARGETS {
        let loc = res!(HttpLocator::new(*t));
        assert_eq!(loc.to_string(), *t, "the locator rewrote {}", t);
    }
    Ok(())
}

#[test]
fn test_a_query_with_a_question_mark_is_kept_whole() -> Outcome<()> {
    let loc = res!(HttpLocator::new("/p?q=a?b&r=1"));
    assert_eq!(loc.query, "q=a?b&r=1");
    assert_eq!(loc.query_str("q"), Some(fmt!("a?b")));
    assert_eq!(loc.query_str("r"), Some(fmt!("1")));
    Ok(())
}

#[cfg(feature = "async")]
#[test]
fn test_a_websocket_dial_sends_the_target_it_was_given() -> Outcome<()> {
    for t in TARGETS {
        let (msg, _key) = res!(oxedyne_fe2o3_net::ws::core::connect_request("h.example", t, None));
        let line = fmt!("GET {} HTTP/1.1\r\n", t);
        assert!(msg.header.to_string().starts_with(&line), "the dial rewrote {}", t);
    }
    Ok(())
}
