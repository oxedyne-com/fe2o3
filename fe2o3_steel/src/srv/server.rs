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

/// Interval between dashboard-state persistence writes. A restart or
/// crash can lose at most one tick's worth of derived history, which is
/// a reasonable trade-off against ozone write load.
pub const PERSIST_INTERVAL_SECS: u64 = 60;


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

    /// Resolve the database handle associated with the primary vhost.
    /// Used by admin-state persistence to settle on a single database
    /// for dashboard-wide state without a second configuration knob.
    fn primary_db_handle(&self) -> Option<(Arc<std::sync::RwLock<DB>>, UID)> {
        let default_vhost = match &self.context.protocol {
            Protocol::Web { default_vhost, .. } => default_vhost.clone(),
        };
        self.context.db_for_vhost(&default_vhost)
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

        // Spawn the periodic traffic sampler if the traffic
        // recorder is configured. Samples counters into the
        // recorder's bounded history ring every
        // DEFAULT_SAMPLE_INTERVAL_SECS seconds so the dashboard
        // traffic view has time-series data to chart. Lives here
        // (inside the async Server::start) rather than in the sync
        // AppShellContext::start_server so the current tokio
        // runtime is active when the spawn happens.
        if let Some(recorder) = self.context.traffic.clone() {
            tokio::spawn(async move {
                let mut ticker = tokio::time::interval(
                    std::time::Duration::from_secs(
                        crate::srv::admin::traffic::DEFAULT_SAMPLE_INTERVAL_SECS,
                    ),
                );
                // Skip the first immediate tick so the first
                // real sample is taken after one full interval.
                ticker.tick().await;
                loop {
                    ticker.tick().await;
                    if let Err(e) = recorder.sample_now() {
                        warn!("traffic sampler: {}", e);
                    }
                }
            });
        }

        // Spawn the periodic host sampler if the admin state is
        // configured. Reads `/proc/*` via `fe2o3_sys::Snapshot`
        // and pushes a new entry into the sampler's bounded
        // history ring on every tick, so the dashboard's host
        // resource strip has real CPU / memory / disk / network
        // data to render. Lives next to the traffic sampler so
        // both get the same late-spawn treatment.
        //
        // Before the sampler starts, load any persisted sparkline
        // points the previous run saved to ozone and seed the
        // sampler so the Overview chart does not reset to blank
        // across a restart.
        if let Some(admin) = self.context.admin_state.clone() {
            let sampler = admin.host_sampler.clone();

            // Start-up restore. Pick the default vhost's database
            // (the primary / the vhost the dashboard is attached
            // to) and pull the derived history back.
            let primary_db = self.primary_db_handle();
            if let Some((db, _uid)) = primary_db.as_ref() {
                let db_guard = db.read();
                if let Ok(db_g) = db_guard {
                    match crate::srv::admin::persist::load_host_points(&*db_g) {
                        Ok(points) if !points.is_empty() => {
                            info!("admin persist: restored {} derived host points.",
                                points.len());
                            if let Err(e) = sampler.seed_persisted(points) {
                                warn!("host sampler seed: {}", e);
                            }
                        },
                        Ok(_) => {
                            info!("admin persist: no previous host history to restore.");
                        },
                        Err(e) => {
                            warn!("admin persist: load host points failed: {}", e);
                        },
                    }
                }
            }

            let sampler_for_task = sampler.clone();
            tokio::spawn(async move {
                let mut ticker = tokio::time::interval(
                    std::time::Duration::from_secs(
                        crate::srv::admin::host_sampler::DEFAULT_SAMPLE_INTERVAL_SECS,
                    ),
                );
                // Prime with an immediate read so the dashboard
                // has a non-empty ring on the first request.
                if let Err(e) = sampler_for_task.sample_now() {
                    warn!("host sampler prime: {}", e);
                }
                ticker.tick().await;
                loop {
                    ticker.tick().await;
                    if let Err(e) = sampler_for_task.sample_now() {
                        warn!("host sampler: {}", e);
                    }
                }
            });

            // Periodic saver: write the merged derived history to
            // ozone every PERSIST_INTERVAL_SECS so a crash or
            // restart loses at most one tick's worth of history.
            // Spawned in addition to -- not instead of -- the
            // live sampler task so a slow disk cannot throttle
            // the sampling cadence.
            if let Some((db, uid)) = primary_db {
                let sampler_for_save = sampler.clone();
                tokio::spawn(async move {
                    let mut ticker = tokio::time::interval(
                        std::time::Duration::from_secs(PERSIST_INTERVAL_SECS),
                    );
                    ticker.tick().await;
                    loop {
                        ticker.tick().await;
                        let points = match sampler_for_save.merged_derived_history() {
                            Ok(p) => p,
                            Err(e) => {
                                warn!("admin persist: merged history failed: {}", e);
                                continue;
                            },
                        };
                        if points.is_empty() {
                            continue;
                        }
                        let db_guard = db.read();
                        let db_g = match db_guard {
                            Ok(g) => g,
                            Err(_) => {
                                warn!("admin persist: db read lock poisoned.");
                                continue;
                            },
                        };
                        if let Err(e) = crate::srv::admin::persist::save_host_points(
                            &*db_g,
                            uid,
                            &points,
                        ) {
                            warn!("admin persist: save host points failed: {}", e);
                        }
                    }
                });
            }
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

        // Spawn the mail listeners only when the mail server is enabled. A site that
        // merely sends newsletters wants a DKIM identity and an outbound client, not
        // to bind SMTP-receive, submission and IMAP and become an MX -- so sending is
        // built separately (see the newsletter sender in `app/server.rs`), and this
        // gate keeps a send-only host from standing up a mail server it never asked
        // for.
        if let Some(mail_cfg) = res!(self.context.cfg.get_mail()) {
            if mail_cfg.enabled {
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

        // Shared address guard, cloned once per accept. Cheap -- it's an Arc.
        let addr_guard = self.context.admin_state.as_ref()
            .map(|a| a.addr_guard.clone());

        loop {
            let (stream, src_addr) = match listener.accept().await {
                Ok(pair) => pair,
                Err(e) => {
                    error!(err!(e, "TCP connection aborted."; IO, Network));
                    continue;
                }
            };

            // Address-guard check runs before the TLS handshake so that a
            // blacklisted attacker costs the server only a TCP SYN/ACK.
            // Absent admin state (unusual -- only happens when the admin
            // dashboard is not configured) the guard is skipped entirely.
            if let Some(guard) = addr_guard.as_ref() {
                match guard.check(&src_addr.ip()) {
                    Ok(decision) if decision.should_drop() => {
                        debug!("addr guard dropped TCP from {}: {:?}",
                            src_addr, decision);
                        drop(stream);
                        continue;
                    }
                    Ok(_) => (),
                    Err(e) => {
                        warn!("addr guard error for {}: {}", src_addr, e);
                    }
                }
            }

            // ── Per-connection processing ────────────────────────
            //
            // Spawn immediately so the accept loop is never blocked
            // by a slow client, an incomplete TLS handshake, or a
            // scanner that connects without sending data.  Without
            // this, a single hung peek() or TLS accept() would
            // prevent all new connections from being accepted.
            let context_clone = self.context.clone();
            let tls_acceptor_conn = tls_acceptor.clone();
            tokio::spawn(async move {
                // Peek at first bytes to detect TLS handshake.
                // Non-TLS requests receive a 308 to redirect to HTTPS.
                let mut peek_buf = [0u8; 5];
                match stream.peek(&mut peek_buf).await {
                    Ok(n) if n >= 5 && peek_buf[0] == 0x16 && peek_buf[1] == 0x03 => {
                        match tls_acceptor_conn.accept(stream).await {
                            Ok(tls_stream) => {
                                // Extract SNI now, before we hand
                                // ownership of the stream to the handler.
                                let sni = tls_stream.get_ref().1.server_name()
                                    .map(|s| s.to_string());
                                match &context_clone.protocol {
                                    Protocol::Web { .. } => {
                                        if let Err(e) = context_clone.handle_https(
                                            tls_stream,
                                            sni,
                                            src_addr,
                                        ).await {
                                            error!(err!(e,
                                                "Error handling HTTPS connection.";
                                                IO, Network));
                                        }
                                    }
                                }
                            }
                            Err(e) => {
                                error!(err!(e,
                                    "TLS handshake aborted.";
                                    IO, Network, Init));
                            }
                        }
                    }
                    _ => {
                        // Non-TLS connection on the TLS port: redirect
                        // to HTTPS using the incoming Host header so the
                        // redirect target matches whatever the client
                        // typed.
                        let https_port = context_clone.cfg.server_port_tcp;
                        if let Err(e) = handle_redirect(
                            stream,
                            src_addr,
                            https_port,
                        ).await {
                            error!(err!(e,
                                "Failed to redirect plaintext HTTP on TLS port.";
                                IO, Network, Write));
                        }
                    }
                }
            });
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

    // DKIM signers. Shared with the alerter, which signs its own mail: an
    // unsigned alert from a domain that signs everything else is exactly the
    // message a spam filter is entitled to distrust, and it is the one message
    // that has to arrive.
    let dkim = res!(load_dkim_signers(cfg, root));

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

/// Load the DKIM signing identities named in a `MailConfig`.
///
/// Shared by the mail listeners and by the operator alerter, so an alert is
/// signed with the same keys as everything else the domain sends.
///
/// RFC 8463 says a signer SHOULD publish and sign with both an ed25519 and an
/// RSA key. The reason is practical: ed25519 verification is still patchy in
/// the wild, and a receiver that cannot verify a signature sees an *unsigned*
/// message, leaving DMARC to rest on SPF alone. Two signatures cost a few
/// hundred bytes and let each receiver take whichever it understands.
pub fn load_dkim_signers(
    cfg:    &MailConfig,
    root:   &oxedyne_fe2o3_core::path::NormPathBuf,
)
    -> Outcome<Vec<Arc<DkimSigner>>>
{
    use oxedyne_fe2o3_core::path::NormalPath;
    use std::path::{Path, PathBuf};

    let resolve = |rel: &str| -> PathBuf {
        let p = Path::new(rel);
        if p.is_absolute() {
            p.to_path_buf()
        } else {
            root.clone().join(p.normalise()).absolute().as_pathbuf()
        }
    };

    let mut dkim: Vec<Arc<DkimSigner>> = Vec::new();

    let domain = if cfg.dkim_domain.is_empty() {
        cfg.hostname.clone()
    } else {
        cfg.dkim_domain.clone()
    };

    // The ed25519 key, generated in tree if absent.
    if !cfg.dkim_key_file.is_empty() {
        let path = resolve(&cfg.dkim_key_file);
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
            info!("DKIM: no key at {:?}, generating a fresh ed25519 pair.", path);
            let s = res!(DkimSigner::generate(domain.clone(), selector.clone()));
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            if let Err(e) = std::fs::write(&path, s.pkcs8_bytes()) {
                return Err(err!(e,
                    "Writing DKIM key {:?}.", path;
                    IO, File, Write));
            }
            s.pkcs8_bytes().to_vec()
        };
        let signer = res!(DkimSigner::from_pkcs8(&bytes, domain.clone(), selector.clone()));
        info!("DKIM ({}) TXT record for {}._domainkey.{}: {}",
            signer.algorithm(), selector, domain, signer.dns_txt_record());
        dkim.push(Arc::new(signer));
    }

    // The RSA key. Never generated -- `ring` will not generate RSA keys, and
    // hand-rolling the arithmetic to do so is not a road worth taking to save
    // one command:
    //
    //   openssl genpkey -algorithm RSA -pkeyopt rsa_keygen_bits:2048 \
    //       -outform DER -out mail/dkim_rsa.key
    if !cfg.dkim_rsa_key_file.is_empty() {
        let path = resolve(&cfg.dkim_rsa_key_file);
        let selector = if cfg.dkim_rsa_selector.is_empty() {
            "rsa".to_string()
        } else {
            cfg.dkim_rsa_selector.clone()
        };
        if !path.exists() {
            return Err(err!(
                "MailConfig: dkim_rsa_key_file {:?} does not exist. An RSA DKIM \
                key is generated once, offline: `openssl genpkey -algorithm RSA \
                -pkeyopt rsa_keygen_bits:2048 -outform DER -out {:?}`. Steel will \
                not generate it, because ring will not.",
                path, path;
                Init, Missing, File));
        }
        let bytes = match std::fs::read(&path) {
            Ok(b) => b,
            Err(e) => return Err(err!(e,
                "Reading RSA DKIM key {:?}.", path;
                IO, File, Read)),
        };
        let signer = res!(DkimSigner::from_pkcs8(&bytes, domain.clone(), selector.clone()));
        info!("DKIM ({}) TXT record for {}._domainkey.{}: {}",
            signer.algorithm(), selector, domain, signer.dns_txt_record());
        dkim.push(Arc::new(signer));
    }

    if dkim.is_empty() {
        warn!("Mail is enabled with no DKIM key. Outbound mail will be unsigned, \
            and receivers will judge it on SPF alone.");
    }
    Ok(dkim)
}
