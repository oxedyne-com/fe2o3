//! A public write that a browser sent from another site is refused, and a sign-up says one thing.
//!
//! These call `AppWebHandler::handle_post` as the server does, against a real Ozone instance. They
//! do not skip when no database starts: a skipped test cannot be shown red.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use oxedyne_fe2o3_core::{
    file::OsPath,
    prelude::*,
};
use oxedyne_fe2o3_net::http::{
    fields::{
        HeaderFieldValue,
        HeaderFields,
        HeaderName,
    },
    handler::WebHandler,
    header::HttpHeadline,
    loc::HttpLocator,
    msg::HttpMessage,
    status::HttpStatus,
};
use oxedyne_fe2o3_steel::{
    app::https::AppWebHandler,
    srv::{
        api::ApiHandlerRegistry,
        cfg::ServerConfig,
        id,
        publish::{
            PublishConfig,
            Source,
            rate::Window,
            send::MailSender,
            subscribe::{
                self,
                SubState,
            },
        },
        webhook::WebhookRegistry,
    },
};

use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::Arc,
};

mod common;

const SITE: &str = "https://site.test";

type Web = AppWebHandler<HashMap<String, OsPath>>;

fn web() -> Outcome<Web> {
    let cfg = PublishConfig {
        path:       fmt!("/news"),
        base_url:   fmt!("{}", SITE),
        comments:   true,
        source:     Source::Store,
        ..Default::default()
    };
    let sender = res!(MailSender::new("mail.site.test".to_string(), Vec::new(), "news@site.test".to_string()));
    Ok(AppWebHandler::new(
        ServerConfig::default(),
        std::env::temp_dir(),
        HashMap::new(),
        vec![fmt!("index.html")],
        true,
        Vec::new(),
        Vec::new(),
        Arc::new(WebhookRegistry::new()),
        Arc::new(ApiHandlerRegistry::new()),
        None,
        None,
        None,
        Some(Arc::new(cfg)),
        Some(Arc::new(sender)),
        Arc::new(Vec::new()),
    ))
}

// A POST as the server hands it to the handler: the path and query, the lines a browser (or not)
// sent, and a form body.
async fn post(
    web:    &Web,
    handle: &(Arc<std::sync::RwLock<common::TestDb>>, id::Uid),
    target: &str,
    lines:  &[(&str, &str)],
    body:   &str,
)
    -> Outcome<HttpMessage>
{
    let mut fields = HeaderFields::default();
    for (name, value) in lines {
        let nam = HeaderName::from(*name);
        let val = res!(HeaderFieldValue::new(&nam, value));
        fields.insert(nam, val, None);
    }
    let peer: SocketAddr = res!("127.0.0.1:50000".parse(), Network, Init);
    let sid: Option<id::Sid> = None;
    let got = res!(web.handle_post(
        res!(HttpLocator::new(target)),
        None,
        body.as_bytes().to_vec(),
        Arc::new(fields),
        Some((handle.0.clone(), handle.1.clone())),
        &sid,
        peer,
        &fmt!("test"),
    ).await);
    got.ok_or_else(|| err!("the handler answered nothing for {}", target; Test, Missing))
}

fn status_of(resp: &HttpMessage) -> Option<HttpStatus> {
    match &resp.header.headline {
        HttpHeadline::Response { status }   => Some(*status),
        _                                   => None,
    }
}

const SUBSCRIBE: &str = "/news/subscribe";

#[tokio::test]
async fn test_a_cross_site_subscribe_is_refused_and_stores_nothing_00() -> Outcome<()> {
    let (db, uid, _tmp) = res!(common::test_db());
    let handle = (db, uid);
    let web = res!(web());
    let resp = res!(post(&web, &handle, SUBSCRIBE, &[("sec-fetch-site", "cross-site")],
        "email=reader%40nowhere.invalid").await);
    assert_eq!(status_of(&resp), Some(HttpStatus::Forbidden), "a cross-site sign-up was not refused");
    assert_eq!(res!(subscribe::count(&handle, "test")), 0, "a refused sign-up stored a subscriber");
    Ok(())
}

#[tokio::test]
async fn test_a_same_origin_subscribe_passes_01() -> Outcome<()> {
    let (db, uid, _tmp) = res!(common::test_db());
    let handle = (db, uid);
    let web = res!(web());
    // A malformed address reaches the handler and is told so, without a word on the network.
    let resp = res!(post(&web, &handle, SUBSCRIBE, &[("sec-fetch-site", "same-origin")],
        "email=not-an-address").await);
    assert_eq!(status_of(&resp), Some(HttpStatus::OK), "a same-origin sign-up was refused");
    assert!(resp.body_as_string().contains("does not look like an email"),
        "the sign-up did not reach its handler: {}", resp.body_as_string());
    Ok(())
}

#[tokio::test]
async fn test_a_cross_site_comment_is_refused_02() -> Outcome<()> {
    let (db, uid, _tmp) = res!(common::test_db());
    let handle = (db, uid);
    let web = res!(web());
    let resp = res!(post(&web, &handle, "/news/a-post/comment", &[("origin", "https://evil.example")],
        "name=x&body=y").await);
    assert_eq!(status_of(&resp), Some(HttpStatus::Forbidden), "a cross-site comment was not refused");
    Ok(())
}

#[tokio::test]
async fn test_an_unsubscribe_is_never_refused_03() -> Outcome<()> {
    let (db, uid, _tmp) = res!(common::test_db());
    let handle = (db, uid);
    let web = res!(web());
    let sub = match res!(subscribe::add_pending(&handle, "reader@nowhere.invalid", &Window::default(), 0)) {
        Some(s) => s,
        None    => return Err(err!("the reader did not pend"; Test, Missing)),
    };
    res!(subscribe::confirm(&handle, &sub.token, "test"));
    // A mail client's one-click post carries no `Origin`; even one that does is not refused.
    let target = fmt!("/news/unsubscribe?token={}", sub.token);
    let resp = res!(post(&web, &handle, &target,
        &[("sec-fetch-site", "cross-site"), ("origin", "https://mail.example")], "").await);
    assert_eq!(status_of(&resp), Some(HttpStatus::OK), "an unsubscribe was refused");
    match res!(subscribe::get(&handle, "reader@nowhere.invalid")) {
        Some(s) => assert_eq!(s.state, SubState::Unsubscribed, "the unsubscribe did not take"),
        None    => return Err(err!("the reader vanished"; Test, Missing)),
    }
    Ok(())
}

#[tokio::test]
async fn test_a_new_and_a_confirmed_address_get_the_same_json_04() -> Outcome<()> {
    let (db, uid, _tmp) = res!(common::test_db());
    let handle = (db, uid);
    let web = res!(web());
    let sub = match res!(subscribe::add_pending(&handle, "known@nowhere.invalid", &Window::default(), 0)) {
        Some(s) => s,
        None    => return Err(err!("the reader did not pend"; Test, Missing)),
    };
    res!(subscribe::confirm(&handle, &sub.token, "test"));
    let lines = [("sec-fetch-site", "same-origin"), ("accept", "application/json")];
    let fresh = res!(post(&web, &handle, SUBSCRIBE, &lines, "email=fresh%40nowhere.invalid").await);
    let known = res!(post(&web, &handle, SUBSCRIBE, &lines, "email=known%40nowhere.invalid").await);
    assert_eq!(status_of(&fresh), Some(HttpStatus::OK));
    assert_eq!(status_of(&known), Some(HttpStatus::OK));
    assert!(fresh.body_as_string().contains("\"said\": \"sent\""),
        "a new address was not told sent: {}", fresh.body_as_string());
    assert_eq!(fresh.body, known.body, "a new and a confirmed address were told apart");
    Ok(())
}
