//! Whether Steel ends a connection after the reply, by what the request asked.
//!
//! RFC 9112 section 9.3: a request in HTTP/1.1 is persistent unless it carries `Connection:
//! close`; one in HTTP/1.0 is not persistent unless it carries `Connection: keep-alive`. HTTP/1.0
//! requests became acceptable to Steel when the library learned the version (a client such as `ab`
//! or `curl -0` is now answered), but the dispatcher kept every connection open for the next
//! request, so such a client waited out the idle timeout on each one, and a `Connection: close`
//! was honoured by the client dropping the line, never by the server.
//!
//! Each case sends one raw request through `handle_https` and watches the connection afterwards.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

mod vhost_rig;

use vhost_rig::{
    Exchange,
    Rig,
    Scratch,
    Site,
};

use oxedyne_fe2o3_core::prelude::*;

use std::time::Duration;

const HOST: &str = "close.test";

async fn ask(rig: &Rig, request_line: &str, fields: &[&str], patience_ms: u64) -> Outcome<Exchange> {
    let mut raw = fmt!("{}\r\nHost: {}\r\n", request_line, HOST);
    for f in fields {
        raw.push_str(f);
        raw.push_str("\r\n");
    }
    raw.push_str("\r\n");
    rig.exchange(HOST, &raw, Duration::from_millis(patience_ms)).await
}

// A rig over one vhost, and one raw request put to it.
fn put(label: &str, request_line: &str, fields: &[&str], patience_ms: u64) -> Outcome<Exchange> {
    let web = res!(Scratch::new(label));
    res!(web.put("index.html", "<p>home</p>"));
    let runtime = res!(vhost_rig::runtime());
    runtime.block_on(async {
        let rig = res!(Rig::new(vec![Site::new(HOST, &web.0)]));
        ask(&rig, request_line, fields, patience_ms).await
    })
}

#[test]
fn an_http_1_0_request_without_keep_alive_is_closed_after_the_reply() -> Outcome<()> {
    let x = res!(put("h10_plain", "GET / HTTP/1.0", &[], 3000));
    assert_eq!(x.reply.status(), 200, "the HTTP/1.0 request must still be answered: {:?}", x.reply.head);
    assert!(x.closed, "an HTTP/1.0 request without keep-alive must be closed after the reply");
    Ok(())
}

#[test]
fn an_http_1_0_request_that_asks_for_keep_alive_is_kept_open_and_told_so() -> Outcome<()> {
    let x = res!(put("h10_keep", "GET / HTTP/1.0", &["Connection: Keep-Alive"], 600));
    assert_eq!(x.reply.status(), 200);
    assert!(!x.closed, "an HTTP/1.0 request that asks for keep-alive must be kept open");
    assert_eq!(x.reply.field("connection").as_deref(), Some("keep-alive"),
        "the reply must grant the keep-alive an HTTP/1.0 client asked for: {:?}", x.reply.head);
    Ok(())
}

#[test]
fn an_http_1_1_request_is_kept_open_by_default() -> Outcome<()> {
    let x = res!(put("h11_plain", "GET / HTTP/1.1", &[], 600));
    assert_eq!(x.reply.status(), 200);
    assert!(!x.closed, "an HTTP/1.1 request must be kept open by default");
    Ok(())
}

#[test]
fn an_http_1_1_request_with_connection_close_is_closed_by_the_server() -> Outcome<()> {
    let x = res!(put("h11_close", "GET / HTTP/1.1", &["Connection: close"], 3000));
    assert_eq!(x.reply.status(), 200);
    assert_eq!(x.reply.field("connection").as_deref(), Some("close"));
    assert!(x.closed, "an HTTP/1.1 request with Connection: close must be closed after the reply");
    Ok(())
}
