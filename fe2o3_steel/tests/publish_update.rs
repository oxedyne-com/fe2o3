//! The read-modify-write primitive itself, against a real Ozone instance.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_iop_db::api::Database;
use oxedyne_fe2o3_jdat::prelude::*;
use oxedyne_fe2o3_steel::srv::publish::store::{
    self,
    Edit,
};

use std::sync::Barrier;

mod common;

const THREADS: usize = 32;

fn text<T>(r: Outcome<T>) -> Result<T, String> {
    match r {
        Ok(v)   => Ok(v),
        Err(e)  => Err(fmt!("{}", e)),
    }
}

// Runs `f(0..n)` on n threads released together, so they meet in the store at once.
fn together<R, F>(n: usize, f: F) -> Vec<Result<R, String>>
where
    R: Send,
    F: Fn(usize) -> Result<R, String> + Sync,
{
    let gate = Barrier::new(n);
    std::thread::scope(|s| {
        let handles: Vec<_> = (0..n).map(|i| {
            let gate = &gate;
            let f = &f;
            s.spawn(move || {
                gate.wait();
                f(i)
            })
        }).collect();
        handles.into_iter()
            .map(|h| h.join().unwrap_or_else(|_| Err(fmt!("a thread panicked"))))
            .collect()
    })
}

fn all_ok<R>(results: Vec<Result<R, String>>) -> Outcome<Vec<R>> {
    let mut out = Vec::new();
    for r in results {
        match r {
            Ok(v)   => out.push(v),
            Err(e)  => return Err(err!("a thread failed: {}", e; Test, Invalid)),
        }
    }
    Ok(out)
}

/// The primitive itself: thirty-two increments of one key arrive as thirty-two.
#[test]
fn update_loses_no_change() -> Outcome<()> {
    let (db, uid, _tmp) = res!(common::test_db());
    let handle = (db, uid);
    let key = dat!("test/counter");
    let results = together(THREADS, |_| {
        text(store::update(&handle, &key, |old| -> Outcome<(Edit, ())> {
            let n = match old {
                Some(Dat::Str(s))   => res!(s.parse::<u64>().map_err(|e| err!("{}", e; Invalid, Input))),
                _                   => 0,
            };
            Ok((Edit::Set(dat!(fmt!("{}", n + 1))), ()))
        }))
    });
    res!(all_ok(results));
    let (db_arc, _) = &handle;
    let guard = lock_read!(db_arc);
    match res!(guard.get(&key, None)) {
        Some((Dat::Str(s), _))  => assert_eq!(s, fmt!("{}", THREADS)),
        other                   => return Err(err!("unexpected counter value {:?}", other; Test, Invalid)),
    }
    Ok(())
}

/// `Keep` writes nothing, and `Clear` removes the key.
#[test]
fn update_keeps_and_clears() -> Outcome<()> {
    let (db, uid, _tmp) = res!(common::test_db());
    let handle = (db, uid);
    let key = dat!("test/cell");
    // An absent key is handed over as `None`, and `Keep` leaves it absent.
    let saw = res!(store::update(&handle, &key, |old| -> Outcome<(Edit, bool)> {
        Ok((Edit::Keep, old.is_none()))
    }));
    assert!(saw);
    {
        let (db_arc, _) = &handle;
        let guard = lock_read!(db_arc);
        assert!(res!(guard.get(&key, None)).is_none(), "Keep wrote a value");
    }
    res!(store::update(&handle, &key, |_| -> Outcome<(Edit, ())> { Ok((Edit::Set(dat!("v")), ())) }));
    let saw = res!(store::update(&handle, &key, |old| -> Outcome<(Edit, bool)> {
        Ok((Edit::Clear, matches!(old, Some(Dat::Str(ref s)) if s == "v")))
    }));
    assert!(saw, "the value written was not handed back");
    let (db_arc, _) = &handle;
    let guard = lock_read!(db_arc);
    assert!(res!(guard.get(&key, None)).is_none(), "Clear left the value");
    Ok(())
}
