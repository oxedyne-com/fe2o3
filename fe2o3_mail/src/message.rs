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
//! A body is read through `fe2o3_text::mime`'s part walker, the one IMAP SEARCH uses:
//! base64 and quoted-printable are decoded, and ISO-8859-1, ISO-8859-15 and Windows-1252 are
//! transcoded to UTF-8. Any other charset is read as UTF-8 with invalid sequences replaced,
//! which keeps the ASCII every charset shares.

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_text::{
    base64,
    mime::{
        decode_words,
        entity,
        Part,
        Structure,
    },
};

pub use oxedyne_fe2o3_text::mime::headers;


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
    pub attachments: Vec<String>,   // file names, an attached message by its Subject
}

impl ParsedMessage {
    /// Read a whole message from its RFC 5322 bytes.
    ///
    /// Infallible on purpose: a mailbox holds whatever a sender put in it, and a parser that
    /// refused a malformed message would leave a reader unable to see the very message they
    /// most need to.  A field that cannot be found is empty, and a body that cannot be
    /// decoded degrades rather than failing.
    pub fn parse(raw: &[u8]) -> Self {
        let hs = headers(raw);
        Self {
            from:        decode_words(&hget(&hs, "from")),
            to:          decode_words(&hget(&hs, "to")),
            cc:          decode_words(&hget(&hs, "cc")),
            subject:     decode_words(&hget(&hs, "subject")),
            date:        hget(&hs, "date"),
            message_id:  hget(&hs, "message-id"),
            in_reply_to: hget(&hs, "in-reply-to"),
            references:  hget(&hs, "references"),
            body:        readable_text(raw),
            attachments: attachment_names(raw),
        }
    }
}

/// A message as a viewer shows it: the fields a reader sees, the text a reply quotes, the HTML
/// a sandboxed frame renders, and every attachment and embedded part with its bytes.
///
/// One reader for the person and the agent: [`MessageView::text`] is [`readable_text`], the
/// text a model is handed, so what he reads and what the agent reads cannot drift apart.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MessageView {
    pub from:        String,        // decoded for display
    pub to:          String,
    pub cc:          String,
    pub reply_to:    String,
    pub subject:     String,
    pub date:        String,        // verbatim
    pub message_id:  String,
    pub in_reply_to: String,
    pub references:  String,
    pub text:        String,        // readable_text: of an alternative the plain version
    pub html:        String,        // what a frame renders, empty when the message has no HTML
    pub parts:       Vec<ViewPart>, // attachments and embedded parts, in order
}

/// One attachment or embedded part of a [`MessageView`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ViewPart {
    pub name:       String,     // attachment_name: the file name, or an attached message's Subject
    pub media_type: String,     // lower case, e.g. `image/png`
    pub content_id: String,     // without angle brackets, empty when none
    pub inline:     bool,       // the HTML names it by `cid:`, so it is shown there, not listed
    pub bytes:      Vec<u8>,    // out of its transfer encoding
}

impl MessageView {
    /// Read a whole message from its RFC 5322 bytes; infallible, as [`ParsedMessage::parse`].
    ///
    /// The HTML is the message's own markup, untouched but for a plain part shown beside it (a
    /// list footer after an HTML body), which is escaped into a `<pre>`. It is for a frame with
    /// no scripts and no access to the page, never for the page's own document.
    pub fn parse(raw: &[u8]) -> Self {
        let hs  = headers(raw);
        let top = entity(raw);
        let rich = top.rich_parts();
        let html = if rich.iter().any(|p| p.media_type() == "text/html") {
            rich.iter().map(|p| match p.media_type().as_str() {
                "text/html" => p.text(),
                _           => fmt!("<pre style=\"white-space:pre-wrap;font:inherit\">{}</pre>",
                                    escape_html(&p.text())),
            }).collect::<Vec<String>>().join("\n")
        } else {
            String::new()
        };
        let mut parts = Vec::new();
        view_parts_into(&top, &rich, &mut parts);
        let lower = html.to_ascii_lowercase();
        for vp in parts.iter_mut() {
            vp.inline = !vp.content_id.is_empty()
                && lower.contains(&fmt!("cid:{}", vp.content_id.to_ascii_lowercase()));
        }
        Self {
            from:        decode_words(&hget(&hs, "from")),
            to:          decode_words(&hget(&hs, "to")),
            cc:          decode_words(&hget(&hs, "cc")),
            reply_to:    decode_words(&hget(&hs, "reply-to")),
            subject:     decode_words(&hget(&hs, "subject")),
            date:        hget(&hs, "date"),
            message_id:  hget(&hs, "message-id"),
            in_reply_to: hget(&hs, "in-reply-to"),
            references:  hget(&hs, "references"),
            text:        readable_text(raw),
            html,
            parts,
        }
    }
}

/// Every attachment, every embedded non-text part not shown as the body (a `cid:` logo), and
/// each attached message, which is one part and never looked inside.
fn view_parts_into(p: &Part<'_>, shown: &[&Part<'_>], out: &mut Vec<ViewPart>) {
    match &p.structure {
        Structure::Multipart(kids) => for k in kids { view_parts_into(k, shown, out); },
        _ => {
            let is_shown = shown.iter().any(|s| std::ptr::eq(*s, p));
            let media = p.media_type();
            // A named text part shown inline is listed too, as the agent's reader lists it, so
            // it can be saved as the file it is.
            if p.is_attachment() || (!is_shown && !(p.is_text() || media.starts_with("multipart/"))) {
                out.push(ViewPart {
                    name:       p.attachment_name(),
                    media_type: media,
                    content_id: p.content_id().unwrap_or_default(),
                    inline:     false,
                    bytes:      p.bytes(),
                });
            }
        }
    }
}

/// Text made safe to place inside HTML markup.
fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}


// ── Reading ─────────────────────────────────────────────────────────────

/// The first value of a header, or the empty string when it is absent.
fn hget(hs: &[(String, String)], name: &str) -> String {
    hs.iter()
        .find(|(n, _)| n == name)
        .map(|(_, v)| v.clone())
        .unwrap_or_default()
}

/// The readable text of a message, by its MIME structure (RFC 2046): of a
/// `multipart/alternative` the plain version, else the richest, and of a `mixed` each inline
/// text part in order, so a list footer follows the body it is appended to. HTML is reduced
/// to text; nothing inside an attached message is ever this message's text.
pub fn readable_text(raw: &[u8]) -> String {
    let top = entity(raw);
    let texts: Vec<String> = top.display_parts().iter()
        .map(|p| match p.media_type().as_str() {
            "text/html" => norm_lines(strip_html(&p.text()).trim()),
            _           => norm_lines(p.text().trim()),
        })
        .filter(|t| !t.is_empty())
        .collect();
    texts.join("\n\n")
}

/// Line endings a reader wants: a message's `\r\n` and bare `\r` reduced to `\n`, so the
/// text a model is handed is the text and not the wire's carriage returns.
fn norm_lines(s: &str) -> String {
    s.replace("\r\n", "\n").replace('\r', "\n")
}

/// The names of the attachments, in order: each part with a file name or disposed as an
/// attachment, and each attached message as one, named by its file name or its Subject and
/// never looked inside.
fn attachment_names(raw: &[u8]) -> Vec<String> {
    let mut out = Vec::new();
    names_into(&entity(raw), &mut out);
    out
}

fn names_into(p: &Part<'_>, out: &mut Vec<String>) {
    match &p.structure {
        Structure::Multipart(kids)  => for k in kids { names_into(k, out); },
        _                           => if p.is_attachment() { out.push(p.attachment_name()); },
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
    pub attachments: Vec<DraftAttachment>,
}

/// A file attached to a [`DraftMessage`].
#[derive(Clone, Debug, Default)]
pub struct DraftAttachment {
    pub name:       String,     // the file name the recipient sees
    pub media_type: String,     // empty for application/octet-stream
    pub bytes:      Vec<u8>,
}

impl DraftMessage {
    /// Build the RFC 5322 document this draft describes, with the headers that make a reply
    /// thread: a single `text/plain` part, its body quoted-printable, or with attachments a
    /// `multipart/mixed` of that part and each file in base64.
    ///
    /// The boundary is drawn from the Message-ID, so the same draft builds the same bytes, and
    /// it begins `=_`, which neither base64 nor quoted-printable can write.
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
        let text_head = "Content-Type: text/plain; charset=utf-8\r\n\
                         Content-Transfer-Encoding: quoted-printable";
        if self.attachments.is_empty() {
            let doc = fmt!("{}\r\n{}\r\n\r\n{}\r\n", h.join("\r\n"), text_head, encode_qp(&self.body));
            return Ok(doc.into_bytes());
        }

        let bnd = fmt!("=_daimond_{:016x}", fnv1a(self.message_id.trim().as_bytes()));
        h.push(fmt!("Content-Type: multipart/mixed; boundary=\"{}\"", bnd));
        let mut doc = fmt!("{}\r\n\r\nThis is a message in MIME format.\r\n--{}\r\n{}\r\n\r\n{}\r\n",
            h.join("\r\n"), bnd, text_head, encode_qp(&self.body));
        for att in &self.attachments {
            let media = if att.media_type.trim().is_empty() {
                "application/octet-stream"
            } else {
                att.media_type.trim()
            };
            doc.push_str(&fmt!("--{}\r\nContent-Type: {}\r\nContent-Transfer-Encoding: base64\r\n\
                Content-Disposition: attachment; {}\r\n\r\n{}\r\n",
                bnd, media, filename_param(&att.name), base64_lines(&att.bytes)));
        }
        doc.push_str(&fmt!("--{}--\r\n", bnd));
        Ok(doc.into_bytes())
    }
}

/// FNV-1a, 64 bits: a boundary that follows from the Message-ID, not from a random source.
fn fnv1a(b: &[u8]) -> u64 {
    b.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, x| (h ^ *x as u64).wrapping_mul(0x0100_0000_01b3))
}

/// Base64 in lines of 76, as RFC 2045 §6.8 requires of a body.
fn base64_lines(b: &[u8]) -> String {
    let enc = base64::encode(b);
    enc.as_bytes().chunks(76)
        .map(|c| String::from_utf8_lossy(c).into_owned())
        .collect::<Vec<String>>()
        .join("\r\n")
}

/// The `filename` parameter for a name: quoted when ASCII, else RFC 2231's `filename*` in
/// UTF-8, percent-encoded, which every reader decodes and an encoded-word inside quotes is not.
fn filename_param(name: &str) -> String {
    let name = if name.trim().is_empty() { "attachment" } else { name.trim() };
    if is_ascii(name) {
        return fmt!("filename=\"{}\"", name.replace('\\', "\\\\").replace('"', "\\\""));
    }
    let mut pct = String::new();
    for b in name.as_bytes() {
        if b.is_ascii_alphanumeric() || b"!#$&+-.^_`|~".contains(b) {
            pct.push(*b as char);
        } else {
            pct.push_str(&fmt!("%{:02X}", b));
        }
    }
    fmt!("filename*=utf-8''{}", pct)
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
        let hs = headers(raw.as_bytes());
        assert_eq!(hget(&hs, "subject"), "one two three four");
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
        assert_eq!(readable_text(raw.as_bytes()), "the plain text");
    }

    #[test]
    fn falls_back_to_html_when_no_plain_part() {
        let raw = "Content-Type: multipart/alternative; boundary=\"BB\"\r\n\r\n\
                   --BB\r\n\
                   Content-Type: text/html; charset=utf-8\r\n\r\n\
                   <p>first</p><p>second</p>\r\n\
                   --BB--\r\n";
        assert_eq!(readable_text(raw.as_bytes()), "first\nsecond");
    }

    #[test]
    fn decodes_quoted_printable_body() {
        let raw = "Content-Type: text/plain; charset=utf-8\r\n\
                   Content-Transfer-Encoding: quoted-printable\r\n\r\n\
                   caf=C3=A9\r\n";
        assert_eq!(readable_text(raw.as_bytes()), "café");
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
            attachments: Vec::new(),
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
