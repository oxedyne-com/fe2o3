//! Typed message shapes for RFC 8555 (ACME) JSON bodies.
//!
//! This module owns the Rust-side representation of every ACME request and
//! response body Steel actually sends or receives when driving a certificate
//! through a CA such as Let's Encrypt via `tls-alpn-01`. The goal is narrow
//! but complete:
//!
//! - Response bodies from the CA are parsed into typed structs via the
//!   existing `FromDatMap` derive. Field renames cover the camelCase
//!   convention used in the wire format (`newNonce`, `termsOfServiceAgreed`,
//!   etc.), and `#[optional]` marks every field that may legitimately be
//!   missing. **The `token` and `url` fields on `Challenge` are marked
//!   `#[optional]` specifically** because live Let's Encrypt staging
//!   responses sometimes contain challenge objects that omit them, and
//!   without this marking the derive would fail the whole parse with a
//!   `missing field 'token'` style error -- the exact regression the
//!   vendored `rustls-acme` patch existed to guard against.
//!
//! - Request bodies we send to the CA are built via tiny helper functions
//!   that return a `Dat::Map`, so callers get a typed value they can feed
//!   straight into [`crate::acme::jose::JwsSigner::sign_flattened`] as the
//!   JWS payload (after `.json()` and base64url).
//!
//! Nested compound fields (e.g. the identifier inside an authorisation, or
//! the list of challenges) stay as `Dat` / `Vec<Dat>` rather than recursing
//! through another derive, and the enclosing type exposes a small `typed_*`
//! helper that parses them on demand. This mirrors the pattern used by
//! `fe2o3_steel::srv::cfg::ServerConfig` where `vhosts: Dat` is extracted
//! via a dedicated `get_vhosts()` method.

use crate::acme::jose;

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_jdat::{
    prelude::*,
    string::dec::DecoderConfig,
    usr::{
        UsrKind,
        UsrKindCode,
        UsrKindId,
    },
};

use std::collections::BTreeMap;

use ring::digest::{
    Context,
    SHA256,
};


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ HELPERS                                                                   │
// └───────────────────────────────────────────────────────────────────────────┘

/// SHA-256 of the input, returned as a 32-byte array.
fn sha256(data: &[u8]) -> [u8; 32] {
    let mut ctx = Context::new(&SHA256);
    ctx.update(data);
    let digest = ctx.finish();
    let mut out = [0u8; 32];
    out.copy_from_slice(digest.as_ref());
    out
}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ RESPONSE PARSING                                                          │
// └───────────────────────────────────────────────────────────────────────────┘

/// Parse an HTTP response body as a JSON object and deserialise it into a
/// typed ACME struct via [`FromDatMap`].
///
/// The JSON decoder is configured in strict mode (no comments, no trailing
/// commas) so we accept only standards-compliant CA output.
pub fn parse_json_response<T: FromDatMap>(body: &[u8]) -> Outcome<T> {
    let s = match std::str::from_utf8(body) {
        Ok(s) => s.to_string(),
        Err(e) => return Err(err!(e,
            "ACME response body is not valid UTF-8.";
            IO, Network, Decode, Invalid, Input)),
    };
    let cfg: DecoderConfig<
        BTreeMap<UsrKindCode, UsrKind>,
        BTreeMap<String, UsrKindId>,
    > = DecoderConfig::json(None);
    let dat = res!(Dat::decode_string_with_config(s, &cfg));
    match dat {
        Dat::Map(m) => T::from_datmap(m),
        other => Err(err!(
            "Expected a JSON object at the ACME response root, got {:?}.",
            other;
            IO, Network, Invalid, Mismatch, Input)),
    }
}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ STATUS ENUMS (RFC 8555 §7.1.6)                                            │
// └───────────────────────────────────────────────────────────────────────────┘
//
// The wire structs below keep their `status` as a `String`, because the
// `FromDatMap` derive resolves a field's `Dat` getter from its declared type
// and knows nothing of our enums. Parsing the string into one of these enums
// at the point of use lets every decision the client makes be an exhaustive
// `match` rather than a scatter of `== "valid"` comparisons -- which is how a
// state the client never considered (an authorisation that arrives already
// `valid`) came to be handled by falling through into the wrong branch.

/// Order lifecycle status, RFC 8555 §7.1.6.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OrderStatus {
    /// Authorisations are outstanding.
    Pending,
    /// Every authorisation is valid; the order awaits a CSR.
    Ready,
    /// The CSR has been accepted and the CA is issuing.
    Processing,
    /// The certificate has been issued.
    Valid,
    /// The order has failed and cannot be recovered.
    Invalid,
}

impl OrderStatus {

    /// Parse the `status` member of an order as it appears on the wire.
    pub fn from_wire(s: &str) -> Outcome<Self> {
        match s {
            "pending"       => Ok(Self::Pending),
            "ready"         => Ok(Self::Ready),
            "processing"    => Ok(Self::Processing),
            "valid"         => Ok(Self::Valid),
            "invalid"       => Ok(Self::Invalid),
            other           => Err(err!(
                "Unknown ACME order status {:?}; RFC 8555 §7.1.6 defines \
                only pending, ready, processing, valid and invalid.", other;
                IO, Network, Invalid, Mismatch)),
        }
    }

    /// The wire form of this status.
    pub fn as_wire(&self) -> &'static str {
        match self {
            Self::Pending       => "pending",
            Self::Ready         => "ready",
            Self::Processing    => "processing",
            Self::Valid         => "valid",
            Self::Invalid       => "invalid",
        }
    }
}

/// Authorisation status, RFC 8555 §7.1.6.
///
/// Note there is no `processing` state: an authorisation goes straight from
/// `pending` to `valid` or `invalid` once its challenge is decided.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AuthorizationStatus {
    /// A challenge still has to be satisfied.
    Pending,
    /// The CA has validated the identifier. Validations are cached by the CA
    /// (Let's Encrypt for around 30 days), so a freshly-created order can
    /// legitimately carry authorisations that are already `valid`, with
    /// nothing left for the client to prove.
    Valid,
    /// A challenge was attempted and failed.
    Invalid,
    /// The client deactivated the authorisation.
    Deactivated,
    /// The authorisation passed its `expires` time.
    Expired,
    /// The CA revoked the authorisation.
    Revoked,
}

impl AuthorizationStatus {

    /// Parse the `status` member of an authorisation as it appears on the wire.
    pub fn from_wire(s: &str) -> Outcome<Self> {
        match s {
            "pending"       => Ok(Self::Pending),
            "valid"         => Ok(Self::Valid),
            "invalid"       => Ok(Self::Invalid),
            "deactivated"   => Ok(Self::Deactivated),
            "expired"       => Ok(Self::Expired),
            "revoked"       => Ok(Self::Revoked),
            other           => Err(err!(
                "Unknown ACME authorisation status {:?}; RFC 8555 §7.1.6 \
                defines only pending, valid, invalid, deactivated, expired \
                and revoked.", other;
                IO, Network, Invalid, Mismatch)),
        }
    }

    /// The wire form of this status.
    pub fn as_wire(&self) -> &'static str {
        match self {
            Self::Pending       => "pending",
            Self::Valid         => "valid",
            Self::Invalid       => "invalid",
            Self::Deactivated   => "deactivated",
            Self::Expired       => "expired",
            Self::Revoked       => "revoked",
        }
    }
}

/// Challenge status, RFC 8555 §7.1.6.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ChallengeStatus {
    /// The client has not yet signalled that it is ready to be validated.
    Pending,
    /// The client has signalled readiness and the CA is validating.
    Processing,
    /// The CA validated the challenge.
    Valid,
    /// The CA could not validate the challenge.
    Invalid,
}

impl ChallengeStatus {

    /// Parse the `status` member of a challenge as it appears on the wire.
    pub fn from_wire(s: &str) -> Outcome<Self> {
        match s {
            "pending"       => Ok(Self::Pending),
            "processing"    => Ok(Self::Processing),
            "valid"         => Ok(Self::Valid),
            "invalid"       => Ok(Self::Invalid),
            other           => Err(err!(
                "Unknown ACME challenge status {:?}; RFC 8555 §7.1.6 defines \
                only pending, processing, valid and invalid.", other;
                IO, Network, Invalid, Mismatch)),
        }
    }

    /// The wire form of this status.
    pub fn as_wire(&self) -> &'static str {
        match self {
            Self::Pending       => "pending",
            Self::Processing    => "processing",
            Self::Valid         => "valid",
            Self::Invalid       => "invalid",
        }
    }
}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ DIRECTORY (RFC 8555 §7.1.1)                                               │
// └───────────────────────────────────────────────────────────────────────────┘

/// The ACME directory document returned by a `GET {directory_url}`. Each
/// field is a fully-qualified URL that the client uses as the target for
/// subsequent requests.
#[derive(Clone, Debug, Default, FromDatMap)]
pub struct Directory {
    #[rename(name = "newNonce")]
    pub new_nonce:      String,
    #[rename(name = "newAccount")]
    pub new_account:    String,
    #[rename(name = "newOrder")]
    pub new_order:      String,
    #[rename(name = "revokeCert")]
    #[optional]
    pub revoke_cert:    String,
    #[rename(name = "keyChange")]
    #[optional]
    pub key_change:     String,
    /// Free-form metadata (terms of service URL, external account binding
    /// requirement, etc.). Kept as a raw `Dat` because we do not parse any
    /// of it in Steel today.
    #[optional]
    pub meta:           Dat,
}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ ACCOUNT (RFC 8555 §7.3)                                                   │
// └───────────────────────────────────────────────────────────────────────────┘

/// Account object returned by `POST {new_account}` and subsequent account
/// management requests. We care only about `status`; the rest is kept so
/// callers can log it if useful.
#[derive(Clone, Debug, Default, FromDatMap)]
pub struct Account {
    pub status:     String,
    #[optional]
    pub contact:    Vec<Dat>,
    #[optional]
    pub orders:     String,
}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ ORDER (RFC 8555 §7.1.3)                                                   │
// └───────────────────────────────────────────────────────────────────────────┘

/// Order object returned by `POST {new_order}` and by `POST-as-GET` polls of
/// an order URL while issuance is in flight.
#[derive(Clone, Debug, Default, FromDatMap)]
pub struct Order {
    /// Order lifecycle status: `pending`, `ready`, `processing`, `valid` or
    /// `invalid`. Compared as a plain string against RFC 8555 §7.1.6.
    pub status:             String,
    #[optional]
    pub expires:            String,
    /// List of `{"type":"dns","value":"<name>"}` maps.
    #[optional]
    pub identifiers:        Vec<Dat>,
    /// URLs of the authorisations the client must satisfy before the order
    /// can be finalised.
    pub authorizations:     Vec<String>,
    /// URL for the final CSR POST.
    pub finalize:           String,
    /// URL of the issued certificate. Absent until `status == "valid"`.
    #[optional]
    pub certificate:        String,
    /// RFC 7807 problem document attached by the CA when `status == "invalid"`.
    #[optional]
    pub error:              Dat,
}

impl Order {
    /// Parse the `status` string into a typed [`OrderStatus`].
    pub fn typed_status(&self) -> Outcome<OrderStatus> {
        OrderStatus::from_wire(&self.status)
    }

    /// Parse the nested `error` field into a typed [`Problem`], if present.
    pub fn typed_error(&self) -> Outcome<Option<Problem>> {
        match &self.error {
            Dat::Empty => Ok(None),
            Dat::Map(m) => Ok(Some(res!(Problem::from_datmap(m.clone())))),
            other => Err(err!(
                "Order.error is not a JSON object, got {:?}.", other;
                IO, Network, Invalid, Mismatch)),
        }
    }
}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ AUTHORISATION (RFC 8555 §7.1.4)                                           │
// └───────────────────────────────────────────────────────────────────────────┘

/// Authorisation object returned by `POST-as-GET {authz_url}`.
///
/// Every authorisation carries a list of challenges; ACME §8 specifies that
/// the client must satisfy **one** of them. Steel always uses `tls-alpn-01`.
#[derive(Clone, Debug, Default, FromDatMap)]
pub struct Authorization {
    pub status:         String,
    #[optional]
    pub expires:        String,
    /// `{"type":"dns","value":"<name>"}`.
    pub identifier:     Dat,
    /// Challenges the CA is willing to accept.
    pub challenges:     Vec<Dat>,
    /// Present when the authorisation is for a wildcard identifier.
    #[optional]
    pub wildcard:       bool,
}

impl Authorization {
    /// Parse the `status` string into a typed [`AuthorizationStatus`].
    pub fn typed_status(&self) -> Outcome<AuthorizationStatus> {
        AuthorizationStatus::from_wire(&self.status)
    }

    /// Parse each entry in [`Authorization::challenges`] into a typed
    /// [`Challenge`].
    pub fn typed_challenges(&self) -> Outcome<Vec<Challenge>> {
        let mut out = Vec::with_capacity(self.challenges.len());
        for (i, dat) in self.challenges.iter().enumerate() {
            match dat {
                Dat::Map(m) => out.push(res!(Challenge::from_datmap(m.clone()))),
                other => return Err(err!(
                    "Authorization.challenges[{}] is not a JSON object, got {:?}.",
                    i, other;
                    IO, Network, Invalid, Mismatch)),
            }
        }
        Ok(out)
    }

    /// Return the `tls-alpn-01` challenge in this authorisation, if any.
    pub fn tls_alpn_01_challenge(&self) -> Outcome<Option<Challenge>> {
        for chall in res!(self.typed_challenges()) {
            if chall.typ == "tls-alpn-01" {
                return Ok(Some(chall));
            }
        }
        Ok(None)
    }
}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ CHALLENGE (RFC 8555 §8)                                                   │
// └───────────────────────────────────────────────────────────────────────────┘

/// A single challenge on an authorisation.
///
/// Note on optional fields: both `url` and `token` are marked `#[optional]`
/// because Let's Encrypt's current staging responses sometimes contain
/// challenges with neither field (for challenge types Steel does not
/// participate in). Marking them optional makes `FromDatMap` default them
/// to empty strings rather than failing the whole authorisation parse.
/// Steel only ever reads `token` on `tls-alpn-01` challenges, so an empty
/// default on other variants is operationally harmless. This reproduces, in
/// the existing jdat derive, the effect of the `#[serde(default)]` attributes
/// that the vendored `rustls-acme` patch applied on top of upstream.
#[derive(Clone, Debug, Default, FromDatMap)]
pub struct Challenge {
    #[rename(name = "type")]
    pub typ:            String,
    pub status:         String,
    #[optional]
    pub url:            String,
    #[optional]
    pub token:          String,
    #[optional]
    pub validated:      String,
    #[optional]
    pub error:          Dat,
}

impl Challenge {
    /// Parse the `status` string into a typed [`ChallengeStatus`].
    pub fn typed_status(&self) -> Outcome<ChallengeStatus> {
        ChallengeStatus::from_wire(&self.status)
    }

    /// Compute the key authorisation string for this challenge as defined in
    /// RFC 8555 §8.1: `token || '.' || base64url(SHA-256(JWK))`. The account
    /// JWK thumbprint is supplied by the caller (typically
    /// [`crate::acme::jose::JwsSigner::jwk_thumbprint_sha256`]).
    pub fn key_authorization(&self, jwk_thumbprint: &[u8; 32]) -> String {
        fmt!("{}.{}", self.token, jose::base64url_encode(jwk_thumbprint))
    }

    /// Compute the `dns-01` TXT record value for this challenge as defined in
    /// RFC 8555 §8.4: the base64url encoding of the SHA-256 **digest of the
    /// key authorisation string**, not of the token and not of the raw
    /// thumbprint.
    ///
    /// The record is published at `_acme-challenge.<domain>`. Note the
    /// prehash: the digest is taken over the key authorisation's UTF-8 bytes,
    /// and the digest -- not the string -- is what gets base64url-encoded.
    pub fn dns_01_txt_value(&self, jwk_thumbprint: &[u8; 32]) -> String {
        let key_auth = self.key_authorization(jwk_thumbprint);
        let digest = sha256(key_auth.as_bytes());
        jose::base64url_encode(&digest)
    }

    /// Parse the nested `error` field into a typed [`Problem`], if present.
    pub fn typed_error(&self) -> Outcome<Option<Problem>> {
        match &self.error {
            Dat::Empty => Ok(None),
            Dat::Map(m) => Ok(Some(res!(Problem::from_datmap(m.clone())))),
            other => Err(err!(
                "Challenge.error is not a JSON object, got {:?}.", other;
                IO, Network, Invalid, Mismatch)),
        }
    }
}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ PROBLEM (RFC 7807, used by RFC 8555 for errors)                           │
// └───────────────────────────────────────────────────────────────────────────┘

/// A CA-supplied problem document describing why a request failed or why an
/// order or challenge ended up in the `invalid` state.
#[derive(Clone, Debug, Default, FromDatMap)]
pub struct Problem {
    #[rename(name = "type")]
    #[optional]
    pub typ:            String,
    #[optional]
    pub title:          String,
    #[optional]
    pub detail:         String,
    #[optional]
    pub status:         u32,
    #[optional]
    pub subproblems:    Vec<Dat>,
}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ REQUEST BUILDERS                                                          │
// └───────────────────────────────────────────────────────────────────────────┘

/// Build the payload for `POST {new_account}`.
///
/// `contact_mailto` is an email address (no `mailto:` prefix -- the helper
/// adds it). `terms_agreed` must be set to `true` to accept the CA's terms
/// of service, which is a requirement of every public CA we target.
pub fn new_account_request(
    contact_mailto:     &str,
    terms_agreed:       bool,
)
    -> Dat
{
    mapdat!{
        "termsOfServiceAgreed" => terms_agreed,
        "contact" => Dat::List(vec![dat!(fmt!("mailto:{}", contact_mailto))]),
    }
}

/// Build the payload for `POST {new_order}`.
///
/// Each entry in `dns_names` becomes an RFC 8555 §7.1.3 identifier of type
/// `"dns"`. The CA will mint one authorisation per distinct identifier.
pub fn new_order_request(dns_names: &[String]) -> Dat {
    let identifiers: Vec<Dat> = dns_names
        .iter()
        .map(|n| mapdat!{
            "type"  => "dns",
            "value" => n.clone(),
        })
        .collect();
    mapdat!{
        "identifiers" => Dat::List(identifiers),
    }
}

/// Build the payload for `POST {finalize_url}` once every authorisation is
/// satisfied. `csr_der_b64url` is the base64url-encoded DER of the CSR.
pub fn finalize_request(csr_der_b64url: &str) -> Dat {
    mapdat!{
        "csr" => csr_der_b64url.to_string(),
    }
}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ TESTS                                                                     │
// └───────────────────────────────────────────────────────────────────────────┘

#[cfg(test)]
mod tests {
    use super::*;

    /// Parse a realistic Directory response modelled on Let's Encrypt's
    /// current output.
    #[test]
    fn test_parse_directory() -> Outcome<()> {
        let body = br#"{
            "newNonce":   "https://acme-v02.api.letsencrypt.org/acme/new-nonce",
            "newAccount": "https://acme-v02.api.letsencrypt.org/acme/new-acct",
            "newOrder":   "https://acme-v02.api.letsencrypt.org/acme/new-order",
            "revokeCert": "https://acme-v02.api.letsencrypt.org/acme/revoke-cert",
            "keyChange":  "https://acme-v02.api.letsencrypt.org/acme/key-change",
            "meta": {
                "termsOfService": "https://letsencrypt.org/documents/LE-SA-v1.5-February-24-2025.pdf",
                "website":        "https://letsencrypt.org"
            }
        }"#;
        let dir: Directory = res!(parse_json_response(body));
        if !dir.new_nonce.ends_with("/new-nonce") {
            return Err(err!(
                "newNonce parsed as {:?}", dir.new_nonce;
                Test, Mismatch));
        }
        if !dir.new_account.ends_with("/new-acct") {
            return Err(err!(
                "newAccount parsed as {:?}", dir.new_account;
                Test, Mismatch));
        }
        if !dir.new_order.ends_with("/new-order") {
            return Err(err!(
                "newOrder parsed as {:?}", dir.new_order;
                Test, Mismatch));
        }
        Ok(())
    }

    /// Parse an Account response and verify the status round-trips.
    #[test]
    fn test_parse_account() -> Outcome<()> {
        let body = br#"{
            "status":  "valid",
            "contact": ["mailto:hello@example.test"],
            "orders":  "https://acme-v02.api.letsencrypt.org/acme/acct/1/orders"
        }"#;
        let account: Account = res!(parse_json_response(body));
        if account.status != "valid" {
            return Err(err!(
                "account.status parsed as {:?}", account.status;
                Test, Mismatch));
        }
        if account.contact.len() != 1 {
            return Err(err!(
                "account.contact has {} entries, expected 1.", account.contact.len();
                Test, Mismatch));
        }
        Ok(())
    }

    /// Parse an Order in the `pending` state and verify the authorisation
    /// URLs survive.
    #[test]
    fn test_parse_order_pending() -> Outcome<()> {
        let body = br#"{
            "status":    "pending",
            "expires":   "2026-05-01T12:00:00Z",
            "identifiers": [
                {"type":"dns","value":"example.com"},
                {"type":"dns","value":"www.example.com"}
            ],
            "authorizations": [
                "https://acme-v02.api.letsencrypt.org/acme/authz/1",
                "https://acme-v02.api.letsencrypt.org/acme/authz/2"
            ],
            "finalize": "https://acme-v02.api.letsencrypt.org/acme/finalize/1"
        }"#;
        let order: Order = res!(parse_json_response(body));
        if order.status != "pending" {
            return Err(err!("order.status parsed as {:?}", order.status;
                Test, Mismatch));
        }
        if order.authorizations.len() != 2 {
            return Err(err!(
                "order.authorizations has {} entries, expected 2.",
                order.authorizations.len();
                Test, Mismatch));
        }
        if !order.finalize.ends_with("/finalize/1") {
            return Err(err!("order.finalize parsed as {:?}", order.finalize;
                Test, Mismatch));
        }
        if !order.certificate.is_empty() {
            return Err(err!(
                "order.certificate should default to empty when absent, got {:?}.",
                order.certificate;
                Test, Mismatch));
        }
        Ok(())
    }

    /// Parse an Order in the `valid` state with a certificate URL attached.
    #[test]
    fn test_parse_order_valid() -> Outcome<()> {
        let body = br#"{
            "status":      "valid",
            "expires":     "2026-05-01T12:00:00Z",
            "identifiers": [{"type":"dns","value":"example.com"}],
            "authorizations": ["https://acme-v02.api.letsencrypt.org/acme/authz/1"],
            "finalize":    "https://acme-v02.api.letsencrypt.org/acme/finalize/1",
            "certificate": "https://acme-v02.api.letsencrypt.org/acme/cert/abcdef"
        }"#;
        let order: Order = res!(parse_json_response(body));
        if order.status != "valid" {
            return Err(err!("order.status = {:?}", order.status; Test, Mismatch));
        }
        if !order.certificate.ends_with("/cert/abcdef") {
            return Err(err!(
                "order.certificate = {:?}", order.certificate;
                Test, Mismatch));
        }
        Ok(())
    }

    /// Parse an Authorization response and verify the challenge list comes
    /// through intact and `typed_challenges` succeeds.
    #[test]
    fn test_parse_authorization_happy_path() -> Outcome<()> {
        let body = br#"{
            "status":     "pending",
            "expires":    "2026-05-01T12:00:00Z",
            "identifier": {"type":"dns","value":"example.com"},
            "challenges": [
                {
                    "type":   "http-01",
                    "status": "pending",
                    "url":    "https://acme-v02.api.letsencrypt.org/acme/chall/1/a",
                    "token":  "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                },
                {
                    "type":   "dns-01",
                    "status": "pending",
                    "url":    "https://acme-v02.api.letsencrypt.org/acme/chall/1/b",
                    "token":  "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
                },
                {
                    "type":   "tls-alpn-01",
                    "status": "pending",
                    "url":    "https://acme-v02.api.letsencrypt.org/acme/chall/1/c",
                    "token":  "cccccccccccccccccccccccccccccccc"
                }
            ]
        }"#;
        let authz: Authorization = res!(parse_json_response(body));
        if authz.status != "pending" {
            return Err(err!("authz.status = {:?}", authz.status; Test, Mismatch));
        }
        let challenges = res!(authz.typed_challenges());
        if challenges.len() != 3 {
            return Err(err!(
                "typed_challenges returned {} entries, expected 3.", challenges.len();
                Test, Mismatch));
        }
        let tls = match res!(authz.tls_alpn_01_challenge()) {
            Some(c) => c,
            None => return Err(err!(
                "tls_alpn_01_challenge returned None.";
                Test, Missing)),
        };
        if tls.token != "cccccccccccccccccccccccccccccccc" {
            return Err(err!(
                "tls-alpn-01 token = {:?}", tls.token;
                Test, Mismatch));
        }
        Ok(())
    }

    /// **Regression test for the vendor patch.** Parse an authorisation whose
    /// challenges include one that omits both `token` and `url` entirely --
    /// the exact shape that broke upstream `rustls-acme 0.15.1` deserialisation
    /// with `missing field 'token'` against a live ACME staging server.
    /// With our `#[optional]` markings this must succeed, and the affected
    /// challenge must deserialise with empty defaults on both fields while
    /// the `tls-alpn-01` entry is still readable.
    #[test]
    fn test_parse_authorization_with_tokenless_challenge() -> Outcome<()> {
        let body = br#"{
            "status":     "pending",
            "identifier": {"type":"dns","value":"example.com"},
            "challenges": [
                {
                    "type":   "dns-persist-01",
                    "status": "pending"
                },
                {
                    "type":   "tls-alpn-01",
                    "status": "pending",
                    "url":    "https://acme-v02.api.letsencrypt.org/acme/chall/1/tls",
                    "token":  "reallyatlsalpntoken"
                }
            ]
        }"#;
        let authz: Authorization = res!(parse_json_response(body));
        let challenges = res!(authz.typed_challenges());
        if challenges.len() != 2 {
            return Err(err!(
                "typed_challenges returned {} entries, expected 2.", challenges.len();
                Test, Mismatch));
        }
        // The tokenless challenge must parse with empty defaults.
        let tokenless = &challenges[0];
        if tokenless.typ != "dns-persist-01" {
            return Err(err!("tokenless.typ = {:?}", tokenless.typ; Test, Mismatch));
        }
        if !tokenless.token.is_empty() {
            return Err(err!(
                "Tokenless challenge should default to empty token, got {:?}.",
                tokenless.token;
                Test, Mismatch));
        }
        if !tokenless.url.is_empty() {
            return Err(err!(
                "Tokenless challenge should default to empty url, got {:?}.",
                tokenless.url;
                Test, Mismatch));
        }
        // The tls-alpn-01 challenge must still be readable.
        let tls = match res!(authz.tls_alpn_01_challenge()) {
            Some(c) => c,
            None => return Err(err!(
                "tls_alpn_01_challenge returned None despite a tls-alpn-01 entry.";
                Test, Missing)),
        };
        if tls.token != "reallyatlsalpntoken" {
            return Err(err!(
                "tls-alpn-01 token = {:?}", tls.token;
                Test, Mismatch));
        }
        Ok(())
    }

    /// The `new_account_request` helper must emit the exact two-field shape
    /// RFC 8555 §7.3 mandates, with the contact entry wrapped in the
    /// `mailto:` URI scheme.
    #[test]
    fn test_new_account_request_shape() -> Outcome<()> {
        let req = new_account_request("hello@example.test", true);
        match req {
            Dat::Map(m) => {
                match m.get(&dat!("termsOfServiceAgreed")) {
                    Some(Dat::Bool(true)) => (),
                    other => return Err(err!(
                        "termsOfServiceAgreed = {:?}", other;
                        Test, Mismatch)),
                }
                match m.get(&dat!("contact")) {
                    Some(Dat::List(entries)) => {
                        if entries.len() != 1 {
                            return Err(err!(
                                "contact list has {} entries.", entries.len();
                                Test, Mismatch));
                        }
                        match &entries[0] {
                            Dat::Str(s) => {
                                if s != "mailto:hello@example.test" {
                                    return Err(err!(
                                        "contact[0] = {:?}", s;
                                        Test, Mismatch));
                                }
                            },
                            other => return Err(err!(
                                "contact[0] = {:?}", other;
                                Test, Mismatch)),
                        }
                    },
                    other => return Err(err!(
                        "contact = {:?}", other;
                        Test, Mismatch)),
                }
            },
            other => return Err(err!(
                "new_account_request did not produce a Dat::Map, got {:?}.",
                other;
                Test, Mismatch)),
        }
        Ok(())
    }

    /// The `new_order_request` helper must wrap each DNS name in a
    /// `{"type":"dns","value":...}` identifier map.
    #[test]
    fn test_new_order_request_shape() -> Outcome<()> {
        let req = new_order_request(&[
            "example.com".to_string(),
            "www.example.com".to_string(),
        ]);
        match req {
            Dat::Map(m) => match m.get(&dat!("identifiers")) {
                Some(Dat::List(list)) => {
                    if list.len() != 2 {
                        return Err(err!(
                            "identifiers list has {} entries.", list.len();
                            Test, Mismatch));
                    }
                    // Spot-check the second identifier is shaped correctly.
                    match &list[1] {
                        Dat::Map(im) => {
                            match im.get(&dat!("type")) {
                                Some(Dat::Str(s)) if s == "dns" => (),
                                other => return Err(err!(
                                    "identifiers[1].type = {:?}", other;
                                    Test, Mismatch)),
                            }
                            match im.get(&dat!("value")) {
                                Some(Dat::Str(s)) if s == "www.example.com" => (),
                                other => return Err(err!(
                                    "identifiers[1].value = {:?}", other;
                                    Test, Mismatch)),
                            }
                        },
                        other => return Err(err!(
                            "identifiers[1] = {:?}", other;
                            Test, Mismatch)),
                    }
                },
                other => return Err(err!(
                    "identifiers = {:?}", other;
                    Test, Mismatch)),
            },
            other => return Err(err!(
                "new_order_request did not produce a Dat::Map, got {:?}.",
                other;
                Test, Mismatch)),
        }
        Ok(())
    }

    /// Regression test for a jdat encoder boolean bug that broke an ACME
    /// new-account POST against a live staging server: `Dat::Bool(true).json()`
    /// used to emit the JSON string `"true"` instead of the JSON literal
    /// `true`, causing Let's Encrypt to reject the request with
    /// `Error unmarshaling JSON`. After fixing
    /// `fe2o3_jdat/src/string/enc.rs:633` this test asserts that a
    /// realistic ACME payload containing a boolean now serialises
    /// through `.json()` → parses as valid JSON → round-trips via
    /// `parse_json_response` → yields the correct boolean value.
    #[test]
    fn test_new_account_request_json_bool_round_trips() -> Outcome<()> {
        let req = new_account_request("hello@example.test", true);
        let bytes = res!(req.json()).into_bytes();
        let cfg: DecoderConfig<
            BTreeMap<UsrKindCode, UsrKind>,
            BTreeMap<String, UsrKindId>,
        > = DecoderConfig::json(None);
        let s = match std::str::from_utf8(&bytes) {
            Ok(s) => s.to_string(),
            Err(e) => return Err(err!(e,
                "new_account_request .json() produced invalid UTF-8.";
                Test, Decode)),
        };
        let reparsed = res!(Dat::decode_string_with_config(s, &cfg));
        match reparsed {
            Dat::Map(m) => match m.get(&dat!("termsOfServiceAgreed")) {
                Some(Dat::Bool(true)) => Ok(()),
                other => Err(err!(
                    "termsOfServiceAgreed round-tripped as {:?}; expected \
                    Dat::Bool(true). Bool-as-string bug regressed.", other;
                    Test, Mismatch)),
            },
            other => Err(err!(
                "new_account_request .json() did not parse back as a \
                JSON object, got {:?}.", other;
                Test, Mismatch)),
        }
    }

    // ---- status enums (RFC 8555 §7.1.6) ----------------------------------

    /// Every status RFC 8555 §7.1.6 defines must round-trip through its enum,
    /// and anything else must be refused rather than silently mapped onto a
    /// status we do know.
    #[test]
    fn test_status_enums_round_trip_and_reject_unknown() -> Outcome<()> {
        for s in ["pending", "ready", "processing", "valid", "invalid"] {
            let st = res!(OrderStatus::from_wire(s));
            if st.as_wire() != s {
                return Err(err!(
                    "OrderStatus {:?} round-tripped as {:?}.", s, st.as_wire();
                    Test, Mismatch));
            }
        }
        for s in ["pending", "valid", "invalid", "deactivated", "expired", "revoked"] {
            let st = res!(AuthorizationStatus::from_wire(s));
            if st.as_wire() != s {
                return Err(err!(
                    "AuthorizationStatus {:?} round-tripped as {:?}.",
                    s, st.as_wire();
                    Test, Mismatch));
            }
        }
        for s in ["pending", "processing", "valid", "invalid"] {
            let st = res!(ChallengeStatus::from_wire(s));
            if st.as_wire() != s {
                return Err(err!(
                    "ChallengeStatus {:?} round-tripped as {:?}.",
                    s, st.as_wire();
                    Test, Mismatch));
            }
        }
        // An authorisation has no `processing` state (§7.1.6), and an order
        // has no `expired` one. Accepting them would mean the client silently
        // mishandled a status the CA never sends.
        if AuthorizationStatus::from_wire("processing").is_ok() {
            return Err(err!(
                "AuthorizationStatus accepted 'processing', which RFC 8555 \
                §7.1.6 does not define for authorisations.";
                Test, Mismatch));
        }
        if OrderStatus::from_wire("expired").is_ok() {
            return Err(err!(
                "OrderStatus accepted 'expired', which RFC 8555 §7.1.6 does \
                not define for orders.";
                Test, Mismatch));
        }
        Ok(())
    }

    /// An authorisation the CA has already validated must parse as `Valid`;
    /// this is the state a renewal order carries and the one the client used
    /// to mishandle.
    #[test]
    fn test_parse_valid_authorization_from_cached_validation() -> Outcome<()> {
        let body = br#"{
            "status":     "valid",
            "expires":    "2026-08-01T12:00:00Z",
            "identifier": {"type":"dns","value":"example.com"},
            "challenges": [
                {
                    "type":      "tls-alpn-01",
                    "status":    "valid",
                    "url":       "https://acme-v02.api.letsencrypt.org/acme/chall/1/c",
                    "token":     "cccccccccccccccccccccccccccccccc",
                    "validated": "2026-07-01T12:00:00Z"
                }
            ]
        }"#;
        let authz: Authorization = res!(parse_json_response(body));
        if res!(authz.typed_status()) != AuthorizationStatus::Valid {
            return Err(err!(
                "A cached-validation authorisation parsed as {:?}.",
                authz.status;
                Test, Mismatch));
        }
        let chall = match res!(authz.tls_alpn_01_challenge()) {
            Some(c) => c,
            None => return Err(err!(
                "tls_alpn_01_challenge returned None."; Test, Missing)),
        };
        if res!(chall.typed_status()) != ChallengeStatus::Valid {
            return Err(err!(
                "The challenge on a valid authorisation parsed as {:?}.",
                chall.status;
                Test, Mismatch));
        }
        Ok(())
    }

    // ---- key authorisation and dns-01, pinned (RFC 8555 §8.1, §8.4) ------

    /// **External oracle, RFC 8555 §8.1 and §8.4.** Chain the fixed P-256
    /// account key all the way through to the two values a CA actually
    /// recomputes: the key authorisation and the dns-01 TXT record.
    ///
    /// Both expected values were derived outside this crate from the same key,
    /// with `openssl` and `python3` (see [`crate::acme::jose::TEST_P256_PKCS8`]
    /// for the key and its thumbprint derivation):
    ///
    /// ```text
    /// thumbprint = rIV82OX7WtoQ9t9CvXXciOOey0zuRuaonj8p-bQghoA
    /// key_auth   = <token> "." <thumbprint>                       (§8.1)
    /// txt        = base64url(SHA-256(key_auth))                   (§8.4)
    /// ```
    ///
    /// The §8.4 value is the prehash shape that silently broke the ed25519
    /// DKIM signer: the digest is taken over the key authorisation string and
    /// the *digest* is what gets encoded -- not the string, and not the raw
    /// thumbprint.
    #[test]
    fn test_key_authorization_and_dns01_against_external_oracle() -> Outcome<()> {
        let signer = res!(jose::JwsSigner::from_pkcs8(&jose::TEST_P256_PKCS8));
        let thumbprint = res!(signer.jwk_thumbprint_sha256());

        let chall = Challenge {
            typ:        "dns-01".to_string(),
            status:     "pending".to_string(),
            url:        "https://example.test/chall/1".to_string(),
            token:      "evaGxfADs6pSRb2LAv9IZf17Dt3juxGJ-PCt92wr-oA".to_string(),
            validated:  String::new(),
            error:      Dat::Empty,
        };

        let expected_key_auth = "evaGxfADs6pSRb2LAv9IZf17Dt3juxGJ-PCt92wr-oA.\
            rIV82OX7WtoQ9t9CvXXciOOey0zuRuaonj8p-bQghoA";
        let key_auth = chall.key_authorization(&thumbprint);
        if key_auth != expected_key_auth {
            return Err(err!(
                "RFC 8555 §8.1 key authorisation was {:?}, externally-derived \
                value is {:?}.", key_auth, expected_key_auth;
                Test, Mismatch));
        }

        let expected_txt = "XS-wSC2L4p8YkHvL-3QvDUnrIrgSwtrSxnq3xi_9R7U";
        let txt = chall.dns_01_txt_value(&thumbprint);
        if txt != expected_txt {
            return Err(err!(
                "RFC 8555 §8.4 dns-01 TXT value was {:?}, externally-derived \
                value is {:?}.", txt, expected_txt;
                Test, Mismatch));
        }
        Ok(())
    }

    /// `Challenge::key_authorization` must produce `<token>.<b64-thumbprint>`
    /// as specified by RFC 8555 §8.1.
    #[test]
    fn test_challenge_key_authorization() -> Outcome<()> {
        let chall = Challenge {
            typ:        "tls-alpn-01".to_string(),
            status:     "pending".to_string(),
            url:        "https://example.test/chall/1".to_string(),
            token:      "TokenVal".to_string(),
            validated:  String::new(),
            error:      Dat::Empty,
        };
        // Thumbprint here is arbitrary for the test; what matters is the
        // joining format.
        let thumbprint: [u8; 32] = [0u8; 32];
        let ka = chall.key_authorization(&thumbprint);
        // The all-zero thumbprint encodes to 43 `A` characters unpadded.
        let expected_tail = jose::base64url_encode(&thumbprint);
        let expected = fmt!("TokenVal.{}", expected_tail);
        if ka != expected {
            return Err(err!(
                "key_authorization = {:?}, expected {:?}.", ka, expected;
                Test, Mismatch));
        }
        Ok(())
    }
}
