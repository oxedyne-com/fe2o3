#![cfg(feature = "async")]
//! `http::relay`: framing decided strictly from the raw header lines, and one body streamed
//! byte for byte, leaving the next message's bytes to the caller, in bounded memory.

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_net::http::relay::{
    framing,
    relay,
    Framing,
    Subject,
    RELAY_BUF_SIZE,
};

use std::{
    io::Cursor,
    pin::Pin,
    sync::atomic::{
        AtomicU64,
        Ordering,
    },
    task::{
        Context,
        Poll,
    },
};

use tokio::io::{
    AsyncRead,
    AsyncReadExt,
    AsyncWrite,
    ReadBuf,
};


/// Held shared by every test that allocates, and alone by the memory test, so that no other
/// test's buffers show in its resident set.
static SERIAL: std::sync::RwLock<()> = std::sync::RwLock::new(());

const REPLY: Subject<'static> = Subject::Reply { status: 200, method: "GET" };

/// Relays `wire` (after `rest`) to a vector, returning what was written and what is left over.
async fn run(
    wire:   &[u8],
    rest:   &[u8],
    f:      Framing,
    bound:  Option<u64>,
    trickle: bool,
)
    -> Outcome<(Vec<u8>, Vec<u8>, u64)>
{
    let _shared = match SERIAL.read() {
        Ok(g)  => g,
        Err(e) => e.into_inner(),
    };
    run_unlocked(wire, rest, f, bound, trickle).await
}

async fn run_unlocked(
    wire:   &[u8],
    rest:   &[u8],
    f:      Framing,
    bound:  Option<u64>,
    trickle: bool,
)
    -> Outcome<(Vec<u8>, Vec<u8>, u64)>
{
    let count = AtomicU64::new(0);
    let mut out: Vec<u8> = Vec::new();
    let left = if trickle {
        let mut r = Trickle(Cursor::new(wire.to_vec()));
        res!(relay(&mut r, &mut out, f, &count, bound, rest.to_vec()).await)
    } else {
        let mut r = Cursor::new(wire.to_vec());
        res!(relay(&mut r, &mut out, f, &count, bound, rest.to_vec()).await)
    };
    Ok((out, left, count.load(Ordering::Relaxed)))
}

/// A reader that hands over one byte per read, so every line straddles reads.
struct Trickle(Cursor<Vec<u8>>);

impl AsyncRead for Trickle {
    fn poll_read(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>)
        -> Poll<std::io::Result<()>>
    {
        let mut one = [0u8; 1];
        let mut b = ReadBuf::new(&mut one);
        match Pin::new(&mut self.0).poll_read(cx, &mut b) {
            Poll::Ready(Ok(())) => {
                buf.put_slice(b.filled());
                Poll::Ready(Ok(()))
            },
            other => other,
        }
    }
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ FRAMING                                                                   │
// └───────────────────────────────────────────────────────────────────────────┘

#[test]
fn test_framing_is_read_from_the_raw_lines() -> Outcome<()> {
    let none: [(&str, &str); 0] = [];
    assert_eq!(res!(framing(REPLY, [("Content-Length", " 42 ")])), Framing::Length(42));
    assert_eq!(res!(framing(REPLY, [("content-length", "0")])), Framing::Length(0));
    assert_eq!(res!(framing(REPLY, [("Transfer-Encoding", "Chunked")])), Framing::Chunked);
    assert_eq!(res!(framing(Subject::Request, [("transfer-encoding", "chunked")])),
        Framing::Chunked);
    assert_eq!(res!(framing(REPLY, none)), Framing::Close);
    assert_eq!(res!(framing(Subject::Request, none)), Framing::None);
    let head = Subject::Reply { status: 200, method: "HEAD" };
    assert_eq!(res!(framing(head, [("Content-Length", "42")])), Framing::None);
    for status in [101, 204, 304] {
        let s = Subject::Reply { status, method: "GET" };
        assert_eq!(res!(framing(s, [("Content-Length", "42")])), Framing::None);
    }
    let connect = Subject::Reply { status: 200, method: "CONNECT" };
    assert_eq!(res!(framing(connect, none)), Framing::None);
    Ok(())
}

#[test]
fn test_framing_refuses_every_ambiguous_form() {
    let bad: Vec<Vec<(&str, &str)>> = vec![
        vec![("Content-Length", "5"), ("Transfer-Encoding", "chunked")],
        vec![("Transfer-Encoding", "chunked"), ("Content-Length", "5")],
        vec![("Content-Length", "5"), ("Content-Length", "5")],
        vec![("Content-Length", "5, 5")],
        vec![("Content-Length", "+5")],
        vec![("Content-Length", "-1")],
        vec![("Content-Length", "0x10")],
        vec![("Content-Length", "")],
        vec![("Content-Length", "99999999999999999999")],
        vec![("Transfer-Encoding", "gzip, chunked")],
        vec![("Transfer-Encoding", "chunked, chunked")],
        vec![("Transfer-Encoding", "chunked"), ("Transfer-Encoding", "chunked")],
        vec![("Transfer-Encoding", "identity")],
        vec![("Transfer-Encoding", "")],
    ];
    for f in bad {
        assert!(framing(REPLY, f.clone()).is_err(), "framing accepted {:?}", f);
        assert!(framing(Subject::Request, f.clone()).is_err(), "framing accepted {:?}", f);
        // Still checked where the reply has no body.
        let head = Subject::Reply { status: 204, method: "GET" };
        assert!(framing(head, f.clone()).is_err(), "framing accepted {:?} on a 204", f);
    }
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ RELAY                                                                     │
// └───────────────────────────────────────────────────────────────────────────┘

const CHUNKED: &[u8] = b"4;name=val;q=\"x y\"\r\nWiki\r\n5\r\npedia\r\n\
    A ; ext\r\n in chunks\r\n0;last\r\nX-Trailer: yes\r\nOther:\t2\r\n\r\n";

#[tokio::test]
async fn test_a_chunked_body_is_copied_byte_for_byte() -> Outcome<()> {
    let next = b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n";
    let mut wire = CHUNKED.to_vec();
    wire.extend_from_slice(next);
    for trickle in [false, true] {
        for split in [0, 3, 20, CHUNKED.len()] {
            let (out, left, n) = res!(run(&wire[split..], &wire[..split], Framing::Chunked,
                None, trickle).await);
            assert_eq!(out, CHUNKED, "trickle {} split {}", trickle, split);
            assert_eq!(n, CHUNKED.len() as u64);
            if !trickle && split < CHUNKED.len() {
                assert_eq!(left, next, "trickle {} split {}", trickle, split);
            } else {
                // Nothing past the final CRLF has been read: the reader gives one byte at a
                // time, or the whole body was already in `rest`.
                assert!(left.is_empty(), "trickle {} split {}", trickle, split);
            }
        }
    }
    Ok(())
}

#[tokio::test]
async fn test_a_length_body_leaves_the_next_response_as_rest() -> Outcome<()> {
    let body = b"0123456789";
    let next = b"HTTP/1.1 204 No Content\r\n\r\n";
    let mut wire = body[3..].to_vec();
    wire.extend_from_slice(next);
    let (out, left, n) = res!(run(&wire, &body[..3], Framing::Length(10), None, false).await);
    assert_eq!(out, body);
    assert_eq!(left, next);
    assert_eq!(n, 10);
    // All of the body, and the next message, already in `rest`.
    let mut rest = body.to_vec();
    rest.extend_from_slice(next);
    let (out, left, _) = res!(run(b"", &rest, Framing::Length(10), None, false).await);
    assert_eq!(out, body);
    assert_eq!(left, next);
    // No body leaves `rest` untouched.
    let (out, left, n) = res!(run(b"more", next, Framing::None, None, false).await);
    assert!(out.is_empty());
    assert_eq!(left, next);
    assert_eq!(n, 0);
    // A body cut short is an error.
    assert!(run(b"0123", b"", Framing::Length(10), None, false).await.is_err());
    Ok(())
}

#[tokio::test]
async fn test_a_close_body_runs_to_the_end_of_the_connection() -> Outcome<()> {
    let _shared = match SERIAL.read() {
        Ok(g)  => g,
        Err(e) => e.into_inner(),
    };
    let body: Vec<u8> = (0..200_000u32).map(|i| (i % 251) as u8).collect();
    let (out, left, n) = res!(run_unlocked(&body[5..], &body[..5], Framing::Close, None, false).await);
    assert_eq!(out, body);
    assert!(left.is_empty());
    assert_eq!(n, body.len() as u64);
    Ok(())
}

#[tokio::test]
async fn test_a_bad_chunk_line_is_an_error() {
    let bad: [&[u8]; 16] = [
        b"g\r\nx\r\n0\r\n\r\n",                     // Not hex.
        b"\r\n",                                    // No size.
        b" 4\r\nWiki\r\n0\r\n\r\n",                 // Leading blank.
        b"4 \r\nWiki\r\n0\r\n\r\n",                 // Trailing blank and no extension.
        b"4x\r\nWiki\r\n0\r\n\r\n",                 // Junk after the size.
        b"00000000000000004\r\nWiki\r\n0\r\n\r\n",  // 17 digits.
        b"-4\r\nWiki\r\n0\r\n\r\n",
        b"4\nWiki\r\n0\r\n\r\n",                    // Bare LF.
        b"4\r;x\r\nWiki\r\n0\r\n\r\n",              // Bare CR.
        b"4;a\x01b\r\nWiki\r\n0\r\n\r\n",           // Control in an extension.
        b"4\r\nWikiXY0\r\n\r\n",                    // No CRLF after the data.
        b"4\r\nWiki\r\n0\r\nnot a field\r\n\r\n",   // A trailer that is not a field.
        b"4\r\nWiki\r\n0\r\n: v\r\n\r\n",           // A trailer without a name.
        b"4\r\nWiki\r\n0\r\nX\x00: v\r\n\r\n",      // A trailer name with a control.
        b"4\r\nWi",                                 // Cut short in the data.
        b"4\r\nWiki\r\n0\r\nX: y\r\n",              // Cut short in the trailers.
    ];
    for wire in bad {
        assert!(run(wire, b"", Framing::Chunked, None, false).await.is_err(),
            "the relay accepted {:?}", String::from_utf8_lossy(wire));
    }
    // A chunk-size line without end, kept to the line limit.
    let long = vec![b'0'; 10_000];
    assert!(run(&long, b"", Framing::Chunked, None, false).await.is_err());
    // Sixteen digits are allowed.
    let ok = b"0000000000000004\r\nWiki\r\n0\r\n\r\n";
    assert!(run(ok, b"", Framing::Chunked, None, false).await.is_ok());
}

#[tokio::test]
async fn test_a_body_past_the_bound_is_an_error() -> Outcome<()> {
    assert!(run(b"0123456789", b"", Framing::Length(10), Some(9), false).await.is_err());
    res!(run(b"0123456789", b"", Framing::Length(10), Some(10), false).await);
    let n = CHUNKED.len() as u64;
    assert!(run(CHUNKED, b"", Framing::Chunked, Some(n - 1), false).await.is_err());
    res!(run(CHUNKED, b"", Framing::Chunked, Some(n), false).await);
    assert!(run(&[0u8; 100], b"", Framing::Close, Some(99), false).await.is_err());
    Ok(())
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ BOUNDED MEMORY                                                            │
// └───────────────────────────────────────────────────────────────────────────┘

/// The resident set in bytes, from `/proc/self/status`.
fn rss() -> u64 {
    let s = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    s.lines()
        .find(|l| l.starts_with("VmRSS:"))
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|k| k.parse::<u64>().ok())
        .map(|k| k * 1_024)
        .unwrap_or(0)
}

/// A writer that keeps nothing, and samples the resident set every 8 MB.
struct Gauge {
    n:      u64,
    next:   u64,
    peak:   u64,
}

impl AsyncWrite for Gauge {
    fn poll_write(mut self: Pin<&mut Self>, _: &mut Context<'_>, buf: &[u8])
        -> Poll<std::io::Result<usize>>
    {
        self.n += buf.len() as u64;
        if self.n >= self.next {
            self.next += 8 << 20;
            let r = rss();
            if r > self.peak {
                self.peak = r;
            }
        }
        Poll::Ready(Ok(buf.len()))
    }
    fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Poll::Ready(Ok(()))
    }
    fn poll_shutdown(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}

#[tokio::test]
async fn test_a_256_mb_body_streams_in_bounded_memory() -> Outcome<()> {
    const SIZE: u64 = 256 << 20;
    let _alone = match SERIAL.write() {
        Ok(g)  => g,
        Err(e) => e.into_inner(),
    };
    let mut r = tokio::io::repeat(7).take(SIZE).chain(Cursor::new(b"NEXT".to_vec()));
    let start = rss();
    let mut g = Gauge { n: 0, next: 0, peak: start };
    let count = AtomicU64::new(0);
    let left = res!(relay(&mut r, &mut g, Framing::Length(SIZE), &count, None, Vec::new()).await);
    assert_eq!(g.n, SIZE);
    assert_eq!(count.load(Ordering::Relaxed), SIZE);
    assert!(left.is_empty() || left == b"NEXT");
    let growth = g.peak.saturating_sub(start);
    eprintln!("256 MB relayed: peak RSS growth {} KiB (buffer {} KiB)", growth / 1_024,
        RELAY_BUF_SIZE / 1_024);
    assert!(growth < (1 << 20) + RELAY_BUF_SIZE as u64,
        "peak RSS grew by {} bytes relaying 256 MB", growth);
    Ok(())
}
