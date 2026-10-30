//! `#[derive(FromDatMap)]` with `#[deny_unknown_keys]`: a key no field reads is refused, named,
//! where without it the key is ignored.  A comment in the map is a note, not a key.

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_jdat::prelude::*;

#[derive(Clone, Debug, Default, PartialEq, FromDatMap)]
#[deny_unknown_keys]
struct Strict {
    size:               String,
    #[optional] count:  u64,
    #[rename(name = "as_named")]
    named:              String,
}

#[derive(Clone, Debug, Default, PartialEq, FromDatMap)]
struct Lax {
    size:               String,
}

fn map_of(text: &str) -> Outcome<DaticleMap> {
    match res!(Dat::decode_string(text)) {
        Dat::Map(m) => Ok(m),
        d           => Err(err!("Not a map: {:?}.", d; Test)),
    }
}

#[test]
fn an_unknown_key_is_refused_by_name() -> Outcome<()> {
    let m = res!(map_of(r#"{ "size": "5G", "as_named": "x", "unit_memroy_max": "11G" }"#));
    match Strict::from_datmap(m) {
        Ok(v) => Err(err!("Accepted {:?}.", v; Test)),
        Err(e) => {
            let s = e.plain();
            assert!(s.contains("unit_memroy_max") && s.contains("Strict"), "{}", s);
            Ok(())
        },
    }
}

#[test]
fn a_key_under_its_field_name_when_renamed_is_refused() -> Outcome<()> {
    let m = res!(map_of(r#"{ "size": "5G", "as_named": "x", "named": "y" }"#));
    match Strict::from_datmap(m) {
        Ok(v) => Err(err!("Accepted {:?}.", v; Test)),
        Err(e) => {
            assert!(e.plain().contains("'named'"), "{}", e.plain());
            Ok(())
        },
    }
}

#[test]
fn known_keys_and_comments_pass() -> Outcome<()> {
    let m = res!(map_of("{\n  # a comment on its own line\n  \"size\": \"5G\",\n  \"as_named\": \"x\",\n  \"count\": 2,\n}"));
    let v = res!(Strict::from_datmap(m));
    assert_eq!(v, Strict { size: "5G".to_string(), count: 2, named: "x".to_string() });
    Ok(())
}

#[test]
fn without_the_attribute_an_unknown_key_is_ignored() -> Outcome<()> {
    let m = res!(map_of(r#"{ "size": "5G", "other": 1 }"#));
    let v = res!(Lax::from_datmap(m));
    assert_eq!(v.size, "5G");
    Ok(())
}
