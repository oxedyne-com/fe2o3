//! The displayed body and the attachment list follow the MIME structure (RFC 2046): plain over
//! HTML only within a `multipart/alternative`, the parts of a `mixed` in order, and an attached
//! message one attachment of its parent, never the source of this message's text. The cases
//! are QA F2b (b)'s regressions R1 to R5, out of the shared corpus.

mod mime_corpus;

use crate::mime_corpus::corpus;

use oxedyne_fe2o3_mail::message::ParsedMessage;


fn case(name: &str) -> ParsedMessage {
    let raw = corpus().into_iter()
        .find(|(n, _)| n.starts_with(name))
        .map(|(_, r)| r)
        .unwrap_or_default();
    assert!(!raw.is_empty(), "no corpus case {}", name);
    ParsedMessage::parse(&raw)
}

const FOOTER: &str = "_______________\nlist mailing list\nhttps://lists.x/listinfo";

#[test]
fn an_attached_message_is_one_attachment() {
    // R1: a message/rfc822 part is listed under its file name, also when base64-encoded.
    let m = case("07 ");
    assert_eq!(m.body, "see below");
    assert_eq!(m.attachments, vec!["fwd.eml".to_string()]);
    let m = case("25 ");
    assert_eq!(m.body, "the plain body");
    assert_eq!(m.attachments, vec!["m.eml".to_string()]);
}

#[test]
fn text_never_comes_from_inside_an_attached_message() {
    // R2: the outer text is HTML only, or its text/plain is empty.
    let m = case("08 ");
    assert_eq!(m.body, "outer html note");
    assert_eq!(m.attachments, vec!["fwd.eml".to_string()]);
    let m = case("09 ");
    assert_eq!(m.body, "the html body");
    assert_eq!(m.attachments, vec!["fwd.eml".to_string()]);
}

#[test]
fn a_list_footer_follows_the_html_body() {
    // R3: mixed > [related > [html, img], text/plain footer].
    let m = case("19 ");
    assert_eq!(m.body, format!("the html body\n\n{}", FOOTER));
    assert_eq!(m.attachments, vec!["image001.png".to_string()]);
    let m = case("19b");
    assert_eq!(m.body, format!("the plain body\n\n{}", FOOTER));
    let m = case("19c");
    assert_eq!(m.body, format!("the html body\n\n{}", FOOTER));
}

#[test]
fn a_quoted_file_name_keeps_its_semicolon() {
    // R4.
    assert_eq!(case("20 ").attachments, vec!["a;b.pdf".to_string()]);
}

#[test]
fn an_rfc_2231_file_name_is_decoded() {
    assert_eq!(case("21 ").attachments, vec!["résumé.pdf".to_string()]);
}

#[test]
fn a_bounce_lists_the_returned_message_not_its_attachments() {
    // R5: the bounced message is one attachment, named by its subject; its report.pdf is not
    // the bounce's own.
    let m = case("23 ");
    assert_eq!(m.body, "Delivery failed");
    assert_eq!(m.attachments, vec!["Café test".to_string()]);
}

#[test]
fn alternative_prefers_plain_among_its_own_parts() {
    assert_eq!(case("02 ").body, "the plain body");
    assert_eq!(case("05 ").body, "the plain body");
    assert_eq!(case("05 ").attachments, vec!["image001.png".to_string(), "report.pdf".to_string()]);
    assert_eq!(case("06 ").body, "the plain body");
    assert_eq!(case("30 ").body, "the plain body");
    assert_eq!(case("29 ").body, "the plain body");
}

#[test]
fn mixed_parts_read_in_order() {
    // Apple Mail's inline image between two HTML parts, reached when there is no plain.
    let raw = mime_corpus::msg("", &mime_corpus::mp("multipart/mixed", "APM", &[
        mime_corpus::leaf("text/html", "", "<p>before</p>"),
        mime_corpus::leaf("image/png; name=\"i.png\"", "Content-Disposition: inline; filename=\"i.png\"\r\n", "x"),
        mime_corpus::leaf("text/html", "", "<p>after</p>"),
    ]));
    let m = ParsedMessage::parse(raw.as_bytes());
    assert_eq!(m.body, "before\n\nafter");
    assert_eq!(m.attachments, vec!["i.png".to_string()]);
    // An attached text file is not the body.
    assert_eq!(case("24 ").body, "the html body");
    assert_eq!(case("24 ").attachments, vec!["log.txt".to_string()]);
}
