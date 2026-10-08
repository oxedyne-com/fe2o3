//! The paced outbox: a sign-up queues its confirmation and answers, the drainer sends at the host's
//! ceiling and no faster, and nobody is sent to who is no longer owed it.
//!
//! These run the drainer's own `step` against a real Ozone instance with a recording sender and a
//! clock the test sets, so an hour passes in an instant and no mail is dialled.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_iop_db::api::Database;
use oxedyne_fe2o3_jdat::prelude::*;
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
            Stats,
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
            Subscriber,
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
    time::Instant,
};

mod common;

const DAY:      u64 = 86_400;

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
    // The request wrote one queue entry and no record: the sign-up is applied when the entry is drained.
    assert!(res!(subscribe::get(&handle, "a@nowhere.invalid")).is_none(), "the request wrote a record");
    assert_eq!(res!(outbox::queued(&handle, Kind::Confirm)), 1, "the sign-up was not queued");
    let (fake, pacer) = (Fake::default(), Pacer::new(100));
    res!(run_hour(&handle, &fake, &pacer).await);
    let sub = res!(subscribe::get(&handle, "a@nowhere.invalid"));
    let sub = res!(sub.ok_or_else(|| err!("the drainer stored no subscriber"; Test, Missing)));
    assert_eq!(sub.state, SubState::Pending);
    assert!(sub.sent.is_some(), "the confirmation left and no send was recorded");
    assert_eq!(fake.count(), 1);
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

// A1: one backlog episode is told once however the head's age wobbles round the threshold. The queue
// never empties here, so the episode never ends.
#[test]
fn test_a_backlog_that_wobbles_round_the_threshold_is_told_once_05() -> Outcome<()> {
    let mut watch = Watch::default();
    let (limit, base) = (3600u64, T0 / 1000);
    let mut told = Vec::new();
    for i in 0..100u64 {
        let now = base + i * 36;
        let age = if i % 2 == 0 { limit + 20 } else { limit - 20 };
        let st = Stats { queued: 100, oldest: Some(now - age) };
        if let Some(e) = watch.note("site", &st, 100, limit, now) {
            told.push(e);
        }
    }
    assert_eq!(told.len(), 1, "a queue that never drained told the operator {} times", told.len());
    assert!(matches!(told[0], AlertEvent::OutboxBacklog { .. }), "the one telling was {:?}", told[0]);
    Ok(())
}

// A2: three sign-ups for one address, queued over three days behind a hold, leave as one
// confirmation when the hold lifts: "1 a day" is judged when the mail goes, not when it is queued.
#[tokio::test]
async fn test_three_signups_queued_behind_a_hold_send_one_confirmation_06() -> Outcome<()> {
    let (db, uid, _tmp) = res!(common::test_db());
    let handle = (db, uid);
    let cfg = PublishConfig {
        confirm_interval_secs:  DAY,
        confirm_max:            3,
        confirm_window_days:    30,
        ..cfg()
    };
    let base = T0 / 1000;
    for d in 0..3u64 {
        res!(outbox::push(&handle, &[Entry::new(Kind::Confirm, "v@site.test", "", base + d * DAY)]));
    }
    let (fake, pacer) = (Fake::default(), Pacer::new(100));
    let mut now = T0 + 3 * DAY * 1000;
    for _ in 0..20 {
        match res!(outbox::step(&handle, &cfg, &fake, &pacer, "test", now).await) {
            Step::Wait(ms)  => now += ms,
            Step::Idle      => break,
            _               => {},
        }
    }
    assert_eq!(fake.count(), 1, "three sign-ups in a hold sent {} confirmations", fake.count());
    assert_eq!(res!(outbox::queued(&handle, Kind::Confirm)), 0, "a repeat sign-up stayed queued");
    Ok(())
}

fn median(mut v: Vec<u128>) -> u128 {
    v.sort();
    v[v.len() / 2]
}

// A7: the reply takes the same time whether the address is new or on the list, at 5,000 subscribers.
#[tokio::test]
async fn test_a_new_address_and_a_confirmed_one_are_answered_in_the_same_time_07() -> Outcome<()> {
    let (db, uid, _tmp) = res!(common::test_db());
    let handle = (db, uid);
    let n = 5000;
    let mut confirmed = Vec::new();
    {
        let guard = lock_read!(handle.0);
        for i in 0..n {
            let email = fmt!("c{:05}@site.test", i);
            let sub = Subscriber {
                email:      email.clone(),
                state:      SubState::Confirmed,
                token:      subscribe::mint_token(),
                created:    None,
                sent:       None,
            };
            res!(guard.insert(dat!(fmt!("{}{}", subscribe::KEY_PREFIX, email)), sub.to_dat(), handle.1, None));
            confirmed.push(email);
        }
        res!(guard.insert(dat!(subscribe::INDEX_KEY), store::names_dat(&confirmed), handle.1, None));
    }
    let sender = res!(MailSender::new("mail.site.test".to_string(), Vec::new(), "news@site.test".to_string()));
    let mail = Some(Arc::new(sender.with_outbound_hourly(0)));
    let (cfg, hdrs) = (cfg(), HeaderFields::default());
    let (mut fresh, mut known) = (Vec::new(), Vec::new());
    for i in 0..60usize {
        let new_body = fmt!("email=new{}%40site.test", i);
        let old_body = fmt!("email={}", confirmed[i * 7].replace('@', "%40"));
        let t = Instant::now();
        res!(subscribe::handle_subscribe(&cfg, Some(&handle), &mail, &hdrs, new_body.as_bytes(), None, "t").await);
        fresh.push(t.elapsed().as_micros());
        let t = Instant::now();
        res!(subscribe::handle_subscribe(&cfg, Some(&handle), &mail, &hdrs, old_body.as_bytes(), None, "t").await);
        known.push(t.elapsed().as_micros());
    }
    let (mf, mk) = (median(fresh.clone()), median(known.clone()));
    fresh.sort();
    known.sort();
    println!("A7 at {} subscribers: new median {} us (p10 {}, p90 {}), confirmed median {} us (p10 {}, p90 {})",
        n, mf, fresh[6], fresh[54], mk, known[6], known[54]);
    assert!(fresh[6] <= known[54] && known[6] <= fresh[54],
        "the reply times of a new ({} us) and a confirmed ({} us) address do not overlap", mf, mk);
    Ok(())
}

// B-5: a pass over a long lane of backing-off entries never keeps the write lock for the whole lane.
#[tokio::test]
async fn test_a_long_backing_off_lane_does_not_hold_the_write_lock_08() -> Outcome<()> {
    let (db, uid, _tmp) = res!(common::test_db());
    let handle = (db, uid);
    let base = T0 / 1000;
    let mut entries = Vec::new();
    for i in 0..3000 {
        let mut e = Entry::new(Kind::News, &fmt!("n{}@site.test", i), "post", base);
        e.next_try = base + 3600;
        entries.push(e);
    }
    res!(outbox::push(&handle, &entries));
    let (fake, pacer, cfg) = (Fake::default(), Pacer::new(100), cfg());
    let done = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let (probe_db, flag) = ((handle.0.clone(), handle.1), done.clone());
    let probe = std::thread::spawn(move || {
        let mut worst = std::time::Duration::ZERO;
        while !flag.load(std::sync::atomic::Ordering::Relaxed) {
            let t = Instant::now();
            let _ = store::exclusive(&probe_db, |_, _| -> Outcome<()> { Ok(()) });
            worst = worst.max(t.elapsed());
            std::thread::sleep(std::time::Duration::from_micros(200));
        }
        worst
    });
    std::thread::sleep(std::time::Duration::from_millis(20));
    let t = Instant::now();
    res!(outbox::step(&handle, &cfg, &fake, &pacer, "test", T0).await);
    let total = t.elapsed();
    done.store(true, std::sync::atomic::Ordering::Relaxed);
    let waited = probe.join().unwrap_or_default();
    println!("B-5: a pass over 3000 backing-off entries took {:?}, the longest wait for the lock was {:?}",
        total, waited);
    assert!(waited * 3 < total, "a writer waited {:?} of a {:?} pass", waited, total);
    Ok(())
}
