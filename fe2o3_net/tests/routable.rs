//! Which addresses a server may connect to on a user's say-so: the special-purpose ranges that are
//! neither private nor documentation by the usual names.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use oxedyne_fe2o3_net::addr::is_publicly_routable;

use std::net::IpAddr;

fn routable(s: &str) -> bool {
    is_publicly_routable(&s.parse::<IpAddr>().expect("test address"))
}

#[test]
fn test_special_purpose_ranges_are_refused_00() {
    for s in [
        "64:ff9b:1::a00:1",                     // local-use NAT64 carrying 10.0.0.1
        "fec0::1",                              // site-local
        "3fff::1",                              // documentation, RFC 9637
        "2001:0:4136:e378:8000:63bf:3fff:fdd2", // Teredo
        "2001:2::1",                            // benchmarking
        "192.0.0.8",                            // IETF protocol assignments
        "0.1.2.3",                              // "this network"
    ] {
        assert!(!routable(s), "{} should be refused", s);
    }
    for s in ["2606:4700::1111", "2001:4860:4860::8888", "193.0.0.1", "1.0.0.1", "2002:808:808::1"] {
        assert!(routable(s), "{} should be allowed", s);
    }
}
