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
    alert::AlertEvent,
    id,
    publish::{
        Markup,
        PostState,
        PublishConfig,
        Source,
        outbox::{
            self,
            Courier,
            Entry,
            Kind,
            Pacer,
            Step,
            Watch,
        },
        rate::Window,
        send::{
            self,
            MailSender,
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
            Step::Sent | Step::Worked   => {},
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

#[tokio::test]
async fn test_a_backlog_is_told_once_and_cleared_once_when_it_drains_03() -> Outcome<()> {
    let (db, uid, _tmp) = res!(common::test_db());
    let handle = (db, uid);
    // Three confirmations queued at T0, behind a ceiling held at 0.
    let mut entries = Vec::new();
    for i in 0..3 {
        let email = fmt!("b{}@site.test", i);
        res!(subscribe::add_pending(&handle, &email, &Window::default(), 1));
        entries.push(Entry::new(Kind::Confirm, &email, "", T0 / 1000));
    }
    res!(outbox::push(&handle, &entries));
    let cfg = PublishConfig { outbox_alert_secs: 3600, ..cfg() };
    let (fake, held, mut watch) = (Fake::default(), Pacer::new(0), Watch::default());
    let mut told = Vec::new();
    // Three hours held, polled every ten minutes.
    for m in (0..=180u64).step_by(10) {
        let (_, owed) = res!(outbox::tick(
            &handle, &cfg, &fake, &held, &mut watch, "test", T0 + m * 60_000).await);
        if let Some(e) = owed {
            told.push((m, e));
        }
    }
    assert_eq!(told.len(), 1, "a backlog lasting three hours was told {} times, not once", told.len());
    assert_eq!(told[0].0, 70, "the backlog was not told at the first poll past the threshold");
    assert!(matches!(&told[0].1, AlertEvent::OutboxBacklog { queued: 3, hourly: 0, .. }),
        "the backlog told the wrong thing: {:?}", told[0].1);
    assert_eq!(fake.count(), 0, "a held ceiling sent mail");

    // The ceiling is raised, and the queue drains.
    let open = Pacer::new(3600);
    let mut now = T0 + 180 * 60_000;
    let mut cleared = Vec::new();
    for _ in 0..20 {
        let (stepped, owed) = res!(outbox::tick(
            &handle, &cfg, &fake, &open, &mut watch, "test", now).await);
        if let Some(e) = owed {
            cleared.push(e);
        }
        match stepped {
            Step::Wait(ms)  => now += ms,
            Step::Idle      => break,
            _               => {},
        }
    }
    assert_eq!(fake.count(), 3, "the released queue did not send all three");
    assert_eq!(cleared.len(), 1, "draining told {} things, not one", cleared.len());
    assert!(matches!(&cleared[0], AlertEvent::OutboxCleared { sent: 3, .. }),
        "draining told the wrong thing: {:?}", cleared[0]);
    Ok(())
}

#[tokio::test]
async fn test_a_newsletter_is_paced_by_the_ceiling_and_waits_behind_confirmations_04() -> Outcome<()> {
    let (db, uid, _tmp) = res!(common::test_db());
    let handle = (db, uid);
    let rec = Record {
        slug:       fmt!("on-rent"),
        author:     String::new(),
        categories: Vec::new(),
        state:      PostState::Live,
        markup:     Markup::Markdown,
        date:       Some(fmt!("2026-07-17")),
        source:     fmt!("# On rent\n\nAn opening sentence.\n"),
        deliveries: Vec::new(),
        tags:       Vec::new(),
        ai_level:   None,
    };
    res!(store::put(&handle, &rec, "test"));
    // Ten confirmed subscribers.
    for i in 0..10 {
        let email = fmt!("n{}@site.test", i);
        res!(subscribe::add_pending(&handle, &email, &Window::default(), 1));
        let sub = res!(subscribe::get(&handle, &email));
        let sub = res!(sub.ok_or_else(|| err!("no subscriber"; Test, Missing)));
        let _ = res!(subscribe::confirm(&handle, &sub.token, "test"));
    }
    let sender = res!(MailSender::new(
        "mail.site.test".to_string(), Vec::new(), "news@site.test".to_string()));
    let sender = sender.with_outbound_hourly(3);
    let report = res!(send::send_newsletter(&sender, &handle, "on-rent", "test"));
    assert_eq!(report.attempted, 10, "the newsletter was not queued for all ten");
    assert_eq!(res!(outbox::queued(&handle, Kind::News)), 10);
    // A sign-up queued after the newsletter.
    res!(subscribe::add_pending(&handle, "late@site.test", &Window::default(), 1));
    res!(outbox::push(&handle, &[Entry::new(Kind::Confirm, "late@site.test", "", T0 / 1000)]));

    let fake = Fake::default();
    res!(run_hour(&handle, &fake, sender.pacer()).await);
    let to = fake.to.lock().map(|v| v.clone()).unwrap_or_default();
    assert_eq!(to.len(), 3, "a ceiling of 3 an hour sent {} messages in the hour", to.len());
    assert_eq!(to[0], "late@site.test", "the newsletter went before the confirmation");
    let hist = res!(send::send_history(&handle));
    assert_eq!(hist.len(), 1);
    assert_eq!((hist[0].attempted, hist[0].sent, hist[0].waiting()), (10, 2, 8),
        "the send's tally did not follow the queue");
    assert_eq!(res!(outbox::queued(&handle, Kind::News)), 8);
    Ok(())
}
