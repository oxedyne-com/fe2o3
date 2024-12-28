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

use oxedize_fe2o3_core::{
    prelude::*,
    byte::ToBytes,
    rand::Rand,
};
use oxedize_fe2o3_data::{
    ring::RingBuffer,
    time::Timestamp,
};
use oxedize_fe2o3_iop_crypto::enc::Encrypter;
use oxedize_fe2o3_iop_db::api::Database;
use oxedize_fe2o3_iop_hash::api::Hasher;
use oxedize_fe2o3_jdat::id::NumIdDat;
use oxedize_fe2o3_syntax::SyntaxRef;

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
            header: res!(HttpHeader::parse(msg, Some(true))),
            body:   Vec::new(),
        },
        key_str,
    ))
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
            phantom1:       PhantomData,
            phantom2:       PhantomData,
            phantom3:       PhantomData,
            phantom4:       PhantomData,
        }
    }

    pub fn is_server(&self) -> bool { self.is_server }
    pub fn is_client(&self) -> bool { !self.is_server }

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
        >(Pin::new(&mut self.stream), &Vec::new(), Some(false)).await;
        match result {
            Ok((Some(response), _)) => {

                let accept_key = Self::accept_key(&key);

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

    /// The accept key is just a hash of the incoming request key.
    pub fn accept_key(key: &String) -> String {
        let concatenated = fmt!("{}{}", key, constant::WEBSOCKET_GUID);
        let mut hasher = Sha1::new();
        hasher.update(concatenated.as_bytes());
        let hash = hasher.finalize();
        base64::encode(&hash)
    }

    /// The HTTP(S) server has detected a websocket upgrade request message and passes it to this
    /// method to complete the handshake.
    pub async fn connect_as_server(
        &mut self,
        request: HttpMessage,
    )
        -> Outcome<()>
    {
        let key = match request.header.get_the_field_value(&HeaderName::SecWebSocketKey) { 
            Ok(HeaderFieldValue::SecWebSocketKey(key)) => { 
                let key_byts = match base64::decode(key) {
                    Ok(byts) => byts,
                    Err(e) => return Err(err!(e,
                        "The websocket key provided is not valid base64.";
                    IO, Network, Invalid, Input, String, Conversion)),
                };
                if key_byts.len() == 16 {
                    key
                } else {
                    return Err(err!(
                        "The websocket key is {} bytes long, expected 16.", key_byts.len();
                    IO, Network, Invalid, Input, Mismatch, Size));
                }
            },
            _ => return Err(err!(
                "The websocket key string is missing.";
            IO, Network, Input, Missing)),
        };

        let accept_key = Self::accept_key(&key);

        let response = fmt!(
            "HTTP/1.1 101 Switching Protocols\r\n\
            Upgrade: websocket\r\n\
            Connection: Upgrade\r\n\
            Sec-WebSocket-Accept: {}\r\n\r\n",
            accept_key,
        );

        match self.stream.write_all(response.as_bytes()).await {
            Ok(()) => (),
            Err(e) => return Err(err!(e,
                "Could not send websocket handshake response.";
            IO, Network, Wire, Write)),
        }

        info!("Server connection successfully upgraded to a websocket.");

        Ok(())
    }

    pub async fn read(&mut self) -> Outcome<Option<WebSocketMessage>> {
        let mut is_final_frame = false;
        let mut opcode = 0;
    
        while !is_final_frame {
            // Read the first byte of the frame header.
            let mut header_byte = [0u8; 1];
            let result = self.stream.read_exact(&mut header_byte).await;
            match result {
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
            
            if opcode == 0 {
                opcode = header_byte[0] & 0x0F;
            }
        
            // Read the second byte of the frame header.
            let mut length_byte = [0u8; 1];
            let result = self.stream.read_exact(&mut length_byte).await;
            match result {
                Ok(_n) => (),
                Err(e) if e.kind() == tokio::io::ErrorKind::UnexpectedEof => {
                    return Ok(None);
                }
                Err(e) => return Err(err!(e,
                    "While trying to read second byte of the frame header.";
                IO, Network, Read, Wire)),
            }
        
            // Extract the payload length and mask flag from the length byte.
            let masked = (length_byte[0] & 0x80) != 0;
            let payload_length = match length_byte[0] & 0x7F {
                127 => {
                    // 64-bit extended payload length.
                    let mut extended_length_bytes = [0u8; 8];
                    let result = self.stream.read_exact(&mut extended_length_bytes).await;
                    match result {
                        Ok(_n) => (),
                        Err(e) if e.kind() == tokio::io::ErrorKind::UnexpectedEof => {
                            return Ok(None);
                        }
                        Err(e) => return Err(err!(e,
                            "While trying to read second byte of the frame header.";
                        IO, Network, Read, Wire)),
                    }
                    u64::from_be_bytes(extended_length_bytes) as usize
                }
                126 => {
                    // 16-bit extended payload length.
                    let mut extended_length_bytes = [0u8; 2];
                    let result = self.stream.read_exact(&mut extended_length_bytes).await;
                    match result {
                        Ok(_n) => (),
                        Err(e) if e.kind() == tokio::io::ErrorKind::UnexpectedEof => {
                            return Ok(None);
                        }
                        Err(e) => return Err(err!(e,
                            "While trying to read second byte of the frame header.";
                        IO, Network, Read, Wire)),
                    }
                    u16::from_be_bytes(extended_length_bytes) as usize
                }
                len => len as usize,
            };

            // Read the masking key if the frame is masked.
            let mut masking_key = [0u8; 4];
            if masked {
                let result = self.stream.read_exact(&mut masking_key).await;
                match result {
                    Ok(_n) => (),
                    Err(e) if e.kind() == tokio::io::ErrorKind::UnexpectedEof => {
                        return Ok(None);
                    }
                    Err(e) => return Err(err!(e,
                        "While trying to read second byte of the frame header.";
                    IO, Network, Read, Wire)),
                }
            }
        
            // Read the (possibly fragmented) payload data.
            let mut payload = vec![0u8; payload_length];
            let mut bytes_read = 0;
            while bytes_read < payload_length {
                let chunk_size = std::cmp::min(
                    self.chunk_size,
                    payload_length - bytes_read,
                );
                let result = self.stream.read_exact(&mut payload[
                    bytes_read..bytes_read + chunk_size
                ]).await;
                match result {
                    Ok(_n) => {
                        // Unmask the payload chunk if the frame is masked.
                        if masked {
                            for i in 0..chunk_size {
                                payload[bytes_read + i] ^= masking_key[(bytes_read + i) % 4];
                            }
                        }
                        bytes_read += chunk_size;
                        self.buffer.extend_from_slice(&payload[
                            bytes_read - chunk_size..bytes_read
                        ]);
                    }
                    Err(e) if e.kind() == tokio::io::ErrorKind::UnexpectedEof => {
                        return Ok(None);
                    }
                    Err(e) => return Err(err!(e,
                        "While trying to read payload chunk.";
                    IO, Network, Read, Wire)),
                }
            }

            // Unmask the payload if the frame is masked.
            if masked {
                for i in 0..payload_length {
                    payload[i] ^= masking_key[i % 4];
                }
            }
        }
        
        // Construct the appropriate WebSocketMessage variant based on the opcode.
        let message = match opcode {
            0x0 => {
                // Continuation frame (not supported in this example).
                unimplemented!("Continuation frames are not supported");
            }
            0x1 => {
                // Text frame.
                let text = res!(std::str::from_utf8(&self.buffer)).to_string();
                WebSocketMessage::Text(text)
            }
            0x2 => {
                // Binary frame.
                WebSocketMessage::Binary(self.buffer.clone())
            }
            0x8 => {
                // Close frame.
                let status_code = if self.buffer.len() >= 2 {
                    let nu16 = u16::from_be_bytes([self.buffer[0], self.buffer[1]]);
                    let code = res!(WebSocketStatusCode::try_from(nu16));
                    Some(code)
                } else {
                    None
                };
                let reason = if self.buffer.len() > 2 {
                    Some(res!(std::str::from_utf8(&self.buffer[2..])).to_string())
                } else {
                    None
                };
                WebSocketMessage::Close(status_code, reason)
            }
            0x9 => {
                // Ping frame.
                WebSocketMessage::Ping(self.buffer.clone())
            }
            0xA => {
                // Pong frame.
                WebSocketMessage::Pong(self.buffer.clone())
            }
            _ => {
                // Unknown opcode.
                return Err(err!("Unknown opcode: {}", opcode; IO, Network, Invalid, Input));
            }
        };
    
        // Clear the buffer for the next message.
        self.buffer.clear();
    
        Ok(Some(message))
    }    

    pub async fn send(
        &mut self,
        message: &WebSocketMessage,
    )
        -> Outcome<()>
    {
        // Determine the opcode based on the message type.
        let initial_opcode = match message {
            WebSocketMessage::Text(_) => 0x1,
            WebSocketMessage::Binary(_) => 0x2,
            WebSocketMessage::Ping(_) => 0x9,
            WebSocketMessage::Pong(_) => 0xA,
            WebSocketMessage::Close(_, _) => 0x8,
        };
    
        // Get the payload data and its length.
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
    
        // Generate masking key for client-side masking.
        let mut masking_key = [0u8; 4];
        if self.is_client() {
            Rand::fill_u8(&mut masking_key);
        }

        // Determine if chunking is required based on the chunking threshold.
        if payload_length > self.chunk_thresh {
            // Send the message in chunks.
            let mut bytes_sent = 0;
            while bytes_sent < payload_length {
                let remaining_bytes = payload_length - bytes_sent;
                let chunk_size = std::cmp::min(remaining_bytes, self.chunk_size);
                let is_final = remaining_bytes <= self.chunk_size;

                // Use initial opcode for first frame, continuation (0x0) for others
                let opcode = if bytes_sent == 0 { initial_opcode } else { 0x0 };

                // Construct the frame header.
                let mut header = Vec::new();

                // First byte: FIN bit and opcode
                header.push(if is_final { 0x80 | opcode } else { opcode });

                // Second byte: Mask bit set for client-side masking and payload length.
                let mask_bit = if self.is_client() { 0x80 } else { 0x00 };
                if chunk_size <= 125 {
                    header.push(mask_bit | chunk_size as u8);
                } else if chunk_size <= 65535 {
                    header.push(mask_bit | 126);
                    header.extend_from_slice(&(chunk_size as u16).to_be_bytes());
                } else {
                    header.push(mask_bit | 127);
                    header.extend_from_slice(&(chunk_size as u64).to_be_bytes());
                }

                // Write the masking key for client-side masking.
                if self.is_client() {
                    header.extend_from_slice(&masking_key);
                }

                // Write the frame header to the stream.
                let result = self.stream.write_all(&header).await;
                res!(result);

                // Mask and send payload chunk.
                let mut chunk = payload[bytes_sent..bytes_sent + chunk_size].to_vec();

                if self.is_client() {
                    for i in 0..chunk_size {
                        chunk[i] ^= masking_key[i % 4];
                    }
                }

                // Write the payload chunk to the stream.
                let result = self.stream.write_all(&chunk).await;
                res!(result);

                bytes_sent += chunk_size;
            }
        } else {
            // Send the message as a single frame.
            // Construct the frame header.
            let mut header = Vec::new();

            // First byte: FIN bit set and opcode.
            header.push(0x80 | initial_opcode);

            // Second byte: Mask bit set for client-side masking and payload length.
            let mask_bit = if self.is_client() { 0x80 } else { 0x00 };
            if payload_length <= 125 {
                header.push(mask_bit | payload_length as u8);
            } else if payload_length <= 65535 {
                header.push(mask_bit | 126);
                header.extend_from_slice(&(payload_length as u16).to_be_bytes());
            } else {
                header.push(mask_bit | 127);
                header.extend_from_slice(&(payload_length as u64).to_be_bytes());
            }

            // Write the masking key for client-side masking.
            if self.is_client() {
                header.extend_from_slice(&masking_key);
            }

            // Write the frame header to the stream.
            let result = self.stream.write_all(&header).await;
            res!(result);

            // Apply masking to the payload for client-side masking.
            let mut masked_payload = payload.clone();
            if self.is_client() {
                for i in 0..payload_length {
                    masked_payload[i] ^= masking_key[i % 4];
                }
            }

            // Write the masked payload to the stream.
            let result = self.stream.write_all(&masked_payload).await;
            res!(result);
        }
    
        // Flush the stream.
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
                            let e = err!(e,
                                "{}: Error reading websocket message:", id;
                            IO, Network, Wire, Read);
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
