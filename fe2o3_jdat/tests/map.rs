use oxedyne_fe2o3_jdat::prelude::*;

use oxedyne_fe2o3_core::{
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
            None => return Err(err!("Expected value."; Test, Missing, Data)),
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
            None => return Err(err!("Expected value."; Test, Missing, Data)),
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

    res!(test_it(filter, &["Map Get I64 000", "all", "map", "get"], || {
        // A u64 written through mapdat! must be readable back as i64, as the
        // doc comment promises; it was not before the U64 getter arm existed.
        let d = mapdat!{
            "size" => 1_048_576u64,
            "count" => 42u32,
        };
        req!(1_048_576i64, res!(d.map_get_i64(&dat!("size"))));
        req!(42i64, res!(d.map_get_i64(&dat!("count"))));

        // Above i64::MAX the conversion must refuse, not wrap.
        let big = mapdat!{ "size" => u64::MAX };
        match big.map_get_i64(&dat!("size")) {
            Err(_) => (),
            Ok(n) => return Err(err!(
                "u64::MAX coerced to i64 as {}, expected a refusal.", n;
            Test, Mismatch)),
        }
        Ok(())
    }));

    Ok(())
}
