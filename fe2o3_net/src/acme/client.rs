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
//!
//! [Written entirely with AI](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

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
            AuthorizationStatus,
            Challenge,
            ChallengeStatus,
            Directory,
            Order,
            OrderStatus,
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

    /// By the time this returns, any incoming TLS handshake for `hostname` that
    /// advertises the `acme-tls/1` ALPN protocol must be answered with `cert`.
    fn install(&self, hostname: &str, cert: &ChallengeCert) -> Outcome<()>;

    /// Called once the CA has validated the challenge or given up on it, either
    /// way.
    fn remove(&self, hostname: &str) -> Outcome<()>;
}

/// A freshly-issued certificate chain plus its matching private key.
///
/// The key is **not** the ACME account key: it is the fresh P-256 pair rcgen
/// minted while building the CSR, whose public half the CA therefore knows.
#[derive(Clone, Debug)]
pub struct IssuedCertificate {
    pub cert_pem:   Vec<u8>,    // PEM chain, exactly as the CA sent it
    pub key_pkcs8:  Vec<u8>,    // PKCS#8 DER, matching the leaf cert
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
    directory_url:  String,               // full URL of the CA directory endpoint
    contact_email:  String,               // the account is registered with this
    tls_config:     Arc<ClientConfig>,    // trusts the CA's root anchors
    signer:         JwsSigner,            // account key, minted or loaded by the caller
    directory:      Option<Directory>,    // cached after the first fetch
    kid:            Option<String>,       // account URL, set by register_account
    nonce:          Option<String>,       // latest Replay-Nonce, spent on the next POST
}

impl AcmeClient {

    /// Does no I/O; the directory and the first nonce are fetched lazily.
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

    pub fn signer(&self) -> &JwsSigner {
        &self.signer
    }

    /// The account URL the CA assigned, `None` until `register_account` has run.
    pub fn kid(&self) -> Option<&str> {
        self.kid.as_deref()
    }

    // ---- low-level helpers -----------------------------------------------

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

    /// RFC 8555 §7.2 permits GET or HEAD on the new-nonce endpoint. GET is used
    /// because the HTTP reader here always expects a body frame, possibly empty.
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

    /// Fetches one first where none is stashed, the reply stashing it into
    /// `self.nonce` for this call to take straight back out.
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

    /// Only the new-account request signs under `jwk`, the CA knowing no account
    /// URL for it yet.
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

    /// Every authenticated request after `register_account` signs under `kid`.
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

    /// The empty-payload POST-as-GET of RFC 8555 §6.3, under a `kid` header.
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

    /// `use_jwk` picks the `jwk` header of first contact over the `kid` of every
    /// later request. The stashed nonce is updated from the reply, and a
    /// `badNonce` from the server is retried once with the nonce it returned.
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

    /// `post_jose` with an empty payload, still under `kid`.
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

    /// Registers or recovers the account belonging to the signer key, storing
    /// the reply's `Location` header as the `kid`.
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

    /// The string is the order's `Location` URL, which is POST-as-GET'd to poll
    /// the status.
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

    pub async fn fetch_authorization(
        &mut self,
        authz_url:  &str,
    )
        -> Outcome<Authorization>
    {
        let msg = res!(self.post_as_get(authz_url).await);
        parse_json_response(&msg.body)
    }

    /// POSTs `{}` to the challenge URL, which is how readiness is signalled.
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

    pub async fn poll_order(
        &mut self,
        order_url:  &str,
    )
        -> Outcome<Order>
    {
        let msg = res!(self.post_as_get(order_url).await);
        parse_json_response(&msg.body)
    }

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

    /// The body verbatim, an `application/pem-certificate-chain` PEM chain.
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

    /// Drives the whole RFC 8555 cycle, installing challenge certs through
    /// `installer` and removing every one of them afterwards, success or not.
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

        // RFC 8555 §7.1.3: act on the state the order actually arrived in.
        // A brand new order is usually `pending`, but when the CA still holds
        // cached validations for every name it can hand us one that is already
        // `ready`, with no authorisation work left to do at all.
        match res!(order_step(&order, &order_url)) {
            OrderStep::Authorise => {
                // Remember the hostnames we installed challenge certs for, so
                // we can remove them all at the end regardless of success.
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
                        // Log-worthy but not fatal; the issuance may still be
                        // on track.
                        warn!("ACME: installer.remove({:?}) failed: {:?}", host, e);
                    }
                }
                res!(drive_result);

                // Poll the order until the CA marks it ready to be finalised.
                order = res!(self.poll_until_ready(&order_url).await);
            },
            OrderStep::Finalise => {
                info!(
                    "ACME: order for {:?} arrived 'ready'; every \
                    authorisation was already valid, going straight to \
                    finalisation.", dns_names,
                );
            },
        }

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

    /// Walk every authorisation attached to `order` and bring each one to the
    /// `valid` state, doing only the work its current status calls for.
    ///
    /// An authorisation that is already `valid` is skipped without a challenge
    /// POST: the CA caches successful validations (RFC 8555 §7.1.4), so this
    /// is the normal shape of any order placed after a previous issuance for
    /// the same name got as far as validating it.
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

            let step = res!(authz_step(&authz, &hostname));
            // Whether to signal readiness is decided here, with the challenge borrowed rather than
            // taken, because the challenge is needed either way and the decision is needed after.
            let (chall, prove) = match &step {
                AuthzStep::Skip => {
                    // Already validated by the CA; nothing to prove. POSTing
                    // the challenge here is exactly what Boulder answers with
                    // `400 malformed`.
                    info!(
                        "ACME: authorisation for {:?} is already valid; \
                        no challenge needed.", hostname,
                    );
                    continue;
                },
                AuthzStep::Prove(c)             => (c, true),
                AuthzStep::AwaitValidation(c)   => (c, false),
            };

            // The challenge cert has to be reachable before the CA looks, and
            // it must stay up while a validation that is already in flight
            // completes -- so we install it for `AwaitValidation` too.
            let thumbprint = res!(self.signer.jwk_thumbprint_sha256());
            let key_auth = chall.key_authorization(&thumbprint);
            let cert = res!(build_tls_alpn_01_cert(&hostname, &key_auth));

            res!(installer.install(&hostname, &cert));
            installed_hosts.push(hostname.clone());

            // Only signal readiness on a challenge that is still `pending`.
            // Re-POSTing one the CA is already validating is at best wasted
            // and at worst rejected.
            if prove {
                let _ = res!(self.signal_challenge_ready(&chall.url).await);
            }

            // Poll the authorisation itself until it reaches a terminal state.
            let final_authz = res!(self.poll_authorisation_until_final(authz_url).await);
            match res!(final_authz.typed_status()) {
                AuthorizationStatus::Valid => (),
                other => return Err(err!(
                    "Authorisation for {:?} ended in status {:?} instead of \
                    'valid'.", hostname, other.as_wire();
                    IO, Network, Invalid)),
            }
        }
        Ok(())
    }

    async fn poll_authorisation_until_final(
        &mut self,
        authz_url:  &str,
    )
        -> Outcome<Authorization>
    {
        for _ in 0..POLL_MAX_ATTEMPTS {
            let authz = res!(self.fetch_authorization(authz_url).await);
            // RFC 8555 §7.1.6: an authorisation has no `processing` state; it
            // leaves `pending` straight for a terminal one.
            match res!(authz.typed_status()) {
                AuthorizationStatus::Pending => (),
                _ => return Ok(authz),
            }
            tokio::time::sleep(POLL_INTERVAL).await;
        }
        Err(err!(
            "Authorisation {:?} did not leave 'pending' within {} poll \
            attempts.", authz_url, POLL_MAX_ATTEMPTS;
            IO, Network, Timeout))
    }

    /// `valid` returns as well as `ready`: an order the CA finalised while this
    /// was polling is past ready, not short of it.
    async fn poll_until_ready(
        &mut self,
        order_url:  &str,
    )
        -> Outcome<Order>
    {
        for _ in 0..POLL_MAX_ATTEMPTS {
            let order = res!(self.poll_order(order_url).await);
            match res!(order.typed_status()) {
                OrderStatus::Ready | OrderStatus::Valid => return Ok(order),
                OrderStatus::Invalid => return Err(res!(order_invalid_error(
                    &order,
                    order_url,
                    "while waiting for authorisations to complete",
                ))),
                // Still settling: the CA has not yet caught up with the
                // authorisations we just satisfied.
                OrderStatus::Pending | OrderStatus::Processing => (),
            }
            tokio::time::sleep(POLL_INTERVAL).await;
        }
        Err(err!(
            "Order {:?} did not reach 'ready' within {} poll attempts.",
            order_url, POLL_MAX_ATTEMPTS;
            IO, Network, Timeout))
    }

    async fn poll_until_valid(
        &mut self,
        order_url:  &str,
    )
        -> Outcome<Order>
    {
        for _ in 0..POLL_MAX_ATTEMPTS {
            let order = res!(self.poll_order(order_url).await);
            match res!(order.typed_status()) {
                OrderStatus::Valid => return Ok(order),
                OrderStatus::Invalid => return Err(res!(order_invalid_error(
                    &order,
                    order_url,
                    "during finalisation",
                ))),
                // `processing` is the CA issuing; `pending` and `ready` mean
                // it has not yet registered our CSR.
                OrderStatus::Pending
                    | OrderStatus::Ready
                    | OrderStatus::Processing => (),
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

const POLL_INTERVAL: Duration = Duration::from_secs(2);

const POLL_MAX_ATTEMPTS: u32 = 30; // 30 x 2 s, about a minute per transition


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ HELPERS                                                                   │
// └───────────────────────────────────────────────────────────────────────────┘

/// Splits `https://host[:port]/path...` into host, port and path.
///
/// An IPv6 literal host, `https://[::1]/path`, is not supported: ACME traffic
/// goes to DNS names in practice, and the bracketed form costs more parser than
/// it is worth.
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

fn http_status_code(msg: &HttpMessage) -> u16 {
    match &msg.header.headline {
        crate::http::header::HttpHeadline::Response { status } => *status as u16,
        _ => 0,
    }
}

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

/// Anything but a 2xx becomes an error with the embedded ACME problem document
/// folded into its message.
fn require_success(msg: &HttpMessage, context: &str) -> Outcome<()> {
    let status = http_status_code(msg);
    if status / 100 == 2 {
        return Ok(());
    }
    Err(res!(acme_error_from_response(msg, context)))
}

/// A body that parses as a Problem document contributes its `type` and `detail`
/// to the error text.
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

/// RFC 7807. `Ok(None)` where the body is empty or is not a JSON object, since a
/// missing problem document is not itself a failure.
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

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ STATE DECISIONS (RFC 8555 §7.1.3, §7.1.4, §7.1.6)                         │
// └───────────────────────────────────────────────────────────────────────────┘
//
// The two functions below are the whole of the client's protocol reasoning,
// deliberately kept pure: they take a parsed order or authorisation and say
// what must happen next, with no I/O anywhere near them. The async drivers are
// thin executors of their verdict. That split is what makes the state machine
// testable against captured CA responses without a network.

/// What the client must do with a single authorisation.
#[derive(Clone, Debug)]
pub(super) enum AuthzStep {
    Skip,                          // already validated, nothing left to prove
    Prove(Challenge),              // install the cert, then POST the challenge
    AwaitValidation(Challenge),    // keep the cert up and poll, but do not re-POST
}

/// What the client must do with a freshly-created order.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum OrderStep {
    Authorise,    // authorisations are outstanding and must be satisfied first
    Finalise,     // every authorisation is already valid, go straight to the CSR
}

/// RFC 8555 §7.1.4 and §7.1.6.
///
/// The `valid` arm is the one that matters: a CA caches successful validations
/// (Let's Encrypt for roughly 30 days), so any order placed after an issuance
/// that got as far as validating a name will carry an authorisation that is
/// already `valid`. Treating that as though it were `pending` -- which is what
/// an unconditional challenge POST amounts to -- turns a single transient
/// failure into a permanent one, because every retry thereafter POSTs a
/// challenge whose authorisation is no longer pending and is refused.
pub(super) fn authz_step(
    authz:      &Authorization,
    hostname:   &str,
)
    -> Outcome<AuthzStep>
{
    match res!(authz.typed_status()) {
        // Nothing to do. This is the cached-validation case.
        AuthorizationStatus::Valid => Ok(AuthzStep::Skip),

        // Dead authorisations. Naming both the domain and the status matters:
        // this error is the only thing an operator will see, and "invalid" and
        // "expired" call for quite different responses.
        AuthorizationStatus::Invalid
            | AuthorizationStatus::Expired
            | AuthorizationStatus::Revoked
            | AuthorizationStatus::Deactivated => {
            let status = res!(authz.typed_status());
            let mut msg = fmt!(
                "Authorisation for {:?} is in status {:?} and cannot be \
                satisfied; a new order is required.",
                hostname, status.as_wire(),
            );
            // Surface the CA's own explanation when it attached one to the
            // challenge that failed.
            if let Ok(Some(chall)) = authz.tls_alpn_01_challenge() {
                if let Ok(Some(problem)) = chall.typed_error() {
                    msg.push_str(&fmt!(
                        " CA problem type: {:?}, detail: {:?}.",
                        problem.typ, problem.detail,
                    ));
                }
            }
            Err(err!(msg.clone(); IO, Network, Invalid))
        },

        // The ordinary first-issuance path: something still has to be proved.
        AuthorizationStatus::Pending => {
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
            match res!(chall.typed_status()) {
                ChallengeStatus::Pending => {
                    if chall.url.is_empty() {
                        return Err(err!(
                            "tls-alpn-01 challenge for {:?} has an empty url, \
                            so readiness cannot be signalled.", hostname;
                            IO, Network, Missing, Invalid));
                    }
                    Ok(AuthzStep::Prove(chall))
                },
                // Already signalled, or already validated while the enclosing
                // authorisation has yet to catch up: either way, keep the cert
                // up and wait rather than POSTing again.
                ChallengeStatus::Processing
                    | ChallengeStatus::Valid => Ok(AuthzStep::AwaitValidation(chall)),
                ChallengeStatus::Invalid => {
                    let mut msg = fmt!(
                        "The tls-alpn-01 challenge for {:?} is in status \
                        'invalid'; a new order is required.", hostname,
                    );
                    if let Ok(Some(problem)) = chall.typed_error() {
                        msg.push_str(&fmt!(
                            " CA problem type: {:?}, detail: {:?}.",
                            problem.typ, problem.detail,
                        ));
                    }
                    Err(err!(msg.clone(); IO, Network, Invalid))
                },
            }
        },
    }
}

/// RFC 8555 §7.1.3.
///
/// `pending` and `ready` are the two states a `newOrder` reply can legitimately
/// arrive in. The other three are handled explicitly rather than swept into a
/// catch-all, because each means something quite specific has gone sideways.
pub(super) fn order_step(
    order:      &Order,
    order_url:  &str,
)
    -> Outcome<OrderStep>
{
    match res!(order.typed_status()) {
        OrderStatus::Pending => Ok(OrderStep::Authorise),

        // Every authorisation is already valid, from the CA's cache. There is
        // no challenge to answer -- the order wants a CSR and nothing else.
        OrderStatus::Ready => Ok(OrderStep::Finalise),

        OrderStatus::Invalid => Err(res!(order_invalid_error(
            order,
            order_url,
            "on creation",
        ))),

        // Both of these mean the order has already been finalised, against a
        // CSR whose private key belonged to some earlier issuance attempt.
        // That key is not recoverable here (we mint a fresh one per issuance),
        // so the certificate at the far end of this order is unusable to us:
        // installing it would mean serving a chain whose private key we do not
        // hold, and every TLS handshake would fail. Erroring lets the caller's
        // retry place a clean order, which is the recoverable outcome.
        OrderStatus::Processing
            | OrderStatus::Valid => {
            let status = res!(order.typed_status());
            Err(err!(
                "Order {:?} arrived in status {:?}, meaning it was already \
                finalised against a CSR from an earlier attempt whose private \
                key this process does not hold. The resulting certificate \
                cannot be served. A fresh order is required.",
                order_url, status.as_wire();
                IO, Network, Invalid, Conflict))
        },
    }
}

/// Folds in the CA's problem document where one is attached; RFC 8555 §7.1.3
/// puts it on `error`.
fn order_invalid_error(
    order:      &Order,
    order_url:  &str,
    context:    &str,
)
    -> Outcome<Error<ErrTag>>
{
    let mut msg = fmt!(
        "Order {:?} transitioned to 'invalid' {}.", order_url, context,
    );
    match order.typed_error() {
        Ok(Some(problem)) => msg.push_str(&fmt!(
            " CA problem type: {:?}, title: {:?}, detail: {:?}.",
            problem.typ, problem.title, problem.detail,
        )),
        Ok(None) => msg.push_str(" The CA attached no problem document."),
        Err(e) => msg.push_str(&fmt!(
            " The CA's problem document could not be parsed: {:?}.", e,
        )),
    }
    Ok(err!(msg.clone(); IO, Network, Invalid))
}

/// Errors unless the identifier is of type `dns`, which is the only kind this
/// client can satisfy.
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

/// A CSR over a fresh P-256 key pair, returned as `(csr_der, key_pkcs8_der)`.
///
/// `rcgen::CertificateParams::new` defaults the distinguished name's CommonName
/// to the literal `"rcgen self signed cert"`, which Let's Encrypt rejects at the
/// finalise step with `rejectedIdentifier: Domain name contains an invalid
/// character`: LE reads the CN as a candidate domain identifier and that string
/// has spaces in it. The distinguished name is replaced with one whose CN is the
/// first DNS name requested, matching the CN to a valid SAN.
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

    // ---- authorisation state machine (RFC 8555 §7.1.4, §7.1.6) -----------

    /// Build an authorisation body with the given authorisation status and
    /// tls-alpn-01 challenge status, shaped as Let's Encrypt actually sends
    /// them.
    fn authz_json(authz_status: &str, chall_status: &str) -> Vec<u8> {
        fmt!(r#"{{
            "status":     "{}",
            "expires":    "2026-08-01T12:00:00Z",
            "identifier": {{"type":"dns","value":"example.com"}},
            "challenges": [
                {{
                    "type":   "http-01",
                    "status": "{}",
                    "url":    "https://acme-v02.api.letsencrypt.org/acme/chall/1/a",
                    "token":  "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                }},
                {{
                    "type":   "tls-alpn-01",
                    "status": "{}",
                    "url":    "https://acme-v02.api.letsencrypt.org/acme/chall/1/c",
                    "token":  "cccccccccccccccccccccccccccccccc"
                }}
            ]
        }}"#, authz_status, chall_status, chall_status).into_bytes()
    }

    /// **Defect regression.** An authorisation the CA has already validated --
    /// which is what a renewal order carries, because Let's Encrypt caches a
    /// successful validation for around 30 days -- must be skipped outright.
    ///
    /// The old code POSTed the challenge unconditionally. Boulder answers a
    /// POST to a challenge of a non-pending authorisation with `400 malformed`,
    /// so every retry after a transient mid-issuance failure failed the same
    /// way, every 24 hours, until the certificate expired.
    #[test]
    fn test_authz_step_skips_already_valid_authorisation() -> Outcome<()> {
        let authz: Authorization = res!(parse_json_response(
            &authz_json("valid", "valid")));
        match res!(authz_step(&authz, "example.com")) {
            AuthzStep::Skip => Ok(()),
            other => Err(err!(
                "A `valid` authorisation must be skipped, not challenged; \
                authz_step returned {:?}.", other;
                Test, Mismatch)),
        }
    }

    /// The ordinary first-issuance path: a pending authorisation with a
    /// pending challenge must still be proved.
    #[test]
    fn test_authz_step_proves_pending_authorisation() -> Outcome<()> {
        let authz: Authorization = res!(parse_json_response(
            &authz_json("pending", "pending")));
        match res!(authz_step(&authz, "example.com")) {
            AuthzStep::Prove(c) => {
                if c.token != "cccccccccccccccccccccccccccccccc" {
                    return Err(err!(
                        "authz_step selected the wrong challenge: token {:?}.",
                        c.token;
                        Test, Mismatch));
                }
                if !c.url.ends_with("/chall/1/c") {
                    return Err(err!(
                        "authz_step selected the wrong challenge: url {:?}.",
                        c.url;
                        Test, Mismatch));
                }
                Ok(())
            },
            other => Err(err!(
                "A `pending` authorisation must be proved; authz_step \
                returned {:?}.", other;
                Test, Mismatch)),
        }
    }

    /// A challenge already being validated must not be POSTed a second time,
    /// but its cert must still be installed -- so the step is
    /// `AwaitValidation`, not `Prove` and not `Skip`.
    #[test]
    fn test_authz_step_awaits_processing_challenge() -> Outcome<()> {
        let authz: Authorization = res!(parse_json_response(
            &authz_json("pending", "processing")));
        match res!(authz_step(&authz, "example.com")) {
            AuthzStep::AwaitValidation(_) => Ok(()),
            other => Err(err!(
                "A `processing` challenge must be awaited, not re-POSTed; \
                authz_step returned {:?}.", other;
                Test, Mismatch)),
        }
    }

    /// Every dead authorisation status must produce an error that names both
    /// the domain and the status -- never a silent skip, which would let the
    /// client sail on to finalisation and fail confusingly later.
    #[test]
    fn test_authz_step_errors_on_dead_statuses() -> Outcome<()> {
        for status in ["invalid", "expired", "revoked", "deactivated"] {
            let authz: Authorization = res!(parse_json_response(
                &authz_json(status, "invalid")));
            match authz_step(&authz, "example.com") {
                Ok(step) => return Err(err!(
                    "Authorisation status {:?} must be an error, but \
                    authz_step returned {:?}.", status, step;
                    Test, Mismatch)),
                Err(e) => {
                    let msg = fmt!("{}", e);
                    if !msg.contains("example.com") {
                        return Err(err!(
                            "The error for status {:?} does not name the \
                            domain: {}", status, msg;
                            Test, Missing));
                    }
                    if !msg.contains(status) {
                        return Err(err!(
                            "The error for status {:?} does not name the \
                            status: {}", status, msg;
                            Test, Missing));
                    }
                },
            }
        }
        Ok(())
    }

    /// When the CA explains why a challenge failed, that explanation must
    /// reach the operator rather than being swallowed.
    #[test]
    fn test_authz_step_surfaces_ca_problem_document() -> Outcome<()> {
        let body = br#"{
            "status":     "invalid",
            "identifier": {"type":"dns","value":"example.com"},
            "challenges": [
                {
                    "type":   "tls-alpn-01",
                    "status": "invalid",
                    "url":    "https://acme-v02.api.letsencrypt.org/acme/chall/1/c",
                    "token":  "cccccccccccccccccccccccccccccccc",
                    "error":  {
                        "type":   "urn:ietf:params:acme:error:unauthorized",
                        "detail": "Timeout during connect (likely firewall problem)",
                        "status": 403
                    }
                }
            ]
        }"#;
        let authz: Authorization = res!(parse_json_response(body));
        match authz_step(&authz, "example.com") {
            Ok(step) => Err(err!(
                "An invalid authorisation must error, got {:?}.", step;
                Test, Mismatch)),
            Err(e) => {
                let msg = fmt!("{}", e);
                if !msg.contains("Timeout during connect") {
                    return Err(err!(
                        "The CA's problem detail was not surfaced: {}", msg;
                        Test, Missing));
                }
                Ok(())
            },
        }
    }

    /// A pending authorisation that offers no tls-alpn-01 challenge at all is
    /// unsatisfiable by this client and must say so.
    #[test]
    fn test_authz_step_errors_when_no_tls_alpn_challenge() -> Outcome<()> {
        let body = br#"{
            "status":     "pending",
            "identifier": {"type":"dns","value":"example.com"},
            "challenges": [
                {
                    "type":   "http-01",
                    "status": "pending",
                    "url":    "https://acme-v02.api.letsencrypt.org/acme/chall/1/a",
                    "token":  "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                }
            ]
        }"#;
        let authz: Authorization = res!(parse_json_response(body));
        match authz_step(&authz, "example.com") {
            Ok(step) => Err(err!(
                "Expected an error when no tls-alpn-01 challenge is offered, \
                got {:?}.", step;
                Test, Mismatch)),
            Err(_) => Ok(()),
        }
    }

    // ---- order state machine (RFC 8555 §7.1.3) ---------------------------

    /// Build an order body in the given status.
    fn order_json(status: &str, extra: &str) -> Vec<u8> {
        fmt!(r#"{{
            "status":     "{}",
            "identifiers": [{{"type":"dns","value":"example.com"}}],
            "authorizations": ["https://acme-v02.api.letsencrypt.org/acme/authz/1"],
            "finalize":   "https://acme-v02.api.letsencrypt.org/acme/finalize/1"{}
        }}"#, status, extra).into_bytes()
    }

    /// A `pending` order means authorisations are outstanding.
    #[test]
    fn test_order_step_pending_authorises() -> Outcome<()> {
        let order: Order = res!(parse_json_response(&order_json("pending", "")));
        let step = res!(order_step(&order, "https://example.test/order/1"));
        if step != OrderStep::Authorise {
            return Err(err!(
                "A `pending` order must be authorised, got {:?}.", step;
                Test, Mismatch));
        }
        Ok(())
    }

    /// **Defect regression.** An order that arrives already `ready` -- every
    /// authorisation validated from the CA's cache -- must go straight to
    /// finalisation, with no challenge POSTed for any of its authorisations.
    #[test]
    fn test_order_step_ready_goes_straight_to_finalisation() -> Outcome<()> {
        let order: Order = res!(parse_json_response(&order_json("ready", "")));
        let step = res!(order_step(&order, "https://example.test/order/1"));
        if step != OrderStep::Finalise {
            return Err(err!(
                "A `ready` order must go straight to finalisation, got {:?}.",
                step;
                Test, Mismatch));
        }
        Ok(())
    }

    /// An `invalid` order must error, and must carry the CA's problem
    /// document into the message rather than dropping it.
    #[test]
    fn test_order_step_invalid_surfaces_problem_document() -> Outcome<()> {
        let extra = r#",
            "error": {
                "type":   "urn:ietf:params:acme:error:rateLimited",
                "title":  "Too many certificates already issued",
                "detail": "too many certificates already issued for example.com",
                "status": 429
            }"#;
        let order: Order = res!(parse_json_response(&order_json("invalid", extra)));
        match order_step(&order, "https://example.test/order/1") {
            Ok(step) => Err(err!(
                "An `invalid` order must error, got {:?}.", step;
                Test, Mismatch)),
            Err(e) => {
                let msg = fmt!("{}", e);
                if !msg.contains("too many certificates already issued") {
                    return Err(err!(
                        "The CA's problem detail was not surfaced: {}", msg;
                        Test, Missing));
                }
                if !msg.contains("rateLimited") {
                    return Err(err!(
                        "The CA's problem type was not surfaced: {}", msg;
                        Test, Missing));
                }
                Ok(())
            },
        }
    }

    /// An order that is already `valid` or `processing` was finalised against
    /// a CSR from an earlier attempt, whose private key we do not hold. Its
    /// certificate is therefore unusable and must not be quietly returned:
    /// serving a chain whose key we lack would fail every TLS handshake while
    /// looking, to the renewal check, like a perfectly fresh certificate.
    #[test]
    fn test_order_step_errors_on_already_finalised_order() -> Outcome<()> {
        for status in ["valid", "processing"] {
            let extra = r#",
            "certificate": "https://acme-v02.api.letsencrypt.org/acme/cert/abc""#;
            let order: Order = res!(parse_json_response(&order_json(status, extra)));
            match order_step(&order, "https://example.test/order/1") {
                Ok(step) => return Err(err!(
                    "An order in status {:?} must error rather than yield \
                    {:?}.", status, step;
                    Test, Mismatch)),
                Err(e) => {
                    let msg = fmt!("{}", e);
                    if !msg.contains(status) {
                        return Err(err!(
                            "The error for an order in status {:?} does not \
                            name the status: {}", status, msg;
                            Test, Missing));
                    }
                },
            }
        }
        Ok(())
    }

    /// A status the RFC does not define must be refused outright rather than
    /// silently treated as one we know.
    #[test]
    fn test_order_step_rejects_unknown_status() -> Outcome<()> {
        let order: Order = res!(parse_json_response(&order_json("frobnicating", "")));
        match order_step(&order, "https://example.test/order/1") {
            Ok(step) => Err(err!(
                "An unknown order status must error, got {:?}.", step;
                Test, Mismatch)),
            Err(_) => Ok(()),
        }
    }

    // ---- URL splitting ---------------------------------------------------

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
