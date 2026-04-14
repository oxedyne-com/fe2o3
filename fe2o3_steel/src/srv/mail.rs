//! Mail listener spawner.
//!
//! Boots the SMTP receive (port 25), SMTP submission (port 587) and
//! IMAP (port 993) listeners alongside the HTTPS server, sharing the
//! same rustls server config so a single ACME-issued certificate
//! covers every protocol.

use crate::app::mail::AppMailHandler;

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_mail::passwd::PasswdFileUserStore;
use oxedyne_fe2o3_net::{
    imap::server::ImapServer,
    smtp::server::{
        SmtpMode,
        SmtpServer,
    },
};

use std::{
    net::SocketAddr,
    sync::Arc,
};

use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;


/// Bind a single TCP listener to `addr` and run an SMTP server accept
/// loop on it forever. Errors on individual accepts are logged and
/// swallowed so a single bad connection cannot kill the whole port.
pub async fn run_smtp_listener(
    addr:   SocketAddr,
    server: SmtpServer<AppMailHandler, PasswdFileUserStore>,
)
    -> Outcome<()>
{
    let listener = match TcpListener::bind(&addr).await {
        Ok(l) => l,
        Err(e) => return Err(err!(e,
            "Binding SMTP listener on {}.", addr;
            IO, Network, Init)),
    };
    info!("SMTP {:?} listening on {} (mode={:?})",
        server.mode, addr, server.mode);
    loop {
        let (stream, peer) = match listener.accept().await {
            Ok(p) => p,
            Err(e) => {
                error!(err!(e,
                    "SMTP accept error on {}.", addr;
                    IO, Network));
                continue;
            }
        };
        info!("SMTP {:?} {}: connection from {}", server.mode, addr, peer);
        let server = server.clone();
        tokio::spawn(async move {
            if let Err(e) = server.run(stream, peer).await {
                warn!("SMTP session error from {}: {}", peer, e);
            }
        });
    }
}

/// Bind a single TCP listener to `addr` and run an IMAP server with
/// implicit TLS. Each accepted connection performs a TLS handshake
/// before entering the IMAP state machine.
pub async fn run_imap_listener(
    addr:           SocketAddr,
    tls_acceptor:   TlsAcceptor,
    server:         ImapServer<
        oxedyne_fe2o3_mail::maildir::MaildirStore,
        PasswdFileUserStore,
    >,
)
    -> Outcome<()>
{
    let listener = match TcpListener::bind(&addr).await {
        Ok(l) => l,
        Err(e) => return Err(err!(e,
            "Binding IMAP listener on {}.", addr;
            IO, Network, Init)),
    };
    info!("IMAP listening on {}", addr);
    loop {
        let (stream, peer) = match listener.accept().await {
            Ok(p) => p,
            Err(e) => {
                error!(err!(e,
                    "IMAP accept error on {}.", addr;
                    IO, Network));
                continue;
            }
        };
        info!("IMAP {}: connection from {}", addr, peer);
        let acceptor = tls_acceptor.clone();
        let server = server.clone();
        tokio::spawn(async move {
            let tls = match acceptor.accept(stream).await {
                Ok(t) => t,
                Err(e) => {
                    warn!("IMAP TLS handshake from {} failed: {}", peer, e);
                    return;
                }
            };
            if let Err(e) = server.run(tls, peer).await {
                warn!("IMAP session error from {}: {}", peer, e);
            }
        });
    }
}

/// Marker trait alias to keep the function signatures readable. Not
/// strictly necessary, but it shortens the SMTP listener type.
pub type AppSmtpServer = SmtpServer<AppMailHandler, PasswdFileUserStore>;

/// Marker alias for the IMAP listener.
pub type AppImapServer = ImapServer<
    oxedyne_fe2o3_mail::maildir::MaildirStore,
    PasswdFileUserStore,
>;

/// Build both SMTP servers (receive on 25, submission on 587) sharing
/// one handler. The submission server gets a TLS acceptor for the
/// STARTTLS upgrade; the receive server gets the same one so a
/// modern peer can opportunistically encrypt.
pub fn build_smtp_servers(
    handler:        AppMailHandler,
    users:          PasswdFileUserStore,
    tls_acceptor:   Option<TlsAcceptor>,
    hostname:       Arc<String>,
)
    -> (AppSmtpServer, AppSmtpServer)
{
    let receive = SmtpServer {
        handler:        handler.clone(),
        users:          users.clone(),
        tls_acceptor:   tls_acceptor.clone(),
        hostname:       hostname.clone(),
        mode:           SmtpMode::Receive,
    };
    let submission = SmtpServer {
        handler,
        users,
        tls_acceptor,
        hostname,
        mode:           SmtpMode::Submission,
    };
    (receive, submission)
}
