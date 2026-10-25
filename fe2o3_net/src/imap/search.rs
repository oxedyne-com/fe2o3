//! IMAP SEARCH criteria (RFC 3501 §6.4.4): parsing and matching.
//!
//! Every search key of RFC 3501 is read, with `CHARSET UTF-8` or `US-ASCII`, sequence sets and
//! parenthesised lists, and string keys match as case-insensitive substrings. A key outside the
//! standard is refused, never read as `ALL`: a client acts on what SEARCH names, so an answer
//! wider than the question turns a narrow move or delete into one on every message.
//!
//! 2026-10-26 (F2a, D-20261026-01): before this module the server knew ALL, the flags and
//! `UID <set>`, and answered anything else with the whole mailbox.
//!
//! Header keys (HEADER, FROM, TO, CC, BCC, SUBJECT and TEXT's header part) match the value with
//! its RFC 2047 encoded-words decoded, so `SUBJECT Grüße` finds `=?UTF-8?B?R3LDvMOfZQ==?=`
//! (F2a part 2, 2026-10-26). A body is still compared as it is stored, transfer encoding and
//! all.

use crate::mail::store::MessageMeta;

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_text::mime::decode_words;

use std::time::UNIX_EPOCH;


/// One piece of a SEARCH command as it came off the wire: command text, or the contents of a
/// literal, which is a string whatever it holds.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Segment {
    Text(String),
    Literal(String),
}

/// Why a SEARCH was refused.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SearchError {
    Bad(String),    // tagged BAD with this text
    BadCharset,     // tagged NO [BADCHARSET (UTF-8 US-ASCII)]
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Flag {
    Answered,
    Deleted,
    Draft,
    Flagged,
    Recent,
    Seen,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SearchKey {
    All,
    Flag(Flag, bool),           // the flag, and whether it must be set
    New,                        // RECENT UNSEEN
    Old,                        // NOT RECENT
    Keyword(bool),              // KEYWORD (true) or UNKEYWORD: this server keeps no keywords
    Header(String, String),     // field name and needle, both lower case
    Body(String),
    Text(String),
    Before(i64),                // days since the epoch, against INTERNALDATE
    On(i64),
    Since(i64),
    SentBefore(i64),            // days since the epoch, against the Date: field
    SentOn(i64),
    SentSince(i64),
    Larger(u64),
    Smaller(u64),
    Seq(String),                // a validated sequence set
    Uid(String),                // a validated UID set
    Not(Box<SearchKey>),
    Or(Box<SearchKey>, Box<SearchKey>),
    And(Vec<SearchKey>),
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum Token {
    Atom(String),
    Str(String),    // quoted or literal
    Open,
    Close,
}

/// Reads the arguments of SEARCH or UID SEARCH into one key, the AND of all given.
pub fn parse(segments: &[Segment]) -> Result<SearchKey, SearchError> {
    let mut toks: Vec<Token> = Vec::new();
    for seg in segments {
        match seg {
            Segment::Text(t)    => ok!(tokenise(t, &mut toks)),
            Segment::Literal(s) => toks.push(Token::Str(s.clone())),
        }
    }
    let mut pos = 0usize;
    if let Some(Token::Atom(a)) = toks.first() {
        if a.eq_ignore_ascii_case("CHARSET") {
            match toks.get(1) {
                Some(Token::Atom(cs)) | Some(Token::Str(cs)) => {
                    if !cs.eq_ignore_ascii_case("UTF-8") && !cs.eq_ignore_ascii_case("US-ASCII") {
                        return Err(SearchError::BadCharset);
                    }
                },
                _ => return Err(bad("CHARSET needs a charset name")),
            }
            pos = 2;
        }
    }
    let mut keys = Vec::new();
    while pos < toks.len() {
        keys.push(ok!(parse_key(&toks, &mut pos)));
    }
    match keys.len() {
        0 => Err(bad("SEARCH needs at least one search key")),
        1 => Ok(keys.remove(0)),
        _ => Ok(SearchKey::And(keys)),
    }
}

fn bad(text: &str) -> SearchError {
    SearchError::Bad(text.to_string())
}

fn tokenise(text: &str, out: &mut Vec<Token>) -> Result<(), SearchError> {
    let mut chars = text.chars().peekable();
    while let Some(&c) = chars.peek() {
        match c {
            ' ' | '\t' => { chars.next(); },
            '('  => { chars.next(); out.push(Token::Open); },
            ')'  => { chars.next(); out.push(Token::Close); },
            '"'  => {
                chars.next();
                let mut s = String::new();
                loop {
                    match chars.next() {
                        Some('"')   => break,
                        Some('\\')  => match chars.next() {
                            Some(e) => s.push(e),
                            None    => return Err(bad("Unterminated quoted string")),
                        },
                        Some(x)     => s.push(x),
                        None        => return Err(bad("Unterminated quoted string")),
                    }
                }
                out.push(Token::Str(s));
            },
            _ => {
                let mut a = String::new();
                while let Some(&x) = chars.peek() {
                    if x == ' ' || x == '\t' || x == '(' || x == ')' || x == '"' { break; }
                    a.push(x);
                    chars.next();
                }
                out.push(Token::Atom(a));
            },
        }
    }
    Ok(())
}

fn parse_key(toks: &[Token], pos: &mut usize) -> Result<SearchKey, SearchError> {
    let tok = match toks.get(*pos) {
        Some(t) => t.clone(),
        None    => return Err(bad("A search key is missing")),
    };
    *pos += 1;
    let name = match tok {
        Token::Open => {
            let mut keys = Vec::new();
            loop {
                match toks.get(*pos) {
                    Some(Token::Close)  => { *pos += 1; break; },
                    Some(_)             => keys.push(ok!(parse_key(toks, pos))),
                    None                => return Err(bad("Unclosed parenthesis")),
                }
            }
            if keys.is_empty() { return Err(bad("Empty parenthesised list")); }
            return Ok(SearchKey::And(keys));
        },
        Token::Close    => return Err(bad("Unexpected ')'")),
        Token::Str(s)   => return Err(SearchError::Bad(fmt!("{:?} is not a search key", s))),
        Token::Atom(a)  => a,
    };
    let upper = name.to_ascii_uppercase();
    let key = match upper.as_str() {
        "ALL"           => SearchKey::All,
        "ANSWERED"      => SearchKey::Flag(Flag::Answered, true),
        "DELETED"       => SearchKey::Flag(Flag::Deleted, true),
        "DRAFT"         => SearchKey::Flag(Flag::Draft, true),
        "FLAGGED"       => SearchKey::Flag(Flag::Flagged, true),
        "RECENT"        => SearchKey::Flag(Flag::Recent, true),
        "SEEN"          => SearchKey::Flag(Flag::Seen, true),
        "UNANSWERED"    => SearchKey::Flag(Flag::Answered, false),
        "UNDELETED"     => SearchKey::Flag(Flag::Deleted, false),
        "UNDRAFT"       => SearchKey::Flag(Flag::Draft, false),
        "UNFLAGGED"     => SearchKey::Flag(Flag::Flagged, false),
        "UNSEEN"        => SearchKey::Flag(Flag::Seen, false),
        "NEW"           => SearchKey::New,
        "OLD"           => SearchKey::Old,
        "KEYWORD"       => { ok!(take_atom(toks, pos, "KEYWORD")); SearchKey::Keyword(true) },
        "UNKEYWORD"     => { ok!(take_atom(toks, pos, "UNKEYWORD")); SearchKey::Keyword(false) },
        "BCC" | "CC" | "FROM" | "SUBJECT" | "TO" => {
            let needle = ok!(take_string(toks, pos, &upper));
            SearchKey::Header(name.to_ascii_lowercase(), needle.to_lowercase())
        },
        "HEADER"        => {
            let field = ok!(take_string(toks, pos, "HEADER"));
            let needle = ok!(take_string(toks, pos, "HEADER"));
            SearchKey::Header(field.to_ascii_lowercase(), needle.to_lowercase())
        },
        "BODY"          => SearchKey::Body(ok!(take_string(toks, pos, "BODY")).to_lowercase()),
        "TEXT"          => SearchKey::Text(ok!(take_string(toks, pos, "TEXT")).to_lowercase()),
        "BEFORE"        => SearchKey::Before(ok!(take_date(toks, pos, "BEFORE"))),
        "ON"            => SearchKey::On(ok!(take_date(toks, pos, "ON"))),
        "SINCE"         => SearchKey::Since(ok!(take_date(toks, pos, "SINCE"))),
        "SENTBEFORE"    => SearchKey::SentBefore(ok!(take_date(toks, pos, "SENTBEFORE"))),
        "SENTON"        => SearchKey::SentOn(ok!(take_date(toks, pos, "SENTON"))),
        "SENTSINCE"     => SearchKey::SentSince(ok!(take_date(toks, pos, "SENTSINCE"))),
        "LARGER"        => SearchKey::Larger(ok!(take_number(toks, pos, "LARGER"))),
        "SMALLER"       => SearchKey::Smaller(ok!(take_number(toks, pos, "SMALLER"))),
        "UID"           => {
            let set = ok!(take_atom(toks, pos, "UID"));
            if !valid_set(&set) {
                return Err(SearchError::Bad(fmt!("UID {:?} is not a sequence set", set)));
            }
            SearchKey::Uid(set)
        },
        "NOT"           => SearchKey::Not(Box::new(ok!(parse_key(toks, pos)))),
        "OR"            => {
            let a = ok!(parse_key(toks, pos));
            let b = ok!(parse_key(toks, pos));
            SearchKey::Or(Box::new(a), Box::new(b))
        },
        _ if valid_set(&name) => SearchKey::Seq(name),
        _ => return Err(SearchError::Bad(fmt!("Unknown search key {}", name))),
    };
    Ok(key)
}

fn take_atom(toks: &[Token], pos: &mut usize, key: &str) -> Result<String, SearchError> {
    match toks.get(*pos) {
        Some(Token::Atom(a)) => { *pos += 1; Ok(a.clone()) },
        _ => Err(SearchError::Bad(fmt!("{} needs an argument", key))),
    }
}

fn take_string(toks: &[Token], pos: &mut usize, key: &str) -> Result<String, SearchError> {
    match toks.get(*pos) {
        Some(Token::Atom(s)) | Some(Token::Str(s)) => { *pos += 1; Ok(s.clone()) },
        _ => Err(SearchError::Bad(fmt!("{} needs a string", key))),
    }
}

fn take_number(toks: &[Token], pos: &mut usize, key: &str) -> Result<u64, SearchError> {
    let a = ok!(take_atom(toks, pos, key));
    match a.parse::<u64>() {
        Ok(n)   => Ok(n),
        Err(_)  => Err(SearchError::Bad(fmt!("{} needs a number, not {:?}", key, a))),
    }
}

fn take_date(toks: &[Token], pos: &mut usize, key: &str) -> Result<i64, SearchError> {
    let s = ok!(take_string(toks, pos, key));
    match parse_search_date(&s) {
        Some(d) => Ok(d),
        None    => Err(SearchError::Bad(fmt!("{} needs a date such as 1-Feb-1994, not {:?}", key, s))),
    }
}

/// Is `s` a sequence set: `n`, `*`, `n:m`, joined by commas, with no zero?
fn valid_set(s: &str) -> bool {
    !s.is_empty() && s.split(',').all(|piece| {
        let mut ends = piece.splitn(2, ':');
        ends.all(|e| e == "*" || matches!(e.parse::<u32>(), Ok(n) if n > 0))
            && !piece.is_empty()
    })
}

/// Does the set, with `*` read as `max`, hold `n`?
fn set_contains(set: &str, n: u32, max: u32) -> bool {
    let num = |e: &str| if e == "*" { max } else { e.parse::<u32>().unwrap_or(0) };
    set.split(',').any(|piece| {
        let (lo, hi) = match piece.split_once(':') {
            Some((a, b))    => (num(a), num(b)),
            None            => (num(piece), num(piece)),
        };
        let (lo, hi) = if lo > hi { (hi, lo) } else { (lo, hi) };
        n >= lo && n <= hi
    })
}

fn month_number(m: &str) -> Option<u32> {
    const MONTHS: [&str; 12] = [
        "jan", "feb", "mar", "apr", "may", "jun",
        "jul", "aug", "sep", "oct", "nov", "dec",
    ];
    let m = m.to_ascii_lowercase();
    MONTHS.iter().position(|x| *x == m).map(|i| i as u32 + 1)
}

/// `d-Mon-yyyy`, the RFC 3501 `date`, as days since the epoch.
fn parse_search_date(s: &str) -> Option<i64> {
    let mut it = s.split('-');
    let d: u32 = ok!(ok!(it.next()).parse().ok());
    let m = ok!(month_number(ok!(it.next())));
    let y: i32 = ok!(ok!(it.next()).parse().ok());
    if it.next().is_some() || d == 0 || d > 31 { return None; }
    Some(days_from_civil(y, m, d))
}

/// The date of an RFC 5322 `Date:` field, `[Day,] d Mon yyyy hh:mm[:ss] zone`, as days since the
/// epoch. Time and zone are disregarded, as RFC 3501 asks of the SENT keys.
fn parse_header_date(s: &str) -> Option<i64> {
    let s = match s.find(',') {
        Some(i) => &s[i + 1..],
        None    => s,
    };
    let mut it = s.split_whitespace();
    let d: u32 = ok!(ok!(it.next()).parse().ok());
    let m = ok!(month_number(ok!(it.next())));
    let y_str = ok!(it.next());
    let mut y: i32 = ok!(y_str.parse().ok());
    // RFC 5322 §4.3: a two-digit year is 19xx from 50, 20xx below; three digits add 1900.
    if y_str.len() == 2 { y += if y < 50 { 2000 } else { 1900 }; }
    else if y_str.len() == 3 { y += 1900; }
    if d == 0 || d > 31 { return None; }
    Some(days_from_civil(y, m, d))
}

/// Howard Hinnant's algorithm, proleptic Gregorian.
fn days_from_civil(y: i32, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y as i64 - 1 } else { y as i64 };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = if m > 2 { m - 3 } else { m + 9 } as i64;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// What the content keys look at in one message, decoded and lower-cased once.
#[derive(Debug)]
pub struct Content {
    fields:     Vec<(String, String)>,  // lower-case name, unfolded, decoded, lower-case value
    head:       String,                 // the decoded fields as `name: value` lines, for TEXT
    body:       String,                 // lower case, as stored
    sent:       Option<i64>,            // the Date: field, days since the epoch
}

impl Content {
    pub fn new(bytes: &[u8]) -> Self {
        // Split and decode before lower-casing: case is meaningful inside a base64 word.
        let text = String::from_utf8_lossy(bytes);
        let (head, body) = match (text.find("\r\n\r\n"), text.find("\n\n")) {
            (Some(a), Some(b)) if b + 1 < a => (&text[..b], &text[b + 2..]),
            (Some(a), _)                    => (&text[..a], &text[a + 4..]),
            (None, Some(b))                 => (&text[..b], &text[b + 2..]),
            (None, None)                    => (&text[..], ""),
        };
        let mut raw_fields: Vec<(String, String)> = Vec::new();
        for raw in head.split('\n') {
            let line = raw.strip_suffix('\r').unwrap_or(raw);
            if line.starts_with(' ') || line.starts_with('\t') {
                if let Some(last) = raw_fields.last_mut() {
                    last.1.push(' ');
                    last.1.push_str(line.trim());
                }
                continue;
            }
            if let Some((n, v)) = line.split_once(':') {
                raw_fields.push((n.trim().to_string(), v.trim().to_string()));
            }
        }
        let fields: Vec<(String, String)> = raw_fields.iter()
            .map(|(n, v)| (n.to_ascii_lowercase(), decode_words(v).to_lowercase()))
            .collect();
        let mut head = String::new();
        for (n, v) in &fields {
            head.push_str(n);
            head.push_str(": ");
            head.push_str(v);
            head.push('\n');
        }
        let sent = fields.iter()
            .find(|(n, _)| n == "date")
            .and_then(|(_, v)| parse_header_date(v));
        Self {
            fields,
            head,
            body:   body.to_lowercase(),
            sent,
        }
    }
}

impl SearchKey {
    /// Does any part of the key read the message itself rather than its flags, size or date?
    pub fn needs_content(&self) -> bool {
        match self {
            Self::Header(..) | Self::Body(_) | Self::Text(_)
                | Self::SentBefore(_) | Self::SentOn(_) | Self::SentSince(_) => true,
            Self::Not(k)        => k.needs_content(),
            Self::Or(a, b)      => a.needs_content() || b.needs_content(),
            Self::And(ks)       => ks.iter().any(|k| k.needs_content()),
            _                   => false,
        }
    }

    /// Does message `msgs[idx]` match? `content` is `None` only where `needs_content` is false.
    pub fn matches(&self, idx: usize, msgs: &[MessageMeta], content: Option<&Content>) -> bool {
        let m = match msgs.get(idx) {
            Some(m) => m,
            None    => return false,
        };
        let day = || match m.internal.duration_since(UNIX_EPOCH) {
            Ok(d)   => (d.as_secs() / 86_400) as i64,
            Err(e)  => -((e.duration().as_secs() / 86_400) as i64) - 1,
        };
        let sent = || content.and_then(|c| c.sent);
        match self {
            Self::All                       => true,
            Self::Flag(f, want)             => {
                let set = match f {
                    Flag::Answered  => m.flags.answered,
                    Flag::Deleted   => m.flags.deleted,
                    Flag::Draft     => m.flags.draft,
                    Flag::Flagged   => m.flags.flagged,
                    Flag::Recent    => m.flags.recent,
                    Flag::Seen      => m.flags.seen,
                };
                set == *want
            },
            Self::New                       => m.flags.recent && !m.flags.seen,
            Self::Old                       => !m.flags.recent,
            Self::Keyword(want)             => !*want,
            Self::Header(name, needle)      => match content {
                Some(c) => c.fields.iter().any(|(n, v)| n == name && v.contains(needle.as_str())),
                None    => false,
            },
            Self::Body(needle)              => content.is_some_and(|c| c.body.contains(needle.as_str())),
            Self::Text(needle)              => content.is_some_and(|c| {
                c.head.contains(needle.as_str()) || c.body.contains(needle.as_str())
            }),
            Self::Before(d)                 => day() < *d,
            Self::On(d)                     => day() == *d,
            Self::Since(d)                  => day() >= *d,
            Self::SentBefore(d)             => sent().is_some_and(|s| s < *d),
            Self::SentOn(d)                 => sent().is_some_and(|s| s == *d),
            Self::SentSince(d)              => sent().is_some_and(|s| s >= *d),
            Self::Larger(n)                 => m.size > *n,
            Self::Smaller(n)                => m.size < *n,
            Self::Seq(set)                  => set_contains(set, idx as u32 + 1, msgs.len() as u32),
            Self::Uid(set)                  => {
                let max = msgs.last().map(|x| x.uid.0).unwrap_or(0);
                set_contains(set, m.uid.0, max)
            },
            Self::Not(k)                    => !k.matches(idx, msgs, content),
            Self::Or(a, b)                  => a.matches(idx, msgs, content) || b.matches(idx, msgs, content),
            Self::And(ks)                   => ks.iter().all(|k| k.matches(idx, msgs, content)),
        }
    }
}

/// Where a line of SEARCH arguments ends in a literal marker `{n}` or `{n+}`: the text before
/// it, `n`, and whether the client waits for a continuation (`{n}`, synchronising).
pub fn trailing_literal(line: &str) -> Option<(String, usize, bool)> {
    let inner = ok!(line.strip_suffix('}'));
    let open = ok!(inner.rfind('{'));
    let spec = &inner[open + 1..];
    let (digits, sync) = match spec.strip_suffix('+') {
        Some(d) => (d, false),
        None    => (spec, true),
    };
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) { return None; }
    let n: usize = ok!(digits.parse().ok());
    Some((line[..open].to_string(), n, sync))
}
