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
        traffic::TrafficRecorder,
    },
    alert::Alerter,
    cfg::AdminKey,
    fleet::Fleet,
    health::{
        F_DISK_PCT,
        F_MAIL_DOWN,
        F_SEALED_DBS,
        HealthBody,
        HealthStamp,
        RollingCounter,
    },
    mail::ListenerTally,
};

use oxedyne_fe2o3_core::{
    prelude::*,
    rand::Rand,
};
use oxedyne_fe2o3_crypto::{
    enc::EncryptionScheme,
    keystore::Wallet,
};
use oxedyne_fe2o3_net::guard::nonce::NonceTracker;

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
    time::{
        Duration,
        Instant,
        SystemTime,
    },
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
    // When the process began serving, so the health body can report uptime.
    pub started:        Instant,
    // Whether the address guard reported armed by its in-process self-test at
    // start-up, surfaced as `guard_selftest` in the health body so a box proves
    // its own admission control is live without an external synthetic probe.
    pub guard_selftest: bool,
    // Rolling one-minute counts of `429`s emitted and connections dropped at
    // admission, surfaced as `r429_1m` / `dropped_1m`. The accept loop and the
    // 429 site increment these; the health route reads them.
    pub r429:           Arc<RollingCounter>,
    pub dropped:        Arc<RollingCounter>,
    // Mail listeners asked for against those bound, surfaced as `mail_down`. The
    // mail spawner in `Server::start` counts into it.
    pub mail:           Arc<ListenerTally>,
    // What this host's watcher saw of each peer, drawn by `/admin/fleet`. The
    // watcher writes the same `Arc`; an empty fleet on a host that watches nobody.
    pub fleet:          Arc<Fleet>,
    // Job stamps whose ages the health body reports, vetted at start-up by
    // `ServerConfig::get_health_stamps`; empty on a host that names none.
    pub health_stamps:  Arc<Vec<HealthStamp>>,
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
        // The replay window is the signed-login freshness window. The
        // tracker holds each nonce until that window after the later
        // of the envelope's stamp and its first showing: an envelope
        // stamped ahead of the clock stays fresh until its stamp plus
        // the window, so a window from the first showing alone would
        // let it replay.
        let tracker = NonceTracker::new(Duration::from_secs(
            crate::srv::admin::signed_login::SIGNED_LOGIN_FRESHNESS_SECS,
        ));
        let sealed = master_key.is_none();
        // Run the guard's in-process self-test once, at construction, so the
        // health body can report armed/not without an external probe a live
        // guard would blacklist.
        let guard_selftest = addr_guard.self_test();
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
            started:            Instant::now(),
            guard_selftest,
            r429:               RollingCounter::new_shared(),
            dropped:            RollingCounter::new_shared(),
            mail:               ListenerTally::new_shared(),
            fleet:              Fleet::new_shared(String::new(), None),
            health_stamps:      Arc::new(Vec::new()),
        })
    }

    /// Hands the dashboard the rings the watcher writes, and this host's name for
    /// the page's own row. A state built without one has an empty fleet, as a host
    /// that watches nobody does.
    pub fn with_fleet(mut self, fleet: Arc<Fleet>) -> Self {
        self.fleet = fleet;
        self
    }

    pub fn with_health_stamps(mut self, stamps: Vec<HealthStamp>) -> Self {
        self.health_stamps = Arc::new(stamps);
        self
    }

    /// This host's health body: what the health route serves, and what the Fleet
    /// page draws in this host's own row.
    ///
    /// `assemble` makes the original fields; the ones the Fleet view added are
    /// set here, from state this struct holds. Each is absent rather than zero
    /// when it was never read, so no watcher mistakes a missing figure for a
    /// good one. The stamp ages are the exception by design: each is read at
    /// this call, and a stamp that cannot be read reports as never written.
    pub fn health_body(&self) -> HealthBody {
        let host = self.host_sampler.health_metrics().ok().flatten();
        let mut b = HealthBody::assemble(
            host,
            self.addr_guard.live_conns(),
            self.r429.last(60),
            self.dropped.last(60),
            self.guard_selftest,
            self.started.elapsed().as_secs(),
            self.is_sealed(),
        );
        let sealed_dbs = if self.seal_withholds_data() { self.db_count } else { 0 };
        b.set(F_SEALED_DBS, sealed_dbs as i64);
        if let Some(pct) = self.host_sampler.disk_pct().ok().flatten() {
            b.set(F_DISK_PCT, pct);
        }
        if let Some(n) = self.mail.down() {
            b.set(F_MAIL_DOWN, n as i64);
        }
        for r in self.host_sampler.residents().unwrap_or_default() {
            b.set_resident(&r);
        }
        b.set_stamps(&self.health_stamps, SystemTime::now());
        b
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


#[cfg(test)]
mod tests {
    use super::*;

    use crate::srv::health::BUILTIN_FIELDS;

    /// Every field the served body carries of its own accord is on the reserved list, so a stamp
    /// can take none of them, and the stamps the state was handed ride beside them, a stamp that
    /// is not there reading as never written.
    #[test]
    fn the_served_body_is_the_builtin_list_and_the_stamps() -> Outcome<()> {
        let state = res!(AdminState::new(
            Arc::new(RwLock::new(Wallet::default())),
            PathBuf::from("./wallet.jdat"),
            Some([0u8; 32].to_vec()),
            1,
            None,
            TrafficRecorder::new_shared(0),
            HostSampler::new_shared(),
            res!(crate::srv::admin::guard::new_shared()),
            res!(crate::srv::admin::guard::new_shared()),
            Vec::new(),
            None,
        ));
        let plain = state.health_body();
        for k in plain.fields.keys() {
            assert!(BUILTIN_FIELDS.contains(&k.as_str()),
                "the served body carries '{}', which BUILTIN_FIELDS does not reserve", k);
        }

        let state = state.with_health_stamps(vec![HealthStamp {
            field:  fmt!("forge_state_age_s"),
            path:   PathBuf::from("/nonexistent/fe2o3_steel/stamp/state.ok"),
        }]);
        let body = state.health_body();
        assert!(body.get("forge_state_age_s").unwrap_or(0) > 1_000_000_000,
            "a missing stamp must read as never written, a very large age");
        assert_eq!(body.fields.len(), plain.fields.len() + 1,
            "the stamp rides beside the built-in fields and displaces none");
        Ok(())
    }
}
