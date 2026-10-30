use oxedyne_fe2o3_core::file::{
    stamp,
    stamped,
};


const PREFIX: &str = "# ore-unit";

/// **A stamp detects any changed byte.** A stamped text matches its stamp; changing, adding or
/// removing any one byte, after the line or in it, makes it a hand edit; a text whose first line is
/// not the prefix and a space carries no stamp at all.
#[test]
fn a_stamp_detects_any_changed_byte() {
    let body = "[Service]\nMemoryMax=1536M\nNice=10\n";
    let text = stamp(body, PREFIX);
    assert!(text.starts_with("# ore-unit sha256:"), "{}", text);
    assert_eq!(text.lines().next().map(|l| l.len()), Some(PREFIX.len() + 8 + 64));
    assert!(text.ends_with(body));
    assert_eq!(stamped(&text, PREFIX), Some(true));
    // The digest is SHA-256 of the body alone: the empty body's is the published one.
    assert_eq!(
        stamp("", PREFIX),
        "# ore-unit sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855\n",
    );
    // Every byte, flipped in turn, line and body alike.
    let bytes = text.as_bytes();
    for i in PREFIX.len() + 1..bytes.len() {
        let mut b = bytes.to_vec();
        b[i] = match b[i] {
            b'0'    => b'1',
            _       => b'0',
        };
        let changed = String::from_utf8_lossy(&b).to_string();
        assert_eq!(stamped(&changed, PREFIX), Some(false), "byte {} changed and still matched", i);
    }
    // Added and removed bytes.
    assert_eq!(stamped(&fmt_plus(&text, "# by hand\n"), PREFIX), Some(false));
    assert_eq!(stamped(&text[..text.len() - 1], PREFIX), Some(false));
    let line = text.lines().next().unwrap_or("");
    assert_eq!(stamped(line, PREFIX), Some(false));
    // No stamp.
    assert_eq!(stamped(body, PREFIX), None);
    assert_eq!(stamped("", PREFIX), None);
    assert_eq!(stamped(&text.replacen("# ore-unit ", "# ore-unitx", 1), PREFIX), None);
    assert_eq!(stamped(&text, "# other"), None);
}

fn fmt_plus(a: &str, b: &str) -> String {
    let mut s = a.to_string();
    s.push_str(b);
    s
}
