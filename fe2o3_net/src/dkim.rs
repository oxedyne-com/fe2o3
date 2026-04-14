//! DKIM (DomainKeys Identified Mail) signer.
//!
//! Implements the slice of RFC 6376 + RFC 8463 needed to sign outbound
//! messages with the **ed25519-sha256** algorithm using `relaxed/relaxed`
//! canonicalisation. Verification is intentionally not implemented:
//! Hematite's first deployment only signs outbound mail, it does not
//! filter inbound mail by DKIM.
//!
//! Why ed25519 rather than RSA? Ring (the only crypto crate already in
//! Hematite's tree) refuses to generate RSA keys on the grounds that
//! the operation is dangerous and best left to dedicated tools. Adding
//! a third-party RSA key-gen crate would violate the no-deps rule. The
//! ed25519 path keeps the entire signer in-tree, and every major
//! receiver (Gmail, Outlook, Yahoo, Fastmail, Apple) has supported
//! ed25519-sha256 DKIM since 2019.
//!
//! The output is the input message with a single `DKIM-Signature:`
//! header field prepended. The original CRLF line-ending convention is
//! preserved.

use oxedyne_fe2o3_core::prelude::*;

use base64;
use ring::{
    digest::{
        digest as sha,
        SHA256,
    },
    rand::SystemRandom,
    signature::{
        Ed25519KeyPair,
        KeyPair,
    },
};


/// Default header set the signer covers, in the order listed by
/// [`DkimSigner::sign`]. Mirrors the "well-known" minimum every
/// reputable DKIM implementation oversigns.
pub const DEFAULT_SIGNED_HEADERS: &[&str] = &[
    "From",
    "To",
    "Cc",
    "Subject",
    "Date",
    "Message-ID",
    "Reply-To",
    "MIME-Version",
    "Content-Type",
    "Content-Transfer-Encoding",
];


/// One DKIM signing identity.
///
/// Owns a live `Ed25519KeyPair` plus the PKCS#8 bytes the key was
/// loaded from (so the signer can be persisted and reloaded), the
/// signing domain, and the selector under which the corresponding
/// public key is published in DNS.
pub struct DkimSigner {
    pkcs8:      Vec<u8>,
    key_pair:   Ed25519KeyPair,
    domain:     String,
    selector:   String,
}

impl std::fmt::Debug for DkimSigner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DkimSigner")
            .field("pkcs8",     &"<redacted>")
            .field("key_pair",  &"<redacted>")
            .field("domain",    &self.domain)
            .field("selector",  &self.selector)
            .finish()
    }
}

impl DkimSigner {
    /// Generate a fresh ed25519 key pair for `domain` published under
    /// `selector`. The resulting signer can be serialised to disk via
    /// [`DkimSigner::pkcs8_bytes`] and reloaded with
    /// [`DkimSigner::from_pkcs8`].
    pub fn generate(domain: impl Into<String>, selector: impl Into<String>) -> Outcome<Self> {
        let rng = SystemRandom::new();
        let pkcs8 = match Ed25519KeyPair::generate_pkcs8(&rng) {
            Ok(doc) => doc.as_ref().to_vec(),
            Err(_) => return Err(err!(
                "Ed25519KeyPair::generate_pkcs8 failed.";
                Init, Unknown)),
        };
        Self::from_pkcs8(&pkcs8, domain, selector)
    }

    /// Load an existing ed25519 key pair from its PKCS#8 encoding.
    pub fn from_pkcs8(
        pkcs8:      &[u8],
        domain:     impl Into<String>,
        selector:   impl Into<String>,
    )
        -> Outcome<Self>
    {
        let key_pair = match Ed25519KeyPair::from_pkcs8(pkcs8) {
            Ok(kp) => kp,
            Err(e) => return Err(err!(
                "Ed25519KeyPair::from_pkcs8 rejected the supplied bytes: {}.", e;
                Init, Invalid, Input)),
        };
        Ok(Self {
            pkcs8: pkcs8.to_vec(),
            key_pair,
            domain: domain.into(),
            selector: selector.into(),
        })
    }

    /// PKCS#8 serialisation of the private key, for on-disk persistence.
    pub fn pkcs8_bytes(&self) -> &[u8] { &self.pkcs8 }

    /// Domain this signer signs for.
    pub fn domain(&self) -> &str { &self.domain }

    /// Selector under which the public key is (or should be) published
    /// at `<selector>._domainkey.<domain>` in DNS.
    pub fn selector(&self) -> &str { &self.selector }

    /// Render the DNS TXT record value to publish at
    /// `<selector>._domainkey.<domain>`. The result is the full
    /// `v=DKIM1; k=ed25519; p=<base64>` string (no quoting).
    pub fn dns_txt_record(&self) -> String {
        let pk = self.key_pair.public_key().as_ref();
        fmt!("v=DKIM1; k=ed25519; p={}", base64::encode(pk))
    }

    /// Sign `message` and return a fresh buffer with the
    /// `DKIM-Signature:` header prepended. The original message is not
    /// mutated. `headers_to_sign` is the ordered list of header field
    /// names to cover; if empty, [`DEFAULT_SIGNED_HEADERS`] is used.
    pub fn sign(
        &self,
        message:            &[u8],
        headers_to_sign:    &[&str],
        timestamp:          u64,
    )
        -> Outcome<Vec<u8>>
    {
        let names: Vec<&str> = if headers_to_sign.is_empty() {
            DEFAULT_SIGNED_HEADERS.to_vec()
        } else {
            headers_to_sign.to_vec()
        };

        // Split message into headers and body.
        let (raw_headers, body) = res!(split_headers_body(message));
        let parsed_headers = parse_header_block(raw_headers);

        // Body canonicalisation + hash.
        let body_canon = canonicalise_body_relaxed(body);
        let body_hash = sha(&SHA256, &body_canon);
        let bh_b64 = base64::encode(body_hash.as_ref());

        // Pick the headers to sign in declaration order. For each name,
        // include only the *last* occurrence (the convention used by
        // OpenDKIM and rspamd; oversigning is left for v2).
        let mut covered: Vec<(&str, &str)> = Vec::new();
        let mut covered_names_lc: Vec<String> = Vec::new();
        for name in &names {
            let lc = name.to_lowercase();
            // Find the last header with this name.
            if let Some((_, value)) = parsed_headers.iter().rev()
                .find(|(n, _)| n.eq_ignore_ascii_case(name))
            {
                covered.push((name, value.as_str()));
                covered_names_lc.push(lc);
            }
        }

        // Build the DKIM-Signature header value with an empty `b=`. We
        // assemble the unfolded form first because that is what the
        // canonicaliser sees; we will re-fold for output.
        let h_tag = covered.iter().map(|(n, _)| *n).collect::<Vec<_>>().join(":");
        let dkim_value_no_b = fmt!(
            "v=1; a=ed25519-sha256; c=relaxed/relaxed; d={}; s={}; t={}; \
             bh={}; h={}; b=",
            self.domain,
            self.selector,
            timestamp,
            bh_b64,
            h_tag,
        );

        // Canonicalise each covered header relaxed-style, then append
        // the DKIM-Signature itself (also relaxed, with empty b=) and
        // *no* trailing CRLF on the DKIM-Signature line.
        let mut canon = String::new();
        for (name, value) in &covered {
            canon.push_str(&relaxed_header(name, value));
        }
        canon.push_str("dkim-signature:");
        canon.push_str(&relaxed_value(&dkim_value_no_b));
        // No CRLF on the DKIM-Signature line per RFC 6376 §3.7.

        let sig = self.key_pair.sign(canon.as_bytes());
        let b_b64 = base64::encode(sig.as_ref());

        // Assemble the final DKIM-Signature header line, folded so no
        // single line exceeds 78 characters where reasonable.
        let final_value = fmt!(
            "v=1; a=ed25519-sha256; c=relaxed/relaxed; d={}; s={}; t={};\r\n\
             \tbh={};\r\n\
             \th={};\r\n\
             \tb={}",
            self.domain,
            self.selector,
            timestamp,
            bh_b64,
            h_tag,
            b_b64,
        );
        let header_line = fmt!("DKIM-Signature: {}\r\n", final_value);

        // Prepend.
        let mut out = Vec::with_capacity(header_line.len() + message.len());
        out.extend_from_slice(header_line.as_bytes());
        out.extend_from_slice(message);
        // Suppress the unused warning on covered_names_lc in case the
        // future verification path depends on it.
        let _ = covered_names_lc;
        Ok(out)
    }
}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ MESSAGE PARSING + CANONICALISATION                                        │
// └───────────────────────────────────────────────────────────────────────────┘

/// Split a raw RFC 5322 message at the first blank line. Returns
/// `(headers, body)`. The `headers` slice excludes the blank line that
/// terminates the header block; the `body` slice is the rest of the
/// buffer untouched.
fn split_headers_body(message: &[u8]) -> Outcome<(&[u8], &[u8])> {
    // Look for "\r\n\r\n" first; tolerate "\n\n" as a fallback.
    if let Some(i) = find_subseq(message, b"\r\n\r\n") {
        return Ok((&message[..i], &message[i + 4..]));
    }
    if let Some(i) = find_subseq(message, b"\n\n") {
        return Ok((&message[..i], &message[i + 2..]));
    }
    // No blank line: treat the whole thing as headers (body empty).
    Ok((message, &[]))
}

/// Naive substring search.
fn find_subseq(hay: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || needle.len() > hay.len() { return None; }
    for i in 0..=hay.len() - needle.len() {
        if &hay[i..i + needle.len()] == needle {
            return Some(i);
        }
    }
    None
}

/// Parse a header block into `(name, unfolded_value)` pairs in the
/// order they appeared. Continuation lines (starting with WSP) are
/// joined onto the previous value with their leading WSP preserved --
/// canonicalisation collapses it later.
fn parse_header_block(headers: &[u8]) -> Vec<(String, String)> {
    let text = String::from_utf8_lossy(headers);
    let mut out: Vec<(String, String)> = Vec::new();
    let mut name = String::new();
    let mut value = String::new();
    let mut have_current = false;
    for line in text.split('\n') {
        // Strip a trailing CR if present.
        let line = line.strip_suffix('\r').unwrap_or(line);
        if line.is_empty() {
            continue;
        }
        if line.starts_with(' ') || line.starts_with('\t') {
            // Continuation.
            if have_current {
                value.push_str(line);
            }
            continue;
        }
        // Flush previous.
        if have_current {
            out.push((std::mem::take(&mut name), std::mem::take(&mut value)));
        }
        if let Some(i) = line.find(':') {
            name = line[..i].trim().to_string();
            value = line[i + 1..].to_string();
            have_current = true;
        } else {
            have_current = false;
        }
    }
    if have_current {
        out.push((name, value));
    }
    out
}

/// Apply relaxed body canonicalisation (RFC 6376 §3.4.4).
fn canonicalise_body_relaxed(body: &[u8]) -> Vec<u8> {
    let text = String::from_utf8_lossy(body);
    let mut out: Vec<String> = Vec::new();
    for raw_line in text.split('\n') {
        let line = raw_line.strip_suffix('\r').unwrap_or(raw_line);
        // Collapse runs of WSP to one SP.
        let mut collapsed = String::with_capacity(line.len());
        let mut in_ws = false;
        for ch in line.chars() {
            if ch == ' ' || ch == '\t' {
                if !in_ws {
                    collapsed.push(' ');
                    in_ws = true;
                }
            } else {
                collapsed.push(ch);
                in_ws = false;
            }
        }
        // Strip trailing WSP.
        while collapsed.ends_with(' ') {
            collapsed.pop();
        }
        out.push(collapsed);
    }
    // Trim trailing empty lines.
    while out.last().map(|s| s.is_empty()).unwrap_or(false) {
        out.pop();
    }
    let mut bytes = Vec::new();
    for (i, line) in out.iter().enumerate() {
        if i > 0 {
            bytes.extend_from_slice(b"\r\n");
        }
        bytes.extend_from_slice(line.as_bytes());
    }
    if !bytes.is_empty() {
        bytes.extend_from_slice(b"\r\n");
    }
    bytes
}

/// Canonicalise one header field as `lcname:relaxedvalue\r\n` per
/// RFC 6376 §3.4.2 relaxed.
fn relaxed_header(name: &str, value: &str) -> String {
    fmt!("{}:{}\r\n", name.to_lowercase(), relaxed_value(value))
}

/// Apply the relaxed value transform (collapse WSP, strip leading and
/// trailing WSP, unfold). Returns the canonicalised value with no
/// leading or trailing whitespace.
fn relaxed_value(value: &str) -> String {
    // Unfold: replace every CRLF (or bare LF) followed by WSP with a
    // single SP, then collapse all runs of WSP to one SP.
    let unfolded = value.replace("\r\n", "\n");
    let mut out = String::with_capacity(unfolded.len());
    let mut prev_ws = false;
    for ch in unfolded.chars() {
        if ch == '\n' {
            // Treat raw line breaks as a folding boundary -- collapse
            // to SP.
            if !prev_ws {
                out.push(' ');
                prev_ws = true;
            }
        } else if ch == ' ' || ch == '\t' {
            if !prev_ws {
                out.push(' ');
                prev_ws = true;
            }
        } else {
            out.push(ch);
            prev_ws = false;
        }
    }
    out.trim().to_string()
}
