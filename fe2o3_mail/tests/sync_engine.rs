//! The folder sync engine over a scripted [`MailSession`]: an in-memory model of an IMAP server
//! whose capabilities choose the tier, with a log of every verb so each test can say which wire
//! commands the engine used. The `NO-LOSS` and `RESET` laws are stated as unit tests here: an
//! expunge on the server appears as vanished, a UIDVALIDITY change yields one generation and no
//! duplicates, a replayed apply is a no-op, and a delta never undoes a pending local change.

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_mail::sync::{
    act,
    apply,
    file_sent,
    find_message_id,
    link_key,
    maildir_name,
    parse_maildir_name,
    sync_folder,
    ActOutcome,
    Action,
    AppendUid,
    Budget,
    Caps,
    CopyUid,
    Cursor,
    Delta,
    Entry,
    FetchChanged,
    FetchWhat,
    FetchedMessage,
    Filed,
    LinkKey,
    MailSession,
    MailboxInfo,
    MailboxStatus,
    MsgKey,
    Provider,
    Roles,
    SelectWith,
    Snapshot,
    SpecialUse,
    SyncStore,
    Target,
    Tier,
    UidSet,
};

use std::collections::BTreeMap;


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ THE SCRIPTED SERVER                                                       │
// └───────────────────────────────────────────────────────────────────────────┘

#[derive(Clone, Debug)]
struct Msg {
    uid:    u32,
    flags:  Vec<String>,
    raw:    Vec<u8>,
    modseq: u64,
    gm:     u64,
}

#[derive(Clone, Debug, Default)]
struct Folder {
    attrs:      Vec<String>,
    uv:         u32,
    next:       u32,
    modseq:     u64,
    msgs:       Vec<Msg>,
    expunged:   Vec<(u32, u64)>,
}

#[derive(Default)]
struct Server {
    caps:       Vec<String>,
    folders:    BTreeMap<String, Folder>,
    open:       Option<String>,
    loose:      bool,       // SEARCH answers every UID, as Steel's did (B1)
    log:        Vec<String>,
    gm_next:    u64,
}

fn raw(mid: &str, year: u16) -> Vec<u8> {
    fmt!("Message-ID: <{}>\r\nDate: 1 Jan {} 09:00:00 +0000\r\nSubject: {}\r\n\r\nbody of {}\r\n",
        mid, year, mid, mid).into_bytes()
}

impl Server {
    fn new(caps: &str) -> Self {
        let mut s = Self { caps: caps.split_whitespace().map(String::from).collect(), ..Default::default() };
        s.folder("INBOX", "", 100);
        s
    }

    fn folder(&mut self, name: &str, attr: &str, uv: u32) {
        self.folders.insert(name.to_string(), Folder {
            attrs:  attr.split_whitespace().map(String::from).collect(),
            uv,
            next:   1,
            modseq: 1,
            ..Default::default()
        });
    }

    fn deliver(&mut self, mb: &str, mid: &str) -> u32 {
        self.gm_next += 1;
        let gm = self.gm_next;
        let f = self.folders.get_mut(mb).unwrap();
        f.modseq += 1;
        let uid = f.next;
        f.next += 1;
        f.msgs.push(Msg { uid, flags: Vec::new(), raw: raw(mid, 2025), modseq: f.modseq, gm });
        uid
    }

    fn set_flags(&mut self, mb: &str, uid: u32, flags: &[&str]) {
        let f = self.folders.get_mut(mb).unwrap();
        f.modseq += 1;
        let m = f.msgs.iter_mut().find(|m| m.uid == uid).unwrap();
        m.flags = flags.iter().map(|s| s.to_string()).collect();
        m.modseq = f.modseq;
    }

    fn expunge(&mut self, mb: &str, uid: u32) {
        let f = self.folders.get_mut(mb).unwrap();
        f.modseq += 1;
        f.msgs.retain(|m| m.uid != uid);
        f.expunged.push((uid, f.modseq));
    }

    /// The folder is rebuilt under a new UIDVALIDITY with the same messages renumbered.
    fn renumber(&mut self, mb: &str, uv: u32) {
        let f = self.folders.get_mut(mb).unwrap();
        f.uv = uv;
        f.expunged.clear();
        for (i, m) in f.msgs.iter_mut().enumerate() {
            m.uid = 50 + i as u32;
        }
        f.next = 50 + f.msgs.len() as u32;
    }

    fn has(&self, mb: &str, mid: &str) -> bool {
        let r = raw(mid, 2025);
        self.folders[mb].msgs.iter().any(|m| m.raw == r)
    }

    fn cur(&mut self) -> Outcome<&mut Folder> {
        let o = res!(self.open.clone().ok_or_else(|| err!("No mailbox selected."; Missing)));
        self.folders.get_mut(&o).ok_or_else(|| err!("No such mailbox {}.", o; Missing))
    }

    fn cap(&self, c: &str) -> bool {
        self.caps.iter().any(|x| x == c)
    }

    fn fetched(m: &Msg, what: FetchWhat, gmail: bool) -> FetchedMessage {
        let head_end = m.raw.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4;
        let body = match what {
            FetchWhat::Flags | FetchWhat::Meta  => Vec::new(),
            FetchWhat::Headers                  => m.raw[..head_end].to_vec(),
            FetchWhat::Full                     => m.raw.clone(),
            FetchWhat::MessageId                => {
                let t = String::from_utf8_lossy(&m.raw).to_string();
                let line = t.lines().find(|l| l.starts_with("Message-ID")).unwrap().to_string();
                fmt!("{}\r\n\r\n", line).into_bytes()
            }
        };
        FetchedMessage {
            uid:            m.uid,
            flags:          m.flags.clone(),
            internal_date:  "01-Jan-2025 09:00:00 +0000".to_string(),
            size:           m.raw.len() as u32,
            body,
            modseq:         Some(m.modseq),
            gm_msgid:       if gmail { Some(m.gm) } else { None },
            ..Default::default()
        }
    }
}

impl MailSession for Server {
    async fn capabilities(&mut self) -> Outcome<Vec<String>> {
        Ok(self.caps.clone())
    }

    async fn list(&mut self) -> Outcome<Vec<MailboxInfo>> {
        Ok(self.folders.iter().map(|(n, f)| MailboxInfo {
            name: n.clone(), delimiter: Some('/'), attrs: f.attrs.clone(),
        }).collect())
    }

    async fn create(&mut self, mb: &str) -> Outcome<()> {
        self.log.push(fmt!("CREATE {}", mb));
        self.folder(mb, "", 900);
        Ok(())
    }

    async fn select(&mut self, mb: &str, with: &SelectWith) -> Outcome<MailboxStatus> {
        self.log.push(fmt!("SELECT {} {}", mb, match with {
            SelectWith::Plain           => "",
            SelectWith::CondStore       => "CONDSTORE",
            SelectWith::QResync { .. }  => "QRESYNC",
        }));
        let modseqs = self.cap("CONDSTORE") || self.cap("QRESYNC");
        let f = res!(self.folders.get(mb).ok_or_else(|| err!("No mailbox {}.", mb; Missing)));
        let mut st = MailboxStatus {
            name:           mb.to_string(),
            exists:         f.msgs.len() as u32,
            uid_validity:   f.uv,
            uid_next:       f.next,
            highest_modseq: if modseqs { Some(f.modseq) } else { None },
            ..Default::default()
        };
        if let SelectWith::QResync { uid_validity, modseq, known } = with {
            if *uid_validity == f.uv {
                st.changed = f.msgs.iter().filter(|m| m.modseq > *modseq)
                    .map(|m| Server::fetched(m, FetchWhat::Flags, false)).collect();
                for (u, ms) in &f.expunged {
                    if *ms > *modseq && known.as_ref().map(|k| k.contains(*u)).unwrap_or(true) {
                        st.vanished.insert(*u);
                    }
                }
            }
        }
        self.open = Some(mb.to_string());
        Ok(st)
    }

    async fn uid_fetch(&mut self, uids: &UidSet, what: FetchWhat) -> Outcome<Vec<FetchedMessage>> {
        self.log.push(fmt!("UID FETCH {} {:?}", uids, what));
        let gmail = self.cap("X-GM-EXT-1");
        let f = res!(self.cur());
        let mut out: Vec<FetchedMessage> = f.msgs.iter().filter(|m| uids.contains(m.uid))
            .map(|m| Server::fetched(m, what, gmail)).collect();
        // `n:*` past the last UID answers with the last message (RFC 3501 6.4.8).
        if out.is_empty() && uids.last() == Some(u32::MAX) {
            if let Some(m) = f.msgs.last() {
                out.push(Server::fetched(m, what, gmail));
            }
        }
        Ok(out)
    }

    async fn uid_fetch_changed(&mut self, uids: &UidSet, what: FetchWhat, modseq: u64)
        -> Outcome<FetchChanged>
    {
        self.log.push(fmt!("UID FETCH {} {:?} CHANGEDSINCE {}", uids, what, modseq));
        let f = res!(self.cur());
        Ok(FetchChanged {
            messages: f.msgs.iter().filter(|m| uids.contains(m.uid) && m.modseq > modseq)
                .map(|m| Server::fetched(m, what, false)).collect(),
            vanished: UidSet::new(),
        })
    }

    async fn uid_search(&mut self, criteria: &str) -> Outcome<Vec<u32>> {
        self.log.push(fmt!("UID SEARCH {}", criteria));
        let loose = self.loose;
        let f = res!(self.cur());
        let all: Vec<u32> = f.msgs.iter().map(|m| m.uid).collect();
        if criteria == "ALL" || loose {
            return Ok(all);
        }
        if let Some(set) = criteria.strip_prefix("UID ") {
            let set = res!(UidSet::parse(set));
            return Ok(all.into_iter().filter(|u| set.contains(*u)).collect());
        }
        if let Some(mid) = criteria.strip_prefix("HEADER Message-ID ") {
            let mid = mid.trim_matches('"');
            return Ok(f.msgs.iter()
                .filter(|m| String::from_utf8_lossy(&m.raw).contains(mid))
                .map(|m| m.uid).collect());
        }
        Err(err!("Unscripted search {}.", criteria; NoImpl))
    }

    async fn uid_store_flags(&mut self, uids: &UidSet, add: &[&str], remove: &[&str]) -> Outcome<()> {
        self.log.push(fmt!("UID STORE {} +{:?} -{:?}", uids, add, remove));
        let f = res!(self.cur());
        f.modseq += 1;
        let ms = f.modseq;
        for m in f.msgs.iter_mut().filter(|m| uids.contains(m.uid)) {
            m.flags.retain(|x| !remove.contains(&x.as_str()));
            for a in add {
                if !m.flags.iter().any(|x| x == a) {
                    m.flags.push(a.to_string());
                }
            }
            m.modseq = ms;
        }
        Ok(())
    }

    async fn uid_copy(&mut self, uids: &UidSet, to: &str) -> Outcome<Option<CopyUid>> {
        self.log.push(fmt!("UID COPY {} {}", uids, to));
        let src: Vec<Msg> = res!(self.cur()).msgs.iter().filter(|m| uids.contains(m.uid)).cloned().collect();
        let d = res!(self.folders.get_mut(to).ok_or_else(|| err!("No mailbox {}.", to; Missing)));
        for mut m in src {
            m.uid = d.next;
            d.next += 1;
            d.modseq += 1;
            m.modseq = d.modseq;
            d.msgs.push(m);
        }
        Ok(None)
    }

    async fn uid_move(&mut self, uids: &UidSet, to: &str) -> Outcome<Option<CopyUid>> {
        if !self.cap("MOVE") {
            return Err(err!("MOVE not offered."; NoImpl));
        }
        let c = res!(self.uid_copy(uids, to).await);
        self.log.push(fmt!("(the copy was a MOVE)"));
        let o = self.open.clone().unwrap();
        for u in uids.iter() {
            self.expunge(&o, u);
        }
        Ok(c)
    }

    async fn uid_expunge(&mut self, uids: &UidSet) -> Outcome<()> {
        self.log.push(fmt!("UID EXPUNGE {}", uids));
        let o = self.open.clone().unwrap();
        let del: Vec<u32> = self.folders[&o].msgs.iter()
            .filter(|m| uids.contains(m.uid) && m.flags.iter().any(|f| f == "\\Deleted"))
            .map(|m| m.uid).collect();
        for u in del {
            self.expunge(&o, u);
        }
        Ok(())
    }

    async fn append(&mut self, mb: &str, _flags: &[&str], raw: &[u8]) -> Outcome<Option<AppendUid>> {
        self.log.push(fmt!("APPEND {}", mb));
        let f = res!(self.folders.get_mut(mb).ok_or_else(|| err!("No mailbox {}.", mb; Missing)));
        f.modseq += 1;
        let uid = f.next;
        f.next += 1;
        f.msgs.push(Msg { uid, flags: vec!["\\Seen".into()], raw: raw.to_vec(), modseq: f.modseq, gm: 0 });
        Ok(Some(AppendUid { uid_validity: f.uv, uid }))
    }
}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ THE DEVICE'S STORE                                                        │
// └───────────────────────────────────────────────────────────────────────────┘

#[derive(Clone, Debug, Default, PartialEq)]
struct Gen {
    uv:     u32,
    msgs:   BTreeMap<String, Vec<u8>>,  // Maildir name to bytes
}

#[derive(Clone, Debug, Default, PartialEq)]
struct Held {
    gen:    Option<Gen>,
    aside:  Option<Gen>,
    cursor: Option<Cursor>,
}

#[derive(Default)]
struct Store {
    folders:    BTreeMap<String, Held>,
    fail_put:   bool,
}

impl Store {
    fn names(&self, mb: &str) -> Vec<String> {
        self.folders.get(mb).and_then(|h| h.gen.as_ref())
            .map(|g| g.msgs.keys().cloned().collect()).unwrap_or_default()
    }

    fn find(&self, mb: &str, key: MsgKey) -> Option<String> {
        self.names(mb).into_iter().find(|n| parse_maildir_name(n).map(|p| p.0) == Some(key))
    }

    fn gen(&mut self, mb: &str) -> Outcome<&mut Gen> {
        self.folders.get_mut(mb).and_then(|h| h.gen.as_mut())
            .ok_or_else(|| err!("{} has no generation.", mb; Missing))
    }

    /// The held set and cursor to send next, as a device would.
    fn snapshot(&self, mb: &str) -> Snapshot {
        let held: Vec<u32> = self.names(mb).iter()
            .filter_map(|n| parse_maildir_name(n)).map(|p| p.0.uid).collect();
        let held = UidSet::from(&held);
        Snapshot {
            mailbox:    mb.to_string(),
            cursor:     self.folders.get(mb).and_then(|h| h.cursor.clone()).unwrap_or_default(),
            window:     UidSet::new(),
            held,
        }
    }

    fn flags_of(&self, mb: &str, uid: u32) -> Vec<String> {
        let uv = self.folders[mb].gen.as_ref().unwrap().uv;
        let n = self.find(mb, MsgKey { uid_validity: uv, uid }).unwrap();
        parse_maildir_name(&n).unwrap().1
    }
}

impl SyncStore for Store {
    fn generation(&self, mb: &str) -> Outcome<Option<u32>> {
        Ok(self.folders.get(mb).and_then(|h| h.gen.as_ref()).map(|g| g.uv))
    }

    fn begin_generation(&mut self, mb: &str, uv: u32) -> Outcome<()> {
        let h = self.folders.entry(mb.to_string()).or_default();
        h.aside = h.gen.take();
        h.gen = Some(Gen { uv, ..Default::default() });
        Ok(())
    }

    fn end_generation(&mut self, mb: &str) -> Outcome<()> {
        if let Some(h) = self.folders.get_mut(mb) {
            h.aside = None;
        }
        Ok(())
    }

    fn flags(&self, mb: &str, key: MsgKey) -> Outcome<Option<Vec<String>>> {
        Ok(self.find(mb, key).and_then(|n| parse_maildir_name(&n)).map(|p| p.1))
    }

    fn put(&mut self, mb: &str, key: MsgKey, flags: &[String], raw: &[u8], _whole: bool)
        -> Outcome<bool>
    {
        if self.fail_put {
            return Err(err!("The disk is full."; IO));
        }
        if self.find(mb, key).is_some() {
            return Ok(false);
        }
        res!(self.gen(mb)).msgs.insert(maildir_name(key, flags), raw.to_vec());
        Ok(true)
    }

    fn change_flags(&mut self, mb: &str, key: MsgKey, add: &[String], remove: &[String])
        -> Outcome<()>
    {
        let old = res!(self.find(mb, key).ok_or_else(|| err!("Not held."; Missing)));
        let mut f = parse_maildir_name(&old).unwrap().1;
        f.retain(|x| !remove.contains(x));
        f.extend(add.iter().cloned());
        let g = res!(self.gen(mb));
        let bytes = g.msgs.remove(&old).unwrap();
        g.msgs.insert(maildir_name(key, &f), bytes);
        Ok(())
    }

    fn remove(&mut self, mb: &str, key: MsgKey) -> Outcome<bool> {
        match self.find(mb, key) {
            Some(n) => { res!(self.gen(mb)).msgs.remove(&n); Ok(true) }
            None    => Ok(false),
        }
    }

    fn cursor(&self, mb: &str) -> Outcome<Option<Cursor>> {
        Ok(self.folders.get(mb).and_then(|h| h.cursor.clone()))
    }

    fn set_cursor(&mut self, mb: &str, c: &Cursor) -> Outcome<()> {
        self.folders.entry(mb.to_string()).or_default().cursor = Some(c.clone());
        Ok(())
    }
}

/// One sync and its apply, as the client drives it.
async fn round(s: &mut Server, st: &mut Store, budget: &Budget, pending: &[Entry]) -> Outcome<Delta> {
    let caps = Caps::from_list(&res!(s.capabilities().await));
    let d = res!(sync_folder(s, &caps, &st.snapshot("INBOX"), budget).await);
    res!(apply(st, &d, pending));
    Ok(d)
}

const QRESYNC: &str = "IMAP4rev1 CONDSTORE QRESYNC MOVE UIDPLUS SPECIAL-USE";
const CONDSTORE: &str = "IMAP4rev1 CONDSTORE MOVE UIDPLUS SPECIAL-USE X-GM-EXT-1";
const PLAIN: &str = "IMAP4rev1 MOVE UIDPLUS SPECIAL-USE";


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ TIERS, AND NO-LOSS                                                        │
// └───────────────────────────────────────────────────────────────────────────┘

/// Expunge and flag change on the server, then the tier's delta; returns the server's log of the
/// second sync.
async fn expunge_and_flag(caps: &str) -> Outcome<(Delta, Vec<String>, Store)> {
    let mut s = Server::new(caps);
    for i in 1..=4 {
        s.deliver("INBOX", &fmt!("m{}@x", i));
    }
    let mut st = Store::default();
    let b = Budget::default();
    let first = res!(round(&mut s, &mut st, &b, &[]).await);
    req!(4, first.new.len());
    req!(4, st.names("INBOX").len());
    s.expunge("INBOX", 2);
    s.set_flags("INBOX", 3, &["\\Seen"]);
    s.log.clear();
    let d = res!(round(&mut s, &mut st, &b, &[]).await);
    Ok((d, s.log, st))
}

#[tokio::test]
async fn test_qresync_tier_reports_vanished_and_flags_from_select_00() -> Outcome<()> {
    let (d, log, st) = res!(expunge_and_flag(QRESYNC).await);
    req!(Some(Tier::QResync), d.tier);
    req!(UidSet::from([2]), d.vanished);
    req!(true, d.flags.iter().any(|f| f.uid == 3 && f.flags == vec!["\\Seen".to_string()]));
    req!(true, log[0].contains("QRESYNC"), "{:?}", log);
    req!(false, log.iter().any(|l| l.contains("SEARCH")), "{:?}", log);
    req!(3, st.names("INBOX").len());
    req!(vec!["\\Seen".to_string()], st.flags_of("INBOX", 3));
    Ok(())
}

#[tokio::test]
async fn test_condstore_tier_uses_changedsince_and_search_uid_00() -> Outcome<()> {
    let (d, log, st) = res!(expunge_and_flag(CONDSTORE).await);
    req!(Some(Tier::CondStore), d.tier);
    req!(UidSet::from([2]), d.vanished);
    req!(1, d.flags.len());
    req!(true, log.iter().any(|l| l.contains("CHANGEDSINCE")), "{:?}", log);
    req!(true, log.iter().any(|l| l.starts_with("UID SEARCH UID 1:4")), "{:?}", log);
    req!(3, st.names("INBOX").len());
    Ok(())
}

#[tokio::test]
async fn test_full_tier_reconciles_by_search_all_00() -> Outcome<()> {
    let (d, log, st) = res!(expunge_and_flag(PLAIN).await);
    req!(Some(Tier::Full), d.tier);
    req!(UidSet::from([2]), d.vanished);
    req!(true, log.iter().any(|l| l == "UID SEARCH ALL"), "{:?}", log);
    req!(3, st.names("INBOX").len());
    Ok(())
}

/// Tier 3 reads flags inside the window each poll, and over every held message each Nth, so an
/// old message read elsewhere does not stay unread here.
#[tokio::test]
async fn test_full_tier_sweeps_every_held_flag_each_nth_poll_00() -> Outcome<()> {
    let mut s = Server::new(PLAIN);
    for i in 1..=5 {
        s.deliver("INBOX", &fmt!("m{}@x", i));
    }
    let mut st = Store::default();
    let b = Budget { sweep_every: 3, ..Budget::default() };
    res!(round(&mut s, &mut st, &b, &[]).await);           // poll 0: fresh, nothing held yet
    s.set_flags("INBOX", 1, &["\\Seen"]);                   // old, outside the window
    let mut seen_at = None;
    for poll in 1..=4 {
        let caps = Caps::from_list(&s.caps);
        let mut snap = st.snapshot("INBOX");
        snap.window = UidSet::from([5]);
        let d = res!(sync_folder(&mut s, &caps, &snap, &b).await);
        res!(apply(&mut st, &d, &[]));
        if seen_at.is_none() && st.flags_of("INBOX", 1) == vec!["\\Seen".to_string()] {
            seen_at = Some(poll);
        }
    }
    // The fresh sync was the cursor's poll 0; polls 1 and 2 read the window only; poll 3 sweeps.
    req!(Some(3), seen_at);
    Ok(())
}

/// A big arrival is fetched newest first in bounded pieces, and none is skipped.
#[tokio::test]
async fn test_new_messages_bounded_newest_first_none_lost_00() -> Outcome<()> {
    let mut s = Server::new(QRESYNC);
    for i in 1..=7 {
        s.deliver("INBOX", &fmt!("m{}@x", i));
    }
    let mut st = Store::default();
    let b = Budget { max_count: 3, ..Budget::default() };
    let d = res!(round(&mut s, &mut st, &b, &[]).await);
    req!(vec![7, 6, 5], d.new.iter().map(|m| m.uid).collect::<Vec<_>>());
    req!(true, d.more);
    s.expunge("INBOX", 3);                                  // gone before it was fetched
    s.deliver("INBOX", "m8@x");
    let d = res!(round(&mut s, &mut st, &b, &[]).await);
    req!(vec![8, 4, 2], d.new.iter().map(|m| m.uid).collect::<Vec<_>>());
    let d = res!(round(&mut s, &mut st, &b, &[]).await);
    req!(vec![1], d.new.iter().map(|m| m.uid).collect::<Vec<_>>());
    req!(false, d.more);
    req!(7, st.names("INBOX").len());
    let d = res!(round(&mut s, &mut st, &b, &[]).await);
    req!(0, d.new.len());                                   // `9:*` answered with UID 8: dropped
    Ok(())
}

/// Only mail that came after a folder became known is an arrival: what a capped first sync left
/// behind is fetched later without being one, and so is a rebuild's backlog; a message delivered
/// meanwhile is one, and is fetched beside that backlog.
#[tokio::test]
async fn test_only_mail_after_the_first_sync_is_an_arrival_00() -> Outcome<()> {
    for caps in [QRESYNC, CONDSTORE, PLAIN] {
        let mut s = Server::new(caps);
        for i in 1..=7 {
            s.deliver("INBOX", &fmt!("m{}@x", i));
        }
        let mut st = Store::default();
        let b = Budget { max_count: 3, ..Budget::default() };
        let arrived = |d: &Delta| d.new.iter().filter(|m| m.arrival).map(|m| m.uid).collect::<Vec<_>>();
        let d = res!(round(&mut s, &mut st, &b, &[]).await);
        req!(Vec::<u32>::new(), arrived(&d), "{} first", caps);
        req!(7, d.cursor.floor, "{}", caps);
        s.deliver("INBOX", "m8@x");
        let d = res!(round(&mut s, &mut st, &b, &[]).await);
        req!(vec![8, 4, 3], d.new.iter().map(|m| m.uid).collect::<Vec<_>>(), "{}", caps);
        req!(vec![8], arrived(&d), "{} backlog beside a delivery", caps);
        let d = res!(round(&mut s, &mut st, &b, &[]).await);
        req!(vec![2, 1], d.new.iter().map(|m| m.uid).collect::<Vec<_>>(), "{}", caps);
        req!(Vec::<u32>::new(), arrived(&d), "{} completion", caps);
        req!(false, d.more);
        s.deliver("INBOX", "m9@x");
        let d = res!(round(&mut s, &mut st, &b, &[]).await);
        req!(vec![9], arrived(&d), "{} after", caps);
        // A rebuild brings the folder back under new UIDs, above the old floor: none arrives.
        s.renumber("INBOX", 777);
        loop {
            let d = res!(round(&mut s, &mut st, &b, &[]).await);
            req!(Vec::<u32>::new(), arrived(&d), "{} rebuild", caps);
            if !d.more { break; }
        }
        s.deliver("INBOX", "m10@x");
        let d = res!(round(&mut s, &mut st, &b, &[]).await);
        req!(1, arrived(&d).len(), "{} after the rebuild", caps);
    }
    Ok(())
}

/// Header fetches count against `max_bytes` as bodies do, so a request's bytes are bounded by
/// the budget and not by `max_count` times the header size; nothing is skipped meanwhile.
#[tokio::test]
async fn test_header_bytes_count_against_the_byte_budget_00() -> Outcome<()> {
    for bodies in [false, true] {
        let mut s = Server::new(QRESYNC);
        for i in 1..=12 {
            s.deliver("INBOX", &fmt!("m{}@x", i));
        }
        let mut st = Store::default();
        let b = Budget { max_count: 200, max_bytes: 250, bodies, ..Budget::default() };
        let mut rounds = 0;
        loop {
            let d = res!(round(&mut s, &mut st, &b, &[]).await);
            let bytes: u64 = d.new.iter().map(|m| m.raw.len() as u64).sum();
            req!(true, bytes <= b.max_bytes, "round {} carried {} bytes", rounds, bytes);
            req!(true, !d.new.is_empty() || !d.more);
            rounds += 1;
            if !d.more || rounds > 20 {
                break;
            }
        }
        req!(true, rounds > 1);
        req!(12, st.names("INBOX").len());
    }
    // A single header over the whole budget still moves: one message per request.
    let mut s = Server::new(QRESYNC);
    for i in 1..=3 {
        s.deliver("INBOX", &fmt!("m{}@x", i));
    }
    let mut st = Store::default();
    let b = Budget { max_count: 200, max_bytes: 10, ..Budget::default() };
    for want in [vec![3], vec![2], vec![1]] {
        let d = res!(round(&mut s, &mut st, &b, &[]).await);
        req!(want, d.new.iter().map(|m| m.uid).collect::<Vec<_>>());
    }
    req!(3, st.names("INBOX").len());
    Ok(())
}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ RESET, REPLAY, THE CURSOR AND THE JOURNAL                                 │
// └───────────────────────────────────────────────────────────────────────────┘

#[tokio::test]
async fn test_uidvalidity_reset_one_generation_no_duplicates_00() -> Outcome<()> {
    for caps in [QRESYNC, CONDSTORE, PLAIN] {
        let mut s = Server::new(caps);
        for i in 1..=5 {
            s.deliver("INBOX", &fmt!("m{}@x", i));
        }
        let mut st = Store::default();
        let b = Budget { max_count: 2, ..Budget::default() };
        while res!(round(&mut s, &mut st, &b, &[]).await).more {}
        s.renumber("INBOX", 777);
        let d = res!(round(&mut s, &mut st, &b, &[]).await);
        req!(Some(oxedyne_fe2o3_mail::sync::Reset { old: 100, new: 777 }), d.reset, "{}", caps);
        // The old generation is set aside, not mixed in, until the rebuild completes.
        req!(true, st.folders["INBOX"].aside.is_some());
        req!(2, st.names("INBOX").len());
        while res!(round(&mut s, &mut st, &b, &[]).await).more {}
        req!(true, st.folders["INBOX"].aside.is_none());
        let names = st.names("INBOX");
        req!(5, names.len(), "{}", caps);
        req!(true, names.iter().all(|n| n.starts_with("777.")), "{:?}", names);
    }
    Ok(())
}

#[tokio::test]
async fn test_replayed_apply_is_a_no_op_00() -> Outcome<()> {
    let mut s = Server::new(QRESYNC);
    for i in 1..=3 {
        s.deliver("INBOX", &fmt!("m{}@x", i));
    }
    let mut st = Store::default();
    let b = Budget::default();
    res!(round(&mut s, &mut st, &b, &[]).await);
    s.expunge("INBOX", 1);
    s.set_flags("INBOX", 2, &["\\Flagged"]);
    s.deliver("INBOX", "m4@x");
    let caps = Caps::from_list(&s.caps);
    let d = res!(sync_folder(&mut s, &caps, &st.snapshot("INBOX"), &b).await);
    let once = res!(apply(&mut st, &d, &[]));
    req!((1, 1, 1), (once.added, once.changed, once.removed));
    let after = st.folders.clone();
    let twice = res!(apply(&mut st, &d, &[]));
    req!((0, 0, 0), (twice.added, twice.changed, twice.removed));
    req!(true, after == st.folders);
    // A reset delta replayed does not set aside the rebuilt generation.
    s.renumber("INBOX", 555);
    let d = res!(sync_folder(&mut s, &caps, &st.snapshot("INBOX"), &b).await);
    res!(apply(&mut st, &d, &[]));
    let after = st.folders.clone();
    let again = res!(apply(&mut st, &d, &[]));
    req!(0, again.added);
    req!(true, after == st.folders);
    Ok(())
}

/// The cursor is stored only once the writes land: a failed apply leaves it where it was, so the
/// next sync asks for the same messages again.
#[tokio::test]
async fn test_cursor_not_advanced_when_apply_fails_00() -> Outcome<()> {
    let mut s = Server::new(QRESYNC);
    s.deliver("INBOX", "m1@x");
    let mut st = Store::default();
    let b = Budget::default();
    res!(round(&mut s, &mut st, &b, &[]).await);
    let before = st.folders["INBOX"].cursor.clone();
    s.deliver("INBOX", "m2@x");
    st.fail_put = true;
    req!(true, round(&mut s, &mut st, &b, &[]).await.is_err());
    req!(before, st.folders["INBOX"].cursor.clone());
    st.fail_put = false;
    let d = res!(round(&mut s, &mut st, &b, &[]).await);
    req!(vec![2], d.new.iter().map(|m| m.uid).collect::<Vec<_>>());
    Ok(())
}

/// A press he has just made (`+\Seen`, a move away) is not undone by a delta read before the
/// server had it.
#[tokio::test]
async fn test_delta_never_undoes_a_pending_change_00() -> Outcome<()> {
    let mut s = Server::new(CONDSTORE);
    for i in 1..=3 {
        s.deliver("INBOX", &fmt!("m{}@x", i));
    }
    let mut st = Store::default();
    let b = Budget::default();
    res!(round(&mut s, &mut st, &b, &[]).await);
    let uv = 100;
    // Optimistically read here; meanwhile his phone flagged it and cleared nothing.
    let seen = Entry {
        id: 1, mailbox: "INBOX".into(), uid_validity: uv, uid: 1,
        action: Action::Flags { add: vec!["\\Seen".into()], remove: vec![] },
        ..Default::default()
    };
    let key = MsgKey { uid_validity: uv, uid: 1 };
    res!(st.change_flags("INBOX", key, &["\\Seen".into()], &[]));
    s.set_flags("INBOX", 1, &["\\Flagged"]);
    // Optimistically moved away; the server still shows it here, with a flag change.
    let gone = Entry {
        id: 2, mailbox: "INBOX".into(), uid_validity: uv, uid: 2,
        action: Action::Trash, ..Default::default()
    };
    res!(st.remove("INBOX", MsgKey { uid_validity: uv, uid: 2 }));
    s.set_flags("INBOX", 2, &["\\Answered"]);
    // The device still names UID 2 as held, so the server's change to it reaches the delta.
    let caps = Caps::from_list(&s.caps);
    let mut snap = st.snapshot("INBOX");
    snap.held.insert(2);
    let d = res!(sync_folder(&mut s, &caps, &snap, &b).await);
    req!(2, d.flags.len());
    res!(apply(&mut st, &d, &[seen.clone(), gone.clone()]));
    let mut f = st.flags_of("INBOX", 1);
    f.sort();
    req!(vec!["\\Flagged".to_string(), "\\Seen".to_string()], f);
    req!(true, res!(st.flags("INBOX", MsgKey { uid_validity: uv, uid: 2 })).is_none());
    Ok(())
}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ ACTIONS AND THEIR OUTCOMES                                                │
// └───────────────────────────────────────────────────────────────────────────┘

fn account(caps: &str) -> Server {
    let mut s = Server::new(caps);
    s.folder("Sent", "\\Sent", 200);
    s.folder("Trash", "\\Trash", 300);
    s.folder("Junk", "\\Junk", 400);
    s.folder("Elsewhere", "", 500);
    s
}

async fn roles(s: &mut Server) -> Outcome<(Caps, Roles)> {
    let caps = Caps::from_list(&res!(s.capabilities().await));
    let roles = Roles::from_list(&res!(s.list().await));
    Ok((caps, roles))
}

fn entry(uid: u32, mid: &str, action: Action) -> Entry {
    Entry {
        id: uid as u64, mailbox: "INBOX".into(), uid_validity: 100, uid,
        message_id: fmt!("<{}>", mid), size: raw(mid, 2025).len() as u32, year: Some(2025),
        action,
    }
}

#[tokio::test]
async fn test_flag_change_on_an_expunged_uid_is_gone_00() -> Outcome<()> {
    let mut s = account(PLAIN);
    let u = s.deliver("INBOX", "a@x");
    s.expunge("INBOX", u);
    let (caps, r) = res!(roles(&mut s).await);
    let e = entry(u, "a@x", Action::Flags { add: vec!["\\Seen".into()], remove: vec![] });
    req!(ActOutcome::Gone, res!(act(&mut s, &caps, &r, Provider::Steel, &e).await));
    req!(false, s.log.iter().any(|l| l.starts_with("UID STORE")));
    Ok(())
}

#[tokio::test]
async fn test_flags_are_sent_as_deltas_00() -> Outcome<()> {
    let mut s = account(PLAIN);
    let u = s.deliver("INBOX", "a@x");
    s.set_flags("INBOX", u, &["\\Answered", "\\Flagged"]);
    let (caps, r) = res!(roles(&mut s).await);
    let e = entry(u, "a@x", Action::Flags { add: vec!["\\Seen".into()], remove: vec!["\\Flagged".into()] });
    req!(ActOutcome::Done, res!(act(&mut s, &caps, &r, Provider::Steel, &e).await));
    let mut f = s.folders["INBOX"].msgs[0].flags.clone();
    f.sort();
    req!(vec!["\\Answered".to_string(), "\\Seen".to_string()], f);
    Ok(())
}

/// A move whose source is gone: done where the target holds it, superseded where another folder
/// does, lost where none does.
#[tokio::test]
async fn test_move_of_a_gone_source_is_done_superseded_or_lost_00() -> Outcome<()> {
    let mut s = account(QRESYNC);
    let a = s.deliver("INBOX", "a@x");
    let b = s.deliver("INBOX", "b@x");
    let c = s.deliver("INBOX", "c@x");
    s.deliver("Trash", "a@x");
    s.deliver("Elsewhere", "b@x");
    for u in [a, b, c] {
        s.expunge("INBOX", u);
    }
    let (caps, r) = res!(roles(&mut s).await);
    req!(ActOutcome::Done, res!(act(&mut s, &caps, &r, Provider::Steel, &entry(a, "a@x", Action::Trash)).await));
    req!(ActOutcome::Superseded("Elsewhere".into()),
        res!(act(&mut s, &caps, &r, Provider::Steel, &entry(b, "b@x", Action::Trash)).await));
    req!(ActOutcome::Lost, res!(act(&mut s, &caps, &r, Provider::Steel, &entry(c, "c@x", Action::Trash)).await));
    Ok(())
}

/// A loose server's SEARCH names every message; only the FETCH of the Message-ID field decides.
#[tokio::test]
async fn test_loose_search_match_is_refused_by_the_confirming_fetch_00() -> Outcome<()> {
    let mut s = account(PLAIN);
    s.deliver("INBOX", "other@x");
    let want = s.deliver("INBOX", "want@x");
    s.deliver("INBOX", "want@x.evil");
    s.loose = true;
    res!(s.select("INBOX", &SelectWith::Plain).await);
    let found = res!(find_message_id(&mut s, "<want@x>").await);
    req!(vec![want], found.iter().map(|p| p.0).collect::<Vec<_>>());
    req!(0, res!(find_message_id(&mut s, "absent@x").await).len());
    // And a Sent filing on a loose server still appends when the copy is truly absent.
    let (_, r) = res!(roles(&mut s).await);
    s.deliver("Sent", "earlier@x");
    let filed = res!(file_sent(&mut s, &r, Provider::Steel, "<new@x>", &raw("new@x", 2025)).await);
    req!(true, matches!(filed, Filed::Appended(Some(_))), "{:?}", filed);
    // A move whose source is gone is not marked done on a loose match in the target.
    let gone = s.deliver("INBOX", "moved@x");
    s.expunge("INBOX", gone);
    let (caps, r) = res!(roles(&mut s).await);
    req!(ActOutcome::Lost, res!(act(&mut s, &caps, &r, Provider::Steel, &entry(gone, "moved@x", Action::Trash)).await));
    Ok(())
}

/// After a UIDVALIDITY change an entry is re-found by Message-ID and size, and acted on only when
/// exactly one message matches.
#[tokio::test]
async fn test_renumbered_entry_acts_only_on_a_unique_match_00() -> Outcome<()> {
    let mut s = account(PLAIN);
    let a = s.deliver("INBOX", "a@x");
    let d1 = s.deliver("INBOX", "dup@x");
    s.deliver("INBOX", "dup@x");
    s.renumber("INBOX", 101);
    let (caps, r) = res!(roles(&mut s).await);
    let e = entry(a, "a@x", Action::Flags { add: vec!["\\Seen".into()], remove: vec![] });
    req!(ActOutcome::Done, res!(act(&mut s, &caps, &r, Provider::Steel, &e).await));
    req!(vec!["\\Seen".to_string()], s.folders["INBOX"].msgs[0].flags.clone());
    let e = entry(d1, "dup@x", Action::Trash);
    req!(true, matches!(res!(act(&mut s, &caps, &r, Provider::Steel, &e).await), ActOutcome::Stuck(_)));
    let e = entry(9, "none@x", Action::Trash);
    req!(true, matches!(res!(act(&mut s, &caps, &r, Provider::Steel, &e).await), ActOutcome::Stuck(_)));
    req!(0, s.folders["Trash"].msgs.len());
    Ok(())
}

#[tokio::test]
async fn test_no_move_and_no_uidplus_refuses_rather_than_plain_expunge_00() -> Outcome<()> {
    let mut s = account("IMAP4rev1 SPECIAL-USE");
    let u = s.deliver("INBOX", "a@x");
    let (caps, r) = res!(roles(&mut s).await);
    req!(true, matches!(res!(act(&mut s, &caps, &r, Provider::Steel, &entry(u, "a@x", Action::Trash)).await),
        ActOutcome::Stuck(_)));
    req!(true, matches!(res!(act(&mut s, &caps, &r, Provider::Steel, &entry(u, "a@x", Action::Expunge)).await),
        ActOutcome::Stuck(_)));
    req!(false, s.log.iter().any(|l| l.contains("EXPUNGE") || l.contains("COPY")), "{:?}", s.log);
    req!(1, s.folders["INBOX"].msgs.len());
    Ok(())
}

#[tokio::test]
async fn test_copy_store_uid_expunge_without_move_00() -> Outcome<()> {
    let mut s = account("IMAP4rev1 UIDPLUS SPECIAL-USE");
    let u = s.deliver("INBOX", "a@x");
    let (caps, r) = res!(roles(&mut s).await);
    req!(ActOutcome::Done, res!(act(&mut s, &caps, &r, Provider::Steel, &entry(u, "a@x", Action::Junk)).await));
    req!(true, s.has("Junk", "a@x"));
    req!(0, s.folders["INBOX"].msgs.len());
    req!(true, s.log.iter().any(|l| l.starts_with("UID EXPUNGE")));
    Ok(())
}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ ARCHIVE AND FILING RULES, ROLES, LINKS, NAMES                             │
// └───────────────────────────────────────────────────────────────────────────┘

#[tokio::test]
async fn test_archive_by_provider_rule_00() -> Outcome<()> {
    // Steel and the general case: Archive/<year>, created when absent.
    let mut s = account(PLAIN);
    let u = s.deliver("INBOX", "a@x");
    let (caps, r) = res!(roles(&mut s).await);
    req!(ActOutcome::Done, res!(act(&mut s, &caps, &r, Provider::Steel, &entry(u, "a@x", Action::Archive)).await));
    req!(true, s.log.iter().any(|l| l == "CREATE Archive/2025"), "{:?}", s.log);
    req!(true, s.has("Archive/2025", "a@x"));

    // Gmail: a MOVE to the \All role, flat, whatever it is called.
    let mut s = account(CONDSTORE);
    s.folder("[Gmail]/Alle Nachrichten", "\\All", 600);
    let u = s.deliver("INBOX", "g@x");
    let (caps, r) = res!(roles(&mut s).await);
    req!(ActOutcome::Done, res!(act(&mut s, &caps, &r, Provider::Gmail, &entry(u, "g@x", Action::Archive)).await));
    req!(true, s.has("[Gmail]/Alle Nachrichten", "g@x"));
    req!(false, s.log.iter().any(|l| l.starts_with("CREATE")));

    // Office 365: the \Archive role, flat.
    let mut s = account(PLAIN);
    s.folder("Archiv", "\\Archive", 700);
    let u = s.deliver("INBOX", "o@x");
    let (caps, r) = res!(roles(&mut s).await);
    req!(ActOutcome::Done, res!(act(&mut s, &caps, &r, Provider::Office365, &entry(u, "o@x", Action::Archive)).await));
    req!(true, s.has("Archiv", "o@x"));

    // Junk and not-junk.
    let (caps, r) = res!(roles(&mut s).await);
    let u = s.deliver("INBOX", "j@x");
    req!(ActOutcome::Done, res!(act(&mut s, &caps, &r, Provider::Steel, &entry(u, "j@x", Action::Junk)).await));
    let ju = s.folders["Junk"].msgs[0].uid;
    let mut e = entry(ju, "j@x", Action::NotJunk);
    e.mailbox = "Junk".into();
    e.uid_validity = 400;
    req!(ActOutcome::Done, res!(act(&mut s, &caps, &r, Provider::Steel, &e).await));
    req!(true, s.has("INBOX", "j@x"));
    Ok(())
}

#[tokio::test]
async fn test_sent_filing_by_provider_rule_00() -> Outcome<()> {
    // Gmail and Office 365 file it themselves: never APPEND, even with no copy there yet.
    for p in [Provider::Gmail, Provider::Office365] {
        let mut s = account(CONDSTORE);
        let (_, r) = res!(roles(&mut s).await);
        req!(Filed::Server, res!(file_sent(&mut s, &r, p, "<s@x>", &raw("s@x", 2025)).await));
        req!(false, s.log.iter().any(|l| l.starts_with("APPEND")));
    }
    // Steel appends when a confirmed search finds none, and not when it finds one.
    let mut s = account(PLAIN);
    let (_, r) = res!(roles(&mut s).await);
    req!(true, matches!(res!(file_sent(&mut s, &r, Provider::Steel, "<s@x>", &raw("s@x", 2025)).await),
        Filed::Appended(_)));
    req!(Filed::Already, res!(file_sent(&mut s, &r, Provider::Steel, "<s@x>", &raw("s@x", 2025)).await));
    req!(1, s.folders["Sent"].msgs.len());
    Ok(())
}

/// A role is found by its attribute, never by a folder's name.
#[tokio::test]
async fn test_roles_by_special_use_never_by_name_00() -> Outcome<()> {
    let mut s = Server::new(PLAIN);
    s.folder("Sent", "", 1);
    s.folder("Trash", "", 2);
    s.folder("[Gmail]/Gesendet", "\\Sent", 3);
    let r = Roles::from_list(&res!(s.list().await));
    req!(Some("[Gmail]/Gesendet"), r.role(SpecialUse::Sent));
    req!(true, r.role(SpecialUse::Trash).is_none());
    let synced = r.synced();
    req!("INBOX", synced[0].as_str());
    req!(4, synced.len());
    // With no Trash role a trash is stuck, never filed by a guessed name.
    let caps = Caps::from_list(&s.caps);
    let u = s.deliver("INBOX", "a@x");
    req!(true, matches!(res!(act(&mut s, &caps, &r, Provider::Steel, &entry(u, "a@x", Action::Trash)).await),
        ActOutcome::Stuck(_)));
    let e = entry(u, "a@x", Action::Move { to: Target::Role(SpecialUse::Sent) });
    req!(ActOutcome::Done, res!(act(&mut s, &caps, &r, Provider::Steel, &e).await));
    req!(true, s.has("[Gmail]/Gesendet", "a@x"));
    Ok(())
}

/// Copies are linked by Message-ID, and by X-GM-MSGID only where the server advertises it.
#[tokio::test]
async fn test_links_by_message_id_and_gm_msgid_only_where_advertised_00() -> Outcome<()> {
    for (caps, gm) in [(PLAIN, false), (CONDSTORE, true)] {
        let mut s = Server::new(caps);
        s.deliver("INBOX", "l@x");
        let mut st = Store::default();
        let d = res!(round(&mut s, &mut st, &Budget::default(), &[]).await);
        let want = if gm { LinkKey::GmMsgId(1) } else { LinkKey::MessageId("l@x".into()) };
        let ok = d.new[0].link == Some(want);
        req!(true, ok, "{:?}", d.new[0].link);
    }
    let mut m = FetchedMessage { gm_msgid: Some(9), ..Default::default() };
    m.body = raw("z@x", 2025);
    req!(Some(LinkKey::MessageId("z@x".into())), link_key(&m, false));
    Ok(())
}

#[test]
fn test_maildir_name_is_keyed_and_round_trips_00() -> Outcome<()> {
    let key = MsgKey { uid_validity: 7, uid: 42 };
    let n = maildir_name(key, &["\\Seen".into(), "\\Flagged".into(), "$Label".into()]);
    req!("7.42:2,FS", n.as_str());
    let (k, f) = res!(parse_maildir_name(&n).ok_or_else(|| err!("unparsed"; Invalid)));
    req!(key, k);
    req!(vec!["\\Flagged".to_string(), "\\Seen".to_string()], f);
    Ok(())
}
