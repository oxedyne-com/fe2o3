use crate::prelude::*;

use oxedyne_fe2o3_core::{
    prelude::*,
    byte::ToBytes,
};
use oxedyne_fe2o3_num::float::{
    Float32,
    Float64,
};


impl ToBytes for Dat {

    /// Appends the encoded `Dat` to the given byte buffer.
    fn to_bytes(&self, mut buf: Vec<u8>) -> Outcome<Vec<u8>> {
        match self {
            // Atomic Kinds ===========================
            // Logic
            Self::Empty => {
                self.append_code(&mut buf);
            },
            Self::Bool(_b) => {
                self.append_code(&mut buf);
            },
            // Fixed
            Self::U8(n) => {
                self.append_code(&mut buf);
                buf.extend_from_slice(&n.to_be_bytes());
            },
            Self::U16(n) => {
                self.append_code(&mut buf);
                buf.extend_from_slice(&n.to_be_bytes());
            },
            Self::U32(n) => {
                self.append_code(&mut buf);
                buf.extend_from_slice(&n.to_be_bytes());
            },
            Self::U64(n) => {
                self.append_code(&mut buf);
                buf.extend_from_slice(&n.to_be_bytes());
            },
            Self::U128(n) => {
                self.append_code(&mut buf);
                buf.extend_from_slice(&n.to_be_bytes());
            },
            Self::I8(n) => {
                self.append_code(&mut buf);
                buf.extend_from_slice(&n.to_be_bytes());
            },
            Self::I16(n) => {
                self.append_code(&mut buf);
                buf.extend_from_slice(&n.to_be_bytes());
            },
            Self::I32(n) => {
                self.append_code(&mut buf);
                buf.extend_from_slice(&n.to_be_bytes());
            },
            Self::I64(n) => {
                self.append_code(&mut buf);
                buf.extend_from_slice(&n.to_be_bytes());
            },
            Self::I128(n) => {
                self.append_code(&mut buf);
                buf.extend_from_slice(&n.to_be_bytes());
            },
            Self::F32(Float32(f)) => {
                self.append_code(&mut buf);
                buf.extend_from_slice(&f.to_be_bytes());
            },
            Self::F64(Float64(f)) => {
                self.append_code(&mut buf);
                buf.extend_from_slice(&f.to_be_bytes());
            },
            // Variable
            Self::Aint(bigint) => {
                let v = bigint.to_signed_bytes_be();
                self.append_code(&mut buf);
                buf = res!(Dat::C64(v.len() as u64).to_bytes(buf));
                buf.extend_from_slice(&v);
            },
            Self::Adec(bigdec) => {
                let (bigint, expi64) = bigdec.as_bigint_and_exponent();
                let mut vbuf = Vec::new();
                let v = bigint.to_signed_bytes_be();
                vbuf.extend_from_slice(&v);
                vbuf.extend_from_slice(&expi64.to_be_bytes());
                self.append_code(&mut buf);
                buf = res!(Dat::C64((v.len() as u64) + 8).to_bytes(buf));
                buf.extend_from_slice(&vbuf);
            },
            Self::C64(n) => {
                let byts: [u8; 8] = n.to_be_bytes();
                let mut count: usize = 8;
                for byt in &byts[..] {
                    if *byt == 0 {
                        count -= 1;
                    } else {
                        break;
                    }
                }
                self.append_code(&mut buf);
                let last = buf.len() - 1 ;
                buf[last] += count as u8;
                while count > 0 {
                    buf.push(byts[8-count]);
                    count -= 1;
                }
            },
            Self::Str(s) => {
                self.append_code(&mut buf);
                let b = s.as_bytes();
                buf = res!(Dat::C64(b.len() as u64).to_bytes(buf));
                buf.extend_from_slice(b);
            },
            // Molecular Kinds ========================
            // Unitary
            Self::Usr(ukid, optboxd) => {
                self.append_code(&mut buf);
                buf.extend_from_slice(&ukid.code().to_be_bytes());
                match optboxd {
                    Some(boxd) => {
                        buf.push(Self::OPT_SOME_CODE);
                        buf = res!(boxd.to_bytes(buf));
                    },
                    None => buf.push(Self::OPT_NONE_CODE),
                }
            },
            Self::Opt(boxoptd) => {
                self.append_code(&mut buf);
                if let Some(d) = &**boxoptd {
                    buf = res!(d.to_bytes(buf));
                }
            },
            Self::Box(boxd) => {
                self.append_code(&mut buf);
                buf = res!(boxd.to_bytes(buf));
            },
            Self::ABox(ncfg, boxd, s) => {
                self.append_code(&mut buf);
                buf = res!(ncfg.to_bytes(buf));
                buf = res!(boxd.to_bytes(buf));
                let b = s.as_bytes();
                buf = res!(Dat::C64(b.len() as u64).to_bytes(buf));
                buf.extend_from_slice(b);
            },
            // Heterogenous
            Self::List(v) => {
                self.append_code(&mut buf);
                buf = res!(Self::vec_to_bytes(v, buf));
            },
            Self::Tup2(a) => {
                buf = res!(Self::tuple_to_bytes::<Dat, 2, {Self::TUP_SERIES_START}>(a, buf));
            },
            Self::Tup3(a) => {
                buf = res!(Self::tuple_to_bytes::<Dat, 3, {Self::TUP_SERIES_START}>(a, buf));
            },
            Self::Tup4(a) => {
                buf = res!(Self::tuple_to_bytes::<Dat, 4, {Self::TUP_SERIES_START}>(a, buf));
            },
            Self::Tup5(a) => {
                buf = res!(Self::tuple_to_bytes::<Dat, 5, {Self::TUP_SERIES_START}>(a, buf));
            },
            Self::Tup6(a) => {
                buf = res!(Self::tuple_to_bytes::<Dat, 6, {Self::TUP_SERIES_START}>(a, buf));
            },
            Self::Tup7(a) => {
                buf = res!(Self::tuple_to_bytes::<Dat, 7, {Self::TUP_SERIES_START}>(a, buf));
            },
            Self::Tup8(a) => {
                buf = res!(Self::tuple_to_bytes::<Dat, 8, {Self::TUP_SERIES_START}>(a, buf));
            },
            Self::Tup9(a) => {
                buf = res!(Self::tuple_to_bytes::<Dat, 9, {Self::TUP_SERIES_START}>(a, buf));
            },
            Self::Tup10(a) => {
                buf = res!(Self::tuple_to_bytes::<Dat, 10, {Self::TUP_SERIES_START}>(a, buf));
            },
            Self::Map(map) => {
                let mut buf2 = Vec::new();
                for (k, v) in map {
                    buf2 = res!(k.to_bytes(buf2));
                    buf2 = res!(v.to_bytes(buf2));
                }
                self.append_code(&mut buf);
                buf = res!(Dat::C64(buf2.len() as u64).to_bytes(buf));
                buf.extend_from_slice(&buf2);
            },
            Self::OrdMap(map) => {
                let mut buf2 = Vec::new();
                for (k, v) in map {
                    buf2 = res!(k.to_bytes(buf2));
                    buf2 = res!(v.to_bytes(buf2));
                }
                self.append_code(&mut buf);
                buf = res!(Dat::C64(buf2.len() as u64).to_bytes(buf));
                buf.extend_from_slice(&buf2);
            },
            // Homogenous
            Self::Vek(vek) => {
                self.append_code(&mut buf);
                buf = res!(Self::vec_to_bytes(&*vek, buf));
            },
            // Variable length bytes
            Self::BU8(v) => {
                self.append_code(&mut buf);
                // Use a u8 for the data length.
                buf.extend_from_slice(&(v.len() as u8).to_be_bytes());
                buf.extend_from_slice(v);
            },
            Self::BU16(v) => {
                self.append_code(&mut buf);
                // Use a u16 for the data length.
                buf.extend_from_slice(&(v.len() as u16).to_be_bytes());
                buf.extend_from_slice(v);
            },
            Self::BU32(v) => {
                self.append_code(&mut buf);
                // Use a u32 for the data length.
                buf.extend_from_slice(&(v.len() as u32).to_be_bytes());
                buf.extend_from_slice(v);
            },
            Self::BU64(v) => {
                self.append_code(&mut buf);
                // Use a u64 for the data length.
                buf.extend_from_slice(&(v.len() as u64).to_be_bytes());
                buf.extend_from_slice(v);
            },
            Self::BC64(v) => {
                self.append_code(&mut buf);
                // Use a variable length Dat::C64 for the data length.
                buf = res!(Dat::C64(v.len() as u64).to_bytes(buf));
                buf.extend_from_slice(v);
            },
            // Fixed length bytes
            Self::B2(a) => {
                self.append_code(&mut buf);
                buf.extend_from_slice(&a[..]);
            },
            Self::B3(a) => {
                self.append_code(&mut buf);
                buf.extend_from_slice(&a[..]);
            },
            Self::B4(a) => {
                self.append_code(&mut buf);
                buf.extend_from_slice(&a[..]);
            },
            Self::B5(a) => {
                self.append_code(&mut buf);
                buf.extend_from_slice(&a[..]);
            },
            Self::B6(a) => {
                self.append_code(&mut buf);
                buf.extend_from_slice(&a[..]);
            },
            Self::B7(a) => {
                self.append_code(&mut buf);
                buf.extend_from_slice(&a[..]);
            },
            Self::B8(a) => {
                self.append_code(&mut buf);
                buf.extend_from_slice(&a[..]);
            },
            Self::B9(a) => {
                self.append_code(&mut buf);
                buf.extend_from_slice(&a[..]);
            },
            Self::B10(a) => {
                self.append_code(&mut buf);
                buf.extend_from_slice(&a[..]);
            },
            Self::B16(a) => {
                self.append_code(&mut buf);
                buf.extend_from_slice(&a[..]);
            },
            Self::B32(a) => {
                self.append_code(&mut buf);
                buf.extend_from_slice(&a[..]);
            },
            // Fixed length numbers
            Self::Tup2u16(a) => {
                buf = res!(Self::tuple_to_bytes::<u16, 2, {Self::TUP_U16_SERIES_START}>(a, buf));
            },
            Self::Tup3u16(a) => {
                buf = res!(Self::tuple_to_bytes::<u16, 3, {Self::TUP_U16_SERIES_START}>(a, buf));
            },
            Self::Tup4u16(a) => {
                buf = res!(Self::tuple_to_bytes::<u16, 4, {Self::TUP_U16_SERIES_START}>(a, buf));
            },
            Self::Tup5u16(a) => {
                buf = res!(Self::tuple_to_bytes::<u16, 5, {Self::TUP_U16_SERIES_START}>(a, buf));
            },
            Self::Tup6u16(a) => {
                buf = res!(Self::tuple_to_bytes::<u16, 6, {Self::TUP_U16_SERIES_START}>(a, buf));
            },
            Self::Tup7u16(a) => {
                buf = res!(Self::tuple_to_bytes::<u16, 7, {Self::TUP_U16_SERIES_START}>(a, buf));
            },
            Self::Tup8u16(a) => {
                buf = res!(Self::tuple_to_bytes::<u16, 8, {Self::TUP_U16_SERIES_START}>(a, buf));
            },
            Self::Tup9u16(a) => {
                buf = res!(Self::tuple_to_bytes::<u16, 9, {Self::TUP_U16_SERIES_START}>(a, buf));
            },
            Self::Tup10u16(a) => {
                buf = res!(Self::tuple_to_bytes::<u16, 10, {Self::TUP_U16_SERIES_START}>(a, buf));
            },
            Self::Tup2u32(a) => {
                buf = res!(Self::tuple_to_bytes::<u32, 2, {Self::TUP_U32_SERIES_START}>(a, buf));
            },
            Self::Tup3u32(a) => {
                buf = res!(Self::tuple_to_bytes::<u32, 3, {Self::TUP_U32_SERIES_START}>(a, buf));
            },
            Self::Tup4u32(a) => {
                buf = res!(Self::tuple_to_bytes::<u32, 4, {Self::TUP_U32_SERIES_START}>(a, buf));
            },
            Self::Tup5u32(a) => {
                buf = res!(Self::tuple_to_bytes::<u32, 5, {Self::TUP_U32_SERIES_START}>(a, buf));
            },
            Self::Tup6u32(a) => {
                buf = res!(Self::tuple_to_bytes::<u32, 6, {Self::TUP_U32_SERIES_START}>(a, buf));
            },
            Self::Tup7u32(a) => {
                buf = res!(Self::tuple_to_bytes::<u32, 7, {Self::TUP_U32_SERIES_START}>(a, buf));
            },
            Self::Tup8u32(a) => {
                buf = res!(Self::tuple_to_bytes::<u32, 8, {Self::TUP_U32_SERIES_START}>(a, buf));
            },
            Self::Tup9u32(a) => {
                buf = res!(Self::tuple_to_bytes::<u32, 9, {Self::TUP_U32_SERIES_START}>(a, buf));
            },
            Self::Tup10u32(a) => {
                buf = res!(Self::tuple_to_bytes::<u32, 10, {Self::TUP_U32_SERIES_START}>(a, buf));
            },
            Self::Tup2u64(a) => {
                buf = res!(Self::tuple_to_bytes::<u64, 2, {Self::TUP_U64_SERIES_START}>(a, buf));
            },
            Self::Tup3u64(a) => {
                buf = res!(Self::tuple_to_bytes::<u64, 3, {Self::TUP_U64_SERIES_START}>(a, buf));
            },
            Self::Tup4u64(a) => {
                buf = res!(Self::tuple_to_bytes::<u64, 4, {Self::TUP_U64_SERIES_START}>(a, buf));
            },
            Self::Tup5u64(a) => {
                buf = res!(Self::tuple_to_bytes::<u64, 5, {Self::TUP_U64_SERIES_START}>(a, buf));
            },
            Self::Tup6u64(a) => {
                buf = res!(Self::tuple_to_bytes::<u64, 6, {Self::TUP_U64_SERIES_START}>(a, buf));
            },
            Self::Tup7u64(a) => {
                buf = res!(Self::tuple_to_bytes::<u64, 7, {Self::TUP_U64_SERIES_START}>(a, buf));
            },
            Self::Tup8u64(a) => {
                buf = res!(Self::tuple_to_bytes::<u64, 8, {Self::TUP_U64_SERIES_START}>(a, buf));
            },
            Self::Tup9u64(a) => {
                buf = res!(Self::tuple_to_bytes::<u64, 9, {Self::TUP_U64_SERIES_START}>(a, buf));
            },
            Self::Tup10u64(a) => {
                buf = res!(Self::tuple_to_bytes::<u64, 10, {Self::TUP_U64_SERIES_START}>(a, buf));
            },
        }
        Ok(buf)
    }
}
                      
impl Dat {

    /// Appends the kind code to the given byte buffer.
    fn append_code(&self, buf: &mut Vec<u8>) {
        match self {
            Self::Bool(b) => buf.push(if *b { Self::TRUE_CODE } else { Self::FALSE_CODE }),
            Self::Opt(boxoptd) => {
                match &**boxoptd {
                    None => buf.push(Self::OPT_NONE_CODE),
                    Some(_) => buf.push(Self::OPT_SOME_CODE),
                }
            },
            _ => buf.push(self.to_code()),
        }
    }
                      
    pub fn as_bytes(&self) -> Outcome<Vec<u8>> {
        self.to_bytes(Vec::new())
    }

    pub fn wrap_bytes_c64(mut byts: Vec<u8>) -> Outcome<Vec<u8>> {
        let mut pre = Vec::new();
        pre.push(Self::BC64_CODE);
        pre = res!(Dat::C64(byts.len() as u64).to_bytes(pre));
        pre.append(&mut byts);
        Ok(pre)
    }

    pub fn wrap_bytes_u64(mut byts: Vec<u8>) -> Vec<u8> {
        let mut pre = Vec::new();
        pre.push(Self::BU64_CODE);
        pre.extend_from_slice(&(byts.len() as u64).to_be_bytes());
        pre.append(&mut byts);
        pre
    }

    pub fn wrap_bytes_var(mut byts: Vec<u8>) -> Outcome<Vec<u8>> {
        let mut pre = Vec::new();
        let len = byts.len();
        if len < u8::MAX as usize {
            pre.push(Self::BU8_CODE);
            pre.extend_from_slice(&(byts.len() as u8).to_be_bytes());
        } else if len < u16::MAX as usize {
            pre.push(Self::BU16_CODE);
            pre.extend_from_slice(&(byts.len() as u16).to_be_bytes());
        } else if len < u32::MAX as usize {
            pre.push(Self::BU32_CODE);
            pre.extend_from_slice(&(byts.len() as u32).to_be_bytes());
        } else if len < u64::MAX as usize {
            pre.push(Self::BU64_CODE);
            pre.extend_from_slice(&(byts.len() as u64).to_be_bytes());
        } else {
            return Err(err!(
                "The byte length of {}, which exceeds the maximum u64::MAX = {},
                cannot be represented a variable size Dat byte wrapper.",
                len, u64::MAX;
            Size, TooBig));
        }
        pre.append(&mut byts);
        Ok(pre)
    }

    pub fn byte_wrapper_var_len(len: usize) -> Outcome<u8> {
        if len <= u8::MAX as usize { Ok(2) }
        else if len <= u16::MAX as usize { Ok(3) }
        else if len <= u32::MAX as usize { Ok(5) }
        else if len <= u64::MAX as usize { Ok(9) }
        else {
            Err(err!(
                "The byte length of {}, which exceeds the maximum u64::MAX = {},
                cannot be represented a variable size Dat byte wrapper.",
                len, u64::MAX;
            Size, TooBig))
        }
    }

    pub fn wrap_dat(byts: Vec<u8>) -> Self {
        let len = byts.len();
        if len < u8::MAX as usize {
            Dat::BU8(byts)
        } else if len < u16::MAX as usize {
            Dat::BU16(byts)
        } else if len < u32::MAX as usize {
            Dat::BU32(byts)
        } else {
            Dat::BU64(byts)
        }
    }

    pub fn vec_to_bytes(v: &Vec<Dat>, mut buf: Vec<u8>) -> Outcome<Vec<u8>> {
        let mut buf2 = Vec::new();
        for item in v {
            buf2 = res!(item.to_bytes(buf2));
        }
        buf = res!(Dat::C64(buf2.len() as u64).to_bytes(buf));
        buf.append(&mut buf2);
        Ok(buf)
    }

    pub fn tuple_to_bytes<
        T: ToBytes,
        const N: usize,
        const C: u8,
    >(
        //v: &[Dat; N],
        v: &[T; N],
        mut buf: Vec<u8>,
    )
        -> Outcome<Vec<u8>>
    {
        let mut buf2 = Vec::new();
        for item in v {
            buf2 = res!(item.to_bytes(buf2));
        }
        buf.push(C + N as u8 - 2);
        buf.append(&mut buf2);
        Ok(buf)
    }
}
