//! The gzip encoder, checked against `gzip(1)` rather than against itself.
//!
//! A codec tested only against its own decoder agrees with itself, which is
//! something a consistently wrong codec does just as well. The authority on
//! whether a stream really is [RFC 1952] gzip is a program that did not come
//! from this repository and that every client in the world already trusts, so
//! both directions are put to it: what this crate writes, `gunzip` must read,
//! and what `gzip` writes, this crate must read.
//!
//! [RFC 1952]: https://www.rfc-editor.org/rfc/rfc1952

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_net::http::encoding;

use std::{
    fs,
    path::PathBuf,
    process::{
        Command,
        Stdio,
    },
};


/// A working directory under the target tree, which Cargo hands to an
/// integration test for exactly this. Never `/tmp`, which on these machines is
/// memory.
fn work_dir(name: &str) -> Outcome<PathBuf> {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    res!(fs::create_dir_all(&dir));
    Ok(dir)
}

/// Prose with the redundancy real markup has, so a ratio means something.
fn sample() -> Vec<u8> {
    let mut out = String::new();
    for i in 0..400 {
        out.push_str(&fmt!(
            "<p class=\"line\" data-index=\"{}\">the quick brown fox jumps over \
            the lazy dog</p>\n", i));
    }
    out.into_bytes()
}

/// Is the external tool available at all? A missing oracle must be loud, not a
/// quietly passing test.
fn require(tool: &str) -> Outcome<()> {
    let found = match Command::new("which").arg(tool).stdout(Stdio::null()).status() {
        Ok(st)  => st.success(),
        Err(_)  => false,
    };
    if !found {
        return Err(err!(
            "The external oracle '{}' is not on PATH, so this test cannot check \
            anything it claims to check.", tool;
            Missing, Init));
    }
    Ok(())
}

/// What this crate writes, `gunzip` reads back byte for byte.
#[test]
fn gunzip_reads_what_this_crate_wrote() -> Outcome<()> {
    res!(require("gunzip"));
    let plain = sample();
    let encoded = res!(encoding::gzip(&plain));

    let dir = res!(work_dir("gzip_oracle_out"));
    let path = dir.join("body.gz");
    let _ = fs::remove_file(&path);
    let _ = fs::remove_file(dir.join("body"));
    res!(fs::write(&path, &encoded));

    let out = res!(Command::new("gunzip").arg("-c").arg(&path).output());
    if !out.status.success() {
        return Err(err!(
            "gunzip refused the stream this crate produced: {}",
            String::from_utf8_lossy(&out.stderr);
            Invalid, Data));
    }
    assert_eq!(out.stdout, plain,
        "gunzip read back something other than what was encoded");
    // And the saving is the point of the exercise.
    assert!(encoded.len() * 4 < plain.len(),
        "{} bytes of markup encoded to {}, which is not worth doing",
        plain.len(), encoded.len());
    Ok(())
}

/// What `gzip` writes, this crate reads back byte for byte.
#[test]
fn this_crate_reads_what_gzip_wrote() -> Outcome<()> {
    res!(require("gzip"));
    let plain = sample();

    let dir = res!(work_dir("gzip_oracle_in"));
    let path = dir.join("body");
    let gz_path = dir.join("body.gz");
    let _ = fs::remove_file(&path);
    let _ = fs::remove_file(&gz_path);
    res!(fs::write(&path, &plain));

    let out = res!(Command::new("gzip").arg("-9").arg("-k").arg("-f").arg(&path).output());
    if !out.status.success() {
        return Err(err!("gzip failed: {}", String::from_utf8_lossy(&out.stderr);
            IO, Data));
    }
    let encoded = res!(fs::read(&gz_path));
    assert_eq!(res!(encoding::gunzip(&encoded)), plain,
        "a member written by gzip(1) did not read back as what went into it");
    Ok(())
}

/// A whole response, encoded as the server would encode it, put to `gunzip`.
///
/// This is the case that matters: the framing fields must describe the encoded
/// body, and the encoded body must be a stream a client can actually read.
#[test]
fn an_encoded_response_carries_a_stream_gunzip_accepts() -> Outcome<()> {
    use oxedyne_fe2o3_net::http::{
        fields::{
            HeaderFields,
            HeaderFieldValue,
            HeaderName,
        },
        msg::HttpMessage,
        status::HttpStatus,
    };

    res!(require("gunzip"));
    let plain = sample();

    let mut req = HeaderFields::default();
    req.insert(
        HeaderName::AcceptEncoding,
        res!(HeaderFieldValue::new(&HeaderName::AcceptEncoding, "gzip, deflate, br")),
        None,
    );
    let coding = encoding::choose(
        &req,
        "text/html; charset=utf-8",
        plain.len(),
        encoding::MIN_BYTES_DEFAULT,
    );
    assert_eq!(coding, encoding::ContentCoding::Gzip);

    let resp = HttpMessage::new_response(HttpStatus::OK)
        .with_field(
            HeaderName::ContentType,
            res!(HeaderFieldValue::new(&HeaderName::ContentType, "text/html; charset=utf-8")))
        .with_field(
            HeaderName::ETag,
            res!(HeaderFieldValue::new(&HeaderName::ETag, "\"68a1-3b\"")))
        .with_body(plain.clone());

    let rt = res!(tokio::runtime::Runtime::new());
    let resp = res!(rt.block_on(encoding::encode(resp, coding)));

    let held = res!(resp.header.fields.get_one(&HeaderName::ContentEncoding).ok_or_else(||
        err!("The encoded response did not say so."; Missing)));
    assert_eq!(fmt!("{}", held), "gzip");
    let vary = res!(resp.header.fields.get_one(&HeaderName::Vary).ok_or_else(||
        err!("The encoded response did not vary by the coding."; Missing)));
    assert_eq!(fmt!("{}", vary).to_ascii_lowercase(), "accept-encoding");
    let etag = res!(resp.header.fields.get_one(&HeaderName::ETag).ok_or_else(||
        err!("The encoded response lost its validator."; Missing)));
    assert_eq!(fmt!("{}", etag), "\"68a1-3b-gzip\"");
    // `Content-Length` is taken from the body when the message is written, so
    // this is the number that will go on the wire.
    assert_eq!(resp.body_len(), resp.body.len());

    let dir = res!(work_dir("gzip_oracle_resp"));
    let path = dir.join("resp.gz");
    let _ = fs::remove_file(&path);
    res!(fs::write(&path, &resp.body));
    let out = res!(Command::new("gunzip").arg("-c").arg(&path).output());
    if !out.status.success() {
        return Err(err!(
            "gunzip refused the body of an encoded response: {}",
            String::from_utf8_lossy(&out.stderr);
            Invalid, Data));
    }
    assert_eq!(out.stdout, plain);
    Ok(())
}
