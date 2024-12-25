use crate::{
    prelude::*,
    note::NoteConfig,
    usr::UsrKindId,
};

use oxedize_fe2o3_core::prelude::*;
use oxedize_fe2o3_num::{
    float::{
        Float32,
        Float64,
    },
};

use std::{
    collections::{
        BTreeMap,
        VecDeque,
    },
};

use bigdecimal::BigDecimal;
use num_bigint::BigInt;


pub trait Daticle {
    fn kind(&self) -> Kind;
}

/// An enumeration for owned serialisation and deserialisation.
#[derive(Clone, Eq, Ord, PartialEq, PartialOrd)]
pub enum Dat {
    // Atomic Kinds ===========================
    // Logic
    Empty,
    Bool(bool),
    // Fixed
    U8(u8),
    U16(u16),
    U32(u32),
    U64(u64),
    U128(u128),
    I8(i8),
    I16(i16),
    I32(i32),
    I64(i64),
    I128(i128),
    F32(Float32),
    F64(Float64),
    // Variable
    Aint(BigInt),
    Adec(BigDecimal),
    C64(u64),
    Str(String),
    // Molecule Kinds =========================
    // Unitary
    Usr(UsrKindId, Option<Box<Dat>>),
    Box(Box<Dat>),
    Opt(Box<Option<Dat>>),
    ABox(NoteConfig, Box<Dat>, String),
    // Heterogenous
    List(Vec<Dat>),
    Tup2(Box<[Dat; 2]>),
    Tup3(Box<[Dat; 3]>),
    Tup4(Box<[Dat; 4]>),
    Tup5(Box<[Dat; 5]>),
    Tup6(Box<[Dat; 6]>),
    Tup7(Box<[Dat; 7]>),
    Tup8(Box<[Dat; 8]>),
    Tup9(Box<[Dat; 9]>),
    Tup10(Box<[Dat; 10]>),
    Map(DaticleMap),
    OrdMap(OrdDaticleMap),
    // Homogenous
    Vek(Vek),
    // Variable length bytes
    BU8(Vec<u8>),
    BU16(Vec<u8>),
    BU32(Vec<u8>),
    BU64(Vec<u8>),
    BC64(Vec<u8>),
    // Fixed length bytes
    B2([u8; 2]),
    B3([u8; 3]),
    B4([u8; 4]),
    B5([u8; 5]),
    B6([u8; 6]),
    B7([u8; 7]),
    B8([u8; 8]),
    B9([u8; 9]),
    B10([u8; 10]),
    B16([u8; 16]),
    B32(B32),
    // Fixed length numbers
    Tup2u16([u16; 2]),
    Tup3u16([u16; 3]),
    Tup4u16([u16; 4]),
    Tup5u16([u16; 5]),
    Tup6u16([u16; 6]),
    Tup7u16([u16; 7]),
    Tup8u16([u16; 8]),
    Tup9u16([u16; 9]),
    Tup10u16([u16; 10]),

    Tup2u32([u32; 2]),
    Tup3u32([u32; 3]),
    Tup4u32([u32; 4]),
    Tup5u32([u32; 5]),
    Tup6u32([u32; 6]),
    Tup7u32([u32; 7]),
    Tup8u32([u32; 8]),
    Tup9u32([u32; 9]),
    Tup10u32([u32; 10]),

    Tup2u64([u64; 2]),
    Tup3u64([u64; 3]),
    Tup4u64([u64; 4]),
    Tup5u64([u64; 5]),
    Tup6u64([u64; 6]),
    Tup7u64([u64; 7]),
    Tup8u64([u64; 8]),
    Tup9u64([u64; 9]),
    Tup10u64([u64; 10]),

    //// Scheduled for removal
    //PartKey(PartId),
}

impl Default for Dat {
    fn default() -> Self {
        Self::Empty
    }
}

new_type!(Vek, Vec<Dat>, Clone, Eq, Ord, PartialEq, PartialOrd);

impl IntoIterator for Vek {
    type Item = Dat;
    type IntoIter = std::vec::IntoIter<Dat>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

/// Iterates recursively over daticles, including map keys.  This consuming iterator wraps elements
/// of collections of associated data, even if they are not daticles. 
pub struct IterDat {
    stack: VecDeque<Dat>,
}

impl IterDat {

    pub fn new(dat: Dat) -> Self {
        let mut stack = VecDeque::new();
        stack.push_back(dat);
        Self { stack }
    }

    fn flatten(
        dat: Dat,
        stack: &mut VecDeque<Dat>,
    ) {
        match dat {
            Dat::List(v) => {
                for d in v.into_iter().rev() {
                    stack.push_front(d);
                }
            },
            Dat::Tup2(a) => {
                for d in a.into_iter().rev() {
                    stack.push_front(d);
                }
            },
            Dat::Tup3(a) => {
                for d in a.into_iter().rev() {
                    stack.push_front(d);
                }
            },
            Dat::Tup4(a) => {
                for d in a.into_iter().rev() {
                    stack.push_front(d);
                }
            },
            Dat::Tup5(a) => {
                for d in a.into_iter().rev() {
                    stack.push_front(d);
                }
            },
            Dat::Tup6(a) => {
                for d in a.into_iter().rev() {
                    stack.push_front(d);
                }
            },
            Dat::Tup7(a) => {
                for d in a.into_iter().rev() {
                    stack.push_front(d);
                }
            },
            Dat::Tup8(a) => {
                for d in a.into_iter().rev() {
                    stack.push_front(d);
                }
            },
            Dat::Tup9(a) => {
                for d in a.into_iter().rev() {
                    stack.push_front(d);
                }
            },
            Dat::Tup10(a) => {
                for d in a.into_iter().rev() {
                    stack.push_front(d);
                }
            },
            Dat::Map(m) => {
                for (kdat, vdat) in m.into_iter().rev() {
                    stack.push_front(vdat);
                    stack.push_front(kdat);
                }
            },
            Dat::OrdMap(m) => {
                for (mk, vdat) in m.into_iter().rev() {
                    stack.push_front(vdat);
                    stack.push_front(mk.into_dat());
                }
            },
            Dat::Vek(vek) => {
                for d in vek.0.into_iter().rev() {
                    stack.push_front(d);
                }
            },
            Dat::BU8(v)     |
            Dat::BU16(v)    |
            Dat::BU32(v)    |
            Dat::BU64(v)    |
            Dat::BC64(v)    => {
                for n in v.into_iter().rev() {
                    stack.push_front(Dat::U8(n));
                }
            },
            Dat::B2(a) => {
                for n in a.into_iter().rev() {
                    stack.push_front(Dat::U8(n));
                }
            },
            Dat::B3(a) => {
                for n in a.into_iter().rev() {
                    stack.push_front(Dat::U8(n));
                }
            },
            Dat::B4(a) => {
                for n in a.into_iter().rev() {
                    stack.push_front(Dat::U8(n));
                }
            },
            Dat::B5(a) => {
                for n in a.into_iter().rev() {
                    stack.push_front(Dat::U8(n));
                }
            },
            Dat::B6(a) => {
                for n in a.into_iter().rev() {
                    stack.push_front(Dat::U8(n));
                }
            },
            Dat::B7(a) => {
                for n in a.into_iter().rev() {
                    stack.push_front(Dat::U8(n));
                }
            },
            Dat::B8(a) => {
                for n in a.into_iter().rev() {
                    stack.push_front(Dat::U8(n));
                }
            },
            Dat::B9(a) => {
                for n in a.into_iter().rev() {
                    stack.push_front(Dat::U8(n));
                }
            },
            Dat::B10(a) => {
                for n in a.into_iter().rev() {
                    stack.push_front(Dat::U8(n));
                }
            },
            Dat::B16(a) => {
                for n in a.into_iter().rev() {
                    stack.push_front(Dat::U8(n));
                }
            },
            Dat::B32(a) => {
                for n in a.into_iter().rev() {
                    stack.push_front(Dat::U8(n));
                }
            },
            Dat::Tup2u16(a) => {
                for n in a.into_iter().rev() {
                    stack.push_front(Dat::U16(n));
                }
            },
            Dat::Tup3u16(a) => {
                for n in a.into_iter().rev() {
                    stack.push_front(Dat::U16(n));
                }
            },
            Dat::Tup4u16(a) => {
                for n in a.into_iter().rev() {
                    stack.push_front(Dat::U16(n));
                }
            },
            Dat::Tup5u16(a) => {
                for n in a.into_iter().rev() {
                    stack.push_front(Dat::U16(n));
                }
            },
            Dat::Tup6u16(a) => {
                for n in a.into_iter().rev() {
                    stack.push_front(Dat::U16(n));
                }
            },
            Dat::Tup7u16(a) => {
                for n in a.into_iter().rev() {
                    stack.push_front(Dat::U16(n));
                }
            },
            Dat::Tup8u16(a) => {
                for n in a.into_iter().rev() {
                    stack.push_front(Dat::U16(n));
                }
            },
            Dat::Tup9u16(a) => {
                for n in a.into_iter().rev() {
                    stack.push_front(Dat::U16(n));
                }
            },
            Dat::Tup10u16(a) => {
                for n in a.into_iter().rev() {
                    stack.push_front(Dat::U16(n));
                }
            },
            Dat::Tup2u32(a) => {
                for n in a.into_iter().rev() {
                    stack.push_front(Dat::U32(n));
                }
            },
            Dat::Tup3u32(a) => {
                for n in a.into_iter().rev() {
                    stack.push_front(Dat::U32(n));
                }
            },
            Dat::Tup4u32(a) => {
                for n in a.into_iter().rev() {
                    stack.push_front(Dat::U32(n));
                }
            },
            Dat::Tup5u32(a) => {
                for n in a.into_iter().rev() {
                    stack.push_front(Dat::U32(n));
                }
            },
            Dat::Tup6u32(a) => {
                for n in a.into_iter().rev() {
                    stack.push_front(Dat::U32(n));
                }
            },
            Dat::Tup7u32(a) => {
                for n in a.into_iter().rev() {
                    stack.push_front(Dat::U32(n));
                }
            },
            Dat::Tup8u32(a) => {
                for n in a.into_iter().rev() {
                    stack.push_front(Dat::U32(n));
                }
            },
            Dat::Tup9u32(a) => {
                for n in a.into_iter().rev() {
                    stack.push_front(Dat::U32(n));
                }
            },
            Dat::Tup10u32(a) => {
                for n in a.into_iter().rev() {
                    stack.push_front(Dat::U32(n));
                }
            },
            Dat::Tup2u64(a) => {
                for n in a.into_iter().rev() {
                    stack.push_front(Dat::U64(n));
                }
            },
            Dat::Tup3u64(a) => {
                for n in a.into_iter().rev() {
                    stack.push_front(Dat::U64(n));
                }
            },
            Dat::Tup4u64(a) => {
                for n in a.into_iter().rev() {
                    stack.push_front(Dat::U64(n));
                }
            },
            Dat::Tup5u64(a) => {
                for n in a.into_iter().rev() {
                    stack.push_front(Dat::U64(n));
                }
            },
            Dat::Tup6u64(a) => {
                for n in a.into_iter().rev() {
                    stack.push_front(Dat::U64(n));
                }
            },
            Dat::Tup7u64(a) => {
                for n in a.into_iter().rev() {
                    stack.push_front(Dat::U64(n));
                }
            },
            Dat::Tup8u64(a) => {
                for n in a.into_iter().rev() {
                    stack.push_front(Dat::U64(n));
                }
            },
            Dat::Tup9u64(a) => {
                for n in a.into_iter().rev() {
                    stack.push_front(Dat::U64(n));
                }
            },
            Dat::Tup10u64(a) => {
                for n in a.into_iter().rev() {
                    stack.push_front(Dat::U64(n));
                }
            },
            _ => (),
        }
    }
}

impl Iterator for IterDat {
    type Item = Dat;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some(dat) = self.stack.pop_front() {
                if dat.must_iterdat_flatten() {
                    Self::flatten(dat, &mut self.stack);
                } else {
                    return match dat {
                        Dat::Usr(_, optboxd) => match optboxd {
                            Some(boxd) => Some(*boxd),
                            None => None,
                        },
                        Dat::Box(boxd) => Some(*boxd),
                        Dat::Opt(boxoptd) => match *boxoptd {
                            Some(d) => Some(d),
                            None => None,
                        },
                        _ => Some(dat),
                    };
                }
            } else {
                return None;
            }
        }
    }
}

/// Iterates recursively over daticles but ignores map keys.  Unlike `IterDat`, which iterates over
/// individual elements of associated data collections even when they are not made up of daticles,
/// wrapping them in daticles if necessary, this iterator returns daticles with native homogenous
/// data intact.  For example, both iterators return `Dat::U8(42)`, `Dat::Str("hello")` for the
/// heterogenous `Dat::List([42u8, "hello"])`.  But in the case of `Dat::B2([42u8, 43])` containing
/// an homogenous native array, `IterDat` returns `Dat::U8(42)`, `Dat::U8(43)` while
/// `IterDatValsMut` returns `Dat::B2([42u8,43])`.
pub struct IterDatValsMut<'a> {
    stack: VecDeque<&'a mut Dat>,
}

impl<'a> IterDatValsMut<'a> {

    pub fn new(dat: &'a mut Dat) -> Self {
        let mut stack = VecDeque::new();
        stack.push_back(dat);
        Self { stack }
    }

    fn flatten(
        dat: &'a mut Dat,
        stack: &mut VecDeque<&'a mut Dat>,
    ) {
        match dat {
            Dat::List(v) => {
                for d in v.iter_mut().rev() {
                    stack.push_front(d);
                }
            },
            Dat::Tup2(a) => {
                for d in a.iter_mut().rev() {
                    stack.push_front(d);
                }
            },
            Dat::Tup3(a) => {
                for d in a.iter_mut().rev() {
                    stack.push_front(d);
                }
            },
            Dat::Tup4(a) => {
                for d in a.iter_mut().rev() {
                    stack.push_front(d);
                }
            },
            Dat::Tup5(a) => {
                for d in a.iter_mut().rev() {
                    stack.push_front(d);
                }
            },
            Dat::Tup6(a) => {
                for d in a.iter_mut().rev() {
                    stack.push_front(d);
                }
            },
            Dat::Tup7(a) => {
                for d in a.iter_mut().rev() {
                    stack.push_front(d);
                }
            },
            Dat::Tup8(a) => {
                for d in a.iter_mut().rev() {
                    stack.push_front(d);
                }
            },
            Dat::Tup9(a) => {
                for d in a.iter_mut().rev() {
                    stack.push_front(d);
                }
            },
            Dat::Tup10(a) => {
                for d in a.iter_mut().rev() {
                    stack.push_front(d);
                }
            },
            Dat::Map(m) => {
                for (_kdat, vdat) in m.iter_mut().rev() {
                    stack.push_front(vdat);
                }
            },
            Dat::OrdMap(m) => {
                for (_mk, vdat) in m.iter_mut().rev() {
                    stack.push_front(vdat);
                }
            },
            Dat::Vek(vek) => {
                for d in vek.0.iter_mut().rev() {
                    stack.push_front(d);
                }
            },
            _ => (),
        }
    }

}

impl<'a> Iterator for IterDatValsMut<'a> {
    type Item = &'a mut Dat;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some(dat) = self.stack.pop_front() {
                if dat.must_iterdatvalsmut_flatten() {
                    Self::flatten(dat, &mut self.stack);
                } else {
                    return match dat {
                        Dat::Usr(_, optboxd) => match optboxd {
                            Some(boxd) => Some(&mut *boxd),
                            None => None,
                        },
                        Dat::Box(boxd) => Some(&mut *boxd),
                        Dat::Opt(boxoptd) => match &mut **boxoptd {
                            Some(d) => Some(d),
                            None => None,
                        },
                        _ => Some(dat),
                    };
                }
            } else {
                return None;
            }
        }
    }
}

impl Dat {

    pub const USIZE_BYTES: usize = std::mem::size_of::<usize>();
    pub const ISIZE_BYTES: usize = std::mem::size_of::<isize>();

    pub const OMAP_ORDER_START_DEFAULT: u64 = 1_000;
    pub const OMAP_ORDER_DELTA_DEFAULT: u64 = 100;

    pub fn try_vek_from(v: Vec<Self>) -> Outcome<Self> {
        Ok(Self::Vek(res!(Vek::try_from(v))))
    }

    /// Normalise the `Dat`icle.  Currently this includes conversion of `Dat::OrdMap` to `Dat::Map`.
    pub fn normalise(self) -> Self {
        match self {
            Dat::OrdMap(map1) => {
                let mut map2 = DaticleMap::new();
                for (k, v) in map1 {
                    map2.insert(k.into_dat(), v);
                }
                Self::Map(map2)
            },
            _ => self,
        }
    }

    pub fn list_get<'a>(&self, ind: usize) -> Option<&Self> {
        if let Dat::List(v) = self {
            if ind < v.len() {
                return Some(&v[ind]);
            }
        }
        None
    }

    pub fn get_string_list(&self) -> Option<Vec<String>> {
        match self {
            Self::List(v) | Self::Vek(Vek(v)) => {
                let mut vecstr = Vec::new();
                for d in v {
                    if let Dat::Str(s) = d {
                        vecstr.push(s.clone());
                    } else {
                        return None;
                    }
                }
                Some(vecstr)
            },
            _ => None,
        }
    }

    pub fn get_b32_string_map(&self) -> Option<BTreeMap<B32, String>> {
        match self {
            Self::Map(map1) => {
                let mut map2 = BTreeMap::new();
                for (kdat, vdat) in map1 {
                    if let Dat::B32(b) = kdat {
                        if let Dat::Str(s) = vdat {
                            map2.insert(B32(**b), s.clone());
                        } else {
                            return None;
                        }
                    } else {
                        return None;
                    }
                }
                Some(map2)
            },
            _ => None,
        }
    }

    /// Pass-through getter for use by [`FromDatMap`] derive macro.
    pub fn get_dat(&self) -> Option<Dat> {
        Some(self.clone())
    }

    const U8_MAX_AS_U16:    u16 = u8::MAX as u16;

    const U8_MAX_AS_U32:    u32 = u8::MAX as u32;
    const U16_MAX_AS_U32:   u32 = u16::MAX as u32;

    const U8_MAX_AS_U64:    u64 = u8::MAX as u64;
    const U16_MAX_AS_U64:   u64 = u16::MAX as u64;
    const U32_MAX_AS_U64:   u64 = u32::MAX as u64;

    const U8_MAX_AS_USIZE:  usize = u8::MAX as usize;
    const U16_MAX_AS_USIZE: usize = u16::MAX as usize;
    const U32_MAX_AS_USIZE: usize = u32::MAX as usize;

    pub fn u16dat(v: u16) -> Self {
        if v < Self::U8_MAX_AS_U16 {
            Dat::U8(v as u8)
        } else {
            Dat::U16(v)
        }
    }

    pub fn u32dat(v: u32) -> Self {
        if v < Self::U8_MAX_AS_U32 {
            Dat::U8(v as u8)
        } else if v < Self::U16_MAX_AS_U32 {
            Dat::U16(v as u16)
        } else {
            Dat::U32(v)
        }
    }

    pub fn u64dat(v: u64) -> Self {
        if v < Self::U8_MAX_AS_U64 {
            Dat::U8(v as u8)
        } else if v < Self::U16_MAX_AS_U64 {
            Dat::U16(v as u16)
        } else if v < Self::U32_MAX_AS_U64 {
            Dat::U32(v as u32)
        } else {
            Dat::U64(v)
        }
    }

    pub fn bytdat(v: Vec<u8>) -> Self {
        let len = v.len();
        if len < Self::U8_MAX_AS_USIZE {
            Dat::BU8(v)
        } else if len < Self::U16_MAX_AS_USIZE {
            Dat::BU16(v)
        } else if len < Self::U32_MAX_AS_USIZE {
            Dat::BU32(v)
        } else {
            Dat::BU64(v)
        }
    }

    pub fn bytes_move(self) -> Option<Vec<u8>> {
        match self {
            Dat::BU8(v)   |
            Dat::BU16(v)  |
            Dat::BU32(v)  |
            Dat::BU64(v)  |
            Dat::BC64(v) => Some(v),
            _ => None,
        }
    }

    pub fn bytes_ref<'a>(&'a self) -> Option<&'a Vec<u8>> {
        match self {
            Dat::BU8(v)   |
            Dat::BU16(v)  |
            Dat::BU32(v)  |
            Dat::BU64(v)  |
            Dat::BC64(v) => Some(v),
            _ => None,
        }
    }
}
