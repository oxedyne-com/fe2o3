use oxedize_fe2o3_jdat::{
    prelude::*,
    self,
    FromDatMap,
    ToDatMap,
    daticle::{
        IterDat,
        IterDatValsMut,
    },
    usr::UsrKindId,
};

use oxedize_fe2o3_core::{
    prelude::*,
    test::test_it,
};
use oxedize_fe2o3_num::{
    prelude::*,
    BigDecimal,
    BigInt,
    float::{
        Float32,
        Float64,
    },
};

use std::collections::BTreeMap;

pub fn test_daticle_func(filter: &'static str) -> Outcome<()> {

    res!(test_it(filter, &["Map 000", "all", "map"], || {
        let d = mapdat!{
            "key1" => "value1",
            "key2" => "value2",
        };
        req!(res!(d.map_get(&dat!("key1"))).unwrap(), &dat!("value1"));
        Ok(())
    }));

    res!(test_it(filter, &["Map 010", "all", "map"], || {
        let d = mapdat!{
            "key1" => "value1",
            "key2" => "value2",
        };
        if let Some(Dat::Str(s)) = res!(d.map_get_type(&dat!("key1"), &[&Kind::Str])) {
            req!(s, "value1");
        }
        Ok(())
    }));

    res!(test_it(filter, &["Map struct 000", "all", "map", "struct"], || {
        // Loops over the map, but it's really best to loop over the struct fields,
        // requiring the reflection possible in a derive macro.

        #[derive(Debug, Default, PartialEq)]
        struct S {
            a: u32,
            b: bool,
            c: String,
        }

        let d = mapdat!{
            "a" => 42u32,
            "b" => true,
            "c" => "hello".to_string(),
        };

        let mut s = S::default();

        if let Dat::Map(map) = d {
            for (k, v) in map.iter() {
                if let Dat::Str(ks) = k {
                    match &**ks {
                        "a" => if let Dat::U32(n) = v { s.a = n.clone() },
                        "b" => if let Dat::Bool(b) = v { s.b = *b },
                        "c" => if let Dat::Str(t) = v { s.c = t.clone() },
                        _ => (),
                    }
                }
            }
        }
        
        let expected = S {
            a: 42,
            b: true,
            c: "hello".to_string(),
        };

        req!(s, expected);
        Ok(())
    }));

    res!(test_it(filter, &["IterDat 010", "all", "iter"], || {
        let d = omapdat!{
            listdat!["key11", 12u8] => 1u8,
            "key2" => "val2",
            3u8 => "val3",
        };
        let mut iter = IterDat::new(d.clone());
        req!(iter.next(), Some(dat!("key11")));
        req!(iter.next(), Some(dat!(12u8)));
        req!(iter.next(), Some(dat!(1u8)));
        req!(iter.next(), Some(dat!("key2")));
        req!(iter.next(), Some(dat!("val2")));
        req!(iter.next(), Some(dat!(3u8)));
        req!(iter.next(), Some(dat!("val3")));
        req!(iter.next(), None::<Dat>);
        req!(iter.next(), None::<Dat>);
        req!(iter.next(), None::<Dat>);
        let mut iter2 = IterDat::new(d);
        req!(iter2.next(), Some(dat!("key11")));
        Ok(())
    }));

    res!(test_it(filter, &["IterDat 020", "all", "iter"], || {
        let d = omapdat!{
            "key1" => omapdat!{
                2u8 => "val2",
                "key3" => listdat!["val31", 32u8],
            },
            omapdat!{
                "key4" => 4u8,
                5u8 => "val5",
            } => "val6",
            7u8 => "val7",
        };
        let mut iter = IterDat::new(d);
        req!(iter.next(), Some(dat!("key1")));
        req!(iter.next(), Some(dat!(2u8)));
        req!(iter.next(), Some(dat!("val2")));
        req!(iter.next(), Some(dat!("key3")));
        req!(iter.next(), Some(dat!("val31")));
        req!(iter.next(), Some(dat!(32u8)));
        req!(iter.next(), Some(dat!("key4")));
        req!(iter.next(), Some(dat!(4u8)));
        req!(iter.next(), Some(dat!(5u8)));
        req!(iter.next(), Some(dat!("val5")));
        req!(iter.next(), Some(dat!("val6")));
        req!(iter.next(), Some(dat!(7u8)));
        req!(iter.next(), Some(dat!("val7")));
        req!(iter.next(), None::<Dat>);
        req!(iter.next(), None::<Dat>);
        req!(iter.next(), None::<Dat>);
        Ok(())
    }));

    res!(test_it(filter, &["IterDat 030", "all", "iter"], || {
        let mut d = omapdat!{
            listdat!["key11", 12u8] => 1u8,
            "key2" => "val2",
            3u8 => 4u8,
        };
        {
            let mut iter = IterDatValsMut::new(&mut d);
            req!(iter.next(), Some(&mut dat!(1u8)));
            req!(iter.next(), Some(&mut dat!("val2")));
            if let Some(Dat::U8(n)) = iter.next() {
                *n = 6;
            }
        }
        let mut iter2 = IterDatValsMut::new(&mut d);
        iter2.next();
        iter2.next();
        req!(iter2.next(), Some(&mut dat!(6u8)));
        req!(iter2.next(), None::<&mut Dat>);
        req!(iter2.next(), None::<&mut Dat>);
        req!(iter2.next(), None::<&mut Dat>);
        Ok(())
    }));

    res!(test_it(filter, &["Simple dat", "all", "i32"], || {
        let d = dat!(42i32);
        let n = res!(match d {
            Dat::I32(n) => Ok(n),
            _ => Err(err!("A Dat::I32 was expected, found {:?}", d;
            ErrTag::Input, ErrTag::Conversion, ErrTag::Mismatch)),
        });
        req!(n, 42i32);
        Ok(())
    }));

    res!(test_it(filter, &["Derive struct from map 000", "all", "map", "struct"], || {
        
        #[derive(Debug, Default, FromDatMap, PartialEq)]
        struct S0 {
            #[rename(name = "Number")]
            a: i32,
            b: bool,
            s: String,
            s2: String,
            byts:   Vec<u8>,
            f0:     Float32, // cannot use f32, f64
            big:    BigInt,
            key0:   Box<Dat>,
            c64:   Dat,
            u64_0:  u64,
            #[optional]
            u64_1:  u64,
            list0:  Vec<Dat>,
            map0:   DaticleMap,
            map1:   Dat,
            #[skip]
            s1:     S1,
        }

        #[derive(Debug, Default, FromDatMap, PartialEq)]
        struct S1 {
            x: u8,
            y: bool,
        }

        let d0 = mapdat!{
            "Number" => dat!(42i32),
            "b"     =>  dat!(true),
            "s"     =>  "Euler",
            "s2"    =>  "",
            "byts"  =>  vec![1u8, 2, 3, 4],
            "f0"    =>  -42.0f32,
            "big"   =>  dat!(res!(aint!("-4200000000000000000"))),
            "key0"  =>  Box::new(dat!(42u8)),
            "c64"  =>  Dat::C64(42),
            "u64_0" =>  42u64,
            // u64_1 deliberately omitted
            "list0" =>  listdat![1,2,3,4],
            "map0"  =>  mapdat!{
                "x" =>  2u8,
                "y" =>  true,
            },
            "map1"  =>  mapdat!{
                "x" =>  2u8,
                "y" =>  true,
            },
            // s1 deliberately omitted
        };

        let s0 = S0 {
            a:      42i32,
            b:      true,
            s:      "Euler".to_string(),
            s2:     "".to_string(),
            byts:   vec![1u8, 2, 3, 4],
            f0:     Float32(-42.0),
            big:    res!(aint!("-4200000000000000000")),
            key0:   Box::new(dat!(42u8)),
            c64:   Dat::C64(42),
            u64_0:  42,
            u64_1:  0,
            list0:  listdat![1,2,3,4].get_list().unwrap(),
            map0:   mapdat!{
                "x" => 2u8,
                "y" => true,
            }.get_map().unwrap(),
            map1:   mapdat!{
                "x" => 2u8,
                "y" => true,
            },
            s1:     S1::default(),
        };

        let s1 = S1 {
            x:  2,
            y:  true,
        };

        if let Dat::Map(map) = d0 {
            let s_0 = res!(S0::from_datmap(map));
            req!(s_0, s0);
            if let Dat::C64(c64_0) = s0.c64 {
                req!(c64_0, 42);
            }
            let s_1 = res!(S1::from_datmap(s0.map0));
            req!(s_1, s1);
            if let Dat::Map(map) = s0.map1 {
                let s_2 = res!(S1::from_datmap(map));
                req!(s_2, s1);
            }
        }
        Ok(())
    }));

    res!(test_it(filter, &["Derive map from struct 000", "all", "map", "struct"], || {

        #[derive(Debug, Default, ToDatMap)]
        struct S0 {
            #[rename(name = "Number")]
            a: i32,
            b: bool,
            d: Dat,
            m: DaticleMap,
        }

        let m0 = mapdat!{
            1 => 2,
            3 => 4,
        };

        let s = S0 {
            a: 42i32,
            b: true,
            d: Dat::Str("".to_string()),
            m: m0.get_map().unwrap(),
        };

        let d0 = mapdat!{
            "Number" => dat!(42i32),
            "b" => dat!(true),
            "d" => Dat::Str("".to_string()),
            "m" => m0.get_map().unwrap(),
        };

        let d = S0::to_datmap(s);
        req!(d, d0);
        Ok(())
    }));

    // Atomic Kinds ===========================
    // Logic
    
    res!(test_it(filter, &["From 000", "all", "from", "empty"], || {
        req!(Dat::from(()).kind(), Kind::Empty);
        Ok(())
    }));
    
    res!(test_it(filter, &["From 001", "all", "from", "best", "empty"], || {
        req!(Dat::best_from(()).kind(), Kind::Empty);
        Ok(())
    }));
    
    res!(test_it(filter, &["From 010", "all", "from", "bool"], || {
    	req!(Dat::from(true).kind(), Kind::True);
        Ok(())
    }));
    
    res!(test_it(filter, &["From 011", "all", "from", "best", "bool"], || {
    	req!(Dat::best_from(true).kind(), Kind::True);
        Ok(())
    }));
    
    res!(test_it(filter, &["From 020", "all", "from", "bool"], || {
    	req!(Dat::from(false).kind(), Kind::False);
        Ok(())
    }));
    
    res!(test_it(filter, &["From 021", "all", "from", "best", "bool"], || {
    	req!(Dat::best_from(false).kind(), Kind::False);
        Ok(())
    }));
    
    res!(test_it(filter, &["From 025", "all", "from", "opt"], || {
    	req!(Dat::from(None::<bool>).kind(), Kind::None);
        Ok(())
    }));
    
    res!(test_it(filter, &["From 026", "all", "from", "best", "opt"], || {
    	req!(Dat::best_from(None::<bool>).kind(), Kind::None);
        Ok(())
    }));
    // Fixed
    
    res!(test_it(filter, &["Explicit u8", "all", "u8"], || {
    	req!(Dat::U8(0).kind(), Kind::U8);
        Ok(())
    }));
    
    res!(test_it(filter, &["From 031", "all", "from", "u8"], || {
    	req!(Dat::from(0u8).kind(), Kind::U8);
        Ok(())
    }));
    
    res!(test_it(filter, &["From 032", "all", "from", "best", "u8"], || {
    	req!(Dat::best_from(0u8).kind(), Kind::U8);
        Ok(())
    }));

    // Test min-sizing as well as From.
    
    res!(test_it(filter, &["Explicit u16", "all", "u16"], || {
    	req!(Dat::U16(0).kind(), Kind::U16);
        Ok(())
    }));
    
    res!(test_it(filter, &["From 041", "all", "from", "u16", "hex"], || {
    	req!(Dat::from(0x_00_ffu16).kind(), Kind::U16);
        Ok(())
    }));
    
    res!(test_it(filter, &["From 042", "all", "from", "u16", "hex"], || {
    	req!(Dat::from(0x_01_00u16).kind(), Kind::U16);
        Ok(())
    }));
    
    res!(test_it(filter, &["From 043", "all", "from", "best", "u8", "u16", "hex", "minsize"], || {
    	req!(Dat::best_from(0x_00_ffu16).kind(), Kind::U8);
        Ok(())
    }));
    
    res!(test_it(filter, &["From 044", "all", "from", "best", "u16", "hex", "minsize"], || {
    	req!(Dat::best_from(0x_01_00u16).kind(), Kind::U16);
        Ok(())
    }));

    
    res!(test_it(filter, &["Explicit u32", "all", "u32"], || {
    	req!(Dat::U32(0).kind(), Kind::U32);
        Ok(())
    }));
    
    res!(test_it(filter, &["From 051", "all", "from", "u32", "hex"], || {
    	req!(Dat::from(0x_00_00_00_ffu32).kind(), Kind::U32);
        Ok(())
    }));
    
    res!(test_it(filter, &["From 052", "all", "from", "u32", "hex"], || {
    	req!(Dat::from(0x_00_00_ff_ffu32).kind(), Kind::U32);
        Ok(())
    }));
    
    res!(test_it(filter, &["From 053", "all", "from", "u32", "hex"], || {
    	req!(Dat::from(0x_00_01_00_00u32).kind(), Kind::U32);
        Ok(())
    }));
    
    res!(test_it(filter, &["From 054", "all", "from", "best", "u8", "u32", "minsize"], || {
    	req!(Dat::best_from(0x_00_00_00_ffu32).kind(), Kind::U8);
        Ok(())
    }));
    
    res!(test_it(filter, &["From 055", "all", "from", "best", "u16", "u32", "minsize"], || {
    	req!(Dat::best_from(0x_00_00_ff_ffu32).kind(), Kind::U16);
        Ok(())
    }));
    
    res!(test_it(filter, &["From 056", "all", "from", "best", "u32", "minsize"], || {
    	req!(Dat::best_from(0x_00_01_00_00u32).kind(), Kind::U32);
        Ok(())
    }));

    
    res!(test_it(filter, &["Explicit u64", "all", "u64"], || {
    	req!(Dat::U64(0).kind(), Kind::U64);
        Ok(())
    }));
    
    res!(test_it(filter, &["From 061", "all", "from", "u64", "hex"], || {
    	req!(Dat::from(0x_00_00_00_00_00_00_00_ffu64).kind(), Kind::U64);
        Ok(())
    }));
    
    res!(test_it(filter, &["From 062", "all", "from", "u64", "hex"], || {
    	req!(Dat::from(0x_00_00_00_00_00_00_ff_ffu64).kind(), Kind::U64);
        Ok(())
    }));
    
    res!(test_it(filter, &["From 063", "all", "from", "u64", "hex"], || {
    	req!(Dat::from(0x_00_00_00_00_ff_ff_ff_ffu64).kind(), Kind::U64);
        Ok(())
    }));
    
    res!(test_it(filter, &["From 064", "all", "from", "u64", "hex"], || { 
    	req!(Dat::from(0x_00_00_00_01_00_00_00_00u64).kind(), Kind::U64);
        Ok(())
    }));
    
    res!(test_it(filter, &["From 065", "all", "from", "best", "u8", "u64", "hex", "minsize"], || {
    	req!(Dat::best_from(0x_00_00_00_00_00_00_00_ffu64).kind(), Kind::U8);
        Ok(())
    }));
    
    res!(test_it(filter, &["From 066", "all", "from", "best", "u16", "u64", "hex", "minsize"], || {
    	req!(Dat::best_from(0x_00_00_00_00_00_00_ff_ffu64).kind(), Kind::U16);
        Ok(())
    }));
    
    res!(test_it(filter, &["From 067", "all", "from", "best", "u32", "u64", "hex", "minsize"], || {
    	req!(Dat::best_from(0x_00_00_00_00_ff_ff_ff_ffu64).kind(), Kind::U32);
        Ok(())
    }));
    
    res!(test_it(filter, &["From 068", "all", "from", "best", "u64", "hex", "minsize"], || {
    	req!(Dat::best_from(0x_00_00_00_01_00_00_00_00u64).kind(), Kind::U64);
        Ok(())
    }));

    
    res!(test_it(filter, &["Explicit u128", "all", "u128"], || {
    	req!(Dat::U128(0).kind(), Kind::U128);
        Ok(())
    }));
    
    res!(test_it(filter, &["From 071", "all", "from", "u128", "hex"], || {
        req!(Dat::from(0x_00_00_00_00_00_00_00_00_00_00_00_00_00_00_00_ffu128).kind(), Kind::U128);
        Ok(())
    }));
    
    res!(test_it(filter, &["From 072", "all", "from", "u128", "hex"], || {
        req!(Dat::from(0x_00_00_00_00_00_00_00_00_00_00_00_00_00_00_ff_ffu128).kind(), Kind::U128);
        Ok(())
    }));
    
    res!(test_it(filter, &["From 073", "all", "from", "u128", "hex"], || {
        req!(Dat::from(0x_00_00_00_00_00_00_00_00_00_00_00_00_ff_ff_ff_ffu128).kind(), Kind::U128);
        Ok(())
    }));
    
    res!(test_it(filter, &["From 074", "all", "from", "u128", "hex"], || {
        req!(Dat::from(0x_00_00_00_00_00_00_00_00_ff_ff_ff_ff_ff_ff_ff_ffu128).kind(), Kind::U128);
        Ok(())
    }));
    
    res!(test_it(filter, &["From 075", "all", "from", "u128", "hex"], || {
        req!(Dat::from(0x_00_00_00_00_00_00_00_01_00_00_00_00_00_00_00_00u128).kind(), Kind::U128);
        Ok(())
    }));
    
    res!(test_it(filter, &["From 076", "all", "from", "best", "u8", "u128", "hex", "minsize"], || {
        req!(Dat::best_from(0x_00_00_00_00_00_00_00_00_00_00_00_00_00_00_00_ffu128).kind(), Kind::U8);
        Ok(())
    }));
    
    res!(test_it(filter, &["From 077", "all", "from", "best", "u16", "u128", "hex", "minsize"], || { 
        req!(Dat::best_from(0x_00_00_00_00_00_00_00_00_00_00_00_00_00_00_ff_ffu128).kind(), Kind::U16);
        Ok(())
    }));
    
    res!(test_it(filter, &["From 078", "all", "from", "best", "u32", "u128", "hex", "minsize"], || { 
        req!(Dat::best_from(0x_00_00_00_00_00_00_00_00_00_00_00_00_ff_ff_ff_ffu128).kind(), Kind::U32);
        Ok(())
    }));
    
    res!(test_it(filter, &["From 079", "all", "from", "best", "u64", "u128", "hex", "minsize"], || { 
        req!(Dat::best_from(0x_00_00_00_00_00_00_00_00_ff_ff_ff_ff_ff_ff_ff_ffu128).kind(), Kind::U64);
        Ok(())
    }));
    
    res!(test_it(filter, &["From 080", "all", "from", "best", "u128", "hex", "minsize"], || { 
        req!(Dat::best_from(0x_00_00_00_00_00_00_00_01_00_00_00_00_00_00_00_00u128).kind(), Kind::U128);
        Ok(())
    }));

    
    res!(test_it(filter, &["Explicit i8", "all", "i8"], || {
    	req!(Dat::I8(0).kind(), Kind::I8);
        Ok(())
    }));
    
    res!(test_it(filter, &["From 086", "all", "from", "i8"], || {
    	req!(Dat::from(0i8).kind(), Kind::I8);
        Ok(())
    }));
    
    res!(test_it(filter, &["From 087", "all", "from", "best", "i8"], || {
    	req!(Dat::best_from(0i8).kind(), Kind::I8);
        Ok(())
    }));
    
    res!(test_it(filter, &["Explicit i16", "all", "i16"], || {
    	req!(Dat::I16(0).kind(), Kind::I16);
        Ok(())
    }));
    
    res!(test_it(filter, &["From 091", "all", "from", "i16"], || {
    	req!(Dat::from(-128i16).kind(), Kind::I16);
        Ok(())
    }));
    
    res!(test_it(filter, &["From 092", "all", "from", "i16"], || {
    	req!(Dat::from(127i16).kind(), Kind::I16);
        Ok(())
    }));
    
    res!(test_it(filter, &["From 093", "all", "from", "best", "i8", "i16", "minsize"], || {
    	req!(Dat::best_from(-128i16).kind(), Kind::I8);
        Ok(())
    }));
    
    res!(test_it(filter, &["From 094", "all", "from", "best", "u8", "i16", "minsize"], || {
    	req!(Dat::best_from(127i16).kind(), Kind::U8);
        Ok(())
    }));
    
    res!(test_it(filter, &["From 095", "all", "from", "best", "i16", "i16", "minsize"], || {
    	req!(Dat::best_from(-129i16).kind(), Kind::I16);
        Ok(())
    }));
    
    res!(test_it(filter, &["From 096", "all", "from", "best", "u8", "i16", "minsize"], || {
    	req!(Dat::best_from(128i16).kind(), Kind::U8);
        Ok(())
    }));
    
    res!(test_it(filter, &["From 097", "all", "from", "best", "u16", "i16", "minsize"], || {
    	req!(Dat::best_from(256i16).kind(), Kind::U16);
        Ok(())
    }));

    
    res!(test_it(filter, &["Explicit i32", "all", "i32"], || {
    	req!(Dat::I32(0).kind(), Kind::I32);
        Ok(())
    }));
    
    res!(test_it(filter, &["From 101", "all", "from", "i32"], || {
    	req!(Dat::from(-128i32).kind(), Kind::I32);
        Ok(())
    }));
    
    res!(test_it(filter, &["From 102", "all", "from", "i32"], || {
    	req!(Dat::from(127i32).kind(), Kind::I32);
        Ok(())
    }));
    
    res!(test_it(filter, &["From 103", "all", "from", "best", "i8", "i32", "minsize"], || {
    	req!(Dat::best_from(-128i32).kind(), Kind::I8);
        Ok(())
    }));
    
    res!(test_it(filter, &["From 104", "all", "from", "best", "u8", "i32", "minsize"], || {
    	req!(Dat::best_from(127i32).kind(), Kind::U8);
        Ok(())
    }));
    
    res!(test_it(filter, &["From 105", "all", "from", "best", "i16", "i32", "minsize"], || {
    	req!(Dat::best_from(-32_768i32).kind(), Kind::I16);
        Ok(())
    }));
    
    res!(test_it(filter, &["From 106", "all", "from", "best", "u16", "i32", "minsize"], || {
    	req!(Dat::best_from(32_767i32).kind(), Kind::U16);
        Ok(())
    }));
    
    res!(test_it(filter, &["From 107", "all", "from", "best", "i32", "minsize"], || {
    	req!(Dat::best_from(-32_769i32).kind(), Kind::I32);
        Ok(())
    }));
    
    res!(test_it(filter, &["From 108", "all", "from", "best", "u16", "i32", "minsize"], || {
    	req!(Dat::best_from(32_768i32).kind(), Kind::U16);
        Ok(())
    }));

    
    res!(test_it(filter, &["Explicit i64", "all", "i64"], || {
    	req!(Dat::I64(0).kind(), Kind::I64);
        Ok(())
    }));
    
    res!(test_it(filter, &["From 111", "all", "from", "i64"], || {
    	req!(Dat::from(-128i64).kind(), Kind::I64);
        Ok(())
    }));
    
    res!(test_it(filter, &["From 112", "all", "from", "i64"], || {
    	req!(Dat::from(127i64).kind(), Kind::I64);
        Ok(())
    }));
    
    res!(test_it(filter, &["From 113", "all", "from", "best", "i8", "i64", "minsize"], || {
    	req!(Dat::best_from(-128i64).kind(), Kind::I8);
        Ok(())
    }));
    
    res!(test_it(filter, &["From 114", "all", "from", "best", "u8", "i64", "minsize"], || {
    	req!(Dat::best_from(127i64).kind(), Kind::U8);
        Ok(())
    }));
    
    res!(test_it(filter, &["From 115", "all", "from", "best", "i16", "i64", "minsize"], || {
    	req!(Dat::best_from(-32_768i64).kind(), Kind::I16);
        Ok(())
    }));
    
    res!(test_it(filter, &["From 116", "all", "from", "best", "u16", "i64", "minsize"], || {
    	req!(Dat::best_from(32_767i64).kind(), Kind::U16);
        Ok(())
    }));
    
    res!(test_it(filter, &["From 117", "all", "from", "best", "i32", "i64", "minsize"], || {
    	req!(Dat::best_from(-2_147_483_648i64).kind(), Kind::I32);
        Ok(())
    }));
    
    res!(test_it(filter, &["From 118", "all", "from", "best", "u32", "i64", "minsize"], || {
    	req!(Dat::best_from(2_147_483_647i64).kind(), Kind::U32);
        Ok(())
    }));
    
    res!(test_it(filter, &["From 119", "all", "from", "best", "i64", "minsize"], || {
    	req!(Dat::best_from(-2_147_483_649i64).kind(), Kind::I64);
        Ok(())
    }));
    
    res!(test_it(filter, &["From 120", "all", "from", "best", "u32", "i64", "minsize"], || {
    	req!(Dat::best_from(2_147_483_648i64).kind(), Kind::U32);
        Ok(())
    }));

    
    res!(test_it(filter, &["Explicit i128", "all"], || {
    	req!(Dat::I128(0).kind(), Kind::I128);
        Ok(())
    }));
    
    res!(test_it(filter, &["From 131", "all", "from", "i128"], || {
    	req!(Dat::from(-128i128).kind(), Kind::I128);
        Ok(())
    }));
    
    res!(test_it(filter, &["From 132", "all", "from", "i128"], || {
    	req!(Dat::from(127i128).kind(), Kind::I128);
        Ok(())
    }));
    
    res!(test_it(filter, &["From 133", "all", "from", "best", "i8", "i128", "minsize"], || {
    	req!(Dat::best_from(-128i128).kind(), Kind::I8);
        Ok(())
    }));
    
    res!(test_it(filter, &["From 134", "all", "from", "best", "u8", "i128", "minsize"], || {
    	req!(Dat::best_from(127i128).kind(), Kind::U8);
        Ok(())
    }));
    
    res!(test_it(filter, &["From 135", "all", "from", "best", "i16", "i128", "minsize"], || {
    	req!(Dat::best_from(-32_768i128).kind(), Kind::I16);
        Ok(())
    }));
    
    res!(test_it(filter, &["From 136", "all", "from", "best", "u16", "i128", "minsize"], || {
    	req!(Dat::best_from(32_767i128).kind(), Kind::U16);
        Ok(())
    }));
    
    res!(test_it(filter, &["From 137", "all", "from", "best", "i32", "i128", "minsize"], || {
    	req!(Dat::best_from(-2_147_483_648i128).kind(), Kind::I32);
        Ok(())
    }));
    
    res!(test_it(filter, &["From 138", "all", "from", "best", "u32", "i128", "minsize"], || {
    	req!(Dat::best_from(2_147_483_647i128).kind(), Kind::U32);
        Ok(())
    }));
    
    res!(test_it(filter, &["From 139", "all", "from", "best", "i64", "i128", "minsize"], || {
    	req!(Dat::best_from(-9_223_372_036_854_775_808i128).kind(), Kind::I64);
        Ok(())
    }));
    
    res!(test_it(filter, &["From 140", "all", "from", "best", "u64", "i128", "minsize"], || {
    	req!(Dat::best_from(9_223_372_036_854_775_807i128).kind(), Kind::U64);
        Ok(())
    }));
    
    res!(test_it(filter, &["From 141", "all", "from", "best", "i128", "minsize"], || {
    	req!(Dat::best_from(-9_223_372_036_854_775_809i128).kind(), Kind::I128);
        Ok(())
    }));
    
    res!(test_it(filter, &["From 142", "all", "from", "best", "u64", "i128", "minsize"], || {
    	req!(Dat::best_from(9_223_372_036_854_775_808i128).kind(), Kind::U64);
        Ok(())
    }));

    
    res!(test_it(filter, &["From 150", "all", "from", "usize"], || {
        req!(
            res!(Dat::try_from(0usize)).kind(),
            match std::mem::size_of::<usize>() {
                1   => Kind::U8,
                2   => Kind::U16,
                4   => Kind::U32,
                8   => Kind::U64,
                16  => Kind::U128,
                s   => return Err(err!(
                    "The usize for this machine is {}, which has not yet been \
                    mapped to a daticle kind.", s;
                System, Unimplemented, Bug,)),
            },
        );
        Ok(())
    }));

    
    res!(test_it(filter, &["From 151", "all", "from", "isize"], || {
        req!(
            res!(Dat::try_from(0isize)).kind(),
            match std::mem::size_of::<isize>() {
                1   => Kind::I8,
                2   => Kind::I16,
                4   => Kind::I32,
                8   => Kind::I64,
                16  => Kind::I128,
                s   => return Err(err!(
                    "The isize for this machine is {}, which has not yet been \
                    mapped to a daticle kind.", s;
                System, Unimplemented, Bug,)),
            },
        );
        Ok(())
    }));

    
    res!(test_it(filter, &["From 160", "all", "from", "f32"], || {
    	req!(Dat::from(Float32(0.0f32)).kind(), Kind::F32);
        Ok(())
    }));
    
    res!(test_it(filter, &["From 161", "all", "from", "f32"], || {
    	req!(Dat::from(0.0f32).kind(), Kind::F32);
        Ok(())
    }));
    
    res!(test_it(filter, &["From 162", "all", "from", "f64"], || {
    	req!(Dat::from(Float64(0.0f64)).kind(), Kind::F64);
        Ok(())
    }));
    
    res!(test_it(filter, &["From 163", "all", "from", "f64"], || {
    	req!(Dat::from(0.0f64).kind(), Kind::F64);
        Ok(())
    }));
    
    res!(test_it(filter, &["From 164", "all", "from", "best", "f32"], || {
    	req!(Dat::best_from(Float32(0.0f32)).kind(), Kind::F32);
        Ok(())
    }));
    
    res!(test_it(filter, &["From 165", "all", "from", "best", "f32"], || {
    	req!(Dat::best_from(0.0f32).kind(), Kind::F32);
        Ok(())
    }));
    
    res!(test_it(filter, &["From 166", "all", "from", "best", "f64"], || {
    	req!(Dat::best_from(Float64(0.0f64)).kind(), Kind::F64);
        Ok(())
    }));
    
    res!(test_it(filter, &["From 167", "all", "from", "best", "f64"], || {
    	req!(Dat::best_from(0.0f64).kind(), Kind::F64);
        Ok(())
    }));

    // Variable
    
    res!(test_it(filter, &["From 170", "all", "from", "aint"], || {
        req!(Dat::from(res!(BigInt::from_str("0"))).kind(), Kind::Aint);
        Ok(())
    }));
    
    res!(test_it(filter, &["From 171", "all", "from", "best", "aint"], || {
        req!(Dat::best_from(res!(BigInt::from_str("0"))).kind(), Kind::Aint);
        Ok(())
    }));
    
    res!(test_it(filter, &["From 172", "all", "from", "aint"], || {
        req!(Dat::from(res!(aint!("0"))).kind(), Kind::Aint);
        Ok(())
    }));
    
    res!(test_it(filter, &["From 173", "all", "from", "aint"], || {
        req!(Dat::from(res!(aint!(fmt!("{}0", u128::MAX)))).kind(), Kind::Aint);
        Ok(())
    }));
    
    res!(test_it(filter, &["From 180", "all", "from", "adec"], || {
        req!(Dat::from(res!(BigDecimal::from_str("0"))).kind(), Kind::Adec);
        Ok(())
    }));
    
    res!(test_it(filter, &["From 181", "all", "from", "best", "adec"], || {
        req!(Dat::best_from(res!(BigDecimal::from_str("0"))).kind(), Kind::Adec);
        Ok(())
    }));
    
    res!(test_it(filter, &["From 182", "all", "from", "best", "adec"], || {
        req!(Dat::best_from(res!(adec!("0"))).kind(), Kind::Adec);
        Ok(())
    }));

    // No From impl for Dat::C64(_)
    
    res!(test_it(filter, &["From 185", "all", "from", "str"], || {
    	req!(Dat::from(String::new()).kind(), Kind::Str);
        Ok(())
    }));
    
    res!(test_it(filter, &["From 186", "all", "from", "best", "str"], || {
    	req!(Dat::best_from(String::new()).kind(), Kind::Str);
        Ok(())
    }));
    
    res!(test_it(filter, &["From 187", "all", "from", "str"], || {
    	req!(Dat::from("").kind(), Kind::Str);
        Ok(())
    }));
    
    res!(test_it(filter, &["From 188", "all", "from", "str"], || {
    	req!(Dat::best_from("").kind(), Kind::Str);
        Ok(())
    }));

    // Molecular Kinds ========================
    // Unitary
    
    res!(test_it(filter, &["From 190", "all", "from", "usr"], || {
        let ukid = UsrKindId::new(5, Some("my_type"), Some(Kind::I32));
        let d0 = dat!(42);
        let d1 = res!(Dat::try_from((ukid.clone(), Some(d0.clone()))));
        req!(d1.kind(), Kind::Usr(ukid.clone()));
        Ok(())
    }));
    
    res!(test_it(filter, &["From 193", "all", "from", "box"], || {
        req!(
            Dat::from(Box::new(0u8)).kind(),
            Kind::Box(Some(Box::new(Kind::U8))),
        );
        Ok(())
    }));
    
    res!(test_it(filter, &["From 194", "all", "from", "best", "box"], || {
        req!(
            Dat::best_from(Box::new(0u8)).kind(),
            Kind::Box(Some(Box::new(Kind::U8))),
        );
        Ok(())
    }));

    
    res!(test_it(filter, &["From 200", "all", "from", "opt"], || {
        req!(
            Dat::from(Some(0u8)).kind(),
            Kind::Some(Some(Box::new(Kind::U8))),
        );
        Ok(())
    }));
    
    res!(test_it(filter, &["From 201", "all", "from", "best", "opt"], || {
        req!(
            Dat::best_from(Some(0u8)).kind(),
            Kind::Some(Some(Box::new(Kind::U8))),
        );
        Ok(())
    }));

    // Heterogenous
    
    res!(test_it(filter, &["From 214", "all", "from", "list"], || {
    	req!(Dat::from(vec![dat!(false)]).kind(), Kind::List);
        Ok(())
    }));
    
    res!(test_it(filter, &["From 215", "all", "from", "best", "list"], || {
    	req!(Dat::best_from(vec![dat!(false)]).kind(), Kind::List);
        Ok(())
    }));

    
    res!(test_it(filter, &["From 220", "all", "from", "tuple"], || {
        req!(Dat::from([
            dat!(0),
            dat!(""),
        ]).kind(), Kind::Tup2);
        Ok(())
    }));
    
    res!(test_it(filter, &["From 221", "all", "from", "tuple"], || {
        req!(Dat::from([
            dat!(0),
            dat!(""),
            dat!(0),
        ]).kind(), Kind::Tup3);
        Ok(())
    }));
    
    res!(test_it(filter, &["From 222", "all", "from", "tuple"], || {
        req!(Dat::from([
            dat!(0),
            dat!(""),
            dat!(0),
            dat!(""),
        ]).kind(), Kind::Tup4);
        Ok(())
    }));
    
    res!(test_it(filter, &["From 223", "all", "from", "tuple"], || {
        req!(Dat::from([
            dat!(0),
            dat!(""),
            dat!(0),
            dat!(""),
            dat!(0),
        ]).kind(), Kind::Tup5);
        Ok(())
    }));
    
    res!(test_it(filter, &["From 224", "all", "from", "tuple"], || {
        req!(Dat::from([
            dat!(0),
            dat!(""),
            dat!(0),
            dat!(""),
            dat!(0),
            dat!(""),
        ]).kind(), Kind::Tup6);
        Ok(())
    }));
    
    res!(test_it(filter, &["From 225", "all", "from", "tuple"], || {
        req!(Dat::from([
            dat!(0),
            dat!(""),
            dat!(0),
            dat!(""),
            dat!(0),
            dat!(""),
            dat!(0),
        ]).kind(), Kind::Tup7);
        Ok(())
    }));
    
    res!(test_it(filter, &["From 226", "all", "from", "tuple"], || {
        req!(Dat::from([
            dat!(0),
            dat!(""),
            dat!(0),
            dat!(""),
            dat!(0),
            dat!(""),
            dat!(0),
            dat!(""),
        ]).kind(), Kind::Tup8);
        Ok(())
    }));
    
    res!(test_it(filter, &["From 227", "all", "from", "tuple"], || {
        req!(Dat::from([
            dat!(0),
            dat!(""),
            dat!(0),
            dat!(""),
            dat!(0),
            dat!(""),
            dat!(0),
            dat!(""),
            dat!(0),
        ]).kind(), Kind::Tup9);
        Ok(())
    }));
    
    res!(test_it(filter, &["From 228", "all", "from", "tuple"], || {
        req!(Dat::from([
            dat!(0),
            dat!(""),
            dat!(0),
            dat!(""),
            dat!(0),
            dat!(""),
            dat!(0),
            dat!(""),
            dat!(0),
            dat!(""),
        ]).kind(), Kind::Tup10);
        Ok(())
    }));
    
    res!(test_it(filter, &["From 230", "all", "from", "best", "tuple"], || {
        req!(Dat::best_from([
            dat!(0),
            dat!(""),
        ]).kind(), Kind::Tup2);
        Ok(())
    }));
    
    res!(test_it(filter, &["From 231", "all", "from", "best", "tuple"], || {
        req!(Dat::best_from([
            dat!(0),
            dat!(""),
            dat!(0),
        ]).kind(), Kind::Tup3);
        Ok(())
    }));
    
    res!(test_it(filter, &["From 232", "all", "from", "best", "tuple"], || {
        req!(Dat::best_from([
            dat!(0),
            dat!(""),
            dat!(0),
            dat!(""),
        ]).kind(), Kind::Tup4);
        Ok(())
    }));
    
    res!(test_it(filter, &["From 233", "all", "from", "best", "tuple"], || {
        req!(Dat::best_from([
            dat!(0),
            dat!(""),
            dat!(0),
            dat!(""),
            dat!(0),
        ]).kind(), Kind::Tup5);
        Ok(())
    }));
    
    res!(test_it(filter, &["From 234", "all", "from", "best", "tuple"], || {
        req!(Dat::best_from([
            dat!(0),
            dat!(""),
            dat!(0),
            dat!(""),
            dat!(0),
            dat!(""),
        ]).kind(), Kind::Tup6);
        Ok(())
    }));
    
    res!(test_it(filter, &["From 235", "all", "from", "best", "tuple"], || {
        req!(Dat::best_from([
            dat!(0),
            dat!(""),
            dat!(0),
            dat!(""),
            dat!(0),
            dat!(""),
            dat!(0),
        ]).kind(), Kind::Tup7);
        Ok(())
    }));
    
    res!(test_it(filter, &["From 236", "all", "from", "best", "tuple"], || {
        req!(Dat::best_from([
            dat!(0),
            dat!(""),
            dat!(0),
            dat!(""),
            dat!(0),
            dat!(""),
            dat!(0),
            dat!(""),
        ]).kind(), Kind::Tup8);
        Ok(())
    }));
    
    res!(test_it(filter, &["From 237", "all", "from", "best", "tuple"], || {
        req!(Dat::best_from([
            dat!(0),
            dat!(""),
            dat!(0),
            dat!(""),
            dat!(0),
            dat!(""),
            dat!(0),
            dat!(""),
            dat!(0),
        ]).kind(), Kind::Tup9);
        Ok(())
    }));
    
    res!(test_it(filter, &["From 238", "all", "from", "best", "tuple"], || {
        req!(Dat::best_from([
            dat!(0),
            dat!(""),
            dat!(0),
            dat!(""),
            dat!(0),
            dat!(""),
            dat!(0),
            dat!(""),
            dat!(0),
            dat!(""),
        ]).kind(), Kind::Tup10);
        Ok(())
    }));

    
    res!(test_it(filter, &["From 240", "all", "from", "map"], || {
        let mut map = DaticleMap::new();
        map.insert(1u32.into(), dat!("first"));
        map.insert(dat!(2u32), dat!("second"));
        let d = Dat::from(map); 
        req!(d.kind(), Kind::Map);
        Ok(())
    }));

    // Homogenous
    
    res!(test_it(filter, &["From 250", "all", "from", "vek"], || {
        let v = vec![
            dat!("hello"),
            dat!("world"),
            dat!(42),
        ];
        match Dat::try_vek_from(v.clone()) {
