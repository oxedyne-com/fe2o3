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

                if content_length > 0 {
                    let mut body = Vec::with_capacity(content_length);
                    body.extend_from_slice(&remnant);
                    let mut bytes_read = body.len();

                    while bytes_read < content_length {
                        let mut chunk = [0; BODY_CHUNK_SIZE];
                        let result = stream.as_mut().read(&mut chunk).await;
                        match result {
                            Ok(0) => {
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
