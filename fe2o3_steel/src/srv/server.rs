use crate::srv::{
    cert::Certificate,
    context::{
        Protocol,
        ServerContext,
    },
};

use oxedize_fe2o3_core::prelude::*;
use oxedize_fe2o3_iop_crypto::enc::Encrypter;
use oxedize_fe2o3_iop_db::api::Database;
use oxedize_fe2o3_iop_hash::api::Hasher;
use oxedize_fe2o3_jdat::id::NumIdDat;
use oxedize_fe2o3_net::{
    http::handler::WebHandler,
    //smtp::handler::EmailHandler,
    ws::handler::WebSocketHandler,
};

use std::{
    net::SocketAddr,
    sync::Arc,
};

use tokio::{
    net::TcpListener,
    io::AsyncWriteExt,
};
use tokio_rustls::TlsAcceptor;


pub struct Server<
    const UIDL: usize,
    UID:    NumIdDat<UIDL> + 'static,
    ENC:    Encrypter,        // Symmetric encryption of database.
    KH:     Hasher,           // Hashes database keys.
    DB:     Database<UIDL, UID, ENC, KH>, 
    //EH:     EmailHandler,
    WH:     WebHandler,
    WSH:    WebSocketHandler,
> {
    //pub context: ServerContext<UIDL, UID, ENC, KH, DB, EH, WH, WSH>,
    pub context: ServerContext<UIDL, UID, ENC, KH, DB, WH, WSH>,
}

impl<
    const UIDL: usize,
    UID:    NumIdDat<UIDL> + 'static,
    ENC:    Encrypter + 'static,
    KH:     Hasher + 'static,
    DB:     Database<UIDL, UID, ENC, KH> + 'static, 
    //EH:     EmailHandler + 'static,
    WH:     WebHandler + 'static,
    WSH:    WebSocketHandler + 'static,
>
    //Server<UIDL, UID, ENC, KH, DB, EH, WH, WSH>
    Server<UIDL, UID, ENC, KH, DB, WH, WSH>
{
    pub fn new(
        //context: ServerContext<UIDL, UID, ENC, KH, DB, EH, WH, WSH>,
        context: ServerContext<UIDL, UID, ENC, KH, DB, WH, WSH>,
    )
        -> Self
    {
        Self { context }
    }

    pub async fn start(&self) -> Outcome<()> {

        let dev_mode = match &self.context.protocol {
            Protocol::Web { dev_mode, .. } => *dev_mode,
        };
    
        let server_config = res!(Certificate::load(
            &self.context.cfg,
            &self.context.root,
            dev_mode,
        ));
    
        let tls_acceptor = TlsAcceptor::from(Arc::new(server_config));
    
        let addr = SocketAddr::from(([0, 0, 0, 0], 8443));
        let result = TcpListener::bind(&addr).await;
        let listener = res!(result, IO, Network);
        info!("Listening on: {}", addr);
    
        loop {
            let result = listener.accept().await;
            let (mut stream, src_addr) = match result {
                Ok((stream, src_addr)) => (stream, src_addr),
                Err(e) => {
                    error!(err!(e, "TCP connection aborted."; IO, Network));
                    continue;
                }
            };
    
            // Peek at first bytes to detect TLS handshake.
            let mut peek_buf = [0u8; 5];
            match stream.peek(&mut peek_buf).await {
                Ok(n) if n >= 5 && peek_buf[0] == 0x16 && peek_buf[1] == 0x03 => {
                    // Valid TLS handshake - proceed with TLS.
                    match tls_acceptor.accept(stream).await {
                        Ok(tls_stream) => {
                            let context_clone = self.context.clone();
                            match self.context.protocol.clone() {
                                Protocol::Web { web_handler, ws_handler, ws_syntax, .. } => {
                                    tokio::spawn(async move {
                                        if let Err(e) = context_clone.handle_https(
                                            tls_stream,
                                            web_handler,
                                            ws_handler,
                                            ws_syntax,
                                            src_addr,
                                        ).await {
                                            error!(err!(e,
                                                "Error handling HTTPS connection.";
                                                IO, Network));
                                        }
                                    });
                                }
                            }
                        }
                        Err(e) => {
                            error!(err!(e, "TLS handshake aborted."; IO, Network, Init));
                            continue;
                        }
                    }
                },
                _ => {
                    // Non-TLS connection - send redirect response.
                    let response = "HTTP/1.1 308 Permanent Redirect\r\n\
                        Location: https://localhost:8443\r\n\
                        Connection: close\r\n\
                        Content-Type: text/plain\r\n\
                        Content-Length: 63\r\n\
                        \r\n\
                        This server requires HTTPS. Please use https://localhost:8443 instead";
    
                    if let Err(e) = stream.write_all(response.as_bytes()).await {
                        error!(err!(e, "Failed to send HTTPS redirect"; IO, Network, Write));
                    }
                    continue;
                }
            }
        }
    }

}
