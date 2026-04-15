//! Runtime state shared by every dashboard request.
//!
//! Built once at Steel start-up from the unlocked wallet and threaded
//! through the admin handler. Holds the pieces of state that both
//! dashboard auth and session decoding need:
//!
//! - A shared handle to the [`Wallet`] so login calls `unlock` against
//!   the same admin list the CLI sees, and admin management from the
//!   dashboard mutates the same file on disk.
//! - An [`EncryptionScheme`] pre-keyed with the 32-byte dashboard
//!   session key, so session encode/decode does not re-derive on
//!   every request.
//!
//! The session key is derived from the wallet master key via a
//! domain-separated SHA3-256, so it is cryptographically distinct
//! from the ozone encryption key even though both ultimately trace
//! back to the same wallet unlock.

use crate::srv::admin::{
    host_sampler::HostSampler,
    traffic::TrafficRecorder,
};

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_crypto::{
    enc::EncryptionScheme,
    keys::Wallet,
};
use oxedyne_fe2o3_hash::hash::HashScheme;
use oxedyne_fe2o3_iop_hash::api::Hasher;

use std::{
    path::PathBuf,
    sync::{
        Arc,
        RwLock,
    },
};

/// Domain-separation string mixed into the master key when deriving
/// the dashboard session key. Must never be reused for any other
/// derivation. Bumping the `v1` suffix rotates every outstanding
/// session cookie on the next restart.
pub const SESSION_KEY_DERIVATION_INFO: &[u8] =
    b"steel.admin.dashboard.session.v1";

/// Shared dashboard runtime state.
///
/// Cheaply cloneable: the wallet is behind an `Arc<RwLock<_>>` and
/// the encryption scheme clones its key material.
#[derive(Clone, Debug)]
pub struct AdminState {
    /// Shared wallet handle. Login reads from this; admin management
    /// (task #11) writes through it.
    pub wallet:         Arc<RwLock<Wallet>>,
    /// On-disk path the wallet is persisted to. Held in the admin
    /// state so the dashboard's admin-management UI can call
    /// `Wallet::save` without depending on `app::constant`.
    pub wallet_path:    PathBuf,
    /// Wallet master key recovered at startup from the operator's
    /// unlock passphrase. The dashboard's admin-management UI
    /// needs it to call `Wallet::enrol` (which wraps the master
    /// key under each new admin's password). Stored here so the
    /// operator does not have to re-type their passphrase on
    /// every dashboard mutation. The key is held in clear in
    /// process memory, in line with the overall wallet-v2
    /// design where the operator types the passphrase once at
    /// boot and the unlocked secret lives in RAM until the
    /// process restarts.
    pub master_key:     Vec<u8>,
    /// AES-256-GCM pre-keyed with the derived session key. Used by
    /// [`session`](super::session) to encrypt and decrypt session
    /// cookies.
    pub session_enc:    EncryptionScheme,
    /// Shared traffic recorder. The dashboard reads from this when
    /// rendering the `/admin/traffic` view; the request pipeline
    /// in `srv/https.rs` writes to it on every completed response.
    /// Both sides hold the same `Arc`, so the dashboard sees live
    /// data without any per-vhost coordination.
    pub traffic:        Arc<TrafficRecorder>,
    /// Shared host-resource sampler. A background task in
    /// `Server::start` calls `HostSampler::sample_now` on a fixed
    /// interval; the dashboard reads from the same `Arc` when
    /// drawing the host resource strip.
    pub host_sampler:   Arc<HostSampler>,
}

impl AdminState {
    /// Build a fresh admin state from an unlocked wallet, its
    /// on-disk path, the recovered master key, the shared traffic
    /// recorder, and the shared host sampler. Called from the TUI
    /// startup path once the wallet has been unlocked, before the
    /// server listeners bind.
    pub fn new(
        wallet:       Arc<RwLock<Wallet>>,
        wallet_path:  PathBuf,
        master_key:   &[u8],
        traffic:      Arc<TrafficRecorder>,
        host_sampler: Arc<HostSampler>,
    )
        -> Outcome<Self>
    {
        let session_key = res!(derive_session_key(master_key));
        let session_enc = res!(
            EncryptionScheme::new_aes_256_gcm_with_key(&session_key));
        Ok(Self {
            wallet,
            wallet_path,
            master_key: master_key.to_vec(),
            session_enc,
            traffic,
            host_sampler,
        })
    }
}

/// Derive the 32-byte dashboard session key from the wallet master
/// key via `SHA3-256(master_key || SESSION_KEY_DERIVATION_INFO)`.
///
/// Domain separation is structural: by mixing a fixed info string
/// into the hash input the derived key is unrelated to any other
/// key derived from the same master, even though every derivation
/// is deterministic.
pub fn derive_session_key(master_key: &[u8]) -> Outcome<Vec<u8>> {
    let hasher = HashScheme::new_sha3_256();
    let h = hasher.hash(
        &[master_key, SESSION_KEY_DERIVATION_INFO],
        [0u8; 0],
    );
    let bytes = h.as_vec();
    if bytes.len() != 32 {
        return Err(err!(
            "Expected a 32-byte hash from SHA3-256, got {} bytes.", bytes.len();
            Bug, Mismatch));
    }
    Ok(bytes)
}
