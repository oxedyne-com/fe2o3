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
            s.push_str(&fmt!("  FAILED{} {}\n      {}\n",
                if f.chunked { " (chunked)" } else { "" }, k, f.error));
        }
        s
    }
}

/// What reading one key back through the ordinary read path found.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum KeyCheck {
    Whole   { chunked: bool },                  // read, rejoined, decrypted and decoded
    Absent,                                     // no record, or the newest is a deletion
    Failed  { chunked: bool, error: String },   // a record the index holds that will not read
}

/// Reads `key` back whole, gathering and rejoining a chunked value, and says what it found.  It
/// issues no write, so unlike [`verify_live_set`] it is safe against a live store: it is one
/// ordinary read.  A record the key index holds but whose bytes fail their checksum, decryption
/// or decoding is [`KeyCheck::Failed`], which a presence lookup (`contains_wait`) cannot see.
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
        Err(e)              => return KeyCheck::Failed { chunked: false, error: fmt!("{}", e) },
        Ok(None)            => return KeyCheck::Absent,
        Ok(Some((head, _))) => head,
    };
    if !matches!(head, Dat::Tup5u64(_)) {
        return KeyCheck::Whole { chunked: false };
    }
    match api.fetch_chunks(&head, schms2) {
        Ok(_)   => KeyCheck::Whole { chunked: true },
        Err(e)  => KeyCheck::Failed { chunked: true, error: fmt!("{}", e) },
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
            KeyCheck::Failed { chunked, error } => {
                if chunked { rep.chunked += 1; }
                rep.failures.push(ReadFailure { key: k, chunked, error });
            },
        }
    }
    Ok(rep)
}
