//! RFC 2047 encoded-words, as the mail reader and IMAP SEARCH both decode them.

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_text::{
    base64,
    mime::{
        decode_qp,
        decode_words,
        leaf_parts,
        to_utf8,
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
