use crate::srv::{
    admin::traffic::{
        self,
        RequestRecord,
    },
    cfg::{
        ProxyRoute,
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
    io::{
        AsyncRead,
        AsyncReadExt,
        AsyncWrite,
        AsyncWriteExt,
    },
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
                        // Check proxy routes first — if a proxy route
                        // matches, tunnel the WebSocket to the upstream
                        // instead of handling it with Steel's own WS
                        // handler.  This allows proxied applications
                        // that use WebSocket (e.g. web terminals) to
                        // work through the reverse proxy.
                        if !vhost.proxy_routes.is_empty() {
                            if let HttpHeadline::Request { ref loc, .. } =
                                request.header.headline
                            {
                                let proxy_path = loc.path.as_string().to_string();
                                if let Some(proxy_route) = vhost.proxy_routes.iter()
                                    .filter(|r| proxy_path.starts_with(&r.path_prefix))
                                    .max_by_key(|r| r.path_prefix.len())
                                {
                                    log!(log_level,
                                        "{}: proxy ws {} -> {}:{}{}",
                                        id, proxy_path,
                                        proxy_route.upstream_host,
                                        proxy_route.upstream_port,
                                        if proxy_route.upstream_tls { " (tls)" } else { "" },
                                    );
                                    let reunited = read_stream.unsplit(write_stream);
                                    return self.handle_proxy_websocket(
                                        reunited,
                                        request,
                                        proxy_route,
                                        src_addr,
                                        &id,
                                    ).await;
                                }
                            }
                        }
                        // ── Red chat WS dispatch ───────────────────
                        // If the request path is /chat and the vhost
                        // ── Terminal WS dispatch ──────────────────
                        // If the request path starts with /term/,
                        // route to the terminal I/O bridge instead
                        // of the normal text-protocol WS handler.
                        // This allows binary terminal data to flow
                        // over a separate WS channel.
                        if let HttpHeadline::Request { ref loc, .. } =
                            request.header.headline
                        {
                            let req_path = loc.path.as_string();
                            if req_path.starts_with("/term/") {
                                let session_name = req_path
                                    .strip_prefix("/term/")
                                    .unwrap_or("")
                                    .to_string();
                                if session_name.is_empty() {
                                    log!(log_level,
                                        "{}: terminal WS missing session name.", id);
                                    let mut resp = HttpMessage::respond_with_text(
                                        HttpStatus::BadRequest,
                                        "Missing terminal session name.",
                                    );
                                    resp.set_connection_close(true);
                                    let _ = resp.write_all(&mut write_stream).await;
                                    break;
                                }
                                log!(log_level,
                                    "{}: terminal ws -> '{}'", id, session_name);
                                let reunited = read_stream.unsplit(write_stream);
                                return crate::srv::ws::term::handle_terminal_websocket::<
                                    UIDL, UID, ENC, KH, DB, _,
                                >(
                                    reunited,
                                    session_name,
                                    request,
                                    &id,
                                ).await;
                            }
                        }
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
                                // ── Reverse proxy routes ──────────────
                                // Checked after redirects, before static
                                // files and API routes.  Longest matching
                                // prefix wins.  WebSocket upgrades are
                                // tunnelled; regular HTTP is streamed.
                                if !vhost.proxy_routes.is_empty() {
                                    let proxy_path = loc.path.as_string().to_string();
                                    if let Some(proxy_route) = vhost.proxy_routes.iter()
                                        .filter(|r| proxy_path.starts_with(&r.path_prefix))
                                        .max_by_key(|r| r.path_prefix.len())
                                    {
                                        log!(log_level,
                                            "{}: proxy {} -> {}:{}{}",
                                            id, proxy_path,
                                            proxy_route.upstream_host,
                                            proxy_route.upstream_port,
                                            if proxy_route.upstream_tls { " (tls)" } else { "" },
                                        );

                                        if request.is_websocket_upgrade() {
                                            let reunited = read_stream.unsplit(write_stream);
                                            return self.handle_proxy_websocket(
                                                reunited,
                                                request,
                                                proxy_route,
                                                src_addr,
                                                &id,
                                            ).await;
                                        }

                                        let proxy_result = self.handle_proxy_http(
                                            request,
                                            proxy_route,
                                            &mut write_stream,
                                            src_addr,
                                            &id,
                                        ).await;

                                        let (proxy_status, proxy_bytes) = match proxy_result {
                                            Ok(sb) => sb,
                                            Err(e) => {
                                                warn!("{}: proxy error: {}", id, e);
                                                let mut resp = HttpMessage::respond_with_text(
                                                    HttpStatus::BadGateway,
                                                    "Bad Gateway: upstream proxy error.",
                                                );
                                                resp.set_connection_close(true);
                                                match resp.write_all(&mut write_stream).await {
                                                    Ok(()) => (),
                                                    Err(we) => warn!(
                                                        "{}: failed to emit 502: {}",
                                                        id, we),
                                                }
                                                break;
                                            }
                                        };

                                        // Record traffic for the proxied request.
                                        if let Some(recorder) = self.traffic.as_ref() {
                                            let dur_us = req_started_at
                                                .elapsed().as_micros() as u64;
                                            let record = RequestRecord {
                                                when_ns:       traffic::now_ns(),
                                                vhost:         vhost.primary_hostname()
                                                                .to_string(),
                                                method:        rec_method.clone(),
                                                path:          rec_path.clone(),
                                                status:        proxy_status,
                                                peer:          fmt!("{}", src_addr),
                                                bytes:         proxy_bytes,
                                                duration_us:   dur_us,
                                            };
                                            if let Err(e) = recorder.record(record) {
                                                warn!("{}: traffic recorder rejected entry: {}",
                                                    id, e);
                                            }
                                        }

                                        // Proxied responses are written
                                        // directly to the stream.  Close
                                        // the connection because the
                                        // upstream uses Connection: close
                                        // and we cannot guarantee
                                        // keep-alive semantics.
                                        break;
                                    }
                                }

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
                                    // A `HEAD` asks what the `GET` would answer, minus the
                                    // answer. RFC 9110 9.3.2 says the fields must be the ones
                                    // the `GET` would carry, so the only honest way to produce
                                    // them is to do the `GET` and withhold the body at the
                                    // wire. Handled here rather than left to fall through:
                                    // unhandled, it reached no branch at all, no response was
                                    // ever built, and the caller sat until the read timed out
                                    // -- so `curl -I`, and every uptime monitor that speaks
                                    // `HEAD` first, saw a 408 from a server that was fine.
                                    HttpMethod::GET | HttpMethod::HEAD => {
                                        let head_only = method == HttpMethod::HEAD;
                                        // Told to the handler as well as applied at the wire,
                                        // so a `GET` path that keeps a tally can decline to
                                        // count a request that asked for no prose.
                                        let mut loc = loc;
                                        if head_only {
                                            loc.data.insert(
                                                dat!("head_only"),
                                                dat!(true),
                                            );
                                        }
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
                                            src_addr,
                                            &id,
                                        ).await;
                                        response = res!(result);
                                        if head_only {
                                            response = response.map(|r| r.head_only());
                                        }
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
                                            src_addr,
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

    /// Forward a regular (non-WebSocket) HTTP request to the upstream
    /// proxy target and stream the response back to the client.
    ///
    /// The request is forwarded with `Connection: close` to the
    /// upstream so the response termination is unambiguous.  The
    /// response is streamed in chunks — not buffered — so SSE and
    /// other streaming responses work correctly.
    ///
    /// Returns `(status_code, body_byte_count)` for traffic recording.
    async fn handle_proxy_http<W>(
        &self,
        request:    HttpMessage,
        route:      &ProxyRoute,
        client_w:   &mut W,
        src_addr:   SocketAddr,
        id:         &str,
    )
        -> Outcome<(u16, Option<u64>)>
        where W: AsyncWriteExt + Unpin,
    {
        // Extract method, request path and the raw query. The query must ride
        // through verbatim: an upstream that dispatches on a query parameter
        // (e.g. `?view=`) never sees it otherwise, and silently gets the
        // default.
        let (method, path, query) = match &request.header.headline {
            HttpHeadline::Request { method, loc } => {
                (fmt!("{}", method), loc.path.as_string().to_string(), loc.query.clone())
            }
            _ => return Err(err!(
                "{}: proxy: request is not an HTTP request.", id;
                Invalid, Bug)),
        };

        let upstream_path = match query.is_empty() {
            true  => route.upstream_path_for(&path),
            false => fmt!("{}?{}", route.upstream_path_for(&path), query),
        };

        // Connect to the upstream.
        let mut upstream = match TcpStream::connect(
            (route.upstream_host.as_str(), route.upstream_port),
        ).await {
            Ok(s) => s,
            Err(e) => return Err(err!(e,
                "{}: proxy: failed to connect to {}:{}.",
                id, route.upstream_host, route.upstream_port;
                IO, Network, Init)),
        };

        // Build the request bytes to send to the upstream.
        // We reconstruct from the parsed HttpMessage, forwarding all
        // original headers except Host, Connection and Content-Length
        // which we manage ourselves.
        let mut req = String::with_capacity(512 + request.body.len());
        req.push_str(&fmt!("{} {} HTTP/1.1\r\n", method, upstream_path));
        req.push_str(&fmt!("Host: {}\r\n", route.upstream_host));

        // Forward original headers (skip hop-by-hop and managed ones).
        for (name, values) in request.header.fields.iter() {
            let name_str = fmt!("{}", name);
            if name_str.eq_ignore_ascii_case("host")
                || name_str.eq_ignore_ascii_case("connection")
                || name_str.eq_ignore_ascii_case("content-length")
                || name_str.eq_ignore_ascii_case("transfer-encoding")
            {
                continue;
            }
            for value in values {
                req.push_str(&fmt!("{}: {}\r\n", name_str, value));
            }
        }

        // Proxy headers.
        req.push_str(&fmt!("X-Forwarded-For: {}\r\n", src_addr));
        req.push_str("X-Forwarded-Proto: https\r\n");
        req.push_str("Connection: close\r\n");
        req.push_str(&fmt!("Content-Length: {}\r\n", request.body.len()));
        req.push_str("\r\n");

        // Write request to upstream.
        match upstream.write_all(req.as_bytes()).await {
            Ok(()) => (),
            Err(e) => return Err(err!(e,
                "{}: proxy: failed to write request to upstream.", id;
                IO, Network, Wire, Write)),
        }
        if !request.body.is_empty() {
            match upstream.write_all(&request.body).await {
                Ok(()) => (),
                Err(e) => return Err(err!(e,
                    "{}: proxy: failed to write request body to upstream.", id;
                    IO, Network, Wire, Write)),
            }
        }
        match upstream.flush().await {
            Ok(()) => (),
            Err(e) => return Err(err!(e,
                "{}: proxy: failed to flush upstream.", id;
                IO, Network, Wire, Write)),
        }

        // Stream the response from upstream to client.
        // Read into a buffer, find the header/body boundary,
        // parse the status code, then stream everything.
        let mut buf = vec![0u8; 16384];
        let mut accum: Vec<u8> = Vec::new();
        let mut status_code: u16 = 0;
        let mut total_body_bytes: u64 = 0;
        let mut headers_forwarded = false;

        loop {
            let n = match upstream.read(&mut buf).await {
                Ok(0) => break,
                Ok(n) => n,
                Err(e) => return Err(err!(e,
                    "{}: proxy: error reading upstream response.", id;
                    IO, Network, Wire, Read)),
            };

            if !headers_forwarded {
                accum.extend_from_slice(&buf[..n]);
                // Look for end-of-headers marker.
                if let Some(pos) = accum.windows(4).position(|w| w == b"\r\n\r\n") {
                    let header_end = pos + 4;
                    let header_bytes = &accum[..header_end];
                    let body_start = &accum[header_end..];

                    // Parse status code from the first line.
                    if let Some(line_end) = header_bytes.iter().position(|&b| b == b'\r') {
                        let status_line = String::from_utf8_lossy(&header_bytes[..line_end]);
                        // Format: "HTTP/1.1 200 OK"
                        let parts: Vec<&str> = status_line.splitn(3, ' ').collect();
                        if parts.len() >= 2 {
                            if let Ok(code) = parts[1].parse::<u16>() {
                                status_code = code;
                            }
                        }
                    }

                    // Forward the response headers to the client.
                    match client_w.write_all(header_bytes).await {
                        Ok(()) => (),
                        Err(e) => return Err(err!(e,
                            "{}: proxy: failed to write response headers to client.", id;
                            IO, Network, Wire, Write)),
                    }

                    // Forward any body bytes that arrived with the headers.
                    if !body_start.is_empty() {
                        match client_w.write_all(body_start).await {
                            Ok(()) => (),
                            Err(e) => return Err(err!(e,
                                "{}: proxy: failed to write initial body to client.", id;
                                IO, Network, Wire, Write)),
                        }
                        total_body_bytes += body_start.len() as u64;
                    }

                    headers_forwarded = true;
                } else if accum.len() > 65536 {
                    return Err(err!(
                        "{}: proxy: upstream response headers exceed 64 KiB.", id;
                        IO, Network, Input, TooBig));
                }
            } else {
                // Stream body chunks directly.
                match client_w.write_all(&buf[..n]).await {
                    Ok(()) => (),
                    Err(e) => return Err(err!(e,
                        "{}: proxy: failed to stream body to client.", id;
                        IO, Network, Wire, Write)),
                }
                total_body_bytes += n as u64;
            }
        }

        match client_w.flush().await {
            Ok(()) => (),
            Err(e) => return Err(err!(e,
                "{}: proxy: failed to flush client stream.", id;
                IO, Network, Wire, Write)),
        }

        if status_code == 0 {
            status_code = 200;  // Fallback if parsing failed.
        }

        log!(log_get_level!(),
            "{}: proxy: {} {} -> {} ({} body bytes)",
            id, method, path, status_code, total_body_bytes);

        Ok((status_code, Some(total_body_bytes)))
    }

    /// Tunnel a WebSocket upgrade request to the upstream proxy target.
    ///
    /// Reconstructs the upgrade handshake, sends it to the upstream,
    /// forwards the upstream's 101 response to the client, then
    /// bidirectionally pipes raw bytes between client and upstream for
    /// the lifetime of the WebSocket connection.  Returns when either
    /// direction closes.
    async fn handle_proxy_websocket<S>(
        self,
        client:     &mut S,
        request:    HttpMessage,
        route:      &ProxyRoute,
        src_addr:   SocketAddr,
        id:         &str,
    )
        -> Outcome<()>
        where S: AsyncRead + AsyncWrite + Unpin,
    {
        // Extract path and raw query from the request, forwarding the query
        // verbatim as the HTTP path does.
        let (path, query) = match &request.header.headline {
            HttpHeadline::Request { loc, .. } =>
                (loc.path.as_string().to_string(), loc.query.clone()),
            _ => return Err(err!(
                "{}: proxy ws: request is not an HTTP request.", id;
                Invalid, Bug)),
        };
        let upstream_path = match query.is_empty() {
            true  => route.upstream_path_for(&path),
            false => fmt!("{}?{}", route.upstream_path_for(&path), query),
        };

        // Connect to the upstream.
        let mut upstream = match TcpStream::connect(
            (route.upstream_host.as_str(), route.upstream_port),
        ).await {
            Ok(s) => s,
            Err(e) => return Err(err!(e,
                "{}: proxy ws: failed to connect to {}:{}.",
                id, route.upstream_host, route.upstream_port;
                IO, Network, Init)),
        };

        // Reconstruct the WebSocket upgrade request for the upstream.
        let mut req = String::with_capacity(512);
        req.push_str(&fmt!("GET {} HTTP/1.1\r\n", upstream_path));
        req.push_str(&fmt!("Host: {}\r\n", route.upstream_host));

        // Forward all original headers except Host, Connection and
        // Content-Length which we manage.  WebSocket upgrade headers
        // (Upgrade, Sec-WebSocket-Key, Sec-WebSocket-Version, etc.)
        // are forwarded verbatim.
        for (name, values) in request.header.fields.iter() {
            let name_str = fmt!("{}", name);
            if name_str.eq_ignore_ascii_case("host")
                || name_str.eq_ignore_ascii_case("connection")
                || name_str.eq_ignore_ascii_case("content-length")
            {
                continue;
            }
            for value in values {
                req.push_str(&fmt!("{}: {}\r\n", name_str, value));
            }
        }

        // Connection: Upgrade for the upstream.
        req.push_str("Connection: Upgrade\r\n");
        req.push_str(&fmt!("X-Forwarded-For: {}\r\n", src_addr));
        req.push_str("X-Forwarded-Proto: https\r\n");
        req.push_str("\r\n");

        // Send the upgrade request to the upstream.
        match upstream.write_all(req.as_bytes()).await {
            Ok(()) => (),
            Err(e) => return Err(err!(e,
                "{}: proxy ws: failed to send upgrade request to upstream.", id;
                IO, Network, Wire, Write)),
        }
        match upstream.flush().await {
            Ok(()) => (),
            Err(e) => return Err(err!(e,
                "{}: proxy ws: failed to flush upstream.", id;
                IO, Network, Wire, Write)),
        }

        // Read the upstream's response (should be 101 Switching Protocols)
        // and forward it to the client.  Read until we find \r\n\r\n.
        let mut buf = vec![0u8; 8192];
        let mut accum: Vec<u8> = Vec::new();
        loop {
            let n = match upstream.read(&mut buf).await {
                Ok(0) => {
                    return Err(err!(
                        "{}: proxy ws: upstream closed before sending \
                        a WebSocket upgrade response.", id;
                        IO, Network, Wire, Read, Missing));
                }
                Ok(n) => n,
                Err(e) => return Err(err!(e,
                    "{}: proxy ws: error reading upstream response.", id;
                    IO, Network, Wire, Read)),
            };
            accum.extend_from_slice(&buf[..n]);
            if let Some(pos) = accum.windows(4).position(|w| w == b"\r\n\r\n") {
                let header_end = pos + 4;
                let response_bytes = &accum[..header_end];
                let extra_bytes = &accum[header_end..];

                // Forward the response to the client.
                match client.write_all(response_bytes).await {
                    Ok(()) => (),
                    Err(e) => return Err(err!(e,
                        "{}: proxy ws: failed to forward upgrade response.", id;
                        IO, Network, Wire, Write)),
                }

                // If the upstream sent any data after the headers
                // (early WebSocket frames), forward those too.
                if !extra_bytes.is_empty() {
                    match client.write_all(extra_bytes).await {
                        Ok(()) => (),
                        Err(e) => return Err(err!(e,
                            "{}: proxy ws: failed to forward early frames.", id;
                            IO, Network, Wire, Write)),
                    }
                }
                match client.flush().await {
                    Ok(()) => (),
                    Err(e) => return Err(err!(e,
                        "{}: proxy ws: failed to flush client.", id;
                        IO, Network, Wire, Write)),
                }
                break;
            }
            if accum.len() > 65536 {
                return Err(err!(
                    "{}: proxy ws: upstream response headers exceed 64 KiB.", id;
                    IO, Network, Input, TooBig));
            }
        }

        // Bidirectionally pipe between client and upstream.
        // Split both streams and use tokio::select to wait for
        // either direction to close.
        let (mut client_r, mut client_w) = tokio::io::split(client);
        let (mut upstream_r, mut upstream_w) = upstream.into_split();

        log!(log_get_level!(),
            "{}: proxy ws: tunnel established, piping bidirectionally.", id);

        tokio::select! {
            // Client -> Upstream
            res = tokio::io::copy(&mut client_r, &mut upstream_w) => {
                match res {
                    Ok(_) => log!(log_get_level!(),
                        "{}: proxy ws: client -> upstream closed.", id),
                    Err(e) => log!(log_get_level!(),
                        "{}: proxy ws: client -> upstream error: {}", id, e),
                }
                let _ = upstream_w.shutdown().await;
            }
            // Upstream -> Client
            res = tokio::io::copy(&mut upstream_r, &mut client_w) => {
                match res {
                    Ok(_) => log!(log_get_level!(),
                        "{}: proxy ws: upstream -> client closed.", id),
                    Err(e) => log!(log_get_level!(),
                        "{}: proxy ws: upstream -> client error: {}", id, e),
                }
                let _ = client_w.shutdown().await;
            }
        }

        log!(log_get_level!(), "{}: proxy ws: tunnel closed.", id);
        Ok(())
    }
}
