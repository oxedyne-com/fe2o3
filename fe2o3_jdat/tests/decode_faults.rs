//! `Dat::decode_string` on text that is wrong. It was lenient where leniency hid a fault, found
//! while building the Oxegen settings reader (D-20261005-12): a map left open returned its inner
//! map, text after the value was ignored, and digits parted by a space read as one number. Each
//! of those is an error now that names the line, the column and the key. What the format allows
//! by design (unquoted words, single quotes, kind annotations, trailing commas, comments that
//! run to the end of the line) is checked to stay.

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_jdat::{
    prelude::*,
    string::dec::DecoderConfig,
    usr::{
        UsrKindId,
        UsrKinds,
    },
};
use oxedyne_fe2o3_num::float::Float64;

use std::collections::BTreeMap;


fn refused(s: &str, words: &[&str]) {
    match Dat::decode_string(s) {
        Ok(d) => panic!("{:?} should be refused, and became {}", s, d),
        Err(e) => {
            let msg = e.plain();
            for w in words {
                assert!(msg.contains(w), "the refusal of {:?} should hold {:?}, and reads: {}",
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
fn test_refusal_names_line_column_and_key_00() {
    refused("{\n\t\"roles\": {\n\t\t\"qr\": {\n\t\t\t\"size\": 1 2,\n\t\t},\n\t},\n}",
        &["line 4", "column 14", "roles.qr.size"]);
    // A list item is named by its index.
    refused("{\"screens\": {\"cs\": [\"a\",\n \"b\" \"c\"]}}", &["line 2", "screens.cs[1]"]);
    refused("{\"a\": [1, [2, 3 4]]}", &["a[1][1]"]);
    // A duplicate key, named where the second one is closed.
    refused("{\n\"roles\": {\n\"qr\": 1,\n\"qr\": 2}}", &["line 4", "roles.qr", "already exists"]);
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
            assert_eq!(at.key, "k.s3cret");
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
    // Only the digits before the point were parsed, so these read as 1 and 1.
    refused("(u8|1.5)", &["whole number", "1.5"]);
    refused("(i32|-2.5)", &["whole number"]);
    refused("(u16|1e2)", &["whole number", "1e2"]);
    refused("{\"n\": (u64|3.0)}", &["whole number", "key n"]);
    // A float kind keeps its fraction, and hex digits are not an exponent.
    assert_eq!(read("(f64|1.5)"), Dat::F64(Float64(1.5)));
    assert_eq!(read("(u8|0xff)"), Dat::U8(255));
    assert_eq!(read("(u16|0x1e5)"), Dat::U16(485));
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
        "{\"a\": 1,, \"b\": 2}",
    ] {
        refused(s, &["':'"]);
    }
    // The separators in their places, and inside strings and notes, read.
    assert_eq!(read("(u8|1)"), Dat::U8(1));
    assert_eq!(read("{\"a|b\": \"c:d|e\", ! n|: ! \"f\": (u8|2)}").to_string(),
        "{ \"a|b\": \"c:d|e\", \"f\" !n|: !: (u8|2)}");
    read("{(u8|1): 2, [3]: 4}");
}
