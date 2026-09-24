use crate::{
    constant,
    http::msg::HttpMessage,
    ws::{
        core::{
            accept_key,
            assemble,
            connect_request,
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
    time::Duration,
};

use tokio::{
    io::{
        AsyncReadExt,
        AsyncWriteExt,
        Chain,
    },
    net::TcpStream,
    time::{
        timeout_at,
        Instant,
    },
};


const READ_CHUNK: usize = 4_096; // payload bytes per read call

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum State {
    Open,
    Closed, // close frames exchanged
    Broken, // the stream lost its place in a frame, or the peer vanished
}

/// A websocket client that owns its TCP connection, for a caller that wants to talk to a server
/// and has no database or handler to give [`crate::ws::core::WebSocket`].
///
/// It answers pings, reassembles fragmented messages (answering a ping that arrives between the
/// fragments), masks every frame it sends, and applies [`WebSocketLimits`] to everything it
/// reads. A timed-out [`recv`](Self::recv) loses nothing, because it only ever gives up between
/// frames.
pub struct WsClient {
    stream: Chain<Cursor<Vec<u8>>, TcpStream>,  // bytes read past the handshake, then the socket
    limits: WebSocketLimits,
    opcode: Option<u8>,                         // of the fragmented message in progress
    buffer: Vec<u8>,                            // its payload so far
    state:  State,
}

impl WsClient {

    /// Connects to `addr` (`host:port`), upgrading `path` to a websocket, and verifies the
    /// server's `Sec-WebSocket-Accept` against the random key sent. `addr` doubles as the `Host`
    /// field. A refusal is an error naming the status; a wrong accept key, one naming both keys.
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
        let (request, key) = res!(connect_request(addr, path, origin));
        let mut tcp = match TcpStream::connect(addr).await {
            Ok(tcp) => tcp,
            Err(e)  => return Err(err!(e,
                "While connecting to websocket server {}.", addr;
            IO, Network)),
        };
        // Frames are small and latency is the point.
        res!(tcp.set_nodelay(true), IO, Network);
        if let Err(e) = request.write_all(&mut tcp).await {
            return Err(err!(e,
                "While sending the websocket upgrade request for {} to {}.", path, addr;
            IO, Network, Wire, Write));
        }
        let (response, remnant) = match HttpMessage::read::<
            { constant::HTTP_DEFAULT_HEADER_CHUNK_SIZE },
            { constant::HTTP_DEFAULT_BODY_CHUNK_SIZE },
            _,
        >(Pin::new(&mut tcp), &Vec::new(), Some(false), None).await {
            Ok((Some(response), remnant)) => (response, remnant),
            Ok((None, _)) => return Err(err!(
                "The websocket server {} closed the connection before answering the upgrade \
                request for {}.", addr, path;
            IO, Network, Wire, Read)),
            Err(e) => return Err(err!(e,
                "While reading the websocket server {}'s answer to the upgrade request for {}.",
                addr, path;
            IO, Network, Wire, Read)),
        };
        if let Err(e) = response.check_websocket_handshake(&accept_key(&key)) {
            return Err(err!(e,
                "The websocket upgrade of {} on {} failed.", path, addr;
            IO, Network, Wire, Invalid, Input));
        }
        // The server may speak straight after its 101, and whatever of that the header reader
        // took is the start of the frame stream.
        Ok(Self {
            stream: Cursor::new(remnant).chain(tcp),
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
                    let _ = self.tcp().shutdown().await;
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
                let _ = self.tcp().shutdown().await;
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
        let _ = self.tcp().shutdown().await;
        if let Err(e) = sent {
            return Err(err!(e, "While sending a websocket close."; IO, Network, Wire, Write));
        }
        Ok(())
    }

    fn tcp(&mut self) -> &mut TcpStream { self.stream.get_mut().1 }

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
        let tcp = self.tcp();
        if let Err(e) = tcp.write_all(&byts).await {
            self.state = State::Broken;
            return Err(err!(e, "While writing a websocket frame."; IO, Network, Wire, Write));
        }
        if let Err(e) = tcp.flush().await {
            self.state = State::Broken;
            return Err(err!(e, "While flushing a websocket frame."; IO, Network, Wire, Write));
        }
        Ok(())
    }

    /// Waits until `deadline` for a frame's first byte, consuming nothing, so that giving up here
    /// leaves the stream exactly where it was. End of stream counts as begun, for `read_frame` to
    /// report.
    async fn frame_begun(&mut self, deadline: Instant) -> Outcome<bool> {
        let (remnant, tcp) = self.stream.get_mut();
        if (remnant.position() as usize) < remnant.get_ref().len() {
            return Ok(true);
        }
        let mut probe = [0u8; 1];
        match timeout_at(deadline, tcp.peek(&mut probe)).await {
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
                let _ = self.tcp().shutdown().await;
                Err(e)
            }
        }
    }
}
