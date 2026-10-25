//! MIME text decoding: RFC 2047 encoded-words in header values, quoted-printable bodies and
//! the whitespace-wrapped base64 of a MIME part.
//!
//! Moved from `fe2o3_mail::message` on 2026-10-26 so that `fe2o3_net`'s IMAP SEARCH, which
//! `fe2o3_mail` depends on, matches header values decoded, and the sans-io message reader,
//! which builds without `fe2o3_net`, reads them through the same code.
//!
//! A word in a charset other than UTF-8 is read as UTF-8 with invalid sequences replaced,
//! not transcoded: the ASCII every charset shares is exact.

use crate::base64;


/// Decode RFC 2047 encoded-words (`=?charset?B?...?=` or `?Q?`) in a header value.
///
/// Adjacent encoded-words separated only by whitespace are joined with none, which is what
/// the standard says to do so a name split across two words reads as one.
pub fn decode_words(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    // Whether the previous token emitted was itself an encoded-word, so the whitespace
    // before the next one can be dropped where that next one is also encoded.
    let mut last_was_word = false;
    while i < bytes.len() {
        if bytes[i] == b'=' && i + 1 < bytes.len() && bytes[i + 1] == b'?' {
            if let Some((decoded, next)) = decode_one_word(s, i) {
                // Drop the run of whitespace this word was separated from the previous
                // encoded-word by.
                if last_was_word {
                    while out.ends_with(' ') || out.ends_with('\t')
                        || out.ends_with('\r') || out.ends_with('\n')
                    {
                        out.pop();
                    }
                }
                out.push_str(&decoded);
                i = next;
                last_was_word = true;
                continue;
            }
        }
        // Any ordinary character.  A non-whitespace one means the run of encoded-words has
        // ended, so a later word no longer joins to an earlier one.
        let ch = s[i..].chars().next().unwrap_or('\u{fffd}');
        if !ch.is_whitespace() {
            last_was_word = false;
        }
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

/// Decode the one encoded-word beginning at `start`, returning it and the offset just past
/// it, or `None` when the bytes there are not a well-formed encoded-word.
fn decode_one_word(s: &str, start: usize) -> Option<(String, usize)> {
    // `=?charset?enc?text?=`
    let rest = &s[start + 2..];
    let q1 = match rest.find('?') {
        Some(n) => n,
        None    => return None,
    };
    let charset = &rest[..q1];
    let after_cs = &rest[q1 + 1..];
    let q2 = match after_cs.find('?') {
        Some(n) => n,
        None    => return None,
    };
    let enc = &after_cs[..q2];
    let after_enc = &after_cs[q2 + 1..];
    let q3 = match after_enc.find("?=") {
        Some(n) => n,
        None    => return None,
    };
    let text = &after_enc[..q3];
    if charset.is_empty() || enc.len() != 1 {
        return None;
    }
    let end = start + 2 + q1 + 1 + q2 + 1 + q3 + 2;
    let raw = match enc.as_bytes()[0].to_ascii_lowercase() {
        b'b' => decode_base64_lenient(text),
        b'q' => decode_q(text),
        _    => return None,
    };
    Some((String::from_utf8_lossy(&raw).into_owned(), end))
}

/// Decode a `Q` encoded-word body: `_` is a space, `=HH` is a byte, everything else stands.
fn decode_q(s: &str) -> Vec<u8> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'_' => { out.push(b' '); i += 1; },
            b'=' if i + 2 < b.len() => {
                match hex2(b[i + 1], b[i + 2]) {
                    Some(v) => { out.push(v); i += 3; },
                    None    => { out.push(b'='); i += 1; },
                }
            },
            other => { out.push(other); i += 1; },
        }
    }
    out
}

/// Decode a quoted-printable body to bytes: `=\n` soft breaks vanish, `=HH` is one byte.
pub fn decode_qp(s: &str) -> Vec<u8> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'=' {
            // A soft line break: `=` at end of line.
            if i + 1 < b.len() && b[i + 1] == b'\n' { i += 2; continue; }
            if i + 2 < b.len() && b[i + 1] == b'\r' && b[i + 2] == b'\n' { i += 3; continue; }
            if i + 2 < b.len() {
                if let Some(v) = hex2(b[i + 1], b[i + 2]) {
                    out.push(v);
                    i += 3;
                    continue;
                }
            }
        }
        out.push(b[i]);
        i += 1;
    }
    out
}

/// Base64, tolerant of the line breaks and whitespace a MIME body wraps it in.
pub fn decode_base64_lenient(s: &str) -> Vec<u8> {
    let cleaned: String = s.chars().filter(|c| !c.is_whitespace()).collect();
    if cleaned.is_empty() {
        return Vec::new();
    }
    match base64::decode(&cleaned) {
        Ok(v)  => v,
        // A body that will not decode is handed back as its own bytes rather than lost: a
        // reader seeing the encoded text is better served than one seeing nothing.
        Err(_) => s.as_bytes().to_vec(),
    }
}

/// Two hex digits to a byte, or `None` when either is not hex.
fn hex2(a: u8, b: u8) -> Option<u8> {
    match ((a as char).to_digit(16), (b as char).to_digit(16)) {
        (Some(hi), Some(lo))    => Some((hi * 16 + lo) as u8),
        _                       => None,
    }
}
