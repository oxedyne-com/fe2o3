use crate::{
    prelude::*,
};

use oxedize_fe2o3_core::{
    prelude::*,
    rand::Rand,
};


pub trait DaticleInteger {
    fn int_kind(&self) -> DatIntKind;
    fn as_dat(self) -> Dat;
    fn to_vec(&self) -> Vec<u8>;
    fn is_signed(&self) -> bool;
    fn fmt_hex(&self) -> String;
    fn fmt_oct(&self) -> String;
    fn fmt_bin(&self) -> String;
    fn fmt_dec(&self) -> String;
    fn min_size(&self) -> Self;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DatInt {
    U8(u8),
    U16(u16),
    U32(u32),
    U64(u64),
    U128(u128),
    BU32(B32), // Big endian, i.e. largest byte first, interpreted as unsigned.
    I8(i8),
    I16(i16),
    I32(i32),
    I64(i64),
    I128(i128),
}

#[derive(Clone, Debug)]
pub enum DatIntKind {
    U8,
    U16,
    U32,
    U64,
    U128,
    BU32,
    I8,
    I16,
    I32,
    I64,
    I128,
}

impl From<u8> for DatInt {
    fn from(n: u8) -> Self {
        Self::U8(n)
    }
}

impl From<u16> for DatInt {
    fn from(n: u16) -> Self {
        Self::U16(n)
    }
}

impl From<u32> for DatInt {
    fn from(n: u32) -> Self {
        Self::U32(n)
    }
}

impl From<u64> for DatInt {
    fn from(n: u64) -> Self {
        Self::U64(n)
    }
}

impl From<u128> for DatInt {
    fn from(n: u128) -> Self {
        Self::U128(n)
    }
}

impl From<usize> for DatInt {
    fn from(n: usize) -> Self {
        Self::U128(n as u128)
    }
}

impl From<[u8; 32]> for DatInt {
    fn from(a: [u8; 32]) -> Self {
        Self::BU32(B32(a))
    }
}

impl From<B32> for DatInt {
    fn from(a: B32) -> Self {
        Self::BU32(a)
    }
}

impl From<i8> for DatInt {
    fn from(n: i8) -> Self {
        Self::I8(n)
    }
}

impl From<i16> for DatInt {
    fn from(n: i16) -> Self {
        Self::I16(n)
    }
}

impl From<i32> for DatInt {
    fn from(n: i32) -> Self {
        Self::I32(n)
    }
}

impl From<i64> for DatInt {
    fn from(n: i64) -> Self {
        Self::I64(n)
    }
}

impl From<i128> for DatInt {
    fn from(n: i128) -> Self {
        Self::I128(n)
    }
}

impl From<isize> for DatInt {
    fn from(n: isize) -> Self {
        Self::I128(n as i128)
    }
}

impl DaticleInteger for DatInt {

    fn int_kind(&self) -> DatIntKind {
        match self {
            Self::U8(_)     => DatIntKind::U8,
            Self::U16(_)    => DatIntKind::U16,
            Self::U32(_)    => DatIntKind::U32,
            Self::U64(_)    => DatIntKind::U64,
            Self::U128(_)   => DatIntKind::U128,
            Self::BU32(_)   => DatIntKind::BU32,
            Self::I8(_)     => DatIntKind::I8,
            Self::I16(_)    => DatIntKind::I16,
            Self::I32(_)    => DatIntKind::I32,
            Self::I64(_)    => DatIntKind::I64,
            Self::I128(_)   => DatIntKind::I128,
        }
    }

    fn as_dat(self) -> Dat {
        match self {
            Self::U8(n)     => Dat::U8(n),
            Self::U16(n)    => Dat::U16(n),
            Self::U32(n)    => Dat::U32(n),
            Self::U64(n)    => Dat::U64(n),
            Self::U128(n)   => Dat::U128(n),
            Self::BU32(b)   => Dat::B32(b),
            Self::I8(n)     => Dat::I8(n),
            Self::I16(n)    => Dat::I16(n),
            Self::I32(n)    => Dat::I32(n),
            Self::I64(n)    => Dat::I64(n),
            Self::I128(n)   => Dat::I128(n),
        }
    }

    fn to_vec(&self) -> Vec<u8> {
        match self {
            Self::U8(n)     => n.to_be_bytes().to_vec(),
            Self::U16(n)    => n.to_be_bytes().to_vec(),
            Self::U32(n)    => n.to_be_bytes().to_vec(),
            Self::U64(n)    => n.to_be_bytes().to_vec(),
            Self::U128(n)   => n.to_be_bytes().to_vec(),
            Self::BU32(a)   => a.to_vec(),
            Self::I8(n)     => n.to_be_bytes().to_vec(),
            Self::I16(n)    => n.to_be_bytes().to_vec(),
            Self::I32(n)    => n.to_be_bytes().to_vec(),
            Self::I64(n)    => n.to_be_bytes().to_vec(),
            Self::I128(n)   => n.to_be_bytes().to_vec(),
        }
    }

    fn is_signed(&self) -> bool {
        match self {
            Self::U8(_)     |
            Self::U16(_)    |
            Self::U32(_)    |
            Self::U64(_)    |
            Self::U128(_)   |
            Self::BU32(_)   => false, 
            Self::I8(_)     |
            Self::I16(_)    |
            Self::I32(_)    |
            Self::I64(_)    |
            Self::I128(_)   => true,
        }
    }

    fn fmt_hex(&self) -> String {
        match self {
            Self::U8(n)     => fmt!("0x{:02x}", n),
            Self::U16(n)    => fmt!("0x{:04x}", n),
            Self::U32(n)    => fmt!("0x{:08x}", n),
            Self::U64(n)    => fmt!("0x{:016x}", n),
            Self::U128(n)   => fmt!("0x{:032x}", n),
            Self::BU32(a) =>
                a.iter().map(|b| format!("0x{:02x}", b)).collect::<Vec<_>>().join(", "),
            Self::I8(n)     => {
                let sign = if *n < 0 { "-" } else { "" };
                let a = match n.checked_abs() {
                    Some(a) => a as u8,
                    None => i8::MAX as u8 + 1,
                };
                fmt!("{}0x{:02x}", sign, a)
            },
            Self::I16(n)    => {
                let sign = if *n < 0 { "-" } else { "" };
                let a = match n.checked_abs() {
                    Some(a) => a as u16,
                    None => i16::MAX as u16 + 1,
                };
                fmt!("{}0x{:04x}", sign, a)
            },
            Self::I32(n)    => {
                let sign = if *n < 0 { "-" } else { "" };
                let a = match n.checked_abs() {
                    Some(a) => a as u32,
                    None => i32::MAX as u32 + 1,
                };
                fmt!("{}0x{:08x}", sign, a)
            },
            Self::I64(n)    => {
                let sign = if *n < 0 { "-" } else { "" };
                let a = match n.checked_abs() {
                    Some(a) => a as u64,
                    None => i64::MAX as u64 + 1,
                };
                fmt!("{}0x{:016x}", sign, a)
            },
            Self::I128(n)   => {
                let sign = if *n < 0 { "-" } else { "" };
                let a = match n.checked_abs() {
                    Some(a) => a as u128,
                    None => i128::MAX as u128 + 1,
                };
                fmt!("{}0x{:032x}", sign, a)
            },
        }
    }

    fn fmt_oct(&self) -> String {
        match self {
            Self::U8(n)     => fmt!("0o{:o}", n),
            Self::U16(n)    => fmt!("0o{:o}", n),
            Self::U32(n)    => fmt!("0o{:o}", n),
            Self::U64(n)    => fmt!("0o{:o}", n),
            Self::U128(n)   => fmt!("0o{:o}", n),
            Self::BU32(a) =>
                a.iter().map(|b| format!("0o{:03x}", b)).collect::<Vec<_>>().join(", "),
            Self::I8(n)     => {
                let sign = if *n < 0 { "-" } else { "" };
                let a = match n.checked_abs() {
                    Some(a) => a as u8,
                    None => i8::MAX as u8 + 1,
                };
                fmt!("{}0o{:o}", sign, a)
            },
            Self::I16(n)    => {
                let sign = if *n < 0 { "-" } else { "" };
                let a = match n.checked_abs() {
                    Some(a) => a as u16,
                    None => i16::MAX as u16 + 1,
                };
                fmt!("{}0o{:o}", sign, a)
            },
            Self::I32(n)    => {
                let sign = if *n < 0 { "-" } else { "" };
                let a = match n.checked_abs() {
                    Some(a) => a as u32,
                    None => i32::MAX as u32 + 1,
                };
                fmt!("{}0o{:o}", sign, a)
            },
            Self::I64(n)    => {
                let sign = if *n < 0 { "-" } else { "" };
                let a = match n.checked_abs() {
                    Some(a) => a as u64,
                    None => i64::MAX as u64 + 1,
                };
                fmt!("{}0o{:o}", sign, a)
            },
            Self::I128(n)   => {
                let sign = if *n < 0 { "-" } else { "" };
                let a = match n.checked_abs() {
                    Some(a) => a as u128,
                    None => i128::MAX as u128 + 1,
                };
                fmt!("{}0o{:o}", sign, a)
            },
        }
    }

    fn fmt_bin(&self) -> String {
        let (s, sign) = match self {
            Self::U8(n) => (
                n.to_be_bytes().iter().map(|b| fmt!("{:08b}", b)).collect::<Vec<_>>().join("_"),
                "",
            ),
            Self::U16(n) => (
                n.to_be_bytes().iter().map(|b| fmt!("{:08b}", b)).collect::<Vec<_>>().join("_"),
                "",
            ),
            Self::U32(n) => (
                n.to_be_bytes().iter().map(|b| fmt!("{:08b}", b)).collect::<Vec<_>>().join("_"),
                "",
            ),
            Self::U64(n) => (
                n.to_be_bytes().iter().map(|b| fmt!("{:08b}", b)).collect::<Vec<_>>().join("_"),
                "",
            ),
            Self::U128(n) => (
                n.to_be_bytes().iter().map(|b| fmt!("{:08b}", b)).collect::<Vec<_>>().join("_"),
                "",
            ),
            Self::BU32(a) => (
                a.iter().map(|b| fmt!("{:08b}", b)).collect::<Vec<_>>().join("_"),
                "",
            ),
            Self::I8(n) => (
                (
                    match n.checked_abs() {
                        Some(a) => a as u8,
                        None => i8::MAX as u8 + 1,
                    }
                ).to_be_bytes().iter().map(|b| fmt!("{:08b}", b)).collect::<Vec<_>>().join("_"),
                if *n < 0 { "-" } else { "" },
            ),
            Self::I16(n) => (
                (
                    match n.checked_abs() {
                        Some(a) => a as u16,
                        None => i16::MAX as u16 + 1,
                    }
                ).to_be_bytes().iter().map(|b| fmt!("{:08b}", b)).collect::<Vec<_>>().join("_"),
                if *n < 0 { "-" } else { "" },
            ),
            Self::I32(n) => (
                (
                    match n.checked_abs() {
                        Some(a) => a as u32,
                        None => i32::MAX as u32 + 1,
                    }
                ).to_be_bytes().iter().map(|b| fmt!("{:08b}", b)).collect::<Vec<_>>().join("_"),
                if *n < 0 { "-" } else { "" },
            ),
            Self::I64(n) => (
                (
                    match n.checked_abs() {
                        Some(a) => a as u64,
                        None => i64::MAX as u64 + 1,
                    }
                ).to_be_bytes().iter().map(|b| fmt!("{:08b}", b)).collect::<Vec<_>>().join("_"),
                if *n < 0 { "-" } else { "" },
            ),
            Self::I128(n) => (
                (
                    match n.checked_abs() {
                        Some(a) => a as u128,
                        None => i128::MAX as u128 + 1,
                    }
                ).to_be_bytes().iter().map(|b| fmt!("{:08b}", b)).collect::<Vec<_>>().join("_"),
                if *n < 0 { "-" } else { "" },
            ),
        };
        fmt!("{}0b{}", sign, s)
    }

    fn fmt_dec(&self) -> String {
        match self {
            Self::U8(n)     => n.to_string(),
            Self::U16(n)    => n.to_string(),
            Self::U32(n)    => n.to_string(),
            Self::U64(n)    => n.to_string(),
            Self::U128(n)   => n.to_string(),
            Self::BU32(a)    => a.iter().map(|b| b.to_string()).collect::<Vec<_>>().join(", "),
            Self::I8(n)     => n.to_string(),
            Self::I16(n)    => n.to_string(),
            Self::I32(n)    => n.to_string(),
            Self::I64(n)    => n.to_string(),
            Self::I128(n)   => n.to_string(),
        }
    }

    /// Re-wrap the integer into the variant with the smallest binary representation.  Positive
    /// integers get represented as uint variants, regardless of the original `DatIntKind`.  If a zero
    /// is originally a signed int, the result will be an I8.
    fn min_size(&self) -> Self {
        match self {
            Self::U8(n)     => Self::min_size_uint(*n as u128),
            Self::U16(n)    => Self::min_size_uint(*n as u128),
            Self::U32(n)    => Self::min_size_uint(*n as u128),
            Self::U64(n)    => Self::min_size_uint(*n as u128),
            Self::U128(n)   => Self::min_size_uint(*n as u128),

            Self::I8(n) => if *n <= 0 {
                Self::min_size_int(*n as i128)
            } else {
                Self::min_size_uint(n.abs() as u128)
            },
            Self::I16(n) => if *n <= 0 {
                Self::min_size_int(*n as i128)
            } else {
                Self::min_size_uint(n.abs() as u128)
            },
            Self::I32(n) => if *n <= 0 {
                Self::min_size_int(*n as i128)
            } else {
                Self::min_size_uint(n.abs() as u128)
            },
            Self::I64(n) => if *n <= 0 {
                Self::min_size_int(*n as i128)
            } else {
                Self::min_size_uint(n.abs() as u128)
            },
            Self::I128(n) => if *n <= 0 {
                Self::min_size_int(*n as i128)
            } else {
                Self::min_size_uint(n.abs() as u128)
            },

            Self::BU32(byts) => {
                // Count the number of leading zero bytes:
                let mut count: usize = 0;
                for byt in **byts {
                    if byt == 0 {
                        count += 1;
                    } else {
                        break;
                    }
                }
                // If the top 16 most significant are zero, we can use a u128.
                // If the top 16 + 8 = 24 most significant are zero, we can use a u64.
                // If the top 16 + 8 + 4 = 28 most significant are zero, we can use a u32.
                // If the top 16 + 8 + 4 + 2 = 30 most significant are zero, we can use a u16.
                // If the top 16 + 8 + 4 + 2 + 1 = 31 most significant are zero, we can use a u8.
                if count >= 31 {
                    let mut a = [0u8; 1];
                    a.copy_from_slice(&byts[31..]);
                    Self::U8(u8::from_be_bytes(a))
                } else if count >= 30 {
                    let mut a = [0u8; 2];
                    a.copy_from_slice(&byts[30..]);
                    Self::U16(u16::from_be_bytes(a))
                } else if count >= 28 {
                    let mut a = [0u8; 4];
                    a.copy_from_slice(&byts[28..]);
                    Self::U32(u32::from_be_bytes(a))
                } else if count >= 24 {
                    let mut a = [0u8; 8];
                    a.copy_from_slice(&byts[24..]);
                    Self::U64(u64::from_be_bytes(a))
                } else if count >= 16 {
                    let mut a = [0u8; 16];
                    a.copy_from_slice(&byts[16..]);
                    Self::U128(u128::from_be_bytes(a))
                } else {
                    *self
                }
            },
        }
    }
}

impl DatInt {

    const U8_MAX_AS_U128: u128 = u8::MAX as u128;
    const U16_MAX_AS_U128: u128 = u16::MAX as u128;
    const U32_MAX_AS_U128: u128 = u32::MAX as u128;
    const U64_MAX_AS_U128: u128 = u64::MAX as u128;

    pub fn min_size_uint(n: u128) -> Self {
        if n <= Self::U8_MAX_AS_U128 {
            Self::U8(n as u8)
        } else if n <= Self::U16_MAX_AS_U128 {
            Self::U16(n as u16)
        } else if n <= Self::U32_MAX_AS_U128 {
            Self::U32(n as u32)
        } else if n <= Self::U64_MAX_AS_U128 {
            Self::U64(n as u64)
        } else {
            Self::U128(n)
        }
    }

    const I8_MAX_AS_I128: i128 = i8::MAX as i128;
    const I16_MAX_AS_I128: i128 = i16::MAX as i128;
    const I32_MAX_AS_I128: i128 = i32::MAX as i128;
    const I64_MAX_AS_I128: i128 = i64::MAX as i128;

    const I8_MIN_AS_I128: i128 = i8::MIN as i128;
    const I16_MIN_AS_I128: i128 = i16::MIN as i128;
    const I32_MIN_AS_I128: i128 = i32::MIN as i128;
    const I64_MIN_AS_I128: i128 = i64::MIN as i128;

    pub fn min_size_int(n: i128) -> Self {
        if Self::I8_MIN_AS_I128 <= n && n <= Self::I8_MAX_AS_I128 {
            Self::I8(n as i8)
        } else if Self::I16_MIN_AS_I128 <= n && n <= Self::I16_MAX_AS_I128 {
            Self::I16(n as i16)
        } else if Self::I32_MIN_AS_I128 <= n && n <= Self::I32_MAX_AS_I128 {
            Self::I32(n as i32)
        } else if Self::I64_MIN_AS_I128 <= n && n <= Self::I64_MAX_AS_I128 {
            Self::I64(n as i64)
        } else {
            Self::I128(n)
        }
    }
}

impl ToDat for DatInt {
    fn to_dat(&self) -> Outcome<Dat> {
        Ok(match self {
            Self::U8(n)     => Dat::U8(*n),
            Self::U16(n)    => Dat::U16(*n),
            Self::U32(n)    => Dat::U32(*n),
            Self::U64(n)    => Dat::U64(*n),
            Self::U128(n)   => Dat::U128(*n),
            Self::BU32(b)   => Dat::B32(*b),
            Self::I8(n)     => Dat::I8(*n),
            Self::I16(n)    => Dat::I16(*n),
            Self::I32(n)    => Dat::I32(*n),
            Self::I64(n)    => Dat::I64(*n),
            Self::I128(n)   => Dat::I128(*n),
        })
    }
}

impl DatIntKind {

    pub fn rand(&self) -> DatInt {
        match self {
            Self::U8    => DatInt::U8(Rand::rand_u8()),
            Self::U16   => DatInt::U16(Rand::rand_u16()),
            Self::U32   => DatInt::U32(Rand::rand_u32()),
            Self::U64   => DatInt::U64(Rand::rand_u64()),
            Self::U128  => DatInt::U128(Rand::rand_u128()),
            Self::BU32  => {
                let mut b = [0; 32];
                Rand::fill_u8(&mut b);
                DatInt::BU32(B32(b))
            },
            Self::I8    => DatInt::I8(Rand::rand_u8() as i8),
            Self::I16   => DatInt::I16(Rand::rand_u16() as i16),
            Self::I32   => DatInt::I32(Rand::rand_u32() as i32),
            Self::I64   => DatInt::I64(Rand::rand_u64() as i64),
            Self::I128  => DatInt::I128(Rand::rand_u128() as i128),
        }
    }
}
