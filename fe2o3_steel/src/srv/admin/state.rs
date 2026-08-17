//! Runtime state shared by every dashboard request.
//!
//! Built once at Steel start-up and threaded through the admin
//! handler. Holds the pieces of state that both dashboard auth and
//! session decoding need:
//!
//! - A shared handle to the [`Wallet`] so login calls `unlock` against
//!   the same admin list the CLI sees, and admin management from the
//!   dashboard mutates the same file on disk.
//! - An [`EncryptionScheme`] pre-keyed with the 32-byte dashboard
//!   session key, so session encode/decode does not re-derive on
//!   every request.
//! - The seal: the wallet master key, when it is known.
//!
//! # The seal
//!
//! Steel starts *sealed*. The wallet file is readable without any
//! passphrase -- it holds each admin's password-wrapped copy of the
//! master key -- so the process can bind its listeners, serve every
//! static vhost and renew its certificates while the master key is
//! still unknown and the databases are still shut. Only the routes
//! that actually need a database are refused, with a 503, until an
//! admin unseals.
//!
//! This is the whole point of the arrangement: the *database* key
//! stops being a precondition for the *websites* being up. A restart
//! is no longer an outage that waits on a human at a terminal.
//!
//! The session key is therefore **not** derived from the master key.
//! It is 32 random bytes minted at start-up, because sessions have to
//! work while sealed -- an admin has to be able to reach the unseal
//! page and be issued a cookie before any master key exists. A
//! restart consequently invalidates outstanding dashboard cookies,
//! which is the correct behaviour anyway.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use crate::srv::{
    admin::{
        guard::SteelAddressGuard,
        host_sampler::HostSampler,
        signed_login::NonceTracker,
        traffic::TrafficRecorder,
    },
    alert::Alerter,
    cfg::AdminKey,
};

use oxedyne_fe2o3_core::{
    prelude::*,
    rand::Rand,
};
use oxedyne_fe2o3_crypto::{
    enc::EncryptionScheme,
    keystore::Wallet,
};

use std::{
    path::PathBuf,
    sync::{
        Arc,
        Mutex,
        RwLock,
        atomic::{
            AtomicBool,
            Ordering,
        },
    },
    time::Duration,
};

use secrecy::ExposeSecret;
use tokio::sync::Notify;

pub const SESSION_KEY_LEN: usize = 32; // bytes

/// Result of a successful passphrase unwrap against the wallet.
#[derive(Clone, Debug)]
pub struct Unsealed {
    pub name:   String,       // admin entry whose wrap the passphrase opened
    pub scopes: Vec<String>,
    // True only when this call lifted the seal, rather than finding it already
    // lifted. Only the first correct passphrase after a start unseals; a later
    // one merely re-authenticates. Alerting turns on the distinction: an unseal
    // is a rare, notable event worth an email, and a routine dashboard login is
    // not.
    pub lifted: bool,
}

/// Shared dashboard runtime state.
///
/// Cheaply cloneable: the wallet is behind an `Arc<RwLock<_>>` and the
/// encryption scheme clones its key material.
#[derive(Clone, Debug)]
pub struct AdminState {
    pub wallet:         Arc<RwLock<Wallet>>,
    // Held here so the dashboard's admin-management UI can call `Wallet::save`
    // without depending on `app::constant`.
    pub wallet_path:    PathBuf,
    // `None` while the process is sealed. Read it through
    // `AdminState::master_key`, which fails with a `Sealed` tag rather than
    // handing back an `Option` every caller would have to interpret for itself.
    //
    // Behind an `Arc<RwLock<_>>` because the unseal happens after the listeners
    // are up: the login handler that recovers the key and the server task that
    // opens the databases with it hold different clones of this state. The key
    // is held in clear in process memory, in line with the wallet-v2 design --
    // a human supplies the passphrase, and the unwrapped secret lives in RAM
    // until the process restarts. It is never written to disk.
    master_key:         Arc<RwLock<Option<Vec<u8>>>>,
    // An atomic alongside `master_key` so the request path can test the seal
    // without taking a lock.
    sealed:             Arc<AtomicBool>,
    // Signalled once, when the master key is installed. The database starter
    // task in `Server::start` waits on this; it cannot open the Ozone instances
    // until the key exists.
    unseal_notify:      Arc<Notify>,
    // How many vhosts have a database configured. Zero is common: a deployment
    // serving only static sites, redirects and proxy routes never touches
    // Ozone. The seal is reported against this count, because telling an
    // operator who has no database that "the databases are shut" is how a
    // healthy server gets mistaken for a broken one.
    db_count:           usize,
    // `None` disables alerting. It lives here because the events worth alerting
    // on -- an unseal, a run of failed passphrase attempts -- all happen on the
    // login path, which is the one place that already holds this state.
    alerter:            Option<Alerter>,
    pub session_enc:    EncryptionScheme,    // AES-256-GCM
    // The dashboard reads this when rendering `/admin/traffic`; the request
    // pipeline in `srv/https.rs` writes to it on every completed response. Both
    // sides hold the same `Arc`, so the dashboard sees live data without any
    // per-vhost coordination.
    pub traffic:        Arc<TrafficRecorder>,
    // A background task in `Server::start` calls `HostSampler::sample_now` on a
    // fixed interval; the dashboard reads the same `Arc` when drawing the host
    // resource strip.
    pub host_sampler:   Arc<HostSampler>,
    // The TCP accept loop in `srv/server.rs` calls `check` before handing any
    // stream to the TLS acceptor, and the dashboard's Security view reads
    // snapshots and drives whitelist / blacklist / unblock actions against the
    // same `Arc`.
    pub addr_guard:     Arc<SteelAddressGuard>,
    // A tighter limiter for sensitive URL prefixes (login forms, admin login),
    // consulted by the HTTPS handler once the request line has been parsed. A
    // block returns 429 without reaching the application handler and without
    // counting against, or affecting, `addr_guard`'s state for that address.
    pub auth_guard:     Arc<SteelAddressGuard>,
    // Authorised public keys for the signed-admin-login flow, parsed from the
    // primary vhost's `admin_keys` config block. Empty when the feature is not
    // configured.
    pub admin_keys:     Arc<Vec<AdminKey>>,
    pub nonce_tracker:  Arc<Mutex<NonceTracker>>,
    // Injected into every admin-served page's `<head>`, copied from the primary
    // vhost's `head_injection_url` at start-up. `None` leaves the default head
    // untouched.
    pub head_injection_url: Arc<Option<String>>,
}

impl AdminState {
    /// Builds a fresh admin state around a loaded wallet.
    ///
    /// The state starts **sealed**: the wallet has been read from
    /// disk, but no passphrase has unwrapped a master key out of it
    /// yet. Call [`AdminState::unseal`] with an admin's passphrase to
    /// install the key. When the operator has already supplied a
    /// passphrase before the listeners bind -- via `STEEL_ADMIN_PASS`
    /// or the shell's `unseal` command -- the caller passes the
    /// recovered key here and the state starts unsealed.
    pub fn new(
        wallet:             Arc<RwLock<Wallet>>,
        wallet_path:        PathBuf,
        master_key:         Option<Vec<u8>>,
        db_count:           usize,
        alerter:            Option<Alerter>,
        traffic:            Arc<TrafficRecorder>,
        host_sampler:       Arc<HostSampler>,
        addr_guard:         Arc<SteelAddressGuard>,
        auth_guard:         Arc<SteelAddressGuard>,
        admin_keys:         Vec<AdminKey>,
        head_injection_url: Option<String>,
    )
        -> Outcome<Self>
    {
        // The session key is random, not derived from the master key:
        // a sealed Steel has no master key, yet it must still issue
        // and validate the session cookie of the admin who is on
        // their way to the unseal page.
        let mut session_key = [0u8; SESSION_KEY_LEN];
        Rand::fill_u8(&mut session_key);
        let session_enc = res!(
            EncryptionScheme::new_aes_256_gcm_with_key(&session_key));
        // The replay-window matches the signed-login freshness
        // window -- an envelope older than the window would fail
        // the verify_fresh check anyway, so there is no point
        // tracking nonces past that horizon.
        let tracker = NonceTracker::new(Duration::from_secs(
            crate::srv::admin::signed_login::SIGNED_LOGIN_FRESHNESS_SECS,
        ));
        let sealed = master_key.is_none();
        Ok(Self {
            wallet,
            wallet_path,
            master_key:         Arc::new(RwLock::new(master_key)),
            sealed:             Arc::new(AtomicBool::new(sealed)),
            unseal_notify:      Arc::new(Notify::new()),
            db_count,
            alerter,
            session_enc,
            traffic,
            host_sampler,
            addr_guard,
            auth_guard,
            admin_keys:         Arc::new(admin_keys),
            nonce_tracker:      Arc::new(Mutex::new(tracker)),
            head_injection_url: Arc::new(head_injection_url),
        })
    }

    /// True while no master key is known, so the databases are shut and
    /// DB-backed routes must refuse.
    pub fn is_sealed(&self) -> bool {
        self.sealed.load(Ordering::Acquire)
    }

    pub fn db_count(&self) -> usize {
        self.db_count
    }

    pub fn alerter(&self) -> Option<&Alerter> {
        self.alerter.as_ref()
    }

    /// Is the seal actually holding something shut -- no master key, and at
    /// least one database that needs it?
    ///
    /// Distinct from `is_sealed` because a deployment of static sites, redirects
    /// and proxy routes has no database at all, and for it the seal is
    /// inconsequential: nothing is locked, nothing is waiting, and there is no
    /// reason to tell an operator otherwise.
    pub fn seal_withholds_data(&self) -> bool {
        self.is_sealed() && self.db_count > 0
    }

    /// Fails with a `Sealed` tag while the process is sealed. Callers that merely
    /// want the seal state should ask `is_sealed` rather than probing this for an
    /// error.
    pub fn master_key(&self) -> Outcome<Vec<u8>> {
        // Always the message-carrying form of the lock macros on this
        // field: the bare form formats the locked value into the error
        // message with `{:?}`, which for a master key would print the
        // secret into the log.
        let guard = lock_read!(self.master_key, "Reading the wallet master key.");
        match &*guard {
            Some(k) => Ok(k.clone()),
            None => Err(err!(
                "Steel is sealed: no wallet master key is loaded. An admin \
                must unseal before this operation can proceed.";
                Sealed, Unauthorised)),
        }
    }

    /// Unwraps the wallet master key with an admin's passphrase and installs it,
    /// lifting the seal.
    ///
    /// Authenticates against the wallet **only** -- the wallet file is readable
    /// while sealed, which is precisely what makes a web unseal page possible
    /// without a database behind it.
    ///
    /// Unsealing an already-unsealed process re-verifies the passphrase and
    /// leaves the key in place, so a second admin logging in cannot swap the key
    /// out from under the running databases.
    pub fn unseal(&self, passphrase: &[u8]) -> Outcome<Unsealed> {
        let unlocked = {
            let wallet = lock_read!(self.wallet, "Reading the wallet to unseal.");
            res!(wallet.unlock(passphrase))
        };
        let name = unlocked.admin_name.clone();
        let scopes = unlocked.admin_scopes.clone();

        let mut guard = lock_write!(self.master_key,
            "Installing the wallet master key.");
        let lifted = guard.is_none();
        if lifted {
            *guard = Some(unlocked.master_key.expose_secret().clone());
            self.sealed.store(false, Ordering::Release);
            drop(guard);
            // Wake the database starter. `notify_waiters` only reaches
            // tasks already waiting, which is the case here: the
            // starter task is spawned before the listeners accept.
            self.unseal_notify.notify_waiters();
            info!("Steel unsealed by admin '{}'; starting databases.", name);
        }
        Ok(Unsealed { name, scopes, lifted })
    }

    /// Waits until an admin installs the master key, returning immediately when
    /// the process is already unsealed.
    pub async fn await_master_key(&self) -> Outcome<Vec<u8>> {
        loop {
            if !self.is_sealed() {
                return self.master_key();
            }
            // Register interest *before* re-testing the flag, so an
            // unseal landing between the test and the wait cannot be
            // missed.
            let notified = self.unseal_notify.notified();
            if !self.is_sealed() {
                return self.master_key();
            }
            notified.await;
        }
    }
}
