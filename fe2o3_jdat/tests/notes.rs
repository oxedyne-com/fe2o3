//! A jdat file's comments, seen through by `Dat::without_notes`.

use oxedyne_fe2o3_jdat::prelude::*;

use oxedyne_fe2o3_core::prelude::*;

/// **A list with comments between its rows decodes as the rows alone.**  The decoder keeps a
/// comment on a line of its own as a list entry, so a reader matching each row's shape would
/// otherwise meet a note where it expects a row.
#[test]
fn test_list_with_interleaved_comments_decodes() -> Outcome<()> {
    let text = "{\n\
        # Before the key.\n\
        \"rows\": [\n\
            [\"a\", 1],\n\
            # Between rows,\n\
            # over two lines.\n\
            [\"b\", 2], # After a row, past its comma, so alone.\n\
            [\"c\", 3]\n\
        ],\n\
        \"n\": 4 # On a value.\n\
    }";
    let dat = res!(Dat::decode_string(text));
    let rows = match dat.map_get(&Dat::Str(fmt!("rows"))) {
        Ok(Some(Dat::List(l)))  => l.clone(),
        other                   => return Err(err!("rows is {:?}", other; Test, Unexpected)),
    };
    assert_eq!(rows.iter().filter(|d| d.is_lone_note()).count(), 3,
        "the decoder no longer keeps a lone comment as an entry: {:?}", rows);

    let bare = dat.without_notes();
    let mut want = DaticleMap::new();
    want.insert(Dat::Str(fmt!("rows")), Dat::List(vec![
        Dat::List(vec![Dat::Str(fmt!("a")), Dat::U8(1)]),
        Dat::List(vec![Dat::Str(fmt!("b")), Dat::U8(2)]),
        Dat::List(vec![Dat::Str(fmt!("c")), Dat::U8(3)]),
    ]));
    want.insert(Dat::Str(fmt!("n")), Dat::U8(4));
    assert_eq!(bare, Dat::Map(want));
    Ok(())
}

/// **Notes leave data that holds none unchanged, and an empty entry is not a note.**
#[test]
fn test_without_notes_keeps_plain_data() -> Outcome<()> {
    let dat = res!(Dat::decode_string("[1, [2, 3], {\"k\": \"v\"}]"));
    assert_eq!(dat.clone().without_notes(), dat);
    assert!(!Dat::is_note_entry(&Dat::Empty, &Dat::Empty));
    assert!(!Dat::Empty.is_lone_note());
    Ok(())
}
