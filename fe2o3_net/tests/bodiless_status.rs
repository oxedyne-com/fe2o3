#![cfg(feature = "async")]
//! A response whose status carries no body must not be framed as though it had one.
//!
//! RFC 9110 §8.6 forbids `Content-Length` on a `1xx` or `204`, and on a `304` the field would
//! have to state the size of the representation not sent. The writer used to add
//! `Content-Length: 0` to every message it sent, so a `204` for an empty map tile went out
//! carrying a field the specification forbids.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_net::http::{
    msg::HttpMessage,
    status::HttpStatus,
};


async fn wire(msg: HttpMessage) -> Outcome<String> {
    let mut out: Vec<u8> = Vec::new();
    res!(msg.write_all(&mut out).await);
    Ok(String::from_utf8_lossy(&out).to_lowercase())
}

#[tokio::test]
async fn no_content_length_on_bodiless_statuses() -> Outcome<()> {
    for status in [HttpStatus::NoContent, HttpStatus::NotModified, HttpStatus::EarlyHints] {
        let head = res!(wire(HttpMessage::new_response(status)).await);
        assert!(!head.contains("content-length"),
            "a {} response carried a Content-Length:\n{}", status as u16, head);
    }
    Ok(())
}

#[tokio::test]
async fn content_length_still_frames_other_statuses() -> Outcome<()> {
    let head = res!(wire(HttpMessage::new_response(HttpStatus::OK)).await);
    assert!(head.contains("content-length: 0\r\n"),
        "an empty 200 lost its Content-Length:\n{}", head);
    let head = res!(wire(HttpMessage::respond_with_text(HttpStatus::NotFound, "gone")).await);
    assert!(head.contains("content-length: 4\r\n"),
        "a 404 with a body lost its Content-Length:\n{}", head);
    Ok(())
}
