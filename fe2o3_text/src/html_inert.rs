//! A stranger's HTML made inert for a sandboxed frame: nothing in it fetches until the reader
//! asks for pictures.
//!
//! Hard-won, 2026-10-26: a `<meta>` Content-Security-Policy is not enough. Firefox's speculative
//! preload fetches `<img srcset>` and `<picture><source srcset>` before the policy applies, so a
//! tracking pixel fired on open with pictures off. The markup itself must therefore name nothing
//! remote, and the policy stays as the second wall.
//!
//! The output is re-serialised, not patched: every tag is rebuilt from what was read here, every
//! attribute value double-quoted and escaped, every text `<` escaped and every comment dropped.
//! The browser therefore parses exactly the structure read here, and a difference between this
//! reader and a browser's parser (a quote that runs on, a `<title>` read as text) cannot hide a
//! tag. A `<style>` body has its `<` and `&` written as CSS escapes, so it reads the same whether
//! the browser takes it as raw text or, inside `<svg>`, as markup.

/// May the markup reach the network?
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Remote {
    Blocked,    // pictures off: every remote URL is removed
    Allowed,    // the reader asked for pictures: image URLs stay, for the frame's policy to judge
}

/// Elements whose tags are dropped and whose content stays: the document's own frame (`html`,
/// `head`, `body`, whose attributes a browser would merge onto the host body), everything that
/// loads or embeds a document, plugin or resource, and SVG animation, which can set an `href`.
const DROPPED_TAGS: [&str; 20] = [
    "animate", "animatecolor", "animatemotion", "animatetransform", "applet", "base", "bgsound",
    "body", "embed", "fencedframe", "frame", "frameset", "head", "html", "link", "meta", "object",
    "param", "portal", "set",
];

/// Elements dropped with their content, which a browser reads as raw text (or RCDATA) and never
/// shows: code, a nested document, and the page title.
const DROPPED_RAW: [&str; 5] = ["iframe", "noembed", "noframes", "script", "title"];

/// Attributes whose value is a URL the browser fetches without being asked. Any other attribute
/// ending in `href` counts too, except a link's.
const URL_ATTRS: [&str; 14] = [ // fixed: HTML attribute names.
    "archive", "background", "classid", "code", "codebase", "data", "dynsrc", "href", "icon",
    "lowsrc", "manifest", "poster", "profile", "src",
];

/// Attributes holding a list of URLs, never left with pictures off.
const URL_LISTS: [&str; 2] = ["imagesrcset", "srcset"]; // fixed: HTML attribute names.

/// The named character references whose expansion holds an ASCII character: the only ones that
/// could spell `url(` or `cid:` in a value, so the only ones this reader must decode.
const ASCII_NAMED: [(&str, &str); 49] = [
    ("AMP", "&"), ("DiacriticalGrave", "`"), ("GT", ">"), ("Hat", "^"), ("LT", "<"),
    ("NewLine", "\n"), ("QUOT", "\""), ("Tab", "\t"), ("UnderBar", "_"), ("VerticalLine", "|"),
    ("amp", "&"), ("apos", "'"), ("ast", "*"), ("bne", "=\u{20e5}"), ("bsol", "\\"),
    ("colon", ":"), ("comma", ","), ("commat", "@"), ("dollar", "$"), ("equals", "="),
    ("excl", "!"), ("fjlig", "fj"), ("grave", "`"), ("gt", ">"), ("lbrace", "{"),
    ("lbrack", "["), ("lcub", "{"), ("lowbar", "_"), ("lpar", "("), ("lsqb", "["), ("lt", "<"),
    ("midast", "*"), ("num", "#"), ("nvgt", ">\u{20d2}"), ("nvlt", "<\u{20d2}"),
    ("percnt", "%"), ("period", "."), ("plus", "+"), ("quest", "?"), ("quot", "\""),
    ("rbrace", "}"), ("rbrack", "]"), ("rcub", "}"), ("rpar", ")"), ("rsqb", "]"),
    ("semi", ";"), ("sol", "/"), ("verbar", "|"), ("vert", "|"),
];

/// The legacy references a browser decodes without a `;` that expand to ASCII.
const ASCII_LEGACY: [(&str, &str); 8] = [
    ("AMP", "&"), ("GT", ">"), ("LT", "<"), ("QUOT", "\""),
    ("amp", "&"), ("gt", ">"), ("lt", "<"), ("quot", "\""),
];

/// Windows-1252 for the C1 range of a numeric reference (HTML §13.2.5.80); a code not listed
/// keeps its value.
const C1: [(u32, u32); 27] = [
    (0x80, 0x20ac), (0x82, 0x201a), (0x83, 0x0192), (0x84, 0x201e), (0x85, 0x2026),
    (0x86, 0x2020), (0x87, 0x2021), (0x88, 0x02c6), (0x89, 0x2030), (0x8a, 0x0160),
    (0x8b, 0x2039), (0x8c, 0x0152), (0x8e, 0x017d), (0x91, 0x2018), (0x92, 0x2019),
    (0x93, 0x201c), (0x94, 0x201d), (0x95, 0x2022), (0x96, 0x2013), (0x97, 0x2014),
    (0x98, 0x02dc), (0x99, 0x2122), (0x9a, 0x0161), (0x9b, 0x203a), (0x9c, 0x0153),
    (0x9e, 0x017e), (0x9f, 0x0178),
];

/// The HTML a sandboxed frame is given for a message. With [`Remote::Blocked`] it names no
/// resource outside itself: `src`, `srcset`, `background`, `poster`, every non-link `href` and
/// the other fetching attributes are removed unless they hold a `data:` or `cid:` URL, and CSS
/// `url()`, `image()`, `image-set()` and `src()` (in `style` attributes, `<style>` and SVG
/// presentation attributes) are renamed so they no longer parse. In either mode `<script>`,
/// frames, objects, `<link>`, `<meta>` (a refresh), `<base>`, `@import`, `ping`, event handlers
/// and comments are removed.
pub fn inert_html(html: &str, remote: Remote) -> String {
    let s = html;
    let b = s.as_bytes();
    let mut out = String::with_capacity(s.len() + s.len() / 8);
    let mut i = 0;
    while i < b.len() {
        let lt = match s[i..].find('<') {
            Some(k) => i + k,
            None    => {
                push_text(&mut out, &s[i..]);
                break;
            }
        };
        push_text(&mut out, &s[i..lt]);
        i = lt;
        let rest = &s[i..];
        if rest.starts_with("<!--") {
            i = comment_end(s, i + 4);
        } else if rest.starts_with("<!") || rest.starts_with("<?") {
            i = bogus_end(s, i + 2);
        } else if rest.starts_with("</") {
            match b.get(i + 2) {
                Some(c) if c.is_ascii_alphabetic() => {
                    let (tag, next) = read_tag(s, i + 2);
                    i = next;
                    if let Some(t) = tag {
                        end_tag(&mut out, &t.name);
                    }
                }
                Some(b'>')  => i += 3,
                Some(_)     => i = bogus_end(s, i + 2),
                None        => {
                    out.push_str("&lt;/");
                    i += 2;
                }
            }
        } else if b.get(i + 1).map_or(false, |c| c.is_ascii_alphabetic()) {
            let (tag, next) = read_tag(s, i + 1);
            i = next;
            // A tag the input ends inside is dropped, as a browser drops it.
            if let Some(t) = tag {
                i = start_tag(&mut out, s, &t, i, remote);
            }
        } else {
            out.push_str("&lt;");
            i += 1;
        }
    }
    out
}

struct Tag {
    name:   String,
    attrs:  Vec<(String, String)>,  // lower-case name, raw value; the first of a name wins
    closed: bool,                   // written `<x/>`
}

fn is_ws(c: u8) -> bool {
    matches!(c, b'\t' | b'\n' | b'\x0c' | b'\r' | b' ')
}

/// After `<!--`: `<!-->` and `<!--->` end at once, else at `-->` or `--!>`, else at the end.
fn comment_end(s: &str, j: usize) -> usize {
    let r = &s[j..];
    if r.starts_with('>') {
        return j + 1;
    }
    if r.starts_with("->") {
        return j + 2;
    }
    let a = r.find("-->").map(|k| j + k + 3);
    let b = r.find("--!>").map(|k| j + k + 4);
    match (a, b) {
        (Some(x), Some(y))  => x.min(y),
        (Some(x), None)     => x,
        (None, Some(y))     => y,
        (None, None)        => s.len(),
    }
}

fn bogus_end(s: &str, j: usize) -> usize {
    match s[j..].find('>') {
        Some(k) => j + k + 1,
        None    => s.len(),
    }
}

/// A tag from its name's first byte to just past its `>`, read as HTML's tokenizer reads it
/// (§13.2.5.6 onward); `None` when the input ends inside it.
fn read_tag(s: &str, j: usize) -> (Option<Tag>, usize) {
    let b = s.as_bytes();
    let n = b.len();
    let mut k = j;
    while k < n && !is_ws(b[k]) && b[k] != b'/' && b[k] != b'>' {
        k += 1;
    }
    let mut t = Tag { name: s[j..k].to_ascii_lowercase(), attrs: Vec::new(), closed: false };
    loop {
        while k < n && is_ws(b[k]) {
            k += 1;
        }
        if k >= n {
            return (None, n);
        }
        match b[k] {
            b'>' => return (Some(t), k + 1),
            b'/' => {
                if b.get(k + 1) == Some(&b'>') {
                    t.closed = true;
                    return (Some(t), k + 2);
                }
                k += 1;
                continue;
            }
            _ => {}
        }
        // An attribute name; a leading `=` belongs to it.
        let a0 = k;
        k += 1;
        while k < n && !is_ws(b[k]) && b[k] != b'/' && b[k] != b'>' && b[k] != b'=' {
            k += 1;
        }
        let name = s[a0..k].to_ascii_lowercase();
        while k < n && is_ws(b[k]) {
            k += 1;
        }
        let mut value = String::new();
        if k < n && b[k] == b'=' {
            k += 1;
            while k < n && is_ws(b[k]) {
                k += 1;
            }
            if k >= n {
                return (None, n);
            }
            match b[k] {
                q @ (b'"' | b'\'') => {
                    let v0 = k + 1;
                    match s[v0..].find(q as char) {
                        Some(e) => {
                            value = s[v0..v0 + e].to_string();
                            k = v0 + e + 1;
                        }
                        None => return (None, n),
                    }
                }
                b'>' => {}
                _ => {
                    let v0 = k;
                    while k < n && !is_ws(b[k]) && b[k] != b'>' {
                        k += 1;
                    }
                    value = s[v0..k].to_string();
                }
            }
        }
        if !t.attrs.iter().any(|(m, _)| *m == name) {
            t.attrs.push((name, value));
        }
    }
}

/// Where the raw text of `name` ends: at `</name` followed by a space, `/` or `>`, else at the
/// end of the input.
fn raw_end(s: &str, from: usize, name: &str) -> usize {
    let b = s.as_bytes();
    let lower = s[from..].to_ascii_lowercase();
    let pat = format!("</{}", name);
    let mut at = 0;
    while let Some(k) = lower[at..].find(&pat) {
        let e = from + at + k + pat.len();
        match b.get(e) {
            None                                        => return from + at + k,
            Some(c) if is_ws(*c) || *c == b'/' || *c == b'>' => return from + at + k,
            _                                           => at += k + pat.len(),
        }
    }
    s.len()
}

/// Write a start tag and, for the elements a browser reads as raw text, their content; return
/// where reading resumes.
fn start_tag(out: &mut String, s: &str, t: &Tag, i: usize, remote: Remote) -> usize {
    let name = t.name.as_str();
    if DROPPED_RAW.contains(&name) {
        return raw_end(s, i, name);
    }
    match name {
        "style" => {
            let e = raw_end(s, i, name);
            out.push_str("<style>");
            push_css(out, &css_inert(&s[i..e], remote));
            out.push_str("</style>");
            // The end tag is consumed here, so a missing one is supplied.
            return if e < s.len() { bogus_end(s, e) } else { e };
        }
        "xmp" | "plaintext" => {
            // Raw text shown as it stands: a `<pre>` of the escaped text.
            let e = if name == "plaintext" { s.len() } else { raw_end(s, i, name) };
            out.push_str("<pre>");
            push_literal(out, &s[i..e]);
            out.push_str("</pre>");
            return if e < s.len() { bogus_end(s, e) } else { e };
        }
        _ => {}
    }
    if DROPPED_TAGS.contains(&name) || !valid_tag(name) {
        return i;
    }
    out.push('<');
    out.push_str(name);
    for (n, v) in &t.attrs {
        if let Some(val) = attr_value(name, n, v, remote) {
            out.push(' ');
            out.push_str(n);
            out.push_str("=\"");
            out.push_str(&val);
            out.push('"');
        }
    }
    if t.closed {
        out.push_str(" /");
    }
    out.push('>');
    if name == "textarea" {
        // RCDATA: its content is text, whatever it looks like.
        let e = raw_end(s, i, name);
        push_text(out, &s[i..e]);
        return e;
    }
    i
}

fn end_tag(out: &mut String, name: &str) {
    if DROPPED_TAGS.contains(&name) || DROPPED_RAW.contains(&name) || !valid_tag(name) {
        return;
    }
    out.push_str("</");
    out.push_str(name);
    out.push('>');
}

/// A tag name worth keeping, Outlook's `o:p` and VML's `v:shape` among them.
fn valid_tag(n: &str) -> bool {
    let b = n.as_bytes();
    !b.is_empty() && b[0].is_ascii_lowercase()
        && b.iter().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, b'-' | b':' | b'_' | b'.'))
}

fn valid_attr(n: &str) -> bool {
    let b = n.as_bytes();
    !b.is_empty() && (b[0].is_ascii_lowercase() || b[0] == b'_' || b[0] == b':')
        && b.iter().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, b'-' | b':' | b'_' | b'.'))
}

/// The escaped value an attribute is written with, or `None` when it is dropped.
fn attr_value(el: &str, n: &str, raw: &str, remote: Remote) -> Option<String> {
    if !valid_attr(n) || n == "ping" || n.starts_with("on") {
        return None;
    }
    let (dec, esc) = decode_attr(raw);
    let link = (el == "a" || el == "area") && (n == "href" || n.ends_with(":href"));
    if link {
        return Some(esc);
    }
    if remote == Remote::Blocked {
        if URL_LISTS.contains(&n) {
            return None;
        }
        if (URL_ATTRS.contains(&n) || n.ends_with("href")) && !is_local(&dec.to_ascii_lowercase()) {
            return None;
        }
    }
    let css = css_inert(&dec, remote);
    if css == dec {
        Some(esc)
    } else {
        let mut o = String::with_capacity(css.len());
        for c in css.chars() {
            push_attr_char(&mut o, c);
        }
        Some(o)
    }
}

/// Is this (lower-case) URL the message's own, a `data:` or `cid:` that reaches no host?
fn is_local(u: &str) -> bool {
    let t = u.trim_start_matches(|c: char| c.is_ascii_whitespace() || c == '"' || c == '\'');
    t.starts_with("data:") || t.starts_with("cid:")
}

fn push_attr_char(o: &mut String, c: char) {
    match c {
        '&' => o.push_str("&amp;"),
        '"' => o.push_str("&quot;"),
        '<' => o.push_str("&lt;"),
        '>' => o.push_str("&gt;"),
        _   => o.push(c),
    }
}

/// An attribute value as a browser decodes it, as far as ASCII goes, and the same value
/// escaped for a double-quoted attribute. A reference this reader does not know expands, if at
/// all, to no ASCII character, so it is kept as written for the browser to expand.
fn decode_attr(raw: &str) -> (String, String) {
    let mut dec = String::with_capacity(raw.len());
    let mut esc = String::with_capacity(raw.len() + 8);
    let b = raw.as_bytes();
    let mut i = 0;
    while i < raw.len() {
        if b[i] != b'&' {
            let c = raw[i..].chars().next().unwrap_or('\u{fffd}');
            dec.push(c);
            push_attr_char(&mut esc, c);
            i += c.len_utf8();
            continue;
        }
        // A numeric reference, `;` optional.
        if b.get(i + 1) == Some(&b'#') {
            let hex = matches!(b.get(i + 2), Some(b'x') | Some(b'X'));
            let d0 = if hex { i + 3 } else { i + 2 };
            let mut k = d0;
            while k < b.len() && (if hex { b[k].is_ascii_hexdigit() } else { b[k].is_ascii_digit() }) {
                k += 1;
            }
            if k > d0 {
                let code = u32::from_str_radix(&raw[d0..k.min(d0 + 8)], if hex { 16 } else { 10 })
                    .unwrap_or(0x110000);
                let code = if k - d0 > 8 { 0x110000 } else { code };
                let code = C1.iter().find(|(f, _)| *f == code).map(|(_, t)| *t).unwrap_or(code);
                let c = match code {
                    0 => '\u{fffd}',
                    _ => char::from_u32(code).unwrap_or('\u{fffd}'),
                };
                dec.push(c);
                push_attr_char(&mut esc, c);
                i = if b.get(k) == Some(&b';') { k + 1 } else { k };
                continue;
            }
        }
        // A named reference.
        let mut k = i + 1;
        while k < b.len() && b[k].is_ascii_alphanumeric() {
            k += 1;
        }
        let name = &raw[i + 1..k];
        if b.get(k) == Some(&b';') {
            if let Some((_, x)) = ASCII_NAMED.iter().find(|(m, _)| *m == name) {
                dec.push_str(x);
                for c in x.chars() {
                    push_attr_char(&mut esc, c);
                }
                i = k + 1;
                continue;
            }
        } else if b.get(k) != Some(&b'=') {
            // In an attribute a legacy name followed by a letter, digit or `=` is not decoded.
            if let Some((_, x)) = ASCII_LEGACY.iter().find(|(m, _)| *m == name) {
                dec.push_str(x);
                for c in x.chars() {
                    push_attr_char(&mut esc, c);
                }
                i = k;
                continue;
            }
        }
        // Not one of ours: kept, `&` and all, for the browser.
        dec.push('&');
        esc.push('&');
        i += 1;
    }
    (dec, esc)
}

/// Text between tags: `<` and `>` escaped, references left for the browser, which expands
/// them as text and never as markup.
fn push_text(out: &mut String, s: &str) {
    for c in s.chars() {
        match c {
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            _   => out.push(c),
        }
    }
}

/// Raw text shown as text: `&` escaped too, since nothing in it was a reference.
fn push_literal(out: &mut String, s: &str) {
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            _   => out.push(c),
        }
    }
}

/// A stylesheet's text for a `<style>` body: `<` and `&` as CSS escapes, so no end tag and no
/// reference can appear in it.
fn push_css(out: &mut String, css: &str) {
    for c in css.chars() {
        match c {
            '<' => out.push_str("\\3c "),
            '&' => out.push_str("\\26 "),
            _   => out.push(c),
        }
    }
}

fn is_ident(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'-' || c == b'_' || c >= 0x80
}

/// CSS with nothing left that fetches: `@import` always, and with pictures off every `url()`
/// not naming a `data:` or `cid:` URL, `image()`, `image-set()` and `src()` (any vendor
/// prefix). Each is renamed with an `x-` the parser does not know, so its declaration or rule
/// is dropped. Escapes that spell a letter, digit, `-` or `_` are decoded first, so `\75 rl(`
/// is seen as the `url(` it is.
pub fn css_inert(css: &str, remote: Remote) -> String {
    let d = css_unescape(css);
    let lower = d.to_ascii_lowercase();
    let b = lower.as_bytes();
    let mut out = String::with_capacity(d.len() + 16);
    let mut last = 0;
    for (k, &c) in b.iter().enumerate() {
        match c {
            b'(' => {
                let mut st = k;
                while st > 0 && is_ident(b[st - 1]) {
                    st -= 1;
                }
                if css_fetches(&lower[st..k], &lower[k + 1..], remote) {
                    out.push_str(&d[last..st]);
                    out.push_str("x-");
                    last = st;
                }
            }
            b'@' if lower[k + 1..].starts_with("import") => {
                out.push_str(&d[last..k + 1]);
                out.push_str("x-");
                last = k + 1;
            }
            _ => {}
        }
    }
    out.push_str(&d[last..]);
    out
}

/// Does a CSS function of this (lower-case) name, with these arguments, fetch?
fn css_fetches(name: &str, args: &str, remote: Remote) -> bool {
    if remote == Remote::Allowed {
        return false;
    }
    // `-webkit-image-set` is `image-set`.
    let base = match name.strip_prefix('-') {
        Some(r) => match r.find('-') {
            Some(k) => &r[k + 1..],
            None    => name,
        },
        None => name,
    };
    match base {
        "url"                           => !is_local(args),
        "image" | "image-set" | "src"   => true,
        _                               => false,
    }
}

/// CSS escapes for a letter, digit, `-` or `_` replaced by the character; any other escape kept,
/// a hexadecimal one rewritten with its terminating space so a decoded character after it
/// cannot extend it.
fn css_unescape(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < s.len() {
        if b[i] != b'\\' {
            let c = s[i..].chars().next().unwrap_or('\u{fffd}');
            out.push(c);
            i += c.len_utf8();
            continue;
        }
        match b.get(i + 1) {
            None | Some(b'\n') | Some(b'\r') | Some(b'\x0c') => {
                out.push('\\');
                i += 1;
            }
            Some(h) if h.is_ascii_hexdigit() => {
                let mut k = i + 1;
                while k < b.len() && k < i + 7 && b[k].is_ascii_hexdigit() {
                    k += 1;
                }
                let code = u32::from_str_radix(&s[i + 1..k], 16).unwrap_or(0xfffd);
                let hexs = &s[i + 1..k];
                // One whitespace ends the escape, `\r\n` counting as one.
                if s[k..].starts_with("\r\n") {
                    k += 2;
                } else if k < b.len() && is_ws(b[k]) {
                    k += 1;
                }
                match char::from_u32(code) {
                    Some(c) if c.is_ascii_alphanumeric() || c == '-' || c == '_' => out.push(c),
                    _ => {
                        out.push('\\');
                        out.push_str(hexs);
                        out.push(' ');
                    }
                }
                i = k;
            }
            Some(_) => {
                let c = s[i + 1..].chars().next().unwrap_or('\u{fffd}');
                if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                    out.push(c);
                } else {
                    out.push('\\');
                    out.push(c);
                }
                i += 1 + c.len_utf8();
            }
        }
    }
    out
}
