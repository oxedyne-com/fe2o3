use crate::srv::{
    admin::traffic::{
        self,
        RequestRecord,
    },
    cfg::{
        RedirectMatch,
        RedirectRule,
    },
    constant,
    context::ServerContext,
};

use oxedyne_fe2o3_core::{
    prelude::*,
    error::ErrTag,
    id::ParseId,
    rand::RanDef,
};
use oxedyne_fe2o3_iop_crypto::enc::Encrypter;
use oxedyne_fe2o3_iop_db::api::Database;
use oxedyne_fe2o3_iop_hash::api::Hasher;
use oxedyne_fe2o3_jdat::{
    prelude::*,
    id::{
        IdDat,
        NumIdDat,
    },
};
use oxedyne_fe2o3_net::{
    conc::AsyncReadIterator,
    http::{
        fields::{
            Cookie,
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
            ReadLimits,
        },
        status::HttpStatus,
    },
    id::Sid,
    ws::handler::WebSocketHandler,
};

use std::{
    net::SocketAddr,
    pin::Pin,
    sync::Arc,
    time::{
        Duration,
        Instant,
    },
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

        // Build per-connection read limits from ServerConfig so the
        // reader enforces the configured header / body bounds and the
        // slowloris read deadline. A zero value in the config means
        // "disabled" and maps to `None` in `ReadLimits`.
        let limits = ReadLimits {
            max_header_bytes: if self.cfg.http_max_header_bytes == 0 {
                None
            } else {
                Some(self.cfg.http_max_header_bytes as usize)
            },
            max_body_bytes: if self.cfg.http_max_body_bytes == 0 {
                None
            } else {
                Some(self.cfg.http_max_body_bytes as usize)
            },
            header_read_timeout: if self.cfg.http_header_read_timeout_ms == 0 {
                None
            } else {
                Some(Duration::from_millis(
                    self.cfg.http_header_read_timeout_ms,
                ))
            },
        };

        let mut reader: HttpMessageReader<
            '_,
            { constant::HTTP_DEFAULT_HEADER_CHUNK_SIZE },
            { constant::HTTP_DEFAULT_BODY_CHUNK_SIZE },
            _,
        > = HttpMessageReader::with_limits(Pin::new(&mut read_stream), limits);

        loop {
            let result = reader.next().await;
            // Capture request start time + method/path before the
            // request is consumed by the dispatch chain. Used at
            // the bottom of the loop to emit a TrafficRecord that
            // covers the full handle-to-write duration.
            let req_started_at = Instant::now();
            match result {
                Some(Ok(request)) => {
                    log!(log_level, "{}: Incoming from {:?}:", id, src_addr);
                    request.log(log_get_level!());

                    // Pull method+path out for traffic recording
                    // before the request is moved into the dispatch
                    // chain. Both are cheap clones.
                    let (rec_method, rec_path) = match &request.header.headline {
                        HttpHeadline::Request { method, loc } => (
                            fmt!("{}", method),
                            loc.path.as_string().to_string(),
                        ),
                        _ => (String::new(), String::new()),
                    };

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

                    // Resolve (or issue) the session identifier for this
                    // request. If the client already carries a session
                    // cookie, parse it. Otherwise, when anonymous sessions
                    // are enabled, mint a fresh `Sid`, remember it as a
                    // pending `Set-Cookie` header to attach to the response,
                    // and use it to scope session commands on this request.
                    let raw_sid_str = request.header.fields.get_session_id();
                    let mut issued_cookie: Option<Cookie> = None;
                    let (sid_opt, sid_str) = match raw_sid_str {
                        Some(ref s) => {
                            let parsed = Sid::parse_id(s).ok();
                            (parsed, raw_sid_str.clone())
                        }
                        None => {
                            if self.cfg.allow_anonymous_sessions {
                                let new_sid: Sid = Sid::randef();
                                let s = fmt!("{}", new_sid);
                                issued_cookie = Some(
                                    self.cfg.session_cookie_default(s.clone()),
                                );
                                log!(log_level,
                                    "{}: issuing anonymous session {}.", id, s);
                                (Some(new_sid), Some(s))
                            } else {
                                (None, None)
                            }
                        }
                    };

                    if request.is_websocket_upgrade() {
                        log!(log_level, "Connection upgrading to websocket...");
                        // The raw sid string is enough for the WS handler:
                        // it only needs a stable per-client key prefix, not
                        // the typed numeric identifier.
                        let ws_handler = vhost.ws_handler.clone()
                            .attach_sid(sid_str.clone());
                        let reunited_stream = read_stream.unsplit(write_stream);
                        let vhost_db = self.db_for_vhost(vhost.primary_hostname());
                        return self.handle_websocket(
                            reunited_stream,
                            ws_handler,
                            vhost.ws_syntax.clone(),
                            vhost_db,
                            request,
                            &id,
                        ).await;
                    }

                    let mut response = None;
                    let close_requested = request.get_connection_close();
                    if close_requested {
                        let mut msg = HttpMessage::new_response(HttpStatus::OK);
                        msg.set_connection_close(true);
                        response = Some(msg);
                    }

                    // Per-route rate limit: sensitive URL prefixes
                    // (login forms, admin login) go through a
                    // dedicated, tighter guard so a brute-force
                    // password hammer gets kicked off the login
                    // path faster than a normal browsing session.
                    if let HttpHeadline::Request { ref loc, .. } =
                        request.header.headline
                    {
                        let path_str = loc.path.as_string();
                        if self.cfg.auth_path_prefixes.iter()
                            .any(|p| path_str.starts_with(p.as_str()))
                        {
                            if let Some(admin) = self.admin_state.as_ref() {
                                match admin.auth_guard.check(&src_addr.ip()) {
                                    Ok(d) if d.should_drop() => {
                                        warn!("{}: auth guard dropping {} from {}: {:?}",
                                            id, path_str, src_addr, d);
                                        let mut resp = HttpMessage::respond_with_text(
                                            HttpStatus::TooManyRequests,
                                            "Too many authentication attempts. \
                                            Please wait and try again.",
                                        );
                                        resp.set_connection_close(true);
                                        match resp.write_all(&mut write_stream).await {
                                            Ok(()) => (),
                                            Err(we) => warn!(
                                                "{}: failed to emit 429: {}",
                                                id, we),
                                        }
                                        break;
                                    }
                                    Ok(_) => (),
                                    Err(e) => warn!(
                                        "{}: auth guard error for {}: {}",
                                        id, src_addr, e),
                                }
                            }
                        }
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
                                // Wrap the incoming header fields in an
                                // `Arc` so the downstream handler (and
                                // any API / webhook handler it dispatches
                                // to) can read request headers without
                                // another copy. Cloning here rather than
                                // moving because the POST branch still
                                // needs `request.header.fields` a few
                                // lines down.
                                let req_headers = Arc::new(
                                    request.header.fields.clone(),
                                );
                                let body = request.body;
                                match method {
                                    HttpMethod::GET => {
                                        let vhost_db = self.db_for_vhost(
                                            vhost.primary_hostname(),
                                        );
                                        let result = vhost.web_handler.handle_get(
                                            loc,
                                            response,
                                            body,
                                            req_headers.clone(),
                                            vhost_db,
                                            &sid_opt,
                                            &id,
                                        ).await;
                                        response = res!(result);
                                    }
                                    HttpMethod::POST => {
                                        // Carry Content-Type from the incoming
                                        // request into loc.data so the handler
                                        // can forward it to the upstream.
                                        let mut loc = loc;
                                        if let Some((vals, _)) = request.header.fields.get_all(
                                            &HeaderName::ContentType,
                                        ) {
                                            if let Some(v) = vals.first() {
                                                loc.data.insert(
                                                    dat!("content_type"),
                                                    dat!(v.to_string()),
                                                );
                                            }
                                        }
                                        let vhost_db = self.db_for_vhost(
                                            vhost.primary_hostname(),
                                        );
                                        let result = vhost.web_handler.handle_post(
                                            loc,
                                            response,
                                            body,
                                            req_headers.clone(),
                                            vhost_db,
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
                    let mut rec_status: u16 = 0;
                    let mut rec_bytes: Option<u64> = None;
                    match response {
                        Some(mut msg) => {
                            // Attach the freshly-issued session cookie to
                            // the outgoing response, if any. Works for both
                            // file responses and redirect responses.
                            if let Some(cookie) = issued_cookie.take() {
                                msg = msg.set_cookie(cookie);
                            }
                            // Inject HSTS header if configured, so browsers
                            // remember to use HTTPS for subsequent visits.
                            if self.cfg.hsts_max_age_secs > 0 {
                                msg.header.fields.insert(
                                    HeaderName::StrictTransportSecurity,
                                    HeaderFieldValue::Generic(fmt!(
                                        "max-age={}; includeSubDomains",
                                        self.cfg.hsts_max_age_secs,
                                    )),
                                    None,
                                );
                            }
                            // Baseline security response headers: cheap
                            // defence in depth against content sniffing,
                            // clickjacking and referrer leakage. Each one
                            // is a hard-coded conservative default; tighten
                            // via per-deployment patches if the defaults
                            // bite an integration.
                            if self.cfg.security_headers_enabled {
                                msg.header.fields.insert(
                                    HeaderName::XContentTypeOptions,
                                    HeaderFieldValue::Generic(
                                        "nosniff".to_string()),
                                    None,
                                );
                                msg.header.fields.insert(
                                    HeaderName::XFrameOptions,
                                    HeaderFieldValue::Generic(
                                        "SAMEORIGIN".to_string()),
                                    None,
                                );
                                msg.header.fields.insert(
                                    HeaderName::ReferrerPolicy,
                                    HeaderFieldValue::Generic(
                                        "strict-origin-when-cross-origin"
                                        .to_string()),
                                    None,
                                );
                                // Permissions-Policy: deny every sensor
                                // feature by default. Apps that need any
                                // of camera / microphone / geolocation /
                                // payment can override by tweaking this
                                // string (future per-vhost config).
                                msg.header.fields.insert(
                                    HeaderName::PermissionsPolicy,
                                    HeaderFieldValue::Generic(
                                        "accelerometer=(), camera=(), \
                                        geolocation=(), gyroscope=(), \
                                        magnetometer=(), microphone=(), \
                                        payment=(), usb=()".to_string()),
                                    None,
                                );
                            }
                            // Content-Security-Policy: only emit if the
                            // operator configured a value. An empty CSP
                            // string is not injected at all because the
                            // absence of the header is different from
                            // `default-src 'none'` -- the former is a
                            // no-op, the latter blocks the whole page.
                            if !self.cfg.content_security_policy.is_empty() {
                                msg.header.fields.insert(
                                    HeaderName::ContentSecurityPolicy,
                                    HeaderFieldValue::Generic(
                                        self.cfg.content_security_policy.clone()),
                                    None,
                                );
                            }
                            // Pull the status code out for the
                            // traffic record before the message is
                            // consumed by write_all. HttpStatus is
                            // repr(u16), so a direct cast yields
                            // the wire code (200, 404, etc.).
                            if let HttpHeadline::Response { status } =
                                &msg.header.headline
                            {
                                rec_status = *status as u16;
                            }
                            rec_bytes = Some(msg.body.len() as u64);
                            match msg.write_all(&mut write_stream).await {
                                Ok(()) => (),
                                Err(e) => return Err(err!(e,
                                    "{}: Could not send response.", id;
                                    IO, Network, Wire, Write)),
                            }
                        }
                        None => log!(log_level, " None"),
                    }

                    // Emit a traffic record for this request now that
                    // the response has been fully written. Recording
                    // is a bounded short critical section; failures
                    // are logged but never propagated, since the
                    // request itself succeeded and we do not want
                    // the dashboard to break the data path.
                    if let Some(recorder) = self.traffic.as_ref() {
                        let dur_us = req_started_at.elapsed().as_micros() as u64;
                        let record = RequestRecord {
                            when_ns:        traffic::now_ns(),
                            vhost:          vhost.primary_hostname().to_string(),
                            method:         rec_method,
                            path:           rec_path,
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
                }
                Some(Err(e)) => {
                    // A reader error is often a configured limit
                    // breach (oversized body, oversized header,
                    // slowloris timeout). Translate those into a
                    // proper HTTP status so the client sees a
                    // deliberate rejection instead of a silent
                    // connection drop, then close the connection.
                    let tags = e.tags();
                    let (status, msg) = if tags.contains(&ErrTag::TooBig) {
                        (HttpStatus::ContentTooLarge,
                         "Request exceeds the configured size limit.")
                    } else if tags.contains(&ErrTag::Timeout) {
                        (HttpStatus::RequestTimeout,
                         "Request timed out while reading headers.")
                    } else {
                        warn!("{}: HTTP read error: {}", id, e);
                        return Err(e);
                    };
                    warn!("{}: dropping connection ({}): {}", id, status, e);
                    let mut resp = HttpMessage::respond_with_text(status, msg);
                    resp.set_connection_close(true);
                    match resp.write_all(&mut write_stream).await {
                        Ok(()) => (),
                        Err(we) => warn!("{}: failed to emit {} response: {}",
                            id, status, we),
                    }
                    break;
                }
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
