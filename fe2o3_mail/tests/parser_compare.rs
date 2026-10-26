//! Every corpus message through the reader as it stood before the `fe2o3_text::mime` walker
//! (d81121d5) and the reader now. Each field that differs must be listed in `FIXED`, with the
//! case it belongs to: an unlisted difference is a regression, a listed one that no longer
//! differs is a stale entry.

mod mime_corpus;

use crate::mime_corpus::{
    corpus,
    old_message,
};

use oxedyne_fe2o3_mail::message::ParsedMessage;


const FIXED: &[(&str, &str)] = &[
    // An inline image under `mixed` is listed, as the old reader listed it under `related`.
    ("05 outlook mixed>alt>related+pdf",            "attach"),
    // Latin-1, Windows-1252 and 8-bit bodies transcoded, not replaced by U+FFFD.
    ("10 qp latin1",                                "body"),
    ("12 cp1252",                                   "body"),
    ("33 8bit latin1",                              "body"),
    // The parts of a `mixed` in order: the body and the list footer after it.
    ("19 mailman footer after related",             "body"),
    ("19 mailman footer after related",             "attach"),
    ("19b mailman footer after alt",                "body"),
    ("19c mailman footer after html",               "body"),
    ("34 plain part with name only (no disposition)", "body"),
    // RFC 2231 file name decoded.
    ("21 rfc2231",                                  "attach"),
    // The bounced message is the bounce's one attachment, named by its Subject.
    ("23 bounce report",                            "attach"),
    // An attached text file is not the body.
    ("24 txt attachment + html",                    "body"),
    // `boundary = "SP"` read; a delimiter only at the start of a line.
    ("28 boundary = spaced",                        "body"),
    ("35 boundary mid-line",                        "body"),
];

#[test]
fn only_the_listed_fields_differ_from_the_old_reader() {
    let mut found = Vec::new();
    for (name, raw) in corpus() {
        let o = old_message::ParsedMessage::parse(&raw);
        let n = ParsedMessage::parse(&raw);
        let fields = [
            ("from",    o.from.clone(),                     n.from.clone()),
            ("subject", o.subject.clone(),                  n.subject.clone()),
            ("date",    o.date.clone(),                     n.date.clone()),
            ("msgid",   o.message_id.clone(),               n.message_id.clone()),
            ("body",    o.body.clone(),                     n.body.clone()),
            ("attach",  format!("{:?}", o.attachments),     format!("{:?}", n.attachments)),
        ];
        for (f, a, b) in fields {
            if a != b {
                eprintln!("{} {}\n  OLD {:?}\n  NEW {:?}", name, f, a, b);
                found.push((name, f));
            }
        }
    }
    eprintln!("corpus {} messages, {} field differences", corpus().len(), found.len());
    let unlisted: Vec<_> = found.iter().filter(|d| !FIXED.contains(d)).collect();
    let stale: Vec<_> = FIXED.iter().filter(|d| !found.contains(d)).collect();
    assert!(unlisted.is_empty(), "differences not listed as fixes: {:?}", unlisted);
    assert!(stale.is_empty(), "listed fixes that no longer differ: {:?}", stale);
}
