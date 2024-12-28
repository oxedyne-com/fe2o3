use crate::srv::{
    cfg::ServerConfig,
    constant,
};

use oxedize_fe2o3_core::{
    prelude::*,
    path::{
        NormalPath,
        NormPathBuf,
    },
};
use oxedize_fe2o3_net::dns::Fqdn;

use std::{
    fs::{
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
};

use rustls::{
    self,
    pki_types::{
        CertificateDer,
        PrivateKeyDer,
        PrivatePkcs8KeyDer,
    },
};

use rcgen;


pub struct Certificate;

impl Certificate {

    pub fn filepath(
        root:       &NormPathBuf,
        dir_root:   &String,
        subdir:     &str,
        name:       &String,
        ext:        &str,
    )
        -> PathBuf
    {
        let mut relpath = PathBuf::from(dir_root);
        relpath.push(subdir);
        relpath.push(name.clone());
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
        -> Outcome<rustls::server::ServerConfig>
    {
        debug!("DEV_MODE = {}", dev_mode);

        let tls_subdir = if dev_mode {
            constant::TLS_DIR_DEV
        } else {
            constant::TLS_DIR_PROD
        };
    
        let cert_path = Self::filepath(
            root,
            &cfg.tls_dir_rel,   // Use updated field name.
            tls_subdir,
            &cfg.tls_cert_name,
            "pem",
        ); 
        info!("Certificate path = {:?}", cert_path);
    
        let key_path = Self::filepath(
            root,
            &cfg.tls_dir_rel,   // Use updated field name.
            tls_subdir,
            &cfg.tls_private_key_name,
            "pem",
        ); 
        info!("Private key path = {:?}", key_path);

        // Load and parse the certificate.
        let cert_file = res!(File::open(&cert_path));
        let mut cert_reader = BufReader::new(cert_file);
        let certs: Result<Vec<CertificateDer>, _> =
            rustls_pemfile::certs(&mut cert_reader)
            .map(|cert_result| cert_result.map_err(|e| err!(e,
                "Error reading cert at {:?}.", cert_path; File)))
            .collect();
        let certs = res!(certs);
    
        // Load and parse the private key.
        let key_file = res!(File::open(&key_path));
        let mut key_reader = BufReader::new(key_file);
        let keys: Result<Vec<PrivatePkcs8KeyDer>, _> =
            rustls_pemfile::pkcs8_private_keys(&mut key_reader)
            .map(|key_result| key_result.map_err(|e| err!(e,
                "Error reading private key at {:?}.", key_path; File)))
            .collect();
        let keys = res!(keys);
    
        let private_key: PrivateKeyDer<'_> = match keys.into_iter().next() {
            Some(key) => key.into(),
            None => return Err(err!("No keys found in key file."; Missing, Input, File)),
        };
    
        let server_cfg = res!(rustls::server::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(certs, private_key));
    
        Ok(server_cfg)
    }

    pub fn new_dev(
        cfg:        &ServerConfig,
        root:       &NormPathBuf,
    )
        -> Outcome<()>
    {
        //let scheme = res!(rcgen::SignatureAlgorithm::from_oid(constant::PKCS_ED25519));
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
        // Add basic constraints.
        params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        // Add key usages.
        params.key_usages = vec![
            rcgen::KeyUsagePurpose::DigitalSignature,
            rcgen::KeyUsagePurpose::KeyEncipherment,
        ];
        // Add extended key usages.
        params.extended_key_usages = vec![
            rcgen::ExtendedKeyUsagePurpose::ServerAuth,
            rcgen::ExtendedKeyUsagePurpose::ClientAuth,
        ];

        let cert = res!(rcgen::Certificate::from_params(params));

        let cert_path = Self::filepath(
            root,
            &cfg.tls_dir_rel,
            constant::TLS_DIR_DEV,
            &cfg.tls_cert_name,
            "pem",
        );

        //let key_path = Self::filepath(
        //    root,
        //    &cfg.tls_dir_rel,
        //    constant::TLS_DIR_DEV,
        //    &cfg.tls_private_key_name,
        //    "pem",
        //);

        // Create directories if they don't exist.
        let dir_path = match cert_path.parent() {
            Some(p) => p,
            None => return Err(err!(
                "Could not get parent directory from {:?}.", cert_path;
                Path)),
        };

        res!(create_dir_all(dir_path));

        res!(Self::write_to_file(
            Self::filepath(
                root,
                &cfg.tls_dir_rel,
                constant::TLS_DIR_DEV,
                &cfg.tls_public_key_name,
                "pem",
            ),
            &key_pair.public_key_pem().as_bytes(),
        ));

        res!(Self::write_to_file(
            Self::filepath(
                root,
                &cfg.tls_dir_rel,
                constant::TLS_DIR_DEV,
                &cfg.tls_private_key_name,
                "pem",
            ),
            &cert.serialize_private_key_pem().as_bytes(),
        ));

        res!(Self::write_to_file(
            Self::filepath(
                root,
                &cfg.tls_dir_rel,
                constant::TLS_DIR_DEV,
                &cfg.tls_cert_name,
                "pem",
            ),
            &res!(cert.serialize_pem()).as_bytes(),
        ));

        //// DER (binary) format.
        //res!(Self::write_to_file(
        //    Self::filepath(root, &dir_str, &cfg.tls_public_key_name, "der"),
        //    &key_pair.public_key_der(),
        //));

        //res!(Self::write_to_file(
        //    Self::filepath(root, &dir_str, &cfg.tls_private_key_name, "der"),
        //    &cert.serialize_private_key_der(),
        //));

        //res!(Self::write_to_file(
        //    Self::filepath(root, &dir_str, &cfg.tls_cert_name, "der"),
        //    &res!(cert.serialize_der()),
        //));

        Ok(())
    }

    /// Generate self-signed certificates.  The problem is that they are not widely trusted.  Use
    /// Lets Encrypt instead.
    ///
    ///     sudo apt-get update
    ///     sudo apt-get install certbot
    ///     sudo certbot certonly --standalone
    ///
    /// Upon successful issuance, your certificate and key will be stored in
    /// /etc/letsencrypt/live/your_domain_name/. The important files are:
    /// The certificate file:
    ///     /etc/letsencrypt/live/your_domain_name/fullchain.pem
    /// The private key file:
    ///     /etc/letsencrypt/live/your_domain_name/privkey.pem
    #[cfg(target_os = "linux")]
    pub fn new_lets_encrypt(
        domains:    &Vec<Fqdn>,
        tls_dir:    &Path,
    )
        -> Outcome<()>
    {
        res!(Self::check_requirements());

        let mut cmd = std::process::Command::new("sudo");
        cmd.arg("certbot")
            .arg("certonly")
            .arg("--standalone")
            .arg("--non-interactive")
            .arg("--force-renewal");

        // Add each domain with -d flag.
        for domain in domains {
            cmd.arg("-d").arg(domain.as_str());
        }

        // Run certbot standalone.
        let output = res!(cmd.output());
    
        if !output.status.success() {
            return Err(err!(
                "Certificate creation failed: {}", String::from_utf8_lossy(&output.stderr);
                IO, File));
        }
    
        // Copy certs from /etc/letsencrypt/live/{domain}/ to tls/
        // Note: Let's Encrypt uses the first domain as the primary domain.
        let src_dir = PathBuf::from("/etc/letsencrypt/live").join(domains[0].as_str());
        res!(std::fs::create_dir_all(tls_dir));
    
        // Get current user.
        let user = res!(std::env::var("USER"));
        
        // Copy and set ownership for both files.
        for file in &["fullchain.pem", "privkey.pem"] {
            res!(std::process::Command::new("sudo")
                .arg("cp")
                .arg(src_dir.join(file))
                .arg(tls_dir.join(file))
                .output());
    
            res!(std::process::Command::new("sudo")
                .arg("chown")
                .arg(fmt!("{}:{}", user, user))
                .arg(tls_dir.join(file))
                .output());
        }
    
        Ok(())
    }
    
    #[cfg(target_os = "windows")] 
    // TODO Windows users this needs to be tested and probably fixed.
    pub fn new_lets_encrypt(
        domains:    &Vec<Fqdn>,
        tls_dir:    &Path,
    )
        -> Outcome<()>
    {
        res!(Self::check_requirements());

        // Create DNS string for multiple domains
        let domain_names = domains
            .iter()
            .map(|d| fmt!("\"{}\"", d))
            .collect::<Vec<_>>()
            .join(", ");
        
        // Run wacs with appropriate parameters
        let output = res!(std::process::Command::new("wacs")
            .arg("--target")
            .arg("manual")
            .arg("--host")
            .arg(domain_list)
            .arg("--installation")
            .arg("script")
            .arg("--script")
            .arg(format!("copy %PfxPath% \"{}\"",
                tls_dir.join("certificate.pfx").display()))
            .arg("--scriptparameters")
            .arg("\"%PfxPath%\"")
            .arg("--store")
            .arg("false")
            .output());

        if !output.status.success() {
            return Err(err!(
                "Certificate creation failed: {}", String::from_utf8_lossy(&output.stderr);
                IO, File));
        }

        // Convert PFX to PEM format
        let pfx_path = tls_dir.join("certificate.pfx");
        let output = res!(std::process::Command::new("openssl")
            .arg("pkcs12")
            .arg("-in")
            .arg(&pfx_path)
            .arg("-out")
            .arg(tls_dir.join("combined.pem"))
            .arg("-nodes")
            .arg("-password")
            .arg("pass:")
            .output());

        if !output.status.success() {
            return Err(err!(
                "PFX to PEM conversion failed: {}", String::from_utf8_lossy(&output.stderr);
                IO, File));
        }

        // Split the combined PEM into separate cert and key files
        let ps_script = fmt!(
            "$pemContent = Get-Content \"{}\"
             $certContent = $pemContent[0..($pemContent.Length-1)] | Where-Object {{ $_ -match 'CERTIFICATE' -or ($_ -notmatch 'KEY' -and $_.trim() -ne '') }}
             $keyContent = $pemContent[0..($pemContent.Length-1)] | Where-Object {{ $_ -match 'PRIVATE KEY' -or ($_ -notmatch 'CERTIFICATE' -and $_.trim() -ne '') }}
             Set-Content -Path \"{}\" -Value $certContent
             Set-Content -Path \"{}\" -Value $keyContent",
            tls_dir.join("combined.pem").display(),
            tls_dir.join("fullchain.pem").display(),
            tls_dir.join("privkey.pem").display()
        );

        let output = res!(std::process::Command::new("powershell")
            .arg("-Command")
            .arg(&ps_script)
            .output());

        if !output.status.success() {
            return Err(err!(
                "Certificate splitting failed: {}", String::from_utf8_lossy(&output.stderr);
                IO, File));
        }

        // Clean up intermediate files
        let _ = std::fs::remove_file(tls_dir.join("certificate.pfx"));
        let _ = std::fs::remove_file(tls_dir.join("combined.pem"));

        // Verify files exist
        let cert_path = tls_dir.join("fullchain.pem");
        let key_path = tls_dir.join("privkey.pem");

        if !cert_path.exists() || !key_path.exists() {
            return Err(err!(
                "Certificate files were not created properly at {:?}", tls_dir;
                IO, File, Missing));
        }

        Ok(())
    }    
    
    #[cfg(target_os = "macos")]
    pub fn new_lets_encrypt(
        domains:    &Vec<Fqdn>,
        tls_dir:    &Path,
    )
        -> Outcome<()>
    {
        res!(Self::check_requirements());

        // Build certbot command with multiple domains.
        let mut cmd = std::process::Command::new("certbot");
        cmd.arg("certonly")
            .arg("--standalone")
            .arg("--non-interactive")
            .arg("--force-renewal");
        
        // Add each domain with -d flag
        for domain in &domains {
            cmd.arg("-d").arg(domain_as_str());
        }

        let output = res!(cmd.output());
    
        if !output.status.success() {
            return Err(err!(
                "Certificate creation failed: {}", String::from_utf8_lossy(&output.stderr);
                IO, File));
        }
    
        // Copy certs from /etc/letsencrypt/live/{domain}/ to tls/
        let src_dir = PathBuf::from("/etc/letsencrypt/live").join(domains[0].as_str());
        res!(std::fs::create_dir_all(tls_dir));
        
        for file in &["fullchain.pem", "privkey.pem"] {
            res!(std::fs::copy(
                src_dir.join(file),
                tls_dir.join(file)
            ));
        }
    
        Ok(())
    }
    
    pub fn check_requirements() -> Outcome<()> {
        #[cfg(target_os = "linux")]
        {
            // Check for certbot.
            let has_certbot = std::process::Command::new("certbot")
                .arg("--version")
                .output()
                .map_or(false, |output| output.status.success());
    
            if !has_certbot {
                return Err(err!(
                    "Certbot not found. Please install via: sudo apt-get install certbot";
                    System, Missing));
            }
        }
    
        #[cfg(target_os = "windows")]
        {
            // Check for admin rights
            if !is_elevated::is_elevated() {
                return Err(err!(
                    "Administrator privileges required. Please run as administrator.";
                    System, Missing));
            }
        
            // Check for win-acme
            let has_wacs = std::process::Command::new("wacs")
                .arg("--version")
                .output()
                .map_or(false, |output| output.status.success());
        
            if !has_wacs {
                return Err(err!(
                    "win-acme (WACS) not found. Please install from https://www.win-acme.com";
                    System, Missing));
            }
        
            // Check for OpenSSL
            let has_openssl = std::process::Command::new("openssl")
                .arg("version")
                .output()
                .map_or(false, |output| output.status.success());
        
            if !has_openssl {
                return Err(err!(
                    "OpenSSL not found. Please install OpenSSL and add it to your PATH.";
                    System, Missing));
            }
        }
    
        #[cfg(target_os = "macos")]
        {
            // Check for homebrew.
            let has_brew = std::process::Command::new("brew")
                .arg("--version")
                .output()
                .map_or(false, |output| output.status.success());
    
            if !has_brew {
                return Err(err!(
                    "Homebrew not found. Please install from https://brew.sh";
                    System, Missing));
            }
        }
    
        Ok(())
    }
}
