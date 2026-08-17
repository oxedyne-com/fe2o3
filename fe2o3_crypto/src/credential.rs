//! Generic signed credentials: "issuer attests that this public key is bound
//! to this subject for this time range".
//!
//! A [`SignedCredential`] is a self-contained, typed record carrying a
//! signature over a canonical byte encoding of its fields. It is
//! agnostic about what issuers and subjects mean semantically -- a
//! caller is free to decide that a "subject" is a device, a peer, a
//! user, a delegated agent, or anything else with an identifier. The
//! credential format is just "issuer says this binding holds from A to
//! B".
//!
//! # Design points
//!
//! - *Issuer and subject IDs are opaque bytes*. Applications hash whatever
//!   they consider stable (a public key, a name, a URL) into the ID
//!   space of their choice. This module does not impose a hash.
//! - *Signature scheme is named, not typed*. The scheme's registered
//!   name (see [`SignatureScheme`] and its `Debug` impl) is stored in
//!   the credential so a verifier can reconstruct the right algorithm
//!   from the serialised bytes alone.
//! - *Self-signed is a special case*. When `issuer_id == subject_id`,
//!   the credential asserts that the holder of the bound secret key
//!   has declared their own identity. Useful for bootstrap: the very
//!   first credential in a system cannot be signed by anyone else.
//! - *Validity window is inclusive on the lower bound and exclusive on
//!   the upper*. `0` as `valid_to` is a sentinel for "no expiry".
//! - *No at-rest encryption*. A credential's purpose is to be shown;
//!   it carries only public data plus a signature. If you want to
//!   protect the credential's existence (not its contents), encrypt
//!   it at a different layer.
//!
//! # Canonical byte encoding
//!
//! The signed bytes are produced by [`SignedCredential::signed_bytes`]
//! and consist of, in order:
//!
//! ```text
//! [u8 version = 1]
//! [u32 LE scheme_len][scheme_bytes]
//! [u32 LE subject_id_len][subject_id]
//! [u32 LE subject_pk_len][subject_pk]
//! [u32 LE issuer_id_len][issuer_id]
//! [u64 LE valid_from]
//! [u64 LE valid_to]
//! ```
//!
//! The encoding is length-prefixed rather than delimiter-based so
//! there is no ambiguity for callers that put arbitrary bytes in the
//! id fields. The leading version byte lets a future schema change
//! surface as a verify-fails-loudly rather than a quietly-different
//! hash.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

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
        SystemTime,
        UNIX_EPOCH,
    },
};


// Bump the version if the field set or the layout changes in a way that would
// alter the signed bytes.
pub const CREDENTIAL_VERSION: u8 = 1;


/// A signed attestation that `issuer_id` vouches for `subject_pk` being bound
/// to `subject_id` for a stated time range.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SignedCredential {
    // Both identifiers are opaque: a hash of a name, a hash of a key, an
    // assigned serial, whatever the caller decides.  They are equal in a
    // self-signed credential.
    pub subject_id:     Vec<u8>,
    // Zero length is legal and binds an identifier alone, with no key.
    pub subject_pk:     Vec<u8>,
    pub issuer_id:      Vec<u8>,
    pub scheme:         String, // SignatureScheme's Debug string
    pub valid_from:     u64,    // seconds since epoch, inclusive; 0 for no start
    pub valid_to:       u64,    // seconds since epoch, exclusive; 0 for no expiry
    pub sig:            Vec<u8>,
}

impl SignedCredential {

    /// A self-signed credential: the holder of the secret key declares who it
    /// is, and vouches for nothing else.  `subject_scheme` must carry both keys.
    pub fn self_sign(
        subject_id:     Vec<u8>,
        subject_scheme: &SignatureScheme,
        valid_from:     u64,
        valid_to:       u64,
    )
        -> Outcome<Self>
    {
        let subject_pk = res!(res!(subject_scheme.get_public_key()).ok_or_else(|| err!(
            "self_sign requires the signature scheme to carry a public key.";
            Missing, Configuration)))
            .to_vec();
        Self::sign(
            subject_id.clone(),
            subject_pk,
            subject_id,
            subject_scheme,
            valid_from,
            valid_to,
        )
    }

    /// A third-party credential: the issuer attests that `subject_pk` belongs
    /// to `subject_id`.
    ///
    /// The subject's public key is passed in rather than derived, so the issuer
    /// never needs the subject's secret key.  `issuer_scheme` must carry both of
    /// the issuer's.
    pub fn sign(
        subject_id:     Vec<u8>,
        subject_pk:     Vec<u8>,
        issuer_id:      Vec<u8>,
        issuer_scheme:  &SignatureScheme,
        valid_from:     u64,
        valid_to:       u64,
    )
        -> Outcome<Self>
    {
        if valid_to != 0 && valid_to <= valid_from {
            return Err(err!(
                "Credential validity window is empty: valid_from {}, \
                valid_to {}.", valid_from, valid_to;
                Invalid, Input, Size));
        }
        let scheme = fmt!("{:?}", issuer_scheme);
        let mut cred = Self {
            subject_id,
            subject_pk,
            issuer_id,
            scheme,
            valid_from,
            valid_to,
            sig:    Vec::new(),
        };
        let bytes = cred.signed_bytes();
        cred.sig = res!(issuer_scheme.sign(&bytes));
        Ok(cred)
    }

    /// The canonical encoding the signature covers; the module header gives the
    /// layout.
    pub fn signed_bytes(&self) -> Vec<u8> {
        let scheme_bytes = self.scheme.as_bytes();
        let cap = 1
            + 4 + scheme_bytes.len()
            + 4 + self.subject_id.len()
            + 4 + self.subject_pk.len()
            + 4 + self.issuer_id.len()
            + 8 + 8;
        let mut out = Vec::with_capacity(cap);
        out.push(CREDENTIAL_VERSION);
        out.extend_from_slice(&(scheme_bytes.len() as u32).to_le_bytes());
        out.extend_from_slice(scheme_bytes);
        out.extend_from_slice(&(self.subject_id.len() as u32).to_le_bytes());
        out.extend_from_slice(&self.subject_id);
        out.extend_from_slice(&(self.subject_pk.len() as u32).to_le_bytes());
        out.extend_from_slice(&self.subject_pk);
        out.extend_from_slice(&(self.issuer_id.len() as u32).to_le_bytes());
        out.extend_from_slice(&self.issuer_id);
        out.extend_from_slice(&self.valid_from.to_le_bytes());
        out.extend_from_slice(&self.valid_to.to_le_bytes());
        out
    }

    /// Verifies the signature and that the credential is in its window now.
    ///
    /// A bad signature, an expired window and an unrecognised scheme each carry
    /// their own error tag, so a caller can tell them apart.
    pub fn verify(&self, issuer_pk: &[u8]) -> Outcome<()> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        self.verify_at(issuer_pk, now)
    }

    /// As [`Self::verify`], against a supplied `now` in seconds.
    pub fn verify_at(&self, issuer_pk: &[u8], now: u64) -> Outcome<()> {
        // Validity window check first -- cheap, fails fast on
        // expired credentials without wasting a signature verify.
        if self.valid_from != 0 && now < self.valid_from {
            return Err(err!(
                "Credential not yet valid: now = {}, valid_from = {}.",
                now, self.valid_from;
                Invalid, Security, Order));
        }
        if self.valid_to != 0 && now >= self.valid_to {
            return Err(err!(
                "Credential expired: now = {}, valid_to = {}.",
                now, self.valid_to;
                Invalid, Security, Order));
        }
        // Reconstruct the scheme from its stored name and clone it
        // with the supplied public key so we can call verify.
        let scheme = res!(SignatureScheme::from_str(&self.scheme));
        let scheme = res!(scheme.clone_with_keys(Some(issuer_pk), None));
        let bytes = self.signed_bytes();
        let ok = res!(scheme.verify(&bytes, &self.sig));
        if !ok {
            return Err(err!(
                "Credential signature did not verify under the supplied \
                issuer public key (scheme: {}).", self.scheme;
                Invalid, Security, Mismatch));
        }
        Ok(())
    }

    /// Is this credential self-signed?
    pub fn is_self_signed(&self) -> bool {
        self.issuer_id == self.subject_id
    }
}


impl ToDat for SignedCredential {
    fn to_dat(&self) -> Outcome<Dat> {
        let mut m = DaticleMap::new();
        m.insert(dat!("subject_id"),    Dat::bytdat(self.subject_id.clone()));
        m.insert(dat!("subject_pk"),    Dat::bytdat(self.subject_pk.clone()));
        m.insert(dat!("issuer_id"),     Dat::bytdat(self.issuer_id.clone()));
        m.insert(dat!("scheme"),        dat!(self.scheme.clone()));
        m.insert(dat!("valid_from"),    dat!(self.valid_from));
        m.insert(dat!("valid_to"),      dat!(self.valid_to));
        m.insert(dat!("sig"),           Dat::bytdat(self.sig.clone()));
        Ok(Dat::Map(m))
    }
}

impl FromDat for SignedCredential {
    fn from_dat(mut dat: Dat) -> Outcome<Self> {
        let subject_id = try_extract_dat!(
            res!(dat.map_remove_must(&dat!("subject_id"))),
            BU8, BU16, BU32, BU64,
        );
        let subject_pk = try_extract_dat!(
            res!(dat.map_remove_must(&dat!("subject_pk"))),
            BU8, BU16, BU32, BU64,
        );
        let issuer_id = try_extract_dat!(
            res!(dat.map_remove_must(&dat!("issuer_id"))),
            BU8, BU16, BU32, BU64,
        );
        let scheme = try_extract_dat!(
            res!(dat.map_remove_must(&dat!("scheme"))),
            Str,
        );
        let valid_from = match res!(dat.map_remove_must(&dat!("valid_from"))) {
            Dat::U64(n) => n,
            Dat::U32(n) => n as u64,
            other => return Err(err!(
                "SignedCredential 'valid_from' must be u64, got {:?}.",
                other.kind();
                Invalid, Input, Mismatch)),
        };
        let valid_to = match res!(dat.map_remove_must(&dat!("valid_to"))) {
            Dat::U64(n) => n,
            Dat::U32(n) => n as u64,
            other => return Err(err!(
                "SignedCredential 'valid_to' must be u64, got {:?}.",
                other.kind();
                Invalid, Input, Mismatch)),
        };
        let sig = try_extract_dat!(
            res!(dat.map_remove_must(&dat!("sig"))),
            BU8, BU16, BU32, BU64,
        );
        Ok(Self {
            subject_id,
            subject_pk,
            issuer_id,
            scheme,
            valid_from,
            valid_to,
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

    fn issuer_pk(scheme: &SignatureScheme) -> Vec<u8> {
        scheme.get_public_key().unwrap().unwrap().to_vec()
    }

    #[test]
    fn self_sign_round_trip_verifies() -> Outcome<()> {
        let scheme = ed25519_scheme();
        let pk = issuer_pk(&scheme);
        let cred = res!(SignedCredential::self_sign(
            vec![0x42; 32],
            &scheme,
            0,
            0,
        ));
        assert!(cred.is_self_signed());
        res!(cred.verify(&pk));
        Ok(())
    }

    #[test]
    fn third_party_sign_round_trip_verifies() -> Outcome<()> {
        let issuer = ed25519_scheme();
        let subject = ed25519_scheme();
        let subject_pk = issuer_pk(&subject);
        let issuer_pk_bytes = issuer_pk(&issuer);
        let cred = res!(SignedCredential::sign(
            vec![0x01; 16],
            subject_pk,
            vec![0x02; 16],
            &issuer,
            0,
            0,
        ));
        assert!(!cred.is_self_signed());
        res!(cred.verify(&issuer_pk_bytes));
        Ok(())
    }

    #[test]
    fn tampered_field_fails_verify() -> Outcome<()> {
        let scheme = ed25519_scheme();
        let pk = issuer_pk(&scheme);
        let mut cred = res!(SignedCredential::self_sign(
            vec![0x55; 32], &scheme, 0, 0,
        ));
        // Flip a bit in subject_pk after signing.
        if !cred.subject_pk.is_empty() {
            cred.subject_pk[0] ^= 0x01;
        }
        assert!(cred.verify(&pk).is_err());
        Ok(())
    }

    #[test]
    fn wrong_issuer_pk_fails_verify() -> Outcome<()> {
        let scheme = ed25519_scheme();
        let other = ed25519_scheme();
        let cred = res!(SignedCredential::self_sign(
            vec![0x66; 32], &scheme, 0, 0,
        ));
        let wrong_pk = issuer_pk(&other);
        assert!(cred.verify(&wrong_pk).is_err());
        Ok(())
    }

    #[test]
    fn validity_window_not_yet_valid() -> Outcome<()> {
        let scheme = ed25519_scheme();
        let pk = issuer_pk(&scheme);
        let cred = res!(SignedCredential::self_sign(
            vec![0x77; 32], &scheme, 2_000_000_000, 0,
        ));
        // "Now" before valid_from.
        assert!(cred.verify_at(&pk, 1_000_000_000).is_err());
        // "Now" at or after valid_from passes.
        res!(cred.verify_at(&pk, 2_000_000_000));
        Ok(())
    }

    #[test]
    fn validity_window_expired() -> Outcome<()> {
        let scheme = ed25519_scheme();
        let pk = issuer_pk(&scheme);
        let cred = res!(SignedCredential::self_sign(
            vec![0x88; 32], &scheme, 0, 2_000_000_000,
        ));
        // "Now" before valid_to passes.
        res!(cred.verify_at(&pk, 1_999_999_999));
        // "Now" at or after valid_to fails (upper bound is exclusive).
        assert!(cred.verify_at(&pk, 2_000_000_000).is_err());
        Ok(())
    }

    #[test]
    fn empty_validity_window_rejected_at_sign_time() -> Outcome<()> {
        let scheme = ed25519_scheme();
        // valid_to <= valid_from (and != 0) is nonsense.
        assert!(SignedCredential::self_sign(
            vec![0x99; 32], &scheme, 100, 50,
        ).is_err());
        assert!(SignedCredential::self_sign(
            vec![0x99; 32], &scheme, 100, 100,
        ).is_err());
        Ok(())
    }

    #[test]
    fn jdat_round_trip_preserves_signature() -> Outcome<()> {
        let scheme = ed25519_scheme();
        let pk = issuer_pk(&scheme);
        let cred = res!(SignedCredential::self_sign(
            vec![0xab; 32], &scheme, 0, 0,
        ));
        let dat = res!(cred.to_dat());
        let back = res!(SignedCredential::from_dat(dat));
        assert_eq!(back, cred);
        res!(back.verify(&pk));
        Ok(())
    }

    #[test]
    fn version_byte_in_signed_bytes() {
        let cred = SignedCredential {
            subject_id: vec![0x01],
            subject_pk: vec![0x02],
            issuer_id:  vec![0x03],
            scheme:     "Ed25519".to_string(),
            valid_from: 0,
            valid_to:   0,
            sig:        Vec::new(),
        };
        let bytes = cred.signed_bytes();
        assert_eq!(bytes[0], CREDENTIAL_VERSION);
    }
}
