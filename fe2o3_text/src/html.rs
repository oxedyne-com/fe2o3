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

/// Every named character reference HTML 4 defines, with `apos` and the upper-case
/// `AMP`, `LT`, `GT` and `QUOT` that HTML5 keeps from SGML, sorted by name for a
/// binary search. Names are case-sensitive: `&Eacute;` is not `&eacute;`. HTML5's
/// further two thousand names, almost all mathematical, are left as written, and
/// any numeric reference is decoded by its code point. `&nbsp;` reads as a plain
/// space, since this text is for reading rather than setting.
const NAMED: [(&str, char); 257] = [
    ("AElig",    '\u{c6}'),
    ("AMP",      '&'),
    ("Aacute",   '\u{c1}'),
    ("Acirc",    '\u{c2}'),
    ("Agrave",   '\u{c0}'),
    ("Alpha",    '\u{391}'),
    ("Aring",    '\u{c5}'),
    ("Atilde",   '\u{c3}'),
    ("Auml",     '\u{c4}'),
    ("Beta",     '\u{392}'),
    ("Ccedil",   '\u{c7}'),
    ("Chi",      '\u{3a7}'),
    ("Dagger",   '\u{2021}'),
    ("Delta",    '\u{394}'),
    ("ETH",      '\u{d0}'),
    ("Eacute",   '\u{c9}'),
    ("Ecirc",    '\u{ca}'),
    ("Egrave",   '\u{c8}'),
    ("Epsilon",  '\u{395}'),
    ("Eta",      '\u{397}'),
    ("Euml",     '\u{cb}'),
    ("GT",       '>'),
    ("Gamma",    '\u{393}'),
    ("Iacute",   '\u{cd}'),
    ("Icirc",    '\u{ce}'),
    ("Igrave",   '\u{cc}'),
    ("Iota",     '\u{399}'),
    ("Iuml",     '\u{cf}'),
    ("Kappa",    '\u{39a}'),
    ("LT",       '<'),
    ("Lambda",   '\u{39b}'),
    ("Mu",       '\u{39c}'),
    ("Ntilde",   '\u{d1}'),
    ("Nu",       '\u{39d}'),
    ("OElig",    '\u{152}'),
    ("Oacute",   '\u{d3}'),
    ("Ocirc",    '\u{d4}'),
    ("Ograve",   '\u{d2}'),
    ("Omega",    '\u{3a9}'),
    ("Omicron",  '\u{39f}'),
    ("Oslash",   '\u{d8}'),
    ("Otilde",   '\u{d5}'),
    ("Ouml",     '\u{d6}'),
    ("Phi",      '\u{3a6}'),
    ("Pi",       '\u{3a0}'),
    ("Prime",    '\u{2033}'),
    ("Psi",      '\u{3a8}'),
    ("QUOT",     '"'),
    ("Rho",      '\u{3a1}'),
    ("Scaron",   '\u{160}'),
    ("Sigma",    '\u{3a3}'),
    ("THORN",    '\u{de}'),
    ("Tau",      '\u{3a4}'),
    ("Theta",    '\u{398}'),
    ("Uacute",   '\u{da}'),
    ("Ucirc",    '\u{db}'),
    ("Ugrave",   '\u{d9}'),
    ("Upsilon",  '\u{3a5}'),
    ("Uuml",     '\u{dc}'),
    ("Xi",       '\u{39e}'),
    ("Yacute",   '\u{dd}'),
    ("Yuml",     '\u{178}'),
    ("Zeta",     '\u{396}'),
    ("aacute",   '\u{e1}'),
    ("acirc",    '\u{e2}'),
    ("acute",    '\u{b4}'),
    ("aelig",    '\u{e6}'),
    ("agrave",   '\u{e0}'),
    ("alefsym",  '\u{2135}'),
    ("alpha",    '\u{3b1}'),
    ("amp",      '&'),
    ("and",      '\u{2227}'),
    ("ang",      '\u{2220}'),
    ("apos",     '\''),
    ("aring",    '\u{e5}'),
    ("asymp",    '\u{2248}'),
    ("atilde",   '\u{e3}'),
    ("auml",     '\u{e4}'),
    ("bdquo",    '\u{201e}'),
    ("beta",     '\u{3b2}'),
    ("brvbar",   '\u{a6}'),
    ("bull",     '\u{2022}'),
    ("cap",      '\u{2229}'),
    ("ccedil",   '\u{e7}'),
    ("cedil",    '\u{b8}'),
    ("cent",     '\u{a2}'),
    ("chi",      '\u{3c7}'),
    ("circ",     '\u{2c6}'),
    ("clubs",    '\u{2663}'),
    ("cong",     '\u{2245}'),
    ("copy",     '\u{a9}'),
    ("crarr",    '\u{21b5}'),
    ("cup",      '\u{222a}'),
    ("curren",   '\u{a4}'),
    ("dArr",     '\u{21d3}'),
    ("dagger",   '\u{2020}'),
    ("darr",     '\u{2193}'),
    ("deg",      '\u{b0}'),
    ("delta",    '\u{3b4}'),
    ("diams",    '\u{2666}'),
    ("divide",   '\u{f7}'),
    ("eacute",   '\u{e9}'),
    ("ecirc",    '\u{ea}'),
    ("egrave",   '\u{e8}'),
    ("empty",    '\u{2205}'),
    ("emsp",     '\u{2003}'),
    ("ensp",     '\u{2002}'),
    ("epsilon",  '\u{3b5}'),
    ("equiv",    '\u{2261}'),
    ("eta",      '\u{3b7}'),
    ("eth",      '\u{f0}'),
    ("euml",     '\u{eb}'),
    ("euro",     '\u{20ac}'),
    ("exist",    '\u{2203}'),
    ("fnof",     '\u{192}'),
    ("forall",   '\u{2200}'),
    ("frac12",   '\u{bd}'),
    ("frac14",   '\u{bc}'),
    ("frac34",   '\u{be}'),
    ("frasl",    '\u{2044}'),
    ("gamma",    '\u{3b3}'),
    ("ge",       '\u{2265}'),
    ("gt",       '>'),
    ("hArr",     '\u{21d4}'),
    ("harr",     '\u{2194}'),
    ("hearts",   '\u{2665}'),
    ("hellip",   '\u{2026}'),
    ("iacute",   '\u{ed}'),
    ("icirc",    '\u{ee}'),
    ("iexcl",    '\u{a1}'),
    ("igrave",   '\u{ec}'),
    ("image",    '\u{2111}'),
    ("infin",    '\u{221e}'),
    ("int",      '\u{222b}'),
    ("iota",     '\u{3b9}'),
    ("iquest",   '\u{bf}'),
    ("isin",     '\u{2208}'),
    ("iuml",     '\u{ef}'),
    ("kappa",    '\u{3ba}'),
    ("lArr",     '\u{21d0}'),
    ("lambda",   '\u{3bb}'),
    ("lang",     '\u{2329}'),
    ("laquo",    '\u{ab}'),
    ("larr",     '\u{2190}'),
    ("lceil",    '\u{2308}'),
    ("ldquo",    '\u{201c}'),
    ("le",       '\u{2264}'),
    ("lfloor",   '\u{230a}'),
    ("lowast",   '\u{2217}'),
    ("loz",      '\u{25ca}'),
    ("lrm",      '\u{200e}'),
    ("lsaquo",   '\u{2039}'),
    ("lsquo",    '\u{2018}'),
    ("lt",       '<'),
    ("macr",     '\u{af}'),
    ("mdash",    '\u{2014}'),
    ("micro",    '\u{b5}'),
    ("middot",   '\u{b7}'),
    ("minus",    '\u{2212}'),
    ("mu",       '\u{3bc}'),
    ("nabla",    '\u{2207}'),
    ("nbsp",     ' '),
    ("ndash",    '\u{2013}'),
    ("ne",       '\u{2260}'),
    ("ni",       '\u{220b}'),
    ("not",      '\u{ac}'),
    ("notin",    '\u{2209}'),
    ("nsub",     '\u{2284}'),
    ("ntilde",   '\u{f1}'),
    ("nu",       '\u{3bd}'),
    ("oacute",   '\u{f3}'),
    ("ocirc",    '\u{f4}'),
    ("oelig",    '\u{153}'),
    ("ograve",   '\u{f2}'),
    ("oline",    '\u{203e}'),
    ("omega",    '\u{3c9}'),
    ("omicron",  '\u{3bf}'),
    ("oplus",    '\u{2295}'),
    ("or",       '\u{2228}'),
    ("ordf",     '\u{aa}'),
    ("ordm",     '\u{ba}'),
    ("oslash",   '\u{f8}'),
    ("otilde",   '\u{f5}'),
    ("otimes",   '\u{2297}'),
    ("ouml",     '\u{f6}'),
    ("para",     '\u{b6}'),
    ("part",     '\u{2202}'),
    ("permil",   '\u{2030}'),
    ("perp",     '\u{22a5}'),
    ("phi",      '\u{3c6}'),
    ("pi",       '\u{3c0}'),
    ("piv",      '\u{3d6}'),
    ("plusmn",   '\u{b1}'),
    ("pound",    '\u{a3}'),
    ("prime",    '\u{2032}'),
    ("prod",     '\u{220f}'),
    ("prop",     '\u{221d}'),
    ("psi",      '\u{3c8}'),
    ("quot",     '"'),
    ("rArr",     '\u{21d2}'),
    ("radic",    '\u{221a}'),
    ("rang",     '\u{232a}'),
    ("raquo",    '\u{bb}'),
    ("rarr",     '\u{2192}'),
    ("rceil",    '\u{2309}'),
    ("rdquo",    '\u{201d}'),
    ("real",     '\u{211c}'),
    ("reg",      '\u{ae}'),
    ("rfloor",   '\u{230b}'),
    ("rho",      '\u{3c1}'),
    ("rlm",      '\u{200f}'),
    ("rsaquo",   '\u{203a}'),
    ("rsquo",    '\u{2019}'),
    ("sbquo",    '\u{201a}'),
    ("scaron",   '\u{161}'),
    ("sdot",     '\u{22c5}'),
    ("sect",     '\u{a7}'),
    ("shy",      '\u{ad}'),
    ("sigma",    '\u{3c3}'),
    ("sigmaf",   '\u{3c2}'),
    ("sim",      '\u{223c}'),
    ("spades",   '\u{2660}'),
    ("sub",      '\u{2282}'),
    ("sube",     '\u{2286}'),
    ("sum",      '\u{2211}'),
    ("sup",      '\u{2283}'),
    ("sup1",     '\u{b9}'),
    ("sup2",     '\u{b2}'),
    ("sup3",     '\u{b3}'),
    ("supe",     '\u{2287}'),
    ("szlig",    '\u{df}'),
    ("tau",      '\u{3c4}'),
    ("there4",   '\u{2234}'),
    ("theta",    '\u{3b8}'),
    ("thetasym", '\u{3d1}'),
    ("thinsp",   '\u{2009}'),
    ("thorn",    '\u{fe}'),
    ("tilde",    '\u{2dc}'),
    ("times",    '\u{d7}'),
    ("trade",    '\u{2122}'),
    ("uArr",     '\u{21d1}'),
    ("uacute",   '\u{fa}'),
    ("uarr",     '\u{2191}'),
    ("ucirc",    '\u{fb}'),
    ("ugrave",   '\u{f9}'),
    ("uml",      '\u{a8}'),
    ("upsih",    '\u{3d2}'),
    ("upsilon",  '\u{3c5}'),
    ("uuml",     '\u{fc}'),
    ("weierp",   '\u{2118}'),
    ("xi",       '\u{3be}'),
    ("yacute",   '\u{fd}'),
    ("yen",      '\u{a5}'),
    ("yuml",     '\u{ff}'),
    ("zeta",     '\u{3b6}'),
    ("zwj",      '\u{200d}'),
    ("zwnj",     '\u{200c}'),
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

/// Decode the character references a page's prose uses: every name HTML 4
/// defines (see `NAMED`), and any numeric one, decimal or hexadecimal.
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
    if let Ok(i) = NAMED.binary_search_by(|(n, _)| n.cmp(&name)) {
        return Some(NAMED[i].1.to_string());
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
    fn test_every_html4_name_is_decoded_and_case_matters() {
        assert_eq!(decode_entities("n&eacute;e &Eacute;mile &ccedil;a &deg;C &pound;5 &mu;m &acirc;&aacute;&ecirc;"),
            "née Émile ça °C £5 μm âáê");
        assert_eq!(decode_entities("&AMP; &EACUTE; &Amp;"), "& &EACUTE; &Amp;");
    }

    #[test]
    fn test_the_named_table_is_sorted_for_its_binary_search() {
        for w in NAMED.windows(2) {
            assert!(w[0].0 < w[1].0, "{} before {}", w[0].0, w[1].0);
        }
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
