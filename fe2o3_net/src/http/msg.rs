use crate::{
    conc::AsyncReadIterator, 
    constant,
    http::{
        fields::{
            ConnectionType,
            Cookie,
            HeaderFields,
            HeaderFieldValue,
            HeaderFieldCategory,
            HeaderName,
        },
        header::{
            HttpHeader,
            HttpHeadline,
            HttpMethod,
            HttpVersion,
        },
        status::HttpStatus,
    },
    media::{
        ContentTypeValue,
        MEDIA_PLAIN_TEXT,
    },
};

use oxedyne_fe2o3_core::prelude::*;

use std::{
    str::FromStr,
    future::Future,
    pin::Pin,
    time::Duration,
};

use tokio::{
    io::{
        AsyncRead,
        AsyncReadExt,
        AsyncWriteExt,
    },
};

/// Caller-supplied bounds applied to an HTTP read. Each field is
/// optional so a caller can tune only the bounds it cares about,
/// and the absence of a `ReadLimits` argument (`None`) restores the
/// unbounded pre-feature behaviour used by tests and outbound HTTP
/// clients that trust their own peers.
///
/// Applied at the server accept path to harden the connection
/// against three common cheap-resource-exhaustion attacks:
///
/// - Oversized headers (memory blow through a small number of
///   connections) are bounded by `max_header_bytes`.
/// - Oversized bodies (same, per request) are bounded by
///   `max_body_bytes` which is checked against the `Content-Length`
///   header up front and enforced during the body read.
/// - Slowloris-style trickle attacks (pin a connection for hours
///   by sending one byte at a time) are bounded by
///   `header_read_timeout` which wraps every header-phase stream
///   read in a `tokio::time::timeout`.
#[derive(Clone, Debug, Default)]
pub struct ReadLimits {
    /// Maximum total bytes accepted for the request / response header
    /// block before the reader aborts with `TooBig`. The count covers
    /// every byte read up to the terminating `CRLF CRLF`.
    pub max_header_bytes:       Option<usize>,
    /// Maximum bytes permitted in the message body. Checked against
    /// `Content-Length` first (rejecting oversize requests before a
    /// single byte is read) and enforced as an upper bound during the
    /// body read loop.
    pub max_body_bytes:         Option<usize>,
    /// Upper bound on the wall-clock duration between entering the
    /// header read loop and the `CRLF CRLF` terminator arriving. A
    /// slow client exceeding this limit has its connection dropped
    /// with a `TimedOut` error.
    pub header_read_timeout:    Option<Duration>,
}

impl ReadLimits {
    /// Shortcut for constructing a fully permissive limits struct
    /// with explicit `None` for every field. Mostly a readability
    /// alias for `ReadLimits::default()`.
    pub fn unbounded() -> Self {
        Self::default()
    }
}

#[derive(Debug, Default)]
pub struct HttpMessage {
    pub header:     HttpHeader,
    pub body:       Vec<u8>,
}

impl HttpMessage {

    pub fn new_response(status: HttpStatus) -> Self {
        Self {
            header: HttpHeader {
                version:    HttpVersion::Http1_1,
                headline:   HttpHeadline::Response { status },
                fields:     HeaderFields::default(),
            },
            body: Vec::new(),
        }
    }

    pub fn respond_with_text<S: AsRef<str>>(status: HttpStatus, txt: S) -> Self {
        let mut fields = HeaderFields::default();
        fields.insert(
            HeaderName::ContentType,
            HeaderFieldValue::ContentType(MEDIA_PLAIN_TEXT),
            Some(HeaderFieldCategory::Entity as u16),
        );
        Self {
            header: HttpHeader {
                version:    HttpVersion::Http1_1,
                headline:   HttpHeadline::Response { status },
                fields,
            },
            body: txt.as_ref().as_bytes().to_vec(),
        }
    }

    pub fn ok_respond_with_text<S: AsRef<str>>(txt: S) -> Self {
        Self::respond_with_text(HttpStatus::OK, txt)
    }

    /// Set the message status if the message is a response, otherwise do nothing.
    pub fn with_status(mut self, status: HttpStatus) -> Self {
        match self.header.headline {
            HttpHeadline::Response { .. } => 
                self.header.headline = HttpHeadline::Response { status },
            _ => ()
        }
        self
    }

    pub fn with_field(
        mut self,
        nam: HeaderName,
        val: HeaderFieldValue,
    )
        -> Self
    {
        self.header.fields.insert(nam, val, None);
        self
    }

    pub fn with_field_with_order(
        mut self,
        nam: HeaderName,
        val: HeaderFieldValue,
        ord: Option<u16>,
    )
        -> Self
    {
        self.header.fields.insert(nam, val, ord);
        self
    }

    /// Set the message status, returning an error if the message is not a response.
    pub fn set_response_status(&mut self, status: HttpStatus) -> Outcome<()> {
        match &mut self.header.headline {
            HttpHeadline::Response { status: old_status } => {
                *old_status = status;
                Ok(())
            },
            _ => Err(err!("HTTP message is not a response."; Invalid, Mismatch)),
        }
    }

    pub fn set_response_code(&mut self, code: &str) -> Outcome<()> {
        match &mut self.header.headline {
            HttpHeadline::Response { status } => {
                *status = res!(HttpStatus::from_str(code));
                Ok(())
            },
            _ => Err(err!("HTTP message is not a response."; Invalid, Mismatch)),
        }
    }

    pub fn set_cookie(mut self, cookie: Cookie) -> Self {
        self.header.fields.insert(
            HeaderName::SetCookie,
            HeaderFieldValue::SetCookie(cookie),
            None,
        );
        self
    }

    pub fn insert(
        &mut self,
        nam: HeaderName,
        val: HeaderFieldValue,
        ord: Option<u16>,
    )
        -> bool
    {
        self.header.fields.insert(nam, val, ord)
    }

    pub fn log(&self, log_level: LogLevel) {
        if log_level >= LogLevel::Trace {
            //Header.
            let is_request = match &self.header.headline {
                HttpHeadline::Request { method, loc } => {
                    trace!("HTTP Request Header:");
                    trace!("  {} {} {}", method, loc, self.header.version);
                    true
                },
                HttpHeadline::Response { status } => {
                    trace!("HTTP Response Header:");
                    trace!("  {} {} {}", self.header.version, status, status.desc());
                    false
                },
            };
            for (k, header_field_values) in self.header.fields.iter() {
                for header_field_value in header_field_values {
                    let prefix = match header_field_value {
                        HeaderFieldValue::Generic(_) => "   ",
                        _ => " > ",
                    };
                    trace!("{}{}: {}", prefix, k, header_field_value);
                }
            }

            // Body.
            let body_is_text = match self.body_is_text() {
                Some(true) => true,
                Some(false) => false,
                None => {
                    trace!("HTTP message has no body.");
                    return;
                },
            };
            const LIM: usize = constant::HTTP_BODY_BYTES_MAX_VIEW;
            let (display_all_bytes, bytes_report) = if self.body.len() < LIM {
                (true, fmt!("[{} bytes, all displayed]", self.body.len()))
            } else {
                (false, fmt!("[{} bytes, only {} displayed]", self.body.len(), LIM))
            };
            match is_request {
                true => trace!("HTTP Request Body {}:", bytes_report),
                false => trace!("HTTP Response Body {}:", bytes_report),
            }
            // Text dump of body, if necessary.
            if body_is_text {
                if display_all_bytes {
                    trace!("\n{}", String::from_utf8_lossy(&self.body[..]));
                } else {
                    trace!("\n{{START}}{}{{END}}", String::from_utf8_lossy(&self.body[..LIM]));
                }
                return;
            }
            // Binary dump of body.
            let lines = if display_all_bytes {
                dump!(" {:02x}", &self.body[..], 16)
            } else {
                dump!(" {:02x}", &self.body[..LIM], 16)
            };
            for line in lines {
                trace!(" {}", line);
            }
        }
    }

    pub async fn read<
        'a,
        const HEADER_CHUNK_SIZE: usize,
        const BODY_CHUNK_SIZE: usize,
        R: AsyncRead + Unpin,
    >(
        mut stream: Pin<&mut R>,
        remnant:    &Vec<u8>,
        is_request: Option<bool>,
        limits:     Option<&ReadLimits>,
    )
        -> Outcome<(Option<Self>, Vec<u8>)>
    {
        trace!("Entered HttpMessage::read");
        let result = HttpHeader::read::<HEADER_CHUNK_SIZE, _>(
            stream.as_mut(),
            remnant,
            is_request,
            limits,
        ).await;

        match result {
            Ok(Some((header, mut remnant, content_length))) => {
                trace!("remnant size = {}, content_length = {}", remnant.len(), content_length);

                // Reject the request up front if its declared content
                // length exceeds the caller's bound. Cheaper than
                // accumulating bytes and erroring halfway through, and
                // surfaces the error before any body bytes are read.
                if let Some(lim) = limits.and_then(|l| l.max_body_bytes) {
                    if content_length > lim {
                        return Err(err!(
                            "HTTP request body of {} bytes exceeds the \
                            configured limit of {}.", content_length, lim;
                            IO, Network, Input, TooBig));
                    }
                }

                let mut msg = HttpMessage::default();
                msg.header = header;

                // A chunked body carries its own lengths on the wire, and
                // says so instead of declaring a `Content-Length`. Much of
                // the web answers this way, so a client that cannot decode
                // it reads every such page as empty.
                if is_chunked(&msg.header.fields) {
                    let (body, rest) = res!(read_chunked::<BODY_CHUNK_SIZE, _>(
                        stream.as_mut(),
                        remnant,
                        limits,
                    ).await);
                    msg.body = body;
                    return Ok((Some(msg), rest));
                }

                if content_length > 0 {
                    let mut body = Vec::with_capacity(content_length);
                    body.extend_from_slice(&remnant);
                    let mut bytes_read = body.len();

                    while bytes_read < content_length {
                        let mut chunk = [0; BODY_CHUNK_SIZE];
                        let result = stream.as_mut().read(&mut chunk).await;
                        match result {
                            Ok(0) => {
                                // A reply to `HEAD` states the length of a body
                                // it will never send, so the stream ends with
                                // nothing received -- and that is complete, not
                                // a failure, or a client could never make a HEAD
                                // request at all. A response that delivered some
                                // of a declared body and then broke off is a
                                // genuine truncation, and still an error: an
                                // empty body is the tell that distinguishes the
                                // two, since HEAD yields no bytes and a cut-off
                                // GET yields the part that arrived. A truncated
                                // *request* is a broken client, dropped as before.
                                if is_request == Some(false) && body.is_empty() {
                                    msg.body = body;
                                    return Ok((Some(msg), Vec::new()));
                                }
                                warn!("UnexpectedEof treated as connection closure.");
                                return Ok((None, body.to_vec()));
                            }
                            Ok(n) => {
                                body.extend_from_slice(&chunk[..n]);
                                bytes_read += n;
                                // Defence-in-depth: the upfront
                                // Content-Length check already
                                // rejected oversize requests, but a
                                // buggy or malicious remote could
                                // keep streaming beyond the declared
                                // length. Stop as soon as we have
                                // enough to satisfy the header.
                                if let Some(lim) =
                                    limits.and_then(|l| l.max_body_bytes)
                                {
                                    if bytes_read > lim {
                                        return Err(err!(
                                            "HTTP request body overflowed \
                                            the configured limit of {} \
                                            bytes during read.", lim;
                                            IO, Network, Input, TooBig));
                                    }
                                }
                            }
                            Err(e) => return Err(e.into()),
                        }
                    }

                    remnant = if bytes_read > content_length {
                        body[content_length..].to_vec()
                    } else {
                        Vec::new()
                    };

                    msg.body = body[..content_length].to_vec();
                    Ok((Some(msg), remnant))
                } else {
                    Ok((Some(msg), remnant))
                }
            }
            Ok(None) => Ok((None, Vec::new())),
            Err(e) => Err(e),
        }
    }

    pub async fn write_all<
        R: AsyncWriteExt + Unpin,
    >(
        mut self,
        stream:         &mut R,
    )
        -> Outcome<()>
    {
        let _ = self.insert(
            HeaderName::ContentLength,
            HeaderFieldValue::ContentLength(self.body.len()),
            Some(HeaderFieldCategory::Entity as u16),
        );
        self.log(log_get_level!());
        let result = stream.write_all(&self.header.as_vec()).await;
        res!(result);
        let result = stream.write_all(&self.body).await;
        res!(result);
        let result = stream.flush().await;
        res!(result);
        Ok(())
    }

    pub fn body_text(&mut self, txt: &str) {
        self.body = txt.as_bytes().to_vec()
    }

    pub fn with_body(mut self, byts: Vec<u8>) -> Self {
        self.body = byts;
        self
    }

    pub fn body_as_string(&self) -> std::borrow::Cow<'_, str> {
        String::from_utf8_lossy(&self.body[..])
    }

    pub fn body_is_text(&self) -> Option<bool> {
        if self.body.len() > 0 {
            if let Some(hfv) = self.header.fields.get_one(&HeaderName::ContentType) {
                if let HeaderFieldValue::ContentType(ContentTypeValue::MediaType((mt, _))) = hfv {
                    Some(mt.is_text())
                } else {
                    None
                }
            } else {
                Some(false)    
            }
        } else {
            None
        }
    }

    pub fn get_connection_close(&self) -> bool {
        if let Some(hfv) = self.header.get_a_field_value(&HeaderName::Connection) {
            if let HeaderFieldValue::Connection(Some(ct), _) = hfv {
                if let ConnectionType::Close = ct {
                    return true;
                }
            }
        }
        false
    }

    pub fn set_connection_close(&mut self, close: bool) -> bool {
        self.insert(
            HeaderName::Connection,
            HeaderFieldValue::Connection(Some(ConnectionType::new(close)), Vec::new()),
            Some(HeaderFieldCategory::General as u16),
        )
    }

    pub fn has_websocket_headers(&self) -> bool {

        let connection_upgrade = match self.header.get_the_field_value(&HeaderName::Connection) { 
            Ok(HeaderFieldValue::Connection(ct_opt, list)) => match ct_opt { 
                Some(ConnectionType::KeepAlive) | None => {
                    list.iter().any(|s| s == "upgrade")
                }
                _ => false,
            },
            _ => false,
        };

        let upgrade_websocket = match self.header.get_the_field_value(&HeaderName::Upgrade) { 
            Ok(HeaderFieldValue::Upgrade(list)) => { 
                list.iter().any(|s| s == "websocket")
            },
            _ => false,
        };

        trace!("connection_upgrade = {}", connection_upgrade);
        trace!("upgrade_websocket = {}", upgrade_websocket);

        if connection_upgrade && upgrade_websocket {
            true
        } else {
            false
        }
    }

    pub fn is_websocket_upgrade(&self) -> bool {

        let is_websocket_request = match &self.header.headline {
            HttpHeadline::Request { method, .. } => {
                match method {
                    HttpMethod::GET => {
                        true
                    }
                    _ => false,
                }
            }
            _ => false,
        };

        let key_present = match self.header.get_the_field_value(&HeaderName::SecWebSocketKey) { 
            Ok(HeaderFieldValue::SecWebSocketKey(key)) => { 
                key.len() == 24 &&
                    key.chars().all(|c|
                        c.is_ascii_alphanumeric()
                        || c == '+'
                        || c == '/'
                        || c == '='
                    )
            },
            _ => false,
        };

        trace!("is_websocket_request = {}", is_websocket_request);
        trace!("key_present = {}", key_present);

        if self.has_websocket_headers() && key_present {
            true
        } else {
            false
        }
    }

    pub fn is_websocket_handshake(&self, expected_accept_key: &str) -> bool {

        let is_websocket_response = match self.header.headline {
            HttpHeadline::Response { status } => {
                match status {
                    HttpStatus::SwitchingProtocols => {
                        true
                    }
                    _ => false,
                }
            }
            _ => false,
        };

        let key_correct = match self.header.get_the_field_value(&HeaderName::SecWebSocketAccept) { 
            Ok(HeaderFieldValue::SecWebSocketKey(key)) => { 
                key == expected_accept_key
            },
            _ => false,
        };

        trace!("is_websocket_response = {}", is_websocket_response);
        trace!("key_correct = {}", key_correct);

        if self.has_websocket_headers() && key_correct {
            true
        } else {
            false
        }
    }
}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ CHUNKED TRANSFER ENCODING                                                 │
// └───────────────────────────────────────────────────────────────────────────┘

/// Whether the message says its body arrives in chunks (RFC 9112 §7.1).
///
/// `chunked` is the last coding applied, so a value of `gzip, chunked` is
/// chunked too, and the test is for its presence in the list.
pub fn is_chunked(fields: &HeaderFields) -> bool {
    match fields.get_one(&HeaderName::TransferEncoding) {
        Some(v) => fmt!("{}", v).to_lowercase().contains("chunked"),
        None    => false,
    }
}

/// Read a chunked body to its terminating zero-length chunk, returning the
/// decoded bytes and whatever the peer sent after them.
///
/// Each chunk is a hex length, optional extensions after a `;`, a CRLF, that
/// many bytes, and another CRLF. A zero length ends the body, after which the
/// peer may send trailer fields, which are consumed and discarded.
///
/// `limits.max_body_bytes` bounds the *decoded* size: a chunked body declares
/// no total up front, so the only way to bound it is to stop reading when it
/// gets too big, which is what happens here.
async fn read_chunked<
    const BODY_CHUNK_SIZE: usize,
    R: AsyncRead + Unpin,
>(
    mut stream: Pin<&mut R>,
    remnant:    Vec<u8>,
    limits:     Option<&ReadLimits>,
)
    -> Outcome<(Vec<u8>, Vec<u8>)>
{
    let max = limits.and_then(|l| l.max_body_bytes);
    let mut raw: Vec<u8> = remnant;   // Undecoded bytes not yet consumed.
    let mut out: Vec<u8> = Vec::new(); // The decoded body.

    loop {
        // The chunk size line.
        let line = match res!(take_line::<BODY_CHUNK_SIZE, _>(stream.as_mut(), &mut raw).await) {
            Some(l) => l,
            None => return Err(err!(
                "The peer closed the connection in the middle of a chunked \
                body, before its terminating chunk.";
                IO, Network, Wire, Read, Missing)),
        };
        let size_txt = match line.split(';').next() {
            Some(s) => s.trim().to_string(),
            None    => String::new(),
        };
        let size = match usize::from_str_radix(&size_txt, 16) {
            Ok(n) => n,
            Err(e) => return Err(err!(e,
                "A chunked body declared a chunk size of {:?}, which is not a \
                hexadecimal length.", size_txt;
                IO, Network, Wire, Read, Invalid)),
        };

        // The last chunk is empty, and any trailer fields follow it up to a
        // blank line. They are read so the stream is left where the next
        // message begins, and are then discarded.
        if size == 0 {
            loop {
                match res!(take_line::<BODY_CHUNK_SIZE, _>(stream.as_mut(), &mut raw).await) {
                    Some(l) if l.is_empty() => break,
                    Some(_)                 => continue,
                    // A peer that hangs up rather than closing off its
                    // trailers has still sent us the whole body.
                    None                    => return Ok((out, Vec::new())),
                }
            }
            return Ok((out, raw));
        }

        if let Some(lim) = max {
            if out.len().saturating_add(size) > lim {
                return Err(err!(
                    "A chunked HTTP body overflowed the configured limit of {} \
                    bytes.", lim;
                    IO, Network, Input, TooBig));
            }
        }

        // The chunk data, and the CRLF that closes it. The `+ 2` is a checked
        // add so a hostile chunk-size line, `ffffffffffffffff` and the like,
        // cannot overflow `usize` into a panic or a wrapped-round short slice.
        // A caller that set `max_body_bytes` never reaches here with a size
        // that large -- the limit above trips first -- but a caller that
        // trusts its peer and set no limit must still not be crashable by one.
        let need = match size.checked_add(2) {
            Some(n) => n,
            None    => return Err(err!(
                "A chunked body declared a chunk of {} bytes, which is too \
                large to be a real length.", size;
                IO, Network, Wire, Read, Invalid)),
        };
        while raw.len() < need {
            if !res!(fill::<BODY_CHUNK_SIZE, _>(stream.as_mut(), &mut raw).await) {
                return Err(err!(
                    "The peer closed the connection {} bytes into a chunk it \
                    said was {} bytes long.", raw.len(), size;
                    IO, Network, Wire, Read, Missing));
            }
        }
        out.extend_from_slice(&raw[..size]);
        raw.drain(..need);
    }
}

/// Take the next CRLF-terminated line off the buffer, reading more from the
/// stream until there is one. `None` means the peer closed first.
async fn take_line<
    const BODY_CHUNK_SIZE: usize,
    R: AsyncRead + Unpin,
>(
    mut stream: Pin<&mut R>,
    raw:        &mut Vec<u8>,
)
    -> Outcome<Option<String>>
{
    loop {
        if let Some(i) = raw.windows(2).position(|w| w == b"\r\n") {
            let line = String::from_utf8_lossy(&raw[..i]).to_string();
            raw.drain(..i + 2);
            return Ok(Some(line));
        }
        if !res!(fill::<BODY_CHUNK_SIZE, _>(stream.as_mut(), raw).await) {
            return Ok(None);
        }
    }
}

/// Read one more chunk of bytes onto the buffer. `false` means the peer closed.
async fn fill<
    const BODY_CHUNK_SIZE: usize,
    R: AsyncRead + Unpin,
>(
    mut stream: Pin<&mut R>,
    raw:        &mut Vec<u8>,
)
    -> Outcome<bool>
{
    let mut buf = [0u8; BODY_CHUNK_SIZE];
    match stream.as_mut().read(&mut buf).await {
        Ok(0)  => Ok(false),
        Ok(n)  => {
            raw.extend_from_slice(&buf[..n]);
            Ok(true)
        }
        Err(e) => Err(err!(e,
            "Reading a chunked HTTP body."; IO, Network, Wire, Read)),
    }
}


#[cfg(test)]
mod body_tests {
    use super::*;

    /// Read one message off a buffer of wire bytes, as a client reading a
    /// response does.
    fn read_reply(wire: &str, is_request: Option<bool>) -> Outcome<Option<HttpMessage>> {
        let bytes = wire.as_bytes();
        let mut stream = std::io::Cursor::new(bytes);
        let rt = res!(tokio::runtime::Runtime::new());
        let (msg, _rest) = res!(rt.block_on(HttpMessage::read::<
            { constant::HTTP_DEFAULT_HEADER_CHUNK_SIZE },
            { constant::HTTP_DEFAULT_BODY_CHUNK_SIZE },
            _,
        >(
            Pin::new(&mut stream),
            &Vec::new(),
            is_request,
            None,
        )));
        Ok(msg)
    }

    #[test]
    fn test_a_chunked_response_body_is_decoded() -> Outcome<()> {
        // Much of the web answers this way, and a client that cannot decode
        // it reads every such page as empty.
        let wire = "HTTP/1.1 200 OK\r\n\
            Content-Type: text/html\r\n\
            Transfer-Encoding: chunked\r\n\
            \r\n\
            9\r\n<!doctype\r\n\
            6\r\n html>\r\n\
            0\r\n\r\n";
        let msg = match res!(read_reply(wire, Some(false))) {
            Some(m) => m,
            None    => return Err(err!(
                "A chunked response was read as no response at all.";
                Test, Missing)),
        };
        assert_eq!(msg.body, b"<!doctype html>");
        Ok(())
    }

    #[test]
    fn test_a_chunk_may_carry_extensions_and_be_followed_by_trailers() -> Outcome<()> {
        let wire = "HTTP/1.1 200 OK\r\n\
            Transfer-Encoding: chunked\r\n\
            \r\n\
            5;name=value\r\nhello\r\n\
            0\r\n\
            X-Checksum: 1234\r\n\
            \r\n";
        let msg = match res!(read_reply(wire, Some(false))) {
            Some(m) => m,
            None    => return Err(err!(
                "A chunked response with trailers was read as no response.";
                Test, Missing)),
        };
        assert_eq!(msg.body, b"hello");
        Ok(())
    }

    #[test]
    fn test_a_reply_to_head_states_a_length_it_never_sends() -> Outcome<()> {
        // A `HEAD` reply carries the `Content-Length` the body *would* have
        // had, and then no body. Waiting for those bytes waits forever, so a
        // client that treats the close as a failure cannot make the request
        // at all.
        let wire = "HTTP/1.1 200 OK\r\n\
            Content-Type: text/html\r\n\
            Content-Length: 5120\r\n\
            \r\n";
        let msg = match res!(read_reply(wire, Some(false))) {
            Some(m) => m,
            None    => return Err(err!(
                "A reply to HEAD was read as no reply at all.";
                Test, Missing)),
        };
        assert!(msg.body.is_empty());
        Ok(())
    }

    #[test]
    fn test_a_truncated_request_is_still_dropped() -> Outcome<()> {
        // The tolerance above is for responses only. A request whose body
        // stops short is a broken client, and is dropped as it always was.
        let wire = "POST /x HTTP/1.1\r\n\
            Host: example.test\r\n\
            Content-Length: 100\r\n\
            \r\n\
            short";
        assert!(res!(read_reply(wire, None)).is_none());
        assert!(res!(read_reply(wire, Some(true))).is_none());
        Ok(())
    }

    #[test]
    fn test_a_truncated_response_body_is_dropped_not_kept() -> Outcome<()> {
        // A response that delivered part of a declared body and then broke off
        // is a genuine truncation. The HEAD tolerance keys on an *empty* body,
        // so this partial one must not be waved through as complete.
        let wire = "HTTP/1.1 200 OK\r\n\
            Content-Type: text/html\r\n\
            Content-Length: 1000\r\n\
            \r\n\
            only these bytes arrived";
        assert!(res!(read_reply(wire, Some(false))).is_none());
        Ok(())
    }

    #[test]
    fn test_a_chunk_size_that_would_overflow_is_an_error_not_a_panic() -> Outcome<()> {
        // A hostile chunk-size line of `usize::MAX` once overflowed the `+ 2`
        // that reads past the chunk's own CRLF, panicking the reader. It is now
        // rejected as the impossible length it is.
        let wire = "HTTP/1.1 200 OK\r\n\
            Transfer-Encoding: chunked\r\n\
            \r\n\
            ffffffffffffffff\r\n";
        assert!(read_reply(wire, Some(false)).is_err());
        Ok(())
    }
}


pub struct HttpMessageReader<
    'a,
    const HEADER_CHUNK_SIZE: usize,
    const BODY_CHUNK_SIZE: usize,
    R: AsyncRead + Unpin + Send,
> {
    stream: Pin<&'a mut R>,
    buffer: Vec<u8>,
    limits: Option<ReadLimits>,
}

impl<
    'a,
    const HEADER_CHUNK_SIZE: usize,
    const BODY_CHUNK_SIZE: usize,
    R: AsyncRead + Unpin + Send,
>
    HttpMessageReader<'a, HEADER_CHUNK_SIZE, BODY_CHUNK_SIZE, R>
{
    /// Build a reader with no read limits applied. Suitable for
    /// outbound HTTP clients, tests, and anywhere the peer is trusted.
    pub fn new(
        stream: Pin<&'a mut R>,
    )
        -> Self
    {
        Self {
            stream,
            buffer: Vec::new(),
            limits: None,
        }
    }

    /// Build a reader that enforces the supplied limits on every
    /// successive read. Use from HTTPS accept paths where the peer
    /// is untrusted.
    pub fn with_limits(
        stream: Pin<&'a mut R>,
        limits: ReadLimits,
    )
        -> Self
    {
        Self {
            stream,
            buffer: Vec::new(),
            limits: Some(limits),
        }
    }
}

impl<
    'a,
    const HEADER_CHUNK_SIZE: usize,
    const BODY_CHUNK_SIZE: usize,
    R: AsyncRead + Unpin + Send,
>
    AsyncReadIterator for HttpMessageReader<'a, HEADER_CHUNK_SIZE, BODY_CHUNK_SIZE, R>
{
    type Item = Outcome<HttpMessage>;

    fn next<'b>(&'b mut self) -> Pin<Box<dyn Future<Output = Option<Self::Item>> + Send + 'b>> {
        let mut stream = self.stream.as_mut();
        let buffer = &mut self.buffer;
        let limits = self.limits.as_ref();

        Box::pin(async move {
            let result = HttpMessage::read::<HEADER_CHUNK_SIZE, BODY_CHUNK_SIZE, _>(
                stream.as_mut(),
                buffer,
                None,
                limits,
            )
            .await;

            match result {
                Ok((Some(message), remnant)) => {
                    *buffer = remnant;
                    trace!("Remnant = {} bytes", buffer.len());
                    Some(Ok(message))
                }
                Ok((None, _)) => None,
                Err(e) => Some(Err(e)),
            }
        })
    }
}
