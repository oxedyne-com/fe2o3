//! A vhost's `response_headers` reach every response it sends, a static file, a 404 and a
//! redirect alike, each as the one field of its name: Steel's baseline `Referrer-Policy` is
//! replaced, not joined, since a browser given two takes the laxer. A vhost that sets none still
//! gets the baseline. The key is read from config with its names lower-cased, and a name that is
//! not a token, a value with a line break, or a field that frames the message is refused.
//!
//! The server's HSTS, like the baseline, fills only an empty field: a handler that sends its own
//! `Strict-Transport-Security` is not given a second one beside it.
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
use oxedyne_fe2o3_net::http::{
    fields::{
        HeaderFieldValue,
        HeaderFields,
        HeaderName,
    },
    header::HttpMethod,
    loc::HttpLocator,
    msg::HttpMessage,
    status::HttpStatus,
};
use oxedyne_fe2o3_steel::srv::{
    api::ApiHandler,
    cfg::{
        ApiRoute,
        ServerConfig,
        VhostConfig,
    },
};

use std::{
    future::Future,
    pin::Pin,
    sync::Arc,
};

use tokio_rustls::rustls::ClientConfig;

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

const OWN_HSTS: &str = "max-age=60";

/// Answers with a policy of its own.
struct OwnHsts;

impl ApiHandler for OwnHsts {
    fn handle<'a>(
        &'a self,
        _route:         &'a ApiRoute,
        _method:        HttpMethod,
        _loc:           &'a HttpLocator,
        _body:          &'a [u8],
        _req_headers:   &'a HeaderFields,
        _tls_client:    &'a Option<Arc<ClientConfig>>,
        _id:            &'a str,
    ) -> Pin<Box<dyn Future<Output = Outcome<HttpMessage>> + Send + 'a>> {
        Box::pin(async move {
            let mut msg = HttpMessage::respond_with_text(HttpStatus::OK, "own");
            msg.header.fields.insert(
                HeaderName::StrictTransportSecurity,
                HeaderFieldValue::Generic(OWN_HSTS.to_string()),
                None,
            );
            Ok(msg)
        })
    }
}

#[test]
fn a_handlers_own_hsts_is_not_doubled() -> Outcome<()> {
    let web = res!(Scratch::new("resp_hsts"));
    res!(web.put("index.html", "<p>home</p>"));
    let mut site = Site::new(SET, &web.0);
    site.api_routes = vec![ApiRoute {
        path:           fmt!("/api/own"),
        upstream_host:  None,
        upstream_port:  None,
        upstream_path:  None,
        upstream_tls:   false,
        headers:        Vec::new(),
        handler:        Some(fmt!("own_hsts")),
        config:         Vec::new(),
    }];
    site.handlers.register("own_hsts", OwnHsts);
    let mut cfg = ServerConfig::default();
    cfg.hsts_max_age_secs = 31_536_000;
    let rig = res!(Rig::with_config(vec![site], cfg));
    let runtime = res!(vhost_rig::runtime());
    runtime.block_on(async {
        let r = res!(rig.fetch(SET, "GET", "/api/own", &[], "").await);
        assert_eq!(r.body, "own", "{:?}", r);
        assert_eq!(count(&r, "strict-transport-security"), 1, "{:?}", r);
        assert_eq!(r.field("strict-transport-security").as_deref(), Some(OWN_HSTS), "{:?}", r);
        // A response that sets none is given the server's, once.
        let r = res!(rig.fetch(SET, "GET", "/index.html", &[], "").await);
        assert_eq!(count(&r, "strict-transport-security"), 1, "{:?}", r);
        assert_eq!(r.field("strict-transport-security").as_deref(),
            Some("max-age=31536000; includeSubDomains"), "{:?}", r);
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
