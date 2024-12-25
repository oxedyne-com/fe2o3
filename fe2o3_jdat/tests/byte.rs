use oxedize_fe2o3_jdat::{
    prelude::*,
    test_binary_encode_decode_byte_tuple,
    usr::{
        UsrKinds,
        UsrKindId,
    },
};

use oxedize_fe2o3_core::{
    prelude::*,
    rand::Rand,
    test::test_it,
};

use oxedize_fe2o3_num::{
    prelude::*,
    BigDecimal,
    BigInt,
};

use std::collections::BTreeMap;

pub fn test_binary_encdec_func(filter: &'static str) -> Outcome<()> {

    res!(test_it(filter, &["Binary omnibus", "all", "omnibus"], || {

        let ukid = UsrKindId::new(5, Some("my_type"), Some(Kind::U8));
        let mut ukids = UsrKinds::new(BTreeMap::new(), BTreeMap::new());
        res!(ukids.add(ukid.clone()));

        // Vec of tuples to be encoded then decoded with various codec settings, and the kind
        // expected from decoding with encoding KindScope::Nothing (as in JSON). 
        let dats: Vec<Dat> = vec![
            dat!(()),           // 1 
            dat!(true),         // 2   
            dat!(false),        // 3 
            dat!(None::<u8>),   // 4 
            dat!(0u8),          // 5 
            dat!(0u16),         // 6 
            dat!(0u32),         // 7 
            dat!(0u64),         // 8 
            dat!(0u128),        // 9 
            dat!(0i8),          // 10
            dat!(0i16),         // 11
            dat!(0i32),         // 12
            dat!(0i64),         // 13
            dat!(0i128),        // 14
            dat!(u8::MAX),      // 15
            dat!(u16::MAX),     // 16
            dat!(u32::MAX),     // 17
            dat!(u64::MAX),     // 18
            dat!(u128::MAX),    // 19
            dat!(i8::MIN),      // 20
            dat!(i16::MIN),     // 21
            dat!(i32::MIN),     // 22
            dat!(i64::MIN),     // 23
            dat!(i128::MIN),    // 24
            dat!(i8::MAX),      // 25
            dat!(i16::MAX),     // 26
            dat!(i32::MAX),     // 27
            dat!(i64::MAX),     // 28
            dat!(i128::MAX),    // 29
            dat!(0.0f32),       // 30
            dat!(f32::MIN),     // 31
            dat!(f32::MAX),     // 32
            dat!(0.0f64),       // 33
            dat!(f64::MIN),     // 34
            dat!(f64::MAX),     // 35
            dat!(res!(aint!(fmt!("{}0", u128::MAX)))),                  // 36
            dat!(res!(aint!(fmt!("{}0", u128::MIN)))),                  // 37
            dat!(res!(adec!(fmt!("{:e}0", f64::MAX)))),                 // 38
            dat!(res!(adec!(fmt!("{:e}0", f64::MIN)))),                 // 39
            Dat::C64(u32::MAX as u64),                                  // 40
            dat!("hello"),                                              // 41
            res!(Dat::try_from((ukid.clone(), Some(best_dat!(42))))),   // 42
            dat!(Box::new(best_dat!(-42))),                             // 43
            dat!(Some(best_dat!(-256))),                                // 44
        ];
        let mut count: usize = 1;
        let total = dats.len();

        for (i, d1) in dats.iter().enumerate() {
            let mut buf = Vec::new();
            buf = res!(d1.to_bytes(buf));
            let (d2, n) = res!(Dat::from_bytes(&buf));
            test!("Encoded, decoded {:?} using {} bytes", d1, n);

            if *d1 != d2 {
                return Err(err!(errmsg!(
                    "Omnibus test {} of {} using dat #{}: The daticle {:?} was \
                    encoded to {:?} then decoded to {:?}.",
                    count, total, i+1, d1, buf, d2,
                ), ErrTag::Test, ErrTag::Mismatch));
            }
            
            test!("Omnibus test {} of {} successfully completed.", count, total);
            count += 1;
        }
        test!("{} tests run", count - 1);
        Ok(())
    }));

    res!(test_it(filter, &["Binary encdec bool true", "all", "unit", "bool"], || {
        let v1 = Dat::Bool(true);
        let mut buf = Vec::new();
        buf = res!(v1.to_bytes(buf));
        let (v2, n) = res!(Dat::from_bytes(&buf));
        req!(n, 1);
        req!(v1, v2);
        req!(v1.byte_len(), Some(n));
        Ok(())
    }));

    res!(test_it(filter, &["Binary encdec bool false", "all", "unit", "bool"], || {
        let v1 = Dat::Bool(false);
        let mut buf = Vec::new();
        buf = res!(v1.to_bytes(buf));
        let (v2, n) = res!(Dat::from_bytes(&buf));
        req!(n, 1);
        req!(v1, v2);
        req!(v1.byte_len(), Some(n));
        Ok(())
    }));

    res!(test_it(filter, &["Binary encdec u8 rand loop", "all", "unit", "u8", "u8"], || {
        for i in 0..100_000 {
            let v1 = Dat::U8(Rand::rand_u32() as u8);
            let mut buf = Vec::new();
            buf = res!(v1.to_bytes(buf));
            let (v2, n) = res!(Dat::from_bytes(&buf));
            req!(n, 2, "Completed {} successful comparisons", i);
            req!(v1, v2, "Completed {} successful comparisons", i);
            req!(v1.byte_len(), Some(n));
        }
        Ok(())
    }));

    res!(test_it(filter, &["Binary encdec u16 rand loop", "all", "unit", "u16"], || {
        for i in 0..100_000 {
            let v1 = Dat::U16(Rand::rand_u32() as u16);
            let mut buf = Vec::new();
            buf = res!(v1.to_bytes(buf));
            let (v2, n) = res!(Dat::from_bytes(&buf));
            req!(n, 3, "Completed {} successful comparisons", i);
            req!(v1, v2, "Completed {} successful comparisons", i);
            req!(v1.byte_len(), Some(n));
        }
        Ok(())
    }));

    res!(test_it(filter, &["Binary encdec u32 rand loop", "all", "unit", "u32"], || {
        for i in 0..100_000 {
            let v1 = Dat::U32(Rand::rand_u32());
            let mut buf = Vec::new();
            buf = res!(v1.to_bytes(buf));
            let (v2, n) = res!(Dat::from_bytes(&buf));
            req!(n, 5, "Completed {} successful comparisons", i);
            req!(v1, v2, "Completed {} successful comparisons", i);
            req!(v1.byte_len(), Some(n));
        }
        Ok(())
    }));

    res!(test_it(filter, &["Binary encdec u64 rand loop", "all", "unit", "u64"], || {
        for i in 0..100_000 {
            let v1 = Dat::U64(Rand::rand_u64());
            let mut buf = Vec::new();
            buf = res!(v1.to_bytes(buf));
            let (v2, n) = res!(Dat::from_bytes(&buf));
            req!(n, 9, "Completed {} successful comparisons", i);
            req!(v1, v2, "Completed {} successful comparisons", i);
            req!(v1.byte_len(), Some(n));
        }
        Ok(())
    }));

    res!(test_it(filter, &["Binary encdec u128 rand loop", "all", "unit", "u128"], || {
        for i in 0..100_000 {
            let v1 = Dat::U128(Rand::rand_u128());
            let mut buf = Vec::new();
            buf = res!(v1.to_bytes(buf));
            let (v2, n) = res!(Dat::from_bytes(&buf));
            req!(n, 17, "Completed {} successful comparisons", i);
            req!(v1, v2, "Completed {} successful comparisons", i);
            req!(v1.byte_len(), Some(n));
        }
        Ok(())
    }));

    res!(test_it(filter, &["Binary encdec i8 rand loop", "all", "unit", "i8"], || {
        for i in 0..100_000 {
            let v1 = Dat::I8(Rand::rand_u32() as i8);
            let mut buf = Vec::new();
            buf = res!(v1.to_bytes(buf));
            let (v2, n) = res!(Dat::from_bytes(&buf));
            req!(n, 2, "Completed {} successful comparisons", i);
            req!(v1, v2, "Completed {} successful comparisons", i);
            req!(v1.byte_len(), Some(n));
        }
        Ok(())
    }));

    res!(test_it(filter, &["Binary encdec i16 rand loop", "all", "unit", "i16"], || {
        for i in 0..100_000 {
            let v1 = Dat::I16(Rand::rand_u32() as i16);
            let mut buf = Vec::new();
            buf = res!(v1.to_bytes(buf));
            let (v2, n) = res!(Dat::from_bytes(&buf));
            req!(n, 3, "Completed {} successful comparisons", i);
            req!(v1, v2, "Completed {} successful comparisons", i);
            req!(v1.byte_len(), Some(n));
        }
        Ok(())
    }));

    res!(test_it(filter, &["Binary encdec i32 rand loop", "all", "unit", "i32"], || {
        for i in 0..100_000 {
            let v1 = Dat::I32(Rand::rand_u32() as i32);
            let mut buf = Vec::new();
            buf = res!(v1.to_bytes(buf));
            let (v2, n) = res!(Dat::from_bytes(&buf));
            req!(n, 5, "Completed {} successful comparisons", i);
            req!(v1, v2, "Completed {} successful comparisons", i);
            req!(v1.byte_len(), Some(n));
        }
        Ok(())
    }));

    res!(test_it(filter, &["Binary encdec i64 rand loop", "all", "unit", "i64"], || {
        for i in 0..100_000 {
            let v1 = Dat::I64(Rand::rand_u64() as i64);
            let mut buf = Vec::new();
            buf = res!(v1.to_bytes(buf));
            let (v2, n) = res!(Dat::from_bytes(&buf));
            req!(n, 9, "Completed {} successful comparisons", i);
            req!(v1, v2, "Completed {} successful comparisons", i);
            req!(v1.byte_len(), Some(n));
        }
        Ok(())
    }));

    res!(test_it(filter, &["Binary encdec i128 rand loop", "all", "unit", "i128"], || {
        for i in 0..100_000 {
            let v1 = Dat::I128(Rand::rand_u128() as i128);
            let mut buf = Vec::new();
            buf = res!(v1.to_bytes(buf));
            let (v2, n) = res!(Dat::from_bytes(&buf));
            req!(n, 17, "Completed {} successful comparisons", i);
            req!(v1, v2, "Completed {} successful comparisons", i);
            req!(v1.byte_len(), Some(n));
        }
        Ok(())
    }));

    res!(test_it(filter, &["Binary encdec f32 rand loop", "all", "unit", "f32"], || {
        for i in 0..100_000 {
            let b = Rand::rand_u32().to_be_bytes();
            let v1 = Dat::from(f32::from_be_bytes(b));
            let mut buf = Vec::new();
            buf = res!(v1.to_bytes(buf));
            let (v2, n) = res!(Dat::from_bytes(&buf));
            req!(n, 5, "Completed {} successful comparisons", i);
            req!(v1, v2, "Completed {} successful comparisons", i);
            req!(v1.byte_len(), Some(n));
        }
        Ok(())
    }));

    res!(test_it(filter, &["Binary encdec f64 rand loop", "all", "unit", "f64"], || {
        for i in 0..100_000 {
            let b = Rand::rand_u64().to_be_bytes();
            let v1 = Dat::from(f64::from_be_bytes(b));
            let mut buf = Vec::new();
            buf = res!(v1.to_bytes(buf));
            let (v2, n) = res!(Dat::from_bytes(&buf));
            req!(n, 9, "Completed {} successful comparisons", i);
            req!(v1, v2, "Completed {} successful comparisons", i);
            req!(v1.byte_len(), Some(n));
        }
        Ok(())
    }));

    res!(test_it(filter, &["Binary encdec BU8 vec", "all", "unit", "bu8"], || {
        let v1 = Dat::BU8(vec![0x72, 0x75, 0x73, 0x74]);
        let mut buf = Vec::new();
        buf = res!(v1.to_bytes(buf));
        let (v2, n) = res!(Dat::from_bytes(&buf));
        req!(n, 1 + 1 + 4);
        req!(v1, v2);
        req!(v1.byte_len(), Some(n));
        Ok(())
    }));

    res!(test_it(filter, &["Binary encdec BU16 vec", "all", "unit", "bu16"], || {
        let v1 = Dat::BU16(vec![0x72, 0x75, 0x73, 0x74]);
        let mut buf = Vec::new();
        buf = res!(v1.to_bytes(buf));
        let (v2, n) = res!(Dat::from_bytes(&buf));
        req!(n, 1 + 2 + 4);
        req!(v1, v2);
        req!(v1.byte_len(), Some(n));
        Ok(())
    }));

    res!(test_it(filter, &["Binary encdec BU32 vec", "all", "unit", "bu32"], || {
        let v1 = Dat::BU32(vec![0x72, 0x75, 0x73, 0x74]);
        let mut buf = Vec::new();
        buf = res!(v1.to_bytes(buf));
        let (v2, n) = res!(Dat::from_bytes(&buf));
        req!(n, 1 + 4 + 4);
        req!(v1, v2);
        req!(v1.byte_len(), Some(n));
        Ok(())
    }));

    res!(test_it(filter, &["Binary encdec BU64 vec", "all", "unit", "bu64"], || {
        let v1 = Dat::BU64(vec![0x72, 0x75, 0x73, 0x74]);
        let mut buf = Vec::new();
        buf = res!(v1.to_bytes(buf));
        let (v2, n) = res!(Dat::from_bytes(&buf));
        req!(n, 1 + 8 + 4);
        req!(v1, v2);
        req!(v1.byte_len(), Some(n));
        Ok(())
    }));
    
    res!(test_it(filter, &["Binary encdec BC64 vec", "all", "unit", "bc64"], || {
        let v1 = Dat::BC64(vec![0x72, 0x75, 0x73, 0x74]);
        let mut buf = Vec::new();
        buf = res!(v1.to_bytes(buf));
        let (v2, n) = res!(Dat::from_bytes(&buf));
        req!(n, 1 + 2 + 4);
        req!(v1, v2);
        req!(v1.byte_len(), Some(n));
        Ok(())
    }));

    res!(test_it(filter, &["Binary encdec Tup5u64 array", "all", "unit", "tuple", "u64"], || {
        let v1 = Dat::Tup5u64([0x72, 0x75, 0x73, 0x74, 0x75]);
        let mut buf = Vec::new();
        buf = res!(v1.to_bytes(buf));
        let (v2, n) = res!(Dat::from_bytes(&buf));
        req!(n, 1 + 5 * 8);
        req!(v1, v2);
        req!(v1.byte_len(), Some(n));
        Ok(())
    }));

    res!(test_it(filter, &["Binary encdec u8 2-tuple", "all", "unit", "tuple", "u8"], || {
        test_binary_encode_decode_byte_tuple! { [42u8; 2] }
        Ok(())
    }));
    
    res!(test_it(filter, &["Binary encdec u8 3-tuple", "all", "unit", "tuple", "u8"], || {
        test_binary_encode_decode_byte_tuple! { [42u8; 3] }
        Ok(())
    }));
    
    res!(test_it(filter, &["Binary encdec u8 4-tuple", "all", "unit", "tuple", "u8"], || {
        test_binary_encode_decode_byte_tuple! { [42u8; 4] }
        Ok(())
    }));
    
    res!(test_it(filter, &["Binary encdec u8 5-tuple", "all", "unit", "tuple", "u8"], || {
        test_binary_encode_decode_byte_tuple! { [42u8; 5] }
        Ok(())
    }));
    
    res!(test_it(filter, &["Binary encdec u8 6-tuple", "all", "unit", "tuple", "u8"], || {
        test_binary_encode_decode_byte_tuple! { [42u8; 6] }
        Ok(())
    }));
    
    res!(test_it(filter, &["Binary encdec u8 7-tuple", "all", "unit", "tuple", "u8"], || {
        test_binary_encode_decode_byte_tuple! { [42u8; 7] }
        Ok(())
    }));
    
    res!(test_it(filter, &["Binary encdec u8 8-tuple", "all", "unit", "tuple", "u8"], || {
        test_binary_encode_decode_byte_tuple! { [42u8; 8] }
        Ok(())
    }));
    
    res!(test_it(filter, &["Binary encdec u8 9-tuple", "all", "unit", "tuple", "u8"], || {
        test_binary_encode_decode_byte_tuple! { [42u8; 9] }
        Ok(())
    }));
    
    res!(test_it(filter, &["Binary encdec u8 10-tuple", "all", "unit", "tuple", "u8"], || {
        test_binary_encode_decode_byte_tuple! { [42u8; 10] }
        Ok(())
    }));
    
    res!(test_it(filter, &["Binary encdec u8 16-tuple", "all", "unit", "tuple", "u8"], || {
        test_binary_encode_decode_byte_tuple! { [42u8; 16] }
        Ok(())
    }));
    
    res!(test_it(filter, &["Binary encdec u8 32-tuple", "all", "unit", "tuple", "u8"], || {
        test_binary_encode_decode_byte_tuple! { [42u8; 32] }
        Ok(())
    }));
    
    res!(test_it(filter, &["Binary encdec u16 2-tuple", "all", "unit", "tuple", "u16"], || {
        test_binary_encode_decode_byte_tuple! { [42u16; 2] }
        Ok(())
    }));
    
    res!(test_it(filter, &["Binary encdec u16 3-tuple", "all", "unit", "tuple", "u16"], || {
        test_binary_encode_decode_byte_tuple! { [42u16; 3] }
        Ok(())
    }));
    
    res!(test_it(filter, &["Binary encdec u16 4-tuple", "all", "unit", "tuple", "u16"], || {
        test_binary_encode_decode_byte_tuple! { [42u16; 4] }
        Ok(())
    }));
    
    res!(test_it(filter, &["Binary encdec u16 5-tuple", "all", "unit", "tuple", "u16"], || {
        test_binary_encode_decode_byte_tuple! { [42u16; 5] }
        Ok(())
    }));
    
    res!(test_it(filter, &["Binary encdec u16 6-tuple", "all", "unit", "tuple", "u16"], || {
        test_binary_encode_decode_byte_tuple! { [42u16; 6] }
        Ok(())
    }));
    
    res!(test_it(filter, &["Binary encdec u16 7-tuple", "all", "unit", "tuple", "u16"], || {
        test_binary_encode_decode_byte_tuple! { [42u16; 7] }
        Ok(())
    }));
    
    res!(test_it(filter, &["Binary encdec u16 8-tuple", "all", "unit", "tuple", "u16"], || {
        test_binary_encode_decode_byte_tuple! { [42u16; 8] }
        Ok(())
    }));
    
    res!(test_it(filter, &["Binary encdec u16 9-tuple", "all", "unit", "tuple", "u16"], || {
        test_binary_encode_decode_byte_tuple! { [42u16; 9] }
        Ok(())
    }));
    
    res!(test_it(filter, &["Binary encdec u16 10-tuple", "all", "unit", "tuple", "u16"], || {
        test_binary_encode_decode_byte_tuple! { [42u16; 10] }
        Ok(())
    }));

    res!(test_it(filter, &["Binary encdec u32 2-tuple", "all", "unit", "tuple", "u32"], || {
        test_binary_encode_decode_byte_tuple! { [42u32; 2] }
        Ok(())
    }));
    
    res!(test_it(filter, &["Binary encdec u32 3-tuple", "all", "unit", "tuple", "u32"], || {
        test_binary_encode_decode_byte_tuple! { [42u32; 3] }
        Ok(())
    }));
    
    res!(test_it(filter, &["Binary encdec u32 4-tuple", "all", "unit", "tuple", "u32"], || {
        test_binary_encode_decode_byte_tuple! { [42u32; 4] }
        Ok(())
    }));
    
    res!(test_it(filter, &["Binary encdec u32 5-tuple", "all", "unit", "tuple", "u32"], || {
        test_binary_encode_decode_byte_tuple! { [42u32; 5] }
        Ok(())
    }));
    
    res!(test_it(filter, &["Binary encdec u32 6-tuple", "all", "unit", "tuple", "u32"], || {
        test_binary_encode_decode_byte_tuple! { [42u32; 6] }
        Ok(())
    }));
    
    res!(test_it(filter, &["Binary encdec u32 7-tuple", "all", "unit", "tuple", "u32"], || {
        test_binary_encode_decode_byte_tuple! { [42u32; 7] }
        Ok(())
    }));
    
    res!(test_it(filter, &["Binary encdec u32 8-tuple", "all", "unit", "tuple", "u32"], || {
        test_binary_encode_decode_byte_tuple! { [42u32; 8] }
        Ok(())
    }));
    
    res!(test_it(filter, &["Binary encdec u32 9-tuple", "all", "unit", "tuple", "u32"], || {
        test_binary_encode_decode_byte_tuple! { [42u32; 9] }
        Ok(())
    }));
    
    res!(test_it(filter, &["Binary encdec u32 10-tuple", "all", "unit", "tuple", "u32"], || {
        test_binary_encode_decode_byte_tuple! { [42u32; 10] }
        Ok(())
    }));

    res!(test_it(filter, &["Binary encdec u64 2-tuple", "all", "unit", "tuple", "u64"], || {
        test_binary_encode_decode_byte_tuple! { [42u64; 2] }
        Ok(())
    }));
    
    res!(test_it(filter, &["Binary encdec u64 3-tuple", "all", "unit", "tuple", "u64"], || {
        test_binary_encode_decode_byte_tuple! { [42u64; 3] }
        Ok(())
    }));
    
    res!(test_it(filter, &["Binary encdec u64 4-tuple", "all", "unit", "tuple", "u64"], || {
        test_binary_encode_decode_byte_tuple! { [42u64; 4] }
        Ok(())
    }));
    
    res!(test_it(filter, &["Binary encdec u64 5-tuple", "all", "unit", "tuple", "u64"], || {
        test_binary_encode_decode_byte_tuple! { [42u64; 5] }
        Ok(())
    }));
    
    res!(test_it(filter, &["Binary encdec u64 6-tuple", "all", "unit", "tuple", "u64"], || {
        test_binary_encode_decode_byte_tuple! { [42u64; 6] }
        Ok(())
    }));
    
    res!(test_it(filter, &["Binary encdec u64 7-tuple", "all", "unit", "tuple", "u64"], || {
        test_binary_encode_decode_byte_tuple! { [42u64; 7] }
        Ok(())
    }));
    
    res!(test_it(filter, &["Binary encdec u64 8-tuple", "all", "unit", "tuple", "u64"], || {
        test_binary_encode_decode_byte_tuple! { [42u64; 8] }
        Ok(())
    }));
    
    res!(test_it(filter, &["Binary encdec u64 9-tuple", "all", "unit", "tuple", "u64"], || {
        test_binary_encode_decode_byte_tuple! { [42u64; 9] }
        Ok(())
    }));
    
    res!(test_it(filter, &["Binary encdec u64 10-tuple", "all", "unit", "tuple", "u64"], || {
        test_binary_encode_decode_byte_tuple! { [42u64; 10] }
        Ok(())
    }));

    res!(test_it(filter, &["Binary encdec string", "all", "unit", "str"], || {
        let v1 = Dat::from("hello world");
        let mut buf = Vec::new();
        buf = res!(v1.to_bytes(buf));
        let (v2, n) = res!(Dat::from_bytes(&buf));
        req!(n, 1 + 2 + 11);
        req!(v1, v2);
        req!(v1.byte_len(), Some(n));
        Ok(())
    }));

    res!(test_it(filter, &["Binary encdec aint 01", "all", "unit", "aint"], || {
        if let Some(bigint) = BigInt::parse_bytes(b"0", 10) {
            let v1 = Dat::from(bigint);
            let mut buf = Vec::new();
            buf = res!(v1.to_bytes(buf));
            let (v2, _) = res!(Dat::from_bytes(&buf));
            req!(v1, v2);
        } else {
            return Err(err!(errmsg!("Problem generating BigInt."), Integer));
        }
        Ok(())
    }));

    res!(test_it(filter, &["Binary encdec aint 02", "all", "unit", "aint"], || {
        if let Some(bigint) = BigInt::parse_bytes(b"12345678901234567890123456789", 10) {
            let v1 = Dat::from(bigint);
            let mut buf = Vec::new();
            buf = res!(v1.to_bytes(buf));
            let (v2, _) = res!(Dat::from_bytes(&buf));
            req!(v1, v2);
        } else {
            return Err(err!(errmsg!("Problem generating BigInt."), Integer));
        }
        Ok(())
    }));

    res!(test_it(filter, &["Binary encdec adec", "all", "unit", "adec"], || {
        let bigdec = res!(BigDecimal::from_str(&"-123.45678901234567890123456789"));
        let v1 = Dat::from(bigdec);
        let mut buf = Vec::new();
        buf = res!(v1.to_bytes(buf));
        let (v2, _) = res!(Dat::from_bytes(&buf));
        req!(v1, v2);
        Ok(())
    }));

    res!(test_it(filter, &["Binary encdec list", "all", "unit", "list"], || {
        let v1 = Dat::from(vec![
            dat!("hello"),
            dat!("world"),
            dat!(42),
        ]);
        let mut buf = Vec::new();
        buf = res!(v1.to_bytes(buf));
        let (v2, _n) = res!(Dat::from_bytes(&buf));
        req!(v1, v2);
        Ok(())
    }));

    res!(test_it(filter, &["Binary encdec list len 2", "all", "unit", "list"], || {
        let v1 = Dat::from([
            dat!("one"),
            dat!("two"),
        ]);
        let mut buf = Vec::new();
        buf = res!(v1.to_bytes(buf));
        let (v2, _n) = res!(Dat::from_bytes(&buf));
        req!(v1, v2);
        Ok(())
    }));

    res!(test_it(filter, &["Binary encdec list len 3", "all", "unit", "list"], || {
        let v1 = Dat::from([
            dat!("one"),
            dat!("two"),
            dat!("three"),
        ]);
        let mut buf = Vec::new();
        buf = res!(v1.to_bytes(buf));
        let (v2, _n) = res!(Dat::from_bytes(&buf));
        req!(v1, v2);
        Ok(())
    }));

    res!(test_it(filter, &["Binary encdec list len 4", "all", "unit", "list"], || {
        let v1 = Dat::from([
            dat!("one"),
            dat!("two"),
            dat!("three"),
            dat!("four"),
        ]);
        let mut buf = Vec::new();
        buf = res!(v1.to_bytes(buf));
        let (v2, _n) = res!(Dat::from_bytes(&buf));
        req!(v1, v2);
        Ok(())
    }));

    res!(test_it(filter, &["Binary encdec list len 5", "all", "unit", "list"], || {
        let v1 = Dat::from([
            dat!("one"),
            dat!("two"),
            dat!("three"),
            dat!("four"),
            dat!("five"),
        ]);
        let mut buf = Vec::new();
        buf = res!(v1.to_bytes(buf));
        let (v2, _n) = res!(Dat::from_bytes(&buf));
        req!(v1, v2);
        Ok(())
    }));

    res!(test_it(filter, &["Binary encdec list len 6", "all", "unit", "list"], || {
        let v1 = Dat::from([
            dat!("one"),
            dat!("two"),
            dat!("three"),
            dat!("four"),
            dat!("five"),
            dat!("six"),
        ]);
        let mut buf = Vec::new();
        buf = res!(v1.to_bytes(buf));
        let (v2, _n) = res!(Dat::from_bytes(&buf));
        req!(v1, v2);
        Ok(())
    }));

    res!(test_it(filter, &["Binary encdec list len 7", "all", "unit", "list"], || {
        let v1 = Dat::from([
            dat!("one"),
            dat!("two"),
            dat!("three"),
            dat!("four"),
            dat!("five"),
            dat!("six"),
            dat!("seven"),
        ]);
        let mut buf = Vec::new();
        buf = res!(v1.to_bytes(buf));
        let (v2, _n) = res!(Dat::from_bytes(&buf));
        req!(v1, v2);
        Ok(())
    }));

    res!(test_it(filter, &["Binary encdec list len 8", "all", "unit", "list"], || {
        let v1 = Dat::from([
            dat!("one"),
            dat!("two"),
            dat!("three"),
            dat!("four"),
            dat!("five"),
            dat!("six"),
            dat!("seven"),
            dat!("eight"),
        ]);
        let mut buf = Vec::new();
        buf = res!(v1.to_bytes(buf));
        let (v2, _n) = res!(Dat::from_bytes(&buf));
        req!(v1, v2);
        Ok(())
    }));

    res!(test_it(filter, &["Binary encdec list len 9", "all", "unit", "list"], || {
        let v1 = Dat::from([
            dat!("one"),
            dat!("two"),
            dat!("three"),
            dat!("four"),
            dat!("five"),
            dat!("six"),
            dat!("seven"),
            dat!("eight"),
            dat!("nine"),
        ]);
        let mut buf = Vec::new();
        buf = res!(v1.to_bytes(buf));
        let (v2, _n) = res!(Dat::from_bytes(&buf));
        req!(v1, v2);
        Ok(())
    }));

    res!(test_it(filter, &["Binary encdec list len 10", "all", "unit", "list"], || {
        let v1 = Dat::from([
            dat!("one"),
            dat!("two"),
            dat!("three"),
            dat!("four"),
            dat!("five"),
            dat!("six"),
            dat!("seven"),
            dat!("eight"),
            dat!("nine"),
            dat!("ten"),
        ]);
        let mut buf = Vec::new();
        buf = res!(v1.to_bytes(buf));
        let (v2, _n) = res!(Dat::from_bytes(&buf));
        req!(v1, v2);
        Ok(())
    }));

    res!(test_it(filter, &["Binary encdec vek", "all", "unit", "vek"], || {
        let v1 = res!(Dat::try_vek_from(vec![
            dat!("hello"),
            dat!("world"),
        ]));
        let mut buf = Vec::new();
        buf = res!(v1.to_bytes(buf));
        let (v2, _n) = res!(Dat::from_bytes(&buf));
        req!(v1, v2);
        Ok(())
    }));
    
    res!(test_it(filter, &["Binary encdec ordmap", "all", "unit", "ordmap"], || {
        let v1 = omapdat!{
            "Meaning of life" => 42u8,
            "key" => "value",
        };
        let mut buf = Vec::new();
        buf = res!(v1.to_bytes(buf));
        let (v2, n) = res!(Dat::from_bytes(&buf));
        req!(n, 37);
        req!(v1, v2);
        Ok(())
    }));

    res!(test_it(filter, &["Binary encdec map", "all", "unit", "map"], || {
        let v1 = mapdat!{
            "Meaning of life" => 42u8,
            "key" => "value",
        };
        let mut buf = Vec::new();
        buf = res!(v1.to_bytes(buf));
        let (v2, n) = res!(Dat::from_bytes(&buf));
        req!(n, 37);
        req!(v1, v2);
        Ok(())
    }));

    res!(test_it(filter, &["Binary encdec usr", "all", "unit", "usr"], || {
        let ukid = UsrKindId::new(5, Some("my_type"), Some(Kind::Str));
        let v0 = dat!("hello world");
        let v1 = Dat::Usr(ukid, Some(Box::new(v0.clone())));
        let mut buf = Vec::new();
        buf = res!(v1.to_bytes(buf));
        let (v2, n) = res!(Dat::from_bytes(&buf));
        req!(n, 1 + 2 + 1 + 1 + 2 + 11);
        req!(v1, v2);
        Ok(())
    }));

    res!(test_it(filter, &["Binary encdec box", "all", "unit", "box"], || {
        let v0 = dat!("hello world");
        let v1 = Dat::Box(Box::new(v0.clone()));
        let mut buf = Vec::new();
        buf = res!(v1.to_bytes(buf));
        let (v2, n) = res!(Dat::from_bytes(&buf));
        req!(n, 1 + 1 + 2 + 11);
        req!(v1, v2);
        Ok(())
    }));

    res!(test_it(filter, &["Binary encdec opt 01", "all", "unit", "opt"], || {
        let v = dat!("hello world");
        let v0 = dat!(Some(v.clone()));
        let v1 = Dat::Opt(Box::new(Some(v)));
        req!(v0, v1);
        let mut buf = Vec::new();
        buf = res!(v1.to_bytes(buf));
        let (v2, n) = res!(Dat::from_bytes(&buf));
        req!(n, 1 + 1 + 2 + 11);
        req!(v1, v2);
        Ok(())
    }));
    
    res!(test_it(filter, &["Binary encdec opt 02", "all", "unit", "opt"], || {
        let v1 = Dat::Opt(Box::new(None));
        let mut buf = Vec::new();
        buf = res!(v1.to_bytes(buf));
        let (v2, n) = res!(Dat::from_bytes(&buf));
        req!(n, 1);
        req!(v1, v2);
        Ok(())
    }));

    res!(test_it(filter, &["Binary encdec c64", "all", "unit", "c64"], || {
        for i in 0..100_000 {
            //let mask = 0x_00_00_00_00_00_00_FF_FF;
            let mask = 0x_FF_FF_FF_FF_FF_FF_FF_FF;
            let v1 = Dat::C64(Rand::rand_u64() & mask);
            let mut buf = Vec::new();
            buf = res!(v1.to_bytes(buf));
            let (v2, _n) = res!(Dat::from_bytes(&buf));
            req!(v1, v2, "Completed {} successful comparisons", i);
        }
        Ok(())
    }));

    res!(test_it(filter, &["Binary count 000", "all", "unit", "count"], || {
        let mut buf = std::io::Cursor::new(Vec::new());
        let c = res!(Dat::count_bytes(&mut buf));
        req!(c, 0);
        Ok(())
    }));

    res!(test_it(filter, &["Binary count 010", "all", "unit", "count"], || {
        let d = dat!("hello this is a test");
        let mut buf = std::io::Cursor::new(Vec::new());
        buf = std::io::Cursor::new(res!(d.to_bytes(buf.into_inner())));
        let c1 = buf.get_ref().len();
        let c2 = res!(Dat::count_bytes(&mut buf));
        req!(c1, c2);
        Ok(())
    }));
    
    res!(test_it(filter, &["Binary count 020", "all", "unit", "count"], || {
        let d1 = dat!("hello this is a test");
        let d2 = dat!("another test");
        let mut buf = std::io::Cursor::new(Vec::new());
        buf = std::io::Cursor::new(res!(d1.to_bytes(buf.into_inner())));
        let c1 = buf.get_ref().len();
        buf = std::io::Cursor::new(res!(d2.to_bytes(buf.into_inner())));
        let c2 = buf.get_ref().len() - c1;
        let c3 = res!(Dat::count_bytes(&mut buf));
        req!(c1, c3);
        let c3 = res!(Dat::count_bytes(&mut buf));
        req!(c2, c3);
        Ok(())
    }));
    
    res!(test_it(filter, &["Binary wrap bytes 00", "all", "unit", "wrap"], || {
        let mut byts = vec![1,2,3,4,5,6,7,8,9];
        let expected = Dat::BC64(byts.clone());
        byts = res!(Dat::wrap_bytes_c64(byts));
        let (result, _) = res!(Dat::from_bytes(&byts));
        req!(result, expected);
        Ok(())
    }));

    res!(test_it(filter, &["Binary wrap bytes 01", "all", "unit", "wrap"], || {
        let mut byts = vec![1,2,3,4,5,6,7,8,9];
        let expected = Dat::BU8(byts.clone());
        byts = res!(Dat::wrap_bytes_var(byts));
        let (result, _) = res!(Dat::from_bytes(&byts));
        req!(result, expected);
        Ok(())
    }));
    
    res!(test_it(filter, &["Binary wrap bytes 02", "all", "unit", "wrap"], || {
        let mut byts = vec![42; 300];
        let expected = Dat::BU16(byts.clone());
        byts = res!(Dat::wrap_bytes_var(byts));
        let (result, _) = res!(Dat::from_bytes(&byts));
        req!(result, expected);
        Ok(())
    }));

    Ok(())
}
