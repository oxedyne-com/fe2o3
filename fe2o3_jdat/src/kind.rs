use crate::{
    prelude::*,
    usr::UsrKindId,
};

use oxedyne_fe2o3_core::prelude::*;

use std::fmt;


/// A lightweight twin of `Dat` without any included data, intended to keep string decoding a
/// bit simpler.  All the variants of `Dat` should be included, with the addition of
/// Kind::None, which allows us to avoid wrapping in an `Option`.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum Kind {
    Unknown, // for id conversion
    // Atomic Kinds ===========================
    // Logic
    Empty,
    True,
    False, 
    None,
    // Fixed
    U8,
    U16,
    U32,
    U64,
    U128,
    I8,
    I16,
    I32,
    I64,
    I128,
    F32,
    F64,
    // Variable
    Aint,
    Adec,
    C64,
    Str,
    // Molecular Kinds ========================
    // Unitary
    Usr(UsrKindId),
    Box(Option<Box<Kind>>),
    Some(Option<Box<Kind>>),
    ABox(Option<Box<Kind>>),
    // Heterogenous
    List,
    Tup2,
    Tup3,
    Tup4,
    Tup5,
    Tup6,
    Tup7,
    Tup8,
    Tup9,
    Tup10,
    Map,
    OrdMap,
    // Homogenous
    Vek,
    // Variable length bytes
    BU8,
    BU16,
    BU32,
    BU64,
    BC64,
    // Fixed length bytes
    B2,
    B3,
    B4,
    B5,
    B6,
    B7,
    B8,
    B9,
    B10,
    B16,
    B32,
    // Variable length numbers
    Vu16, // Not yet implemented
    Vu32, // Not yet implemented
    Vu64, // Not yet implemented
    Vu128, // Not yet implemented
    Vi8, // Not yet implemented
    Vi16, // Not yet implemented
    Vi32, // Not yet implemented
    Vi64, // Not yet implemented
    Vi128, // Not yet implemented
    // Fixed length numbers
    Tup2u16,
    Tup3u16,
    Tup4u16,
    Tup5u16,
    Tup6u16,
    Tup7u16,
    Tup8u16,
    Tup9u16,
    Tup10u16,

    Tup2u32,
    Tup3u32,
    Tup4u32,
    Tup5u32,
    Tup6u32,
    Tup7u32,
    Tup8u32,
    Tup9u32,
    Tup10u32,

    Tup2u64,
    Tup3u64,
    Tup4u64,
    Tup5u64,
    Tup6u64,
    Tup7u64,
    Tup8u64,
    Tup9u64,
    Tup10u64,

    Tup2i8, // Not yet implemented
    Tup3i8, // Not yet implemented
    Tup4i8, // Not yet implemented
    Tup5i8, // Not yet implemented
    Tup6i8, // Not yet implemented
    Tup7i8, // Not yet implemented
    Tup8i8, // Not yet implemented
    Tup9i8, // Not yet implemented
    Tup10i8, // Not yet implemented
             //
    Tup2i16, // Not yet implemented
    Tup3i16, // Not yet implemented
    Tup4i16, // Not yet implemented
    Tup5i16, // Not yet implemented
    Tup6i16, // Not yet implemented
    Tup7i16, // Not yet implemented
    Tup8i16, // Not yet implemented
    Tup9i16, // Not yet implemented
    Tup10i16, // Not yet implemented
              //
    Tup2i32, // Not yet implemented
    Tup3i32, // Not yet implemented
    Tup4i32, // Not yet implemented
    Tup5i32, // Not yet implemented
    Tup6i32, // Not yet implemented
    Tup7i32, // Not yet implemented
    Tup8i32, // Not yet implemented
    Tup9i32, // Not yet implemented
    Tup10i32, // Not yet implemented
              //
    Tup2i64, // Not yet implemented
    Tup3i64, // Not yet implemented
    Tup4i64, // Not yet implemented
    Tup5i64, // Not yet implemented
    Tup6i64, // Not yet implemented
    Tup7i64, // Not yet implemented
    Tup8i64, // Not yet implemented
    Tup9i64, // Not yet implemented
    Tup10i64, // Not yet implemented

    //// Scheduled for removal
    //PartKey,
}

impl Daticle for Dat {

    /// Establish mapping from a `Dat` to a `Kind`.
    fn kind(&self) -> Kind {
        match &*self {
            // Atomic Kinds ===========================
            // Logic
            Self::Empty         => Kind::Empty,
            Self::Bool(b)       => if *b { Kind::True } else { Kind::False },
            // Fixed
            Self::U8(_)         => Kind::U8,
            Self::U16(_)        => Kind::U16,
            Self::U32(_)        => Kind::U32,
            Self::U64(_)        => Kind::U64,
            Self::U128(_)       => Kind::U128,
            Self::I8(_)         => Kind::I8,
            Self::I16(_)        => Kind::I16,
            Self::I32(_)        => Kind::I32,
            Self::I64(_)        => Kind::I64,
            Self::I128(_)       => Kind::I128,
            Self::F32(_)        => Kind::F32,
            Self::F64(_)        => Kind::F64,
            // Variable
            Self::Aint(_)       => Kind::Aint,
            Self::Adec(_)       => Kind::Adec,
            Self::C64(_)        => Kind::C64,
            Self::Str(_)        => Kind::Str,
            // Molecular Kinds ========================
            // Unitary
            Self::Usr(ukid, _optboxd)   => Kind::Usr(ukid.clone()),
            Self::Box(boxd)             => Kind::Box(Some(Box::new((*boxd).kind()))),
            Self::Opt(boxoptd) => match &**boxoptd {
                None => Kind::None,
                Some(d) => Kind::Some(Some(Box::new(d.kind()))),
            },
            Self::ABox(_, boxd, _)      => Kind::ABox(Some(Box::new((*boxd).kind()))),
            // Heterogenous
            Self::List(_)       => Kind::List,
            Self::Tup2(_)       => Kind::Tup2,
            Self::Tup3(_)       => Kind::Tup3,
            Self::Tup4(_)       => Kind::Tup4,
            Self::Tup5(_)       => Kind::Tup5,
            Self::Tup6(_)       => Kind::Tup6,
            Self::Tup7(_)       => Kind::Tup7,
            Self::Tup8(_)       => Kind::Tup8,
            Self::Tup9(_)       => Kind::Tup9,
            Self::Tup10(_)      => Kind::Tup10,
            Self::Map(_)        => Kind::Map,
            Self::OrdMap(_)     => Kind::OrdMap,
            // Homogenous
            Self::Vek(_)        => Kind::Vek,
            // Variable length bytes
            Self::BU8(_)        => Kind::BU8,
            Self::BU16(_)       => Kind::BU16,
            Self::BU32(_)       => Kind::BU32,
            Self::BU64(_)       => Kind::BU64,
            Self::BC64(_)       => Kind::BC64,
            // Fixed length bytes
            Self::B2(_)         => Kind::B2,
            Self::B3(_)         => Kind::B3,
            Self::B4(_)         => Kind::B4,
            Self::B5(_)         => Kind::B5,
            Self::B6(_)         => Kind::B6,
            Self::B7(_)         => Kind::B7,
            Self::B8(_)         => Kind::B8,
            Self::B9(_)         => Kind::B9,
            Self::B10(_)        => Kind::B10,
            Self::B16(_)        => Kind::B16,
            Self::B32(_)        => Kind::B32,

            // Fixed length numbers
            Self::Tup2u16(_)    => Kind::Tup2u16,
            Self::Tup3u16(_)    => Kind::Tup3u16,
            Self::Tup4u16(_)    => Kind::Tup4u16,
            Self::Tup5u16(_)    => Kind::Tup5u16,
            Self::Tup6u16(_)    => Kind::Tup6u16,
            Self::Tup7u16(_)    => Kind::Tup7u16,
            Self::Tup8u16(_)    => Kind::Tup8u16,
            Self::Tup9u16(_)    => Kind::Tup9u16,
            Self::Tup10u16(_)   => Kind::Tup10u16,

            Self::Tup2u32(_)    => Kind::Tup2u32,
            Self::Tup3u32(_)    => Kind::Tup3u32,
            Self::Tup4u32(_)    => Kind::Tup4u32,
            Self::Tup5u32(_)    => Kind::Tup5u32,
            Self::Tup6u32(_)    => Kind::Tup6u32,
            Self::Tup7u32(_)    => Kind::Tup7u32,
            Self::Tup8u32(_)    => Kind::Tup8u32,
            Self::Tup9u32(_)    => Kind::Tup9u32,
            Self::Tup10u32(_)   => Kind::Tup10u32,

            Self::Tup2u64(_)    => Kind::Tup2u64,
            Self::Tup3u64(_)    => Kind::Tup3u64,
            Self::Tup4u64(_)    => Kind::Tup4u64,
            Self::Tup5u64(_)    => Kind::Tup5u64,
            Self::Tup6u64(_)    => Kind::Tup6u64,
            Self::Tup7u64(_)    => Kind::Tup7u64,
            Self::Tup8u64(_)    => Kind::Tup8u64,
            Self::Tup9u64(_)    => Kind::Tup9u64,
            Self::Tup10u64(_)   => Kind::Tup10u64,

            //// Scheduled for removal
            //Self::PartKey(_)    => Kind::PartKey,
        }
    }

}

#[derive(Debug, Eq, PartialEq, Copy, Clone)]
pub enum KindCase {
    None,
    AtomLogic,
    AtomFixed,
    AtomVariable,
    MoleculeUnitary,
    MoleculeMixed,
    MoleculeSame,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KindClass {
    None,
    Atomic,
    Molecular,
}

impl KindCase {
    pub fn class(&self) -> KindClass {
        match self {
            Self::None              => KindClass::None,
            Self::MoleculeUnitary   |
            Self::MoleculeMixed     |
            Self::MoleculeSame      => KindClass::Molecular,
            _ => KindClass::Atomic,
        }
    }

    pub fn accepts_strings(&self) -> bool {
        match self {
            Self::MoleculeUnitary | Self::MoleculeMixed => true,
            _ => false,
        }
    }
}

impl Kind {

    /// Primary classification of `Kind`s.
    pub fn case(&self) -> KindCase {
        match *self {
            Self::Empty     |
            Self::True      |
            Self::False     |
            Self::None      => KindCase::AtomLogic,
            Self::U8        |
            Self::U16       |
            Self::U32       |
            Self::U64       |
            Self::U128      |
            Self::I8        |
            Self::I16       |
            Self::I32       |
            Self::I64       |
            Self::I128      |
            Self::F32       |
            Self::F64       => KindCase::AtomFixed,
            Self::Aint      |
            Self::Adec      |
            Self::C64       |
            Self::Str       => KindCase::AtomVariable,
            Self::Usr(_)    |
            Self::Box(_)    |
            Self::Some(_)   |
            Self::ABox(_)   => KindCase::MoleculeUnitary,
            Self::List      |
            Self::Tup2      |
            Self::Tup3      |
            Self::Tup4      |
            Self::Tup5      |
            Self::Tup6      |
            Self::Tup7      |
            Self::Tup8      |
            Self::Tup9      |
            Self::Tup10     |
            Self::Map       |
            Self::OrdMap    => KindCase::MoleculeMixed,
            Self::BU8       |
            Self::BU16      |
            Self::BU32      |
            Self::BU64      |
            Self::BC64      |
            Self::B32       |
            Self::Tup5u64   => KindCase::MoleculeSame,
            _ => KindCase::None,
        }
    }

    pub fn is_dataless(&self) -> bool {
        if self.case() == KindCase::AtomLogic {
            return true;
        }
        match self {
            Self::Usr(ukid) if ukid.kind().is_none() => true,
            _ => false,
        }
    }

    pub fn is_abox(&self) -> bool {
        match self {
            Self::ABox(_) => true,
            _ => false,
        }
    }

    pub fn is_usr(&self) -> bool {
        match self {
            Self::Usr(_ukid) => true,
            _ => false,
        }
    }

    pub fn is_map(&self) -> bool {
        match self {
            Self::Map | Self::OrdMap => true,
            _ => false,
        }
    }

    pub fn usr(
        &self,
        code: u16,
        lab_opt: Option<&str>,
    )
        -> Self
    {
        Self::Usr(UsrKindId::new(code, lab_opt, Some(self.clone())))
    }

    /// Human description of kinds.
    pub fn desc(&self) -> &'static str {
        match *self {
            // "Which represents..."
            Self::Unknown => "A daticle of unknown kind",
            // Atomic Kinds ===========================
            // Logic
            Self::Empty => "A daticle that is empty",
            Self::True  => "A true boolean value",
            Self::False => "A false boolean value", 
            Self::None  => "The absence of an optional value",
            // Fixed
            Self::U8    => "An unsigned 8 bit integer",
            Self::U16   => "An unsigned 16 bit integer",
            Self::U32   => "An unsigned 32 bit integer",
            Self::U64   => "An unsigned 64 bit integer",
            Self::U128  => "An unsigned 128 bit integer",
            Self::I8    => "A signed 8 bit integer",
            Self::I16   => "A signed 16 bit integer", 
            Self::I32   => "A signed 32 bit integer",
            Self::I64   => "A signed 64 bit integer",
            Self::I128  => "A signed 128 bit integer",
            Self::F32   => "A 32 bit floating point number",
            Self::F64   => "A 64 bit floating point number",
            // Variable
            Self::Aint  => "An integer of arbitrary size",
            Self::Adec  => "A decimal number of arbitary size and precision",
            Self::C64   => "A binary-compressed u64",
            Self::Str   => "A character string",
            // Molecular Kinds ========================
            // Unitary
            Self::Usr(_)    => "A user defined kind with an optionally nested daticle",
            Self::Box(_)    => "A nested daticle",
            Self::Some(_)   => "An optional daticle containing a nested daticle",
            Self::ABox(_)   => "A nested daticle including textual annotation",
            // Heterogenous
            Self::List      => "A heterogenous list of daticles",
            Self::Tup2      => "A heterogenous tuple of two daticles",
            Self::Tup3      => "A heterogenous tuple of three daticles",
            Self::Tup4      => "A heterogenous tuple of four daticles",
            Self::Tup5      => "A heterogenous tuple of five daticles",
            Self::Tup6      => "A heterogenous tuple of six daticles",
            Self::Tup7      => "A heterogenous tuple of seven daticles",
            Self::Tup8      => "A heterogenous tuple of eight daticles",
            Self::Tup9      => "A heterogenous tuple of nine daticles",
            Self::Tup10     => "A heterogenous tuple of ten daticles",
            Self::Map       => "A heterogenous map of daticles",
            Self::OrdMap    => "A heterogenous ordered map of daticles",
            // Homogenous
            Self::Vek   => "An homogenous vector of daticles",
            // Variable length bytes
            Self::BU8   => "A vector of u8 bytes with length encoded as a u8",
            Self::BU16  => "A vector of u8 bytes with length encoded as a u16",
            Self::BU32  => "A vector of u8 bytes with length encoded as a u32",
            Self::BU64  => "A vector of u8 bytes with length encoded as a u64",
            Self::BC64  => "A vector of u8 bytes with length encoded as a binary-compressed u64",
            // Fixed length bytes
            Self::B2    => "A tuple of two u8 bytes",
            Self::B3    => "A tuple of three u8 bytes",
            Self::B4    => "A tuple of four u8 bytes",
            Self::B5    => "A tuple of five u8 bytes",
            Self::B6    => "A tuple of six u8 bytes",
            Self::B7    => "A tuple of seven u8 bytes",
            Self::B8    => "A tuple of eight u8 bytes",
            Self::B9    => "A tuple of nine u8 bytes",
            Self::B10   => "A tuple of ten u8 bytes",
            Self::B16   => "A tuple of sixteen u8 bytes",
            Self::B32   => "A tuple of thirty-two u8 bytes",
            // Variable length numbers
            Self::Vu16  => "A vector of u16 values, not yet implemented",
            Self::Vu32  => "A vector of u32 values, not yet implemented",
            Self::Vu64  => "A vector of u64 values, not yet implemented",
            Self::Vu128 => "A vector of u128 values, not yet implemented",
            Self::Vi8   => "A vector of i8 values, not yet implemented",
            Self::Vi16  => "A vector of i16 values, not yet implemented",
            Self::Vi32  => "A vector of i32 values, not yet implemented",
            Self::Vi64  => "A vector of i64 values, not yet implemented",
            Self::Vi128 => "A vector of i128 values, not yet implemented",
            // Fixed length numbers
            Self::Tup2u16   => "A tuple of two u16 values",
            Self::Tup3u16   => "A tuple of three u16 values",
            Self::Tup4u16   => "A tuple of four u16 values",
            Self::Tup5u16   => "A tuple of five u16 values" ,
            Self::Tup6u16   => "A tuple of six u16 values",
            Self::Tup7u16   => "A tuple of seven u16 values",
            Self::Tup8u16   => "A tuple of eight u16 values",
            Self::Tup9u16   => "A tuple of nine u16 values",
            Self::Tup10u16  => "A tuple of ten u16 values",
        
            Self::Tup2u32   => "A tuple of two u32 values",  
            Self::Tup3u32   => "A tuple of three u32 values",
            Self::Tup4u32   => "A tuple of four u32 values", 
            Self::Tup5u32   => "A tuple of five u32 values" ,
            Self::Tup6u32   => "A tuple of six u32 values",  
            Self::Tup7u32   => "A tuple of seven u32 values",
            Self::Tup8u32   => "A tuple of eight u32 values",
            Self::Tup9u32   => "A tuple of nine u32 values", 
            Self::Tup10u32  => "A tuple of ten u32 values",  
        
            Self::Tup2u64   => "A tuple of two u64 values",  
            Self::Tup3u64   => "A tuple of three u64 values",
            Self::Tup4u64   => "A tuple of four u64 values", 
            Self::Tup5u64   => "A tuple of five u64 values" ,
            Self::Tup6u64   => "A tuple of six u64 values",  
            Self::Tup7u64   => "A tuple of seven u64 values",
            Self::Tup8u64   => "A tuple of eight u64 values",
            Self::Tup9u64   => "A tuple of nine u64 values", 
            Self::Tup10u64  => "A tuple of ten u64 values",  
        
            Self::Tup2i8    => "A tuple of two i8 values, not yet implemented",  
            Self::Tup3i8    => "A tuple of three i8 values, not yet implemented",
            Self::Tup4i8    => "A tuple of four i8 values, not yet implemented", 
            Self::Tup5i8    => "A tuple of five i8 values, not yet implemented" ,
            Self::Tup6i8    => "A tuple of six i8 values, not yet implemented",  
            Self::Tup7i8    => "A tuple of seven i8 values, not yet implemented",
            Self::Tup8i8    => "A tuple of eight i8 values, not yet implemented",
            Self::Tup9i8    => "A tuple of nine i8 values, not yet implemented", 
            Self::Tup10i8   => "A tuple of ten i8 values, not yet implemented",  

            Self::Tup2i16   => "A tuple of two i16 values, not yet implemented",  
            Self::Tup3i16   => "A tuple of three i16 values, not yet implemented",
            Self::Tup4i16   => "A tuple of four i16 values, not yet implemented", 
            Self::Tup5i16   => "A tuple of five i16 values, not yet implemented" ,
            Self::Tup6i16   => "A tuple of six i16 values, not yet implemented",  
            Self::Tup7i16   => "A tuple of seven i16 values, not yet implemented",
            Self::Tup8i16   => "A tuple of eight i16 values, not yet implemented",
            Self::Tup9i16   => "A tuple of nine i16 values, not yet implemented", 
            Self::Tup10i16  => "A tuple of ten i16 values, not yet implemented",  

            Self::Tup2i32   => "A tuple of two i32 values, not yet implemented",  
            Self::Tup3i32   => "A tuple of three i32 values, not yet implemented",
            Self::Tup4i32   => "A tuple of four i32 values, not yet implemented", 
            Self::Tup5i32   => "A tuple of five i32 values, not yet implemented" ,
            Self::Tup6i32   => "A tuple of six i32 values, not yet implemented",  
            Self::Tup7i32   => "A tuple of seven i32 values, not yet implemented",
            Self::Tup8i32   => "A tuple of eight i32 values, not yet implemented",
            Self::Tup9i32   => "A tuple of nine i32 values, not yet implemented", 
            Self::Tup10i32  => "A tuple of ten i32 values, not yet implemented",  

            Self::Tup2i64   => "A tuple of two i64 values, not yet implemented",  
            Self::Tup3i64   => "A tuple of three i64 values, not yet implemented",
            Self::Tup4i64   => "A tuple of four i64 values, not yet implemented", 
            Self::Tup5i64   => "A tuple of five i64 values, not yet implemented" ,
            Self::Tup6i64   => "A tuple of six i64 values, not yet implemented",  
            Self::Tup7i64   => "A tuple of seven i64 values, not yet implemented",
            Self::Tup8i64   => "A tuple of eight i64 values, not yet implemented",
            Self::Tup9i64   => "A tuple of nine i64 values, not yet implemented", 
            Self::Tup10i64  => "A tuple of ten i64 values, not yet implemented",  
        
            //// Scheduled for removal
            //PartKey,
        }
    }

    pub fn is_option(&self) -> bool {
        match self {
            Self::None | Self::Some(_) => true,
            _ => false,
        }
    }

    pub fn equals(&self, other: &Kind) -> bool {
        if self.is_option() && other.is_option() {
            true
        } else {
            self == other
        }
    }
}

impl Dat {

    /// Used mainly for error reporting during byte decoding.
    pub fn code_name(code: u8) -> String {
        fmt!("{:?}", match code {
            // Atomic Kinds ===========================
            // Logic
            Self::EMPTY_CODE    => Kind::Empty,
            Self::TRUE_CODE     => Kind::True,
            Self::FALSE_CODE    => Kind::False,
            Self::OPT_NONE_CODE => Kind::None,
            // Fixed
            Self::U8_CODE       => Kind::U8,
            Self::U16_CODE      => Kind::U16,
            Self::U32_CODE      => Kind::U32,
            Self::U64_CODE      => Kind::U64,
            Self::U128_CODE     => Kind::U128,
            Self::I8_CODE       => Kind::I8,
            Self::I16_CODE      => Kind::I16,
            Self::I32_CODE      => Kind::I32,
            Self::I64_CODE      => Kind::I64,
            Self::I128_CODE     => Kind::I128,
            Self::F32_CODE      => Kind::F32,
            Self::F64_CODE      => Kind::F64,
            // Variable
            Self::AINT_CODE     => Kind::Aint,
            Self::ADEC_CODE     => Kind::Adec,
            Self::C64_CODE_START..=Dat::C64_CODE_END => Kind::C64,
            Self::STR_CODE      => Kind::Str,
            // Molecule Kinds =========================
            // Unitary
            Self::USR_CODE      => Kind::Usr(UsrKindId::new(0, None, None)),
            Self::BOX_CODE      => Kind::Box(None),
            Self::OPT_SOME_CODE => Kind::Some(None),
            // Heterogenous
            Self::LIST_CODE     => Kind::List,
            Self::TUP2_CODE     => Kind::Tup2,
            Self::TUP3_CODE     => Kind::Tup3,
            Self::TUP4_CODE     => Kind::Tup4,
            Self::TUP5_CODE     => Kind::Tup5,
            Self::TUP6_CODE     => Kind::Tup6,
            Self::TUP7_CODE     => Kind::Tup7,
            Self::TUP8_CODE     => Kind::Tup8,
            Self::TUP9_CODE     => Kind::Tup9,
            Self::TUP10_CODE    => Kind::Tup10,
            Self::MAP_CODE      => Kind::Map,
            Self::OMAP_CODE   => Kind::OrdMap,
            // Homogenous
            Self::VEK_CODE      => Kind::Vek,
            // Variable length bytes
            Self::BU8_CODE      => Kind::BU8,
            Self::BU16_CODE     => Kind::BU16,
            Self::BU32_CODE     => Kind::BU32,
            Self::BU64_CODE     => Kind::BU64,
            Self::BC64_CODE     => Kind::BC64,
            // Fixed length bytes
            Self::B2_CODE       => Kind::B2,
            Self::B3_CODE       => Kind::B3,
            Self::B4_CODE       => Kind::B4,
            Self::B5_CODE       => Kind::B5,
            Self::B6_CODE       => Kind::B6,
            Self::B7_CODE       => Kind::B7,
            Self::B8_CODE       => Kind::B8,
            Self::B9_CODE       => Kind::B9,
            Self::B10_CODE      => Kind::B10,
            Self::B16_CODE      => Kind::B16,
            Self::B32_CODE      => Kind::B32,
            // Fixed length numbers
            Self::TUP2_U16_CODE     => Kind::Tup2u16,
            Self::TUP3_U16_CODE     => Kind::Tup3u16,
            Self::TUP4_U16_CODE     => Kind::Tup4u16,
            Self::TUP5_U16_CODE     => Kind::Tup5u16,
            Self::TUP6_U16_CODE     => Kind::Tup6u16,
            Self::TUP7_U16_CODE     => Kind::Tup7u16,
            Self::TUP8_U16_CODE     => Kind::Tup8u16,
            Self::TUP9_U16_CODE     => Kind::Tup9u16,
            Self::TUP10_U16_CODE    => Kind::Tup10u16,

            Self::TUP2_U32_CODE     => Kind::Tup2u32,
            Self::TUP3_U32_CODE     => Kind::Tup3u32,
            Self::TUP4_U32_CODE     => Kind::Tup4u32,
            Self::TUP5_U32_CODE     => Kind::Tup5u32,
            Self::TUP6_U32_CODE     => Kind::Tup6u32,
            Self::TUP7_U32_CODE     => Kind::Tup7u32,
            Self::TUP8_U32_CODE     => Kind::Tup8u32,
            Self::TUP9_U32_CODE     => Kind::Tup9u32,
            Self::TUP10_U32_CODE    => Kind::Tup10u32,

            Self::TUP2_U64_CODE     => Kind::Tup2u64,
            Self::TUP3_U64_CODE     => Kind::Tup3u64,
            Self::TUP4_U64_CODE     => Kind::Tup4u64,
            Self::TUP5_U64_CODE     => Kind::Tup5u64,
            Self::TUP6_U64_CODE     => Kind::Tup6u64,
            Self::TUP7_U64_CODE     => Kind::Tup7u64,
            Self::TUP8_U64_CODE     => Kind::Tup8u64,
            Self::TUP9_U64_CODE     => Kind::Tup9u64,
            Self::TUP10_U64_CODE    => Kind::Tup10u64,

            //// Scheduled for removal
            //Self::PARTKEY_CODE  => fmt!("Dat::PartKey"),
            
            _   => Kind::Unknown,
        })
    }

    #[allow(dead_code)]
    pub fn to_code(&self) -> u8 {
        match self {
            // Atomic Kinds ===========================
            // Logic
            Self::Empty     => Self::EMPTY_CODE,
            Self::Bool(b)   => if *b { Self::TRUE_CODE } else { Self::FALSE_CODE },
            // Fixed
            Self::U8(_)     => Self::U8_CODE,
            Self::U16(_)    => Self::U16_CODE,
            Self::U32(_)    => Self::U32_CODE,
            Self::U64(_)    => Self::U64_CODE,
            Self::U128(_)   => Self::U128_CODE,
            Self::I8(_)     => Self::I8_CODE,
            Self::I16(_)    => Self::I16_CODE,
            Self::I32(_)    => Self::I32_CODE,
            Self::I64(_)    => Self::I64_CODE,
            Self::I128(_)   => Self::I128_CODE,
            Self::F32(_)    => Self::F32_CODE,
            Self::F64(_)    => Self::F64_CODE,
            // Variable
            Self::Aint(_)   => Self::AINT_CODE,
            Self::Adec(_)   => Self::ADEC_CODE,
            Self::C64(_)   => Self::C64_CODE_START, // baseline value, c64 = 0
            //                 C64_CODE_START+1: 0 < c64 <= 255
            //                 C64_CODE_START+2: 255 < c64 <= 65535
            //                 C64_CODE_START+3: 65535 < c64 <= 16777215
            //                 C64_CODE_START+4: 16777215 < c64 <= 4294967295
            //                 C64_CODE_START+5: 4294967295 < c64 <= 1099511627775
            //                 C64_CODE_START+6: 1099511627775 < c64 <= 281474976710655
            //                 C64_CODE_START+7: 281474976710655 < c64 <= 72057594037927935
            //                 C64_CODE_START+8: 72057594037927935 < c64 <= 18446744073709551615
            Self::Str(_)    => Self::STR_CODE,
            // Molecular Kinds ========================
            // Unitary
            Self::Usr(_, _)     => Self::USR_CODE,
            Self::Box(_)        => Self::BOX_CODE,
            Self::Opt(boxoptd) => match **boxoptd {
                None    => Self::OPT_NONE_CODE,
                Some(_) => Self::OPT_SOME_CODE,
            },
            Self::ABox(_, _, _)  => Self::ABOX_CODE,
            // Heterogenous
            Self::List(_)   => Self::LIST_CODE,
            Self::Tup2(_)   => Self::TUP2_CODE,
            Self::Tup3(_)   => Self::TUP3_CODE,
            Self::Tup4(_)   => Self::TUP4_CODE,
            Self::Tup5(_)   => Self::TUP5_CODE,
            Self::Tup6(_)   => Self::TUP6_CODE,
            Self::Tup7(_)   => Self::TUP7_CODE,
            Self::Tup8(_)   => Self::TUP8_CODE,
            Self::Tup9(_)   => Self::TUP9_CODE,
            Self::Tup10(_)  => Self::TUP10_CODE,
            Self::Map(_)    => Self::MAP_CODE,
            Self::OrdMap(_) => Self::OMAP_CODE,
            // Homogenous
            Self::Vek(_)    => Self::VEK_CODE,
            // Variable length bytes
            Self::BU8(_)    => Self::BU8_CODE,
            Self::BU16(_)   => Self::BU16_CODE,
            Self::BU32(_)   => Self::BU32_CODE,
            Self::BU64(_)   => Self::BU64_CODE,
            Self::BC64(_)   => Self::BC64_CODE,
            // Fixed length bytes
            Self::B2(_)     => Self::B2_CODE,
            Self::B3(_)     => Self::B3_CODE,
            Self::B4(_)     => Self::B4_CODE,
            Self::B5(_)     => Self::B5_CODE,
            Self::B6(_)     => Self::B6_CODE,
            Self::B7(_)     => Self::B7_CODE,
            Self::B8(_)     => Self::B8_CODE,
            Self::B9(_)     => Self::B9_CODE,
            Self::B10(_)    => Self::B10_CODE,
            Self::B16(_)    => Self::B16_CODE,
            Self::B32(_)    => Self::B32_CODE,
            // Fixed length numbers
            Self::Tup2u16(_)    => Self::TUP2_U16_CODE,
            Self::Tup3u16(_)    => Self::TUP3_U16_CODE,
            Self::Tup4u16(_)    => Self::TUP4_U16_CODE,
            Self::Tup5u16(_)    => Self::TUP5_U16_CODE,
            Self::Tup6u16(_)    => Self::TUP6_U16_CODE,
            Self::Tup7u16(_)    => Self::TUP7_U16_CODE,
            Self::Tup8u16(_)    => Self::TUP8_U16_CODE,
            Self::Tup9u16(_)    => Self::TUP9_U16_CODE,
            Self::Tup10u16(_)   => Self::TUP10_U16_CODE,

            Self::Tup2u32(_)    => Self::TUP2_U32_CODE,
            Self::Tup3u32(_)    => Self::TUP3_U32_CODE,
            Self::Tup4u32(_)    => Self::TUP4_U32_CODE,
            Self::Tup5u32(_)    => Self::TUP5_U32_CODE,
            Self::Tup6u32(_)    => Self::TUP6_U32_CODE,
            Self::Tup7u32(_)    => Self::TUP7_U32_CODE,
            Self::Tup8u32(_)    => Self::TUP8_U32_CODE,
            Self::Tup9u32(_)    => Self::TUP9_U32_CODE,
            Self::Tup10u32(_)   => Self::TUP10_U32_CODE,

            Self::Tup2u64(_)    => Self::TUP2_U64_CODE,
            Self::Tup3u64(_)    => Self::TUP3_U64_CODE,
            Self::Tup4u64(_)    => Self::TUP4_U64_CODE,
            Self::Tup5u64(_)    => Self::TUP5_U64_CODE,
            Self::Tup6u64(_)    => Self::TUP6_U64_CODE,
            Self::Tup7u64(_)    => Self::TUP7_U64_CODE,
            Self::Tup8u64(_)    => Self::TUP8_U64_CODE,
            Self::Tup9u64(_)    => Self::TUP9_U64_CODE,
            Self::Tup10u64(_)   => Self::TUP10_U64_CODE,
            
            //// Scheduled for removal
            //Self::PartKey(_)    => Self::PARTKEY_CODE,
        }
    }

    /// Used by `IterDat`.
    pub fn must_iterdat_flatten(&self) -> bool {
        match self {
            // Atomic Kinds ===========================
            // Logic
            Self::Empty     |
            Self::Bool(_)   |
            // Fixed
            Self::U8(_)     |
            Self::U16(_)    |
            Self::U32(_)    |
            Self::U64(_)    |
            Self::U128(_)   |
            Self::I8(_)     |
            Self::I16(_)    |
            Self::I32(_)    |
            Self::I64(_)    |
            Self::I128(_)   |
            Self::F32(_)    |
            Self::F64(_)    |
            // Variable
            Self::Aint(_)   |
            Self::Adec(_)   |
            Self::C64(_)    |
            Self::Str(_)    |
            // Molecular Kinds ========================
            // Unitary
            Self::Usr(_, _) |
            Self::Box(_)    |
            Self::Opt(_)    => false,
            _ => true,
        }
    }

    /// Used by `IterDatValsMut`.
    pub fn must_iterdatvalsmut_flatten(&self) -> bool {
        match self {
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
            Self::OrdMap(_) |
            // Homogenous
            Self::Vek(_)    => true,
            _ => false,
        }
    }
}

impl Default for Kind {
    fn default() -> Self {
        Self::Unknown
    }
}

impl fmt::Display for Kind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unknown   => f.write_str("unknown"),
            // Atomic Kinds ===========================
            // Logic
            Self::Empty     => f.write_str("empty"),
            //Self::Bool => f.write_str("BOOL"),
            Self::True      => f.write_str("true"),
            Self::False     => f.write_str("false"),
            Self::None      => f.write_str("none"),
            // Fixed
            Self::U8        => f.write_str("u8"),
            Self::U16       => f.write_str("u16"),
            Self::U32       => f.write_str("u32"),
            Self::U64       => f.write_str("u64"),
            Self::U128      => f.write_str("u128"),
            Self::I8        => f.write_str("i8"),
            Self::I16       => f.write_str("i16"),
            Self::I32       => f.write_str("i32"),
            Self::I64       => f.write_str("i64"),
            Self::I128      => f.write_str("i128"),
            Self::F32       => f.write_str("f32"),
            Self::F64       => f.write_str("f64"),
            // Variable
            Self::Aint      => f.write_str("aint"),
            Self::Adec      => f.write_str("adec"),
            Self::C64       => f.write_str("c64"),
            Self::Str       => f.write_str("str"),
            // Molecule Kinds =========================
            // Unitary
            Self::Usr(ukid) => write!(f, "usr({})", ukid.label()),
            Self::Box(_)    => f.write_str("box"),
            Self::Some(_)   => f.write_str("some"),
            Self::ABox(_)   => f.write_str("abox"),
            // Heterogenous
            Self::List      => f.write_str("list"),
            Self::Tup2      => f.write_str("t2"),
            Self::Tup3      => f.write_str("t3"),
            Self::Tup4      => f.write_str("t4"),
            Self::Tup5      => f.write_str("t5"),
            Self::Tup6      => f.write_str("t6"),
            Self::Tup7      => f.write_str("t7"),
            Self::Tup8      => f.write_str("t8"),
            Self::Tup9      => f.write_str("t9"),
            Self::Tup10     => f.write_str("t10"),
            Self::Map       => f.write_str("map"),
            Self::OrdMap    => f.write_str("omap"),
            // Homogenous
            Self::Vek       => f.write_str("vek"),
            // Variable length bytes - length itself is encoded as a u8, u16, ...
            Self::BU8       => f.write_str("bu8"),
            Self::BU16      => f.write_str("bu16"),
            Self::BU32      => f.write_str("bu32"),
            Self::BU64      => f.write_str("bu64"),
            Self::BC64      => f.write_str("bc64"),
            // Fixed length bytes
            Self::B2        => f.write_str("b2"),
            Self::B3        => f.write_str("b3"),
            Self::B4        => f.write_str("b4"),
            Self::B5        => f.write_str("b5"),
            Self::B6        => f.write_str("b6"),
            Self::B7        => f.write_str("b7"),
            Self::B8        => f.write_str("b8"),
            Self::B9        => f.write_str("b9"),
            Self::B10       => f.write_str("b10"),
            Self::B16       => f.write_str("b16"),
            Self::B32       => f.write_str("b32"),
            // Variable length numbers
            Self::Vu16      => f.write_str("vu16"),
            Self::Vu32      => f.write_str("vu32"),
            Self::Vu64      => f.write_str("vu64"),
            Self::Vu128     => f.write_str("vu128"),
            Self::Vi8       => f.write_str("vi8"),
            Self::Vi16      => f.write_str("vi16"),
            Self::Vi32      => f.write_str("vi32"),
            Self::Vi64      => f.write_str("vi64"),
            Self::Vi128     => f.write_str("vi128"),
            // Fixed length numbers
            // u16 tuples
            Self::Tup2u16   => f.write_str("t2u16"),
            Self::Tup3u16   => f.write_str("t3u16"),
            Self::Tup4u16   => f.write_str("t4u16"),
            Self::Tup5u16   => f.write_str("t5u16"),
            Self::Tup6u16   => f.write_str("t6u16"),
            Self::Tup7u16   => f.write_str("t7u16"),
            Self::Tup8u16   => f.write_str("t8u16"),
            Self::Tup9u16   => f.write_str("t9u16"),
            Self::Tup10u16  => f.write_str("t10u16"),
            // u32 tuples
            Self::Tup2u32   => f.write_str("t2u32"),
            Self::Tup3u32   => f.write_str("t3u32"),
            Self::Tup4u32   => f.write_str("t4u32"),
            Self::Tup5u32   => f.write_str("t5u32"),
            Self::Tup6u32   => f.write_str("t6u32"),
            Self::Tup7u32   => f.write_str("t7u32"),
            Self::Tup8u32   => f.write_str("t8u32"),
            Self::Tup9u32   => f.write_str("t9u32"),
            Self::Tup10u32  => f.write_str("t10u32"),
            // u64 tuples
            Self::Tup2u64   => f.write_str("t2u64"),
            Self::Tup3u64   => f.write_str("t3u64"),
            Self::Tup4u64   => f.write_str("t4u64"),
            Self::Tup5u64   => f.write_str("t5u64"),
            Self::Tup6u64   => f.write_str("t6u64"),
            Self::Tup7u64   => f.write_str("t7u64"),
            Self::Tup8u64   => f.write_str("t8u64"),
            Self::Tup9u64   => f.write_str("t9u64"),
            Self::Tup10u64  => f.write_str("t10u64"),
            // i8 tuples
            Self::Tup2i8   => f.write_str("t2i8"),
            Self::Tup3i8   => f.write_str("t3i8"),
            Self::Tup4i8   => f.write_str("t4i8"),
            Self::Tup5i8   => f.write_str("t5i8"),
            Self::Tup6i8   => f.write_str("t6i8"),
            Self::Tup7i8   => f.write_str("t7i8"),
            Self::Tup8i8   => f.write_str("t8i8"),
            Self::Tup9i8   => f.write_str("t9i8"),
            Self::Tup10i8  => f.write_str("t10i8"),
            // i16 tuples
            Self::Tup2i16   => f.write_str("t2i16"),
            Self::Tup3i16   => f.write_str("t3i16"),
            Self::Tup4i16   => f.write_str("t4i16"),
            Self::Tup5i16   => f.write_str("t5i16"),
            Self::Tup6i16   => f.write_str("t6i16"),
            Self::Tup7i16   => f.write_str("t7i16"),
            Self::Tup8i16   => f.write_str("t8i16"),
            Self::Tup9i16   => f.write_str("t9i16"),
            Self::Tup10i16  => f.write_str("t10i16"),
            // i32 tuples
            Self::Tup2i32   => f.write_str("t2i32"),
            Self::Tup3i32   => f.write_str("t3i32"),
            Self::Tup4i32   => f.write_str("t4i32"),
            Self::Tup5i32   => f.write_str("t5i32"),
            Self::Tup6i32   => f.write_str("t6i32"),
            Self::Tup7i32   => f.write_str("t7i32"),
            Self::Tup8i32   => f.write_str("t8i32"),
            Self::Tup9i32   => f.write_str("t9i32"),
            Self::Tup10i32  => f.write_str("t10i32"),
            // i64 tuples
            Self::Tup2i64   => f.write_str("t2i64"),
            Self::Tup3i64   => f.write_str("t3i64"),
            Self::Tup4i64   => f.write_str("t4i64"),
            Self::Tup5i64   => f.write_str("t5i64"),
            Self::Tup6i64   => f.write_str("t6i64"),
            Self::Tup7i64   => f.write_str("t7i64"),
            Self::Tup8i64   => f.write_str("t8i64"),
            Self::Tup9i64   => f.write_str("t9i64"),
            Self::Tup10i64  => f.write_str("t10i64"),
        }
    }
}

impl FromStr for Kind {
    type Err = Error<ErrTag>;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            // Atomic Kinds ===========================
            // Logic
            "empty"     => Ok(Self::Empty),
            "true"      => Ok(Self::True),
            "false"     => Ok(Self::False),
            "none"      => Ok(Self::None),
            // Fixed
            "u8"        => Ok(Self::U8),
            "u16"       => Ok(Self::U16),
            "u32"       => Ok(Self::U32),
            "u64"       => Ok(Self::U64),
            "u128"      => Ok(Self::U128),
            "i8"        => Ok(Self::I8),
            "i16"       => Ok(Self::I16),
            "i32"       => Ok(Self::I32),
            "i64"       => Ok(Self::I64),
            "i128"      => Ok(Self::I128),
            "f32"       => Ok(Self::F32),
            "f64"       => Ok(Self::F64),
            // Variable
            "aint"      => Ok(Self::Aint),
            "adec"      => Ok(Self::Adec),
            "c64"       => Ok(Self::C64),
            "str"       => Ok(Self::Str),
            // Molecule Kinds =========================
            // Unitary
            "box"       => Ok(Self::Box(None)),
            "some"      => Ok(Self::Some(None)),
            "abox"      => Ok(Self::ABox(None)),
            // Heterogenous
            "list"      => Ok(Self::List),
            "t2"        => Ok(Self::Tup2),
            "t3"        => Ok(Self::Tup3),
            "t4"        => Ok(Self::Tup4),
            "t5"        => Ok(Self::Tup5),
            "t6"        => Ok(Self::Tup6),
            "t7"        => Ok(Self::Tup7),
            "t8"        => Ok(Self::Tup8),
            "t9"        => Ok(Self::Tup9),
            "t10"       => Ok(Self::Tup10),
            "map"       => Ok(Self::Map),
            "omap"      => Ok(Self::OrdMap),
            // Homogenous
            "vek"       => Ok(Self::Vek),
            // Variable length bytes
            "bu8"       => Ok(Self::BU8),
            "bu16"      => Ok(Self::BU16),
            "bu32"      => Ok(Self::BU32),
            "bu64"      => Ok(Self::BU64),
            "bc64"      => Ok(Self::BC64),
            // Fixed length bytes
            "b2"        => Ok(Self::B2),
            "b3"        => Ok(Self::B3),
            "b4"        => Ok(Self::B4),
            "b5"        => Ok(Self::B5),
            "b6"        => Ok(Self::B6),
            "b7"        => Ok(Self::B7),
            "b8"        => Ok(Self::B8),
            "b9"        => Ok(Self::B9),
            "b10"       => Ok(Self::B10),
            "b16"       => Ok(Self::B16),
            "b32"       => Ok(Self::B32),
            // Fixed length numbers
            "t2u16"     => Ok(Self::Tup2u16),
            "t3u16"     => Ok(Self::Tup3u16),
            "t4u16"     => Ok(Self::Tup4u16),
            "t5u16"     => Ok(Self::Tup5u16),
            "t6u16"     => Ok(Self::Tup6u16),
            "t7u16"     => Ok(Self::Tup7u16),
            "t8u16"     => Ok(Self::Tup8u16),
            "t9u16"     => Ok(Self::Tup9u16),
            "t10u16"    => Ok(Self::Tup10u16),
            "t2u32"     => Ok(Self::Tup2u32),
            "t3u32"     => Ok(Self::Tup3u32),
            "t4u32"     => Ok(Self::Tup4u32),
            "t5u32"     => Ok(Self::Tup5u32),
            "t6u32"     => Ok(Self::Tup6u32),
            "t7u32"     => Ok(Self::Tup7u32),
            "t8u32"     => Ok(Self::Tup8u32),
            "t9u32"     => Ok(Self::Tup9u32),
            "t10u32"    => Ok(Self::Tup10u32),
            "t2u64"     => Ok(Self::Tup2u64),
            "t3u64"     => Ok(Self::Tup3u64),
            "t4u64"     => Ok(Self::Tup4u64),
            "t5u64"     => Ok(Self::Tup5u64),
            "t6u64"     => Ok(Self::Tup6u64),
            "t7u64"     => Ok(Self::Tup7u64),
            "t8u64"     => Ok(Self::Tup8u64),
            "t9u64"     => Ok(Self::Tup9u64),
            "t10u64"    => Ok(Self::Tup10u64),

            _ => Err(err!(
                "Daticle kind label not recognised as standard: '{}'", s;
            Input, String, Unknown)),
        }
    }
}

