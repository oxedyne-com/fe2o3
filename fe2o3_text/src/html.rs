//! Reducing an HTML document to the text a reader would take from it.
//!
//! Not a parser, and deliberately not one. Nothing here builds a DOM, resolves
//! a namespace or recovers from a mis-nested tag the way a browser must,
//! because the question being answered is narrower: what does this page *say*?
//! A single pass that drops the markup, drops the elements that hold no prose
//! (a script, a stylesheet, a navigation bar), keeps the ones that do, and
//! collapses what is left into lines, answers it -- and answers it on malformed
//! input too, which a strict parser would refuse.
//!
//! The two callers this exists for are a server fetching a page on a user's
//! behalf, and a mail client rendering an HTML part as text.

use std::collections::BTreeSet;


/// Elements whose content is not prose, and is dropped along with the tags.
///
/// A script or a stylesheet is code; a navigation bar and a footer are the
/// furniture around the page rather than the page.
const DROPPED: [&str; 9] = [
    "script",
    "style",
    "noscript",
    "svg",
    "canvas",
    "template",
    "iframe",
    "nav",
    "footer",
];

/// Elements that begin a new line of text. Anything not named here is inline,
/// so the text of a link, an emphasis or a span joins the sentence it sits in
/// rather than breaking it.
const BLOCK: [&str; 27] = [
    "address",
    "article",
    "aside",
    "blockquote",
    "br",
    "dd",
    "div",
    "dl",
    "dt",
    "figcaption",
    "figure",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "header",
    "hr",
    "li",
    "main",
    "ol",
    "p",
    "pre",
    "section",
    "table",
    "tr",
];

/// The named character references worth knowing, which are the handful that
/// appear in prose. Anything else numeric is decoded by its code point.
const NAMED: [(&str, &str); 14] = [
    ("amp",    "&"),
    ("lt",     "<"),
    ("gt",     ">"),
    ("quot",   "\""),
    ("apos",   "'"),
    ("nbsp",   " "),
    ("ndash",  "\u{2013}"),
    ("mdash",  "\u{2014}"),
    ("hellip", "\u{2026}"),
    ("lsquo",  "\u{2018}"),
    ("rsquo",  "\u{2019}"),
    ("ldquo",  "\u{201c}"),
    ("rdquo",  "\u{201d}"),
    ("middot", "\u{b7}"),
];


/// What a page says, once its markup is gone.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PageText {
    /// The document title, empty when the page names none.
    pub title: String,
    /// The readable text, one block element to a line.
    pub text:  String,
}

/// Strip an HTML document to its title and its readable text.
///
/// Headings, paragraphs, list items and the text of links all survive; scripts,
/// stylesheets, navigation and footers do not; runs of whitespace collapse to
/// one space, and blank lines are dropped.
pub fn html_to_text(html: &str) -> PageText {
    let dropped: BTreeSet<&str> = DROPPED.iter().copied().collect();
    let block:   BTreeSet<&str> = BLOCK.iter().copied().collect();

    let mut title_raw = String::new();
    let mut body_raw  = String::with_capacity(html.len() / 2);
    // The element whose content is being discarded, if any. Only its own
    // closing tag ends the discard, so a `<script>` containing `if (a < b)`
    // cannot end it early.
    let mut skip:     Option<String> = None;
    let mut in_title = false;
    let mut rest     = html;

    loop {
        let lt = match rest.find('<') {
            Some(i) => i,
            None => {
                push(&mut body_raw, &mut title_raw, rest, &skip, in_title);
                break;
            }
        };
        push(&mut body_raw, &mut title_raw, &rest[..lt], &skip, in_title);
        let tag = &rest[lt..];

        // A comment, or a declaration such as the doctype. Neither says
        // anything, and a comment may hold markup that must not be read.
        if tag.starts_with("<!--") {
            rest = match tag.find("-->") {
                Some(i) => &tag[i + 3..],
                None    => "",
            };
            continue;
        }
        if tag.starts_with("<!") {
            rest = match tag.find('>') {
                Some(i) => &tag[i + 1..],
                None    => "",
            };
            continue;
        }

        let end = match tag_end(tag) {
            Some(i) => i,
            // An unterminated tag: there is no more markup, and no more text.
            None => break,
        };
        let inner = &tag[1..end];
        rest = &tag[end + 1..];

        let closing = inner.starts_with('/');
        let name: String = inner
            .trim_start_matches('/')
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric())
            .collect::<String>()
            .to_lowercase();
        if name.is_empty() {
            continue;
        }

        if let Some(open) = &skip {
            if closing && &name == open {
                skip = None;
            }
            continue;
        }
        if !closing && dropped.contains(name.as_str()) {
            // A self-closing tag encloses nothing, so nothing is discarded.
            if !inner.trim_end().ends_with('/') {
                skip = Some(name);
            }
            continue;
        }
        if name == "title" {
            in_title = !closing;
            continue;
        }
        if block.contains(name.as_str()) {
            body_raw.push('\n');
        }
    }

    PageText {
        title: squash(&decode_entities(&title_raw)),
        text:  lines(&decode_entities(&body_raw)),
    }
}

/// Add a run of text to the title or the body, unless it belongs to an element
/// whose content is being discarded.
///
/// Newlines within the text are not breaks: HTML wraps its source wherever it
/// likes, and a sentence split across two lines of markup is still one
/// sentence. Only a block tag breaks a line, so every whitespace character in
/// the text itself becomes a space, and the layout of the file is forgotten.
fn push(
    body:       &mut String,
    title:      &mut String,
    text:       &str,
    skip:       &Option<String>,
    in_title:   bool,
) {
    if skip.is_some() || text.is_empty() {
        return;
    }
    let flat: String = text
        .chars()
        .map(|c| if c.is_whitespace() { ' ' } else { c })
        .collect();
    if in_title {
        title.push_str(&flat);
    } else {
        body.push_str(&flat);
    }
}

/// The index of the `>` that closes a tag, ignoring any inside a quoted
/// attribute value -- `<a title="a > b">` is one tag, not two.
fn tag_end(s: &str) -> Option<usize> {
    let mut quote: Option<char> = None;
    for (i, c) in s.char_indices().skip(1) {
        match quote {
            Some(q) => if c == q {
                quote = None;
            },
            None => match c {
                '"' | '\'' => quote = Some(c),
                '>'        => return Some(i),
                _          => (),
            },
        }
    }
    None
}

/// Decode the character references a page's prose actually uses: the named ones
/// worth knowing, and any numeric one, decimal or hexadecimal.
///
/// An `&` that begins nothing recognisable is left exactly as it is, which is
/// what a browser does and what a reader expects.
pub fn decode_entities(s: &str) -> String {
    let mut out  = String::with_capacity(s.len());
    let mut rest = s;
    loop {
        let amp = match rest.find('&') {
            Some(i) => i,
            None => {
                out.push_str(rest);
                return out;
            }
        };
        out.push_str(&rest[..amp]);
        let after = &rest[amp + 1..];
        // A reference is short; a `&` with no `;` close behind it is just an
        // ampersand.
        let semi = match after.char_indices().take(12).find(|(_, c)| *c == ';') {
            Some((i, _)) => i,
            None => {
                out.push('&');
                rest = after;
                continue;
            }
        };
        let name = &after[..semi];
        match entity(name) {
            Some(c) => out.push_str(&c),
            None    => {
                out.push('&');
                out.push_str(name);
                out.push(';');
            }
        }
        rest = &after[semi + 1..];
    }
}

/// One character reference, by name or by code point.
fn entity(name: &str) -> Option<String> {
    for (n, v) in NAMED {
        if name.eq_ignore_ascii_case(n) {
            return Some(v.to_string());
        }
    }
    let digits = match name.strip_prefix('#') {
        Some(d) => d,
        None    => return None,
    };
    let code = match digits.strip_prefix('x').or_else(|| digits.strip_prefix('X')) {
        Some(hex) => match u32::from_str_radix(hex, 16) {
            Ok(n)  => n,
            Err(_) => return None,
        },
        None => match digits.parse::<u32>() {
            Ok(n)  => n,
            Err(_) => return None,
        },
    };
    char::from_u32(code).map(|c| c.to_string())
}

/// Collapse every run of whitespace to one space and trim, leaving one line.
pub fn squash(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Collapse the whitespace within each line, and drop the lines that hold
/// nothing.
fn lines(s: &str) -> String {
    s.split('\n')
        .map(squash)
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ TESTS                                                                     │
// └───────────────────────────────────────────────────────────────────────────┘

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_the_title_is_taken_and_kept_out_of_the_text() {
        let p = html_to_text("<html><head><title> The  Page </title></head>\
            <body><p>Hello</p></body></html>");
        assert_eq!(p.title, "The Page");
        assert_eq!(p.text,  "Hello");
    }

    #[test]
    fn test_scripts_styles_navigation_and_footers_are_dropped() {
        let p = html_to_text("
            <nav><a href='/x'>Home</a></nav>
            <script>var a = 1 < 2 && 3 > 2;</script>
            <style>body { color: red; }</style>
            <p>The only prose.</p>
            <footer>Copyright</footer>
        ");
        assert_eq!(p.text, "The only prose.");
    }

    #[test]
    fn test_headings_paragraphs_list_items_and_link_text_survive() {
        let p = html_to_text("
            <h1>Title</h1>
            <p>A <a href='/a'>link</a> in a sentence.</p>
            <ul><li>One</li><li>Two</li></ul>
        ");
        // A link is inline, so its text joins the sentence rather than
        // breaking it; a list item is a block, so each takes a line.
        assert_eq!(p.text, "Title\nA link in a sentence.\nOne\nTwo");
    }

    #[test]
    fn test_entities_are_decoded() {
        let p = html_to_text("<p>Tom &amp; Jerry &lt;3 caf&#233; &#x2014; \
            &quot;quoted&quot;&nbsp;&hellip; R&D</p>");
        assert_eq!(p.text, "Tom & Jerry <3 café — \"quoted\" … R&D");
    }

    #[test]
    fn test_an_angle_bracket_in_an_attribute_does_not_end_the_tag() {
        let p = html_to_text("<p title=\"a > b\">Text</p>");
        assert_eq!(p.text, "Text");
    }

    #[test]
    fn test_comments_and_the_doctype_say_nothing() {
        let p = html_to_text("<!DOCTYPE html><!-- <p>hidden</p> --><p>shown</p>");
        assert_eq!(p.text, "shown");
    }

    #[test]
    fn test_broken_markup_still_yields_its_text() {
        // No closing tags, an unterminated tag at the end: a browser reads
        // this, and so must this.
        let p = html_to_text("<p>One<p>Two<p>Three<p");
        assert_eq!(p.text, "One\nTwo\nThree");
    }
}
