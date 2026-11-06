//! A vhost's `path_headers` reach the responses to the paths each entry names and no others,
//! after `response_headers` and replacing a field of the same name, so a frame's shell can run
//! under a policy of its own beside a strict page. A path is matched as the static router reads
//! it, and one ending in `/` covers its directory by whole segments. The key is read from config,
//! and a path that is not absolute and normal, or a header the vhost could not set, is refused.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

mod vhost_rig;

use vhost_rig::{
    Rig,
    Scratch,
    Site,
};

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_jdat::prelude::*;
use oxedyne_fe2o3_steel::srv::cfg::{
    PathHeaders,
    VhostConfig,
};

const HOST:     &str = "frames.test";
const PAGE:     &str = "script-src 'self'";
const SHELL:    &str = "default-src 'none'; script-src 'unsafe-inline'";
const DOCS:     &str = "noindex";

fn entry(path: &str, headers: &[(&str, &str)]) -> PathHeaders {
    PathHeaders {
        path:       path.to_string(),
        headers:    headers.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
    }
}

#[test]
fn a_path_gets_its_own_headers_and_no_other_does() -> Outcome<()> {
    let web = res!(Scratch::new("path_headers"));
    res!(web.put("index.html", "<p>home</p>"));
    res!(web.put("frame.html", "<p>shell</p>"));
    res!(web.put("framex.html", "<p>other</p>"));
    res!(web.dir("docs"));
    res!(web.put("docs/a.html", "<p>a</p>"));
    res!(web.put("docsx.html", "<p>x</p>"));
    let mut site = Site::new(HOST, &web.0);
    site.response_headers = vec![(fmt!("content-security-policy"), fmt!("{}", PAGE))];
    site.path_headers = vec![
        entry("/frame.html", &[("content-security-policy", SHELL)]),
        entry("/docs/", &[("x-robots-tag", DOCS)]),
    ];
    let rig = res!(Rig::new(vec![site]));
    let runtime = res!(vhost_rig::runtime());
    runtime.block_on(async {
        for (path, csp, robots) in [
            ("/frame.html",         SHELL,  None),
            ("//frame.html",        SHELL,  None),
            ("/docs/../frame.html", SHELL,  None),
            ("/framex.html",        PAGE,   None),
            ("/",                   PAGE,   None),
            ("/index.html",         PAGE,   None),
            ("/docs/a.html",        PAGE,   Some(DOCS)),
            ("/docsx.html",         PAGE,   None),
            ("/frame.html/missing", PAGE,   None),
        ] {
            let r = res!(rig.fetch(HOST, "GET", path, &[], "").await);
            assert_eq!(r.field("content-security-policy").as_deref(), Some(csp),
                "GET {}: {:?}", path, r);
            let n = r.head.lines().skip(1)
                .filter(|l| l.to_lowercase().starts_with("content-security-policy:")).count();
            assert_eq!(n, 1, "GET {} doubled the policy: {:?}", path, r);
            assert_eq!(r.field("x-robots-tag").as_deref(), robots, "GET {}: {:?}", path, r);
        }
        Ok(())
    })
}

#[test]
fn a_path_entry_is_matched_as_the_router_reads_it() {
    let shell = entry("/frame.html", &[]);
    for (req, want) in [
        ("/frame.html", true), ("//frame.html", true), ("/./frame.html", true),
        ("/a/../frame.html", true), ("/frame.htm", false), ("/sub/frame.html", false),
    ] {
        assert_eq!(shell.matches(req), want, "{}", req);
    }
    let dir = entry("/docs/", &[]);
    for (req, want) in [
        ("/docs", true), ("/docs/", true), ("/docs/a/b.html", true), ("/docsx", false),
        ("/", false),
    ] {
        assert_eq!(dir.matches(req), want, "{}", req);
    }
    assert!(entry("/", &[]).matches("/anything"));
}

fn vhost(path_headers: &str) -> Outcome<VhostConfig> {
    let text = fmt!("{{\"hostnames\": [\"a.test\"], \"path_headers\": {}}}", path_headers);
    let dat = res!(Dat::decode_string(&text));
    match dat {
        Dat::Map(m) => VhostConfig::from_datmap(&m),
        _           => Err(err!("Not a map."; Invalid, Input)),
    }
}

#[test]
fn the_key_is_read_and_checked() -> Outcome<()> {
    let v = res!(vhost("[{\"path\": \"/frame.html\", \"headers\": {\"Content-Security-Policy\": \"x\"}}]"));
    assert_eq!(v.path_headers, vec![entry("/frame.html", &[("content-security-policy", "x")])]);
    assert!(res!(vhost("[]")).path_headers.is_empty());
    let none = res!(Dat::decode_string("{\"hostnames\": [\"a.test\"]}"));
    if let Dat::Map(m) = none {
        assert!(res!(VhostConfig::from_datmap(&m)).path_headers.is_empty());
    }
    for bad in [
        "{}",
        "[\"/frame.html\"]",
        "[{\"headers\": {}}]",
        "[{\"path\": \"/a\"}]",
        "[{\"path\": \"frame.html\", \"headers\": {}}]",
        "[{\"path\": \"//frame.html\", \"headers\": {}}]",
        "[{\"path\": \"/a/../b\", \"headers\": {}}]",
        "[{\"path\": \"/a\", \"headers\": {\"set-cookie\": \"s=1\"}}]",
        "[{\"path\": \"/a\", \"headers\": {\"x-a\": \"one\\r\\nx-b: 2\"}}]",
        "[{\"path\": \"/a\", \"headers\": [\"x-a\"]}]",
    ] {
        assert!(vhost(bad).is_err(), "{} must be refused", bad);
    }
    Ok(())
}
