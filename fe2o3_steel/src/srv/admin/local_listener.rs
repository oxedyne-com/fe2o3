//! Localhost plain-HTTP admin listener.
//!
//! When `ServerConfig::admin_local_port` is non-zero, Steel binds
//! `127.0.0.1:<port>` in addition to the public TLS listener and
//! serves the `/admin/*` routes over plain HTTP. The use case is
//! emergency or operator-only access via SSH tunnel: an operator
//! who has SSH access to the host can forward the local port to
//! their workstation and reach the dashboard without the public
//! TLS chain being healthy. This is invaluable when ACME has
//! broken, when the cert has expired, or when an admin needs to
//! make an emergency password rotation.
//!
//! Security model:
//!
//! - **Bind to loopback only.** The listener binds `127.0.0.1`
//!   unconditionally, never `0.0.0.0`. There is no "expose to
//!   network" knob. Reaching the listener requires shell access
//!   to the host.
//! - **Same auth gate as the public path.** Sessions are decoded
//!   using the same `AdminState::session_enc` cipher; cookies
//!   issued via the local listener work via the public path and
//!   vice versa.
//! - **No vhost dispatch.** The listener has no concept of
//!   vhosts; every request is dispatched to the dashboard handler
//!   regardless of `Host` header. Non-`/admin*` paths return 404.
//! - **Ozone view falls back to the first vhost db.** With no
//!   per-vhost routing, the local listener picks the first vhost
//!   in `ServerContext::vhost_dbs` (in iteration order) as the
//!   default for `/admin/database`. Operators who want a specific
//!   vhost's ozone use the public path.

use crate::srv::{
    admin::{
        handler as admin_handler,
        ozone_view as admin_ozone_view,
        traffic::{
            self,
            RequestRecord,
        },
    },
    context::ServerContext,
};

use oxedyne_fe2o3_core::{
    prelude::*,
    rand::RanDef,
};
use oxedyne_fe2o3_iop_crypto::enc::Encrypter;
use oxedyne_fe2o3_iop_db::api::Database;
use oxedyne_fe2o3_iop_hash::api::Hasher;
use oxedyne_fe2o3_jdat::id::{
    IdDat,
    NumIdDat,
};
use oxedyne_fe2o3_net::{
    conc::AsyncReadIterator,
    http::{
        handler::WebHandler,
        header::{
            HttpHeadline,
            HttpMethod,
        },
        msg::{
            HttpMessage,
            HttpMessageReader,
        },
        status::HttpStatus,
    },
    ws::handler::WebSocketHandler,
};

use std::{
    net::SocketAddr,
    pin::Pin,
    sync::Arc,
    time::Instant,
};

use tokio::net::{
    TcpListener,
    TcpStream,
};

use crate::srv::constant;

impl<
    const UIDL: usize,
    UID:    NumIdDat<UIDL> + 'static,
    ENC:    Encrypter + 'static,
    KH:     Hasher + 'static,
    DB:     Database<UIDL, UID, ENC, KH> + 'static,
    WH:     WebHandler + 'static,
    WSH:    WebSocketHandler + 'static,
>
    ServerContext<UIDL, UID, ENC, KH, DB, WH, WSH>
{
    /// Run the localhost plain-HTTP admin listener accept loop on
    /// `127.0.0.1:<port>`. Returns only on a fatal bind error;
    /// per-connection failures are logged and the loop keeps
    /// running.
    pub async fn run_admin_local_listener(self, port: u16) -> Outcome<()> {
        let addr: SocketAddr = SocketAddr::new(
            std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST),
            port,
        );
        let listener = res!(TcpListener::bind(&addr).await, IO, Network);
        info!("Admin localhost listener bound to {}", addr);

        loop {
            let (stream, src_addr) = match listener.accept().await {
                Ok(pair) => pair,
                Err(e) => {
                    error!(err!(e,
                        "Admin localhost listener: accept failed.";
                        IO, Network));
                    continue;
                },
            };
            let ctx = self.clone();
            tokio::spawn(async move {
                if let Err(e) = ctx.handle_admin_local_connection(
                    stream,
                    src_addr,
                ).await {
                    error!(err!(e,
                        "Admin localhost listener: connection failed.";
                        IO, Network));
                }
            });
        }
    }

    /// Drive a single localhost admin connection: read HTTP
    /// requests, dispatch `/admin/*` to the dashboard handler,
    /// return 404 for everything else, record traffic for each
    /// completed request.
    async fn handle_admin_local_connection(
        self,
        mut stream: TcpStream,
        src_addr:   SocketAddr,
    )
        -> Outcome<()>
    {
        let id = fmt!(
            "AdminLocal|Cx:{}",
            IdDat::<4, u32>::randef(),
        );
        debug!("{}: accepted from {:?}", id, src_addr);

        let admin_state = match self.admin_state.as_ref() {
            Some(s) => s.clone(),
            None => {
                // Listener is bound but the admin runtime was not
                // configured. Send a single 503 and close.
                let mut resp = HttpMessage::respond_with_text(
                    HttpStatus::ServiceUnavailable,
                    "Dashboard runtime not initialised.",
                );
                resp.set_connection_close(true);
                let _ = resp.write_all(&mut stream).await;
                return Ok(());
            },
        };

        // Pick the first registered vhost db as the default for
        // ozone view, if any vhost has one configured. The map is empty
        // while Steel is sealed, so this is `None` until an admin unseals.
        let default_db = {
            let guard = lock_read!(self.vhost_dbs,
                "Reading the vhost database map for the local admin listener.");
            guard.values().next().cloned()
        };

        let (mut read_stream, mut write_stream) = tokio::io::split(&mut stream);
        let mut reader: HttpMessageReader<
            '_,
            { constant::HTTP_DEFAULT_HEADER_CHUNK_SIZE },
            { constant::HTTP_DEFAULT_BODY_CHUNK_SIZE },
            _,
        > = HttpMessageReader::new(Pin::new(&mut read_stream));

        loop {
            let req_started_at = Instant::now();
            let result = reader.next().await;
            match result {
                Some(Ok(request)) => {
                    // The path and the query are parsed apart, so a handler wanting
                    // the query must be handed it: `path` never carries one.
                    let (method, path, query) = match &request.header.headline {
                        HttpHeadline::Request { method, loc } => (
                            method.clone(),
                            loc.path.as_string().to_string(),
                            loc.query.clone(),
                        ),
                        _ => {
                            // Not a request line; bail out.
                            return Ok(());
                        },
                    };
                    let req_headers = Arc::new(request.header.fields.clone());
                    let body = request.body;

                    // Only /admin/* is served via the local
                    // listener. Everything else returns 404 so
                    // operators do not accidentally use this as a
                    // general HTTP server.
                    let response: HttpMessage = if path == "/admin"
                        || path.starts_with("/admin/")
                    {
                        if path == "/admin/database"
                            || path.starts_with("/admin/database/")
                        {
                            match method {
                                HttpMethod::GET => {
                                    match admin_ozone_view::handle_get(
                                        admin_state.as_ref(),
                                        default_db.as_ref(),
                                        &path,
                                        &query,
                                        &req_headers,
                                        &id,
                                    ).await {
                                        Ok(r) => r,
                                        Err(e) => {
                                            error!(e);
                                            HttpMessage::respond_with_text(
                                                HttpStatus::InternalServerError,
                                                "Dashboard error.",
                                            )
                                        },
                                    }
                                },
                                _ => HttpMessage::respond_with_text(
                                    HttpStatus::MethodNotAllowed,
                                    "Only GET is supported on the ozone route.",
                                ),
                            }
                        } else {
                            match method {
                                HttpMethod::GET => {
                                    match admin_handler::handle_get(
                                        admin_state.as_ref(),
                                        &path,
                                        &req_headers,
                                        src_addr,
                                        &id,
                                    ).await {
                                        Ok(r) => r,
                                        Err(e) => {
                                            error!(e);
                                            HttpMessage::respond_with_text(
                                                HttpStatus::InternalServerError,
                                                "Dashboard error.",
                                            )
                                        },
                                    }
                                },
                                HttpMethod::POST => {
                                    match admin_handler::handle_post(
                                        admin_state.as_ref(),
                                        &path,
                                        &body,
                                        &req_headers,
                                        src_addr,
                                        &id,
                                    ).await {
                                        Ok(r) => r,
                                        Err(e) => {
                                            error!(e);
                                            HttpMessage::respond_with_text(
                                                HttpStatus::InternalServerError,
                                                "Dashboard error.",
                                            )
                                        },
                                    }
                                },
                                _ => HttpMessage::respond_with_text(
                                    HttpStatus::MethodNotAllowed,
                                    "Only GET and POST are supported.",
                                ),
                            }
                        }
                    } else {
                        HttpMessage::respond_with_text(
                            HttpStatus::NotFound,
                            "The localhost listener serves /admin only.",
                        )
                    };

                    // Pull rec_status / rec_bytes off the response
                    // before consuming it in write_all.
                    let rec_status: u16 =
                        if let HttpHeadline::Response { status }
                            = &response.header.headline
                        {
                            *status as u16
                        } else {
                            0
                        };
                    let rec_bytes = Some(response.body.len() as u64);

                    if let Err(e) = response.write_all(&mut write_stream).await {
                        return Err(err!(e,
                            "{}: failed to send local admin response.", id;
                            IO, Network, Wire, Write));
                    }

                    // Record traffic for the local listener too.
                    // Vhost field is set to "_admin_local" so
                    // dashboard filters can distinguish local from
                    // public traffic.
                    if let Some(recorder) = self.traffic.as_ref() {
                        let dur_us = req_started_at
                            .elapsed()
                            .as_micros() as u64;
                        let record = RequestRecord {
                            when_ns:        traffic::now_ns(),
                            vhost:          "_admin_local".to_string(),
                            method:         fmt!("{}", method),
                            path,
                            status:         rec_status,
                            peer:           fmt!("{}", src_addr),
                            bytes:          rec_bytes,
                            duration_us:    dur_us,
                        };
                        if let Err(e) = recorder.record(record) {
                            warn!("{}: traffic recorder rejected entry: {}",
                                id, e);
                        }
                    }
                },
                Some(Err(e)) => return Err(e),
                None => break,
            }
        }
        Ok(())
    }
}

