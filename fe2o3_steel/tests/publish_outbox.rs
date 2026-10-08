//! The paced outbox: a sign-up queues its confirmation and answers, the drainer sends at the host's
//! ceiling and no faster, and nobody is sent to who is no longer owed it.
//!
//! These run the drainer's own `step` against a real Ozone instance with a recording sender and a
//! clock the test sets, so an hour passes in an instant and no mail is dialled.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_net::http::fields::HeaderFields;
use oxedyne_fe2o3_steel::srv::{
    id,
    publish::{
        PublishConfig,
        Source,
        outbox::{
            self,
            Courier,
            Entry,
            Kind,
            Pacer,
            Step,
        },
        rate::Window,
        send::MailSender,
        subscribe::{
            self,
            SubState,
        },
    },
};

use std::{
    future::Future,
    sync::{
        Arc,
        Mutex,
        RwLock,
    },
};

mod common;

const SITE:     &str = "https://site.test";
const HOUR_MS:  u64 = 3_600_000;
const T0:       u64 = 1_800_000_000_000;

type Handle = (Arc<RwLock<common::TestDb>>, id::Uid);

// A sender that records who it was asked to send to and dials nobody.
#[derive(Default)]
struct Fake {
    to: Mutex<Vec<String>>,
}

impl Fake {
    fn count(&self) -> usize {
        self.to.lock().map(|v| v.len()).unwrap_or(0)
    }
}

impl Courier for Fake {

    fn default_from(&self) -> &str {
        "news@site.test"
    }

    fn deliver(&self, _from: &str, to: &str, _msg: &str)
        -> impl Future<Output = Outcome<String>> + Send
    {
        let to = to.to_string();
        async move {
            let mut v = lock_mutex!(self.to);
            v.push(to);
            Ok(fmt!("q1"))
        }
    }
}

fn cfg() -> PublishConfig {
    PublishConfig {
        path:       fmt!("/news"),
        base_url:   fmt!("{}", SITE),
        source:     Source::Store,
        ..Default::default()
    }
}

// `n` pending subscribers, each with a confirmation queued and due at once.
fn queue_pending(db: &Handle, n: usize) -> Outcome<Vec<String>> {
    let mut emails = Vec::new();
    let mut entries = Vec::new();
    for i in 0..n {
        let email = fmt!("p{}@site.test", i);
        res!(subscribe::add_pending(db, &email, &Window::default(), 1));
        entries.push(Entry::new(Kind::Confirm, &email, "", 0));
        emails.push(email);
    }
    res!(outbox::push(db, &entries));
    Ok(emails)
}

// Runs the drainer for one simulated hour from `T0`, jumping the clock to each slot it is told of.
async fn run_hour(db: &Handle, fake: &Fake, pacer: &Pacer) -> Outcome<()> {
    let cfg = cfg();
    let mut now = T0;
    loop {
        match res!(outbox::step(db, &cfg, fake, pacer, "test", now).await) {
            Step::Worked    => {},
            Step::Wait(ms)  => {
                if now + ms >= T0 + HOUR_MS {
                    break;
                }
                now += ms;
            },
            _               => break,
        }
    }
    Ok(())
}

#[tokio::test]
async fn test_a_signup_queues_its_confirmation_and_sends_nothing_00() -> Outcome<()> {
    let (db, uid, _tmp) = res!(common::test_db());
    let handle = (db, uid);
    let sender = res!(MailSender::new("mail.site.test".to_string(), Vec::new(), "news@site.test".to_string()));
    let mail = Some(Arc::new(sender));
    let _ = res!(subscribe::handle_subscribe(
        &cfg(), Some(&handle), &mail, &HeaderFields::default(), b"email=a%40nowhere.invalid", None, "test",
    ).await);
    let sub = res!(subscribe::get(&handle, "a@nowhere.invalid"));
    let sub = res!(sub.ok_or_else(|| err!("the sign-up stored no subscriber"; Test, Missing)));
    assert_eq!(sub.state, SubState::Pending, "the request dialled out and changed the record");
    assert_eq!(sub.sent, None, "a send was recorded though none had left");
    assert_eq!(res!(outbox::queued(&handle, Kind::Confirm)), 1, "the confirmation was not queued");
    Ok(())
}

#[tokio::test]
async fn test_the_ceiling_bounds_what_the_drainer_sends_in_an_hour_01() -> Outcome<()> {
    let (db, uid, _tmp) = res!(common::test_db());
    let handle = (db, uid);
    let emails = res!(queue_pending(&handle, 10));
    let (fake, pacer) = (Fake::default(), Pacer::new(3));
    res!(run_hour(&handle, &fake, &pacer).await);
    assert_eq!(fake.count(), 3, "a ceiling of 3 an hour did not send exactly 3 in the hour");
    // The time of the send is on the record, and only on the ones that left.
    let sent = emails.iter()
        .filter(|e| matches!(subscribe::get(&handle, e), Ok(Some(s)) if s.sent.is_some()))
        .count();
    assert_eq!(sent, 3, "the sent time was not recorded on exactly the three that left");
    assert_eq!(res!(outbox::queued(&handle, Kind::Confirm)), 7);

    let (fake, pacer) = (Fake::default(), Pacer::new(0));
    res!(run_hour(&handle, &fake, &pacer).await);
    assert_eq!(fake.count(), 0, "a ceiling of 0 sent mail");
    assert_eq!(res!(outbox::queued(&handle, Kind::Confirm)), 7, "a held queue lost entries");
    Ok(())
}

#[tokio::test]
async fn test_a_record_no_longer_pending_is_skipped_02() -> Outcome<()> {
    let (db, uid, _tmp) = res!(common::test_db());
    let handle = (db, uid);
    let emails = res!(queue_pending(&handle, 2));
    // The first confirms, by its token, before the drainer reaches it.
    let sub = res!(subscribe::get(&handle, &emails[0]));
    let sub = res!(sub.ok_or_else(|| err!("no subscriber"; Test, Missing)));
    let _ = res!(subscribe::confirm(&handle, &sub.token, "test"));
    let (fake, pacer) = (Fake::default(), Pacer::new(3600));
    res!(run_hour(&handle, &fake, &pacer).await);
    let sent = fake.to.lock().map(|v| v.clone()).unwrap_or_default();
    assert_eq!(sent, vec![emails[1].clone()], "a confirmed address was sent a confirmation");
    assert_eq!(res!(outbox::queued(&handle, Kind::Confirm)), 0, "the skipped entry stayed queued");
    Ok(())
}
