//! The raw `insert` and `get_data` WebSocket commands, driven the way an
//! unauthenticated stranger reaches them.
//!
//! Every legitimate client store command beside them is either session-scoped
//! (`sess_get`/`sess_put`) or authenticated and user-scoped (`user_get`/
//! `user_put`). These two are neither: an arbitrary key, read or written, gated
//! only on the vhost having a database. A connection that never logged in and
//! carries no operator session drives them here against a real Ozone store, so
//! the store must be left untouched and the secret left unread.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_jdat::prelude::*;
use oxedyne_fe2o3_iop_db::api::Database;
use oxedyne_fe2o3_jdat::version::SemVer;
use oxedyne_fe2o3_net::ws::{
    core::WebSocketMessage,
    handler::WebSocketHandler,
};
use oxedyne_fe2o3_steel::srv::ws::{
    handler::AppWebSocketHandler,
    syntax::WebSocketSyntax,
};

mod common;

// The text a `WebSocketMessage::Text` reply carries, or a failure if the reply
// was anything else. The reply's leading word is the response command: `info`
// on a successful insert, `data` on a read that returned, `error` on a refusal.
fn reply_text(msg: Option<WebSocketMessage>) -> Outcome<String> {
    match msg {
        Some(WebSocketMessage::Text(s)) => Ok(s),
        other => Err(err!(
            "Expected a text reply, got {:?}.", other; Test, Mismatch)),
    }
}

// A handler as a fresh, unauthenticated connection presents it: an anonymous
// session id, no operator principal resolved. This is exactly what the router
// hands `handle_text` for a stranger's socket.
fn stranger_handler() -> AppWebSocketHandler {
    AppWebSocketHandler::new(None)
        .attach_sid(Some(fmt!("anon-stranger-sid")))
        .with_operator_authed(false)
}

#[test]
fn unauthenticated_insert_is_refused() -> Outcome<()> {
    let (db, uid, _tmp) = match common::test_db() {
        Ok(t)   => t,
        Err(e)  => {
            println!("no test database available, skipping: {}", e);
            return Ok(());
        }
    };
    let handle = (db.clone(), uid);
    let syntax = res!(WebSocketSyntax::new(
        "steel_ws", &SemVer::new(0, 1, 0), "unscoped store test"));
    let id = fmt!("test");

    // A key another part of the app trusts: the console reads `publish/admins`
    // to decide who administers the site. Seed it with the one true admin.
    let admins_key = dat!("publish/admins");
    {
        let g = res!(db.write().map_err(|_| err!("poisoned"; Test)));
        res!(g.insert(admins_key.clone(), dat!(vec![dat!("real-admin")]), uid, None));
    }

    // A stranger tries to overwrite the admin list with their own handle.
    let mut h = stranger_handler();
    let txt = fmt!("insert (str|publish/admins) (str|attacker-owns-this)");
    let reply = res!(reply_text(res!(h.handle_text(
        txt, Some(handle.clone()), syntax.clone(), &id))));
    assert!(reply.starts_with("error"),
        "unauthenticated insert was not refused, replied: {}", reply);

    // The property, not just the reply: the admin list is untouched.
    let g = res!(db.read().map_err(|_| err!("poisoned"; Test)));
    match res!(g.get(&admins_key, None)) {
        Some((v, _)) => assert_eq!(v, dat!(vec![dat!("real-admin")]),
            "the admin list was overwritten by an unauthenticated caller"),
        None => panic!("the seeded admin list vanished"),
    }
    Ok(())
}

#[test]
fn unauthenticated_get_data_is_refused() -> Outcome<()> {
    let (db, uid, _tmp) = match common::test_db() {
        Ok(t)   => t,
        Err(e)  => {
            println!("no test database available, skipping: {}", e);
            return Ok(());
        }
    };
    let handle = (db.clone(), uid);
    let syntax = res!(WebSocketSyntax::new(
        "steel_ws", &SemVer::new(0, 1, 0), "unscoped store test"));
    let id = fmt!("test");

    // A stand-in for credential material: a stored user record's hash, under
    // the `user:<name>` key the auth path writes.
    let secret_key = dat!("user:victim");
    let secret_val = dat!("kdf-hash-secret-material");
    {
        let g = res!(db.write().map_err(|_| err!("poisoned"; Test)));
        res!(g.insert(secret_key.clone(), secret_val.clone(), uid, None));
    }

    // A stranger tries to read that arbitrary key.
    let mut h = stranger_handler();
    let txt = fmt!("get_data (str|\"user:victim\")");
    let reply = res!(reply_text(res!(h.handle_text(
        txt, Some(handle.clone()), syntax.clone(), &id))));
    assert!(reply.starts_with("error"),
        "unauthenticated get_data was not refused, replied: {}", reply);
    assert!(!reply.contains("kdf-hash-secret-material"),
        "unauthenticated get_data returned the secret, replied: {}", reply);
    Ok(())
}

// The gate is on the operator session, not a wall: an authenticated operator
// still reaches the raw commands, so this proves the refusals above discriminate
// on the principal rather than disabling the commands outright.
fn operator_handler() -> AppWebSocketHandler {
    AppWebSocketHandler::new(None)
        .attach_sid(Some(fmt!("operator-sid")))
        .with_operator_authed(true)
}

#[test]
fn operator_insert_and_get_data_are_allowed() -> Outcome<()> {
    let (db, uid, _tmp) = match common::test_db() {
        Ok(t)   => t,
        Err(e)  => {
            println!("no test database available, skipping: {}", e);
            return Ok(());
        }
    };
    let handle = (db.clone(), uid);
    let syntax = res!(WebSocketSyntax::new(
        "steel_ws", &SemVer::new(0, 1, 0), "unscoped store test"));
    let id = fmt!("test");

    // Seed a key directly, then read it back through the operator's get_data.
    let key = dat!("publish/index");
    {
        let g = res!(db.write().map_err(|_| err!("poisoned"; Test)));
        res!(g.insert(key.clone(), dat!("operator-may-read-this"), uid, None));
    }
    let mut h = operator_handler();
    let reply = res!(reply_text(res!(h.handle_text(
        fmt!("get_data (str|publish/index)"),
        Some(handle.clone()), syntax.clone(), &id))));
    assert!(reply.starts_with("data") && reply.contains("operator-may-read-this"),
        "operator get_data was refused or returned nothing, replied: {}", reply);

    // And the operator's insert lands.
    let mut h = operator_handler();
    let reply = res!(reply_text(res!(h.handle_text(
        fmt!("insert (str|publish/index) (str|operator-wrote-this)"),
        Some(handle.clone()), syntax.clone(), &id))));
    assert!(reply.starts_with("info"),
        "operator insert was refused, replied: {}", reply);
    Ok(())
}
