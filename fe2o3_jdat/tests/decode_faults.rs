//! `Dat::decode_string` on text that is wrong. It was lenient where leniency hid a fault, found
//! while building the Oxegen settings reader (D-20261005-12): a map left open returned its inner
//! map, text after the value was ignored, and digits parted by a space read as one number. Each
//! of those is an error now that names the line and the column, and no key or value. What the
//! format allows by design (unquoted words, single quotes, kind annotations, trailing commas, comments that
//! run to the end of the line) is checked to stay.

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_jdat::{
    prelude::*,
    bdat::DecodeLimits,
    string::dec::DecoderConfig,
    usr::{
        UsrKindId,
        UsrKinds,
    },
};
use oxedyne_fe2o3_num::float::Float64;

use std::collections::BTreeMap;


fn refused(s: &str, words: &[&str]) {
    refused_without(s, words, &[]);
}

// As `refused`, and the refusal must hold none of `absent`, a value or a key of `s`.
fn refused_without(s: &str, words: &[&str], absent: &[&str]) {
    match Dat::decode_string(s) {
        Ok(d) => panic!("{:?} should be refused, and became {}", s, d),
        Err(e) => {
            let msg = e.plain();
            for w in words {
                assert!(msg.contains(w), "the refusal of {:?} should hold {:?}, and reads: {}",
                    s, w, msg);
            }
            for w in absent {
                assert!(!msg.contains(w), "the refusal of {:?} should not hold {:?}, and reads: {}",
                    s, w, msg);
            }
        },
    }
}

fn read(s: &str) -> Dat {
    match Dat::decode_string(s) {
        Ok(d) => d,
        Err(e) => panic!("{:?} should read, and was refused: {}", s, e.plain()),
    }
}

#[test]
fn test_map_left_open_is_refused_00() {
    // The inner map used to come back, and the key "a" was dropped.
    refused("{\"a\": {\"size\": 1}", &["closure", "line 1"]);
    refused("{\"a\": [1]", &["closure"]);
    refused("[[1]", &["closure"]);
    refused("[(u8|5)", &["closure"]);
    refused("{\"a\": (u8|5)", &["closure"]);
    refused("{\"a\": 1,", &["closure"]);
    refused("[1, 2", &["closure"]);
}

#[test]
fn test_kind_annotation_left_open_is_refused_00() {
    refused("(u8|5", &["closure", "')'"]);
    refused("{\"n\": (u8|1", &["closure"]);
    refused("{\"n\": (u8|1}", &[]);
}

#[test]
fn test_text_after_the_value_is_refused_00() {
    refused("{\"a\": 1} x", &["after the end", "line 1"]);
    refused("{\"a\": 1} {\"b\": 2}", &["after the end"]);
    refused("{\"a\": 1}\n}", &["after the end", "line 2"]);
    refused("[1] 2", &["after the end"]);
    refused("(u8|5) 6", &["after the end"]);
    // Whitespace and comments may follow.
    read("{\"a\": 1} \n\t ");
    read("{\"a\": 1} ! end !");
    read("{\"a\": 1} # to the end of the line");
    read("[1] ! end ! \n # again");
    read("(u8|5) ! end !");
}

#[test]
fn test_values_need_a_separator_00() {
    // These read as the number 12, and as "ab".
    refused("[1 2]", &["no ',' or ':'"]);
    refused("{\"a\": 1 2}", &["no ',' or ':'"]);
    refused("{\"a\": \"x\" \"y\"}", &["no ',' or ':'"]);
    refused("[\"a\" \"b\"]", &["no ',' or ':'"]);
    refused("{\"a\": 1 \"b\": 2}", &["no ',' or ':'"]);
    refused("{\"a\": 1\n\"b\": 2}", &["line 2"]);
    // A space beside the separators, and around a value, is fine.
    assert_eq!(read("[ 1 , 2 ]").to_string(), read("[1,2]").to_string());
    read("{ \"a\" : 1 , \"b\" : \"x\" }");
}

#[test]
fn test_lines_are_counted_by_newlines_00() {
    // A backslash used to count as a line, so a file with escapes named the wrong line.
    refused("{\n\"a\": \"x\\\\y\\\"z\",\n\"b\": 1 2}", &["line 3"]);
    refused("{\"a\": \"\\n\\n\\n\", \"b\": 1 2}", &["line 1"]);
}

#[test]
fn test_refusal_names_line_and_column_not_the_key_00() {
    // A key may be personal data (an email, a huser code), so a refusal places itself by line and
    // column and names none (lead ruling, U0-13).
    refused_without("{\n\t\"roles\": {\n\t\t\"qr\": {\n\t\t\t\"size\": 1 2,\n\t\t},\n\t},\n}",
        &["line 4", "column 14"], &["roles", "qr", "size"]);
    refused_without("{\"screens\": {\"cs\": [\"a\",\n \"b\" \"c\"]}}", &["line 2"], &["screens", "cs"]);
    // A duplicate key, said where the second one is closed.
    refused_without("{\n\"roles\": {\n\"qr\": 1,\n\"qr\": 2}}", &["line 4", "duplicate key", "already exists"],
        &["roles", "qr\""]);
    // Column counts characters, a tab as one, and not bytes.
    refused("{\n\t\"é日\": 1 2}", &["line 2", "column 10"]);
}

#[test]
fn test_located_gives_the_place_without_the_text_00() {
    let cfg = DecoderConfig::<BTreeMap<_, _>, BTreeMap<_, _>>::default();
    match Dat::decode_string_located("{\n  \"k\": {\"s3cret\": 1 2}}", &cfg) {
        Ok(d) => panic!("should be refused, and became {}", d),
        Err(at) => {
            assert_eq!((at.line, at.col), (2, 21));
            // The place is a line and a column; neither key nor value is in the words.
            let said = at.error.msgs().join(" ");
            assert!(!said.contains("s3cret") && !said.contains("\"k\""), "{}", said);
        },
    }
}

#[test]
fn test_comments_are_free_text_00() {
    // A quote or a brace in a comment is not one outside it.
    let d = read("{ ! was \"logo\": { [ 1, 2 ] } # not a close ! \"a\": 1, # it's \"fine\"\n \"b\": 2 }");
    let s = d.to_string();
    assert!(s.contains("\"a\"") && s.contains("\"b\""), "read as {}", s);
    // A marker in a string is not a comment.
    assert_eq!(read(r##"{"a": "# not ! a comment #"}"##).to_string(), r##"{ "a": "# not ! a comment #"}"##);
}

#[test]
fn test_the_format_is_kept_00() {
    // Features, by design: an unquoted word is a string, a single quote makes a string, a kind
    // annotation types a value, a trailing comma is allowed, 0x is hex, and a comment runs to the
    // end of its line.
    assert_eq!(read("{\"a\": yes}").to_string(), "{ \"a\": \"yes\"}");
    assert_eq!(read("{\"a\": 29x.4}").to_string(), "{ \"a\": \"29x.4\"}");
    assert_eq!(read("{\"a\": 'x'}").to_string(), "{ \"a\": \"x\"}");
    assert_eq!(read("{a: 1}").to_string(), "{ \"a\": (u8|1)}");
    assert_eq!(read("{\"a\": (u8|1), \"b\": 0x10,}").to_string(), "{ \"a\": (u8|1), \"b\": (u8|16)}");
    assert_eq!(read("[1, 2,]").to_string(), "[ (u8|1), (u8|2)]");
    assert!(Dat::decode_string("{\"n\": (u8|300)}").is_err());
    assert!(Dat::decode_string("{\"n\": (zzz|1)}").is_err());
    read("{\"a\": 1, # a note\n \"b\": 2}");
}

#[test]
fn test_a_user_kind_payload_keeps_its_closing_paren_00() {
    // `(node|{...})` ends its frame at the payload's brace, and the root takes the `)` itself.
    let mut uks = UsrKinds::new(BTreeMap::new(), BTreeMap::new());
    let ukind = UsrKindId::new(1, Some("node"), None);
    uks.add(ukind.clone()).unwrap_or_default();
    let cfg = DecoderConfig::<_, _>::jdat(Some(uks));
    let at = |s: &str| match Dat::decode_string_located(s, &cfg) {
        Ok(d) => format!("read {}", d),
        Err(at) => format!("refused line {} col {}: {}", at.line, at.col, at.error.plain()),
    };
    assert!(at("(node|{\"a\":1})").starts_with("read"), "{}", at("(node|{\"a\":1})"));
    assert!(at("(node|[1, 2])").starts_with("read"));
    assert!(at("(node|{\"a\":1} ! note ! \n )  # end").starts_with("read"));
    assert!(at("(node|{\"a\":1}").starts_with("refused"), "open: {}", at("(node|{\"a\":1}"));
    assert!(at("(node|{\"a\":1} x)").starts_with("refused line 1 col 15"), "{}", at("(node|{\"a\":1} x)"));
    assert!(at("(node|{\"a\":1}) x").starts_with("refused line 1 col 16"), "{}", at("(node|{\"a\":1}) x"));
    assert!(at("(node|[1] ").starts_with("refused"));
}

#[test]
fn test_a_user_kind_payload_in_a_container_00() {
    let mut uks = UsrKinds::new(BTreeMap::new(), BTreeMap::new());
    uks.add(UsrKindId::new(1, Some("node"), None)).unwrap_or_default();
    let cfg = DecoderConfig::<_, _>::jdat(Some(uks));
    let at = |s: &str| match Dat::decode_string_located(s, &cfg) {
        Ok(d) => format!("read {}", d),
        Err(at) => format!("refused line {} col {}", at.line, at.col),
    };
    for s in [
        "[(node|{\"a\":1}), 2]",
        "{\"k\": (node|[1, 2]), \"j\": 3}",
        "{\"k\": (node|{\"a\":(node|{\"b\":1})}) ! after ! , \"j\": 3}",
        "(node|(node|{\"x\":1}))",
    ] {
        assert!(at(s).starts_with("read"), "{} became {}", s, at(s));
    }
    // A closing paren too many, or one missing, is a fault wherever it falls.
    for s in [
        "{\"k\": (node|{\"a\":1})) }",
        "{\"k\": (node|{\"a\":1}  }",
        "[(node|{\"a\":1}), 2))",
        "(node|(node|{\"x\":1})",
    ] {
        assert!(at(s).starts_with("refused"), "{} became {}", s, at(s));
    }
}

#[test]
fn test_a_whole_number_kind_takes_no_fraction_00() {
    // Only the digits before the point were parsed, so these read as 1 and 2. A whole value
    // written with a point or an exponent is read (lead ruling, U0-5 L3); a fraction is not.
    refused_without("(u8|1.5)", &["whole number", "has a fraction"], &["1.5"]);
    refused("(i32|-2.5)", &["whole number"]);
    assert_eq!(read("(u16|1e2)"), Dat::U16(100));
    assert_eq!(read("{\"n\": (u64|3.0)}"), one_key("n", Dat::U64(3)));
    refused_without("{\"nKeyQ\": (u64|3.5)}", &["whole number", "line 1"], &["nKeyQ"]);
    // A float kind keeps its fraction, and hex digits are not an exponent.
    assert_eq!(read("(f64|1.5)"), Dat::F64(Float64(1.5)));
    assert_eq!(read("(u8|0xff)"), Dat::U8(255));
    assert_eq!(read("(u16|0x1e5)"), Dat::U16(485));
}

#[test]
fn test_a_number_with_no_digit_is_refused_00() {
    // A number with no digit in its significand (`e2`, `.`, `0x`) read as 0, and an exponent mark
    // with no digit after it (`1e`) read as 1 (U0-13 M1; the root is `NumberString::validate`).
    for s in [
        "(u8|e2)", "(u8|e)", "(u8|.)", "(u8|.e5)", "(u64|E9)", "(i32|-e2)", "(u8|1e)", "(u8|1e-)",
        "(u8|1e+)", "(adec|e2)", "(adec|1e)", "(u8|0x)", "(aint|e2)", "(i64|-.)", "(f64|e2)",
        "(f64|1e)", "(f32|.)",
    ] {
        refused_without(s, &["line 1", "column"], &["e2", "E9"]);
    }
    refused("{\n\"k\": (u8|e2)}", &["line 2", "column"]);
    // The kinds read what they read.
    assert_eq!(read("(u8|0e5)"), Dat::U8(0));
    assert_eq!(read("(u8|1e2)"), Dat::U8(100));
    assert_eq!(read("(u8|0x1f)"), Dat::U8(31));
    assert_eq!(read("(f64|1e2)"), Dat::F64(Float64(100.0)));
    // With no kind, text that is no number is a string, as it always was.
    assert_eq!(read("e2"), Dat::Str("e2".to_string()));
    assert_eq!(read("."), Dat::Str(".".to_string()));
    assert_eq!(read("1e"), Dat::Str("1e".to_string()));
    assert_eq!(read("[e2, ., 1e-]"), Dat::List(vec![
        Dat::Str("e2".to_string()), Dat::Str(".".to_string()), Dat::Str("1e-".to_string()),
    ]));
    // One rule in a list, a map and a nest of both: the atom no number takes is a string.
    assert_eq!(read("{\"k\": e2}"), one_key("k", Dat::Str("e2".to_string())));
    assert_eq!(read("{\"k\": .}"), one_key("k", Dat::Str(".".to_string())));
    assert_eq!(read("{\"k\": 1e-}"), one_key("k", Dat::Str("1e-".to_string())));
    assert_eq!(read("{\"k\": [e2, 1e]}"), one_key("k", Dat::List(vec![
        Dat::Str("e2".to_string()), Dat::Str("1e".to_string()),
    ])));
    assert_eq!(read("[{\"k\": e2}, 3]"), Dat::List(vec![
        one_key("k", Dat::Str("e2".to_string())), Dat::U8(3),
    ]));
    // A typed atom still refuses it.
    refused("[(u8|e2)]", &["line 1", "column"]);
}

#[test]
fn test_a_negative_zero_reads_as_zero_in_every_whole_number_kind_00() {
    // `(u8|-0.0)` read and `(u8|-0)` was refused: one rule for both (U0-13 L1), a zero has no sign.
    for (s, want) in [
        ("(u8|-0)", Dat::U8(0)), ("(u8|-0.0)", Dat::U8(0)), ("(u8|+0)", Dat::U8(0)),
        ("(u16|-0e3)", Dat::U16(0)), ("(u64|-0)", Dat::U64(0)), ("(i8|-0)", Dat::I8(0)),
        ("(i32|-0.0)", Dat::I32(0)),
    ] {
        assert_eq!(read(s), want, "{}", s);
    }
    // A number below zero is still refused in an unsigned kind.
    refused("(u8|-1)", &["cannot be negative"]);
    refused("(u8|-0.5)", &["whole number"]);
    refused("(u8|-1e0)", &["is negative"]);
}

fn bx(d: Dat) -> Dat {
    Dat::Box(Box::new(d))
}

fn sm(d: Dat) -> Dat {
    Dat::Opt(Box::new(Some(d)))
}

#[test]
fn test_a_wrapper_round_a_dataless_value_reads_its_own_paren_00() {
    // The encoder writes these, and the wrapper's frame ended at the inner ')' and left the
    // outer one over: "Found text after the end of the value".
    assert_eq!(read("(box|(true))"), bx(Dat::Bool(true)));
    assert_eq!(read("(box|(false))"), bx(Dat::Bool(false)));
    assert_eq!(read("(some|(none))"), sm(Dat::Opt(Box::new(None))));
    assert_eq!(read("(some|(empty))"), sm(Dat::Empty));
    assert_eq!(read("(some|(some|(none)))"), sm(sm(Dat::Opt(Box::new(None)))));
    assert_eq!(read("(box|(box|(box|(empty))))"), bx(bx(bx(Dat::Empty))));
    // The same in a map, where the stray ')' once emptied the map, and in a list.
    let d = read("{\"k\": (box|(none)), \"j\": (some|(true)), \"i\": 1}");
    assert_eq!(d.to_string(), "{ \"i\": (u8|1), \"j\": (some|(true)), \"k\": (box|(none))}");
    assert_eq!(read("[(some|(empty)), (box|(true)), 3]").to_string(),
        "[ (some|(empty)), (box|(true)), (u8|3)]");
    // Whitespace and a note may stand inside the wrapper, round its payload.
    assert_eq!(read("(box| ( true ) )"), bx(Dat::Bool(true)));
    assert_eq!(read("(box|(true) ! n ! )"), bx(Dat::Bool(true)));
    // A paren too many, or too few, is a fault wherever it falls.
    refused("(box|(true)))", &["after the end"]);
    refused("(box|(true)", &["closure"]);
    refused("{\"k\": (box|(none)))}", &[]);
    refused("{\"k\": (box|(none)}", &[]);
    refused("[(some|(empty))), 3]", &[]);
}

#[test]
fn test_a_finished_value_takes_no_other_before_a_separator_00() {
    // A value of any kind, once finished, refuses the next until a ',' or a ':'. Each of these
    // read without a fault: the first value was dropped, or the second joined the wrong key.
    for s in [
        "{\n  \"server\": {\"port\": 80}\n  \"db\": {\"path\": \"x.db\"}\n}\n",
        "{\"roles\": {\"qr\": {\"size\": 29}\n \"logo\": {\"size\": 4}}}",
        "{\"list\": [1, 2]\n \"name\": \"x\"}",
        "{\n  \"a\": (u8|1)\n  \"b\": 2\n}",
        "[{\"a\":1} {\"b\":2}]",
        "[1 (u8|2)]",
        "[(u8|1) (u8|2)]",
        "[[1] [2]]",
        "[[1] \"x\"]",
        "[[1] x]",
        "[{} {}]",
        "[{} [1]]",
        "{\"a\": [1] 2}",
        "{\"a\": 1 [2]}",
        "{\"a\": 1 {\"b\": 2}}",
        "{\"a\": \"x\" (u8|1)}",
        "{\"a\": 'x' [1]}",
        "{\"a\": (box|(true)) (false)}",
        "(box|(true) x)",
        "(box|(true) \"x\")",
        "x{\"a\":1}",
        "x[1]",
        "[x(u8|1)]",
        "[1(u8|2)]",
        "[\"a\"[1]]",
        "(u 8|1)",
    ] {
        refused(s, &["no ',' or ':'"]);
    }
    refused("{\n  \"server\": {\"port\": 80}\n  \"db\": 1\n}", &["line 3"]);
    // With the commas, and with a note between, they read. A quote glued to a word is one string.
    assert_eq!(read("{\"server\": {\"port\": 80},\n \"db\": {\"path\": \"x.db\"}}").to_string(),
        "{ \"db\": { \"path\": \"x.db\"}, \"server\": { \"port\": (u8|80)}}");
    assert_eq!(read("[{\"a\":1}, {\"b\":2}]").to_string(), "[ { \"a\": (u8|1)}, { \"b\": (u8|2)}]");
    assert_eq!(read("[ [1] ! n ! , [2] ]").to_string(), "[ [ (u8|1)] !n !, [ (u8|2)]]");
    assert_eq!(read("[he\"(]\"o, (u8|1)]"), Dat::List(vec![Dat::Str(fmt!("he(]o")), Dat::U8(1)]));
    assert_eq!(read("(u8 |1)"), Dat::U8(1));
}

fn map_of(k: &str, v: Dat) -> Dat {
    let mut m = BTreeMap::new();
    m.insert(Dat::Str(k.to_string()), v);
    Dat::Map(m)
}

#[test]
fn test_a_kind_takes_only_the_opener_that_fits_it_00() {
    // A box, a some and a user kind wrap a list or a map; an atom or the other molecule's kind
    // takes neither. These all read as the bare molecule, wrapper lost, or were refused.
    assert_eq!(read("(box|{})"), Dat::Box(Box::new(Dat::Map(BTreeMap::new()))));
    assert_eq!(read("(box|{\"k\": (u8|1)})"), Dat::Box(Box::new(map_of("k", Dat::U8(1)))));
    assert_eq!(read("(some|{\"k\": (u8|1)})"), Dat::Opt(Box::new(Some(map_of("k", Dat::U8(1))))));
    assert_eq!(read("(box|[])"), Dat::Box(Box::new(Dat::List(Vec::new()))));
    assert_eq!(read("(box|[1])"), Dat::Box(Box::new(Dat::List(vec![Dat::U8(1)]))));
    assert_eq!(read("(some|[1, 2])"),
        Dat::Opt(Box::new(Some(Dat::List(vec![Dat::U8(1), Dat::U8(2)])))));
    assert_eq!(read("(box|(some|{\"a\": (box|[1])}))"), Dat::Box(Box::new(Dat::Opt(Box::new(Some(
        map_of("a", Dat::Box(Box::new(Dat::List(vec![Dat::U8(1)])))))))))
    );
    assert_eq!(read("[(box|{}), (some|[])]"), Dat::List(vec![
        Dat::Box(Box::new(Dat::Map(BTreeMap::new()))),
        Dat::Opt(Box::new(Some(Dat::List(Vec::new())))),
    ]));
    for s in [
        "(u8|{\"a\":1})", "(str|{\"a\":1})", "(f64|{})", "(bool|{})", "(u8|[1])", "(str|[])",
        "(map|[1])", "(omap|[1])", "(list|{\"a\":1})", "(t2|{\"a\":1})", "(vek|{})", "(bu8|{})",
        "{\"x\": (u8|{\"a\":1})}", "{\"x\": (map|[1])}", "[(list|{}), 1]", "{\"x\": (str|[1])}",
    ] {
        refused(s, &["line 1"]);
    }
    refused("(box|[1] [2])", &["no ',' or ':'"]);
    refused("(box|{} {})", &["no ',' or ':'"]);
    refused("(box|{} [1])", &["no ',' or ':'"]);
    refused("(box|{\"a\":1}", &["closure"]);
}

#[test]
fn test_a_user_kind_wraps_its_list_or_map_payload_00() {
    let mut uks = UsrKinds::new(BTreeMap::new(), BTreeMap::new());
    let ukid = UsrKindId::new(1, Some("node"), None);
    uks.add(ukid.clone()).unwrap_or_default();
    let cfg = DecoderConfig::<_, _>::jdat(Some(uks));
    let at = |s: &str| match Dat::decode_string_with_config(s, &cfg) {
        Ok(d) => d,
        Err(e) => panic!("{:?} should read, and was refused: {}", s, e.plain()),
    };
    assert_eq!(at("(node|{\"a\": (u8|1)})"), Dat::Usr(ukid.clone(), Some(Box::new(map_of("a", Dat::U8(1))))));
    assert_eq!(at("(node|[(u8|1)])"), Dat::Usr(ukid.clone(), Some(Box::new(Dat::List(vec![Dat::U8(1)])))));
    assert_eq!(at("[(node|[]), (node)]"), Dat::List(vec![
        Dat::Usr(ukid.clone(), Some(Box::new(Dat::List(Vec::new())))),
        Dat::Usr(ukid.clone(), None),
    ]));
}

#[test]
fn test_a_plain_list_inside_a_tuple_is_a_list_00() {
    // The inner list took its parent's kind: `(t2|[ [1, 2], 3 ])` read the inner list as a t2.
    let l12 = Dat::List(vec![Dat::U8(1), Dat::U8(2)]);
    assert_eq!(read("(t2|[ [1, 2], 3 ])"), Dat::Tup2(Box::new([l12.clone(), Dat::U8(3)])));
    assert_eq!(read("(t2|[ 3, [1, 2] ])"), Dat::Tup2(Box::new([Dat::U8(3), l12.clone()])));
    assert_eq!(read("(t2|[ [], [[1, 2]] ])"), Dat::Tup2(Box::new([
        Dat::List(Vec::new()), Dat::List(vec![l12.clone()]),
    ])));
    assert_eq!(read("(t3|[ [1], (t2|[ [1, 2], 3 ]), 4 ])"), Dat::Tup3(Box::new([
        Dat::List(vec![Dat::U8(1)]),
        Dat::Tup2(Box::new([l12.clone(), Dat::U8(3)])),
        Dat::U8(4),
    ])));
    assert_eq!(read("(list|[ [1, 2], [] ])"), Dat::List(vec![l12.clone(), Dat::List(Vec::new())]));
    assert_eq!(read("(box|(t2|[ [1, 2], 3 ]))"),
        Dat::Box(Box::new(Dat::Tup2(Box::new([l12.clone(), Dat::U8(3)])))));
    assert_eq!(read("(t2|[ {\"k\": [1, 2]}, 3 ])"),
        Dat::Tup2(Box::new([map_of("k", l12.clone()), Dat::U8(3)])));
}

#[test]
fn test_a_kind_or_quote_left_open_at_the_top_is_refused_00() {
    // Each of these read: `(` as (empty), `(true` as true, `(none` as none, `(u8` as the string
    // "u8", `"abc` and `'abc` as "abc".
    for s in ["(", "(true", "(false", "(none", "(empty", "(u8", "(box", "(my_kind", " ( "] {
        refused(s, &["closure", "')'"]);
    }
    for s in ["\"abc", "'abc", "\"", "'", "\"a\\\"", "he\"llo", "{\"a\": \"x}", "[\"x, 1]", "{\"a\": 'x}"] {
        refused(s, &["closure", "quote"]);
    }
    // A quote or a kind that is closed reads, wherever it sits.
    assert_eq!(read("(true)"), Dat::Bool(true));
    assert_eq!(read("\"a'b\""), Dat::Str(fmt!("a'b")));
    assert_eq!(read("'a\"b'"), Dat::Str(fmt!("a\"b")));
}

#[test]
fn test_a_stray_bar_or_colon_is_refused_00() {
    // `(u8|1|2)` read as 12 and `{"a": 1:2}` as {"a": 2}; `{:1}` gave an (empty) key.
    for s in [
        "(u8|1|2)", "{\"a\": 1|2}", "[1|2]", "|", "||", "[|]", "{\"a\": |}", "(u8||1)", "a|b",
        "{a|b: 1}", "(box|(u8|1)|)", "[(u8|1)|]",
    ] {
        refused(s, &["'|'"]);
    }
    for s in [
        "{\"a\": 1:2}", "{\"a\"::1}", "{\"a\": 1, \"b\": : 2}", "{\"a\": \"b\": 1}",
    ] {
        refused(s, &["':'"]);
    }
    // The separators in their places, and inside strings and notes, read.
    assert_eq!(read("(u8|1)"), Dat::U8(1));
    assert_eq!(read("{\"a|b\": \"c:d|e\", ! n|: ! \"f\": (u8|2)}").to_string(),
        "{ \"a|b\": \"c:d|e\", \"f\" !n|: !: (u8|2)}");
    read("{(u8|1): 2, [3]: 4}");
}

#[test]
fn test_an_empty_quoted_key_is_a_key_00() {
    // A `""` key before a comma was replaced by a random u32 key, the one before the closing
    // brace was kept, so `{"": 1}` and `{"": 1, "b": 2}` disagreed and the key was lost.
    let one = |ks: &[&str]| {
        let mut m = BTreeMap::new();
        for (i, k) in ks.iter().enumerate() {
            m.insert(Dat::Str(k.to_string()), Dat::U8(i as u8 + 1));
        }
        Dat::Map(m)
    };
    assert_eq!(read("{\"\": 1}"), one(&[""]));
    assert_eq!(read("{\"\": 1, \"b\": 2}"), one(&["", "b"]));
    assert_eq!(read("{\"b\": 1, \"\": 2}"), one(&["b", ""]));
    assert_eq!(read("{'': 1, \"c\": 2}"), one(&["", "c"]));
    // Two empty keys are a duplicate, as any other key is.
    refused("{\"\": 1, \"\": 2}", &["already exists"]);
    // The encoder's own text reads back.
    let d = one(&["", "b"]);
    assert_eq!(read(&d.jdat().unwrap_or_default()), d);
}

#[test]
fn test_a_comma_with_nothing_before_it_is_refused_00() {
    // `[1,,2]` read as [1, (empty), 2] and `{"a": 1,, "b": 2}` was refused only at the colon
    // after it; a ',' that ends no item is a missing item, in a list as in a map.
    for s in [
        "[,]", "[,1]", "[1,,2]", "[1, ,2]", "[1,2,,]", "[ , ]", "(list|[,])", "[[,]]",
        "{,}", "{,\"a\": 1}", "{\"a\": 1,, \"b\": 2}", "{\"a\": 1, , \"b\": 2}", "{\"a\": {,}}",
        "{\"a\": [1,,2]}", "(box|[1,,2])", "(t2|[1,,2])",
    ] {
        refused(s, &["','", "nothing"]);
    }
    // A member that has a key and no value is the same fault as one cut off by the brace.
    for s in ["{\"a\":,}", "{\"a\": 1, \"b\":, \"c\": 2}", "{\"a\": ,}", "{:,}"] {
        refused(s, &["no value"]);
    }
    // An item, a string, an explicit empty or a note before the comma is something.
    assert_eq!(read("[1,]"), Dat::List(vec![Dat::U8(1)]));
    assert_eq!(read("[\"\", 1]"), Dat::List(vec![Dat::Str(fmt!("")), Dat::U8(1)]));
    assert_eq!(read("[(empty), 1]"), Dat::List(vec![Dat::Empty, Dat::U8(1)]));
    assert_eq!(read("{\"a\": 1,}"), one_key("a", Dat::U8(1)));
    assert_eq!(read("{\"a\": (empty), \"b\": \"\"}").to_string(), "{ \"a\": (empty), \"b\": \"\"}");
    read("[ ! a note ! , 1]");
    read("{\"a\": ! a note !, \"b\": 1}");
}

#[test]
fn test_a_comma_where_a_colon_belongs_is_refused_00() {
    // `{"a", 1}` read as {"a": 1}, the comma setting the key; the format's colon is the only
    // separator of a key from its value (lead ruling, consistent with H2).
    for s in [
        "{\"a\", 1}", "{\"a\", 1, \"b\", 2}", "{\"a\": 1, \"b\", 2}", "{1, 2}", "{\"a\" ! c !, 1}",
        "{! c !, \"a\": 1}", "{\"a\": {\"b\", 1}}", "{\"a\",}", "(map|{\"a\", 1})",
    ] {
        refused(s, &["','", "':'"]);
    }
    read("{\"a\": 1, \"b\": 2}");
}

#[test]
fn test_whitespace_in_a_kind_label_reads_as_a_space_00() {
    // A tab, a CR and a LF are whitespace as a space is, around a label; a gap inside one is not.
    for s in ["(u8 |1)", "(u8\t|1)", "(u8\r|1)", "(u8\n|1)", "(u8\r\n|1)", "(\r\nu8|1)", "( \tu8 \t|1)"] {
        assert_eq!(read(s), Dat::U8(1), "{:?}", s);
    }
    assert_eq!(read("{\"a\": (u8\r\n|1)}"), one_key("a", Dat::U8(1)));
    for s in ["(u 8|1)", "(u\t8|1)", "(u\r\n8|1)"] {
        refused(s, &["gap"]);
    }
}

fn one_key(k: &str, v: Dat) -> Dat {
    map_of(k, v)
}

#[test]
fn test_an_empty_key_is_refused_unless_it_keys_a_note_00() {
    // `{:1}` read as an (empty) key; the format keys a note to the empty dat, `{:! note !,}`.
    for s in ["{:1}", "{\"a\": 1, :2}", "{ : \"x\"}", "{: ! n ! 5}", "{:[1]}", "{:{}}"] {
        refused(s, &["':'", "key"]);
    }
    refused("{:}", &["key"]);
    read("{:! a note !}");
    read("{\"a\": 1, :! a note !, \"b\": 2}");
    read("{1:2, :! a singleton comment !, ! a self-keyed comment ! 3:4}");
    // An explicit (empty) is a key, as the encoder writes it.
    let mut m = BTreeMap::new();
    m.insert(Dat::Empty, Dat::U8(1));
    assert_eq!(read("{(empty): 1}"), Dat::Map(m));
}

// A note is no part of what a value is, so these compare what was read with the notes taken off.
fn strip(d: &Dat) -> Dat {
    match d {
        Dat::ABox(_, inner, _)  => strip(inner),
        Dat::Map(m)             => Dat::Map(m.iter().map(|(k, v)| (strip(k), strip(v))).collect()),
        Dat::List(v)            => Dat::List(v.iter().map(strip).collect()),
        other                   => other.clone(),
    }
}

fn big_map(n: usize) -> String {
    let mut s = String::from("{");
    for i in 0..n {
        s.push_str(&format!("\"k{}\": 0, ", i));
    }
    s
}

#[test]
fn test_a_duplicate_key_is_refused_in_few_words_00() {
    // The refusal held a debug print of the whole map being read, megabytes for a large one.
    let s = format!("{}\"k0\": 1}}", big_map(500));
    refused_without(&s, &["A duplicate key, which already exists in the map"], &["k0"]);
    match Dat::decode_string(&s) {
        Err(e) => assert!(e.plain().len() < 400, "the refusal is {} bytes", e.plain().len()),
        Ok(d) => panic!("should be refused, and became {}", d),
    }
    // A key behind a note is the same key.
    refused("{\"a\": 1, ! n ! \"a\": 2}", &["already exists"]);
    refused("{\"a\": 1, \"b\": 2, # n\n \"a\":! c\n}", &["already exists"]);
    // As it is in an ordered map.
    let mut cfg = DecoderConfig::<BTreeMap<_, _>, BTreeMap<_, _>>::default();
    cfg.use_ordmaps = true;
    for s in [format!("{}\"k0\": 1}}", big_map(500)), "{\"a\": 1, ! n ! \"a\": 2}".to_string()] {
        match Dat::decode_string_with_config(s.as_str(), &cfg) {
            Err(e) => assert!(e.plain().contains("already exists") && e.plain().len() < 400,
                "the refusal of an ordered map is {}", e.plain()),
            Ok(d) => panic!("should be refused, and became {}", d),
        }
    }
}

#[test]
fn test_an_unpaired_member_is_refused_without_the_map_00() {
    let s = format!("{}\"z\"}}", big_map(500));
    refused(&s, &["value with no key at the end of a map"]);
    let s = format!("{}\"z\":}}", big_map(500));
    refused_without(&s, &["A key has no value at the end of a map"], &["\"z\""]);
    for s in [format!("{}\"z\"}}", big_map(500)), format!("{}\"z\":}}", big_map(500))] {
        match Dat::decode_string(&s) {
            Err(e) => assert!(e.plain().len() < 400, "the refusal is {} bytes", e.plain().len()),
            Ok(d) => panic!("should be refused, and became {}", d),
        }
    }
}

#[test]
fn test_a_byte_order_mark_is_not_text_00() {
    let want = read("{\"a\":1}");
    assert_eq!(read("\u{feff}{\"a\":1}"), want);
    assert_eq!(read("\u{feff}\n{\"a\":1}"), want);
    refused("\u{feff}\u{feff}{\"a\":1}", &[]);
    refused("{\"a\":1}\u{feff}", &[]);
    // Inside a string it is a character like another.
    assert_eq!(read("{\"a\":\"\u{feff}x\"}"), one_key("a", Dat::Str("\u{feff}x".to_string())));
}

#[test]
fn test_a_comment_before_the_root_value_is_dropped_00() {
    for (s, want) in [
        ("! one !\n{\"a\": 1}", "{\"a\": 1}"),
        ("# one\n{\"a\": 1}", "{\"a\": 1}"),
        ("# one\n# two\n{\"a\": 1}", "{\"a\": 1}"),
        ("! one !\n[1, 2]", "[1, 2]"),
        ("# one\n[1, 2]", "[1, 2]"),
        ("# one\n(u8|1)", "(u8|1)"),
        ("# one\n\"x\"", "\"x\""),
        ("# one\n42", "42"),
    ] {
        assert_eq!(read(s), read(want), "{:?}", s);
    }
    // And one after it, or between a value's marks, stays as it was.
    assert_eq!(read("{\"a\": 1} # end"), read("{\"a\": 1}"));
    assert_eq!(read("{\"a\": 1} ! end !"), read("{\"a\": 1}"));
    // Outside a list or a map a comment has no item to be the note of, and the refusal says so
    // in the decoder's words and not the old "only allowed in molecules".
    refused("42 # c\n43", &["comment", "outside any list or map", "line "]);
}

#[test]
fn test_a_whole_number_kind_takes_a_whole_number_in_any_form_00() {
    assert_eq!(read("(u8|1.0)"), Dat::U8(1));
    assert_eq!(read("(u8|10e-1)"), Dat::U8(1));
    assert_eq!(read("(u64|1e3)"), Dat::U64(1000));
    assert_eq!(read("(i8|-2.0)"), Dat::I8(-2));
    assert_eq!(read("(i32|-1.5e1)"), Dat::I32(-15));
    assert_eq!(read("{\"n\": (u16|65535.0)}"), one_key("n", Dat::U16(65535)));
    let t0 = std::time::Instant::now();
    for s in [
        "(u8|1.5)", "(u8|256.0)", "(u8|-1.0)", "(u8|1e-1)", "(u8|1e999999)", "(u64|1e40)",
        "(u8|1e9999999999999999999)", "(i8|128.0)", "(u8|0.5e1e1)",
        "(f64|1e400)", "(f32|1e39)", "(f64|-1e400)",
    ] {
        refused(s, &[]);
    }
    assert!(t0.elapsed().as_secs() < 2, "the refusals took {:?}", t0.elapsed());
    // A bare number is as it was.
    assert!(Dat::decode_string("{\"n\": 1e400}").is_ok());
    assert!(matches!(read("(f64|1e300)"), Dat::F64(_)));
}

#[test]
fn test_a_comment_is_allowed_where_whitespace_is_00() {
    let two = read("{\"a\": 1, \"b\": 2}");
    for s in [
        "{\"a\": 1 # c\n, \"b\": 2}",
        "{\"a\": 1 ! c !, \"b\": 2}",
        "{\"a\" # c\n: 1, \"b\": 2}",
        "{\"a\": 1, \"b\" # c\n: 2}",
        "{\"a\" ! c ! : 1, \"b\": 2}",
        "{\"a\": 1, \"b\": 2 # c\n}",
    ] {
        assert_eq!(strip(&read(s)), strip(&two), "{:?}", s);
    }
    let two = read("[1, 2]");
    for s in ["[1 # c\n, 2]", "[1, 2 # c\n]", "[1 ! c !, 2]", "[1 # a\n\n# b\n, 2]"] {
        assert_eq!(strip(&read(s)), strip(&two), "{:?}", s);
    }
    assert_eq!(strip(&read("[[1] # c\n, 2]")), strip(&read("[[1], 2]")));
    assert_eq!(strip(&read("{\"a\": [1 # c\n, 2] # d\n, \"b\": {\"x\": 1 # e\n} # f\n}")),
        strip(&read("{\"a\": [1, 2], \"b\": {\"x\": 1}}")));
    // The note is kept: it is on the value, and the key comes out bare.
    assert!(read("{\"a\": 1 # c\n, \"b\": 2}").to_string().contains("#c#"));
    // A comment is not a separator, nor does it supply a key or a value.
    for s in [
        "{\"a\": 1 # c\n \"b\": 2}", "[1 # c\n 2]", "{\"a\" # c\n, \"b\": 2}", "{\"a\": 1 # c\n : 2}",
        "{\"a\": 1 # c\n,, \"b\": 2}", "{# c\n, \"a\": 1}", "[1, # c\n, 2]", "{\"a\": 1 # c\n\"a\": 2}",
    ] {
        refused(s, &[]);
    }
    // A comment on a line of its own, or after a ':', is the note it was; one after a ',' is an
    // entry of its own.
    assert!(Dat::decode_string("{\"a\": 1 # c\n, # d\n \"b\": 2}").is_ok());
    assert!(Dat::decode_string("{\"a\": 1,\n # c\n \"b\": 2}").is_ok());
    assert!(Dat::decode_string("{\"a\":! c\n \"b\": 2}").is_ok());
}

#[test]
fn test_a_refusal_at_a_line_end_names_that_line_00() {
    // The newline that ends a comment closes the entry before it, and the refusal is of that entry.
    let cfg = DecoderConfig::<BTreeMap<_, _>, BTreeMap<_, _>>::default();
    match Dat::decode_string_located("{\"a\": 1, \"b\": 2, \"b\":! c\n}", &cfg) {
        Ok(d) => panic!("should be refused, and became {}", d),
        Err(at) => assert_eq!((at.line, at.col), (1, 25)),
    }
    match Dat::decode_string_located("{\n\"a\": 1,\n\"a\":! c\n}", &cfg) {
        Ok(d) => panic!("should be refused, and became {}", d),
        Err(at) => assert_eq!((at.line, at.col), (3, 8)),
    }
}

#[test]
fn test_a_refusal_holds_no_cursor_dump_and_no_internal_names_00() {
    for s in [
        "[1,,2]", "{\"a\", 1}", "{\"a\": 1 \"b\"}", "{:1}", "[1, 2", "{\"a\": 1", "(u8|1", "(u8|1|2)",
        "{\"a\": 1, \"a\": 2}", "(u8 1)", "[1 2]", "{\"a\"}", "{\"a\":}", "(map|[1])", "(list|{\"a\": 1})",
        "(u8|(u8|1))", "(tup2|[1])", "(tup2|[1, 2, 3])", "(abox|)", "]", "}", ")", ":", "|",
    ] {
        match Dat::decode_string(s) {
            Ok(d) => panic!("{:?} should be refused, and became {}", s, d),
            Err(e) => {
                let msg = e.plain();
                for bad in ["char '", " pos ", "store.", "DaticleMap", "OrdDaticleMap", "Map({", "List(["] {
                    assert!(!msg.contains(bad), "the refusal of {:?} holds {:?}: {}", s, bad, msg);
                }
            },
        }
    }
    refused("[1, 2", &["list"]);
    refused("{\"a\": 1", &["map"]);
}

#[test]
fn test_a_key_with_a_line_break_is_not_named_00() {
    let cfg = DecoderConfig::<BTreeMap<_, _>, BTreeMap<_, _>>::default();
    for (s, name) in [("{\"a\\nb\": 1 2}", "a\\nb"), ("{\"a\\rb\": 1 2}", "a\\rb"), ("{\"a\\\"b\": 1 2}", "a\\\"b")] {
        match Dat::decode_string_located(s, &cfg) {
            Ok(d) => panic!("{:?} should be refused, and became {}", s, d),
            Err(at) => assert!(!at.error.msgs().join(" ").contains(name), "{:?}", s),
        }
        match Dat::decode_string(s) {
            Ok(d) => panic!("{:?} should be refused, and became {}", s, d),
            Err(e) => {
                let msg = e.plain();
                assert!(!msg.contains('\n') && !msg.contains('\r'), "{:?} refused with a break: {:?}", s, msg);
            },
        }
    }
}

#[test]
fn test_a_refusal_has_a_class_00() {
    // A store scan counts refusals by class and prints no text; the label comes from the
    // decoder's words, so a reworded refusal fails here and not silently in a count.
    use oxedyne_fe2o3_jdat::string::dec::Located;
    let cfg = DecoderConfig::<BTreeMap<_, _>, BTreeMap<_, _>>::default();
    let class = |s: &str| match Dat::decode_string_located(s, &cfg) {
        Ok(d) => format!("READ {}", d),
        Err(at) => at.class(s).to_string(),
    };
    for (text, want) in [
        // A comma-less comment, whichever mark and wherever it falls, and not one in a string.
        ("[1 # c\n 2]",                 "no separator after a comment"),
        ("[1 ! c ! 2]",                 "no separator after a comment"),
        ("{\"a\":1 # c\n \"b\":2}",     "no separator after a comment"),
        ("[\"a # b\" 2]",               "no separator"),
        ("[1 2]",                       "no separator"),
        ("{\"a\":1 \"b\":2}",           "no separator"),
        ("[1 # c\n, 2 3]",              "no separator"),
        // A whole-number kind with a fraction.
        ("(u8|1.5)",                    "whole-number kind with a fraction"),
        ("{\"n\": (i32|-2.5)}",         "whole-number kind with a fraction"),
        ("[,]",                         "comma follows nothing"),
        ("{\"a\", 1}",                  "comma where a colon belongs"),
        ("{:1}",                        "colon with no key"),
        ("{\"a\"::1}",                  "second colon"),
        ("{\"a\":,}",                   "key without value"),
        ("{\"a\":1,\"a\":2}",           "duplicate key"),
        ("[1, 2",                       "unclosed"),
        ("[1] x",                       "text after the value"),
        ("[1|2]",                       "kind label"),
    ] {
        assert_eq!(class(text), want, "the class of {:?}", text);
        assert!(Located::CLASSES.contains(&want), "{:?} is not a listed class", want);
    }
    // A text that reads has no class to give.
    assert!(class("[1, # c\n 2]").starts_with("READ"));
}


// A refusal names the kind, the position and the class, never the value's text (U0-13 M2). The
// messages reach a browser through `jdat_to_json` and an app's log, and a value in a refusal is a
// value in a log. A key is no safer, a stored map is keyed by an email or a huser code (lead
// ruling, U0-13). Four marks are planted: a word and a run of digits in the values, a word in the
// keys, and a letter outside ASCII for the refusals that name one character.
const WORD:     &str = "Zq9xW";
const DIGITS:   &str = "7351846";
const KEYW:     &str = "Qv7kXp";
const LETTER:   char = 'Ж';

// What a refusal says, in every form a caller can print.
fn said_by(at: &oxedyne_fe2o3_jdat::string::dec::Located) -> String {
    fmt!("{} | {} | {} | {:?}", at.error.plain(), at.error, at.error.msgs().join(" "), at.error)
        .to_lowercase()
}

// Is a mark in the words? A mark of three characters or more is looked for in every window of
// three, so that a fragment of the value fails as well as the whole.
fn leaked(said: &str) -> Option<String> {
    // The debug form of an error carries the decoder's own source lines, `dec.rs:1735`, which
    // move with every edit and can hold any run of digits, so they are not the value's text.
    let mut plain = String::new();
    let mut rest = said;
    while let Some(i) = rest.find(".rs:") {
        plain.push_str(&rest[..i + 4]);
        rest = rest[i + 4..].trim_start_matches(|c: char| c.is_ascii_digit());
    }
    plain.push_str(rest);
    let said = plain.as_str();
    for mark in [WORD, DIGITS, KEYW] {
        let m: Vec<char> = mark.to_lowercase().chars().collect();
        for w in m.windows(3) {
            let w: String = w.iter().collect();
            if said.contains(&w) {
                return Some(fmt!("{:?} in the refusal: {}", w, said));
            }
        }
    }
    if said.contains(LETTER.to_lowercase().next().unwrap_or(LETTER)) {
        return Some(fmt!("{:?} in the refusal: {}", LETTER, said));
    }
    None
}

fn node_cfg() -> DecoderConfig<BTreeMap<u16, oxedyne_fe2o3_jdat::usr::UsrKind>, BTreeMap<String, UsrKindId>> {
    let mut uks = UsrKinds::new(BTreeMap::new(), BTreeMap::new());
    uks.add(UsrKindId::new(1, Some("node"), None)).unwrap_or_default();
    DecoderConfig::<_, _>::jdat(Some(uks))
}

#[test]
fn test_no_refusal_holds_the_values_text_00() {
    use oxedyne_fe2o3_jdat::string::dec::Located;
    let cfg = node_cfg();
    // One text for each class `Located::class` gives, and the several ways into the 'other'
    // class, each with the marks where a value stands.
    let by_class: Vec<(&str, Vec<String>)> = vec![
        (Located::CLASSES[0], vec![
            fmt!("(u8|{}.{})", DIGITS, DIGITS), fmt!("{{\"{}\": (i32|-{}.5)}}", KEYW, DIGITS),
            fmt!("(u32|4294967295.{})", DIGITS),
        ]),
        (Located::CLASSES[1], vec![
            fmt!("(u8|{}e0)", DIGITS), fmt!("(i8|{}.0)", DIGITS), fmt!("(u16|{}e1)", DIGITS),
            fmt!("{{\"{}\": (u8|{}e0)}}", KEYW, DIGITS),
        ]),
        (Located::CLASSES[2], vec![fmt!("[{} # c\n {}]", WORD, WORD), fmt!("{{\"{}\": {} # c\n \"{}\": 1}}", KEYW, WORD, KEYW)]),
        (Located::CLASSES[3], vec![
            fmt!("[{} {}]", WORD, WORD), fmt!("[\"{}\" \"{}\"]", WORD, WORD),
            fmt!("{{\"{}\": {} \"{}\": 1}}", KEYW, WORD, KEYW), fmt!("{} {}", WORD, DIGITS),
            fmt!("(u8|1 {})", LETTER), fmt!("(node|{}{})", WORD, "{"),
        ]),
        (Located::CLASSES[4], vec![fmt!("[,{}]", WORD), fmt!("{{\"{}\": [,{}]}}", KEYW, WORD)]),
        (Located::CLASSES[5], vec![
            fmt!("{{\"k\": {}, \"{}\", {}}}", WORD, KEYW, WORD), fmt!("{{\"{}\", {}}}", KEYW, WORD),
        ]),
        (Located::CLASSES[6], vec![fmt!("{{:{}}}", WORD), fmt!("{{\"{}\": 1, :{}}}", KEYW, WORD)]),
        (Located::CLASSES[7], vec![fmt!("{{\"k\":: {}}}", WORD), fmt!("{{\"{}\":: {}}}", KEYW, WORD)]),
        (Located::CLASSES[8], vec![
            fmt!("{{\"k\": {}, \"j\":,}}", WORD), fmt!("{{\"{}\": {}, \"{}2\":,}}", KEYW, WORD, KEYW),
            fmt!("{{\"{}\": {}, \"{}2\":}}", KEYW, WORD, KEYW),
        ]),
        (Located::CLASSES[9], vec![
            fmt!("{{\"k\": {}, \"k\": {}}}", WORD, WORD), fmt!("{{\"{}\": {}, \"{}\": {}}}", KEYW, WORD, KEYW, WORD),
            fmt!("{{\"{}\": 1, ! {} ! \"{}\": 2}}", KEYW, WORD, KEYW),
        ]),
        (Located::CLASSES[10], vec![
            fmt!("[{}, {}", WORD, DIGITS), fmt!("(u8|{}", DIGITS), fmt!("{{\"{}\": {}", KEYW, WORD),
        ]),
        (Located::CLASSES[11], vec![
            fmt!("[{}] {}", WORD, WORD), fmt!("[{}] {}", WORD, DIGITS), fmt!("{{\"{}\": {}}} {}", KEYW, WORD, KEYW),
        ]),
        (Located::CLASSES[12], vec![
            fmt!("[{}|{}]", WORD, WORD), fmt!("(u8 {}|1)", LETTER), fmt!("(u8 x{}|1)", WORD),
            fmt!("({}|1)", LETTER), fmt!("({}|1)", WORD), fmt!("(str|[1, {}])", WORD),
            fmt!("{{\"{}\": (zzz|{})}}", KEYW, WORD),
        ]),
        (Located::CLASSES[13], vec![
            fmt!("(u8|-{})", DIGITS), fmt!("(u16|-{})", DIGITS), fmt!("(u32|-{})", DIGITS),
            fmt!("(u64|-{})", DIGITS), fmt!("(u128|-{})", DIGITS), fmt!("(c64|-{})", DIGITS),
            fmt!("(f32|{}e38)", DIGITS), fmt!("(f64|{}e999)", DIGITS), fmt!("(f32|3.{}e38)", DIGITS),
            fmt!("{{\"{}\": (u8|-{})}}", KEYW, DIGITS),
        ]),
        (Located::CLASSES[14], vec![
            // A number with no digit, or an exponent mark with no digit after it (M1).
            fmt!("(u8|e{})", DIGITS), fmt!("(u8|{}e)", DIGITS), fmt!("(i32|-e{})", DIGITS),
            fmt!("{{\"{}\": (u8|e{})}}", KEYW, DIGITS), fmt!("(adec|{}e-)", DIGITS),
            // The rest.
            fmt!("(u8|\"{}\")", WORD), fmt!("(u8|{})", WORD), fmt!("{{\"k\": [1, {}]}}", "(u8|\"Zq9xW\")"),
            fmt!("(bu8|[1, \"{}\"])", WORD), fmt!("(f64|{})", WORD),
            fmt!("(f64|e{})", DIGITS), fmt!("(i8|{}{})", DIGITS, WORD),
            fmt!("(aint|{}.5)", DIGITS), fmt!("(aint|{}{})", DIGITS, WORD), fmt!("(adec|{}x)", DIGITS),
            fmt!("(b32|\"{}\")", WORD), fmt!("(b2|\"{}\")", WORD),
            fmt!("(u8|0x{})", WORD), fmt!("(u8|{}_{})", DIGITS, DIGITS), fmt!("(u8|+{})", WORD),
            fmt!("\"a\\{}b\"", LETTER), fmt!("\"\\u{}\"", LETTER),
            fmt!("\"\\uD800\\u{}\"", LETTER),
            fmt!("(u8|{}){}", DIGITS, WORD),
        ]),
    ];
    // Every text that is refused under another class than its own, listed together.
    let mut astray: Vec<String> = Vec::new();
    for (class, texts) in &by_class {
        for text in texts {
            match Dat::decode_string_located(text, &cfg) {
                Ok(d)   => astray.push(fmt!("{:?} is read as {}, not refused", text, d)),
                Err(at) => if at.class(text) != *class {
                    astray.push(fmt!("{:?} is {:?}, not {:?}", text, at.class(text), class));
                },
            }
        }
    }
    assert!(astray.is_empty(), "texts of another class than listed:\n{}", astray.join("\n"));
    let mut n = 0;
    for (class, texts) in &by_class {
        for text in texts {
            for (name, c) in [("jdat", &cfg)] {
                let at = match Dat::decode_string_located(text, c) {
                    Ok(d) => panic!("{} {:?} should be refused, and read as {}", name, text, d),
                    Err(at) => at,
                };
                let said = said_by(&at);
                assert_eq!(at.class(text), *class, "the class of {:?}: {}", text, said);
                assert!(leaked(&said).is_none(), "{:?} is refused with: {}", text, leaked(&said).unwrap_or_default());
                // A refusal still says where (rc condition): a line and a column, nonzero.
                assert!(at.line >= 1 && at.col >= 1, "{:?} is refused at line {}, column {}", text, at.line, at.col);
                let place = fmt!("line {}, column {}", at.line, at.col);
                // The refusal `decode_string` hands a caller says the same, and says where.
                match Dat::decode_string_with_config(text, c) {
                    Ok(_) => panic!("{:?} should be refused by decode_string", text),
                    Err(e) => {
                        let said = fmt!("{} | {} | {:?}", e.plain(), e, e).to_lowercase();
                        assert!(leaked(&said).is_none(), "{:?} is refused with: {}", text, leaked(&said).unwrap_or_default());
                        assert!(e.plain().contains(&place), "{:?}: the refusal should say {:?}, and reads: {}",
                            text, place, e.plain());
                    },
                }
                // As does the browser's reader, which refuses what the decoder does (the one
                // `(node|..)` text is an unknown kind to it, and refused as well).
                match Dat::jdat_to_json(text, &DecodeLimits::text(), true) {
                    Ok(j) => panic!("{:?} should be refused by jdat_to_json, and became {}", text, j),
                    Err(e) => {
                        let said = fmt!("{} | {} | {:?}", e.plain(), e, e).to_lowercase();
                        assert!(leaked(&said).is_none(), "{:?} is refused by jdat_to_json with: {}", text, leaked(&said).unwrap_or_default());
                        assert!(e.plain().contains("line ") && e.plain().contains("column "),
                            "{:?}: jdat_to_json should say where, and reads: {}", text, e.plain());
                    },
                }
                n += 1;
            }
        }
    }
    assert!(n >= 60, "only {} refusals were driven", n);
}

#[test]
fn test_no_refusal_to_a_mutated_text_holds_the_values_text_00() {
    // Truncations, deletions and insertions at every position of documents whose values hold the
    // marks: every text that is refused is refused without them, under both configurations.
    let ucfg = node_cfg();
    let jcfg = DecoderConfig::<BTreeMap<u16, oxedyne_fe2o3_jdat::usr::UsrKind>, BTreeMap<String, UsrKindId>>::json(None);
    let docs = [
        fmt!("{{\"{k}\": (u8|{d}), \"j\": [{w}, \"{w}\", (f32|{d}e38)], \"m{k}\": (bu8|[1, \"{w}\"]), \"n\": (u16|-{d})}}", d = DIGITS, w = WORD, k = KEYW),
        fmt!("[(node|\"{w}\"), (node|{d}), (node|{{\"a\": \"{w}\"}}), (t2|[{w}, {d}]), (box|(u8|{d})), (some|\"{w}\")]", d = DIGITS, w = WORD),
        fmt!("{{\"a\": (adec|{d}.{d}e5), \"b\": (b32|\"{w}\"), \"c\": (c64|{d}), \"e\": (i8|-{d}.0), \"f\": \"x\\u00e9\\n{w}\"}}", d = DIGITS, w = WORD),
        fmt!("{{\"{k}\": 1, \"{k}\": 2, \"{k}x\": [{w} {w}], {k}y: {d}, \"{k}z\"}}", d = DIGITS, w = WORD, k = KEYW),
        fmt!("{{\"k\": {d}, \"j\": [\"{w}\", {w}, {d}.{d}, -{d}e-{d}, true, null], \"q\": \"{w}\"}}", d = DIGITS, w = WORD),
    ];
    let junk = ['(', ')', '|', '[', ']', '{', '}', ',', ':', '"', '\'', '#', '!', '\\', 'x', '-', '.', 'e', ' ', '\n', LETTER];
    let mut refused = 0;
    let mut bad = Vec::new();
    let mut unplaced = Vec::new();
    for doc in &docs {
        let cs: Vec<char> = doc.chars().collect();
        let mut texts: Vec<String> = Vec::new();
        for i in 0..=cs.len() {
            texts.push(cs[..i].iter().collect());
            if i < cs.len() {
                let mut d = cs.clone();
                d.remove(i);
                texts.push(d.iter().collect());
            }
            for j in junk {
                let mut d = cs.clone();
                d.insert(i, j);
                texts.push(d.iter().collect());
            }
        }
        for text in &texts {
            for c in [&ucfg] {
                if let Err(at) = Dat::decode_string_located(text, c) {
                    refused += 1;
                    let said = said_by(&at);
                    if let Some(l) = leaked(&said) {
                        bad.push(fmt!("{:?} -> {}", text, l));
                    }
                    if let Err(e) = Dat::decode_string_with_config(text, c) {
                        if !e.plain().contains(&fmt!("line {}, column {}", at.line, at.col)) {
                            unplaced.push(fmt!("{:?} -> {}", text, e.plain()));
                        }
                    }
                }
            }
            if let Err(at) = Dat::decode_string_located(text, &jcfg) {
                refused += 1;
                let said = said_by(&at);
                if let Some(l) = leaked(&said) {
                    bad.push(fmt!("{:?} -> {}", text, l));
                }
                if let Err(e) = Dat::decode_string_with_config(text, &jcfg) {
                    if !e.plain().contains(&fmt!("line {}, column {}", at.line, at.col)) {
                        unplaced.push(fmt!("{:?} -> {}", text, e.plain()));
                    }
                }
            }
        }
    }
    bad.sort_by_key(|b| b.len());
    for b in bad.iter().take(10) {
        println!("{}", b.chars().take(500).collect::<String>());
    }
    assert!(refused >= 3000, "only {} refusals were driven", refused);
    assert!(unplaced.is_empty(), "{} refusals do not say where, the first being {:?}", unplaced.len(), unplaced.first());
    assert!(bad.is_empty(), "{} refusals hold a value's text, the shortest being {}", bad.len(),
        bad.first().map(|b| b.chars().take(500).collect::<String>()).unwrap_or_default());
}
