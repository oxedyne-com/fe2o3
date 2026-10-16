//! Percent-encoding, the escape a URI uses for everything it cannot spell.
//!
//! Both directions are byte-wise rather than character-wise, because what is escaped need not be
//! text: a data URL carries arbitrary bytes, and a query parameter may carry UTF-8 that the
//! encoder must not have opinions about.
//!
//! [`encode_component`] escapes what JavaScript's `encodeURIComponent` escapes, and nothing else.
//! That is a deliberate match: a value a browser wrote and a peer reads has to survive the round
//! trip byte for byte, and the browser's rule is the one that is not ours to choose.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use oxedyne_fe2o3_core::prelude::*;


/// Decode percent escapes, over bytes.
///
/// A `%` must be followed by two hexadecimal digits; anything else is an error rather than a
/// character passed through, since a caller that cannot tell an escape from a literal percent
/// cannot tell what it decoded.
pub fn decode(s: &str) -> Outcome<Vec<u8>> {
    let src = s.as_bytes();
    let mut out = Vec::with_capacity(src.len());
    let mut i = 0;
    while i < src.len() {
        match src[i] {
            b'%' => {
                if i + 2 >= src.len() {
                    return Err(err!(
                        "A percent escape must be followed by two hexadecimal digits.";
                        Invalid, Input, Decode));
                }
                let hex = match std::str::from_utf8(&src[i + 1..i + 3]) {
                    Ok(h)   => h,
                    Err(_)  => return Err(err!(
                        "A percent escape is not text."; Invalid, Input, Decode)),
                };
                match u8::from_str_radix(hex, 16) {
                    Ok(b)   => out.push(b),
                    Err(e)  => return Err(err!(e,
                        "'{}' is not a pair of hexadecimal digits.", hex;
                        Invalid, Input, Decode)),
                }
                i += 3;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    Ok(out)
}

/// Decode percent escapes and require the result to be UTF-8 text.
pub fn decode_str(s: &str) -> Outcome<String> {
    let byts = res!(decode(s));
    match String::from_utf8(byts) {
        Ok(s)   => Ok(s),
        Err(e)  => Err(err!(e,
            "The percent-decoded bytes are not UTF-8 text.";
            Invalid, Input, Decode, String)),
    }
}

/// Decode an `application/x-www-form-urlencoded` body into its fields, in the order sent.
///
/// A `+` is a space, as an HTML form writes one, and the rest is [`decode_str`]: a broken escape or
/// a field that is not UTF-8 is an error naming the field. A field with no `=` has an empty value,
/// and an empty pair (`a=1&&b=2`) is skipped. A key may repeat; the caller decides what that means.
pub fn form(body: &[u8]) -> Outcome<Vec<(String, String)>> {
    let s = match std::str::from_utf8(body) {
        Ok(s)   => s,
        Err(e)  => return Err(err!(e,
            "A form body must be ASCII, its other bytes percent-escaped."; Invalid, Input, Decode)),
    };
    let mut out = Vec::new();
    for (i, pair) in s.split('&').enumerate() {
        if pair.is_empty() {
            continue;
        }
        let (k, v) = match pair.find('=') {
            Some(at)    => (&pair[..at], &pair[at + 1..]),
            None        => (pair, ""),
        };
        let key = match decode_str(&k.replace('+', " ")) {
            Ok(k)   => k,
            Err(e)  => return Err(err!(e,
                "Decoding the name of form field {}.", i; Invalid, Input, Decode)),
        };
        let val = match decode_str(&v.replace('+', " ")) {
            Ok(v)   => v,
            Err(e)  => return Err(err!(e,
                "Decoding the value of form field {} ('{}').", i, key; Invalid, Input, Decode)),
        };
        out.push((key, val));
    }
    Ok(out)
}

/// Percent-encode one component of a URI, escaping exactly what `encodeURIComponent` escapes.
///
/// The unreserved set left alone is `A-Z a-z 0-9 - _ . ! ~ * ' ( )`. Everything else, including
/// `/`, `:`, `?`, `&`, `=` and every byte of a multi-byte character, is escaped with upper-case
/// hexadecimal digits, as the browsers emit them.
pub fn encode_component(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9'
            | b'-' | b'_' | b'.' | b'!' | b'~' | b'*' | b'\'' | b'(' | b')' => {
                out.push(*b as char)
            },
            other => out.push_str(&fmt!("%{:02X}", other)),
        }
    }
    out
}


#[cfg(test)]
mod tests {
    use super::*;

    /// The escapes `encodeURIComponent` makes, and the ones it does not. The expected strings are
    /// what a browser console prints, which is the whole reason this function exists.
    #[test]
    fn test_encode_component_matches_the_browser_00() {
        assert_eq!(encode_component("Funky Bear"), "Funky%20Bear");
        assert_eq!(encode_component("a/b:c?d&e=f"), "a%2Fb%3Ac%3Fd%26e%3Df");
        assert_eq!(encode_component("-_.!~*'()"), "-_.!~*'()");
        assert_eq!(encode_component("Cœur"), "C%C5%93ur");
        assert_eq!(encode_component(""), "");
    }

    /// What was encoded decodes back to what went in, including the bytes of a character no ASCII
    /// escape could carry.
    #[test]
    fn test_percent_round_trip_00() -> Outcome<()> {
        for original in ["Funky Bear", "Tree Hugger", "a/b:c", "Cœur — 日本", ""] {
            let encoded = encode_component(original);
            assert_eq!(res!(decode_str(&encoded)), original);
        }
        Ok(())
    }

    /// A truncated or malformed escape is an error, not a percent sign passed through: a caller
    /// that cannot tell the two apart does not know what it decoded.
    #[test]
    fn test_decode_rejects_a_broken_escape_00() {
        assert!(decode("%2").is_err(), "a truncated escape must be refused");
        assert!(decode("%").is_err(), "a bare percent must be refused");
        assert!(decode("%zz").is_err(), "non-hexadecimal digits must be refused");
    }

    /// Bytes that are not text decode as bytes, and are only refused when a caller asked for text.
    #[test]
    fn test_decode_bytes_that_are_not_text_00() -> Outcome<()> {
        assert_eq!(res!(decode("%FF%FE")), vec![0xFF, 0xFE]);
        assert!(decode_str("%FF%FE").is_err(), "those bytes are not UTF-8");
        Ok(())
    }
    /// A form body decodes field by field, `+` as a space and `%2B` as a plus, in the order sent.
    #[test]
    fn test_form_fields_00() -> Outcome<()> {
        let f = res!(form(b"summary=Fixed+a+typo&text=1%2B1+%3D+2%0A%5B%5BHachik%C5%8D%5D%5D&&flag&summary=again"));
        assert_eq!(f, vec![
            ("summary".to_string(),	"Fixed a typo".to_string()),
            ("text".to_string(),	"1+1 = 2\n[[Hachikō]]".to_string()),
            ("flag".to_string(),	String::new()),
            ("summary".to_string(),	"again".to_string()),
        ]);
        assert!(form(b"a=%2").is_err(), "a broken escape must be refused");
        assert!(form(b"a=%FF").is_err(), "a value that is not UTF-8 must be refused");
        assert_eq!(res!(form(b"")), Vec::new());
        Ok(())
    }
}
