//! Streams one HTTP/1.1 message body from a reader to a writer by its framing, for a proxy that
//! must pass on bodies of any size (model files run to gigabytes) without holding them.
//!
//! Unlike `msg::read_chunked`, which decodes a chunked body into memory and is lenient about
//! what it accepts, the relay never decodes: chunk-size lines, extensions and trailers go out
//! exactly as they came in, and every line is checked strictly first, since a proxy and the
//! server behind it that disagree on where a body ends is how requests are smuggled.

use crate::constant;

use oxedyne_fe2o3_core::prelude::*;

use std::sync::atomic::{
    AtomicU64,
    Ordering,
};

use tokio::io::{
    AsyncRead,
    AsyncReadExt,
    AsyncWrite,
    AsyncWriteExt,
};


pub const RELAY_BUF_SIZE:       usize   = 64 * 1_024;   // One read.
pub const RELAY_CHUNK_HEX_MAX:  usize   = 16;           // Digits in a chunk size, 64 bits.
pub const RELAY_TRAILER_MAX:    usize   = 64 * 1_024;   // The whole trailer section.

/// How a message body is delimited (RFC 9112 §6.3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Framing {
    Length(u64),    // `Content-Length`.
    Chunked,        // `Transfer-Encoding: chunked`.
    Close,          // A reply that runs to the end of the connection.
    None,           // No body.
}

/// Which message the header fields belong to, since a reply's framing depends on its status and
/// on the method of the request it answers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Subject<'a> {
    Request,
    Reply {
        status: u16,
        method: &'a str,    // The request's.
    },
}

/// The framing the header fields declare, refusing every ambiguous form rather than guessing:
/// `Content-Length` together with `Transfer-Encoding`, a second `Content-Length` (even one
/// agreeing with the first), a length that is not plain decimal digits, and any
/// `Transfer-Encoding` other than exactly `chunked`.
///
/// The fields are the raw lines as received, because `HeaderFields` keeps one `Content-Length`
/// and drops it in favour of a `Transfer-Encoding`, which hides precisely the conflicts this
/// must see. A reply to `HEAD`, a `1xx`, `204` or `304`, and a `2xx` to `CONNECT` have no body
/// whatever the fields say, though the fields are still checked.
pub fn framing<I, N, V>(subject: Subject<'_>, fields: I) -> Outcome<Framing>
where
    I: IntoIterator<Item = (N, V)>,
    N: AsRef<str>,
    V: AsRef<str>,
{
    let ows = |c: char| c == ' ' || c == '\t';
    let mut len: Option<u64> = None;
    let mut codings: Vec<String> = Vec::new();
    for (n, v) in fields {
        let (n, v) = (n.as_ref(), v.as_ref().trim_matches(ows));
        if n.eq_ignore_ascii_case("content-length") {
            if len.is_some() {
                return Err(err!(
                    "A message carries more than one Content-Length."; Invalid, Input));
            }
            if v.is_empty() || v.len() > 19 || !v.bytes().all(|b| b.is_ascii_digit()) {
                return Err(err!(
                    "A Content-Length of {:?} is not a decimal length.", v; Invalid, Input));
            }
            len = Some(res!(v.parse::<u64>(), Invalid, Input));
        } else if n.eq_ignore_ascii_case("transfer-encoding") {
            for c in v.split(',') {
                codings.push(c.trim_matches(ows).to_ascii_lowercase());
            }
        }
    }
    let chunked = match codings.as_slice() {
        []                      => false,
        [c] if c == "chunked"   => true,
        _                       => return Err(err!(
            "A Transfer-Encoding of {:?} is not exactly 'chunked'.", codings; Invalid, Input)),
    };
    if chunked && len.is_some() {
        return Err(err!(
            "A message carries both Content-Length and Transfer-Encoding."; Invalid, Input));
    }
    if let Subject::Reply { status, method } = subject {
        let bodiless = (100..200).contains(&status)
            || status == 204
            || status == 304
            || method.eq_ignore_ascii_case("HEAD")
            || (method.eq_ignore_ascii_case("CONNECT") && (200..300).contains(&status));
        if bodiless {
            return Ok(Framing::None);
        }
    }
    Ok(match (chunked, len, subject) {
        (true,  _,          _)                  => Framing::Chunked,
        (false, Some(n),    _)                  => Framing::Length(n),
        (false, None,       Subject::Request)   => Framing::None,
        (false, None,       Subject::Reply{..}) => Framing::Close,
    })
}

/// Copies exactly one message body from `r` to `w`, as framed, and returns the bytes read past
/// its end: the start of the next message on the connection.
///
/// A chunked body goes out verbatim, chunk-size lines with their extensions and the trailer
/// section included, each line checked first: a size of 1 to 16 hex digits, an extension only
/// after a `;`, no control characters, and CRLF line ends with no bare CR or LF. Memory stays at
/// about one read buffer whatever the body's size.
///
/// # Arguments
/// * `count` - has every byte written added to it, framing included, as it is written.
/// * `bound` - the most bytes this body may put on `w`, framing included; a `Length` over it is
///   refused before anything is written, and any other body errs when it would pass it.
/// * `rest` - bytes already read from `r` after the header, which begin the body.
pub async fn relay<R, W>(
    r:      &mut R,
    w:      &mut W,
    framing: Framing,
    count:  &AtomicU64,
    bound:  Option<u64>,
    rest:   Vec<u8>,
)
    -> Outcome<Vec<u8>>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let mut p = Pipe { r, w, buf: rest, pos: 0, count, bound, sent: 0 };
    match framing {
        Framing::None       => (),
        Framing::Length(n)  => {
            if let Some(b) = bound {
                if n > b {
                    return Err(err!(
                        "A body of {} bytes exceeds the relay bound of {}.", n, b;
                        IO, Network, Input, TooBig));
                }
            }
            res!(p.copy(n).await);
        },
        Framing::Close      => {
            loop {
                if p.avail().is_empty() && !res!(p.fill().await) {
                    break;
                }
                let n = p.avail().len();
                res!(p.send(n).await);
            }
        },
        Framing::Chunked    => res!(p.chunked().await),
    }
    if let Err(e) = p.w.flush().await {
        return Err(err!(e, "Flushing a relayed HTTP body."; IO, Network, Wire, Write));
    }
    Ok(p.leftover())
}

/// The strict reading of a chunk-size line without its CRLF (RFC 9112 §7.1): 1 to 16 hex digits,
/// then nothing, or optional blanks and a `;` opening the extensions, with no control character
/// but a tab anywhere.
pub fn chunk_size(line: &[u8]) -> Outcome<u64> {
    let hex = line.iter().position(|b| !b.is_ascii_hexdigit()).unwrap_or(line.len());
    if hex == 0 || hex > RELAY_CHUNK_HEX_MAX {
        return Err(err!(
            "A chunk-size line {:?} does not begin with 1 to {} hex digits.",
            String::from_utf8_lossy(line), RELAY_CHUNK_HEX_MAX;
            IO, Network, Wire, Invalid));
    }
    let ext = &line[hex..];
    if !ext.is_empty() {
        let semi = ext.iter().position(|&b| b != b' ' && b != b'\t');
        let opens = match semi {
            Some(i) => ext[i] == b';',
            None    => false,
        };
        if !opens || ext.iter().any(|&b| is_ctl(b)) {
            return Err(err!(
                "A chunk-size line {:?} has something other than an extension after its size.",
                String::from_utf8_lossy(line);
                IO, Network, Wire, Invalid));
        }
    }
    let digits = String::from_utf8_lossy(&line[..hex]);
    Ok(res!(u64::from_str_radix(&digits, 16), IO, Network, Wire, Invalid))
}

fn is_ctl(b: u8) -> bool {
    (b < 0x20 && b != b'\t') || b == 0x7f
}

/// RFC 9110 §5.6.2 `tchar`, a field name's character.
fn is_tchar(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b)
}

struct Pipe<'a, R, W> {
    r:      &'a mut R,
    w:      &'a mut W,
    buf:    Vec<u8>,        // Read and not yet written, from `pos`.
    pos:    usize,
    count:  &'a AtomicU64,
    bound:  Option<u64>,
    sent:   u64,
}

impl<'a, R, W> Pipe<'a, R, W>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    fn avail(&self) -> &[u8] {
        &self.buf[self.pos..]
    }

    fn leftover(mut self) -> Vec<u8> {
        self.buf.drain(..self.pos);
        self.buf
    }

    /// Reads once more onto the buffer, after dropping what has been written. `false` means the
    /// peer closed.
    async fn fill(&mut self) -> Outcome<bool> {
        if self.pos > 0 {
            self.buf.drain(..self.pos);
            self.pos = 0;
        }
        let len = self.buf.len();
        self.buf.resize(len + RELAY_BUF_SIZE, 0);
        match self.r.read(&mut self.buf[len..]).await {
            Ok(n) => {
                self.buf.truncate(len + n);
                Ok(n > 0)
            },
            Err(e) => {
                self.buf.truncate(len);
                Err(err!(e, "Reading an HTTP body to relay."; IO, Network, Wire, Read))
            },
        }
    }

    /// Writes the next `n` buffered bytes.
    async fn send(&mut self, n: usize) -> Outcome<()> {
        let n64 = n as u64;
        if let Some(b) = self.bound {
            if self.sent.saturating_add(n64) > b {
                return Err(err!(
                    "A relayed body passed the bound of {} bytes.", b;
                    IO, Network, Input, TooBig));
            }
        }
        if let Err(e) = self.w.write_all(&self.buf[self.pos..self.pos + n]).await {
            return Err(err!(e, "Writing a relayed HTTP body."; IO, Network, Wire, Write));
        }
        self.pos += n;
        self.sent += n64;
        self.count.fetch_add(n64, Ordering::Relaxed);
        Ok(())
    }

    /// Relays exactly `n` bytes.
    async fn copy(&mut self, mut n: u64) -> Outcome<()> {
        while n > 0 {
            if self.avail().is_empty() && !res!(self.fill().await) {
                return Err(err!(
                    "The peer closed with {} bytes of the body still to come.", n;
                    IO, Network, Wire, Read, Missing));
            }
            let k = std::cmp::min(self.avail().len() as u64, n) as usize;
            res!(self.send(k).await);
            n -= k as u64;
        }
        Ok(())
    }

    /// The length, CRLF included, of the next line in the buffer, reading until it is whole. A
    /// bare LF or CR, or a line longer than `max`, is an error.
    async fn line(&mut self, max: usize, what: &str) -> Outcome<usize> {
        let mut scanned = 0;
        loop {
            let a = self.avail();
            if let Some(i) = a[scanned..].iter().position(|&b| b == b'\n') {
                let i = scanned + i;
                if i == 0 || a[i - 1] != b'\r' || a[..i - 1].contains(&b'\r') {
                    return Err(err!(
                        "A {} line in a chunked body has a bare CR or LF.", what;
                        IO, Network, Wire, Invalid));
                }
                return Ok(i + 1);
            }
            if a.len() > max {
                return Err(err!(
                    "A {} line in a chunked body ran past {} bytes with no CRLF.", what, max;
                    IO, Network, Input, TooBig));
            }
            scanned = a.len();
            if !res!(self.fill().await) {
                return Err(err!(
                    "The peer closed in the middle of a {} line of a chunked body.", what;
                    IO, Network, Wire, Read, Missing));
            }
        }
    }

    async fn chunked(&mut self) -> Outcome<()> {
        loop {
            let n = res!(self.line(constant::HTTP_CHUNK_LINE_MAX, "chunk-size").await);
            let size = res!(chunk_size(&self.avail()[..n - 2]));
            res!(self.send(n).await);
            if size == 0 {
                break;
            }
            res!(self.copy(size).await);
            while self.avail().len() < 2 {
                if !res!(self.fill().await) {
                    return Err(err!(
                        "The peer closed before the CRLF that ends a chunk.";
                        IO, Network, Wire, Read, Missing));
                }
            }
            if &self.avail()[..2] != b"\r\n" {
                return Err(err!(
                    "A chunk of {} bytes is not followed by CRLF.", size;
                    IO, Network, Wire, Invalid));
            }
            res!(self.send(2).await);
        }
        // The trailer section, to its empty line.
        let mut total = 0;
        loop {
            let n = res!(self.line(constant::HTTP_CHUNK_LINE_MAX, "trailer").await);
            total += n;
            if total > RELAY_TRAILER_MAX {
                return Err(err!(
                    "A chunked body's trailers ran past {} bytes.", RELAY_TRAILER_MAX;
                    IO, Network, Input, TooBig));
            }
            if n > 2 {
                let field = &self.avail()[..n - 2];
                let name = field.iter().position(|&b| b == b':').unwrap_or(0);
                if name == 0
                    || !field[..name].iter().all(|&b| is_tchar(b))
                    || field.iter().any(|&b| is_ctl(b))
                {
                    return Err(err!(
                        "A trailer line {:?} is not a header field.",
                        String::from_utf8_lossy(field);
                        IO, Network, Wire, Invalid));
                }
            }
            res!(self.send(n).await);
            if n == 2 {
                return Ok(());
            }
        }
    }
}
