//! A read-only check that every live value of an Ozone store reads back whole.
//!
//! Every live user key is read through the ordinary read path, so a chunked value is gathered,
//! rejoined, decrypted and decoded exactly as a caller would have it.  In an encrypted store the
//! GCM tag covers the whole rejoined ciphertext, so a value whose chunks belong to two different
//! writes (a torn value, A3 QA B1 and B2, 2026-10-09) fails to decrypt and is reported here.  In
//! a store without encryption nothing covers a value across its chunks, and a torn value is found
//! only when its bytes no longer decode.
//!
//! [`verify_key`] is the same check for one key, and issues one ordinary read, so a running
//! process may call it on its own open store to find a record its index holds and its bytes no
//! longer back.
//!
//! Opening a store starts its bots, which survey and may settle its files.  Check a `cp -a` copy
//! taken while the store's own process is stopped, never a live store, and never with a second
//! process on the same store.

use crate::{
    prelude::*,
    comm::response::Wait,
};

use oxedyne_fe2o3_iop_db::api::{
    RestSchemesOverride,
    ScanOpts,
};
use oxedyne_fe2o3_jdat::{
    prelude::*,
    id::NumIdDat,
};


/// A live key that did not read back.
#[derive(Clone, Debug)]
pub struct ReadFailure {
    pub key:        Dat,
    pub chunked:    bool,   // stored under a bunch key
    pub damaged:    bool,   // the bytes failed, not the store's answer
    pub error:      String,
}

/// What [`verify_live_set`] found.
#[derive(Clone, Debug, Default)]
pub struct VerifyReport {
    pub keys:       usize,  // live keys the scan emitted
    pub present:    usize,  // read back whole
    pub absent:     usize,  // newest record a deletion
    pub chunked:    usize,  // stored under a bunch key, read back or not
    pub failures:   Vec<ReadFailure>,
}

impl VerifyReport {

    /// Did every live key read back?
    pub fn clean(&self) -> bool { self.failures.is_empty() }

    /// A multi-line summary for a console, naming each key that failed.  `max_key` caps the
    /// characters of each key shown.
    pub fn summary(&self, max_key: usize) -> String {
        let mut s = fmt!("Read check {}:\n", if self.clean() { "CLEAN" } else { "FAILED" });
        s.push_str(&fmt!("  live keys      : {}\n", self.keys));
        s.push_str(&fmt!("  read whole     : {}\n", self.present));
        s.push_str(&fmt!("  deleted        : {}\n", self.absent));
        s.push_str(&fmt!("  chunked        : {}\n", self.chunked));
        s.push_str(&fmt!("  failed to read : {}\n", self.failures.len()));
        for f in &self.failures {
            let k: String = fmt!("{:?}", f.key).chars().take(max_key).collect();
            s.push_str(&fmt!("  FAILED{}{} {}\n      {}\n",
                if f.chunked { " (chunked)" } else { "" },
                if f.damaged { "" } else { " (unanswered)" }, k, f.error));
        }
        s
    }
}

/// What reading one key back through the ordinary read path found.
///
/// `Damaged` and `Unavailable` are kept apart because only the first says anything about the
/// bytes.  A store too busy to answer within its deadline fails a read just as a damaged record
/// does, and a caller that removes damaged records must never remove one on a timeout
/// (CHUNK-VERIFY D1, 2026-10-17).
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum KeyCheck {
    Whole       { chunked: bool },                  // read, rejoined, decrypted and decoded
    Absent,                                         // no record, or the newest is a deletion
    Damaged     { chunked: bool, error: String },   // the bytes fail checksum, decryption or decoding
    Unavailable { chunked: bool, error: String },   // no answer: a timeout, a channel, anything else
}

impl KeyCheck {

    /// Classifies a failed read by the tags anywhere in its chain.  Only `Mismatch` (a checksum,
    /// or a record other than the one named, on every attempt), `Decrypt` and `Decode` say the
    /// bytes are bad; every other failure, a responder timeout included, is `Unavailable`, so
    /// an error nobody foresaw never condemns a record.
    pub fn of_error(chunked: bool, e: &Error<ErrTag>) -> Self {
        let error = fmt!("{}", e);
        let damaged = e.tags().iter()
            .any(|t| matches!(t, ErrTag::Mismatch | ErrTag::Decrypt | ErrTag::Decode));
        if damaged {
            Self::Damaged { chunked, error }
        } else {
            Self::Unavailable { chunked, error }
        }
    }
}

/// Reads `key` back whole, gathering and rejoining a chunked value, and says what it found.  It
/// issues no write, so unlike [`verify_live_set`] it is safe against a live store: it is one
/// ordinary read.  A record the key index holds but whose bytes fail their checksum, decryption
/// or decoding is [`KeyCheck::Damaged`], which a presence lookup (`contains_wait`) cannot see.
pub fn verify_key<
    const UIDL: usize,
    UID:    NumIdDat<UIDL> + 'static,
    ENC:    Encrypter + 'static,
    KH:     Hasher + 'static,
    PR:     Hasher + 'static,
    CS:     Checksummer + 'static,
>(
    api:    &OzoneApi<UIDL, UID, ENC, KH, PR, CS>,
    key:    &Dat,
    schms2: Option<&RestSchemesOverride<ENC, KH>>,
)
    -> KeyCheck
{
    // The head first, so that a chunked value is known as one whether or not its chunks read.
    let head = match api.get_head_wait(key, schms2) {
        Err(e)              => return KeyCheck::of_error(false, &e),
        Ok(None)            => return KeyCheck::Absent,
        Ok(Some((head, _))) => head,
    };
    if !matches!(head, Dat::Tup5u64(_)) {
        return KeyCheck::Whole { chunked: false };
    }
    match api.fetch_chunks(&head, schms2) {
        Ok(_)   => KeyCheck::Whole { chunked: true },
        Err(e)  => KeyCheck::of_error(true, &e),
    }
}

/// Reads every live key of the store behind `api` and reports each that fails to read, decrypt
/// or decode, with counts of the keys and of the chunked ones among them.  It issues no write.
/// An error is returned only when the store cannot be scanned; a key that cannot be read is a
/// finding, not an error.
///
/// # Arguments
/// * `scan_wait` - how long each zone's scan may take; a large store needs a generous wait.
pub fn verify_live_set<
    const UIDL: usize,
    UID:    NumIdDat<UIDL> + 'static,
    ENC:    Encrypter + 'static,
    KH:     Hasher + 'static,
    PR:     Hasher + 'static,
    CS:     Checksummer + 'static,
>(
    api:        &OzoneApi<UIDL, UID, ENC, KH, PR, CS>,
    schms2:     Option<&RestSchemesOverride<ENC, KH>>,
    scan_wait:  Wait,
)
    -> Outcome<VerifyReport>
{
    let entries = res!(api.scan_with_wait(&ScanOpts::all(), schms2, scan_wait));
    let mut rep = VerifyReport { keys: entries.len(), ..Default::default() };
    for (k, _, _) in entries {
        match verify_key(api, &k, schms2) {
            KeyCheck::Whole { chunked }         => {
                rep.present += 1;
                if chunked { rep.chunked += 1; }
            },
            KeyCheck::Absent                    => rep.absent += 1,
            KeyCheck::Damaged { chunked, error } => {
                if chunked { rep.chunked += 1; }
                rep.failures.push(ReadFailure { key: k, chunked, damaged: true, error });
            },
            KeyCheck::Unavailable { chunked, error } => {
                if chunked { rep.chunked += 1; }
                rep.failures.push(ReadFailure { key: k, chunked, damaged: false, error });
            },
        }
    }
    Ok(rep)
}
