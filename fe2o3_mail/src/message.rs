//! Sans-io RFC 5322 message reading and draft building.
//!
//! This module owns no socket and reads no clock.  A message is parsed from bytes a caller
//! already holds, and a draft is built into bytes the caller does something with -- so the
//! whole of it is a pure transform that is proven natively, in the tests below, rather than
//! only against a live server.  The two facts a builder would otherwise reach for the
//! environment for -- the `Date` header and the `Message-ID` -- are inputs to
//! [`DraftMessage`], for the same reason: a builder that read the clock could not be tested
//! for the bytes it emits.
//!
//! A DRAFT IS NOT A SEND.  [`DraftMessage::build`] produces the RFC 5322 document that a
//! message would be, and stops there.  Nothing here opens a connection, and there is
//! deliberately no counterpart that puts the bytes on the wire: putting a finished document
//! in front of a person to send is the whole of the contribution, and the send stays theirs.
//!
//! # Character sets
//!
//! A body or a header word in a charset other than UTF-8 is decoded as UTF-8 with the
//! invalid sequences replaced, rather than transcoded, because a transcoder is a table per
//! charset and this crate carries no such table.  The common case -- UTF-8, and the ASCII
//! subset every charset shares -- is exact; a legacy ISO-8859 body degrades to readable
//! text with the odd replacement character rather than failing.

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_text::base64;


/// A parsed message, reduced to what a reader needs: the headers that identify it and the
/// readable text of its body.
///
/// The address and subject fields are decoded for display -- RFC 2047 encoded-words are
/// turned back into the characters they stand for -- so what a reader sees is the name and
/// the subject rather than `=?utf-8?B?...?=`.  The raw header block is not kept; a caller
/// that wants a header this struct does not name can call [`headers`] on the bytes itself.
#[derive(Clone, Debug, Default)]
pub struct ParsedMessage {
    pub from:        String,    // decoded display, e.g. "Ada <ada@x.example>"
    pub to:          String,
    pub cc:          String,
    pub subject:     String,    // RFC 2047 decoded
    pub date:        String,    // the Date header, verbatim
    pub message_id:  String,
    pub in_reply_to: String,
    pub references:  String,
    pub body:        String,    // readable text of the message
    pub attachments: Vec<String>,   // filenames of the non-text parts
}

impl ParsedMessage {
    /// Read a whole message from its RFC 5322 bytes.
    ///
    /// Infallible on purpose: a mailbox holds whatever a sender put in it, and a parser that
    /// refused a malformed message would leave a reader unable to see the very message they
    /// most need to.  A field that cannot be found is empty, and a body that cannot be
    /// decoded degrades rather than failing.
    pub fn parse(raw: &[u8]) -> Self {
        let text = String::from_utf8_lossy(raw);
        let hs   = headers(&text);
        Self {
            from:        decode_words(&hget(&hs, "from")),
            to:          decode_words(&hget(&hs, "to")),
            cc:          decode_words(&hget(&hs, "cc")),
            subject:     decode_words(&hget(&hs, "subject")),
            date:        hget(&hs, "date"),
            message_id:  hget(&hs, "message-id"),
            in_reply_to: hget(&hs, "in-reply-to"),
            references:  hget(&hs, "references"),
            body:        readable_text(&text),
            attachments: attachment_names(&text),
        }
    }
}


// ── Reading ─────────────────────────────────────────────────────────────

/// Split the header block from the body at the first blank line, unfold continuation lines,
/// and return the headers as an ordered list of `(lowercased-name, value)`.
///
/// The name is lowercased because a header is looked up by a caller who did not write it and
/// cannot know whether the sender wrote `Message-ID` or `Message-Id`; the value is left as it
/// was, since its case may matter.
pub fn headers(text: &str) -> Vec<(String, String)> {
    let block = match find_blank_line(text) {
        Some(i) => &text[..i],
        None    => text,
    };
    let mut out: Vec<(String, String)> = Vec::new();
    for line in block.split('\n') {
        let line = line.strip_suffix('\r').unwrap_or(line);
        // A line that begins with a space or tab continues the header above it.
        if line.starts_with(' ') || line.starts_with('\t') {
            if let Some(last) = out.last_mut() {
                last.1.push(' ');
                last.1.push_str(line.trim());
                continue;
            }
        }
        if let Some(i) = line.find(':') {
            let name = line[..i].trim().to_lowercase();
            let val  = line[i + 1..].trim().to_string();
            if !name.is_empty() {
                out.push((name, val));
            }
        }
    }
    out
}

/// The first value of a header, or the empty string when it is absent.
fn hget(hs: &[(String, String)], name: &str) -> String {
    hs.iter()
        .find(|(n, _)| n == name)
        .map(|(_, v)| v.clone())
        .unwrap_or_default()
}

/// The byte offset of the blank line that ends the header block, `\r\n\r\n` or `\n\n`.
fn find_blank_line(text: &str) -> Option<usize> {
    let b = text.as_bytes();
    let mut i = 0;
    while i + 1 < b.len() {
        if b[i] == b'\n' && b[i + 1] == b'\n' {
            return Some(i);
        }
        if i + 3 < b.len() && b[i] == b'\r' && b[i + 1] == b'\n'
            && b[i + 2] == b'\r' && b[i + 3] == b'\n'
        {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// The body, which is everything after the blank line that ends the headers.
fn body_of(text: &str) -> &str {
    match find_blank_line(text) {
        Some(i) => {
            // Step over the separator itself, whichever spelling it was.
            let b = text.as_bytes();
            if i + 3 < b.len() && b[i] == b'\r' {
                &text[i + 4..]
            } else {
                &text[i + 2..]
            }
        },
        None => "",
    }
}

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
    let q1 = rest.find('?')?;
    let charset = &rest[..q1];
    let after_cs = &rest[q1 + 1..];
    let q2 = after_cs.find('?')?;
    let enc = &after_cs[..q2];
    let after_enc = &after_cs[q2 + 1..];
    let q3 = after_enc.find("?=")?;
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

/// The readable text of a message: the `text/plain` part of a multipart, or the decoded
/// body, or `text/html` reduced to text where that is all there is.
///
/// One level of multipart nesting is followed, which is the shape of a message with an
/// attachment -- a `multipart/mixed` whose first part is the `multipart/alternative` that
/// holds the text.
pub fn readable_text(raw: &str) -> String {
    let hs    = headers(raw);
    let ctype = {
        let c = hget(&hs, "content-type");
        if c.is_empty() { fmt!("text/plain") } else { c }
    };
    let body  = body_of(raw);

    if ctype.to_lowercase().contains("multipart") {
        if let Some(boundary) = param(&ctype, "boundary") {
            let marker = fmt!("--{}", boundary);
            let mut plain: Option<String> = None;
            let mut html:  Option<String> = None;
            for part in body.split(marker.as_str()) {
                let part = part.strip_prefix('\r').unwrap_or(part);
                let part = part.strip_prefix('\n').unwrap_or(part);
                let phs  = headers(part);
                let pct  = hget(&phs, "content-type");
                let pcte = hget(&phs, "content-transfer-encoding").to_lowercase();
                let pbody = body_of(part);
                if pbody.trim().is_empty() {
                    continue;
                }
                let pctl = pct.to_lowercase();
                if pctl.contains("multipart") && plain.is_none() {
                    // A nested multipart: read one level down and take its text.
                    let inner = readable_text(&fmt!("Content-Type: {}\r\n\r\n{}", pct, pbody));
                    if !inner.trim().is_empty() {
                        plain = Some(inner);
                    }
                    continue;
                }
                let decoded = decode_transfer(pbody, &pcte, param(&pct, "charset").as_deref());
                if pctl.contains("text/plain") && plain.is_none() {
                    plain = Some(decoded);
                } else if pctl.contains("text/html") && html.is_none() {
                    html = Some(decoded);
                }
            }
            if let Some(p) = plain {
                return norm_lines(p.trim());
            }
            if let Some(h) = html {
                return norm_lines(strip_html(&h).trim());
            }
            return String::new();
        }
    }

    let cte = hget(&hs, "content-transfer-encoding").to_lowercase();
    let decoded = decode_transfer(body, &cte, param(&ctype, "charset").as_deref());
    if ctype.to_lowercase().contains("text/html") {
        return norm_lines(strip_html(&decoded).trim());
    }
    norm_lines(decoded.trim())
}

/// Line endings a reader wants: a message's `\r\n` and bare `\r` reduced to `\n`, so the
/// text a model is handed is the text and not the wire's carriage returns.
fn norm_lines(s: &str) -> String {
    s.replace("\r\n", "\n").replace('\r', "\n")
}

/// Decode one part's body out of its transfer encoding, then read the bytes as text.
fn decode_transfer(body: &str, cte: &str, charset: Option<&str>) -> String {
    let bytes = match cte.trim() {
        "base64"           => decode_base64_lenient(body),
        "quoted-printable" => decode_qp(body),
        _                  => body.as_bytes().to_vec(),
    };
    // Every supported charset shares ASCII, and UTF-8 is the one this crate decodes exactly;
    // anything else degrades (see the module header).  `charset` is accepted so a future
    // transcoder has the name to hand.
    let _ = charset;
    String::from_utf8_lossy(&bytes).into_owned()
}

/// The filenames of the parts that are not the readable text -- what a reader means by "the
/// attachments".  A part with a `filename` or a `name` parameter, or a `Content-Disposition`
/// of `attachment`, counts.
fn attachment_names(raw: &str) -> Vec<String> {
    let hs = headers(raw);
    let ctype = hget(&hs, "content-type");
    let mut out: Vec<String> = Vec::new();
    if let Some(boundary) = param(&ctype, "boundary") {
        let marker = fmt!("--{}", boundary);
        for part in body_of(raw).split(marker.as_str()) {
            let phs  = headers(part);
            let disp = hget(&phs, "content-disposition");
            let pct  = hget(&phs, "content-type");
            let name = param(&pct, "name")
                .or_else(|| param(&disp, "filename"));
            if let Some(n) = name {
                if !n.trim().is_empty() {
                    out.push(decode_words(n.trim()));
                }
            } else if disp.to_lowercase().contains("attachment") {
                out.push(fmt!("attachment"));
            }
        }
    }
    out
}

/// The value of a `name="value"` parameter on a header, unquoted.
fn param(header: &str, name: &str) -> Option<String> {
    let lower = header.to_lowercase();
    let key   = fmt!("{}=", name.to_lowercase());
    let at    = lower.find(&key)?;
    let after = &header[at + key.len()..];
    let after = after.trim_start();
    if let Some(stripped) = after.strip_prefix('"') {
        let end = stripped.find('"')?;
        Some(stripped[..end].to_string())
    } else {
        let end = after.find(|c: char| c == ';' || c == '\r' || c == '\n' || c == ' ')
            .unwrap_or(after.len());
        let v = after[..end].trim();
        if v.is_empty() { None } else { Some(v.to_string()) }
    }
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
fn decode_base64_lenient(s: &str) -> Vec<u8> {
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
    let hi = (a as char).to_digit(16)?;
    let lo = (b as char).to_digit(16)?;
    Some((hi * 16 + lo) as u8)
}

/// Reduce HTML to its readable text.  A mail body is the least trustworthy string a reader
/// meets, so nothing here is ever treated as markup to render -- the tags are removed and
/// only the text between them is kept.
pub fn strip_html(html: &str) -> String {
    let s0 = remove_blocks(html, "<style", "</style>");
    let s  = remove_blocks(&s0, "<script", "</script>");

    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < s.len() {
        if s.as_bytes()[i] == b'<' {
            // A tag: skip to the matching '>'.  A block-level close or a break becomes a
            // newline so the text does not run together.
            let tag_end = s[i..].find('>').map(|j| i + j + 1).unwrap_or(s.len());
            let tag = s[i..tag_end].to_lowercase();
            if is_break_tag(&tag) {
                out.push('\n');
            }
            i = tag_end;
        } else {
            let ch = s[i..].chars().next().unwrap_or('\u{fffd}');
            out.push(ch);
            i += ch.len_utf8();
        }
    }
    collapse_blank_lines(&decode_entities(&out))
}

/// Does this tag end a block, so its removal should leave a line break behind?
fn is_break_tag(tag: &str) -> bool {
    const BREAKERS: [&str; 12] = [
        "</p", "</div", "</tr", "</li", "</h1", "</h2", "</h3", "</h4", "</h5", "</h6",
        "<br", "</table",
    ];
    BREAKERS.iter().any(|b| tag.starts_with(b))
}

/// Remove every `<open ... close>` block, tag and content, case-insensitively.
fn remove_blocks(s: &str, open: &str, close: &str) -> String {
    let lower = s.to_lowercase();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < s.len() {
        if lower[i..].starts_with(open) {
            if let Some(rel) = lower[i..].find(close) {
                i += rel + close.len();
                continue;
            }
            // An unterminated block: drop the rest.
            break;
        }
        let ch = s[i..].chars().next().unwrap_or('\u{fffd}');
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

/// Decode the handful of HTML entities that carry meaning in plain text.
fn decode_entities(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&nbsp;", " ")
}

/// Collapse three or more newlines to two, so the text keeps its paragraphs without the
/// gaps a stripped layout leaves.
fn collapse_blank_lines(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut runs = 0;
    for ch in s.chars() {
        if ch == '\n' {
            runs += 1;
            if runs <= 2 {
                out.push('\n');
            }
        } else {
            runs = 0;
            out.push(ch);
        }
    }
    out
}


// ── Building a draft ─────────────────────────────────────────────────────

/// A message being composed, before it is turned into the bytes a person would send.
///
/// The `date` and `message_id` are fields rather than something the builder works out,
/// because a builder that read the clock or drew a random id could not be tested for the
/// bytes it emits (see the module header).  A caller in a browser fills them from the page's
/// own clock and randomness; a test fills them with fixed strings.
#[derive(Clone, Debug, Default)]
pub struct DraftMessage {
    pub from:        String,        // the bare address the message is from
    pub from_name:   String,        // the display name, or empty
    pub to:          Vec<String>,   // each "addr" or "Name <addr>"
    pub cc:          Vec<String>,
    pub subject:     String,
    pub body:        String,
    pub in_reply_to: String,        // the Message-ID this replies to, or empty
    pub references:  String,        // the References header, or empty to derive from in_reply_to
    pub date:        String,        // the Date header, spelled as RFC 5322 wants it
    pub message_id:  String,        // this message's own Message-ID, angle brackets and all
}

impl DraftMessage {
    /// Build the RFC 5322 document this draft describes: a single `text/plain` part, its body
    /// quoted-printable, with the headers that make a reply thread.
    ///
    /// This does NOT send.  It hands back bytes; what becomes of them is the caller's, and in
    /// Daimond the caller writes them to a drafts folder for a person to read and send.
    pub fn build(&self) -> Outcome<Vec<u8>> {
        if self.from.trim().is_empty() {
            return Err(err!(
                "A draft has no sender address, so it cannot be built."; Invalid, Input, Missing));
        }
        if self.to.iter().all(|a| a.trim().is_empty()) {
            return Err(err!(
                "A draft has no recipient in its To list, so it cannot be built.";
                Invalid, Input, Missing));
        }
        if self.date.trim().is_empty() {
            return Err(err!(
                "A draft was built with no Date header; the caller must supply one.";
                Invalid, Input, Missing));
        }
        if self.message_id.trim().is_empty() {
            return Err(err!(
                "A draft was built with no Message-ID; the caller must supply one.";
                Invalid, Input, Missing));
        }

        let mut h: Vec<String> = Vec::new();
        h.push(fmt!("Message-ID: {}", self.message_id.trim()));
        h.push(fmt!("Date: {}", self.date.trim()));
        h.push(fmt!("From: {}", encode_addr(&self.from_name, &self.from)));
        h.push(fmt!("To: {}", join_addrs(&self.to)));
        if !self.cc.iter().all(|a| a.trim().is_empty()) {
            h.push(fmt!("Cc: {}", join_addrs(&self.cc)));
        }
        h.push(fmt!("Subject: {}", encode_word(&self.subject)));
        if !self.in_reply_to.trim().is_empty() {
            h.push(fmt!("In-Reply-To: {}", self.in_reply_to.trim()));
            let refs = if self.references.trim().is_empty() {
                self.in_reply_to.trim()
            } else {
                self.references.trim()
            };
            h.push(fmt!("References: {}", refs));
        }
        h.push(fmt!("MIME-Version: 1.0"));
        h.push(fmt!("User-Agent: Daimond"));
        h.push(fmt!("Content-Type: text/plain; charset=utf-8"));
        h.push(fmt!("Content-Transfer-Encoding: quoted-printable"));

        let doc = fmt!("{}\r\n\r\n{}\r\n", h.join("\r\n"), encode_qp(&self.body));
        Ok(doc.into_bytes())
    }
}

/// Join a list of addresses for a `To` or `Cc` header, each encoded and comma-separated.
fn join_addrs(list: &[String]) -> String {
    list.iter()
        .map(|a| a.trim())
        .filter(|a| !a.is_empty())
        .map(|a| {
            let (name, addr) = split_addr(a);
            encode_addr(&name, &addr)
        })
        .collect::<Vec<String>>()
        .join(", ")
}

/// Split `Name <addr>` into its two parts; a bare address gives an empty name.
fn split_addr(s: &str) -> (String, String) {
    if let Some(open) = s.rfind('<') {
        if let Some(close) = s[open..].find('>') {
            let addr = s[open + 1..open + close].trim().to_string();
            let name = s[..open].trim().trim_matches('"').trim().to_string();
            return (name, addr);
        }
    }
    (String::new(), s.trim().to_string())
}

/// One address as a header writes it: `Name <addr>`, the name encoded if it is not ASCII and
/// quoted if it holds a character that would otherwise punctuate the header.
fn encode_addr(name: &str, addr: &str) -> String {
    let addr = addr.trim();
    let name = name.trim();
    if addr.is_empty() {
        return String::new();
    }
    if name.is_empty() {
        return addr.to_string();
    }
    let shown = if is_ascii(name) {
        if name.contains(|c| "(),:;<>@[]\".".contains(c)) {
            fmt!("\"{}\"", name.replace('\\', "\\\\").replace('"', "\\\""))
        } else {
            name.to_string()
        }
    } else {
        encode_word(name)
    };
    fmt!("{} <{}>", shown, addr)
}

/// Whether every character is printable ASCII.
fn is_ascii(s: &str) -> bool {
    s.bytes().all(|b| (0x20..=0x7e).contains(&b))
}

/// A header value with anything but plain ASCII in it, as one or more RFC 2047 base64
/// encoded-words.  The words are cut at a character boundary, never inside a multi-byte one,
/// so no recipient decodes a half character.
pub fn encode_word(s: &str) -> String {
    if is_ascii(s) {
        return s.to_string();
    }
    let mut words: Vec<String> = Vec::new();
    let mut chunk = String::new();
    let mut bytes = 0usize;
    for ch in s.chars() {
        let n = ch.len_utf8();
        // 39 source bytes keeps one encoded-word's base64 under the 76-character line limit.
        if bytes + n > 39 && !chunk.is_empty() {
            words.push(fmt!("=?utf-8?B?{}?=", base64::encode(chunk.as_bytes())));
            chunk.clear();
            bytes = 0;
        }
        chunk.push(ch);
        bytes += n;
    }
    if !chunk.is_empty() {
        words.push(fmt!("=?utf-8?B?{}?=", base64::encode(chunk.as_bytes())));
    }
    words.join("\r\n ")
}

/// Quoted-printable over a body's UTF-8 bytes.
///
/// The three rules that bite: a space or tab at the end of a line is invisible and would be
/// stripped in transit, so it is encoded; a line is folded with a soft break before it
/// reaches the 76-character limit; and a line that would begin `From ` is escaped, because
/// some software still reads one as the start of a new message.
pub fn encode_qp(text: &str) -> String {
    let norm = text.replace("\r\n", "\n").replace('\r', "\n");
    let bytes = norm.as_bytes();
    let mut lines: Vec<String> = Vec::new();
    let mut line  = String::new();
    let mut held: Option<u8> = None;    // a pending space or tab, held in case a newline follows

    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b == 0x0a {
            if let Some(h) = held.take() {
                qp_push(&mut lines, &mut line, if h == 0x20 { "=20" } else { "=09" });
            }
            lines.push(std::mem::take(&mut line));
            i += 1;
            continue;
        }
        if let Some(h) = held.take() {
            qp_push(&mut lines, &mut line, &(h as char).to_string());
        }
        if b == 0x20 { held = Some(0x20); i += 1; continue; }
        if b == 0x09 { held = Some(0x09); i += 1; continue; }
        if (33..=126).contains(&b) && b != 61 {
            qp_push(&mut lines, &mut line, &(b as char).to_string());
        } else {
            qp_push(&mut lines, &mut line, &fmt!("={:02X}", b));
        }
        // A line may not begin "From ".
        if line == "From" && i + 1 < bytes.len() && bytes[i + 1] == 0x20 {
            line = fmt!("=46rom");
        }
        i += 1;
    }
    if let Some(h) = held.take() {
        qp_push(&mut lines, &mut line, if h == 0x20 { "=20" } else { "=09" });
    }
    lines.push(line);
    lines.join("\r\n")
}

/// Append one token to the current quoted-printable line, folding with a soft break first
/// when it would otherwise pass the 76-character limit.
fn qp_push(lines: &mut Vec<String>, line: &mut String, tok: &str) {
    if line.len() + tok.len() > 75 {
        line.push('=');
        lines.push(std::mem::take(line));
    }
    line.push_str(tok);
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_simple_message() {
        let raw = "From: Ada <ada@x.example>\r\n\
                   To: Bob <bob@y.example>\r\n\
                   Subject: Hello there\r\n\
                   Date: Mon, 1 Sep 2026 10:00:00 +0000\r\n\
                   Message-ID: <abc@x.example>\r\n\
                   \r\n\
                   This is the body.\r\n";
        let m = ParsedMessage::parse(raw.as_bytes());
        assert_eq!(m.from, "Ada <ada@x.example>");
        assert_eq!(m.subject, "Hello there");
        assert_eq!(m.message_id, "<abc@x.example>");
        assert_eq!(m.body, "This is the body.");
    }

    #[test]
    fn unfolds_a_continued_header() {
        let raw = "Subject: one two\r\n three four\r\nFrom: a@b\r\n\r\nbody\r\n";
        let hs = headers(raw);
        assert_eq!(hget(&hs, "subject"), "one two three four");
    }

    #[test]
    fn decodes_a_2047_subject() {
        // "Grüße" base64 in UTF-8.
        let enc = base64::encode("Grüße".as_bytes());
        let s = fmt!("=?utf-8?B?{}?=", enc);
        assert_eq!(decode_words(&s), "Grüße");
    }

    #[test]
    fn decodes_a_q_word() {
        // "_" is a space in Q, so "=5F" is a literal underscore.
        let s = "=?utf-8?Q?a=5Fb?=";
        assert_eq!(decode_words(s), "a_b");
    }

    #[test]
    fn joins_two_adjacent_words() {
        let a = base64::encode("Ma".as_bytes());
        let b = base64::encode("ría".as_bytes());
        let s = fmt!("=?utf-8?B?{}?= =?utf-8?B?{}?=", a, b);
        assert_eq!(decode_words(&s), "María");
    }

    #[test]
    fn reads_the_plain_part_of_a_multipart() {
        let raw = "Content-Type: multipart/alternative; boundary=\"BB\"\r\n\r\n\
                   --BB\r\n\
                   Content-Type: text/plain; charset=utf-8\r\n\r\n\
                   the plain text\r\n\
                   --BB\r\n\
                   Content-Type: text/html; charset=utf-8\r\n\r\n\
                   <p>the html</p>\r\n\
                   --BB--\r\n";
        assert_eq!(readable_text(raw), "the plain text");
    }

    #[test]
    fn falls_back_to_html_when_no_plain_part() {
        let raw = "Content-Type: multipart/alternative; boundary=\"BB\"\r\n\r\n\
                   --BB\r\n\
                   Content-Type: text/html; charset=utf-8\r\n\r\n\
                   <p>first</p><p>second</p>\r\n\
                   --BB--\r\n";
        assert_eq!(readable_text(raw), "first\nsecond");
    }

    #[test]
    fn decodes_quoted_printable_body() {
        let raw = "Content-Type: text/plain; charset=utf-8\r\n\
                   Content-Transfer-Encoding: quoted-printable\r\n\r\n\
                   caf=C3=A9\r\n";
        assert_eq!(readable_text(raw), "café");
    }

    #[test]
    fn names_an_attachment() {
        let raw = "Content-Type: multipart/mixed; boundary=\"BB\"\r\n\r\n\
                   --BB\r\n\
                   Content-Type: text/plain\r\n\r\n\
                   see attached\r\n\
                   --BB\r\n\
                   Content-Type: application/pdf; name=\"report.pdf\"\r\n\
                   Content-Disposition: attachment; filename=\"report.pdf\"\r\n\r\n\
                   JVBERi0=\r\n\
                   --BB--\r\n";
        let m = ParsedMessage::parse(raw.as_bytes());
        assert_eq!(m.attachments, vec!["report.pdf".to_string()]);
        assert_eq!(m.body, "see attached");
    }

    #[test]
    fn builds_a_draft_and_reads_it_back() {
        let d = DraftMessage {
            from:        fmt!("me@x.example"),
            from_name:   fmt!("Me Myself"),
            to:          vec![fmt!("You <you@y.example>")],
            cc:          Vec::new(),
            subject:     fmt!("Re: café"),
            body:        fmt!("Hello,\nThis is a draft.\n"),
            in_reply_to: fmt!("<orig@y.example>"),
            references:  String::new(),
            date:        fmt!("Mon, 1 Sep 2026 12:00:00 +0000"),
            message_id:  fmt!("<new@x.example>"),
        };
        let bytes = d.build().expect("the draft builds");
        let m = ParsedMessage::parse(&bytes);
        assert_eq!(m.from, "Me Myself <me@x.example>");
        assert_eq!(m.to, "You <you@y.example>");
        assert_eq!(m.subject, "Re: café");
        assert_eq!(m.in_reply_to, "<orig@y.example>");
        assert_eq!(m.references, "<orig@y.example>");
        assert_eq!(m.body, "Hello,\nThis is a draft.");
    }

    #[test]
    fn a_draft_with_no_recipient_is_refused() {
        let d = DraftMessage {
            from:       fmt!("me@x.example"),
            to:         Vec::new(),
            date:       fmt!("Mon, 1 Sep 2026 12:00:00 +0000"),
            message_id: fmt!("<new@x.example>"),
            ..Default::default()
        };
        assert!(d.build().is_err());
    }

    #[test]
    fn quoted_printable_encodes_trailing_space_and_from() {
        let out = encode_qp("From here \nnext");
        // The trailing space before the newline is encoded, and a line beginning "From " is
        // escaped at its F.
        assert!(out.starts_with("=46rom here=20"), "got {:?}", out);
    }

    #[test]
    fn encode_word_round_trips_through_decode() {
        let s = "Grüße aus München — a longer non-ASCII subject line that must be chunked";
        let encoded = encode_word(s);
        // Joining the folded words back and decoding yields the original.
        assert_eq!(decode_words(&encoded.replace("\r\n ", "")), s);
    }
}
