//! `Dat::jdat_to_json` and `Dat::decode_jdat_strict`: the JSON-shaped jdat a settings file is
//! written in, read once, in Rust, for the browser (the wasm core exports `jdat_to_json`).
//!
//! The general text decoder cannot be that reader, because it is lenient by design. Found while
//! building the Oxegen settings reader (D-20261005-12): `{"size": 29x.4}` reads as the string
//! `"29x.4"`, a map left open at the end of the text returns its inner map and drops the rest, and
//! a comment never closed becomes a key. A settings file is edited by hand, so each of those must
//! be an error that names the line and the key. What comes out is held to `serde_json`, which did
//! not come from here and stands for the browser's `JSON.parse`.

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_jdat::{
    prelude::*,
    bdat::DecodeLimits,
};

use serde_json::Value;


const SHAPE: &str = include_str!("fixtures/ui_settings_shape.jdat");

fn to_json(s: &str) -> Outcome<String> {
    Dat::jdat_to_json(s, &DecodeLimits::text())
}

/// The JSON `s` becomes, read back by an independent reader.
fn browser(s: &str) -> Outcome<Value> {
    let json = res!(to_json(s));
    match serde_json::from_str::<Value>(&json) {
        Ok(v) => Ok(v),
        Err(e) => Err(err!("jdat_to_json wrote text that serde_json refuses ({}): {}", e, json;
            Invalid, Input, Decode)),
    }
}

/// Refuse `s`, and require the message to hold every word in `words`.
fn refused(s: &str, words: &[&str]) {
    match to_json(s) {
        Ok(json) => panic!("{:?} should be refused, and became {}", s, json),
        Err(e) => {
            let msg = e.plain();
            for w in words {
                assert!(msg.contains(w), "the refusal of {:?} should hold {:?}, and reads: {}",
                    s, w, msg);
            }
        },
    }
}

#[test]
fn test_shape_reads_as_json_00() -> Outcome<()> {
    let v = res!(browser(SHAPE));
    assert_eq!(v["copy"]["identity.create"], "Create Oxenym");
    assert_eq!(v["copy"]["identity.remaining"],
        "{real} real and {vuser} virtual oxenyms left, for life");
    assert_eq!(v["copy"]["common.close"], "Close");
    assert_eq!(v["roles"]["popup"]["mark"], "none");
    assert_eq!(v["roles"]["qr"]["size"], 296.4);
    assert_eq!(v["roles"]["identity-card"]["photo"], "avatar");
    assert_eq!(v["kinds"]["oxenym"]["bot"]["icon"], "bot_icon");
    assert_eq!(v["screens"]["current_status"].as_array().map(|a| a.len()), Some(6));
    assert_eq!(v["screens"]["current_status"][5], "scan");
    Ok(())
}

#[test]
fn test_members_keep_file_order_00() -> Outcome<()> {
    // The editor draws a field per key "grouped as the file is", so order is data.
    let json = res!(to_json(SHAPE));
    let at = |k: &str| json.find(k).unwrap_or(usize::MAX);
    assert!(at("\"real\"") < at("\"mask\"") && at("\"mask\"") < at("\"org\"")
        && at("\"org\"") < at("\"bot\""), "oxenym kinds out of file order in {}", json);
    assert!(at("\"copy\"") < at("\"roles\"") && at("\"roles\"") < at("\"kinds\"")
        && at("\"kinds\"") < at("\"screens\""), "sections out of file order");
    assert!(json.find("\"identity.current_title\"").unwrap_or(usize::MAX)
        < json.find("\"identity.create\"").unwrap_or(0), "copy keys out of file order");
    Ok(())
}

#[test]
fn test_comments_are_dropped_everywhere_00() -> Outcome<()> {
    let plain = r#"{"a":{"b":[1,2],"c":"x"},"d":[],"e":{}}"#;
    let commented = "! top ! { ! after brace !\n\t\"a\" ! before colon ! : ! after colon ! {\n\
        \"b\": [ ! in list ! 1 ! after one ! , # hash comment # 2 ! before bracket ! ] ! after \
        list !, \"c\": \"x\" # before brace # }, \"d\": [ ! empty ! ], \"e\": { # empty # } \
        ! last member ! } ! end !";
    assert_eq!(res!(to_json(commented)), plain);
    Ok(())
}

#[test]
fn test_comment_text_is_free_00() -> Outcome<()> {
    // Quotes, braces, brackets, colons, commas and the other marker are all inert inside a comment.
    let s = "{ ! was \"logo\": { [ 1, 2 ] } # not a close ! \"a\": 1, # a ! is inert here # \"b\": 2 }";
    assert_eq!(res!(to_json(s)), r#"{"a":1,"b":2}"#);
    // A marker inside a string is not a comment.
    assert_eq!(res!(to_json(r##"{"a": "# not ! a comment #"}"##)), r##"{"a":"# not ! a comment #"}"##);
    Ok(())
}

#[test]
fn test_every_value_kind_00() -> Outcome<()> {
    let s = r#"{
        "s": "plain", "empty": "", "esc": "q\"b\\s\/n\nt\tu\u00e9", "raw": "é — 日本 😀",
        "int": 12, "neg": -3, "zero": 0, "float": 296.4, "negf": -0.5, "exp": 1.5e3, "bigexp": 2E-2,
        "t": true, "f": false, "nothing": null,
        "emptyobj": {}, "emptylist": [], "nest": [[ "a", ["b"] ], { "k": [ 1, { "z": null } ] }],
    }"#;
    let v = res!(browser(s));
    assert_eq!(v["s"], "plain");
    assert_eq!(v["empty"], "");
    assert_eq!(v["esc"], "q\"b\\s/n\nt\tu\u{e9}");
    assert_eq!(v["raw"], "é — 日本 😀");
    assert_eq!(v["int"], 12);
    assert_eq!(v["neg"], -3);
    assert_eq!(v["zero"], 0);
    assert_eq!(v["float"], 296.4);
    assert_eq!(v["negf"], -0.5);
    assert_eq!(v["exp"], 1500.0);
    assert_eq!(v["bigexp"], 0.02);
    assert_eq!(v["t"], true);
    assert_eq!(v["f"], false);
    assert!(v["nothing"].is_null());
    assert_eq!(v["emptyobj"], serde_json::json!({}));
    assert_eq!(v["emptylist"], serde_json::json!([]));
    assert_eq!(v["nest"][1]["k"][1]["z"], Value::Null);
    assert_eq!(v["nest"][0][1][0], "b");
    // Whole numbers stay integers and fractions stay fractions.
    let json = res!(to_json(r#"{"i": 12, "f": 296.4}"#));
    assert_eq!(json, r#"{"i":12,"f":296.4}"#);
    Ok(())
}

#[test]
fn test_trailing_commas_00() -> Outcome<()> {
    assert_eq!(res!(to_json(r#"{"a": [1, 2,], "b": {"c": 3,},}"#)), r#"{"a":[1,2],"b":{"c":3}}"#);
    // A comma with nothing before it is not a trailing one.
    refused("{,}", &["line 1"]);
    refused("[,]", &["line 1"]);
    refused("{\"a\": 1,, \"b\": 2}", &["line 1"]);
    Ok(())
}

#[test]
fn test_kind_annotations_take_the_value_00() -> Outcome<()> {
    let s = r#"{"a": (u8|1), "b": (i16|-5), "c": (f64|1.5), "d": (str|"x"), "e": (u32|70000),
        "g": (f32 | 2), "h": ! note ! (u64|9007199254740991)}"#;
    assert_eq!(res!(to_json(s)),
        r#"{"a":1,"b":-5,"c":1.5,"d":"x","e":70000,"g":2,"h":9007199254740991}"#);
    Ok(())
}

#[test]
fn test_kind_annotations_are_checked_00() -> Outcome<()> {
    refused("{\n\"n\": (u8|300)}", &["line 2", "key n", "u8"]);
    refused("{\n\"n\": (u8|-1)}", &["line 2", "key n", "u8"]);
    refused("{\n\"n\": (u8|1.5)}", &["line 2", "key n", "u8"]);
    refused("{\n\"n\": (str|3)}", &["line 2", "key n", "str"]);
    refused("{\n\"n\": (f64|\"x\")}", &["line 2", "key n", "f64"]);
    refused("{\n\"n\": (zzz|1)}", &["line 2", "key n", "zzz"]);
    refused("{\n\"n\": (list|1)}", &["line 2", "key n", "list"]);
    refused("{\n\"n\": (u8 1)}", &["line 2", "key n"]);
    refused("{\n\"n\": (u8|1}", &["line 2", "key n"]);
    refused("{\n\"n\": (u8|1", &["line 2", "key n"]);
    Ok(())
}

#[test]
fn test_error_names_line_and_key_00() -> Outcome<()> {
    // The brief's case: a bad value, three levels down, on line 4.
    refused("{\n\t\"roles\": {\n\t\t\"qr\": {\n\t\t\t\"size\": 29x.4,\n\t\t},\n\t},\n}",
        &["line 4", "roles.qr.size"]);
    // Column too, counting characters and not bytes: the value starts after two tabs and a key.
    refused("{\n\t\"a\": 12x}", &["line 2", "column 9"]);
    refused("{\n\t\"é日\": 12x}", &["line 2", "column 10"]);
    // An array element is named by its index.
    refused("{\n\"screens\": {\"cs\": [\"a\",\n \"b\" \"c\"]}}", &["line 3", "screens.cs[1]"]);
    Ok(())
}

#[test]
fn test_malformed_files_are_refused_00() -> Outcome<()> {
    // A word with no quotes: the general decoder reads it as a string.
    refused("{\n\"a\": yes}", &["line 2", "key a"]);
    // A key with no quotes.
    refused("{\n\"a\": 1,\n b: 2}", &["line 3"]);
    // A single-quoted string.
    refused("{\n\"a\": 'x'}", &["line 2", "key a"]);
    // A missing colon after a name.
    refused("{\n\"a\": \"b\",\n\"c\" \"d\"}", &["line 3", "key c", "':'"]);
    // A missing comma between members.
    refused("{\n\"a\": 1\n\"b\": 2}", &["line 3", "key a"]);
    // A map left open: the general decoder returns the inner map and drops the rest.
    refused("{\n\"a\": {\n\"size\": 1,\n\n}", &["line 5", "never closed"]);
    refused("{\n\"a\": [1,\n2", &["line 3", "never closed", "key a"]);
    // A string left open.
    refused("{\n\"a\": \"never", &["line 2", "key a", "never closed"]);
    // A comment left open: the general decoder reads it into a key.
    refused("{\n\"a\": 1, ! never closed \n\"b\": 2}", &["line 2", "comment", "never closed"]);
    // Anything after the root.
    refused("{\n\"a\": 1}\n}", &["line 3"]);
    refused("{\"a\": 1} {\"b\": 2}", &["line 1"]);
    // Nothing at all, and not an object or list at the root is still a value, so only empty is wrong.
    refused("", &["line 1"]);
    refused("   \n ! only a comment !  ", &["line 2"]);
    // Bad numbers and bad escapes.
    refused("{\n\"a\": 01}", &["line 2", "key a"]);
    refused("{\n\"a\": +1}", &["line 2", "key a"]);
    refused("{\n\"a\": 1.}", &["line 2", "key a"]);
    refused("{\n\"a\": \"\\x\"}", &["line 2", "key a"]);
    refused("{\n\"a\": \"\\ud800\"}", &["line 2", "key a"]);
    // A raw control character in a string.
    refused("{\n\"a\": \"x\ty\"}", &["line 2", "key a"]);
    Ok(())
}

#[test]
fn test_duplicate_key_names_both_lines_00() -> Outcome<()> {
    refused("{\n\"roles\": {\n\"qr\": 1,\n\"qr\": 2}}", &["line 4", "roles.qr", "line 3"]);
    Ok(())
}

#[test]
fn test_numbers_a_browser_cannot_hold_are_refused_00() -> Outcome<()> {
    refused("{\n\"n\": 9007199254740993}", &["line 2", "key n", "2^53"]);
    refused("{\n\"n\": -9007199254740993}", &["line 2", "key n", "2^53"]);
    refused("{\n\"n\": 1e999}", &["line 2", "key n"]);
    assert_eq!(res!(to_json(r#"{"n": 9007199254740991}"#)), r#"{"n":9007199254740991}"#);
    Ok(())
}

#[test]
fn test_limits_apply_00() -> Outcome<()> {
    let deep = fmt!("{}{}", "[".repeat(100_000), "]".repeat(100_000));
    assert!(to_json(&deep).is_err(), "a hundred thousand levels must be refused, not recursed");
    let tiny = DecodeLimits::new(8, 4);
    assert!(Dat::jdat_to_json("[1,2,3]", &tiny).is_err(), "seven bytes is past four");
    let flat = DecodeLimits::new(3, 1024);
    res!(Dat::jdat_to_json("[[1]]", &flat));
    assert!(Dat::jdat_to_json("[[[1]]]", &flat).is_err(), "depth 4 is past 3");
    Ok(())
}

#[test]
fn test_plain_json_reads_as_itself_00() -> Outcome<()> {
    // Every JSON document is a settings document: the canonical text of the shape is a fixed point.
    let once = res!(to_json(SHAPE));
    assert_eq!(res!(to_json(&once)), once);
    // And the strict JSON reader and this one agree on the value.
    let doc = r#"{"b":[1,"x",{"c":null}],"a":{"d":true,"e":-4}}"#;
    let a = res!(Dat::decode_json_strict(doc, &DecodeLimits::text()));
    let b = res!(Dat::decode_jdat_strict(doc, &DecodeLimits::text()));
    assert_eq!(res!(a.json_canonical()), res!(b.json_canonical()));
    assert_eq!(res!(to_json(doc)), doc);
    Ok(())
}

#[test]
fn test_decode_gives_ordered_map_00() -> Outcome<()> {
    let d = res!(Dat::decode_jdat_strict_ordered(SHAPE, &DecodeLimits::text()));
    assert!(matches!(d, Dat::OrdMap(_)), "the root should keep its member order");
    let m = res!(Dat::decode_jdat_strict("{\"b\": 1, \"a\": 2}", &DecodeLimits::text()));
    assert!(matches!(m, Dat::Map(_)));
    Ok(())
}
