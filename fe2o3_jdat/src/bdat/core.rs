use crate::{
    prelude::*,
    usr::UsrKindId,
};

use oxedyne_fe2o3_core::prelude::*;


impl Dat {

    /// Returns the length of the encoding in bytes, if it can be known without actually performing
    /// the encoding.
    ///
    /// A kind that encloses others answers from its children, so a whole tree is measured without
    /// a byte of it being built: a list is its code, the compact length of its payload, and the
    /// payload.  `None` travels upwards, since a container holding one child it cannot measure is
    /// one it cannot measure either.
    ///
    /// Only [`Dat::Aint`] and [`Dat::Adec`] answer `None` on their own account.  An
    /// arbitrary-precision number is written under a length taken from the digits it serialises
    /// to, so measuring one is encoding one, and this promises not to.
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
            Self::List(v)   => Self::run_framed(v.iter()),
            Self::Tup2(a)   => Self::run_tagged(a.iter()),
            Self::Tup3(a)   => Self::run_tagged(a.iter()),
            Self::Tup4(a)   => Self::run_tagged(a.iter()),
            Self::Tup5(a)   => Self::run_tagged(a.iter()),
            Self::Tup6(a)   => Self::run_tagged(a.iter()),
            Self::Tup7(a)   => Self::run_tagged(a.iter()),
            Self::Tup8(a)   => Self::run_tagged(a.iter()),
            Self::Tup9(a)   => Self::run_tagged(a.iter()),
            Self::Tup10(a)  => Self::run_tagged(a.iter()),
            Self::Map(map)  => Self::run_framed(map.iter().flat_map(|(k, v)| [k, v])),
            // An ordered map's key is written as its daticle alone; the ordinal beside it
            // orders the entries and never leaves memory.
            Self::OrdMap(m) => Self::run_framed(m.iter().flat_map(|(k, v)| [k.dat(), v])),
            // Homogenous
            Self::Vek(vek)  => Self::run_framed(vek.iter()),
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

            Self::Tup2u8(_)     => Some(1 + 2 * 1),
            Self::Tup3u8(_)     => Some(1 + 3 * 1),
            Self::Tup4u8(_)     => Some(1 + 4 * 1),
            Self::Tup5u8(_)     => Some(1 + 5 * 1),
            Self::Tup6u8(_)     => Some(1 + 6 * 1),
            Self::Tup7u8(_)     => Some(1 + 7 * 1),
            Self::Tup8u8(_)     => Some(1 + 8 * 1),
            Self::Tup9u8(_)     => Some(1 + 9 * 1),
            Self::Tup10u8(_)    => Some(1 + 10 * 1),

            Self::Tup2i8(_)     => Some(1 + 2 * 1),
            Self::Tup3i8(_)     => Some(1 + 3 * 1),
            Self::Tup4i8(_)     => Some(1 + 4 * 1),
            Self::Tup5i8(_)     => Some(1 + 5 * 1),
            Self::Tup6i8(_)     => Some(1 + 6 * 1),
            Self::Tup7i8(_)     => Some(1 + 7 * 1),
            Self::Tup8i8(_)     => Some(1 + 8 * 1),
            Self::Tup9i8(_)     => Some(1 + 9 * 1),
            Self::Tup10i8(_)    => Some(1 + 10 * 1),

            Self::Tup2i16(_)    => Some(1 + 2 * 2),
            Self::Tup3i16(_)    => Some(1 + 3 * 2),
            Self::Tup4i16(_)    => Some(1 + 4 * 2),
            Self::Tup5i16(_)    => Some(1 + 5 * 2),
            Self::Tup6i16(_)    => Some(1 + 6 * 2),
            Self::Tup7i16(_)    => Some(1 + 7 * 2),
            Self::Tup8i16(_)    => Some(1 + 8 * 2),
            Self::Tup9i16(_)    => Some(1 + 9 * 2),
            Self::Tup10i16(_)   => Some(1 + 10 * 2),

            Self::Tup2i32(_)    => Some(1 + 2 * 4),
            Self::Tup3i32(_)    => Some(1 + 3 * 4),
            Self::Tup4i32(_)    => Some(1 + 4 * 4),
            Self::Tup5i32(_)    => Some(1 + 5 * 4),
            Self::Tup6i32(_)    => Some(1 + 6 * 4),
            Self::Tup7i32(_)    => Some(1 + 7 * 4),
            Self::Tup8i32(_)    => Some(1 + 8 * 4),
            Self::Tup9i32(_)    => Some(1 + 9 * 4),
            Self::Tup10i32(_)   => Some(1 + 10 * 4),

            Self::Tup2i64(_)    => Some(1 + 2 * 8),
            Self::Tup3i64(_)    => Some(1 + 3 * 8),
            Self::Tup4i64(_)    => Some(1 + 4 * 8),
            Self::Tup5i64(_)    => Some(1 + 5 * 8),
            Self::Tup6i64(_)    => Some(1 + 6 * 8),
            Self::Tup7i64(_)    => Some(1 + 7 * 8),
            Self::Tup8i64(_)    => Some(1 + 8 * 8),
            Self::Tup9i64(_)    => Some(1 + 9 * 8),
            Self::Tup10i64(_)   => Some(1 + 10 * 8),

            //// Scheduled for removal
            //Self::PartKey(_) => Some(41),
        }

    }

    /// The bytes a run of daticles comes to, in order and with nothing between them.
    fn run_len<'a, I: Iterator<Item = &'a Dat>>(items: I) -> Option<usize> {
        let mut sum = 0usize;
        for item in items {
            match item.byte_len() {
                Some(len) => sum += len,
                None => return None,
            }
        }
        Some(sum)
    }

    /// A run under a code byte and a compact length, which is how a list, a vek and a map are all
    /// written.  A map's run is its keys and values alternating, and the sum does not depend on
    /// which order the pairs come out in.
    fn run_framed<'a, I: Iterator<Item = &'a Dat>>(items: I) -> Option<usize> {
        match Self::run_len(items) {
            Some(len) => Some(1 + Self::c64_len(len) + len),
            None => None,
        }
    }

    /// A run under a code byte alone.  A tuple carries its length in its code, so nothing measures
    /// the payload on the wire and nothing needs to here.
    fn run_tagged<'a, I: Iterator<Item = &'a Dat>>(items: I) -> Option<usize> {
        match Self::run_len(items) {
            Some(len) => Some(1 + len),
            None => None,
        }
    }

    pub fn c64_len(num: usize) -> usize {
        // Compare in `u64`: the upper literals exceed a 32-bit `usize` (e.g. on
        // `wasm32`); widening `num` is lossless and keeps the ladder correct on
        // both 32- and 64-bit targets.
        let num = num as u64;
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

#[cfg(test)]
mod tests {
    use super::*;

    use crate::prelude::*;

    use oxedyne_fe2o3_num::BigInt;

    /// Every kind here is one the encoder writes as a code and a run of children, which is what
    /// `byte_len` had to be taught and what it could get subtly wrong.
    fn containers() -> Vec<(&'static str, Dat)> {
        let atoms = vec![
            Dat::Empty,
            Dat::U8(7),
            Dat::U64(1 << 40),
            Dat::Str("a string".to_string()),
            Dat::BU64(vec![9u8; 300]),
            Dat::Opt(Box::new(Some(Dat::I32(-5)))),
            Dat::Opt(Box::new(None)),
        ];
        let mut map = DaticleMap::new();
        map.insert(Dat::Str("k".to_string()), Dat::List(atoms.clone()));
        map.insert(Dat::U8(1), Dat::Empty);
        let mut ordmap = OrdDaticleMap::new();
        ordmap.insert(MapKey::new(0, Dat::Str("first".to_string())), Dat::BU8(vec![1, 2, 3]));
        ordmap.insert(MapKey::new(1, Dat::U16(9)), Dat::List(vec![Dat::Empty]));
        vec![
            ("an empty list",       Dat::List(Vec::new())),
            ("a list of atoms",     Dat::List(atoms.clone())),
            ("a nested list",       Dat::List(vec![Dat::List(atoms.clone()), Dat::List(vec![
                                        Dat::List(atoms.clone()),
                                    ])])),
            ("an empty vek",        Dat::Vek(Vek(Vec::new()))),
            ("a vek",               Dat::Vek(Vek(atoms.clone()))),
            ("a tup2",              Dat::Tup2(Box::new([Dat::Empty, Dat::List(atoms.clone())]))),
            ("a tup3",              Dat::Tup3(Box::new([Dat::U8(1), Dat::U8(2), Dat::U8(3)]))),
            ("a tup4",              Dat::Tup4(Box::new([
                                        Dat::Empty, Dat::Empty, Dat::Empty,
                                        Dat::List(atoms.clone()),
                                    ]))),
            ("a tup5",              Dat::Tup5(Box::new([
                                        Dat::U8(1), Dat::U8(2), Dat::U8(3), Dat::U8(4),
                                        Dat::List(atoms.clone()),
                                    ]))),
            ("a tup6",              Dat::Tup6(Box::new([
                                        Dat::U8(1), Dat::U8(2), Dat::U8(3), Dat::U8(4),
                                        Dat::U8(5), Dat::List(atoms.clone()),
                                    ]))),
            ("a tup7",              Dat::Tup7(Box::new([
                                        Dat::U8(1), Dat::U8(2), Dat::U8(3), Dat::U8(4),
                                        Dat::U8(5), Dat::U8(6), Dat::List(atoms.clone()),
                                    ]))),
            ("a tup8",              Dat::Tup8(Box::new([
                                        Dat::U8(1), Dat::U8(2), Dat::U8(3), Dat::U8(4),
                                        Dat::U8(5), Dat::U8(6), Dat::U8(7),
                                        Dat::List(atoms.clone()),
                                    ]))),
            ("a tup9",              Dat::Tup9(Box::new([
                                        Dat::U8(1), Dat::U8(2), Dat::U8(3), Dat::U8(4),
                                        Dat::U8(5), Dat::U8(6), Dat::U8(7), Dat::U8(8),
                                        Dat::List(atoms.clone()),
                                    ]))),
            ("a tup10",             Dat::Tup10(Box::new([
                                        Dat::U8(1), Dat::U8(2), Dat::U8(3), Dat::U8(4),
                                        Dat::U8(5), Dat::U8(6), Dat::U8(7), Dat::U8(8),
                                        Dat::U8(9), Dat::List(atoms.clone()),
                                    ]))),
            ("an empty map",        Dat::Map(DaticleMap::new())),
            ("a map",               Dat::Map(map.clone())),
            ("an empty ordmap",     Dat::OrdMap(OrdDaticleMap::new())),
            ("an ordmap",           Dat::OrdMap(ordmap.clone())),
            ("a map inside a list", Dat::List(vec![Dat::Map(map), Dat::OrdMap(ordmap)])),
        ]
    }

    /// Proved red by returning `None` from every container arm, which is where this started, and
    /// again by dropping the compact length from `run_framed`.
    #[test]
    fn test_byte_len_is_what_the_encoder_writes() -> Outcome<()> {
        for (name, dat) in containers() {
            let bytes = res!(dat.to_bytes(Vec::new()));
            match dat.byte_len() {
                Some(len) => assert_eq!(len, bytes.len(),
                    "{} is measured at {} bytes and encodes to {}", name, len, bytes.len()),
                None => return Err(err!(
                    "{} could not be measured at all, so a caller sizing a reply by it has \
                    to encode it to find out.", name;
                Test, Invalid)),
            }
        }
        Ok(())
    }

    /// The payload's length is itself written under a compact length that widens with it, so a
    /// measurement that assumed one width would be right until it was not.
    ///
    /// Proved red by fixing `run_framed` at two bytes of length prefix.
    #[test]
    fn test_byte_len_widens_with_the_compact_length() -> Outcome<()> {
        let mut widths = std::collections::BTreeSet::new();
        for payload in [0usize, 1, 253, 254, 255, 256, 257, 65_534, 65_535, 65_536, 65_537] {
            let child = Dat::BC64(vec![3u8; payload]);
            let inner = match child.byte_len() {
                Some(len) => len,
                None => return Err(err!(
                    "A byte payload of {} could not be measured.", payload; Test, Invalid)),
            };
            let dat = Dat::List(vec![child]);
            let bytes = res!(dat.to_bytes(Vec::new()));
            let len = match dat.byte_len() {
                Some(len) => len,
                None => return Err(err!(
                    "A list holding {} bytes could not be measured.", payload;
                Test, Invalid)),
            };
            assert_eq!(len, bytes.len(),
                "a list holding {} bytes is measured at {} and encodes to {}",
                payload, len, bytes.len());
            widths.insert(Dat::c64_len(inner));
        }
        assert!(widths.len() >= 3,
            "every case took the same {:?} byte length prefix, so the widening is untested",
            widths);
        Ok(())
    }

    /// A container answers `None` rather than a number that leaves out what it could not measure.
    ///
    /// Proved red by having `run_len` skip an unmeasurable child instead of giving up on the run.
    #[test]
    fn test_byte_len_declines_what_it_cannot_measure() -> Outcome<()> {
        let unknown = Dat::Aint(BigInt::from(1u8));
        assert_eq!(unknown.byte_len(), None, "an arbitrary-precision integer was measured");
        let mut map = DaticleMap::new();
        map.insert(Dat::U8(1), unknown.clone());
        let mut ordmap = OrdDaticleMap::new();
        ordmap.insert(MapKey::new(0, Dat::U8(1)), unknown.clone());
        for (name, dat) in [
            ("a list",              Dat::List(vec![Dat::Empty, unknown.clone()])),
            ("a list of lists",     Dat::List(vec![Dat::List(vec![unknown.clone()])])),
            ("a vek",               Dat::Vek(Vek(vec![unknown.clone()]))),
            ("a tuple",             Dat::Tup2(Box::new([Dat::Empty, unknown.clone()]))),
            ("a map",               Dat::Map(map)),
            ("an ordered map",      Dat::OrdMap(ordmap)),
        ] {
            assert_eq!(dat.byte_len(), None,
                "{} holding a number nothing can measure answered with a length", name);
        }
        Ok(())
    }
}
