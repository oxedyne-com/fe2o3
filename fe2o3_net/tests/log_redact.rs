//! A credential's value never reaches a log, at trace level or any other.

use oxedyne_fe2o3_net::http::{
    fields::{
        HeaderFieldValue,
        HeaderName,
    },
    msg::HttpMessage,
    status::HttpStatus,
};

const SECRET: &str = "s3cr3t-token-value";

fn message() -> HttpMessage {
    let v = |s: &str| HeaderFieldValue::Generic(s.to_string());
    HttpMessage::new_response(HttpStatus::OK)
        .with_field(HeaderName::Cookie,             v(&format!("sid={}", SECRET)))
        .with_field(HeaderName::SetCookie,          v(&format!("sid={}; HttpOnly", SECRET)))
        .with_field(HeaderName::Authorization,      v(&format!("Bearer {}", SECRET)))
        .with_field(HeaderName::ProxyAuthorization, v(&format!("Basic {}", SECRET)))
        .with_field(HeaderName::from("x-trace-note"), v("plain-value"))
}

#[test]
fn credential_values_are_redacted() {
    let lines = message().header_log_lines();
    let all = lines.join("\n");
    assert!(!all.contains(SECRET), "a credential reached the log lines:\n{}", all);
    for name in ["cookie", "set-cookie", "authorization", "proxy-authorization"] {
        assert!(lines.iter().any(|l| l.contains(&format!("{}: [redacted", name))),
            "no redacted line for {}:\n{}", name, all);
    }
    // A field that is not a credential is still written in full.
    assert!(all.contains("x-trace-note: plain-value"), "{}", all);
}

#[test]
fn credential_names_are_classified() {
    assert!(HeaderName::Cookie.is_credential());
    assert!(HeaderName::SetCookie.is_credential());
    assert!(HeaderName::Authorization.is_credential());
    assert!(!HeaderName::Host.is_credential());
    assert!(!HeaderName::from("x-forwarded-for").is_credential());
}
