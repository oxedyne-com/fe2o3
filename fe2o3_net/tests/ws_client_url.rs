#![cfg(feature = "async")]
//! `WsClient::connect_url` against hand-written loopback servers, plain and TLS: the scheme
//! chooses the transport, the query and further fields reach the upgrade request as given, a
//! refusal quotes its reason, and ping, pong and the close handshake work over TLS as over TCP.

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_net::{
    tls::ensure_crypto_provider,
    ws::{
        accept_key,
        connect_request_with,
        encode_message,
        read_frame,
        status::WebSocketStatusCode,
        WebSocketFrame,
        WebSocketLimits,
        WebSocketMessage,
        WsClient,
        WsConnect,
    },
};

use std::{
    future::Future,
    sync::Arc,
    time::Duration,
};

use tokio::{
    io::{
        AsyncRead,
        AsyncReadExt,
        AsyncWrite,
        AsyncWriteExt,
    },
    net::{
        TcpListener,
        TcpStream,
    },
    task::JoinHandle,
};
use tokio_rustls::{
    rustls::{
        pki_types::{
            CertificateDer,
            PrivateKeyDer,
            PrivatePkcs8KeyDer,
        },
        ClientConfig,
        RootCertStore,
        ServerConfig,
    },
    server::TlsStream,
    TlsAcceptor,
};


const HOST: &str = "localhost";
const WAIT: Duration = Duration::from_secs(5);
const TARGET: &str = "/api/chat/relay?v=1&device=d0&ts=1793352150&nonce=n_0-x&sigv=2&sig=YWJj-_w";

/// A certificate for [`HOST`], a server holding it, and a client whose one root it is.
fn tls_pair() -> Outcome<(ServerConfig, ClientConfig)> {
    ensure_crypto_provider();
    let cert = res!(rcgen::generate_simple_self_signed(vec![HOST.to_string()]), Init);
    let der = CertificateDer::from(res!(cert.serialize_der(), Init));
    let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(cert.serialize_private_key_der()));
    let srv = res!(ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![der.clone()], key), Init);
    let mut roots = RootCertStore::empty();
    res!(roots.add(der), Init);
    let cli = ClientConfig::builder().with_root_certificates(roots).with_no_client_auth();
    Ok((srv, cli))
}

/// Binds a loopback listener and runs `serve` on the first connection, returning the port and
/// the server task.
async fn spawn_plain<F, Fut>(serve: F) -> Outcome<(u16, JoinHandle<Outcome<()>>)>
where
    F:      FnOnce(TcpStream, u16) -> Fut + Send + 'static,
    Fut:    Future<Output = Outcome<()>> + Send + 'static,
{
    let listener = res!(TcpListener::bind("127.0.0.1:0").await);
    let port = res!(listener.local_addr()).port();
    let task = tokio::spawn(async move {
        let (tcp, _) = res!(listener.accept().await);
        serve(tcp, port).await
    });
    Ok((port, task))
}

/// As [`spawn_plain`], with the connection TLS-wrapped under `cfg`.
async fn spawn_tls<F, Fut>(cfg: ServerConfig, serve: F) -> Outcome<(u16, JoinHandle<Outcome<()>>)>
where
    F:      FnOnce(TlsStream<TcpStream>, u16) -> Fut + Send + 'static,
    Fut:    Future<Output = Outcome<()>> + Send + 'static,
{
    spawn_plain(move |tcp, port| async move {
        let tls = res!(TlsAcceptor::from(Arc::new(cfg)).accept(tcp).await);
        serve(tls, port).await
    }).await
}

async fn read_request<S: AsyncRead + Unpin>(stream: &mut S) -> Outcome<String> {
    let mut head = Vec::new();
    while !head.ends_with(b"\r\n\r\n") {
        let mut b = [0u8; 1];
        res!(stream.read_exact(&mut b).await);
        head.push(b[0]);
    }
    Ok(res!(String::from_utf8(head)))
}

fn field<'a>(request: &'a str, name: &str) -> Option<&'a str> {
    for line in request.lines() {
        if let Some((n, v)) = line.split_once(':') {
            if n.trim().eq_ignore_ascii_case(name) {
                return Some(v.trim());
            }
        }
    }
    None
}

/// Completes the handshake, returning the request text.
async fn accept<S: AsyncRead + AsyncWrite + Unpin>(stream: &mut S) -> Outcome<String> {
    let request = res!(read_request(stream).await);
    let key = res!(field(&request, "sec-websocket-key")
        .ok_or_else(|| err!("No Sec-WebSocket-Key:\n{}", request; Test, Missing)));
    res!(stream.write_all(fmt!("HTTP/1.1 101 Switching Protocols\r\n\
        Upgrade: websocket\r\n\
        Connection: Upgrade\r\n\
        Sec-WebSocket-Accept: {}\r\n\r\n", accept_key(key)).as_bytes()).await);
    Ok(request)
}

async fn read_masked<S: AsyncRead + Unpin>(stream: &mut S) -> Outcome<WebSocketFrame> {
    match res!(read_frame(stream, 1_024, WebSocketLimits::default(), 0).await) {
        Some(frame) if frame.masked => Ok(frame),
        Some(frame) => Err(err!("Unmasked client frame {:#x}.", frame.opcode; Test, Invalid)),
        None        => Err(err!("The client closed before sending a frame."; Test, Missing)),
    }
}

async fn join(task: JoinHandle<Outcome<()>>) -> Outcome<()> {
    match task.await {
        Ok(outcome) => outcome,
        Err(e)      => Err(err!(e, "The test server task failed."; Test)),
    }
}

/// Over `wss`: the query and a further field arrive verbatim, a text echoes, a ping is answered
/// with its pong, and a close from the client completes when the server echoes it.
#[tokio::test]
async fn test_wss_url_carries_query_and_fields_and_closes_cleanly_00() -> Outcome<()> {
    let (srv, cli) = res!(tls_pair());
    let (port, server) = res!(spawn_tls(srv, move |mut s, port| async move {
        let request = res!(accept(&mut s).await);
        if !request.starts_with(&fmt!("GET {} HTTP/1.1\r\n", TARGET))
            || field(&request, "host") != Some(&fmt!("{}:{}", HOST, port))
            || field(&request, "x-probe") != Some("one two")
            || field(&request, "origin") != Some("https://next.example")
        {
            return Err(err!("Unexpected upgrade request:\n{}", request; Test, Mismatch));
        }
        let text = res!(read_masked(&mut s).await);
        res!(s.write_all(&res!(encode_message(
            &WebSocketMessage::Text(res!(String::from_utf8(text.payload))), false, 1_024, 1_024,
        ))).await);
        res!(s.write_all(&res!(encode_message(
            &WebSocketMessage::Ping(b"hb".to_vec()), false, 125, 125))).await);
        let pong = res!(read_masked(&mut s).await);
        if pong.opcode != 0xA || pong.payload != b"hb" {
            return Err(err!("Expected a pong echoing 'hb', got {:?}.", pong; Test, Mismatch));
        }
        let close = res!(read_masked(&mut s).await);
        if close.opcode != 0x8 || close.payload[..2] != 1000u16.to_be_bytes() {
            return Err(err!("Expected a normal close, got {:?}.", close; Test, Mismatch));
        }
        res!(s.write_all(&res!(encode_message(
            &WebSocketMessage::Close(Some(WebSocketStatusCode::NormalClosure), None),
            false, 125, 125))).await);
        Ok(())
    }).await);

    let opts = WsConnect::default()
        .origin("https://next.example")
        .field("X-Probe", "one two")
        .tls(Arc::new(cli));
    let url = fmt!("wss://{}:{}{}", HOST, port, TARGET);
    let mut client = res!(WsClient::connect_url(&url, &opts).await);
    res!(client.send_text("over tls").await);
    match res!(client.recv(WAIT).await) {
        Some(WebSocketMessage::Text(txt)) => req!(txt, fmt!("over tls")),
        other => return Err(err!("Expected the echo, got {:?}.", other; Test, Mismatch)),
    }
    // The ping is answered inside recv, which then waits for a data message that never comes.
    match res!(client.recv(Duration::from_millis(300)).await) {
        None        => (),
        Some(msg)   => return Err(err!("Expected nothing after the ping, got {:?}.", msg;
            Test, Mismatch)),
    }
    res!(client.close(WAIT).await);
    if client.is_open() {
        return Err(err!("The client stayed open after its close."; Test, Unexpected));
    }
    join(server).await
}

/// A `ws` URL is plain TCP, the default port is left out of `Host` only when it is the scheme's,
/// and the target goes out unchanged.
#[tokio::test]
async fn test_ws_url_is_plain_and_keeps_its_target_00() -> Outcome<()> {
    let (port, server) = res!(spawn_plain(move |mut s, port| async move {
        let request = res!(accept(&mut s).await);
        if !request.starts_with(&fmt!("GET {} HTTP/1.1\r\n", TARGET))
            || field(&request, "host") != Some(&fmt!("127.0.0.1:{}", port))
        {
            return Err(err!("Unexpected upgrade request:\n{}", request; Test, Mismatch));
        }
        res!(s.write_all(&res!(encode_message(
            &WebSocketMessage::Text(fmt!("plain")), false, 1_024, 1_024))).await);
        Ok(())
    }).await);

    let url = fmt!("ws://127.0.0.1:{}{}", port, TARGET);
    let mut client = res!(WsClient::connect_url(&url, &WsConnect::default()).await);
    match res!(client.recv(WAIT).await) {
        Some(WebSocketMessage::Text(txt)) => req!(txt, fmt!("plain")),
        other => return Err(err!("Expected the greeting, got {:?}.", other; Test, Mismatch)),
    }
    join(server).await
}

/// A server refusing before its 101 is reported with the status and the reason it gave.
#[tokio::test]
async fn test_a_refusal_quotes_its_reason_00() -> Outcome<()> {
    let (srv, cli) = res!(tls_pair());
    let (port, server) = res!(spawn_tls(srv, |mut s, _| async move {
        res!(read_request(&mut s).await);
        res!(s.write_all(b"HTTP/1.1 403 Forbidden\r\nContent-Length: 20\r\n\
            Connection: close\r\n\r\ndevice not approved.").await);
        res!(s.shutdown().await);
        Ok(())
    }).await);

    let url = fmt!("https://{}:{}/api/chat/relay?v=1", HOST, port);
    match WsClient::connect_url(&url, &WsConnect::default().tls(Arc::new(cli))).await {
        Ok(_)   => return Err(err!("A 403 was taken as a handshake."; Test, Unexpected)),
        Err(e)  => {
            let msg = fmt!("{}", e);
            if !msg.contains("403") || !msg.contains("device not approved") {
                return Err(err!("The refusal lacks its status or reason: {}", msg;
                    Test, Mismatch));
            }
        }
    }
    join(server).await
}

/// A `wss` server whose certificate the client does not trust is refused in the TLS handshake.
#[tokio::test]
async fn test_wss_refuses_an_untrusted_certificate_00() -> Outcome<()> {
    let (srv, _) = res!(tls_pair());
    let listener = res!(TcpListener::bind("127.0.0.1:0").await);
    let port = res!(listener.local_addr()).port();
    let server = tokio::spawn(async move {
        let (tcp, _): (TcpStream, _) = res!(listener.accept().await);
        // The client gives up part way, so how this side ends does not matter.
        let _ = TlsAcceptor::from(Arc::new(srv)).accept(tcp).await;
        Ok(())
    });
    let none = ClientConfig::builder()
        .with_root_certificates(RootCertStore::empty())
        .with_no_client_auth();
    let url = fmt!("wss://{}:{}/", HOST, port);
    match WsClient::connect_url(&url, &WsConnect::default().tls(Arc::new(none))).await {
        Ok(_)   => return Err(err!("An untrusted certificate was accepted."; Test, Unexpected)),
        Err(e)  => if !e.tags().contains(&ErrTag::Security) {
            return Err(err!(e, "The refusal is not tagged as the certificate's."; Test, Mismatch));
        },
    }
    join(server).await
}

#[test]
fn test_a_scheme_that_is_not_a_websocket_one_is_refused_00() {
    let rt = match tokio::runtime::Runtime::new() {
        Ok(rt)  => rt,
        Err(e)  => panic!("No runtime: {}", e),
    };
    for url in ["ftp://localhost/", "file:///etc/passwd", "localhost:80/"] {
        assert!(rt.block_on(WsClient::connect_url(url, &WsConnect::default())).is_err(),
            "{} was accepted", url);
    }
}

#[test]
fn test_upgrade_fields_that_could_forge_the_request_are_refused_00() -> Outcome<()> {
    let ok = res!(connect_request_with("h.example", "/p?a=1", None,
        &[(fmt!("Authorization"), fmt!("Bearer abc"))]));
    let text = fmt!("{}", ok.0.header);
    if !text.to_lowercase().contains("authorization: bearer abc") {
        return Err(err!("The field did not reach the request:\n{}", text; Test, Missing));
    }
    for (name, value) in [
        ("Host",                    "elsewhere"),
        ("sec-websocket-key",       "AAAA"),
        ("X-Split",                 "a\r\nHost: elsewhere"),
        ("Bad Name",                "x"),
        ("",                        "x"),
        ("Sec-WebSocket-Extensions","permessage-deflate"),
    ] {
        if connect_request_with("h.example", "/", None, &[(fmt!("{}", name), fmt!("{}", value))])
            .is_ok()
        {
            return Err(err!("The field {:?}: {:?} was accepted.", name, value; Test, Unexpected));
        }
    }
    // The path, host and origin go into the request as written, so none may break a line.
    for (host, path, origin) in [
        ("h.example",               "/p\r\nX-Evil: 1",    None),
        ("h.example",               "/p q",                 None),
        ("h.example\r\nX-Evil: 1",  "/",                    None),
        ("h.example",               "/",                    Some("a\r\nHost: b")),
    ] {
        if connect_request_with(host, path, origin, &[]).is_ok() {
            return Err(err!("The request for {:?} {:?} {:?} was accepted.", host, path, origin;
                Test, Unexpected));
        }
    }
    Ok(())
}
