//! A certificate authority that lives only in memory and can vouch only for the names it was
//! given, for a TLS-terminating proxy that must show a client a certificate for a host it does
//! not own.
//!
//! RFC 5280 has no "strictly below" form of a dNSName constraint, so a `Suffix(".x.y")` rule
//! permits `x.y` itself in the CA's name constraints, and an `Exact("x.y")` rule permits every
//! host below `x.y`. `Mint::leaf` is the narrower gate: the constraints bound what a stolen key
//! could do, and the rules bound what the `Mint` will do.

use crate::tls::{
    ensure_crypto_provider,
    pem_certificate,
};

use oxedyne_fe2o3_core::prelude::*;

use std::{
    collections::HashMap,
    fmt,
    net::IpAddr,
    sync::{
        Arc,
        Mutex,
    },
    time::{
        Duration,
        SystemTime,
        UNIX_EPOCH,
    },
};

use rcgen::{
    BasicConstraints,
    Certificate,
    CertificateParams,
    CidrSubnet,
    DistinguishedName,
    DnType,
    ExtendedKeyUsagePurpose,
    GeneralSubtree,
    IsCa,
    KeyUsagePurpose,
    NameConstraints,
    SanType,
    PKCS_ECDSA_P256_SHA256,
};
use tokio_rustls::rustls::{
    crypto::ring::sign::any_ecdsa_type,
    pki_types::{
        CertificateDer,
        PrivateKeyDer,
        PrivatePkcs8KeyDer,
    },
    server::{
        ClientHello,
        ResolvesServerCert,
    },
    sign::CertifiedKey,
};


pub const MINT_LEAF_SECS:       i64     = 24 * 3_600;   // A leaf's lifetime.
pub const MINT_RENEW_SECS:      i64     = 3_600;        // A leaf with less left is re-issued.
pub const MINT_BACKDATE_SECS:   i64     = 3_600;        // notBefore lies this far back, for skew.
pub const MINT_LEAF_CACHE_MAX:  usize   = 1_024;        // Leaves held at once.

/// A name, or a family of names, a [`Mint`] may vouch for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NameRule {
    Exact(String),  // One host, `pypi.org`.
    Suffix(String), // Every host strictly below a domain, `.pythonhosted.org`.
}

impl NameRule {

    /// The rule with its name checked and in lower case.
    fn normal(&self) -> Outcome<Self> {
        match self {
            Self::Exact(n)  => Ok(Self::Exact(res!(dns_name(n)))),
            Self::Suffix(s) => match s.strip_prefix('.') {
                Some(d) => Ok(Self::Suffix(fmt!(".{}", res!(dns_name(d))))),
                None    => Err(err!(
                    "The suffix rule {:?} must begin with a '.'.", s;
                    Invalid, Input)),
            },
        }
    }

    /// Does the rule admit this host name, already checked and in lower case?
    pub fn admits(&self, name: &str) -> bool {
        match self {
            Self::Exact(n)  => name == n,
            Self::Suffix(s) => name.len() > s.len() && name.ends_with(s.as_str()),
        }
    }

    fn subtree(&self) -> GeneralSubtree {
        match self {
            Self::Exact(n)  => GeneralSubtree::DnsName(n.clone()),
            Self::Suffix(s) => GeneralSubtree::DnsName(s.trim_start_matches('.').to_string()),
        }
    }
}

/// The host name in lower case without a trailing dot, refused if it is not a DNS host name:
/// an IP address, a wildcard, an empty or over-long label, or a label with a character outside
/// letters, digits and `-`.
pub fn dns_name(s: &str) -> Outcome<String> {
    let n = s.strip_suffix('.').unwrap_or(s).to_ascii_lowercase();
    if n.parse::<IpAddr>().is_ok() {
        return Err(err!("{:?} is an IP address, not a host name.", s; Invalid, Input));
    }
    if n.is_empty() || n.len() > 253 {
        return Err(err!("{:?} is not a host name: it is {} bytes long.", s, n.len();
            Invalid, Input));
    }
    let mut last = "";
    for label in n.split('.') {
        let ok = !label.is_empty()
            && label.len() <= 63
            && !label.starts_with('-')
            && !label.ends_with('-')
            && label.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
        if !ok {
            return Err(err!("{:?} is not a host name: its label {:?} is not a DNS label.",
                s, label; Invalid, Input));
        }
        last = label;
    }
    // A top-level label of digits alone is how an address in another notation looks.
    if last.bytes().all(|b| b.is_ascii_digit()) {
        return Err(err!("{:?} is not a host name: its last label is numeric.", s;
            Invalid, Input));
    }
    Ok(n)
}

/// An in-memory ECDSA P-256 certificate authority, constrained to the names in its rules, that
/// issues a 24-hour leaf per host name on demand and serves it by SNI as a rustls
/// [`ResolvesServerCert`].
///
/// The CA's key is held here and nowhere else: no method returns or serialises it, and `Debug`
/// omits it. The CA certificate carries a critical name-constraints extension permitting the
/// rules' dNSName subtrees and excluding every IPv4 and IPv6 address, so even a leaked key cannot
/// vouch for an address or for a name outside the rules to a client that honours the extension.
pub struct Mint {
    ca:         Certificate,                                // The CA, with its key.
    ca_der:     CertificateDer<'static>,
    ca_pem:     String,
    rules:      Vec<NameRule>,
    not_after:  i64,                                        // The CA's, in Unix seconds.
    leaves:     Mutex<HashMap<String, (Arc<CertifiedKey>, i64)>>, // Name to leaf and its expiry.
}

impl fmt::Debug for Mint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Mint")
            .field("rules", &self.rules)
            .field("not_after", &self.not_after)
            .finish_non_exhaustive()
    }
}

impl Mint {

    /// A new CA, with a fresh key, for the given rules.
    ///
    /// # Arguments
    /// * `ca_days` - the CA's lifetime in days, at least one.
    pub fn new(names: &[NameRule], ca_days: u32) -> Outcome<Self> {
        ensure_crypto_provider();
        let now = res!(now_secs());
        let (params, rules) = res!(Self::ca_params_at(names, ca_days, now));
        let ca = res!(Certificate::from_params(params), Init);
        let der = res!(ca.serialize_der(), Init);
        let ca_pem = pem_certificate(&der);
        Ok(Self {
            ca,
            ca_der:     CertificateDer::from(der),
            ca_pem,
            rules,
            not_after:  now + i64::from(ca_days) * 86_400,
            leaves:     Mutex::new(HashMap::new()),
        })
    }

    /// The parameters `new` gives its CA, without a key. Public so that a test can build a CA of
    /// exactly this shape and sign with it what a `Mint` itself never would.
    ///
    /// # Arguments
    /// * `ca_days` - the CA's lifetime in days, at least one.
    pub fn ca_params(names: &[NameRule], ca_days: u32) -> Outcome<CertificateParams> {
        let now = res!(now_secs());
        let (params, _) = res!(Self::ca_params_at(names, ca_days, now));
        Ok(params)
    }

    fn ca_params_at(
        names:      &[NameRule],
        ca_days:    u32,
        now:        i64,
    )
        -> Outcome<(CertificateParams, Vec<NameRule>)>
    {
        // No permitted subtree would be no constraint at all.
        if names.is_empty() {
            return Err(err!("A Mint needs at least one name rule."; Invalid, Input, Missing));
        }
        if ca_days == 0 {
            return Err(err!("A Mint's CA must last at least one day."; Invalid, Input));
        }
        let mut rules = Vec::with_capacity(names.len());
        for r in names {
            rules.push(res!(r.normal()));
        }
        let mut p = CertificateParams::default();
        p.alg = &PKCS_ECDSA_P256_SHA256;
        validity(&mut p, now - MINT_BACKDATE_SECS, now + i64::from(ca_days) * 86_400);
        let mut dn = DistinguishedName::new();
        dn.push(DnType::CommonName, "fe2o3 Mint");
        p.distinguished_name = dn;
        p.is_ca = IsCa::Ca(BasicConstraints::Constrained(0));
        p.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
        p.name_constraints = Some(NameConstraints {
            permitted_subtrees: rules.iter().map(|r| r.subtree()).collect(),
            excluded_subtrees:  vec![
                GeneralSubtree::IpAddress(CidrSubnet::V4([0; 4], [0; 4])),     // 0.0.0.0/0
                GeneralSubtree::IpAddress(CidrSubnet::V6([0; 16], [0; 16])),   // ::/0
            ],
        });
        Ok((p, rules))
    }

    /// The CA certificate, PEM-encoded, for a client's trust store.
    pub fn ca_pem(&self) -> String {
        self.ca_pem.clone()
    }

    pub fn ca_der(&self) -> CertificateDer<'static> {
        self.ca_der.clone()
    }

    pub fn rules(&self) -> &[NameRule] {
        &self.rules
    }

    /// Will the CA expire within `secs`? A clock before 1970 counts as yes.
    pub fn expires_within(&self, secs: u64) -> bool {
        match now_secs() {
            Ok(now) => self.not_after.saturating_sub(now) < i64::try_from(secs).unwrap_or(i64::MAX),
            Err(_)  => true,
        }
    }

    /// The leaf for `name`, with the chain `[leaf, CA]`, from the cache unless it has under an
    /// hour left. A name no rule admits is refused.
    pub fn leaf(&self, name: &str) -> Outcome<Arc<CertifiedKey>> {
        let name = res!(dns_name(name));
        if !self.rules.iter().any(|r| r.admits(&name)) {
            return Err(err!("The Mint refuses {:?}: no name rule admits it.", name;
                Invalid, Input));
        }
        let now = res!(now_secs());
        let mut leaves = lock_mutex!(self.leaves);
        if let Some((ck, until)) = leaves.get(&name) {
            if until - now >= MINT_RENEW_SECS {
                return Ok(ck.clone());
            }
        }
        let (ck, until) = res!(self.issue(&name, now));
        // A suffix rule admits names without end, chosen by whoever sends the SNI.
        if leaves.len() >= MINT_LEAF_CACHE_MAX {
            leaves.retain(|_, (_, u)| *u - now >= MINT_RENEW_SECS);
            if leaves.len() >= MINT_LEAF_CACHE_MAX {
                leaves.clear();
            }
        }
        leaves.insert(name, (ck.clone(), until));
        Ok(ck)
    }

    fn issue(&self, name: &str, now: i64) -> Outcome<(Arc<CertifiedKey>, i64)> {
        let until = std::cmp::min(now + MINT_LEAF_SECS, self.not_after);
        let mut p = CertificateParams::default();
        p.alg = &PKCS_ECDSA_P256_SHA256;
        validity(&mut p, now - MINT_BACKDATE_SECS, until);
        p.subject_alt_names = vec![SanType::DnsName(name.to_string())];
        let mut dn = DistinguishedName::new();
        dn.push(DnType::CommonName, name);
        p.distinguished_name = dn;
        p.is_ca = IsCa::ExplicitNoCa;
        p.key_usages = vec![KeyUsagePurpose::DigitalSignature];
        p.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
        p.use_authority_key_identifier_extension = true;
        let cert = res!(Certificate::from_params(p), Init);
        let der = res!(cert.serialize_der_with_signer(&self.ca), Init);
        let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(cert.serialize_private_key_der()));
        let signer = res!(any_ecdsa_type(&key), Init);
        let chain = vec![CertificateDer::from(der), self.ca_der.clone()];
        Ok((Arc::new(CertifiedKey::new(chain, signer)), until))
    }
}

/// The SNI picks the leaf. No SNI, or a name the `Mint` refuses, answers `None`, and rustls then
/// aborts the handshake.
impl ResolvesServerCert for Mint {
    fn resolve(&self, hello: ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
        match hello.server_name() {
            Some(n) => self.leaf(n).ok(),
            None    => None,
        }
    }
}

fn now_secs() -> Outcome<i64> {
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(d)  => Ok(i64::try_from(d.as_secs()).unwrap_or(i64::MAX)),
        Err(e) => Err(err!(e, "The system clock reads before 1970."; Init, Invalid)),
    }
}

/// Sets notBefore and notAfter from Unix times, through rcgen's own `OffsetDateTime`, so the
/// crate need not depend on `time` to name it.
fn validity(p: &mut CertificateParams, from: i64, to: i64) {
    let epoch = rcgen::date_time_ymd(1970, 1, 1);
    p.not_before = epoch + Duration::from_secs(u64::try_from(from).unwrap_or(0));
    p.not_after = epoch + Duration::from_secs(u64::try_from(to).unwrap_or(0));
}
