//! RFC 2047 encoded-words, as the mail reader and IMAP SEARCH both decode them.

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_text::{
    base64,
    mime::{
        decode_qp,
        decode_words,
        entity,
        leaf_parts,
        param,
        to_utf8,
        Structure,
    },
};


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
    assert_eq!(decode_words("=?UTF-8?Q?Ren=C3=A9e_Dupont?= <renee@x.example>"),
        "Renée Dupont <renee@x.example>");
}

#[test]
fn joins_two_adjacent_words() {
    let a = base64::encode("Ma".as_bytes());
    let b = base64::encode("ría".as_bytes());
    let s = fmt!("=?utf-8?B?{}?= =?utf-8?B?{}?=", a, b);
    assert_eq!(decode_words(&s), "María");
}

#[test]
fn leaves_plain_text_and_malformed_words_alone() {
    assert_eq!(decode_words("Lunch on Friday"), "Lunch on Friday");
    assert_eq!(decode_words("=?utf-8?X?abc?= and =?broken"), "=?utf-8?X?abc?= and =?broken");
    // Ordinary text between two words keeps its spaces.
    assert_eq!(decode_words("=?utf-8?Q?a?= b =?utf-8?Q?c?="), "a b c");
}

#[test]
fn decodes_a_quoted_printable_body() {
    assert_eq!(decode_qp("caf=C3=A9 =\r\nau lait"), "café au lait".as_bytes());
}

#[test]
fn decodes_a_latin1_word() {
    assert_eq!(decode_words("=?ISO-8859-1?Q?Andr=E9?= <a@b>"), "André <a@b>");
    assert_eq!(decode_words("=?iso-8859-15*fr?Q?=A4?="), "\u{20AC}");
}

#[test]
fn transcodes_the_three_legacy_charsets() {
    assert_eq!(to_utf8(&[0x63, 0x61, 0x66, 0xE9], Some("ISO-8859-1")), "café");
    assert_eq!(to_utf8(&[0xA4, 0xBD], Some("iso-8859-15")), "\u{20AC}\u{0153}");
    assert_eq!(to_utf8(&[0x93, 0x78, 0x94, 0x80], Some("windows-1252")), "\u{201C}x\u{201D}\u{20AC}");
    assert_eq!(to_utf8("é".as_bytes(), Some("utf-8")), "é");
}

#[test]
fn walks_nested_multiparts_by_boundary() {
    let mut raw: Vec<u8> = b"Content-Type: multipart/mixed; boundary=\"A\"\r\n\r\npre\r\n\
        --A\r\nContent-Type: multipart/alternative; boundary=B\r\n\r\n\
        --B\r\nContent-Type: text/plain; charset=iso-8859-1\r\n\
        Content-Transfer-Encoding: 8bit\r\n\r\ncaf".to_vec();
    raw.push(0xE9);
    raw.extend_from_slice(b"\r\n--B\r\nContent-Type: text/html\r\n\r\n<p>x</p>\r\n--B--\r\n\
        --A\r\nContent-Type: application/pdf; name=\"r.pdf\"\r\n\
        Content-Transfer-Encoding: base64\r\n\r\nJVBERi0=\r\n--A--\r\nepilogue\r\n");
    let parts = leaf_parts(&raw);
    assert_eq!(parts.len(), 3);
    assert_eq!(parts[0].text(), "café");
    assert!(parts[0].is_text() && !parts[0].is_attachment());
    assert_eq!(parts[1].media_type(), "text/html");
    assert_eq!(parts[2].filename().as_deref(), Some("r.pdf"));
    assert_eq!(parts[2].text(), "%PDF-");
}

#[test]
fn param_honours_quoted_strings() {
    // RFC 2045 §5.1: a quoted value may hold `;` and `=`, and a backslash quotes the next
    // character.
    assert_eq!(param("attachment; filename=\"a;b.pdf\"", "filename").as_deref(), Some("a;b.pdf"));
    assert_eq!(param("attachment; filename=\"say \\\"hi\\\".txt\"; size=3", "filename").as_deref(),
        Some("say \"hi\".txt"));
    assert_eq!(param("text/plain; name=\"x=1;y\"; charset=utf-8", "charset").as_deref(), Some("utf-8"));
    assert_eq!(param("multipart/mixed; boundary = \"SP\"", "boundary").as_deref(), Some("SP"));
    assert_eq!(param("attachment; filename=\"open", "filename").as_deref(), Some("open"));
    assert_eq!(param("attachment; filename=\"\"", "filename"), None);
}

#[test]
fn param_reads_rfc_2231_values_and_continuations() {
    assert_eq!(param("attachment; filename*=UTF-8''r%C3%A9sum%C3%A9.pdf", "filename").as_deref(),
        Some("résumé.pdf"));
    assert_eq!(param("attachment; filename*=iso-8859-1'fr'caf%E9.txt", "filename").as_deref(),
        Some("café.txt"));
    assert_eq!(param("attachment; filename*1=\"name.pdf\"; filename*0=\"long \"", "filename").as_deref(),
        Some("long name.pdf"));
    assert_eq!(param("attachment;\r\n filename*0*=UTF-8''r%C3%A9; filename*1*=sum%C3%A9; filename*2=.pdf",
        "filename").as_deref(), Some("résumé.pdf"));
    // The extended value is preferred to the plain one a sender adds for old readers.
    assert_eq!(param("attachment; filename=\"resume.pdf\"; filename*=UTF-8''r%C3%A9sum%C3%A9.pdf",
        "filename").as_deref(), Some("résumé.pdf"));
}

#[test]
fn an_attached_message_is_one_part_of_its_parent() {
    let raw = b"Content-Type: multipart/mixed; boundary=M\r\n\r\n\
        --M\r\nContent-Type: text/plain\r\n\r\nsee below\r\n\
        --M\r\nContent-Type: message/rfc822\r\n\r\n\
        Subject: inner one\r\nContent-Type: text/plain\r\n\r\ninner text\r\n--M--\r\n";
    let top = entity(raw);
    let kids = match &top.structure {
        Structure::Multipart(k) => k,
        other                   => panic!("not a multipart: {:?}", other),
    };
    assert_eq!(kids.len(), 2);
    assert!(kids[1].is_attachment());
    assert_eq!(kids[1].attachment_name(), "inner one");
    match &kids[1].structure {
        Structure::Message(inner)   => assert_eq!(inner.text(), "inner text"),
        other                       => panic!("not a message: {:?}", other),
    }
    // The reader shows only the outer text; SEARCH still reads the inner one.
    let shown: Vec<String> = top.display_parts().iter().map(|p| p.text()).collect();
    assert_eq!(shown, vec!["see below".to_string()]);
    let leaves: Vec<String> = leaf_parts(raw).iter().map(|p| p.text()).collect();
    assert_eq!(leaves, vec!["see below".to_string(), "inner text".to_string()]);
}
