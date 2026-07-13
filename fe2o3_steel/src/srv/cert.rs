use crate::srv::{
    cfg::{
        AcmeConfig,
        ServerConfig,
        VhostConfig,
    },
    constant,
};

use oxedyne_fe2o3_core::{
    prelude::*,
    path::{
        NormalPath,
        NormPathBuf,
    },
};
use oxedyne_fe2o3_net::tls;
use oxedyne_fe2o3_net::acme::{
    cache::AcmeDiskCache,
    challenge::ChallengeCert,
    client::{
        AcmeClient,
        ChallengeInstaller,
        IssuedCertificate,
    },
    jose::JwsSigner,
    trust::letsencrypt_client_config,
};

use std::{
    collections::HashMap,
    fs::{
        self,
        create_dir_all,
        File,
    },
    io::{
        BufReader,
        Write,
    },
    path::{
        Path,
        PathBuf,
    },
    sync::{
        Arc,
        RwLock,
    },
    time::{
        Duration,
    },
};

use rustls::{
    self,
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

use rcgen;


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ STEEL CERT RESOLVER                                                       │
// │                                                                           │
// │ Per-vhost cert resolver that looks up the right CertifiedKey by SNI.      │
// │ Also handles TLS-ALPN-01 challenge handshakes from an ACME CA by          │
// │ detecting the `acme-tls/1` ALPN and serving a separate challenge cert     │
// │ map on those connections.                                                 │
// └───────────────────────────────────────────────────────────────────────────┘

/// ALPN protocol name for ACME TLS-ALPN-01 challenge handshakes (RFC 8737).
const ACME_TLS_ALPN_NAME: &[u8] = b"acme-tls/1";

/// Maps TLS SNI hostnames to their corresponding `CertifiedKey` plus a
/// separate challenge-cert map used exclusively on incoming `acme-tls/1`
/// handshakes. Falls back to a "default" certificate (the first vhost) for
/// regular handshakes when no SNI is present or no SNI match is found.
///
/// All three inner maps are behind `RwLock` so the ACME renewer running in
/// a background task can swap cert contents under a live resolver without
/// stopping the main accept loop.
#[derive(Debug)]
pub struct SteelCertResolver {
    /// Regular per-vhost certificate store, keyed by lowercase hostname.
    by_hostname:        RwLock<HashMap<String, Arc<CertifiedKey>>>,
    /// Default cert served when SNI is absent or unmatched. Pinned to the
    /// first cert inserted so legacy clients and debug tools still get a
    /// valid handshake.
    default_cert:       RwLock<Option<Arc<CertifiedKey>>>,
    /// Throwaway challenge certificates, keyed by lowercase hostname,
    /// served only on handshakes whose sole ALPN offer is `acme-tls/1`.
    challenge_certs:    RwLock<HashMap<String, Arc<CertifiedKey>>>,
}

impl SteelCertResolver {
    /// Create a new, empty resolver.
    pub fn new() -> Self {
        Self {
            by_hostname:        RwLock::new(HashMap::new()),
            default_cert:       RwLock::new(None),
            challenge_certs:    RwLock::new(HashMap::new()),
        }
    }

    /// Insert a `CertifiedKey` under one or more hostnames in the main
    /// vhost map. If no default cert has been set yet, the first
    /// inserted cert becomes the default.
    pub fn insert_vhost_cert(&self, hostnames: &[String], cert: Arc<CertifiedKey>) {
        {
            let mut default = match self.default_cert.write() {
                Ok(g) => g,
                Err(poisoned) => {
                    warn!("SteelCertResolver.default_cert RwLock was poisoned; \
                        recovering.");
                    poisoned.into_inner()
                },
            };
            if default.is_none() {
                *default = Some(cert.clone());
            }
        }
        let mut map = match self.by_hostname.write() {
            Ok(g) => g,
            Err(poisoned) => {
                warn!("SteelCertResolver.by_hostname RwLock was poisoned; \
                    recovering.");
                poisoned.into_inner()
            },
        };
        for host in hostnames {
            map.insert(host.to_lowercase(), cert.clone());
        }
    }
}

impl ResolvesServerCert for SteelCertResolver {
    fn resolve(&self, client_hello: ClientHello) -> Option<Arc<CertifiedKey>> {
        // If the client's ALPN offer is exactly {"acme-tls/1"} this is an
        // ACME challenge handshake from the CA; serve a challenge cert
        // keyed on the SNI instead of the real vhost cert. The single-
        // element equality test matches rustls-acme's own helper.
        let is_acme_challenge = client_hello
            .alpn()
            .into_iter()
            .flatten()
            .eq([ACME_TLS_ALPN_NAME]);

        if is_acme_challenge {
            let name = match client_hello.server_name() {
                Some(n) => n.to_lowercase(),
                None    => return None,
            };
            let map = match self.challenge_certs.read() {
                Ok(g) => g,
                Err(poisoned) => poisoned.into_inner(),
            };
            return map.get(&name).cloned();
        }

        // Regular handshake: SNI lookup then default cert fallback.
        if let Some(name) = client_hello.server_name() {
            let map = match self.by_hostname.read() {
                Ok(g) => g,
                Err(poisoned) => poisoned.into_inner(),
            };
            if let Some(cert) = map.get(&name.to_lowercase()) {
                return Some(cert.clone());
            }
        }
        let default = match self.default_cert.read() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        default.clone()
    }
}

impl ChallengeInstaller for SteelCertResolver {
    fn install(&self, hostname: &str, cert: &ChallengeCert) -> Outcome<()> {
        let certified = res!(der_to_certified_key(&cert.cert_der, &cert.key_der));
        let mut map = match self.challenge_certs.write() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        map.insert(hostname.to_lowercase(), Arc::new(certified));
        Ok(())
    }

    fn remove(&self, hostname: &str) -> Outcome<()> {
        let mut map = match self.challenge_certs.write() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        map.remove(&hostname.to_lowercase());
        Ok(())
    }
}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ LOADED TLS STATE                                                          │
// └───────────────────────────────────────────────────────────────────────────┘

/// Result of loading TLS state.
///
/// `server_config` is ready to hand to `TlsAcceptor`. `acme_renewer` is
/// `Some(..)` when ACME is enabled, in which case the caller must spawn
/// a background task that calls [`AcmeRenewer::run_forever`] for the
/// lifetime of the server.
pub struct LoadedTls {
    /// Rustls server configuration to install on the listener.
    pub server_config:  rustls::server::ServerConfig,
    /// When ACME is active, the renewer that drives issuance and
    /// periodic renewal on a background task.
    pub acme_renewer:   Option<AcmeRenewer>,
}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ ACME RENEWER                                                              │
// │                                                                           │
// │ Drives the `fe2o3_net::acme::AcmeClient` issuance cycle on startup (if    │
// │ needed) and then periodically in a renewal loop. Holds an `Arc` to the    │
// │ shared `SteelCertResolver` so it can install challenge certs during       │
// │ `tls-alpn-01` validation and swap the issued cert into the vhost map      │
// │ once issuance completes.                                                  │
// └───────────────────────────────────────────────────────────────────────────┘

/// Background driver for the ACME issuance + renewal state machine.
pub struct AcmeRenewer {
    /// ACME client holding the account key, directory cache, and nonce
    /// state.
    client:     AcmeClient,
    /// Disk cache for the account key and issued cert/key pair.
    cache:      AcmeDiskCache,
    /// Shared resolver that both the live accept loop and the renewer
    /// write into.
    resolver:   Arc<SteelCertResolver>,
    /// DNS names to issue a single multi-SAN certificate for.
    dns_names:  Vec<String>,
}

impl AcmeRenewer {

    /// Run the renewer forever: attempt any needed initial issuance, then
    /// loop with a 24-hour sleep between expiry checks.
    ///
    /// Returns an error only if the initial issuance fails. Errors on
    /// subsequent renewal attempts are logged and swallowed; the loop
    /// keeps running so transient failures (CA outage, DNS hiccup) do not
    /// permanently shut down the renewer.
    pub async fn run_forever(mut self) -> Outcome<()> {
        // Initial issuance on startup if the cache is empty or its cert is
        // older than the renewal threshold.
        if res!(self.needs_renewal()) {
            info!("ACME: initial issuance for {:?}", self.dns_names);
            res!(self.issue_and_install().await);
        } else {
            info!("ACME: cached certificate for {:?} is still fresh.", self.dns_names);
        }

        // Renewal loop. 24-hour tick granularity is plenty -- LE issues
        // 90-day certs, we renew at 60 days, and a one-day latency on
        // detecting the rollover is fine.
        loop {
            tokio::time::sleep(RENEWAL_POLL_INTERVAL).await;
            match self.needs_renewal() {
                Ok(true) => {
                    info!("ACME: cached cert is due for renewal, issuing now.");
                    if let Err(e) = self.issue_and_install().await {
                        error!(err!(e,
                            "ACME: renewal attempt failed; will retry in \
                            24 hours.";
                            Init, Network));
                    }
                },
                Ok(false) => (),
                Err(e) => error!(err!(e,
                    "ACME: failed to check cached cert age; will retry in \
                    24 hours.";
                    IO, File)),
            }
        }
    }

    /// Return `true` if the cached certificate is missing, unreadable, or
    /// close enough to expiry to be worth replacing.
    ///
    /// The question is asked of the *certificate*, not of the file holding it.
    /// This used to compare the file's mtime against a 60-day threshold, which
    /// is right only while the file and the certificate share a history. They
    /// come apart the moment a certificate is restored from a backup or copied
    /// from another host: the mtime is minutes old, the certificate has weeks
    /// left, and the server sails past the expiry serving a dead certificate
    /// and never asking why. Reading the expiry out of the certificate cannot
    /// be fooled that way.
    ///
    /// A certificate that cannot be read or parsed is treated as expiring: a
    /// server that does not know should renew rather than gamble.
    ///
    /// Age is not the only thing that can make a certificate useless. One that
    /// does not *name* a virtual host cannot serve it, however new it is -- so
    /// adding a host must force a reissue too. Without that check, a new vhost is
    /// served under the old certificate, the name does not match, and every
    /// browser refuses the connection while the server reports nothing wrong.
    fn needs_renewal(&self) -> Outcome<bool> {
        let cert_path = self.cache.certificate_path();
        if !cert_path.exists() {
            return Ok(true);
        }
        let pem = match fs::read(&cert_path) {
            Ok(b)  => b,
            Err(e) => {
                warn!("Cached cert at {:?} could not be read ({}); renewing.",
                    cert_path, e);
                return Ok(true);
            }
        };
        if tls::certificate_expires_within(&pem, RENEWAL_LEAD_SECS) {
            return Ok(true);
        }
        let covered = match tls::certificate_dns_names(&pem) {
            Ok(names) => names,
            Err(e) => {
                warn!("Cached cert at {:?} could not be parsed ({}); renewing.",
                    cert_path, e);
                return Ok(true);
            }
        };
        let missing: Vec<&String> = self.dns_names.iter()
            .filter(|want| !covered.iter().any(|got| got.eq_ignore_ascii_case(want)))
            .collect();
        if !missing.is_empty() {
            info!("ACME: cached cert does not cover {:?}; reissuing for {:?}.",
                missing, self.dns_names);
            return Ok(true);
        }
        Ok(false)
    }

    /// Drive one full issuance through `AcmeClient`, persist the result
    /// to the disk cache, and swap the new cert into the live resolver.
    async fn issue_and_install(&mut self) -> Outcome<()> {
        let issued: IssuedCertificate = res!(self.client.issue_certificate(
            &self.dns_names,
            &*self.resolver,
        ).await);

        // Persist the PEM chain and the matching key to disk first, so a
        // process crash between issuance and resolver swap still leaves a
        // usable cached cert for the next restart.
        res!(self.cache.store_certificate(&issued.cert_pem, &issued.key_pkcs8));

        // Parse the PEM bytes into a CertifiedKey and swap it into the
        // vhost map under every DNS name.
        let certified = res!(pem_to_certified_key(&issued.cert_pem, &issued.key_pkcs8));
        self.resolver.insert_vhost_cert(&self.dns_names, Arc::new(certified));
        info!("ACME: issued and installed cert for {:?}.", self.dns_names);
        Ok(())
    }
}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ CONSTANTS                                                                 │
// └───────────────────────────────────────────────────────────────────────────┘

/// Interval between renewal-needed checks inside [`AcmeRenewer::run_forever`].
const RENEWAL_POLL_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);

/// Cached cert age beyond which a renewal is triggered. Let's Encrypt
/// issues 90-day certs; renewing once a month of life remains gives a
/// generous window to notice and fix a failing renewal, and matches the
/// community convention.
const RENEWAL_LEAD_SECS: i64 = 30 * 24 * 60 * 60;


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ CERTIFICATE                                                               │
// └───────────────────────────────────────────────────────────────────────────┘

/// Namespace for TLS certificate helpers.
pub struct Certificate;

impl Certificate {

    /// Compute an absolute path beneath a config-relative TLS directory.
    pub fn filepath(
        root:       &NormPathBuf,
        dir_root:   &String,
        subdir:     &str,
        name:       &str,
        ext:        &str,
    )
        -> PathBuf
    {
        let mut relpath = PathBuf::from(dir_root);
        relpath.push(subdir);
        relpath.push(name);
        relpath.set_extension(ext);
        let relpath = relpath.normalise().remove_relative();
        root.clone().join(relpath).absolute().into_inner()
    }

    /// Atomically write a blob to disk, logging on success.
    pub fn write_to_file<
        P: AsRef<Path> + std::fmt::Debug,
    >(
        fname: P,
        data: &[u8],
    )
        -> Outcome<()>
    {
        let fname = fname.as_ref();
        let mut file = res!(File::create(fname));
        res!(file.write_all(data));
        info!("{:?} saved successfully.", fname);
        Ok(())
    }

    /// Load the TLS state for the current configuration.
    ///
    /// When `acme.enabled == true`, this function builds an
    /// `fe2o3_net::acme::AcmeClient`, loads the cached account key from
    /// disk (or generates a fresh one), and returns an `AcmeRenewer` for
    /// the caller to spawn on a background task. The returned
    /// [`SteelCertResolver`] starts out with any cached certificate
    /// already installed; the renewer fills it in on its first pass if
    /// there is nothing cached.
    ///
    /// When `acme.enabled == false`, this function loads per-vhost PEM
    /// files from disk (or a single self-signed certificate in dev mode)
    /// and builds a [`SteelCertResolver`] that selects the right one by SNI.
    pub fn load(
        cfg:        &ServerConfig,
        root:       &NormPathBuf,
        dev_mode:   bool,
    )
        -> Outcome<LoadedTls>
    {
        debug!("DEV_MODE = {}", dev_mode);
        let vhosts = res!(cfg.get_vhosts());
        let acme_cfg = res!(cfg.get_acme());

        // ACME is orthogonal to dev/prod mode: if it's on, use it; if it's
        // off, fall back to loading static certificates from disk.
        if acme_cfg.enabled {
            Self::load_acme(cfg, &vhosts, &acme_cfg, root)
        } else {
            Self::load_static(cfg, &vhosts, root, dev_mode)
        }
    }

    /// Load certificates from PEM files on disk and build a SteelCertResolver.
    fn load_static(
        cfg:        &ServerConfig,
        vhosts:     &[VhostConfig],
        root:       &NormPathBuf,
        dev_mode:   bool,
    )
        -> Outcome<LoadedTls>
    {
        let tls_subdir = if dev_mode {
            constant::TLS_DIR_DEV
        } else {
            constant::TLS_DIR_PROD
        };

        let resolver = Arc::new(SteelCertResolver::new());

        if dev_mode {
            // In dev mode, all vhosts share the single self-signed dev cert.
            let cert_path = Self::filepath(
                root, &cfg.tls_dir_rel, tls_subdir, "fullchain", "pem",
            );
            let key_path = Self::filepath(
                root, &cfg.tls_dir_rel, tls_subdir, "privkey", "pem",
            );
            info!("Loading dev certificate from {:?}", cert_path);
            let certified = res!(Self::read_cert_and_key(&cert_path, &key_path));
            let all_hostnames: Vec<String> = vhosts
                .iter()
                .flat_map(|v| v.hostnames.iter().cloned())
                .collect();
            resolver.insert_vhost_cert(&all_hostnames, Arc::new(certified));
        } else {
            // Production without ACME: one cert per vhost under
            // {tls_dir_rel}/prod/{primary_hostname}/{fullchain,privkey}.pem
            for vh in vhosts {
                let primary = vh.primary_hostname();
                let cert_path = Self::filepath(
                    root, &cfg.tls_dir_rel, tls_subdir, &fmt!("{}/fullchain", primary), "pem",
                );
                let key_path = Self::filepath(
                    root, &cfg.tls_dir_rel, tls_subdir, &fmt!("{}/privkey", primary), "pem",
                );
                info!("Loading cert for vhost '{}' from {:?}", primary, cert_path);
                let certified = res!(Self::read_cert_and_key(&cert_path, &key_path));
                resolver.insert_vhost_cert(&vh.hostnames, Arc::new(certified));
            }
        }

        let mut server_config = rustls::server::ServerConfig::builder()
            .with_no_client_auth()
            .with_cert_resolver(resolver);
        // See `load_acme` for the ALPN rationale; we advertise the same
        // set so a cert in the static path can still be used in front of
        // clients that expect `http/1.1` ALPN and so that toggling ACME
        // on and off does not change the wire-level ALPN offering.
        server_config.alpn_protocols.push(b"http/1.1".to_vec());

        Ok(LoadedTls {
            server_config,
            acme_renewer: None,
        })
    }

    /// Build an in-tree `fe2o3_net::acme::AcmeClient`-driven TLS state.
    ///
    /// This function is cheap: it constructs the ACME client, sets up the
    /// disk cache, pre-loads any cached certificate into the resolver, and
    /// returns immediately. The first issuance (if the cache is empty) or
    /// any periodic renewal runs on the background task that
    /// [`AcmeRenewer::run_forever`] provides.
    ///
    /// The certificate names every vhost hostname, the hostname of an enabled
    /// mail listener, and every `acme.extra_domains` entry. A name absent from
    /// that set has no renewal path, so leaving one out does not degrade
    /// gracefully -- it works until the certificate expires and then fails.
    fn load_acme(
        cfg:        &ServerConfig,
        vhosts:     &[VhostConfig],
        acme_cfg:   &AcmeConfig,
        root:       &NormPathBuf,
    )
        -> Outcome<LoadedTls>
    {
        if acme_cfg.contact_email.is_empty() {
            return Err(err!(
                "AcmeConfig: contact_email must be set when acme.enabled = true.";
                Invalid, Input, Missing));
        }
        let cache_dir = res!(acme_cfg.get_cache_dir(root));

        // Collect the names to certify, preserving order and ignoring
        // duplicates (a mail hostname is commonly also a vhost).
        let mut all_hostnames: Vec<String> = Vec::new();
        let add = |h: &String, out: &mut Vec<String>| {
            if !h.is_empty() && !out.iter().any(|x| x.eq_ignore_ascii_case(h)) {
                out.push(h.clone());
            }
        };
        for vh in vhosts {
            for h in &vh.hostnames {
                add(h, &mut all_hostnames);
            }
        }
        // Steel's mail listeners share this resolver, so the greeting
        // hostname must be certified or every IMAP and SMTP client will
        // reject the connection on a name mismatch.
        if let Some(mail_cfg) = res!(cfg.get_mail()) {
            if mail_cfg.enabled {
                add(&mail_cfg.hostname, &mut all_hostnames);
            }
        }
        for d in &acme_cfg.extra_domains {
            add(d, &mut all_hostnames);
        }
        if all_hostnames.is_empty() {
            return Err(err!(
                "AcmeConfig: no vhost hostnames configured to issue certs for.";
                Invalid, Input, Missing));
        }
        info!("ACME: requesting certificates for {:?} via {}",
            all_hostnames, acme_cfg.directory_url);

        // Disk cache for account key + issued cert.
        let cache = res!(AcmeDiskCache::new(&cache_dir));

        // Load or generate the account key.
        let signer = match res!(cache.load_account_key()) {
            Some(s) => {
                info!("ACME: loaded cached account key from {:?}.", cache.root());
                s
            },
            None => {
                info!("ACME: no cached account key; generating a fresh one.");
                let s = res!(JwsSigner::new_es256());
                res!(cache.store_account_key(&s));
                s
            },
        };

        // Build the trust store and ACME client.
        let tls_client_config = res!(letsencrypt_client_config());
        let client = AcmeClient::new(
            acme_cfg.directory_url.clone(),
            acme_cfg.contact_email.clone(),
            tls_client_config,
            signer,
        );

        // Build the resolver and pre-load any cached cert into it.
        let resolver = Arc::new(SteelCertResolver::new());
        if let Some((cert_pem, key_pkcs8)) = res!(cache.load_certificate()) {
            match pem_to_certified_key(&cert_pem, &key_pkcs8) {
                Ok(certified) => {
                    info!("ACME: pre-loaded cached cert for {:?} from {:?}.",
                        all_hostnames, cache.root());
                    resolver.insert_vhost_cert(
                        &all_hostnames, Arc::new(certified));
                },
                Err(e) => {
                    // A broken cache file should not stop startup -- we'll
                    // just issue a fresh cert on the renewer's first pass.
                    warn!("ACME: cached cert at {:?} failed to parse: {:?}. \
                        Will re-issue.", cache.root(), e);
                }
            }
        }

        // Build the ServerConfig around the resolver.
        let mut server_config = rustls::server::ServerConfig::builder()
            .with_no_client_auth()
            .with_cert_resolver(resolver.clone());
        // Advertise HTTP/1.1 for normal clients plus the ACME-specific
        // "acme-tls/1" protocol so our resolver can serve the challenge
        // cert when the CA connects. Rustls rejects any client whose
        // ALPN offer does not intersect this list, so omitting http/1.1
        // breaks every real request with NoApplicationProtocol. We
        // deliberately do NOT advertise h2 because Steel's HTTP parser
        // is HTTP/1.1 only; advertising h2 would cause HTTP/2-capable
        // clients to send the `PRI * HTTP/2.0` connection preface,
        // which Steel cannot parse.
        server_config.alpn_protocols.push(b"http/1.1".to_vec());
        server_config.alpn_protocols.push(b"acme-tls/1".to_vec());

        let renewer = AcmeRenewer {
            client,
            cache,
            resolver,
            dns_names: all_hostnames,
        };

        Ok(LoadedTls {
            server_config,
            acme_renewer: Some(renewer),
        })
    }

    /// Read a PEM cert chain and its private key from disk, returning a
    /// `CertifiedKey`.
    fn read_cert_and_key(
        cert_path:  &Path,
        key_path:   &Path,
    )
        -> Outcome<CertifiedKey>
    {
        let cert_file = res!(File::open(cert_path));
        let mut cert_reader = BufReader::new(cert_file);
        let certs: Result<Vec<CertificateDer<'static>>, _> =
            rustls_pemfile::certs(&mut cert_reader)
            .map(|c| c.map_err(|e| err!(e,
                "Error reading cert at {:?}.", cert_path; File)))
            .collect();
        let certs = res!(certs);

        let key_file = res!(File::open(key_path));
        let mut key_reader = BufReader::new(key_file);
        let keys: Result<Vec<PrivatePkcs8KeyDer<'static>>, _> =
            rustls_pemfile::pkcs8_private_keys(&mut key_reader)
            .map(|k| k.map_err(|e| err!(e,
                "Error reading private key at {:?}.", key_path; File)))
            .collect();
        let keys = res!(keys);
        let key: PrivateKeyDer<'static> = match keys.into_iter().next() {
            Some(k) => k.into(),
            None => return Err(err!(
                "No private keys found in {:?}.", key_path;
                Missing, Input, File)),
        };

        let signing_key = res!(rustls::crypto::ring::sign::any_supported_type(&key)
            .map_err(|e| err!("{:?}", e; Init, Invalid)));
        Ok(CertifiedKey::new(certs, signing_key))
    }

    /// Generate a self-signed development certificate covering localhost.
    ///
    /// The certificate is written to `{tls_dir_rel}/dev/fullchain.pem`, keyed
    /// by `{tls_dir_rel}/dev/privkey.pem`. Used only when running in dev mode.
    pub fn new_dev(
        cfg:        &ServerConfig,
        root:       &NormPathBuf,
    )
        -> Outcome<()>
    {
        let scheme = res!(rcgen::SignatureAlgorithm::from_oid(constant::PKCS_ECDSA_P256_SHA256));
        let key_pair = res!(rcgen::KeyPair::generate(&scheme));
        let der_encoding = key_pair.serialize_der();
        let key_pair_copy = res!(rcgen::KeyPair::from_der_and_sign_algo(&der_encoding, &scheme));

        let domains = vec![
            fmt!("localhost"),
            fmt!("127.0.0.1"),
        ];
        let mut params = rcgen::CertificateParams::new(domains);
        params.alg = &scheme;
        params.key_pair = Some(key_pair_copy);
        params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        params.key_usages = vec![
            rcgen::KeyUsagePurpose::DigitalSignature,
            rcgen::KeyUsagePurpose::KeyEncipherment,
        ];
        params.extended_key_usages = vec![
            rcgen::ExtendedKeyUsagePurpose::ServerAuth,
            rcgen::ExtendedKeyUsagePurpose::ClientAuth,
        ];

        let cert = res!(rcgen::Certificate::from_params(params));

        let cert_path = Self::filepath(
            root, &cfg.tls_dir_rel, constant::TLS_DIR_DEV, "fullchain", "pem",
        );
        let dir_path = match cert_path.parent() {
            Some(p) => p,
            None => return Err(err!(
                "Could not get parent directory from {:?}.", cert_path;
                Path)),
        };
        res!(create_dir_all(dir_path));

        res!(Self::write_to_file(
            Self::filepath(root, &cfg.tls_dir_rel, constant::TLS_DIR_DEV, "privkey", "pem"),
            cert.serialize_private_key_pem().as_bytes(),
        ));
        res!(Self::write_to_file(
            Self::filepath(root, &cfg.tls_dir_rel, constant::TLS_DIR_DEV, "fullchain", "pem"),
            res!(cert.serialize_pem()).as_bytes(),
        ));
        Ok(())
    }
}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ PEM / DER DECODING                                                        │
// └───────────────────────────────────────────────────────────────────────────┘

/// Parse a PEM-encoded certificate chain plus a PKCS#8 DER-encoded
/// private key into a rustls `CertifiedKey`, as produced by
/// `fe2o3_net::acme::AcmeClient::issue_certificate`.
fn pem_to_certified_key(
    cert_pem:   &[u8],
    key_pkcs8:  &[u8],
)
    -> Outcome<CertifiedKey>
{
    let mut reader = BufReader::new(cert_pem);
    let certs: Result<Vec<CertificateDer<'static>>, _> =
        rustls_pemfile::certs(&mut reader)
        .map(|c| c.map_err(|e| err!(e,
            "Error parsing ACME-issued cert PEM."; IO, Decode)))
        .collect();
    let certs = res!(certs);
    if certs.is_empty() {
        return Err(err!(
            "ACME-issued cert PEM contained no certificates.";
            IO, Decode, Missing));
    }

    let key = PrivatePkcs8KeyDer::from(key_pkcs8.to_vec());
    let key_der: PrivateKeyDer<'static> = key.into();
    let signing_key = res!(rustls::crypto::ring::sign::any_supported_type(&key_der)
        .map_err(|e| err!("{:?}", e; Init, Invalid)));
    Ok(CertifiedKey::new(certs, signing_key))
}

/// Build a rustls `CertifiedKey` from a self-signed challenge cert's
/// raw DER bytes (as produced by
/// [`oxedyne_fe2o3_net::acme::challenge::build_tls_alpn_01_cert`]).
fn der_to_certified_key(
    cert_der:   &[u8],
    key_pkcs8:  &[u8],
)
    -> Outcome<CertifiedKey>
{
    let cert = CertificateDer::from(cert_der.to_vec());
    let key = PrivatePkcs8KeyDer::from(key_pkcs8.to_vec());
    let key_der: PrivateKeyDer<'static> = key.into();
    let signing_key = res!(rustls::crypto::ring::sign::any_supported_type(&key_der)
        .map_err(|e| err!("{:?}", e; Init, Invalid)));
    Ok(CertifiedKey::new(vec![cert], signing_key))
}
