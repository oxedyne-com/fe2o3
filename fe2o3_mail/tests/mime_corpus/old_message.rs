//! The reader of `fe2o3_mail::message` as it stood at d81121d5 (and a5c60dfe), before it used
//! `fe2o3_text::mime`'s walker, kept as the baseline `parser_compare` holds the new one to.
//! Only its reading half is kept; the draft builder never changed.
#![allow(dead_code, unused_imports)]

//! text with the odd replacement character rather than failing.

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_text::{
    base64,
    mime::{
        decode_base64_lenient,
        decode_qp,
        decode_words,
    },
};


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

