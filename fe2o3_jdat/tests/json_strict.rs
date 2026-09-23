//! `Dat::decode_json_strict` against `serde_json`, an RFC 8259 reader that did not come from here:
//! every document it accepts and refuses by hand, and a fuzzed corpus where the two must agree
//! on what is JSON and on what it says.

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_jdat::{
    prelude::*,
    bdat::DecodeLimits,
    string::dec::DecoderConfig,
};

use serde_json::Value;


fn strict(s: &str) -> Outcome<Dat> {
    Dat::decode_json_strict(s, &DecodeLimits::text())
}

/// Does `d` say what `v` says? Numbers are compared as integers where both are integers, and as
/// the nearest `f64` otherwise, since `serde_json` reads a fraction or an exponent as one.
fn same(d: &Dat, v: &Value) -> bool {
    match (d, v) {
        (Dat::Opt(o),   Value::Null)        => o.is_none(),
        (Dat::Bool(a),  Value::Bool(b))     => a == b,
        (Dat::Str(a),   Value::String(b))   => a == b,
        (Dat::List(a),  Value::Array(b))    =>
            a.len() == b.len() && a.iter().zip(b.iter()).all(|(x, y)| same(x, y)),
        (Dat::Map(a),   Value::Object(b))   =>
            a.len() == b.len() && b.iter().all(|(k, y)| match a.get(&Dat::Str(k.clone())) {
                Some(x) => same(x, y),
                None    => false,
            }),
        (_,             Value::Number(n))   => same_number(d, n),
        _ => false,
    }
}

fn integer(d: &Dat) -> Option<i128> {
    match d {
        Dat::U8(n)      => Some(*n as i128),
        Dat::U16(n)     => Some(*n as i128),
        Dat::U32(n)     => Some(*n as i128),
        Dat::U64(n)     => Some(*n as i128),
        Dat::I8(n)      => Some(*n as i128),
        Dat::I16(n)     => Some(*n as i128),
        Dat::I32(n)     => Some(*n as i128),
        Dat::I64(n)     => Some(*n as i128),
        Dat::I128(n)    => Some(*n),
        _               => None,
    }
}

fn same_number(d: &Dat, n: &serde_json::Number) -> bool {
    if let (Some(a), Some(b)) = (integer(d), n.as_i64()) {
        return a == b as i128;
    }
    if let (Some(a), Some(b)) = (integer(d), n.as_u64()) {
        return a == b as i128;
    }
    let ours = match d {
        Dat::Adec(x)    => x.to_string().parse::<f64>().ok(),
        Dat::U128(x)    => Some(*x as f64),
        Dat::Aint(x)    => x.to_string().parse::<f64>().ok(),
        other           => integer(other).map(|i| i as f64),
    };
    // Within a few units in the last place, since serde_json's default float reading is fast
    // rather than exactly rounded.
    match (ours, n.as_f64()) {
        (Some(a), Some(b)) => a == b || (a - b).abs() <= 4.0 * f64::EPSILON * a.abs().max(b.abs()),
        _ => false,
    }
}

/// JSON documents, each read the same way by `serde_json`, by this reader and by the JDAT text
/// decoder, since valid JSON is a subset of what that decoder reads.
#[test]
fn test_rfc_8259_documents_are_read_as_json_readers_read_them() -> Outcome<()> {
    let docs = [
        "{}", "[]", "0", "-0", "1", "-1", "255", "256", "-128", "-129", "65535", "65536",
        "4294967296", "9007199254740991", "18446744073709551615", "18446744073709551616",
        "-9223372036854775808", "-9223372036854775809", "0.5", "-0.5", "1.25", "1e2", "1E2",
        "1e+2", "1e-2", "-1.5E-3", "123.456e7", "0e0", "0.0", "true", "false", "null",
        "\"\"", "\"plain\"", "\"\\\"\\\\\\/\\b\\f\\n\\r\\t\"", "\"\\u0041\\u00e9\\u20AC\"",
        "\"\\ud83d\\ude00\"", "\"\\u0000\"", "\"caf\u{e9} \u{1f600}\"", "\"\u{7f}\"",
        "[1,2,3]", " [ 1 , 2 ] ", "\t\n\r[\n1\n]\n", "{\"a\":1}", "{ \"a\" : [ true , null ] }",
        "{\"a\":{\"b\":{\"c\":[[],{}]}}}", "[\"\",0,-0,1e0,{\"\":null}]",
        "{\"v\":\"present/1\",\"ts\":1800000000,\"predicates\":[\"adult\"],\"proof\":{\"alg\":\"linkring/1\"}}",
    ];
    let jdat = DecoderConfig::<(), ()>::json(None);
    for doc in docs {
        let ours = match strict(doc) {
            Ok(d) => d,
            Err(e) => return Err(err!(e, "The JSON document {:?} was refused.", doc; Test)),
        };
        let theirs: Value = match serde_json::from_str(doc) {
            Ok(v) => v,
            Err(e) => return Err(err!("serde_json refused {:?}: {}", doc, e; Test)),
        };
        req!(same(&ours, &theirs), true, "{:?} read as {:?}, serde_json as {:?}", doc, ours, theirs);
        let text = res!(Dat::decode_string_with_config(doc, &jdat));
        req!(ours, text, "{:?} against the JDAT text decoder", doc);
    }
    Ok(())
}

/// What is not JSON is refused, and `serde_json` refuses each too, so every refusal here is of
/// something that is not JSON rather than of something this reader cannot read.
#[test]
fn test_what_is_not_json_is_refused() {
    let bad = [
        // The forms JDAT reads and JSON does not.
        "(u64|5)", "{\"ts\":(u64|5)}", "(str|\"x\")", "0x10", "0o7", "0b1", "abc", "'abc'",
        "{a:1}", "[1 2]", "1 2", "[1,]", "{\"a\":1,}", "[,]", "[,1]", "1_000", "empty", "none",
        "{\"a\":1} x", "{\"a\":1}{\"b\":2}", "!comment! 1", "# 1",
        // Numbers.
        "-", "+1", "01", "-01", "00", "1.", ".5", "-.5", "1.e2", "1e", "1e+", "1e-", "--1", "0x",
        "NaN", "Infinity", "-Infinity", "1.5.5", "1e2e3",
        // Strings.
        "\"abc", "\"\\x41\"", "\"\\u12\"", "\"\\u12G4\"", "\"\\'\"", "\"a\u{1}b\"", "\"a\nb\"",
        "\"a\tb\"", "\"\\ud800\"", "\"\\udc00\"", "\"\\ud800\\u0041\"", "\"\\ud800x\"",
        // Structure.
        "", " ", "[", "]", "{", "}", "[1", "{\"a\"", "{\"a\":", "{\"a\" 1}", "{\"a\",1}",
        "{1:1}", "[1:2]", "{\"a\":1 \"b\":2}", "tru", "nul", "fals", "True", "NULL",
        "\u{feff}{}", "\u{a0}1", "[1]\u{0}",
    ];
    for doc in bad {
        assert!(strict(doc).is_err(), "{:?} is not JSON and was read", doc);
        assert!(serde_json::from_str::<Value>(doc).is_err(),
            "serde_json reads {:?}, so it does not belong on this list", doc);
    }
}

/// Where RFC 8259 leaves the reader a choice, this one refuses: a member named twice, which
/// `serde_json` resolves by keeping the last.
#[test]
fn test_a_member_named_twice_is_refused() {
    assert!(strict("{\"a\":1,\"a\":2}").is_err(), "a member named twice");
    assert!(strict("{\"a\":1,\"b\":{\"a\":1,\"a\":1}}").is_err(), "a member named twice, nested");
    assert!(strict("{\"a\":{\"a\":1}}").is_ok(), "the same name at two levels is two members");
}

/// Fuzzed documents: small edits to JSON seeds, each read by both readers, which must agree on
/// whether it is JSON and, where it is, on what it says. The two known differences are skipped:
/// a member named twice, which this reader refuses by choice, and a number beyond the range of
/// an `f64`, which `serde_json` refuses and this reader holds exactly.
#[test]
fn test_a_fuzzed_corpus_agrees_with_an_independent_reader() -> Outcome<()> {
    let seeds = [
        "{\"v\":\"present/1\",\"mode\":\"named\",\"ts\":1800000000,\"predicates\":[\"adult\"],\
        \"proof\":{\"alg\":\"linkring/1\",\"body\":\"AAEC\"},\"prev\":null,\"live\":true}",
        "[0,-1,2.5,-3e4,5E-6,\"\\u00e9\\n\\\"\",[],{},[{\"a\":[1,[2,[3]]]}],false,null]",
        "{\"s\":\"caf\u{e9} \\ud83d\\ude00 \\/ \\\\ \\t\",\"n\":-0.0,\"big\":18446744073709551616}",
    ];
    let alphabet: Vec<char> = "{}[],:\"\\/bfnrtu019-+.eEx a\t\n'(|)_\u{1}\u{e9}".chars().collect();
    let mut state: u64 = 0x2545_f491_4f6c_dd1d;
    let mut next = |n: usize| -> usize {
        state = state.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
        ((state >> 33) as usize) % n
    };
    let (mut read, mut refused) = (0usize, 0usize);
    for seed in seeds {
        for _ in 0..4_000 {
            let mut doc: Vec<char> = seed.chars().collect();
            for _ in 0..(1 + next(3)) {
                let at = next(doc.len() + 1);
                match next(3) {
                    0 => doc.insert(at, alphabet[next(alphabet.len())]),
                    1 if at < doc.len() => { doc.remove(at); },
                    _ if at < doc.len() => doc[at] = alphabet[next(alphabet.len())],
                    _ => doc.push(alphabet[next(alphabet.len())]),
                }
            }
            let doc: String = doc.into_iter().collect();
            let ours = strict(&doc);
            let theirs = serde_json::from_str::<Value>(&doc);
            match (&ours, &theirs) {
                (Ok(d), Ok(v)) => {
                    read += 1;
                    req!(same(d, v), true, "{:?} read as {:?}, serde_json as {:?}", doc, d, v);
                },
                (Err(_), Err(_)) => refused += 1,
                (Err(e), Ok(_)) if fmt!("{}", e).contains("a second time") => refused += 1,
                (Ok(_), Err(e)) if fmt!("{}", e).contains("out of range") => read += 1,
                (Ok(d), Err(e)) => return Err(err!(
                    "{:?} was read as {:?}, and serde_json refuses it: {}", doc, d, e; Test)),
                (Err(e), Ok(v)) => return Err(err!(
                    "{:?} was refused, and serde_json reads it as {:?}: {}", doc, v, e; Test)),
            }
        }
    }
    // Both verdicts must be common, or the corpus proves little.
    req!(read > 1_000 && refused > 1_000, true, "{} read and {} refused", read, refused);
    Ok(())
}
