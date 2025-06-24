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

    Ok(())
}
