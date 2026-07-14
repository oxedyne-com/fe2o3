//! DKIM (DomainKeys Identified Mail) signer.
//!
//! Implements the slice of RFC 6376 + RFC 8463 needed to sign outbound
//! messages with **ed25519-sha256** or **rsa-sha256**, using
//! `relaxed/relaxed` canonicalisation. Verification is intentionally not
//! implemented: Hematite signs outbound mail, it does not filter inbound
//! mail by DKIM.
//!
//! Sign with **both**, under two selectors. RFC 8463 §5 says a signer SHOULD,
//! and the reason is practical: ed25519 verification is still patchy in the
//! wild, and a receiver that cannot verify a signature sees an *unsigned*
//! message, leaving DMARC to rest on SPF alone. RSA is understood by
//! everybody. Two signatures cost a few hundred bytes and let each receiver
//! take whichever it knows. See [`DkimKey`] for why the RSA key is loaded
//! rather than generated.
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
        RsaKeyPair,
        RSA_PKCS1_SHA256,
    },
};


/// The signing algorithm behind a [`DkimSigner`].
///
/// # Why both
///
/// ed25519 signatures (RFC 8463) are small and the key generation is in
/// tree, but verification of them is still patchy in the wild -- Microsoft
/// notably. A receiver that cannot verify the signature sees an *unsigned*
/// message, and DMARC then rests entirely on SPF. RSA is verified by
/// everybody. RFC 8463 §5 says a signer SHOULD publish both and sign with
/// both, and that is what Steel does: two selectors, two signatures, and a
/// receiver takes whichever it understands.
///
/// # Why RSA is loaded, never generated
///
/// `ring` deliberately refuses to *generate* RSA keys -- it takes the view
/// that key generation is dangerous and belongs in dedicated tools -- but it
/// signs with an existing one perfectly well. So the key is generated once,
/// offline, with the `openssl` command line:
///
/// ```text
/// openssl genpkey -algorithm RSA -pkeyopt rsa_keygen_bits:2048 \
///     -outform DER -out dkim_rsa.key
/// ```
///
/// and Steel loads it. The alternative -- implementing RSA signing on top of
/// a bignum -- means hand-writing a modular exponentiation over a private
/// exponent, which is precisely the code that leaks its secret through
/// timing if you get it wrong. Loading an audited implementation is not a
/// compromise of the no-dependency rule: `ring` is already in the tree, and
/// is where every other primitive here comes from.
pub enum DkimKey {
    /// ed25519-sha256 (RFC 8463). Generated in tree.
    Ed25519(Ed25519KeyPair),
    /// rsa-sha256 (RFC 6376). Loaded; see the type documentation.
    Rsa(Box<RsaKeyPair>),
}

impl DkimKey {
    /// The value of the DKIM `a=` tag for this key.
    pub fn algorithm(&self) -> &'static str {
        match self {
            Self::Ed25519(_) => "ed25519-sha256",
            Self::Rsa(_)     => "rsa-sha256",
        }
    }

    /// The value of the DNS `k=` tag for this key.
    pub fn key_type(&self) -> &'static str {
        match self {
            Self::Ed25519(_) => "ed25519",
            Self::Rsa(_)     => "rsa",
        }
    }
}


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
    key:        DkimKey,
    domain:     String,
    selector:   String,
}

impl std::fmt::Debug for DkimSigner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DkimSigner")
            .field("pkcs8",     &"<redacted>")
            .field("key",       &self.key.algorithm())
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
    ///
    /// There is no RSA equivalent, because `ring` will not generate RSA
    /// keys. Generate one offline with `openssl` and load it -- see
    /// [`DkimKey`].
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

    /// Load a private key and work out what it is.
    ///
    /// Accepts an ed25519 PKCS#8 key, an RSA PKCS#8 key, or a bare PKCS#1
    /// RSA key, and selects the signing algorithm accordingly. The operator
    /// points the config at a key file; they should not also have to tell
    /// Steel what kind of key they just gave it, when the bytes say so.
    pub fn from_pkcs8(
        pkcs8:      &[u8],
        domain:     impl Into<String>,
        selector:   impl Into<String>,
    )
        -> Outcome<Self>
    {
        let key = if let Ok(kp) = Ed25519KeyPair::from_pkcs8(pkcs8) {
            DkimKey::Ed25519(kp)
        } else if let Ok(kp) = RsaKeyPair::from_pkcs8(pkcs8) {
            DkimKey::Rsa(Box::new(kp))
        } else if let Ok(kp) = RsaKeyPair::from_der(pkcs8) {
            // A bare PKCS#1 RSAPrivateKey, which is what older openssl
            // invocations emit.
            DkimKey::Rsa(Box::new(kp))
        } else {
            return Err(err!(
                "The supplied {} bytes are not an ed25519 PKCS#8 key, an RSA \
                PKCS#8 key, or a PKCS#1 RSA key. Generate an RSA DKIM key with \
                `openssl genpkey -algorithm RSA -pkeyopt rsa_keygen_bits:2048 \
                -outform DER -out dkim_rsa.key`.",
                pkcs8.len();
                Init, Invalid, Input));
        };
        Ok(Self {
            pkcs8: pkcs8.to_vec(),
            key,
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

    /// The signing algorithm, as it appears in the `a=` tag.
    pub fn algorithm(&self) -> &'static str { self.key.algorithm() }

    /// Render the DNS TXT record value to publish at
    /// `<selector>._domainkey.<domain>`. The result is the full
    /// `v=DKIM1; k=<type>; p=<base64>` string (no quoting).
    ///
    /// For RSA the published key is a `SubjectPublicKeyInfo`, which is what
    /// RFC 6376 calls for and what `openssl rsa -pubout` emits. `ring` hands
    /// back the bare PKCS#1 `RSAPublicKey`, so it is wrapped here.
    pub fn dns_txt_record(&self) -> String {
        let (k, p) = match &self.key {
            DkimKey::Ed25519(kp) => (
                "ed25519",
                base64::encode(kp.public_key().as_ref()),
            ),
            DkimKey::Rsa(kp) => (
                "rsa",
                base64::encode(rsa_spki_der(kp.public_key().as_ref())),
            ),
        };
        fmt!("v=DKIM1; k={}; p={}", k, p)
    }

    /// Sign the canonicalised bytes with whichever key this signer holds.
    fn sign_canonical(&self, canon: &[u8]) -> Outcome<Vec<u8>> {
        match &self.key {
            DkimKey::Ed25519(kp) => Ok(kp.sign(canon).as_ref().to_vec()),
            DkimKey::Rsa(kp) => {
                let rng = SystemRandom::new();
                let mut sig = vec![0u8; kp.public_modulus_len()];
                match kp.sign(&RSA_PKCS1_SHA256, &rng, canon, &mut sig) {
                    Ok(()) => Ok(sig),
                    Err(_) => Err(err!(
                        "RSA signing failed for DKIM selector '{}' on domain \
                        '{}'.", self.selector, self.domain;
                        Encrypt, Invalid)),
                }
            }
        }
    }

    /// Sign `message` and return a fresh buffer with the
    /// `DKIM-Signature:` header prepended. The original message is not
    /// mutated. `headers_to_sign` is the ordered list of header field
    /// names to cover; if empty, [`DEFAULT_SIGNED_HEADERS`] is used.
    /// The exact bytes this signer will sign: the canonicalised header block
    /// with the `DKIM-Signature` field appended, its `b=` tag empty.
    ///
    /// Public because when a receiver rejects a signature, the only question
    /// worth asking is what was actually signed, and without this the answer
    /// is buried. Also what the tests hand to an independent verifier.
    pub fn signing_input(
        &self,
        message:            &[u8],
        headers_to_sign:    &[&str],
        timestamp:          u64,
    )
        -> Outcome<String>
    {
        let (canon, _) = res!(self.prepare(message, headers_to_sign, timestamp));
        Ok(canon)
    }

    /// Canonicalise, returning the signing input and the pieces the header
    /// line is built from: `(canon, (bh_b64, h_tag))`.
    fn prepare(
        &self,
        message:            &[u8],
        headers_to_sign:    &[&str],
        timestamp:          u64,
    )
        -> Outcome<(String, (String, String))>
    {
        let names: Vec<&str> = if headers_to_sign.is_empty() {
            DEFAULT_SIGNED_HEADERS.to_vec()
        } else {
            headers_to_sign.to_vec()
        };

        let (raw_headers, body) = res!(split_headers_body(message));
        let parsed_headers = parse_header_block(raw_headers);

        let body_canon = canonicalise_body_relaxed(body);
        let body_hash = sha(&SHA256, &body_canon);
        let bh_b64 = base64::encode(body_hash.as_ref());

        let mut covered: Vec<(&str, &str)> = Vec::new();
        for name in &names {
            if let Some((_, value)) = parsed_headers.iter().rev()
                .find(|(n, _)| n.eq_ignore_ascii_case(name))
            {
                covered.push((name, value.as_str()));
            }
        }

        let algo = self.key.algorithm();
        let h_tag = covered.iter().map(|(n, _)| *n).collect::<Vec<_>>().join(":");
        let dkim_value_no_b = fmt!(
            "v=1; a={}; c=relaxed/relaxed; d={}; s={}; t={}; \
             bh={}; h={}; b=",
            algo,
            self.domain,
            self.selector,
            timestamp,
            bh_b64,
            h_tag,
        );

        let mut canon = String::new();
        for (name, value) in &covered {
            canon.push_str(&relaxed_header(name, value));
        }
        canon.push_str("dkim-signature:");
        canon.push_str(&relaxed_value(&dkim_value_no_b));
        // No CRLF on the DKIM-Signature line per RFC 6376 §3.7.

        Ok((canon, (bh_b64, h_tag)))
    }

    pub fn sign(
        &self,
        message:            &[u8],
        headers_to_sign:    &[&str],
        timestamp:          u64,
    )
        -> Outcome<Vec<u8>>
    {
        let (canon, (bh_b64, h_tag)) =
            res!(self.prepare(message, headers_to_sign, timestamp));
        let algo = self.key.algorithm();

        let sig = res!(self.sign_canonical(canon.as_bytes()));
        let b_b64 = base64::encode(&sig);

        // Assemble the final DKIM-Signature header line, folded so no
        // single line exceeds 78 characters where reasonable.
        let final_value = fmt!(
            "v=1; a={}; c=relaxed/relaxed; d={}; s={}; t={};\r\n\
             \tbh={};\r\n\
             \th={};\r\n\
             \tb={}",
            algo,
            self.domain,
            self.selector,
            timestamp,
            bh_b64,
            h_tag,
            b_b64,
        );
        let header_line = fmt!("DKIM-Signature: {}\r\n", final_value);

        // Prepend. The message itself is untouched, and a second signer may
        // prepend its own header to this output: the covered headers and the
        // body are unchanged, so the two signatures are independent.
        let mut out = Vec::with_capacity(header_line.len() + message.len());
        out.extend_from_slice(header_line.as_bytes());
        out.extend_from_slice(message);
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


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ RSA PUBLIC KEY ENCODING                                                   │
// └───────────────────────────────────────────────────────────────────────────┘

/// DER `AlgorithmIdentifier` for `rsaEncryption` with the ASN.1 NULL
/// parameter: `SEQUENCE { OID 1.2.840.113549.1.1.1, NULL }`. Fixed bytes.
const RSA_ALG_ID_DER: [u8; 15] = [
    0x30, 0x0d,                                             // SEQUENCE, 13 bytes
    0x06, 0x09, 0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d,         // OID rsaEncryption
    0x01, 0x01, 0x01,
    0x05, 0x00,                                             // NULL
];

/// Wrap a PKCS#1 `RSAPublicKey` in a `SubjectPublicKeyInfo`.
///
/// DKIM publishes the RSA public key as a `SubjectPublicKeyInfo` (RFC 6376
/// §3.6.1, by reference to RFC 5280) -- the same thing `openssl rsa -pubout`
/// writes. `ring` hands back the bare PKCS#1 `RSAPublicKey`, which is the
/// inner `SEQUENCE { INTEGER n, INTEGER e }` and nothing else, so publishing
/// it as-is yields a record every verifier rejects.
///
/// ```text
/// SubjectPublicKeyInfo ::= SEQUENCE {
///     algorithm           AlgorithmIdentifier,    -- rsaEncryption, NULL
///     subjectPublicKey    BIT STRING              -- the RSAPublicKey DER
/// }
/// ```
pub fn rsa_spki_der(pkcs1: &[u8]) -> Vec<u8> {
    // BIT STRING: tag, length, and a leading octet giving the number of
    // unused bits in the final octet -- always zero for a whole-byte payload.
    let mut bit_string = Vec::with_capacity(pkcs1.len() + 8);
    bit_string.push(0x03);
    der_write_len(&mut bit_string, pkcs1.len() + 1);
    bit_string.push(0x00);
    bit_string.extend_from_slice(pkcs1);

    let body_len = RSA_ALG_ID_DER.len() + bit_string.len();
    let mut out = Vec::with_capacity(body_len + 8);
    out.push(0x30);
    der_write_len(&mut out, body_len);
    out.extend_from_slice(&RSA_ALG_ID_DER);
    out.extend_from_slice(&bit_string);
    out
}

/// Append a DER definite-form length.
///
/// Lengths below 128 are a single byte. Anything larger is the long form: a
/// leading byte carrying the count of length octets with the high bit set,
/// then the length itself, big-endian and minimally encoded. A 2048-bit key
/// needs the long form, so getting this wrong is not a corner case.
fn der_write_len(out: &mut Vec<u8>, len: usize) {
    if len < 0x80 {
        out.push(len as u8);
        return;
    }
    let mut be = Vec::with_capacity(8);
    let mut n = len;
    while n > 0 {
        be.push((n & 0xff) as u8);
        n >>= 8;
    }
    be.reverse();
    out.push(0x80 | (be.len() as u8));
    out.extend_from_slice(&be);
}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ TESTS                                                                     │
// └───────────────────────────────────────────────────────────────────────────┘

#[cfg(test)]
mod tests {
    use super::*;

    /// A 2048-bit RSA key in PKCS#8 DER, generated once with:
    /// `openssl genpkey -algorithm RSA -pkeyopt rsa_keygen_bits:2048 \
    ///     -outform DER | base64 -w0`
    /// A test key and nothing else: it signs nothing that exists.
    const TEST_RSA_PKCS8_B64: &str = include_str!("../tests/data/dkim_rsa_test_key.b64");

    fn rsa_signer() -> DkimSigner {
        let der = match base64::decode(TEST_RSA_PKCS8_B64.trim()) {
            Ok(d) => d,
            Err(e) => panic!("decoding the test key: {}", e),
        };
        match DkimSigner::from_pkcs8(&der, "example.com", "rsa1") {
            Ok(s) => s,
            Err(e) => panic!("loading the test key: {}", e),
        }
    }

    fn message() -> Vec<u8> {
        let m = "From: sender@example.com\r\n\
                 To: rcpt@elsewhere.example\r\n\
                 Subject: a test\r\n\
                 Date: Mon, 14 Jul 2026 12:00:00 +0000\r\n\
                 \r\n\
                 Hello.\r\n";
        m.as_bytes().to_vec()
    }

    /// An RSA key must be recognised as one, and produce an rsa-sha256
    /// signature rather than quietly signing with something else.
    #[test]
    fn test_an_rsa_key_signs_rsa_sha256_00() {
        let s = rsa_signer();
        assert_eq!(s.algorithm(), "rsa-sha256");
        let signed = match s.sign(&message(), &[], 1_784_000_000) {
            Ok(b) => b,
            Err(e) => panic!("signing: {}", e),
        };
        let text = String::from_utf8_lossy(&signed);
        assert!(text.starts_with("DKIM-Signature: "),
            "the signature header must be prepended");
        assert!(text.contains("a=rsa-sha256"),
            "the a= tag must name the algorithm actually used:\n{}", text);
        assert!(text.contains("s=rsa1") && text.contains("d=example.com"));
        // The original message must survive untouched below the header.
        assert!(text.contains("\r\nHello.\r\n"));
    }

    /// An ed25519 key must still sign ed25519 -- the algorithm follows the
    /// key, and adding RSA must not have quietly changed the existing path.
    #[test]
    fn test_an_ed25519_key_still_signs_ed25519_00() {
        let s = match DkimSigner::generate("example.com", "ed1") {
            Ok(s) => s,
            Err(e) => panic!("generate: {}", e),
        };
        assert_eq!(s.algorithm(), "ed25519-sha256");
        let signed = match s.sign(&message(), &[], 1_784_000_000) {
            Ok(b) => b,
            Err(e) => panic!("signing: {}", e),
        };
        let text = String::from_utf8_lossy(&signed);
        assert!(text.contains("a=ed25519-sha256"));
        assert!(s.dns_txt_record().starts_with("v=DKIM1; k=ed25519; p="));
    }

    /// The published RSA record must be a SubjectPublicKeyInfo, because that
    /// is what every verifier parses. Publishing ring's bare PKCS#1 key would
    /// yield a record that looks fine and that nothing can read.
    #[test]
    fn test_the_rsa_record_publishes_a_subject_public_key_info_00() {
        let s = rsa_signer();
        let rec = s.dns_txt_record();
        assert!(rec.starts_with("v=DKIM1; k=rsa; p="), "got: {}", rec);
        let p = match rec.split("p=").nth(1) {
            Some(p) => p,
            None => panic!("no p= tag"),
        };
        let der = match base64::decode(p) {
            Ok(d) => d,
            Err(e) => panic!("p= is not base64: {}", e),
        };
        // A SubjectPublicKeyInfo is a SEQUENCE whose first element is the
        // rsaEncryption AlgorithmIdentifier. A bare PKCS#1 RSAPublicKey would
        // begin SEQUENCE, INTEGER (0x02) instead.
        assert_eq!(der[0], 0x30, "SubjectPublicKeyInfo must be a SEQUENCE");
        assert!(der.windows(RSA_ALG_ID_DER.len())
            .any(|w| w == RSA_ALG_ID_DER),
            "the rsaEncryption AlgorithmIdentifier is missing: this is a bare \
            PKCS#1 key, which no verifier will read");
    }

    /// The signature must verify against an *independent* implementation,
    /// using the public key exactly as Steel publishes it. A signer that is
    /// merely self-consistent -- one whose own code agrees with itself -- can
    /// still emit something every receiver on earth rejects, and DKIM fails
    /// silently: the mail is simply treated as unsigned.
    ///
    /// So: sign here, then hand openssl the canonical input, the signature,
    /// and the SubjectPublicKeyInfo from the DNS record, and make it agree.
    #[test]
    fn test_the_rsa_signature_verifies_under_openssl_00() {
        use std::io::Write;
        use std::process::Command;

        let s = rsa_signer();
        let msg = message();
        let canon = match s.signing_input(&msg, &[], 1_784_000_000) {
            Ok(c) => c,
            Err(e) => panic!("canonicalising: {}", e),
        };
        let signed = match s.sign(&msg, &[], 1_784_000_000) {
            Ok(b) => b,
            Err(e) => panic!("signing: {}", e),
        };

        // Pull b= back out of the header we just wrote, unfolding it.
        let text = String::from_utf8_lossy(&signed);
        let b_tag = match text.split("b=").nth(1) {
            Some(t) => t,
            None => panic!("no b= tag in:\n{}", text),
        };
        let b64: String = b_tag.chars()
            .take_while(|c| *c != '\r' && *c != '\n')
            .filter(|c| !c.is_whitespace())
            .collect();
        let sig = match base64::decode(&b64) {
            Ok(v) => v,
            Err(e) => panic!("b= is not base64 ({}): {:?}", e, b64),
        };

        // The public key, exactly as it goes into DNS.
        let rec = s.dns_txt_record();
        let p = match rec.split("p=").nth(1) {
            Some(p) => p,
            None => panic!("no p="),
        };
        let spki = match base64::decode(p) {
            Ok(d) => d,
            Err(e) => panic!("p= is not base64: {}", e),
        };

        let dir = std::env::temp_dir().join("fe2o3_dkim_openssl_test");
        let _ = std::fs::create_dir_all(&dir);
        let write = |name: &str, bytes: &[u8]| -> std::path::PathBuf {
            let path = dir.join(name);
            match std::fs::File::create(&path)
                .and_then(|mut f| f.write_all(bytes))
            {
                Ok(()) => (),
                Err(e) => panic!("writing {}: {}", name, e),
            }
            path
        };
        let key_path  = write("pub.der",  &spki);
        let sig_path  = write("sig.bin",  &sig);
        let data_path = write("data.txt", canon.as_bytes());

        let out = Command::new("openssl")
            .arg("dgst").arg("-sha256")
            .arg("-verify").arg(&key_path)
            .arg("-keyform").arg("DER")
            .arg("-signature").arg(&sig_path)
            .arg(&data_path)
            .output();
        let out = match out {
            Ok(o) => o,
            Err(e) => panic!("openssl not runnable: {}", e),
        };
        let stdout = String::from_utf8_lossy(&out.stdout);
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(stdout.contains("Verified OK"),
            "openssl refused the signature.\nstdout: {}\nstderr: {}\n\
            canonical input was:\n{:?}", stdout, stderr, canon);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Dual signing: an ed25519 signature and an RSA signature on the same
    /// message must each stand on its own.
    ///
    /// The second signer runs over the output of the first, which already
    /// carries a `DKIM-Signature` header. That is only safe because a
    /// DKIM-Signature field is not among the covered headers and the body is
    /// untouched -- so the first signature must still verify afterwards. If
    /// the second pass disturbed it, both signatures would break and the mail
    /// would be treated as unsigned by everyone.
    #[test]
    fn test_two_signatures_do_not_disturb_each_other_00() {
        let ed = match DkimSigner::generate("example.com", "ed1") {
            Ok(s) => s,
            Err(e) => panic!("generate: {}", e),
        };
        let rsa = rsa_signer();
        let msg = message();

        // What the ed25519 signer signs, before anything else touches it.
        let ed_input = match ed.signing_input(&msg, &[], 1_784_000_000) {
            Ok(c) => c,
            Err(e) => panic!("{}", e),
        };

        let once = match ed.sign(&msg, &[], 1_784_000_000) {
            Ok(b) => b,
            Err(e) => panic!("{}", e),
        };
        let twice = match rsa.sign(&once, &[], 1_784_000_000) {
            Ok(b) => b,
            Err(e) => panic!("{}", e),
        };

        let text = String::from_utf8_lossy(&twice);
        assert_eq!(text.matches("DKIM-Signature:").count(), 2,
            "both signatures must be present:\n{}", text);
        assert!(text.contains("a=rsa-sha256") && text.contains("a=ed25519-sha256"),
            "one of each algorithm:\n{}", text);

        // The crux: what the ed25519 signer would sign over the *doubly*
        // signed message is byte-for-byte what it signed originally. So its
        // signature still verifies, despite the RSA header now sitting above
        // it.
        let ed_input_after = match ed.signing_input(&twice, &[], 1_784_000_000) {
            Ok(c) => c,
            Err(e) => panic!("{}", e),
        };
        assert_eq!(ed_input, ed_input_after,
            "the second signature changed what the first one covers");
    }

    /// DER long-form lengths: a 2048-bit key needs them, so an off-by-one
    /// here produces a record that is silently unparseable.
    #[test]
    fn test_der_lengths_00() {
        let mut out = Vec::new();
        der_write_len(&mut out, 0x7f);
        assert_eq!(out, vec![0x7f], "short form up to 127");

        out.clear();
        der_write_len(&mut out, 0x80);
        assert_eq!(out, vec![0x81, 0x80], "long form starts at 128");

        out.clear();
        der_write_len(&mut out, 270);
        assert_eq!(out, vec![0x82, 0x01, 0x0e], "two length octets");
    }
}
