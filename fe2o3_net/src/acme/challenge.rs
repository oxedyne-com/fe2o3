//! TLS-ALPN-01 challenge cert generation (RFC 8737).
//!
//! When the ACME client has to satisfy a `tls-alpn-01` challenge for a given
//! hostname, it must be able to answer an incoming TLS handshake from the CA
//! with a self-signed certificate that:
//!
//! - Lists the hostname as its single subject alternative name.
//! - Carries a critical extension with OID `1.3.6.1.5.5.7.1.31`
//!   (id-pe-acmeIdentifier) whose content is an ASN.1 `OCTET STRING` holding
//!   the SHA-256 of the ACME key authorisation string
//!   (`<challenge-token>.<account-JWK-thumbprint>`). RFC 8737 §3.
//!
//! This module hands both the certificate DER bytes and the matching PKCS#8
//! private key DER bytes back to the caller as plain `Vec<u8>` values. It
//! deliberately does not construct a `rustls::sign::CertifiedKey` itself --
//! the rustls-side wrapping lives in the cert resolver module that runs
//! inside Steel's accept path, keeping `fe2o3_net::acme::challenge` free of
//! rustls types and therefore testable without pulling rustls into the
//! test loop.
//!
//! The cert is produced with `rcgen` using P-256 (the default). The
//! [`CustomExtension::new_acme_identifier`] helper inside `rcgen` already
//! encodes the OID, the `OCTET STRING` wrapper and the `critical = true`
//! flag, so we only have to supply the 32-byte SHA-256 digest.
//!
//! Validity period is left at the `rcgen` default (a multi-century span).
//! The CA never inspects `notBefore` / `notAfter` on a challenge cert --
//! it just executes the TLS handshake, checks the `acmeIdentifier`
//! extension, tears down and moves on -- and the resulting artefact is
//! never persisted to disk.

use oxedyne_fe2o3_core::prelude::*;

use rcgen::{
    Certificate,
    CertificateParams,
    CustomExtension,
};
use ring::digest::{
    Context,
    SHA256,
};


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ CHALLENGE CERT                                                            │
// └───────────────────────────────────────────────────────────────────────────┘

/// A self-signed cert plus its private key, both DER-encoded, ready to hand
/// to a rustls cert resolver when the CA opens a `tls-alpn-01` handshake.
#[derive(Clone, Debug)]
pub struct ChallengeCert {
    /// Self-signed certificate in DER (one entry in the chain).
    pub cert_der:   Vec<u8>,
    /// Matching PKCS#8 private key in DER.
    pub key_der:    Vec<u8>,
}

/// Build a `tls-alpn-01` challenge certificate for a single DNS name.
///
/// `hostname` is the DNS name the CA is attempting to validate. It must
/// match the SNI the CA sends on its validation handshake, so it is placed
/// in the cert as a single `dNSName` subject alternative name.
///
/// `key_authorization` is the full ACME key authorisation string for the
/// challenge: `<challenge-token>.<base64url(SHA-256(JWK))>`. The caller
/// typically obtains this by calling
/// [`crate::acme::rfc8555::Challenge::key_authorization`]. This function
/// takes the string rather than a pre-computed digest because RFC 8737 §3
/// is specified in terms of the key authorisation's SHA-256, and threading
/// the untrimmed input through the whole module is easier to audit than
/// passing a raw 32-byte digest.
pub fn build_tls_alpn_01_cert(
    hostname:           &str,
    key_authorization:  &str,
)
    -> Outcome<ChallengeCert>
{
    let digest = sha256(key_authorization.as_bytes());
    let acme_ext = CustomExtension::new_acme_identifier(&digest);

    let mut params = CertificateParams::new(vec![hostname.to_string()]);
    params.custom_extensions = vec![acme_ext];

    let cert = match Certificate::from_params(params) {
        Ok(c) => c,
        Err(e) => return Err(err!(e,
            "rcgen::Certificate::from_params failed while building a \
            tls-alpn-01 challenge cert for {:?}.", hostname;
            Init, Invalid)),
    };

    let cert_der = match cert.serialize_der() {
        Ok(b) => b,
        Err(e) => return Err(err!(e,
            "rcgen::Certificate::serialize_der failed while serialising a \
            tls-alpn-01 challenge cert for {:?}.", hostname;
            Init, Invalid)),
    };
    let key_der = cert.serialize_private_key_der();

    Ok(ChallengeCert {
        cert_der,
        key_der,
    })
}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ HELPERS                                                                   │
// └───────────────────────────────────────────────────────────────────────────┘

/// SHA-256 of the input, returned as a 32-byte array. Local to this module
/// so `challenge` remains independent of [`crate::acme::jose`].
fn sha256(data: &[u8]) -> [u8; 32] {
    let mut ctx = Context::new(&SHA256);
    ctx.update(data);
    let digest = ctx.finish();
    let mut out = [0u8; 32];
    out.copy_from_slice(digest.as_ref());
    out
}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ TESTS                                                                     │
// └───────────────────────────────────────────────────────────────────────────┘

#[cfg(test)]
mod tests {
    use super::*;

    use crate::acme::rfc8555::Challenge;
    use oxedyne_fe2o3_jdat::prelude::*;

    /// OID for id-pe-acmeIdentifier as listed in RFC 8737 §3.
    /// Encoded on the wire as the component sequence 1.3.6.1.5.5.7.1.31.
    /// In DER that's the byte sequence `06 08 2B 06 01 05 05 07 01 1F`:
    /// - `06` = OBJECT IDENTIFIER tag
    /// - `08` = length (8 bytes)
    /// - `2B` = 40*1 + 3
    /// - `06 01 05 05 07 01 1F` = 6, 1, 5, 5, 7, 1, 31 (base-128 varints)
    const ACME_OID_DER: [u8; 10] = [
        0x06, 0x08, 0x2b, 0x06, 0x01, 0x05, 0x05, 0x07, 0x01, 0x1f,
    ];

    /// Build a challenge cert from a plausible key authorisation string
    /// and verify we got non-empty DER for both the cert and the key.
    #[test]
    fn test_build_challenge_cert_returns_non_empty_der() -> Outcome<()> {
        let cc = res!(build_tls_alpn_01_cert(
            "example.com",
            "token.thumbprint",
        ));
        if cc.cert_der.is_empty() {
            return Err(err!("cert_der was empty"; Test, Mismatch));
        }
        if cc.key_der.is_empty() {
            return Err(err!("key_der was empty"; Test, Mismatch));
        }
        Ok(())
    }

    /// The generated cert must carry the id-pe-acmeIdentifier OID in its
    /// DER bytes. The most robust check that does not depend on a full
    /// ASN.1 parser is a byte-subsequence search for the OID's DER
    /// encoding -- if the extension is present at all, this sequence
    /// appears verbatim.
    #[test]
    fn test_challenge_cert_contains_acme_oid() -> Outcome<()> {
        let cc = res!(build_tls_alpn_01_cert(
            "example.com",
            "some.keyauth",
        ));
        let found = cc.cert_der
            .windows(ACME_OID_DER.len())
            .any(|w| w == ACME_OID_DER);
        if !found {
            return Err(err!(
                "Challenge cert DER does not contain the id-pe-acmeIdentifier \
                OID sequence {:02x?}.", ACME_OID_DER;
                Test, Missing));
        }
        Ok(())
    }

    /// The 32-byte digest inside the `acmeIdentifier` extension must match
    /// `SHA-256(key_authorization)` byte-for-byte. We verify this directly
    /// by scanning the DER output for the digest bytes as a subsequence.
    /// If this assertion ever breaks it means either the extension is not
    /// being written or the digest we're computing diverged from the one
    /// rcgen embedded.
    #[test]
    fn test_challenge_cert_embeds_correct_digest() -> Outcome<()> {
        let key_auth = "abcdefghijklmnop.qrstuvwxyz0123456789AB";
        let cc = res!(build_tls_alpn_01_cert("example.test", key_auth));
        let digest = sha256(key_auth.as_bytes());
        let found = cc.cert_der
            .windows(digest.len())
            .any(|w| w == digest);
        if !found {
            return Err(err!(
                "Challenge cert DER does not contain the expected SHA-256 \
                digest of the key authorisation.";
                Test, Missing));
        }
        Ok(())
    }

    /// The hostname must appear in the cert (it is placed as a dNSName SAN).
    /// Simple substring check over the DER bytes -- the name is encoded as
    /// IA5String so it appears verbatim.
    #[test]
    fn test_challenge_cert_contains_hostname() -> Outcome<()> {
        let host = "example.com";
        let cc = res!(build_tls_alpn_01_cert(host, "tok.auth"));
        let needle = host.as_bytes();
        let found = cc.cert_der
            .windows(needle.len())
            .any(|w| w == needle);
        if !found {
            return Err(err!(
                "Challenge cert DER does not contain the requested \
                hostname {:?} as a subject alternative name.", host;
                Test, Missing));
        }
        Ok(())
    }

    /// End-to-end integration: given a typed RFC 8555 `Challenge` and a
    /// faked JWK thumbprint, use `Challenge::key_authorization` to build
    /// the authorisation string, hand it to `build_tls_alpn_01_cert`, and
    /// verify the digest shows up inside the resulting cert. This exercises
    /// the full data flow the ACME client will drive at runtime.
    #[test]
    fn test_integration_challenge_to_cert() -> Outcome<()> {
        let chall = Challenge {
            typ:        "tls-alpn-01".to_string(),
            status:     "pending".to_string(),
            url:        "https://acme-v02.api.letsencrypt.org/acme/chall/1".to_string(),
            token:      "RealLiveToken".to_string(),
            validated:  String::new(),
            error:      Dat::Empty,
        };
        let thumbprint = [0x11u8; 32];
        let key_auth = chall.key_authorization(&thumbprint);
        let cc = res!(build_tls_alpn_01_cert("example.com", &key_auth));

        let expected_digest = sha256(key_auth.as_bytes());
        let found = cc.cert_der
            .windows(expected_digest.len())
            .any(|w| w == expected_digest);
        if !found {
            return Err(err!(
                "Integration: cert DER does not contain SHA-256 of the \
                Challenge's key_authorization.";
                Test, Missing));
        }
        Ok(())
    }
}
