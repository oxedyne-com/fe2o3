#![cfg(feature = "async")]
//! `WsClient` against a hand-written tokio server on loopback. The server side is built from the
//! crate's framing functions plus raw bytes, and every client frame it reads is checked for the
//! mask RFC 6455 §5.3 requires.

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_net::ws::{
    accept_key,
    encode_message,
    read_frame,
    status::WebSocketStatusCode,
    WebSocketFrame,
    WebSocketLimits,
    WebSocketMessage,
    WsClient,
};

use std::time::Duration;

use tokio::{
    io::{
        AsyncReadExt,
        AsyncWriteExt,
    },
    net::{
        TcpListener,
        TcpStream,
    },
    task::JoinHandle,
};


const WAIT: Duration = Duration::from_secs(5);

/// Binds a loopback listener and runs `serve` on the first connection it accepts, returning the
/// address to connect to and the server task.
async fn spawn_server<F, Fut>(serve: F) -> Outcome<(String, JoinHandle<Outcome<()>>)>
where
    F:      FnOnce(TcpStream) -> Fut + Send + 'static,
    Fut:    std::future::Future<Output = Outcome<()>> + Send + 'static,
{
    let listener = res!(TcpListener::bind("127.0.0.1:0").await);
    let addr = res!(listener.local_addr());
    let task = tokio::spawn(async move {
        let (stream, _) = res!(listener.accept().await);
        serve(stream).await
    });
    Ok((addr.to_string(), task))
}

/// Reads the upgrade request up to its blank line, byte by byte so that nothing after it is
/// consumed.
async fn read_request(stream: &mut TcpStream) -> Outcome<String> {
    let mut head = Vec::new();
    while !head.ends_with(b"\r\n\r\n") {
        let mut b = [0u8; 1];
        res!(stream.read_exact(&mut b).await);
        head.push(b[0]);
    }
    Ok(res!(String::from_utf8(head)))
}

fn request_key(request: &str) -> Outcome<String> {
    for line in request.lines() {
        if let Some((name, value)) = line.split_once(':') {
            if name.trim().eq_ignore_ascii_case("sec-websocket-key") {
                return Ok(value.trim().to_string());
            }
        }
    }
    Err(err!("The upgrade request has no Sec-WebSocket-Key:\n{}", request; Test, Missing))
}

fn switching(accept: &str) -> String {
    fmt!("HTTP/1.1 101 Switching Protocols\r\n\
        Upgrade: websocket\r\n\
        Connection: Upgrade\r\n\
        Sec-WebSocket-Accept: {}\r\n\r\n", accept)
}

/// Completes the handshake properly, returning the request text.
async fn accept(stream: &mut TcpStream) -> Outcome<String> {
    let request = res!(read_request(stream).await);
    let key = res!(request_key(&request));
    res!(stream.write_all(switching(&accept_key(&key)).as_bytes()).await);
    Ok(request)
}

/// Reads one client frame, insisting that it is masked.
async fn read_masked(stream: &mut TcpStream) -> Outcome<WebSocketFrame> {
    match res!(read_frame(stream, 1_024, WebSocketLimits::default(), 0).await) {
        Some(frame) => {
            if !frame.masked {
                return Err(err!(
                    "The client sent an unmasked frame with opcode {:#x}.", frame.opcode;
                Test, Invalid));
            }
            Ok(frame)
        }
        None => Err(err!("The client closed before sending a frame."; Test, Missing)),
    }
}

async fn join(task: JoinHandle<Outcome<()>>) -> Outcome<()> {
    match task.await {
        Ok(outcome) => outcome,
        Err(e)      => Err(err!(e, "The test server task failed."; Test)),
    }
}

#[tokio::test]
async fn test_ws_client_handshake_and_echo_00() -> Outcome<()> {
    let (addr, server) = res!(spawn_server(|mut stream| async move {
        let request = res!(accept(&mut stream).await);
        let addr = res!(stream.local_addr());
        // The path goes on the request line and the Host field carries the authority alone.
        // Field names are case-insensitive (RFC 9110 §5.1).
        let lower = request.to_lowercase();
        if !request.starts_with("GET /peer HTTP/1.1\r\n")
            || !lower.contains(&fmt!("\r\nhost: {}\r\n", addr))
            || !lower.contains("\r\norigin: https://test.example\r\n")
        {
            return Err(err!("Unexpected upgrade request:\n{}", request; Test, Mismatch));
        }
        let frame = res!(read_masked(&mut stream).await);
        let echo = res!(encode_message(
            &WebSocketMessage::Text(res!(String::from_utf8(frame.payload))), false, 1_024, 1_024));
        res!(stream.write_all(&echo).await);
        Ok(())
    }).await);

    let mut client = res!(WsClient::connect(&addr, "/peer", Some("https://test.example")).await);
    res!(client.send_text("hello peer").await);
    match res!(client.recv(WAIT).await) {
        Some(WebSocketMessage::Text(txt)) => req!(txt, fmt!("hello peer")),
        other => return Err(err!("Expected the echo, got {:?}.", other; Test, Mismatch)),
    }
    join(server).await
}

#[tokio::test]
async fn test_ws_client_refusal_names_status_00() -> Outcome<()> {
    let (addr, server) = res!(spawn_server(|mut stream| async move {
        res!(read_request(&mut stream).await);
        res!(stream.write_all(
            b"HTTP/1.1 400 Bad Request\r\nContent-Length: 4\r\n\r\nnope").await);
        Ok(())
    }).await);

    match WsClient::connect(&addr, "/peer", None).await {
        Ok(_)   => return Err(err!("A 400 refusal was taken as a handshake."; Test, Unexpected)),
        Err(e)  => {
            let msg = fmt!("{}", e);
            if !msg.contains("400") {
                return Err(err!("The refusal error does not name the status: {}", msg;
                    Test, Mismatch));
            }
        }
    }
    join(server).await
}

#[tokio::test]
async fn test_ws_client_wrong_accept_key_refused_00() -> Outcome<()> {
    let wrong = accept_key("AAAAAAAAAAAAAAAAAAAAAA==");
    let wrong_srv = wrong.clone();
    let (addr, server) = res!(spawn_server(|mut stream| async move {
        res!(read_request(&mut stream).await);
        res!(stream.write_all(switching(&wrong_srv).as_bytes()).await);
        Ok(())
    }).await);

    match WsClient::connect(&addr, "/peer", None).await {
        Ok(_)   => return Err(err!(
            "A 101 carrying the wrong Sec-WebSocket-Accept was taken as a handshake.";
        Test, Unexpected)),
        Err(e)  => {
            let msg = fmt!("{}", e);
            if !msg.contains(&wrong) {
                return Err(err!("The key error does not name the key received: {}", msg;
                    Test, Mismatch));
            }
        }
    }
    join(server).await
}

/// A text message in three fragments with a ping between the first two: the client must answer
/// the ping at once and still deliver the text whole.
#[tokio::test]
async fn test_ws_client_reassembles_fragments_and_answers_ping_00() -> Outcome<()> {
    let text: String = "0123456789abcdefghij".to_string();
    let text_srv = text.clone();
    let (addr, server) = res!(spawn_server(|mut stream| async move {
        res!(accept(&mut stream).await);
        // Frames of 8 payload bytes, each with a two-byte header: 10 + 10 + 6.
        let frags = res!(encode_message(&WebSocketMessage::Text(text_srv), false, 8, 8));
        if frags.len() != 26 || frags[0] != 0x01 || frags[10] != 0x00 || frags[20] != 0x80 {
            return Err(err!("Unexpected fragmentation: {:02x?}", frags; Test, Mismatch));
        }
        let ping = res!(encode_message(&WebSocketMessage::Ping(b"hb".to_vec()), false, 125, 125));
        res!(stream.write_all(&frags[..10]).await);
        res!(stream.write_all(&ping).await);
        // The pong must come back while the message is still incomplete.
        let pong = res!(read_masked(&mut stream).await);
        if pong.opcode != 0xA || pong.payload != b"hb" {
            return Err(err!("Expected a pong echoing 'hb', got {:?}.", pong; Test, Mismatch));
        }
        res!(stream.write_all(&frags[10..]).await);
        Ok(())
    }).await);

    let mut client = res!(WsClient::connect(&addr, "/", None).await);
    match res!(client.recv(WAIT).await) {
        Some(WebSocketMessage::Text(got)) => req!(got, text),
        other => return Err(err!("Expected the reassembled text, got {:?}.", other;
            Test, Mismatch)),
    }
    join(server).await
}

/// A frame over the client's limit is refused before its payload is read, the server hears a
/// 1009 close, and the client is left unusable.
#[tokio::test]
async fn test_ws_client_refuses_over_limit_frame_00() -> Outcome<()> {
    let (addr, server) = res!(spawn_server(|mut stream| async move {
        res!(accept(&mut stream).await);
        let big = res!(encode_message(
            &WebSocketMessage::Binary(vec![7u8; 1_000]), false, 4_096, 4_096));
        res!(stream.write_all(&big).await);
        let close = res!(read_masked(&mut stream).await);
        let code = u16::from_be_bytes([close.payload[0], close.payload[1]]);
        if close.opcode != 0x8 || code != 1009 {
            return Err(err!("Expected a 1009 close, got {:?}.", close; Test, Mismatch));
        }
        Ok(())
    }).await);

    let mut client = res!(WsClient::connect(&addr, "/", None).await)
        .with_limits(WebSocketLimits::new(64));
    match client.recv(WAIT).await {
        Ok(msg) => return Err(err!("An over-limit frame was accepted: {:?}", msg;
            Test, Unexpected)),
        Err(e)  => if !e.tags().contains(&ErrTag::TooBig) {
            return Err(err!(e, "The refusal lacks the TooBig tag."; Test, Mismatch));
        },
    }
    if client.is_open() {
        return Err(err!("The client stayed open after a refused frame."; Test, Unexpected));
    }
    join(server).await
}

/// Text, binary, ping-answering pong and close all leave the client masked, and the close
/// handshake completes when the server echoes.
#[tokio::test]
async fn test_ws_client_masks_every_frame_00() -> Outcome<()> {
    let (addr, server) = res!(spawn_server(|mut stream| async move {
        res!(accept(&mut stream).await);
        let ping = res!(encode_message(&WebSocketMessage::Ping(vec![1, 2]), false, 125, 125));
        res!(stream.write_all(&ping).await);
        let mut opcodes = Vec::new();
        loop {
            let frame = res!(read_masked(&mut stream).await);
            opcodes.push(frame.opcode);
            if frame.opcode == 0x8 {
                let echo = res!(encode_message(
                    &WebSocketMessage::Close(Some(WebSocketStatusCode::NormalClosure), None),
                    false, 125, 125));
                res!(stream.write_all(&echo).await);
                break;
            }
        }
        req!(opcodes, vec![0xA, 0x1, 0x2, 0x8]);
        Ok(())
    }).await);

    let mut client = res!(WsClient::connect(&addr, "/", None).await);
    // Nothing but the ping arrives, so this times out having answered it.
    match res!(client.recv(Duration::from_millis(300)).await) {
        None        => (),
        Some(msg)   => return Err(err!("Expected nothing, got {:?}.", msg; Test, Unexpected)),
    }
    res!(client.send_text("t").await);
    res!(client.send(&WebSocketMessage::Binary(vec![9; 200])).await);
    res!(client.close(WAIT).await);
    if client.send_text("late").await.is_ok() {
        return Err(err!("A closed client sent a message."; Test, Unexpected));
    }
    join(server).await
}

/// A deadline passing between two fragments loses nothing: the next call completes the message.
#[tokio::test]
async fn test_ws_client_timeout_keeps_partial_message_00() -> Outcome<()> {
    let (addr, server) = res!(spawn_server(|mut stream| async move {
        res!(accept(&mut stream).await);
        let frags = res!(encode_message(
            &WebSocketMessage::Binary(b"first-second".to_vec()), false, 6, 6));
        res!(stream.write_all(&frags[..8]).await);
        tokio::time::sleep(Duration::from_millis(400)).await;
        res!(stream.write_all(&frags[8..]).await);
        Ok(())
    }).await);

    let mut client = res!(WsClient::connect(&addr, "/", None).await);
    match res!(client.recv(Duration::from_millis(100)).await) {
        None        => (),
        Some(msg)   => return Err(err!("Expected a timeout, got {:?}.", msg; Test, Unexpected)),
    }
    match res!(client.recv(WAIT).await) {
        Some(WebSocketMessage::Binary(got)) => req!(got, b"first-second".to_vec()),
        other => return Err(err!("Expected the whole message, got {:?}.", other;
            Test, Mismatch)),
    }
    join(server).await
}

/// A server-initiated close is echoed with the same status and returned, and the client is then
/// closed.
#[tokio::test]
async fn test_ws_client_echoes_server_close_00() -> Outcome<()> {
    let (addr, server) = res!(spawn_server(|mut stream| async move {
        res!(accept(&mut stream).await);
        let close = res!(encode_message(
            &WebSocketMessage::Close(Some(WebSocketStatusCode::GoingAway), Some(fmt!("bye"))),
            false, 125, 125));
        res!(stream.write_all(&close).await);
        let echo = res!(read_masked(&mut stream).await);
        let code = u16::from_be_bytes([echo.payload[0], echo.payload[1]]);
        if echo.opcode != 0x8 || code != 1001 {
            return Err(err!("Expected a 1001 close echo, got {:?}.", echo; Test, Mismatch));
        }
        Ok(())
    }).await);

    let mut client = res!(WsClient::connect(&addr, "/", None).await);
    match res!(client.recv(WAIT).await) {
        Some(WebSocketMessage::Close(Some(WebSocketStatusCode::GoingAway), Some(reason))) =>
            req!(reason, fmt!("bye")),
        other => return Err(err!("Expected the server's close, got {:?}.", other;
            Test, Mismatch)),
    }
    if client.is_open() {
        return Err(err!("The client stayed open after the server closed."; Test, Unexpected));
    }
    join(server).await
}
