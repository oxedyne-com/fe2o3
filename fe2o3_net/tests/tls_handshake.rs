#![cfg(feature = "async")]
//! Whose fault a failed server-side TLS handshake is.
//!
//! A public listener is probed all day by scanners, old clients and clients that hang up, and
//! every one of those ends a handshake in an error. A caller that logs each at `ERROR` buries the
//! faults that are the server's own, so `BoundedTlsAcceptor` says which kind it was:
//! `Handshake::PeerFailed` for a peer that offered nothing acceptable, spoke nonsense or hung up,
//! `Handshake::Failed` for anything that may be ours -- a certificate that cannot be served, or a
//! client rejecting the one it was given. When in doubt the answer is `Failed`.

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_net::tls::{
    ensure_crypto_provider,
    BoundedTlsAcceptor,
    Handshake,
};

use std::{
    future::Future,
    sync::Arc,
    time::Duration,
};

use tokio::{
    io::AsyncWriteExt,
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
            ClientHello,
            ResolvesServerCert,
            ServerConfig,
        },
        sign::CertifiedKey,
        version,
        RootCertStore,
    },
    TlsAcceptor,
    TlsConnector,
};


const HOST: &str = "scan.test.local";
const WAIT: Duration = Duration::from_secs(5);

/// A certificate for [`HOST`] and the key that goes with it.
fn identity() -> Outcome<(CertificateDer<'static>, PrivateKeyDer<'static>)> {
    let cert = res!(rcgen::generate_simple_self_signed(vec![HOST.to_string()]), Init);
    let der = res!(cert.serialize_der(), Init);
    let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(cert.serialize_private_key_der()));
    Ok((CertificateDer::from(der), key))
}

/// A server that holds a certificate and key and is willing to speak TLS 1.2 and 1.3, offering
/// `http/1.1` the way Steel does.
fn server_tls() -> Outcome<ServerConfig> {
    ensure_crypto_provider();
    let (der, key) = res!(identity());
    let mut cfg = res!(ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![der], key), Init);
    cfg.alpn_protocols.push(b"http/1.1".to_vec());
    Ok(cfg)
}

/// A client with an empty root store, which trusts no certificate a server could show it.
fn client_tls_trusting_nothing() -> ClientConfig {
    ClientConfig::builder()
        .with_root_certificates(RootCertStore::empty())
        .with_no_client_auth()
}

/// Which arm a handshake took, as a word an assertion can show.
fn kind<IO>(h: &Handshake<IO>) -> &'static str {
    match h {
        Handshake::Ok(_)         => "ok",
        Handshake::TimedOut      => "timed out",
        Handshake::PeerFailed(_) => "peer failed",
        Handshake::Failed(_)     => "failed",
    }
}

/// Runs one server-side handshake on loopback against whatever `client` does with its end of the
/// connection, and returns what the acceptor made of it.
async fn serve<F, Fut>(server: ServerConfig, client: F) -> Outcome<Handshake<TcpStream>>
where
    F:      FnOnce(TcpStream) -> Fut + Send + 'static,
    Fut:    Future<Output = ()> + Send + 'static,
{
    let listener = res!(TcpListener::bind("127.0.0.1:0").await, IO, Network);
    let addr = res!(listener.local_addr(), IO, Network);
    let peer = tokio::spawn(async move {
        if let Ok(s) = TcpStream::connect(addr).await {
            client(s).await;
        }
    });
    let (stream, _) = res!(listener.accept().await, IO, Network);
    let acceptor = BoundedTlsAcceptor::new(TlsAcceptor::from(Arc::new(server)), 0, Some(WAIT));
    let h = acceptor.accept(stream).await;
    let _ = peer.await;
    Ok(h)
}

/// A client that completes the handshake as far as it can with `cfg`, then lets go.
async fn connect_with(cfg: ClientConfig, s: TcpStream) {
    let name = match ServerName::try_from(HOST) {
        Ok(n)  => n,
        Err(_) => return,
    };
    let _ = TlsConnector::from(Arc::new(cfg)).connect(name, s).await;
}

/// A server that has no certificate for anybody, as when none loaded at start.
#[derive(Debug)]
struct NoCert;

impl ResolvesServerCert for NoCert {
    fn resolve(&self, _hello: ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
        None
    }
}

#[tokio::test]
async fn test_a_client_that_trusts_the_server_completes_the_handshake() -> Outcome<()> {
    // The client's one root is the server's own certificate, so nothing about it can be refused.
    ensure_crypto_provider();
    let (der, key) = res!(identity());
    let mut srv = res!(ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![der.clone()], key), Init);
    srv.alpn_protocols.push(b"http/1.1".to_vec());
    let mut roots = RootCertStore::empty();
    res!(roots.add(der), Init);
    let mut cli = ClientConfig::builder().with_root_certificates(roots).with_no_client_auth();
    cli.alpn_protocols.push(b"http/1.1".to_vec());
    let h = res!(serve(srv, move |s| connect_with(cli, s)).await);
    assert_eq!(kind(&h), "ok");
    Ok(())
}

#[tokio::test]
async fn test_a_client_with_no_application_protocol_in_common_is_the_peers_fault() -> Outcome<()> {
    let mut cli = client_tls_trusting_nothing();
    cli.alpn_protocols.push(b"h2".to_vec());
    let h = res!(serve(res!(server_tls()), move |s| connect_with(cli, s)).await);
    assert_eq!(kind(&h), "peer failed", "an ALPN offer the server cannot meet");
    Ok(())
}

#[tokio::test]
async fn test_a_client_offering_no_protocol_version_we_serve_is_the_peers_fault() -> Outcome<()> {
    ensure_crypto_provider();
    // A server that will only speak TLS 1.3 and a client that will only speak TLS 1.2.
    let (der, key) = res!(identity());
    let srv = res!(ServerConfig::builder_with_protocol_versions(&[&version::TLS13])
        .with_no_client_auth()
        .with_single_cert(vec![der], key), Init);
    let cli = ClientConfig::builder_with_protocol_versions(&[&version::TLS12])
        .with_root_certificates(RootCertStore::empty())
        .with_no_client_auth();
    let h = res!(serve(srv, move |s| connect_with(cli, s)).await);
    assert_eq!(kind(&h), "peer failed", "a version the server does not offer");
    Ok(())
}

#[tokio::test]
async fn test_a_client_that_hangs_up_before_it_speaks_is_the_peers_fault() -> Outcome<()> {
    let h = res!(serve(res!(server_tls()), |s| async move { drop(s); }).await);
    assert_eq!(kind(&h), "peer failed", "a connection closed with no ClientHello");
    Ok(())
}

#[tokio::test]
async fn test_a_client_that_speaks_plain_http_to_a_tls_port_is_the_peers_fault() -> Outcome<()> {
    let h = res!(serve(res!(server_tls()), |mut s| async move {
        let _ = s.write_all(b"\x16\x03\x01\x00\x05GET /\r\n\r\n").await;
        let _ = s.flush().await;
    }).await);
    assert_eq!(kind(&h), "peer failed", "nonsense after a TLS record header");
    Ok(())
}

#[tokio::test]
async fn test_a_server_with_no_certificate_to_serve_is_not_blamed_on_the_client() -> Outcome<()> {
    ensure_crypto_provider();
    let srv = ServerConfig::builder()
        .with_no_client_auth()
        .with_cert_resolver(Arc::new(NoCert));
    let cli = client_tls_trusting_nothing();
    let h = res!(serve(srv, move |s| connect_with(cli, s)).await);
    assert_eq!(kind(&h), "failed", "a fault of the server's own must stay a fault");
    Ok(())
}

#[tokio::test]
async fn test_a_client_that_rejects_the_certificate_is_not_blamed_on_the_client() -> Outcome<()> {
    // The client's alert says our certificate is unacceptable, which is how an expired or
    // mismatched certificate shows up here, so it is not written off as a scanner.
    let cli = client_tls_trusting_nothing();
    let h = res!(serve(res!(server_tls()), move |s| connect_with(cli, s)).await);
    assert_eq!(kind(&h), "failed", "a client's alert against the server's certificate");
    Ok(())
}
