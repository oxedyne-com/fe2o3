use oxedize_fe2o3_jdat::prelude::*;

use oxedize_fe2o3_core::{
    prelude::*,
    test::test_it,
};


pub fn test_map_func(filter: &'static str) -> Outcome<()> {

    res!(test_it(filter, &["Map Find 000", "all", "map", "find"], || {
        let d = mapdat!{
            1u8 => mapdat!{
                2u8 => mapdat!{
                    4321u16 => mapdat!{
                        "user" => 1234u128,
                        "time" => 5678u64,
                        "value" => 42u8,
                    },
                },
            }
        };
        let found_opt = res!(d.find(&listdat![1u8, 2u8, 4321u16, "value"]));
        match found_opt {
            Some(found) => {
                test!("Found it! {:?}", found);
                req!(&dat!(42u8), found);
            }
            None => return Err(err!(errmsg!("Expected value."), Test, Missing, Data)),
        }
        Ok(())
    }));

    res!(test_it(filter, &["Map Find 010", "all", "map", "find"], || {
        let d = omapdat!{
            1u8 => omapdat!{
                2u8 => omapdat!{
                    4321u16 => omapdat!{
                        "user" => 1234u128,
                        "time" => 5678u64,
                        "value" => 42u8,
                    },
                },
            }
        };
        let found_opt = res!(d.find(&listdat![1u8, 2u8, 4321u16, "value"]));
        match found_opt {
            Some(found) => {
                test!("Found it! {:?}", found);
                req!(&dat!(42u8), found);
            }
            None => return Err(err!(errmsg!("Expected value."), Test, Missing, Data)),
        }
        Ok(())
    }));

    res!(test_it(filter, &["Map Find 100", "all", "map", "find"], || {
        let d = mapdat!{
            1u8 => mapdat!{
                4321u16 => mapdat!{
                    "user" => 1234u128,
                    "time" => 5678u64,
                    "value" => 42u8,
                },
                5678u16 => mapdat!{
                    "user" => 1234u128,
                    "time" => 5678u64,
                    "value" => 84u8,
                }
            },
            2u8 => mapdat!{
                9012u16 => mapdat!{
                    "user" => 1234u128,
                    "time" => 5678u64,
                    "value" => 168u8,
                }
            }
        };
        let found = res!(d.find_all(&dat!("value")));
        req!(3, found.len());
        test!("Found it! {:?}", found);
        req!(&dat!(42u8), found[0]);
        req!(&dat!(84u8), found[1]);
        req!(&dat!(168u8), found[2]);
        Ok(())
    }));

    Ok(())
}
