//! `Dat::jdat_to_json`: a jdat settings file written as plain JSON for a browser's `JSON.parse`
//! (the wasm core exports it; U0, D-20261005-12). It reads with `Dat::decode_string`'s rules, so
//! what the format allows by design stays allowed (a comment ends at its mark or at the end of
//! its line, trailing commas, `(kind|value)` annotations), and what is wrong is refused with the
//! line, column and key. What comes out is held to `serde_json`, which did not come from here
//! and stands for the browser's `JSON.parse`.

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_jdat::{
    prelude::*,
    bdat::DecodeLimits,
};

use serde_json::Value;


const SHAPE: &str = include_str!("fixtures/ui_settings_shape.jdat");

fn to_json(s: &str) -> Outcome<String> {
    Dat::jdat_to_json(s, &DecodeLimits::text(), true)
}

// The JSON `s` becomes, read back by an independent reader.
fn browser(s: &str) -> Outcome<Value> {
    let json = res!(to_json(s));
    match serde_json::from_str::<Value>(&json) {
        Ok(v)   => Ok(v),
        Err(e)  => Err(err!("jdat_to_json wrote text that serde_json refuses ({}): {}", e, json;
            Invalid, Input, Decode)),
    }
}

// Refuse `s`, and require the message to hold every word in `words`.
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
    // Order is the file's and not alphabetical: "z" is written before "a".
    assert_eq!(res!(to_json(r#"{"z": 1, "a": {"y": 2, "b": 3}, "m": [4]}"#)),
        r#"{"z":1,"a":{"y":2,"b":3},"m":[4]}"#);
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
    // Quotes, braces, brackets, colons, commas and the other mark are all inert inside a comment.
    let s = "{ ! was \"logo\": { [ 1, 2 ] } # not a close ! \"a\": 1, # a ! is inert here # \"b\": 2 }";
    assert_eq!(res!(to_json(s)), r#"{"a":1,"b":2}"#);
    // An apostrophe in prose does not open a string.
    assert_eq!(res!(to_json("{ ! the names are the gate's ! \"a\": 1 }")), r#"{"a":1}"#);
    // A mark inside a string is not a comment.
    assert_eq!(res!(to_json(r##"{"a": "# not ! a comment #"}"##)), r##"{"a":"# not ! a comment #"}"##);
    Ok(())
}

#[test]
fn test_a_comment_ends_at_its_mark_or_its_line_00() -> Outcome<()> {
    // A comment with no closing mark runs to the end of its line, and the next line is the file's.
    assert_eq!(res!(to_json("{\"a\": 1, ! to the end of the line\n\"b\": 2}")), r#"{"a":1,"b":2}"#);
    assert_eq!(res!(to_json("{\"a\": 1, # likewise\n\"b\": 2}")), r#"{"a":1,"b":2}"#);
    // A comment ends at its own mark, so what follows on the line is read.
    assert_eq!(res!(to_json("{ ! one ! \"a\": 1, ! two ! \"b\": 2 }")), r#"{"a":1,"b":2}"#);
    // At the end of the text a comment runs to the end, and nothing is left to read.
    assert_eq!(res!(to_json("{\"a\": 1} ! last")), r#"{"a":1}"#);
    assert_eq!(res!(to_json("{\"a\": 1}\n# last\n")), r#"{"a":1}"#);
    // A comment of several lines is one mark pair per line: a '#..#' across a line break is not
    // a comment, because its second line is read as the file.
    assert_eq!(res!(to_json("{ # one\n # two\n \"a\": 1 }")), r#"{"a":1}"#);
    refused("{ # one\n two # \"a\": 1 }", &["line 2"]);
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
    assert_eq!(res!(to_json(r#"{"i": 12, "f": 296.4}"#)), r#"{"i":12,"f":296.4}"#);
    // A root need not be a map.
    assert_eq!(res!(to_json(r#"[1, "a", null]"#)), r#"[1,"a",null]"#);
    assert_eq!(res!(to_json("\"x\"")), "\"x\"");
    Ok(())
}

#[test]
fn test_trailing_commas_00() -> Outcome<()> {
    assert_eq!(res!(to_json(r#"{"a": [1, 2,], "b": {"c": 3,},}"#)), r#"{"a":[1,2],"b":{"c":3}}"#);
    // A comma with nothing before it is not a trailing one.
    refused("{,}", &["line 1"]);
    refused("[,]", &["nothing", "line 1, column 2"]);
    refused("{\"a\": 1,, \"b\": 2}", &["nothing", "line 1, column 9"]);
    refused("[1,,2]", &["nothing", "line 1, column 4"]);
    Ok(())
}

#[test]
fn test_kind_annotations_take_the_value_00() -> Outcome<()> {
    let s = r#"{"a": (u8|1), "b": (i16|-5), "c": (f64|1.5), "d": (str|"x"), "e": (u32|70000),
        "g": (f32 | 2), "h": ! note ! (u64|9007199254740991)}"#;
    assert_eq!(res!(to_json(s)),
        r#"{"a":1,"b":-5,"c":1.5,"d":"x","e":70000,"g":2,"h":9007199254740991}"#);
    // The str kind takes whatever follows as text.
    assert_eq!(res!(to_json(r#"{"a": (str|3)}"#)), r#"{"a":"3"}"#);
    Ok(())
}

#[test]
fn test_kind_annotations_are_checked_00() -> Outcome<()> {
    refused("{\n\"n\": (u8|300)}", &["line 2", "key n"]);
    refused("{\n\"n\": (u8|-1)}", &["line 2", "key n"]);
    refused("{\n\"n\": (u8|1.5)}", &["line 2", "key n"]);
    refused("{\n\"n\": (f64|\"x\")}", &["line 2", "key n"]);
    refused("{\n\"n\": (zzz|1)}", &["line 2", "key n"]);
    refused("{\n\"n\": (u8|1}", &["line 2", "key n"]);
    refused("{\n\"n\": (u8|1", &["line 2", "key n"]);
    Ok(())
}

#[test]
fn test_error_names_line_and_key_00() -> Outcome<()> {
    // A bad value, three levels down, on line 4.
    refused("{\n\t\"roles\": {\n\t\t\"qr\": {\n\t\t\t\"size\": (u8|300),\n\t\t},\n\t},\n}",
        &["line 4", "roles.qr.size"]);
    // Column counts characters and not bytes, and a tab as one: the value starts after a tab,
    // a quoted name, a colon and a space.
    refused("{\n\t\"a\": (u8|300)}", &["line 2", "column "]);
    let wide = refused_text("{\n\t\"é日\": (u8|300)}");
    let narrow = refused_text("{\n\t\"ab\": (u8|300)}");
    assert_eq!(wide, narrow, "a multi-byte name must not move the column");
    // An array element is named by its index.
    refused("{\n\"screens\": {\"cs\": [\"a\",\n (u8|300)]}}", &["line 3", "screens.cs[1]"]);
    Ok(())
}

// The message of the refusal of `s`, with the name in the key, to compare columns.
fn refused_text(s: &str) -> String {
    match to_json(s) {
        Ok(json)    => panic!("{:?} should be refused, and became {}", s, json),
        Err(e)      => {
            let msg = e.plain();
            match msg.find("column ") {
                Some(i) => msg[i..].chars().take_while(|c| *c != ',' && *c != '.').collect(),
                None    => panic!("the refusal of {:?} names no column: {}", s, msg),
            }
        },
    }
}

#[test]
fn test_malformed_files_are_refused_00() -> Outcome<()> {
    // A missing colon after a name.
    refused("{\n\"a\": \"b\",\n\"c\" \"d\"}", &["line 3"]);
    // A missing comma between members.
    refused("{\n\"a\": 1\n\"b\": 2}", &["line 3"]);
    // A map or list left open: the decoder once returned the inner value and dropped the rest.
    refused("{\n\"a\": {\n\"size\": 1,\n\n}", &["closure"]);
    refused("{\n\"a\": [1,\n2", &["closure"]);
    // A string left open.
    refused("{\n\"a\": \"never", &["line 2"]);
    // Values parted by a space are two values, not one.
    refused("{\n\"a\": [1 2]}", &["line 2"]);
    refused("{\n\"a\": \"x\" \"y\"}", &["line 2"]);
    // Anything after the root but a comment.
    refused("{\n\"a\": 1}\n}", &["line 3"]);
    refused("{\"a\": 1} {\"b\": 2}", &["line 1"]);
    refused("{\"a\": 1} x", &["line 1"]);
    // Nothing at all, or nothing but a comment.
    refused("", &[]);
    refused("   \n ! only a comment !  ", &[]);
    // Bad escapes.
    refused("{\n\"a\": \"\\x\"}", &["line 2"]);
    refused("{\n\"a\": \"\\ud800\"}", &["line 2"]);
    Ok(())
}

#[test]
fn test_a_member_with_no_value_is_refused_00() -> Outcome<()> {
    refused("{\n\"a\": }", &["line 2"]);
    refused("{\n\"a\": 1,\n\"b\":\n}", &["line 4", "key b"]);
    Ok(())
}

#[test]
fn test_duplicate_key_is_refused_00() -> Outcome<()> {
    refused("{\n\"roles\": {\n\"qr\": 1,\n\"qr\": 2}}", &["line 4", "roles.qr"]);
    Ok(())
}

#[test]
fn test_numbers_a_browser_cannot_hold_are_refused_00() -> Outcome<()> {
    // These are found after the text is read, so they name the key and not the line.
    refused("{\n\"n\": 9007199254740993}", &["key 'n'", "2^53"]);
    refused("{\n\"n\": -9007199254740993}", &["key 'n'", "2^53"]);
    refused("{\n\"n\": (u64|18446744073709551615)}", &["key 'n'", "2^53"]);
    refused("{\"a\": {\"n\": [0, 1e999]}}", &["a.n[1]", "finite"]);
    assert_eq!(res!(to_json(r#"{"n": 9007199254740991}"#)), r#"{"n":9007199254740991}"#);
    assert_eq!(res!(to_json(r#"{"n": -9007199254740991}"#)), r#"{"n":-9007199254740991}"#);
    Ok(())
}

#[test]
fn test_js_safe_off_lets_large_whole_numbers_through_00() -> Outcome<()> {
    let limits = DecodeLimits::text();
    assert_eq!(res!(Dat::jdat_to_json("{\"n\": 9007199254740993}", &limits, false)),
        r#"{"n":9007199254740993}"#);
    assert_eq!(res!(Dat::jdat_to_json("{\"n\": (u64|18446744073709551615)}", &limits, false)),
        r#"{"n":18446744073709551615}"#);
    // A float that is not finite has no JSON form either way.
    assert!(Dat::jdat_to_json("{\"n\": 1e999}", &limits, false).is_err());
    Ok(())
}

#[test]
fn test_limits_apply_00() -> Outcome<()> {
    let deep = fmt!("{}{}", "[".repeat(100_000), "]".repeat(100_000));
    assert!(to_json(&deep).is_err(), "a hundred thousand levels must be refused, not recursed");
    let tiny = DecodeLimits::new(8, 4);
    assert!(Dat::jdat_to_json("[1,2,3]", &tiny, true).is_err(), "seven bytes is past four");
    let flat = DecodeLimits::new(3, 1024);
    res!(Dat::jdat_to_json("[[1]]", &flat, true));
    assert!(Dat::jdat_to_json("[[[1]]]", &flat, true).is_err(), "depth 4 is past 3");
    Ok(())
}

#[test]
fn test_plain_json_reads_as_itself_00() -> Outcome<()> {
    // Every JSON document is a settings document: the text of the shape is a fixed point.
    let once = res!(to_json(SHAPE));
    assert_eq!(res!(to_json(&once)), once);
    // And the independent reader agrees on the value, before and after.
    let doc = r#"{"b":[1,"x",{"c":null}],"a":{"d":true,"e":-4,"f":1.25}}"#;
    assert_eq!(res!(to_json(doc)), doc);
    let a: Value = match serde_json::from_str(doc) {
        Ok(v)   => v,
        Err(e)  => return Err(err!("serde_json refuses its own fixture: {}", e; Invalid, Input)),
    };
    assert_eq!(res!(browser(doc)), a);
    Ok(())
}

#[test]
fn test_a_kind_with_no_plain_form_is_refused_00() -> Outcome<()> {
    refused("{\n\"a\": (bin|1,2)}", &["line 2"]);
    Ok(())
}

#[test]
fn test_a_map_of_many_keys_is_read_in_linear_time_00() -> Outcome<()> {
    // The ordered map searched every key it held for each key it added: 40,000 keys took 10 s in
    // a release build.
    let mut s = String::from("{");
    for i in 0..30000 {
        s.push_str(&fmt!("\"k{}\": {}, ", i, i));
    }
    s.push('}');
    let t0 = std::time::Instant::now();
    let json = res!(to_json(&s));
    let took = t0.elapsed();
    assert!(json.len() > 300000, "the JSON is {} bytes", json.len());
    assert!(took.as_secs() < 5, "30,000 keys took {:?}", took);
    Ok(())
}

#[test]
fn test_a_comment_where_whitespace_could_be_changes_nothing_00() -> Outcome<()> {
    let plain = res!(to_json("{\"a\": 1, \"b\": [1, 2]}"));
    for s in [
        "{\"a\" # c\n: 1, \"b\": [1, 2]}",
        "{\"a\": 1 # c\n, \"b\": [1 # d\n, 2]}",
        "{\"a\": 1, \"b\": [1, 2] # c\n}",
        "# c\n{\"a\": 1, \"b\": [1, 2]}",
        "\u{feff}{\"a\": 1, \"b\": [1, 2]}",
    ] {
        assert_eq!(res!(to_json(s)), plain, "{:?}", s);
    }
    Ok(())
}
