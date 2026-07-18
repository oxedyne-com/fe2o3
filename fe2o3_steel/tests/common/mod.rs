//! Shared test scaffolding.

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_crypto::enc::EncryptionScheme;
use oxedyne_fe2o3_hash::{
    csum::ChecksumScheme,
    hash::HashScheme,
};
use oxedyne_fe2o3_o3db_sync::O3db;
use oxedyne_fe2o3_steel::{
    app::constant as app_const,
    srv::{
        context::new_db,
        id,
    },
};

use std::{
    path::PathBuf,
    sync::{
        atomic::{
            AtomicU64,
            Ordering,
        },
        Arc,
        RwLock,
    },
    time::Duration,
};

/// The database type a Steel vhost holds.
pub type TestDb = O3db<
    { id::UID_LEN },
    id::Uid,
    EncryptionScheme,
    HashScheme,
    HashScheme,
    ChecksumScheme,
>;

/// A directory that removes itself.
pub struct TmpDir(pub PathBuf);

impl Drop for TmpDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A started Ozone database in a temporary directory, and the user id to write under.
///
/// The directory goes when the returned `TmpDir` does, so a test leaves nothing behind and two runs
/// of the same test do not see each other's posts. The name carries a per-call counter as well as the
/// process id, because two tests in one binary run in parallel by default: keyed on the process alone
/// they would share a directory, and each call's `remove_dir_all` would wipe the other's database
/// mid-test.
static DB_SEQ: AtomicU64 = AtomicU64::new(0);

pub fn test_db() -> Outcome<(Arc<RwLock<TestDb>>, id::Uid, TmpDir)> {
    let seq = DB_SEQ.fetch_add(1, Ordering::Relaxed);
    let root = std::env::temp_dir()
        .join(fmt!("steel-publish-test-{}-{}", std::process::id(), seq));
    let _ = std::fs::remove_dir_all(&root);
    res!(std::fs::create_dir_all(&root), IO, File);
    let tmp = TmpDir(root.clone());

    // Any key will do: the test is about what comes back out, not about what it looks like at rest.
    let enc_key = [0u8; 32];
    let mut db = res!(new_db(&root, &enc_key));
    // The same sequence the server runs. Starting without the rest of it leaves the bots up but not
    // answering, and every read then fails on a five-second responder timeout rather than saying the
    // database is not ready -- so mirror it exactly rather than guess which parts matter.
    res!(db.start("db_test"));
    res!(ok!(db.updated_api()).activate_gc(true));
    std::thread::sleep(Duration::from_millis(200));
    res!(db.api().ping_bots(app_const::GET_DATA_WAIT));

    Ok((Arc::new(RwLock::new(db)), id::Uid::default(), tmp))
}
