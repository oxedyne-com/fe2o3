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

const ACME_TLS_ALPN_NAME: &[u8] = b"acme-tls/1";

#[derive(Debug)]
pub struct SteelCertResolver {
    by_hostname:        RwLock<HashMap<String, Arc<CertifiedKey>>>,
    default_cert:       RwLock<Option<Arc<CertifiedKey>>>,
    challenge_certs:    RwLock<HashMap<String, Arc<CertifiedKey>>>,
}

impl SteelCertResolver {
    pub fn new() -> Self {
        Self {
            by_hostname:        RwLock::new(HashMap::new()),
            default_cert:       RwLock::new(None),
            challenge_certs:    RwLock::new(HashMap::new()),
        }
    }

    pub fn insert_vhost_cert(&self, hostnames: &[String], cert: Arc<CertifiedKey>) {
        {
            let mut default = lock_write_or_recover!(self.default_cert,
                "SteelCertResolver.default_cert RwLock was poisoned; \
                    recovering.");
            if default.is_none() {
                *default = Some(cert.clone());
            }
        }
        let mut map = lock_write_or_recover!(self.by_hostname,
            "SteelCertResolver.by_hostname RwLock was poisoned; \
                recovering.");
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
            let map = lock_read_or_recover!(self.challenge_certs);
            return map.get(&name).cloned();
        }

        // Regular handshake: SNI lookup then default cert fallback.
        if let Some(name) = client_hello.server_name() {
            let map = lock_read_or_recover!(self.by_hostname);
            if let Some(cert) = map.get(&name.to_lowercase()) {
                return Some(cert.clone());
            }
        }
        let default = lock_read_or_recover!(self.default_cert);
        default.clone()
    }
}

impl ChallengeInstaller for SteelCertResolver {
    fn install(&self, hostname: &str, cert: &ChallengeCert) -> Outcome<()> {
        let certified = res!(der_to_certified_key(&cert.cert_der, &cert.key_der));
        let mut map = lock_write_or_recover!(self.challenge_certs);
        map.insert(hostname.to_lowercase(), Arc::new(certified));
        Ok(())
    }

    fn remove(&self, hostname: &str) -> Outcome<()> {
        let mut map = lock_write_or_recover!(self.challenge_certs);
        map.remove(&hostname.to_lowercase());
        Ok(())
    }
}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ LOADED TLS STATE                                                          │
// └───────────────────────────────────────────────────────────────────────────┘

pub struct LoadedTls {
    pub server_config:  rustls::server::ServerConfig,
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

pub struct AcmeRenewer {
    client:     AcmeClient,
    cache:      AcmeDiskCache,
    resolver:   Arc<SteelCertResolver>,
    dns_names:  Vec<String>,
}

impl AcmeRenewer {

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

const RENEWAL_POLL_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);

const RENEWAL_LEAD_SECS: i64 = 30 * 24 * 60 * 60;


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ CERTIFICATE                                                               │
// └───────────────────────────────────────────────────────────────────────────┘

pub struct Certificate;

impl Certificate {

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
