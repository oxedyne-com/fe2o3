use oxedyne_fe2o3_core::{
    prelude::*,
    rand::RanDef,
};
use oxedyne_fe2o3_namex::id;

#[test]
fn genids() -> Outcome<()> {
    
    // Generate and print 10 random namex ids.
    for _ in 0..9 {
        let id = id::NamexId::randef();
        println!("{}", id);
    }

    Ok(())
}
