use oxedize_fe2o3_steel::srv::{
    constant,
    context,
    id,
    ws::syntax::WebSocketSyntax,
};

use oxedize_fe2o3_core::{
    prelude::*,
};
use oxedize_fe2o3_crypto::enc::EncryptionScheme;
use oxedize_fe2o3_hash::{
    csum::ChecksumScheme,
    hash::HashScheme,
};
use oxedize_fe2o3_jdat::version::SemVer;
use oxedize_fe2o3_net::{
    conc::AsyncReadIterator,
    http::msg::{
        //AsyncReadIterator,
        HttpMessage,
        HttpMessageReader,
    },
    ws::{
        self,
        WebSocket,
        WebSocketMessage,
        handler::WebSocketSinkHandler,
        status::WebSocketStatusCode,
    },
};
use oxedize_fe2o3_o3db_sync::O3db;

use std::{
    fs::File,
    io::BufReader,
    path::Path,
    pin::Pin,
    sync::{
        Arc,
        RwLock,
    },
    thread,
    time::Duration,
};

//use rustls_pki_types;
use tokio::{
    self,
    io::{
        AsyncWriteExt,
    },
    net::TcpStream,
};
use tokio_rustls::{
    client::TlsStream,
    rustls::{
        self,
        client::danger::ServerCertVerifier,
        ClientConfig,
        RootCertStore,
    },
    TlsConnector,
};

fn load_certs() -> Outcome<RootCertStore> {
    let mut root_store = RootCertStore::empty();
    let home = res!(std::env::var("HOME"));
    let path = Path::new(home).join("usr/code/web/apps/test/tls/fullchain.pem");
    let cert_file = res!(File::open(path));
    let mut reader = BufReader::new(cert_file);
    let certs = res!(rustls_pemfile::certs(&mut reader).collect::<Result<Vec<_>, _>>());
    for cert in certs {
        res!(root_store.add(cert));
    }
    Ok(root_store)
}

pub async fn new_stream(host: &str, port: u16) -> Outcome<TlsStream<TcpStream>> {
    //let mut root_cert_store = RootCertStore::empty();
    //root_cert_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let root_cert_store = res!(load_certs());
    let config = ClientConfig::builder()
        .with_root_certificates(root_cert_store)
        .with_no_client_auth();

    let connector = TlsConnector::from(Arc::new(config));
    let host_clone = host.to_string();
    let dnsname = res!(rustls::pki_types::ServerName::try_from(host_clone));
    
    let result = TcpStream::connect((host, port)).await;
    let stream = res!(result);
    let result = connector.connect(dnsname, stream).await;
    let stream = res!(result);
    Ok(stream)
}

pub async fn test_client(filter: &'static str) -> Outcome<()> {

    let host = "localhost";
    let port = 8443;

    match filter {
        "all" | "firefox" | "get" => {
            let result = new_stream(host, port).await;
            let mut stream = res!(result);

            //tokio::time::sleep(tokio::time::Duration::from_secs(10)).await;

            let request_string = fmt!("GET / HTTP/1.1\r\n\
                                  Host: {}\r\n\
                                  User-Agent: Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:93.0) Gecko/20100101 Firefox/93.0\r\n\
                                  Accept: text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,*/*;q=0.8\r\n\
                                  Accept-Language: en-US,en;q=0.5\r\n\
                                  Accept-Encoding: gzip, deflate, br\r\n\
                                  Connection: keep-alive\r\n\
                                  Upgrade-Insecure-Requests: 1\r\n\
                                  Sec-Fetch-Dest: document\r\n\
                                  Sec-Fetch-Mode: navigate\r\n\
                                  Sec-Fetch-Site: none\r\n\
                                  Sec-Fetch-User: ?1\r\n\
                                  Cache-Control: max-age=0\r\n\r\n",
                                  host);

            debug!("Writing to stream now...");
            for line in request_string.lines() {
                debug!(" {}", line);
            }
            let result = stream.write_all(request_string.as_bytes()).await;
            res!(result);

            loop {
                trace!("Entered response read loop");
                let result = HttpMessage::read::<
                    { constant::HTTP_DEFAULT_HEADER_CHUNK_SIZE },
                    { constant::HTTP_DEFAULT_BODY_CHUNK_SIZE },
                    _,
                >(Pin::new(&mut stream), &Vec::new(), Some(false)).await;

                match result {
                    Ok((Some(response), remnant)) => {
                        trace!("Incoming Response:");
                        response.log(log_get_level!());
                        trace!("##### Remnant is {} bytes", remnant.len());
                    }
                    Ok((None, _)) => break,
                    Err(e) => return Err(e),
                }
            }
        }
        _ => (),
    }

    match filter {
        "all" | "firefox" | "get" | "iter" => {
            let result = new_stream(host, port).await;
            let mut stream = res!(result);

            //tokio::time::sleep(tokio::time::Duration::from_secs(10)).await;

            let request_string = fmt!("GET / HTTP/1.1\r\n\
                                  Host: {}\r\n\
                                  User-Agent: Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:93.0) Gecko/20100101 Firefox/93.0\r\n\
                                  Accept: text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,*/*;q=0.8\r\n\
                                  Accept-Language: en-US,en;q=0.5\r\n\
                                  Accept-Encoding: gzip, deflate, br\r\n\
                                  Connection: keep-alive\r\n\
                                  Upgrade-Insecure-Requests: 1\r\n\
                                  Sec-Fetch-Dest: document\r\n\
                                  Sec-Fetch-Mode: navigate\r\n\
                                  Sec-Fetch-Site: none\r\n\
                                  Sec-Fetch-User: ?1\r\n\
                                  Cache-Control: max-age=0\r\n\r\n",
                                  host);

            debug!("Writing to stream now...");
            for line in request_string.lines() {
                debug!(" {}", line);
            }
            let result = stream.write_all(request_string.as_bytes()).await;
            res!(result);

            let mut reader: HttpMessageReader<
                '_,
                { constant::HTTP_DEFAULT_HEADER_CHUNK_SIZE },
                { constant::HTTP_DEFAULT_BODY_CHUNK_SIZE },
                _,
            > = HttpMessageReader::new(Pin::new(&mut stream));

            while let Some(result) = reader.next().await {
                match result {
                    Ok(response) => {
                        trace!("Incoming Response:");
                        response.log(log_get_level!());
                    }
                    Err(e) => {
                        return Err(e);
                    }
                }
            }

        }
        _ => (),
    }

    match filter {
        "all" | "websocket" | "text" => {
            let result = new_stream(host, port).await;
            let mut stream = res!(result);
            
            // Create the websocket and connect to the server, using a standard HTTP request
            // message.
            let result = context::new_ws_no_db(
                &mut stream,
                WebSocketSinkHandler,
            );
            let mut ws = res!(result);
            let (request, key) = res!(ws::connect_request(host));
            let result = ws.connect(request, Some(key)).await;
            res!(result);
            
            // Send a websocket text message..
            let txt = fmt!("echo (str|\"Hello, WebSocket!\")");
            let msg = WebSocketMessage::Text(txt.clone());
            let result = ws.send(&msg).await;
            res!(result);
            test!("Sent text message: '{}'", txt);
            test!("Sent text message as bytes: {:02x?}", txt.clone().as_bytes().to_vec());
            
            // ..and wait for the echo.
            match ws.read().await {
                Ok(Some(WebSocketMessage::Text(txt2))) => {
                    test!("Received text message: '{}'", txt2);
                    req!(txt2, txt);
                }
                Err(e) => return Err(err!(e, "Error receiving message."; IO, Network, Read, Wire)),
                Ok(None) => return Err(err!(
                    "The server has closed the connection unexpectedly.";
                IO, Network, Read, Wire)),
                Ok(Some(msg)) => return Err(err!(
                    "Expecting text message from server, received: {:?}", msg;
                IO, Network, Unexpected, Read)),
            }
            
            // Send a small websocket binary message..
            let byts = vec![0xaa, 0x12, 0x34];
            let msg = WebSocketMessage::Binary(byts.clone());
            let result = ws.send(&msg).await;
            res!(result);
            test!("Sent binary message: {:02x?}", byts);

            // ..and wait for the echo.
            match ws.read().await {
                Ok(Some(WebSocketMessage::Binary(byts2))) => {
                    test!("Received binary message: {:02x?}", byts2);
                    req!(byts2, byts);
                }
                Err(e) => return Err(err!(e, "Error receiving message."; IO, Network, Read, Wire)),
                Ok(None) => return Err(err!(
                    "The server has closed the connection unexpectedly.";
                IO, Network, Read, Wire)),
                Ok(Some(msg)) => return Err(err!(
                    "Expecting binary message from server, received: {:?}", msg;
                IO, Network, Unexpected, Read)),
            }
            
            // Send a large websocket binary message..
            let byts = vec![
                0xef, 0xd3, 0x1f, 0x85, 0xfe, 0xd2, 0x36, 0xd4, 0xbd, 0x90,
                0xae, 0x09, 0x31, 0x59, 0x3f, 0xe6, 0x96, 0x6a, 0x84, 0x11,
                0xac, 0x0f, 0x57, 0x5a, 0xbf, 0x3f, 0xd3, 0x1b, 0x06, 0x7e,
                0xcd, 0x86, 0x1a, 0x84, 0x29, 0xbd, 0x24,
            ];
            let msg = WebSocketMessage::Binary(byts.clone());
            let result = ws.send(&msg).await;
            res!(result);
            test!("Sent binary message: {:02x?}", byts);

            // ..and wait for the echo.
            match ws.read().await {
                Ok(Some(WebSocketMessage::Binary(byts2))) => {
                    test!("Received binary message: {:02x?}", byts2);
                    req!(byts2, byts);
                }
                Err(e) => return Err(err!(e, "Error receiving message."; IO, Network, Read, Wire)),
                Ok(None) => return Err(err!(
                    "The server has closed the connection unexpectedly.";
                IO, Network, Read, Wire)),
                Ok(Some(msg)) => return Err(err!(
                    "Expecting binary message from server, received: {:?}", msg;
                IO, Network, Unexpected, Read)),
            }
            
            // Send some data as text to store..
            let txt = r#"insert (t2|[(str|a/b/c),{(str|name):(str|jane),(str|age):(u8|21)}])"#;
            let msg = WebSocketMessage::Text(txt.to_string());
            let result = ws.send(&msg).await;
            res!(result);
            test!("Sent text message: '{}'", txt);
            test!("Sent text message as bytes: {:02x?}", txt.clone().as_bytes().to_vec());

            // ..and wait for the reply.
            match ws.read().await {
                Ok(Some(WebSocketMessage::Text(txt2))) => {
                    test!("Received text message: '{}'", txt2);
                }
                Err(e) => return Err(err!(e, "Error receiving message."; IO, Network, Read, Wire)),
                Ok(None) => return Err(err!(
                    "The server has closed the connection unexpectedly.";
                IO, Network, Read, Wire)),
                Ok(Some(msg)) => return Err(err!(
                    "Expecting binary message from server, received: {:?}", msg;
                IO, Network, Unexpected, Read)),
            }

            thread::sleep(Duration::from_secs(1));

            // Retrieve data as text..
            let txt = fmt!("get_data (str|a/b/c)");
            let msg = WebSocketMessage::Text(txt.clone());
            let result = ws.send(&msg).await;
            res!(result);
            test!("Sent text message: '{}'", txt);
            test!("Sent text message as bytes: {:02x?}", txt.clone().as_bytes().to_vec());

            // ..and wait for the reply.
            match ws.read().await {
                Ok(Some(WebSocketMessage::Text(txt2))) => {
                    test!("Received text message: '{}'", txt2);
                }
                Err(e) => return Err(err!(e, "Error receiving message."; IO, Network, Read, Wire)),
                Ok(None) => return Err(err!(
                    "The server has closed the connection unexpectedly.";
                IO, Network, Read, Wire)),
                Ok(Some(msg)) => return Err(err!(
                    "Expecting binary message from server, received: {:?}", msg;
                IO, Network, Unexpected, Read)),
            }
            
            // Now listen to the websocket..
            let listen_time_limit = tokio::time::Duration::from_secs(60);

            let ws_syntax = res!(WebSocketSyntax::new(
                "steel_ws",
                &SemVer::new(0, 1, 0),
                "Steel Websocket Test Client",
            ));

            tokio::time::timeout(listen_time_limit, async {
                ws.listen(
                    res!(context::no_db()),
                    ws_syntax,
                    Some(30),
                    0,
                    &fmt!("client"),
                ).await
            }).await;

            // And finally, close it.
            test!("Closing websocket now.");
            let result = ws.close(
                Some(WebSocketStatusCode::NormalClosure),
                Some(fmt!("Closing the connection")),
            ).await;
            res!(result);
        }
        _ => (),
    }
            
    Ok(())
}

