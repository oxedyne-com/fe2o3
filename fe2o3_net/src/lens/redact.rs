//! The redactor of Daimond's feed, over a `Dat`.
//!
//! `secret_name` is `SECRET_RE` and `fingerprint` is `fingerprint` from `www/js/debugshare.js`
//! (the lines marked `SECRET_RE`, `SECRET_RE_LOOSE` and `function fingerprint` there), and a
//! test holds each to the JavaScript over a table that JavaScript generated. `Redact` is its
//! `redact` walk: a deep copy in which every field whose name looks secret has its value
//! replaced by a fingerprint, so the operator still sees which fields were present and how they
//! relate, and only the value is gone. Two hooks are added for a caller's own knowledge: a
//! deny-list of names and paths, and a test over strings.
//!
//! Free text is the caller's string test, and `Shapes` is a stock one for the shapes a credential
//! wears. It is the union of two things in `fe2o3_text::secret`: the scanner behind the commit
//! hook, and a port of the content scrubber of `debugshare.js` (its `SCRUB_SHAPES`, its pairs and
//! its entropy catch), so a feed is at least as well guarded here as it is in Daimond's browser.
//! A caller adds what only it knows, as Oxegen adds its eight-word passphrase, by passing its own
//! test.

use super::row::Row;

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_jdat::prelude::*;
use oxedyne_fe2o3_text::secret::{
    self,
    scrub,
};

use std::collections::HashSet;


// The walk's bound, as in `redact`. A `Dat` cannot hold a cycle, so this is a depth bound alone.
const MAX_DEPTH: usize = 40;
const DEEP:      &str = "[redacted:cycle-or-deep]";
const OBJECT:    &str = "[redacted:object]";
const ABSENT:    &str = "[redacted:absent]";

// SECRET_RE's words, in its order.
const WORDS: [&str; 17] = [
    "apikey", "api_key", "key", "token", "secret", "passphrase", "password", "salt", "wrapped",
    "wrappedpriv", "sealed", "seal", "privatekey", "priv", "mnemonic", "seed", "masterkey",
];

fn sep(b: u8) -> bool {
    matches!(b, b'_' | b'.' | b'-')
}

// JavaScript's `\w`, which is ASCII alone.
fn word(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// Does the name look like a secret's? This is Daimond's `SECRET_RE`,
///
/// `/(?:^|[_.-])(?:apikey|api_key|key|token|secret|passphrase|password|salt|wrapped|wrappedpriv|
/// sealed|seal|privatekey|priv|mnemonic|seed|masterkey)(?:$|[_.-]|enc\b)/i`,
///
/// tested anywhere in the name. A word counts only at the start of the name or after a `_`, `.`
/// or `-`, so a camel-joined `pushToken` slides past it; `secret_name_loose` catches those.
///
/// This is a hand matcher and not a `fe2o3_text::regex::Regex` on purpose. JavaScript's `\b` and
/// `/i` are ASCII-only, while the crate's engine follows the `regex` crate, whose `\b` and case
/// folding are Unicode, so the two would disagree on a name such as `keyenc` followed by `é`. A
/// test holds the matcher to the JavaScript over generated names, `é` included, and to the engine
/// over the ASCII ones.
pub fn secret_name(name: &str) -> bool {
    let b = name.as_bytes();
    let mut starts = vec![0usize];
    for (i, &c) in b.iter().enumerate() {
        if sep(c) {
            starts.push(i + 1);
        }
    }
    for s in starts {
        for w in WORDS {
            let n = w.len();
            if b.len() < s + n || !b[s..s + n].eq_ignore_ascii_case(w.as_bytes()) {
                continue;
            }
            let e = s + n;
            // `$`, a separator, or `enc` ending at a word boundary.
            let tail = e == b.len()
                || sep(b[e])
                || (b.len() >= e + 3
                    && b[e..e + 3].eq_ignore_ascii_case(b"enc")
                    && (b.len() == e + 3 || !word(b[e + 3])));
            if tail {
                return true;
            }
        }
    }
    false
}

/// `secret_name`, or Daimond's `SECRET_RE_LOOSE`, `/token|secret|passphrase|password|key$/i`.
///
/// The loose pattern has no boundary, so it also takes a real, non-secret field such as
/// `tokenStats`. Daimond uses it only on its free-form `config`, and so should a caller that
/// knows its names are camel-joined.
pub fn secret_name_loose(name: &str) -> bool {
    if secret_name(name) {
        return true;
    }
    let l = name.to_ascii_lowercase();
    l.contains("token") || l.contains("secret") || l.contains("passphrase") || l.contains("password")
        || l.ends_with("key")
}

/// A short, non-reversible rendering of a secret for a person's eye: its first `head`
/// characters, a stable hash and its length. With `head` of 6 this is Daimond's `fingerprint`
/// exactly, which hashes and counts UTF-16 code units as JavaScript does.
///
/// The hash is djb2, not a cryptographic hash. It only has to tell two secrets apart and match
/// two sightings of one. The head is itself a disclosure of up to `head` characters, and `0`
/// withholds it.
pub fn fingerprint(v: &str, head: usize) -> String {
    if v.is_empty() {
        return "[redacted:empty]".to_string();
    }
    let u: Vec<u16> = v.encode_utf16().collect();
    let mut h: u32 = 5381;
    for &c in &u {
        h = (h << 5).wrapping_add(h).wrapping_add(c as u32);
    }
    let hd = String::from_utf16_lossy(&u[..head.min(u.len())]);
    fmt!("[redacted {}…#{:x}/{}]", hd, h, u.len())
}

// Is this a marker the redactor itself wrote? A second pass leaves it alone, since a
// fingerprint of a fingerprint tells the reader nothing and hides the first.
fn marker(s: &str) -> bool {
    s.starts_with("[redacted") && s.ends_with(']')
}

/// A caller's test over strings: does this one hold a secret?
///
/// Any closure over `&str` is one. A string that passes is replaced whole by its fingerprint,
/// so the test should look *inside* a string (Oxegen's asks whether eight words of the EFF list
/// stand in a row) and not only at all of it.
pub trait StrTest: Send + Sync {
    fn hit(&self, s: &str) -> bool;

    /// The same question of a string found under a correlation-id field (see
    /// [`Redact::id_key`]). A test may excuse an id from its heuristics here and keep its shapes;
    /// by default an id is held to the same test as any other string.
    fn hit_id(&self, s: &str) -> bool { self.hit(s) }
}

/// The test that never fires, for a redactor that has no string test of its own.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoTest;

impl StrTest for NoTest {
    fn hit(&self, _s: &str) -> bool { false }
}

impl<F: Fn(&str) -> bool + Send + Sync> StrTest for F {
    fn hit(&self, s: &str) -> bool { self(s) }
}

/// The stock string test: a string that holds a credential is covered whole.
///
/// A credential here is anything `fe2o3_text::secret::holds` finds, which honours no `allowlist
/// secret` marker and skips no text for a NUL since a feed's text is the very thing in doubt, or
/// anything Daimond's content scrubber would mark: every shape in its `SCRUB_SHAPES`, a secret's
/// name standing against a value (`token=...`, an `Authorization` header, a URL's `?key=`), and
/// a long unbroken run of high entropy. That last is a heuristic, so a string found under a
/// correlation-id field is excused from it alone ([`StrTest::hit_id`]); a shape still hits there.
///
/// `hit` says whether, and `scrub` is the in-place form Daimond writes, which cuts only the
/// credential out of a string and keeps the words round it. A [`Redact`] covers a hit whole, which
/// is the stronger of the two. Pair it with `Redact::with_head(0)` where the first characters of a
/// credential must not show in its fingerprint.
#[derive(Clone, Copy, Debug, Default)]
pub struct Shapes;

impl Shapes {

    /// The string with each credential replaced by `[redacted <shape> #<hash>/<length>]`, exactly
    /// as the JavaScript scrubber writes it. `is_id` suppresses the entropy catch alone.
    pub fn scrub(&self, s: &str, is_id: bool) -> String {
        scrub::text(s, is_id)
    }
}

impl StrTest for Shapes {
    fn hit(&self, s: &str) -> bool { secret::holds(s) || scrub::hit(s, false) }
    fn hit_id(&self, s: &str) -> bool { secret::holds(s) || scrub::hit(s, true) }
}

/// A test that fires when either of two does, so an app can add its own to the stock shapes:
/// `Or(Shapes, Phrase::new(words, 8))`.
#[derive(Clone, Copy, Debug, Default)]
pub struct Or<A, B>(pub A, pub B);

impl<A: StrTest, B: StrTest> StrTest for Or<A, B> {
    fn hit(&self, s: &str) -> bool { self.0.hit(s) || self.1.hit(s) }
    fn hit_id(&self, s: &str) -> bool { self.0.hit_id(s) || self.1.hit_id(s) }
}

/// A run of words from a caller's list: `n` of them in a row are a secret.
///
/// An app whose passphrases are drawn from a list of words (Oxegen's are eight words of the EFF
/// large list) cannot name the secret by shape, since nothing about a word is secret, but a string
/// in which `n` of the list's words stand together is a passphrase or near enough to one that no
/// row may keep it. Words are the runs of ASCII letters in the text, whatever separates them
/// (a space, a hyphen, a comma, a line break, a digit, a letter outside ASCII) and whatever the
/// case; any run of letters that is not on the list breaks the run. The list is the caller's, so a
/// second app passes its own and the test is the same.
///
/// A list entry may hold hyphens (the EFF list has `drop-down`, `felt-tip`, `t-shirt` and
/// `yo-yo`). Where the text spells such an entry, hyphens and all, it counts as the one word it is,
/// and the longest entry wins. Splitting on every non-letter cannot see those four entries, and
/// breaks a passphrase that holds one into runs shorter than eight. An entry whose every part is
/// a list word counts for its parts, since a dashed passphrase joins its words with the same hyphen.
///
/// This is the Rust half of `createRunGuard` in `redact.js`, and `tests/data/lens_phrase.tsv` holds
/// the two to the same answers.
#[derive(Clone, Debug)]
pub struct Phrase {
    words:  HashSet<String>,    // lower case
    n:      usize,              // words in a row that make a secret
    parts:  usize,              // the most hyphen-joined parts any entry has
}

impl Phrase {

    /// A test over `words`, any case, firing on `n` in a row (at least one).
    pub fn new<'a, I: IntoIterator<Item = &'a str>>(words: I, n: usize) -> Self {
        let words: HashSet<String> = words.into_iter()
            .map(|w| w.trim().to_ascii_lowercase())
            .filter(|w| !w.is_empty())
            .collect();
        let parts = words.iter().map(|w| w.matches('-').count() + 1).max().unwrap_or(1);
        Self { words, n: n.max(1), parts }
    }

    // The runs of ASCII letters in `b`, as byte ranges.
    fn tokens(b: &[u8]) -> Vec<(usize, usize)> {
        let mut out = Vec::new();
        let mut at = None;
        for (i, c) in b.iter().enumerate() {
            match (c.is_ascii_alphabetic(), at) {
                (true, None)        => at = Some(i),
                (false, Some(from)) => {
                    out.push((from, i));
                    at = None;
                },
                _ => {},
            }
        }
        if let Some(from) = at {
            out.push((from, b.len()));
        }
        out
    }

    // The word starting at token `i`, as (tokens it takes up, words it counts for); (0, 0) if none.
    // The longest hyphen-joined entry of the list that the text spells wins. It counts for one
    // word, or for as many as it has parts where each part is itself a word, since hyphens also
    // join the words of a dashed passphrase and the safer count is the larger.
    fn word_at(&self, s: &str, t: &[(usize, usize)], i: usize) -> (usize, usize) {
        let mut chain = 1;
        while chain < self.parts && i + chain < t.len() && &s[t[i + chain - 1].1..t[i + chain].0] == "-" {
            chain += 1;
        }
        for k in (2..=chain).rev() {
            if self.words.contains(&s[t[i].0..t[i + k - 1].1].to_ascii_lowercase()) {
                let each = (i..i + k).all(|j| self.words.contains(&s[t[j].0..t[j].1].to_ascii_lowercase()));
                return (k, if each { k } else { 1 });
            }
        }
        if self.words.contains(&s[t[i].0..t[i].1].to_ascii_lowercase()) { (1, 1) } else { (0, 0) }
    }

    /// The byte ranges of `s` that hold `n` words or more in a row, each running from the first
    /// word's first letter to the last word's last, with whatever stood between the words.
    pub fn spans(&self, s: &str) -> Vec<(usize, usize)> {
        let mut out = Vec::new();
        if s.len() < self.n.saturating_mul(2).saturating_sub(1) {
            return out;
        }
        let t = Self::tokens(s.as_bytes());
        let (mut from, mut to, mut run) = (0usize, 0usize, 0usize);
        let mut i = 0;
        while i < t.len() {
            let (took, count) = self.word_at(s, &t, i);
            if took == 0 {
                if run >= self.n {
                    out.push((from, to));
                }
                run = 0;
                i += 1;
                continue;
            }
            if run == 0 {
                from = t[i].0;
            }
            to = t[i + took - 1].1;
            run += count;
            i += took;
        }
        if run >= self.n {
            out.push((from, to));
        }
        out
    }

    /// Does `s` hold `n` words of the list in a row?
    pub fn has(&self, s: &str) -> bool {
        !self.spans(s).is_empty()
    }
}

impl StrTest for Phrase {
    fn hit(&self, s: &str) -> bool { self.has(s) }
}

/// A place a caller knows holds a secret, whatever it is called.
#[derive(Clone, Debug)]
pub enum Deny {
    Name(String),           // a field of this exact name, at any depth
    Path(Vec<String>),      // the fields along this path from the root, where `*` is any one name
}

/// Walks a `Dat` and covers what looks secret.
///
/// A value is covered when its field's name is a secret's (`secret_name`, or the loose pattern if
/// asked), when its name or its path is in the deny-list, or, for a string, when the caller's
/// test fires. A covered scalar becomes its fingerprint, a covered object or list becomes
/// `[redacted:object]` without being walked, and an empty value becomes `[redacted:absent]`.
/// Lists are transparent to a deny path, so `["sess", "nonce"]` reaches the members of a list of
/// sessions. Map keys that are strings meet the caller's test as well, and a string under a
/// correlation-id field meets its `hit_id` instead (see [`Redact::id_key`]).
pub struct Redact<T: StrTest = NoTest> {
    deny:   Vec<Deny>,
    test:   T,
    head:   usize,
    loose:  bool,
    ids:    Vec<String>,    // fields whose strings are correlation ids
}

impl Redact<NoTest> {

    pub fn new() -> Self {
        Self {
            deny:   Vec::new(),
            test:   NoTest,
            head:   Self::DEFAULT_HEAD,
            loose:  false,
            ids:    scrub::ID_KEYS.iter().map(|k| k.to_string()).collect(),
        }
    }
}

impl Default for Redact<NoTest> {
    fn default() -> Self { Self::new() }
}

impl<T: StrTest> Redact<T> {

    pub const DEFAULT_HEAD: usize = 6;

    pub fn deny(mut self, d: Deny) -> Self {
        self.deny.push(d);
        self
    }

    pub fn deny_name(self, name: &str) -> Self {
        self.deny(Deny::Name(name.to_string()))
    }

    pub fn deny_path(self, path: &[&str]) -> Self {
        self.deny(Deny::Path(path.iter().map(|s| s.to_string()).collect()))
    }

    /// Takes `secret_name_loose` for the names, which catches `pushToken` and `refreshSecret`.
    pub fn loose(mut self) -> Self {
        self.loose = true;
        self
    }

    /// How many leading characters a fingerprint shows. Daimond shows 6.
    pub fn with_head(mut self, head: usize) -> Self {
        self.head = head;
        self
    }

    /// Names a field whose strings are correlation ids, which a reader joins rows by: a string
    /// under it, or under it through any lists, meets the test's `hit_id` and not its `hit`.
    /// Daimond's own list (`id`, `callId`, `turn`, `device`, `d`, `b`, `n` and the rest of
    /// `SCRUB_ID_KEYS`) is there from the start; an application adds the ids it stamps, such as a
    /// session or a connection.
    pub fn id_key(mut self, name: &str) -> Self {
        self.ids.push(name.to_string());
        self
    }

    pub fn with_test<U: StrTest>(self, test: U) -> Redact<U> {
        Redact {
            deny:   self.deny,
            test,
            head:   self.head,
            loose:  self.loose,
            ids:    self.ids,
        }
    }

    /// A deep copy with the secrets covered.
    pub fn dat(&self, d: &Dat) -> Dat {
        self.walk(d).0
    }

    /// `dat`, and whether anything was covered. A caller that finds nothing changed can keep the
    /// text it parsed the `Dat` from, byte for byte.
    pub fn walk(&self, d: &Dat) -> (Dat, bool) {
        let mut path = Vec::new();
        self.value(d, &mut path, 0, false)
    }

    /// A copy of the row with its payload covered. The payload's field names are the root's.
    pub fn row(&self, row: &Row) -> Row {
        let mut out = row.clone();
        let mut path = Vec::new();
        for (k, v) in out.body.iter_mut() {
            path.push(k.clone());
            let (nv, _) = self.member(k, v, &mut path, 1);
            path.pop();
            *v = nv;
        }
        out
    }

    /// The caller's string test.
    pub fn test(&self) -> &T { &self.test }

    /// The fingerprint this redactor writes over a string it covers, with its own head.
    pub fn mark(&self, s: &str) -> String {
        fingerprint(s, self.head)
    }

    /// The fingerprint of a free string if the caller's test fires on it.
    pub fn text(&self, s: &str) -> Option<String> {
        if self.test.hit(s) {
            Some(fingerprint(s, self.head))
        } else {
            None
        }
    }

    fn secret(&self, name: &str) -> bool {
        if self.loose { secret_name_loose(name) } else { secret_name(name) }
    }

    fn denied(&self, path: &[String]) -> bool {
        self.deny.iter().any(|d| match d {
            Deny::Name(n) => path.last().map_or(false, |l| l == n),
            Deny::Path(p) => p.len() == path.len()
                && p.iter().zip(path.iter()).all(|(a, b)| a == "*" || a == b),
        })
    }

    // One field of a map: covered by name or place, or walked.
    fn member(&self, name: &str, v: &Dat, path: &mut Vec<String>, depth: usize) -> (Dat, bool) {
        if self.secret(name) || self.denied(path) {
            return self.cover(v);
        }
        let id = self.ids.iter().any(|k| k == name);
        self.value(v, path, depth, id)
    }

    // The caller's test, in the form that suits a correlation id or any other string.
    fn hit(&self, s: &str, id: bool) -> bool {
        if id { self.test.hit_id(s) } else { self.test.hit(s) }
    }

    // A value covered by its field: a scalar's fingerprint, or a note of what it was.
    fn cover(&self, v: &Dat) -> (Dat, bool) {
        let s = match v {
            Dat::Empty              => return (Dat::Str(ABSENT.to_string()), true),
            Dat::Opt(o)             => match &**o {
                Some(x) => return self.cover(x),
                None    => return (Dat::Str(ABSENT.to_string()), true),
            },
            Dat::Box(b)             => return self.cover(b),
            Dat::Str(s)             => s.clone(),
            Dat::Bool(b)            => fmt!("{}", b),
            Dat::U8(n)              => fmt!("{}", n),
            Dat::U16(n)             => fmt!("{}", n),
            Dat::U32(n)             => fmt!("{}", n),
            Dat::U64(n)             => fmt!("{}", n),
            Dat::U128(n)            => fmt!("{}", n),
            Dat::I8(n)              => fmt!("{}", n),
            Dat::I16(n)             => fmt!("{}", n),
            Dat::I32(n)             => fmt!("{}", n),
            Dat::I64(n)             => fmt!("{}", n),
            Dat::I128(n)            => fmt!("{}", n),
            Dat::F32(n)             => fmt!("{}", n),
            Dat::F64(n)             => fmt!("{}", n),
            Dat::C64(n)             => fmt!("{}", n),
            // A list, a map, bytes: noted as present, never shown, as `[redacted:object]` is.
            _                       => return (Dat::Str(OBJECT.to_string()), true),
        };
        if marker(&s) {
            return (Dat::Str(s), false);
        }
        (Dat::Str(fingerprint(&s, self.head)), true)
    }

    fn key(&self, k: &Dat) -> (Dat, bool) {
        match k {
            Dat::Str(s) if self.test.hit(s) && !marker(s) => (Dat::Str(fingerprint(s, self.head)), true),
            _ => (k.clone(), false),
        }
    }

    fn value(&self, v: &Dat, path: &mut Vec<String>, depth: usize, id: bool) -> (Dat, bool) {
        let deep = depth > MAX_DEPTH;
        match v {
            Dat::Str(s) => {
                if !marker(s) && self.hit(s, id) {
                    return (Dat::Str(fingerprint(s, self.head)), true);
                }
            },
            Dat::Map(m) => {
                if deep {
                    return (Dat::Str(DEEP.to_string()), true);
                }
                let mut out = DaticleMap::new();
                let mut changed = false;
                for (k, val) in m {
                    let name = key_text(k);
                    let (mut nk, kc) = self.key(k);
                    while out.contains_key(&nk) {
                        nk = Dat::Str(fmt!("{}~", key_text(&nk)));
                    }
                    path.push(name.clone());
                    let (nv, vc) = self.member(&name, val, path, depth + 1);
                    path.pop();
                    changed |= kc | vc;
                    out.insert(nk, nv);
                }
                return (Dat::Map(out), changed);
            },
            Dat::OrdMap(m) => {
                if deep {
                    return (Dat::Str(DEEP.to_string()), true);
                }
                let mut out = OrdDaticleMap::new();
                let mut changed = false;
                for (mk, val) in m {
                    let name = key_text(mk.dat());
                    let (nk, kc) = self.key(mk.dat());
                    path.push(name.clone());
                    let (nv, vc) = self.member(&name, val, path, depth + 1);
                    path.pop();
                    changed |= kc | vc;
                    out.insert(MapKey::new(mk.ord(), nk), nv);
                }
                return (Dat::OrdMap(out), changed);
            },
            Dat::List(l) => {
                if deep {
                    return (Dat::Str(DEEP.to_string()), true);
                }
                let mut out = Vec::with_capacity(l.len());
                let mut changed = false;
                for x in l {
                    let (nx, c) = self.value(x, path, depth + 1, id);
                    changed |= c;
                    out.push(nx);
                }
                return (Dat::List(out), changed);
            },
            Dat::Box(b) => {
                let (nb, c) = self.value(b, path, depth, id);
                return (Dat::Box(Box::new(nb)), c);
            },
            Dat::Opt(o) => {
                if let Some(x) = &**o {
                    let (nx, c) = self.value(x, path, depth, id);
                    return (Dat::Opt(Box::new(Some(nx))), c);
                }
            },
            Dat::Usr(uid, Some(b)) => {
                let (nb, c) = self.value(b, path, depth, id);
                return (Dat::Usr(uid.clone(), Some(Box::new(nb))), c);
            },
            _ => {},
        }
        // A tuple is a list that has a length of its own.
        macro_rules! tuples {
            ($($t:ident),*) => {
                match v {
                    $(
                        Dat::$t(a) => {
                            if deep {
                                return (Dat::Str(DEEP.to_string()), true);
                            }
                            let mut b = a.clone();
                            let mut changed = false;
                            for x in b.iter_mut() {
                                let (nx, c) = self.value(x, path, depth + 1, id);
                                *x = nx;
                                changed |= c;
                            }
                            return (Dat::$t(b), changed);
                        },
                    )*
                    _ => {},
                }
            };
        }
        tuples!(Tup2, Tup3, Tup4, Tup5, Tup6, Tup7, Tup8, Tup9, Tup10);
        (v.clone(), false)
    }
}

fn key_text(k: &Dat) -> String {
    match k {
        Dat::Str(s) => s.clone(),
        other       => fmt!("{}", other),
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enc_needs_a_word_boundary_after_it() {
        // `enc\b`: the end of the name or a non-word character, and not `_`, a digit or a letter.
        assert!(secret_name("keyenc"));
        assert!(secret_name("keyENC"));
        assert!(secret_name("key.enc"));
        assert!(!secret_name("keyenc_x"));
        assert!(!secret_name("keyenc1"));
        assert!(!secret_name("keyencx"));
        assert!(secret_name("keyenc é"));
    }

    #[test]
    fn a_word_counts_only_at_the_start_or_after_a_separator() {
        assert!(secret_name("key"));
        assert!(secret_name("x_key"));
        assert!(secret_name("x.token-y"));
        assert!(!secret_name("xkey"));
        assert!(!secret_name("monkey"));
        assert!(!secret_name("keys"));
        assert!(!secret_name(""));
        assert!(secret_name_loose("monkey"), "key$ is boundary-free");
        assert!(!secret_name_loose("keyboard"));
    }

    #[test]
    fn a_phrase_is_n_list_words_in_a_row_whatever_stands_between() {
        let p = Phrase::new(["red", "green", "blue"], 3);
        assert!(p.hit("Red, GREEN; blue"));
        assert!(p.hit("red1green2blue"));
        assert!(!p.hit("red green"));
        assert!(!p.hit("red green purple blue"), "a stranger breaks the run");
        assert_eq!(p.spans("x red green blue y red"), vec![(2, 16)]);
    }

    #[test]
    fn a_hyphenated_entry_is_one_word_where_the_text_spells_it() {
        let p = Phrase::new(["red", "t-shirt", "blue"], 3);
        assert!(p.hit("red t-shirt blue"), "the entry is one word");
        assert!(!p.hit("red t shirt blue"), "and spelled otherwise it is not");
        assert!(!p.hit("red t-shirts blue"));
    }

    #[test]
    fn either_of_two_tests_is_a_test() {
        let t = Or(|s: &str| s.contains("one"), Phrase::new(["a", "b"], 2));
        assert!(t.hit("one") && t.hit("a b") && !t.hit("a c b"));
    }

    #[test]
    fn a_marker_is_recognised_so_it_is_not_covered_twice() {
        assert!(marker(&fingerprint("abcdefgh", 6)));
        assert!(marker(OBJECT) && marker(ABSENT) && marker(DEEP));
        assert!(!marker("[redacted"));
        assert!(!marker("not [redacted x]"));
    }

    #[test]
    fn the_stock_test_is_the_commit_hooks_and_ignores_its_excuses() {
        // A credential's shape, in two pieces so that this file passes the hook it tests.
        let k = fmt!("{}{}", "sk-ant", "-api03-AbCdEfGhIjKlMnOpQrStUvWx");
        assert!(Shapes.hit(&k));
        assert!(Shapes.hit(&fmt!("{} # allowlist secret", k)));
        assert!(!Shapes.hit("sk-ant-short"));
        // Covered whole, with none of its characters left standing in the fingerprint.
        let red = Redact::new().with_head(0).with_test(Shapes);
        let out = red.text(&fmt!("deployed with {}", k)).unwrap_or_default();
        assert!(out.starts_with("[redacted") && !out.contains("sk-ant"));
    }
}
