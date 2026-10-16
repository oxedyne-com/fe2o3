use crate::{
    prelude::*,
    int::DatInt,
    usr::UsrKindId,
};

use oxedyne_fe2o3_core::{
    prelude::*,
    error::ErrWire,
};
use oxedyne_fe2o3_num::{
    float::{
        Float32,
        Float64,
    },
};

use std::{
    collections::BTreeMap,
    convert::TryFrom,
    time::Duration,
};

use bigdecimal::BigDecimal;
use num_bigint::BigInt;


pub trait FromDatMap : Default {
    fn from_datmap(map: DaticleMap) -> Outcome<Self>;
}

pub trait ToDatMap : Default {
    fn to_datmap(input_struct: Self) -> Dat;
}

pub trait ToDat {
    fn to_dat(&self) -> Outcome<Dat>;

    /// How a slice of this type is encoded, so that `Vec<T>` can have one impl: a list by
    /// default, bytes for `u8`.
    fn slice_to_dat(items: &[Self]) -> Outcome<Dat> where Self: Sized {
        let mut list = Vec::with_capacity(items.len());
        for (i, item) in items.iter().enumerate() {
            match item.to_dat() {
                Ok(dat) => list.push(dat),
                Err(e)  => return Err(err!(e, "Encoding item {} of a list.", i; Encode)),
            }
        }
        Ok(Dat::List(list))
    }
}

pub trait IntoDat: ToDat {
    fn into_dat(self) -> Outcome<Dat> where Self: Sized {
        self.to_dat()
    }
}

pub trait FromDat {
    fn from_dat(dat: Dat) -> Outcome<Self> where Self: Sized;

    /// The counterpart of `ToDat::slice_to_dat`.
    fn vec_from_dat(dat: Dat) -> Outcome<Vec<Self>> where Self: Sized {
        let list = match dat {
            Dat::List(list)     => list,
            Dat::Vek(Vek(list)) => list,
            other               => return Err(err!(
                "A list was expected, a {:?} was found.", other.kind();
            Decode, Mismatch)),
        };
        let mut out = Vec::with_capacity(list.len());
        for (i, dat) in list.into_iter().enumerate() {
            match Self::from_dat(dat) {
                Ok(v)   => out.push(v),
                Err(e)  => return Err(err!(e, "Decoding item {} of a list.", i; Decode)),
            }
        }
        Ok(out)
    }
}

impl AsRef<Dat> for &Dat {
    fn as_ref(&self) -> &Dat {
        *self
    }
}

impl ToDat for Dat {
    fn to_dat(&self) -> Outcome<Dat> {
        Ok(self.clone())
    }
}

impl FromDat for Dat {
    fn from_dat(dat: Dat) -> Outcome<Self> {
        Ok(dat)
    }
}

// From, ToDat and FromDat conversions for native and fundamental types.
//
// Atomic Kinds ===========================
// Logic
to_from_dat! { bool,    Bool    }
// Fixed
to_from_dat! { u16,     U16     }
to_from_dat! { u32,     U32     }
to_from_dat! { u64,     U64     }
to_from_dat! { u128,    U128    }
to_from_dat! { i8,      I8      }
to_from_dat! { i16,     I16     }
to_from_dat! { i32,     I32     }
to_from_dat! { i64,     I64     }
to_from_dat! { i128,    I128    }
to_from_dat! { Float32, F32     }
to_from_dat! { Float64, F64     }
// Variable
to_from_dat! { BigInt,      Aint    }
to_from_dat! { BigDecimal,  Adec    }
to_from_dat! { String,      Str     }
// Molecule Kinds =========================
// Unitary
// Heterogenous
to_from_dat_boxed!  { [Dat;  2],    Tup2    }
to_from_dat_boxed!  { [Dat;  3],    Tup3    }
to_from_dat_boxed!  { [Dat;  4],    Tup4    }
to_from_dat_boxed!  { [Dat;  5],    Tup5    }
to_from_dat_boxed!  { [Dat;  6],    Tup6    }
to_from_dat_boxed!  { [Dat;  7],    Tup7    }
to_from_dat_boxed!  { [Dat;  8],    Tup8    }
to_from_dat_boxed!  { [Dat;  9],    Tup9    }
to_from_dat_boxed!  { [Dat; 10],    Tup10   }
// Homogenous
// Variable length bytes
// Fixed length bytes
to_from_dat! { [u8; 2],     B2  }
to_from_dat! { [u8; 3],     B3  }
to_from_dat! { [u8; 4],     B4  }
to_from_dat! { [u8; 5],     B5  }
to_from_dat! { [u8; 6],     B6  }
to_from_dat! { [u8; 7],     B7  }
to_from_dat! { [u8; 8],     B8  }
to_from_dat! { [u8; 9],     B9  }
to_from_dat! { [u8; 10],    B10 }
to_from_dat! { [u8; 16],    B16 }
to_from_dat! { B32,         B32 }
impl From<[u8; 32]> for Dat {
    fn from(x: [u8; 32]) -> Self {
        Dat::B32(B32(x))
    }
}
// Fixed length numbers

to_from_dat! { [u16; 2],    Tup2u16     }
to_from_dat! { [u16; 3],    Tup3u16     }
to_from_dat! { [u16; 4],    Tup4u16     }
to_from_dat! { [u16; 5],    Tup5u16     }
to_from_dat! { [u16; 6],    Tup6u16     }
to_from_dat! { [u16; 7],    Tup7u16     }
to_from_dat! { [u16; 8],    Tup8u16     }
to_from_dat! { [u16; 9],    Tup9u16     }
to_from_dat! { [u16; 10],   Tup10u16    }

to_from_dat! { [u32; 2],    Tup2u32     }
to_from_dat! { [u32; 3],    Tup3u32     }
to_from_dat! { [u32; 4],    Tup4u32     }
to_from_dat! { [u32; 5],    Tup5u32     }
to_from_dat! { [u32; 6],    Tup6u32     }
to_from_dat! { [u32; 7],    Tup7u32     }
to_from_dat! { [u32; 8],    Tup8u32     }
to_from_dat! { [u32; 9],    Tup9u32     }
to_from_dat! { [u32; 10],   Tup10u32    }

to_from_dat! { [u64; 2],    Tup2u64     }
to_from_dat! { [u64; 3],    Tup3u64     }
to_from_dat! { [u64; 4],    Tup4u64     }
to_from_dat! { [u64; 5],    Tup5u64     }
to_from_dat! { [u64; 6],    Tup6u64     }
to_from_dat! { [u64; 7],    Tup7u64     }
to_from_dat! { [u64; 8],    Tup8u64     }
to_from_dat! { [u64; 9],    Tup9u64     }
to_from_dat! { [u64; 10],   Tup10u64    }

to_from_dat! { [i8; 2],     Tup2i8      }
to_from_dat! { [i8; 3],     Tup3i8      }
to_from_dat! { [i8; 4],     Tup4i8      }
to_from_dat! { [i8; 5],     Tup5i8      }
to_from_dat! { [i8; 6],     Tup6i8      }
to_from_dat! { [i8; 7],     Tup7i8      }
to_from_dat! { [i8; 8],     Tup8i8      }
to_from_dat! { [i8; 9],     Tup9i8      }
to_from_dat! { [i8; 10],    Tup10i8     }

to_from_dat! { [i16; 2],    Tup2i16     }
to_from_dat! { [i16; 3],    Tup3i16     }
to_from_dat! { [i16; 4],    Tup4i16     }
to_from_dat! { [i16; 5],    Tup5i16     }
to_from_dat! { [i16; 6],    Tup6i16     }
to_from_dat! { [i16; 7],    Tup7i16     }
to_from_dat! { [i16; 8],    Tup8i16     }
to_from_dat! { [i16; 9],    Tup9i16     }
to_from_dat! { [i16; 10],   Tup10i16    }

to_from_dat! { [i32; 2],    Tup2i32     }
to_from_dat! { [i32; 3],    Tup3i32     }
to_from_dat! { [i32; 4],    Tup4i32     }
to_from_dat! { [i32; 5],    Tup5i32     }
to_from_dat! { [i32; 6],    Tup6i32     }
to_from_dat! { [i32; 7],    Tup7i32     }
to_from_dat! { [i32; 8],    Tup8i32     }
to_from_dat! { [i32; 9],    Tup9i32     }
to_from_dat! { [i32; 10],   Tup10i32    }

to_from_dat! { [i64; 2],    Tup2i64     }
to_from_dat! { [i64; 3],    Tup3i64     }
to_from_dat! { [i64; 4],    Tup4i64     }
to_from_dat! { [i64; 5],    Tup5i64     }
to_from_dat! { [i64; 6],    Tup6i64     }
to_from_dat! { [i64; 7],    Tup7i64     }
to_from_dat! { [i64; 8],    Tup8i64     }
to_from_dat! { [i64; 9],    Tup9i64     }
to_from_dat! { [i64; 10],   Tup10i64    }

impl From<Vec<String>> for Dat {
    fn from(v: Vec<String>) -> Self {
        let mut vd = Vec::new();
        for d in v {
            vd.push(dat!(d));
        }
        Dat::Vek(Vek(vd))
    }
}

// Conversions for standard library compound types.

impl ToDat for Duration {
    fn to_dat(&self) -> Outcome<Dat> {
        Ok(Dat::U64(self.as_nanos() as u64))
    }
}

impl FromDat for Duration {
    fn from_dat(dat: Dat) -> Outcome<Self> {
        Ok(Self::from_nanos(try_extract_dat!(dat, U64)))
    }
}

best_from_int_to_dat! { u8      }
best_from_int_to_dat! { u16     }
best_from_int_to_dat! { u32     }
best_from_int_to_dat! { u64     }
best_from_int_to_dat! { u128    }
best_from_int_to_dat! { i8      }
best_from_int_to_dat! { i16     }
best_from_int_to_dat! { i32     }
best_from_int_to_dat! { i64     }
best_from_int_to_dat! { i128    }

// The BestFrom trait defaults to From, so implement this default for all the following types.
// Atomic Kinds ===========================
// Logic
impl BestFrom<bool> for Dat {}
// Fixed
impl BestFrom<Float32> for Dat {}
impl BestFrom<Float64> for Dat {}
// Variable
impl BestFrom<BigInt> for Dat {}
impl BestFrom<BigDecimal> for Dat {}
impl BestFrom<String> for Dat {}
// Molecule Kinds =========================
// Unitary
// Heterogenous
impl BestFrom<Vec<Dat>> for Dat {}
impl BestFrom<[Dat; 2]> for Dat {}
impl BestFrom<[Dat; 3]> for Dat {}
impl BestFrom<[Dat; 4]> for Dat {}
impl BestFrom<[Dat; 5]> for Dat {}
impl BestFrom<[Dat; 6]> for Dat {}
impl BestFrom<[Dat; 7]> for Dat {}
impl BestFrom<[Dat; 8]> for Dat {}
impl BestFrom<[Dat; 9]> for Dat {}
impl BestFrom<[Dat; 10]> for Dat {}
// Homogenous
// Variable length bytes
impl BestFrom<Vec<u8>> for Dat {}
// Fixed length bytes
impl BestFrom<[u8; 2]> for Dat {}
impl BestFrom<[u8; 3]> for Dat {}
impl BestFrom<[u8; 4]> for Dat {}
impl BestFrom<[u8; 5]> for Dat {}
impl BestFrom<[u8; 6]> for Dat {}
impl BestFrom<[u8; 7]> for Dat {}
impl BestFrom<[u8; 8]> for Dat {}
impl BestFrom<[u8; 9]> for Dat {}
impl BestFrom<[u8; 10]> for Dat {}
impl BestFrom<[u8; 16]> for Dat {}
impl BestFrom<B32> for Dat {}
impl BestFrom<[u8; 32]> for Dat {}
// Fixed length numbers
impl BestFrom<[u16; 2]> for Dat {}
impl BestFrom<[u16; 3]> for Dat {}
impl BestFrom<[u16; 4]> for Dat {}
impl BestFrom<[u16; 5]> for Dat {}
impl BestFrom<[u16; 6]> for Dat {}
impl BestFrom<[u16; 7]> for Dat {}
impl BestFrom<[u16; 8]> for Dat {}
impl BestFrom<[u16; 9]> for Dat {}
impl BestFrom<[u16; 10]> for Dat {}

impl BestFrom<[u32; 2]> for Dat {}
impl BestFrom<[u32; 3]> for Dat {}
impl BestFrom<[u32; 4]> for Dat {}
impl BestFrom<[u32; 5]> for Dat {}
impl BestFrom<[u32; 6]> for Dat {}
impl BestFrom<[u32; 7]> for Dat {}
impl BestFrom<[u32; 8]> for Dat {}
impl BestFrom<[u32; 9]> for Dat {}
impl BestFrom<[u32; 10]> for Dat {}

impl BestFrom<[u64; 2]> for Dat {}
impl BestFrom<[u64; 3]> for Dat {}
impl BestFrom<[u64; 4]> for Dat {}
impl BestFrom<[u64; 5]> for Dat {}
impl BestFrom<[u64; 6]> for Dat {}
impl BestFrom<[u64; 7]> for Dat {}
impl BestFrom<[u64; 8]> for Dat {}
impl BestFrom<[u64; 9]> for Dat {}
impl BestFrom<[u64; 10]> for Dat {}

impl From<()> for Dat {
    fn from(_: ()) -> Self {
        Self::Empty
    }
}

impl BestFrom<()> for Dat {}

impl TryFrom<usize> for Dat {
    type Error = Error<ErrTag>;

    fn try_from(n: usize) -> std::result::Result<Self, Self::Error> {
        Ok(match std::mem::size_of::<usize>() {
            1   => Self::U8(n as u8),
            2   => Self::U16(n as u16),
            4   => Self::U32(n as u32),
            8   => Self::U64(n as u64),
            16  => Self::U128(n as u128),
            s   => return Err(err!(
                "The usize for this machine is {}, which has not yet been \
                mapped to a daticle kind.", s;
            System, Unimplemented)),
        })
    }
}

impl TryFrom<isize> for Dat {
    type Error = Error<ErrTag>;

    fn try_from(n: isize) -> std::result::Result<Self, Self::Error> {
        Ok(match std::mem::size_of::<isize>() {
            1   => Self::I8(n as i8),
            2   => Self::I16(n as i16),
            4   => Self::I32(n as i32),
            8   => Self::I64(n as i64),
            16  => Self::I128(n as i128),
            s   => return Err(err!(
                "The isize for this machine is {}, which has not yet been \
                mapped to a daticle kind.", s;
            System, Unimplemented)),
        })
    }
}

impl From<f32> for Dat {
    fn from(f: f32) -> Self {
        Self::F32(Float32(f))
    }
}

impl BestFrom<f32> for Dat {}

impl From<f64> for Dat {
    fn from(f: f64) -> Self {
        Self::F64(Float64(f))
    }
}

impl BestFrom<f64> for Dat {}

impl<'a> From<&'a [u8]> for Dat {
    fn from(v: &'a [u8]) -> Self {
        Self::BU64(v.to_vec())
    }
}

impl<'a> BestFrom<&'a [u8]> for Dat {}

impl<'a> From<&'a str> for Dat {
    fn from(s: &'a str) -> Self {
        Self::Str(s.to_string())
    }
}

impl<'a> BestFrom<&'a str> for Dat {}

impl<D: Into<Dat>> From<Box<D>> for Dat {
    fn from(v: Box<D>) -> Self {
        Self::Box(Box::new((*v).into()))
    }
}

impl<D: Into<Dat>> BestFrom<Box<D>> for Dat {}

impl<D: Into<Dat>> From<Option<D>> for Dat {
    fn from(v: Option<D>) -> Self {
        match v {
            Some(d) => Self::Opt(Box::new(Some(d.into()))),
            None => Self::Opt(Box::new(None)),
        }
    }
}

impl<D: Into<Dat>> BestFrom<Option<D>> for Dat {}

impl TryFrom<Dat> for Vec<String> {
    type Error = Error<ErrTag>;

    fn try_from(dat: Dat) -> Result<Self, Self::Error> {
        dat.get_string_list().ok_or(err!(
            "Daticle '{:?}' is not a list or vek of strings.", dat;
        Conversion, String))
    }
}

impl<'a> TryFrom<&'a Dat> for Vec<String> {
    type Error = Error<ErrTag>;

    fn try_from(dat: &'a Dat) -> Result<Self, Self::Error> {
        dat.get_string_list().ok_or(err!(
            "Daticle '{:?}' is not a list or vek of strings.", dat;
        Conversion, String))
    }
}

impl TryFrom<(UsrKindId, Option<Dat>)> for Dat {
    type Error = Error<ErrTag>;

    fn try_from((ukid, optd): (UsrKindId, Option<Dat>)) -> std::result::Result<Self, Self::Error> {
        let ok = match &optd {
            Some(dat) => match ukid.kind() {
                Some(kind) => if dat.kind() == **kind { true } else { false },
                None => false,
            },
            None => match ukid.kind() {
                Some(_kind) => false,
                None => true,
            },
        };
        if !ok {
            return Err(err!(
                "Usr daticle requires kind {:?} but {:?} received.",
                ukid.kind(), optd;
            Input, Invalid, Mismatch));
        }
        Ok(Self::Usr(ukid, match optd {
            None => None,
            Some(dat) => Some(Box::new(dat)),
        }))
    }
}

impl TryFrom<Vec<Dat>> for Vek {
    type Error = Error<ErrTag>;

    fn try_from(v: Vec<Dat>) -> std::result::Result<Self, Self::Error> {
        if v.len() > 1 {
            let kind = v[0].kind();
            let mut count: usize = 2;
            for d in v.iter().skip(1) {
                if !d.kind().equals(&kind) {
                    return Err(err!(
                        "Cannot construct a Vek from the given Vec because the \
                        kind of item {}, {:?} differs from the kind, {:?} of the \
                        first item.", count, d.kind(), kind;
                    Input, Invalid, Mismatch));
                }
                count += 1;
            }
        }
        Ok(Self(v))
    }
}

impl<
    K: Into<Dat>,
    V: Into<Dat>,
>
    From<BTreeMap<K, V>> for Dat
{
    fn from(v: BTreeMap<K, V>) -> Self {
        let mut map = DaticleMap::new();
        for (key, val) in v {
            map.insert(key.into(), val.into());
        }
        Self::Map(map)
    }
}

impl Dat {

    // Used in FromDatMap
    enum_getter! { get_bool,        bool,               Bool    }
    enum_getter! { get_bigint,      BigInt,             Aint    }
    enum_getter! { get_bigdecimal,  BigDecimal,         Adec    }
    enum_getter! { get_string,      String,             Str     }
    enum_getter! { get_bytes,       Vec<u8>,            BU64    }
    // TODO B256??
    enum_getter! { get_list,        Vec<Dat>,           List    }
    enum_getter! { get_map,         DaticleMap,         Map     }
    enum_getter! { get_box,         Box<Dat>,           Box     }
    enum_getter! { get_box_opt,     Box<Option<Dat>>,   Opt     }

    enum_getter_numeric! { get_u8,   u8   }
    enum_getter_numeric! { get_u16,  u16  }
    enum_getter_numeric! { get_u32,  u32  }
    enum_getter_numeric! { get_u64,  u64  }
    enum_getter_numeric! { get_u128, u128 }
    enum_getter_numeric! { get_i8,   i8   }
    enum_getter_numeric! { get_i16,  i16  }
    enum_getter_numeric! { get_i32,  i32  }
    enum_getter_numeric! { get_i64,  i64  }
    enum_getter_numeric! { get_i128, i128 }
    enum_getter_numeric! { get_float32,  Float32  }
    enum_getter_numeric! { get_float64,  Float64  }
}



// The byte, whose slices are bytes rather than lists.
impl From<u8> for Dat {
    fn from(x: u8) -> Self { Dat::U8(x) }
}

impl ToDat for u8 {
    fn to_dat(&self) -> Outcome<Dat> { Ok(Dat::U8(*self)) }

    fn slice_to_dat(items: &[Self]) -> Outcome<Dat> { Ok(Dat::BU64(items.to_vec())) }
}

impl FromDat for u8 {
    fn from_dat(dat: Dat) -> Outcome<Self> { Ok(try_extract_dat!(dat, U8)) }

    fn vec_from_dat(dat: Dat) -> Outcome<Vec<Self>> {
        Ok(try_extract_dat!(dat, BU8, BU16, BU32, BU64, BC64))
    }
}

impl From<Vec<Dat>> for Dat {
    fn from(x: Vec<Dat>) -> Self { Dat::List(x) }
}

impl From<Vec<u8>> for Dat {
    fn from(x: Vec<u8>) -> Self { Dat::BU64(x) }
}

// Blanket conversions for standard library types, so that derived types nest.

impl<T: ToDat> ToDat for Vec<T> {
    fn to_dat(&self) -> Outcome<Dat> { T::slice_to_dat(self) }
}

impl<T: FromDat> FromDat for Vec<T> {
    fn from_dat(dat: Dat) -> Outcome<Self> { T::vec_from_dat(dat) }
}

impl<T: ToDat> ToDat for Option<T> {
    fn to_dat(&self) -> Outcome<Dat> {
        Ok(Dat::Opt(Box::new(match self {
            Some(v) => Some(res!(v.to_dat())),
            None    => None,
        })))
    }
}

impl<T: FromDat> FromDat for Option<T> {
    fn from_dat(dat: Dat) -> Outcome<Self> {
        match *try_extract_dat!(dat, Opt) {
            Some(inner) => Ok(Some(res!(T::from_dat(inner)))),
            None        => Ok(None),
        }
    }
}

/// A box is transparent: it encodes as what it holds.
impl<T: ToDat> ToDat for Box<T> {
    fn to_dat(&self) -> Outcome<Dat> { (**self).to_dat() }
}

impl<T: FromDat> FromDat for Box<T> {
    fn from_dat(dat: Dat) -> Outcome<Self> { Ok(Box::new(res!(T::from_dat(dat)))) }
}

impl<K: ToDat, V: ToDat> ToDat for BTreeMap<K, V> {
    fn to_dat(&self) -> Outcome<Dat> {
        let mut map = DaticleMap::new();
        for (k, v) in self {
            map.insert(res!(k.to_dat()), res!(v.to_dat()));
        }
        Ok(Dat::Map(map))
    }
}

impl<K: FromDat + Ord, V: FromDat> FromDat for BTreeMap<K, V> {
    fn from_dat(dat: Dat) -> Outcome<Self> {
        let mut out = BTreeMap::new();
        for (k, v) in try_extract_dat!(dat, Map) {
            out.insert(res!(K::from_dat(k)), res!(V::from_dat(v)));
        }
        Ok(out)
    }
}

/// A result is an enum like any derived one: `{"Ok": value}` or `{"Err": error}`.
impl<T: ToDat, E: ToDat> ToDat for Result<T, E> {
    fn to_dat(&self) -> Outcome<Dat> {
        Ok(match self {
            Ok(v)   => crate::derive::variant("Ok", res!(crate::derive::enc(v, "Result::Ok", "0"))),
            Err(e)  => crate::derive::variant("Err", res!(crate::derive::enc(e, "Result::Err", "0"))),
        })
    }
}

impl<T: FromDat, E: FromDat> FromDat for Result<T, E> {
    fn from_dat(dat: Dat) -> Outcome<Self> {
        let (name, payload) = res!(crate::derive::take_variant(dat, "Result"));
        match name.as_str() {
            "Ok"    => Ok(Ok(res!(T::from_dat(payload)))),
            "Err"   => Ok(Err(res!(E::from_dat(payload)))),
            _       => Err(crate::derive::unknown_variant("Result", &name)),
        }
    }
}

macro_rules! tuple_dat {
    ($variant:ident; $($t:ident $v:ident $i:tt),+) => {
        impl<$($t: ToDat),+> ToDat for ($($t,)+) {
            fn to_dat(&self) -> Outcome<Dat> {
                Ok(Dat::$variant(Box::new([$(res!(self.$i.to_dat())),+])))
            }
        }

        impl<$($t: FromDat),+> FromDat for ($($t,)+) {
            fn from_dat(dat: Dat) -> Outcome<Self> {
                let [$($v),+] = *try_extract_dat!(dat, $variant);
                Ok(($(res!($t::from_dat($v)),)+))
            }
        }
    };
}

tuple_dat! { Tup2; A a 0, B b 1 }
tuple_dat! { Tup3; A a 0, B b 1, C c 2 }
tuple_dat! { Tup4; A a 0, B b 1, C c 2, D d 3 }
tuple_dat! { Tup5; A a 0, B b 1, C c 2, D d 3, E e 4 }
tuple_dat! { Tup6; A a 0, B b 1, C c 2, D d 3, E e 4, F f 5 }

impl ToDat for () {
    fn to_dat(&self) -> Outcome<Dat> { Ok(Dat::Empty) }
}

impl FromDat for () {
    fn from_dat(dat: Dat) -> Outcome<Self> {
        match dat {
            Dat::Empty  => Ok(()),
            other       => Err(err!("An empty daticle was expected, a {:?} was found.", other.kind();
                Decode, Mismatch)),
        }
    }
}

impl ToDat for f64 {
    fn to_dat(&self) -> Outcome<Dat> { Ok(Dat::F64(Float64(*self))) }
}

impl FromDat for f64 {
    fn from_dat(dat: Dat) -> Outcome<Self> {
        match dat {
            Dat::F64(Float64(f))    => Ok(f),
            Dat::F32(Float32(f))    => Ok(f as f64),
            other                   => Err(err!("A float was expected, a {:?} was found.", other.kind();
                Decode, Mismatch)),
        }
    }
}

impl ToDat for f32 {
    fn to_dat(&self) -> Outcome<Dat> { Ok(Dat::F32(Float32(*self))) }
}

impl FromDat for f32 {
    fn from_dat(dat: Dat) -> Outcome<Self> { Ok(try_extract_dat!(dat, F32).0) }
}

/// A `usize` travels as a `u64`, and any narrower unsigned width is taken back.
impl ToDat for usize {
    fn to_dat(&self) -> Outcome<Dat> { Ok(Dat::U64(*self as u64)) }
}

impl FromDat for usize {
    fn from_dat(dat: Dat) -> Outcome<Self> {
        let n = try_extract_dat_as!(dat, u64, U8, U16, U32, U64);
        match usize::try_from(n) {
            Ok(n)   => Ok(n),
            Err(_)  => Err(err!("{} does not fit a usize on this machine.", n; Decode, Overflow)),
        }
    }
}

/// An `isize` travels as an `i64`, and any narrower signed width is taken back.
impl ToDat for isize {
    fn to_dat(&self) -> Outcome<Dat> { Ok(Dat::I64(*self as i64)) }
}

impl FromDat for isize {
    fn from_dat(dat: Dat) -> Outcome<Self> {
        let n = try_extract_dat_as!(dat, i64, I8, I16, I32, I64);
        match isize::try_from(n) {
            Ok(n)   => Ok(n),
            Err(_)  => Err(err!("{} does not fit an isize on this machine.", n; Decode, Overflow)),
        }
    }
}

/// An error travels as its `ErrWire` form, encoded like a derived enum.
impl ToDat for ErrWire {
    fn to_dat(&self) -> Outcome<Dat> {
        use crate::derive as d;
        Ok(match self {
            ErrWire::Frame { msg, tags, cause } => {
                let mut map = DaticleMap::new();
                d::put(&mut map, "msg", Dat::Str(msg.clone()));
                d::put(&mut map, "tags", res!(tags.to_dat()));
                d::put(&mut map, "cause", res!(cause.to_dat()));
                d::variant("Frame", Dat::Map(map))
            },
            ErrWire::Collection(list) => d::variant("Collection", res!(list.to_dat())),
        })
    }
}

impl FromDat for ErrWire {
    fn from_dat(dat: Dat) -> Outcome<Self> {
        use crate::derive as d;
        let (name, payload) = res!(d::take_variant(dat, "ErrWire"));
        match name.as_str() {
            "Frame" => {
                let ty = "ErrWire::Frame";
                let mut map = res!(d::take_map(payload, ty));
                let msg = res!(d::field(&mut map, ty, "msg"));
                let tags = res!(d::field(&mut map, ty, "tags"));
                let cause = res!(d::field(&mut map, ty, "cause"));
                res!(d::done(map, ty));
                Ok(ErrWire::Frame { msg, tags, cause })
            },
            "Collection" => Ok(ErrWire::Collection(res!(Vec::<ErrWire>::from_dat(payload)))),
            _ => Err(d::unknown_variant("ErrWire", &name)),
        }
    }
}

impl ToDat for Error<ErrTag> {
    fn to_dat(&self) -> Outcome<Dat> { self.to_wire().to_dat() }
}

impl FromDat for Error<ErrTag> {
    fn from_dat(dat: Dat) -> Outcome<Self> { Ok(res!(ErrWire::from_dat(dat)).to_error()) }
}
