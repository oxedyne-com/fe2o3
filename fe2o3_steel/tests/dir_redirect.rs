//! A directory asked for without its trailing slash is moved to the slash form, query string kept.
//!
//! Served in place, `/invitation` answered 200 with the directory's index, so the one page lived at
//! two addresses and its relative links (`reader.css`, `../fonts/x.woff2`) resolved against the
//! wrong directory when it was opened at the shorter one. Every static server answers a 301 to
//! `/invitation/` instead, and so does Steel now.
//!
//! One vhost over a web root holding a directory with an index, a nested one, a directory without
//! one, and a plain file goes through `handle_https` over TLS. What is held: the redirect and its
//! exact `Location`, with the query string as it arrived; no loop (the slash form is a 200); a
//! directory with no index is the 404 it always was, so no redirect points at nothing; a plain file
//! is not touched; and no request can make the `Location` leave the site, whether by a leading
//! `//`, a `..`, or a bare `.`.
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

const HOST: &str = "static.test";

#[test]
fn a_directory_without_its_slash_is_a_301_to_the_slash_form() -> Outcome<()> {
    let web = res!(Scratch::new("dir_redirect"));
    res!(web.put("index.html", "<p>home</p>"));
    res!(web.put("invitation/index.html", "<p>the invitation</p>"));
    res!(web.put("invitation/reader.css", "p { margin: 0 }"));
    res!(web.put("docs/guide/index.html", "<p>the guide</p>"));
    res!(web.put("bare/readme.txt", "no index here"));
    res!(web.put("file.txt", "a plain file"));
    let rig = res!(Rig::new(vec![Site::new(HOST, &web.0)]));
    let runtime = res!(vhost_rig::runtime());
    runtime.block_on(async {
        // The directory, with and without a query, and for a HEAD as for a GET.
        for method in ["GET", "HEAD"] {
            let r = res!(rig.fetch(HOST, method, "/invitation", &[], "").await);
            assert_eq!(r.status(), 301, "{} /invitation: {:?}", method, r);
            assert_eq!(r.field("location").as_deref(), Some("/invitation/"), "{:?}", r);
            assert!(r.body.is_empty(), "a redirect carries no page: {:?}", r);
        }
        let r = res!(rig.fetch(HOST, "GET", "/invitation?a=1&b=two%20words", &[], "").await);
        assert_eq!(r.status(), 301, "{:?}", r);
        assert_eq!(r.field("location").as_deref(), Some("/invitation/?a=1&b=two%20words"),
            "the query string must arrive as it was sent: {:?}", r);
        let r = res!(rig.fetch(HOST, "GET", "/docs/guide", &[], "").await);
        assert_eq!(r.status(), 301, "{:?}", r);
        assert_eq!(r.field("location").as_deref(), Some("/docs/guide/"), "{:?}", r);

        // The slash form is the page, and the redirect does not loop back to itself.
        let r = res!(rig.fetch(HOST, "GET", "/invitation/", &[], "").await);
        assert_eq!(r.status(), 200, "{:?}", r);
        assert!(r.body.contains("the invitation"), "{:?}", r);
        assert_eq!(r.field("location"), None, "{:?}", r);
        let r = res!(rig.fetch(HOST, "GET", "/invitation/?x=1", &[], "").await);
        assert_eq!(r.status(), 200, "{:?}", r);
        let r = res!(rig.fetch(HOST, "GET", "/", &[], "").await);
        assert_eq!(r.status(), 200, "{:?}", r);
        assert!(r.body.contains("home"), "{:?}", r);

        // A directory with no index is not a page, and no redirect points at nothing: it is the
        // 404 it was, with the slash or without.
        for path in ["/bare", "/bare/", "/docs", "/docs/"] {
            let r = res!(rig.fetch(HOST, "GET", path, &[], "").await);
            assert_eq!(r.status(), 404, "GET {}: {:?}", path, r);
            assert_eq!(r.field("location"), None, "GET {}: {:?}", path, r);
        }

        // Files are files.
        for (path, want) in [("/file.txt", "a plain file"), ("/invitation/reader.css", "margin")] {
            let r = res!(rig.fetch(HOST, "GET", path, &[], "").await);
            assert_eq!(r.status(), 200, "GET {}: {:?}", path, r);
            assert!(r.body.contains(want), "GET {}: {:?}", path, r);
        }

        // The Location is built from the normalised path, so it names this site and one place on
        // it, whatever the request spelled: a doubled slash must not make a scheme-relative link,
        // and a `..` is resolved, not repeated.
        for (path, want) in [
            ("//invitation",        "/invitation/"),
            ("///invitation",       "/invitation/"),
            ("/./invitation",       "/invitation/"),
            ("/docs/../invitation", "/invitation/"),
            ("/docs/guide/../../invitation", "/invitation/"),
            ("/.",                  "/"),
        ] {
            let r = res!(rig.fetch(HOST, "GET", path, &[], "").await);
            assert_eq!(r.status(), 301, "GET {}: {:?}", path, r);
            assert_eq!(r.field("location").as_deref(), Some(want), "GET {}: {:?}", path, r);
        }
        // And one that climbs out of the web root is refused, not redirected.
        let r = res!(rig.fetch(HOST, "GET", "/../invitation", &[], "").await);
        assert_ne!(r.status(), 301, "{:?}", r);
        assert_eq!(r.field("location"), None, "{:?}", r);
        Ok(())
    })
}
