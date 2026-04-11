use crate::srv::{
    cfg::{
        RedirectMatch,
        RedirectRule,
    },
    constant,
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
        fields::{
            HeaderFieldValue,
            HeaderName,
        },
        handler::WebHandler,
        header::{
            HttpHeadline,
            HttpMethod,
        },
        msg::{
            HttpMessageReader,
            HttpMessage,
        },
        status::HttpStatus,
    },
    ws::handler::WebSocketHandler,
};

use std::{
    net::SocketAddr,
    pin::Pin,
};

use tokio::{
    net::TcpStream,
    io::AsyncWriteExt,
};
use tokio_rustls::server::TlsStream;


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
    /// Handle one established TLS connection: perform vhost dispatch based on
    /// the SNI hostname, apply redirect rules, validate the `Host` header,
    /// and drive the selected vhost's HTTP or WebSocket handler.
    pub async fn handle_https(
        self,
        mut stream: TlsStream<TcpStream>,
        sni:        Option<String>,
        src_addr:   SocketAddr,
    )
        -> Outcome<()>
    {
        let id = fmt!("Https|Cx:{}", IdDat::<4, u32>::randef()); // Cx = Connection id.

        // Resolve the vhost once per connection from the SNI. All requests
        // on a single TLS connection are considered to target the same vhost,
        // which matches how every HTTP/1.1 and HTTP/2 client behaves.
        let vhost = self.vhost_for(sni.as_deref());
        let log_level = res!(self.cfg.log_level());
        log!(log_level, "{}: connection from {:?}, sni={:?}, vhost='{}'.",
            id, src_addr, sni, vhost.primary_hostname());

        let (mut read_stream, mut write_stream) = tokio::io::split(&mut stream);

        let mut reader: HttpMessageReader<
            '_,
            { constant::HTTP_DEFAULT_HEADER_CHUNK_SIZE },
            { constant::HTTP_DEFAULT_BODY_CHUNK_SIZE },
            _,
        > = HttpMessageReader::new(Pin::new(&mut read_stream));

        loop {
            let result = reader.next().await;
            match result {
                Some(Ok(request)) => {
                    log!(log_level, "{}: Incoming from {:?}:", id, src_addr);
                    request.log(log_get_level!());

                    if request.is_websocket_upgrade() {
                        log!(log_level, "Connection upgrading to websocket...");
                        let reunited_stream = read_stream.unsplit(write_stream);
                        return self.handle_websocket(
                            reunited_stream,
                            vhost.ws_handler.clone(),
                            vhost.ws_syntax.clone(),
                            request,
                            &id,
                        ).await;
                    }

                    // Validate the Host header against the vhost hostnames.
                    // A mismatch means an SNI/Host disagreement, which is a
                    // misdirected client; we return 421.
                    if let Some(HeaderFieldValue::Generic(host_hdr)) =
                        request.header.fields.get_one(&HeaderName::Host)
                    {
                        if !vhost.accepts_host(host_hdr) {
                            warn!("{}: Host header '{}' does not match vhost '{}' \
                                (hostnames={:?}); returning 421 Misdirected Request.",
                                id, host_hdr, vhost.primary_hostname(), vhost.hostnames);
                            let mut resp = HttpMessage::respond_with_text(
                                HttpStatus::MisdirectedRequest,
                                "Misdirected request: Host header does not match SNI.",
                            );
                            resp.set_connection_close(true);
                            match resp.write_all(&mut write_stream).await {
                                Ok(()) => (),
                                Err(e) => return Err(err!(e,
                                    "{}: Could not send 421 response.", id;
                                    IO, Network, Wire, Write)),
                            }
                            break;
                        }
                    }

                    let sid_opt = Self::get_session_id(&request, &src_addr);

                    let mut response = None;
                    let close_requested = request.get_connection_close();
                    if close_requested {
                        let mut msg = HttpMessage::new_response(HttpStatus::OK);
                        msg.set_connection_close(true);
                        response = Some(msg);
                    }

                    match request.header.headline.clone() {
                        HttpHeadline::Request { method, loc } => {
                            // Redirect rules fire before the file router.
                            let request_uri = loc.path.as_string().to_string();
                            if let Some(rule) = Self::match_redirect(
                                &vhost.redirects,
                                &request_uri,
                            ) {
                                let target = rule.resolve_target(&request_uri);
                                log!(log_level,
                                    "{}: redirect {} {} -> {} ({})",
                                    id, rule.status, request_uri, target,
                                    match rule.match_kind {
                                        RedirectMatch::Exact    => "exact",
                                        RedirectMatch::Prefix   => "prefix",
                                        RedirectMatch::All      => "all",
                                    });
                                let status = match rule.status {
                                    301 => HttpStatus::MovedPermanently,
                                    302 => HttpStatus::Found,
                                    303 => HttpStatus::SeeOther,
                                    307 => HttpStatus::TemporaryRedirect,
                                    308 => HttpStatus::PermanentRedirect,
                                    _   => HttpStatus::MovedPermanently,
                                };
                                let resp = HttpMessage::new_response(status)
                                    .with_field(
                                        HeaderName::Location,
                                        HeaderFieldValue::Generic(target),
                                    );
                                response = Some(resp);
                            } else {
                                let body = request.body;
                                match method {
                                    HttpMethod::GET => {
                                        let result = vhost.web_handler.handle_get(
                                            loc,
                                            response,
                                            body,
                                            self.db.clone(),
                                            &sid_opt,
                                            &id,
                                        ).await;
                                        response = res!(result);
                                    }
                                    _ => fault!("{}: Unsupported HTTP request method '{}'.",
                                        id, method),
                                }
                            }
                        }
                        _ => fault!("{}: Unsupported HTTP '{:?}'.", id, request.header.headline),
                    }

                    log!(log_level, "Outgoing HTTPS message:");
                    match response {
                        Some(msg) => {
                            match msg.write_all(&mut write_stream).await {
                                Ok(()) => (),
                                Err(e) => return Err(err!(e,
                                    "{}: Could not send response.", id;
                                    IO, Network, Wire, Write)),
                            }
                        }
                        None => log!(log_level, " None"),
                    }
                }
                Some(Err(e)) => return Err(e),
                None => {
                    break;
                }
            }
        }

        // Gracefully close the TLS connection.
        let reunited_stream = read_stream.unsplit(write_stream);
        let result = reunited_stream.shutdown().await;
        if let Err(e) = result {
            error!(e.into());
        }
        log!(log_level, "{}: Connection with {:?} closed.", id, src_addr);

        Ok(())
    }

    /// Find the first redirect rule, if any, that matches the given request
    /// path. Rules are tried in declaration order.
    fn match_redirect<'a>(
        rules:          &'a [RedirectRule],
        request_path:   &str,
    )
        -> Option<&'a RedirectRule>
    {
        for rule in rules {
            if rule.matches(request_path) {
                return Some(rule);
            }
        }
        None
    }
}
