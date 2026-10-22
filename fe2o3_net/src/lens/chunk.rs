//! Chunk rows: a bundle sent as base64 in `ds <kind> <id> i/N` rows, and the guard that reads them.
//!
//! Daimond sends its telemetry and its snapshots, and Oxegen's test page its snapshots, as minified
//! JSON that the page has already scrubbed whole, base64-encoded and cut into rows of about three
//! hundred characters. A content guard that looks at a row's text cannot be left to judge such a
//! row as it stands: base64 is a long, unbroken, high-entropy run by construction, which is exactly
//! what the entropy catch of [`Shapes`](super::Shapes) fingerprints, so every chunk of every bundle
//! was stored as a marker and no bundle could be read back (found on the first end-to-end run of the
//! Oxegen test site, 2026-10-03).
//!
//! The fix is not to exempt the rows from the guard but to put the guard where the text is. A row
//! whose tag is strictly a chunk's ([`ChunkTag`]) and whose payload is verifiably the base64 of text
//! ([`text`]) is judged by the decoded text, which meets the caller's whole string test: the shapes,
//! the entropy catch, and whatever the app adds (Oxegen's eight words). A hit covers the payload by
//! its fingerprint, and a reader then finds the bundle broken. Anything else about a tagged row is
//! guarded exactly as any row is: a payload that is not base64, or whose bytes are not text (an
//! image sent under a snapshot's tag), is not exempt from anything.
//!
//! A secret can straddle a cut, so the chunks of one bundle in one post are judged together, in
//! order, as the run of text they make ([`judge`]). A bundle that crosses two posts is judged
//! chunk by chunk and run by run within each, which is how the pages send it: a bundle is one post.
//!
//! A bundle whose text parses as JSON meets the sink's whole [`Redact`] as well, and not the string
//! test alone (2026-10-04, F5 of the Fable QA). A plain row is held to its field names, its
//! deny-list and its strings, so a `passphrase` field inside a snapshot was kept where the same
//! field in a row would have been covered; and a run of words was missed whenever a letter-only key
//! stood between them, since the test read the JSON's own text. The words are now also sought in
//! the bundle's string values joined in order, which no key can break, and what is found is
//! covered where it stands. The covered JSON is encoded again in the alphabet it came in, and cut
//! into the rows it came in ([`Verdict::Cover`]). Where it will not fit them, or the rows are not
//! the whole bundle, the whole is covered instead, which is the safe side of the same choice.
//!
//! Two things were added the same day (N2 of the Opus QA). A bundle that is the whole of its post
//! and opens as an object or an array, but that the strict reader refuses (a member named twice,
//! half a surrogate pair, nesting past the limit), cannot be walked, and is now covered whole where
//! it had been stored as it came: another reader takes all three, so a reader's refusing them was
//! no defence. And the runs of words are sought before the walk, across the values, across the names
//! and across both, since the walk covers a field named for a secret in the middle of a run and
//! leaves the two halves too short to be one.

use super::{
    redact::{
        marker,
        Redact,
        StrTest,
    },
    row::compact,
};

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_jdat::{
    bdat::limits::DecodeLimits,
    prelude::*,
};
use oxedyne_fe2o3_text::base64;

use std::collections::HashMap;


// The most bytes of a partial character that may stand at either end of a chunk's text.
const EDGE: usize = 3;

/// What a sink should do with one row of a post.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Verdict {
    Row,                // not a chunk, or a chunk that is not the base64 of text: guarded as any row is
    Clean,              // the base64 of text that the guard passes: kept as it stands
    Hit,                // the base64 of text that the string test fires on: covered whole
    Cover(String),      // the base64 of JSON with something covered in it: the row's data, replaced
}

/// The tag of a chunk row, `ds snapshot|telemetry <id> <i>/<N>`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChunkTag {
    pub kind:   String,     // snapshot or telemetry
    pub id:     String,     // the bundle's id, one word
    pub i:      u32,        // this chunk, from 1
    pub n:      u32,        // chunks in the bundle
}

impl ChunkTag {

    /// The tag, if it is exactly a chunk's: four words split by single spaces, a kind of
    /// `snapshot` or `telemetry`, an id, and `i/N` in plain digits with `1 <= i <= N`. Anything
    /// else (another kind, a double space, a `0/3`, a `4/3`, a trailing word, a clipped tag) is not.
    pub fn parse(tag: &str) -> Option<Self> {
        let mut it = tag.split(' ');
        if it.next() != Some("ds") {
            return None;
        }
        let kind = it.next()?;
        if kind != "snapshot" && kind != "telemetry" {
            return None;
        }
        let id = it.next()?;
        let frac = it.next()?;
        if id.is_empty() || it.next().is_some() {
            return None;
        }
        let (i, n) = frac.split_once('/')?;
        let num = |s: &str| -> Option<u32> {
            if s.is_empty() || s.len() > 9 || !s.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            s.parse::<u32>().ok()
        };
        let (i, n) = (num(i)?, num(n)?);
        if i < 1 || i > n {
            return None;
        }
        Some(Self { kind: kind.to_string(), id: id.to_string(), i, n })
    }
}

/// The text a chunk's payload encodes, if it is verifiably the base64 of text.
///
/// The payload may be Daimond's padded `A-Za-z0-9+/` or the page's unpadded `A-Za-z0-9-_`, and
/// white space in it is ignored, which is how the first Oxegen pages wrapped it (words of sixteen,
/// as MIME does). The decoders are `fe2o3_text::base64`'s, which are strict, so a payload that
/// is nearly base64 is not. The bytes must also be UTF-8, allowing a partial character at either
/// end where a cut fell, so that a plain-text payload made of letters, which decodes to bytes
/// that are anything but text, is not mistaken for base64 and let through unread.
pub fn text(payload: &str) -> Option<String> {
    let flat: String = payload.chars().filter(|c| !c.is_ascii_whitespace()).collect();
    let std_only = flat.bytes().any(|b| matches!(b, b'+' | b'/' | b'='));
    let url_only = flat.bytes().any(|b| matches!(b, b'-' | b'_'));
    let bytes = match (std_only, url_only) {
        (true, true)    => return None,
        (true, false)   => base64::decode(&flat).ok()?,
        (false, _)      => base64::decode_url(&flat).ok()?,
    };
    // Text, give or take the edges a cut can fall inside.
    let mut lo = 0;
    while lo < bytes.len().min(EDGE) && bytes[lo] & 0xC0 == 0x80 {
        lo += 1;
    }
    let body = &bytes[lo..];
    match std::str::from_utf8(body) {
        Ok(_)   => {},
        Err(e)  => {
            // Only an unfinished character at the very end is allowed.
            if e.error_len().is_some() || body.len() - e.valid_up_to() > EDGE {
                return None;
            }
        },
    }
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

/// The verdict on each row of a post, given as `(tag, data)` exactly as they will be stored.
///
/// The chunks of one bundle are taken in order of index and cut into runs of consecutive indices.
/// A run is decoded whole and its text meets the guard, so a secret that straddles a cut is seen and
/// the chunks on either side of it are all covered. A run that will not decode as one (a chunk
/// among them is not base64 of text) is judged chunk by chunk, and a chunk that will not decode
/// alone, such as one cut off a four character boundary and sent with no neighbour, is a plain row.
///
/// The guard is `red`'s: its string test over the text as it stands, then, if the text is JSON,
/// its names and strings over the parsed value and its word runs over the strings joined in
/// order. `cap` is the most bytes a row's data may hold, which a covered re-encoding is cut to fit.
pub fn judge<T: StrTest>(red: &Redact<T>, cap: usize, rows: &[(String, String)]) -> Vec<Verdict> {
    let mut out = vec![Verdict::Row; rows.len()];
    // The chunk rows by bundle, in the order the bundles first appear.
    let mut sets: Vec<(String, String)> = Vec::new();
    let mut by: HashMap<(String, String), Vec<(u32, usize, u32)>> = HashMap::new();
    for (at, (tag, _)) in rows.iter().enumerate() {
        if let Some(t) = ChunkTag::parse(tag) {
            let key = (t.kind, t.id);
            if !by.contains_key(&key) {
                sets.push(key.clone());
            }
            by.entry(key).or_default().push((t.i, at, t.n));
        }
    }
    for key in sets {
        let mut members = match by.remove(&key) {
            Some(m) => m,
            None    => continue,
        };
        members.sort();
        // Runs of consecutive indices; a repeated index starts a run of its own.
        let mut runs: Vec<Vec<(u32, usize, u32)>> = Vec::new();
        let mut last = 0u32;
        for m in members {
            if runs.is_empty() || m.0 != last + 1 {
                runs.push(Vec::new());
            }
            last = m.0;
            if let Some(run) = runs.last_mut() {
                run.push(m);
            }
        }
        for run in runs {
            let at: Vec<usize> = run.iter().map(|&(_, at, _)| at).collect();
            let k = at.len();
            // Whole if it starts at the first chunk and every row of it says the set is this long.
            let entire = run.first().map_or(false, |m| m.0 == 1) && run.iter().all(|m| m.2 as usize == k);
            let flat: String = at.iter().map(|&a| rows[a].1.as_str()).collect::<Vec<&str>>().concat();
            match text(&flat) {
                Some(t) => {
                    let found = of_text(red, cap, &t, &flat, k, entire);
                    for (&a, v) in at.iter().zip(found) {
                        out[a] = v;
                    }
                },
                None => {
                    for &a in &at {
                        if let Some(t) = text(&rows[a].1) {
                            out[a] = if red.test().hit(&t) { Verdict::Hit } else { Verdict::Clean };
                        }
                    }
                },
            }
        }
    }
    out
}

// What is to be done with a text that is verifiably a run of chunks, `k` rows whose data was `flat`.
fn of_text<T: StrTest>(red: &Redact<T>, cap: usize, text: &str, flat: &str, k: usize, entire: bool) -> Vec<Verdict> {
    if red.test().hit(text) {
        return vec![Verdict::Hit; k];
    }
    let covered = match cover(red, text, entire) {
        Seen::Plain         => return vec![Verdict::Clean; k],
        Seen::Whole         => return vec![Verdict::Hit; k],
        Seen::Covered(c)    => c,
    };
    // Covering changes the text, so it cannot be re-cut where some of the set is elsewhere.
    if !entire {
        return vec![Verdict::Hit; k];
    }
    match recut(&covered, flat, k, cap) {
        Some(rows)  => rows.into_iter().map(Verdict::Cover).collect(),
        None        => vec![Verdict::Hit; k],
    }
}

// What a text asks of the redactor.
enum Seen {
    Plain,              // not JSON, or JSON with nothing to cover
    Covered(String),    // JSON with something covered, as compact JSON
    Whole,              // JSON that cannot be covered in part
}

// Which of a structure's strings a run of words is read across.
#[derive(Clone, Copy, Eq, PartialEq)]
enum Reads {
    Values,     // the string values: no member name stands between the words
    Names,      // the member names: no value stands between the words
    Both,       // the names and the values, in the order the document gives them
}

// A text that parses as JSON, put through `red`: the runs of words first, across its strings joined
// in order, then its names and strings by the walk. The runs come first because the walk covers a
// field named for a secret in the middle of a run, and the two halves left over are each too short
// to be one. `entire` says the text is the whole of a bundle.
fn cover<T: StrTest>(red: &Redact<T>, text: &str, entire: bool) -> Seen {
    let parsed = match Dat::decode_json_strict_ordered(text, &DecodeLimits::default()) {
        Ok(d)   => d,
        // A whole bundle that the strict reader refuses (a member named twice, half a surrogate pair,
        // nesting past the limit) is one that a lenient reader takes, and it cannot be walked, so it
        // is covered as a whole and never stored as it came. A text that does not open as a bundle is
        // no bundle: the string test has had it.
        Err(_)  => return if entire && opens(text) { Seen::Whole } else { Seen::Plain },
    };
    let mut d = parsed;
    let mut changed = false;
    for reads in [Reads::Values, Reads::Names, Reads::Both] {
        let strs = strings_of(&d, reads);
        let spans = red.test().joined_spans(&strs.join("\n"));
        if spans.is_empty() {
            continue;
        }
        // Each string that a run touches is covered, as a plain row's string would be.
        let mut at = 0;
        let mut take = Vec::with_capacity(strs.len());
        for l in &strs {
            take.push(spans.iter().any(|&(a, b)| a < at + l.len() && b > at));
            at += l.len() + 1;
        }
        let mut i = 0;
        d = map_strings(&d, reads, &mut |s| {
            i += 1;
            if take[i - 1] && !marker(s) { Some(red.mark(s)) } else { None }
        });
        changed = true;
    }
    // Covering must leave nothing of a run, or it has not covered it.
    for reads in [Reads::Values, Reads::Names, Reads::Both] {
        if !red.test().joined_spans(&strings_of(&d, reads).join("\n")).is_empty() {
            return Seen::Whole;
        }
    }
    let (walked, c) = red.walk(&d);
    if !changed && !c {
        return Seen::Plain;
    }
    match compact(&walked) {
        Ok(c)   => Seen::Covered(c),
        Err(_)  => Seen::Whole,
    }
}

// Does the text open as a JSON object or array, as a bundle does, whatever follows?
fn opens(text: &str) -> bool {
    text.trim_start_matches([' ', '\t', '\n', '\r']).starts_with(['{', '['])
}

fn strings_of(d: &Dat, reads: Reads) -> Vec<String> {
    let mut out = Vec::new();
    map_strings(d, reads, &mut |s| {
        out.push(s.to_string());
        None
    });
    out
}

// A copy of `d` in which each string that `reads` names, in the order written, is given to `f`, which
// may answer a replacement. A member's name comes before its value. JSON makes only maps, lists and
// strings of the kinds that hold others, so those are all that is walked.
fn map_strings<F: FnMut(&str) -> Option<String>>(d: &Dat, reads: Reads, f: &mut F) -> Dat {
    let name = |k: &Dat, f: &mut F| match k {
        Dat::Str(s) if reads != Reads::Values   => Dat::Str(f(s).unwrap_or_else(|| s.clone())),
        other                                   => other.clone(),
    };
    match d {
        Dat::Str(s) if reads == Reads::Names    => Dat::Str(s.clone()),
        Dat::Str(s)                             => Dat::Str(f(s).unwrap_or_else(|| s.clone())),
        Dat::List(l)    => Dat::List(l.iter().map(|x| map_strings(x, reads, f)).collect()),
        Dat::Map(m)     => {
            let mut out = DaticleMap::new();
            for (k, v) in m {
                let mut nk = name(k, f);
                // Two names covered the same way must stay two members.
                while out.contains_key(&nk) {
                    nk = Dat::Str(fmt!("{}~", match &nk { Dat::Str(s) => s.as_str(), _ => "" }));
                }
                out.insert(nk, map_strings(v, reads, f));
            }
            Dat::Map(out)
        },
        Dat::OrdMap(m)  => {
            let mut at: Vec<(&MapKey, &Dat)> = m.iter().collect();
            at.sort_by_key(|(k, _)| k.ord());
            let mut out = OrdDaticleMap::new();
            for (k, v) in at {
                let nk = name(k.dat(), f);
                out.insert(MapKey::new(k.ord(), nk), map_strings(v, reads, f));
            }
            Dat::OrdMap(out)
        },
        other           => other.clone(),
    }
}

// The text as base64 in the alphabet `like` is written in, cut into `k` rows of `cap` bytes at most,
// or none if it will not go. A row takes whole quanta, as the pages cut them, so every row but the
// last decodes alone, and the rows are filled evenly. A text of fewer quanta than rows leaves the
// last ones empty, which a reader joins as nothing.
fn recut(text: &str, like: &str, k: usize, cap: usize) -> Option<Vec<String>> {
    let std = like.bytes().any(|b| matches!(b, b'+' | b'/' | b'='));
    let enc = if std { base64::encode(text.as_bytes()) } else { base64::encode_url(text.as_bytes()) };
    let per = (enc.len().div_ceil(4)).div_ceil(k.max(1)) * 4;
    if per == 0 || per > cap {
        return None;
    }
    let mut rows: Vec<String> = enc.as_bytes().chunks(per).map(|c| String::from_utf8_lossy(c).into_owned()).collect();
    rows.resize(k, String::new());
    Some(rows)
}


#[cfg(test)]
mod tests {
    use super::*;
    use super::super::redact::Phrase;

    // Any closure is a string test, so the judge is tried with a plain word.
    fn secret(s: &str) -> bool { s.contains("SECRET") }

    fn red() -> Redact<fn(&str) -> bool> { Redact::new().with_head(0).with_test(secret as fn(&str) -> bool) }

    fn row(tag: &str, data: &str) -> (String, String) { (tag.to_string(), data.to_string()) }

    #[test]
    fn a_tag_is_a_chunks_only_when_it_is_exactly_so() {
        assert_eq!(ChunkTag::parse("ds snapshot a-1 3/7").map(|t| (t.i, t.n)), Some((3, 7)));
        for bad in ["ds snapshot a 0/7", "ds snapshot a 8/7", "ds snapshot a 3/7 x", "ds  snapshot a 3/7", "ds shot a 3/7", "ds snapshot a 3-7"] {
            assert_eq!(ChunkTag::parse(bad), None, "{}", bad);
        }
    }

    #[test]
    fn a_payload_is_text_or_it_is_nothing() {
        assert_eq!(text("U0VDUkVU").as_deref(), Some("SECRET"));
        assert_eq!(text("U0VD UkVU").as_deref(), Some("SECRET"), "white space is skipped");
        assert_eq!(text("U0VDUkVU=="), None, "padding that is not there is refused");
        // Letters alone, taken for base64, make bytes that are not text.
        assert_eq!(text("thequickbrownfoxjumpsoverthelazydog"), None);
    }

    #[test]
    fn a_recut_text_reads_back_in_the_alphabet_it_came_in() {
        let t = "{\"a\":\"\u{e9}\u{e9}\u{e9}?>>\"}";
        for (like, std) in [("Zm9v", false), ("Zm9v+/", true), ("Zm9v=", true)] {
            let rows = recut(t, like, 3, 400).unwrap_or_default();
            assert_eq!(rows.len(), 3, "{}", like);
            // Whole quanta to a row, so the padding of the standard form stands in the last alone.
            assert!(rows[..2].iter().all(|r| r.len() % 4 == 0 && !r.contains('=')), "{}", like);
            let flat = rows.concat();
            let back = if std { base64::decode(&flat) } else { base64::decode_url(&flat) };
            assert_eq!(back.map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default(), t, "{}", like);
        }
        // A short text still fills the rows it came in, with nothing at the end.
        let rows = recut("{}", "Zm9v", 4, 400).unwrap_or_default();
        assert_eq!(rows, vec!["e30".to_string(), String::new(), String::new(), String::new()]);
        // And one that will not go in the rows is refused, to the byte.
        let big = "x".repeat(300);
        assert!(recut(&big, "Zm9v", 1, 400).is_some());
        assert!(recut(&big, "Zm9v", 1, 399).is_none());
        assert!(recut(&big, "Zm9v", 2, 400).is_some(), "two rows hold what one cannot");
    }

    #[test]
    fn json_is_covered_by_its_names_and_its_strings_joined_in_order() {
        let enc = |t: &str| base64::encode_url(t.as_bytes());
        let one = |t: &str| vec![("ds snapshot j 1/1".to_string(), enc(t))];
        let phrase = Redact::new().with_head(0).with_test(Phrase::new(["red", "green", "blue"], 3));
        let verdicts = |t: &str| judge(&phrase, 400, &one(t));
        // Keys of letters between the words, which broke the run of the text as it stands.
        assert!(!phrase.test().has("{\"a\":\"red\",\"b\":\"green\",\"c\":\"blue\"}"));
        assert!(matches!(verdicts("{\"a\":\"red\",\"b\":\"green\",\"c\":\"blue\"}")[..], [Verdict::Cover(_)]));
        // In the order of the document and not of the names, which a plain map would sort: `bb` is
        // after `c` where it was written, and between `b` and `c` where it would be sorted.
        assert!(matches!(verdicts("{\"a\":\"red\",\"b\":\"green\",\"c\":\"blue\",\"bb\":\"x\"}")[..], [Verdict::Cover(_)]));
        assert!(matches!(verdicts("{\"a\":\"red\",\"b\":\"green\",\"bb\":\"x\",\"c\":\"blue\"}")[..], [Verdict::Clean]));
        // A name, with nothing for the string test to find.
        assert!(matches!(verdicts("{\"seed\":\"x\"}")[..], [Verdict::Cover(_)]));
        assert_eq!(verdicts("{\"state\":\"live\"}"), vec![Verdict::Clean]);
        assert_eq!(verdicts("not json {"), vec![Verdict::Clean]);
    }

    #[test]
    fn a_bundle_the_strict_reader_refuses_is_covered_whole() {
        let enc = |t: &str| base64::encode_url(t.as_bytes());
        let one = |t: &str| vec![("ds snapshot r 1/1".to_string(), enc(t))];
        let red = Redact::new().with_head(0).with_test(Phrase::new(["ant", "bee", "cow"], 3));
        let deep = ["{\"passphrase\":\"hunter2\",\"d\":", &"[".repeat(65), &"]".repeat(65), "}"].concat();
        // Each of these is a bundle to a lenient reader, and to none that is strict: a name twice,
        // half a surrogate pair (which a page makes by cutting a string inside an emoji), and nesting
        // one level past the limit. Held as it came, the field named for a secret would be stored.
        let refused = [
            ("a name twice",            "{\"passphrase\":\"hunter2\",\"passphrase\":\"x\"}"),
            ("half a surrogate pair",   "{\"passphrase\":\"hunter2\",\"t\":\"\\ud83d\"}"),
            ("nesting past the limit",  deep.as_str()),
            ("blanks before it",        " \n\t{\"passphrase\":\"hunter2\",\"passphrase\":\"x\"}"),
            ("an array",                "[\"a\",{\"passphrase\":\"hunter2\",\"passphrase\":\"x\"}]"),
        ];
        for (what, t) in refused {
            assert!(Dat::decode_json_strict_ordered(t, &DecodeLimits::default()).is_err(), "{}: refused", what);
            assert_eq!(judge(&red, 400, &one(t)), vec![Verdict::Hit], "{}: covered whole", what);
        }
        // The same field in a bundle that is read is covered where it stands.
        assert!(matches!(judge(&red, 400, &one("{\"passphrase\":\"hunter2\"}"))[..], [Verdict::Cover(_)]));
        // What does not open as an object or an array is no bundle, and is left to the string test.
        for t in ["not json {", "\"\\ud83d\""] {
            assert_eq!(judge(&red, 400, &one(t)), vec![Verdict::Clean], "{}", t);
        }
        // A run of chunks that is not the whole bundle cannot be read as one, so it is not refused as one.
        let half = vec![("ds snapshot r 1/2".to_string(), enc("{\"passphrase\":\"hunter2\",\"passphrase\":"))];
        assert_eq!(judge(&red, 400, &half), vec![Verdict::Clean]);
    }

    #[test]
    fn a_word_run_is_sought_before_the_walk_and_in_the_names_as_well() {
        let enc = |t: &str| base64::encode_url(t.as_bytes());
        let stored = |red: &Redact<Phrase>, t: &str| -> String {
            match judge(red, 400, &[("ds snapshot w 1/1".to_string(), enc(t))]).pop() {
                Some(Verdict::Cover(d))  => text(&d).unwrap_or_default(),
                other                   => fmt!("{:?}", other),
            }
        };
        let words = ["ant", "bee", "cow", "dog", "eel"];
        let five = Redact::new().with_head(0).with_test(Phrase::new(words, 5));
        let three = Redact::new().with_head(0).with_test(Phrase::new(words, 3));
        let gone = |t: &str| words.iter().all(|w| !t.contains(&fmt!("\"{}\"", w)));
        // A field named for a secret, inside the run: the walk covers its value, and the run is then
        // two short ones that no longer reach five, so the other four were left standing.
        let t = stored(&five, "{\"a\":\"ant\",\"b\":\"bee\",\"seed\":\"cow\",\"c\":\"dog\",\"d\":\"eel\"}");
        assert!(gone(&t) && t.matches("[redacted").count() == 5, "all five are covered: {}", t);
        // Words that are the names, with nothing between them that is a string.
        let t = stored(&three, "{\"ant\":true,\"bee\":true,\"cow\":true}");
        assert!(gone(&t) && t.matches("[redacted").count() == 3, "the three names are covered: {}", t);
        // Names and values in turn, which neither alone makes a run of. The `true` between breaks the
        // run in the text as it stands, and no string is in its way.
        let t = stored(&five, "{\"ant\":\"bee\",\"cow\":true,\"dog\":\"eel\"}");
        assert!(gone(&t) && t.matches("[redacted").count() == 5, "the five strings are covered: {}", t);
        // Letters that are not on the list still break a run, as they do in a row's text.
        assert_eq!(judge(&three, 400, &[("ds snapshot w 1/1".to_string(), enc("{\"ant\":\"bee\",\"x\":1,\"cow\":\"y\"}"))]), vec![Verdict::Clean]);
    }

    #[test]
    fn the_chunks_of_a_bundle_are_judged_as_one_text() {
        // The word is cut in two on a quantum boundary: neither half holds it, together they do.
        let enc = base64::encode_url(b"xxxSECRETxxx");
        let (a, b) = enc.split_at(8);
        let alone = [row("ds snapshot s 1/2", a)];
        assert_eq!(judge(&red(), 400, &alone), vec![Verdict::Clean]);
        let both = [row("ds snapshot s 2/2", b), row("ds snapshot s 1/2", a), row("diag", "SECRET")];
        // The order of the post does not matter, and an untagged row is left to the sink.
        assert_eq!(judge(&red(), 400, &both), vec![Verdict::Hit, Verdict::Hit, Verdict::Row]);
    }
}
