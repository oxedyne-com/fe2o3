//! A message Steel cannot file is answered `451` so its sender retries, never `550` and never a
//! silent drop of one recipient beside others (RFC 5321 §4.2.5: a server that has not taken
//! responsibility for a message answers a temporary failure).

use oxedyne_fe2o3_core::{
    file::ExclusiveLock,
    prelude::*,
};
use oxedyne_fe2o3_mail::{
    maildir::MaildirStore,
    outbound::OutboundSpool,
    passwd::PasswdFileUserStore,
};
use oxedyne_fe2o3_net::{
    mail::{
        store::{
            FolderName,
            MailStore,
        },
        user::UserStore,
    },
    smtp::handler::{
        HandlerOutcome,
        SmtpHandler,
        SmtpTransaction,
    },
};
use oxedyne_fe2o3_steel::app::mail::AppMailHandler;

use std::{
    fs,
    path::PathBuf,
    sync::Arc,
    time::Duration,
};


const USERS: &str = "{ \"users\": [ \
    { \"address\": \"alice@test.local\", \"delivery_dir\": \"test.local/alice\", \"argon2id\": \"x\" }, \
    { \"address\": \"bob@test.local\", \"delivery_dir\": \"test.local/bob\", \"argon2id\": \"x\" } ] }";

const MESSAGE: &str = "From: carol@a.example\r\nTo: alice@test.local, bob@test.local\r\n\
    Message-ID: <retry-1@a.example>\r\nSubject: Hi\r\n\r\nHello.\r\n";

fn scratch(tag: &str) -> Outcome<PathBuf> {
    let dir = std::env::temp_dir().join(fmt!("fe2o3_steel_mail_{}_{}", tag, std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    res!(fs::create_dir_all(dir.join("mail")), IO, File);
    res!(fs::create_dir_all(dir.join("spool")), IO, File);
    res!(fs::write(dir.join("users.jdat"), USERS), IO, File);
    Ok(dir)
}

fn handler(dir: &PathBuf) -> Outcome<AppMailHandler> {
    let store = res!(MaildirStore::new(dir.join("mail"), "mx.test.local"))
        .with_lock_wait(Duration::from_millis(300));
    let users = PasswdFileUserStore::new(dir.join("users.jdat"));
    for addr in ["alice@test.local", "bob@test.local"] {
        let u = res!(res!(users.lookup(addr)).ok_or_else(|| err!("No user {}.", addr; Missing)));
        res!(store.ensure_user(&u));
    }
    Ok(AppMailHandler {
        store,
        users,
        spool:          res!(OutboundSpool::new(dir.join("spool"))),
        dkim:           Vec::new(),
        local_domains:  Arc::new(vec![fmt!("test.local")]),
    })
}

fn txn() -> SmtpTransaction {
    SmtpTransaction {
        mail_from:      fmt!("carol@a.example"),
        rcpt_to:        vec![fmt!("alice@test.local"), fmt!("bob@test.local")],
        helo_domain:    fmt!("a.example"),
        auth_user:      None,
        peer:           ([192, 0, 2, 1], 25).into(),
        tls:            false,
        raw_message:    MESSAGE.as_bytes().to_vec(),
    }
}

fn inbox_count(h: &AppMailHandler, addr: &str) -> Outcome<usize> {
    let u = res!(res!(h.users.lookup(addr)).ok_or_else(|| err!("No user {}.", addr; Missing)));
    Ok(res!(h.store.list_messages(&u, &FolderName::new("INBOX"), true)).len())
}

/// Bob's INBOX is locked past the wait: the whole message is answered 451 with 4.3.0, alice's copy
/// notwithstanding, and the retry once the lock is free lands one copy in each INBOX.
#[test]
fn a_failed_local_delivery_is_answered_451_and_the_retry_duplicates_nothing() -> Outcome<()> {
    let dir = res!(scratch("451"));
    let h = res!(handler(&dir));
    let lock_path = dir.join("mail").join("test.local/bob").join("hematite-folder.lock");
    let held = res!(ExclusiveLock::acquire(&lock_path));

    match res!(h.deliver_inbound(txn())) {
        HandlerOutcome::RejectTemporary(reason) => assert!(reason.starts_with("4.3.0"),
            "the reason should carry the enhanced code 4.3.0: {:?}", reason),
        other => return Err(err!("A failed delivery was answered {:?}, not 451.", other; Test)),
    }
    // The same for a sole recipient, which was answered 550 and bounced.
    let mut sole = txn();
    sole.rcpt_to = vec![fmt!("bob@test.local")];
    match res!(h.deliver_inbound(sole)) {
        HandlerOutcome::RejectTemporary(_) => {},
        other => return Err(err!("A sole failed recipient was answered {:?}, not 451.", other; Test)),
    }
    // Submission's local delivery is the same contract.
    match res!(h.submit_outbound(txn())) {
        HandlerOutcome::RejectTemporary(_) => {},
        other => return Err(err!("A failed local submission was answered {:?}, not 451.", other; Test)),
    }
    drop(held);

    match res!(h.deliver_inbound(txn())) {
        HandlerOutcome::Accepted(_) => {},
        other => return Err(err!("The retry was answered {:?}.", other; Test)),
    }
    assert!(res!(inbox_count(&h, "bob@test.local")) >= 1, "bob holds the message");
    let _ = fs::remove_dir_all(&dir);
    Ok(())
}
