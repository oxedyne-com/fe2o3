//! Lifted out of `crate::dkim` on 2026-09-23, so the submission server reads a message's
//! header fields exactly as the signer that then signs them does.

use oxedyne_fe2o3_core::prelude::*;


/// A message split at its first blank line: the header block, without the blank line, and the
/// body, untouched. `\r\n\r\n` is looked for first and `\n\n` tolerated; a message with no blank
/// line is all header block.
pub fn split_headers_body(message: &[u8]) -> (&[u8], &[u8]) {
    if let Some(i) = find_subseq(message, b"\r\n\r\n") {
        return (&message[..i], &message[i + 4..]);
    }
    if let Some(i) = find_subseq(message, b"\n\n") {
        return (&message[..i], &message[i + 2..]);
    }
    (message, &[])
}

fn find_subseq(hay: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || needle.len() > hay.len() { return None; }
    for i in 0..=hay.len() - needle.len() {
        if &hay[i..i + needle.len()] == needle {
            return Some(i);
        }
    }
    None
}

/// `(name, unfolded value)` pairs, in the order they appeared. A continuation line, one starting
/// with WSP, is joined onto the previous value with its leading WSP kept, which DKIM's relaxed
/// canonicalisation collapses later. A line with no colon is not a field and is passed over.
pub fn header_fields(headers: &[u8]) -> Vec<(String, String)> {
    let text = String::from_utf8_lossy(headers);
    let mut out: Vec<(String, String)> = Vec::new();
    let mut name = String::new();
    let mut value = String::new();
    let mut have_current = false;
    for line in text.split('\n') {
        // Strip a trailing CR if present.
        let line = line.strip_suffix('\r').unwrap_or(line);
        if line.is_empty() {
            continue;
        }
        if line.starts_with(' ') || line.starts_with('\t') {
            // Continuation.
            if have_current {
                value.push_str(line);
            }
            continue;
        }
        // Flush previous.
        if have_current {
            out.push((std::mem::take(&mut name), std::mem::take(&mut value)));
        }
        if let Some(i) = line.find(':') {
            name = line[..i].trim().to_string();
            value = line[i + 1..].to_string();
            have_current = true;
        } else {
            have_current = false;
        }
    }
    if have_current {
        out.push((name, value));
    }
    out
}

/// The addresses in an address-list field such as `From`, `Sender` or `To`, each as its bare
/// addr-spec, in order and as written.
///
/// RFC 5322 §3.4, in the forms mail clients write: `a@b`, `Name <a@b>`, `"Last, First" <a@b>`,
/// comments in parentheses, and groups, `Team: a@b, c@d;` and `undisclosed-recipients:;`. A
/// member with no address in it, or a quote, comment or angle bracket left open, is an error
/// rather than a guess: the caller is deciding whose name a message goes out under.
pub fn addresses(value: &str) -> Outcome<Vec<String>> {
    let mut out: Vec<String> = Vec::new();
    let mut cur = String::new();            // the member so far, comments removed
    let mut angle: Option<String> = None;   // the member's `<...>`, once one opens
    let mut in_angle = false;
    let mut in_quote = false;
    let mut depth = 0usize;                 // comment nesting
    let mut escaped = false;
    for c in value.chars() {
        if depth > 0 {
            // A comment is dropped whole, escapes and nested comments included.
            if escaped {
                escaped = false;
            } else {
                match c {
                    '\\'    => escaped = true,
                    '('     => depth += 1,
                    ')'     => depth -= 1,
                    _       => {},
                }
            }
            continue;
        }
        let buf = match (&mut angle, in_angle) {
            (Some(a), true) => a,
            _               => &mut cur,
        };
        if in_quote {
            // A quoted string is kept as written, quotes and escapes included, since a quoted
            // local part is part of the address.
            buf.push(c);
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_quote = false;
            }
            continue;
        }
        match c {
            '('                 => depth = 1,
            '"'                 => { in_quote = true; buf.push(c); },
            '<' if !in_angle    => {
                if angle.is_some() {
                    return Err(err!(
                        "An address-list member has two angle-bracketed addresses: {:?}.", value;
                        Invalid, Input));
                }
                angle = Some(String::new());
                in_angle = true;
            },
            '>' if in_angle     => in_angle = false,
            // A group's display name ends at its colon and names nobody.
            ':' if !in_angle    => cur.clear(),
            ',' | ';' if !in_angle => {
                if let Some(a) = res!(member_address(&cur, angle.take(), value)) {
                    out.push(a);
                }
                cur.clear();
            },
            _                   => buf.push(c),
        }
    }
    if in_angle || in_quote || depth > 0 {
        return Err(err!(
            "An address-list leaves a quote, comment or angle bracket open: {:?}.", value;
            Invalid, Input));
    }
    if let Some(a) = res!(member_address(&cur, angle.take(), value)) {
        out.push(a);
    }
    Ok(out)
}

/// One member's addr-spec: the angle-bracketed address where there is one, and otherwise the
/// member itself. `None` for an empty member, such as the gap in an empty group.
fn member_address(
    cur:    &str,
    angle:  Option<String>,
    value:  &str,
)
    -> Outcome<Option<String>>
{
    let spec = match angle {
        Some(a) => a.trim().to_string(),
        None => {
            let t = cur.trim();
            if t.is_empty() {
                return Ok(None);
            }
            t.to_string()
        },
    };
    match spec.rfind('@') {
        Some(i) if i > 0 && i + 1 < spec.len() => Ok(Some(spec)),
        _ => Err(err!(
            "An address-list member {:?} holds no address, in {:?}.", spec, value;
            Invalid, Input, Missing)),
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    fn addrs(v: &str) -> Vec<String> {
        match addresses(v) {
            Ok(a) => a,
            Err(e) => panic!("{:?} would not read: {}", v, e),
        }
    }

    #[test]
    fn the_forms_mail_clients_write_are_read_00() {
        assert_eq!(addrs(" hello@oxegen.io"), vec!["hello@oxegen.io"]);
        assert_eq!(addrs(" Jason Hoogland <hello@oxegen.io>"), vec!["hello@oxegen.io"]);
        assert_eq!(addrs(" \"Hoogland, Jason\" <hello@oxegen.io>"), vec!["hello@oxegen.io"],
            "a comma inside a quoted display name is not a separator");
        assert_eq!(addrs(" hello@oxegen.io (the owner)"), vec!["hello@oxegen.io"]);
        assert_eq!(addrs(" =?UTF-8?Q?Caf=C3=A9?= <news@oxegen.io>"), vec!["news@oxegen.io"]);
        assert_eq!(addrs(" a@x.test, \"B\" <b@y.test>"), vec!["a@x.test", "b@y.test"]);
        assert_eq!(addrs(" Team: a@x.test, b@y.test;"), vec!["a@x.test", "b@y.test"]);
        assert_eq!(addrs(" undisclosed-recipients:;"), Vec::<String>::new());
        assert_eq!(addrs(" \"john doe\"@x.test"), vec!["\"john doe\"@x.test"],
            "a quoted local part is part of the address");
        assert_eq!(addrs(" Name (a comment, with a comma) <a@x.test>"), vec!["a@x.test"]);
    }

    /// A member with no address, or a bracket left open, is refused rather than guessed at.
    #[test]
    fn what_cannot_be_read_is_refused_00() {
        for bad in [
            " Jason Hoogland",
            " <>",
            " Doe, John <j@x.test>",    // an unquoted comma splits the name from its address
            " Name <a@x.test",
            " \"open quote <a@x.test>",
            " (open comment a@x.test",
            " <a@x.test> <b@y.test>",
            " @x.test",
            " a@",
        ] {
            assert!(addresses(bad).is_err(), "{:?} was read as {:?}", bad, addresses(bad));
        }
    }

    #[test]
    fn header_fields_unfold_and_keep_their_order_00() {
        let (head, body) = split_headers_body(
            b"From: A\r\n <a@x.test>\r\nSubject: hi\r\nFrom: b@y.test\r\n\r\nbody\r\n");
        let fields = header_fields(head);
        assert_eq!(fields.len(), 3);
        assert_eq!(fields[0].0, "From");
        assert_eq!(fields[0].1, " A <a@x.test>");
        assert_eq!(fields[2], (fmt!("From"), fmt!(" b@y.test")));
        assert_eq!(body, b"body\r\n");
        let (head, body) = split_headers_body(b"Subject: no body");
        assert_eq!(head, b"Subject: no body");
        assert!(body.is_empty());
    }
}
