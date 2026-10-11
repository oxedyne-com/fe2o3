//! A vhost's `db_sync_on_write` reaches the Ozone its store is opened with.
//!
//! The key is read from JDAT as a boolean that defaults to off, so every config written before it
//! existed opens its store on the library's interval sync as before. A vhost that sets it opens a
//! store that fsyncs every write.

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_jdat::prelude::*;
use oxedyne_fe2o3_jdat::string::enc::EncoderConfig;
use oxedyne_fe2o3_steel::srv::{
    cfg::VhostConfig,
    context,
};

/// The vhost a JDAT map gives, read once, written back to JDAT text and read again, with the two
/// readings required to agree.
fn read_twice(src: &str) -> Outcome<VhostConfig> {
    let dat = res!(Dat::decode_string(src));
    let map = match &dat {
        Dat::Map(m) => m.clone(),
        other       => return Err(err!("Expected a map, got {:?}.", other; Invalid, Input)),
    };
    let first = res!(VhostConfig::from_datmap(&map));
    let text = res!(dat.encode_string_with_config(&EncoderConfig::<(), ()>::default()));
    let again = match res!(Dat::decode_string(&text)) {
        Dat::Map(m) => m,
        other       => return Err(err!("Expected a map, got {:?}.", other; Invalid, Input)),
    };
    let second = res!(VhostConfig::from_datmap(&again));
    assert_eq!(first.db_sync_on_write, second.db_sync_on_write,
        "the key did not survive a JDAT round trip: {}", text);
    Ok(second)
}

#[test]
fn a_vhost_that_sets_the_key_opens_a_store_that_syncs_every_write() -> Outcome<()> {
    let on = res!(read_twice(r#"{"hostnames": ["wiki.test"], "db_dir_rel": "./o3db", "db_sync_on_write": true}"#));
    let off = res!(read_twice(r#"{"hostnames": ["site.test"], "db_dir_rel": "./o3db"}"#));
    assert!(on.db_sync_on_write, "the key was set and not read");
    assert!(!off.db_sync_on_write, "a vhost that never named the key must not sync every write");
    assert!(!VhostConfig::default().db_sync_on_write);

    // The configuration a store is opened with carries the choice, and nothing else moves.
    assert!(context::db_config(on.db_sync_on_write).sync_on_write);
    assert!(!context::db_config(off.db_sync_on_write).sync_on_write);
    let (a, b) = (context::db_config(true), context::db_config(false));
    assert_eq!(a.sync_interval_ms, b.sync_interval_ms);
    assert_eq!(a.cache_size_limit_bytes, b.cache_size_limit_bytes);

    // And the store itself is opened with it.
    let root = std::env::temp_dir().join(fmt!("steel_db_sync_on_write_{}", std::process::id()));
    for (flag, name) in [(true, "on"), (false, "off")] {
        let dir = root.join(name);
        res!(std::fs::create_dir_all(&dir));
        let db = res!(context::new_db(&dir, &[7u8; 32], flag));
        assert_eq!(db.cfg().sync_on_write, flag, "the {} store was opened with the wrong sync", name);
    }
    let _ = std::fs::remove_dir_all(&root);
    Ok(())
}

#[test]
fn a_key_that_is_not_a_boolean_is_refused() -> Outcome<()> {
    let dat = res!(Dat::decode_string(r#"{"hostnames": ["x.test"], "db_sync_on_write": "yes"}"#));
    let map = match dat {
        Dat::Map(m) => m,
        other       => return Err(err!("Expected a map, got {:?}.", other; Invalid, Input)),
    };
    match VhostConfig::from_datmap(&map) {
        Ok(_)   => Err(err!("A string 'db_sync_on_write' was accepted."; Test)),
        Err(e)  => {
            assert!(fmt!("{}", e).contains("db_sync_on_write"), "the error must name the key: {}", e);
            Ok(())
        }
    }
}
