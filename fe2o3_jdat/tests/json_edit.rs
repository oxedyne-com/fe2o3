//! `JsonObjectText`, the editor of one JSON object's text in place: every byte outside the members
//! it is asked to change comes back as it was, and text that is not one JSON object is refused.

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_jdat::string::json_edit::JsonObjectText;


fn edit(s: &str) -> JsonObjectText {
    match JsonObjectText::new(s) {
        Ok(o)   => o,
        Err(e)  => panic!("{:?} must read as one JSON object: {}", s, e),
    }
}

#[test]
fn test_set_replaces_only_the_value_00() -> Outcome<()> {
    let mut o = edit(r#"{"s":"x", "n":1,  "a":[1,{"b":"}"}]}"#);
    res!(o.set("s", "\"y\""));
    assert_eq!(o.as_str(), r#"{"s":"y", "n":1,  "a":[1,{"b":"}"}]}"#);
    res!(o.set("a", "null"));
    assert_eq!(o.as_str(), r#"{"s":"y", "n":1,  "a":null}"#);
    Ok(())
}

#[test]
fn test_set_adds_a_missing_member_at_the_end_00() -> Outcome<()> {
    let mut o = edit(r#"{"a":1 }"#);
    res!(o.set("z", "0"));
    assert_eq!(o.as_str(), r#"{"a":1,"z":0 }"#);
    let mut e = edit("{}");
    res!(e.set("z", "0"));
    assert_eq!(e.as_str(), r#"{"z":0}"#);
    // A key that needs escaping is written escaped, and found again by what it says.
    res!(e.set("q\"k", "true"));
    assert_eq!(e.as_str(), r#"{"z":0,"q\"k":true}"#);
    assert_eq!(e.get("q\"k"), Some("true"));
    Ok(())
}

#[test]
fn test_remove_takes_the_member_and_one_comma_00() -> Outcome<()> {
    let raw = r#"{"a":1, "s":"{x}", "n":2}"#;
    let mut first = edit(raw);
    assert!(res!(first.remove("a")));
    assert_eq!(first.as_str(), r#"{"s":"{x}", "n":2}"#);
    let mut mid = edit(raw);
    assert!(res!(mid.remove("s")));
    assert_eq!(mid.as_str(), r#"{"a":1, "n":2}"#);
    let mut last = edit(raw);
    assert!(res!(last.remove("n")));
    assert_eq!(last.as_str(), r#"{"a":1, "s":"{x}"}"#);
    let mut only = edit(r#"{ "z":0 }"#);
    assert!(res!(only.remove("z")));
    assert_eq!(only.as_str(), "{  }");
    assert!(!res!(only.remove("z")), "a key not there is no fault and changes nothing");
    assert_eq!(only.as_str(), "{  }");
    Ok(())
}

#[test]
fn test_braces_and_quotes_inside_strings_are_not_structure_00() -> Outcome<()> {
    let mut o = edit(r#"{"note":"a } b, \"c\": {d", "k":"v"}"#);
    assert_eq!(o.get("k"), Some("\"v\""));
    res!(o.set("k", "\"w\""));
    assert_eq!(o.as_str(), r#"{"note":"a } b, \"c\": {d", "k":"w"}"#);
    assert_eq!(o.keys(), vec![fmt!("note"), fmt!("k")]);
    Ok(())
}

#[test]
fn test_a_nested_member_of_the_same_name_is_left_alone_00() -> Outcome<()> {
    let mut o = edit(r#"{"in":{"k":1,"deep":[{"k":2}]},"k":3}"#);
    res!(o.set("k", "4"));
    assert_eq!(o.as_str(), r#"{"in":{"k":1,"deep":[{"k":2}]},"k":4}"#);
    assert!(res!(o.remove("k")));
    assert_eq!(o.as_str(), r#"{"in":{"k":1,"deep":[{"k":2}]}}"#);
    Ok(())
}

#[test]
fn test_layout_and_line_endings_are_kept_00() -> Outcome<()> {
    let mut o = edit("{\r\n  \"a\": 1,\r\n  \"b\": \"x\"\r\n}\r\n");
    res!(o.set("a", "2"));
    assert_eq!(o.as_str(), "{\r\n  \"a\": 2,\r\n  \"b\": \"x\"\r\n}\r\n");
    Ok(())
}

#[test]
fn test_unknown_members_survive_an_edit_byte_for_byte_00() -> Outcome<()> {
    let raw = r#"{"id":"7","x-new":{"u":[1.50e3,"é"]},"rel":"Cites","t":-0.0}"#;
    let mut o = edit(raw);
    res!(o.set("rel", "\"cites\""));
    res!(o.set("share", "true"));
    assert!(res!(o.remove("share")));
    assert_eq!(o.as_str(), raw.replace("Cites", "cites"));
    Ok(())
}

#[test]
fn test_what_is_not_one_object_is_refused_00() {
    for bad in [
        "",
        "[1]",
        "\"s\"",
        r#"{"a":1} trailing"#,
        r#"{"a":1}{"b":2}"#,
        r#"{"a":"open"#,
        r#"{"a":1,}"#,
        r#"{a:1}"#,
        r#"{"a":01}"#,
        r#"{"a":tru}"#,
        r#"{"a":1,"a":2}"#,     // a member named twice has no one meaning to edit
        "{\"a\":\"\u{1}\"}",    // a raw control character in a string
    ] {
        assert!(JsonObjectText::new(bad).is_err(), "{:?} must be refused", bad);
    }
}

#[test]
fn test_a_value_that_is_not_one_json_value_is_refused_00() {
    let mut o = edit(r#"{"a":1}"#);
    for bad in ["", "1 2", "1,\"b\":2", "{", "'s'", "x"] {
        assert!(o.set("a", bad).is_err(), "{:?} must be refused as a value", bad);
        assert_eq!(o.as_str(), r#"{"a":1}"#, "a refused set changes nothing");
    }
}

#[test]
fn test_the_grammar_is_held_inside_values_too_00() {
    for bad in [
        r#"{"a":{"b":1,"b":2}}"#,   // a nested member named twice
        r#"{"a":"\ud800"}"#,        // half a surrogate pair
        r#"{"a":"\x"}"#,            // an escape JSON does not have
        r#"{"a":[1,]}"#,
        r#"{"a":1.}"#,
        r#"{"a":-}"#,
        r#"{"a":1e}"#,
    ] {
        assert!(JsonObjectText::new(bad).is_err(), "{:?} must be refused", bad);
    }
    assert!(JsonObjectText::new(r#"{"a":"😀","b":[-0.5e+3,true,false,null,{}]}"#).is_ok());
}

#[test]
fn test_nesting_past_the_limit_is_refused_00() {
    let deep = fmt!("{{\"a\":{}1{}}}", "[".repeat(100_000), "]".repeat(100_000));
    assert!(JsonObjectText::new(&deep).is_err(), "a hundred thousand levels must be refused");
}
