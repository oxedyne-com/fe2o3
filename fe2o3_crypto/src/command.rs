//! Authenticated command envelopes.
//!
//! A [`SignedCommand`] bundles a command name, a typed argument
//! payload, a timestamp, a fresh nonce and a signature into a single
//! self-describing unit that can ride any transport -- HTTP POST,
//! WebSocket frame, UDP datagram, filesystem drop. Verifying the
//! envelope requires only the signer's public key plus (for
//! freshness) a clock and a nonce tracker.
//!
//! # Design points
//!
//! - *Transport-agnostic*. The envelope does not assume HTTP or
//!   WebSocket or Shield; applications serialise it via JDAT and
//!   send the bytes over whatever pipe fits.
//! - *Replay protection lives outside*. The envelope carries a
//!   timestamp and a nonce; a stateful nonce tracker that rejects
//!   re-used `(signer_id, nonce)` pairs within a time window is a
//!   companion primitive (see `fe2o3_shield::replay`). This module
//!   deliberately owns only the envelope and the signature.
//! - *Signature scheme is named*. Like [`super::credential::SignedCredential`],
//!   the issuer's scheme name travels with the envelope so a
//!   verifier reconstructs the right algorithm from the wire bytes.
//! - *Arguments are [`Dat`]*. Any serialisable typed payload is
//!   acceptable; the envelope canonicalises them through the JDAT
//!   binary encoding for signing, so any two peers agree on the
//!   signed bytes without coordinating a schema.
//!
//! # Canonical byte encoding
//!
//! Produced by [`SignedCommand::signed_bytes`] in order:
//!
//! ```text
//! [u8 version = 1]
//! [u32 LE scheme_len][scheme_bytes]
//! [u32 LE signer_id_len][signer_id]
//! [u32 LE cmd_len][cmd_bytes]
//! [u32 LE args_len][args_bytes]
//! [u64 LE timestamp]
//! [32 bytes nonce]
//! ```
//!
//! `args_bytes` is the output of [`Dat::as_bytes`] for the envelope's
//! [`args`] field, so the signed encoding is insensitive to map
//! iteration order and other JDAT ambiguities.

use crate::sign::SignatureScheme;

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_iop_crypto::{
    keys::KeyManager,
    sign::Signer,
};
use oxedyne_fe2o3_jdat::prelude::*;

use std::{
    str::FromStr,
    time::{
        Duration,
        SystemTime,
        UNIX_EPOCH,
    },
};

use rand_core::{
    OsRng,
    RngCore,
};


/// Current canonical-byte-encoding version. Bumped if the field set
/// or layout changes in a way that would alter the signed bytes.
pub const COMMAND_VERSION: u8 = 1;

/// Nonce length in bytes. Fixed at 32 to match the Kademlia /
/// RecordId / credential-identifier byte widths already in use
/// across the distributed Hematite stack.
pub const COMMAND_NONCE_LEN: usize = 32;


/// A signed command envelope.
///
/// Construct via [`SignedCommand::sign`] on the originator side and
/// verify via [`SignedCommand::verify`] (or
/// [`SignedCommand::verify_fresh`] for clock-bounded freshness) on
/// the receiver side. A separate nonce tracker is responsible for
/// rejecting duplicates within the freshness window.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SignedCommand {
    /// Opaque identifier of the signer. The verifier uses this to
    /// look up the matching public key; the envelope itself does not
    /// interpret the bytes.
    pub signer_id:  Vec<u8>,
    /// Signature scheme name, matching the `Debug` output of
    /// [`SignatureScheme`] (`"Ed25519"`, `"Dilithium2"`,
    /// `"Dilithium2_fe2o3"`).
    pub scheme:     String,
    /// Application-defined command name. The envelope treats this as
    /// opaque UTF-8; verb-based authorisation and dispatch are the
    /// caller's job.
    pub cmd:        String,
    /// Typed argument payload. The envelope signs the JDAT binary
    /// encoding of this value.
    pub args:       Dat,
    /// Unix seconds since epoch when the command was signed.
    pub timestamp:  u64,
    /// Fresh random nonce per command. The companion replay-window
    /// tracker rejects duplicate `(signer_id, nonce)` pairs.
    pub nonce:      [u8; COMMAND_NONCE_LEN],
    /// Signature over [`Self::signed_bytes`].
    pub sig:        Vec<u8>,
}

impl SignedCommand {

    /// Signs a fresh command envelope. Stamps `timestamp` from the
    /// system clock and draws `nonce` from `OsRng`. `signer_scheme`
    /// must carry both the signer's public and secret keys.
    pub fn sign(
        signer_id:      Vec<u8>,
        cmd:            impl Into<String>,
        args:           Dat,
        signer_scheme:  &SignatureScheme,
    )
        -> Outcome<Self>
    {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let mut nonce = [0u8; COMMAND_NONCE_LEN];
        OsRng.fill_bytes(&mut nonce);
        Self::sign_with(
            signer_id,
            cmd.into(),
            args,
            signer_scheme,
            timestamp,
            nonce,
        )
    }

    /// As [`Self::sign`] but with caller-supplied `timestamp` and
    /// `nonce`. Useful for deterministic tests and for callers whose
    /// entropy or clock source differs from the defaults.
    pub fn sign_with(
        signer_id:      Vec<u8>,
        cmd:            String,
        args:           Dat,
        signer_scheme:  &SignatureScheme,
        timestamp:      u64,
        nonce:          [u8; COMMAND_NONCE_LEN],
    )
        -> Outcome<Self>
    {
        let scheme = fmt!("{:?}", signer_scheme);
        let mut env = Self {
            signer_id,
            scheme,
            cmd,
            args,
            timestamp,
            nonce,
            sig: Vec::new(),
        };
        let bytes = res!(env.signed_bytes());
        env.sig = res!(signer_scheme.sign(&bytes));
        Ok(env)
    }

    /// Canonical byte encoding the signature covers. See the
    /// module-level documentation for the exact layout.
    pub fn signed_bytes(&self) -> Outcome<Vec<u8>> {
        let scheme_bytes = self.scheme.as_bytes();
        let cmd_bytes = self.cmd.as_bytes();
        let args_bytes = res!(self.args.as_bytes());
        let cap = 1
            + 4 + scheme_bytes.len()
            + 4 + self.signer_id.len()
            + 4 + cmd_bytes.len()
            + 4 + args_bytes.len()
            + 8
            + COMMAND_NONCE_LEN;
        let mut out = Vec::with_capacity(cap);
        out.push(COMMAND_VERSION);
        out.extend_from_slice(&(scheme_bytes.len() as u32).to_le_bytes());
        out.extend_from_slice(scheme_bytes);
        out.extend_from_slice(&(self.signer_id.len() as u32).to_le_bytes());
        out.extend_from_slice(&self.signer_id);
        out.extend_from_slice(&(cmd_bytes.len() as u32).to_le_bytes());
        out.extend_from_slice(cmd_bytes);
        out.extend_from_slice(&(args_bytes.len() as u32).to_le_bytes());
        out.extend_from_slice(&args_bytes);
        out.extend_from_slice(&self.timestamp.to_le_bytes());
        out.extend_from_slice(&self.nonce);
        Ok(out)
    }

    /// Verifies the signature against `signer_pk`. Does not check
    /// freshness -- use [`Self::verify_fresh`] when the caller wants
    /// a clock-bounded freshness guarantee.
    pub fn verify(&self, signer_pk: &[u8]) -> Outcome<()> {
        let scheme = res!(SignatureScheme::from_str(&self.scheme));
        let scheme = res!(scheme.clone_with_keys(Some(signer_pk), None));
        let bytes = res!(self.signed_bytes());
        let ok = res!(scheme.verify(&bytes, &self.sig));
        if !ok {
            return Err(err!(
                "SignedCommand signature did not verify under the \
                supplied signer public key (scheme: {}).", self.scheme;
                Invalid, Security, Mismatch));
        }
        Ok(())
    }

    /// Verifies the signature and confirms `self.timestamp` falls
    /// within `window` of the current system time (in either
    /// direction). The caller is responsible for checking the nonce
    /// against its replay-window tracker separately.
    pub fn verify_fresh(
        &self,
        signer_pk:  &[u8],
        window:     Duration,
    )
        -> Outcome<()>
    {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        self.verify_fresh_at(signer_pk, now, window)
    }

    /// As [`Self::verify_fresh`] but against a caller-supplied `now`
    /// in unix seconds.
    pub fn verify_fresh_at(
        &self,
        signer_pk:  &[u8],
        now:        u64,
        window:     Duration,
    )
        -> Outcome<()>
    {
        let window_secs = window.as_secs();
        let diff = if now >= self.timestamp {
            now - self.timestamp
        } else {
            self.timestamp - now
        };
        if diff > window_secs {
            return Err(err!(
                "SignedCommand timestamp {} is outside the {} s \
                freshness window around now = {}.",
                self.timestamp, window_secs, now;
                Invalid, Security, Order));
        }
        self.verify(signer_pk)
    }
}


impl ToDat for SignedCommand {
    fn to_dat(&self) -> Outcome<Dat> {
        let mut m = DaticleMap::new();
        m.insert(dat!("signer_id"), Dat::bytdat(self.signer_id.clone()));
        m.insert(dat!("scheme"),    dat!(self.scheme.clone()));
        m.insert(dat!("cmd"),       dat!(self.cmd.clone()));
        m.insert(dat!("args"),      self.args.clone());
        m.insert(dat!("timestamp"), dat!(self.timestamp));
        m.insert(dat!("nonce"),     Dat::bytdat(self.nonce.to_vec()));
        m.insert(dat!("sig"),       Dat::bytdat(self.sig.clone()));
        Ok(Dat::Map(m))
    }
}

impl FromDat for SignedCommand {
    fn from_dat(mut dat: Dat) -> Outcome<Self> {
        let signer_id = try_extract_dat!(
            res!(dat.map_remove_must(&dat!("signer_id"))),
            BU8, BU16, BU32, BU64,
        );
        let scheme = try_extract_dat!(
            res!(dat.map_remove_must(&dat!("scheme"))),
            Str,
        );
        let cmd = try_extract_dat!(
            res!(dat.map_remove_must(&dat!("cmd"))),
            Str,
        );
        let args = res!(dat.map_remove_must(&dat!("args")));
        let timestamp = match res!(dat.map_remove_must(&dat!("timestamp"))) {
            Dat::U64(n) => n,
            Dat::U32(n) => n as u64,
            other => return Err(err!(
                "SignedCommand 'timestamp' must be u64, got {:?}.",
                other.kind();
                Invalid, Input, Mismatch)),
        };
        let nonce_bytes = try_extract_dat!(
            res!(dat.map_remove_must(&dat!("nonce"))),
            BU8, BU16, BU32, BU64,
        );
        if nonce_bytes.len() != COMMAND_NONCE_LEN {
            return Err(err!(
                "SignedCommand nonce length is {}, expected {}.",
                nonce_bytes.len(), COMMAND_NONCE_LEN;
                Invalid, Input, Size));
        }
        let mut nonce = [0u8; COMMAND_NONCE_LEN];
        nonce.copy_from_slice(&nonce_bytes);
        let sig = try_extract_dat!(
            res!(dat.map_remove_must(&dat!("sig"))),
            BU8, BU16, BU32, BU64,
        );
        Ok(Self {
            signer_id,
            scheme,
            cmd,
            args,
            timestamp,
            nonce,
            sig,
        })
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    fn ed25519_scheme() -> SignatureScheme {
        SignatureScheme::new_ed25519()
    }

    fn signer_pk(scheme: &SignatureScheme) -> Vec<u8> {
        scheme.get_public_key().unwrap().unwrap().to_vec()
    }

    #[test]
    fn sign_and_verify_round_trip() -> Outcome<()> {
        let scheme = ed25519_scheme();
        let pk = signer_pk(&scheme);
        let env = res!(SignedCommand::sign(
            vec![0x01; 32],
            "admin_login",
            dat!("unlock-please"),
            &scheme,
        ));
        res!(env.verify(&pk));
        Ok(())
    }

    #[test]
    fn tampered_cmd_fails_verify() -> Outcome<()> {
        let scheme = ed25519_scheme();
        let pk = signer_pk(&scheme);
        let mut env = res!(SignedCommand::sign(
            vec![0x02; 32],
            "reload",
            Dat::Empty,
            &scheme,
        ));
        env.cmd = "shutdown".to_string();
        assert!(env.verify(&pk).is_err());
        Ok(())
    }

    #[test]
    fn tampered_nonce_fails_verify() -> Outcome<()> {
        let scheme = ed25519_scheme();
        let pk = signer_pk(&scheme);
        let mut env = res!(SignedCommand::sign(
            vec![0x03; 32],
            "reload",
            Dat::Empty,
            &scheme,
        ));
        env.nonce[0] ^= 0xff;
        assert!(env.verify(&pk).is_err());
        Ok(())
    }

    #[test]
    fn tampered_timestamp_fails_verify() -> Outcome<()> {
        let scheme = ed25519_scheme();
        let pk = signer_pk(&scheme);
        let mut env = res!(SignedCommand::sign_with(
            vec![0x04; 32],
            "reload".to_string(),
            Dat::Empty,
            &scheme,
            1_000_000_000,
            [0x11; COMMAND_NONCE_LEN],
        ));
        env.timestamp = 1_000_000_001;
        assert!(env.verify(&pk).is_err());
        Ok(())
    }

    #[test]
    fn tampered_args_fails_verify() -> Outcome<()> {
        let scheme = ed25519_scheme();
        let pk = signer_pk(&scheme);
        let env = res!(SignedCommand::sign(
            vec![0x05; 32],
            "store",
            dat!("original"),
            &scheme,
        ));
        // Reconstruct with different args but same signature.
        let forged = SignedCommand {
            args: dat!("forged"),
            ..env
        };
        assert!(forged.verify(&pk).is_err());
        Ok(())
    }

    #[test]
    fn wrong_signer_pk_fails_verify() -> Outcome<()> {
        let signer = ed25519_scheme();
        let other = ed25519_scheme();
        let env = res!(SignedCommand::sign(
            vec![0x06; 32],
            "reload",
            Dat::Empty,
            &signer,
        ));
        assert!(env.verify(&signer_pk(&other)).is_err());
        Ok(())
    }

    #[test]
    fn freshness_window_accepts_in_window() -> Outcome<()> {
        let scheme = ed25519_scheme();
        let pk = signer_pk(&scheme);
        let env = res!(SignedCommand::sign_with(
            vec![0x07; 32],
            "reload".to_string(),
            Dat::Empty,
            &scheme,
            1_000_000_000,
            [0x22; COMMAND_NONCE_LEN],
        ));
        // Slightly past timestamp, well inside a 60s window.
        res!(env.verify_fresh_at(&pk, 1_000_000_030, Duration::from_secs(60)));
        // Slightly before timestamp (clock skew the other way).
        res!(env.verify_fresh_at(&pk, 999_999_970, Duration::from_secs(60)));
        Ok(())
    }

    #[test]
    fn freshness_window_rejects_stale() -> Outcome<()> {
        let scheme = ed25519_scheme();
        let pk = signer_pk(&scheme);
        let env = res!(SignedCommand::sign_with(
            vec![0x08; 32],
            "reload".to_string(),
            Dat::Empty,
            &scheme,
            1_000_000_000,
            [0x33; COMMAND_NONCE_LEN],
        ));
        // Two minutes past timestamp, 60s window.
        assert!(env.verify_fresh_at(
            &pk, 1_000_000_120, Duration::from_secs(60),
        ).is_err());
        Ok(())
    }

    #[test]
    fn two_successive_signs_have_distinct_nonces() -> Outcome<()> {
        let scheme = ed25519_scheme();
        let a = res!(SignedCommand::sign(
            vec![0x09; 32], "ping", Dat::Empty, &scheme,
        ));
        let b = res!(SignedCommand::sign(
            vec![0x09; 32], "ping", Dat::Empty, &scheme,
        ));
        assert_ne!(a.nonce, b.nonce,
            "two successive sign() calls produced the same nonce -- \
            OsRng is not behaving");
        Ok(())
    }

    #[test]
    fn jdat_round_trip_preserves_signature() -> Outcome<()> {
        let scheme = ed25519_scheme();
        let pk = signer_pk(&scheme);
        let env = res!(SignedCommand::sign(
            vec![0x0a; 32],
            "store",
            mapdat!{
                "k" => "val",
                "n" => 42u64,
            },
            &scheme,
        ));
        let dat = res!(env.to_dat());
        let back = res!(SignedCommand::from_dat(dat));
        assert_eq!(back, env);
        res!(back.verify(&pk));
        Ok(())
    }

    #[test]
    fn from_dat_rejects_wrong_nonce_length() {
        let mut m = DaticleMap::new();
        m.insert(dat!("signer_id"), Dat::bytdat(vec![0u8; 4]));
        m.insert(dat!("scheme"),    dat!("Ed25519"));
        m.insert(dat!("cmd"),       dat!("x"));
        m.insert(dat!("args"),      Dat::Empty);
        m.insert(dat!("timestamp"), dat!(0u64));
        m.insert(dat!("nonce"),     Dat::bytdat(vec![0u8; 16])); // wrong
        m.insert(dat!("sig"),       Dat::bytdat(Vec::new()));
        assert!(SignedCommand::from_dat(Dat::Map(m)).is_err());
    }

    #[test]
    fn version_byte_in_signed_bytes() -> Outcome<()> {
        let scheme = ed25519_scheme();
        let env = res!(SignedCommand::sign_with(
            vec![0x0b; 32],
            "x".to_string(),
            Dat::Empty,
            &scheme,
            0,
            [0u8; COMMAND_NONCE_LEN],
        ));
        let bytes = res!(env.signed_bytes());
        assert_eq!(bytes[0], COMMAND_VERSION);
        Ok(())
    }
}
