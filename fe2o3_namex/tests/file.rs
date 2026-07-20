use oxedyne_fe2o3_namex::{
    id::NamexId,
    db::{
        Entity,
        MapKey,
        Namex,
    },
};

use oxedyne_fe2o3_core::prelude::*;

use std::{
    collections::BTreeMap,
};

pub fn test_file(filter: &'static str) -> Outcome<()> {

    test_it!(filter, "Load and save 000", "all", "load", "save", {

        let db = res!(Namex::<
            BTreeMap<MapKey, Entity>,
            BTreeMap<NamexId, Entity>,
        >::load("./namex.jdat"));

        test!("Successfully loaded namex file.");

        res!(db.to_file(
            res!(db.export_jdat()),
            "./namex_echo.jdat",
        ));

        res!(db.to_file(
            res!(db.export_json()),
            "./namex_echo.json",
        ));
    });

    test_it!(filter, "Lookup by canonical id 010", "all", "load", "lookup", {

        let db = res!(Namex::<
            BTreeMap<MapKey, Entity>,
            BTreeMap<NamexId, Entity>,
        >::load("./namex.jdat"));

        // Ids are quoted in the registry exactly as `Display` renders them, so every id a
        // caller holds as a string must resolve to an entity.
        for (id_str, nam) in [
            ("VybbHNWeNXeTqTrXj66TzZScbSTsEFVy0W79QnbroFA=", "SHA3_256"),
            ("kCnQluCVX4v62XUObBIPJhg+VZaXjXHQOLoNDVrOZso=", "SHA-256"),
        ] {
            let id = res!(NamexId::try_from(id_str));
            match db.by_id(&id) {
                Some(entity) => if !entity.nams.iter().any(|n| n == nam) {
                    return Err(err!(
                        "Id {} resolved to names {:?}, expected it to include '{}'.",
                        id_str, entity.nams, nam;
                    Test, Invalid, Mismatch));
                },
                None => return Err(err!(
                    "Id {} is not present in the registry.", id_str;
                Test, Invalid, Missing)),
            }
            // The canonical text form must survive a round trip through the id type.
            if fmt!("{}", id) != id_str {
                return Err(err!(
                    "Id {} does not render back to itself, got {}.", id_str, id;
                Test, Invalid, Mismatch));
            }
        }
    });

    Ok(())
}
