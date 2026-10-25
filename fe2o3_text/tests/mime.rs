//! RFC 2047 encoded-words, as the mail reader and IMAP SEARCH both decode them.

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_text::{
    base64,
    mime::{
        decode_qp,
        decode_words,
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
