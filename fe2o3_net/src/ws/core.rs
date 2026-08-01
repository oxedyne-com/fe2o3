use crate::{
    constant,
    http::{
        fields::{
            HeaderFieldValue,
            HeaderName,
        },
        header::HttpHeader,
        msg::HttpMessage,
    },
    ws::{
        handler::WebSocketHandler,
        status::WebSocketStatusCode,
    },
};

use oxedyne_fe2o3_core::{
    prelude::*,
    byte::ToBytes,
    rand::Rand,
};
use oxedyne_fe2o3_data::{
    ring::RingBuffer,
    time::Timestamp,
};
use oxedyne_fe2o3_iop_crypto::enc::Encrypter;
use oxedyne_fe2o3_iop_db::api::Database;
use oxedyne_fe2o3_iop_hash::api::Hasher;
use oxedyne_fe2o3_jdat::id::NumIdDat;
use oxedyne_fe2o3_syntax::SyntaxRef;

use std::{
    convert::TryFrom,
    marker::PhantomData,
    pin::Pin,
    sync::{
        Arc,
        RwLock,
    },
    time::Instant,
};

use base64;
use sha1::{
    Digest,
    Sha1,
};
use tokio::{
    self,
    io::{
        AsyncRead,
        AsyncWrite,
        AsyncReadExt,
        AsyncWriteExt,
    },
};


/// Bounds on what a peer may make a websocket reader allocate.
///
/// Both numbers are needed. A frame declares its own payload length, so the frame bound stops one
/// frame naming a size no machine can honour; but a message may arrive as any number of
/// continuation frames, each of them under the frame bound, so the message bound stops a peer
/// reaching the same total in instalments.
///
/// The defaults are [`constant::WEBSOCKET_MAX_FRAME_BYTES`] and
/// [`constant::WEBSOCKET_MAX_MESSAGE_BYTES`]. An application that knows its own traffic can raise
/// or lower them; there is deliberately no way to express "no bound".
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WebSocketLimits {
    /// Most a single frame may declare, in bytes.
    pub max_frame:  usize,
    /// Most an assembled message may reach, in bytes.
    pub max_msg:    usize,
}

impl Default for WebSocketLimits {
    fn default() -> Self {
        Self {
            max_frame:  constant::WEBSOCKET_MAX_FRAME_BYTES,
            max_msg:    constant::WEBSOCKET_MAX_MESSAGE_BYTES,
        }
    }
}

impl WebSocketLimits {

    /// Bounds a message at `max_msg` bytes, and each of its frames at the same number.
    ///
    /// The common case: an application knows how big its largest message is and does not care how
    /// the peer chooses to fragment it.
    pub fn new(max_msg: usize) -> Self {
        Self {
            max_frame:  max_msg,
            max_msg,
        }
    }

    /// Refuses a frame whose declared payload length is over the frame bound.
    ///
    /// The length is checked while it is still the 64-bit number that came off the wire, because
    /// narrowing it first would truncate on a 32-bit target: a declared 2^32 + 1 bytes becomes one
    /// byte, and the frame passes a bound it is vastly over. Nothing is allocated until this
    /// returns.
    pub fn check_frame(&self, declared: u64) -> Outcome<()> {
        if declared > self.max_frame as u64 {
            return Err(err!(
                "A websocket frame declares a payload of {} bytes, over the {} byte frame limit.",
                declared, self.max_frame;
            IO, Network, Invalid, Input, Wire, TooBig));
        }
        Ok(())
    }

    /// Refuses a message whose assembled payload would be over the message bound.
    ///
    /// Called with what is already buffered plus what the next frame declares, so a run of legal
    /// continuation frames is stopped at the one that would take the total past the bound, before
    /// that frame is allocated for.
    pub fn check_msg(&self, total: usize) -> Outcome<()> {
        if total > self.max_msg {
            return Err(err!(
                "A websocket message would reach {} bytes, over the {} byte message limit.",
                total, self.max_msg;
            IO, Network, Invalid, Input, Wire, TooBig));
        }
        Ok(())
    }
}

#[derive(Debug)]
pub enum WebSocketMessage {
    Text(String),
    Binary(Vec<u8>),
    Ping(Vec<u8>),
    Pong(Vec<u8>),
    Close(Option<WebSocketStatusCode>, Option<String>),
}

pub fn connect_request(
    host: &str,
)
    -> Outcome<(HttpMessage, String)>
{
    let mut key = [0u8; 16];
    Rand::fill_u8(&mut key);
    let key_str = base64::encode(&key);
    let msg = fmt!(
        "GET /ws HTTP/1.1\r\n\
        Host: {}/ws\r\n\
        Upgrade: websocket\r\n\
        Connection: Upgrade\r\n\
        Sec-WebSocket-Key: {}\r\n\
        Sec-WebSocket-Version: 13\r\n\r\n",
        host, key_str.clone(),
    );
    Ok((
        HttpMessage {
            header:    res!(HttpHeader::parse(msg, Some(true))),
            body:      Vec::new(),
            head_only: false,
            file:      None,
        },
        key_str,
    ))
}

/// Derives the `Sec-WebSocket-Accept` value from the client's `Sec-WebSocket-Key`, as RFC 6455
/// §4.2.2 step 5.4 defines it: the key and the WebSocket GUID are concatenated as US-ASCII, hashed
/// with SHA-1, and the digest is base64-encoded with the standard alphabet.
///
/// Every browser recomputes this value and refuses the handshake if it disagrees, so the algorithm
/// is fixed by the peer, not by us. It is a free function because it needs none of `WebSocket`'s
/// generic parameters, and because a value a third party verifies must be testable on its own.
pub fn accept_key(key: &str) -> String {
    let concatenated = fmt!("{}{}", key, constant::WEBSOCKET_GUID);
    let mut hasher = Sha1::new();
    hasher.update(concatenated.as_bytes());
    let hash = hasher.finalize();
    base64::encode(&hash)
}

/// Builds the `101 Switching Protocols` response that answers a client's upgrade request.
///
/// The request's `Sec-WebSocket-Key` is validated (present, valid base64, 16 bytes once decoded)
/// before the accept value is derived from it, so a malformed handshake is refused here rather
/// than half-completed.
///
/// A free function, because a server that owns its own socket -- one that reads the upgrade
/// request itself and then splits the stream into halves for two tasks -- needs the response text
/// without needing a [`WebSocket`], whose type parameters describe a database it has no use for.
pub fn accept_response(request: &HttpMessage) -> Outcome<String> {
    let key = match request.header.get_the_field_value(&HeaderName::SecWebSocketKey) {
        Ok(HeaderFieldValue::SecWebSocketKey(key)) => {
            let key_byts = match base64::decode(&key) {
                Ok(byts) => byts,
                Err(e) => return Err(err!(e,
                    "The websocket key provided is not valid base64.";
                IO, Network, Invalid, Input, String, Conversion)),
            };
            if key_byts.len() != 16 {
                return Err(err!(
                    "The websocket key is {} bytes long, expected 16.", key_byts.len();
                IO, Network, Invalid, Input, Mismatch, Size));
            }
            key
        },
        _ => return Err(err!(
            "The websocket key string is missing.";
        IO, Network, Input, Missing)),
    };
    Ok(fmt!(
        "HTTP/1.1 101 Switching Protocols\r\n\
        Upgrade: websocket\r\n\
        Connection: Upgrade\r\n\
        Sec-WebSocket-Accept: {}\r\n\r\n",
        accept_key(&key),
    ))
}

/// Encodes a message as the bytes of one or more RFC 6455 frames.
///
/// `mask` is set by a client and clear by a server: RFC 6455 §5.3 requires client-to-server frames
/// to be masked and forbids the mask on server-to-client frames. A payload longer than
/// `chunk_thresh` is fragmented into frames of at most `chunk_size` bytes, the first carrying the
/// message's opcode and the rest the continuation opcode, with `FIN` on the last.
///
/// A free function so that the write half of a split stream can be framed by a task that never
/// touches the read half.
pub fn encode_message(
    message:        &WebSocketMessage,
    mask:           bool,
    chunk_size:     usize,
    chunk_thresh:   usize,
)
    -> Outcome<Vec<u8>>
{
    if chunk_size == 0 {
        return Err(err!(
            "A websocket chunk size of zero cannot make progress.";
        Invalid, Input, Size));
    }

    // Determine the opcode based on the message type.
    let initial_opcode = match message {
        WebSocketMessage::Text(_) => 0x1,
        WebSocketMessage::Binary(_) => 0x2,
        WebSocketMessage::Ping(_) => 0x9,
        WebSocketMessage::Pong(_) => 0xA,
        WebSocketMessage::Close(_, _) => 0x8,
    };

    // Get the payload data.
    let payload = match message {
        WebSocketMessage::Text(text) => text.as_bytes().to_vec(),
        WebSocketMessage::Binary(data) => data.clone(),
        WebSocketMessage::Ping(data) => data.clone(),
        WebSocketMessage::Pong(data) => data.clone(),
        WebSocketMessage::Close(status_code, reason) => {
            let mut data = Vec::new();
            if let Some(code) = status_code {
                data.extend_from_slice(&code.to_bytes());
            }
            if let Some(reason_str) = reason {
                data.extend_from_slice(reason_str.as_bytes());
            }
            data
        }
    };
    let payload_length = payload.len();

    // Generate the masking key for client-side masking.
    let mut masking_key = [0u8; 4];
    if mask {
        Rand::fill_u8(&mut masking_key);
    }
    let mask_bit = if mask { 0x80 } else { 0x00 };

    let mut out = Vec::with_capacity(payload_length + 16);
    if payload_length > chunk_thresh {
        // Send the message in chunks.
        let mut bytes_sent = 0;
        while bytes_sent < payload_length {
            let remaining_bytes = payload_length - bytes_sent;
            let chunk = std::cmp::min(remaining_bytes, chunk_size);
            let is_final = remaining_bytes <= chunk_size;

            // Use the initial opcode for the first frame, continuation (0x0) for the others.
            let opcode = if bytes_sent == 0 { initial_opcode } else { 0x0 };

            // First byte: FIN bit and opcode.
            out.push(if is_final { 0x80 | opcode } else { opcode });

            // Second byte: mask bit and payload length.
            if chunk <= 125 {
                out.push(mask_bit | chunk as u8);
            } else if chunk <= 65535 {
                out.push(mask_bit | 126);
                out.extend_from_slice(&(chunk as u16).to_be_bytes());
            } else {
                out.push(mask_bit | 127);
                out.extend_from_slice(&(chunk as u64).to_be_bytes());
            }
            if mask {
                out.extend_from_slice(&masking_key);
            }
            for i in 0..chunk {
                let b = payload[bytes_sent + i];
                out.push(if mask { b ^ masking_key[i % 4] } else { b });
            }
            bytes_sent += chunk;
        }
    } else {
        // Send the message as a single frame.
        out.push(0x80 | initial_opcode);
        if payload_length <= 125 {
            out.push(mask_bit | payload_length as u8);
        } else if payload_length <= 65535 {
            out.push(mask_bit | 126);
            out.extend_from_slice(&(payload_length as u16).to_be_bytes());
        } else {
            out.push(mask_bit | 127);
            out.extend_from_slice(&(payload_length as u64).to_be_bytes());
        }
        if mask {
            out.extend_from_slice(&masking_key);
        }
        for (i, b) in payload.iter().enumerate() {
            out.push(if mask { b ^ masking_key[i % 4] } else { *b });
        }
    }
    Ok(out)
}

/// Reads one message -- however many frames it arrives in -- from `stream`.
///
/// `buffer` accumulates the payload across the frames of a fragmented message and is cleared
/// before the message is returned, so the same buffer can be reused for the next call. `Ok(None)`
/// means the peer closed the connection.
///
/// `limits` bounds what the peer can make this allocate. Every frame declares its own payload
/// length, and that number is checked against [`WebSocketLimits`] before a byte is reserved for it,
/// so a frame announcing more than the connection will accept costs nothing but the eight bytes of
/// header it was declared in. An error from either bound carries the `TooBig` tag, which is how
/// [`WebSocket::read`] knows to answer with a 1009 close.
///
/// A free function so that the read half of a split stream can be decoded by a task that never
/// touches the write half.
pub async fn read_message<R: AsyncRead + Unpin>(
    stream:     &mut R,
    buffer:     &mut Vec<u8>,
    chunk_size: usize,
    limits:     WebSocketLimits,
)
    -> Outcome<Option<WebSocketMessage>>
{
    if chunk_size == 0 {
        return Err(err!(
            "A websocket chunk size of zero cannot make progress.";
        Invalid, Input, Size));
    }
    let mut is_final_frame = false;
    let mut opcode = 0;

    while !is_final_frame {
        // Read the first byte of the frame header.
        let mut header_byte = [0u8; 1];
        match stream.read_exact(&mut header_byte).await {
            Ok(_n) => (),
            Err(e) if e.kind() == tokio::io::ErrorKind::UnexpectedEof => {
                return Ok(None);
            }
            Err(e) => return Err(err!(e,
                "While trying to read first byte of the frame header.";
            IO, Network, Read, Wire)),
        }

        // Extract the FIN bit and opcode from the header byte.
        is_final_frame = (header_byte[0] & 0x80) != 0;
        let frame_opcode = header_byte[0] & 0x0F;

        if opcode == 0 {
            opcode = frame_opcode;
        }

        // Read the second byte of the frame header.
        let mut length_byte = [0u8; 1];
        match stream.read_exact(&mut length_byte).await {
            Ok(_n) => (),
            Err(e) if e.kind() == tokio::io::ErrorKind::UnexpectedEof => {
                return Ok(None);
            }
            Err(e) => return Err(err!(e,
                "While trying to read second byte of the frame header.";
            IO, Network, Read, Wire)),
        }

        // Extract the payload length and mask flag from the length byte. The length stays 64 bits
        // wide until it has been checked, since narrowing it first would truncate on a 32-bit
        // target and let a huge declaration through as a small one.
        let masked = (length_byte[0] & 0x80) != 0;
        let declared: u64 = match length_byte[0] & 0x7F {
            127 => {
                // 64-bit extended payload length.
                let mut extended_length_bytes = [0u8; 8];
                match stream.read_exact(&mut extended_length_bytes).await {
                    Ok(_n) => (),
                    Err(e) if e.kind() == tokio::io::ErrorKind::UnexpectedEof => {
                        return Ok(None);
                    }
                    Err(e) => return Err(err!(e,
                        "While trying to read the 64-bit extended payload length.";
                    IO, Network, Read, Wire)),
                }
                u64::from_be_bytes(extended_length_bytes)
            }
            126 => {
                // 16-bit extended payload length.
                let mut extended_length_bytes = [0u8; 2];
                match stream.read_exact(&mut extended_length_bytes).await {
                    Ok(_n) => (),
                    Err(e) if e.kind() == tokio::io::ErrorKind::UnexpectedEof => {
                        return Ok(None);
                    }
                    Err(e) => return Err(err!(e,
                        "While trying to read the 16-bit extended payload length.";
                    IO, Network, Read, Wire)),
                }
                u16::from_be_bytes(extended_length_bytes) as u64
            }
            len => len as u64,
        };

        // A control frame carries at most 125 bytes and is never fragmented (RFC 6455 §5.5), so a
        // control opcode declaring more than that is malformed however generous the limits are.
        if (frame_opcode & 0x08) != 0 && declared > constant::WEBSOCKET_MAX_CONTROL_FRAME_BYTES {
            return Err(err!(
                "A websocket control frame with opcode {:#x} declares a payload of {} bytes; \
                RFC 6455 §5.5 allows at most {}.",
                frame_opcode, declared, constant::WEBSOCKET_MAX_CONTROL_FRAME_BYTES;
            IO, Network, Invalid, Input, Wire, TooBig));
        }

        // Bound the frame, and then the message the frame would join, before anything is allocated
        // to hold either. The peer's number is not believed until it has been agreed to.
        //
        // `TooBig` is repeated on the wrapper deliberately: `Error::tags` reads the outermost frame
        // of an error and not the chain beneath it, so a tag that is only on the inner error is a
        // tag no caller will find, and answering with a 1009 close depends on finding it.
        res!(limits.check_frame(declared),
            IO, Network, Invalid, Input, Wire, TooBig);
        let payload_length = declared as usize; // Narrowing is safe: `check_frame` bounded it.
        res!(limits.check_msg(buffer.len().saturating_add(payload_length)),
            IO, Network, Invalid, Input, Wire, TooBig);

        // Read the masking key if the frame is masked.
        let mut masking_key = [0u8; 4];
        if masked {
            match stream.read_exact(&mut masking_key).await {
                Ok(_n) => (),
                Err(e) if e.kind() == tokio::io::ErrorKind::UnexpectedEof => {
                    return Ok(None);
                }
                Err(e) => return Err(err!(e,
                    "While trying to read the frame masking key.";
                IO, Network, Read, Wire)),
            }
        }

        // Read the payload data, unmasking each chunk as it lands.
        let mut payload = vec![0u8; payload_length];
        let mut bytes_read = 0;
        while bytes_read < payload_length {
            let chunk = std::cmp::min(chunk_size, payload_length - bytes_read);
            match stream.read_exact(&mut payload[bytes_read..bytes_read + chunk]).await {
                Ok(_n) => {
                    if masked {
                        for i in 0..chunk {
                            payload[bytes_read + i] ^= masking_key[(bytes_read + i) % 4];
                        }
                    }
                    bytes_read += chunk;
                    buffer.extend_from_slice(&payload[bytes_read - chunk..bytes_read]);
                }
                Err(e) if e.kind() == tokio::io::ErrorKind::UnexpectedEof => {
                    return Ok(None);
                }
                Err(e) => return Err(err!(e,
                    "While trying to read payload chunk.";
                IO, Network, Read, Wire)),
            }
        }
    }

    // Construct the appropriate WebSocketMessage variant based on the opcode.
    let message = match opcode {
        0x0 => {
            // A continuation frame with nothing to continue: the peer either lost track of the
            // message it was sending or is probing for a panic. Neither is our business to
            // complete.
            return Err(err!(
                "The first frame of a message carries the continuation opcode, so there is \
                no message for it to continue.";
            IO, Network, Invalid, Input, Wire));
        }
        0x1 => {
            // Text frame.
            let text = res!(std::str::from_utf8(buffer)).to_string();
            WebSocketMessage::Text(text)
        }
        0x2 => {
            // Binary frame.
            WebSocketMessage::Binary(buffer.clone())
        }
        0x8 => {
            // Close frame.
            let status_code = if buffer.len() >= 2 {
                let nu16 = u16::from_be_bytes([buffer[0], buffer[1]]);
                let code = res!(WebSocketStatusCode::try_from(nu16));
                Some(code)
            } else {
                None
            };
            let reason = if buffer.len() > 2 {
                Some(res!(std::str::from_utf8(&buffer[2..])).to_string())
            } else {
                None
            };
            WebSocketMessage::Close(status_code, reason)
        }
        0x9 => {
            // Ping frame.
            WebSocketMessage::Ping(buffer.clone())
        }
        0xA => {
            // Pong frame.
            WebSocketMessage::Pong(buffer.clone())
        }
        _ => {
            // Unknown opcode.
            return Err(err!("Unknown opcode: {}", opcode; IO, Network, Invalid, Input));
        }
    };

    // Clear the buffer for the next message.
    buffer.clear();

    Ok(Some(message))
}

pub struct WebSocket<
    'a,
    const UIDL: usize,
    UID:    NumIdDat<UIDL> + 'static,
    ENC:    Encrypter,
    KH:     Hasher,
    DB:     Database<UIDL, UID, ENC, KH>,
    S:      AsyncRead + AsyncWrite + Unpin,
    WSH:    WebSocketHandler,
> {
    stream:         Pin<&'a mut S>,
    is_server:      bool,
    buffer:         Vec<u8>,
    pub latency:    RingBuffer<{ constant::WEBSOCKET_LATENCY_HISTORY_SIZE }, Option<u16>>,
    pub handler:    WSH,
    chunk_size:     usize,
    chunk_thresh:   usize,
    limits:         WebSocketLimits,
    phantom1:       PhantomData<UID>,
    phantom2:       PhantomData<ENC>,
    phantom3:       PhantomData<KH>,
    phantom4:       PhantomData<DB>,
}

impl<
    'a,
    const UIDL: usize,
    UID:    NumIdDat<UIDL> + 'static,
    ENC:    Encrypter + 'static,
    KH:     Hasher + 'static,
    DB:     Database<UIDL, UID, ENC, KH> + 'static,
    S:      AsyncRead + AsyncWrite + Unpin,
    WSH:    WebSocketHandler,
>
    WebSocket<'a, UIDL, UID, ENC, KH, DB, S, WSH>
{
    pub fn new_client(
        stream:         &'a mut S,
        handler:        WSH,
        chunk_size:     usize,
        chunk_thresh:   usize,
    )
        -> Self
    {
        Self {
            stream:         Pin::new(stream),
            is_server:      false,
            buffer:         Vec::new(),
            latency:        RingBuffer::default(),
            handler,
            chunk_size,
            chunk_thresh,
            limits:         WebSocketLimits::default(),
            phantom1:       PhantomData,
            phantom2:       PhantomData,
            phantom3:       PhantomData,
            phantom4:       PhantomData,
        }
    }

    pub fn new_server(
        stream:         &'a mut S,
        handler:        WSH,
        chunk_size:     usize,
        chunk_thresh:   usize,
    )
        -> Self
    {
        Self {
            stream:         Pin::new(stream),
            is_server:      true,
            buffer:         Vec::new(),
            latency:        RingBuffer::default(),
            handler,
            chunk_size,
            chunk_thresh,
            limits:         WebSocketLimits::default(),
            phantom1:       PhantomData,
            phantom2:       PhantomData,
            phantom3:       PhantomData,
            phantom4:       PhantomData,
        }
    }

    pub fn is_server(&self) -> bool { self.is_server }
    pub fn is_client(&self) -> bool { !self.is_server }

    /// The bounds applied to incoming messages.
    pub fn limits(&self) -> WebSocketLimits { self.limits }

    /// Replaces the bounds applied to incoming messages, for an application whose traffic differs
    /// from the defaults in [`WebSocketLimits`].
    pub fn with_limits(mut self, limits: WebSocketLimits) -> Self {
        self.limits = limits;
        self
    }

    pub async fn connect(
        &mut self,
        request:    HttpMessage,
        key:        Option<String>,
    )
        -> Outcome<()>
    {
        if self.is_client() {
            match key {
                Some(key) => {
                    self.connect_as_client(request, key).await
                }
                None => Err(err!(
                    "Expected a key string, received: {:?}", key;
                Input, Missing)),
            }
        } else {
            self.connect_as_server(request).await
        }
    }

    pub async fn connect_as_client(
        &mut self,
        request:    HttpMessage,
        key:        String,
    )
        -> Outcome<()>
    {
        let result = request.write_all(&mut self.stream).await;
        res!(result);
        let result = HttpMessage::read::<
            { constant::HTTP_DEFAULT_HEADER_CHUNK_SIZE },
            { constant::HTTP_DEFAULT_BODY_CHUNK_SIZE },
            _,
        >(Pin::new(&mut self.stream), &Vec::new(), Some(false), None).await;
        match result {
            Ok((Some(response), _)) => {

                let accept_key = accept_key(&key);

                if response.is_websocket_handshake(&accept_key) {
                    info!("Client connection successfully upgraded to a websocket.");
                } else {
                    return Err(err!(
                        "While checking server websocket upgrade response.";
                    IO, Network));
                }
            },
            Err(e) => return Err(err!(e,
                "While checking server websocket upgrade response.";
            IO, Network, Wire, Read)),
            Ok((None, _)) => return Err(err!(
                "UnexpectedEof indicates connection closure.";
            IO, Network, Wire, Read)),
        }

        Ok(())
    }

    /// The HTTP(S) server has detected a websocket upgrade request message and passes it to this
    /// method to complete the handshake.
    pub async fn connect_as_server(
        &mut self,
        request: HttpMessage,
    )
        -> Outcome<()>
    {
        let response = res!(accept_response(&request));

        match self.stream.write_all(response.as_bytes()).await {
            Ok(()) => (),
            Err(e) => return Err(err!(e,
                "Could not send websocket handshake response.";
            IO, Network, Wire, Write)),
        }

        info!("Server connection successfully upgraded to a websocket.");

        Ok(())
    }

    /// Reads one message from the stream, however many frames it arrives in. `Ok(None)` means the
    /// peer closed the connection.
    ///
    /// A message that breaches this socket's [`WebSocketLimits`] is answered with a close frame
    /// carrying status 1009, which is what RFC 6455 §7.4.1 reserves for a message too big to
    /// process, and is the only notice the peer gets that its message was refused rather than
    /// lost. The refusal leaves the stream part way through a frame whose payload was never read,
    /// so there is nothing to resynchronise to and the connection ends: the close frame is the
    /// last thing sent on it.
    pub async fn read(&mut self) -> Outcome<Option<WebSocketMessage>> {
        let chunk_size = self.chunk_size;
        let limits = self.limits;
        let mut buffer = std::mem::take(&mut self.buffer);
        let result = read_message(
            self.stream.as_mut().get_mut(),
            &mut buffer,
            chunk_size,
            limits,
        ).await;
        self.buffer = buffer;
        if let Err(e) = &result {
            if e.tags().contains(&ErrTag::TooBig) {
                // Whatever was gathered of the refused message is dropped here rather than left to
                // join the front of whatever is read next.
                self.buffer.clear();
                let close = WebSocketMessage::Close(
                    Some(WebSocketStatusCode::MessageTooBig),
                    Some(fmt!("Message too big")),
                );
                if let Err(e) = self.send(&close).await {
                    // The error being returned is the peer's, and stands whether or not it heard
                    // about it.
                    error!(err!(e,
                        "While sending a 1009 close frame to a peer whose message was over the \
                        limit.";
                    IO, Network, Wire, Write));
                }
            }
        }
        result
    }

    /// Frames `message` and writes it to the stream, masking it when this end is the client.
    pub async fn send(
        &mut self,
        message: &WebSocketMessage,
    )
        -> Outcome<()>
    {
        let byts = res!(encode_message(
            message,
            self.is_client(),
            self.chunk_size,
            self.chunk_thresh,
        ));
        let result = self.stream.write_all(&byts).await;
        res!(result);
        let result = self.stream.flush().await;
        res!(result);
        Ok(())
    }


    pub async fn close(
        &mut self,
        status_code:    Option<WebSocketStatusCode>,
        reason:         Option<String>,
    )
        -> Outcome<()>
    {
        // Construct the close frame payload
        let mut payload = Vec::new();
        if let Some(code) = status_code {
            let code_u16: u16 = code.into();
            payload.extend_from_slice(&code_u16.to_be_bytes());
        }
        if let Some(reason_str) = reason.clone() {
            payload.extend_from_slice(reason_str.as_bytes());
        }

        // Send the close frame
        let close_frame = WebSocketMessage::Close(status_code, reason.clone());
        let result = self.send(&close_frame).await;
        res!(result);

        if self.is_server() {
            // Server-side: Wait for the client to send a close frame
            let close_response;
            loop {
                let result = self.read().await;
                match result {
                    Ok(Some(message)) => match message {
                        WebSocketMessage::Close(_, _) => {
                            close_response = Some(message);
                            break;
                        }
                        _ => {
                            // Ignore any other messages until we receive a close frame
                            continue;
                        }
                    },
                    Ok(None) => {
                        info!("The client has closed the connection.");
                        return Ok(());
                    }
                    Err(e) => return Err(e.into()),
                }
            }
    
            // Verify the close response from the client
            if let Some(WebSocketMessage::Close(client_status_code, client_reason)) = close_response {
                if let Some(code) = status_code {
                    if client_status_code != Some(code) {
                        return Err(err!(
                            "Received unexpected close status code from client: {:?}", client_status_code;
                        IO, Network, Invalid, Input));
                    }
                }
                if let Some(reason_str) = reason {
                    if client_reason != Some(reason_str) {
                        return Err(err!(
                            "Received unexpected close reason from client: {:?}", client_reason;
                        IO, Network, Invalid, Input));
                    }
                }
            } else {
                return Err(err!(
                    "Expected close frame response from client, but received: {:?}", close_response;
                IO, Network, Invalid, Input));
            }
        } else {
            // Client-side: Read the close frame response from the server
            let close_response;
            loop {
                let result = self.read().await;
    
                match result {
                    Ok(Some(msg)) => match msg {
                        WebSocketMessage::Close(_, _) => {
                            close_response = Some(msg);
                            break;
                        }
                        _ => {
                            // Ignore any other messages until we receive a close frame
                            continue;
                        }
                    }
                    Ok(None) => {
                        info!("The server has closed the connection.");
                        return Ok(());
                    }
                    Err(e) => return Err(e.into()),
                }
            }
    
            // Verify the close response from the server
            if let Some(WebSocketMessage::Close(server_status_code, server_reason)) = close_response {
                if let Some(code) = status_code {
                    if server_status_code != Some(code) {
                        return Err(err!(
                            "Received unexpected close status code from server: {:?}", server_status_code;
                        IO, Network, Invalid, Input));
                    }
                }
                if let Some(reason_str) = reason {
                    if server_reason != Some(reason_str) {
                        return Err(err!(
                            "Received unexpected close reason from server: {:?}", server_reason;
                        IO, Network, Invalid, Input));
                    }
                }
            } else {
                return Err(err!(
                    "Expected close frame response from server, but received: {:?}", close_response;
                IO, Network, Invalid, Input));
            }
        }
    
        // Close the underlying TCP connection
        let result = self.stream.shutdown().await;
        res!(result);
    
        Ok(())
    }

    async fn response_handler(
        &mut self,
        result:     Outcome<Option<WebSocketMessage>>,
        err_count:  &mut usize,
        max_errors: usize,
        in_typ:     &str,
        id:         &String,
    )
        -> Outcome<()>
    {
        match result {
            Ok(response_opt) => {
                if let Some(response) = response_opt {
                    let result = self.send(&response).await;
                    if let Err(e) = result {
                        *err_count += 1;
                        if *err_count > max_errors {
                            let e = err!(e,
                                "{}: The number of websocket handler errors has exceeded the limit of {}, \
                                the connection will now be terminated.", id, max_errors;
                            IO, Network, Wire, Excessive);
                            error!(e.clone());
                            return Err(e);
                        } else {
                            error!(e, "{}: While trying to send response to an incoming {} message. This \
                                websocket handler error leaves {} more before connection termination.",
                                id, in_typ, max_errors - *err_count,
                            );
                        }
                    }
                }
            }
            Err(e) => {
                *err_count += 1;
                if *err_count > max_errors {
                    let e = err!(e,
                        "{}: The number of websocket handler errors has exceeded the limit of {}, \
                        the connection will now be terminated.", id, max_errors;
                    IO, Network, Wire, Excessive);
                    error!(e.clone());
                    return Err(e);
                } else {
                    error!(e, "{}: This websocket handler error leaves {} more before connection \
                        termination.", id, max_errors - *err_count,
                    );
                }
            }
        }
        Ok(())
    }

    pub async fn listen(
        &mut self,
        db:             Option<(Arc<RwLock<DB>>, UID)>,
        syntax:         SyntaxRef,
        ping_interval:  Option<u8>,
        max_errors:     u8,
        id:             &String,
    )
        -> Outcome<()>
    {
        let mut err_count = 0;
        let max_errors = max_errors as usize;
        let mut ping_timestamp: Option<Instant> = None;
        
        // Get dev_receiver if available for development refresh messages.
        let mut dev_receiver = res!(self.handler.dev_receiver(id));

        let mut interval = ping_interval.map(|interval| {
            let duration = tokio::time::Duration::from_secs(interval as u64);
            tokio::time::interval(duration)
        });

        // Start pinging at t = dt not t = 0.
        if let Some(interval) = &mut interval {
            interval.tick().await;
        }

        loop {
            tokio::select! {
                result = self.read() => {
                    match result {
                        Ok(Some(msg)) => {
                            match msg {
                                WebSocketMessage::Text(txt) => {
                                    let result = self.handler.handle_text(
                                        txt,
                                        db.clone(),
                                        syntax.clone(),
                                        id,
                                    );
                                    let result = self.response_handler(
                                        result,
                                        &mut err_count,
                                        max_errors,
                                        "text",
                                        id,
                                    ).await;
                                    //res!(result);
                                    if let Err(e) = result {
                                        error!(e);
                                        continue;
                                    }
                                }
                                WebSocketMessage::Binary(byts) => {
                                    let result = self.handler.handle_binary(
                                        byts,
                                        db.clone(),
                                        syntax.clone(),
                                        id,
                                    );
                                    let result = self.response_handler(
                                        result,
                                        &mut err_count,
                                        max_errors,
                                        "binary",
                                        id,
                                    ).await;
                                    res!(result);
                                }
                                WebSocketMessage::Ping(byts) => {
                                    let result = self.response_handler(
                                        Ok(Some(WebSocketMessage::Pong(byts))),
                                        &mut err_count,
                                        max_errors,
                                        "ping",
                                        id,
                                    ).await;
                                    res!(result);
                                }
                                WebSocketMessage::Pong(_byts) => {
                                    if let Some(timestamp) = ping_timestamp {
                                        let latency = timestamp.elapsed().as_millis();
                                        self.latency.set_and_adv(match u16::try_from(latency) {
                                            Ok(nu16) => Some(nu16),
                                            Err(_) => None,
                                        });
                                        ping_timestamp = None;
                                    } else {
                                        warn!("{}: Received unsolicited pong message.", id);
                                    }
                                }
                                WebSocketMessage::Close(status_code, reason) => {
                                    let result = self.close(status_code, reason).await;
                                    if let Err(e) = result {
                                        error!(err!(e,
                                            "{}: Error during WebSocket close:", id;
                                        IO, Network, Wire, Write));
                                    }
                                    break;
                                }
                            }
                        }
                        Ok(None) => {
                            break;
                        }
                        Err(e) => {
                            // Asked before the wrap, because wrapping puts the outer frame's tags
                            // in front of the ones underneath and `Error::tags` reads no deeper.
                            let too_big = e.tags().contains(&ErrTag::TooBig);
                            let e = err!(e,
                                "{}: Error reading websocket message:", id;
                            IO, Network, Wire, Read);
                            // A message over the limits was refused part way through a frame, so
                            // the stream no longer starts on a frame boundary and every further
                            // read would be of payload mistaken for a header. `read` has already
                            // sent the 1009 close; the connection ends here rather than counting
                            // this as one error among a permitted few.
                            if too_big {
                                error!(e);
                                break;
                            }
                            let result = self.response_handler(
                                Err(e),
                                &mut err_count,
                                max_errors,
                                "",
                                id,
                            ).await;
                            //res!(result);
                            if let Err(e) = result {
                                error!(e);
                                continue;
                            }
                        }
                    }
                }
                // Development refresh notifications.
                _ = async {
                    if let Some(receiver) = &mut dev_receiver {
                        // If a () message is received here...
                        receiver.recv().await.ok()
                    } else {
                        std::future::pending().await
                    }
                } => {
                    // ... a refresh message will be sent to the client here.
                    let refresh = WebSocketMessage::Text(WSH::DEV_REFRESH_MSG.to_string());
                    debug!("{}: POO Sending {:?}", id, refresh);
                    if let Err(e) = self.send(&refresh).await {
                        error!(err!(e,
                            "{}: Error sending refresh message:", id;
                        IO, Network, Wire, Write));
                    }
                }
                // Pings.
                _ = async {
                    if let Some(interval) = &mut interval {
                        interval.tick().await;
                    } else {
                        tokio::time::sleep(std::time::Duration::from_secs(std::u64::MAX)).await;
                    }
                } => {
                    if let Some(_) = &interval {
                        // Send a ping message.
                        let now = res!(Timestamp::now());
                        let ping_data = res!(now.to_bytes(Vec::new()));
                        let ping = WebSocketMessage::Ping(ping_data);
                        let result = self.send(&ping).await;
                        if let Err(e) = result {
                            let e = err!(e,
                                "{}: Error sending ping message:", id;
                            IO, Network, Wire, Write);
                            let result = self.response_handler(Err(e), &mut err_count, max_errors, "", id).await;
                            if let Err(e) = result {
                                error!(e);
                                continue;
                            }
                        } else {
                            ping_timestamp = Some(Instant::now());
                        }
                    }
                }
            }
        }

        Ok(())
    }

}

#[cfg(test)]
mod tests {
    use super::*;

    /// RFC 6455 §1.3 publishes a worked example of the handshake, and this is its key pair.  The
    /// expected value comes from the RFC, not from us, which is the whole point: a browser
    /// recomputes it independently, so agreeing with ourselves proves nothing.
    #[test]
    fn test_accept_key_rfc6455_vector_00() {
        assert_eq!(
            accept_key("dGhlIHNhbXBsZSBub25jZQ=="),
            "s3pPLMBiTxaQ9kYGzzhZRbK+xOo=",
        );
    }

    /// The GUID is byte-exact per RFC 6455 §4.2.2, and appending it is what distinguishes the
    /// accept value from a bare hash of the key.  Pinned so that a "tidy-up" cannot silently
    /// break every handshake.
    #[test]
    fn test_accept_key_guid_is_appended_00() {
        assert_eq!(constant::WEBSOCKET_GUID, "258EAFA5-E914-47DA-95CA-C5AB0DC85B11");
        // Without the GUID the digest differs, so this must not equal the vector above.
        let mut hasher = Sha1::new();
        hasher.update(b"dGhlIHNhbXBsZSBub25jZQ==");
        assert_ne!(base64::encode(&hasher.finalize()), "s3pPLMBiTxaQ9kYGzzhZRbK+xOo=");
    }

    /// The upgrade request RFC 6455 §1.2 prints, answered. The accept value is the RFC's, and the
    /// three header lines are the ones a browser insists on before it will call the socket open.
    #[test]
    fn test_accept_response_rfc6455_request_00() -> Outcome<()> {
        let req = HttpMessage {
            header:    res!(HttpHeader::parse(fmt!(
                "GET /chat HTTP/1.1\r\n\
                Host: server.example.com\r\n\
                Upgrade: websocket\r\n\
                Connection: Upgrade\r\n\
                Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\
                Sec-WebSocket-Version: 13\r\n\r\n"), Some(true))),
            body:      Vec::new(),
            head_only: false,
            file:      None,
        };
        let response = res!(accept_response(&req));
        assert_eq!(
            response,
            "HTTP/1.1 101 Switching Protocols\r\n\
            Upgrade: websocket\r\n\
            Connection: Upgrade\r\n\
            Sec-WebSocket-Accept: s3pPLMBiTxaQ9kYGzzhZRbK+xOo=\r\n\r\n",
        );
        Ok(())
    }

    /// A request with no key at all cannot be answered, and must say so rather than answer with a
    /// digest of nothing.
    #[test]
    fn test_accept_response_rejects_missing_key_00() -> Outcome<()> {
        let req = HttpMessage {
            header:    res!(HttpHeader::parse(fmt!(
                "GET /chat HTTP/1.1\r\n\
                Host: server.example.com\r\n\
                Upgrade: websocket\r\n\
                Connection: Upgrade\r\n\
                Sec-WebSocket-Version: 13\r\n\r\n"), Some(true))),
            body:      Vec::new(),
            head_only: false,
            file:      None,
        };
        assert!(accept_response(&req).is_err(),
            "a handshake with no Sec-WebSocket-Key must be refused");
        Ok(())
    }

    /// A server frame is unmasked and, for a short payload, exactly two header bytes: RFC 6455
    /// §5.2 fixes FIN|opcode then the length. The bytes below are what a browser's own decoder
    /// expects to see, so they are written out rather than recomputed.
    #[test]
    fn test_encode_message_server_text_frame_00() -> Outcome<()> {
        let byts = res!(encode_message(
            &WebSocketMessage::Text("hello".to_string()), false, 1024, 4096,
        ));
        assert_eq!(byts, vec![0x81, 0x05, b'h', b'e', b'l', b'l', b'o']);
        Ok(())
    }

    /// A client frame carries the mask bit and a four-byte key, and the payload is the plaintext
    /// XORed with it. RFC 6455 §5.3 requires this of every client-to-server frame.
    #[test]
    fn test_encode_message_client_masks_payload_00() -> Outcome<()> {
        let byts = res!(encode_message(
            &WebSocketMessage::Text("hello".to_string()), true, 1024, 4096,
        ));
        assert_eq!(byts[0], 0x81);
        assert_eq!(byts[1], 0x80 | 0x05, "the mask bit must be set on a client frame");
        assert_eq!(byts.len(), 2 + 4 + 5);
        let key = &byts[2..6];
        let unmasked: Vec<u8> = byts[6..].iter().enumerate()
            .map(|(i, b)| b ^ key[i % 4])
            .collect();
        assert_eq!(unmasked, b"hello".to_vec());
        Ok(())
    }

    /// What one end frames, the other end reads: a masked client message decodes back to the same
    /// text on the server side.
    #[tokio::test]
    async fn test_read_message_round_trip_masked_00() -> Outcome<()> {
        let byts = res!(encode_message(
            &WebSocketMessage::Text("the ceremony begins".to_string()), true, 1024, 4096,
        ));
        let mut src = &byts[..];
        let mut buffer = Vec::new();
        match res!(read_message(&mut src, &mut buffer, 1024, WebSocketLimits::default()).await) {
            Some(WebSocketMessage::Text(txt)) => assert_eq!(txt, "the ceremony begins"),
            other => return Err(err!(
                "Expected a text message, got {:?}.", other; Test, Mismatch)),
        }
        assert!(buffer.is_empty(), "the buffer must be left ready for the next message");
        Ok(())
    }

    /// A payload past the chunking threshold goes out as several frames -- first opcode, then
    /// continuations, `FIN` only on the last -- and arrives as one message.
    #[tokio::test]
    async fn test_read_message_reassembles_fragments_00() -> Outcome<()> {
        let payload: String = std::iter::repeat('x').take(300).collect();
        let byts = res!(encode_message(
            &WebSocketMessage::Binary(payload.as_bytes().to_vec()), false, 100, 100,
        ));
        // Four frames of 100 bytes' payload, each with a two-byte header.
        assert_eq!(byts.len(), 3 * (2 + 100));
        assert_eq!(byts[0], 0x02, "the first frame carries the opcode and no FIN");
        assert_eq!(byts[102], 0x00, "a middle frame carries the continuation opcode");
        assert_eq!(byts[204], 0x80, "the last frame carries FIN and the continuation opcode");
        let mut src = &byts[..];
        let mut buffer = Vec::new();
        match res!(read_message(&mut src, &mut buffer, 64, WebSocketLimits::default()).await) {
            Some(WebSocketMessage::Binary(got)) => {
                assert_eq!(got.len(), 300);
                assert_eq!(got, payload.as_bytes().to_vec());
            },
            other => return Err(err!(
                "Expected a binary message, got {:?}.", other; Test, Mismatch)),
        }
        Ok(())
    }

    /// A first frame carrying the continuation opcode continues nothing. It must be an error: this
    /// arrives from whoever is on the other end of the socket, and a panic there is a server a
    /// stranger can stop.
    #[tokio::test]
    async fn test_read_message_rejects_lone_continuation_00() -> Outcome<()> {
        let byts = vec![0x80, 0x02, b'h', b'i'];
        let mut src = &byts[..];
        let mut buffer = Vec::new();
        assert!(read_message(&mut src, &mut buffer, 1024, WebSocketLimits::default()).await.is_err(),
            "a lone continuation frame must be refused, not unwound");
        Ok(())
    }

    /// A closed connection reads as the end of the stream, not as an error.
    #[tokio::test]
    async fn test_read_message_eof_is_none_00() -> Outcome<()> {
        let byts: Vec<u8> = Vec::new();
        let mut src = &byts[..];
        let mut buffer = Vec::new();
        assert!(res!(read_message(&mut src, &mut buffer, 1024, WebSocketLimits::default()).await)
            .is_none(),
            "an immediately-closed stream must read as None");
        Ok(())
    }

    /// The header of a frame that declares more than the connection accepts, with as much of the
    /// payload as `tail` says following it. Written by hand rather than by `encode_message`,
    /// because a length no honest sender would write is the whole point.
    fn oversize_frame_header(declared: u64, tail: &[u8]) -> Vec<u8> {
        let mut byts = vec![0x82]; // FIN, binary.
        byts.push(127); // 64-bit extended length follows.
        byts.extend_from_slice(&declared.to_be_bytes());
        byts.extend_from_slice(tail);
        byts
    }

    /// A frame declaring more than the limit allows is refused, and refused on the strength of the
    /// declaration alone: the reader stops at the header, so the bytes that followed it are still
    /// unread when the error comes back.
    ///
    /// Before the bound existed this returned `Ok(None)` -- the reader allocated the megabyte the
    /// frame asked for, found the stream ended, and reported a closed connection.
    #[tokio::test]
    async fn test_read_message_refuses_oversize_frame_00() -> Outcome<()> {
        let limits = WebSocketLimits::new(1_024);
        let byts = oversize_frame_header(1_025, b"payload");
        let mut src = &byts[..];
        let mut buffer = Vec::new();
        let result = read_message(&mut src, &mut buffer, 256, limits).await;
        match result {
            Err(e) => assert!(e.tags().contains(&ErrTag::TooBig),
                "an over-limit frame must be tagged TooBig, so that a 1009 close can answer it; \
                got tags {:?}", e.tags()),
            Ok(other) => return Err(err!(
                "Expected a frame of 1025 bytes to be refused against a 1024 byte limit, got \
                {:?}.", other; Test, Mismatch)),
        }
        assert_eq!(src.len(), 7,
            "the seven payload bytes must be left unread: the frame was refused on its declared \
            length, before anything was reserved or read for it");
        assert!(buffer.is_empty(), "nothing of a refused frame belongs in the message buffer");
        Ok(())
    }

    /// A frame declaring more bytes than the machine has memory for is refused just the same, and
    /// the run of this test is itself the proof that the check precedes the allocation: a reader
    /// that allocated first would abort the process here, and an abort cannot be caught or
    /// reported. Nothing but a check before the `vec!` can make this test pass.
    #[tokio::test]
    async fn test_read_message_refuses_impossible_length_00() -> Outcome<()> {
        // The largest length RFC 6455 §5.2 permits: the most significant bit must be clear.
        let byts = oversize_frame_header(0x7FFF_FFFF_FFFF_FFFF, &[]);
        let mut src = &byts[..];
        let mut buffer = Vec::new();
        assert!(read_message(&mut src, &mut buffer, 1_024, WebSocketLimits::default()).await
            .is_err(),
            "a frame declaring eight exabytes must be refused, not reserved for");
        Ok(())
    }

    /// A large frame that is nonetheless within the limits still arrives whole. A bound that is
    /// only ever proved by what it rejects could be a reader that rejects everything.
    #[tokio::test]
    async fn test_read_message_allows_large_frame_within_limits_00() -> Outcome<()> {
        let payload: Vec<u8> = (0..200_000u32).map(|i| (i % 251) as u8).collect();
        // One frame, so the 64-bit length branch is the one exercised.
        let byts = res!(encode_message(
            &WebSocketMessage::Binary(payload.clone()), false, 1_000_000, 1_000_000,
        ));
        assert_eq!(byts[1] & 0x7F, 127, "a payload this size is framed with a 64-bit length");
        let limits = WebSocketLimits::new(256 * 1_024);
        let mut src = &byts[..];
        let mut buffer = Vec::new();
        match res!(read_message(&mut src, &mut buffer, 4_096, limits).await) {
            Some(WebSocketMessage::Binary(got)) => assert_eq!(got, payload),
            other => return Err(err!(
                "Expected a 200,000 byte binary message, got {:?}.", other; Test, Mismatch)),
        }
        Ok(())
    }

    /// Frames that each pass the frame bound can still add up past the message bound, since a
    /// message may be fragmented into as many continuations as the peer likes. The message bound
    /// is what stops the total, and it stops it at the frame that would breach it rather than
    /// after.
    #[tokio::test]
    async fn test_read_message_refuses_oversize_message_00() -> Outcome<()> {
        // Four frames of 1,000 bytes each: every one of them inside the 1,024 byte frame bound,
        // and together over the 2,048 byte message bound.
        let payload = vec![b'x'; 4_000];
        let byts = res!(encode_message(
            &WebSocketMessage::Binary(payload), false, 1_000, 1_000,
        ));
        assert_eq!(byts[0], 0x02, "the first of several frames carries the opcode and no FIN");
        let limits = WebSocketLimits {
            max_frame:  1_024,
            max_msg:    2_048,
        };
        let mut src = &byts[..];
        let mut buffer = Vec::new();
        match read_message(&mut src, &mut buffer, 256, limits).await {
            Err(e) => assert!(e.tags().contains(&ErrTag::TooBig),
                "an over-limit message must be tagged TooBig; got tags {:?}", e.tags()),
            Ok(other) => return Err(err!(
                "Expected four 1,000 byte frames to breach a 2,048 byte message limit, got {:?}.",
                other; Test, Mismatch)),
        }
        // Two frames were taken; the third was refused on its four-byte header, so the rest of
        // that frame and the whole of the fourth are still unread.
        assert_eq!(src.len(), 2 * (4 + 1_000) - 4,
            "the reader must stop at the header of the frame that would breach the limit");
        Ok(())
    }

    /// Every frame of a fragmented message is bounded, not merely the first: a peer that opens
    /// with a modest frame and continues with an enormous one is refused on the continuation.
    #[tokio::test]
    async fn test_read_message_bounds_continuation_frames_00() -> Outcome<()> {
        let mut byts = vec![0x02, 0x02, b'h', b'i']; // Binary, no FIN, two bytes.
        // A continuation frame with FIN set, declaring far more than the bound.
        byts.push(0x80);
        byts.push(127);
        byts.extend_from_slice(&(1u64 << 40).to_be_bytes());
        let mut src = &byts[..];
        let mut buffer = Vec::new();
        assert!(read_message(&mut src, &mut buffer, 256, WebSocketLimits::new(4_096)).await.is_err(),
            "a continuation frame is as capable of declaring a huge payload as a first frame");
        Ok(())
    }

    /// RFC 6455 §5.5 caps a control frame at 125 bytes, so a ping declaring more than that is
    /// malformed whatever the connection's own limits say. Checked separately because a generous
    /// limit would otherwise let a peer make the reader hold megabytes for a frame the protocol
    /// says is small.
    #[tokio::test]
    async fn test_read_message_refuses_oversize_control_frame_00() -> Outcome<()> {
        // FIN, ping, 16-bit length of 126 -- one byte over what a control frame may carry.
        let mut byts = vec![0x89, 126];
        byts.extend_from_slice(&126u16.to_be_bytes());
        byts.extend_from_slice(&vec![0u8; 126]);
        let mut src = &byts[..];
        let mut buffer = Vec::new();
        assert!(read_message(&mut src, &mut buffer, 256, WebSocketLimits::default()).await.is_err(),
            "a control frame over 125 bytes must be refused");
        Ok(())
    }

    /// The defaults bound both dimensions, and the message bound is the looser of the two -- a
    /// message bound below the frame bound would make the frame bound unreachable and the pair
    /// misleading.
    #[test]
    fn test_websocket_limits_defaults_00() {
        let limits = WebSocketLimits::default();
        assert_eq!(limits.max_frame, constant::WEBSOCKET_MAX_FRAME_BYTES);
        assert_eq!(limits.max_msg, constant::WEBSOCKET_MAX_MESSAGE_BYTES);
        assert!(limits.max_msg >= limits.max_frame,
            "a message must be allowed to be at least as large as one frame of it");
    }

    /// The declared length is checked as the 64-bit number it arrived as. Narrowed to a `usize`
    /// first, this length would be one byte on a 32-bit target and would pass any bound at all.
    #[test]
    fn test_websocket_limits_check_frame_is_64_bit_00() -> Outcome<()> {
        let limits = WebSocketLimits::new(1_024);
        assert!(limits.check_frame((1u64 << 32) + 1).is_err(),
            "a length that truncates to 1 in 32 bits must still be refused");
        assert!(limits.check_frame(1_024).is_ok(), "a length exactly at the bound is allowed");
        assert!(limits.check_frame(1_025).is_err(), "a length one over the bound is refused");
        Ok(())
    }
}
