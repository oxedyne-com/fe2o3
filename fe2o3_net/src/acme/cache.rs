//! Disk-backed cache for ACME client state.
//!
//! The ACME client needs three things to persist across restarts so it can
//! resume instead of re-registering and re-issuing from scratch every time:
//!
//! 1. The account private key (PKCS#8), generated once and reused for the
//!    lifetime of the ACME account.
//! 2. The currently-issued certificate chain in PEM form.
//! 3. The matching private key in PKCS#8 DER form.
//!
//! This module owns the file layout under a single cache directory:
//!
//! ```text
//! <cache_dir>/
//!     account_key.pkcs8    <- raw PKCS#8 DER bytes for the ACME account
//!     cert.pem             <- issued TLS cert chain in PEM
//!     cert_key.pkcs8       <- matching TLS private key in PKCS#8 DER
//! ```
//!
//! All writes go through an atomic write-then-rename helper so a crashed
//! or killed process cannot leave a partial file behind that the next
//! start-up would read as truncated garbage. The two private keys go
//! through `fe2o3_core`'s secret variant on top of that, so they land at
//! mode 0600 whatever the umask; `cert.pem` is public and keeps the plain
//! atomic write.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use crate::acme::jose::JwsSigner;

use oxedyne_fe2o3_core::{
    prelude::*,
    file as core_file,
};

use std::{
    fs,
    io::Write,
    path::{
        Path,
        PathBuf,
    },
};


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ CACHE                                                                     │
// └───────────────────────────────────────────────────────────────────────────┘

/// An ACME client's account key and last-issued certificate, under one root
/// directory.
#[derive(Clone, Debug)]
pub struct AcmeDiskCache {
    root: PathBuf,
}

impl AcmeDiskCache {

    /// Creates the root directory, at mode 0700, if it is not already there.
    pub fn new<P: AsRef<Path>>(root: P) -> Outcome<Self> {
        let root = root.as_ref().to_path_buf();
        if let Err(e) = core_file::create_secret_dir(&root) {
            return Err(err!(e,
                "Failed to create ACME cache directory {:?}.", root;
                File, IO, Init));
        }
        Ok(Self { root })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Named whether or not the file exists, so a caller may probe its mtime or
    /// delete it.
    pub fn certificate_path(&self) -> PathBuf {
        self.root.join(CERT_PEM_FILE)
    }

    /// Named whether or not the file exists.
    pub fn account_key_path(&self) -> PathBuf {
        self.root.join(ACCOUNT_KEY_FILE)
    }

    /// `Ok(None)` where no key has been stored yet, which is the first run.
    pub fn load_account_key(&self) -> Outcome<Option<JwsSigner>> {
        let path = self.root.join(ACCOUNT_KEY_FILE);
        if !path.exists() {
            return Ok(None);
        }
        // Tighten a key that predates `save_secret`, or that arrived at a
        // wider mode some other way, before it is ever read.
        res!(core_file::restrict_secret(&path));
        let bytes = match fs::read(&path) {
            Ok(b) => b,
            Err(e) => return Err(err!(e,
                "Failed to read cached account key at {:?}.", path;
                File, IO, Read)),
        };
        Ok(Some(res!(JwsSigner::from_pkcs8(&bytes))))
    }

    /// The write is atomic, replaces any existing key, and leaves the file
    /// at mode 0600 whatever the umask: this is the account's private key.
    pub fn store_account_key(&self, signer: &JwsSigner) -> Outcome<()> {
        let path = self.root.join(ACCOUNT_KEY_FILE);
        res!(core_file::save_secret(&path, signer.pkcs8_bytes()));
        Ok(())
    }

    /// The pair is `(cert_pem, key_pkcs8_der)`, verbatim as stored; parsing the
    /// chain and handing the key to `rustls` is the caller's job. `Ok(None)` where
    /// either file is missing, so a half-populated cache reads as no cache.
    pub fn load_certificate(&self) -> Outcome<Option<(Vec<u8>, Vec<u8>)>> {
        let cert_path = self.root.join(CERT_PEM_FILE);
        let key_path  = self.root.join(CERT_KEY_FILE);
        if !cert_path.exists() || !key_path.exists() {
            return Ok(None);
        }
        let cert = match fs::read(&cert_path) {
            Ok(b) => b,
            Err(e) => return Err(err!(e,
                "Failed to read cached certificate at {:?}.", cert_path;
                File, IO, Read)),
        };
        // Tighten the private key before reading it; the certificate above
        // stays untouched, since it is public.
        res!(core_file::restrict_secret(&key_path));
        let key = match fs::read(&key_path) {
            Ok(b) => b,
            Err(e) => return Err(err!(e,
                "Failed to read cached certificate key at {:?}.", key_path;
                File, IO, Read)),
        };
        Ok(Some((cert, key)))
    }

    /// Each file is written atomically, but the pair is not: an interruption
    /// between the two leaves a new certificate beside the old key. The
    /// certificate is public, so it keeps the ordinary atomic write; the key
    /// goes through the secret path and lands at mode 0600.
    pub fn store_certificate(
        &self,
        cert_pem:   &[u8],
        key_pkcs8:  &[u8],
    )
        -> Outcome<()>
    {
        res!(write_atomic(&self.root.join(CERT_PEM_FILE), cert_pem));
        res!(core_file::save_secret(&self.root.join(CERT_KEY_FILE), key_pkcs8));
        Ok(())
    }
}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ CONSTANTS                                                                 │
// └───────────────────────────────────────────────────────────────────────────┘

const ACCOUNT_KEY_FILE: &str = "account_key.pkcs8";
const CERT_PEM_FILE:    &str = "cert.pem";
const CERT_KEY_FILE:    &str = "cert_key.pkcs8";


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ ATOMIC WRITE                                                              │
// └───────────────────────────────────────────────────────────────────────────┘

/// Writes to `<path>.tmp`, fsyncs, then renames, so an interrupted writer never
/// leaves a half-written file under the real name.
fn write_atomic(path: &Path, data: &[u8]) -> Outcome<()> {
    let file_name = match path.file_name() {
        Some(n) => n.to_os_string(),
        None => return Err(err!(
            "ACME cache path {:?} has no file-name component.", path;
            Invalid, Input, Path)),
    };
    let mut tmp = path.to_path_buf();
    tmp.set_file_name(fmt!("{}.tmp", file_name.to_string_lossy()));

    {
        let mut f = match fs::File::create(&tmp) {
            Ok(f) => f,
            Err(e) => return Err(err!(e,
                "Failed to create temporary file {:?}.", tmp;
                File, IO, Create)),
        };
        if let Err(e) = f.write_all(data) {
            return Err(err!(e,
                "Failed to write to temporary file {:?}.", tmp;
                File, IO, Write));
        }
        if let Err(e) = f.sync_all() {
            return Err(err!(e,
                "Failed to fsync temporary file {:?}.", tmp;
                File, IO, Write));
        }
    }

    if let Err(e) = fs::rename(&tmp, path) {
        return Err(err!(e,
            "Failed to rename {:?} -> {:?}.", tmp, path;
            File, IO, Write));
    }
    Ok(())
}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ TESTS                                                                     │
// └───────────────────────────────────────────────────────────────────────────┘

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::atomic::{
        AtomicU64,
        Ordering,
    };

    // Combined with the PID this gives each test a path that cannot collide,
    // even when the suite runs across threads.
    static COUNTER: AtomicU64 = AtomicU64::new(0);

    /// A cache directory under `/tmp` that is unique per test run and per
    /// test, with a best-effort Drop that cleans it up.
    struct ScratchDir {
        path: PathBuf,
    }

    impl ScratchDir {
        fn new(label: &str) -> Self {
            let n = COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(fmt!(
                "fe2o3_acme_cache_test_{}_{}_{}",
                std::process::id(),
                n,
                label,
            ));
            let _ = fs::remove_dir_all(&path);
            Self { path }
        }
    }

    impl Drop for ScratchDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    /// New cache in an empty directory: account key load returns None,
    /// certificate load returns None.
    #[test]
    fn test_empty_cache_reports_none() -> Outcome<()> {
        let scratch = ScratchDir::new("empty");
        let cache = res!(AcmeDiskCache::new(&scratch.path));
        match res!(cache.load_account_key()) {
            None => (),
            Some(_) => return Err(err!(
                "Empty cache returned Some(account_key).";
                Test, Mismatch)),
        }
        match res!(cache.load_certificate()) {
            None => (),
            Some(_) => return Err(err!(
                "Empty cache returned Some(certificate).";
                Test, Mismatch)),
        }
        Ok(())
    }

    /// Store an account key and load it back: the reloaded signer must
    /// expose the same PKCS#8 bytes, which in turn proves it holds the
    /// same key material.
    #[test]
    fn test_account_key_round_trip() -> Outcome<()> {
        let scratch = ScratchDir::new("account_key");
        let cache = res!(AcmeDiskCache::new(&scratch.path));
        let signer = res!(JwsSigner::new_es256());
        let original_pkcs8 = signer.pkcs8_bytes().to_vec();

        res!(cache.store_account_key(&signer));
        let loaded = match res!(cache.load_account_key()) {
            Some(s) => s,
            None => return Err(err!(
                "load_account_key returned None immediately after \
                store_account_key.";
                Test, Missing)),
        };

        if loaded.pkcs8_bytes() != original_pkcs8.as_slice() {
            return Err(err!(
                "Reloaded account key has different PKCS#8 bytes (orig {} \
                bytes, reload {} bytes).",
                original_pkcs8.len(), loaded.pkcs8_bytes().len();
                Test, Mismatch));
        }
        Ok(())
    }

    /// Store a certificate and load it back: both blobs must round-trip
    /// byte-for-byte.
    #[test]
    fn test_certificate_round_trip() -> Outcome<()> {
        let scratch = ScratchDir::new("cert");
        let cache = res!(AcmeDiskCache::new(&scratch.path));

        let cert_pem  = b"-----BEGIN CERTIFICATE-----\nFAKE\n-----END CERTIFICATE-----\n";
        let key_pkcs8 = &[0x30u8, 0x01, 0x02, 0x03, 0x04];

        res!(cache.store_certificate(cert_pem, key_pkcs8));
        let (loaded_cert, loaded_key) = match res!(cache.load_certificate()) {
            Some(pair) => pair,
            None => return Err(err!(
                "load_certificate returned None immediately after \
                store_certificate.";
                Test, Missing)),
        };

        if loaded_cert != cert_pem {
            return Err(err!(
                "Reloaded cert PEM does not match stored bytes.";
                Test, Mismatch));
        }
        if loaded_key != key_pkcs8 {
            return Err(err!(
                "Reloaded cert key does not match stored bytes.";
                Test, Mismatch));
        }
        Ok(())
    }

    /// With only the cert file present and the key file missing, the
    /// load must return None (both-or-nothing semantics).
    #[test]
    fn test_partial_cert_state_reports_none() -> Outcome<()> {
        let scratch = ScratchDir::new("partial_cert");
        let cache = res!(AcmeDiskCache::new(&scratch.path));

        let cert_path = scratch.path.join(CERT_PEM_FILE);
        if let Err(e) = fs::write(&cert_path, b"not a real cert") {
            return Err(err!(e,
                "Failed to pre-seed the cert file for the partial-state test.";
                Test, File, IO, Write));
        }

        match res!(cache.load_certificate()) {
            None => Ok(()),
            Some(_) => Err(err!(
                "load_certificate returned Some even though the key file \
                is missing.";
                Test, Mismatch)),
        }
    }

    /// Store twice with different contents to confirm the atomic rename
    /// actually replaces the previous file rather than appending.
    #[test]
    fn test_atomic_overwrite() -> Outcome<()> {
        let scratch = ScratchDir::new("overwrite");
        let cache = res!(AcmeDiskCache::new(&scratch.path));

        res!(cache.store_certificate(b"v1 cert", b"v1 key"));
        res!(cache.store_certificate(b"v2 cert much longer", b"v2 key"));

        let (cert, key) = match res!(cache.load_certificate()) {
            Some(p) => p,
            None => return Err(err!(
                "load_certificate returned None after overwrite.";
                Test, Missing)),
        };
        if cert != b"v2 cert much longer" {
            return Err(err!(
                "cert did not overwrite: got {:?}.",
                String::from_utf8_lossy(&cert);
                Test, Mismatch));
        }
        if key != b"v2 key" {
            return Err(err!(
                "key did not overwrite: got {:?}.",
                String::from_utf8_lossy(&key);
                Test, Mismatch));
        }
        Ok(())
    }

    /// The two private keys must be readable only by their owner; the
    /// certificate is public and must not be locked down by the same change.
    #[test]
    #[cfg(unix)]
    fn test_key_files_are_saved_0600_and_cert_pem_is_not() -> Outcome<()> {
        use std::os::unix::fs::PermissionsExt;

        let scratch = ScratchDir::new("mode");
        let cache = res!(AcmeDiskCache::new(&scratch.path));
        let signer = res!(JwsSigner::new_es256());

        res!(cache.store_account_key(&signer));
        res!(cache.store_certificate(b"cert bytes", b"key bytes"));

        let account_mode = res!(fs::metadata(cache.account_key_path()))
            .permissions().mode() & 0o777;
        if account_mode != 0o600 {
            return Err(err!(
                "account_key.pkcs8 saved at mode {:o}, not 0600.", account_mode;
                Test, Mismatch));
        }

        let key_path = scratch.path.join(CERT_KEY_FILE);
        let key_mode = res!(fs::metadata(&key_path)).permissions().mode() & 0o777;
        if key_mode != 0o600 {
            return Err(err!(
                "cert_key.pkcs8 saved at mode {:o}, not 0600.", key_mode;
                Test, Mismatch));
        }

        // cert.pem must end up at whatever mode an ordinary, non-secret write
        // gets in this environment -- not specifically 0600 -- since a strict
        // umask (0077, as under this fleet's `UMask=` hardening) puts a plain
        // `fs::write` at 0600 too. Comparing against a control file written
        // the same ordinary way, rather than asserting `!= 0o600` outright,
        // is what actually distinguishes "not specially restricted" from
        // "happens to match the secret mode under this umask".
        let control_path = scratch.path.join("control.pem");
        res!(fs::write(&control_path, b"not a secret"));
        let control_mode = res!(fs::metadata(&control_path)).permissions().mode() & 0o777;

        let cert_mode = res!(fs::metadata(cache.certificate_path()))
            .permissions().mode() & 0o777;
        if cert_mode != control_mode {
            return Err(err!(
                "cert.pem saved at mode {:o}, but an ordinary write here lands at {:o}: \
                cert.pem is being restricted like a secret.", cert_mode, control_mode;
                Test, Mismatch));
        }
        Ok(())
    }
}
