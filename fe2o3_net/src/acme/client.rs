//! ACME client state machine for RFC 8555 via the `tls-alpn-01` challenge.
//!
//! [`AcmeClient`] drives one end-to-end issuance against a CA such as Let's
//! Encrypt. The happy path is:
//!
//! 1. Fetch the CA directory (cached on the client after first call).
//! 2. Fetch a fresh nonce.
//! 3. Register (or recover) the ACME account.
//! 4. Submit a new order for one or more DNS identifiers.
//! 5. For each authorisation URL the CA returns, fetch it, locate the
//!    `tls-alpn-01` challenge, build an ephemeral challenge certificate
//!    via [`crate::acme::challenge`], install it into the caller's
//!    resolver (via [`ChallengeInstaller`]), and POST the challenge URL
//!    to signal readiness.
//! 6. Poll the authorisation until it reaches `valid` or `invalid`.
//! 7. Poll the order until it reaches `ready`.
//! 8. Generate a fresh P-256 key pair and a CSR for the requested DNS
//!    names, POST the CSR to the order's finalise URL, and poll the
//!    order until it reaches `valid`.
//! 9. POST-as-GET the order's certificate URL and return the PEM chain
//!    plus the matching PKCS#8 private key.
//!
//! Every POST to the CA is wrapped in a JWS produced by
//! [`crate::acme::jose::JwsSigner`]. The first request (new-account)
//! carries the full public key in the `jwk` header field; subsequent
//! requests carry the account URL in a `kid` field as RFC 8555 §6.2
//! requires.
//!
//! Nonces are threaded through every request by extracting the
//! `Replay-Nonce` response header from each successful reply and stashing
//! it for the next request. When the CA rejects a request with a
//! `badNonce` error we automatically retry once with the fresh nonce the
//! server returned in the same response.
//!
//! The HTTP transport is [`crate::http::client::https_request`], which is
//! the caller-agnostic `tokio` + `tokio_rustls` + `HttpMessage` client
//! also used for any other outbound HTTPS call in `fe2o3_net`. The caller
//! supplies an `Arc<ClientConfig>` that pins the Let's Encrypt root
//! anchors; see [`crate::acme::trust::letsencrypt_client_config`].

use crate::{
    acme::{
        challenge::{
            build_tls_alpn_01_cert,
            ChallengeCert,
        },
        jose::{
            base64url_encode,
            JwsSigner,
        },
        rfc8555::{
            finalize_request,
            new_account_request,
            new_order_request,
            parse_json_response,
            Authorization,
            Challenge,
            Directory,
            Order,
            Problem,
        },
    },
    http::{
        client::https_request,
        fields::{
            HeaderFieldValue,
            HeaderName,
        },
        header::HttpMethod,
        msg::HttpMessage,
    },
};

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_jdat::prelude::*;

use std::{
    sync::Arc,
    time::Duration,
};

use rcgen::{
    Certificate,
    CertificateParams,
    DistinguishedName,
    DnType,
};
use tokio_rustls::rustls::ClientConfig;


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ PUBLIC TYPES                                                              │
// └───────────────────────────────────────────────────────────────────────────┘

/// An installer callback that plugs and removes `tls-alpn-01` challenge
/// certificates from the caller's live rustls cert resolver while an ACME
/// issuance is in flight.
///
/// The methods are synchronous because the typical installer is an
/// `Arc<RwLock<HashMap<String, Arc<CertifiedKey>>>>` whose inserts and
/// removes are non-blocking, and keeping the trait synchronous avoids the
/// ergonomic friction of `async fn` in traits.
pub trait ChallengeInstaller: Send + Sync {

    /// Install `cert` as the ALPN-gated challenge certificate for
    /// `hostname`. By the time this method returns, any incoming TLS
    /// handshake for `hostname` that advertises the `acme-tls/1` ALPN
    /// protocol must be answered with `cert`.
    fn install(&self, hostname: &str, cert: &ChallengeCert) -> Outcome<()>;

    /// Remove the challenge certificate previously installed for
    /// `hostname`. Called after the CA has validated (or given up on)
    /// the challenge, regardless of outcome.
    fn remove(&self, hostname: &str) -> Outcome<()>;
}

/// A freshly-issued certificate chain plus its matching private key.
#[derive(Clone, Debug)]
pub struct IssuedCertificate {
    /// PEM-encoded certificate chain exactly as the CA sent it.
    pub cert_pem:   Vec<u8>,
    /// PKCS#8 DER-encoded private key matching the leaf cert. This is
    /// **not** the ACME account key -- it is a fresh P-256 key pair that
    /// rcgen generated while building the CSR and which the CA therefore
    /// knows the public half of.
    pub key_pkcs8:  Vec<u8>,
}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ ACME CLIENT                                                               │
// └───────────────────────────────────────────────────────────────────────────┘

/// ACME client state held across the steps of a single issuance.
///
/// The client is deliberately single-threaded: every method takes `&mut
/// self` and every request must complete before the next one begins. This
/// matches the protocol -- which is intrinsically serial because of the
/// nonce chain -- and avoids any need for locks inside the client.
pub struct AcmeClient {
    /// Full URL of the CA directory endpoint.
    directory_url:  String,
    /// Contact email the account will be registered with.
    contact_email:  String,
    /// rustls client config trusting the CA's root anchors.
    tls_config:     Arc<ClientConfig>,
    /// Persistent account signing key. Generated fresh or loaded from the
    /// disk cache by the caller before construction.
    signer:         JwsSigner,
    /// Directory document, cached after the first fetch.
    directory:      Option<Directory>,
    /// Account URL (the `kid`), set once `register_account` has returned.
    kid:            Option<String>,
    /// Most recent `Replay-Nonce` header value, to be consumed on the
    /// next POST.
    nonce:          Option<String>,
}

impl AcmeClient {

    /// Build a new client. Does no I/O; the directory and nonce are
    /// fetched lazily on first use.
    pub fn new(
        directory_url:  impl Into<String>,
        contact_email:  impl Into<String>,
        tls_config:     Arc<ClientConfig>,
        signer:         JwsSigner,
    )
        -> Self
    {
        Self {
            directory_url:  directory_url.into(),
            contact_email:  contact_email.into(),
            tls_config,
            signer,
            directory:      None,
            kid:            None,
            nonce:          None,
        }
    }

    /// Borrow the underlying account signer so the caller can persist the
    /// PKCS#8 bytes to disk via [`crate::acme::cache::AcmeDiskCache`].
    pub fn signer(&self) -> &JwsSigner {
        &self.signer
    }

    /// Return the account URL assigned by the CA during
    /// `register_account`, or `None` if registration has not yet run.
    pub fn kid(&self) -> Option<&str> {
        self.kid.as_deref()
    }

    // ---- low-level helpers -----------------------------------------------

    /// Fetch the CA directory if not already cached, then return a
    /// reference to it.
    async fn ensure_directory(&mut self) -> Outcome<&Directory> {
        if self.directory.is_none() {
            let dir = res!(self.fetch_directory().await);
            self.directory = Some(dir);
        }
        match &self.directory {
            Some(d) => Ok(d),
            None    => Err(err!(
                "Internal: ensure_directory left self.directory empty.";
                Bug)),
        }
    }

    /// GET the directory document.
    async fn fetch_directory(&self) -> Outcome<Directory> {
        let (host, port, path) = res!(split_https_url(&self.directory_url));
        let msg = res!(https_request(
            &host,
            port,
            HttpMethod::GET,
            &path,
            &[],
            &[],
            self.tls_config.clone(),
        ).await);
        res!(require_success(&msg, "GET directory"));
        parse_json_response(&msg.body)
    }

    /// GET the new-nonce endpoint to obtain a fresh nonce. RFC 8555 §7.2
    /// permits either GET or HEAD; GET is simpler because our HTTP
    /// reader always expects a body frame (possibly empty).
    async fn refresh_nonce(&mut self) -> Outcome<()> {
        let new_nonce_url = {
            let dir = res!(self.ensure_directory().await);
            dir.new_nonce.clone()
        };
        let (host, port, path) = res!(split_https_url(&new_nonce_url));
        let msg = res!(https_request(
            &host,
            port,
            HttpMethod::GET,
            &path,
            &[],
            &[],
            self.tls_config.clone(),
        ).await);
        res!(require_success(&msg, "GET new-nonce"));
        self.nonce = Some(res!(read_replay_nonce(&msg, "new-nonce response")));
        Ok(())
    }

    /// Consume the stashed nonce. If none is cached, fetch a fresh one
    /// first. The fresh one is stashed into `self.nonce` by the underlying
    /// HTTP reply and then immediately taken.
    async fn take_nonce(&mut self) -> Outcome<String> {
        if self.nonce.is_none() {
            res!(self.refresh_nonce().await);
        }
        match self.nonce.take() {
            Some(n) => Ok(n),
            None    => Err(err!(
                "Internal: take_nonce found no nonce after refresh_nonce \
                returned Ok.";
                Bug)),
        }
    }

    /// Sign a payload with a `jwk` protected header. Used only for the
    /// new-account request, where the CA does not yet know the account URL.
    fn sign_with_jwk(
        &self,
        url:        &str,
        nonce:      &str,
        payload:    &Dat,
    )
        -> Outcome<Vec<u8>>
    {
        let jwk = res!(self.signer.public_jwk());
        let header = mapdat!{
            "alg"   => "ES256",
            "nonce" => nonce,
            "url"   => url,
            "jwk"   => jwk,
        };
        let payload_bytes = res!(payload.json()).into_bytes();
        let jws = res!(self.signer.sign_flattened(&header, &payload_bytes));
        Ok(res!(jws.json()).into_bytes())
    }

    /// Sign a payload with a `kid` protected header. Used for every
    /// authenticated request after `register_account` has run.
    fn sign_with_kid(
        &self,
        url:        &str,
        nonce:      &str,
        payload:    &Dat,
    )
        -> Outcome<Vec<u8>>
    {
        let kid = match &self.kid {
            Some(k) => k.clone(),
            None    => return Err(err!(
                "sign_with_kid called before register_account; no account \
                URL is known yet.";
                Bug)),
        };
        let header = mapdat!{
            "alg"   => "ES256",
            "nonce" => nonce,
            "url"   => url,
            "kid"   => kid,
        };
        let payload_bytes = res!(payload.json()).into_bytes();
        let jws = res!(self.signer.sign_flattened(&header, &payload_bytes));
        Ok(res!(jws.json()).into_bytes())
    }

    /// Sign an empty-payload request in POST-as-GET style (RFC 8555 §6.3)
    /// using a `kid` header.
    fn sign_post_as_get(
        &self,
        url:        &str,
        nonce:      &str,
    )
        -> Outcome<Vec<u8>>
    {
        let kid = match &self.kid {
            Some(k) => k.clone(),
            None    => return Err(err!(
                "sign_post_as_get called before register_account.";
                Bug)),
        };
        let header = mapdat!{
            "alg"   => "ES256",
            "nonce" => nonce,
            "url"   => url,
            "kid"   => kid,
        };
        let jws = res!(self.signer.sign_flattened(&header, b""));
        Ok(res!(jws.json()).into_bytes())
    }

    /// Low-level JOSE POST. Signs `payload` either with `jwk` (when
    /// `use_jwk` is true -- first contact, new-account case) or with `kid`
    /// (every other request), POSTs the result to `url` with the correct
    /// `Content-Type`, updates the stashed nonce from the response, and
    /// transparently retries once on a `badNonce` server error.
    async fn post_jose(
        &mut self,
        url:        &str,
        payload:    &Dat,
        use_jwk:    bool,
    )
        -> Outcome<HttpMessage>
    {
        let mut attempts_remaining: u8 = 2;
        loop {
            attempts_remaining -= 1;

            let nonce = res!(self.take_nonce().await);
            let body = if use_jwk {
                res!(self.sign_with_jwk(url, &nonce, payload))
            } else {
                res!(self.sign_with_kid(url, &nonce, payload))
            };

            let (host, port, path) = res!(split_https_url(url));
            let msg = res!(https_request(
                &host,
                port,
                HttpMethod::POST,
                &path,
                &[("Content-Type", "application/jose+json")],
                &body,
                self.tls_config.clone(),
            ).await);

            // Stash the fresh nonce the CA gave us, if any, before doing
            // anything else. This is required for the retry path as well
            // as the normal success path.
            if let Ok(n) = read_replay_nonce(&msg, "POST JOSE response") {
                self.nonce = Some(n);
            }

            let status = http_status_code(&msg);
            if status / 100 == 2 {
                return Ok(msg);
            }
            if status == 400 && attempts_remaining > 0 {
                // Only retry if the problem document indicates badNonce.
                if let Ok(Some(problem)) = parse_problem_body(&msg.body) {
                    if problem.typ.ends_with(":badNonce") {
                        continue;
                    }
                }
            }
            return Err(res!(acme_error_from_response(&msg, url)));
        }
    }

    /// Same as `post_jose` but for POST-as-GET style requests (empty
    /// payload), still using the `kid` authentication mode.
    async fn post_as_get(
        &mut self,
        url:    &str,
    )
        -> Outcome<HttpMessage>
    {
        let mut attempts_remaining: u8 = 2;
        loop {
            attempts_remaining -= 1;

            let nonce = res!(self.take_nonce().await);
            let body = res!(self.sign_post_as_get(url, &nonce));

            let (host, port, path) = res!(split_https_url(url));
            let msg = res!(https_request(
                &host,
                port,
                HttpMethod::POST,
                &path,
                &[("Content-Type", "application/jose+json")],
                &body,
                self.tls_config.clone(),
            ).await);

            if let Ok(n) = read_replay_nonce(&msg, "POST-as-GET response") {
                self.nonce = Some(n);
            }

            let status = http_status_code(&msg);
            if status / 100 == 2 {
                return Ok(msg);
            }
            if status == 400 && attempts_remaining > 0 {
                if let Ok(Some(problem)) = parse_problem_body(&msg.body) {
                    if problem.typ.ends_with(":badNonce") {
                        continue;
                    }
                }
            }
            return Err(res!(acme_error_from_response(&msg, url)));
        }
    }

    // ---- protocol steps --------------------------------------------------

    /// Register or recover the ACME account for our signer key. On
    /// success, stores the `Location` header as `self.kid`.
    pub async fn register_account(&mut self) -> Outcome<()> {
        let new_account_url = {
            let dir = res!(self.ensure_directory().await);
            dir.new_account.clone()
        };
        let payload = new_account_request(&self.contact_email, true);
        let msg = res!(self.post_jose(&new_account_url, &payload, true).await);

        let kid = res!(read_location(&msg, "new-account response"));
        self.kid = Some(kid);
        Ok(())
    }

    /// Submit a new order for the given DNS identifiers. Returns the
    /// order's `Location` URL (which the client must POST-as-GET to poll
    /// the status) and the parsed `Order`.
    pub async fn new_order(
        &mut self,
        dns_names:  &[String],
    )
        -> Outcome<(String, Order)>
    {
        let new_order_url = {
            let dir = res!(self.ensure_directory().await);
            dir.new_order.clone()
        };
        let payload = new_order_request(dns_names);
        let msg = res!(self.post_jose(&new_order_url, &payload, false).await);

        let order_url = res!(read_location(&msg, "new-order response"));
        let order: Order = res!(parse_json_response(&msg.body));
        Ok((order_url, order))
    }

    /// Fetch an authorisation object in POST-as-GET style.
    pub async fn fetch_authorization(
        &mut self,
        authz_url:  &str,
    )
        -> Outcome<Authorization>
    {
        let msg = res!(self.post_as_get(authz_url).await);
        parse_json_response(&msg.body)
    }

    /// POST `{}` to a challenge URL to tell the CA we are ready to be
    /// validated.
    pub async fn signal_challenge_ready(
        &mut self,
        challenge_url:  &str,
    )
        -> Outcome<Challenge>
    {
        let payload = mapdat!{};
        let msg = res!(self.post_jose(challenge_url, &payload, false).await);
        parse_json_response(&msg.body)
    }

    /// POST-as-GET an order URL to read its current status.
    pub async fn poll_order(
        &mut self,
        order_url:  &str,
    )
        -> Outcome<Order>
    {
        let msg = res!(self.post_as_get(order_url).await);
        parse_json_response(&msg.body)
    }

    /// POST the CSR to the order's finalise URL.
    pub async fn finalize_order(
        &mut self,
        finalize_url:   &str,
        csr_der:        &[u8],
    )
        -> Outcome<Order>
    {
        let csr_b64 = base64url_encode(csr_der);
        let payload = finalize_request(&csr_b64);
        let msg = res!(self.post_jose(finalize_url, &payload, false).await);
        parse_json_response(&msg.body)
    }

    /// POST-as-GET the issued certificate URL. Returns the raw response
    /// body, which is a PEM-encoded certificate chain
    /// (`application/pem-certificate-chain`).
    pub async fn download_certificate(
        &mut self,
        cert_url:   &str,
    )
        -> Outcome<Vec<u8>>
    {
        let msg = res!(self.post_as_get(cert_url).await);
        Ok(msg.body)
    }

    // ---- high-level driver -----------------------------------------------

    /// Drive the full RFC 8555 issuance cycle for the given DNS names,
    /// installing challenge certs via `installer` at the appropriate
    /// moments and removing them afterwards.
    pub async fn issue_certificate<I: ChallengeInstaller>(
        &mut self,
        dns_names:  &[String],
        installer:  &I,
    )
        -> Outcome<IssuedCertificate>
    {
        if dns_names.is_empty() {
            return Err(err!(
                "AcmeClient::issue_certificate called with an empty \
                dns_names slice.";
                Invalid, Input, Missing));
        }

        // Register account if we haven't already this session.
        if self.kid.is_none() {
            res!(self.register_account().await);
        }

        // Submit the order and fetch its authorisation URLs.
        let (order_url, mut order) = res!(self.new_order(dns_names).await);

        // Remember the hostnames we installed challenge certs for, so we
        // can remove them all at the end regardless of success.
        let mut installed_hosts: Vec<String> = Vec::new();

        // Drive each authorisation to the "valid" state.
        let drive_result = self.drive_all_authorisations(
            &order,
            installer,
            &mut installed_hosts,
        ).await;

        // Uninstall challenge certs unconditionally.
        for host in &installed_hosts {
            if let Err(e) = installer.remove(host) {
                // Log-worthy but not fatal; the issuance may still be on
                // track.  Wrap into the error chain if `drive_result` is
                // already broken, otherwise report it separately via the
                // standard logging macros.
                warn!("ACME: installer.remove({:?}) failed: {:?}", host, e);
            }
        }
        res!(drive_result);

        // Poll the order until it is ready to be finalised.
        order = res!(self.poll_until_ready(&order_url).await);

        // Build the CSR key pair for the end-entity cert, generate the
        // CSR, and finalise the order. We do not bind the finalise reply
        // to a local because the subsequent poll loop re-reads the order
        // anyway; we only care that the POST returned 2xx.
        let (csr_der, key_pkcs8) = res!(build_csr(dns_names));
        let _ = res!(self.finalize_order(&order.finalize, &csr_der).await);

        // Poll until valid.
        let order = res!(self.poll_until_valid(&order_url).await);

        if order.certificate.is_empty() {
            return Err(err!(
                "Order reached status 'valid' but did not include a \
                certificate URL.";
                IO, Network, Missing, Invalid));
        }
        let cert_pem = res!(self.download_certificate(&order.certificate).await);

        Ok(IssuedCertificate {
            cert_pem,
            key_pkcs8,
        })
    }

    /// Walk every authorisation attached to `order`, build a challenge
    /// cert for each, install it via `installer`, signal readiness, and
    /// poll the authorisation until it becomes `valid`.
    async fn drive_all_authorisations<I: ChallengeInstaller>(
        &mut self,
        order:              &Order,
        installer:          &I,
        installed_hosts:    &mut Vec<String>,
    )
        -> Outcome<()>
    {
        for authz_url in &order.authorizations {
            let authz = res!(self.fetch_authorization(authz_url).await);

            // We only satisfy DNS identifiers via tls-alpn-01.
            let hostname = res!(dns_identifier(&authz));
            let chall = match res!(authz.tls_alpn_01_challenge()) {
                Some(c) => c,
                None    => return Err(err!(
                    "Authorisation for {:?} did not offer a tls-alpn-01 \
                    challenge.", hostname;
                    IO, Network, Missing, Invalid)),
            };
            if chall.token.is_empty() {
                return Err(err!(
                    "tls-alpn-01 challenge for {:?} has an empty token.",
                    hostname;
                    IO, Network, Missing, Invalid));
            }

            let thumbprint = res!(self.signer.jwk_thumbprint_sha256());
            let key_auth = chall.key_authorization(&thumbprint);
            let cert = res!(build_tls_alpn_01_cert(&hostname, &key_auth));

            res!(installer.install(&hostname, &cert));
            installed_hosts.push(hostname.clone());

            let _ = res!(self.signal_challenge_ready(&chall.url).await);

            // Poll the authorisation itself until it reaches a terminal
            // state.
            let final_authz = res!(self.poll_authorisation_until_final(authz_url).await);
            if final_authz.status != "valid" {
                return Err(err!(
                    "Authorisation for {:?} ended in status {:?} instead \
                    of 'valid'.", hostname, final_authz.status;
                    IO, Network, Invalid));
            }
        }
        Ok(())
    }

    /// Poll an authorisation URL until its status is no longer `pending`.
    /// Returns the terminal authorisation object.
    async fn poll_authorisation_until_final(
        &mut self,
        authz_url:  &str,
    )
        -> Outcome<Authorization>
    {
        for _ in 0..POLL_MAX_ATTEMPTS {
            let authz = res!(self.fetch_authorization(authz_url).await);
            if authz.status != "pending" && authz.status != "processing" {
                return Ok(authz);
            }
            tokio::time::sleep(POLL_INTERVAL).await;
        }
        Err(err!(
            "Authorisation {:?} did not leave 'pending' within {} poll \
            attempts.", authz_url, POLL_MAX_ATTEMPTS;
            IO, Network, Timeout))
    }

    /// Poll an order URL until its status is `ready`, `valid` or `invalid`.
    /// Used after challenges are satisfied, before the finalise POST.
    async fn poll_until_ready(
        &mut self,
        order_url:  &str,
    )
        -> Outcome<Order>
    {
        for _ in 0..POLL_MAX_ATTEMPTS {
            let order = res!(self.poll_order(order_url).await);
            match order.status.as_str() {
                "ready" | "valid" => return Ok(order),
                "invalid" => return Err(err!(
                    "Order {:?} transitioned to 'invalid' while waiting for \
                    authorisations to complete.", order_url;
                    IO, Network, Invalid)),
                _ => (),
            }
            tokio::time::sleep(POLL_INTERVAL).await;
        }
        Err(err!(
            "Order {:?} did not reach 'ready' within {} poll attempts.",
            order_url, POLL_MAX_ATTEMPTS;
            IO, Network, Timeout))
    }

    /// Poll an order URL until its status is `valid`. Used after the
    /// finalise POST to wait for the CA to issue the certificate.
    async fn poll_until_valid(
        &mut self,
        order_url:  &str,
    )
        -> Outcome<Order>
    {
        for _ in 0..POLL_MAX_ATTEMPTS {
            let order = res!(self.poll_order(order_url).await);
            match order.status.as_str() {
                "valid" => return Ok(order),
                "invalid" => return Err(err!(
                    "Order {:?} transitioned to 'invalid' during \
                    finalisation.", order_url;
                    IO, Network, Invalid)),
                _ => (),
            }
            tokio::time::sleep(POLL_INTERVAL).await;
        }
        Err(err!(
            "Order {:?} did not reach 'valid' within {} poll attempts \
            after finalisation.",
            order_url, POLL_MAX_ATTEMPTS;
            IO, Network, Timeout))
    }
}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ CONSTANTS                                                                 │
// └───────────────────────────────────────────────────────────────────────────┘

/// Interval between successive authorisation / order polls.
const POLL_INTERVAL: Duration = Duration::from_secs(2);

/// Maximum number of polls before giving up on a single order or
/// authorisation transition. Combined with `POLL_INTERVAL`, this gives a
/// total budget of roughly one minute.
const POLL_MAX_ATTEMPTS: u32 = 30;


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ HELPERS                                                                   │
// └───────────────────────────────────────────────────────────────────────────┘

/// Split an `https://host[:port]/path...` URL into its three components.
///
/// IPv6 literal hosts (`https://[::1]/path`) are not supported because
/// ACME traffic goes to DNS names in practice, and handling the bracketed
/// form would significantly complicate the parser.
pub(super) fn split_https_url(url: &str) -> Outcome<(String, u16, String)> {
    let rest = match url.strip_prefix("https://") {
        Some(r) => r,
        None    => return Err(err!(
            "URL {:?} does not start with the https:// scheme.", url;
            Invalid, Input, Mismatch)),
    };
    let (authority, path) = match rest.find('/') {
        Some(pos) => (&rest[..pos], &rest[pos..]),
        None      => (rest, "/"),
    };
    if authority.is_empty() {
        return Err(err!(
            "URL {:?} has an empty authority component.", url;
            Invalid, Input, Missing));
    }
    let (host, port) = match authority.rfind(':') {
        Some(pos) => {
            let port_str = &authority[pos + 1..];
            let port: u16 = match port_str.parse() {
                Ok(p) => p,
                Err(e) => return Err(err!(e,
                    "URL {:?} has an invalid port {:?}.", url, port_str;
                    Invalid, Input, Mismatch)),
            };
            (&authority[..pos], port)
        },
        None => (authority, 443u16),
    };
    Ok((host.to_string(), port, path.to_string()))
}

/// Extract the current HTTP status code from a response message as a
/// plain `u16`.
fn http_status_code(msg: &HttpMessage) -> u16 {
    match &msg.header.headline {
        crate::http::header::HttpHeadline::Response { status } => *status as u16,
        _ => 0,
    }
}

/// Read the `Replay-Nonce` header off a response, returning it as an
/// owned string.
fn read_replay_nonce(msg: &HttpMessage, context: &str) -> Outcome<String> {
    match msg.header.get_a_field_value(&HeaderName::ReplayNonce) {
        Some(HeaderFieldValue::Generic(s)) => Ok(s.clone()),
        Some(other) => Err(err!(
            "{}: Replay-Nonce header had unexpected parsed form {:?}.",
            context, other;
            IO, Network, Invalid, Mismatch)),
        None => Err(err!(
            "{}: Replay-Nonce header was missing from the response.", context;
            IO, Network, Missing)),
    }
}

/// Read the `Location` header off a response, returning it as an owned
/// string.
fn read_location(msg: &HttpMessage, context: &str) -> Outcome<String> {
    match msg.header.get_a_field_value(&HeaderName::Location) {
        Some(HeaderFieldValue::Generic(s)) => Ok(s.clone()),
        Some(other) => Err(err!(
            "{}: Location header had unexpected parsed form {:?}.",
            context, other;
            IO, Network, Invalid, Mismatch)),
        None => Err(err!(
            "{}: Location header was missing from the response.", context;
            IO, Network, Missing)),
    }
}

/// Ensure an HTTP response carries a 2xx status, otherwise turn it into
/// an Outcome error with any embedded ACME problem document folded in.
fn require_success(msg: &HttpMessage, context: &str) -> Outcome<()> {
    let status = http_status_code(msg);
    if status / 100 == 2 {
        return Ok(());
    }
    Err(res!(acme_error_from_response(msg, context)))
}

/// Build an error from a non-2xx ACME response. If the body parses as a
/// Problem document, include the `type` and `detail` in the error text.
fn acme_error_from_response(
    msg:        &HttpMessage,
    context:    &str,
)
    -> Outcome<Error<ErrTag>>
{
    let status = http_status_code(msg);
    let mut message = fmt!(
        "ACME server returned status {} on {}.",
        status, context,
    );
    if let Ok(Some(problem)) = parse_problem_body(&msg.body) {
        message.push_str(&fmt!(
            " Problem type: {:?}, detail: {:?}.",
            problem.typ, problem.detail,
        ));
    }
    Ok(err!(message.clone(); IO, Network, Unknown))
}

/// Try to parse the body as an RFC 7807 Problem document. Returns
/// `Ok(None)` if the body is empty or clearly not a JSON object.
fn parse_problem_body(body: &[u8]) -> Outcome<Option<Problem>> {
    if body.is_empty() {
        return Ok(None);
    }
    // Try to parse. If it fails, treat as no problem document.
    match parse_json_response::<Problem>(body) {
        Ok(p) => Ok(Some(p)),
        Err(_) => Ok(None),
    }
}

/// Extract the DNS name from an authorisation's `identifier` field.
fn dns_identifier(authz: &Authorization) -> Outcome<String> {
    match &authz.identifier {
        Dat::Map(m) => {
            let typ = match m.get(&dat!("type")) {
                Some(Dat::Str(s)) => s.clone(),
                _ => return Err(err!(
                    "Authorisation identifier has no `type` field.";
                    IO, Network, Missing, Invalid)),
            };
            if typ != "dns" {
                return Err(err!(
                    "Authorisation identifier type {:?} is not `dns`.", typ;
                    IO, Network, Invalid, Mismatch));
            }
            match m.get(&dat!("value")) {
                Some(Dat::Str(s)) => Ok(s.clone()),
                _ => Err(err!(
                    "Authorisation identifier has no `value` field.";
                    IO, Network, Missing, Invalid)),
            }
        },
        other => Err(err!(
            "Authorisation identifier is not a JSON object; got {:?}.", other;
            IO, Network, Invalid, Mismatch)),
    }
}

/// Build a CSR for the given DNS names using a fresh P-256 key pair.
/// Returns `(csr_der, key_pkcs8_der)`.
///
/// `rcgen::CertificateParams::new` defaults the distinguished name's
/// CommonName to the literal string `"rcgen self signed cert"`, which
/// Let's Encrypt rejects at the finalise step with `rejectedIdentifier:
/// Domain name contains an invalid character` because LE interprets the
/// CN as a candidate domain identifier and the default string contains
/// spaces. We replace the distinguished name with one whose CN is the
/// first DNS name being requested; this matches the CN to a valid SAN
/// and satisfies every CA we care about without producing a CN that the
/// CA would reject.
fn build_csr(dns_names: &[String]) -> Outcome<(Vec<u8>, Vec<u8>)> {
    let mut params = CertificateParams::new(dns_names.to_vec());
    let mut dn = DistinguishedName::new();
    if let Some(first) = dns_names.first() {
        dn.push(DnType::CommonName, first.clone());
    }
    params.distinguished_name = dn;
    let cert = match Certificate::from_params(params) {
        Ok(c) => c,
        Err(e) => return Err(err!(e,
            "rcgen::Certificate::from_params failed while building an \
            ACME CSR for {:?}.", dns_names;
            Init, Invalid)),
    };
    let csr_der = match cert.serialize_request_der() {
        Ok(b) => b,
        Err(e) => return Err(err!(e,
            "rcgen::Certificate::serialize_request_der failed while \
            building an ACME CSR for {:?}.", dns_names;
            Init, Invalid)),
    };
    let key_pkcs8 = cert.serialize_private_key_der();
    Ok((csr_der, key_pkcs8))
}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ TESTS                                                                     │
// └───────────────────────────────────────────────────────────────────────────┘

#[cfg(test)]
mod tests {
    use super::*;

    /// A bare hostname-only URL must parse to (host, 443, "/").
    #[test]
    fn test_split_url_default_port_root_path() -> Outcome<()> {
        let (host, port, path) = res!(split_https_url("https://acme.example"));
        if host != "acme.example" || port != 443 || path != "/" {
            return Err(err!(
                "Parsed as {:?}, {}, {:?}.", host, port, path;
                Test, Mismatch));
        }
        Ok(())
    }

    /// Host + path must preserve the path verbatim.
    #[test]
    fn test_split_url_with_path() -> Outcome<()> {
        let (host, port, path) = res!(split_https_url(
            "https://acme-v02.api.letsencrypt.org/directory"));
        if host != "acme-v02.api.letsencrypt.org"
            || port != 443
            || path != "/directory"
        {
            return Err(err!(
                "Parsed as {:?}, {}, {:?}.", host, port, path;
                Test, Mismatch));
        }
        Ok(())
    }

    /// Deeper paths and explicit ports must both parse correctly.
    #[test]
    fn test_split_url_with_port_and_deep_path() -> Outcome<()> {
        let (host, port, path) = res!(split_https_url(
            "https://acme-staging-v02.api.letsencrypt.org:8443/acme/authz/abc/1"));
        if host != "acme-staging-v02.api.letsencrypt.org"
            || port != 8443
            || path != "/acme/authz/abc/1"
        {
            return Err(err!(
                "Parsed as {:?}, {}, {:?}.", host, port, path;
                Test, Mismatch));
        }
        Ok(())
    }

    /// Query strings on the path must be preserved too.
    #[test]
    fn test_split_url_preserves_query() -> Outcome<()> {
        let (_host, _port, path) = res!(split_https_url(
            "https://example.test/acme/foo?bar=baz"));
        if path != "/acme/foo?bar=baz" {
            return Err(err!(
                "Expected path with query preserved, got {:?}.", path;
                Test, Mismatch));
        }
        Ok(())
    }

    /// Missing scheme must error.
    #[test]
    fn test_split_url_rejects_missing_scheme() -> Outcome<()> {
        match split_https_url("http://acme.example/") {
            Ok(_) => Err(err!(
                "split_https_url accepted a non-https scheme.";
                Test, Mismatch)),
            Err(_) => Ok(()),
        }
    }

    /// Empty authority must error.
    #[test]
    fn test_split_url_rejects_empty_authority() -> Outcome<()> {
        match split_https_url("https:///directory") {
            Ok(_) => Err(err!(
                "split_https_url accepted an empty authority.";
                Test, Mismatch)),
            Err(_) => Ok(()),
        }
    }

    /// Non-numeric port must error.
    #[test]
    fn test_split_url_rejects_non_numeric_port() -> Outcome<()> {
        match split_https_url("https://acme.example:abc/directory") {
            Ok(_) => Err(err!(
                "split_https_url accepted a non-numeric port.";
                Test, Mismatch)),
            Err(_) => Ok(()),
        }
    }

    /// `build_csr` must produce non-empty CSR and private key DER blobs
    /// for a one-name request, and the hostname bytes must appear in the
    /// CSR (IA5String encoding) confirming the SAN was written.
    #[test]
    fn test_build_csr_single_name() -> Outcome<()> {
        let names = vec!["example.com".to_string()];
        let (csr, key) = res!(build_csr(&names));
        if csr.is_empty() {
            return Err(err!("CSR DER was empty."; Test, Mismatch));
        }
        if key.is_empty() {
            return Err(err!("CSR private key was empty."; Test, Mismatch));
        }
        let needle = b"example.com";
        let found = csr.windows(needle.len()).any(|w| w == needle);
        if !found {
            return Err(err!(
                "CSR DER does not contain the requested hostname as a SAN.";
                Test, Missing));
        }
        Ok(())
    }

    /// Multi-name CSR must contain every requested hostname.
    #[test]
    fn test_build_csr_multi_name() -> Outcome<()> {
        let names = vec![
            "example.com".to_string(),
            "www.example.com".to_string(),
        ];
        let (csr, _key) = res!(build_csr(&names));
        for host in &names {
            let needle = host.as_bytes();
            if !csr.windows(needle.len()).any(|w| w == needle) {
                return Err(err!(
                    "CSR DER does not contain {:?}.", host;
                    Test, Missing));
            }
        }
        Ok(())
    }
}
