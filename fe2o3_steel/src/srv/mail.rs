//! Mail listener spawner.
//!
//! Boots the SMTP receive (port 25), SMTP submission (port 587) and
//! IMAP (port 993) listeners alongside the HTTPS server, sharing the
//! same rustls server config so a single ACME-issued certificate
//! covers every protocol.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use crate::app::mail::AppMailHandler;

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_mail::passwd::PasswdFileUserStore;
use oxedyne_fe2o3_net::{
    imap::server::ImapServer,
    smtp::server::{
        SmtpMode,
        SmtpServer,
    },
    tls::{
        BoundedTlsAcceptor,
        Handshake,
    },
};

use std::{
    net::SocketAddr,
    sync::{
        Arc,
        atomic::{
            AtomicU32,
            Ordering,
        },
    },
};

use tokio::net::TcpListener;


/// How many mail listeners bound, against how many the configuration asked for,
/// surfaced in the health body as `mail_down`.
///
/// Counted up front from the configuration, before anything that can fail, so a
/// set-up that dies before it binds anything still shows every listener as down
/// rather than showing no mail server at all.
#[derive(Debug, Default)]
pub struct ListenerTally {
    wanted: AtomicU32,
    bound:  AtomicU32,
}

impl ListenerTally {
    pub fn new_shared() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub fn want(&self) {
        self.wanted.fetch_add(1, Ordering::Relaxed);
    }

    pub fn bound(&self) {
        self.bound.fetch_add(1, Ordering::Relaxed);
    }

    /// Listeners asked for and not bound, or `None` when none were asked for.
    pub fn down(&self) -> Option<u32> {
        let wanted = self.wanted.load(Ordering::Relaxed);
        if wanted == 0 {
            return None;
        }
        Some(wanted.saturating_sub(self.bound.load(Ordering::Relaxed)))
    }
}


/// Runs the accept loop forever. An error on an individual accept is logged and
/// swallowed, so a single bad connection cannot kill the whole port.
pub async fn run_smtp_listener(
    addr:   SocketAddr,
    server: SmtpServer<AppMailHandler, PasswdFileUserStore>,
    tally:  Option<Arc<ListenerTally>>,
)
    -> Outcome<()>
{
    let listener = match TcpListener::bind(&addr).await {
        Ok(l) => l,
        Err(e) => return Err(err!(e,
            "Binding SMTP listener on {}.", addr;
            IO, Network, Init)),
    };
    if let Some(t) = &tally {
        t.bound();
    }
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

/// Implicit TLS: each accepted connection completes a TLS handshake before
/// entering the IMAP state machine.
pub async fn run_imap_listener(
    addr:           SocketAddr,
    tls_acceptor:   BoundedTlsAcceptor,
    server:         ImapServer<
        oxedyne_fe2o3_mail::maildir::MaildirStore,
        PasswdFileUserStore,
    >,
    tally:          Option<Arc<ListenerTally>>,
)
    -> Outcome<()>
{
    let listener = match TcpListener::bind(&addr).await {
        Ok(l) => l,
        Err(e) => return Err(err!(e,
            "Binding IMAP listener on {}.", addr;
            IO, Network, Init)),
    };
    if let Some(t) = &tally {
        t.bound();
    }
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
                Handshake::Ok(t) => t,
                Handshake::TimedOut => {
                    warn!("IMAP TLS handshake from {} timed out.", peer);
                    return;
                }
                Handshake::Failed(e) => {
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

pub type AppSmtpServer = SmtpServer<AppMailHandler, PasswdFileUserStore>;

pub type AppImapServer = ImapServer<
    oxedyne_fe2o3_mail::maildir::MaildirStore,
    PasswdFileUserStore,
>;

/// Builds both SMTP servers -- receive on 25, submission on 587 -- sharing one
/// handler. Both get the TLS acceptor: submission needs it for the STARTTLS
/// upgrade, and the receive server offers it so a modern peer can
/// opportunistically encrypt.
pub fn build_smtp_servers(
    handler:        AppMailHandler,
    users:          PasswdFileUserStore,
    tls_acceptor:   Option<BoundedTlsAcceptor>,
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


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn listener_tally_reports_only_what_was_asked_for() {
        let t = ListenerTally::default();
        assert_eq!(t.down(), None, "a host with no mail server reports no mail field");
        t.want();
        t.want();
        t.want();
        t.bound();
        t.bound();
        assert_eq!(t.down(), Some(1), "three asked for, two bound");
        t.bound();
        assert_eq!(t.down(), Some(0));
    }
}
