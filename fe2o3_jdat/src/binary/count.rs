use crate::{
    prelude::*,
    usr::UsrKindId,
};

use oxedize_fe2o3_core::prelude::*;

use std::{
    convert::TryFrom,
    io::{
        self,
        SeekFrom,
    },
};


impl Dat {

    fn count_c64<R: io::Read>(r: &mut R, code: u8) -> Outcome<(usize, usize)> {
        match code - Self::C64_CODE_START {
            1   => {
                let mut v = [0;1];
                res!(r.read_exact(&mut v));
                Ok((Self::read_c64_as_usize(&v), 1))
            },
            2   => {
                let mut v = [0;2];
                res!(r.read_exact(&mut v));
                Ok((Self::read_c64_as_usize(&v), 2))
            },
            3   => {
                let mut v = [0;3];
                res!(r.read_exact(&mut v));
                Ok((Self::read_c64_as_usize(&v), 3))
            },
            4   => {
                let mut v = [0;4];
                res!(r.read_exact(&mut v));
                Ok((Self::read_c64_as_usize(&v), 4))
            },
            5   => {
                let mut v = [0;5];
                res!(r.read_exact(&mut v));
                Ok((Self::read_c64_as_usize(&v), 5))
            },
            6   => {
                let mut v = [0;6];
                res!(r.read_exact(&mut v));
                Ok((Self::read_c64_as_usize(&v), 6))
            },
            7   => {
                let mut v = [0;7];
                res!(r.read_exact(&mut v));
                Ok((Self::read_c64_as_usize(&v), 7))
            },
            8   => {
                let mut v = [0;8];
                res!(r.read_exact(&mut v));
                Ok((Self::read_c64_as_usize(&v), 8))
            },
            _ => unreachable!(),
        }
    }

    pub fn count_bytes<RS: io::Read + io::Seek>(rs: &mut RS) -> Outcome<usize> {
        let mut count: usize = 0;
        res!(Self::count_bytes_muncher(rs, &mut count));
        Ok(count)
    }

    /// This method counts the raw bytes for an encoded `Dat`, whilst also moving the reader
    /// cursor position to the end of the `Dat`.
    pub fn count_bytes_muncher<RS: io::Read + io::Seek>(
        mut rs: &mut RS,
        count:  &mut usize,
    )
        -> Outcome<()>
    {
        let mut dcode = [0; 1];
        match rs.read_exact(&mut dcode) {
            Err(e) => match e.kind() {
                std::io::ErrorKind::UnexpectedEof => return Ok(()),
                _ => return Err(err!(e, errmsg!(
                    "While trying to read Dat code from reader.",
                ), Decode, Bytes)),
            },
            Ok(()) => (),
        }
        *count += 1;
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
                res!(rs.seek(SeekFrom::Current(1)));
                *count += 1;
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
                res!(rs.seek(SeekFrom::Current(2)));
                *count += 2;
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
                res!(rs.seek(SeekFrom::Current(4)));
                *count += 4;
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
                res!(rs.seek(SeekFrom::Current(8)));
                *count += 8;
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
                res!(rs.seek(SeekFrom::Current(16)));
                *count += 16;
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
                res!(rs.seek(SeekFrom::Current(1)));
                *count += 1;
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
                res!(rs.seek(SeekFrom::Current(2)));
                *count += 2;
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
                res!(rs.seek(SeekFrom::Current(4)));
                *count += 4;
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
                res!(rs.seek(SeekFrom::Current(8)));
                *count += 8;
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
                res!(rs.seek(SeekFrom::Current(16)));
                *count += 16;
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
                res!(rs.seek(SeekFrom::Current(4)));
                *count += 4;
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
                res!(rs.seek(SeekFrom::Current(8)));
                *count += 8;
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
                res!(rs.read_exact(&mut c64code));
                *count += 1;
                if c64code[0] < Self::C64_CODE_START || c64code[0] > Self::C64_CODE_START + 8 {
                    return Err(err!(errmsg!(
                        "Expected a valid Dat::C64 code between {} and {} inclusive, \
                        instead found {}.", Self::C64_CODE_START, Self::C64_CODE_START + 8,
                        c64code[0],
                    ), Invalid, Input, Decode, Bytes));
                }
                if c64code[0] == Self::C64_CODE_START {
                    return Ok(());
                }
                // Avoid the use of the heap for the C64, for what it's worth.
                let (vlen, c64len) = res!(Self::count_c64(&mut rs, c64code[0]));
                *count += c64len + vlen;
                if vlen > i64::MAX as usize {
                    return Err(err!(errmsg!(
                        "The size of the payload (the value contained in the Dat::C64), \
                        {} bytes, exceeds the maximum increment that can be added to the \
                        reader cursor position of {}.", vlen, i64::MAX,
                    ), Invalid, Input, Decode, Bytes));
                }
                res!(rs.seek(SeekFrom::Current(vlen as i64)));
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
                    return Err(err!(errmsg!(
                        "Expected a valid Dat::C64 code between {} and {} inclusive, \
                        instead found {}.", Self::C64_CODE_START, Self::C64_CODE_START + 8,
                        dcode[0],
                    ), Invalid, Input, Decode, Bytes));
                }
                if dcode[0] == Self::C64_CODE_START {
                    return Ok(());
                }
                let c64len = dcode[0] - Self::C64_CODE_START;
                *count += c64len as usize;
                res!(rs.seek(SeekFrom::Current(c64len as i64)));
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
                let code_len = try_into!(i64, CODE_LEN);
                res!(rs.seek(SeekFrom::Current(code_len)));
                *count += CODE_LEN;
                let mut v = [0; 1];
                res!(rs.read_exact(&mut v));
                *count += 1;
                let opt_code = v[0];
                if opt_code == Self::OPT_SOME_CODE {
                    res!(Self::count_bytes_muncher(rs, count));
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
                res!(Self::count_bytes_muncher(rs, count));
                return Ok(())
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
                res!(Self::count_bytes_muncher(rs, count));
                return Ok(());
            },
            // Heterogenous
            Self::TUP2_CODE     |
            Self::TUP3_CODE     |
            Self::TUP4_CODE     |
            Self::TUP5_CODE     |
            Self::TUP6_CODE     |
            Self::TUP7_CODE     |
            Self::TUP8_CODE     |
            Self::TUP9_CODE     |
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
                    res!(Self::count_bytes_muncher(rs, count));
                }
                return Ok(())
            },
            // Homogenous
            // Variable length bytes
            Self::BU8_CODE  => {
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
                res!(rs.read_exact(&mut v));
                let vlen = u8::from_be_bytes(
                    res!(<[u8; 1]>::try_from(&v[..]), Decode, Bytes)
                ) as usize;
                *count += 1 + vlen;
                res!(rs.seek(SeekFrom::Current(vlen as i64)));
                return Ok(())
            },
            Self::BU16_CODE  => {
                //
                //   0   1   2   1   2  ...  v
                // +---+---+---+---+---+---+---+  Fixed size, raw u16 for payload length
                // | c |   |   |   |  ...  |   |
                // +---+---+---+---+---+---+---+
                //      \______/\______________/
                //         |            |
                //      raw u16    payload bytes
                //
                let mut v = [0;2];
                res!(rs.read_exact(&mut v));
                let vlen = u16::from_be_bytes(
                    res!(<[u8; 2]>::try_from(&v[..]), Decode, Bytes)
                ) as usize;
                *count += 2 + vlen;
                res!(rs.seek(SeekFrom::Current(vlen as i64)));
                return Ok(())
            },
            Self::BU32_CODE  => {
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
                res!(rs.read_exact(&mut v));
                let vlen = u32::from_be_bytes(
                    res!(<[u8; 4]>::try_from(&v[..]), Decode, Bytes)
                ) as usize;
                *count += 4 + vlen;
                res!(rs.seek(SeekFrom::Current(vlen as i64)));
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
                res!(rs.read_exact(&mut v));
                let vlen = u64::from_be_bytes(
                    res!(<[u8; 8]>::try_from(&v[..]), Decode, Bytes)
                ) as usize;
                *count += 8 + vlen;
                if vlen > i64::MAX as usize {
                    return Err(err!(errmsg!(
                        "The size of the payload (the value contained in the u64), \
                        {} bytes, exceeds the maximum increment that can be added to the \
                        reader cursor position of {}.", vlen, i64::MAX,
                    ), Invalid, Input, Decode, Bytes));
                }
                res!(rs.seek(SeekFrom::Current(vlen as i64)));
                return Ok(())
            },
            // Fixed length bytes
            Self::B2_CODE   => binary_count_byte_tuple! { u8,  2, rs, count },
            Self::B3_CODE   => binary_count_byte_tuple! { u8,  3, rs, count },
            Self::B4_CODE   => binary_count_byte_tuple! { u8,  4, rs, count },
            Self::B5_CODE   => binary_count_byte_tuple! { u8,  5, rs, count },
            Self::B6_CODE   => binary_count_byte_tuple! { u8,  6, rs, count },
            Self::B7_CODE   => binary_count_byte_tuple! { u8,  7, rs, count },
            Self::B8_CODE   => binary_count_byte_tuple! { u8,  8, rs, count },
            Self::B9_CODE   => binary_count_byte_tuple! { u8,  9, rs, count },
            Self::B10_CODE  => binary_count_byte_tuple! { u8, 10, rs, count },
            Self::B16_CODE  => binary_count_byte_tuple! { u8, 16, rs, count },
            Self::B32_CODE  => binary_count_byte_tuple! { u8, 32, rs, count },
            // Fixed length numbers
            Self::TUP2_U16_CODE     => binary_count_byte_tuple! { u16,  2, rs, count },
            Self::TUP3_U16_CODE     => binary_count_byte_tuple! { u16,  3, rs, count },
            Self::TUP4_U16_CODE     => binary_count_byte_tuple! { u16,  4, rs, count },
            Self::TUP5_U16_CODE     => binary_count_byte_tuple! { u16,  5, rs, count },
            Self::TUP6_U16_CODE     => binary_count_byte_tuple! { u16,  6, rs, count },
            Self::TUP7_U16_CODE     => binary_count_byte_tuple! { u16,  7, rs, count },
            Self::TUP8_U16_CODE     => binary_count_byte_tuple! { u16,  8, rs, count },
            Self::TUP9_U16_CODE     => binary_count_byte_tuple! { u16,  9, rs, count },
            Self::TUP10_U16_CODE    => binary_count_byte_tuple! { u16, 10, rs, count },

            Self::TUP2_U32_CODE     => binary_count_byte_tuple! { u32,  2, rs, count },
            Self::TUP3_U32_CODE     => binary_count_byte_tuple! { u32,  3, rs, count },
            Self::TUP4_U32_CODE     => binary_count_byte_tuple! { u32,  4, rs, count },
            Self::TUP5_U32_CODE     => binary_count_byte_tuple! { u32,  5, rs, count },
            Self::TUP6_U32_CODE     => binary_count_byte_tuple! { u32,  6, rs, count },
            Self::TUP7_U32_CODE     => binary_count_byte_tuple! { u32,  7, rs, count },
            Self::TUP8_U32_CODE     => binary_count_byte_tuple! { u32,  8, rs, count },
            Self::TUP9_U32_CODE     => binary_count_byte_tuple! { u32,  9, rs, count },
            Self::TUP10_U32_CODE    => binary_count_byte_tuple! { u32, 10, rs, count },

            Self::TUP2_U64_CODE     => binary_count_byte_tuple! { u64,  2, rs, count },
            Self::TUP3_U64_CODE     => binary_count_byte_tuple! { u64,  3, rs, count },
            Self::TUP4_U64_CODE     => binary_count_byte_tuple! { u64,  4, rs, count },
            Self::TUP5_U64_CODE     => binary_count_byte_tuple! { u64,  5, rs, count },
            Self::TUP6_U64_CODE     => binary_count_byte_tuple! { u64,  6, rs, count },
            Self::TUP7_U64_CODE     => binary_count_byte_tuple! { u64,  7, rs, count },
            Self::TUP8_U64_CODE     => binary_count_byte_tuple! { u64,  8, rs, count },
            Self::TUP9_U64_CODE     => binary_count_byte_tuple! { u64,  9, rs, count },
            Self::TUP10_U64_CODE    => binary_count_byte_tuple! { u64, 10, rs, count },

            code => return Err(err!(errmsg!(
                "Dat identification code {} not recognised.", code,
            ), Invalid, Input)),
        }
    }
}
