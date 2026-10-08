//! The daily sweep: a sign-up that never confirmed is deleted, and a rate row whose window has
//! passed goes with it.
//!
//! Each runs `outbox::sweep` against a real Ozone instance with the clock an argument, so a month
//! passes in an instant. The sweep finds the rate rows by a scan of the store, which these tests
//! therefore exercise as well. They do not skip when no database starts.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_iop_db::api::Database;
use oxedyne_fe2o3_jdat::prelude::*;
use oxedyne_fe2o3_steel::srv::{
    id,
    publish::{
        PublishConfig,
        Source,
        comment,
        outbox::{
            self,
            Swept,
        },
        rate::{
            self,
            Window,
        },
        send,
        store,
        subscribe::{
            self,
            SubState,
        },
    },
};

use std::sync::{
    Arc,
    RwLock,
};

mod common;

const DAY: u64 = 86_400;

type Handle = (Arc<RwLock<common::TestDb>>, id::Uid);

fn cfg() -> PublishConfig {
    PublishConfig {
        path:                   fmt!("/news"),
        base_url:               fmt!("https://site.test"),
        source:                 Source::Store,
        pending_expiry_days:    7,
        subscribe_rate_secs:    60,
        subscribe_rate_hourly:  5,
        comment_rate_secs:      30,
        comment_rate_hourly:    10,
        confirm_interval_secs:  DAY,
        confirm_max:            3,
        confirm_window_days:    30,
        ..Default::default()
    }
}

// A pending sign-up that was sent its confirmation `ago` seconds before `now`.
fn sent_pending(db: &Handle, email: &str, now: u64, ago: u64) -> Outcome<subscribe::Subscriber> {
    let sub = res!(subscribe::add_pending(db, email, &Window::default(), now));
    let sub = res!(sub.ok_or_else(|| err!("no pending record was made for {}", email; Test, Missing)));
    let at = res!(send::iso_of((now - ago) as i64));
    assert!(res!(subscribe::mark_sent(db, email, &at)), "the send of {} was not recorded", email);
    Ok(sub)
}

fn state_of(db: &Handle, email: &str) -> Outcome<Option<SubState>> {
    Ok(res!(subscribe::get(db, email)).map(|s| s.state))
}

// The addresses the subscriber index names, read straight from the store.
fn indexed(db: &Handle) -> Outcome<Vec<String>> {
    let guard = lock_read!(db.0);
    let old = res!(guard.get(&dat!(subscribe::INDEX_KEY), None)).map(|(v, _)| v);
    store::names_of(old, "subscriber index")
}

fn row_exists(db: &Handle, key: &str) -> Outcome<bool> {
    let guard = lock_read!(db.0);
    Ok(res!(guard.get(&dat!(key.to_string()), None)).is_some())
}

#[test]
fn test_a_pending_sign_up_sent_eight_days_ago_is_deleted_from_record_and_index_00() -> Outcome<()> {
    let (db, uid, _tmp) = res!(common::test_db());
    let handle = (db, uid);
    let now = rate::now_secs();
    res!(sent_pending(&handle, "old@site.test", now, 8 * DAY));
    res!(sent_pending(&handle, "new@site.test", now, DAY));

    let swept = outbox::sweep(&handle, &cfg(), now, "test");

    assert_eq!(swept.pending, 1, "the sweep removed {} sign-ups, not the one that had lapsed", swept.pending);
    assert_eq!(state_of(&handle, "old@site.test")?, None, "the lapsed record is still in the store");
    assert_eq!(indexed(&handle)?, vec![fmt!("new@site.test")], "the index still names the lapsed address");
    assert_eq!(state_of(&handle, "new@site.test")?, Some(SubState::Pending), "a recent sign-up was removed");
    Ok(())
}

#[test]
fn test_a_pending_sign_up_never_sent_a_confirmation_is_kept_however_old_01() -> Outcome<()> {
    let (db, uid, _tmp) = res!(common::test_db());
    let handle = (db, uid);
    let now = rate::now_secs();
    // Held behind a ceiling of 0: queued, never sent, so it has no clock.
    let held = res!(subscribe::add_pending(&handle, "held@site.test", &Window::default(), now));
    assert!(held.is_some_and(|s| s.sent.is_none()), "a record nothing was sent to has a sent time");

    let swept = outbox::sweep(&handle, &cfg(), now + 400 * DAY, "test");

    assert_eq!(swept, Swept::default(), "the sweep removed {:?} from a store holding one held sign-up", swept);
    assert_eq!(state_of(&handle, "held@site.test")?, Some(SubState::Pending), "a held sign-up expired");
    assert_eq!(indexed(&handle)?, vec![fmt!("held@site.test")]);
    Ok(())
}

#[test]
fn test_confirmed_unsubscribed_and_bounced_records_are_never_expired_02() -> Outcome<()> {
    let (db, uid, _tmp) = res!(common::test_db());
    let handle = (db, uid);
    let now = rate::now_secs();
    let ok = res!(sent_pending(&handle, "ok@site.test", now, 30 * DAY));
    let _ = res!(subscribe::confirm(&handle, &ok.token, "test"));
    res!(sent_pending(&handle, "left@site.test", now, 30 * DAY));
    assert!(res!(subscribe::unsubscribe_email(&handle, "left@site.test", "test")));
    res!(sent_pending(&handle, "bad@site.test", now, 30 * DAY));
    assert!(res!(subscribe::mark_bounced(&handle, "bad@site.test", "test")));

    let swept = outbox::sweep(&handle, &cfg(), now, "test");

    assert_eq!(swept.pending, 0, "the sweep removed {} settled records", swept.pending);
    assert_eq!(state_of(&handle, "ok@site.test")?, Some(SubState::Confirmed));
    assert_eq!(state_of(&handle, "left@site.test")?, Some(SubState::Unsubscribed));
    assert_eq!(state_of(&handle, "bad@site.test")?, Some(SubState::Bounced));
    assert_eq!(indexed(&handle)?.len(), 3, "the index lost a settled record");
    Ok(())
}

#[test]
fn test_a_rate_row_past_its_window_is_deleted_and_a_live_one_kept_03() -> Outcome<()> {
    let (db, uid, _tmp) = res!(common::test_db());
    let handle = (db, uid);
    let now = rate::now_secs();
    let cfg = cfg();
    let sender = Window::hourly(cfg.subscribe_rate_secs, cfg.subscribe_rate_hourly);
    let comment_w = Window::hourly(cfg.comment_rate_secs, cfg.comment_rate_hourly);
    let recipient = cfg.confirm_window();
    // (key, window it was counted in, how long ago it was last counted)
    let rows = [
        (fmt!("{}old", subscribe::RATE_PREFIX),     sender,     2 * 3600),
        (fmt!("{}new", subscribe::RATE_PREFIX),     sender,     10),
        (fmt!("{}old", subscribe::TO_PREFIX),       recipient,  31 * DAY),
        (fmt!("{}new", subscribe::TO_PREFIX),       recipient,  29 * DAY),
        (fmt!("{}old", comment::RATE_PREFIX),       comment_w,  2 * 3600),
    ];
    for (key, w, ago) in &rows {
        assert!(res!(rate::allow_at(&handle, key, w, now - ago)), "row {} was refused", key);
    }
    // A key under no rate prefix is not the sweep's to touch.
    let other = fmt!("publish/subscribe-rates/old");
    assert!(res!(rate::allow_at(&handle, &other, &sender, now - 400 * DAY)));

    let swept = outbox::sweep(&handle, &cfg, now, "test");

    assert_eq!(swept.rows, 3, "the sweep removed {} rows, not the three that had run out", swept.rows);
    for (key, _, ago) in &rows {
        let spent = *ago == 2 * 3600 || *ago == 31 * DAY;
        assert_eq!(row_exists(&handle, key)?, !spent, "row {} ({}s old) was wrongly {}",
            key, ago, if spent { "kept" } else { "removed" });
    }
    assert!(row_exists(&handle, &other)?, "a key outside the rate prefixes was removed");
    Ok(())
}
