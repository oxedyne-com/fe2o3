//! The publish store against a real Ozone instance.
//!
//! The unit tests cover the record's encoding and the index's arithmetic. This covers the thing they
//! cannot: that a post written to a database comes back out of it.

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_steel::srv::publish::{
    Markup,
    PostKind,
    PostState,
    store::{
        self,
        Record,
    },
};

mod common;

#[test]
fn a_post_survives_the_database() -> Outcome<()> {
    let (db, uid, _tmp) = match common::test_db() {
        Ok(t)   => t,
        Err(e)  => {
            println!("no test database available, skipping: {}", e);
            return Ok(());
        }
    };
    let handle = (db, uid);

    // Nothing published is an empty list, not an error.
    let posts = res!(store::list(&handle, "test"));
    assert_eq!(posts.len(), 0, "a fresh store is not empty");

    let rec = Record {
        slug:   fmt!("on-rent"),
        kind:   PostKind::Essay,
        state:  PostState::Live,
        markup: Markup::Markdown,
        date:   Some(fmt!("2026-07-17")),
        source: fmt!("# On rent\n\nAn opening sentence.\n"),
    };
    res!(store::put(&handle, &rec, "test"));

    // It comes back as the record it went in as.
    let back = res!(store::get(&handle, "on-rent"));
    assert_eq!(back.as_ref(), Some(&rec));

    // And as a rendered post, titled by its own heading.
    let posts = res!(store::list(&handle, "test"));
    assert_eq!(posts.len(), 1);
    assert_eq!(posts[0].title, "On rent");
    assert_eq!(posts[0].date, Some(fmt!("2026-07-17")));
    assert!(posts[0].html.contains("<h1>On rent</h1>"), "got: {}", posts[0].html);
    assert_eq!(posts[0].excerpt, "An opening sentence.");

    // Writing the same slug twice is one post, not two: the index does not grow.
    res!(store::put(&handle, &rec, "test"));
    let posts = res!(store::list(&handle, "test"));
    assert_eq!(posts.len(), 1, "the index grew on a rewrite");

    // A draft is written, and served to nobody.
    let draft = Record {
        slug:   fmt!("unfinished"),
        state:  PostState::Draft,
        ..rec.clone()
    };
    res!(store::put(&handle, &draft, "test"));
    assert!(res!(store::get(&handle, "unfinished")).is_some(), "the draft was not stored");
    let posts = res!(store::list(&handle, "test"));
    assert_eq!(posts.len(), 1, "a draft reached a reader");

    // Newest first.
    let older = Record {
        slug:   fmt!("older"),
        date:   Some(fmt!("2026-07-01")),
        ..rec.clone()
    };
    res!(store::put(&handle, &older, "test"));
    let posts = res!(store::list(&handle, "test"));
    assert_eq!(posts.len(), 2);
    assert_eq!(posts[0].slug, "on-rent", "not newest first");
    assert_eq!(posts[1].slug, "older");

    // Deleting takes it out of the index too, so a list does not name what is gone.
    assert!(res!(store::delete(&handle, "older", "test")));
    let posts = res!(store::list(&handle, "test"));
    assert_eq!(posts.len(), 1);
    assert_eq!(posts[0].slug, "on-rent");

    // A deleted post is gone to a reader that asks for it by name.
    assert!(res!(store::get(&handle, "older")).is_none(), "a deleted post still reads back");

    // The index can be rebuilt from what the database holds, drafts included but not the deleted one.
    //
    // This is the assertion that matters. `Database::delete` "marks for deletion", and a scan still
    // hands back the marked key -- so a rebuild that trusted the scan would resurrect every post ever
    // deleted, silently, the next time anything repaired the index.
    let n = res!(store::rebuild_index(&handle, "test"));
    assert_eq!(n, 2, "rebuild found {} posts, expected the live one and the draft", n);
    let posts = res!(store::list(&handle, "test"));
    assert_eq!(posts.len(), 1, "the rebuilt index resurrected a deleted post");
    assert_eq!(posts[0].slug, "on-rent");

    Ok(())
}
