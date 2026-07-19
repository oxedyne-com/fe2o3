//! The publish store against a real Ozone instance.
//!
//! The unit tests cover the record's encoding and the index's arithmetic. This covers the thing they
//! cannot: that a post written to a database comes back out of it.

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_steel::srv::{
    console::union_admins,
    publish::{
        Markup,
        PostKind,
        PostState,
        PublishConfig,
        send::{
            self,
            BlueskyCreds,
            DestCreds,
            MailSender,
            MastodonCreds,
            SendReport,
        },
        store::{
            self,
            Record,
        },
        subscribe::{
            self,
            SubState,
        },
    },
};

mod common;

#[test]
fn a_post_survives_the_database() -> Outcome<()> {
    let (db, uid, _tmp) = match common::test_db() {
        Ok(t)   => t,
        Err(e)  => {
            println!("no test database available, skipping: {}", e);
            return Ok(());
        }
    };
    let handle = (db, uid);

    // Nothing published is an empty list, not an error.
    let posts = res!(store::list(&handle, "test"));
    assert_eq!(posts.len(), 0, "a fresh store is not empty");

    let rec = Record {
        slug:   fmt!("on-rent"),
        kind:   PostKind::Essay,
        state:  PostState::Live,
        markup: Markup::Markdown,
        date:   Some(fmt!("2026-07-17")),
        source: fmt!("# On rent\n\nAn opening sentence.\n"),
        deliveries: Vec::new(),
        tags:   vec![fmt!("rust"), fmt!("web")],
    };
    res!(store::put(&handle, &rec, "test"));

    // It comes back as the record it went in as, tags and all.
    let back = res!(store::get(&handle, "on-rent"));
    assert_eq!(back.as_ref(), Some(&rec));

    // The tags travel through to the rendered post.
    let posts = res!(store::list(&handle, "test"));
    assert_eq!(posts[0].tags, vec![fmt!("rust"), fmt!("web")]);

    // And the vocabulary is the sorted union across the store.
    assert_eq!(res!(store::all_tags(&handle, "test")), vec![fmt!("rust"), fmt!("web")]);

    // And as a rendered post, titled by its own heading.
    let posts = res!(store::list(&handle, "test"));
    assert_eq!(posts.len(), 1);
    assert_eq!(posts[0].title, "On rent");
    assert_eq!(posts[0].date, Some(fmt!("2026-07-17")));
    assert!(posts[0].html.contains("<h1>On rent</h1>"), "got: {}", posts[0].html);
    assert_eq!(posts[0].excerpt, "An opening sentence.");

    // Writing the same slug twice is one post, not two: the index does not grow.
    res!(store::put(&handle, &rec, "test"));
    let posts = res!(store::list(&handle, "test"));
    assert_eq!(posts.len(), 1, "the index grew on a rewrite");

    // A draft is written, and served to nobody.
    let draft = Record {
        slug:   fmt!("unfinished"),
        state:  PostState::Draft,
        ..rec.clone()
    };
    res!(store::put(&handle, &draft, "test"));
    assert!(res!(store::get(&handle, "unfinished")).is_some(), "the draft was not stored");
    let posts = res!(store::list(&handle, "test"));
    assert_eq!(posts.len(), 1, "a draft reached a reader");

    // Newest first.
    let older = Record {
        slug:   fmt!("older"),
        date:   Some(fmt!("2026-07-01")),
        ..rec.clone()
    };
    res!(store::put(&handle, &older, "test"));
    let posts = res!(store::list(&handle, "test"));
    assert_eq!(posts.len(), 2);
    assert_eq!(posts[0].slug, "on-rent", "not newest first");
    assert_eq!(posts[1].slug, "older");

    // Deleting takes it out of the index too, so a list does not name what is gone.
    assert!(res!(store::delete(&handle, "older", "test")));
    let posts = res!(store::list(&handle, "test"));
    assert_eq!(posts.len(), 1);
    assert_eq!(posts[0].slug, "on-rent");

    // A deleted post is gone to a reader that asks for it by name.
    assert!(res!(store::get(&handle, "older")).is_none(), "a deleted post still reads back");

    // The index can be rebuilt from what the database holds, drafts included but not the deleted one.
    //
    // This is the assertion that matters. `Database::delete` "marks for deletion", and a scan still
    // hands back the marked key -- so a rebuild that trusted the scan would resurrect every post ever
    // deleted, silently, the next time anything repaired the index.
    let n = res!(store::rebuild_index(&handle, "test"));
    assert_eq!(n, 2, "rebuild found {} posts, expected the live one and the draft", n);
    let posts = res!(store::list(&handle, "test"));
    assert_eq!(posts.len(), 1, "the rebuilt index resurrected a deleted post");
    assert_eq!(posts[0].slug, "on-rent");

    Ok(())
}

/// Destination credentials written to the database come back out of it, and the effective set lays the
/// stored ones over the config's.
#[test]
fn credentials_survive_the_database() -> Outcome<()> {
    let (db, uid, _tmp) = match common::test_db() {
        Ok(t)   => t,
        Err(e)  => {
            println!("no test database available, skipping: {}", e);
            return Ok(());
        }
    };
    let handle = (db, uid);

    // Nothing set is an empty set, not an error.
    let creds = res!(send::get_creds(&handle));
    assert_eq!(creds, DestCreds::default());

    // A Bluesky credential written and read back is the one that went in.
    let stored = DestCreds {
        mastodon:   None,
        bluesky:    Some(BlueskyCreds {
            host:           fmt!("bsky.social"),
            handle:         fmt!("me.bsky.social"),
            app_password:   fmt!("app-secret"),
        }),
    };
    res!(send::put_creds(&handle, &stored));
    assert_eq!(res!(send::get_creds(&handle)), stored);

    // The effective set lays the store over the config: the config's Mastodon survives, the store's
    // Bluesky wins where both could name one.
    let cfg = PublishConfig {
        creds: DestCreds {
            mastodon:   Some(MastodonCreds { base_url: fmt!("https://m"), token: fmt!("cfg-tok") }),
            bluesky:    Some(BlueskyCreds {
                host: fmt!("bsky.social"), handle: fmt!("cfg"), app_password: fmt!("cfg-pw"),
            }),
        },
        ..Default::default()
    };
    let eff = res!(send::effective_creds(&handle, &cfg));
    assert!(eff.mastodon.is_some(), "config Mastodon was lost");
    match &eff.bluesky {
        Some(b) => assert_eq!(b.handle, "me.bsky.social", "store Bluesky did not win"),
        None    => return Err(err!("effective Bluesky was lost"; Test, Missing)),
    }

    Ok(())
}

/// The database admin list bootstraps a site and manages it: empty to begin, a first admin claimed, a
/// second granted, one revoked, and the union with config throughout deciding whether a claim is open.
#[test]
fn admins_bootstrap_and_management() -> Outcome<()> {
    let (db, uid, _tmp) = match common::test_db() {
        Ok(t)   => t,
        Err(e)  => {
            println!("no test database available, skipping: {}", e);
            return Ok(());
        }
    };
    let handle = (db, uid);

    // Two well-formed member id-hashes: 64 lowercase hex characters, as a passphrase's SHA-256 renders.
    let a = "0123456789abcdef".repeat(4);
    let b = "fedcba9876543210".repeat(4);

    // A fresh store has granted no admins, and no config pin either, so the effective set is empty --
    // which is exactly the state a claim is open in.
    assert!(res!(store::admins_get(&handle, "test")).is_empty(), "a fresh admin list is not empty");
    assert!(union_admins(&[], &res!(store::admins_get(&handle, "test"))).is_empty(),
        "an unclaimed site is not claimable");

    // The first admin claims: the database list now holds them, and the site is no longer claimable.
    res!(store::admins_add(&handle, "test", &a));
    assert_eq!(res!(store::admins_get(&handle, "test")), vec![a.clone()]);
    assert!(!union_admins(&[], &res!(store::admins_get(&handle, "test"))).is_empty(),
        "a claimed site is still claimable");

    // Granting the same admin again grants them once: the list does not grow.
    res!(store::admins_add(&handle, "test", &a));
    assert_eq!(res!(store::admins_get(&handle, "test")).len(), 1, "an idempotent add grew the list");

    // A second admin is granted, alongside the first.
    res!(store::admins_add(&handle, "test", &b));
    assert_eq!(res!(store::admins_get(&handle, "test")), vec![a.clone(), b.clone()]);

    // The first is revoked, leaving the second.
    res!(store::admins_remove(&handle, "test", &a));
    assert_eq!(res!(store::admins_get(&handle, "test")), vec![b.clone()]);

    // Revoking one the list does not hold is a no-op, not an error.
    res!(store::admins_remove(&handle, "test", &a));
    assert_eq!(res!(store::admins_get(&handle, "test")), vec![b.clone()]);

    // A config-pinned admin is always effective, whatever the database holds -- the union carries it
    // even though the database list never named it.
    let pinned = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string();
    let eff = union_admins(&[pinned.clone()], &res!(store::admins_get(&handle, "test")));
    assert!(eff.iter().any(|h| h == &pinned), "the config pin was lost from the union");
    assert!(eff.iter().any(|h| h == &b), "the granted admin was lost from the union");

    Ok(())
}

/// The subscriber store against a real database: a sign-up pends, a confirmation promotes, an
/// unsubscribe demotes, only the confirmed are in the send set, re-subscribing a confirmed address is
/// silent, and a bad token confirms nobody.
#[test]
fn subscribers_survive_the_database() -> Outcome<()> {
    let (db, uid, _tmp) = match common::test_db() {
        Ok(t)   => t,
        Err(e)  => {
            println!("no test database available, skipping: {}", e);
            return Ok(());
        }
    };
    let handle = (db, uid);

    // A fresh store has no subscribers, and no confirmed send set.
    assert_eq!(res!(subscribe::count(&handle, "test")), 0, "a fresh subscriber list is not empty");
    assert!(res!(subscribe::confirmed(&handle, "test")).is_empty(), "a fresh send set is not empty");

    // A sign-up records a pending subscriber and hands back its token to send a confirmation with.
    let sub = match res!(subscribe::add_pending(&handle, "  Me@Example.COM ")) {
        Some(s) => s,
        None    => return Err(err!("a fresh address did not pend"; Test, Missing)),
    };
    assert_eq!(sub.email, "me@example.com", "the address was not normalised");
    assert_eq!(sub.state, SubState::Pending);
    let token = sub.token.clone();

    // Pending is not confirmed: it is not in the send set.
    assert!(res!(subscribe::confirmed(&handle, "test")).is_empty(), "a pending address reached the send set");

    // A bad token confirms nobody, and does not disturb the pending one.
    assert_eq!(res!(subscribe::confirm(&handle, "not-a-real-token", "test")),
        subscribe::ConfirmOutcome::Unknown);
    match res!(subscribe::get(&handle, "me@example.com")) {
        Some(s) => assert_eq!(s.state, SubState::Pending, "a bad token changed a subscriber"),
        None    => return Err(err!("the pending subscriber vanished"; Test, Missing)),
    }

    // The real token promotes them, and now they are the send set.
    assert_eq!(res!(subscribe::confirm(&handle, &token, "test")), subscribe::ConfirmOutcome::Confirmed);
    let send_set = res!(subscribe::confirmed(&handle, "test"));
    assert_eq!(send_set.len(), 1);
    assert_eq!(send_set[0].email, "me@example.com");

    // Confirming again is idempotent, not a second welcome.
    assert_eq!(res!(subscribe::confirm(&handle, &token, "test")), subscribe::ConfirmOutcome::Already);

    // Re-subscribing a confirmed address is silent: no confirmation to send, and the list does not grow.
    assert!(res!(subscribe::add_pending(&handle, "me@example.com")).is_none(),
        "re-subscribing a confirmed address asked for a second confirmation");
    assert_eq!(res!(subscribe::count(&handle, "test")), 1, "the subscriber list grew on a re-subscribe");

    // Unsubscribing by token takes them out of the send set but keeps the record.
    assert_eq!(res!(subscribe::unsubscribe(&handle, &token, "test")), subscribe::UnsubOutcome::Done);
    assert!(res!(subscribe::confirmed(&handle, "test")).is_empty(), "an unsubscribed address stayed in the send set");
    assert_eq!(res!(subscribe::count(&handle, "test")), 1, "an unsubscribe deleted the record");

    // A second subscriber, confirmed, so the export carries more than one row.
    let sub2 = match res!(subscribe::add_pending(&handle, "other@example.net")) {
        Some(s) => s,
        None    => return Err(err!("a second address did not pend"; Test, Missing)),
    };
    res!(subscribe::confirm(&handle, &sub2.token, "test"));

    // The export is a CSV with a header and a row per subscriber.
    let csv = res!(subscribe::export(&handle, "test"));
    assert!(csv.starts_with("email,state,created\n"), "no CSV header: {}", csv);
    assert!(csv.contains("me@example.com,unsubscribed,"), "the unsubscribed row is wrong: {}", csv);
    assert!(csv.contains("other@example.net,confirmed,"), "the confirmed row is wrong: {}", csv);

    Ok(())
}

/// Suppression: a bounced address leaves the send set and stays out of it, and a re-subscribe does not
/// resurrect it -- unlike an unsubscribe, which a re-subscribe reopens as a fresh opt-in.
#[test]
fn suppression_keeps_a_bounced_address_off_the_list() -> Outcome<()> {
    let (db, uid, _tmp) = match common::test_db() {
        Ok(t)   => t,
        Err(e)  => {
            println!("no test database available, skipping: {}", e);
            return Ok(());
        }
    };
    let handle = (db, uid);

    // A confirmed subscriber is in the send set.
    let sub = match res!(subscribe::add_pending(&handle, "gone@example.com")) {
        Some(s) => s,
        None    => return Err(err!("a fresh address did not pend"; Test, Missing)),
    };
    res!(subscribe::confirm(&handle, &sub.token, "test"));
    assert_eq!(res!(subscribe::confirmed(&handle, "test")).len(), 1, "the confirmed address is not in the send set");

    // A permanent delivery failure suppresses them: the mark that send_newsletter makes on a 5xx.
    assert!(res!(subscribe::mark_bounced(&handle, "gone@example.com", "test")), "the address was not found to bounce");
    match res!(subscribe::get(&handle, "gone@example.com")) {
        Some(s) => assert_eq!(s.state, SubState::Bounced, "the address was not marked bounced"),
        None    => return Err(err!("the bounced subscriber vanished"; Test, Missing)),
    }

    // Bounced is not confirmed, so it is out of the send set.
    assert!(res!(subscribe::confirmed(&handle, "test")).is_empty(), "a bounced address stayed in the send set");

    // A re-subscribe does not resurrect it: the same suppressed answer as an address already on the list,
    // and the record stays bounced.
    assert!(res!(subscribe::add_pending(&handle, "gone@example.com")).is_none(),
        "re-subscribing a bounced address asked for a confirmation");
    match res!(subscribe::get(&handle, "gone@example.com")) {
        Some(s) => assert_eq!(s.state, SubState::Bounced, "a re-subscribe resurrected a bounced address"),
        None    => return Err(err!("the bounced subscriber vanished on re-subscribe"; Test, Missing)),
    }

    // An unsubscribe, by contrast, IS reopened by a re-subscribe -- the difference between a choice and a
    // bounce. A second, confirmed, then unsubscribed address proves it.
    let sub2 = match res!(subscribe::add_pending(&handle, "back@example.net")) {
        Some(s) => s,
        None    => return Err(err!("a second address did not pend"; Test, Missing)),
    };
    res!(subscribe::confirm(&handle, &sub2.token, "test"));
    res!(subscribe::unsubscribe(&handle, &sub2.token, "test"));
    assert!(res!(subscribe::confirmed(&handle, "test")).is_empty(), "an unsubscribed address stayed in the send set");
    // Re-subscribing an unsubscribed address pends afresh: a confirmation is due.
    assert!(res!(subscribe::add_pending(&handle, "back@example.net")).is_some(),
        "re-subscribing an unsubscribed address did not reopen it");

    Ok(())
}

/// Admin-side management: an admin unsubscribes an address by its name, keeping the record, and erases
/// another outright, leaving nothing -- the two ways a subscriber leaves, told apart.
#[test]
fn admin_unsubscribe_and_remove_round_trip() -> Outcome<()> {
    let (db, uid, _tmp) = match common::test_db() {
        Ok(t)   => t,
        Err(e)  => {
            println!("no test database available, skipping: {}", e);
            return Ok(());
        }
    };
    let handle = (db, uid);

    let a = match res!(subscribe::add_pending(&handle, "a@example.com")) {
        Some(s) => s,
        None    => return Err(err!("a did not pend"; Test, Missing)),
    };
    res!(subscribe::confirm(&handle, &a.token, "test"));
    let b = match res!(subscribe::add_pending(&handle, "b@example.com")) {
        Some(s) => s,
        None    => return Err(err!("b did not pend"; Test, Missing)),
    };
    res!(subscribe::confirm(&handle, &b.token, "test"));
    assert_eq!(res!(subscribe::count(&handle, "test")), 2);

    // The admin unsubscribes a by name: the record is kept, out of the send set, still counted.
    assert!(res!(subscribe::unsubscribe_email(&handle, "a@example.com", "test")), "a was not found to unsubscribe");
    match res!(subscribe::get(&handle, "a@example.com")) {
        Some(s) => assert_eq!(s.state, SubState::Unsubscribed, "a was not unsubscribed"),
        None    => return Err(err!("a vanished on unsubscribe"; Test, Missing)),
    }
    assert_eq!(res!(subscribe::count(&handle, "test")), 2, "an admin unsubscribe erased the record");
    assert_eq!(res!(subscribe::confirmed(&handle, "test")).len(), 1, "a stayed in the send set");

    // Unsubscribing an address the store does not hold is false, not an error.
    assert!(!res!(subscribe::unsubscribe_email(&handle, "nobody@example.com", "test")),
        "unsubscribing an unknown address claimed a subscriber");

    // The admin erases b outright: gone from the store and the index both.
    assert!(res!(subscribe::remove(&handle, "b@example.com", "test")), "b was not found to erase");
    assert!(res!(subscribe::get(&handle, "b@example.com")).is_none(), "an erased subscriber still reads back");
    assert_eq!(res!(subscribe::count(&handle, "test")), 1, "the erase did not shrink the list");
    // Erasing one the store does not hold is false, not an error.
    assert!(!res!(subscribe::remove(&handle, "b@example.com", "test")), "erasing a gone address claimed one");

    Ok(())
}

/// Send history: a recorded send comes back with its counts, appended, and read most recent first.
#[test]
fn send_history_records_each_send() -> Outcome<()> {
    let (db, uid, _tmp) = match common::test_db() {
        Ok(t)   => t,
        Err(e)  => {
            println!("no test database available, skipping: {}", e);
            return Ok(());
        }
    };
    let handle = (db, uid);

    // A fresh store has sent nothing.
    assert!(res!(send::send_history(&handle)).is_empty(), "a fresh send history is not empty");

    // One send recorded, with the report's counts intact.
    let r1 = SendReport { attempted: 5, sent: 4, failed: 1, suppressed: 0 };
    res!(send::record_send(&handle, &send::SendEntry::of("on-rent", "2026-07-18T10:00:00Z", &r1)));
    let hist = res!(send::send_history(&handle));
    assert_eq!(hist.len(), 1);
    assert_eq!(hist[0].slug, "on-rent");
    assert_eq!(hist[0].attempted, 5);
    assert_eq!(hist[0].sent, 4);
    assert_eq!(hist[0].failed, 1);
    assert_eq!(hist[0].suppressed, 0);

    // A second send appends, and the history reads newest first.
    let r2 = SendReport { attempted: 6, sent: 5, failed: 0, suppressed: 1 };
    res!(send::record_send(&handle, &send::SendEntry::of("on-work", "2026-07-19T09:00:00Z", &r2)));
    let hist = res!(send::send_history(&handle));
    assert_eq!(hist.len(), 2, "the second send did not append");
    assert_eq!(hist[0].slug, "on-work", "the history is not most-recent-first");
    assert_eq!(hist[0].suppressed, 1);
    assert_eq!(hist[1].slug, "on-rent");

    Ok(())
}

/// A test-send touches no subscriber state and writes no history: on the pre-network paths it exercises
/// -- a malformed address, and a draft post -- nothing in the store moves.
#[tokio::test]
async fn a_test_send_touches_no_state_or_history() -> Outcome<()> {
    let (db, uid, _tmp) = match common::test_db() {
        Ok(t)   => t,
        Err(e)  => {
            println!("no test database available, skipping: {}", e);
            return Ok(());
        }
    };
    let handle = (db, uid);

    // A confirmed subscriber and a draft post: the state a test must not disturb.
    let sub = match res!(subscribe::add_pending(&handle, "reader@example.com")) {
        Some(s) => s,
        None    => return Err(err!("the reader did not pend"; Test, Missing)),
    };
    res!(subscribe::confirm(&handle, &sub.token, "test"));
    let draft = Record {
        slug:   fmt!("wip"),
        kind:   PostKind::Note,
        state:  PostState::Draft,
        markup: Markup::Markdown,
        date:   None,
        source: fmt!("# WIP\n\nNot yet.\n"),
        deliveries: Vec::new(),
        tags:   Vec::new(),
    };
    res!(store::put(&handle, &draft, "test"));

    let sender = res!(MailSender::new("mail.x.test".to_string(), Vec::new(), "news@x.test".to_string()));
    let cfg = PublishConfig { base_url: fmt!("https://x.test"), ..Default::default() };

    // A malformed address is refused before anything is read or sent.
    assert!(send::send_test(&sender, &handle, &cfg, "news@x.test", "wip", "not-an-address", "test").await.is_err(),
        "a test to a malformed address did not error");
    // A draft is refused before the network: a test sends the live post a subscriber would get.
    assert!(send::send_test(&sender, &handle, &cfg, "news@x.test", "wip", "you@example.com", "test").await.is_err(),
        "a test of a draft did not error");

    // Neither path moved the subscriber, marked anything bounced, or wrote a line of history.
    match res!(subscribe::get(&handle, "reader@example.com")) {
        Some(s) => assert_eq!(s.state, SubState::Confirmed, "a test-send changed a subscriber's state"),
        None    => return Err(err!("the reader vanished during a test-send"; Test, Missing)),
    }
    assert_eq!(res!(subscribe::confirmed(&handle, "test")).len(), 1, "a test-send changed the send set");
    assert!(res!(send::send_history(&handle)).is_empty(), "a test-send wrote to the history");

    Ok(())
}
