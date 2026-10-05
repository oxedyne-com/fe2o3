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
};

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
    assert_eq!(read("[1, 2,]").to_string(), "[1, 2]");
    assert!(Dat::decode_string("{\"n\": (u8|300)}").is_err());
    assert!(Dat::decode_string("{\"n\": (zzz|1)}").is_err());
    read("{\"a\": 1, # a note\n \"b\": 2}");
}
