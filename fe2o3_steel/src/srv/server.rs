use crate::{
    app::mail::{
        AppMailHandler,
        run_outbound_worker,
    },
    srv::{
        cert::Certificate,
        cfg::MailConfig,
        context::{
            Protocol,
            ServerContext,
        },
        http::{
            handle_redirect,
            run_redirect_listener,
        },
        mail::{
            build_smtp_servers,
            run_imap_listener,
            run_smtp_listener,
            AppImapServer,
        },
    },
};

use oxedyne_fe2o3_mail::{
    maildir::MaildirStore,
    outbound::OutboundSpool,
    passwd::PasswdFileUserStore,
};
use oxedyne_fe2o3_net::{
    dkim::DkimSigner,
    imap::server::ImapServer,
    smtp::client::OutboundClient,
};

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_iop_crypto::enc::Encrypter;
use oxedyne_fe2o3_iop_db::api::Database;
use oxedyne_fe2o3_iop_hash::api::Hasher;
use oxedyne_fe2o3_jdat::id::NumIdDat;
use oxedyne_fe2o3_net::{
    http::handler::WebHandler,
    ws::handler::WebSocketHandler,
};

use std::{
    net::SocketAddr,
    sync::Arc,
};

use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;


/// The Steel TCP/TLS server, wrapping a `ServerContext`.
pub struct Server<
    const UIDL: usize,
    UID:    NumIdDat<UIDL> + 'static,
    ENC:    Encrypter,
    KH:     Hasher,
    DB:     Database<UIDL, UID, ENC, KH>,
    WH:     WebHandler,
    WSH:    WebSocketHandler,
> {
    pub context: ServerContext<UIDL, UID, ENC, KH, DB, WH, WSH>,
}

impl<
    const UIDL: usize,
    UID:    NumIdDat<UIDL> + 'static,
    ENC:    Encrypter + 'static,
    KH:     Hasher + 'static,
    DB:     Database<UIDL, UID, ENC, KH> + 'static,
    WH:     WebHandler + 'static,
    WSH:    WebSocketHandler + 'static,
>
    Server<UIDL, UID, ENC, KH, DB, WH, WSH>
{
    /// Construct a new server from a pre-built context.
    pub fn new(
        context: ServerContext<UIDL, UID, ENC, KH, DB, WH, WSH>,
    )
        -> Self
    {
        Self { context }
    }

    /// Bind the configured address and port, perform TLS + vhost dispatch
    /// in an accept loop, and hand each accepted connection off to the
    /// `handle_https` method on a fresh Tokio task.
    pub async fn start(&self) -> Outcome<()> {

        let dev_mode = match &self.context.protocol {
            Protocol::Web { dev_mode, .. } => *dev_mode,
        };

        let loaded = res!(Certificate::load(
            &self.context.cfg,
            &self.context.root,
            dev_mode,
        ));

        // If ACME is enabled, spawn the renewer task. It drives the
        // initial issuance (if the cache is empty) and then loops with
        // a 24-hour tick, re-issuing whenever the cached cert is older
        // than the renewal threshold.
        if let Some(renewer) = loaded.acme_renewer {
            tokio::spawn(async move {
                if let Err(e) = renewer.run_forever().await {
                    error!(err!(e,
                        "ACME renewer task exited.";
                        Init, Network));
                }
            });
        }

        // Spawn the plaintext HTTP redirect listener if configured.
        // This binds a separate port (typically 80) and responds to every
        // incoming HTTP request with a 301 to the HTTPS equivalent, so
        // browsers that do not default to HTTPS-first mode can still reach
        // the site by typing a bare hostname.
        let http_port = self.context.cfg.server_port_tcp_plaintext;
        if http_port != 0 {
            let address = self.context.cfg.server_address.clone();
            let https_port = self.context.cfg.server_port_tcp;
            tokio::spawn(async move {
                if let Err(e) = run_redirect_listener(
                    address,
                    http_port,
                    https_port,
                ).await {
                    error!(err!(e,
                        "Plaintext HTTP redirect listener exited.";
                        Init, Network));
                }
            });
        }

        // Spawn the localhost plain-HTTP admin listener if
        // configured. This binds 127.0.0.1:<port> for /admin only,
        // intended to be reached via SSH tunnel when the public
        // TLS chain is broken or the operator wants emergency
        // access. Bound only to loopback by design; there is no
        // network-exposed knob.
        let admin_local_port = self.context.cfg.admin_local_port;
        if admin_local_port != 0 {
            let ctx_for_local = self.context.clone();
            tokio::spawn(async move {
                if let Err(e) = ctx_for_local.run_admin_local_listener(
                    admin_local_port,
                ).await {
                    error!(err!(e,
                        "Admin localhost listener exited.";
                        Init, Network));
                }
            });
        }

        let tls_acceptor = TlsAcceptor::from(Arc::new(loaded.server_config));

        // Spawn mail listeners if a mail block is configured.
        if let Some(mail_cfg) = res!(self.context.cfg.get_mail()) {
            if let Err(e) = spawn_mail_listeners(
                &mail_cfg,
                &self.context.root,
                tls_acceptor.clone(),
                &self.context.cfg.server_address,
            ).await {
                error!(err!(e,
                    "Failed to spawn mail listeners.";
                    Init, Network));
            }
        }

        // Build the bind address from the (now honoured) server_cfg.
        let addr: SocketAddr = {
            let ip: std::net::IpAddr = match self.context.cfg.server_address.parse() {
                Ok(ip) => ip,
                Err(e) => return Err(err!(e,
                    "Invalid server_address '{}' in config.",
                    self.context.cfg.server_address;
                    Invalid, Input, Network)),
            };
            SocketAddr::new(ip, self.context.cfg.server_port_tcp)
        };
        let listener = res!(TcpListener::bind(&addr).await, IO, Network);
        info!("Listening on: {}", addr);

        loop {
            let (stream, src_addr) = match listener.accept().await {
                Ok(pair) => pair,
                Err(e) => {
                    error!(err!(e, "TCP connection aborted."; IO, Network));
                    continue;
                }
            };

            // Peek at first bytes to detect TLS handshake. Non-TLS requests
            // receive a 308 to redirect the caller to HTTPS.
            let mut peek_buf = [0u8; 5];
            match stream.peek(&mut peek_buf).await {
                Ok(n) if n >= 5 && peek_buf[0] == 0x16 && peek_buf[1] == 0x03 => {
                    match tls_acceptor.accept(stream).await {
                        Ok(tls_stream) => {
                            // Extract SNI now, before we hand ownership of
                            // the stream to the handler task.
                            let sni = tls_stream.get_ref().1.server_name()
                                .map(|s| s.to_string());
                            let context_clone = self.context.clone();
                            match &self.context.protocol {
                                Protocol::Web { .. } => {
                                    tokio::spawn(async move {
                                        if let Err(e) = context_clone.handle_https(
                                            tls_stream,
                                            sni,
                                            src_addr,
                                        ).await {
                                            error!(err!(e,
                                                "Error handling HTTPS connection.";
                                                IO, Network));
                                        }
                                    });
                                }
                            }
                        }
                        Err(e) => {
                            error!(err!(e,
                                "TLS handshake aborted.";
                                IO, Network, Init));
                            continue;
                        }
                    }
                }
                _ => {
                    // Non-TLS connection on the TLS port: redirect to HTTPS
                    // using the incoming Host header so the redirect target
                    // matches whatever the client typed.
                    let https_port = self.context.cfg.server_port_tcp;
                    if let Err(e) = handle_redirect(
                        stream,
                        src_addr,
                        https_port,
                    ).await {
                        error!(err!(e,
                            "Failed to redirect plaintext HTTP on TLS port.";
                            IO, Network, Write));
                    }
                    continue;
                }
            }
        }
    }
}


/// Build and spawn the SMTP and IMAP listeners + outbound worker.
///
/// Resolves every relative path under the steel app root, opens (or
/// generates) the DKIM key, and binds three TCP listeners that share
/// the same TLS acceptor as the HTTPS server.
async fn spawn_mail_listeners(
    cfg:            &MailConfig,
    root:           &oxedyne_fe2o3_core::path::NormPathBuf,
    tls_acceptor:   TlsAcceptor,
    bind_address:   &str,
)
    -> Outcome<()>
{
    use oxedyne_fe2o3_core::path::NormalPath;
    use std::path::{Path, PathBuf};

    // Resolve paths under the app root.
    let resolve = |rel: &str| -> PathBuf {
        let p = Path::new(rel);
        if p.is_absolute() {
            p.to_path_buf()
        } else {
            root.clone().join(p.normalise()).absolute().as_pathbuf()
        }
    };

    // Maildir storage root.
    let maildir_root = if cfg.maildir_root.is_empty() {
        return Err(err!(
            "MailConfig: maildir_root must be set."; Invalid, Input, Missing));
    } else {
        resolve(&cfg.maildir_root)
    };
    if !maildir_root.exists() {
        if let Err(e) = std::fs::create_dir_all(&maildir_root) {
            return Err(err!(e,
                "Creating maildir root {:?}.", maildir_root;
                IO, File, Init));
        }
    }
    let store = res!(MaildirStore::new(maildir_root, cfg.hostname.clone()));

    // User store.
    if cfg.users_file_rel.is_empty() {
        return Err(err!(
            "MailConfig: users_file_rel must be set."; Invalid, Input, Missing));
    }
    let users = PasswdFileUserStore::new(resolve(&cfg.users_file_rel));

    // Outbound spool.
    if cfg.spool_dir_rel.is_empty() {
        return Err(err!(
            "MailConfig: spool_dir_rel must be set."; Invalid, Input, Missing));
    }
    let spool = res!(OutboundSpool::new(resolve(&cfg.spool_dir_rel)));

    // DKIM signer (optional).
    let dkim: Option<Arc<DkimSigner>> = if cfg.dkim_key_file.is_empty() {
        None
    } else {
        let path = resolve(&cfg.dkim_key_file);
        let domain = if cfg.dkim_domain.is_empty() {
            cfg.hostname.clone()
        } else {
            cfg.dkim_domain.clone()
        };
        let selector = if cfg.dkim_selector.is_empty() {
            "default".to_string()
        } else {
            cfg.dkim_selector.clone()
        };
        let bytes = if path.exists() {
            match std::fs::read(&path) {
                Ok(b) => b,
                Err(e) => return Err(err!(e,
                    "Reading DKIM key {:?}.", path;
                    IO, File, Read)),
            }
        } else {
            // Generate a fresh key, persist it, and log the public DNS
            // record value so the operator can publish it.
            info!("DKIM: no key at {:?}, generating fresh ed25519 pair.", path);
            let s = res!(DkimSigner::generate(domain.clone(), selector.clone()));
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            if let Err(e) = std::fs::write(&path, s.pkcs8_bytes()) {
                return Err(err!(e,
                    "Writing DKIM key {:?}.", path;
                    IO, File, Write));
            }
            info!("DKIM TXT record for {}._domainkey.{}: {}",
                selector, domain, s.dns_txt_record());
            s.pkcs8_bytes().to_vec()
        };
        let signer = res!(DkimSigner::from_pkcs8(&bytes, domain.clone(), selector.clone()));
        info!("DKIM TXT record for {}._domainkey.{}: {}",
            selector, domain, signer.dns_txt_record());
        Some(Arc::new(signer))
    };

    let handler = AppMailHandler {
        store:          store.clone(),
        users:          users.clone(),
        spool:          spool.clone(),
        dkim,
        local_domains:  Arc::new(cfg.local_domains.clone()),
    };

    let hostname = Arc::new(cfg.hostname.clone());

    // Build the SMTP servers.
    let (recv_server, sub_server) = build_smtp_servers(
        handler,
        users.clone(),
        Some(tls_acceptor.clone()),
        hostname.clone(),
    );

    // Bind addresses.
    let bind_ip: std::net::IpAddr = match bind_address.parse() {
        Ok(ip) => ip,
        Err(e) => return Err(err!(e,
            "Invalid server_address '{}'.", bind_address;
            Invalid, Input, Network)),
    };

    // SMTP receive on port 25.
    if cfg.smtp_port != 0 {
        let addr = SocketAddr::new(bind_ip, cfg.smtp_port);
        let server = recv_server.clone();
        tokio::spawn(async move {
            if let Err(e) = run_smtp_listener(addr, server).await {
                error!(err!(e, "SMTP receive listener exited."; IO, Network));
            }
        });
    }
    // SMTP submission on port 587.
    if cfg.submission_port != 0 {
        let addr = SocketAddr::new(bind_ip, cfg.submission_port);
        let server = sub_server.clone();
        tokio::spawn(async move {
            if let Err(e) = run_smtp_listener(addr, server).await {
                error!(err!(e, "SMTP submission listener exited."; IO, Network));
            }
        });
    }
    // IMAP on port 993.
    if cfg.imap_port != 0 {
        let addr = SocketAddr::new(bind_ip, cfg.imap_port);
        let server: AppImapServer = ImapServer {
            store,
            users,
            hostname: hostname.clone(),
        };
        let acceptor = tls_acceptor.clone();
        tokio::spawn(async move {
            if let Err(e) = run_imap_listener(addr, acceptor, server).await {
                error!(err!(e, "IMAP listener exited."; IO, Network));
            }
        });
    }
    // Outbound delivery worker.
    let client = res!(OutboundClient::with_system_roots(cfg.hostname.clone()));
    tokio::spawn(async move {
        if let Err(e) = run_outbound_worker(spool, client).await {
            error!(err!(e, "Outbound delivery worker exited."; IO, Network));
        }
    });

    Ok(())
}
