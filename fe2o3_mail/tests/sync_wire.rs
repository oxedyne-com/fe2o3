//! The sync records through their JSON wire form and back (`sync::wire`), as the gateway sends
//! them and the page reads them.

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_jdat::{
    prelude::*,
    bdat::limits::DecodeLimits,
};
use oxedyne_fe2o3_mail::sync::{
    wire::{
        self,
        ActReply,
    },
    ActOutcome,
    Action,
    Budget,
    Cursor,
    Delta,
    Entry,
    FlagState,
    LinkKey,
    NewMessage,
    Reset,
    Snapshot,
    SpecialUse,
    Target,
    Tier,
    UidSet,
};


/// Through JSON text and back, as a JavaScript peer would see it. The text is returned with the
/// encoder's space after each colon taken out, so the checks on it read as compact JSON.
fn json_trip<T: ToDat + FromDat>(x: &T) -> Outcome<(String, T)> {
    let text = res!(res!(x.to_dat()).json());
    let back = res!(Dat::decode_json_strict(&text, &DecodeLimits::text()));
    Ok((text.replace("\": ", "\":"), res!(T::from_dat(back))))
}

fn parse(s: &str) -> Outcome<Dat> {
    Dat::decode_json_strict(s, &DecodeLimits::text())
}

fn cursor() -> Outcome<Cursor> {
    Ok(Cursor {
        uid_validity:   7,
        highest_uid:    4_000_000_000,
        highest_modseq: Some(1_234_567_890_123),
        polls:          9,
        backlog:        res!(UidSet::parse("1:5,9,100:120")),
    })
}

#[test]
fn test_delta_round_trips_with_sets_and_base64_bodies() -> Outcome<()> {
    // Every byte value, so the base64 is not merely ASCII passed through.
    let raw: Vec<u8> = (0..=255u8).cycle().take(1027).collect();
    let d = Delta {
        mailbox:    fmt!("Archive/2024 \"quoted\""),
        tier:       Some(Tier::CondStore),
        reset:      Some(Reset { old: 3, new: 7 }),
        new:        vec![
            NewMessage {
                uid:            130,
                flags:          vec![fmt!("\\Seen"), fmt!("$Label1")],
                internal_date:  fmt!("26-Oct-2026 09:00:00 +0000"),
                size:           1027,
                raw:            raw.clone(),
                whole:          true,
                link:           Some(LinkKey::MessageId(fmt!("a@b.c"))),
            },
            NewMessage {
                uid:            129,
                flags:          Vec::new(),
                internal_date:  String::new(),
                size:           25_000_000,
                raw:            b"Subject: big\r\n\r\n".to_vec(),
                whole:          false,
                link:           Some(LinkKey::GmMsgId(u64::MAX)),
            },
            NewMessage { uid: 128, raw: Vec::new(), ..Default::default() },
        ],
        flags:      vec![FlagState { uid: 12, flags: vec![fmt!("\\Flagged")] }],
        vanished:   res!(UidSet::parse("3,7,40:60")),
        cursor:     res!(cursor()),
        more:       true,
    };
    let (text, back) = res!(json_trip(&d));
    assert_eq!(back, d);
    // The sets travel as their IMAP form and the bytes as base64.
    assert!(text.contains("\"vanished\":\"3,7,40:60\""), "{}", text);
    assert!(text.contains("\"backlog\":\"1:5,9,100:120\""), "{}", text);
    assert!(text.contains(&fmt!("\"raw\":\"{}\"", oxedyne_fe2o3_text::base64::encode(&raw))));
    assert!(text.contains("\"link_gm\":\"18446744073709551615\""), "{}", text);

    // None of the optional parts, and a reader that keeps the bytes elsewhere leaves `raw` out.
    let bare = Delta {
        mailbox:    fmt!("INBOX"),
        cursor:     Cursor { uid_validity: 1, ..Default::default() },
        new:        vec![NewMessage { uid: 5, whole: true, ..Default::default() }],
        ..Default::default()
    };
    let (text, back) = res!(json_trip(&bare));
    assert_eq!(back, bare);
    assert!(text.contains("\"tier\":\"\""), "{}", text);
    assert!(!text.contains("reset") && !text.contains("highest_modseq"), "{}", text);
    let no_raw = res!(wire::delta_from(&res!(parse(r#"{"mailbox":"INBOX",
        "cursor":{"uid_validity":1},"new":[{"uid":5,"whole":true}]}"#))));
    assert_eq!(no_raw, bare);
    Ok(())
}

#[test]
fn test_delta_refuses_what_it_cannot_read() -> Outcome<()> {
    let bad = [
        r#"{"mailbox":"INBOX","cursor":{"uid_validity":1},"vanished":"0:x"}"#,
        r#"{"mailbox":"INBOX","cursor":{"uid_validity":1,"backlog":"7:"}}"#,
        r#"{"mailbox":"INBOX","cursor":{"uid_validity":0}}"#,
        r#"{"mailbox":"INBOX"}"#,
        r#"{"cursor":{"uid_validity":1}}"#,
        r#"{"mailbox":"INBOX","cursor":{"uid_validity":4294967296}}"#,
        r#"{"mailbox":"INBOX","cursor":{"uid_validity":1},"new":[{"uid":3,"raw":"not base64!"}]}"#,
        r#"{"mailbox":"INBOX","cursor":{"uid_validity":1},"new":[{"uid":0}]}"#,
        r#"{"mailbox":"INBOX","cursor":{"uid_validity":1},"flags":[{"uid":3,"flags":[1]}]}"#,
        r#"{"mailbox":"INBOX","cursor":{"uid_validity":1},"tier":"psychic"}"#,
    ];
    for b in bad {
        assert!(wire::delta_from(&res!(parse(b))).is_err(), "read: {}", b);
    }
    // A folder the server refused comes back as an error naming it.
    let refused = wire::refused_dat("Gone", "no such mailbox");
    match wire::delta_from(&refused) {
        Err(e)  => assert!(e.plain().contains("Gone") && e.plain().contains("no such mailbox"),
                    "{}", e.plain()),
        Ok(_)   => return Err(err!("A refused folder read as a delta."; Test)),
    }
    Ok(())
}

#[test]
fn test_snapshot_cursor_and_budget() -> Outcome<()> {
    let s = Snapshot {
        mailbox:    fmt!("INBOX"),
        cursor:     res!(cursor()),
        held:       res!(UidSet::parse("6:40,42")),
        window:     res!(UidSet::parse("30:42")),
    };
    let (_, back) = res!(json_trip(&s));
    assert_eq!(back, s);
    let (_, c) = res!(json_trip(&s.cursor));
    assert_eq!(c, s.cursor);

    // The gateway's request form: no cursor is a folder never synced.
    let f = res!(wire::snapshot_from(&res!(parse(r#"{"mailbox":"Sent","held":"","window":"1"}"#))));
    assert_eq!((f.cursor, f.held.len(), f.window.len()), (Cursor::default(), 0, 1));
    assert!(wire::snapshot_from(&res!(parse(r#"{"mailbox":"X","held":"0:x"}"#))).is_err());
    assert!(wire::snapshot_from(&res!(parse(r#"{"mailbox":"X","cursor":7}"#))).is_err());
    assert!(wire::snapshot_from(&res!(parse(r#"{"held":"1"}"#))).is_err());

    // A request may only lower the limit.
    let limit = Budget { max_count: 500, max_bytes: 4096, ..Budget::default() };
    let req = res!(parse(r#"{"max_count":9999,"max_bytes":1000,"bodies":true}"#));
    let b = res!(wire::budget_within(Some(&req), &limit));
    assert_eq!((b.max_count, b.max_bytes, b.bodies), (500, 1000, true));
    let b = res!(wire::budget_within(None, &limit));
    assert_eq!((b.max_count, b.max_bytes, b.bodies), (500, 4096, false));
    let b = res!(wire::budget_within(Some(&wire::budget_dat(&b)), &limit));
    assert_eq!((b.max_count, b.max_bytes, b.bodies), (500, 4096, false));
    assert!(wire::budget_within(Some(&res!(parse(r#"{"bodies":"yes"}"#))), &limit).is_err());
    Ok(())
}

#[test]
fn test_entries_round_trip_every_action() -> Outcome<()> {
    let base = Entry {
        id:             u32::MAX as u64 + 5,
        mailbox:        fmt!("INBOX"),
        uid_validity:   5,
        uid:            9,
        message_id:     fmt!("<a@b>"),
        size:           120,
        year:           Some(2024),
        action:         Action::Archive,
    };
    let actions = vec![
        Action::Flags { add: vec![fmt!("\\Seen")], remove: vec![fmt!("\\Flagged")] },
        Action::Flags { add: Vec::new(), remove: vec![fmt!("\\Seen")] },
        Action::Move { to: Target::Inbox },
        Action::Move { to: Target::Path(fmt!("Work/2024")) },
        Action::Archive,
        Action::Trash,
        Action::Junk,
        Action::NotJunk,
        Action::Expunge,
    ];
    let roles = [
        SpecialUse::Sent, SpecialUse::Drafts, SpecialUse::Trash, SpecialUse::Junk,
        SpecialUse::Archive, SpecialUse::All, SpecialUse::Flagged,
    ];
    let all = actions.into_iter()
        .chain(roles.iter().map(|r| Action::Move { to: Target::Role(*r) }));
    for a in all {
        let e = Entry { action: a, ..base.clone() };
        let (_, back) = res!(json_trip(&e));
        assert_eq!(back, e);
    }
    let lean = Entry { message_id: String::new(), size: 0, year: None, ..base.clone() };
    let (text, back) = res!(json_trip(&lean));
    assert_eq!(back, lean);
    assert!(!text.contains("message_id") && !text.contains("year"), "{}", text);

    // The page's form: a string id, no optional fields.
    let e = res!(wire::entry_from(&res!(parse(r#"{"id":"2","mailbox":"INBOX","uid_validity":5,
        "uid":10,"action":"move","to":"role:junk"}"#))));
    assert_eq!((e.id, e.action), (2, Action::Move { to: Target::Role(SpecialUse::Junk) }));
    let bad = [
        r#"{"id":1,"mailbox":"INBOX","uid_validity":5,"uid":9,"action":"flags"}"#,
        r#"{"id":1,"mailbox":"INBOX","uid_validity":5,"uid":9,"action":"move","to":"elsewhere"}"#,
        r#"{"id":1,"mailbox":"INBOX","uid_validity":5,"uid":9,"action":"move","to":"role:inbox"}"#,
        r#"{"id":1,"mailbox":"INBOX","uid_validity":5,"uid":9,"action":"burn"}"#,
        r#"{"id":1,"mailbox":"INBOX","uid":9,"action":"trash"}"#,
        r#"{"id":"x","mailbox":"INBOX","uid_validity":5,"uid":9,"action":"trash"}"#,
        r#"{"mailbox":"INBOX","uid_validity":5,"uid":9,"action":"trash"}"#,
        r#"{"id":1,"mailbox":"INBOX","uid_validity":5,"uid":9,"action":"trash","year":70000}"#,
    ];
    for b in bad {
        assert!(wire::entry_from(&res!(parse(b))).is_err(), "read: {}", b);
    }
    assert_eq!(wire::target_of("path:"), None);
    Ok(())
}

#[test]
fn test_replies_round_trip_every_outcome() -> Outcome<()> {
    let all = [
        Ok(ActOutcome::Done),
        Ok(ActOutcome::Gone),
        Ok(ActOutcome::Superseded(fmt!("Archive/2024"))),
        Ok(ActOutcome::Lost),
        Ok(ActOutcome::Stuck(fmt!("two copies match"))),
        Err(fmt!("The mail server could not be reached.")),
    ];
    for o in all {
        let r = ActReply { id: (1u64 << 60) + 3, outcome: o };
        let (text, back) = res!(json_trip(&r));
        assert_eq!(back, r);
        assert!(text.contains("\"id\":\"1152921504606846979\""), "{}", text);
    }
    assert!(wire::reply_from(&res!(parse(r#"{"id":"1","outcome":"maybe"}"#))).is_err());
    Ok(())
}
