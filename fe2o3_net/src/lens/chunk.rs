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

use super::redact::StrTest;

use oxedyne_fe2o3_text::base64;

use std::collections::HashMap;


// The most bytes of a partial character that may stand at either end of a chunk's text.
const EDGE: usize = 3;

/// What a sink should do with one row of a post.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Verdict {
    Row,    // not a chunk, or a chunk that is not the base64 of text: guarded as any row is
    Clean,  // the base64 of text that the string test passes: kept as it stands
    Hit,    // the base64 of text that the string test fires on: covered whole
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
/// A run is decoded whole and its text meets `test`, so a secret that straddles a cut is seen and
/// the chunks on either side of it are all covered. A run that will not decode as one (a chunk
/// among them is not base64 of text) is judged chunk by chunk, and a chunk that will not decode
/// alone, such as one cut off a four character boundary and sent with no neighbour, is a plain row.
pub fn judge<T: StrTest>(test: &T, rows: &[(String, String)]) -> Vec<Verdict> {
    let mut out = vec![Verdict::Row; rows.len()];
    // The chunk rows by bundle, in the order the bundles first appear.
    let mut sets: Vec<(String, String)> = Vec::new();
    let mut by: HashMap<(String, String), Vec<(u32, usize)>> = HashMap::new();
    for (at, (tag, _)) in rows.iter().enumerate() {
        if let Some(t) = ChunkTag::parse(tag) {
            let key = (t.kind, t.id);
            if !by.contains_key(&key) {
                sets.push(key.clone());
            }
            by.entry(key).or_default().push((t.i, at));
        }
    }
    let verdict = |text: &str| if test.hit(text) { Verdict::Hit } else { Verdict::Clean };
    for key in sets {
        let mut members = match by.remove(&key) {
            Some(m) => m,
            None    => continue,
        };
        members.sort();
        // Runs of consecutive indices; a repeated index starts a run of its own.
        let mut runs: Vec<Vec<usize>> = Vec::new();
        let mut last = 0u32;
        for (i, at) in members {
            if runs.is_empty() || i != last + 1 {
                runs.push(Vec::new());
            }
            if let Some(run) = runs.last_mut() {
                run.push(at);
            }
            last = i;
        }
        for run in runs {
            let whole: String = run.iter().map(|&at| rows[at].1.as_str()).collect::<Vec<&str>>().concat();
            match text(&whole) {
                Some(t) => {
                    let v = verdict(&t);
                    for &at in &run {
                        out[at] = v;
                    }
                },
                None => {
                    for &at in &run {
                        if let Some(t) = text(&rows[at].1) {
                            out[at] = verdict(&t);
                        }
                    }
                },
            }
        }
    }
    out
}


#[cfg(test)]
mod tests {
    use super::*;

    // Any closure is a string test, so the judge is tried with a plain word.
    fn secret(s: &str) -> bool { s.contains("SECRET") }

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
    fn the_chunks_of_a_bundle_are_judged_as_one_text() {
        // The word is cut in two on a quantum boundary: neither half holds it, together they do.
        let enc = base64::encode_url(b"xxxSECRETxxx");
        let (a, b) = enc.split_at(8);
        let alone = [row("ds snapshot s 1/2", a)];
        assert_eq!(judge(&secret, &alone), vec![Verdict::Clean]);
        let both = [row("ds snapshot s 2/2", b), row("ds snapshot s 1/2", a), row("diag", "SECRET")];
        // The order of the post does not matter, and an untagged row is left to the sink.
        assert_eq!(judge(&secret, &both), vec![Verdict::Hit, Verdict::Hit, Verdict::Row]);
    }
}
