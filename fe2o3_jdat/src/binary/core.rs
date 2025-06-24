use crate::{
    prelude::*,
    usr::UsrKindId,
};

use oxedyne_fe2o3_core::prelude::*;


impl Dat {

    /// Returns the length of the encoding in bytes, if it can be known without actually performing
    /// the encoding.
    pub fn byte_len(&self) -> Option<usize> {
        match self {
            // Atomic Kinds ===========================
            // Logic
            Self::Empty     |
            Self::Bool(_)   => Some(1),
            // Fixed
            Self::U8(_)     |
            Self::I8(_)     => Some(2),
            Self::U16(_)    |
            Self::I16(_)    => Some(3),
            Self::U32(_)    |
            Self::I32(_)    |
            Self::F32(_)    => Some(5),
            Self::U64(_)    |
            Self::I64(_)    |
            Self::F64(_)    => Some(9),
            Self::U128(_)   |
            Self::I128(_)   => Some(17),
            // Variable
            Self::Aint(_)   |
            Self::Adec(_)   => None,
            Self::C64(v) => Some(Self::c64_len(*v as usize)),
            Self::Str(s) => {
                let len = s.as_bytes().len();
                Some(1 + Self::c64_len(len) + len)
            },
            // Molecule Kinds =========================
            // Unitary
            Self::Usr(_ukid, optboxd) => {
                const CODE_LEN: usize = UsrKindId::CODE_BYTE_LEN;
                match optboxd {
                    None => Some(2 + CODE_LEN),
                    Some(boxd) => match boxd.byte_len() {
                        None => None,
                        Some(len) => Some(2 + CODE_LEN + len),
                    },
                }
            },
            Self::Box(boxd) => {
                match boxd.byte_len() {
                    None => None,
                    Some(len) => Some(1 + len),
                }
            },
            Self::Opt(boxoptd) => {
                match &**boxoptd {
                    None => Some(1),
                    Some(d) => match d.byte_len() {
                        None => None,
                        Some(len) => Some(1 + len),
                    }
                }
            },
            Self::ABox(_, boxd, s) => {
                match boxd.byte_len() {
                    None => None,
                    Some(boxd_len) => {
                        let s_len = s.as_bytes().len();
                        Some(1 + boxd_len + Self::c64_len(s_len) + s_len)
                    }
                }
            },
            // Heterogenous
            Self::List(_)   |
            Self::Tup2(_)   |
            Self::Tup3(_)   |
            Self::Tup4(_)   |
            Self::Tup5(_)   |
            Self::Tup6(_)   |
            Self::Tup7(_)   |
            Self::Tup8(_)   |
            Self::Tup9(_)   |
            Self::Tup10(_)  |
            Self::Map(_)    |
            Self::OrdMap(_) => None,
            // Homogenous
            Self::Vek(_)    => None,
            // Variable length bytes
            Self::BU8(v)    => Some(1 + 1 + v.len()),
            Self::BU16(v)   => Some(1 + 2 + v.len()),
            Self::BU32(v)   => Some(1 + 4 + v.len()),
            Self::BU64(v)   => Some(1 + 8 + v.len()),
            Self::BC64(v) => {
                let len = v.len();
                Some(1 + Self::c64_len(len) + len)
            },
            // Fixed length bytes
            Self::B2(_)     => Some(3),
            Self::B3(_)     => Some(4),
            Self::B4(_)     => Some(5),
            Self::B5(_)     => Some(6),
            Self::B6(_)     => Some(7),
            Self::B7(_)     => Some(8),
            Self::B8(_)     => Some(9),
            Self::B9(_)     => Some(10),
            Self::B10(_)    => Some(11),
            Self::B16(_)    => Some(17),
            Self::B32(_)    => Some(33),
            // Fixed length numbers
            Self::Tup2u16(_)    => Some(1 + 2 * 2),
            Self::Tup3u16(_)    => Some(1 + 3 * 2),
            Self::Tup4u16(_)    => Some(1 + 4 * 2),
            Self::Tup5u16(_)    => Some(1 + 5 * 2),
            Self::Tup6u16(_)    => Some(1 + 6 * 2),
            Self::Tup7u16(_)    => Some(1 + 7 * 2),
            Self::Tup8u16(_)    => Some(1 + 8 * 2),
            Self::Tup9u16(_)    => Some(1 + 9 * 2),
            Self::Tup10u16(_)   => Some(1 + 10 * 2),

            Self::Tup2u32(_)    => Some(1 + 2 * 4),
            Self::Tup3u32(_)    => Some(1 + 3 * 4),
            Self::Tup4u32(_)    => Some(1 + 4 * 4),
            Self::Tup5u32(_)    => Some(1 + 5 * 4),
            Self::Tup6u32(_)    => Some(1 + 6 * 4),
            Self::Tup7u32(_)    => Some(1 + 7 * 4),
            Self::Tup8u32(_)    => Some(1 + 8 * 4),
            Self::Tup9u32(_)    => Some(1 + 9 * 4),
            Self::Tup10u32(_)   => Some(1 + 10 * 4),

            Self::Tup2u64(_)    => Some(1 + 2 * 8),
            Self::Tup3u64(_)    => Some(1 + 3 * 8),
            Self::Tup4u64(_)    => Some(1 + 4 * 8),
            Self::Tup5u64(_)    => Some(1 + 5 * 8),
            Self::Tup6u64(_)    => Some(1 + 6 * 8),
            Self::Tup7u64(_)    => Some(1 + 7 * 8),
            Self::Tup8u64(_)    => Some(1 + 8 * 8),
            Self::Tup9u64(_)    => Some(1 + 9 * 8),
            Self::Tup10u64(_)   => Some(1 + 10 * 8),

            //// Scheduled for removal
            //Self::PartKey(_) => Some(41),
        }

    }

    pub fn c64_len(num: usize) -> usize {
        if num == 0 {
            1
        } else if num <= 0xff {
            2
        } else if num <= 0xffff {
            3
        } else if num <= 0xffffff {
            4
        } else if num <= 0xffffffff {
            5
        } else if num <= 0xffffffffff {
            6
        } else if num <= 0xffffffffffff {
            7
        } else {
            8
        }
    }

}
