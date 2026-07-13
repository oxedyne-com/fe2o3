use crate::{
    prelude::*,
    usr::UsrKindId,
};

use oxedyne_fe2o3_core::prelude::*;

use std::{
    convert::TryFrom,
    io,
};


impl Dat {

    /// Used to read the `Dat::C64` bytes into the provided `byts` buffer, returning the
    /// actual value of the `C64` as a `usize`.
    fn load_c64<R: io::Read>(
        r:      &mut R,
        code:   u8,
        byts:   &mut Vec<u8>,
    )
        -> Outcome<usize>
    {
        match code - Self::C64_CODE_START {
            1   => {
                let mut v = [0;1];
                res!(r.read_exact(&mut v));
                byts.extend_from_slice(&v[..]);
                Ok(Self::read_c64_as_usize(&v))
            },
            2   => {
                let mut v = [0;2];
                res!(r.read_exact(&mut v));
                byts.extend_from_slice(&v[..]);
                Ok(Self::read_c64_as_usize(&v))
            },
            3   => {
                let mut v = [0;3];
                res!(r.read_exact(&mut v));
                byts.extend_from_slice(&v[..]);
                Ok(Self::read_c64_as_usize(&v))
            },
            4   => {
                let mut v = [0;4];
                res!(r.read_exact(&mut v));
                byts.extend_from_slice(&v[..]);
                Ok(Self::read_c64_as_usize(&v))
            },
            5   => {
                let mut v = [0;5];
                res!(r.read_exact(&mut v));
                byts.extend_from_slice(&v[..]);
                Ok(Self::read_c64_as_usize(&v))
            },
            6   => {
                let mut v = [0;6];
                res!(r.read_exact(&mut v));
                byts.extend_from_slice(&v[..]);
                Ok(Self::read_c64_as_usize(&v))
            },
            7   => {
                let mut v = [0;7];
                res!(r.read_exact(&mut v));
                byts.extend_from_slice(&v[..]);
                Ok(Self::read_c64_as_usize(&v))
            },
            8   => {
                let mut v = [0;8];
                res!(r.read_exact(&mut v));
                byts.extend_from_slice(&v[..]);
                Ok(Self::read_c64_as_usize(&v))
            },
            _ => unreachable!(),
        }
    }

    pub fn load_bytes<R: io::Read>(r: &mut R) -> Outcome<Vec<u8>> {
        let mut byts = Vec::new();
        res!(Self::load_bytes_muncher(r, &mut byts));
        Ok(byts)
    }

    /// This method returns the raw bytes for an encoded `Dat`.
    pub fn load_bytes_muncher<R: io::Read>(
        mut r: &mut R,
        mut byts: &mut Vec<u8>,
    )
        -> Outcome<()>
    {
        let mut dcode = [0;1];
        match r.read_exact(&mut dcode) {
            Err(e) => match e.kind() {
                std::io::ErrorKind::UnexpectedEof => return Ok(()),
                _ => return Err(err!(e,
                    "While trying to read Dat code from reader.";
                Decode, Bytes)),
            },
            Ok(()) => (),
        }
        byts.push(dcode[0]);
        match dcode[0] {
            // Atomic Kinds ===========================
            // Logic
            Self::EMPTY_CODE    |
            Self::TRUE_CODE     |
            Self::FALSE_CODE    |
            Self::OPT_NONE_CODE => return Ok(()),
            // Fixed
            Self::U8_CODE => {
                // +---+---+
                // | c |   |
                // +---+---+
                //     \___/
                //       |
                //       u8
                //
                let mut v = [0;1];
                res!(r.read_exact(&mut v));
                byts.extend_from_slice(&v[..]);
                return Ok(());
            },
            Self::U16_CODE => {
                //      MSB LSB
                // +---+---+---+
                // | c |   |   |
                // +---+---+---+
                //     \_______/
                //         |
                //        u16
                //
                let mut v = [0;2];
                res!(r.read_exact(&mut v));
                byts.extend_from_slice(&v[..]);
                return Ok(());
            },
            Self::U32_CODE => {
                //      MSB         LSB
                // +---+---+---+---+---+
                // | c |   |   |   |   |
                // +---+---+---+---+---+
                //     \_______________/
                //             |
                //            u32
                //
                let mut v = [0;4];
                res!(r.read_exact(&mut v));
                byts.extend_from_slice(&v[..]);
                return Ok(());
            },
            Self::U64_CODE => {
                //      MSB                         LSB
                // +---+---+---+---+---+---+---+---+---+
                // | c |   |   |   |   |   |   |   |   |
                // +---+---+---+---+---+---+---+---+---+
                //     \_______________________________/
                //                     |
                //                    u64
                //
                let mut v = [0;8];
                res!(r.read_exact(&mut v));
                byts.extend_from_slice(&v[..]);
                return Ok(());
            },
            Self::U128_CODE => {
                //      MSB                         LSB
                // +---+---+---+---+---+---+---+---+---+
                // | c |   |   |   |   |   |   |   |   |
                // +---+---+---+---+---+---+---+---+---+
                //     \_______________________________/
                //                     |
                //                    u128
                //
                let mut v = [0;16];
                res!(r.read_exact(&mut v));
                byts.extend_from_slice(&v[..]);
                return Ok(());
            },
            Self::I8_CODE => {
                // +---+---+
                // | c |   |
                // +---+---+
                //     \___/
                //       |
                //       i8
                //
                let mut v = [0;1];
                res!(r.read_exact(&mut v));
                byts.extend_from_slice(&v[..]);
                return Ok(());
            },
            Self::I16_CODE => {
                //      MSB LSB
                // +---+---+---+
                // | c |   |   |
                // +---+---+---+
                //     \_______/
                //         |
                //        i16
                //
                let mut v = [0;2];
                res!(r.read_exact(&mut v));
                byts.extend_from_slice(&v[..]);
                return Ok(());
            },
            Self::I32_CODE => {
                //      MSB         LSB
                // +---+---+---+---+---+
                // | c |   |   |   |   |
                // +---+---+---+---+---+
                //     \_______________/
                //             |
                //            i32
                //
                let mut v = [0;4];
                res!(r.read_exact(&mut v));
                byts.extend_from_slice(&v[..]);
                return Ok(());
            },
            Self::I64_CODE => {
                //      MSB                         LSB
                // +---+---+---+---+---+---+---+---+---+
                // | c |   |   |   |   |   |   |   |   |
                // +---+---+---+---+---+---+---+---+---+
                //     \_______________________________/
                //                     |
                //                    i64
                //
                let mut v = [0;8];
                res!(r.read_exact(&mut v));
                byts.extend_from_slice(&v[..]);
                return Ok(());
            },
            Self::I128_CODE => {
                //      MSB                         LSB
                // +---+---+---+---+---+---+---+---+---+
                // | c |   |   |   |   |   |   |   |   |
                // +---+---+---+---+---+---+---+---+---+
                //     \_______________________________/
                //                     |
                //                    i128
                //
                let mut v = [0;16];
                res!(r.read_exact(&mut v));
                byts.extend_from_slice(&v[..]);
                return Ok(());
            },
            Self::F32_CODE => {
                //      MSB         LSB
                // +---+---+---+---+---+
                // | c |   |   |   |   |
                // +---+---+---+---+---+
                //     \_______________/
                //             |
                //            f32
                //
                let mut v = [0;4];
                res!(r.read_exact(&mut v));
                byts.extend_from_slice(&v[..]);
                return Ok(());
            },
            Self::F64_CODE => {
                //      MSB                         LSB
                // +---+---+---+---+---+---+---+---+---+
                // | c |   |   |   |   |   |   |   |   |
                // +---+---+---+---+---+---+---+---+---+
                //     \_______________________________/
                //                     |
                //                    f64
                //
                let mut v = [0;8];
                res!(r.read_exact(&mut v));
                byts.extend_from_slice(&v[..]);
                return Ok(());
            },
            // Variable
            Self::BC64_CODE |
            Self::STR_CODE  |
            Self::AINT_CODE |
            Self::ADEC_CODE |
            Self::LIST_CODE |
            Self::VEK_CODE  |
            Self::MAP_CODE  |
            Self::OMAP_CODE => {
                //
                //   0   1   2  ...  n   1   2  ...  v
                // +---+---+---+---+---+---+---+---+---+
                // | c |   |   |   |   |   |  ...  |   |
                // +---+---+---+---+---+---+---+---+---+
                //      \______________/\______________/
                //              |               |
                //             c64       payload bytes
                //
                let mut c64code = [0;1];
                res!(r.read_exact(&mut c64code));
                byts.extend_from_slice(&c64code[..]);
                if c64code[0] < Self::C64_CODE_START || c64code[0] > Self::C64_CODE_START + 8 {
                    return Err(err!(
                        "Expected a valid Dat::C64 code between {} and {} inclusive, \
                        instead found {}.", Self::C64_CODE_START, Self::C64_CODE_START + 8,
                        c64code[0];
                    Invalid, Input, Decode, Bytes));
                }
                if c64code[0] == Self::C64_CODE_START {
                    return Ok(());
                }
                // Avoid the use of the heap for the C64, for what it's worth.
                let vlen = res!(Self::load_c64(&mut r, c64code[0], &mut byts));
                let mut v = vec![0;vlen];
                res!(r.read_exact(&mut v));
                byts.extend_from_slice(&v[..]);
                return Ok(())
            },
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
                if dcode[0] < Self::C64_CODE_START || dcode[0] > Self::C64_CODE_START + 8 {
                    return Err(err!(
                        "Expected a valid Dat::C64 code between {} and {} inclusive, \
                        instead found {}.", Self::C64_CODE_START, Self::C64_CODE_START + 8,
                        dcode[0];
                    Invalid, Input, Decode, Bytes));
                }
                if dcode[0] == Self::C64_CODE_START {
                    return Ok(());
                }
                // Avoid the use of the heap for the C64, for what it's worth.
                match dcode[0] - Self::C64_CODE_START {
                    1   => {
                        let mut v = [0;1];
                        res!(r.read_exact(&mut v));
                        byts.extend_from_slice(&v[..]);
                    },
                    2   => {
                        let mut v = [0;2];
                        res!(r.read_exact(&mut v));
                        byts.extend_from_slice(&v[..]);
                    },
                    3   => {
                        let mut v = [0;3];
                        res!(r.read_exact(&mut v));
                        byts.extend_from_slice(&v[..]);
                    },
                    4   => {
                        let mut v = [0;4];
                        res!(r.read_exact(&mut v));
                        byts.extend_from_slice(&v[..]);
                    },
                    5   => {
                        let mut v = [0;5];
                        res!(r.read_exact(&mut v));
                        byts.extend_from_slice(&v[..]);
                    },
                    6   => {
                        let mut v = [0;6];
                        res!(r.read_exact(&mut v));
                        byts.extend_from_slice(&v[..]);
                    },
                    7   => {
                        let mut v = [0;7];
                        res!(r.read_exact(&mut v));
                        byts.extend_from_slice(&v[..]);
                    },
                    8   => {
                        let mut v = [0;8];
                        res!(r.read_exact(&mut v));
                        byts.extend_from_slice(&v[..]);
                    },
                    _ => unreachable!(),
                }
                return Ok(());
            },
            // Molecule Kinds =========================
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
                let mut v = [0; CODE_LEN];
                res!(r.read_exact(&mut v));
                byts.extend_from_slice(&v[..]);
                let mut v = [0; 1];
                res!(r.read_exact(&mut v));
                let opt_code = v[0];
                byts.push(opt_code);
                if opt_code == Self::OPT_SOME_CODE {
                    res!(Self::load_bytes_muncher(r, byts));
                }
                return Ok(());
            },
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
                res!(Self::load_bytes_muncher(r, byts));
                return Ok(());
            },
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
                res!(Self::load_bytes_muncher(r, byts));
                return Ok(());
            },
            // Heterogenous
            Self::TUP2_CODE    |
            Self::TUP3_CODE    |
            Self::TUP4_CODE    |
            Self::TUP5_CODE    |
            Self::TUP6_CODE    |
            Self::TUP7_CODE    |
            Self::TUP8_CODE    |
            Self::TUP9_CODE    |
            Self::TUP10_CODE    => {
                //
                //   0   1  ...
                // +---+---+---+---+---+---+---+---+---+---+---+---+---+---+
                // | c | c |   |   | c |   |   | c |   |   |   |  ...  |   |
                // +---+---+---+---+---+---+---+---+---+---+---+---+---+---+
                //      \__________/\__________/\______________/    ...
                //            |          |            |
                //           item       item         item
                //
                let n = dcode[0] - Self::TUP_SERIES_START + 2;
                for _ in 0..n {
                    res!(Self::load_bytes_muncher(r, byts));
                }
                return Ok(())
            },
            // Homogenous
            // Variable length bytes
            Self::BU8_CODE => {
                //
                //   0   1   1   2  ...  v
                // +---+---+---+---+---+---+  Fixed size, raw u8 for payload length
                // | c |   |   |  ...  |   |
                // +---+---+---+---+---+---+
                //      \__/\______________/
                //       |         |
                //    raw u8  payload bytes
                //
                let mut v = [0;1];
                res!(r.read_exact(&mut v));
                byts.extend_from_slice(&v[..]);
                let vlen = u8::from_be_bytes(
                    res!(<[u8; 1]>::try_from(&v[..]), Decode, Bytes)
                ) as usize;
                let mut v = vec![0;vlen];
                res!(r.read_exact(&mut v));
                byts.extend_from_slice(&v[..]);
                return Ok(())
            },
            Self::BU16_CODE => {
                //
                //   0   1   2   1   2  ...  v
                // +---+---+---+---+---+---+---+  Fixed size, raw u16 for payload length
                // | c |   |   |   |  ...  |   |
                // +---+---+---+---+---+---+---+
                //      \______/\______________/
                //         |           |
                //      raw u16    payload bytes
                //
                let mut v = [0;2];
                res!(r.read_exact(&mut v));
                byts.extend_from_slice(&v[..]);
                let vlen = u16::from_be_bytes(
                    res!(<[u8; 2]>::try_from(&v[..]), Decode, Bytes)
                ) as usize;
                let mut v = vec![0;vlen];
                res!(r.read_exact(&mut v));
                byts.extend_from_slice(&v[..]);
                return Ok(())
            },
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
                let mut v = [0;4];
                res!(r.read_exact(&mut v));
                byts.extend_from_slice(&v[..]);
                let vlen = u32::from_be_bytes(
                    res!(<[u8; 4]>::try_from(&v[..]), Decode, Bytes)
                ) as usize;
                let mut v = vec![0;vlen];
                res!(r.read_exact(&mut v));
                byts.extend_from_slice(&v[..]);
                return Ok(())
            },
            Self::BU64_CODE => {
                //
                //   0   1   2  ...  8   1   2  ...  v
                // +---+---+---+---+---+---+---+---+---+  Fixed size, raw u64 for payload length
                // | c |   |   |   |   |   |  ...  |   |
                // +---+---+---+---+---+---+---+---+---+
                //      \______________/\______________/
                //              |               |
                //           raw u64      payload bytes
                //
                let mut v = [0;8];
                res!(r.read_exact(&mut v));
                byts.extend_from_slice(&v[..]);
                let vlen = u64::from_be_bytes(
                    res!(<[u8; 8]>::try_from(&v[..]), Decode, Bytes)
                ) as usize;
                let mut v = vec![0;vlen];
                res!(r.read_exact(&mut v));
                byts.extend_from_slice(&v[..]);
                return Ok(())
            },
            // Fixed length bytes
            Self::B2_CODE   => binary_load_byte_tuple! { B2, u8, 2, r, byts },
            Self::B3_CODE   => binary_load_byte_tuple! { B3, u8, 3, r, byts },
            Self::B4_CODE   => binary_load_byte_tuple! { B4, u8, 4, r, byts },
            Self::B5_CODE   => binary_load_byte_tuple! { B5, u8, 5, r, byts },
            Self::B6_CODE   => binary_load_byte_tuple! { B6, u8, 6, r, byts },
            Self::B7_CODE   => binary_load_byte_tuple! { B7, u8, 7, r, byts },
            Self::B8_CODE   => binary_load_byte_tuple! { B8, u8, 8, r, byts },
            Self::B9_CODE   => binary_load_byte_tuple! { B9, u8, 9, r, byts },
            Self::B10_CODE  => binary_load_byte_tuple! { B10, u8, 10, r, byts },
            Self::B16_CODE  => binary_load_byte_tuple! { B16, u8, 16, r, byts },
            Self::B32_CODE  => binary_load_byte_tuple! { B32, u8, 32, r, byts },
            // Fixed length numbers
            Self::TUP2_U16_CODE     => binary_load_byte_tuple! { Tup2u16, u16, 2, r, byts },
            Self::TUP3_U16_CODE     => binary_load_byte_tuple! { Tup3u16, u16, 3, r, byts },
            Self::TUP4_U16_CODE     => binary_load_byte_tuple! { Tup4u16, u16, 4, r, byts },
            Self::TUP5_U16_CODE     => binary_load_byte_tuple! { Tup5u16, u16, 5, r, byts },
            Self::TUP6_U16_CODE     => binary_load_byte_tuple! { Tup6u16, u16, 6, r, byts },
            Self::TUP7_U16_CODE     => binary_load_byte_tuple! { Tup7u16, u16, 7, r, byts },
            Self::TUP8_U16_CODE     => binary_load_byte_tuple! { Tup8u16, u16, 8, r, byts },
            Self::TUP9_U16_CODE     => binary_load_byte_tuple! { Tup9u16, u16, 9, r, byts },
            Self::TUP10_U16_CODE    => binary_load_byte_tuple! { Tup10u16, u16, 10, r, byts },

            Self::TUP2_U32_CODE     => binary_load_byte_tuple! { Tup2u32, u32, 2, r, byts },
            Self::TUP3_U32_CODE     => binary_load_byte_tuple! { Tup3u32, u32, 3, r, byts },
            Self::TUP4_U32_CODE     => binary_load_byte_tuple! { Tup4u32, u32, 4, r, byts },
            Self::TUP5_U32_CODE     => binary_load_byte_tuple! { Tup5u32, u32, 5, r, byts },
            Self::TUP6_U32_CODE     => binary_load_byte_tuple! { Tup6u32, u32, 6, r, byts },
            Self::TUP7_U32_CODE     => binary_load_byte_tuple! { Tup7u32, u32, 7, r, byts },
            Self::TUP8_U32_CODE     => binary_load_byte_tuple! { Tup8u32, u32, 8, r, byts },
            Self::TUP9_U32_CODE     => binary_load_byte_tuple! { Tup9u32, u32, 9, r, byts },
            Self::TUP10_U32_CODE    => binary_load_byte_tuple! { Tup10u32, u32, 10, r, byts },

            Self::TUP2_U64_CODE     => binary_load_byte_tuple! { Tup2u64, u64, 2, r, byts },
            Self::TUP3_U64_CODE     => binary_load_byte_tuple! { Tup3u64, u64, 3, r, byts },
            Self::TUP4_U64_CODE     => binary_load_byte_tuple! { Tup4u64, u64, 4, r, byts },
            Self::TUP5_U64_CODE     => binary_load_byte_tuple! { Tup5u64, u64, 5, r, byts },
            Self::TUP6_U64_CODE     => binary_load_byte_tuple! { Tup6u64, u64, 6, r, byts },
            Self::TUP7_U64_CODE     => binary_load_byte_tuple! { Tup7u64, u64, 7, r, byts },
            Self::TUP8_U64_CODE     => binary_load_byte_tuple! { Tup8u64, u64, 8, r, byts },
            Self::TUP9_U64_CODE     => binary_load_byte_tuple! { Tup9u64, u64, 9, r, byts },
            Self::TUP10_U64_CODE    => binary_load_byte_tuple! { Tup10u64, u64, 10, r, byts },

            code => return Err(err!(
                "Dat identification code {} not recognised.", code;
            Invalid, Input)),
        }
    }
}
