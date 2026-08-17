//! Closing a database that more than one handle is holding.
//!
//! `O3db` is `Clone`, and until now the only way to close one was
//! `shutdown(self)`, which consumes. That shape has two failures, and both of
//! them are silent hangs rather than errors:
//!
//! 1. A handle inside an `Arc` -- which is how a server shares a store with the
//!    threads doing its slow work -- cannot be consumed at all, because
//!    `Arc::try_unwrap` never succeeds while another thread is holding one.
//! 2. Even when a handle *can* be consumed, the shutdown ended on
//!    `WaitGroup::wait`, and a wait group counts its clones. Every clone of the
//!    database carried a clone of the group, so the shutdown waited for handles
//!    that were still alive and never returned.
//!
//! `O3db::close(&self)` is the answer to both. What is checked here is that it
//! returns while a second handle is alive, that calling it twice is safe, and --
//! the part that matters to whatever is storing data -- that a database closed
//! this way opens again with its contents intact.
//!
//! Every wait in this file is bounded and reported as a failure. A test for a
//! deadlock that deadlocks says nothing except that somebody has to press
//! Ctrl-C.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_hash::{
    csum::ChecksumScheme,
    hash::HashScheme,
};
use oxedyne_fe2o3_jdat::prelude::*;
use oxedyne_fe2o3_o3db_sync::{
    O3db,
    base::constant,
    comm::msg::OzoneMsg,
    data::core::RestSchemesInput,
    test::setup::{
        self,
        Uid,
        UID_LEN,
    },
};

use std::{
    path::PathBuf,
    sync::{
        mpsc,
        Arc,
    },
    thread,
    time::Duration,
};

// How long a close may take before it is called a deadlock. A shutdown of an
// idle database is a message and a thread join, and takes well under a second.
// Thirty seconds is not a measurement, it is the difference between a failing
// test and a hung one.
const CLOSE_TIMEOUT: Duration = Duration::from_secs(30);

fn the_key() -> Dat {
    dat!("a value that has to survive a shared close")
}

fn the_value() -> Dat {
    dat!(42u8)
}

#[test]
fn main() -> Outcome<()> {
    log_set_level!("warn");
    let outcome = run();
    log_finish_wait!();
    outcome
}

fn run() -> Outcome<()> {

    // A directory of this test's own. A fixture shared with another test is
    // wiped by whichever of them gets there second.
    let db_dir = PathBuf::from("./test_db_shared_shutdown");
    let _ = std::fs::remove_dir_all(&db_dir);
    res!(std::fs::create_dir_all(&db_dir));
    let db_root = res!(db_dir.canonicalize());

    // ------------------------------------------------ write, then close shared
    {
        let db = Arc::new(res!(open(&db_root)));
        thread::sleep(Duration::from_secs(1));

        let user = Default::default();
        let resp = res!(db.api().store(the_key(), the_value(), user));
        for _ in 0..2 {
            match res!(resp.recv_timeout(constant::USER_REQUEST_TIMEOUT)) {
                OzoneMsg::Error(e) => return Err(err!(e,
                    "The database refused the write this test is built on."; Test, IO)),
                _ => {},
            }
        }
        match res!(db.api().get_wait(&the_key(), None)) {
            Some((v, _meta)) => req!(v, the_value(),
                "The value did not come back before the database was closed."),
            None => return Err(err!(
                "The value was not there before the database was closed, so \
                nothing this test goes on to check would mean anything."; Test, Missing)),
        }

        // A second handle, held for the whole of the close. This is the shape
        // that used to hang: a clone alive means a clone of the wait group
        // alive, and the wait group's wait counts clones.
        let held = db.clone();

        // The close itself, on a thread, so that a hang is a failure rather
        // than a test that never ends.
        let closing = db.clone();
        let (tx, rx) = mpsc::channel();
        res!(thread::Builder::new()
            .name("closing".to_string())
            .spawn(move || {
                let _ = tx.send(closing.close());
            }));
        match rx.recv_timeout(CLOSE_TIMEOUT) {
            Ok(result) => res!(result),
            Err(_) => return Err(err!(
                "A close through a shared handle did not return within {:?}, \
                with a second handle alive. That is the wait group counting its \
                own clones again.", CLOSE_TIMEOUT; Test, Timeout)),
        }

        // And a second close, which two threads both noticing a stop will make.
        // It must be safe and it must not wait for anything.
        let (tx, rx) = mpsc::channel();
        res!(thread::Builder::new()
            .name("closing-again".to_string())
            .spawn(move || {
                let _ = tx.send(held.close());
            }));
        match rx.recv_timeout(CLOSE_TIMEOUT) {
            Ok(result) => res!(result),
            Err(_) => return Err(err!(
                "Closing an already closed database did not return within {:?}.",
                CLOSE_TIMEOUT; Test, Timeout)),
        }
    }

    // ------------------------------------------------ and it opens again
    // The claim worth making: a store closed through a shared handle is a store
    // that can be opened, not one that has to be repaired first.
    {
        let db = res!(open(&db_root));
        thread::sleep(Duration::from_secs(1));
        match res!(db.api().get_wait(&the_key(), None)) {
            Some((v, _meta)) => req!(v, the_value(),
                "A database closed through a shared handle reopened with the \
                wrong value in it."),
            None => return Err(err!(
                "A database closed through a shared handle reopened without the \
                value it was holding."; Test, Missing, Data)),
        }
        res!(db.shutdown());
    }

    let _ = std::fs::remove_dir_all(&db_dir);
    Ok(())
}

/// No encryption, so that the second opening reads what the first one wrote
/// without a key having to be carried between them.
type TestDb = O3db<
    { UID_LEN },
    Uid,
    (),
    HashScheme,
    HashScheme,
    ChecksumScheme,
>;

/// Keeps whatever is already in the store.
fn open(db_root: &PathBuf) -> Outcome<TestDb> {
    let schms_input = RestSchemesInput::new(
        None::<()>,
        None::<HashScheme>,
        None::<HashScheme>,
        Some(ChecksumScheme::new_crc32()),
    );
    let cfg = res!(setup::default_cfg());
    setup::start_db(
        db_root.clone(),
        Some(cfg),
        schms_input,
        None,
        false,
        // The config file is left alone: this test reopens the same database
        // and a wipe would make it a different one.
        false,
    )
}
