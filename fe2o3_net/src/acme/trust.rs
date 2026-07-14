//! Pinned Let's Encrypt root CA trust anchors for the ACME client.
//!
//! The ACME client needs to verify the TLS certificate of the CA it talks
//! to (e.g. `acme-v02.api.letsencrypt.org`). Rather than pull in the
//! `webpki-roots` crate -- which carries the entire Mozilla root programme,
//! hundreds of CAs, most of which are irrelevant to our use case -- we
//! embed only the two Let's Encrypt root anchors that actually sign the
//! LE API endpoints today:
//!
//! - **ISRG Root X1** (RSA 4096), valid through 4 June 2035. Serial
//!   `172886928669790476064670243504169061120`. SHA-256 fingerprint
//!   `96:bc:ec:06:26:49:76:f3:74:60:77:9a:cf:28:c5:a7:cf:e8:a3:c0:aa:e1:1a:8f:fc:ee:05:c0:bd:df:08:c6`.
//! - **ISRG Root X2** (ECDSA P-384), valid through 17 September 2040.
//!   Serial `87493402998870891108772069816698636114`. SHA-256 fingerprint
//!   `69:72:9b:8e:15:a8:6e:fc:17:7a:57:af:b7:17:1d:fc:64:ad:d2:8c:2f:ca:8c:f1:50:7e:34:45:3c:cb:14:70`.
//!
//! The base64 bodies below are the exact bytes between the `-----BEGIN
//! CERTIFICATE-----` and `-----END CERTIFICATE-----` markers on Let's
//! Encrypt's published PEM files, retained in PEM line-break form for
//! readability and stripped of whitespace at runtime before base64
//! decoding.
//!
//! When either root reaches end of life, add the replacement here and
//! leave the old one in place long enough to cover the rollover window.
//! Both anchors are advertised to `rustls` at every ACME startup, so
//! having multiple active is harmless.

use oxedyne_fe2o3_core::prelude::*;

use std::sync::Arc;

use base64;
use tokio_rustls::rustls::{
    pki_types::CertificateDer,
    ClientConfig,
    RootCertStore,
};


/// ISRG Root X1, DER body encoded as base64. Valid through 2035-06-04.
const ISRG_ROOT_X1_B64: &str = "\
MIIFazCCA1OgAwIBAgIRAIIQz7DSQONZRGPgu2OCiwAwDQYJKoZIhvcNAQELBQAw\
TzELMAkGA1UEBhMCVVMxKTAnBgNVBAoTIEludGVybmV0IFNlY3VyaXR5IFJlc2Vh\
cmNoIEdyb3VwMRUwEwYDVQQDEwxJU1JHIFJvb3QgWDEwHhcNMTUwNjA0MTEwNDM4\
WhcNMzUwNjA0MTEwNDM4WjBPMQswCQYDVQQGEwJVUzEpMCcGA1UEChMgSW50ZXJu\
ZXQgU2VjdXJpdHkgUmVzZWFyY2ggR3JvdXAxFTATBgNVBAMTDElTUkcgUm9vdCBY\
MTCCAiIwDQYJKoZIhvcNAQEBBQADggIPADCCAgoCggIBAK3oJHP0FDfzm54rVygc\
h77ct984kIxuPOZXoHj3dcKi/vVqbvYATyjb3miGbESTtrFj/RQSa78f0uoxmyF+\
0TM8ukj13Xnfs7j/EvEhmkvBioZxaUpmZmyPfjxwv60pIgbz5MDmgK7iS4+3mX6U\
A5/TR5d8mUgjU+g4rk8Kb4Mu0UlXjIB0ttov0DiNewNwIRt18jA8+o+u3dpjq+sW\
T8KOEUt+zwvo/7V3LvSye0rgTBIlDHCNAymg4VMk7BPZ7hm/ELNKjD+Jo2FR3qyH\
B5T0Y3HsLuJvW5iB4YlcNHlsdu87kGJ55tukmi8mxdAQ4Q7e2RCOFvu396j3x+UC\
B5iPNgiV5+I3lg02dZ77DnKxHZu8A/lJBdiB3QW0KtZB6awBdpUKD9jf1b0SHzUv\
KBds0pjBqAlkd25HN7rOrFleaJ1/ctaJxQZBKT5ZPt0m9STJEadao0xAH0ahmbWn\
OlFuhjuefXKnEgV4We0+UXgVCwOPjdAvBbI+e0ocS3MFEvzG6uBQE3xDk3SzynTn\
jh8BCNAw1FtxNrQHusEwMFxIt4I7mKZ9YIqioymCzLq9gwQbooMDQaHWBfEbwrbw\
qHyGO0aoSCqI3Haadr8faqU9GY/rOPNk3sgrDQoo//fb4hVC1CLQJ13hef4Y53CI\
rU7m2Ys6xt0nUW7/vGT1M0NPAgMBAAGjQjBAMA4GA1UdDwEB/wQEAwIBBjAPBgNV\
HRMBAf8EBTADAQH/MB0GA1UdDgQWBBR5tFnme7bl5AFzgAiIyBpY9umbbjANBgkq\
hkiG9w0BAQsFAAOCAgEAVR9YqbyyqFDQDLHYGmkgJykIrGF1XIpu+ILlaS/V9lZL\
ubhzEFnTIZd+50xx+7LSYK05qAvqFyFWhfFQDlnrzuBZ6brJFe+GnY+EgPbk6ZGQ\
3BebYhtF8GaV0nxvwuo77x/Py9auJ/GpsMiu/X1+mvoiBOv/2X/qkSsisRcOj/KK\
NFtY2PwByVS5uCbMiogziUwthDyC3+6WVwW6LLv3xLfHTjuCvjHIInNzktHCgKQ5\
ORAzI4JMPJ+GslWYHb4phowim57iaztXOoJwTdwJx4nLCgdNbOhdjsnvzqvHu7Ur\
TkXWStAmzOVyyghqpZXjFaH3pO3JLF+l+/+sKAIuvtd7u+Nxe5AW0wdeRlN8NwdC\
jNPElpzVmbUq4JUagEiuTDkHzsxHpFKVK7q4+63SM1N95R1NbdWhscdCb+ZAJzVc\
oyi3B43njTOQ5yOf+1CceWxG1bQVs5ZufpsMljq4Ui0/1lvh+wjChP4kqKOJ2qxq\
4RgqsahDYVvTH9w7jXbyLeiNdd8XM2w9U/t7y0Ff/9yi0GE44Za4rF2LN9d11TPA\
mRGunUHBcnWEvgJBQl9nJEiU0Zsnvgc/ubhPgXRR4Xq37Z0j4r7g1SgEEzwxA57d\
emyPxgcYxn/eR44/KJ4EBs+lVDR3veyJm+kXQ99b21/+jh5Xos1AnX5iItreGCc=";

/// ISRG Root X2, DER body encoded as base64. Valid through 2040-09-17.
const ISRG_ROOT_X2_B64: &str = "\
MIICGzCCAaGgAwIBAgIQQdKd0XLq7qeAwSxs6S+HUjAKBggqhkjOPQQDAzBPMQsw\
CQYDVQQGEwJVUzEpMCcGA1UEChMgSW50ZXJuZXQgU2VjdXJpdHkgUmVzZWFyY2gg\
R3JvdXAxFTATBgNVBAMTDElTUkcgUm9vdCBYMjAeFw0yMDA5MDQwMDAwMDBaFw00\
MDA5MTcxNjAwMDBaME8xCzAJBgNVBAYTAlVTMSkwJwYDVQQKEyBJbnRlcm5ldCBT\
ZWN1cml0eSBSZXNlYXJjaCBHcm91cDEVMBMGA1UEAxMMSVNSRyBSb290IFgyMHYw\
EAYHKoZIzj0CAQYFK4EEACIDYgAEzZvVn4CDCuwJSvMWSj5cz3es3mcFDR0HttwW\
+1qLFNvicWDEukWVEYmO6gbf9yoWHKS5xcUy4APgHoIYOIvXRdgKam7mAHf7AlF9\
ItgKbppbd9/w+kHsOdx1ymgHDB/qo0IwQDAOBgNVHQ8BAf8EBAMCAQYwDwYDVR0T\
AQH/BAUwAwEB/zAdBgNVHQ4EFgQUfEKWrt5LSDv6kviejM9ti6lyN5UwCgYIKoZI\
zj0EAwMDaAAwZQIwe3lORlCEwkSHRhtFcP9Ymd70/aTSVaYgLXTWNLxBo1BfASdW\
tL4ndQavEi51mI38AjEAi/V3bNTIZargCyzuFJ0nN6T5U6VR5CmD1/iQMVtCnwr1\
/q4AaOeMSQ+2b1tbFfLn";


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ CLIENT CONFIG BUILDER                                                     │
// └───────────────────────────────────────────────────────────────────────────┘

/// Build a `rustls::ClientConfig` that trusts only the Let's Encrypt root
/// anchors compiled into this module.
///
/// The returned `Arc<ClientConfig>` is suitable to hand directly to
/// [`crate::http::client::https_request`] when talking to an ACME
/// directory URL. Callers typically build it once at service startup and
/// reuse it for every ACME request.
pub fn letsencrypt_client_config() -> Outcome<Arc<ClientConfig>> {
    let mut store = RootCertStore::empty();
    for (label, b64) in [
        ("ISRG Root X1", ISRG_ROOT_X1_B64),
        ("ISRG Root X2", ISRG_ROOT_X2_B64),
    ] {
        let der = res!(decode_pem_body(label, b64));
        let anchor = CertificateDer::from(der);
        if let Err(e) = store.add(anchor) {
            return Err(err!(
                "rustls refused to add the pinned root {:?}: {:?}.", label, e;
                Init, Invalid));
        }
    }
    // rustls's builder panics when it cannot resolve a crypto provider from
    // process-global state. Install one if the application has not.
    crate::tls::ensure_crypto_provider();
    let config = ClientConfig::builder()
        .with_root_certificates(store)
        .with_no_client_auth();
    Ok(Arc::new(config))
}

/// Strip whitespace from a multi-line base64 body and decode to bytes.
///
/// `label` is used only to make the error message point at the right
/// constant if decoding fails.
fn decode_pem_body(label: &str, body: &str) -> Outcome<Vec<u8>> {
    let cleaned: String = body.chars().filter(|c| !c.is_whitespace()).collect();
    match base64::decode(&cleaned) {
        Ok(v) => Ok(v),
        Err(e) => Err(err!(e,
            "Failed to base64-decode the embedded {:?} certificate body.", label;
            Decode, Invalid, Input)),
    }
}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ TESTS                                                                     │
// └───────────────────────────────────────────────────────────────────────────┘

#[cfg(test)]
mod tests {
    use super::*;

    /// Both embedded base64 bodies must decode to non-empty DER.
    #[test]
    fn test_decode_embedded_roots() -> Outcome<()> {
        let x1 = res!(decode_pem_body("ISRG Root X1", ISRG_ROOT_X1_B64));
        let x2 = res!(decode_pem_body("ISRG Root X2", ISRG_ROOT_X2_B64));
        // X1 is an RSA-4096 self-signed root; a realistic length is around
        // 1.4-1.6 KB. X2 is ECDSA P-384 and closer to 0.5 KB.
        if x1.len() < 1000 || x1.len() > 2500 {
            return Err(err!(
                "ISRG Root X1 decoded to {} bytes; expected roughly 1.4-1.6 KB.",
                x1.len();
                Test, Mismatch));
        }
        if x2.len() < 300 || x2.len() > 1000 {
            return Err(err!(
                "ISRG Root X2 decoded to {} bytes; expected roughly 0.5 KB.",
                x2.len();
                Test, Mismatch));
        }
        // X.509 certificates always begin with the SEQUENCE tag 0x30.
        if x1[0] != 0x30 {
            return Err(err!(
                "ISRG Root X1 does not begin with the X.509 SEQUENCE tag 0x30.";
                Test, Mismatch));
        }
        if x2[0] != 0x30 {
            return Err(err!(
                "ISRG Root X2 does not begin with the X.509 SEQUENCE tag 0x30.";
                Test, Mismatch));
        }
        Ok(())
    }

    /// Building the ClientConfig must succeed, must return a fresh `Arc`
    /// on each call, and rustls must have accepted both roots into the
    /// trust store without complaint.
    #[test]
    fn test_build_letsencrypt_client_config() -> Outcome<()> {
        let config_a = res!(letsencrypt_client_config());
        let config_b = res!(letsencrypt_client_config());
        // The two Arcs should point at different allocations; we do not
        // cache anything.
        if Arc::ptr_eq(&config_a, &config_b) {
            return Err(err!(
                "letsencrypt_client_config must return a freshly-built \
                ClientConfig on each call; got two Arcs pointing at the \
                same allocation.";
                Test, Mismatch));
        }
        Ok(())
    }

    /// The cleaner strips whitespace deterministically: given the same
    /// input with extra spacing, we must decode to the same DER.
    #[test]
    fn test_decode_pem_body_tolerates_whitespace() -> Outcome<()> {
        let without = res!(decode_pem_body("ISRG Root X2", ISRG_ROOT_X2_B64));
        let spaced  = res!(decode_pem_body(
            "ISRG Root X2",
            &fmt!("\n  {}  \n  ", ISRG_ROOT_X2_B64),
        ));
        if without != spaced {
            return Err(err!(
                "decode_pem_body produced different output when extra \
                whitespace was inserted.";
                Test, Mismatch));
        }
        Ok(())
    }
}
