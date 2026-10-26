//! `MessageView`, the reader a viewer and the page share, and `DraftMessage` with attachments
//! read back through it (C1a, D-20261026-01).

mod mime_corpus;

use crate::mime_corpus::corpus;

use oxedyne_fe2o3_mail::message::{
    DraftAttachment,
    DraftMessage,
    MessageView,
    ParsedMessage,
    Remote,
};


const LOGO: &[u8] = b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR-not-a-real-png";

fn related_with_logo() -> Vec<u8> {
    let b64 = oxedyne_fe2o3_text::base64::encode(LOGO);
    format!("From: Shop <shop@x.example>\r\nSubject: Logo\r\nContent-Type: multipart/related; boundary=\"R\"\r\n\r\n\
        --R\r\nContent-Type: multipart/alternative; boundary=\"A\"\r\n\r\n\
        --A\r\nContent-Type: text/plain\r\n\r\nplain version\r\n\
        --A\r\nContent-Type: text/html\r\n\r\n<p>Hi <img src=\"cid:Logo.1@x\"></p>\r\n--A--\r\n\
        --R\r\nContent-Type: image/png; name=\"logo.png\"\r\nContent-Transfer-Encoding: base64\r\n\
        Content-ID: <logo.1@x>\r\nContent-Disposition: inline\r\n\r\n{}\r\n--R--\r\n", b64).into_bytes()
}

#[test]
fn an_embedded_logo_is_a_part_the_html_names_by_cid() {
    let v = MessageView::parse(&related_with_logo());
    assert_eq!(v.text, "plain version");
    assert!(v.html.contains("cid:Logo.1@x"), "{:?}", v.html);
    assert_eq!(v.parts.len(), 1, "{:?}", v.parts);
    let p = &v.parts[0];
    assert_eq!((p.name.as_str(), p.media_type.as_str(), p.content_id.as_str(), p.inline),
        ("logo.png", "image/png", "logo.1@x", true));
    assert_eq!(p.bytes, LOGO);
}

#[test]
fn a_part_no_html_names_is_not_inline() {
    let raw = b"Content-Type: multipart/mixed; boundary=\"M\"\r\n\r\n--M\r\nContent-Type: text/plain\r\n\r\nbody\r\n\
        --M\r\nContent-Type: image/png\r\nContent-ID: <a@b>\r\n\r\nxx\r\n--M--\r\n";
    let v = MessageView::parse(raw);
    assert_eq!(v.html, "");
    assert_eq!(v.parts.len(), 1);
    assert!(!v.parts[0].inline);
}

#[test]
fn a_plain_footer_beside_an_html_body_is_escaped_into_the_html() {
    let raw = b"Content-Type: multipart/mixed; boundary=\"M\"\r\n\r\n--M\r\nContent-Type: text/html\r\n\r\n<b>body</b>\r\n\
        --M\r\nContent-Type: text/plain\r\n\r\nlist <footer>\r\n--M--\r\n";
    let v = MessageView::parse(raw);
    assert!(v.html.starts_with("<b>body</b>"), "{:?}", v.html);
    assert!(v.html.contains("list &lt;footer&gt;"), "{:?}", v.html);
    assert_eq!(v.text, "body\n\nlist <footer>");
}

/// The view's text is the agent's text, and its parts are the agent's attachments, for every
/// message in the corpus.
#[test]
fn the_view_agrees_with_the_agents_reader_on_the_corpus() {
    for (name, raw) in corpus() {
        let v = MessageView::parse(&raw);
        let m = ParsedMessage::parse(&raw);
        assert_eq!(v.text, m.body, "{}", name);
        let listed: Vec<String> = v.parts.iter().map(|p| p.name.clone()).collect();
        for a in &m.attachments {
            assert!(listed.contains(a), "{}: {:?} not in {:?}", name, a, listed);
        }
    }
}

#[test]
fn a_draft_with_attachments_round_trips_through_the_reader() {
    let d = DraftMessage {
        from:        String::from("me@x.example"),
        to:          vec![String::from("you@y.example")],
        subject:     String::from("Files"),
        body:        String::from("See attached.\n"),
        date:        String::from("Mon, 1 Sep 2026 12:00:00 +0000"),
        message_id:  String::from("<f@x.example>"),
        attachments: vec![
            DraftAttachment { name: String::from("report \"q3\".pdf"), media_type: String::from("application/pdf"),
                bytes: (0..=255u8).cycle().take(1000).collect() },
            DraftAttachment { name: String::from("Grüße.txt"), media_type: String::new(),
                bytes: b"hallo".to_vec() },
        ],
        ..Default::default()
    };
    let raw = d.build().expect("builds");
    assert_eq!(raw, d.build().expect("builds again"), "the same draft builds the same bytes");
    assert!(String::from_utf8_lossy(&raw).lines().all(|l| l.len() <= 78), "a line over 78");
    let v = MessageView::parse(&raw);
    assert_eq!(v.subject, "Files");
    assert_eq!(v.text, "See attached.");
    assert_eq!(v.parts.len(), 2, "{:?}", v.parts);
    assert_eq!(v.parts[0].name, "report \"q3\".pdf");
    assert_eq!(v.parts[0].media_type, "application/pdf");
    assert_eq!(v.parts[0].bytes, d.attachments[0].bytes);
    assert_eq!(v.parts[1].name, "Grüße.txt");
    assert_eq!(v.parts[1].media_type, "application/octet-stream");
    assert_eq!(v.parts[1].bytes, b"hallo");
}

#[test]
fn a_tracking_pixel_is_gone_until_pictures_are_allowed() {
    let raw = b"From: Shop <shop@x.example>\r\nSubject: Sale\r\nContent-Type: text/html\r\n\r\n\
        <p>Sale</p><img src=\"https://track.example/p.gif\" srcset=\"https://track.example/p2.gif 2x\">\
        <picture><source srcset=\"https://track.example/p.webp\"></picture>\r\n";
    let off = MessageView::parse(raw);
    assert_eq!(off, MessageView::read(raw, Remote::Blocked));
    assert!(!off.html.contains("track.example"), "{:?}", off.html);
    assert!(off.html.starts_with("<p>Sale</p><img>"), "{:?}", off.html);
    let on = MessageView::read(raw, Remote::Allowed);
    assert!(on.html.contains(r#"<img src="https://track.example/p.gif" srcset="https://track.example/p2.gif 2x">"#), "{:?}", on.html);
    assert_eq!(on.text, off.text);
}
