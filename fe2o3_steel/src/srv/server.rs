use crate::srv::{
    cert::Certificate,
    context::{
        Protocol,
        ServerContext,
    },
};

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_iop_crypto::enc::Encrypter;
use oxedyne_fe2o3_iop_db::api::Database;
use oxedyne_fe2o3_iop_hash::api::Hasher;
use oxedyne_fe2o3_jdat::id::NumIdDat;
use oxedyne_fe2o3_net::{
    http::handler::WebHandler,
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


/// The Steel TCP/TLS server, wrapping a `ServerContext`.
pub struct Server<
    const UIDL: usize,
    UID:    NumIdDat<UIDL> + 'static,
    ENC:    Encrypter,
    KH:     Hasher,
    DB:     Database<UIDL, UID, ENC, KH>,
    WH:     WebHandler,
    WSH:    WebSocketHandler,
> {
    pub context: ServerContext<UIDL, UID, ENC, KH, DB, WH, WSH>,
}

impl<
    const UIDL: usize,
    UID:    NumIdDat<UIDL> + 'static,
    ENC:    Encrypter + 'static,
    KH:     Hasher + 'static,
    DB:     Database<UIDL, UID, ENC, KH> + 'static,
    WH:     WebHandler + 'static,
    WSH:    WebSocketHandler + 'static,
>
    Server<UIDL, UID, ENC, KH, DB, WH, WSH>
{
    /// Construct a new server from a pre-built context.
    pub fn new(
        context: ServerContext<UIDL, UID, ENC, KH, DB, WH, WSH>,
    )
        -> Self
    {
        Self { context }
    }

    /// Bind the configured address and port, perform TLS + vhost dispatch
    /// in an accept loop, and hand each accepted connection off to the
    /// `handle_https` method on a fresh Tokio task.
    pub async fn start(&self) -> Outcome<()> {

        let dev_mode = match &self.context.protocol {
            Protocol::Web { dev_mode, .. } => *dev_mode,
        };

        let loaded = res!(Certificate::load(
            &self.context.cfg,
            &self.context.root,
            dev_mode,
        ));

        // If ACME is enabled, spawn the renewer task. It drives the
        // initial issuance (if the cache is empty) and then loops with
        // a 24-hour tick, re-issuing whenever the cached cert is older
        // than the renewal threshold.
        if let Some(renewer) = loaded.acme_renewer {
            tokio::spawn(async move {
                if let Err(e) = renewer.run_forever().await {
                    error!(err!(e,
                        "ACME renewer task exited.";
                        Init, Network));
                }
            });
        }

        let tls_acceptor = TlsAcceptor::from(Arc::new(loaded.server_config));

        // Build the bind address from the (now honoured) server_cfg.
        let addr: SocketAddr = {
            let ip: std::net::IpAddr = match self.context.cfg.server_address.parse() {
                Ok(ip) => ip,
                Err(e) => return Err(err!(e,
                    "Invalid server_address '{}' in config.",
                    self.context.cfg.server_address;
                    Invalid, Input, Network)),
            };
            SocketAddr::new(ip, self.context.cfg.server_port_tcp)
        };
        let listener = res!(TcpListener::bind(&addr).await, IO, Network);
        info!("Listening on: {}", addr);

        loop {
            let (mut stream, src_addr) = match listener.accept().await {
                Ok(pair) => pair,
                Err(e) => {
                    error!(err!(e, "TCP connection aborted."; IO, Network));
                    continue;
                }
            };

            // Peek at first bytes to detect TLS handshake. Non-TLS requests
            // receive a 308 to redirect the caller to HTTPS.
            let mut peek_buf = [0u8; 5];
            match stream.peek(&mut peek_buf).await {
                Ok(n) if n >= 5 && peek_buf[0] == 0x16 && peek_buf[1] == 0x03 => {
                    match tls_acceptor.accept(stream).await {
                        Ok(tls_stream) => {
                            // Extract SNI now, before we hand ownership of
                            // the stream to the handler task.
                            let sni = tls_stream.get_ref().1.server_name()
                                .map(|s| s.to_string());
                            let context_clone = self.context.clone();
                            match &self.context.protocol {
                                Protocol::Web { .. } => {
                                    tokio::spawn(async move {
                                        if let Err(e) = context_clone.handle_https(
                                            tls_stream,
                                            sni,
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
                            error!(err!(e,
                                "TLS handshake aborted.";
                                IO, Network, Init));
                            continue;
                        }
                    }
                }
                _ => {
                    // Non-TLS connection: redirect to HTTPS.
                    let port = self.context.cfg.server_port_tcp;
                    let body = fmt!(
                        "This server requires HTTPS. Please use https://<host>:{} instead.",
                        port,
                    );
                    let response = fmt!(
                        "HTTP/1.1 308 Permanent Redirect\r\n\
                        Location: https://{}:{}\r\n\
                        Connection: close\r\n\
                        Content-Type: text/plain\r\n\
                        Content-Length: {}\r\n\
                        \r\n\
                        {}",
                        self.context.cfg.server_address,
                        port,
                        body.len(),
                        body,
                    );
                    if let Err(e) = stream.write_all(response.as_bytes()).await {
                        error!(err!(e,
                            "Failed to send HTTPS redirect.";
                            IO, Network, Write));
                    }
                    continue;
                }
            }
        }
    }
}
