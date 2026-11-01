//! Reading one account's mail on the server, for an agent that asks rather than syncs: the
//! newest messages of a folder, a search of From and Subject, one message read by part, one
//! message raw, and the folders by role.
//!
//! - Nothing here changes the mailbox: every folder is opened with `EXAMINE`, so the session can
//!   write no flag and `\Recent` is left for the next `SELECT` (UNBLOCK-63), and every body is
//!   fetched with `BODY.PEEK`, so a daimon's read leaves `\Seen` as it found it.
//! - A folder is named by its role ([`Target`]), resolved by its RFC 6154 attribute, so Gmail's
//!   "[Gmail]/All Mail" and a localised "[Gmail]/Gesendet" are found without their names.
//! - [`find`] trusts the server's `SEARCH` for candidates only, since a server may match more
//!   than asked (Gmail's search is fuzzy): each candidate's decoded From or Subject must hold the
//!   text, case-insensitively, or it is dropped.
//! - [`read`] fetches `BODYSTRUCTURE` first, then the text parts within the byte cap, and lists
//!   attachments by name and size without their bytes, so a large attachment never cuts off the
//!   body it came with.
//! - Every size and count is the caller's, from its config ([`Limits`]).

use crate::{
    message,
    sync::{
        act::Target,
        BodyPart,
        FetchWhat,
        FetchedMessage,
        MailSession,
        Roles,
        SelectWith,
        SpecialUse,
        UidSet,
    },
};

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_text::mime;


/// The bounds on what one query returns, from the caller's config.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Limits {
    pub max_rows:       usize,  // rows a listing or a search returns
    pub max_read_bytes: u32,    // of text parts one read fetches, across its parts
    pub max_folders:    usize,  // folders counted by `folders`; the rest are listed uncounted
}

/// A calendar day, as `SEARCH SINCE` and `BEFORE` take it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Day {
    pub year:   u16,
    pub month:  u8,
    pub day:    u8,
}

impl Day {
    /// `1-Feb-2026` (RFC 3501 `date`).
    pub fn imap(&self) -> Outcome<String> {
        // fixed: the month names of RFC 3501's date-month.
        const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun",
            "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
        if self.month < 1 || self.month > 12 || self.day < 1 || self.day > 31 {
            return Err(err!("The day {}-{}-{} is not a calendar day.",
                self.year, self.month, self.day; Invalid, Input));
        }
        Ok(fmt!("{}-{}-{}", self.day, MONTHS[(self.month - 1) as usize], self.year))
    }
}

/// The dates a query is bounded by, by the server's internal date: on or after `since`, and
/// before `before`.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Range {
    pub since:  Option<Day>,
    pub before: Option<Day>,
}

impl Range {
    fn criteria(&self) -> Outcome<String> {
        let mut c = Vec::new();
        if let Some(d) = &self.since {
            c.push(fmt!("SINCE {}", res!(d.imap())));
        }
        if let Some(d) = &self.before {
            c.push(fmt!("BEFORE {}", res!(d.imap())));
        }
        Ok(c.join(" "))
    }
}

/// One message as a listing shows it, its fields with their encoded words decoded.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Header {
    pub uid:            u32,
    pub flags:          Vec<String>,
    pub internal_date:  String,     // as the server gave it
    pub size:           u32,
    pub date:           String,     // the Date field
    pub from:           String,
    pub to:             String,
    pub cc:             String,
    pub subject:        String,
    pub message_id:     String,     // without its angle brackets
}

impl Header {
    fn of(m: &FetchedMessage) -> Self {
        let hs = message::headers(&m.body);
        let get = |n: &str| hs.iter()
            .find(|(k, _)| k == n)
            .map(|(_, v)| mime::decode_words(v).trim().to_string())
            .unwrap_or_default();
        Self {
            uid:            m.uid,
            flags:          m.flags.clone(),
            internal_date:  m.internal_date.clone(),
            size:           m.size,
            date:           get("date"),
            from:           get("from"),
            to:             get("to"),
            cc:             get("cc"),
            subject:        get("subject"),
            message_id:     crate::sync::bare_message_id(&get("message-id")).to_string(),
        }
    }

    pub fn seen(&self) -> bool {
        self.flags.iter().any(|f| f.eq_ignore_ascii_case("\\Seen"))
    }
}

/// What a listing or a search found in one folder, newest first.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Listing {
    pub mailbox:    String,
    pub total:      u32,        // messages in the folder
    pub rows:       Vec<Header>,
    pub more:       bool,       // more matched than `max_rows`
}

/// One message read by part.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Read {
    pub header:         Header,
    pub texts:          Vec<TextPart>,
    pub attachments:    Vec<Attachment>,
}

impl Read {
    /// Was any text part cut at the byte cap, or left unread?
    pub fn cut(&self) -> bool {
        self.texts.iter().any(|t| t.cut)
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TextPart {
    pub section:    String,     // `1.1`
    pub media:      String,     // `text/plain` or `text/html`
    pub text:       String,     // decoded to UTF-8, as far as it was read
    pub cut:        bool,       // the cap stopped it short
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Attachment {
    pub section:    String,
    pub media:      String,
    pub name:       Option<String>,
    pub size:       u32,        // octets in its transfer encoding
}

/// One message's bytes, read without marking it read.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Raw {
    pub bytes:  Vec<u8>,
    pub size:   u32,    // the whole message's
    pub cut:    bool,   // `max` stopped it short
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Role {
    Inbox,
    Use(SpecialUse),
}

/// One folder, by role where it has one.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Folder {
    pub name:   String,
    pub role:   Option<Role>,
    pub count:  Option<u32>,    // `None` past `max_folders`
}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ THE QUERIES                                                               │
// └───────────────────────────────────────────────────────────────────────────┘

/// The mailbox a target names, by its role's attribute.
pub fn resolve(roles: &Roles, target: &Target) -> Outcome<String> {
    match target {
        Target::Inbox       => Ok(roles.inbox.clone()),
        Target::Role(r)     => Ok(res!(roles.role(*r)
            .ok_or_else(|| err!("The account has no folder with the {:?} role.", r; Missing)))
            .to_string()),
        Target::Path(p)     => if roles.exists(p) {
            Ok(p.clone())
        } else {
            Err(err!("The account has no folder {:?}.", p; Missing))
        },
    }
}

/// The newest messages of a folder within `range`.
pub async fn list<S: MailSession>(
    s:      &mut S,
    roles:  &Roles,
    target: &Target,
    range:  &Range,
    lim:    &Limits,
)
    -> Outcome<Listing>
{
    let mailbox = res!(resolve(roles, target));
    let st = res!(s.examine(&mailbox, &SelectWith::Plain).await);
    let crit = res!(range.criteria());
    let mut uids = res!(s.uid_search(if crit.is_empty() { "ALL" } else { &crit }).await);
    uids.sort_unstable();
    let more = uids.len() > lim.max_rows;
    let newest: Vec<u32> = uids.iter().rev().take(lim.max_rows).copied().collect();
    let rows = res!(summaries(s, &newest).await);
    Ok(Listing { mailbox, total: st.exists, rows, more })
}

/// The messages of a folder within `range` whose From or Subject holds `text`, in any case
/// (RFC 3501 §6.4.4), newest first. Nothing else is searched: the server's matches are
/// confirmed against the decoded fields, so one it matched in another field is dropped.
pub async fn find<S: MailSession>(
    s:      &mut S,
    roles:  &Roles,
    target: &Target,
    text:   &str,
    range:  &Range,
    lim:    &Limits,
)
    -> Outcome<Listing>
{
    let text = text.trim();
    if text.is_empty() || text.chars().any(|c| c.is_control()) {
        return Err(err!("The search text {:?} is empty or holds a control character.", text;
            Invalid, Input));
    }
    let mailbox = res!(resolve(roles, target));
    let st = res!(s.examine(&mailbox, &SelectWith::Plain).await);
    let q = fmt!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\""));
    let charset = if text.is_ascii() { "" } else { "CHARSET UTF-8 " };
    let crit = res!(range.criteria());
    let crit = fmt!("{}{}{}OR FROM {} SUBJECT {}",
        charset, crit, if crit.is_empty() { "" } else { " " }, q, q);
    let mut uids = res!(s.uid_search(&crit).await);
    uids.sort_unstable();
    uids.reverse();

    // Confirmed a batch at a time, newest first, until the rows are full.
    let want = text.to_lowercase();
    let mut rows = Vec::new();
    let mut more = false;
    let mut checked = 0;
    'batches: for batch in uids.chunks(lim.max_rows.max(1)) {
        for h in res!(summaries(s, batch).await) {
            if h.from.to_lowercase().contains(&want) || h.subject.to_lowercase().contains(&want) {
                if rows.len() == lim.max_rows {
                    more = true;
                    break 'batches;
                }
                rows.push(h);
            }
        }
        checked += batch.len();
        if rows.len() == lim.max_rows {
            more = checked < uids.len(); // candidates left unconfirmed
            break;
        }
    }
    Ok(Listing { mailbox, total: st.exists, rows, more })
}

/// One message by part, `None` where the folder does not hold `uid`: its structure first, then
/// each text body within `max_read_bytes` across them, the plain form of an alternative over
/// its HTML, and every other part listed by name.
pub async fn read<S: MailSession>(
    s:      &mut S,
    roles:  &Roles,
    target: &Target,
    uid:    u32,
    lim:    &Limits,
)
    -> Outcome<Option<Read>>
{
    let mailbox = res!(resolve(roles, target));
    res!(s.examine(&mailbox, &SelectWith::Plain).await);
    let one = UidSet::from([uid]);
    let m = match res!(s.uid_fetch(&one, FetchWhat::Structure).await).into_iter()
        .find(|m| m.uid == uid)
    {
        Some(m) => m,
        None    => return Ok(None),
    };
    let mut out = Read { header: Header::of(&m), ..Default::default() };
    let tree = match &m.structure {
        Some(t) => t.clone(),
        // A structure the server gave malformed: the whole message as one part, within the cap.
        None => BodyPart {
            section:    String::new(),
            media:      "message/rfc822".to_string(),
            size:       m.size,
            ..Default::default()
        },
    };
    let mut texts: Vec<&BodyPart> = Vec::new();
    bodies(&tree, &mut texts, &mut out.attachments);
    if texts.is_empty() && tree.parts.is_empty() && tree.media == "message/rfc822" {
        texts.push(&tree);
    }
    let mut left = lim.max_read_bytes;
    for p in texts {
        let ask = p.size.min(left);
        let mut t = TextPart {
            section:    p.section.clone(),
            media:      p.media.clone(),
            cut:        ask < p.size,
            ..Default::default()
        };
        if ask > 0 {
            if let Some(f) = res!(s.uid_fetch_section(uid, &p.section, ask).await) {
                left = left.saturating_sub(f.body.len() as u32);
                t.text = if p.section.is_empty() {
                    message::readable_text(&f.body)
                } else {
                    mime::decode_body(&f.body, &p.encoding, p.param("charset"))
                };
            }
        }
        out.texts.push(t);
    }
    Ok(Some(out))
}

/// The text bodies to read and the attachments to list: an alternative gives its first child
/// that holds plain text, else its last, its richest (RFC 2046 §5.1.4).
fn bodies<'a>(p: &'a BodyPart, texts: &mut Vec<&'a BodyPart>, atts: &mut Vec<Attachment>) {
    if p.parts.is_empty() {
        if p.is_text_body() {
            texts.push(p);
        } else if !p.section.is_empty() {
            atts.push(Attachment {
                section:    p.section.clone(),
                media:      p.media.clone(),
                name:       p.filename.clone(),
                size:       p.size,
            });
        }
        return;
    }
    if p.media == "multipart/alternative" {
        let plain = p.parts.iter()
            .find(|c| c.leaves().iter().any(|l| l.media == "text/plain" && l.is_text_body()));
        if let Some(c) = plain.or(p.parts.last()) {
            bodies(c, texts, atts);
        }
        return;
    }
    for c in &p.parts {
        bodies(c, texts, atts);
    }
}

/// One whole message, at most `max` bytes of it, leaving `\Seen` as it was.
pub async fn raw<S: MailSession>(
    s:      &mut S,
    roles:  &Roles,
    target: &Target,
    uid:    u32,
    max:    u32,
)
    -> Outcome<Option<Raw>>
{
    let mailbox = res!(resolve(roles, target));
    res!(s.examine(&mailbox, &SelectWith::Plain).await);
    Ok(res!(s.uid_fetch_section(uid, "", max).await).map(|m| Raw {
        cut:    (m.body.len() as u32) < m.size,
        size:   m.size,
        bytes:  m.body,
    }))
}

/// Every folder, the inbox and the roles first, each with its role found by attribute and, for
/// the first `max_folders`, its message count.
pub async fn folders<S: MailSession>(s: &mut S, lim: &Limits) -> Outcome<(Roles, Vec<Folder>)> {
    let roles = Roles::from_list(&res!(s.list().await));
    let mut out = Vec::new();
    for (i, name) in roles.synced().into_iter().enumerate() {
        let role = if name == roles.inbox {
            Some(Role::Inbox)
        } else {
            roles.mailboxes.iter().find(|m| m.name == name)
                .and_then(|m| m.special_use())
                .filter(|r| roles.role(*r) == Some(name.as_str()))
                .map(Role::Use)
        };
        let count = if i < lim.max_folders { Some(res!(s.count(&name).await)) } else { None };
        out.push(Folder { name, role, count });
    }
    Ok((roles, out))
}

/// The listing rows of `uids`, newest first.
async fn summaries<S: MailSession>(s: &mut S, uids: &[u32]) -> Outcome<Vec<Header>> {
    if uids.is_empty() {
        return Ok(Vec::new());
    }
    let asked = UidSet::from(uids);
    let mut rows: Vec<Header> = res!(s.uid_fetch(&asked, FetchWhat::Summary).await).iter()
        .filter(|m| asked.contains(m.uid))
        .map(Header::of)
        .collect();
    rows.sort_by(|a, b| b.uid.cmp(&a.uid));
    rows.dedup_by_key(|h| h.uid);
    Ok(rows)
}
