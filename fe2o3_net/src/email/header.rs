//! Lifted out of `crate::dkim` on 2026-09-23, so the submission server reads a message's
//! header fields exactly as the signer that then signs them does.
//!
//! Made strict on 2026-09-24 (D-06 audit F4-1). The first reader split lines on LF alone, dropped
//! a line with no colon before unfolding, and let a colon clear the address before it, so
//! `From: ceo@x:hello@x;` and `From: ceo@x\r<hello@x>` passed the sender check as `hello@x`
//! while a receiver's parser read `ceo@x`. Whatever cannot be read as RFC 5322 reads it is now an
//! error rather than a guess, because the caller is deciding whose name a message goes out under.

use oxedyne_fe2o3_core::prelude::*;


/// A message split at its first blank line: the header block, without the blank line, and the
/// body, untouched. Only `\r\n\r\n` ends the header section. A message without one is all header
/// block, and [`header_fields`] then refuses the bare line ends it holds, rather than this
/// guessing at a second rule for where the header stops.
pub fn split_headers_body(message: &[u8]) -> (&[u8], &[u8]) {
    match find_subseq(message, b"\r\n\r\n") {
        Some(i) => (&message[..i], &message[i + 4..]),
        None    => (message, &[]),
    }
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

/// Is `c` a line end to some reader somewhere? A control character other than HTAB, or a Unicode
/// line or paragraph separator.
fn breaks_lines(c: char) -> bool {
    (c.is_control() && c != '\t') || c == '\u{2028}' || c == '\u{2029}'
}

/// `(name, unfolded value)` pairs, in the order they appeared, read as RFC 5322 §2.2 reads them.
///
/// Lines end at CRLF. A CR or an LF standing alone, any other control character, and bytes that
/// are not UTF-8 are refused rather than read one way here and another by the receiver. Folding
/// is undone before any line is judged, so a field's name cannot hide on a line of its own. An
/// empty line, a continuation with nothing before it to continue, and a line with no colon or
/// with a name that is not printable ASCII are all refused. A value keeps its folding WSP, which
/// DKIM's relaxed canonicalisation collapses later; the obsolete WSP before the colon (§4.5) is
/// no part of the name.
pub fn header_fields(headers: &[u8]) -> Outcome<Vec<(String, String)>> {
    let text = match std::str::from_utf8(headers) {
        Ok(t) => t,
        Err(e) => return Err(err!(
            "The header section is not UTF-8, from byte {}.", e.valid_up_to();
            Invalid, Input, Decode)),
    };
    // Physical lines, each ended by CRLF. A trailing CRLF leaves nothing after it.
    let mut lines: Vec<&str> = Vec::new();
    let mut start = 0usize;
    let mut chars = text.char_indices();
    while let Some((i, c)) = chars.next() {
        match c {
            '\r' => match chars.next() {
                Some((_, '\n')) => {
                    lines.push(&text[start..i]);
                    start = i + 2;
                },
                _ => return Err(err!(
                    "The header section holds a CR not followed by LF, at byte {}.", i;
                    Invalid, Input)),
            },
            '\n' => return Err(err!(
                "The header section holds an LF not preceded by CR, at byte {}.", i;
                Invalid, Input)),
            c if breaks_lines(c) => return Err(err!(
                "The header section holds the control character {:?}, at byte {}.", c, i;
                Invalid, Input)),
            _ => {},
        }
    }
    if start < text.len() {
        lines.push(&text[start..]);
    }
    // Unfolded fields: a line starting with WSP continues the one before it.
    let mut fields: Vec<String> = Vec::new();
    for (n, line) in lines.iter().enumerate() {
        if line.is_empty() {
            return Err(err!(
                "Header line {} is empty, which would end the header section there.", n + 1;
                Invalid, Input));
        }
        if line.starts_with(' ') || line.starts_with('\t') {
            match fields.last_mut() {
                Some(f) => f.push_str(line),
                None    => return Err(err!(
                    "Header line 1 is a continuation, with no field before it to continue.";
                    Invalid, Input)),
            }
        } else {
            fields.push(line.to_string());
        }
    }
    let mut out = Vec::with_capacity(fields.len());
    for (n, field) in fields.into_iter().enumerate() {
        let colon = match field.find(':') {
            Some(c) => c,
            None    => return Err(err!(
                "Header field {} has no colon, so it is not a field: {:?}.", n + 1, field;
                Invalid, Input, Missing)),
        };
        let name = field[..colon].trim_end_matches([' ', '\t']);
        if name.is_empty() || !name.bytes().all(|b| (33..=126).contains(&b)) {
            return Err(err!(
                "Header field {} has the name {:?}, which is not printable ASCII.", n + 1, name;
                Invalid, Input));
        }
        out.push((name.to_string(), field[colon + 1..].to_string()));
    }
    Ok(out)
}

/// The addresses in an address-list field such as `From`, `Sender` or `To`, each as its bare
/// addr-spec, in order, with comments and folding white space removed.
///
/// RFC 5322 §3.4, with the UTF-8 of RFC 6532: `a@b`, `Name <a@b>`, `"Last, First" <a@b>`,
/// comments in parentheses, groups (`Team: a@b, c@d;` and `undisclosed-recipients:;`), and the
/// obsolete empty members and dotted phrases. Anything else is an error rather than a guess --
/// a member with no address, text after an address, a quote, comment or bracket left open, an
/// obsolete route -- since the caller is deciding whose name a message goes out under.
pub fn addresses(value: &str) -> Outcome<Vec<String>> {
    if let Some(c) = value.chars().find(|c| breaks_lines(*c)) {
        return Err(err!(
            "An address list holds the control character {:?}: {:?}.", c, value;
            Invalid, Input));
    }
    let toks = res!(tokens(value));
    let mut r = Reader { toks: &toks, pos: 0, value };
    r.address_list()
}

/// One lexical piece of an address list; comments and white space are gone by now.
#[derive(Clone, Debug, Eq, PartialEq)]
enum Tok {
    Atom(String),       // a run of atext
    Quoted(String),     // a quoted-string, quotes and escapes kept as written
    Literal(String),    // a domain-literal, brackets kept
    Special(char),      // one of `< > : ; @ , .`
}

/// Is `c` RFC 5322 atext? Beyond ASCII, anything printable is, per RFC 6532.
fn is_atext(c: char) -> bool {
    c.is_ascii_alphanumeric()
        || "!#$%&'*+-/=?^_`{|}~".contains(c)
        || (!c.is_ascii() && !c.is_whitespace() && !breaks_lines(c))
}

fn tokens(value: &str) -> Outcome<Vec<Tok>> {
    let mut out = Vec::new();
    let mut chars = value.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            ' ' | '\t' => {},
            '(' => {
                // A comment, nested comments and quoted pairs included, is white space.
                let mut depth = 1usize;
                while depth > 0 {
                    match chars.next() {
                        Some('\\')  => { let _ = chars.next(); },
                        Some('(')   => depth += 1,
                        Some(')')   => depth -= 1,
                        Some(_)     => {},
                        None        => return Err(err!(
                            "An address list leaves a comment open: {:?}.", value;
                            Invalid, Input)),
                    }
                }
            },
            '"' => {
                let mut q = String::from('"');
                loop {
                    match chars.next() {
                        Some('\\') => match chars.next() {
                            Some(e) => {
                                q.push('\\');
                                q.push(e);
                            },
                            None => return Err(err!(
                                "An address list leaves a quoted string open: {:?}.", value;
                                Invalid, Input)),
                        },
                        Some('"') => {
                            q.push('"');
                            break;
                        },
                        Some(c) => q.push(c),
                        None    => return Err(err!(
                            "An address list leaves a quoted string open: {:?}.", value;
                            Invalid, Input)),
                    }
                }
                out.push(Tok::Quoted(q));
            },
            '[' => {
                let mut l = String::from('[');
                loop {
                    match chars.next() {
                        Some(']') => {
                            l.push(']');
                            break;
                        },
                        Some(c) if c == '[' || c == '\\' => return Err(err!(
                            "An address list's domain literal holds {:?}: {:?}.", c, value;
                            Invalid, Input)),
                        Some(c) => l.push(c),
                        None    => return Err(err!(
                            "An address list leaves a domain literal open: {:?}.", value;
                            Invalid, Input)),
                    }
                }
                out.push(Tok::Literal(l));
            },
            '<' | '>' | ':' | ';' | '@' | ',' | '.' => out.push(Tok::Special(c)),
            c if is_atext(c) => {
                let mut a = String::from(c);
                while let Some(&n) = chars.peek() {
                    if !is_atext(n) {
                        break;
                    }
                    a.push(n);
                    let _ = chars.next();
                }
                out.push(Tok::Atom(a));
            },
            c => return Err(err!(
                "An address list holds {:?}, which may not stand there: {:?}.", c, value;
                Invalid, Input)),
        }
    }
    Ok(out)
}

/// A recursive-descent reader of RFC 5322 §3.4 over [`Tok`]s.
struct Reader<'a> {
    toks:   &'a [Tok],
    pos:    usize,
    value:  &'a str,    // the field as written, for the errors
}

impl<'a> Reader<'a> {
    fn peek(&self) -> Option<&'a Tok> {
        self.toks.get(self.pos)
    }

    fn at(&self, c: char) -> bool {
        matches!(self.peek(), Some(Tok::Special(s)) if *s == c)
    }

    fn expect(&mut self, c: char, phase: &str) -> Outcome<()> {
        if self.at(c) {
            self.pos += 1;
            return Ok(());
        }
        Err(err!(
            "An address list wants {:?} {}, and has {:?} at token {}: {:?}.",
            c, phase, self.peek(), self.pos, self.value;
            Invalid, Input))
    }

    /// The first `<`, `:`, `@`, `,` or `;` from here on, which says what the next member is:
    /// an angle address, a group, a bare addr-spec, or no address at all.
    fn lookahead(&self) -> Option<char> {
        self.toks.iter().skip(self.pos).find_map(|t| match t {
            Tok::Special(c) if matches!(c, '<' | ':' | '@' | ',' | ';') => Some(*c),
            _ => None,
        })
    }

    fn address_list(&mut self) -> Outcome<Vec<String>> {
        let mut out = Vec::new();
        while self.peek().is_some() {
            if self.at(',') {
                // The obsolete empty member.
                self.pos += 1;
                continue;
            }
            match self.lookahead() {
                Some(':')   => out.extend(res!(self.group())),
                _           => out.push(res!(self.mailbox())),
            }
            if self.peek().is_some() {
                res!(self.expect(',', "after an address"));
            }
        }
        Ok(out)
    }

    /// `display-name ":" [group-list] ";"`, whose members are mailboxes and never groups.
    fn group(&mut self) -> Outcome<Vec<String>> {
        res!(self.phrase(true));
        res!(self.expect(':', "after a group's name"));
        let mut out = Vec::new();
        loop {
            if self.at(';') {
                self.pos += 1;
                return Ok(out);
            }
            if self.at(',') {
                self.pos += 1;
                continue;
            }
            if self.peek().is_none() {
                return Err(err!(
                    "An address list leaves a group without its closing ';': {:?}.", self.value;
                    Invalid, Input, Missing));
            }
            out.push(res!(self.mailbox()));
            if !self.at(';') {
                res!(self.expect(',', "after a group member"));
            }
        }
    }

    /// `name-addr / addr-spec`.
    fn mailbox(&mut self) -> Outcome<String> {
        match self.lookahead() {
            Some('<') => {
                res!(self.phrase(false));
                res!(self.expect('<', "before an address"));
                if self.at('@') {
                    return Err(err!(
                        "An address list holds an obsolete source route, which is not read: \
                        {:?}.", self.value;
                        Invalid, Input, Unimplemented));
                }
                let spec = res!(self.addr_spec());
                res!(self.expect('>', "after an address"));
                Ok(spec)
            },
            Some('@') => self.addr_spec(),
            _ => Err(err!(
                "An address list member at token {} holds no address: {:?}.", self.pos, self.value;
                Invalid, Input, Missing)),
        }
    }

    /// A display name: words, with the obsolete `.` after the first. A group's is required.
    fn phrase(&mut self, required: bool) -> Outcome<()> {
        let mut words = 0usize;
        loop {
            match self.peek() {
                Some(Tok::Atom(_)) | Some(Tok::Quoted(_))   => words += 1,
                Some(Tok::Special('.')) if words > 0        => {},
                _                                           => break,
            }
            self.pos += 1;
        }
        if required && words == 0 {
            return Err(err!(
                "An address list holds a group with no name: {:?}.", self.value;
                Invalid, Input, Missing));
        }
        Ok(())
    }

    /// `local-part "@" domain`, as one string.
    fn addr_spec(&mut self) -> Outcome<String> {
        let mut spec = res!(self.dotted(true, "local part"));
        res!(self.expect('@', "after a local part"));
        spec.push('@');
        match self.peek() {
            Some(Tok::Literal(l)) => {
                spec.push_str(l);
                self.pos += 1;
            },
            _ => spec.push_str(&res!(self.dotted(false, "domain"))),
        }
        Ok(spec)
    }

    /// `word *("." word)`, where a quoted word may stand in a local part but not in a domain.
    fn dotted(&mut self, quoted_ok: bool, what: &str) -> Outcome<String> {
        let mut s = String::new();
        loop {
            match self.peek() {
                Some(Tok::Atom(a))                  => s.push_str(a),
                Some(Tok::Quoted(q)) if quoted_ok   => s.push_str(q),
                other => return Err(err!(
                    "An address's {} wants a word, and has {:?} at token {}: {:?}.",
                    what, other, self.pos, self.value;
                    Invalid, Input)),
            }
            self.pos += 1;
            if !self.at('.') {
                return Ok(s);
            }
            self.pos += 1;
            s.push('.');
        }
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

    fn fields(head: &[u8]) -> Vec<(String, String)> {
        match header_fields(head) {
            Ok(f) => f,
            Err(e) => panic!("{:?} would not read: {}", String::from_utf8_lossy(head), e),
        }
    }

    #[test]
    fn the_forms_mail_clients_write_are_read_00() {
        assert_eq!(addrs(" hello@oxegen.io"), vec!["hello@oxegen.io"]);
        assert_eq!(addrs(" Jason Hoogland <hello@oxegen.io>"), vec!["hello@oxegen.io"]);
        assert_eq!(addrs(" \"Hoogland, Jason\" <hello@oxegen.io>"), vec!["hello@oxegen.io"],
            "a comma inside a quoted display name is not a separator");
        assert_eq!(addrs(" \"Hoogland <Jason>: CEO@x.test\" <hello@oxegen.io>"),
            vec!["hello@oxegen.io"], "nor is an angle bracket, colon or @ inside one");
        assert_eq!(addrs(" hello@oxegen.io (the owner)"), vec!["hello@oxegen.io"]);
        assert_eq!(addrs(" =?UTF-8?Q?Caf=C3=A9?= <news@oxegen.io>"), vec!["news@oxegen.io"]);
        assert_eq!(addrs(" Café Oxegen <news@oxegen.io>"), vec!["news@oxegen.io"],
            "RFC 6532 UTF-8 in a display name");
        assert_eq!(addrs(" a@x.test, \"B\" <b@y.test>"), vec!["a@x.test", "b@y.test"]);
        assert_eq!(addrs(" Team: a@x.test, B <b@y.test>;"), vec!["a@x.test", "b@y.test"]);
        assert_eq!(addrs(" undisclosed-recipients:;"), Vec::<String>::new());
        assert_eq!(addrs(" \"john doe\"@x.test"), vec!["\"john doe\"@x.test"],
            "a quoted local part is part of the address");
        assert_eq!(addrs(" Name (a comment, with a comma) <a@x.test>"), vec!["a@x.test"]);
        assert_eq!(addrs(" J. Hoogland <a@x.test>"), vec!["a@x.test"], "the obsolete dotted phrase");
        assert_eq!(addrs(" a@x.test,, b@y.test,"), vec!["a@x.test", "b@y.test"],
            "the obsolete empty members");
        assert_eq!(addrs(" a@[192.0.2.1]"), vec!["a@[192.0.2.1]"]);
    }

    /// A member with no address, text after an address, or a bracket left open, is refused
    /// rather than guessed at.
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
            // D-06 audit F4-1: a standard parser reads each of these as ceo@x.test.
            " ceo@x.test:hello@x.test;",
            " ceo@x.test <hello@x.test>",
            " ceo@x.test\r<hello@x.test>",
            " ceo@x.test\rX: hello@x.test",
            // A colon after an addr-spec does not make it a group's name.
            " ceo@x.test: hello@x.test;",
            " Team: a@x.test",          // a group must be closed
            " Outer: Inner: a@x.test;;",// and never nests
            " <@route.test:a@x.test>",  // the obsolete source route
            " hello@x.test;",
            " a@x.test\u{2028}",
        ] {
            assert!(addresses(bad).is_err(), "{:?} was read as {:?}", bad, addresses(bad));
        }
    }

    #[test]
    fn header_fields_unfold_and_keep_their_order_00() {
        let (head, body) = split_headers_body(
            b"From: A\r\n <a@x.test>\r\nSubject: hi\r\nFrom: b@y.test\r\n\r\nbody\r\n");
        let f = fields(head);
        assert_eq!(f.len(), 3);
        assert_eq!(f[0].0, "From");
        assert_eq!(f[0].1, " A <a@x.test>");
        assert_eq!(f[2], (fmt!("From"), fmt!(" b@y.test")));
        assert_eq!(body, b"body\r\n");
        let (head, body) = split_headers_body(b"Subject: no body");
        assert_eq!(head, b"Subject: no body");
        assert!(body.is_empty());
        assert_eq!(fields(b"From : a@x.test"), vec![(fmt!("From"), fmt!(" a@x.test"))],
            "the obsolete WSP before a colon is no part of the name");
    }

    /// Lines end at CRLF, and folding is undone before a line is judged, so a name cannot hide
    /// on a line of its own (D-06 audit F4-1).
    #[test]
    fn header_fields_read_lines_as_rfc_5322_does_00() {
        // Unfolded first, the field is `From : ceo@x.test`, so there are two `From`s.
        let f = fields(b"From\r\n : ceo@x.test\r\nFrom: hello@x.test");
        assert_eq!(f.iter().filter(|(n, _)| n == "From").count(), 2, "{:?}", f);
        for bad in [
            &b"From: ceo@x.test\rX: hello@x.test"[..],
            b"From: ceo@x.test\r<hello@x.test>",
            b"From: hello@x.test\nFrom: ceo@x.test",
            b"From: hello@x.test\r\nX: a\x00b",
            b"From: hello@x.test\r\nX: a\x0bb",
            b"\r\nFrom: hello@x.test",          // an empty first line ends the header there
            b" From: hello@x.test",             // a continuation of nothing
            b"From: hello@x.test\r\nNot a field",
            b"From: hello@x.test\r\nX Y: z",
            b"From: \xff@x.test",
        ] {
            assert!(header_fields(bad).is_err(), "{:?} was read as {:?}",
                String::from_utf8_lossy(bad), header_fields(bad));
        }
        // With no CRLF CRLF, the whole message is header, and its bare LFs are refused rather
        // than taken for the end of the header.
        let (head, body) = split_headers_body(b"From: hello@x.test\n\nX\r\nFrom: ceo@x.test\r\n");
        assert!(body.is_empty());
        assert!(header_fields(head).is_err());
    }
}
