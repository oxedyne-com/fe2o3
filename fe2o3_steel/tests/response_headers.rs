//! A vhost's `response_headers` reach every response it sends, a static file, a 404 and a
//! redirect alike, each as the one field of its name: Steel's baseline `Referrer-Policy` is
//! replaced, not joined, since a browser given two takes the laxer. A vhost that sets none still
//! gets the baseline. The key is read from config with its names lower-cased, and a name that is
//! not a token, a value with a line break, or a field that frames the message is refused.
//!
//! `ServerConfig::binds_loopback_only` says whether every listener a config opens is loopback.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

mod vhost_rig;

use vhost_rig::{
    Reply,
    Rig,
    Scratch,
    Site,
};

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_jdat::prelude::*;
use oxedyne_fe2o3_steel::srv::cfg::{
    ServerConfig,
    VhostConfig,
};

const SET:  &str = "private.test";
const BARE: &str = "plain.test";

fn count(r: &Reply, name: &str) -> usize {
    let want = fmt!("{}:", name);
    r.head.lines().skip(1).filter(|l| l.to_lowercase().starts_with(&want)).count()
}

#[test]
fn every_response_carries_the_vhost_headers_once() -> Outcome<()> {
    let web = res!(Scratch::new("resp_headers"));
    res!(web.put("index.html", "<p>home</p>"));
    res!(web.put("a.css", "p{}"));
    res!(web.dir("sub"));
    let mut set = Site::new(SET, &web.0);
    set.response_headers = vec![
        (fmt!("referrer-policy"),           fmt!("no-referrer")),
        (fmt!("x-robots-tag"),              fmt!("noindex")),
        (fmt!("content-security-policy"),   fmt!("default-src 'self'")),
    ];
    let bare = Site::new(BARE, &web.0);
    let rig = res!(Rig::new(vec![set, bare]));
    let runtime = res!(vhost_rig::runtime());
    runtime.block_on(async {
        for path in ["/", "/index.html", "/a.css", "/missing.html", "/sub"] {
            let r = res!(rig.fetch(SET, "GET", path, &[], "").await);
            assert_eq!(r.field("referrer-policy").as_deref(), Some("no-referrer"),
                "GET {}: {:?}", path, r);
            assert_eq!(count(&r, "referrer-policy"), 1, "GET {} doubled the field: {:?}", path, r);
            assert_eq!(r.field("x-robots-tag").as_deref(), Some("noindex"), "GET {}: {:?}", path, r);
            assert_eq!(r.field("content-security-policy").as_deref(), Some("default-src 'self'"),
                "GET {}: {:?}", path, r);
            // The rest of the baseline is untouched.
            assert_eq!(r.field("x-content-type-options").as_deref(), Some("nosniff"),
                "GET {}: {:?}", path, r);
        }
        let r = res!(rig.fetch(BARE, "GET", "/index.html", &[], "").await);
        assert_eq!(r.field("referrer-policy").as_deref(), Some("strict-origin-when-cross-origin"),
            "{:?}", r);
        assert_eq!(r.field("x-robots-tag"), None, "{:?}", r);
        Ok(())
    })
}

fn vhost(headers: &str) -> Outcome<VhostConfig> {
    let text = fmt!("{{\"hostnames\": [\"a.test\"], \"response_headers\": {}}}", headers);
    let dat = res!(Dat::decode_string(&text));
    match dat {
        Dat::Map(m) => VhostConfig::from_datmap(&m),
        _           => Err(err!("Not a map."; Invalid, Input)),
    }
}

#[test]
fn the_key_is_read_and_checked() -> Outcome<()> {
    let v = res!(vhost("{\"X-Robots-Tag\": \"noindex\"}"));
    assert_eq!(v.response_headers, vec![(fmt!("x-robots-tag"), fmt!("noindex"))]);
    assert!(res!(vhost("{}")).response_headers.is_empty());
    for bad in [
        "{\"bad name\": \"x\"}",
        "{\"x-a\": \"one\\r\\nSet-Cookie: s=1\"}",
        "{\"Content-Length\": \"3\"}",
        "{\"set-cookie\": \"s=1\"}",
        "{\"x-a\": 1}",
        "[\"x-a\"]",
    ] {
        assert!(vhost(bad).is_err(), "{} must be refused", bad);
    }
    Ok(())
}

#[test]
fn loopback_only_is_the_server_address() -> Outcome<()> {
    let mut cfg = ServerConfig::default();
    for (addr, want) in [("127.0.0.1", true), ("::1", true), ("0.0.0.0", false),
        ("192.168.1.2", false)]
    {
        cfg.server_address = addr.to_string();
        assert_eq!(res!(cfg.binds_loopback_only()), want, "{}", addr);
    }
    cfg.server_address = fmt!("localhost");
    assert!(cfg.binds_loopback_only().is_err());
    Ok(())
}
