//! The cases rc's Opus QA of 2026-10-05 wrote against the text decoder (U0-5, D-20261005-12): each is
//! text that must be refused, or must read, and the fixture holds the expectation beside the text.
//! Every case the QA wrote now has an answer, the whitespace in a kind label (read as a space) and
//! `(f64|1e400)` (refused) among them, and the U0-8 cases for comments, a BOM and whole-number kinds.

use oxedyne_fe2o3_jdat::prelude::*;

#[test]
fn test_the_qa_cases_are_read_or_refused_00() {
    let text = include_str!("fixtures/jdat_qa_cases.json");
    let cases: serde_json::Value = serde_json::from_str(text).unwrap_or_default();
    let cases = cases.as_array().cloned().unwrap_or_default();
    assert!(cases.len() > 100, "the fixture holds {} cases", cases.len());
    let mut wrong = Vec::new();
    for (i, c) in cases.iter().enumerate() {
        let s = c["s"].as_str().unwrap_or_default();
        let want = c["want"].as_str().unwrap_or_default();
        let tag = c["tag"].as_str().unwrap_or_default();
        match (want, Dat::decode_string(s)) {
            ("read", Err(e))    => wrong.push(format!(
                "#{} {} {:?} should read, and was refused: {}", i, tag, s, e.plain())),
            ("refuse", Ok(d))   => wrong.push(format!(
                "#{} {} {:?} should be refused, and became {}", i, tag, s, d)),
            _ => (),
        }
    }
    for w in &wrong {
        println!("{}", w);
    }
    assert!(wrong.is_empty(), "{} of {} cases went the wrong way, the first being: {}",
        wrong.len(), cases.len(), wrong.first().cloned().unwrap_or_default());
}
