use crate::{
    constant,
    http::{
        loc::Url,
        msg::HttpMessage,
    },
    tls::{
        self,
        ClientStream,
    },
    ws::{
        core::{
            accept_key,
            assemble,
            connect_request_with,
            encode_message,
            read_frame,
            Assembled,
            WebSocketLimits,
            WebSocketMessage,
        },
        status::WebSocketStatusCode,
    },
};

use oxedyne_fe2o3_core::prelude::*;

use std::{
    io::Cursor,
    pin::Pin,
    sync::Arc,
    time::Duration,
};

use tokio::{
    io::{
        AsyncBufReadExt,
        AsyncReadExt,
        AsyncWriteExt,
        BufReader,
        Chain,
    },
    net::TcpStream,
    time::{
        timeout_at,
        Instant,
    },
};
use tokio_rustls::rustls::ClientConfig;


const READ_CHUNK: usize = 4_096; // payload bytes per read call
const REFUSAL_SHOWN: usize = 512; // bytes of a refusal's body quoted in its error

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum State {
    Open,
    Closed, // close frames exchanged
    Broken, // the stream lost its place in a frame, or the peer vanished
}

/// What [`WsClient::connect_url`] sends beyond the URL, and whom it trusts.
#[derive(Clone, Debug, Default)]
pub struct WsConnect {
    pub origin: Option<String>,             // sent as `Origin`
    pub fields: Vec<(String, String)>,      // further upgrade request fields, in order
    pub tls:    Option<Arc<ClientConfig>>,  // for `wss`; `None` takes `tls::default_client_config`
}

impl WsConnect {

    pub fn origin(mut self, origin: &str) -> Self {
        self.origin = Some(origin.to_string());
        self
    }

    /// Adds one field to the upgrade request.
    pub fn field(mut self, name: &str, value: &str) -> Self {
        self.fields.push((name.to_string(), value.to_string()));
        self
    }

    pub fn tls(mut self, cfg: Arc<ClientConfig>) -> Self {
        self.tls = Some(cfg);
        self
    }
}

/// A websocket client that owns its connection, plain or TLS, for a caller that wants to talk to
/// a server and has no database or handler to give [`crate::ws::core::WebSocket`].
///
/// It answers pings, reassembles fragmented messages (answering a ping that arrives between the
/// fragments), masks every frame it sends, and applies [`WebSocketLimits`] to everything it
/// reads. A timed-out [`recv`](Self::recv) loses nothing, because it only ever gives up between
/// frames.
pub struct WsClient {
    stream: Chain<Cursor<Vec<u8>>, BufReader<ClientStream>>, // bytes read past the handshake, then the socket
    limits: WebSocketLimits,
    opcode: Option<u8>,                                         // of the fragmented message in progress
    buffer: Vec<u8>,                                            // its payload so far
    state:  State,
}

impl WsClient {

    /// Connects to `addr` (`host:port`) in plain TCP, upgrading `path` to a websocket, and
    /// verifies the server's `Sec-WebSocket-Accept` against the random key sent. `addr` doubles
    /// as the `Host` field. A refusal is an error naming the status; a wrong accept key, one
    /// naming both keys.
    ///
    /// Nothing bounds the handshake's duration, and dropping the future abandons it cleanly, so a
    /// caller wanting a bound wraps this in `tokio::time::timeout`.
    pub async fn connect(
        addr:   &str,
        path:   &str,
        origin: Option<&str>,
    )
        -> Outcome<Self>
    {
        let tcp = res!(Self::dial(addr, addr).await);
        Self::handshake(ClientStream::Plain(tcp), addr, path, origin, &[], addr).await
    }

    /// Connects to `url`, whose scheme chooses the transport: `ws` or `http` plain, `wss` or
    /// `https` over TLS (as a browser's `WebSocket` takes all four), with the server's
    /// certificate checked for the URL's host against `opts.tls`. The path and any query string
    /// go on the request line exactly as written, and `opts` adds the origin and further fields.
    /// A refusal is an error naming the status and quoting the start of the server's reason.
    ///
    /// As [`connect`](Self::connect), nothing bounds the whole of it but the caller's timeout.
    pub async fn connect_url(url: &str, opts: &WsConnect) -> Outcome<Self> {
        let (url, tls) = res!(Self::parse_url(url));
        // RFC 9110 §7.2: the port only where it is not the scheme's, an IPv6 literal bracketed.
        let name = if url.host.contains(':') { fmt!("[{}]", url.host) } else { url.host.clone() };
        let host = if url.port == url.scheme.default_port() {
            name.clone()
        } else {
            fmt!("{}:{}", name, url.port)
        };
        let dial = fmt!("{}:{}", name, url.port);
        let tcp = res!(Self::dial(&dial, &host).await);
        let stream = if tls {
            let cfg = match &opts.tls {
                Some(cfg)   => cfg.clone(),
                None        => Arc::new(res!(tls::default_client_config())),
            };
            res!(tls::handshake(tcp, &url.host, cfg).await)
        } else {
            ClientStream::Plain(tcp)
        };
        Self::handshake(
            stream,
            &host,
            &url.target,
            opts.origin.as_deref(),
            &opts.fields,
            &host,
        ).await
    }

    /// The URL read as the HTTP one it stands for (RFC 6455 §3), and whether it wants TLS.
    fn parse_url(raw: &str) -> Outcome<(Url, bool)> {
        let raw = raw.trim();
        let (scheme, rest) = match raw.find("://") {
            Some(i) => (raw[..i].to_lowercase(), &raw[i..]),
            None    => return Err(err!(
                "The websocket URL {:?} names no scheme; ws://, wss://, http:// or https:// is \
                required.", raw;
            Invalid, Input, Missing)),
        };
        let http = match scheme.as_str() {
            "ws"    | "http"    => "http",
            "wss"   | "https"   => "https",
            other               => return Err(err!(
                "The URL scheme {:?} is not a websocket one; only ws, wss, http and https are.",
                other;
            Invalid, Input, Unimplemented)),
        };
        let url = res!(Url::parse(&fmt!("{}{}", http, rest)));
        let tls = url.scheme.is_tls();
        Ok((url, tls))
    }

    async fn dial(addr: &str, host: &str) -> Outcome<TcpStream> {
        let tcp = match TcpStream::connect(addr).await {
            Ok(tcp) => tcp,
            Err(e)  => return Err(err!(e,
                "While connecting to websocket server {}.", host;
            IO, Network)),
        };
        // Frames are small and latency is the point.
        res!(tcp.set_nodelay(true), IO, Network);
        Ok(tcp)
    }

    /// Sends the upgrade request for `path` over `stream` and checks the answer. `label` names
    /// the server in errors.
    async fn handshake(
        mut stream: ClientStream,
        host:       &str,
        path:       &str,
        origin:     Option<&str>,
        fields:     &[(String, String)],
        label:      &str,
    )
        -> Outcome<Self>
    {
        let (request, key) = res!(connect_request_with(host, path, origin, fields));
        if let Err(e) = request.write_all(&mut stream).await {
            return Err(err!(e,
                "While sending the websocket upgrade request for {} to {}.", path, label;
            IO, Network, Wire, Write));
        }
        let (response, remnant) = match HttpMessage::read::<
            { constant::HTTP_DEFAULT_HEADER_CHUNK_SIZE },
            { constant::HTTP_DEFAULT_BODY_CHUNK_SIZE },
            _,
        >(Pin::new(&mut stream), &Vec::new(), Some(false), None).await {
            Ok((Some(response), remnant)) => (response, remnant),
            Ok((None, _)) => return Err(err!(
                "The websocket server {} closed the connection before answering the upgrade \
                request for {}.", label, path;
            IO, Network, Wire, Read)),
            Err(e) => return Err(err!(e,
                "While reading the websocket server {}'s answer to the upgrade request for {}.",
                label, path;
            IO, Network, Wire, Read)),
        };
        if let Err(e) = response.check_websocket_handshake(&accept_key(&key)) {
            // A server that refuses before its 101 usually says why in the body.
            let n = response.body.len().min(REFUSAL_SHOWN);
            let reason = String::from_utf8_lossy(&response.body[..n]);
            return Err(err!(e,
                "The websocket upgrade of {} on {} failed{}{}.", path, label,
                if reason.trim().is_empty() { "" } else { ": " }, reason.trim();
            IO, Network, Wire, Invalid, Input));
        }
        // The server may speak straight after its 101, and whatever of that the header reader
        // took is the start of the frame stream.
        Ok(Self {
            stream: Cursor::new(remnant).chain(BufReader::new(stream)),
            limits: WebSocketLimits::default(),
            opcode: None,
            buffer: Vec::new(),
            state:  State::Open,
        })
    }

    /// Replaces the bounds applied to incoming messages.
    pub fn with_limits(mut self, limits: WebSocketLimits) -> Self {
        self.limits = limits;
        self
    }

    pub fn limits(&self) -> WebSocketLimits { self.limits }

    /// Is the connection still usable for sending and receiving?
    pub fn is_open(&self) -> bool { self.state == State::Open }

    pub async fn send_text(&mut self, text: &str) -> Outcome<()> {
        self.send(&WebSocketMessage::Text(text.to_string())).await
    }

    /// Sends `message` as one masked frame.
    pub async fn send(&mut self, message: &WebSocketMessage) -> Outcome<()> {
        res!(self.check_open());
        self.write(message).await
    }

    /// Waits up to `timeout` for the next text, binary or close message. `Ok(None)` means the time
    /// ran out; a message half-assembled then is kept for the next call.
    ///
    /// Pings are answered and pongs dropped on the way. A close from the server is echoed and
    /// returned, after which the client is closed. The peer vanishing without a close, a read
    /// failing, or the time running out part way through a frame each leave the stream unusable,
    /// and are errors.
    pub async fn recv(&mut self, timeout: Duration) -> Outcome<Option<WebSocketMessage>> {
        res!(self.check_open());
        let deadline = Instant::now() + timeout;
        loop {
            if !res!(self.frame_begun(deadline).await) {
                return Ok(None);
            }
            let msg = match res!(self.next_frame(deadline).await) {
                Assembled::Pending          => continue,
                Assembled::Message(msg)     => return Ok(Some(msg)),
                Assembled::Control(msg)     => msg,
            };
            match msg {
                WebSocketMessage::Ping(data) => {
                    if let Err(e) = self.write(&WebSocketMessage::Pong(data)).await {
                        return Err(err!(e,
                            "While answering a websocket ping."; IO, Network, Wire, Write));
                    }
                }
                WebSocketMessage::Pong(_) => (),
                WebSocketMessage::Close(code, reason) => {
                    // RFC 6455 §5.5.1: answer a close with a close, echoing the status.
                    let echo = self.write(&WebSocketMessage::Close(code, None)).await;
                    self.state = State::Closed;
                    let _ = self.conn().shutdown().await;
                    if let Err(e) = echo {
                        return Err(err!(e,
                            "While echoing the server's websocket close."; IO, Network, Wire, Write));
                    }
                    return Ok(Some(WebSocketMessage::Close(code, reason)));
                }
                WebSocketMessage::Text(_) | WebSocketMessage::Binary(_) => return Err(err!(
                    "A websocket data message was classed as a control frame."; Bug)),
            }
        }
    }

    /// Sends a normal close and waits up to `timeout` for the server's, discarding anything that
    /// arrives first, then shuts the socket. Closing a client already closed does nothing.
    pub async fn close(&mut self, timeout: Duration) -> Outcome<()> {
        match self.state {
            State::Closed => return Ok(()),
            State::Broken => {
                let _ = self.conn().shutdown().await;
                return Ok(());
            }
            State::Open => (),
        }
        let sent = self.write(&WebSocketMessage::Close(
            Some(WebSocketStatusCode::NormalClosure), None)).await;
        self.state = State::Closed;
        if sent.is_ok() {
            let deadline = Instant::now() + timeout;
            // The server's close, its vanishing, or the deadline all end the wait; none of them
            // leaves anything more to do than shut the socket.
            while let Ok(true) = self.frame_begun(deadline).await {
                match self.next_frame(deadline).await {
                    Ok(Assembled::Control(WebSocketMessage::Close(..))) => break,
                    Ok(_) => (),
                    Err(_) => break,
                }
            }
        }
        // A failed read above may already have shut the socket, and a shutdown refused now leaves
        // the caller nothing to do, so only the close frame's fate is reported.
        let _ = self.conn().shutdown().await;
        if let Err(e) = sent {
            return Err(err!(e, "While sending a websocket close."; IO, Network, Wire, Write));
        }
        Ok(())
    }

    fn conn(&mut self) -> &mut BufReader<ClientStream> { self.stream.get_mut().1 }

    fn check_open(&self) -> Outcome<()> {
        match self.state {
            State::Open     => Ok(()),
            State::Closed   => Err(err!(
                "The websocket client is closed."; IO, Network, Invalid, Input)),
            State::Broken   => Err(err!(
                "The websocket client's connection failed earlier and cannot be used.";
            IO, Network, Invalid, Input)),
        }
    }

    async fn write(&mut self, message: &WebSocketMessage) -> Outcome<()> {
        // One frame per message: a client is required to mask (RFC 6455 §5.3), and it need not
        // fragment.
        let byts = res!(encode_message(message, true, usize::MAX, usize::MAX));
        let conn = self.conn();
        if let Err(e) = conn.write_all(&byts).await {
            self.state = State::Broken;
            return Err(err!(e, "While writing a websocket frame."; IO, Network, Wire, Write));
        }
        if let Err(e) = conn.flush().await {
            self.state = State::Broken;
            return Err(err!(e, "While flushing a websocket frame."; IO, Network, Wire, Write));
        }
        Ok(())
    }

    /// Waits until `deadline` for a frame's first byte, consuming nothing, so that giving up here
    /// leaves the stream exactly where it was. End of stream counts as begun, for `read_frame` to
    /// report.
    async fn frame_begun(&mut self, deadline: Instant) -> Outcome<bool> {
        let (remnant, conn) = self.stream.get_mut();
        if (remnant.position() as usize) < remnant.get_ref().len() {
            return Ok(true);
        }
        // Filling the read buffer is cancel safe, and a TLS stream has no peek.
        match timeout_at(deadline, conn.fill_buf()).await {
            Err(_)      => Ok(false),
            Ok(Ok(_))   => Ok(true),
            Ok(Err(e))  => {
                self.state = State::Broken;
                Err(err!(e, "While waiting for a websocket frame."; IO, Network, Wire, Read))
            }
        }
    }

    /// Reads the frame that has begun and folds it into the message in progress. Any failure here
    /// leaves the stream part way through a frame, so the client is marked broken.
    async fn next_frame(&mut self, deadline: Instant) -> Outcome<Assembled> {
        let limits = self.limits;
        let buffered = self.buffer.len();
        let result = match timeout_at(
            deadline,
            read_frame(&mut self.stream, READ_CHUNK, limits, buffered),
        ).await {
            Err(_) => Err(err!(
                "The deadline passed part way through a websocket frame, so the stream has lost \
                its place.";
            IO, Network, Wire, Read, Timeout)),
            Ok(Ok(None)) => Err(err!(
                "The websocket server closed the connection without a close frame.";
            IO, Network, Wire, Read)),
            Ok(Ok(Some(frame))) => assemble(&mut self.opcode, &mut self.buffer, frame),
            Ok(Err(e)) => Err(e),
        };
        match result {
            Ok(step) => Ok(step),
            Err(e) => {
                self.state = State::Broken;
                // As `WebSocket::read`: an over-limit message is the one refusal the peer is told
                // about, with status 1009 (RFC 6455 §7.4.1).
                if e.tags().contains(&ErrTag::TooBig) {
                    let close = WebSocketMessage::Close(
                        Some(WebSocketStatusCode::MessageTooBig),
                        Some(fmt!("Message too big")),
                    );
                    if let Err(e2) = self.write(&close).await {
                        error!(err!(e2,
                            "While sending a 1009 close to a server whose message was over the \
                            limit.";
                        IO, Network, Wire, Write));
                    }
                }
                let _ = self.conn().shutdown().await;
                Err(e)
            }
        }
    }
}
