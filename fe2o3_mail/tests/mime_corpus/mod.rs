//! The 36-message MIME corpus of QA F2b (b), 2026-10-26: the shapes real clients send
//! (Outlook, Apple Mail, Mailman, bounces, forwards as attachments) and the malformed ones a
//! mailbox still holds. Shared by `message_structure` and `parser_compare`.
#![allow(dead_code)]

pub mod old_message;

pub fn mp(ct: &str, b: &str, parts: &[String]) -> String {
    let mut s = fmt(ct, b);
    s.push_str("This is a multi-part message in MIME format.\r\n\r\n");
    for p in parts { s.push_str(&format!("--{}\r\n{}\r\n", b, p)); }
    s.push_str(&format!("--{}--\r\n", b));
    s
}
pub fn fmt(ct: &str, b: &str) -> String { format!("Content-Type: {}; boundary=\"{}\"\r\n\r\n", ct, b) }
pub fn leaf(ct: &str, extra: &str, body: &str) -> String { format!("Content-Type: {}\r\n{}\r\n{}", ct, extra, body) }
pub fn msg(hdr: &str, rest: &str) -> String {
    format!("From: Bob <bob@x.org>\r\nTo: a@b\r\nSubject: =?ISO-8859-1?Q?Caf=E9?= test\r\n{}{}", hdr, rest)
}

pub fn corpus() -> Vec<(&'static str, Vec<u8>)> {
    let plain = leaf("text/plain; charset=utf-8", "", "the plain body");
    let html = leaf("text/html; charset=utf-8", "", "<p>the html body</p>");
    let pdf = leaf("application/pdf; name=\"report.pdf\"", "Content-Transfer-Encoding: base64\r\nContent-Disposition: attachment; filename=\"report.pdf\"\r\n", "JVBERi0xLjQK");
    let img = leaf("image/png; name=\"image001.png\"", "Content-Transfer-Encoding: base64\r\nContent-ID: <i1>\r\nContent-Disposition: inline; filename=\"image001.png\"\r\n", "iVBORw0KGgo=");
    let alt = mp("multipart/alternative", "ALT", &[plain.clone(), html.clone()]);
    let inner_msg = msg("Message-ID: <in@x>\r\nMIME-Version: 1.0\r\n", &mp("multipart/alternative", "IN", &[leaf("text/plain", "", "INNER forwarded text"), leaf("text/html", "", "<p>INNER html</p>")]));
    let rfc822 = format!("Content-Type: message/rfc822; name=\"fwd.eml\"\r\nContent-Disposition: attachment; filename=\"fwd.eml\"\r\n\r\n{}", inner_msg);
    let mut v: Vec<(&str, String)> = Vec::new();
    v.push(("01 plain", msg("Content-Type: text/plain; charset=utf-8\r\n\r\n", "hello\r\nworld\r\n")));
    v.push(("02 alt", msg("", &alt)));
    v.push(("03 html only", msg("Content-Type: text/html\r\n\r\n", "<html><body><p>only html</p></body></html>")));
    v.push(("04 mixed alt+pdf", msg("", &mp("multipart/mixed", "MIX", &[alt.clone(), pdf.clone()]))));
    let related = mp("multipart/related", "REL", &[html.clone(), img.clone()]);
    let ol_alt = mp("multipart/alternative", "OALT", &[plain.clone(), related.clone()]);
    v.push(("05 outlook mixed>alt>related+pdf", msg("", &mp("multipart/mixed", "OMIX", &[ol_alt.clone(), pdf.clone()]))));
    v.push(("06 outlook related>alt,img", msg("", &mp("multipart/related", "OREL", &[alt.clone(), img.clone()]))));
    v.push(("07 fwd-as-attachment plain", msg("", &mp("multipart/mixed", "F1", &[leaf("text/plain", "", "see below"), rfc822.clone()]))));
    let empty_alt = mp("multipart/alternative", "EALT", &[leaf("text/plain", "", "\r\n"), leaf("text/html", "", "<p>outer html note</p>")]);
    v.push(("08 fwd empty-plain outer", msg("", &mp("multipart/mixed", "F2", &[empty_alt, rfc822.clone()]))));
    v.push(("09 fwd html-only outer", msg("", &mp("multipart/mixed", "F3", &[html.clone(), rfc822.clone()]))));
    v.push(("10 qp latin1", msg("Content-Type: text/plain; charset=iso-8859-1\r\nContent-Transfer-Encoding: quoted-printable\r\n\r\n", "Gr=FC=DFe aus K=F6ln, caf=E9=\r\n soft\r\n")));
    v.push(("11 b64 utf8", msg("Content-Type: text/plain; charset=UTF-8\r\nContent-Transfer-Encoding: base64\r\n\r\n", "R3LDvMOfZSDwn5iA\r\n")));
    let mut v12 = msg("Content-Type: text/plain; charset=windows-1252\r\n\r\n", "").into_bytes();
    v12.extend_from_slice(b"\x93smart\x94 \x96 dash \x80 euro\r\n");
    let mut v13 = msg("Content-Type: text/plain; charset=koi8-r\r\nContent-Transfer-Encoding: 8bit\r\n\r\n", "").into_bytes();
    v13.extend_from_slice(b"\xf0\xd2\xc9\xd7\xc5\xd4 ascii-tail\r\n");
    let v14 = msg("Content-Type: text/plain; charset=iso-8859-1\r\n\r\n", "mislabelled café\r\n");
    v.push(("15 multipart no boundary", msg("Content-Type: multipart/mixed\r\n\r\n", "--X\r\nContent-Type: text/plain\r\n\r\nbody\r\n--X--\r\n")));
    v.push(("16 unclosed boundary", msg("", &mp("multipart/mixed", "U", &[plain.clone()]).replace("--U--\r\n", ""))));
    v.push(("17 no blank line", "From: a@b\r\nSubject: hi\r\n".to_string()));
    v.push(("18 bare LF", msg("", &alt).replace("\r\n", "\n")));
    let footer = leaf("text/plain; charset=us-ascii", "Content-Disposition: inline\r\n", "_______________\r\nlist mailing list\r\nhttps://lists.x/listinfo");
    v.push(("19 mailman footer after related", msg("", &mp("multipart/mixed", "MM", &[related.clone(), footer.clone()]))));
    v.push(("19b mailman footer after alt", msg("", &mp("multipart/mixed", "MM", &[alt.clone(), footer.clone()]))));
    v.push(("19c mailman footer after html", msg("", &mp("multipart/mixed", "MM", &[html.clone(), footer.clone()]))));
    v.push(("20 filename with ;", msg("", &mp("multipart/mixed", "S", &[plain.clone(), leaf("application/pdf", "Content-Disposition: attachment; filename=\"a;b.pdf\"\r\n", "x")]))));
    v.push(("21 rfc2231", msg("", &mp("multipart/mixed", "S", &[plain.clone(), leaf("application/pdf", "Content-Disposition: attachment;\r\n filename*=UTF-8''r%C3%A9sum%C3%A9.pdf\r\n", "x")]))));
    v.push(("22 encoded-word filename", msg("", &mp("multipart/mixed", "S", &[plain.clone(), leaf("application/pdf; name=\"=?UTF-8?B?csOpc3Vtw6kucGRm?=\"", "Content-Disposition: attachment\r\n", "x")]))));
    v.push(("23 bounce report", msg("", &mp("multipart/report; report-type=delivery-status", "R", &[leaf("text/plain", "", "Delivery failed"), leaf("message/delivery-status", "", "Reporting-MTA: dns; x\r\n"), format!("Content-Type: message/rfc822\r\n\r\n{}", msg("", &mp("multipart/mixed", "BM", &[leaf("text/plain", "", "orig"), pdf.clone()])))]))));
    v.push(("24 txt attachment + html", msg("", &mp("multipart/mixed", "T", &[html.clone(), leaf("text/plain; name=\"log.txt\"", "Content-Disposition: attachment; filename=\"log.txt\"\r\n", "LOG CONTENT")]))));
    v.push(("25 b64 message/rfc822", msg("", &mp("multipart/mixed", "B", &[plain.clone(), format!("Content-Type: message/rfc822\r\nContent-Transfer-Encoding: base64\r\nContent-Disposition: attachment; filename=\"m.eml\"\r\n\r\nRnJvbTogYUBiDQpTdWJqZWN0OiBpbm5lcg0KDQppbm5lciBib2R5DQo=\r\n")]))));
    let mut deep = plain.clone();
    for i in 0..10 { deep = mp("multipart/mixed", &format!("D{}", i), &[deep]); }
    v.push(("26 deep 10", msg("", &deep)));
    v.push(("28 boundary = spaced", msg("Content-Type: Multipart/Mixed; boundary = \"SP\"\r\n\r\n", "--SP\r\nContent-Type: text/plain\r\n\r\nspaced body\r\n--SP--\r\n")));
    v.push(("29 calendar", msg("", &mp("multipart/alternative", "C", &[plain.clone(), html.clone(), leaf("text/calendar; method=REQUEST; name=\"invite.ics\"", "", "BEGIN:VCALENDAR")]))));
    v.push(("30 apple inline", msg("", &mp("multipart/alternative", "AP", &[plain.clone(), mp("multipart/mixed", "APM", &[html.clone(), img.clone(), leaf("text/html", "", "<p>after</p>")])]))));
    v.push(("31 signed", msg("", &mp("multipart/signed; protocol=\"application/pkcs7-signature\"", "SG", &[alt.clone(), leaf("application/pkcs7-signature; name=\"smime.p7s\"", "Content-Disposition: attachment; filename=\"smime.p7s\"\r\n", "MIAG")]))));
    v.push(("32 folded headers", "From: =?UTF-8?Q?J=C3=BCrgen?=\r\n <j@x>\r\nSubject: long\r\n\tsubject\r\nContent-Type: text/plain\r\n\r\nb\r\n".to_string()));
    let mut v33 = msg("Content-Type: text/plain; charset=\"ISO-8859-1\"\r\n\r\n", "").into_bytes();
    v33.extend_from_slice(b"8bit caf\xe9\r\n");
    v.push(("34 plain part with name only (no disposition)", msg("", &mp("multipart/mixed", "N", &[leaf("text/plain; name=\"body.txt\"", "", "named body text"), html.clone()]))));
    v.push(("35 boundary mid-line", msg("", &mp("multipart/mixed", "Q", &[leaf("text/plain", "", "text with --Q inside line"), pdf.clone()]))));
    let mut out: Vec<(&str, Vec<u8>)> = v.into_iter().map(|(n, s)| (n, s.into_bytes())).collect();
    out.push(("12 cp1252", v12));
    out.push(("13 koi8-r", v13));
    out.push(("14 mislabelled", v14.into_bytes()));
    out.push(("33 8bit latin1", v33));
    out
}
