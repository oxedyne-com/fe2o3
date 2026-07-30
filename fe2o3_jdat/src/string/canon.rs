//! Canonical JSON, for daticles a signature has to cover.
//!
//! A signature over JSON is a signature over bytes, so the signer and the verifier must agree on
//! the bytes exactly. [`Dat::json`] does not settle that: it puts a space after each colon and each
//! comma, which is readable and is not what a browser's `JSON.stringify` produces. Verifying a
//! browser's signature therefore needs a second, narrower encoding whose output is fixed.
//!
//! [`Dat::json_canonical`] is that encoding, following RFC 8785 (JSON Canonicalisation Scheme):
//!
//! - No whitespace anywhere outside string literals.
//! - Object members sorted by their keys' UTF-16 code units.
//! - Strings escaped as RFC 8785 §3.2.2.2 requires -- the two mandatory escapes, the five
//!   short escapes, `\u00xx` in lowercase hex for the remaining C0 controls, and every other
//!   character emitted as itself in UTF-8.
//!
//! What it refuses matters as much as what it writes, because a caller who cannot get the same
//! bytes from the other end should be told rather than handed bytes that only look canonical:
//!
//! - A float has no canonical form here. RFC 8785 §3.2.2.3 defers to ECMAScript's number
//!   serialisation, which this encoder does not implement; carry the value as a string instead.
//! - An integer beyond 2^53 - 1 is refused, because a JavaScript signer cannot hold it exactly and
//!   so cannot have signed the digits we would write.
//! - Bytes, tuples, vectors, user kinds and the rest of the daticle catalogue have no JSON form at
//!   all, and each is named in the error rather than silently coerced.
//!
//! Object keys must be strings, which JSON requires and JDAT does not.

use crate::{
    daticle::{
        Dat,
        Daticle,
    },
    map::DaticleMap,
};

use oxedyne_fe2o3_core::prelude::*;


/// The largest integer a JavaScript number holds exactly, `2^53 - 1`.
const JS_SAFE_INTEGER: i128 = 9_007_199_254_740_991;

impl Dat {
    /// Encode this daticle as canonical JSON, per RFC 8785.
    ///
    /// The output carries no whitespace outside string literals and orders object members by their
    /// keys, so two ends that agree on the value agree on the bytes. See the module documentation
    /// for the daticle kinds this refuses and why.
    pub fn json_canonical(&self) -> Outcome<String> {
        let mut out = String::new();
        res!(write_canonical(self, &mut out));
        Ok(out)
    }
}

/// Append the canonical JSON encoding of `dat` to `out`.
fn write_canonical(dat: &Dat, out: &mut String) -> Outcome<()> {
    match dat {
        Dat::Empty      => out.push_str("null"),
        Dat::Bool(b)    => out.push_str(if *b { "true" } else { "false" }),
        Dat::Str(s)     => write_string(s, out),
        Dat::Opt(boxopt) => match &**boxopt {
            None    => out.push_str("null"),
            Some(d) => res!(write_canonical(d, out)),
        },
        Dat::Box(d)         => res!(write_canonical(d, out)),
        Dat::U8(n)          => res!(write_integer(*n as i128, out)),
        Dat::U16(n)         => res!(write_integer(*n as i128, out)),
        Dat::U32(n)         => res!(write_integer(*n as i128, out)),
        Dat::U64(n)         => res!(write_integer(*n as i128, out)),
        Dat::U128(n)        => res!(write_integer(*n as i128, out)),
        Dat::I8(n)          => res!(write_integer(*n as i128, out)),
        Dat::I16(n)         => res!(write_integer(*n as i128, out)),
        Dat::I32(n)         => res!(write_integer(*n as i128, out)),
        Dat::I64(n)         => res!(write_integer(*n as i128, out)),
        Dat::I128(n)        => res!(write_integer(*n, out)),
        Dat::List(items)    => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                res!(write_canonical(item, out));
            }
            out.push(']');
        },
        Dat::Map(m)     => res!(write_map(m, out)),
        other => return Err(err!(
            "A daticle of kind {:?} has no canonical JSON form. If a signature must cover \
            it, carry it as a string.", other.kind();
            Invalid, Input, Unimplemented)),
    }
    Ok(())
}

/// Append an object, its members ordered by their keys' UTF-16 code units.
fn write_map(m: &DaticleMap, out: &mut String) -> Outcome<()> {
    let mut entries: Vec<(&String, &Dat)> = Vec::with_capacity(m.len());
    for (k, v) in m.iter() {
        match k {
            Dat::Str(s) => entries.push((s, v)),
            other => return Err(err!(
                "A JSON object key must be a string, and this one is of kind {:?}.",
                other.kind();
                Invalid, Input, Mismatch)),
        }
    }
    entries.sort_by(|(a, _), (b, _)| utf16_units(a).cmp(&utf16_units(b)));
    out.push('{');
    for (i, (k, v)) in entries.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        write_string(k, out);
        out.push(':');
        res!(write_canonical(v, out));
    }
    out.push('}');
    Ok(())
}

/// Append an integer, refusing one a JavaScript signer could not have held exactly.
fn write_integer(n: i128, out: &mut String) -> Outcome<()> {
    if n > JS_SAFE_INTEGER || n < -JS_SAFE_INTEGER {
        return Err(err!(
            "The integer {} is beyond 2^53 - 1, so a JavaScript signer cannot hold it \
            exactly and cannot have signed these digits. Carry it as a string.", n;
            Invalid, Input, TooBig));
    }
    out.push_str(&fmt!("{}", n));
    Ok(())
}

/// Append a JSON string literal, escaped as RFC 8785 §3.2.2.2 requires.
fn write_string(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"'         => out.push_str("\\\""),
            '\\'        => out.push_str("\\\\"),
            '\u{08}'    => out.push_str("\\b"),
            '\u{0c}'    => out.push_str("\\f"),
            '\n'        => out.push_str("\\n"),
            '\r'        => out.push_str("\\r"),
            '\t'        => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&fmt!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

/// The UTF-16 code units of a string, which is the order RFC 8785 §3.2.3 sorts keys by. It differs
/// from Rust's own string ordering only above the basic multilingual plane, where a surrogate pair
/// sorts below the unpaired code points that follow it.
fn utf16_units(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}


#[cfg(test)]
mod tests {
    use super::*;

    use crate::prelude::*;

    /// The members of RFC 8785 §3.2.3's ordering example, arriving in the order the RFC lists them
    /// (its repeated member dropped, since this crate's decoder refuses a duplicate key). The
    /// expected order is the one the RFC mandates -- ascending UTF-16 code units, so the vertical
    /// tab at U+000B sorts between the newline and the carriage return, and the digit sorts after
    /// all three -- which is not the order they were written in, nor the order a byte-wise sort of
    /// the escaped forms would give.
    #[test]
    fn test_json_canonical_rfc8785_ordering_00() -> Outcome<()> {
        let src = r#"{"\u20ac":"Euro Sign","\r":"Carriage Return","\u000a":"Newline","1":"One","\u0080":"Control","\u00f6":"Latin Small Letter O With Diaeresis","\u000b":"Vertical Tab"}"#;
        let dat = res!(Dat::decode_string(src));
        assert_eq!(
            res!(dat.json_canonical()),
            "{\"\\n\":\"Newline\",\"\\u000b\":\"Vertical Tab\",\"\\r\":\"Carriage Return\",\
            \"1\":\"One\",\"\u{80}\":\"Control\",\"\u{f6}\":\"Latin Small Letter O With \
            Diaeresis\",\"\u{20ac}\":\"Euro Sign\"}",
        );
        Ok(())
    }

    /// What a browser signs, read back and re-encoded, must be the bytes the browser signed. This
    /// is the shape a ceremony attestation takes: nested objects, a boolean, an empty string and a
    /// null.
    #[test]
    fn test_json_canonical_round_trips_a_signed_object_00() -> Outcome<()> {
        // As JSON.stringify emits it once the keys are sorted: no spaces, null for an absent value.
        let signed = "{\"age_band\":\"adult\",\"age_belief\":true,\"captures\":\
            {\"face\":\"aG91c2U\",\"hands\":\"simulated\"},\"evidence\":\"\",\
            \"place\":{\"cell\":null,\"how\":\"device\"}}";
        // Arriving with the members in a different order, as a client is free to send them.
        let arrived = "{\"place\":{\"how\":\"device\",\"cell\":null},\"evidence\":\"\",\
            \"captures\":{\"hands\":\"simulated\",\"face\":\"aG91c2U\"},\
            \"age_belief\":true,\"age_band\":\"adult\"}";
        let dat = res!(Dat::decode_string(arrived));
        assert_eq!(res!(dat.json_canonical()), signed);
        Ok(())
    }

    /// The five short escapes, the two mandatory ones, and a control character with no short form.
    #[test]
    fn test_json_canonical_escapes_00() -> Outcome<()> {
        let dat = dat!("a\"b\\c\nd\re\tf\u{08}g\u{0c}h\u{1f}i");
        assert_eq!(
            res!(dat.json_canonical()),
            "\"a\\\"b\\\\c\\nd\\re\\tf\\bg\\fh\\u001fi\"",
        );
        Ok(())
    }

    /// A character outside ASCII is written as itself, as `JSON.stringify` writes it. Escaping it
    /// would produce bytes no browser signed.
    #[test]
    fn test_json_canonical_leaves_non_ascii_literal_00() -> Outcome<()> {
        let dat = dat!("Cœur — 日本");
        assert_eq!(res!(dat.json_canonical()), "\"Cœur — 日本\"");
        Ok(())
    }

    /// Lists keep their order, which is data, unlike object member order, which is not.
    #[test]
    fn test_json_canonical_list_order_kept_00() -> Outcome<()> {
        let dat = listdat!["z", "a", 1u8, true, Dat::Empty];
        assert_eq!(res!(dat.json_canonical()), "[\"z\",\"a\",1,true,null]");
        Ok(())
    }

    /// A float is refused rather than written in a form the other end may not reproduce.
    #[test]
    fn test_json_canonical_refuses_a_float_00() -> Outcome<()> {
        assert!(dat!(1.5f64).json_canonical().is_err(),
            "a float has no canonical form here and must be refused");
        Ok(())
    }

    /// An integer a JavaScript number cannot hold exactly is refused: the digits we would write are
    /// not the digits the other end signed.
    #[test]
    fn test_json_canonical_refuses_an_unsafe_integer_00() -> Outcome<()> {
        assert!(dat!(9_007_199_254_740_991u64).json_canonical().is_ok(),
            "2^53 - 1 is exactly representable and must be accepted");
        assert!(dat!(9_007_199_254_740_992u64).json_canonical().is_err(),
            "2^53 must be refused");
        Ok(())
    }

    /// Bytes are not JSON, and the error must name the kind so the caller knows what to change.
    #[test]
    fn test_json_canonical_refuses_bytes_00() -> Outcome<()> {
        assert!(Dat::BU8(vec![1, 2, 3]).json_canonical().is_err(),
            "a byte string has no JSON form");
        Ok(())
    }

    /// A non-string object key is not JSON either.
    #[test]
    fn test_json_canonical_refuses_a_non_string_key_00() -> Outcome<()> {
        let mut m = DaticleMap::new();
        m.insert(dat!(1u8), dat!("one"));
        assert!(Dat::Map(m).json_canonical().is_err(),
            "an integer object key has no JSON form");
        Ok(())
    }
}
