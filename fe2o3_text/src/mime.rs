//! MIME text decoding: RFC 2047 encoded-words in header values, quoted-printable and the
//! whitespace-wrapped base64 of a MIME part, the leaf parts of a message walked by boundary,
//! and the charsets a mail body arrives in, as UTF-8.
//!
//! Moved from `fe2o3_mail::message` on 2026-10-26 so that `fe2o3_net`'s IMAP SEARCH, which
//! `fe2o3_mail` depends on, matches header values decoded, and the sans-io message reader,
//! which builds without `fe2o3_net`, reads them through the same code.
//!
//! 2026-10-26 (F2b): the part walker joined it, for SEARCH BODY and TEXT and the message
//! reader alike. It works on bytes, not text, since a Latin-1 body is not UTF-8 and would lose
//! its accents to a lossy read before its charset was known. ISO-8859-1, ISO-8859-15 and
//! Windows-1252 are transcoded; any other charset is read as UTF-8 with invalid sequences
//! replaced, which keeps the ASCII every charset shares.

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
    // RFC 2231 lets a language follow the charset: `iso-8859-1*fr`.
    let cs = charset.split('*').next().unwrap_or(charset);
    Some((to_utf8(&raw, Some(cs)), end))
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
pub fn decode_qp<B: AsRef<[u8]> + ?Sized>(s: &B) -> Vec<u8> {
    let b = s.as_ref();
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
pub fn decode_base64_lenient<B: AsRef<[u8]> + ?Sized>(s: &B) -> Vec<u8> {
    let s = s.as_ref();
    let cleaned: String = s.iter()
        .filter(|b| !b.is_ascii_whitespace())
        .map(|b| *b as char)
        .collect();
    if cleaned.is_empty() {
        return Vec::new();
    }
    match base64::decode(&cleaned) {
        Ok(v)  => v,
        // A body that will not decode is handed back as its own bytes rather than lost: a
        // reader seeing the encoded text is better served than one seeing nothing.
        Err(_) => s.to_vec(),
    }
}

/// Two hex digits to a byte, or `None` when either is not hex.
fn hex2(a: u8, b: u8) -> Option<u8> {
    match ((a as char).to_digit(16), (b as char).to_digit(16)) {
        (Some(hi), Some(lo))    => Some((hi * 16 + lo) as u8),
        _                       => None,
    }
}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ PARTS                                                                     │
// └───────────────────────────────────────────────────────────────────────────┘

const MAX_DEPTH: usize = 16; // nesting followed, against a hostile message

/// One entity of a MIME message, a part or the message itself, with its headers read and its
/// body still in its transfer encoding.
#[derive(Clone, Debug)]
pub struct Part<'a> {
    pub ctype:          String,     // the Content-Type value, `text/plain` when absent
    pub encoding:       String,     // Content-Transfer-Encoding, lower case, empty when absent
    pub disposition:    String,     // Content-Disposition, as given
    pub head:           &'a [u8],   // the header block
    pub body:           &'a [u8],
    pub structure:      Structure<'a>,
}

/// What an entity holds, by RFC 2046: content, a multipart's parts, or an encapsulated
/// message, which is a message in its own right and not part of its parent's text.
#[derive(Clone, Debug)]
pub enum Structure<'a> {
    Leaf,
    Multipart(Vec<Part<'a>>),
    Message(Box<Part<'a>>),
}

impl<'a> Part<'a> {
    /// The media type alone, lower case: `text/plain`.
    pub fn media_type(&self) -> String {
        media_type(&self.ctype)
    }

    pub fn is_text(&self) -> bool {
        self.media_type().starts_with("text/")
    }

    /// Is this an encapsulated message (`message/rfc822`, or RFC 6532's `message/global`),
    /// whether or not its body could be read as one?
    pub fn is_message(&self) -> bool {
        matches!(self.media_type().as_str(), "message/rfc822" | "message/global")
    }

    /// The first value of a header field of this entity, `name` in lower case.
    pub fn header(&self, name: &str) -> Option<String> {
        field(&header_fields(self.head), name)
    }

    /// The name of the file the part carries, from Content-Type's `name` or
    /// Content-Disposition's `filename`, encoded-words decoded.
    pub fn filename(&self) -> Option<String> {
        param(&self.ctype, "name")
            .or_else(|| param(&self.disposition, "filename"))
            .map(|n| decode_words(n.trim()))
            .filter(|n| !n.is_empty())
    }

    /// Is the part disposed as an `attachment`, or named as a file with a disposition other
    /// than `inline`?
    fn is_disposed_attachment(&self) -> bool {
        let d = self.disposition.trim_start().to_ascii_lowercase();
        d.starts_with("attachment") || (!d.is_empty() && !d.starts_with("inline") && self.filename().is_some())
    }

    /// Is this what a reader means by an attachment: a part with a file name, one disposed
    /// as `attachment`, or an encapsulated message?
    pub fn is_attachment(&self) -> bool {
        self.filename().is_some()
            || self.is_message()
            || self.disposition.trim_start().to_ascii_lowercase().starts_with("attachment")
    }

    /// The name a reader is shown for this part as an attachment: its file name, else for an
    /// encapsulated message its Subject, else `forwarded message` or `attachment`.
    pub fn attachment_name(&self) -> String {
        if let Some(n) = self.filename() { return n; }
        if !self.is_message() { return String::from("attachment"); }
        let subject = match &self.structure {
            Structure::Message(inner)   => inner.header("subject"),
            // Encoded against RFC 2046 §5.2.1, so read once decoded.
            _ => {
                let bytes = match self.encoding.as_str() {
                    "base64"            => decode_base64_lenient(self.body),
                    "quoted-printable"  => decode_qp(self.body),
                    _                   => self.body.to_vec(),
                };
                field(&headers(&bytes), "subject")
            }
        };
        subject.map(|s| decode_words(s.trim()))
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| String::from("forwarded message"))
    }

    /// The body out of its transfer encoding and its charset, as UTF-8.
    pub fn text(&self) -> String {
        decode_body(self.body, &self.encoding, param(&self.ctype, "charset").as_deref())
    }

    /// The `text/plain` and `text/html` leaves a reader is shown as this entity's body, in
    /// order, by RFC 2046: of a `multipart/alternative` the plain version among its own parts,
    /// else the last that shows anything; of a `multipart/related` its root (RFC 2387); of any
    /// other multipart each part in turn. An attachment, and anything inside an encapsulated
    /// message, is never shown, nor is a leaf with no text.
    pub fn display_parts(&self) -> Vec<&Part<'a>> {
        let mut out = Vec::new();
        self.display_into(&mut out);
        out
    }

    fn display_into<'s>(&'s self, out: &mut Vec<&'s Part<'a>>) {
        match &self.structure {
            Structure::Leaf => {
                let media = self.media_type();
                // A multipart left a leaf, with no boundary or too deep, is shown as the text
                // it is rather than not at all.
                let shown = media == "text/plain" || media == "text/html" || media.starts_with("multipart/");
                // A file name alone does not make a text part an attachment to the reader:
                // with no disposition it is shown inline, as other readers do (RFC 2183 §2).
                if shown
                    && !self.is_disposed_attachment()
                    && !self.text().trim().is_empty()
                {
                    out.push(self);
                }
            }
            Structure::Message(_) => (),
            Structure::Multipart(kids) => match self.media_type().as_str() {
                "multipart/alternative" => {
                    let plain = kids.iter()
                        .filter(|k| matches!(k.structure, Structure::Leaf) && k.media_type() == "text/plain")
                        .map(|k| k.display_parts())
                        .find(|v| !v.is_empty());
                    let chosen = plain.or_else(|| kids.iter().rev()
                        .map(|k| k.display_parts())
                        .find(|v| !v.is_empty()));
                    if let Some(v) = chosen { out.extend(v); }
                }
                "multipart/related" => {
                    let start = param(&self.ctype, "start");
                    let root = start.and_then(|s| kids.iter().find(|k|
                            k.header("content-id").map(|id| id.trim() == s.trim()).unwrap_or(false)))
                        .or_else(|| kids.first());
                    if let Some(r) = root { r.display_into(out); }
                }
                _ => for k in kids { k.display_into(out); },
            },
        }
    }
}

/// The MIME structure of `raw`, a whole message: each multipart split by its boundary and each
/// encapsulated message read as a message, to a depth of sixteen, below which an entity is a
/// leaf. The preamble and epilogue of a multipart are not parts.
pub fn entity(raw: &[u8]) -> Part<'_> {
    read_entity(raw, 0)
}

fn read_entity(raw: &[u8], depth: usize) -> Part<'_> {
    let (head, body) = split_entity(raw);
    let hs = header_fields(head);
    let ctype = match field(&hs, "content-type") {
        Some(c) if !c.trim().is_empty() => c,
        _                               => String::from("text/plain"),
    };
    let encoding = field(&hs, "content-transfer-encoding").unwrap_or_default().trim().to_ascii_lowercase();
    let media = media_type(&ctype);
    let mut structure = Structure::Leaf;
    if depth < MAX_DEPTH {
        if media.starts_with("multipart/") {
            if let Some(b) = param(&ctype, "boundary") {
                structure = Structure::Multipart(multipart_bodies(body, b.as_bytes()).into_iter()
                    .map(|p| read_entity(p, depth + 1))
                    .collect());
            }
        } else if (media == "message/rfc822" || media == "message/global")
            // An encoded message (RFC 2046 §5.2.1 forbids it) stays a leaf: its bytes would
            // have to be decoded into a buffer the parts could not borrow from.
            && matches!(encoding.as_str(), "" | "7bit" | "8bit" | "binary")
        {
            structure = Structure::Message(Box::new(read_entity(body, depth + 1)));
        }
    }
    Part {
        ctype,
        encoding,
        disposition:    field(&hs, "content-disposition").unwrap_or_default(),
        head,
        body,
        structure,
    }
}

/// Every leaf of `raw`, a whole message, in the order it holds them, inside encapsulated
/// messages too, as SEARCH reads them. A message that is not multipart is its own one leaf.
pub fn leaf_parts(raw: &[u8]) -> Vec<Part<'_>> {
    let mut out = Vec::new();
    flatten(entity(raw), &mut out);
    out
}

fn flatten<'a>(mut p: Part<'a>, out: &mut Vec<Part<'a>>) {
    match std::mem::replace(&mut p.structure, Structure::Leaf) {
        Structure::Leaf             => out.push(p),
        Structure::Multipart(kids)  => for k in kids { flatten(k, out); },
        Structure::Message(inner)   => flatten(*inner, out),
    }
}

/// The bodies of a multipart's parts, each without the line break that belongs to the
/// delimiter after it (RFC 2046 §5.1.1). A multipart cut short before its closing delimiter
/// keeps what it has.
fn multipart_bodies<'a>(body: &'a [u8], boundary: &[u8]) -> Vec<&'a [u8]> {
    let mut parts = Vec::new();
    let mut start: Option<usize> = None;
    let mut pos = 0;
    for line in body.split_inclusive(|b| *b == b'\n') {
        let rest = line.strip_prefix(b"--").and_then(|r| r.strip_prefix(boundary));
        if let Some(rest) = rest {
            let closing = rest.starts_with(b"--");
            // Transport padding may follow the delimiter; anything else makes it a line of
            // the body that only begins like one.
            if closing || rest.iter().all(|b| b.is_ascii_whitespace()) {
                if let Some(s) = start {
                    let content = &body[s..pos];
                    let content = match content.strip_suffix(b"\n") {
                        Some(c) => c.strip_suffix(b"\r").unwrap_or(c),
                        None    => content,
                    };
                    parts.push(content);
                }
                if closing { return parts; }
                start = Some(pos + line.len());
            }
        }
        pos += line.len();
    }
    if let Some(s) = start {
        parts.push(&body[s..]);
    }
    parts
}

/// An entity's header block and body, split at the first blank line. One that opens with a
/// blank line has no headers, and one with no blank line is all header.
pub fn split_entity(raw: &[u8]) -> (&[u8], &[u8]) {
    if let Some(rest) = raw.strip_prefix(b"\r\n") { return (&[], rest); }
    if let Some(rest) = raw.strip_prefix(b"\n") { return (&[], rest); }
    let mut i = 0;
    while i + 1 < raw.len() {
        if raw[i] == b'\n' {
            if raw[i + 1] == b'\n' {
                return (&raw[..i], &raw[i + 2..]);
            }
            if raw[i + 1] == b'\r' && raw.get(i + 2) == Some(&b'\n') {
                return (&raw[..i], &raw[i + 3..]);
            }
        }
        i += 1;
    }
    (raw, &[])
}

/// The header fields of a message or part, unfolded, as `(lower-case name, value)` in order.
///
/// The name is lower-cased because a field is looked up by a caller who cannot know whether
/// the sender wrote `Message-ID` or `Message-Id`; the value keeps its case. Raw 8-bit bytes
/// in a value are read as UTF-8 (RFC 6532).
pub fn headers(raw: &[u8]) -> Vec<(String, String)> {
    header_fields(split_entity(raw).0)
}

fn header_fields(block: &[u8]) -> Vec<(String, String)> {
    let text = String::from_utf8_lossy(block);
    let mut out: Vec<(String, String)> = Vec::new();
    for line in text.split('\n') {
        let line = line.strip_suffix('\r').unwrap_or(line);
        // A line that begins with a space or tab continues the field above it.
        if line.starts_with(' ') || line.starts_with('\t') {
            if let Some(last) = out.last_mut() {
                last.1.push(' ');
                last.1.push_str(line.trim());
            }
            continue;
        }
        if let Some((n, v)) = line.split_once(':') {
            let name = n.trim().to_ascii_lowercase();
            if !name.is_empty() {
                out.push((name, v.trim().to_string()));
            }
        }
    }
    out
}

/// The first value of the field `name` (lower case) in `hs`.
pub fn field(hs: &[(String, String)], name: &str) -> Option<String> {
    hs.iter().find(|(n, _)| n == name).map(|(_, v)| v.clone())
}

fn media_type(ctype: &str) -> String {
    ctype.split(';').next().unwrap_or("").trim().to_ascii_lowercase()
}

/// The value of the parameter `name` of a header field, matched in any case: a token, or a
/// quoted string with its backslash escapes (RFC 2045 §5.1), or an RFC 2231 value, extended
/// (`name*=charset'lang'%xx`) or continued (`name*0`, `name*1*`, ...), which is preferred to a
/// plain one of the same name.
pub fn param(header: &str, name: &str) -> Option<String> {
    let ps = params(header);
    let name = name.to_ascii_lowercase();
    let ext = format!("{}*", name);
    // RFC 2231 sections: (index, extended, value).
    let mut secs: Vec<(usize, bool, &str)> = Vec::new();
    for (k, v) in &ps {
        if *k == ext {
            secs.push((0, true, v));
        } else if let Some(rest) = k.strip_prefix(&ext) {
            let (n, extended) = match rest.strip_suffix('*') {
                Some(n) => (n, true),
                None    => (rest, false),
            };
            if let Ok(i) = n.parse::<usize>() {
                secs.push((i, extended, v));
            }
        }
    }
    let v = if secs.is_empty() {
        ps.into_iter().find(|(k, _)| *k == name).map(|(_, v)| v)
    } else {
        secs.sort_by_key(|s| s.0);
        let mut bytes = Vec::new();
        let mut charset: Option<&str> = None;
        for (i, (_, extended, v)) in secs.iter().enumerate() {
            if !*extended {
                bytes.extend_from_slice(v.as_bytes());
                continue;
            }
            let mut v: &str = v;
            // The first section names the charset and language: `utf-8'en'`.
            if i == 0 {
                let mut it = v.splitn(3, '\'');
                if let (Some(cs), Some(_), Some(rest)) = (it.next(), it.next(), it.next()) {
                    if !cs.is_empty() { charset = Some(cs); }
                    v = rest;
                }
            }
            bytes.extend(percent_decode(v));
        }
        Some(to_utf8(&bytes, charset))
    };
    v.filter(|v| !v.is_empty())
}

/// The parameters of a header field, after its first `;`, as `(lower-case name, value)` in
/// order, a quoted value unquoted. A quote left open runs to the end.
pub fn params(header: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut chars = header.chars().peekable();
    // The field's own value, before the first `;`.
    while let Some(c) = chars.next() {
        if c == ';' { break; }
    }
    loop {
        let mut name = String::new();
        let mut has_value = false;
        while let Some(c) = chars.next() {
            match c {
                ';' => break,
                '=' => { has_value = true; break; }
                _   => name.push(c),
            }
        }
        let name = name.trim().to_ascii_lowercase();
        if !has_value {
            if chars.peek().is_none() { return out; }
            continue;
        }
        while chars.peek().map(|c| c.is_whitespace()).unwrap_or(false) { chars.next(); }
        let mut value = String::new();
        if chars.peek() == Some(&'"') {
            chars.next();
            while let Some(c) = chars.next() {
                match c {
                    '"'     => break,
                    '\\'    => if let Some(q) = chars.next() { value.push(q); },
                    _       => value.push(c),
                }
            }
            // Anything after the closing quote, up to the next `;`, is not part of it.
            while let Some(c) = chars.next() {
                if c == ';' { break; }
            }
        } else {
            while let Some(c) = chars.next() {
                if c == ';' { break; }
                value.push(c);
            }
            value = value.trim().to_string();
        }
        if !name.is_empty() {
            out.push((name, value));
        }
        if chars.peek().is_none() { return out; }
    }
}

/// `%xx` escapes to bytes; a `%` not followed by two hex digits stands for itself.
fn percent_decode(s: &str) -> Vec<u8> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Some(x) = hex2(b[i + 1], b[i + 2]) {
                out.push(x);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    out
}

/// A body out of its Content-Transfer-Encoding and its charset, as UTF-8.
pub fn decode_body(body: &[u8], encoding: &str, charset: Option<&str>) -> String {
    let bytes = match encoding.trim().to_ascii_lowercase().as_str() {
        "base64"            => decode_base64_lenient(body),
        "quoted-printable"  => decode_qp(body),
        _                   => body.to_vec(),
    };
    to_utf8(&bytes, charset)
}

/// Bytes in `charset` as UTF-8. ISO-8859-1, ISO-8859-15 and Windows-1252 are transcoded,
/// unless the bytes are already valid UTF-8; any other charset, or none, is read as UTF-8 with
/// invalid sequences replaced.
pub fn to_utf8(bytes: &[u8], charset: Option<&str>) -> String {
    let cs = charset.map(|c| c.trim().trim_matches('"').to_ascii_lowercase()).unwrap_or_default();
    // A body labelled with a single-byte charset that is valid UTF-8 with non-ASCII in it was
    // written in UTF-8 and mislabelled: real Latin-1 text almost never forms valid UTF-8
    // sequences (`Ã©` and the like), so reading it by its label would only make mojibake.
    if !bytes.is_ascii() {
        if let Ok(s) = std::str::from_utf8(bytes) {
            return s.to_string();
        }
    }
    match cs.as_str() {
        "iso-8859-1" | "iso8859-1" | "iso_8859-1" | "latin1" | "latin-1" | "l1"
            => bytes.iter().map(|b| *b as char).collect(),
        "iso-8859-15" | "iso8859-15" | "iso_8859-15" | "latin-9" | "latin9"
            => bytes.iter().map(|b| latin9(*b)).collect(),
        "windows-1252" | "cp1252" | "x-cp1252"
            => bytes.iter().map(|b| cp1252(*b)).collect(),
        _   => String::from_utf8_lossy(bytes).into_owned(),
    }
}

/// ISO-8859-15: Latin-1 with eight places changed.
fn latin9(b: u8) -> char {
    match b {
        0xA4 => '\u{20AC}',
        0xA6 => '\u{0160}',
        0xA8 => '\u{0161}',
        0xB4 => '\u{017D}',
        0xB8 => '\u{017E}',
        0xBC => '\u{0152}',
        0xBD => '\u{0153}',
        0xBE => '\u{0178}',
        _    => b as char,
    }
}

/// Windows-1252: Latin-1 with 0x80 to 0x9F given printable characters; the five it leaves
/// undefined stand as the C1 controls, as browsers read them.
fn cp1252(b: u8) -> char {
    const HIGH: [char; 32] = [
        '\u{20AC}', '\u{0081}', '\u{201A}', '\u{0192}', '\u{201E}', '\u{2026}', '\u{2020}', '\u{2021}',
        '\u{02C6}', '\u{2030}', '\u{0160}', '\u{2039}', '\u{0152}', '\u{008D}', '\u{017D}', '\u{008F}',
        '\u{0090}', '\u{2018}', '\u{2019}', '\u{201C}', '\u{201D}', '\u{2022}', '\u{2013}', '\u{2014}',
        '\u{02DC}', '\u{2122}', '\u{0161}', '\u{203A}', '\u{0153}', '\u{009D}', '\u{017E}', '\u{0178}',
    ];
    match b {
        0x80..=0x9F => HIGH[(b - 0x80) as usize],
        _           => b as char,
    }
}
