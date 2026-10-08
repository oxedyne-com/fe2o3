//! Read-modify-write rows under contention, against a real Ozone instance.
//!
//! Every derived row in the publish module (the post and subscriber indexes, the rate counters, the
//! site secret) is read, changed and written back. These tests start many threads together on one
//! store and ask whether any of their changes was lost. They do not skip when no database starts:
//! a skipped test cannot be shown red.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_steel::srv::publish::{
    Markup,
    PostState,
    comment,
    store::{
        self,
        Record,
    },
    subscribe,
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

fn record(slug: &str) -> Record {
    Record {
        slug:       slug.to_string(),
        author:     String::new(),
        categories: Vec::new(),
        state:      PostState::Live,
        markup:     Markup::Markdown,
        date:       Some(fmt!("2026-07-17")),
        source:     fmt!("# {}\n\nA sentence.\n", slug),
        deliveries: Vec::new(),
        tags:       Vec::new(),
        ai_level:   None,
    }
}

/// Thirty-two sign-ups at once all reach the index.
#[test]
fn concurrent_signups_all_reach_the_index() -> Outcome<()> {
    let (db, uid, _tmp) = res!(common::test_db());
    let handle = (db, uid);
    let results = together(THREADS, |i| {
        text(subscribe::add_pending(&handle, &fmt!("reader{}@example.com", i)))
    });
    let subs = res!(all_ok(results));
    assert_eq!(subs.len(), THREADS);
    let listed = res!(subscribe::list(&handle, "test"));
    assert_eq!(listed.len(), THREADS, "the index lost {} sign-ups", THREADS - listed.len());
    Ok(())
}

/// Thirty-two writes of different posts all reach the post index.
#[test]
fn concurrent_posts_all_reach_the_index() -> Outcome<()> {
    let (db, uid, _tmp) = res!(common::test_db());
    let handle = (db, uid);
    let results = together(THREADS, |i| {
        text(store::put(&handle, &record(&fmt!("post-{}", i)), "test"))
    });
    res!(all_ok(results));
    let recs = res!(store::list_records(&handle, "test"));
    assert_eq!(recs.len(), THREADS, "the post index lost {} posts", THREADS - recs.len());
    Ok(())
}

/// A counter allowing five an hour allows exactly five of thirty-two simultaneous attempts.
#[test]
fn concurrent_attempts_are_counted_exactly() -> Outcome<()> {
    let (db, uid, _tmp) = res!(common::test_db());
    let handle = (db, uid);
    let results = together(THREADS, |_| {
        text(comment::rate_allows(&handle, "one-sender", 0, 5))
    });
    let allowed = res!(all_ok(results)).into_iter().filter(|a| *a).count();
    assert_eq!(allowed, 5, "{} attempts were allowed against a limit of 5", allowed);
    Ok(())
}

/// Callers racing to make the site secret on an empty store all get the one that was kept.
#[test]
fn concurrent_first_callers_share_one_secret() -> Outcome<()> {
    let (db, uid, _tmp) = res!(common::test_db());
    let handle = (db, uid);
    let results = together(8, |_| text(comment::site_secret(&handle)));
    let secrets = res!(all_ok(results));
    let kept = res!(comment::site_secret(&handle));
    for (i, s) in secrets.iter().enumerate() {
        assert_eq!(s, &kept, "caller {} was handed a secret that was not kept", i);
    }
    Ok(())
}
