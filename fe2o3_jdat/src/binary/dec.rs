use crate::{
    prelude::*,
    note::NoteConfig,
    usr::{
        UsrKindCode,
        UsrKindId,
    },
};

use oxedyne_fe2o3_core::{
    prelude::*,
    byte::FromBytes,
};
use oxedyne_fe2o3_num::float::{
    Float32,
    Float64,
};

use std::convert::TryFrom;

use bigdecimal::{
    BigDecimal,
    Zero,
};
use num_bigint::BigInt;


impl FromBytes for Dat {

    /// Read the `Dat` from the buffer, and include the number of bytes required in the return
    /// tuple.
    fn from_bytes(buf: &[u8]) -> Outcome<(Self, usize)> {
        if buf.len() == 0 {
            return Err(err!("No bytes to decode."; Input, Invalid));
        }
        match buf[0] {
            // Atomic Kinds ===========================
            // Logic
            Self::EMPTY_CODE => return Ok((Self::Empty, 1)),
            Self::TRUE_CODE => return Ok((Self::Bool(true), 1)),
            Self::FALSE_CODE => return Ok((Self::Bool(false), 1)),
            Self::OPT_NONE_CODE => return Ok((Self::Opt(Box::new(None)), 1)),
            // Fixed
            Self::U8_CODE => {
                // +---+---+
                // | c |   |
                // +---+---+
                //     \___/
                //       |
                //       u8
                //
                if buf.len() > 1 {
                    return Ok((
                        Self::U8(u8::from_be_bytes(
                            res!(<[u8; 1]>::try_from(&buf[1..2]), Decode, Bytes)
                        )),
                        2,
                    ));
                } else {
                    return Err(<Dat as FromBytes>::too_few(
                        buf.len(), 2, &Self::code_name(buf[0]), file!(), line!()));
                }
            }
            Self::U16_CODE => {
                //      MSB LSB
                // +---+---+---+
                // | c |   |   |
                // +---+---+---+
                //     \_______/
                //         |
                //        u16
                //
                if buf.len() > 2 {
                    return Ok((
                        Self::U16(u16::from_be_bytes(
                            res!(<[u8; 2]>::try_from(&buf[1..3]), Decode, Bytes)
                        )),
                        3,
                    ));
                } else {
                    return Err(<Dat as FromBytes>::too_few(
                        buf.len(), 3, &Self::code_name(buf[0]), file!(), line!()));
                }
            }
            Self::U32_CODE => {
                //      MSB         LSB
                // +---+---+---+---+---+
                // | c |   |   |   |   |
                // +---+---+---+---+---+
                //     \_______________/
                //             |
                //            u32
                //
                if buf.len() > 4 {
                    return Ok((
                        Self::U32(u32::from_be_bytes(
                            res!(<[u8; 4]>::try_from(&buf[1..5]), Decode, Bytes)
                        )),
                        5,
                    ));
                } else {
                    return Err(<Dat as FromBytes>::too_few(
                        buf.len(), 5, &Self::code_name(buf[0]), file!(), line!()));
                }
            }
            Self::U64_CODE => {
                //      MSB                         LSB
                // +---+---+---+---+---+---+---+---+---+
                // | c |   |   |   |   |   |   |   |   |
                // +---+---+---+---+---+---+---+---+---+
                //     \_______________________________/
                //                     |
                //                    u64
                //
                if buf.len() > 8 {
                    return Ok((
                        Self::U64(u64::from_be_bytes(
                            res!(<[u8; 8]>::try_from(&buf[1..9]), Decode, Bytes)
                        )),
                        9,
                    ));
                } else {
                    return Err(<Dat as FromBytes>::too_few(
                        buf.len(), 9, &Self::code_name(buf[0]), file!(), line!()));
                }
            }
            Self::U128_CODE => {
                //      MSB                         LSB
                // +---+---+---+---+---+---+---+---+---+
                // | c |   |   |   |   |   |   |   |   |
                // +---+---+---+---+---+---+---+---+---+
                //     \_______________________________/
                //                     |
                //                    u128
                //
                if buf.len() > 16 {
                    return Ok((
                        Self::U128(u128::from_be_bytes(
                            res!(<[u8; 16]>::try_from(&buf[1..17]), Decode, Bytes)
                        )),
                        17,
                    ));
                } else {
                    return Err(<Dat as FromBytes>::too_few(
                        buf.len(), 17, &Self::code_name(buf[0]), file!(), line!()));
                }
            }
            Self::I8_CODE => {
                // +---+---+
                // | c |   |
                // +---+---+
                //     \___/
                //       |
                //       i8
                //
                if buf.len() > 1 {
                    return Ok((
                        Self::I8(i8::from_be_bytes(
                            res!(<[u8; 1]>::try_from(&buf[1..2]), Decode, Bytes)
                        )),
                        2,
                    ));
                } else {
                    return Err(<Dat as FromBytes>::too_few(
                        buf.len(), 2, &Self::code_name(buf[0]), file!(), line!()));
                }
            }
            Self::I16_CODE => {
                //      MSB LSB
                // +---+---+---+
                // | c |   |   |
                // +---+---+---+
                //     \_______/
                //         |
                //        i16
                //
                if buf.len() > 2 {
                    return Ok((
                        Self::I16(i16::from_be_bytes(
                            res!(<[u8; 2]>::try_from(&buf[1..3]), Decode, Bytes)
                        )),
                        3,
                    ));
                } else {
                    return Err(<Dat as FromBytes>::too_few(
                        buf.len(), 3, &Self::code_name(buf[0]), file!(), line!()));
                }
            }
            Self::I32_CODE => {
                //      MSB         LSB
                // +---+---+---+---+---+
                // | c |   |   |   |   |
                // +---+---+---+---+---+
                //     \_______________/
                //             |
                //            i32
                //
                if buf.len() > 4 {
                    return Ok((
                        Self::I32(i32::from_be_bytes(
                            res!(<[u8; 4]>::try_from(&buf[1..5]), Decode, Bytes)
                        )),
                        5,
                    ));
                } else {
                    return Err(<Dat as FromBytes>::too_few(
                        buf.len(), 5, &Self::code_name(buf[0]), file!(), line!()));
                }
            }
            Self::I64_CODE => {
                //      MSB                         LSB
                // +---+---+---+---+---+---+---+---+---+
                // | c |   |   |   |   |   |   |   |   |
                // +---+---+---+---+---+---+---+---+---+
                //     \_______________________________/
                //                     |
                //                    i64
                //
                if buf.len() > 8 {
                    return Ok((
                        Self::I64(i64::from_be_bytes(
                            res!(<[u8; 8]>::try_from(&buf[1..9]), Decode, Bytes)
                        )),
                        9,
                    ));
                } else {
                    return Err(<Dat as FromBytes>::too_few(
                        buf.len(), 9, &Self::code_name(buf[0]), file!(), line!()));
                }
            }
            Self::I128_CODE => {
                //      MSB                         LSB
                // +---+---+---+---+---+---+---+---+---+
                // | c |   |   |   |   |   |   |   |   |
                // +---+---+---+---+---+---+---+---+---+
                //     \_______________________________/
                //                     |
                //                    i128
                //
                if buf.len() > 16 {
                    return Ok((
                        Self::I128(i128::from_be_bytes(
                            res!(<[u8; 16]>::try_from(&buf[1..17]), Decode, Bytes)
                        )),
                        17,
                    ));
                } else {
                    return Err(<Dat as FromBytes>::too_few(
                        buf.len(), 17, &Self::code_name(buf[0]), file!(), line!()));
                }
            }
            Self::F32_CODE => {
                //      MSB         LSB
                // +---+---+---+---+---+
                // | c |   |   |   |   |
                // +---+---+---+---+---+
                //     \_______________/
                //             |
                //            f32
                //
                if buf.len() > 4 {
                    return Ok((
                        Self::F32(Float32(f32::from_be_bytes(
                            res!(<[u8; 4]>::try_from(&buf[1..5]), Decode, Bytes)
                        ))),
                        5,
                    ));
                } else {
                    return Err(<Dat as FromBytes>::too_few(
                        buf.len(), 5, &Self::code_name(buf[0]), file!(), line!()));
                }
            }
            Self::F64_CODE => {
                //      MSB                         LSB
                // +---+---+---+---+---+---+---+---+---+
                // | c |   |   |   |   |   |   |   |   |
                // +---+---+---+---+---+---+---+---+---+
                //     \_______________________________/
                //                     |
                //                    f64
                //
                if buf.len() > 8 {
                    return Ok((
                        Self::F64(Float64(f64::from_be_bytes(
                            res!(<[u8; 8]>::try_from(&buf[1..9]), Decode, Bytes)
                        ))),
                        9,
                    ));
                } else {
                    return Err(<Dat as FromBytes>::too_few(
                        buf.len(), 9, &Self::code_name(buf[0]), file!(), line!()));
                }
            }
            // Variable
            Self::C64_CODE_START..=Self::C64_CODE_END => { // C64
                //
                //  MSB        big endian       LSB
                // +---+---+---+---+---+---+---+---+
                // | 0 | 0 | 0 | 0 | 0 | 0 | 0 |128|
                // +---+---+---+---+---+---+---+---+
                // \_______________________________/
                //                 |
                //                u64
                //
                // Potential encoding for: Dat::C64
                // +---+---+---+
                // | c | n |128|  n = 1
                // +---+---+---+
                // \___________/
                //       |
                //     c64
                //
                // Actual encoding for: Dat::C64
                // +---+---+
                // | c |128|  c -> c + 1 (n = 1 incorporated into prefix code)
                // +---+---+
                // \_______/
                //     |
                //   c64
                //
                let (n64, n) = res!(Self::read_c64(buf));
                return Ok((Dat::C64(n64), n));
            }
            Self::BC64_CODE => {
                //
                //   0   1   2  ...  n   1   2  ...  v
                // +---+---+---+---+---+---+---+---+---+
                // | c |   |   |   |   |   |  ...  |   |
                // +---+---+---+---+---+---+---+---+---+
                //      \______________/\______________/
                //             |               |
                //            c64        payload bytes
                //
                if buf.len() > 1 { 
                    match buf[1] {
                        Self::C64_CODE_START..=Self::C64_CODE_END => {
                            let (v, n) = res!(Self::read_c64(&buf[1..]));
                            if v == 0 {
                                return Ok((
                                    Self::BC64(Vec::new()),
                                    1 + n,
                                ));
                            }
                            let v = v as usize;
                            if buf.len() > 1 - 1 + n + v {
                                return Ok((
                                    Self::BC64(buf[1 + n .. 1 + n + v].to_vec()),
                                    1 + n + v,
                                ));
                            } else {
                                return Err(<Dat as FromBytes>::too_few(
                                    buf.len(),
                                    1 + n + v,
                                    &Self::code_name(buf[0]),
                                    file!(),
                                    line!(),
                                ));
                            }
                        }
                        _ => return Err(err!(
                            "{} code was not followed by a code for a Dat::C64 in the correct \
                            range {}..{}, the code found was {}.",
                            Self::code_name(buf[0]), Self::C64_CODE_START, Self::C64_CODE_END,
                            buf[1];
                        Bytes, Input, Decode, Missing)),
                    }
                } else {
                    return Err(<Dat as FromBytes>::too_few(
                        buf.len(), 1, &Self::code_name(buf[0]), file!(), line!()));
                }
            }
            Self::STR_CODE => {
                //
                //   0   1   2  ...  n   1   2  ...  v
                // +---+---+---+---+---+---+---+---+---+
                // | c |   |   |   |   |   |  ...  |   |
                // +---+---+---+---+---+---+---+---+---+
                //      \______________/\______________/
                //             |               |
                //            c64        payload bytes
                //
                if buf.len() > 1 { 
                    match buf[1] {
                        Self::C64_CODE_START..=Self::C64_CODE_END => {
                            let (v, n) = res!(Self::read_c64(&buf[1..]));
                            if v == 0 {
                                return Ok((
                                    Self::Str(String::new()),
                                    1 + n,
                                ));
                            }
                            let v = v as usize;
                            if buf.len() > 1 - 1 + n + v {
                                let owned = &buf[1 + n .. 1 + n + v].to_vec();
                                return Ok((
                                    Self::Str(res!(std::str::from_utf8(
                                        owned
                                    )).to_string()),
                                    1 + n + v,
                                ));
                            } else {
                                return Err(<Dat as FromBytes>::too_few(
                                    buf.len(),
                                    1 + n + v,
                                    &Self::code_name(buf[0]),
                                    file!(),
                                    line!(),
                                ));
                            }
                        }
                        _ => return Err(err!(
                            "{} code was not followed by a code for a Dat::C64 in the correct \
                            range {}..{}, the code found was {}.",
                            Self::code_name(buf[0]), Self::C64_CODE_START, Self::C64_CODE_END,
                            buf[1];
                        Bytes, Input, Decode, Missing)),
                    }
                } else {
                    return Err(<Dat as FromBytes>::too_few(
                        buf.len(), 1, &Self::code_name(buf[0]), file!(), line!()));
                }
            }
            Self::AINT_CODE => {
                //
                //   0   1   2  ...  n   1   2  ...  v
                // +---+---+---+---+---+---+---+---+---+
                // | c |   |   |   |   |   |  ...  |   |
                // +---+---+---+---+---+---+---+---+---+
                //      \______________/\______________/
                //             |               |
                //            c64        payload bytes
                //
                if buf.len() > 1 { 
                    match buf[1] {
                        Self::C64_CODE_START..=Self::C64_CODE_END => {
                            let (v, n) = res!(Self::read_c64(&buf[1..]));
                            if v == 0 {
                                return Ok((
                                    Self::Aint(BigInt::zero()),
                                    1 + n,
                                ));
                            }
                            let v = v as usize;
                            if buf.len() > 1 - 1 + n + v {
                                return Ok((
                                    Self::Aint(BigInt::from_signed_bytes_be(
                                        &buf[1 + n .. 1 + n + v]
                                    )),
                                    1 + n + v,
                                ));
                            } else {
                                return Err(<Dat as FromBytes>::too_few(
                                    buf.len(),
                                    1 + n + v,
                                    &Self::code_name(buf[0]),
                                    file!(),
                                    line!(),
                                ));
                            }
                        }
                        _ => return Err(err!(
                            "{} code was not followed by a code for a Dat::C64 in the correct \
                            range {}..{}, the code found was {}.",
                            Self::code_name(buf[0]), Self::C64_CODE_START, Self::C64_CODE_END,
                            buf[1];
                        Bytes, Input, Decode, Missing)),
                    }
                } else {
                    return Err(<Dat as FromBytes>::too_few(
                        buf.len(), 1, &Self::code_name(buf[0]), file!(), line!()));
                }
            }
            Self::ADEC_CODE => {
                //
                //   0   1   2  ...  n   1   2  ...  v
                // +---+---+---+---+---+---+---+---+---+
                // | c |   |   |   |   |   |  ...  |   |
                // +---+---+---+---+---+---+---+---+---+
                //      \______________/\______________/
                //             |               |
                //            c64        payload bytes
                //
                if buf.len() > 1 { 
                    match buf[1] {
                        Self::C64_CODE_START..=Self::C64_CODE_END => {
                            let (v, n) = res!(Self::read_c64(&buf[1..]));
                            if v == 0 {
                                return Ok((
                                    Self::Adec(BigDecimal::zero()),
                                    1 + n,
                                ));
                            }
                            let v = v as usize;
                            if buf.len() > 1 - 1 + n + v {
                                let bigint = BigInt::from_signed_bytes_be(
                                    &buf[1 + n .. 1 + n + v - 8]
                                );
                                let expi64 = i64::from_be_bytes(
                                    res!(<[u8; 8]>::try_from(&buf[1 + n + v - 8 ..]),
                                        Decode, Bytes)
                                );
                                return Ok((
                                    Self::Adec(BigDecimal::new(bigint, expi64)),
                                    1 + n + v,
                                ));
                            } else {
                                return Err(<Dat as FromBytes>::too_few(
                                    buf.len(),
                                    1 + n + v,
                                    &Self::code_name(buf[0]),
                                    file!(),
                                    line!(),
                                ));
                            }
                        }
                        _ => return Err(err!(
                            "{} code was not followed by a code for a Dat::C64 in the correct \
                            range {}..{}, the code found was {}.",
                            Self::code_name(buf[0]), Self::C64_CODE_START, Self::C64_CODE_END,
                            buf[1];
                        Bytes, Input, Decode, Missing)),
                    }
                } else {
                    return Err(<Dat as FromBytes>::too_few(
                        buf.len(), 1, &Self::code_name(buf[0]), file!(), line!()));
                }
            }
            // Molecular Kinds ========================
            // Unitary
            Self::USR_CODE => {
                // 
                // +---+---+---+---+---+---+---+---+---+
                // | c |  u16  |   |   |   |   |   |   |
                // +---+---+---+---+---+---+---+---+---+
                //   ^     ^     ^ \___________________/   
                //   |     |     |           |                   
                //   |     |  opt code   inner Dat            
                //   |     ukid_code     (if some)
                //  daticle
                //  code
                //
                const CODE_LEN: usize = UsrKindId::CODE_BYTE_LEN;
                if buf.len() > CODE_LEN {
                    let ukid_code = UsrKindCode::from_be_bytes(
                        res!(<[u8; CODE_LEN]>::try_from(&buf[1..1 + CODE_LEN]), Decode, Bytes)
                    );
                    let opt_code = buf[1 + CODE_LEN];
                    match opt_code {
                        Self::OPT_NONE_CODE => return Ok((
                            Dat::Usr(
                                UsrKindId::from(ukid_code),
                                None,
                            ),
                            2 + CODE_LEN,
                        )),
                        Self::OPT_SOME_CODE => {
                            let (inner, n) = res!(Self::from_bytes(&buf[2 + CODE_LEN..]));
                            return Ok((
                                Dat::Usr(
                                    UsrKindId::from(ukid_code),
                                    Some(Box::new(inner)),
                                ),
                                2 + CODE_LEN + n,
                            ));
                        }
                        _ => return Err(err!(
                            "Expecting an option code {} (for none) or {} (for some), \
                            instead found code {}.",
                            Self::OPT_NONE_CODE, Self::OPT_SOME_CODE, opt_code;
                        Bytes, Input, Decode, Missing)),
                    }
                } else {
                    return Err(<Dat as FromBytes>::too_few(
                        buf.len(), CODE_LEN, &Self::code_name(buf[0]), file!(), line!()));
                }
            }
            Self::BOX_CODE => {
                // 
                // +---+---+---+---+---+---+---+---+---+
                // | c |   |   |   |   |   |   |   |   |
                // +---+---+---+---+---+---+---+---+---+
                //     \_______________________________/
                //                     |
                //                inner Dat
                //
                // Dat::Box(k) byte encoding does nothing more than prepend Dat k with
                // Dat::BOX_CODE
                //
                if buf.len() > 1 {
                    let (inner, n) = res!(Self::from_bytes(&buf[1..]));
                    return Ok((Dat::Box(Box::new(inner)), 1 + n));
                } else {
                    return Err(<Dat as FromBytes>::too_few(
                        buf.len(), 1, &Self::code_name(buf[0]), file!(), line!()));
                }
            }
            Self::OPT_SOME_CODE => {
                // 
                // +---+---+---+---+---+---+---+---+---+
                // | c |   |   |   |   |   |   |   |   |
                // +---+---+---+---+---+---+---+---+---+
                //     \_______________________________/
                //                     |
                //                inner Dat
                //
                // Dat::Opt(Some(k)) byte encoding does nothing more than prepend Dat k
                // with Dat::OPT_SOME_CODE
                //
                if buf.len() > 1 {
                    let (inner, n) = res!(Self::from_bytes(&buf[1..]));
                    return Ok((Dat::Opt(Box::new(Some(inner))), 1 + n));
                } else {
                    return Err(<Dat as FromBytes>::too_few(
                        buf.len(), 1, &Self::code_name(buf[0]), file!(), line!()));
                }
            }
            Self::ABOX_CODE => {
                //
                //   0                                       1   2  ...  n   1   2  ...  v
                // +---+---+---+---+---+---+---+---+---+---+---+---+---+---+---+---+---+---+
                // | c |   |   |   |   |   |   |   |   |   |   |   |   |   |   |  ...  |   |
                // +---+---+---+---+---+---+---+---+---+---+---+---+---+---+---+---+---+---+
                //       | \_______________________________/\______________/\______________/
                //       |                 |                         |               |
                //  NoteConfig         inner Dat                    c64        payload bytes
                //
                if buf.len() > 1 { 
                    let mut start: usize = 1;
                    let (ncfg, n) = res!(NoteConfig::from_bytes(&buf[start..]));
                    start += n;
                    let (boxd, n) = if buf.len() > start {
                        let (inner, n) = res!(Self::from_bytes(&buf[start..]));
                        (Box::new(inner), n)
                    } else {
                        return Err(<Dat as FromBytes>::too_few(
                            buf.len(), start, &Self::code_name(buf[0]), file!(), line!()));
                    };
                    start += n;
                    match buf[start] {
                        Self::C64_CODE_START..=Self::C64_CODE_END => {
                            let (v, n) = res!(Self::read_c64(&buf[start..]));
                            if v == 0 {
                                return Ok((
                                    Self::ABox(ncfg, boxd, String::new()),
                                    start + n,
                                ));
                            }
                            let v = v as usize;
                            if buf.len() > start - 1 + n + v {
                                let owned = &buf[start + n .. start + n + v].to_vec();
                                return Ok((
                                    Self::ABox(
                                        ncfg,
                                        boxd,
                                        res!(std::str::from_utf8(owned)).to_string(),
                                    ),
                                    start + n + v,
                                ));
                            } else {
                                return Err(<Dat as FromBytes>::too_few(
                                    buf.len(),
                                    start + n + v,
                                    &Self::code_name(buf[0]),
                                    file!(),
                                    line!(),
                                ));
                            }
                        }
                        _ => return Err(err!(
                            "{} code was not followed by a code for a Dat::C64 in the correct \
                            range {}..{}, the code found was {}.",
                            Self::code_name(buf[0]), Self::C64_CODE_START, Self::C64_CODE_END,
                            buf[start];
                        Bytes, Input, Decode, Missing)),
                    }
                } else {
                    return Err(<Dat as FromBytes>::too_few(
                        buf.len(), 1, &Self::code_name(buf[0]), file!(), line!()));
                }
            }
            // Heterogenous
            Self::LIST_CODE | Self::VEK_CODE => {
                //
                //   0   1  ...
                // +---+---+---+---+---+---+---+---+---+---+---+---+---+---+---+---+---+
                // | c |   |   |   | c |   |   | c |   |   | c |   |   |   |  ...  |   |
                // +---+---+---+---+---+---+---+---+---+---+---+---+---+---+---+---+---+
                //      \__________/\__________/\__________/\______________/    ...
                //            |           |          |            |
                //      payload_len      item       item         item
                //
                if buf.len() > 1 { 
                    match buf[1] {
                        Self::C64_CODE_START..=Self::C64_CODE_END => {
                            let (payload_len, n) = res!(Self::read_c64(&buf[1..]));
                            if payload_len == 0 {
                                return Ok((Self::List(Vec::new()), 1 + n));
                            }
                            let payload_len = payload_len as usize;
                            let byt_len = 1 + n + payload_len;
                            if buf.len() > n + payload_len {
                                let mut list = Vec::new();
                                let mut i = 1 + n;
                                while i < byt_len {
                                    let (dat, n) = res!(Dat::from_bytes(&buf[i..]));
                                    i += n;
                                    list.push(dat);
                                }
                                let result = if buf[0] == Self::LIST_CODE {
                                    Self::List(list)
                                } else {
                                    res!(Self::try_vek_from(list))
                                };
                                return Ok((
                                    result,
                                    byt_len,
                                ));
                            } else {
                                return Err(<Dat as FromBytes>::too_few(
                                    buf.len(), n + payload_len, &Self::code_name(buf[0]), file!(), line!()));
                            }
                        }
                        _ => {
                            return Err(err!(
                                "Dat::List code was not followed by a code for a \
                                Dat::C64 in the correct range {}..{}, the code found \
                                was {}",
                                Self::C64_CODE_START,
                                Self::C64_CODE_END,
                                buf[1];
                            Bytes, Input, Decode, Missing));
                        }
                    }
                } else {
                    return Err(<Dat as FromBytes>::too_few(
                        buf.len(), 1, &Self::code_name(buf[0]), file!(), line!()));
                }
            }
            Self::TUP2_CODE => {
                //
                //   0   1  ...
                // +---+---+---+---+---+---+---+
                // | c | c |   |   | c |   |   |
                // +---+---+---+---+---+---+---+
                //      \__________/\__________/
                //            |          |      
                //           item       item    
                //
                if buf.len() > 1 { 
                    const N: usize = 2;
                    let mut list = [
                        Dat::default(),
                        Dat::default(),
                    ];
                    let mut i: usize = 1;
                    for j in 0..N {
                        let (dat, k) = res!(Dat::from_bytes(&buf[i..]));
                        list[j] = dat;
                        i += k;
                    }
                    return Ok((Self::Tup2(Box::new(list)), i));
                } else {
                    return Err(<Dat as FromBytes>::too_few(
                        buf.len(), 1, &Self::code_name(buf[0]), file!(), line!()));
                }
            }
            Self::TUP3_CODE => {
                //
                //   0   1  ...
                // +---+---+---+---+---+---+---+---+---+---+---+
                // | c | c |   |   | c |   |   | c |   |   |   |
                // +---+---+---+---+---+---+---+---+---+---+---+
                //      \__________/\__________/\______________/
                //            |          |            |
                //           item       item         item
                //
                if buf.len() > 1 { 
                    const N: usize = 3;
                    let mut list = [
                        Dat::default(),
                        Dat::default(),
                        Dat::default(),
                    ];
                    let mut i: usize = 1;
                    for j in 0..N {
                        let (dat, k) = res!(Dat::from_bytes(&buf[i..]));
                        list[j] = dat;
                        i += k;
                    }
                    return Ok((Self::Tup3(Box::new(list)), i));
                } else {
                    return Err(<Dat as FromBytes>::too_few(
                        buf.len(), 1, &Self::code_name(buf[0]), file!(), line!()));
                }
            }
            Self::TUP4_CODE => {
                if buf.len() > 1 { 
                    const N: usize = 4;
                    let mut list = [
                        Dat::default(),
                        Dat::default(),
                        Dat::default(),
                        Dat::default(),
                    ];
                    let mut i: usize = 1;
                    for j in 0..N {
                        let (dat, k) = res!(Dat::from_bytes(&buf[i..]));
                        list[j] = dat;
                        i += k;
                    }
                    return Ok((Self::Tup4(Box::new(list)), i));
                } else {
                    return Err(<Dat as FromBytes>::too_few(
                        buf.len(), 1, &Self::code_name(buf[0]), file!(), line!()));
                }
            }
            Self::TUP5_CODE => {
                if buf.len() > 1 { 
                    const N: usize = 5;
                    let mut list = [
                        Dat::default(),
                        Dat::default(),
                        Dat::default(),
                        Dat::default(),
                        Dat::default(),
                    ];
                    let mut i: usize = 1;
                    for j in 0..N {
                        let (dat, k) = res!(Dat::from_bytes(&buf[i..]));
                        list[j] = dat;
                        i += k;
                    }
                    return Ok((Self::Tup5(Box::new(list)), i));
                } else {
                    return Err(<Dat as FromBytes>::too_few(
                        buf.len(), 1, &Self::code_name(buf[0]), file!(), line!()));
                }
            }
            Self::TUP6_CODE => {
                if buf.len() > 1 { 
                    const N: usize = 6;
                    let mut list = [
                        Dat::default(),
                        Dat::default(),
                        Dat::default(),
                        Dat::default(),
                        Dat::default(),
                        Dat::default(),
                    ];
                    let mut i: usize = 1;
                    for j in 0..N {
                        let (dat, k) = res!(Dat::from_bytes(&buf[i..]));
                        list[j] = dat;
                        i += k;
                    }
                    return Ok((Self::Tup6(Box::new(list)), i));
                } else {
                    return Err(<Dat as FromBytes>::too_few(
                        buf.len(), 1, &Self::code_name(buf[0]), file!(), line!()));
                }
            }
            Self::TUP7_CODE => {
                if buf.len() > 1 { 
                    const N: usize = 7;
                    let mut list = [
                        Dat::default(),
                        Dat::default(),
                        Dat::default(),
                        Dat::default(),
                        Dat::default(),
                        Dat::default(),
                        Dat::default(),
                    ];
                    let mut i: usize = 1;
                    for j in 0..N {
                        let (dat, k) = res!(Dat::from_bytes(&buf[i..]));
                        list[j] = dat;
                        i += k;
                    }
                    return Ok((Self::Tup7(Box::new(list)), i));
                } else {
                    return Err(<Dat as FromBytes>::too_few(
                        buf.len(), 1, &Self::code_name(buf[0]), file!(), line!()));
                }
            }
            Self::TUP8_CODE => {
                if buf.len() > 1 { 
                    const N: usize = 8;
                    let mut list = [
                        Dat::default(),
                        Dat::default(),
                        Dat::default(),
                        Dat::default(),
                        Dat::default(),
                        Dat::default(),
                        Dat::default(),
                        Dat::default(),
                    ];
                    let mut i: usize = 1;
                    for j in 0..N {
                        let (dat, k) = res!(Dat::from_bytes(&buf[i..]));
                        list[j] = dat;
                        i += k;
                    }
                    return Ok((Self::Tup8(Box::new(list)), i));
                } else {
                    return Err(<Dat as FromBytes>::too_few(
                        buf.len(), 1, &Self::code_name(buf[0]), file!(), line!()));
                }
            }
            Self::TUP9_CODE => {
                if buf.len() > 1 { 
                    const N: usize = 9;
                    let mut list = [
                        Dat::default(),
                        Dat::default(),
                        Dat::default(),
                        Dat::default(),
                        Dat::default(),
                        Dat::default(),
                        Dat::default(),
                        Dat::default(),
                        Dat::default(),
                    ];
                    let mut i: usize = 1;
                    for j in 0..N {
                        let (dat, k) = res!(Dat::from_bytes(&buf[i..]));
                        list[j] = dat;
                        i += k;
                    }
                    return Ok((Self::Tup9(Box::new(list)), i));
                } else {
                    return Err(<Dat as FromBytes>::too_few(
                        buf.len(), 1, &Self::code_name(buf[0]), file!(), line!()));
                }
            }
            Self::TUP10_CODE => {
                if buf.len() > 1 { 
                    const N: usize = 10;
                    let mut list = [
                        Dat::default(),
                        Dat::default(),
                        Dat::default(),
                        Dat::default(),
                        Dat::default(),
                        Dat::default(),
                        Dat::default(),
                        Dat::default(),
                        Dat::default(),
                        Dat::default(),
                    ];
                    let mut i: usize = 1;
                    for j in 0..N {
                        let (dat, k) = res!(Dat::from_bytes(&buf[i..]));
                        list[j] = dat;
                        i += k;
                    }
                    return Ok((Self::Tup10(Box::new(list)), i));
                } else {
                    return Err(<Dat as FromBytes>::too_few(
                        buf.len(), 1, &Self::code_name(buf[0]), file!(), line!()));
                }
            }
            Self::MAP_CODE => {
                //
                //   0   1  ...
                // +---+---+---+---+---+---+---+---+---+---+---+---+---+---+---+---+---+
                // | c |   |   |   | c |   |   | c |   |   | c |   |   |   |  ...  |   |
                // +---+---+---+---+---+---+---+---+---+---+---+---+---+---+---+---+---+
                //      \__________/\__________/\__________/\______________/    ...
                //            |           |          |            |
                //      payload_len      key       value         key
                //
                if buf.len() > 1 { 
                    match buf[1] {
                        Self::C64_CODE_START..=Self::C64_CODE_END => {
                            let (payload_len, n) = res!(Self::read_c64(&buf[1..]));
                            if payload_len == 0 {
                                return Ok((Self::Map(DaticleMap::new()), 1 + n));
                            }
                            let payload_len = payload_len as usize;
                            let byt_len = 1 + n + payload_len;
                            if buf.len() > n + payload_len {
                                let mut map = DaticleMap::new();
                                let mut i = 1 + n;
                                let mut count: usize = 0;
                                while i < byt_len {
                                    let (key, n) = res!(Dat::from_bytes(&buf[i..]));
                                    i += n;
                                    if i >= byt_len {
                                        return Err(err!(
                                            "Not enough bytes to decode the required value \
                                            for key {:?} in the {:?}, after successfully \
                                            decoding {} key-value pairs.",
                                            key, Self::code_name(buf[0]), count;
                                        Bytes, Input, Decode, Missing));
                                    }
                                    let (val, n) = res!(Dat::from_bytes(&buf[i..]));
                                    i += n;
                                    map.insert(key, val);
                                    count += 1;
                                }
                                return Ok((
                                    Self::Map(map),
                                    byt_len,
                                ));
                            } else {
                                return Err(err!(
                                    "Not enough bytes to decode the {:?} bytes.  \
                                    The map length is {} bytes, but the remaining buffer \
                                    length is {} bytes.",
                                    Self::code_name(buf[0]), payload_len, buf.len() - 1 - n;
                                Bytes, Input, Decode, Missing));
                            }
                        }
                        _ => {
                            return Err(err!(
                                "{:?} code was not followed by a code for a \
                                Dat::C64 in the correct range {}..{}, the code found \
                                was {}",
                                Self::code_name(buf[0]), Self::C64_CODE_START, Self::C64_CODE_END, buf[1];
                            Bytes, Input, Decode, Missing));
                        }
                    }
                } else {
                    return Err(<Dat as FromBytes>::too_few(
                        buf.len(), 1, &Self::code_name(buf[0]), file!(), line!()));
                }
            }
            Self::OMAP_CODE => {
                //
                //   0   1  ...
                // +---+---+---+---+---+---+---+---+---+---+---+---+---+---+---+---+---+
                // | c |   |   |   | c |   |   | c |   |   | c |   |   |   |  ...  |   |
                // +---+---+---+---+---+---+---+---+---+---+---+---+---+---+---+---+---+
                //      \__________/\__________/\__________/\______________/    ...
                //            |           |          |            |
                //      payload_len      key       value         key
                //
                if buf.len() > 1 { 
                    match buf[1] {
                        Self::C64_CODE_START..=Self::C64_CODE_END => {
                            let (payload_len, n) = res!(Self::read_c64(&buf[1..]));
                            if payload_len == 0 {
                                return Ok((Self::OrdMap(OrdDaticleMap::new()), 1 + n));
                            }
                            let payload_len = payload_len as usize;
                            let byt_len = 1 + n + payload_len;
                            if buf.len() > n + payload_len {
                                let mut map = OrdDaticleMap::new();
                                let mut i = 1 + n;
                                let mut count: u64 = 0;
                                let mut order: u64 = Dat::OMAP_ORDER_START_DEFAULT;
                                while i < byt_len {
                                    let (key, n) = res!(Dat::from_bytes(&buf[i..]));
                                    i += n;
                                    if i >= byt_len {
                                        return Err(err!(
                                            "Not enough bytes to decode the required value \
                                            for key {:?} in the {:?}, after successfully \
                                            decoding {} key-value pairs.",
                                            key, Self::code_name(buf[0]), count;
                                        Bytes, Input, Decode, Missing));
                                    }
                                    let (val, n) = res!(Dat::from_bytes(&buf[i..]));
                                    i += n;
                                    map.insert(MapKey::new(order, key), val);
                                    order = try_add!(order, Dat::OMAP_ORDER_DELTA_DEFAULT);
                                    count = try_add!(count, 1);
                                }
                                return Ok((
                                    Self::OrdMap(map),
                                    byt_len,
                                ));
                            } else {
                                return Err(err!(
                                    "Not enough bytes to decode the {:?} bytes.  \
                                    The map length is {} bytes, but the remaining buffer \
                                    length is {} bytes.",
                                    Self::code_name(buf[0]), payload_len, buf.len() - 1 - n;
                                Bytes, Input, Decode, Missing));
                            }
                        }
                        _ => {
                            return Err(err!(
                                "{:?} code was not followed by a code for a \
                                Dat::C64 in the correct range {}..{}, the code found \
                                was {}",
                                Self::code_name(buf[0]), Self::C64_CODE_START, Self::C64_CODE_END, buf[1];
                            Bytes, Input, Decode, Missing));
                        }
                    }
                } else {
                    return Err(<Dat as FromBytes>::too_few(
                        buf.len(), 1, &Self::code_name(buf[0]), file!(), line!()));
                }
            }
            // Homogenous
            // Byte arrays
            // Variable length bytes
            Self::BU8_CODE => {
                //
                //   0   1   1   2  ...  v
                // +---+---+---+---+---+---+  Fixed size, raw u8 for payload length
                // | c |   |   |  ...  |   |
                // +---+---+---+---+---+---+
                //      \__/\______________/
                //       |         |
                //    raw u8   payload bytes
                //
                if buf.len() > 1 { 
                    let v = u8::from_be_bytes(
                        res!(<[u8; 1]>::try_from(&buf[1..2]), Decode, Bytes)
                    ) as usize;
                    if buf.len() > 1 + v {
                        return Ok((
                            Self::BU8(buf[1 + 1 .. 1 + 1 + v].to_vec()),
                            1 + 1 + v,
                        ));
                    } else {
                        return Err(<Dat as FromBytes>::too_few(
                            buf.len(), 1 + 1 + v, &Self::code_name(buf[0]), file!(), line!()));
                    }
                } else {
                    return Err(<Dat as FromBytes>::too_few(
                        buf.len(), 1, &Self::code_name(buf[0]), file!(), line!()));
                }
            }
            Self::BU16_CODE => {
                //
                //   0   1   2   1   2  ...  v
                // +---+---+---+---+---+---+---+  Fixed size, raw u16 for payload length
                // | c |   |   |   |  ...  |   |
                // +---+---+---+---+---+---+---+
                //      \______/\______________/
                //         |           |
                //      raw u16   payload bytes
                //
                if buf.len() > 2 { 
                    let v = u16::from_be_bytes(
                        res!(<[u8; 2]>::try_from(&buf[1..3]), Decode, Bytes)
                    ) as usize;
                    if buf.len() > 2 + v {
                        return Ok((
                            Self::BU16(buf[1 + 2 .. 1 + 2 + v].to_vec()),
                            1 + 2 + v,
                        ));
                    } else {
                        return Err(<Dat as FromBytes>::too_few(
                            buf.len(), 1 + 2 + v, &Self::code_name(buf[0]), file!(), line!()));
                    }
                } else {
                    return Err(<Dat as FromBytes>::too_few(
                        buf.len(), 1, &Self::code_name(buf[0]), file!(), line!()));
                }
            }
            Self::BU32_CODE => {
                //
                //   0   1   2  ...  4   1   2  ...  v
                // +---+---+---+---+---+---+---+---+---+  Fixed size, raw u32 for payload length
                // | c |   |   |   |   |   |  ...  |   |
                // +---+---+---+---+---+---+---+---+---+
                //      \______________/\______________/
                //              |               |
                //           raw u32      payload bytes
                //
                if buf.len() > 4 { 
                    let v = u32::from_be_bytes(
                        res!(<[u8; 4]>::try_from(&buf[1..5]), Decode, Bytes)
                    ) as usize;
                    if buf.len() > 4 + v {
                        return Ok((
                            Self::BU32(buf[1 + 4 .. 1 + 4 + v].to_vec()),
                            1 + 4 + v,
                        ));
                    } else {
                        return Err(<Dat as FromBytes>::too_few(
                            buf.len(), 1 + 4 + v, &Self::code_name(buf[0]), file!(), line!()));
                    }
                } else {
                    return Err(<Dat as FromBytes>::too_few(
                        buf.len(), 1, &Self::code_name(buf[0]), file!(), line!()));
                }
            }
            Self::BU64_CODE  => {
                //
                //   0   1   2  ...  8   1   2  ...  v
                // +---+---+---+---+---+---+---+---+---+  Fixed size, raw u64 for payload length
                // | c |   |   |   |   |   |  ...  |   |
                // +---+---+---+---+---+---+---+---+---+
                //      \______________/\______________/
                //              |               |
                //           raw u64      payload bytes
                //
                if buf.len() > 8 { 
                    let v = u64::from_be_bytes(
                        res!(<[u8; 8]>::try_from(&buf[1..9]), Decode, Bytes)
                    ) as usize;
                    if buf.len() > 8 + v {
                        return Ok((
                            Self::BU64(buf[1 + 8 .. 1 + 8 + v].to_vec()),
                            1 + 8 + v,
                        ));
                    } else {
                        return Err(<Dat as FromBytes>::too_few(
                            buf.len(), 1 + 8 + v, &Self::code_name(buf[0]), file!(), line!()));
                    }
                } else {
                    return Err(<Dat as FromBytes>::too_few(
                        buf.len(), 1, &Self::code_name(buf[0]), file!(), line!()));
                }
            }
            // Fixed length bytes
            Self::B2_CODE   => binary_decode_byte_tuple! { B2, u8, 2, buf },
            //      MSB LSB
            // +---+---+---+
            // | c |   |   |
            // +---+---+---+
            //     \_______/
            //         |
            //      2 bytes
            //
            Self::B3_CODE   => binary_decode_byte_tuple! { B3, u8, 3, buf },
            Self::B4_CODE   => binary_decode_byte_tuple! { B4, u8, 4, buf },
            Self::B5_CODE   => binary_decode_byte_tuple! { B5, u8, 5, buf },
            Self::B6_CODE   => binary_decode_byte_tuple! { B6, u8, 6, buf },
            Self::B7_CODE   => binary_decode_byte_tuple! { B7, u8, 7, buf },
            Self::B8_CODE   => binary_decode_byte_tuple! { B8, u8, 8, buf },
            Self::B9_CODE   => binary_decode_byte_tuple! { B9, u8, 9, buf },
            Self::B10_CODE  => binary_decode_byte_tuple! { B10, u8, 10, buf },
            Self::B16_CODE  => binary_decode_byte_tuple! { B16, u8, 16, buf },
            Self::B32_CODE  => binary_decode_byte_tuple! { B32, B32, u8, 32, buf },
            //      MSB                         LSB
            // +---+---+---+---+---+---+---+---+---+
            // | c |   |   |   |   |   |   |   |   |
            // +---+---+---+---+---+---+---+---+---+
            //     \_______________________________/
            //                     |
            //                  32 bytes
            //
            // Fixed length numbers
            Self::TUP2_U16_CODE     => binary_decode_byte_tuple! { Tup2u16, u16, 2, buf },
            Self::TUP3_U16_CODE     => binary_decode_byte_tuple! { Tup3u16, u16, 3, buf },
            Self::TUP4_U16_CODE     => binary_decode_byte_tuple! { Tup4u16, u16, 4, buf },
            Self::TUP5_U16_CODE     => binary_decode_byte_tuple! { Tup5u16, u16, 5, buf },
            Self::TUP6_U16_CODE     => binary_decode_byte_tuple! { Tup6u16, u16, 6, buf },
            Self::TUP7_U16_CODE     => binary_decode_byte_tuple! { Tup7u16, u16, 7, buf },
            Self::TUP8_U16_CODE     => binary_decode_byte_tuple! { Tup8u16, u16, 8, buf },
            Self::TUP9_U16_CODE     => binary_decode_byte_tuple! { Tup9u16, u16, 9, buf },
            Self::TUP10_U16_CODE    => binary_decode_byte_tuple! { Tup10u16, u16, 10, buf },

            Self::TUP2_U32_CODE     => binary_decode_byte_tuple! { Tup2u32, u32, 2, buf },
            Self::TUP3_U32_CODE     => binary_decode_byte_tuple! { Tup3u32, u32, 3, buf },
            Self::TUP4_U32_CODE     => binary_decode_byte_tuple! { Tup4u32, u32, 4, buf },
            Self::TUP5_U32_CODE     => binary_decode_byte_tuple! { Tup5u32, u32, 5, buf },
            Self::TUP6_U32_CODE     => binary_decode_byte_tuple! { Tup6u32, u32, 6, buf },
            Self::TUP7_U32_CODE     => binary_decode_byte_tuple! { Tup7u32, u32, 7, buf },
            Self::TUP8_U32_CODE     => binary_decode_byte_tuple! { Tup8u32, u32, 8, buf },
            Self::TUP9_U32_CODE     => binary_decode_byte_tuple! { Tup9u32, u32, 9, buf },
            Self::TUP10_U32_CODE    => binary_decode_byte_tuple! { Tup10u32, u32, 10, buf },

            Self::TUP2_U64_CODE     => binary_decode_byte_tuple! { Tup2u64, u64, 2, buf },
            Self::TUP3_U64_CODE     => binary_decode_byte_tuple! { Tup3u64, u64, 3, buf },
            Self::TUP4_U64_CODE     => binary_decode_byte_tuple! { Tup4u64, u64, 4, buf },
            Self::TUP5_U64_CODE     => binary_decode_byte_tuple! { Tup5u64, u64, 5, buf },
            Self::TUP6_U64_CODE     => binary_decode_byte_tuple! { Tup6u64, u64, 6, buf },
            Self::TUP7_U64_CODE     => binary_decode_byte_tuple! { Tup7u64, u64, 7, buf },
            Self::TUP8_U64_CODE     => binary_decode_byte_tuple! { Tup8u64, u64, 8, buf },
            Self::TUP9_U64_CODE     => binary_decode_byte_tuple! { Tup9u64, u64, 9, buf },
            Self::TUP10_U64_CODE    => binary_decode_byte_tuple! { Tup10u64, u64, 10, buf },

            code => return Err(err!(
                "Byte code 0x{:02x} not implemented.", code;
            Unimplemented, Input)),
        }
    }
}

impl Dat {

    /// Read a `u64` from bytes in the given buffer, and include the number of bytes read in the
    /// return tuple.  The code prefix must be included in the buffer, but it is assumed that it has
    /// already been verified as within the correct range.
    pub fn read_c64(buf: &[u8]) -> Outcome<(u64, usize)> {
        let len = (buf[0] - Dat::C64_CODE_START) as usize;
        if buf.len() < len + 1 {
            return Err(err!(
                "Not enough bytes to decode the Dat::C64.";
            Bytes, Input, Decode, Missing));
        }
        let mut byts = [0_u8; 8];
        let offset = 8-len;
        for i in 0..len {
            byts[offset+i] = buf[i+1];
        }
        return Ok((
            u64::from_be_bytes(byts),
            len+1,
        ));
    }

    /// Assumes the `Dat` code has been removed.
    pub fn read_c64_as_usize(buf: &[u8]) -> usize {
        let len = buf.len();
        let mut byts = [0_u8; 8];
        let offset = 8-len;
        for i in 0..len {
            byts[offset+i] = buf[i];
        }
        u64::from_be_bytes(byts) as usize
    }

}
