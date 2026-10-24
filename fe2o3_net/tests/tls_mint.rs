#![cfg(feature = "async")]
//! `Mint`, the in-memory name-constrained CA: what it vouches for, what it refuses, and whether
//! the constraints in its certificate hold against a client when the `Mint` itself is bypassed,
//! for rustls and, where `gnutls-cli` is present, for GnuTLS.

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_net::tls::{
    ensure_crypto_provider,
    parse_pem_certificates,
    pem_certificate,
    Mint,
    NameRule,
};

use std::{
    net::{
        IpAddr,
        Ipv4Addr,
    },
    process::{
        Command,
        Stdio,
    },
    sync::Arc,
    time::Duration,
};

use rcgen::{
    Certificate,
    CertificateParams,
    DistinguishedName,
    DnType,
    ExtendedKeyUsagePurpose,
    IsCa,
    KeyUsagePurpose,
    SanType,
};
use tokio::{
    io::{
        AsyncReadExt,
        AsyncWriteExt,
    },
    net::{
        TcpListener,
        TcpStream,
    },
};
use tokio_rustls::{
    rustls::{
        client::ClientConfig,
        pki_types::{
            CertificateDer,
            PrivateKeyDer,
            PrivatePkcs8KeyDer,
            ServerName,
        },
        server::{
            ResolvesServerCert,
            ServerConfig,
        },
        RootCertStore,
    },
    TlsAcceptor,
    TlsConnector,
};


const WAIT: Duration = Duration::from_secs(10);

fn rules() -> Vec<NameRule> {
    vec![
        NameRule::Exact("registry.test".to_string()),
        NameRule::Suffix(".files.test".to_string()),
    ]
}

/// A server taking one connection and completing (or failing) one handshake with `cfg`.
async fn serve(cfg: ServerConfig) -> Outcome<u16> {
    let lst = res!(TcpListener::bind("127.0.0.1:0").await);
    let port = res!(lst.local_addr()).port();
    let acc = TlsAcceptor::from(Arc::new(cfg));
    tokio::spawn(async move {
        if let Ok((tcp, _)) = lst.accept().await {
            if let Ok(mut tls) = acc.accept(tcp).await {
                let mut b = [0u8; 16];
                let _ = tls.read(&mut b).await;
                let _ = tls.shutdown().await;
            }
        }
    });
    Ok(port)
}

fn mint_server(mint: Arc<Mint>) -> ServerConfig {
    ensure_crypto_provider();
    let res: Arc<dyn ResolvesServerCert> = mint;
    ServerConfig::builder().with_no_client_auth().with_cert_resolver(res)
}

fn fixed_server(chain: Vec<CertificateDer<'static>>, key: PrivateKeyDer<'static>)
    -> Outcome<ServerConfig>
{
    ensure_crypto_provider();
    Ok(res!(ServerConfig::builder().with_no_client_auth().with_single_cert(chain, key), Init))
}

/// A rustls client that trusts only the CA in `ca_pem`, completing a handshake as `name`.
async fn handshake(port: u16, ca_pem: &str, name: ServerName<'static>) -> Outcome<()> {
    ensure_crypto_provider();
    let mut roots = RootCertStore::empty();
    for der in parse_pem_certificates(ca_pem.as_bytes()) {
        res!(roots.add(CertificateDer::from(der)), Init);
    }
    let cfg = ClientConfig::builder().with_root_certificates(roots).with_no_client_auth();
    let tcp = res!(TcpStream::connect(("127.0.0.1", port)).await);
    let con = TlsConnector::from(Arc::new(cfg));
    match tokio::time::timeout(WAIT, con.connect(name, tcp)).await {
        Ok(Ok(mut tls)) => {
            let _ = tls.write_all(b"ping").await;
            Ok(())
        },
        Ok(Err(e))      => Err(err!(e, "Handshake failed."; Network)),
        Err(_)          => Err(err!("Handshake timed out."; Network, Timeout)),
    }
}

fn dns(name: &str) -> Outcome<ServerName<'static>> {
    Ok(res!(ServerName::try_from(name.to_string()), Invalid, Input))
}

/// A leaf for `san`, signed by `ca`, that a `Mint` would never issue, and its key.
fn signed(ca: &Certificate, san: SanType)
    -> Outcome<(Vec<CertificateDer<'static>>, PrivateKeyDer<'static>)>
{
    let mut p = CertificateParams::default();
    p.subject_alt_names = vec![san];
    let mut dn = DistinguishedName::new();
    dn.push(DnType::CommonName, "leaf");
    p.distinguished_name = dn;
    p.is_ca = IsCa::ExplicitNoCa;
    p.key_usages = vec![KeyUsagePurpose::DigitalSignature];
    p.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    p.use_authority_key_identifier_extension = true;
    let leaf = res!(Certificate::from_params(p), Init);
    let der = res!(leaf.serialize_der_with_signer(ca), Init);
    let ca_der = res!(ca.serialize_der(), Init);
    let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(leaf.serialize_private_key_der()));
    Ok((vec![CertificateDer::from(der), CertificateDer::from(ca_der)], key))
}

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ 1-2. WHAT THE MINT VOUCHES FOR                                            │
// └───────────────────────────────────────────────────────────────────────────┘

#[tokio::test]
async fn test_a_leaf_for_a_permitted_name_verifies_against_the_ca_alone() -> Outcome<()> {
    let mint = Arc::new(res!(Mint::new(&rules(), 2)));
    let pem = mint.ca_pem();
    for name in ["registry.test", "a.files.test", "deep.b.files.test", "REGISTRY.test"] {
        let port = res!(serve(mint_server(mint.clone())).await);
        res!(handshake(port, &pem, res!(dns(name))).await);
    }
    // The chain is [leaf, CA], and the leaf is cached until it nears expiry.
    let a = res!(mint.leaf("registry.test"));
    let b = res!(mint.leaf("Registry.Test."));
    assert!(Arc::ptr_eq(&a, &b), "a second request for one name re-issued its leaf");
    assert_eq!(a.cert.len(), 2);
    assert_eq!(a.cert[1], mint.ca_der());
    assert!(!mint.expires_within(3_600), "a two-day CA reads as expiring within an hour");
    assert!(mint.expires_within(3 * 86_400), "a two-day CA reads as lasting three days");
    Ok(())
}

#[tokio::test]
async fn test_a_name_outside_the_rules_is_refused() -> Outcome<()> {
    let mint = Arc::new(res!(Mint::new(&rules(), 2)));
    for name in ["evil.test", "files.test", "xregistry.test", "a.registry.test", "127.0.0.1",
        "*.files.test", "a..files.test", "a.files.test.evil", ""]
    {
        assert!(mint.leaf(name).is_err(), "the Mint issued a leaf for {:?}", name);
    }
    let port = res!(serve(mint_server(mint.clone())).await);
    assert!(handshake(port, &mint.ca_pem(), res!(dns("evil.test"))).await.is_err(),
        "a client reached a handshake for a name the Mint refuses");
    // Without SNI there is no name to vouch for.
    let port = res!(serve(mint_server(mint.clone())).await);
    let ip = ServerName::IpAddress(IpAddr::V4(Ipv4Addr::LOCALHOST).into());
    assert!(handshake(port, &mint.ca_pem(), ip).await.is_err(),
        "a client without SNI reached a handshake");
    // Rules that would constrain nothing are refused.
    assert!(Mint::new(&[], 2).is_err());
    assert!(Mint::new(&[NameRule::Suffix("files.test".to_string())], 2).is_err());
    assert!(Mint::new(&[NameRule::Exact("10.0.0.1".to_string())], 2).is_err());
    assert!(Mint::new(&rules(), 0).is_err());
    Ok(())
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ 3. THE CA CERTIFICATE                                                     │
// └───────────────────────────────────────────────────────────────────────────┘

#[test]
fn test_the_ca_holds_no_key_and_a_critical_constraint_excluding_every_address() -> Outcome<()> {
    let mint = res!(Mint::new(&rules(), 2));
    let pem = mint.ca_pem();
    assert!(!pem.contains("PRIVATE KEY"), "the CA PEM carries a key");
    assert!(!fmt!("{:?}", mint).contains("PRIVATE"), "Debug shows a key");
    let ders = parse_pem_certificates(pem.as_bytes());
    assert_eq!(ders.len(), 1, "the CA PEM is not exactly one certificate");
    let der = &ders[0];
    assert_eq!(der.as_slice(), mint.ca_der().as_ref());
    // Extension: OID 2.5.29.30, then critical BOOLEAN TRUE.
    assert!(contains(der, &[0x06, 0x03, 0x55, 0x1d, 0x1e, 0x01, 0x01, 0xff]),
        "no critical name-constraints extension");
    // Permitted dNSName subtrees, [2] IA5String: the exact name, and the suffix's domain.
    let mut exact = vec![0x82, 13];
    exact.extend_from_slice(b"registry.test");
    assert!(contains(der, &exact), "the exact rule is not a permitted subtree");
    let mut suffix = vec![0x82, 10];
    suffix.extend_from_slice(b"files.test");
    assert!(contains(der, &suffix), "the suffix rule is not a permitted subtree");
    // Excluded subtrees [1], holding iPAddress [7] 0.0.0.0/0 (8 bytes) and ::/0 (32 bytes).
    let mut v4 = vec![0x30, 0x0a, 0x87, 0x08];
    v4.extend_from_slice(&[0u8; 8]);
    let mut v6 = vec![0x30, 0x22, 0x87, 0x20];
    v6.extend_from_slice(&[0u8; 32]);
    let mut excl = vec![0xa1, (v4.len() + v6.len()) as u8];
    excl.extend_from_slice(&v4);
    excl.extend_from_slice(&v6);
    assert!(contains(der, &excl), "the excluded subtrees are not exactly 0.0.0.0/0 and ::/0");
    // basicConstraints: critical, cA TRUE, pathLen 0.
    assert!(contains(der, &[0x06, 0x03, 0x55, 0x1d, 0x13, 0x01, 0x01, 0xff,
        0x04, 0x08, 0x30, 0x06, 0x01, 0x01, 0xff, 0x02, 0x01, 0x00]),
        "basicConstraints is not cA with pathLen 0");
    // PEM round trip.
    assert_eq!(pem_certificate(der), pem);
    Ok(())
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ 5. RUSTLS HOLDS THE CONSTRAINTS WITHOUT THE MINT                          │
// └───────────────────────────────────────────────────────────────────────────┘

#[tokio::test]
async fn test_rustls_refuses_an_address_or_foreign_name_under_the_constrained_ca()
    -> Outcome<()>
{
    let ca = res!(Certificate::from_params(res!(Mint::ca_params(&rules(), 2))), Init);
    let ca_pem = pem_certificate(&res!(ca.serialize_der(), Init));
    let lo = IpAddr::V4(Ipv4Addr::LOCALHOST);

    // A permitted name under this CA passes, so a refusal below is the constraints'.
    let (chain, key) = res!(signed(&ca, SanType::DnsName("x.files.test".to_string())));
    let port = res!(serve(res!(fixed_server(chain, key))).await);
    res!(handshake(port, &ca_pem, res!(dns("x.files.test"))).await);

    let (chain, key) = res!(signed(&ca, SanType::IpAddress(lo)));
    let port = res!(serve(res!(fixed_server(chain, key))).await);
    assert!(handshake(port, &ca_pem, ServerName::IpAddress(lo.into())).await.is_err(),
        "rustls accepted an IP-SAN leaf under the constrained CA");

    let (chain, key) = res!(signed(&ca, SanType::DnsName("evil.test".to_string())));
    let port = res!(serve(res!(fixed_server(chain, key))).await);
    assert!(handshake(port, &ca_pem, res!(dns("evil.test"))).await.is_err(),
        "rustls accepted a leaf for a name outside the constraints");

    // The break: the same IP leaf under a CA without the exclusions is accepted, so it is the
    // exclusions, not the permitted dNSNames, that refuse an address.
    let mut open = res!(Mint::ca_params(&rules(), 2));
    if let Some(nc) = open.name_constraints.as_mut() {
        nc.excluded_subtrees.clear();
    }
    let open = res!(Certificate::from_params(open), Init);
    let open_pem = pem_certificate(&res!(open.serialize_der(), Init));
    let (chain, key) = res!(signed(&open, SanType::IpAddress(lo)));
    let port = res!(serve(res!(fixed_server(chain, key))).await);
    res!(handshake(port, &open_pem, ServerName::IpAddress(lo.into())).await);
    Ok(())
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ 4. GNUTLS                                                                 │
// └───────────────────────────────────────────────────────────────────────────┘

/// `gnutls-cli`, or the program `GNUTLS_CLI` names, if it runs.
fn gnutls_cli() -> Option<String> {
    let bin = std::env::var("GNUTLS_CLI").unwrap_or_else(|_| "gnutls-cli".to_string());
    match Command::new(&bin).arg("--version").stdout(Stdio::null()).stderr(Stdio::null()).status() {
        Ok(s) if s.success()    => Some(bin),
        _                       => None,
    }
}

/// Does `gnutls-cli`, trusting only `ca_pem`, accept the server on `port` as `name`?
async fn gnutls_accepts(bin: &str, port: u16, ca_pem: &str, name: &str, tag: &str)
    -> Outcome<bool>
{
    let path = std::env::temp_dir().join(fmt!("tls_mint_{}_{}.pem", std::process::id(), tag));
    res!(std::fs::write(&path, ca_pem));
    let mut cmd = Command::new(bin);
    cmd.arg("--x509cafile").arg(&path)
        .arg("--port").arg(port.to_string())
        .arg("--verify-hostname").arg(name);
    if name.parse::<IpAddr>().is_err() {
        cmd.arg("--sni-hostname").arg(name);
    }
    cmd.arg("127.0.0.1").stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    let out = res!(tokio::task::spawn_blocking(move || cmd.output()).await);
    let _ = std::fs::remove_file(&path);
    let out = res!(out);
    let text = fmt!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    let ok = out.status.success() && !text.contains("NOT trusted");
    eprintln!("gnutls-cli {} as {:?}: exit {:?}, {}", tag, name, out.status.code(),
        if ok { "accepted" } else { "refused" });
    for l in text.lines().filter(|l| l.contains("certificate") || l.contains("Status")) {
        eprintln!("    {}", l.trim());
    }
    Ok(ok)
}

#[tokio::test]
async fn test_gnutls_refuses_a_leaf_outside_its_cas_constraints() -> Outcome<()> {
    let bin = match gnutls_cli() {
        Some(b) => b,
        None    => {
            eprintln!("SKIPPED: gnutls-cli is not installed (set GNUTLS_CLI to its path), so \
                the GnuTLS arm of design §3 is unsettled by this run.");
            return Ok(());
        },
    };
    let ca = res!(Certificate::from_params(res!(Mint::ca_params(
        &[NameRule::Exact("good.test".to_string())], 2))), Init);
    let ca_pem = pem_certificate(&res!(ca.serialize_der(), Init));

    let (chain, key) = res!(signed(&ca, SanType::DnsName("good.test".to_string())));
    let port = res!(serve(res!(fixed_server(chain, key))).await);
    assert!(res!(gnutls_accepts(&bin, port, &ca_pem, "good.test", "control").await),
        "gnutls-cli refused a permitted leaf, so its refusals below prove nothing");

    let (chain, key) = res!(signed(&ca, SanType::DnsName("evil.test".to_string())));
    let port = res!(serve(res!(fixed_server(chain, key))).await);
    assert!(!res!(gnutls_accepts(&bin, port, &ca_pem, "evil.test", "unpermitted").await),
        "GnuTLS accepted a leaf for a name outside its CA's constraints");

    let (chain, key) = res!(signed(&ca, SanType::IpAddress(IpAddr::V4(Ipv4Addr::LOCALHOST))));
    let port = res!(serve(res!(fixed_server(chain, key))).await);
    assert!(!res!(gnutls_accepts(&bin, port, &ca_pem, "127.0.0.1", "address").await),
        "GnuTLS accepted an IP-SAN leaf under a CA excluding every address");
    Ok(())
}
