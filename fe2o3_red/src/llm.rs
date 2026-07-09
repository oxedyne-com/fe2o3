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

use crate::protocol::ChatMessage;

use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
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
    pub tls_config: Arc<ClientConfig>,
}

/// The response from a completed chat call.
#[derive(Clone, Debug, Default)]
pub struct ChatResponse {
    pub content:           String,
    pub prompt_tokens:     u64,
    pub completion_tokens: u64,
}

impl LlmClient {

    pub fn new(
        host:       &str,
        port:       u16,
        path:       &str,
        api_key:    &str,
        model:      &str,
        tls_config: Arc<ClientConfig>,
    ) -> Self {
        Self {
            host:       host.to_string(),
            port,
            path:       path.to_string(),
            api_key:    api_key.to_string(),
            model:      model.to_string(),
            tls_config,
        }
    }

    /// Send a streaming chat completion request.
    ///
    /// Calls `on_token` for each text delta as it arrives from the
    /// LLM.  Returns the full accumulated response when the stream
    /// completes.
    pub async fn chat_stream(
        &self,
        messages:   &[ChatMessage],
        on_token:   &mut impl FnMut(&str),
    ) -> Outcome<ChatResponse> {
        let body = self.build_request_body(messages);
        let response_bytes = res!(self.do_request(&body).await);
        let (content, prompt_tok, completion_tok) = parse_sse_stream(&response_bytes, on_token);
        Ok(ChatResponse {
            content,
            prompt_tokens: prompt_tok,
            completion_tokens: completion_tok,
        })
    }

    /// Build the JSON request body for the OpenAI-compatible API.
    ///
    /// Format:
    /// ```json
    /// {
    ///   "model": "...",
    ///   "messages": [{"role":"...","content":"..."}, ...],
    ///   "stream": true
    /// }
    /// ```
    fn build_request_body(&self, messages: &[ChatMessage]) -> String {
        let mut out = String::with_capacity(1024);
        out.push_str("{");
        out.push_str(&fmt!("\"model\":\"{}\",", self.model));
        out.push_str("\"messages\":[");
        for (i, msg) in messages.iter().enumerate() {
            if i > 0 { out.push(','); }
            let dm = msg.to_datmap();
            out.push_str(&datmap_to_json(&dm));
        }
        out.push_str("],");
        out.push_str("\"stream\":true,");
        out.push_str("\"stream_options\":{\"include_usage\":true}");
        out.push_str("}");
        out
    }

    /// Perform the HTTPS request and return the raw response body.
    async fn do_request(&self, body: &str) -> Outcome<Vec<u8>> {
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
        // buffer so they're sent in one TLS record.  Some CDN-
        // fronted servers reject requests where headers and body
        // arrive in separate records.
        let mut req = Vec::with_capacity(request.as_bytes().len() + body_bytes.len());
        req.extend_from_slice(request.as_bytes());
        req.extend_from_slice(body_bytes);
        info!("LLM: sending {} bytes to {}:{}{}", req.len(), self.host, self.port, self.path);
        stream.write_all(&req).await
            .map_err(|e| err!(e, "LLM: write request failed."; IO, Network, Wire, Write))?;
        stream.flush().await
            .map_err(|e| err!(e, "LLM: flush failed."; IO, Network, Wire, Write))?;
        info!("LLM: request sent, waiting for response...");

        // Read entire response into buffer.
        // The response is SSE — we read it all since fe2o3_net's
        // HttpMessage::read buffers the full body.  For true
        // incremental streaming we'd need to read line-by-line,
        // but for Phase 1 reading the full response then parsing
        // is simpler and sufficient.
        let mut buf = Vec::with_capacity(8192);
        let mut chunk = [0u8; 4096];
        loop {
            match stream.read(&mut chunk).await {
                Ok(0) => {
                    info!("LLM: response complete, {} bytes received.", buf.len());
                    break;
                }
                Ok(n) => {
                    info!("LLM: read {} bytes.", n);
                    buf.extend_from_slice(&chunk[..n]);
                }
                Err(e) if e.kind() == tokio::io::ErrorKind::UnexpectedEof => {
                    // Many HTTP servers close the TCP connection
                    // without a TLS close_notify when using
                    // Connection: close.  rustls surfaces this as
                    // UnexpectedEof.  We have already received the
                    // full response body, so treat this as a
                    // graceful end-of-stream.
                    info!("LLM: peer closed (no close_notify), {} bytes received.", buf.len());
                    break;
                }
                Err(e) => {
                    info!("LLM: read error after {} bytes: {}", buf.len(), e);
                    return Err(err!(e,
                    "LLM: read response failed."; IO, Network, Wire, Read))
                }
            }
        }

        // Find the body (after \r\n\r\n).
        let body_start = buf.windows(4)
            .position(|w| w == b"\r\n\r\n")
            .map(|p| p + 4)
            .unwrap_or(0);

        // Check for chunked transfer encoding — if the response uses
        // chunked encoding, we need to dechunk it.
        let headers_str = String::from_utf8_lossy(&buf[..body_start.min(buf.len())]);
        let is_chunked = headers_str.to_ascii_lowercase().contains("transfer-encoding: chunked");

        let raw_body = &buf[body_start..];
        if is_chunked {
            Ok(dechunk(raw_body))
        } else {
            Ok(raw_body.to_vec())
        }
    }
}


// ┌───────────────────────────────────────────────────────────────┐
// │ SSE parsing                                                    │
// └───────────────────────────────────────────────────────────────┘

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
    let needle = fmt!("\"{}\":{{", key);
    let pos = json.find(&needle)?;
    let start = pos + needle.len() - 1; // position of the opening brace
    let bytes = json.as_bytes();
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
    let pos = json.find(&needle)?;
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
fn extract_json_string(json: &str, key: &str) -> Option<String> {
    let needle = fmt!("\"{}\":\"", key);
    let mut search_from = 0;
    loop {
        let pos = match json[search_from..].find(&needle) {
            Some(p) => search_from + p,
            None => return None,
        };
        // Verify this is a complete JSON key by checking the
        // character before the opening quote.  It must be `{`, `,`,
        // or whitespace — not a letter (which would mean it's part
        // of a longer key like "reasoning_content").
        if pos == 0 {
            // Start of string — valid only if the string starts
            // with the key, which is unusual but acceptable.
        } else {
            let prev = json.as_bytes()[pos - 1];
            if prev != b'{' && prev != b',' && !prev.is_ascii_whitespace() {
                // Part of a longer key (e.g. "reasoning_content").
                search_from = pos + needle.len();
                continue;
            }
        }
        let start = pos + needle.len();
        let bytes = json.as_bytes();
        let mut result = String::new();
        let mut i = start;
        while i < bytes.len() {
            let b = bytes[i];
            if b == b'\\' && i + 1 < bytes.len() {
                match bytes[i + 1] {
                    b'"' => result.push('"'),
                    b'\\' => result.push('\\'),
                    b'n' => result.push('\n'),
                    b't' => result.push('\t'),
                    b'r' => result.push('\r'),
                    b'/' => result.push('/'),
                    _ => {
                        result.push('\\');
                        result.push(bytes[i + 1] as char);
                    }
                }
                i += 2;
            } else if b == b'"' {
                return Some(result);
            } else {
                result.push(b as char);
                i += 1;
            }
        }
        return None;
    }
}

/// Dechunk an HTTP chunked transfer-encoded body.
///
/// Format: `<hex-size>\r\n<data>\r\n` repeated, ending with `0\r\n\r\n`.
fn dechunk(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len());
    let mut pos = 0;
    while pos < data.len() {
        // Find the chunk size line.
        let line_end = match data[pos..].windows(2).position(|w| w == b"\r\n") {
            Some(p) => pos + p,
            None => break,
        };
        let size_str = String::from_utf8_lossy(&data[pos..line_end]);
        let size = match usize::from_str_radix(size_str.trim(), 16) {
            Ok(n) => n,
            Err(_) => break,
        };
        if size == 0 {
            break;
        }
        let data_start = line_end + 2;
        let data_end = data_start + size;
        if data_end > data.len() {
            break;
        }
        out.extend_from_slice(&data[data_start..data_end]);
        pos = data_end + 2; // skip \r\n after chunk data
    }
    out
}


/// Convert a JDAT DaticleMap to a minimal JSON string.
///
/// This is used to build the LLM API request body without `serde`.
/// Only handles the types we need: String, U64, Bool, Map, List.
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
        let full = parse_sse_stream(sse.as_bytes(), &mut |t| tokens.push(t.to_string()));
        assert_eq!(tokens, vec!["Hello", " world"]);
        assert_eq!(full, "Hello world");
    }

    #[test]
    fn test_parse_sse_empty_lines() {
        let sse = "\r\ndata: {\"choices\":[{\"delta\":{\"content\":\"Hi\"}}]}\r\n\r\ndata: [DONE]\r\n";
        let mut tokens = Vec::new();
        let full = parse_sse_stream(sse.as_bytes(), &mut |t| tokens.push(t.to_string()));
        assert_eq!(tokens, vec!["Hi"]);
        assert_eq!(full, "Hi");
    }

    #[test]
    fn test_dechunk() {
        // 5 bytes of "Hello", then 0 (end).
        let chunked = b"5\r\nHello\r\n0\r\n\r\n";
        assert_eq!(dechunk(chunked), b"Hello");
    }

    #[test]
    fn test_dechunk_multiple() {
        let chunked = b"5\r\nHello\r\n6\r\n world\r\n0\r\n\r\n";
        assert_eq!(dechunk(chunked), b"Hello world");
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
        let client = LlmClient::new("api.test.com", 443, "/v1/chat", "key", "model", tls);
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
