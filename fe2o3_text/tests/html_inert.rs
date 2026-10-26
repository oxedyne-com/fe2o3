//! `html_inert`: a stranger's HTML names nothing remote until the reader asks for pictures.

use oxedyne_fe2o3_text::html_inert::{
    css_inert,
    inert_html,
    Remote,
};

const OFF: Remote = Remote::Blocked;
const ON:  Remote = Remote::Allowed;

/// Does the inert HTML still name the tracker anywhere a browser could fetch it from? Every
/// fetching position is removed, or (in CSS) renamed with `x-` so it no longer parses; what a
/// renamed function or `@x-import` rule holds is cut before looking.
fn names_host(h: &str) -> bool {
    let mut s = h.to_ascii_lowercase();
    for m in ["x-url(", "x--webkit-image-set(", "x-image-set(", "x-image(", "x-src(", "@x-import"] {
        while let Some(a) = s.find(m) {
            let stop = if m.starts_with('@') { ';' } else { ')' };
            let e = s[a..].find(stop).map(|k| a + k + 1).unwrap_or(s.len());
            s.replace_range(a..e, "");
        }
    }
    s.contains("evil.example")
}

#[test]
fn every_fetching_position_is_removed_with_pictures_off() {
    let cases = [
        r#"<img src="https://evil.example/p.gif">"#,
        r#"<img srcset="https://evil.example/p.gif 1x">"#,
        r#"<picture><source srcset="https://evil.example/p.webp"><img alt=x></picture>"#,
        r#"<link rel=preload as=image imagesrcset="https://evil.example/a 1x" href="https://evil.example/b">"#,
        r#"<table background="https://evil.example/bg"><td background=//evil.example/td>"#,
        r#"<body background="https://evil.example/body">"#,
        r#"<video poster="https://evil.example/p" src="https://evil.example/v"><track src="https://evil.example/t"></video>"#,
        r#"<audio src="https://evil.example/a"></audio><input type=image src="https://evil.example/i">"#,
        r#"<div style="background:url(https://evil.example/u)">"#,
        r#"<div style='background-image:url("https://evil.example/u")'>"#,
        r#"<div style="background:image-set('https://evil.example/s' 1x)">"#,
        r#"<div style="background:-webkit-image-set('https://evil.example/s' 1x)">"#,
        r#"<div style="list-style:image('https://evil.example/s')">"#,
        r#"<style>@import 'https://evil.example/i.css'; p{background:url(https://evil.example/c)}</style>"#,
        r#"<style>@font-face{font-family:x;src:url(https://evil.example/f.woff)}</style>"#,
        r#"<link rel=stylesheet href="https://evil.example/s.css">"#,
        r#"<meta http-equiv="refresh" content="0;url=https://evil.example/r">"#,
        r#"<iframe src="https://evil.example/f"></iframe><object data="https://evil.example/o"></object><embed src="https://evil.example/e">"#,
        r#"<svg><image href="https://evil.example/s"/><use xlink:href="https://evil.example/u#a"/></svg>"#,
        r#"<svg><rect fill="url(https://evil.example/g#p)"/></svg>"#,
        r#"<svg><image><set attributeName="href" to="https://evil.example/s"/></image></svg>"#,
        r#"<base href="https://evil.example/"><img src="a.gif">"#,
        r#"<script src="https://evil.example/s.js"></script>"#,
        r#"<img lowsrc="https://evil.example/l" dynsrc="https://evil.example/d">"#,
        r#"<a href="https://ok.test/" ping="https://evil.example/ping">link</a>"#,
        r#"<bgsound src="https://evil.example/b">"#,
    ];
    for c in cases {
        let h = inert_html(c, OFF);
        assert!(!names_host(&h), "still names the host: {} -> {}", c, h);
    }
}

#[test]
fn encodings_that_hide_a_url_are_seen_through() {
    let cases = [
        // Character references in a style attribute, which the browser decodes before CSS.
        r#"<div style="background:u&#114;l(https://evil.example/a)">"#,
        r#"<div style="background:url&lpar;https://evil.example/a)">"#,
        r#"<div style="background:url&#x28;https://evil.example/a)">"#,
        // CSS escapes.
        r#"<div style="background:\75 rl(https://evil.example/a)">"#,
        r#"<div style="background:\000075rl(https://evil.example/a)">"#,
        r#"<div style="background:u\rl(https://evil.example/a)">"#,
        r#"<style>p{background:U\52L(https://evil.example/a)}</style>"#,
        r#"<style>@\69mport "https://evil.example/i.css";</style>"#,
        // Case and quoting of the tag and attribute.
        r#"<IMG SRC=https://evil.example/a>"#,
        r#"<img/src="https://evil.example/a">"#,
        r#"<img src = 'https://evil.example/a'>"#,
    ];
    for c in cases {
        let h = inert_html(c, OFF);
        assert!(!names_host(&h), "{} -> {}", c, h);
    }
    // `\7\35 ` is U+0007 then `5`, not `\75` (`u`): the kept escape is closed by its space.
    assert!(css_inert(r"a:\7\35 rl(x)", OFF).starts_with(r"a:\7 5rl(x)"));
}

#[test]
fn a_parser_difference_cannot_hide_a_tag() {
    // A `<title>` and a `<textarea>` are text to a browser: the quote inside does not run on.
    let h = inert_html(r#"<title><a title='</title><img src=https://evil.example/t>'"#, OFF);
    assert!(!names_host(&h), "{}", h);
    let h = inert_html(r#"<textarea><a title='</textarea><img src=https://evil.example/t>'"#, OFF);
    assert!(!names_host(&h), "{}", h);
    // Inside `<svg>` a `<style>` is markup, so `<img>` in it breaks out: written as CSS text,
    // it cannot.
    let h = inert_html(r#"<svg><style><img src="https://evil.example/s"></style></svg>"#, ON);
    assert!(h.contains(r"\3c img"), "{}", h);
    assert!(!h.contains("<img"), "{}", h);
    // A comment and a bogus comment go, with whatever they hide.
    let h = inert_html(r#"<!--><img src=https://evil.example/c>--><p>x</p>"#, OFF);
    assert!(!names_host(&h), "{}", h);
    // A quote that never closes ends the input inside the tag, which is dropped.
    assert_eq!(inert_html(r#"<p>a</p><img src="https://evil.example/x"#, OFF), "<p>a</p>");
}

#[test]
fn the_message_own_pictures_and_links_stay() {
    let h = inert_html(r#"<img src="cid:logo@x" alt="Logo"><img src="data:image/png;base64,AAAA">"#, OFF);
    assert_eq!(h, r#"<img src="cid:logo@x" alt="Logo"><img src="data:image/png;base64,AAAA">"#);
    let h = inert_html(r#"<td style="background:url('cid:bg')">"#, OFF);
    assert_eq!(h, r#"<td style="background:url('cid:bg')">"#);
    let h = inert_html(r#"<a href="https://shop.test/?a=1&amp;b=2">Shop</a>"#, OFF);
    assert_eq!(h, r#"<a href="https://shop.test/?a=1&amp;b=2">Shop</a>"#);
    // Text and its references are kept as written; Outlook's `o:p` survives.
    let h = inert_html("<p class=x>caf&eacute; &copy; 1 &lt; 2<o:p></o:p></p>", OFF);
    assert_eq!(h, r#"<p class="x">caf&eacute; &copy; 1 &lt; 2<o:p></o:p></p>"#);
    let h = inert_html("<p style=\"color:red;font:12px Arial\">x</p>", OFF);
    assert_eq!(h, "<p style=\"color:red;font:12px Arial\">x</p>");
}

#[test]
fn with_pictures_allowed_images_load_and_active_content_still_goes() {
    let h = inert_html(r#"<img src="https://pics.test/a.png" srcset="https://pics.test/b.png 2x">"#, ON);
    assert_eq!(h, r#"<img src="https://pics.test/a.png" srcset="https://pics.test/b.png 2x">"#);
    let h = inert_html(r#"<div style="background:url(https://pics.test/bg.png)">"#, ON);
    assert!(h.contains("url(https://pics.test/bg.png)"), "{}", h);
    let h = inert_html(concat!(
        r#"<meta http-equiv=refresh content="0;url=https://evil.example/r">"#,
        r#"<script>x</script><iframe src="https://evil.example/f"></iframe>"#,
        r#"<link rel=stylesheet href="https://evil.example/s.css">"#,
        r#"<style>@import "https://evil.example/i.css";</style>"#,
        r#"<img src=https://pics.test/a.png onerror="x()">"#,
    ), ON);
    assert!(!names_host(&h), "{}", h);
    assert!(h.contains(r#"<img src="https://pics.test/a.png">"#), "{}", h);
}

#[test]
fn css_inert_renames_only_what_fetches() {
    assert_eq!(css_inert("a{color:rgb(1,2,3);width:calc(1px + 2%)}", OFF), "a{color:rgb(1,2,3);width:calc(1px + 2%)}");
    assert_eq!(css_inert("a{background:url(x.png)}", OFF), "a{background:x-url(x.png)}");
    assert_eq!(css_inert("a{background:url( data:image/gif;base64,R0)}", OFF), "a{background:url( data:image/gif;base64,R0)}");
    assert_eq!(css_inert("a{background:linear-gradient(red,blue)}", OFF), "a{background:linear-gradient(red,blue)}");
    assert_eq!(css_inert("@import 'x.css';", ON), "@x-import 'x.css';");
    assert_eq!(css_inert("a{background:url(x.png)}", ON), "a{background:url(x.png)}");
}
