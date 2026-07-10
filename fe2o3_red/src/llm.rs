//! LLM client — OpenAI-compatible chat completions with SSE streaming.
//!
//! Uses `fe2o3_net` for the underlying TLS connection.  Parses the
//! `text/event-stream` response line-by-line, extracting `data:` lines
//! containing JSON objects with `delta` content.
//!
//! No `serde` or `reqwest` — the OpenAI API JSON is simple enough to
//! parse manually using string scanning.  This keeps the dependency
//! surface minimal and stays within the fe2o3 ecosystem.

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_jdat::prelude::*;

use crate::protocol::{ChatMessage, ToolCall};

// Native transport imports — the hand-rolled TLS client lives behind
// tokio + rustls, which do not target wasm32.
#[cfg(not(target_arch = "wasm32"))]
use std::sync::Arc;
#[cfg(not(target_arch = "wasm32"))]
use tokio::io::{AsyncReadExt, AsyncWriteExt};
#[cfg(not(target_arch = "wasm32"))]
use tokio_rustls::rustls::ClientConfig;


// ┌───────────────────────────────────────────────────────────────┐
// │ LlmClient                                                      │
// └───────────────────────────────────────────────────────────────┘

/// Async client for an OpenAI-compatible chat completions API.
///
/// Connects via TLS to the configured host, POSTs a chat completion
/// request with `stream: true`, and parses the SSE response
/// incrementally — calling `on_token` for each text chunk as it
/// arrives.
#[derive(Clone, Debug)]
pub struct LlmClient {
    pub host:       String,
    pub port:       u16,
    pub path:       String,
    pub api_key:    String,
    pub model:      String,
    /// Upper bound on generated tokens per turn.  Prevents runaway
    /// reasoning loops (e.g. GLM-5.2 without a cap).
    pub max_tokens: u32,
    /// Root-trust TLS configuration for the native transport.  The wasm
    /// transport delegates trust to the browser's `fetch`, so this field
    /// is native-only.
    #[cfg(not(target_arch = "wasm32"))]
    pub tls_config: Arc<ClientConfig>,
}

/// The response from a completed streaming chat call.
#[derive(Clone, Debug, Default)]
pub struct ChatResponse {
    pub content:           String,
    pub prompt_tokens:     u64,
    pub completion_tokens: u64,
}

/// The response from a non-streaming chat call, which may include
/// tool calls the model wants executed.
#[derive(Clone, Debug, Default)]
pub struct ChatOnceResponse {
    pub content:           String,
    pub tool_calls:        Vec<ToolCall>,
    pub prompt_tokens:     u64,
    pub completion_tokens: u64,
}

impl LlmClient {

    /// Construct a client for the native transport (tokio + rustls).
    #[cfg(not(target_arch = "wasm32"))]
    pub fn new(
        host:       &str,
        port:       u16,
        path:       &str,
        api_key:    &str,
        model:      &str,
        max_tokens: u32,
        tls_config: Arc<ClientConfig>,
    ) -> Self {
        Self {
            host:       host.to_string(),
            port,
            path:       path.to_string(),
            api_key:    api_key.to_string(),
            model:      model.to_string(),
            max_tokens,
            tls_config,
        }
    }

    /// Construct a client for the wasm transport (browser `fetch`).
    ///
    /// TLS trust is handled by the browser, so no `tls_config` is
    /// required — the streaming API (`chat_stream` / `chat_once`) is
    /// otherwise identical to the native client.
    #[cfg(target_arch = "wasm32")]
    pub fn new(
        host:       &str,
        port:       u16,
        path:       &str,
        api_key:    &str,
        model:      &str,
        max_tokens: u32,
    ) -> Self {
        Self {
            host:       host.to_string(),
            port,
            path:       path.to_string(),
            api_key:    api_key.to_string(),
            model:      model.to_string(),
            max_tokens,
        }
    }

    /// Send a streaming chat completion request.
    ///
    /// Reads the SSE response line-by-line from the TLS stream,
    /// calling `on_token` for each text delta *as it arrives*.
    /// Returns the full accumulated response and token usage when
    /// the stream completes.
    pub async fn chat_stream(
        &self,
        messages:   &[ChatMessage],
        on_token:   &mut impl FnMut(&str),
    ) -> Outcome<ChatResponse> {
        let body = self.build_request_body(messages);
        let (prompt_tok, completion_tok, content) =
            res!(self.do_request_stream(&body, on_token).await);
        Ok(ChatResponse {
            content,
            prompt_tokens: prompt_tok,
            completion_tokens: completion_tok,
        })
    }

    /// Non-streaming chat completion, optionally with tools.
    ///
    /// Returns the assistant content and any `tool_calls` the model
    /// wants executed, plus token usage.  Used by the agent's tool
    /// loop, where the complete `tool_calls` array is easier to parse
    /// reliably than streamed fragments.
    pub async fn chat_once(
        &self,
        messages:   &[ChatMessage],
        tools:      Option<&str>,
    ) -> Outcome<ChatOnceResponse> {
        let body = self.build_body(messages, tools, false);
        let raw = res!(self.do_request_full(&body).await);
        let (content, tool_calls, pt, ct) = parse_full_response(&raw);
        Ok(ChatOnceResponse {
            content,
            tool_calls,
            prompt_tokens: pt,
            completion_tokens: ct,
        })
    }

    /// Build the JSON request body for the OpenAI-compatible API.
    ///
    /// `tools` (if present) is a ready-made JSON array injected as the
    /// `tools` field with `tool_choice: auto`.  `stream` toggles SSE
    /// streaming and usage reporting.
    fn build_body(&self, messages: &[ChatMessage], tools: Option<&str>, stream: bool) -> String {
        let mut out = String::with_capacity(1024);
        out.push('{');
        out.push_str(&fmt!("\"model\":\"{}\",", self.model));
        out.push_str("\"messages\":[");
        for (i, msg) in messages.iter().enumerate() {
            if i > 0 { out.push(','); }
            out.push_str(&message_to_json(msg));
        }
        out.push_str("],");
        if let Some(t) = tools {
            out.push_str(&fmt!("\"tools\":{},", t));
            out.push_str("\"tool_choice\":\"auto\",");
        }
        if stream {
            out.push_str("\"stream\":true,");
            out.push_str("\"stream_options\":{\"include_usage\":true},");
        } else {
            out.push_str("\"stream\":false,");
        }
        out.push_str(&fmt!("\"max_tokens\":{}", self.max_tokens));
        out.push('}');
        out
    }

    /// Streaming body (no tools).  Kept as a thin wrapper for the
    /// pure-chat path and its unit test.
    fn build_request_body(&self, messages: &[ChatMessage]) -> String {
        self.build_body(messages, None, true)
    }

    /// Connect, TLS-handshake, send the request, and consume the
    /// response headers.  Returns the stream positioned at the body
    /// start plus whether the body uses chunked transfer encoding.
    /// Errors on a non-200 status (with body detail).
    #[cfg(not(target_arch = "wasm32"))]
    async fn open(
        &self,
        body: &str,
    )
        -> Outcome<(tokio_rustls::client::TlsStream<tokio::net::TcpStream>, bool)>
    {
        use tokio_rustls::TlsConnector;
        use tokio::net::TcpStream;

        let body_bytes = body.as_bytes();

        let mut request = String::with_capacity(512 + body_bytes.len());
        request.push_str(&fmt!("POST {} HTTP/1.1\r\n", self.path));
        request.push_str(&fmt!("Host: {}\r\n", self.host));
        request.push_str(&fmt!("Authorization: Bearer {}\r\n", self.api_key));
        request.push_str("Content-Type: application/json\r\n");
        request.push_str(&fmt!("Content-Length: {}\r\n", body_bytes.len()));
        request.push_str("Connection: close\r\n");
        request.push_str("\r\n");

        let tcp = match TcpStream::connect((self.host.as_str(), self.port)).await {
            Ok(s) => s,
            Err(e) => return Err(err!(e,
                "LLM: TCP connect to {}:{} failed.", self.host, self.port;
                IO, Network, Init)),
        };
        let server_name = match tokio_rustls::rustls::pki_types::ServerName::try_from(self.host.clone()) {
            Ok(n) => n,
            Err(e) => return Err(err!(e,
                "LLM: invalid server name '{}'.", self.host;
                IO, Network, Invalid, Input)),
        };
        let connector = TlsConnector::from(self.tls_config.clone());
        let mut stream = match connector.connect(server_name, tcp).await {
            Ok(s) => s,
            Err(e) => return Err(err!(e,
                "LLM: TLS handshake to {} failed.", self.host;
                IO, Network, Init)),
        };

        let mut req = Vec::with_capacity(request.as_bytes().len() + body_bytes.len());
        req.extend_from_slice(request.as_bytes());
        req.extend_from_slice(body_bytes);
        res!(stream.write_all(&req).await
            .map_err(|e| err!(e, "LLM: write request failed."; IO, Network, Wire, Write)));
        res!(stream.flush().await
            .map_err(|e| err!(e, "LLM: flush failed."; IO, Network, Wire, Write)));

        // Read headers byte-by-byte until \r\n\r\n.
        let mut hdr_buf = Vec::with_capacity(2048);
        let mut byte = [0u8; 1];
        loop {
            match stream.read(&mut byte).await {
                Ok(0) => break,
                Ok(_) => {
                    hdr_buf.push(byte[0]);
                    if hdr_buf.ends_with(b"\r\n\r\n") { break; }
                }
                Err(e) if e.kind() == tokio::io::ErrorKind::UnexpectedEof => break,
                Err(e) => return Err(err!(e,
                    "LLM: read headers failed."; IO, Network, Wire, Read)),
            }
        }

        let headers_str = String::from_utf8_lossy(&hdr_buf);
        let is_chunked = headers_str
            .to_ascii_lowercase()
            .contains("transfer-encoding: chunked");

        let status_line = headers_str.lines().next().unwrap_or("");
        if !status_line.contains("200") {
            let mut err_body = Vec::new();
            let mut chunk = [0u8; 4096];
            loop {
                match stream.read(&mut chunk).await {
                    Ok(0) => break,
                    Ok(n) => err_body.extend_from_slice(&chunk[..n]),
                    Err(_) => break,
                }
            }
            let err_msg = String::from_utf8_lossy(&err_body);
            return Err(err!(
                "LLM: HTTP error: {} | {}", status_line, &err_msg[..err_msg.len().min(300)];
                IO, Network, Wire, Read));
        }

        Ok((stream, is_chunked))
    }

    /// Perform a non-streaming request and return the full response
    /// body as one string.  Lines are concatenated (JSON does not need
    /// the newlines), dechunking transparently.
    #[cfg(not(target_arch = "wasm32"))]
    async fn do_request_full(&self, body: &str) -> Outcome<String> {
        let (stream, is_chunked) = res!(self.open(body).await);
        let mut reader = LineReader::new(stream, is_chunked);
        let mut full = String::new();
        loop {
            match reader.read_line().await {
                Ok(Some(l)) => full.push_str(&l),
                Ok(None) => break,
                Err(e) if e.kind() == tokio::io::ErrorKind::UnexpectedEof => break,
                Err(e) => return Err(err!(e,
                    "LLM: read response body failed."; IO, Network, Wire, Read)),
            }
        }
        Ok(full)
    }

    /// Send the HTTP request and stream the SSE response line-by-line.
    ///
    /// Reads the response headers first, then reads the body
    /// incrementally — calling `on_token` for each `data:` line
    /// as it arrives.  Handles both chunked and identity transfer
    /// encoding.
    ///
    /// Returns `(prompt_tokens, completion_tokens, full_content)`.
    #[cfg(not(target_arch = "wasm32"))]
    async fn do_request_stream(
        &self,
        body:           &str,
        on_token:       &mut impl FnMut(&str),
    ) -> Outcome<(u64, u64, String)>
    {
        use tokio_rustls::TlsConnector;
        use tokio::net::TcpStream;

        let body_bytes = body.as_bytes();

        // Build HTTP request.
        let mut request = String::with_capacity(512 + body_bytes.len());
        request.push_str(&fmt!("POST {} HTTP/1.1\r\n", self.path));
        request.push_str(&fmt!("Host: {}\r\n", self.host));
        request.push_str(&fmt!("Authorization: Bearer {}\r\n", self.api_key));
        request.push_str("Content-Type: application/json\r\n");
        request.push_str(&fmt!("Content-Length: {}\r\n", body_bytes.len()));
        request.push_str("Connection: close\r\n");
        request.push_str("\r\n");

        // Connect + TLS handshake.
        let tcp = match TcpStream::connect((self.host.as_str(), self.port)).await {
            Ok(s) => s,
            Err(e) => return Err(err!(e,
                "LLM: TCP connect to {}:{} failed.", self.host, self.port;
                IO, Network, Init)),
        };
        let server_name = match tokio_rustls::rustls::pki_types::ServerName::try_from(self.host.clone()) {
            Ok(n) => n,
            Err(e) => return Err(err!(e,
                "LLM: invalid server name '{}'.", self.host;
                IO, Network, Invalid, Input)),
        };
        let connector = TlsConnector::from(self.tls_config.clone());
        let mut stream = match connector.connect(server_name, tcp).await {
            Ok(s) => s,
            Err(e) => return Err(err!(e,
                "LLM: TLS handshake to {} failed.", self.host;
                IO, Network, Init)),
        };

        // Write request — combine headers and body into a single
        // TLS record so CDN-fronted servers don't reject split
        // header/body writes.
        let mut req = Vec::with_capacity(request.as_bytes().len() + body_bytes.len());
        req.extend_from_slice(request.as_bytes());
        req.extend_from_slice(body_bytes);
        res!(stream.write_all(&req).await
            .map_err(|e| err!(e, "LLM: write request failed."; IO, Network, Wire, Write)));
        res!(stream.flush().await
            .map_err(|e| err!(e, "LLM: flush failed."; IO, Network, Wire, Write)));

        // ── Read response headers ──────────────────────────────
        //
        // Read byte-by-byte until we find \r\n\r\n.  This is
        // slightly wasteful but headers are small (< 1KB) and
        // the simplicity is worth it.
        let mut hdr_buf = Vec::with_capacity(2048);
        let mut byte = [0u8; 1];
        loop {
            match stream.read(&mut byte).await {
                Ok(0) => break,
                Ok(_) => {
                    hdr_buf.push(byte[0]);
                    if hdr_buf.ends_with(b"\r\n\r\n") {
                        break;
                    }
                }
                Err(e) if e.kind() == tokio::io::ErrorKind::UnexpectedEof => break,
                Err(e) => return Err(err!(e,
                    "LLM: read headers failed."; IO, Network, Wire, Read)),
            }
        }

        let headers_str = String::from_utf8_lossy(&hdr_buf);
        let is_chunked = headers_str
            .to_ascii_lowercase()
            .contains("transfer-encoding: chunked");

        // Check for HTTP error status.
        let status_line = headers_str.lines().next().unwrap_or("");
        if !status_line.contains("200") {
            // Read remaining body for error details.
            let mut err_body = Vec::new();
            let mut chunk = [0u8; 4096];
            loop {
                match stream.read(&mut chunk).await {
                    Ok(0) => break,
                    Ok(n) => err_body.extend_from_slice(&chunk[..n]),
                    Err(_) => break,
                }
            }
            let err_msg = String::from_utf8_lossy(&err_body);
            return Err(err!(
                "LLM: HTTP error: {} | {}", status_line, &err_msg[..err_msg.len().min(200)];
                IO, Network, Wire, Read));
        }

        // ── Stream body line-by-line ───────────────────────────
        let mut reader = LineReader::new(stream, is_chunked);
        let mut full = String::new();
        let mut prompt_tokens = 0u64;
        let mut completion_tokens = 0u64;

        loop {
            let line = match reader.read_line().await {
                Ok(Some(l)) => l,
                Ok(None) => break,
                Err(e) if e.kind() == tokio::io::ErrorKind::UnexpectedEof => break,
                Err(e) => return Err(err!(e,
                    "LLM: read SSE line failed."; IO, Network, Wire, Read)),
            };
            let line = line.trim();
            if !line.starts_with("data: ") {
                continue;
            }
            let data = &line[6..];
            if data == "[DONE]" {
                break;
            }
            // Extract content token.
            if let Some(content) = extract_json_string(data, "content") {
                on_token(&content);
                full.push_str(&content);
            }
            // Extract usage from the final chunk.
            if let Some(usage_str) = find_json_object(data, "usage") {
                if let Some(pt) = extract_json_number(&usage_str, "prompt_tokens") {
                    prompt_tokens = pt;
                }
                if let Some(ct) = extract_json_number(&usage_str, "completion_tokens") {
                    completion_tokens = ct;
                }
            }
        }

        Ok((prompt_tokens, completion_tokens, full))
    }
}


// ┌───────────────────────────────────────────────────────────────┐
// │ Wasm transport — browser `fetch` + `ReadableStream`            │
// └───────────────────────────────────────────────────────────────┘
//
// The wasm build has no TCP sockets or TLS stack; the browser owns
// both.  These methods mirror the native transport's public contract
// (`do_request_full` / `do_request_stream`) using `fetch`, so the
// `chat_stream` / `chat_once` API above is target-agnostic.

#[cfg(target_arch = "wasm32")]
impl LlmClient {

    /// The absolute request URL for the browser transport.
    fn wasm_url(&self) -> String {
        if self.port == 443 {
            fmt!("https://{}{}", self.host, self.path)
        } else {
            fmt!("https://{}:{}{}", self.host, self.port, self.path)
        }
    }

    /// POST `body` via `fetch` and await the `Response`, mapping any
    /// JS error into an `Outcome`.  TLS trust is the browser's.
    async fn wasm_fetch(&self, body: &str) -> Outcome<web_sys::Response> {
        use wasm_bindgen::JsCast;
        use wasm_bindgen::JsValue;
        use wasm_bindgen_futures::JsFuture;
        use web_sys::{Headers, Request, RequestInit, RequestMode, Response};

        let headers = res!(Headers::new()
            .map_err(|e| err!("LLM: create headers failed: {}.", js_str(&e); IO, Network, Init)));
        res!(headers.append("Authorization", &fmt!("Bearer {}", self.api_key))
            .map_err(|e| err!("LLM: set auth header failed: {}.", js_str(&e); IO, Network, Init)));
        res!(headers.append("Content-Type", "application/json")
            .map_err(|e| err!("LLM: set content-type failed: {}.", js_str(&e); IO, Network, Init)));

        let opts = RequestInit::new();
        opts.set_method("POST");
        opts.set_mode(RequestMode::Cors);
        opts.set_headers(&headers);
        opts.set_body(&JsValue::from_str(body));

        let url = self.wasm_url();
        let request = res!(Request::new_with_str_and_init(&url, &opts)
            .map_err(|e| err!("LLM: build request failed: {}.", js_str(&e); IO, Network, Init)));

        // `fetch` lives on the window in a document context and on the
        // global scope in a worker; support both.
        let promise = if let Some(win) = web_sys::window() {
            win.fetch_with_request(&request)
        } else {
            let scope = res!(js_sys::global()
                .dyn_into::<web_sys::WorkerGlobalScope>()
                .map_err(|_| err!(
                    "LLM: no window or worker scope for fetch."; IO, Network, Init)));
            scope.fetch_with_request(&request)
        };

        let resp_val = res!(JsFuture::from(promise).await
            .map_err(|e| err!("LLM: fetch failed: {}.", js_str(&e); IO, Network, Wire)));
        let resp: Response = res!(resp_val.dyn_into()
            .map_err(|_| err!("LLM: fetch did not return a Response."; IO, Network, Wire)));
        if !resp.ok() {
            return Err(err!(
                "LLM: HTTP error: {} {}.", resp.status(), resp.status_text();
                IO, Network, Wire, Read));
        }
        Ok(resp)
    }

    /// Non-streaming request — await the full response body as text.
    async fn do_request_full(&self, body: &str) -> Outcome<String> {
        use wasm_bindgen_futures::JsFuture;

        let resp = res!(self.wasm_fetch(body).await);
        let text_promise = res!(resp.text()
            .map_err(|e| err!("LLM: read response text failed: {}.", js_str(&e); IO, Network, Wire, Read)));
        let text_val = res!(JsFuture::from(text_promise).await
            .map_err(|e| err!("LLM: await response text failed: {}.", js_str(&e); IO, Network, Wire, Read)));
        Ok(text_val.as_string().unwrap_or_default())
    }

    /// Streaming request — read the SSE body incrementally from the
    /// response's `ReadableStream`, calling `on_token` for each text
    /// delta.  Returns `(prompt_tokens, completion_tokens, content)`.
    async fn do_request_stream(
        &self,
        body:           &str,
        on_token:       &mut impl FnMut(&str),
    ) -> Outcome<(u64, u64, String)>
    {
        use wasm_bindgen::JsValue;
        use wasm_bindgen_futures::JsFuture;
        use web_sys::{ReadableStream, ReadableStreamDefaultReader};

        let resp = res!(self.wasm_fetch(body).await);
        let stream: ReadableStream = match resp.body() {
            Some(s) => s,
            None => return Err(err!(
                "LLM: response has no body stream."; IO, Network, Wire, Read)),
        };
        let reader = res!(ReadableStreamDefaultReader::new(&stream)
            .map_err(|e| err!("LLM: acquire stream reader failed: {}.", js_str(&e); IO, Network, Wire, Read)));

        // Accumulate raw bytes and extract complete SSE lines as they
        // arrive, mirroring the native `LineReader` line discipline.
        let mut buf: Vec<u8> = Vec::with_capacity(8192);
        let mut full = String::new();
        let mut prompt_tokens = 0u64;
        let mut completion_tokens = 0u64;

        loop {
            let result = res!(JsFuture::from(reader.read()).await
                .map_err(|e| err!("LLM: read stream chunk failed: {}.", js_str(&e); IO, Network, Wire, Read)));
            let done = res!(js_sys::Reflect::get(&result, &JsValue::from_str("done"))
                .map_err(|e| err!("LLM: read 'done' failed: {}.", js_str(&e); IO, Network, Wire, Read)))
                .as_bool()
                .unwrap_or(true);
            if done {
                break;
            }
            let value = res!(js_sys::Reflect::get(&result, &JsValue::from_str("value"))
                .map_err(|e| err!("LLM: read 'value' failed: {}.", js_str(&e); IO, Network, Wire, Read)));
            let chunk = js_sys::Uint8Array::new(&value).to_vec();
            buf.extend_from_slice(&chunk);

            // Drain complete lines (terminated by `\n`) from the buffer.
            loop {
                let nl = match buf.iter().position(|&b| b == b'\n') {
                    Some(p) => p,
                    None    => break,
                };
                let line_bytes: Vec<u8> = buf.drain(..=nl).collect();
                let line = String::from_utf8_lossy(&line_bytes[..line_bytes.len() - 1]);
                let line = line.trim();
                if !line.starts_with("data: ") {
                    continue;
                }
                let data = &line[6..];
                if data == "[DONE]" {
                    return Ok((prompt_tokens, completion_tokens, full));
                }
                if let Some(content) = extract_json_string(data, "content") {
                    on_token(&content);
                    full.push_str(&content);
                }
                if let Some(usage_str) = find_json_object(data, "usage") {
                    if let Some(pt) = extract_json_number(&usage_str, "prompt_tokens") {
                        prompt_tokens = pt;
                    }
                    if let Some(ct) = extract_json_number(&usage_str, "completion_tokens") {
                        completion_tokens = ct;
                    }
                }
            }
        }

        Ok((prompt_tokens, completion_tokens, full))
    }
}

/// Render a JS error value as a human-readable string for error tags.
#[cfg(target_arch = "wasm32")]
fn js_str(v: &wasm_bindgen::JsValue) -> String {
    v.as_string().unwrap_or_else(|| fmt!("{:?}", v))
}


// ┌───────────────────────────────────────────────────────────────┐
// │ LineReader — incremental line reader for TLS streams           │
// └───────────────────────────────────────────────────────────────┘

/// Reads lines from a TLS stream, handling HTTP chunked transfer
/// encoding transparently.
///
/// For identity (Content-Length) encoding, lines are read directly
/// from the stream.  For chunked encoding, chunk headers are parsed
/// and chunk data is dechunked on the fly, so the caller sees a
/// continuous stream of lines.
///
/// A line is terminated by `\n` (with or without a preceding `\r`).
#[cfg(not(target_arch = "wasm32"))]
struct LineReader<S: tokio::io::AsyncRead + Unpin> {
    stream:     S,
    buf:        Vec<u8>,
    buf_pos:    usize,
    is_chunked: bool,
    // For chunked encoding: remaining bytes in the current chunk.
    // None means we need to read the next chunk header.
    chunk_remaining: Option<usize>,
    eof:        bool,
}

#[cfg(not(target_arch = "wasm32"))]
impl<S: tokio::io::AsyncRead + Unpin> LineReader<S> {

    fn new(stream: S, is_chunked: bool) -> Self {
        Self {
            stream,
            buf: Vec::with_capacity(8192),
            buf_pos: 0,
            is_chunked,
            chunk_remaining: None,
            eof: false,
        }
    }

    /// Read the next line (without the trailing newline).
    ///
    /// Returns `Ok(None)` at end of stream.
    async fn read_line(&mut self) -> std::io::Result<Option<String>> {
        loop {
            // Try to find a complete line in the buffer.
            if let Some(line) = self.try_extract_line() {
                return Ok(Some(line));
            }
            if self.eof {
                // If there's remaining data without a newline,
                // return it as the last line.
                if self.buf_pos < self.buf.len() {
                    let rest = String::from_utf8_lossy(
                        &self.buf[self.buf_pos..]
                    ).to_string();
                    self.buf_pos = self.buf.len();
                    return Ok(Some(rest));
                }
                return Ok(None);
            }
            // Need more data.
            match self.fill_buf().await {
                Ok(())  => {},
                Err(e)  => return Err(e),
            }
        }
    }

    /// Try to extract a complete line from the buffer.
    fn try_extract_line(&mut self) -> Option<String> {
        let search_start = self.buf_pos;
        let rest = &self.buf[search_start..];
        if let Some(pos) = rest.iter().position(|&b| b == b'\n') {
            let end = search_start + pos;
            let line = &self.buf[self.buf_pos..end];
            // Strip trailing \r if present.
            let line = if line.ends_with(b"\r") { &line[..line.len()-1] } else { line };
            let s = String::from_utf8_lossy(line).to_string();
            self.buf_pos = end + 1; // skip the \n
            // Compact buffer periodically.
            if self.buf_pos > 16384 {
                self.buf.drain(..self.buf_pos);
                self.buf_pos = 0;
            }
            return Some(s);
        }
        None
    }

    /// Read more data into the buffer.
    async fn fill_buf(&mut self) -> std::io::Result<()> {
        let mut tmp = [0u8; 4096];

        if self.is_chunked {
            // For chunked encoding, we need to be careful about
            // chunk boundaries.  However, SSE lines are always
            // within a single chunk in practice (servers don't
            // split a data: line across chunks).  We read raw
            // bytes and handle chunk boundaries in the line
            // buffer.  This is simpler than tracking exact chunk
            // positions and works because we only need lines.
            //
            // For correctness, we parse chunk headers when we
            // run out of chunk data.
            if self.chunk_remaining == Some(0) {
                // Read and discard the trailing \r\n after a chunk,
                // then read the next chunk header.
                let mut crlf = [0u8; 2];
                match self.stream.read_exact(&mut crlf).await {
                    Ok(_) => {}
                    Err(e) if e.kind() == tokio::io::ErrorKind::UnexpectedEof => {
                        self.eof = true;
                        return Ok(());
                    }
                    Err(e) => return Err(e),
                }
                self.chunk_remaining = None;
            }

            if self.chunk_remaining.is_none() {
                // Read chunk size line.
                let mut size_line = Vec::new();
                let mut byte = [0u8; 1];
                loop {
                    match self.stream.read(&mut byte).await {
                        Ok(0) => { self.eof = true; return Ok(()); }
                        Ok(_) => {
                            size_line.push(byte[0]);
                            if size_line.ends_with(b"\r\n") {
                                break;
                            }
                            // Some servers include chunk extensions
                            // after the size: 1a;ext=val\r\n
                            if size_line.ends_with(b"\n") {
                                break;
                            }
                        }
                        Err(e) if e.kind() == tokio::io::ErrorKind::UnexpectedEof => {
                            self.eof = true;
                            return Ok(());
                        }
                        Err(e) => return Err(e),
                    }
                }
                let size_str = String::from_utf8_lossy(&size_line);
                let size_str = size_str.trim();
                // Strip chunk extensions (everything after ;).
                let size_str = size_str.split(';').next().unwrap_or("0").trim();
                let size = match usize::from_str_radix(size_str, 16) {
                    Ok(n) => n,
                    Err(_) => { self.eof = true; return Ok(()); }
                };
                if size == 0 {
                    // Last chunk — end of body.
                    self.eof = true;
                    return Ok(());
                }
                self.chunk_remaining = Some(size);
            }

            // Read up to chunk_remaining bytes or tmp.len(), whichever is smaller.
            let remaining = match self.chunk_remaining {
                Some(r) => r,
                None    => return Err(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    "chunk_remaining unexpectedly unset")),
            };
            let to_read = remaining.min(tmp.len());
            match self.stream.read(&mut tmp[..to_read]).await {
                Ok(0) => { self.eof = true; return Ok(()); }
                Ok(n) => {
                    self.buf.extend_from_slice(&tmp[..n]);
                    self.chunk_remaining = Some(remaining - n);
                }
                Err(e) if e.kind() == tokio::io::ErrorKind::UnexpectedEof => {
                    self.eof = true;
                    return Ok(());
                }
                Err(e) => return Err(e),
            }
        } else {
            // Identity encoding — read directly.
            match self.stream.read(&mut tmp).await {
                Ok(0) => { self.eof = true; return Ok(()); }
                Ok(n) => self.buf.extend_from_slice(&tmp[..n]),
                Err(e) if e.kind() == tokio::io::ErrorKind::UnexpectedEof => {
                    self.eof = true;
                    return Ok(());
                }
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }
}

/// Parse an SSE response body, calling `on_token` for each text delta.
///
/// SSE format:
/// ```text
/// data: {"choices":[{"delta":{"content":"Hello"}}]}
///
/// data: {"choices":[{"delta":{"content":" world"}}]}
///
/// data: [DONE]
/// ```
///
/// We scan for `"content":"..."` in each `data:` line.  This is a
/// deliberately simple parser — it handles the common case without
/// needing a full JSON parser.  Escaped quotes inside content are
/// handled by scanning for the matching unescaped quote.
pub fn parse_sse_stream(body: &[u8], on_token: &mut impl FnMut(&str))
    -> (String, u64, u64)
{
    let text = String::from_utf8_lossy(body);
    let mut full = String::new();
    let mut prompt_tokens = 0u64;
    let mut completion_tokens = 0u64;

    for line in text.lines() {
        let line = line.trim();
        if !line.starts_with("data: ") {
            continue;
        }
        let data = &line[6..];
        if data == "[DONE]" {
            break;
        }
        // Extract content from: {"choices":[{"delta":{"content":"..."}}]}
        if let Some(content) = extract_json_string(data, "content") {
            on_token(&content);
            full.push_str(&content);
        }
        // Extract usage from the final chunk:
        // {"choices":[],"usage":{"prompt_tokens":13,"completion_tokens":200}}
        if let Some(usage_str) = find_json_object(data, "usage") {
            if let Some(pt) = extract_json_number(&usage_str, "prompt_tokens") {
                prompt_tokens = pt;
            }
            if let Some(ct) = extract_json_number(&usage_str, "completion_tokens") {
                completion_tokens = ct;
            }
        }
    }

    (full, prompt_tokens, completion_tokens)
}

/// Extract a JSON object value for a key from a JSON string.
///
/// Scans for `"key":{...}` and returns the inner object string
/// (including the braces).  Used to extract the `usage` object
/// from the final SSE chunk.
fn find_json_object(json: &str, key: &str) -> Option<String> {
    let needle = fmt!("\"{}\":", key);
    let pos = match json.find(&needle) {
        Some(p) => p,
        None    => return None,
    };
    let bytes = json.as_bytes();
    // Skip whitespace after the colon to the opening brace.
    let mut start = pos + needle.len();
    while start < bytes.len() && bytes[start].is_ascii_whitespace() { start += 1; }
    if start >= bytes.len() || bytes[start] != b'{' { return None; }
    let mut depth = 0i32;
    let mut i = start;
    while i < bytes.len() {
        match bytes[i] {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(json[start..=i].to_string());
                }
            }
            b'"' => {
                // Skip string contents.
                i += 1;
                while i < bytes.len() {
                    if bytes[i] == b'\\' { i += 2; continue; }
                    if bytes[i] == b'"' { break; }
                    i += 1;
                }
            }
            _ => (),
        }
        i += 1;
    }
    None
}

/// Extract a numeric value for a key from a JSON string.
///
/// Scans for `"key":number` and returns the parsed value.
fn extract_json_number(json: &str, key: &str) -> Option<u64> {
    let needle = fmt!("\"{}\":", key);
    let pos = match json.find(&needle) {
        Some(p) => p,
        None    => return None,
    };
    let mut start = pos + needle.len();
    let bytes = json.as_bytes();
    // Skip whitespace.
    while start < bytes.len() && bytes[start].is_ascii_whitespace() {
        start += 1;
    }
    let mut end = start;
    while end < bytes.len() && (bytes[end].is_ascii_digit() || bytes[end] == b'-') {
        end += 1;
    }
    json[start..end].parse::<u64>().ok()
}


/// Handles `\"`, `\\`, `\n`, `\t` escapes.
///
/// The search ensures `key` is a complete JSON key, not a suffix of
/// a longer key (e.g. `"content"` must not match inside
/// `"reasoning_content"`).  This is done by requiring the character
/// before the opening quote to be `{` or `,` (whitespace-tolerant).
pub(crate) fn extract_json_string(json: &str, key: &str) -> Option<String> {
    let needle = fmt!("\"{}\":", key);
    let bytes = json.as_bytes();
    let mut search_from = 0;
    loop {
        let pos = match json[search_from..].find(&needle) {
            Some(p) => search_from + p,
            None => return None,
        };
        // Reject suffix matches (e.g. "content" inside
        // "reasoning_content") by checking the character before the
        // key's opening quote.
        let valid_prefix = pos == 0 || {
            let prev = bytes[pos - 1];
            prev == b'{' || prev == b',' || prev.is_ascii_whitespace()
        };
        if !valid_prefix {
            search_from = pos + needle.len();
            continue;
        }
        // Skip whitespace between the colon and the value — real API
        // output uses `"key": "value"` with a space.
        let mut i = pos + needle.len();
        while i < bytes.len() && bytes[i].is_ascii_whitespace() { i += 1; }
        if i >= bytes.len() || bytes[i] != b'"' {
            // Value is not a string (null / number / object); keep
            // searching in case the key appears again.
            search_from = pos + needle.len();
            continue;
        }
        i += 1; // past the opening quote
        // Collect the string value as bytes, then decode as UTF-8, so
        // multi-byte characters survive.
        let mut out: Vec<u8> = Vec::new();
        while i < bytes.len() {
            let b = bytes[i];
            if b == b'\\' && i + 1 < bytes.len() {
                match bytes[i + 1] {
                    b'"'  => out.push(b'"'),
                    b'\\' => out.push(b'\\'),
                    b'n'  => out.push(b'\n'),
                    b't'  => out.push(b'\t'),
                    b'r'  => out.push(b'\r'),
                    b'/'  => out.push(b'/'),
                    other => { out.push(b'\\'); out.push(other); }
                }
                i += 2;
            } else if b == b'"' {
                return Some(String::from_utf8_lossy(&out).to_string());
            } else {
                out.push(b);
                i += 1;
            }
        }
        return None;
    }
}

/// Convert a JDAT DaticleMap to a minimal JSON string.
///
/// This is used to build the LLM API request body without `serde`.
/// Only handles the types we need: String, U64, Bool, Map, List.
/// Serialise a `ChatMessage` to an OpenAI-API JSON object, including
/// assistant `tool_calls` and the `tool` role — which `datmap_to_json`
/// does not carry.
fn message_to_json(msg: &ChatMessage) -> String {
    match msg {
        ChatMessage::System { content } =>
            fmt!("{{\"role\":\"system\",\"content\":\"{}\"}}", json_escape(content)),
        ChatMessage::User { content } =>
            fmt!("{{\"role\":\"user\",\"content\":\"{}\"}}", json_escape(content)),
        ChatMessage::Assistant { content, tool_calls } => {
            if tool_calls.is_empty() {
                fmt!("{{\"role\":\"assistant\",\"content\":\"{}\"}}", json_escape(content))
            } else {
                let calls: Vec<String> = tool_calls.iter().map(|tc| fmt!(
                    "{{\"id\":\"{}\",\"type\":\"function\",\"function\":{{\"name\":\"{}\",\"arguments\":\"{}\"}}}}",
                    json_escape(&tc.id), json_escape(&tc.name), json_escape(&tc.arguments))).collect();
                fmt!("{{\"role\":\"assistant\",\"content\":\"{}\",\"tool_calls\":[{}]}}",
                    json_escape(content), calls.join(","))
            }
        }
        ChatMessage::Tool { tool_call_id, content } =>
            fmt!("{{\"role\":\"tool\",\"tool_call_id\":\"{}\",\"content\":\"{}\"}}",
                json_escape(tool_call_id), json_escape(content)),
    }
}

/// Parse a non-streaming chat completion body into
/// `(content, tool_calls, prompt_tokens, completion_tokens)`.
fn parse_full_response(body: &str) -> (String, Vec<ToolCall>, u64, u64) {
    // Scope content extraction to before "tool_calls" so we don't pick
    // up a "content" key inside a tool call's arguments.
    let scope_end = body.find("\"tool_calls\"").unwrap_or(body.len());
    let content = extract_json_string(&body[..scope_end], "content").unwrap_or_default();

    let mut tool_calls = Vec::new();
    if let Some(arr) = find_json_array(body, "tool_calls") {
        for elem in split_top_level_objects(&arr) {
            let name = match extract_json_string(&elem, "name") {
                Some(n) if !n.is_empty() => n,
                _ => continue,
            };
            let id = extract_json_string(&elem, "id").unwrap_or_default();
            let arguments = extract_json_string(&elem, "arguments")
                .unwrap_or_else(|| "{}".to_string());
            tool_calls.push(ToolCall { id, name, arguments });
        }
    }

    let mut pt = 0u64;
    let mut ct = 0u64;
    if let Some(usage) = find_json_object(body, "usage") {
        pt = extract_json_number(&usage, "prompt_tokens").unwrap_or(0);
        ct = extract_json_number(&usage, "completion_tokens").unwrap_or(0);
    }
    (content, tool_calls, pt, ct)
}

/// Extract a JSON array value for a key, returning the inner text
/// including the surrounding brackets.  String contents are skipped so
/// brackets inside strings don't confuse the depth count.
fn find_json_array(json: &str, key: &str) -> Option<String> {
    let needle = fmt!("\"{}\":", key);
    let pos = match json.find(&needle) {
        Some(p) => p,
        None    => return None,
    };
    let bytes = json.as_bytes();
    // Skip whitespace after the colon to the opening bracket.
    let mut start = pos + needle.len();
    while start < bytes.len() && bytes[start].is_ascii_whitespace() { start += 1; }
    if start >= bytes.len() || bytes[start] != b'[' { return None; }
    let mut depth = 0i32;
    let mut in_str = false;
    let mut i = start;
    while i < bytes.len() {
        let b = bytes[i];
        if in_str {
            if b == b'\\' { i += 2; continue; }
            if b == b'"' { in_str = false; }
        } else {
            match b {
                b'"' => in_str = true,
                b'[' => depth += 1,
                b']' => {
                    depth -= 1;
                    if depth == 0 { return Some(json[start..=i].to_string()); }
                }
                _ => {}
            }
        }
        i += 1;
    }
    None
}

/// Split a JSON array's text into its top-level `{...}` object elements.
fn split_top_level_objects(arr: &str) -> Vec<String> {
    let bytes = arr.as_bytes();
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut start = 0usize;
    let mut in_str = false;
    let mut i = 0usize;
    while i < bytes.len() {
        let b = bytes[i];
        if in_str {
            if b == b'\\' { i += 2; continue; }
            if b == b'"' { in_str = false; }
        } else {
            match b {
                b'"' => in_str = true,
                b'{' => { if depth == 0 { start = i; } depth += 1; }
                b'}' => {
                    depth -= 1;
                    if depth == 0 { out.push(arr[start..=i].to_string()); }
                }
                _ => {}
            }
        }
        i += 1;
    }
    out
}

pub fn datmap_to_json(m: &DaticleMap) -> String {
    let mut out = String::with_capacity(256);
    out.push('{');
    let mut first = true;
    // DaticleMap iteration is not ordered — we sort keys for
    // deterministic output (not required by the API but cleaner).
    let mut entries: Vec<(&Dat, &Dat)> = m.iter().collect();
    entries.sort_by(|a, b| {
        match (a.0, b.0) {
            (Dat::Str(a_s), Dat::Str(b_s)) => a_s.cmp(b_s),
            _ => std::cmp::Ordering::Equal,
        }
    });
    for (k, v) in entries {
        if !first { out.push(','); }
        first = false;
        if let Dat::Str(k_s) = k {
            out.push('"');
            out.push_str(k_s);
            out.push_str("\":");
            out.push_str(&dat_to_json(v));
        }
    }
    out.push('}');
    out
}

/// Escape a string for embedding inside a JSON string literal (no
/// surrounding quotes).  Shared with the tool-definition builder.
pub(crate) fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"'  => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            c if (c as u32) < 0x20 => out.push_str(&fmt!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

/// Convert a JDAT Dat value to JSON.
fn dat_to_json(d: &Dat) -> String {
    match d {
        Dat::Str(s) => {
            let mut out = String::with_capacity(s.len() + 2);
            out.push('"');
            for c in s.chars() {
                match c {
                    '"' => out.push_str("\\\""),
                    '\\' => out.push_str("\\\\"),
                    '\n' => out.push_str("\\n"),
                    '\t' => out.push_str("\\t"),
                    '\r' => out.push_str("\\r"),
                    c if (c as u32) < 0x20 => {
                        out.push_str(&fmt!("\\u{:04x}", c as u32));
                    }
                    c => out.push(c),
                }
            }
            out.push('"');
            out
        }
        Dat::U64(n) => fmt!("{}", n),
        Dat::Bool(b) => fmt!("{}", b),
        Dat::List(list) => {
            let items: Vec<String> = list.iter().map(dat_to_json).collect();
            fmt!("[{}]", items.join(","))
        }
        Dat::Map(m) => datmap_to_json(m),
        Dat::Empty => "null".to_string(),
        _ => "null".to_string(),
    }
}


// ┌───────────────────────────────────────────────────────────────┐
// │ Tests                                                          │
// └───────────────────────────────────────────────────────────────┘

#[cfg(test)]
pub mod tests {
    use super::*;

    #[test]
    fn test_extract_json_string() {
        let json = r#"{"choices":[{"delta":{"content":"hello"}}]}"#;
        assert_eq!(extract_json_string(json, "content"), Some("hello".to_string()));
    }

    #[test]
    fn test_extract_json_string_escaped() {
        let json = r#"{"choices":[{"delta":{"content":"hello \"world\""}}]}"#;
        assert_eq!(extract_json_string(json, "content"), Some("hello \"world\"".to_string()));
    }

    #[test]
    fn test_extract_json_string_newline() {
        let json = r#"{"choices":[{"delta":{"content":"line1\nline2"}}]}"#;
        assert_eq!(extract_json_string(json, "content"), Some("line1\nline2".to_string()));
    }

    #[test]
    fn test_parse_sse_simple() {
        let sse = "data: {\"choices\":[{\"delta\":{\"content\":\"Hello\"}}]}\n\ndata: {\"choices\":[{\"delta\":{\"content\":\" world\"}}]}\n\ndata: [DONE]\n";
        let mut tokens = Vec::new();
        let (full, _pt, _ct) = parse_sse_stream(sse.as_bytes(), &mut |t| tokens.push(t.to_string()));
        assert_eq!(tokens, vec!["Hello", " world"]);
        assert_eq!(full, "Hello world");
    }

    #[test]
    fn test_parse_sse_empty_lines() {
        let sse = "\r\ndata: {\"choices\":[{\"delta\":{\"content\":\"Hi\"}}]}\r\n\r\ndata: [DONE]\r\n";
        let mut tokens = Vec::new();
        let (full, _pt, _ct) = parse_sse_stream(sse.as_bytes(), &mut |t| tokens.push(t.to_string()));
        assert_eq!(tokens, vec!["Hi"]);
        assert_eq!(full, "Hi");
    }

    // Chunked transfer decoding is now handled inline by `LineReader`;
    // the standalone `dechunk` helper and its tests were removed.

    #[test]
    fn test_parse_full_response_tool_calls() {
        let body = r#"{"choices":[{"index":0,"message":{"role":"assistant","content":null,"tool_calls":[{"id":"call_1","type":"function","function":{"name":"file_read","arguments":"{\"path\":\"a.txt\"}"}}]},"finish_reason":"tool_calls"}],"usage":{"prompt_tokens":12,"completion_tokens":8}}"#;
        let (content, calls, pt, ct) = parse_full_response(body);
        assert_eq!(content, "");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].id, "call_1");
        assert_eq!(calls[0].name, "file_read");
        assert_eq!(calls[0].arguments, r#"{"path":"a.txt"}"#);
        assert_eq!(pt, 12);
        assert_eq!(ct, 8);
    }

    #[test]
    fn test_extract_json_string_whitespace() {
        // Real model output has a space after the colon.
        assert_eq!(extract_json_string(r#"{"path": "a.txt"}"#, "path"), Some("a.txt".to_string()));
        assert_eq!(extract_json_string(r#"{ "content": "hi" }"#, "content"), Some("hi".to_string()));
        // A null value is not a string.
        assert_eq!(extract_json_string(r#"{"content": null, "x":"y"}"#, "content"), None);
    }

    #[test]
    fn test_parse_full_response_spaced() {
        // Whitespace after colons, as real APIs emit.
        let body = r#"{"choices": [{"message": {"content": null, "tool_calls": [{"id": "c1", "type": "function", "function": {"name": "file_write", "arguments": "{\"path\": \"a.txt\", \"content\": \"hi\"}"}}]}}], "usage": {"prompt_tokens": 4, "completion_tokens": 2}}"#;
        let (content, calls, pt, ct) = parse_full_response(body);
        assert_eq!(content, "");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "file_write");
        assert_eq!(calls[0].arguments, r#"{"path": "a.txt", "content": "hi"}"#);
        assert_eq!(pt, 4);
        assert_eq!(ct, 2);
        // And the tool can extract the spaced args.
        assert_eq!(extract_json_string(&calls[0].arguments, "path"), Some("a.txt".to_string()));
    }

    #[test]
    fn test_parse_full_response_text() {
        let body = r#"{"choices":[{"message":{"role":"assistant","content":"Hello there."},"finish_reason":"stop"}],"usage":{"prompt_tokens":5,"completion_tokens":3}}"#;
        let (content, calls, pt, ct) = parse_full_response(body);
        assert_eq!(content, "Hello there.");
        assert!(calls.is_empty());
        assert_eq!(pt, 5);
        assert_eq!(ct, 3);
    }

    #[test]
    fn test_parse_full_response_two_calls() {
        let body = r#"{"choices":[{"message":{"content":null,"tool_calls":[{"id":"c1","type":"function","function":{"name":"file_list","arguments":"{}"}},{"id":"c2","type":"function","function":{"name":"shell","arguments":"{\"command\":\"ls\"}"}}]}}]}"#;
        let (_c, calls, _p, _ct) = parse_full_response(body);
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].name, "file_list");
        assert_eq!(calls[1].name, "shell");
        assert_eq!(calls[1].arguments, r#"{"command":"ls"}"#);
    }

    #[test]
    fn test_message_to_json_assistant_tool_calls() {
        let msg = ChatMessage::Assistant {
            content: String::new(),
            tool_calls: vec![ToolCall {
                id: "c1".to_string(),
                name: "shell".to_string(),
                arguments: r#"{"command":"ls"}"#.to_string(),
            }],
        };
        let j = message_to_json(&msg);
        assert!(j.contains(r#""role":"assistant""#));
        assert!(j.contains(r#""tool_calls""#));
        assert!(j.contains(r#""name":"shell""#));
        // Arguments must be re-escaped as a JSON string literal.
        assert!(j.contains(r#""arguments":"{\"command\":\"ls\"}""#));
    }

    #[test]
    fn test_datmap_to_json() {
        let mut m = DaticleMap::new();
        m.insert(dat!("role"), dat!("user"));
        m.insert(dat!("content"), dat!("hello"));
        let json = datmap_to_json(&m);
        // Keys are sorted.
        assert!(json.contains("\"content\":\"hello\""));
        assert!(json.contains("\"role\":\"user\""));
    }

    #[test]
    fn test_datmap_to_json_escaped() {
        let mut m = DaticleMap::new();
        m.insert(dat!("content"), dat!("hello \"world\"\n"));
        let json = datmap_to_json(&m);
        assert!(json.contains("\\\"world\\\""));
        assert!(json.contains("\\n"));
    }

    #[test]
    fn test_build_request_body() {
        use rustls::crypto::ring;
        let _ = ring::default_provider().install_default();
        let tls = Arc::new(
            ClientConfig::builder()
                .dangerous()
                .with_custom_certificate_verifier(Arc::new(NoVerify))
                .with_no_client_auth()
        );
        let client = LlmClient::new("api.test.com", 443, "/v1/chat", "key", "model", 4096, tls);
        let messages = vec![
            ChatMessage::System { content: "You are helpful".to_string() },
            ChatMessage::User { content: "Hello".to_string() },
        ];
        let body = client.build_request_body(&messages);
        assert!(body.contains("\"model\":\"model\""));
        assert!(body.contains("\"stream\":true"));
        assert!(body.contains("\"role\":\"system\""));
        assert!(body.contains("\"role\":\"user\""));
        assert!(body.contains("\"content\":\"You are helpful\""));
        assert!(body.contains("\"content\":\"Hello\""));
    }

    // Test verifier that accepts any certificate (for unit tests only).
    use tokio_rustls::rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
    use std::sync::Arc;

    #[derive(Debug)]
    pub struct NoVerify;

    impl ServerCertVerifier for NoVerify {
        fn verify_server_cert(
            &self,
            _end_entity: &tokio_rustls::rustls::pki_types::CertificateDer<'_>,
            _intermediates: &[tokio_rustls::rustls::pki_types::CertificateDer<'_>],
            _server_name: &tokio_rustls::rustls::pki_types::ServerName<'_>,
            _ocsp_response: &[u8],
            _now: tokio_rustls::rustls::pki_types::UnixTime,
        ) -> Result<ServerCertVerified, tokio_rustls::rustls::Error> {
            Ok(ServerCertVerified::assertion())
        }
        fn verify_tls12_signature(
            &self,
            _message: &[u8],
            _cert: &tokio_rustls::rustls::pki_types::CertificateDer<'_>,
            _dss: &tokio_rustls::rustls::DigitallySignedStruct,
        ) -> Result<HandshakeSignatureValid, tokio_rustls::rustls::Error> {
            Ok(HandshakeSignatureValid::assertion())
        }
        fn verify_tls13_signature(
            &self,
            _message: &[u8],
            _cert: &tokio_rustls::rustls::pki_types::CertificateDer<'_>,
            _dss: &tokio_rustls::rustls::DigitallySignedStruct,
        ) -> Result<HandshakeSignatureValid, tokio_rustls::rustls::Error> {
            Ok(HandshakeSignatureValid::assertion())
        }
        fn supported_verify_schemes(&self) -> Vec<tokio_rustls::rustls::SignatureScheme> {
            vec![
                tokio_rustls::rustls::SignatureScheme::RSA_PKCS1_SHA256,
                tokio_rustls::rustls::SignatureScheme::ECDSA_NISTP256_SHA256,
                tokio_rustls::rustls::SignatureScheme::ED25519,
            ]
        }
    }
}
