use oxedize_fe2o3_jdat::{
    prelude::*,
    tup2dat,
    tup3dat,
    tup4dat,
    tup5dat,
    tup6dat,
    tup7dat,
    tup8dat,
    tup9dat,
    tup10dat,
    test_string_encode_decode_homogenous_tuple,
    note::NoteConfig,
    string::{
        dec::*,
        enc::*,
    },
    usr::{
        UsrKinds,
        UsrKindId,
    },
};

use oxedize_fe2o3_core::{
    prelude::*,
    test::test_it,
};
use oxedize_fe2o3_num::prelude::*;
use oxedize_fe2o3_text::string::Stringer;

use std::{
    collections::BTreeMap,
    path::Path,
};


pub fn test_string_encdec_func(filter: &'static str) -> Outcome<()> {

    res!(test_it(filter, &["String omnibus", "all", "omnibus"], || {
        let kind_scopes = [
            KindScope::Everything,
            KindScope::Nothing,
            KindScope::Most,
        ];
        let type_lower_cases = [true, false];
        let byte_encodings = [
            ByteEncoding::Base2x,
            ByteEncoding::Binary,
            ByteEncoding::Decimal,
            ByteEncoding::Hex,
            ByteEncoding::Octal,
        ];
        let int_encodings = [
            IntEncoding::Binary,
            IntEncoding::Decimal,
            IntEncoding::Hex,
            IntEncoding::Octal,
        ];
        let trailing_commas_opts = [true, false];
        let hide_usr_types_opts = [true, false];

        let ukind = UsrKindId::new(5, Some("my_type"), Some(Kind::U8));
        let mut ukinds = UsrKinds::new(BTreeMap::new(), BTreeMap::new());
        res!(ukinds.add(ukind.clone()));

        // Vec of tuples to be encoded then decoded with various codec settings, and the kind
        // expected from decoding with encoding KindScope::Nothing (as in JSON). 
        let dats: Vec<(Dat, Kind)> = vec![
        //   to encode         json decode
            (dat!(()),          Kind::Str),     // 1 
            (dat!(true),        Kind::Str),     // 2   
            (dat!(false),       Kind::Str),     // 3 
            (dat!(None::<u8>),  Kind::Str),     // 4 
            (dat!(0u8),         Kind::U8),      // 5 
            (dat!(0u16),        Kind::U8),      // 6 
            (dat!(0u32),        Kind::U8),      // 7 
            (dat!(0u64),        Kind::U8),      // 8 
            (dat!(0u128),       Kind::U8),      // 9 
            (dat!(0i8),         Kind::U8),      // 10
            (dat!(0i16),        Kind::U8),      // 11
            (dat!(0i32),        Kind::U8),      // 12
            (dat!(0i64),        Kind::U8),      // 13
            (dat!(0i128),       Kind::U8),      // 14
            (dat!(u8::MAX),     Kind::U8),      // 15
            (dat!(u16::MAX),    Kind::U16),     // 16
            (dat!(u32::MAX),    Kind::U32),     // 17
            (dat!(u64::MAX),    Kind::U64),     // 18
            (dat!(u128::MAX),   Kind::U128),    // 19
            (dat!(i8::MIN),     Kind::I8),      // 20
            (dat!(i16::MIN),    Kind::I16),     // 21
            (dat!(i32::MIN),    Kind::I32),     // 22
            (dat!(i64::MIN),    Kind::I64),     // 23
            (dat!(i128::MIN),   Kind::I128),    // 24
            (dat!(i8::MAX),     Kind::U8),      // 25
            (dat!(i16::MAX),    Kind::U16),     // 26
            (dat!(i32::MAX),    Kind::U32),     // 27
            (dat!(i64::MAX),    Kind::U64),     // 28
            (dat!(i128::MAX),   Kind::U128),    // 29
            (dat!(0.0f32),      Kind::Adec),    // 30
            (dat!(f32::MIN),    Kind::Adec),    // 31
            (dat!(f32::MAX),    Kind::Adec),    // 32
            (dat!(0.0f64),      Kind::Adec),    // 33
            (dat!(f64::MIN),    Kind::Adec),    // 34
            (dat!(f64::MAX),    Kind::Adec),    // 35
            (dat!(res!(aint!(fmt!("{}0", u128::MAX)))),     Kind::Aint),            // 36
            (dat!(res!(aint!(fmt!("{}0", u128::MIN)))),     Kind::U8),              // 37
            (dat!(res!(adec!(fmt!("{:e}0", f64::MAX)))),    Kind::Adec),            // 38
            (dat!(res!(adec!(fmt!("{:e}0", f64::MIN)))),    Kind::Adec),            // 39
            (Dat::C64(u32::MAX as u64),                 Kind::U32),                 // 40
            (dat!("hello"),                             Kind::Str),                 // 41
            (res!(Dat::try_from((ukind.clone(), Some(best_dat!(42))))),  Kind::U8),  // 42
            (dat!(Box::new(best_dat!(-42))),            Kind::I8),                  // 43
            (dat!(Some(best_dat!(-256))),               Kind::I16),                 // 44
            //(dat!("# comment"),                         Kind::ABox),                 // 45
        ];
        let mut count: usize = 1;
        let total = dats.len()
            * kind_scopes.len()
            * type_lower_cases.len()
            * byte_encodings.len()
            * int_encodings.len()
            * trailing_commas_opts.len()
            * hide_usr_types_opts.len();
        let mut enc_cfg: EncoderConfig<_, _>;
        let mut dec_cfg: DecoderConfig<_, _>;

        for (i, (d1, k2)) in dats.iter().enumerate() {
            test!("String encode and decode {:?}", d1);
            for kind_scope in &kind_scopes {
                for type_lower_case in &type_lower_cases {
                    for byte_encoding in &byte_encodings {
                        for int_encoding in &int_encodings {
                            for trailing_commas in &trailing_commas_opts {
                                for hide_usr_types in &hide_usr_types_opts {
                                    enc_cfg = EncoderConfig::default();
                                    enc_cfg.kind_scope =        kind_scope.clone();
                                    enc_cfg.type_lower_case =   *type_lower_case;
                                    enc_cfg.byte_encoding =     byte_encoding.clone();
                                    enc_cfg.int_encoding =      int_encoding.clone();
                                    enc_cfg.trailing_commas =   *trailing_commas;
                                    enc_cfg.hide_usr_types =    *hide_usr_types;
                                    enc_cfg.ukinds_opt =        Some(ukinds.clone());
                                    dec_cfg = DecoderConfig::default();
                                    dec_cfg.trailing_comma_allowed =    *trailing_commas;
                                    dec_cfg.ukinds_opt =                Some(ukinds.clone());

                                    let d1_str = res!(d1.encode_string_with_config(&enc_cfg));
                                    let d2 = res!(Dat::decode_string_with_config(&d1_str, &dec_cfg));
                                    // If we decoded JSON, just be happy that there was no error.
                                    // But if it's partly or entirely JDAT, check that in and out
                                    // match.
                                    if *kind_scope == KindScope::Nothing ||
                                        (d1.kind().is_usr() && *hide_usr_types)
                                    {
                                        if d2.kind() != *k2 {
                                            return Err(err!(errmsg!(
                                                "Omnibus test {} of {} using dat #{}: The daticle {:?} was \
                                                encoded to '{}' then decoded to {:?} using {:?} and {:?}, \
                                                but since the type scope is {:?}, {:?} was expected.",
                                                count, total, i+1, d1, d1_str, d2,
                                                enc_cfg, dec_cfg, kind_scope, k2,
                                            ), ErrTag::Test, ErrTag::Mismatch));
                                        }
                                    } else {
                                        if *d1 != d2 {
                                            return Err(err!(errmsg!(
                                                "Omnibus test {} of {} using dat #{}: The daticle {:?} was \
                                                encoded to '{}' then decoded to {:?} (kind: {:?}) using {:?} \
                                                and {:?}.",
                                                count, total, i+1, d1, d1_str,
                                                d2, d2.kind(), enc_cfg, dec_cfg,
                                            ), ErrTag::Test, ErrTag::Mismatch));
                                        }
                                    }
                                    //test!("Omnibus test {} of {} successfully completed.", count, total);
                                    count += 1;
                                }
                            }
                        }
                    }
                }
            }
        }
        test!("{} tests run", count - 1);
        Ok(())
    }));

    res!(test_it(filter, &["Multiline string encoding 010", "all", "list", "map"], || {
        let d = listdat![
            1,
            "two",
            omapdat!{
                3 => listdat![
                    4,
                    "five",
                    6,
                ],
            },
            listdat![7,8,9],
        ];
        let expected = vec![
            "[",
            "  1,",
            "  \"two\",",
            "  {",
            "    3: [",
            "      4,",
            "      \"five\",",
            "      6",
            "    ]",
            "  },",
            "  [",
            "    7,",
            "    8,",
            "    9",
            "  ]",
            "]",
        ];
        test!("{}", d);
        let json = res!(d.json_to_lines("  "));
        let lines: Vec<&str> = json.lines().collect();
        test!("Display output:");
        for (i, line) in lines.iter().enumerate() {
            test!("{:03}: {}", i + 1, line);
        }
        test!("Expected output:");
        for (i, line) in expected.iter().enumerate() {
            test!("{:03}: {}", i + 1, line);
        }
        req!(lines.len(), expected.len());
        let mut c = 0;
        for line in lines {
            req!(line, expected[c].to_string(), "(L: actual, R: expected), line {}", c + 1);
            c += 1;
        }
        Ok(())
    }));

    res!(test_it(filter, &["Multiline string encoding 013", "all", "map"], || {
        let d = mapdat!{
            1 => 2,
            3 => mapdat!{
                4 => 5,
                6 => mapdat!{
                    7 => 8,
                    9 => 10,
                },
                11 => 12,
            },
            13 => 14,
        };
        let expected = vec![
            "{",
            "  1: 2,",
            "  3: {",
            "    4: 5,",
            "    6: {",
            "      7: 8,",
            "      9: 10",
            "    },",
            "    11: 12",
            "  },",
            "  13: 14",
            "}",
        ];
        test!("{}", d);
        let json = res!(d.json_to_lines("  "));
        let lines: Vec<&str> = json.lines().collect();
        test!("Display output:");
        for line in &lines {
            test!("{}", line);
        }
        test!("Expected output:");
        for line in &expected {
            test!("{}", line);
        }
        req!(lines.len(), expected.len());
        let mut c = 0;
        for line in lines {
            req!(line, expected[c].to_string(), "(L: actual, R: expected), line {}", c + 1);
            c += 1;
        }
        Ok(())
    }));

    res!(test_it(filter, &["Multiline string encoding 020", "all", "list"], || {
        let d = dat!(listdat![
            1,
            "two",
            omapdat!{
                3 => listdat![
                    4,
                    "five",
                    6,
                ],
            },
            listdat![7,8,9],
        ]);
        let expected = vec![
            "(list|[",
            "  (i32|1),",
            "  (str|\"two\"),",
            "  (omap|{",
            "    (i32|3): (list|[",
            "      (i32|4),",
            "      (str|\"five\"),",
            "      (i32|6),",
            "    ]),",
            "  }),",
            "  (list|[",
            "    (i32|7),",
            "    (i32|8),",
            "    (i32|9),",
            "  ]),",
            "])",
        ];
        test!("{:?}", d);
        let lines = d.to_lines("  ", true);
        test!("Debug output:");
        for line in &lines {
            test!("{}", line);
        }
        test!("Expected output:");
        for line in &expected {
            test!("{}", line);
        }
        req!(lines.len(), expected.len());
        let mut c = 0;
        for line in lines {
            req!(line, expected[c].to_string().to_lowercase());
            c += 1;
        }
        Ok(())
    }));

    res!(test_it(filter, &["String decoding 000", "all", "str"], || {
        test!("bring it");
        let d = res!(Dat::decode_string("(STR|\"\")"));
        test!("{:?}", d);
        let expected = Dat::Str("".to_string());
        req!(d, expected);
        Ok(())
    }));

    res!(test_it(filter, &["String decoding 010", "all", "str"], || {
        // Preserve quoted spaces.
        let d = res!(Dat::decode_string("\"  hello \""));
        let expected = dat!("  hello ");
        req!(d, expected);
        Ok(())
    }));

    res!(test_it(filter, &["String decoding 020", "all", "str"], || {
        // If unquoted, with no () brackets, interpret as a string.
        let d = res!(Dat::decode_string("hello"));
        let expected = dat!("hello");
        req!(d, expected);
        Ok(())
    }));

    res!(test_it(filter, &["String decoding 030", "all", "str"], || {
        // Quote protection
        let d = res!(Dat::decode_string("he\"(]\"o"));
        let expected = dat!("he(]o");
        req!(d, expected);
        Ok(())
    }));

    res!(test_it(filter, &["String decoding 040", "all", "str"], || {
        let d = res!(Dat::decode_string("(STR|hello)"));
        let expected = dat!("hello");
        req!(d, expected);
        Ok(())
    }));

    res!(test_it(filter, &["String decoding 050", "all", "str"], || {
        let d = res!(Dat::decode_string("(STR|\"hello\")"));
        let expected = dat!("hello");
        req!(d, expected);
        Ok(())
    }));

    res!(test_it(filter, &["String decoding 060", "all", "empty"], || {
        let d = res!(Dat::decode_string("\"empty\"")); // quotes should protect strings
        let expected = dat!("empty");
        req!(d, expected);
        Ok(())
    }));

    res!(test_it(filter, &["String decoding 070", "all", "empty"], || {
        let d = res!(Dat::decode_string("(EMPTY)"));
        let expected = dat!(());
        req!(d, expected);
        Ok(())
    }));

    res!(test_it(filter, &["String decoding 080", "all", "empty"], || {
        match Dat::decode_string("(Empty|)") {
            Ok(_) => return Err(err!(errmsg!(
                "Decoder should have detected superfluous '|' char.",
            ))),
            Err(e) => test!("Correctly detected error: {}", e),
        };
        Ok(())
    }));

    res!(test_it(filter, &["String decoding 090", "all", "empty"], || {
        let d = res!(Dat::decode_string("()"));
        let expected = dat!(());
        req!(d, expected);
        Ok(())
    }));

    res!(test_it(filter, &["String decoding 100", "all", "bool"], || {
        let d = res!(Dat::decode_string("(TRUE)"));
        let expected = dat!(true);
        req!(d, expected);
        Ok(())
    }));

    res!(test_it(filter, &["String decoding 110", "all", "bool"], || {
        match Dat::decode_string("(true|)") {
            Ok(_) => return Err(err!(errmsg!(
                "Decoder should have detected superfluous '|' char.",
            ))),
            Err(e) => test!("Correctly detected error: {}", e),
        };
        Ok(())
    }));

    res!(test_it(filter, &["String decoding 120", "all", "bool"], || {
        let d = res!(Dat::decode_string("(FALSE)"));
        let expected = dat!(false);
        req!(d, expected);
        Ok(())
    }));

    res!(test_it(filter, &["String decoding 130", "all", "bool"], || {
        match Dat::decode_string("(\nfalse  | )") {
            Ok(_) => return Err(err!(errmsg!(
                "Decoder should have detected superfluous '|' char.",
            ))),
            Err(e) => test!("Correctly detected error: {}", e),
        };
        Ok(())
    }));

    res!(test_it(filter, &["String decoding 140", "all", "u16"], || {
        let d = res!(Dat::decode_string("(U16|0)"));
        let expected = Dat::U16(0);
        req!(d, expected);
        Ok(())
    }));

    res!(test_it(filter, &["String decoding 150", "all", "i64"], || {
        let d = res!(Dat::decode_string("(I64|-4)"));
        let expected = Dat::I64(-4);
        req!(d, expected);
        Ok(())
    }));

    res!(test_it(filter, &["String decoding 160", "all", "i16", "minsize"], || {
        let d = res!(Dat::decode_string("-420"));
        let expected = Dat::I16(-420);
        req!(d, expected);
        Ok(())
    }));

    res!(test_it(filter, &["String decoding 170", "all", "u8"], || {
        let d = res!(Dat::decode_string("(U8|42)"));
        let expected = dat!(42u8);
        req!(d, expected);
        Ok(())
    }));

    res!(test_it(filter, &["String decoding 180", "all", "adec"], || {
        let d = res!(Dat::decode_string("42.1234"));
        let expected = dat!(res!(adec!(42.1234)));
        req!(d, expected);
        Ok(())
    }));

    res!(test_it(filter, &["String decoding 190", "all", "adec"], || {
        let d = res!(Dat::decode_string("-42.1234"));
        let expected = dat!(res!(adec!("-42.1234")));
        req!(d, expected);
        Ok(())
    }));
    
    res!(test_it(filter, &["String decoding 200", "all", "u8", "hex"], || {
        let d = res!(Dat::decode_string("0xec"));
        let expected = Dat::U8(236);
        req!(d, expected);
        Ok(())
    }));
    
    res!(test_it(filter, &["String decoding 210", "all", "u8", "hex"], || {
        let jdat_enc = EncoderConfig::<(), ()>::jdat(None);
        let jdat_dec = DecoderConfig::<(), ()>::jdat(None);
        let d1 = Dat::U8(0xf0u8);
        let d1_str = res!(d1.encode_string_with_config(&jdat_enc));
        test!("jdat = {}", d1_str);
        let d2 = res!(Dat::decode_string_with_config(d1_str.clone(), &jdat_dec));
        req!(d1, d2);
        Ok(())
    }));

    res!(test_it(filter, &["String decoding 215", "all", "b16", "base64"], || {
        let jdat_enc = EncoderConfig::<(), ()>::jdat(None);
        let jdat_dec = DecoderConfig::<(), ()>::jdat(None);
        let d1 = Dat::B16((u128::MAX/13).to_be_bytes());
        let d1_str = res!(d1.encode_string_with_config(&jdat_enc));
        test!("jdat = {}", d1_str);
        let d2 = res!(Dat::decode_string_with_config(d1_str.clone(), &jdat_dec));
        req!(d1, d2);
        Ok(())
    }));

    res!(test_it(filter, &["String decoding 220", "all", "u8", "minsize"], || {
        let d = res!(Dat::decode_string("\"42\""));
        let expected = dat!("42");
        req!(d, expected);
        Ok(())
    }));

    res!(test_it(filter, &["String decoding 230", "all", "u8", "str"], || {
        match Dat::decode_string("(U8|\"42\")") {
            Ok(d) => return Err(err!(errmsg!(
                "String decoding should have rejected the attempt to \
                coerce a string to {:?}.", d,
            ))),
            Err(e) => test!("Correctly detected error: {}", e),
        }
        Ok(())
    }));

    res!(test_it(filter, &["String decoding 240", "all", "c64"], || {
        let d = res!(Dat::decode_string("(C64|42)"));
        let expected = Dat::C64(42);
        req!(d, expected);
        Ok(())
    }));

    res!(test_it(filter, &["String decoding 300", "all", "list"], || {
        let d = res!(Dat::decode_string("[1, 2, 3, 4]"));
        let expected = listdat![1u8, 2u8, 3u8, 4u8];
        req!(d, expected);
        Ok(())
    }));
    
    res!(test_it(filter, &["String decoding 305", "all", "list"], || {
        let d = res!(Dat::decode_string("[1, 2, 3, (empty), 4]"));
        test!("{:?}", d);
        let expected = listdat![1u8, 2u8, 3u8, (), 4u8];
        req!(d, expected);
        Ok(())
    }));
    
    res!(test_it(filter, &["String decoding 310", "all", "list"], || {
        match Dat::decode_string("[1,2,3,4") {
            Ok(_) => return Err(err!(errmsg!(
                "String decoding should have detected incomplete list.",
            ))),
            Err(e) => test!("Correctly detected error: {}", e),
        };
        Ok(())
    }));
    
    res!(test_it(filter, &["String decoding 320", "all", "list"], || {
        match Dat::decode_string("1,2,3,4") {
            Ok(_) => return Err(err!(errmsg!(
                "String decoding should have detected incomplete list.",
            ))),
            Err(e) => test!("Correctly detected error: {}", e),
        };
        Ok(())
    }));
    
    res!(test_it(filter, &["String decoding 330", "all", "list"], || {
        match Dat::decode_string("1,2,3,4]") {
            Ok(_) => return Err(err!(errmsg!(
                "String decoding should have detected incomplete list.",
            ))),
            Err(e) => test!("Correctly detected error: {}", e),
        };
        Ok(())
    }));
    
    res!(test_it(filter, &["String decoding 340", "all", "list"], || {
        // nested list at begining
        let d = res!(Dat::decode_string("[[1,2],3,-408]"));
        let expected = listdat![listdat![1u8,2u8],3u8,-408i16];
        req!(d, expected);
        Ok(())
    }));
    
    res!(test_it(filter, &["String decoding 350", "all", "list"], || {
        // nested list in middle
        let d = res!(Dat::decode_string("[1,[2,3],408]"));
        let expected = listdat![1u8,listdat![2u8,3u8],408u16];
        req!(d, expected);
        Ok(())
    }));
    
    res!(test_it(filter, &["String decoding 360", "all", "list"], || {
        // nested list at end
        let d = res!(Dat::decode_string("[1,2,[3,-4]]"));
        let expected = listdat![1u8,2u8,listdat![3u8,-4i8]];
        req!(d, expected);
        Ok(())
    }));
    
    res!(test_it(filter, &["String decoding 370", "all", "list"], || {
        // trailing comma
        let d = res!(Dat::decode_string("[1,2,3,4,]"));
        let expected = listdat![dat!(1u8),dat!(2u8),dat!(3u8),dat!(4u8)];
        req!(d, expected);
        Ok(())
    }));
    
    res!(test_it(filter, &["String decoding 380", "all", "list"], || {
        // triple nesting
        let d = res!(Dat::decode_string("[1,[2,[3,4]],5]"));
        let expected = listdat![1u8,listdat![2u8,listdat![3u8,4u8]],5u8];
        req!(d, expected);
        Ok(())
    }));
    
    res!(test_it(filter, &["String decoding 390", "all", "list"], || {
        let d = res!(Dat::decode_string("(LIST|[1,2,3,4])"));
        let expected = listdat![1u8,2u8,3u8,4u8];
        req!(d, expected);
        Ok(())
    }));
    
    res!(test_it(filter, &["String decoding 400", "all", "list"], || {
        let d = res!(Dat::decode_string("[(U8|1),(I32|2),(STR|3),(I64|-4)]"));
        let expected = listdat![1u8,2i32,"3".to_string(),-4i64];
        req!(d, expected);
        Ok(())
    }));
    
    res!(test_it(filter, &["String decoding 410", "all", "list"], || {
        let d = res!(Dat::decode_string("(LIST|[(U8|1),(I32|2),(STR|3),(I64|-4)])"));
        let expected = listdat![1u8,2i32,"3".to_string(),-4i64];
        req!(d, expected);
        Ok(())
    }));
    
    res!(test_it(filter, &["String decoding 420", "all", "list"], || {
        let d = res!(Dat::decode_string("[1,[2,(LIST|[(U16|3),4])],5]"));
        let expected = listdat![1u8,listdat![2u8,listdat![3u16,4u8]],5u8];
        req!(d, expected);
        Ok(())
    }));
    
    res!(test_it(filter, &["String decoding 430", "all", "list"], || {
        let d = res!(Dat::decode_string("[1,(FALSE)]"));
        let expected = listdat![1u8,dat!(false)];
        req!(d, expected);
        Ok(())
    }));
    
    res!(test_it(filter, &["String decoding 440", "all", "list"], || {
        let d = res!(Dat::decode_string("[(FALSE),1]"));
        let expected = listdat![dat!(false),1u8];
        req!(d, expected);
        Ok(())
    }));

    res!(test_it(filter, &["String decoding 450", "all", "list"], || {
        let d = res!(Dat::decode_string("[(),1]"));
        let expected = listdat![dat!(()),1u8];
        req!(d, expected);
        Ok(())
    }));

    res!(test_it(filter, &["String decoding 460", "all", "list"], || {
        let d = res!(Dat::decode_string("[(U8|42),1]"));
        let expected = listdat![42u8,1u8];
        req!(d, expected);
        Ok(())
    }));

    res!(test_it(filter, &["String decoding 470", "all", "tuple"], || {
        match Dat::decode_string("(T2|[1, ])") {
            Ok(_) => return Err(err!(errmsg!(
                "String decoding should have detected incorrect list length.",
            ))),
            Err(e) => test!("Correctly detected error: {}", e),
        }
        let d = res!(Dat::decode_string("(T2|[1, 2])"));
        let expected = tup2dat![1u8,2u8,];
        req!(d, expected);
        Ok(())
    }));

    res!(test_it(filter, &["String decoding 480", "all", "tuple"], || {
        match Dat::decode_string("(T3|[1, 2, ])") {
            Ok(_) => return Err(err!(errmsg!(
                "String decoding should have detected incorrect list length.",
            ))),
            Err(e) => test!("Correctly detected error: {}", e),
        }
        let d = res!(Dat::decode_string("(T3|[1, 2, 3])"));
        let expected = tup3dat![1u8,2u8,3u8];
        req!(d, expected);
        Ok(())
    }));

    res!(test_it(filter, &["String decoding 490", "all", "tuple"], || {
        match Dat::decode_string("(T4|[1, 2, ])") {
            Ok(_) => return Err(err!(errmsg!(
                "String decoding should have detected incorrect list length.",
            ))),
            Err(e) => test!("Correctly detected error: {}", e),
        }
        let d = res!(Dat::decode_string("(T4|[1, 2, 3, 4])"));
        let expected = tup4dat![1u8,2u8,3u8,4u8];
        req!(d, expected);
        Ok(())
    }));

    res!(test_it(filter, &["String decoding 500", "all", "tuple"], || {
        match Dat::decode_string("(T5|[1, 2, ])") {
            Ok(_) => return Err(err!(errmsg!(
                "String decoding should have detected incorrect list length.",
            ))),
            Err(e) => test!("Correctly detected error: {}", e),
        }
        let d = res!(Dat::decode_string("(T5|[1, 2, 3, 4, 5])"));
        let expected = tup5dat![1u8,2u8,3u8,4u8,5u8];
        req!(d, expected);
        Ok(())
    }));

    res!(test_it(filter, &["String decoding 510", "all", "tuple"], || {
        match Dat::decode_string("(T6|[1, 2, ])") {
            Ok(_) => return Err(err!(errmsg!(
                "String decoding should have detected incorrect list length.",
            ))),
            Err(e) => test!("Correctly detected error: {}", e),
        }
        let d = res!(Dat::decode_string("(T6|[1, 2, 3, 4, 5, 6])"));
        let expected = tup6dat![1u8,2u8,3u8,4u8,5u8,6u8];
        req!(d, expected);
        Ok(())
    }));

    res!(test_it(filter, &["String decoding 520", "all", "tuple"], || {
        match Dat::decode_string("(T7|[1, 2, ])") {
            Ok(_) => return Err(err!(errmsg!(
                "String decoding should have detected incorrect list length.",
            ))),
            Err(e) => test!("Correctly detected error: {}", e),
        }
        let d = res!(Dat::decode_string("(T7|[1, 2, 3, 4, 5, 6, 7])"));
        let expected = tup7dat![1u8,2u8,3u8,4u8,5u8,6u8,7u8];
        req!(d, expected);
        Ok(())
    }));

    res!(test_it(filter, &["String decoding 530", "all", "tuple"], || {
        match Dat::decode_string("(T8|[1, 2, ])") {
            Ok(_) => return Err(err!(errmsg!(
                "String decoding should have detected incorrect list length.",
            ))),
            Err(e) => test!("Correctly detected error: {}", e),
        }
        let d = res!(Dat::decode_string("(T8|[1, 2, 3, 4, 5, 6, 7, 8])"));
        let expected = tup8dat![1u8,2u8,3u8,4u8,5u8,6u8,7u8,8u8];
        req!(d, expected);
        Ok(())
    }));

    res!(test_it(filter, &["String decoding 540", "all", "tuple"], || {
        match Dat::decode_string("(T9|[1, 2, ])") {
            Ok(_) => return Err(err!(errmsg!(
                "String decoding should have detected incorrect list length.",
            ))),
            Err(e) => test!("Correctly detected error: {}", e),
        }
        let d = res!(Dat::decode_string("(T9|[1, 2, 3, 4, 5, 6, 7, 8, 9])"));
        let expected = tup9dat![1u8,2u8,3u8,4u8,5u8,6u8,7u8,8u8,9u8];
        req!(d, expected);
        Ok(())
    }));

    res!(test_it(filter, &["String decoding 550", "all", "tuple"], || {
        match Dat::decode_string("(T10|[1, 2, ])") {
            Ok(_) => return Err(err!(errmsg!(
                "String decoding should have detected incorrect list length.",
            ))),
            Err(e) => test!("Correctly detected error: {}", e),
        }
        let d = res!(Dat::decode_string("(T10|[1, 2, 3, 4, 5, 6, 7, 8, 9, 10])"));
        let expected = tup10dat![1u8,2u8,3u8,4u8,5u8,6u8,7u8,8u8,9u8,10u8];
        req!(d, expected, "(L: actual, R: expected)");
        Ok(())
    }));

    res!(test_it(filter, &["String decoding 560", "all", "list", "tuple"], || {
        // combining default LIST with a fixed length list, nested in middle
        let d = res!(Dat::decode_string("[1,(T2|[2,3]),4]"));
        let expected = listdat![1u8,tup2dat![2u8,3u8],4u8];
        req!(d, expected, "(L: actual, R: expected)");
        Ok(())
    }));
    
    res!(test_it(filter, &["String decoding 562", "all", "list", "tuple"], || {
        // Infer tuple without kindicle.
        let d = res!(Dat::decode_string("(1,2,3)"));
        let expected = tup3dat![ 1u8, 2u8, 3u8 ];
        req!(d, expected, "(L: actual, R: expected)");
        Ok(())
    }));
    
    res!(test_it(filter, &["String decoding 564", "all", "list", "tuple"], || {
        // Infer tuple without kindicle.
        let d = res!(Dat::decode_string("((u8|1),2,(i16|3))"));
        let expected = tup3dat![ 1u8, 2u8, 3i16 ];
        req!(d, expected, "(L: actual, R: expected)");
        Ok(())
    }));
    
    res!(test_it(filter, &["String decoding 570", "all", "map"], || {
        let d = res!(Dat::decode_string("{1:2,3:4,5:6}"));
        let expected = mapdat!{
            1u8 => 2u8,
            3u8 => 4u8,
            5u8 => 6u8,
        };
        req!(d, expected);
        Ok(())
    }));
    
    res!(test_it(filter, &["String decoding 580", "all", "map"], || {
        match Dat::decode_string("{1:2,3:4") {
            Ok(_) => return Err(err!(errmsg!(
                "String decoding should have detected incomplete map.",
            ))),
            Err(e) => test!("Correctly detected error: {}", e),
        };
        Ok(())
    }));
    
    res!(test_it(filter, &["String decoding 590", "all", "map"], || {
        match Dat::decode_string("1:2,3:4") {
            Ok(_) => return Err(err!(errmsg!(
                "String decoding should have detected incomplete map.",
            ))),
            Err(e) => test!("Correctly detected error: {}", e),
        };
        Ok(())
    }));

    res!(test_it(filter, &["String decoding 600", "all", "map"], || {
        match Dat::decode_string("1:2,3:4}") {
            Ok(_) => return Err(err!(errmsg!(
                "String decoding should have detected incomplete map.",
            ))),
            Err(e) => test!("Correctly detected error: {}", e),
        };
        Ok(())
    }));

    res!(test_it(filter, &["String decoding 610", "all", "map"], || {
        let d = res!(Dat::decode_string("(MAP|{1:2,3:4,5:6})"));
        let expected = dat!(mapdat!{
            1u8 => 2u8,
            3u8 => 4u8,
            5u8 => 6u8,
        });
        req!(d, expected);
        Ok(())
    }));
    
    res!(test_it(filter, &["String decoding 620", "all", "map"], || {
        // nested map at beginning key
        let d = res!(Dat::decode_string("{{1:-2,3:-4}:5,6:7}"));
        let expected = mapdat!{
            mapdat!{
                1u8 => -2i8,
                3u8 => -4i8,
            } => 5u8,
            6u8 => 7u8,
        };
        req!(d, expected);
        Ok(())
    }));
    
    res!(test_it(filter, &["String decoding 630", "all", "map"], || {
        // nested map at beginning pair
        let d = res!(Dat::decode_string("{{1:2,3:4}:{5:6,7:8},9:10}"));
        let expected = mapdat!{
            mapdat!{
                1u8 => 2u8,
                3u8 => 4u8,
            } => mapdat!{
                5u8 => 6u8,
                7u8 => 8u8,
            },
            9u8 => 10u8,
        };
        req!(d, expected);
        Ok(())
    }));
    
    res!(test_it(filter, &["String decoding 640", "all", "map"], || {
        // triple nested map
        let d = res!(Dat::decode_string("
        {
            {
                1:{
                    2:3,
                    4:5,
                },
                5:6,
            }:{
                7:8,
                9:10,
            },
            11:12,
        }"));
        let expected = mapdat!{
            mapdat!{
                1u8 => mapdat!{
                    2u8 => 3u8,
                    4u8 => 5u8,
                },
                5u8 => 6u8,
            } => mapdat!{
                7u8 => 8u8,
                9u8 => 10u8,
            },
            11u8 => 12u8,
        };
        req!(d, expected);
        Ok(())
    }));
    
    res!(test_it(filter, &["String decoding 644", "all", "mixed"], || {
        let d = res!(Dat::decode_string("[1,{2:3,4:5},6]"));
        let expected = listdat![
            1u8,
            mapdat!{
                2u8 => 3u8,
                4u8 => 5u8,
            },
            6u8,
        ];
        req!(d, expected);
        Ok(())
    }));
    
    res!(test_it(filter, &["String decoding 645", "all", "mixed"], || {
        let d = res!(Dat::decode_string("[{1:2,3:4}, (u64|1)]"));
        let expected = listdat![
            mapdat!{
                1u8 => 2u8,
                3u8 => 4u8,
            },
            1u64,
        ];
        req!(d, expected);
        Ok(())
    }));
    
    res!(test_it(filter, &["String decoding 650", "all", "map"], || {
        let d = res!(Dat::decode_string("{1:2,\"3\":\"4\",}"));
        let expected = mapdat!{
            1u8 => 2u8,
            "3" => "4",
        };
        req!(d, expected);
        Ok(())
    }));
    
    res!(test_it(filter, &["String decoding 660", "all", "map"], || {
        let d = res!(Dat::decode_string("{1:[\"a\", \"b\"],}"));
        let expected = mapdat!{
            1u8 => listdat!["a", "b"],
        };
        req!(d, expected);
        Ok(())
    }));
    
    res!(test_it(filter, &["String decoding 670", "all", "map"], || {
        let d = res!(Dat::decode_string("{1:[2000, 3000]}"));
        let expected = mapdat!{
            1u8 => listdat![2000u16, 3000u16],
        };
        req!(d, expected);
        Ok(())
    }));
    
    res!(test_it(filter, &["String decoding 680", "all", "map"], || {
        let d = res!(Dat::decode_string("{1:{2:[3, 4]}}"));
        let expected = mapdat!{
            1u8 => mapdat!{
                2u8 => listdat![3u8, 4u8],
            },
        };
        req!(d, expected);
        Ok(())
    }));

    res!(test_it(filter, &["String decoding 682", "all", "map"], || {
        let d = res!(Dat::decode_string("(omap|{1:{2:[3, 4]}})"));
        let expected = omapdat!{
            1u8 => mapdat!{
                2u8 => listdat![3u8, 4u8],
            },
        };
        req!(d, expected);
        Ok(())
    }));

    res!(test_it(filter, &["String decoding 684", "all", "map", "usr"], || {
        // Provides a baseline for debugging "String decoding 685" test.
        let mut uks = UsrKinds::new(BTreeMap::new(), BTreeMap::new());
        let ukind = UsrKindId::new(1, Some("nams"), None);
        res!(uks.add(ukind.clone()));
        let jdat_dec = DecoderConfig::<_, _>::jdat(Some(uks));
        let input = "{1:{5:2,3:4}}";
        let expected = mapdat!{
            1u8 => best_mapdat!{
                5 => 2,
                3 => 4,
            },
        };
        let dat = res!(Dat::decode_string_with_config(input, &jdat_dec));
        test!("input:");
        for line in Stringer::new(input.to_string()).to_lines("    ") {
            test!("{}", line);
        }
        test!("result:");
        for line in Stringer::new(fmt!("{:?}", dat)).to_lines("    ") {
            test!("{}", line);
        }
        test!("expected:");
        for line in Stringer::new(fmt!("{:?}", expected)).to_lines("    ") {
            test!("{}", line);
        }
        req!(dat, expected);
        Ok(())
    }));

    res!(test_it(filter, &["String decoding 685", "all", "map", "usr"], || {
        let mut uks = UsrKinds::new(BTreeMap::new(), BTreeMap::new());
        let ukind = UsrKindId::new(1, Some("nams"), None);
        res!(uks.add(ukind.clone()));
        let jdat_dec = DecoderConfig::<_, _>::jdat(Some(uks));
        let input = "{1:{(empty):2,3:4}}";
        let expected = mapdat!{
            1u8 => best_mapdat!{
                () => 2,
                3 => 4,
            },
        };
        let dat = res!(Dat::decode_string_with_config(input, &jdat_dec));
        test!("input:");
        for line in Stringer::new(input.to_string()).to_lines("    ") {
            test!("{}", line);
        }
        test!("result:");
        for line in Stringer::new(fmt!("{:?}", dat)).to_lines("    ") {
            test!("{}", line);
        }
        test!("expected:");
        for line in Stringer::new(fmt!("{:?}", expected)).to_lines("    ") {
            test!("{}", line);
        }
        req!(dat, expected);
        Ok(())
    }));

    res!(test_it(filter, &["String decoding 686", "all", "map", "usr"], || {
        let mut uks = UsrKinds::new(BTreeMap::new(), BTreeMap::new());
        let ukind = UsrKindId::new(1, Some("nams"), None);
        res!(uks.add(ukind.clone()));
        let jdat_dec = DecoderConfig::<_, _>::jdat(Some(uks));
        let input = "{1:{(nams):2,3:4}}";
        let expected = mapdat!{
            1u8 => mapdat!{
                res!(Dat::try_from((ukind, None))) => 2u8,
                3u8 => 4u8,
            },
        };
        let dat = res!(Dat::decode_string_with_config(input, &jdat_dec));
        test!("input:");
        for line in Stringer::new(input.to_string()).to_lines("    ") {
            test!("{}", line);
        }
        test!("result:");
        for line in Stringer::new(fmt!("{:?}", dat)).to_lines("    ") {
            test!("{}", line);
        }
        test!("expected:");
        for line in Stringer::new(fmt!("{:?}", expected)).to_lines("    ") {
            test!("{}", line);
        }
        req!(dat, expected);
        Ok(())
    }));

    res!(test_it(filter, &["String integrated decoding 000", "all", "map"], || {
        let d = res!(Dat::decode_string("
        {
            \"1\":{
                2:(I16|-3),
                4:[5,6],
            },
        }"));
        let expected = mapdat!{
            "1".to_string() => mapdat!{
                2u8 => -3i16,
                4u8 => listdat![
                    5u8,
                    6u8,
                ],
            },
        };
        for line in expected.to_lines("  ", true) {
            test!("{}", line);
        }
        req!(d, expected);
        Ok(())
    }));

    res!(test_it(filter, &["String integrated decoding 010", "all", "map", "abox"], || {
        let expected = dat!(mapdat!{
            mapdat!{
                "1".to_string() => mapdat!{
                    2u8 => -3i16,
                    4u8 => listdat![
                        5u8,
                        6u8,
                    ],
                },
                "10".to_string() => 11u8,
            } => mapdat!{
                12u32 => false,
                13i8 => true,
            },
            dat!(14u8) => abox!((), "This is the number 14"),
            300u16 => dat!(7u8),
        });
        for line in expected.to_lines("  ", true) {
            test!("{}", line);
        }
        let d = res!(Dat::decode_string("
        {
            {
                \"1\":{
                    2:(I16|-3),
                    4:[5,6],
                },
                \"10\":(U8|11),
            }:{
                (U32|12):(FALSE),
                (I8|13):true,
            },
            14: !This is the number 14!,
            300: (U8|7),
        }"));
        req!(d, expected);
        Ok(())
    }));

    res!(test_it(filter, &["String config file 000", "all", "map", "file"], || {
        let path = Path::new("./ref/o3db_cfg.dat");
        match std::fs::read_to_string(path) {
            Ok(s) => {
                let dat = res!(Dat::decode_string(s));
                let expected = omapdat!{
                    "num_zones"             => 2u16,
                    "num_caches"            => 2u16,
                    "num_readers"           => 2u16,
                    "cache_size_limit_mb"   => 1000u64,
                    "data_file_max_bytes"   => 1000u64,
                    "index_file_max_bytes"  => 1000u64,
                    "zones" => omapdat!{
                        listdat![0u16] => listdat![
                            "./", 1u8, 100u16,
                        ],
                        listdat![1u16] => listdat![
                            "./", 1u8, 100u16,
                        ],
                    },
                };
                req!(dat, expected);
            },
            Err(e) => return Err(err!(errmsg!(
                "Error while reading {:?}: {}", path, e,
            ), ErrTag::File, ErrTag::Read)),
        }
        Ok(())
    }));

    res!(test_it(filter, &["String encode decode 000", "all", "usr"], || {
        let ukind = UsrKindId::new(5, Some("my_type"), Some(Kind::I32));
        let d0 = dat!(42);
        let d1 = Dat::Usr(ukind, Some(Box::new(d0.clone())));
        req!(fmt!("{:?}", d1), "(my_type|(i32|42))", "(L: debug output, R: expected)");
        req!(fmt!("{}", d1), "(my_type|(i32|42))", "(L: display output, R: expected)");
        Ok(())
    }));

    res!(test_it(filter, &["String encode decode 010", "all", "box"], || {
        let d0 = dat!(42);
        let d1 = Dat::Opt(Box::new(Some(d0)));
        let d1_str = fmt!("{:?}", d1);
        test!("{}", d1_str);
        let d2 = res!(Dat::decode_string(d1_str));
        req!(d1, d2, "(L: expected, R: decoded)");
        Ok(())
    }));

    res!(test_it(filter, &["String encode decode 020", "all", "opt"], || {
        let d1 = Dat::Opt(Box::new(None));
        let d1_str = fmt!("{:?}", d1);
        test!("{}", d1_str);
        let d2 = res!(Dat::decode_string(d1_str));
        req!(d1, d2, "(L: expected, R: decoded)");
        Ok(())
    }));

    res!(test_it(filter, &["String encode decode 030", "all", "bc64"], || {
        let byts = vec![1u8,2,3,4,5];
        let d1 = Dat::BC64(byts);
        let d1_str = fmt!("{:?}", d1);
        test!("{}", d1_str);
        let d2 = res!(Dat::decode_string(d1_str));
        req!(d1, d2, "(L: expected, R: decoded)");
        Ok(())
    }));

    res!(test_it(filter, &["String encode decode 040", "all", "i16"], || {
        let jdat_enc = EncoderConfig::<(), ()>::jdat(None);
        let jdat_dec = DecoderConfig::<(), ()>::jdat(None);
        let d1 = Dat::I16(-3957i16);
        let d1_str = res!(d1.encode_string_with_config(&jdat_enc));
        test!("{}", d1_str);
        test!("jdat = {}", d1_str);
        let d2 = res!(Dat::decode_string_with_config(d1_str.clone(), &jdat_dec));
        req!(d1, d2, "(L: expected, R: decoded)");
        Ok(())
    }));

    res!(test_it(filter, &["String encode decode 050", "all", "i16"], || {
        let jdat_enc = EncoderConfig::<(), ()>::jdat(None);
        let jdat_dec = DecoderConfig::<(), ()>::jdat(None);
        let d1 = Dat::I16(i16::MIN);
        let d1_str = res!(d1.encode_string_with_config(&jdat_enc));
        test!("{}", d1_str);
        test!("jdat = {}", d1_str);
        let d2 = res!(Dat::decode_string_with_config(d1_str.clone(), &jdat_dec));
        req!(d1, d2, "(L: expected, R: decoded)");
        Ok(())
    }));

    res!(test_it(filter, &["String encode decode 060", "all", "i16"], || {
        let mut jdat_enc = EncoderConfig::<(), ()>::jdat(None);
        let jdat_dec = DecoderConfig::<(), ()>::jdat(None);
        jdat_enc.int_encoding = IntEncoding::Octal;
        let d1 = Dat::I16(-3957i16);
        let d1_str = res!(d1.encode_string_with_config(&jdat_enc));
        test!("{}", d1_str);
        test!("jdat = {}", d1_str);
        let d2 = res!(Dat::decode_string_with_config(d1_str.clone(), &jdat_dec));
        req!(d1, d2, "(L: expected, R: decoded)");
        Ok(())
    }));

    res!(test_it(filter, &["String encode decode 070", "all", "i16"], || {
        let mut jdat_enc = EncoderConfig::<(), ()>::jdat(None);
        let jdat_dec = DecoderConfig::<(), ()>::jdat(None);
        jdat_enc.int_encoding = IntEncoding::Octal;
        let d1 = Dat::I16(i16::MIN);
        let d1_str = res!(d1.encode_string_with_config(&jdat_enc));
        test!("{}", d1_str);
        test!("jdat = {}", d1_str);
        let d2 = res!(Dat::decode_string_with_config(d1_str.clone(), &jdat_dec));
        req!(d1, d2, "(L: expected, R: decoded)");
        Ok(())
    }));

    res!(test_it(filter, &["String encode decode 080", "all", "i16"], || {
        let mut jdat_enc = EncoderConfig::<(), ()>::jdat(None);
        let jdat_dec = DecoderConfig::<(), ()>::jdat(None);
        jdat_enc.int_encoding = IntEncoding::Binary;
        let d1 = Dat::I16(-3957i16);
        let d1_str = res!(d1.encode_string_with_config(&jdat_enc));
        test!("{}", d1_str);
        test!("jdat = {}", d1_str);
        let d2 = res!(Dat::decode_string_with_config(d1_str.clone(), &jdat_dec));
        req!(d1, d2, "(L: expected, R: decoded)");
        Ok(())
    }));

    res!(test_it(filter, &["String encode decode 090", "all", "i16"], || {
        let mut jdat_enc = EncoderConfig::<(), ()>::jdat(None);
        let jdat_dec = DecoderConfig::<(), ()>::jdat(None);
        jdat_enc.int_encoding = IntEncoding::Binary;
        let d1 = Dat::I16(i16::MIN);
        let d1_str = res!(d1.encode_string_with_config(&jdat_enc));
        test!("{}", d1_str);
        test!("jdat = {}", d1_str);
        let d2 = res!(Dat::decode_string_with_config(d1_str.clone(), &jdat_dec));
        req!(d1, d2, "(L: expected, R: decoded)");
        Ok(())
    }));

    res!(test_it(filter, &["String encode decode 100", "all", "b32"], || {
        let jdat_enc = EncoderConfig::<(), ()>::jdat(None);
        let jdat_dec = DecoderConfig::<(), ()>::jdat(None);
        let json_enc = EncoderConfig::<(), ()>::json(None);
        let byts = [
            0x9a, 0xc1, 0x78, 0x08, 0xc1, 0xb3, 0x5e, 0x4b,
            0xfe, 0xb9, 0x91, 0xa4, 0x3b, 0x04, 0x15, 0xb3,
            0x00, 0xb1, 0x66, 0xf5, 0x81, 0x08, 0xcc, 0x3d,
            0xa2, 0xab, 0x61, 0x8d, 0xd9, 0xcc, 0xf0, 0x38,
        ];
        let d1 = Dat::B32(B32(byts));
        let d1_str = res!(d1.encode_string_with_config(&jdat_enc));
        test!("jdat = {}", d1_str);
        test!("json = {}", res!(d1.encode_string_with_config(&json_enc)));
        let d2 = res!(Dat::decode_string_with_config(d1_str.clone(), &jdat_dec));
        req!(d1, d2, "(L: expected, R: decoded)");
        Ok(())
    }));

    res!(test_it(filter, &["String encode decode 110", "all", "bu8"], || {
        let jdat_enc = EncoderConfig::<(), ()>::jdat(None);
        let jdat_dec = DecoderConfig::<(), ()>::jdat(None);
        let json_enc = EncoderConfig::<(), ()>::json(None);
        let byts = [
            0x9a, 0xc1, 0x78, 0x08, 0xc1, 0xb3, 0x5e, 0x4b,
            0xfe, 0xb9, 0x91, 0xa4, 0x3b, 0x04, 0x15, 0xb3,
            0x00, 0xb1, 0x66, 0xf5, 0x81, 0x08, 0xcc, 0x3d,
            0xa2, 0xab, 0x61, 0x8d, 0xd9, 0xcc, 0xf0, 0x38,
        ];
        let d1 = Dat::BU8(byts.to_vec());
        let d1_str = res!(d1.encode_string_with_config(&jdat_enc));
        test!("jdat = {}", d1_str);
        test!("json = {}", res!(d1.encode_string_with_config(&json_enc)));
        let d2 = res!(Dat::decode_string_with_config(d1_str.clone(), &jdat_dec));
        req!(d1, d2, "(L: expected, R: decoded)");
        Ok(())
    }));

    res!(test_it(filter, &["String encode decode 120", "all", "bu64"], || {
        let json_enc = EncoderConfig::<(), ()>::json(None);
        let json_dec = DecoderConfig::<(), ()>::json(None);
        let byts = vec![1u8, 2, 3, 4, 5];
        let d1 = Dat::BU64(byts);
        let d3 = listdat![1u8, 2u8, 3u8, 4u8, 5u8];
        let d1_str = res!(d1.encode_string_with_config(&json_enc));
        test!("json = {}", d1_str);
        let d2 = res!(Dat::decode_string_with_config(d1_str.clone(), &json_dec));
        req!(d1_str, "[1, 2, 3, 4, 5]".to_string());
        req!(d2, d3);
        Ok(())
    }));

    res!(test_it(filter, &["String encode decode 130", "all", "bu64"], || {
        let jdat_enc = EncoderConfig::<(), ()>::jdat(None);
        let jdat_dec = DecoderConfig::<(), ()>::jdat(None);
        let byts = vec![1u8,2,3,4,5];
        let d1 = Dat::BU64(byts);
        let d1_str = res!(d1.encode_string_with_config(&jdat_enc));
        test!("jdat = {}", d1_str);
        let d2 = res!(Dat::decode_string_with_config(d1_str, &jdat_dec));
        req!(d1, d2);
        Ok(())
    }));

    res!(test_it(filter, &["String encode decode 140", "all", "tuple"], || {
        test_string_encode_decode_homogenous_tuple! { jdat, [42u8; 2] }
        Ok(())
    }));
    res!(test_it(filter, &["String encode decode 150", "all", "tuple"], || {
        test_string_encode_decode_homogenous_tuple! { jdat, [42u8; 3] }
        Ok(())
    }));
    res!(test_it(filter, &["String encode decode 160", "all", "tuple"], || {
        test_string_encode_decode_homogenous_tuple! { jdat, [42u8; 4] }
        Ok(())
    }));
    res!(test_it(filter, &["String encode decode 170", "all", "tuple"], || {
        test_string_encode_decode_homogenous_tuple! { jdat, [42u8; 5] }
        Ok(())
    }));
    res!(test_it(filter, &["String encode decode 180", "all", "tuple"], || {
        test_string_encode_decode_homogenous_tuple! { jdat, [42u8; 6] }
        Ok(())
    }));
    res!(test_it(filter, &["String encode decode 190", "all", "tuple"], || {
        test_string_encode_decode_homogenous_tuple! { jdat, [42u8; 7] }
        Ok(())
    }));
    res!(test_it(filter, &["String encode decode 200", "all", "tuple"], || {
        test_string_encode_decode_homogenous_tuple! { jdat, [42u8; 8] }
        Ok(())
    }));
    res!(test_it(filter, &["String encode decode 210", "all", "tuple"], || {
        test_string_encode_decode_homogenous_tuple! { jdat, [42u8; 9] }
        Ok(())
    }));
    res!(test_it(filter, &["String encode decode 220", "all", "tuple"], || {
        test_string_encode_decode_homogenous_tuple! { jdat, [42u8; 10] }
        Ok(())
    }));
    res!(test_it(filter, &["String encode decode 230", "all", "tuple"], || {
        test_string_encode_decode_homogenous_tuple! { jdat, [42u8; 16] }
        Ok(())
    }));
    res!(test_it(filter, &["String encode decode 240", "all", "tuple"], || {
        test_string_encode_decode_homogenous_tuple! { jdat, [42u8; 32] }
        Ok(())
    }));

    res!(test_it(filter, &["String encode decode 250", "all", "tuple"], || {
        test_string_encode_decode_homogenous_tuple! { jdat, [42u16; 2] }
        Ok(())
    }));
    res!(test_it(filter, &["String encode decode 260", "all", "tuple"], || {
        test_string_encode_decode_homogenous_tuple! { jdat, [42u16; 3] }
        Ok(())
    }));
    res!(test_it(filter, &["String encode decode 270", "all", "tuple"], || {
        test_string_encode_decode_homogenous_tuple! { jdat, [42u16; 4] }
        Ok(())
    }));
    res!(test_it(filter, &["String encode decode 280", "all", "tuple"], || {
        test_string_encode_decode_homogenous_tuple! { jdat, [42u16; 5] }
        Ok(())
    }));
    res!(test_it(filter, &["String encode decode 290", "all", "tuple"], || {
        test_string_encode_decode_homogenous_tuple! { jdat, [42u16; 6] }
        Ok(())
    }));
    res!(test_it(filter, &["String encode decode 300", "all", "tuple"], || {
        test_string_encode_decode_homogenous_tuple! { jdat, [42u16; 7] }
        Ok(())
    }));
    res!(test_it(filter, &["String encode decode 310", "all", "tuple"], || {
        test_string_encode_decode_homogenous_tuple! { jdat, [42u16; 8] }
        Ok(())
    }));
    res!(test_it(filter, &["String encode decode 320", "all", "tuple"], || {
        test_string_encode_decode_homogenous_tuple! { jdat, [42u16; 9] }
        Ok(())
    }));
    res!(test_it(filter, &["String encode decode 330", "all", "tuple"], || {
        test_string_encode_decode_homogenous_tuple! { jdat, [42u16; 10] }
        Ok(())
    }));

    res!(test_it(filter, &["String encode decode 340", "all", "tuple"], || {
        test_string_encode_decode_homogenous_tuple! { jdat, [42u32; 2] }
        Ok(())
    }));
    res!(test_it(filter, &["String encode decode 350", "all", "tuple"], || {
        test_string_encode_decode_homogenous_tuple! { jdat, [42u32; 3] }
        Ok(())
    }));
    res!(test_it(filter, &["String encode decode 360", "all", "tuple"], || {
        test_string_encode_decode_homogenous_tuple! { jdat, [42u32; 4] }
        Ok(())
    }));
    res!(test_it(filter, &["String encode decode 370", "all", "tuple"], || {
        test_string_encode_decode_homogenous_tuple! { jdat, [42u32; 5] }
        Ok(())
    }));
    res!(test_it(filter, &["String encode decode 380", "all", "tuple"], || {
        test_string_encode_decode_homogenous_tuple! { jdat, [42u32; 6] }
        Ok(())
    }));
    res!(test_it(filter, &["String encode decode 390", "all", "tuple"], || {
        test_string_encode_decode_homogenous_tuple! { jdat, [42u32; 7] }
        Ok(())
    }));
    res!(test_it(filter, &["String encode decode 400", "all", "tuple"], || {
        test_string_encode_decode_homogenous_tuple! { jdat, [42u32; 8] }
        Ok(())
    }));
    res!(test_it(filter, &["String encode decode 410", "all", "tuple"], || {
        test_string_encode_decode_homogenous_tuple! { jdat, [42u32; 9] }
        Ok(())
    }));
    res!(test_it(filter, &["String encode decode 420", "all", "tuple"], || {
        test_string_encode_decode_homogenous_tuple! { jdat, [42u32; 10] }
        Ok(())
    }));

    res!(test_it(filter, &["String encode decode 430", "all", "tuple"], || {
        test_string_encode_decode_homogenous_tuple! { jdat, [42u64; 2] }
        Ok(())
    }));
    res!(test_it(filter, &["String encode decode 440", "all", "tuple"], || {
        test_string_encode_decode_homogenous_tuple! { jdat, [42u64; 3] }
        Ok(())
    }));
    res!(test_it(filter, &["String encode decode 450", "all", "tuple"], || {
        test_string_encode_decode_homogenous_tuple! { jdat, [42u64; 4] }
        Ok(())
    }));
    res!(test_it(filter, &["String encode decode 460", "all", "tuple"], || {
        test_string_encode_decode_homogenous_tuple! { jdat, [42u64; 5] }
        Ok(())
    }));
    res!(test_it(filter, &["String encode decode 470", "all", "tuple"], || {
        test_string_encode_decode_homogenous_tuple! { jdat, [42u64; 6] }
        Ok(())
    }));
    res!(test_it(filter, &["String encode decode 480", "all", "tuple"], || {
        test_string_encode_decode_homogenous_tuple! { jdat, [42u64; 7] }
        Ok(())
    }));
    res!(test_it(filter, &["String encode decode 490", "all", "tuple"], || {
        test_string_encode_decode_homogenous_tuple! { jdat, [42u64; 8] }
        Ok(())
    }));
    res!(test_it(filter, &["String encode decode 500", "all", "tuple"], || {
        test_string_encode_decode_homogenous_tuple! { jdat, [42u64; 9] }
        Ok(())
    }));
    res!(test_it(filter, &["String encode decode 510", "all", "tuple"], || {
        test_string_encode_decode_homogenous_tuple! { jdat, [42u64; 10] }
        Ok(())
    }));

    res!(test_it(filter, &["String encode decode 600", "all", "usr"], || {
        let mut ukinds = UsrKinds::new(BTreeMap::new(), BTreeMap::new());
        let k1 = UsrKindId::new(5, Some("test1"), Some(Kind::Str));
        let k2 = UsrKindId::new(3, Some("test2"), None);
        let k3 = UsrKindId::new(7, Some("test3"), Some(Kind::U8));
        res!(ukinds.add(k1.clone()));
        res!(ukinds.add(k2.clone()));
        res!(ukinds.add(k3.clone()));
        let jdat_enc = EncoderConfig::jdat(Some(ukinds.clone()));
        let jdat_dec = DecoderConfig::jdat(Some(ukinds));

        let d1_enc = res!(Dat::try_from((k1, Some(dat!("hello")))));
        let d1_enc_str = res!(d1_enc.encode_string_with_config(&jdat_enc));
        test!("jdat enc = {}", d1_enc_str);
        let d1_dec = res!(Dat::decode_string_with_config(d1_enc_str, &jdat_dec));
        // Debug has no knowledge of usr kinds.
        let d1_dec_str = res!(d1_dec.encode_string_with_config(&jdat_enc));
        test!("jdat dec = {}", d1_dec_str);
        req!(d1_enc, d1_dec);

        let d2_enc = res!(Dat::try_from((k2, None)));
        let d2_enc_str = res!(d2_enc.encode_string_with_config(&jdat_enc));
        test!("jdat enc = {}", d2_enc_str);
        let d2_dec = res!(Dat::decode_string_with_config(d2_enc_str, &jdat_dec));
        // Debug has no knowledge of usr kinds.
        let d2_dec_str = res!(d2_dec.encode_string_with_config(&jdat_enc));
        test!("jdat dec = {}", d2_dec_str);
        req!(d2_enc, d2_dec);

        let d3_enc = res!(Dat::try_from((k3, Some(dat!(42u8)))));
        let d3_enc_str = res!(d3_enc.encode_string_with_config(&jdat_enc));
        test!("jdat enc = {}", d3_enc_str);
        let d3_dec = res!(Dat::decode_string_with_config(d3_enc_str, &jdat_dec));
        // Debug has no knowledge of usr kinds.
        let d3_dec_str = res!(d3_dec.encode_string_with_config(&jdat_enc));
        test!("jdat dec = {}", d3_dec_str);
        req!(d3_enc, d3_dec);
        Ok(())
    }));

    res!(test_it(filter, &["String encode decode 610", "all", "usr"], || {
        let mut ukinds = UsrKinds::new(BTreeMap::new(), BTreeMap::new());
        let k1 = UsrKindId::new(5, Some("test1"), Some(Kind::Str));
        let k2 = UsrKindId::new(3, Some("test2"), None);
        let k3 = UsrKindId::new(7, Some("test3"), Some(Kind::U8));
        res!(ukinds.add(k1.clone()));
        res!(ukinds.add(k2.clone()));
        res!(ukinds.add(k3.clone()));
        let json_enc = EncoderConfig::json(Some(ukinds.clone()));
        let json_dec = DecoderConfig::json(Some(ukinds));

        let d1_enc = res!(Dat::try_from((k1, Some(dat!("hello")))));
        let d1_enc_str = res!(d1_enc.encode_string_with_config(&json_enc));
        test!("json enc = {}", d1_enc_str);
        let d1_dec = res!(Dat::decode_string_with_config(d1_enc_str.clone(), &json_dec));
        // Debug has no knowledge of usr kinds.
        let d1_dec_str = res!(d1_dec.encode_string_with_config(&json_enc));
        test!("json dec = {}", d1_dec_str);
        req!(&d1_enc_str, "\"hello\"");

        let d2_enc = res!(Dat::try_from((k2, None)));
        let d2_enc_str = res!(d2_enc.encode_string_with_config(&json_enc));
        test!("json enc = {}", d2_enc_str);
        let d2_dec = res!(Dat::decode_string_with_config(d2_enc_str.clone(), &json_dec));
        // Debug has no knowledge of usr kinds.
        let d2_dec_str = res!(d2_dec.encode_string_with_config(&json_enc));
        test!("json dec = {}", d2_dec_str);
        req!(&d2_enc_str, "\"test2\"");

        let d3_enc = res!(Dat::try_from((k3, Some(dat!(42u8)))));
        let d3_enc_str = res!(d3_enc.encode_string_with_config(&json_enc));
        test!("json enc = {}", d3_enc_str);
        let d3_dec = res!(Dat::decode_string_with_config(d3_enc_str.clone(), &json_dec));
        // Debug has no knowledge of usr kinds.
        let d3_dec_str = res!(d3_dec.encode_string_with_config(&json_enc));
        test!("json dec = {}", d3_dec_str);
        req!(&d3_enc_str, "42");
        Ok(())
    }));

    res!(test_it(filter, &["String encode decode 620", "all", "usr"], || {
        let mut ukinds = UsrKinds::new(BTreeMap::new(), BTreeMap::new());
        let k1 = UsrKindId::new(5, Some("test1"), Some(Kind::Str));
        let k2 = UsrKindId::new(3, Some("test2"), None);
        let k3 = UsrKindId::new(7, Some("test3"), Some(Kind::U8));
        res!(ukinds.add(k1.clone()));
        res!(ukinds.add(k2.clone()));
        res!(ukinds.add(k3.clone()));
        let jdat_enc = EncoderConfig::jdat(Some(ukinds.clone()));
        //let jdat_dec = DecoderConfig::jdat(Some(ukinds));

        let d1 = res!(Dat::try_from((k1, Some(dat!("hello")))));
        let d2 = res!(Dat::try_from((k2, None)));
        let d3 = res!(Dat::try_from((k3, Some(dat!(42u8)))));

        let d4 = mapdat! {
            d1 => dat!("val1"),
            d2 => dat!("val2"),
            d3 => dat!("val3"),
        };

        let d4_enc_str = res!(d4.encode_string_with_config(&jdat_enc));

        // Verify that the key ordering is as expected, and refers to the UsrKindCode.
        let expected = [
            fmt!("{{"),
            fmt!("(test2): \"val2\","),
            fmt!("(test1|\"hello\"): \"val1\","),
            fmt!("(test3|(u8|42)): \"val3\","),
            fmt!("}}"),
        ];

        test!("jdat enc:");
        let mut i: usize = 0;
        for line in &Stringer::new(d4_enc_str).to_lines("") {
            test!("{}", line);
            req!(*line, expected[i]);
            i += 1;
        }
        Ok(())

    }));

    res!(test_it(filter, &["Annotated Box encode", "all", "abox"], || {
        let d1 = Dat::ABox(
            NoteConfig::default(),
            Box::new(dat!(1u8)),
            fmt!(" This is a comment "),
        );
        req!(fmt!("{:?}", d1), "(abox|(u8|1) ! This is a comment !)");
        test!("{:?}", d1);
        req!(fmt!("{}", d1), "(u8|1) ! This is a comment !");
        test!("{}", d1);
        Ok(())
    }));

    res!(test_it(filter, &["Annotated Box map decode 000", "all", "abox", "map"], || {
        let d = res!(Dat::decode_string("
        {
            1:2,
            :! A singleton comment keyed to the empty dat !,
            ! A self-keyed comment that can appear anywhere in the map
            3:4,
        }"));
        let lines = d.to_lines("  ", false);
        test!("Map with comments:");
        for line in &lines {
            test!("{}", line);
        }
        let expected = mapdat!{
            1u8 => 2u8,
            () => abox!((), "A singleton comment keyed to the empty dat "),
            abox!((), "A self-keyed comment that can appear anywhere in the map") => (),
            3u8 => 4u8,
        };
        req!(d, expected);
        Ok(())
    }));

    res!(test_it(filter, &["Annotated Box list decode 000", "all", "abox", "list"], || {
        let d = res!(Dat::decode_string("
        [
            1,
            2,
            3 ! A comment !,
            ! Another comment !,
            ! A comment to the end of the line
            4,
        ]"));
        let lines = d.to_lines("  ", false);
        test!("List with comments:");
        for line in &lines {
            test!("{}", line);
        }
        let expected = listdat![
            1u8,
            2u8,
            abox!(3u8, "A comment "),
            abox!((), "Another comment "),
            abox!((), "A comment to the end of the line"), 
            4u8,
        ];
        req!(d, expected);
        Ok(())
    }));

    res!(test_it(filter, &["Annotated Box display encode decode 000", "all", "abox", "list"], || {
        let d0 = Dat::ABox(
            NoteConfig::default(),
            Box::new(dat!(2u8)),
            fmt!("This is a comment"),
        );
        let d1 = Dat::ABox(
            NoteConfig::default(),
            Box::new(Dat::Empty),
            fmt!("Another comment"),
        );
        let d5 = listdat![1u8, d0, 3u8, d1];
        let lines = d5.to_lines("  ", false);
        test!("List display with comments:");
        for line in &lines {
            test!("{}", line);
        }
        let d10 = res!(Dat::decode_string(fmt!("{}", d5)));
        req!(d5, d10);
        Ok(())
    }));

    res!(test_it(filter, &["Annotated Box display encode decode 005", "all", "abox", "list"], || {
        let d0 = Dat::ABox(
            NoteConfig::default(),
            Box::new(dat!(2u8)),
            fmt!("This is a comment"),
        );
        let d1 = Dat::ABox(
            NoteConfig::default().set_type1(false),
            Box::new(Dat::Empty),
            fmt!("A type2 comment"),
        );
        let d5 = listdat![1u8, d0, 3u8, d1, 4u8];
        let lines = d5.to_lines("  ", false);
        test!("List display with comments:");
        for line in &lines {
            test!("{}", line);
        }
        let d10 = res!(Dat::decode_string(fmt!("{}", d5)));
        req!(d5, d10);
        Ok(())
    }));

    res!(test_it(filter, &["Annotated Box display encode decode 010", "all", "abox", "list", "nested"], || {
        let d0 = Dat::ABox(
            NoteConfig::default(),
            Box::new(dat!(2u8)),
            fmt!("This is a comment"),
        );
        let d1 = Dat::ABox(
            NoteConfig::default(),
            Box::new(Dat::Empty),
            fmt!("Another comment"),
        );
        let d2 = Dat::ABox(
            NoteConfig::default(),
            Box::new(Dat::Empty),
            fmt!("Another line to add."),
        );
        let d5 = listdat![1u8, d0, 3u8, listdat![d1, d2, 4u8], 5u8];
        let lines = d5.to_lines("  ", false);
        test!("List display with comments:");
        for line in &lines {
            test!("{}", line);
        }
        let d10 = res!(Dat::decode_string(fmt!("{}", d5)));
        req!(d5, d10);
        Ok(())
    }));

    res!(test_it(filter, &["Annotated Box display encode decode 050", "all", "abox", "map"], || {
        let d0 = Dat::ABox(
            NoteConfig::default(),
            Box::new(dat!(2u8)),
            fmt!("This is a comment"),
        );
        let d1 = Dat::ABox(
            NoteConfig::default(),
            Box::new(Dat::Empty),
            fmt!("Another comment"),
        );
        let d2 = Dat::ABox(
            NoteConfig::default().set_type1(false),
            Box::new(Dat::Empty),
            fmt!("A type2 comment"),
        );
        let d5 = mapdat!{1u8 => d0, d2 => 2u8, 3u8 => d1};
        let lines = d5.to_lines("  ", false);
        test!("Map display with comments:");
        for line in &lines {
            test!("{}", line);
        }
        let d10 = res!(Dat::decode_string(fmt!("{}", d5)));
        req!(d5, d10);
        Ok(())
    }));

    res!(test_it(filter, &["Integrated multiline 000", "all", "abox", "list", "map"], || {
        let d1 = listdat![
            1u8,
            "two",
            omapdat!{
                () => abox!((), "A comment keyed to the empty daticle."),
                10u8 => abox!((), "A comment keyed to a non-empty daticle."),
                abox!((), "A full line comment which is the key, with an empty value daticle.") => (),
                3u8 => listdat![
                    4u8,
                    "five",
                    6u8,
                ],
                11u8 => tup3dat![
                    abox!((), "This is a 3-tuple with a comment."),
                    12u8,
                    13u8,
                ],
            },
            listdat![
                abox!((), "A line comment"), 
                8u8,
                9u8,
            ],
        ];
        let expected = vec![
            "[",
            "  1,",
            "  \"two\",",
            "  (omap|{",
            "    :! A comment keyed to the empty daticle.",
            "    10:! A comment keyed to a non-empty daticle.",
            "    ! A full line comment which is the key, with an empty value daticle.",
            "    3: [",
            "      4,",
            "      \"five\",",
            "      6",
            "    ],",
            "    11: (t3|[",
            "      ! This is a 3-tuple with a comment.",
            "      12,",
            "      13",
            "    ])",
            "  }),",
            "  [",
            "    ! A line comment",
            "    8,",
            "    9",
            "  ]",
            "]",
        ];
        test!("{}", d1);
        let jdat_str = res!(d1.display_some_to_lines("  "));
        let d2 = res!(Dat::decode_string(jdat_str.clone()));
        let lines: Vec<&str> = jdat_str.lines().collect();
        test!("Display output:");
        for (i, line) in lines.iter().enumerate() {
            test!("{:03}: {}", i + 1, line);
        }
        test!("Expected output:");
        for (i, line) in expected.iter().enumerate() {
            test!("{:03}: {}", i + 1, line);
        }
        req!(lines.len(), expected.len(), "(L: actual len, R: expected len)");
        let mut c = 0;
        for line in lines {
            req!(line, expected[c].to_string(), "(L: encoded, R: expected) line {}", c + 1);
            c += 1;
        }
        let result2 = res!(d2.display_to_lines("  "));
        let lines: Vec<&str> = result2.split("\n").collect();
        test!("Display output:");
        for line in &lines {
            test!("{}", line);
        }
        req!(d1, d2, "(L: encoded, R: decoded)");
        Ok(())
    }));

    res!(test_it(filter, &["Integrated multiline 010", "all", "abox", "omnibus"], || {
        let ukind_ipv4 = UsrKindId::new(5, Some("ipv4"), Some(Kind::B4));
        let mut ukinds = UsrKinds::new(BTreeMap::new(), BTreeMap::new());
        res!(ukinds.add(ukind_ipv4.clone()));
        let d1 = listdat![
            1u8,
            "two",
            omapdat!{
                abox!((), "Ordered maps are a great way to fix the order of a map,") => (),
                abox!((), "allowing, for example, you to position multiline comments") => (),
                abox!((), "where you want them.") => (),
                3u8 => listdat![
                    4u8,
                    "five",
                    res!(Dat::try_from((ukind_ipv4.clone(), Some(Dat::B4([1,2,3,4]))))),
                    6u8,
                ],
                11u8 => tup3dat![
                    abox!((), "This is a 3-tuple with a comment."),
                    12u8,
                    13u8,
                ],
            },
            listdat![
                abox!((), "A line comment"), 
                8u8,
                9u8,
            ],
        ];
        let expected = vec![
            "[",
            "  1,",
            "  \"two\",",
            "  (omap|{",
            "    ! Ordered maps are a great way to fix the order of a map,",
            "    ! allowing, for example, you to position multiline comments",
            "    ! where you want them.",
            "    3: [",
            "      4,",
            "      \"five\",",
            "      (ipv4|(b4|[1, 2, 3, 4])),",
            "      6",
            "    ],",
            "    11: (t3|[",
            "      ! This is a 3-tuple with a comment.",
            "      12,",
            "      13",
            "    ])",
            "  }),",
            "  [",
            "    ! A line comment",
            "    8,",
            "    9",
            "  ]",
            "]",
        ];
        test!("{}", d1);
        let jdat_str = res!(d1.display_some_to_lines("  "));
        let mut dec_cfg = DecoderConfig::default();
        dec_cfg.ukinds_opt = Some(ukinds);
        let d2 = res!(Dat::decode_string_with_config(jdat_str.clone(), &dec_cfg));
        let lines: Vec<&str> = jdat_str.lines().collect();
        test!("Display output:");
        for (i, line) in lines.iter().enumerate() {
            test!("{:03}: {}", i + 1, line);
        }
        test!("Expected output:");
        for (i, line) in expected.iter().enumerate() {
            test!("{:03}: {}", i + 1, line);
        }
        req!(lines.len(), expected.len(), "(L: actual len, R: expected len)");
        let mut c = 0;
        for line in lines {
            req!(line, expected[c].to_string(), "(L: encoded, R: expected) line {}", c + 1);
            c += 1;
        }
        let result2 = res!(d2.display_to_lines("  "));
        let lines: Vec<&str> = result2.split("\n").collect();
        test!("Display output:");
        for line in &lines {
            test!("{}", line);
        }
        req!(d1, d2, "(L: encoded, R: decoded)");
        Ok(())
    }));

    res!(test_it(filter, &["Annotated Box debug encode decode 000", "all", "abox", "list"], || {
        let d0 = Dat::ABox(
            NoteConfig::default(),
            Box::new(dat!(2u8)),
            fmt!("This is a comment"),
        );
        let d1 = Dat::ABox(
            NoteConfig::default(),
            Box::new(Dat::Empty),
            fmt!("A standalone comment"),
        );
        let d5 = listdat![1u8, d0, 3u8, d1];
        let d5_str = fmt!("{:?}", d5);
        test!("List debug with comment:");
        for line in str!(d5_str.clone()).to_lines("  ") {
            test!("{}", line);
        }
        let d10 = res!(Dat::decode_string(d5_str));
        req!(d5, d10);
        Ok(())
    }));

    res!(test_it(filter, &["Annotated Box debug encode decode 005", "all", "abox", "list"], || {
        let d0 = Dat::ABox(
            NoteConfig::default(),
            Box::new(dat!(2u8)),
            fmt!("This is a comment"),
        );
        let d1 = Dat::ABox(
            NoteConfig::default(),
            Box::new(Dat::Empty),
            fmt!("A standalone comment"),
        );
        let d5 = listdat![1u8, d0, 3u8, d1, 4u8];
        let d5_str = fmt!("{:?}", d5);
        test!("List debug with comment:");
        for line in str!(d5_str.clone()).to_lines("  ") {
            test!("{}", line);
        }
        let d10 = res!(Dat::decode_string(d5_str));
        req!(d5, d10);
        Ok(())
    }));

    Ok(())
}
