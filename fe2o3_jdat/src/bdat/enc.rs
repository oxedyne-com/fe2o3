use crate::prelude::*;

use oxedyne_fe2o3_core::{
    prelude::*,
    byte::ToBytes,
};
use oxedyne_fe2o3_num::float::{
    Float32,
    Float64,
};


/// The error raised when a kind reaches an encoding frame that is not the one it belongs to.
///
/// The dispatch in [`Dat::to_bytes`] sends every kind to its own frame, so this is unreachable
/// unless the dispatch and the frames disagree, which is a fault in this file rather than in
/// anything given to it.
fn wrong_frame(dat: &Dat) -> Error<ErrTag> {
    err!(
        "The daticle kind {:?} was encoded in a frame that does not encode it.  The dispatch in \
        Dat::to_bytes and the frames it dispatches to disagree.", dat.kind();
    Bug, Mismatch)
}

impl ToBytes for Dat {

    /// Appends the encoded `Dat` to the given byte buffer.
    ///
    /// Every kind that encloses other kinds is encoded in a frame of its own, so that the frame
    /// this dispatch repeats as it descends holds nothing but the dispatch.  A single match over
    /// every kind would give one frame the locals of all of them, and a value nested a few hundred
    /// deep would then exhaust the stack while encoding something that decodes perfectly well.
    /// [`Dat::from_bytes_depth`](crate::Dat) keeps its frames small for the same reason.
    fn to_bytes(&self, buf: Vec<u8>) -> Outcome<Vec<u8>> {
        match self {
            // Molecular Kinds ========================
            // Unitary
            Self::Usr(..)       => self.to_bytes_usr(buf),
            Self::Opt(..)       => self.to_bytes_opt(buf),
            Self::Box(..)       => self.to_bytes_box(buf),
            Self::ABox(..)      => self.to_bytes_abox(buf),
            // Heterogenous
            Self::List(..)      |
            Self::Vek(..)       => self.to_bytes_list(buf),
            Self::Tup2(..)      |
            Self::Tup3(..)      |
            Self::Tup4(..)      |
            Self::Tup5(..)      |
            Self::Tup6(..)      |
            Self::Tup7(..)      |
            Self::Tup8(..)      |
            Self::Tup9(..)      |
            Self::Tup10(..)     => self.to_bytes_tuple(buf),
            Self::Map(..)       |
            Self::OrdMap(..)    => self.to_bytes_map(buf),
            // Every other kind is atomic, enclosing no other kind.  Those arms carry the bulk of
            // the encoder's stack frame, so they too are encoded in a frame of their own, which is
            // entered at a leaf and left there.
            _                   => self.to_bytes_atomic(buf),
        }
    }
}

impl Dat {

    /// Encodes a user-defined kind: the kind code, then the payload it carries, if any.
    #[inline(never)]
    fn to_bytes_usr(&self, mut buf: Vec<u8>) -> Outcome<Vec<u8>> {
        match self {
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
                Ok(buf)
            },
            _ => Err(wrong_frame(self)),
        }
    }

    /// Encodes an optional value: the code alone when it is none, the value when it is some.
    #[inline(never)]
    fn to_bytes_opt(&self, mut buf: Vec<u8>) -> Outcome<Vec<u8>> {
        match self {
            Self::Opt(boxoptd) => {
                self.append_code(&mut buf);
                if let Some(d) = &**boxoptd {
                    buf = res!(d.to_bytes(buf));
                }
                Ok(buf)
            },
            _ => Err(wrong_frame(self)),
        }
    }

    /// Encodes a boxed value.
    #[inline(never)]
    fn to_bytes_box(&self, mut buf: Vec<u8>) -> Outcome<Vec<u8>> {
        match self {
            Self::Box(boxd) => {
                self.append_code(&mut buf);
                Ok(res!(boxd.to_bytes(buf)))
            },
            _ => Err(wrong_frame(self)),
        }
    }

    /// Encodes an annotated box: its configuration, the value, then the note.
    #[inline(never)]
    fn to_bytes_abox(&self, mut buf: Vec<u8>) -> Outcome<Vec<u8>> {
        match self {
            Self::ABox(ncfg, boxd, s) => {
                self.append_code(&mut buf);
                buf = res!(ncfg.to_bytes(buf));
                buf = res!(boxd.to_bytes(buf));
                let b = s.as_bytes();
                buf = res!(Dat::C64(b.len() as u64).to_bytes(buf));
                buf.extend_from_slice(b);
                Ok(buf)
            },
            _ => Err(wrong_frame(self)),
        }
    }

    /// Encodes a list or a vek: the code, the byte length, then the items.
    #[inline(never)]
    fn to_bytes_list(&self, mut buf: Vec<u8>) -> Outcome<Vec<u8>> {
        match self {
            Self::List(v) => {
                self.append_code(&mut buf);
                Ok(res!(Self::vec_to_bytes(v, buf)))
            },
            Self::Vek(vek) => {
                self.append_code(&mut buf);
                Ok(res!(Self::vec_to_bytes(&*vek, buf)))
            },
            _ => Err(wrong_frame(self)),
        }
    }

    /// Encodes a tuple of daticles.
    #[inline(never)]
    fn to_bytes_tuple(&self, buf: Vec<u8>) -> Outcome<Vec<u8>> {
        match self {
            Self::Tup2(a)   => Self::tuple_to_bytes::<Dat, 2, {Self::TUP_SERIES_START}>(a, buf),
            Self::Tup3(a)   => Self::tuple_to_bytes::<Dat, 3, {Self::TUP_SERIES_START}>(a, buf),
            Self::Tup4(a)   => Self::tuple_to_bytes::<Dat, 4, {Self::TUP_SERIES_START}>(a, buf),
            Self::Tup5(a)   => Self::tuple_to_bytes::<Dat, 5, {Self::TUP_SERIES_START}>(a, buf),
            Self::Tup6(a)   => Self::tuple_to_bytes::<Dat, 6, {Self::TUP_SERIES_START}>(a, buf),
            Self::Tup7(a)   => Self::tuple_to_bytes::<Dat, 7, {Self::TUP_SERIES_START}>(a, buf),
            Self::Tup8(a)   => Self::tuple_to_bytes::<Dat, 8, {Self::TUP_SERIES_START}>(a, buf),
            Self::Tup9(a)   => Self::tuple_to_bytes::<Dat, 9, {Self::TUP_SERIES_START}>(a, buf),
            Self::Tup10(a)  => Self::tuple_to_bytes::<Dat, 10, {Self::TUP_SERIES_START}>(a, buf),
            _ => Err(wrong_frame(self)),
        }
    }

    /// Encodes a map or an ordered map: the code, the byte length, then the entries.
    ///
    /// The two are kept apart because their keys are of different types, an ordered map keying by
    /// insertion rather than by the key itself.
    #[inline(never)]
    fn to_bytes_map(&self, mut buf: Vec<u8>) -> Outcome<Vec<u8>> {
        let mut buf2 = Vec::new();
        match self {
            Self::Map(map) => {
                for (k, v) in map {
                    buf2 = res!(k.to_bytes(buf2));
                    buf2 = res!(v.to_bytes(buf2));
                }
            },
            Self::OrdMap(map) => {
                for (k, v) in map {
                    buf2 = res!(k.to_bytes(buf2));
                    buf2 = res!(v.to_bytes(buf2));
                }
            },
            _ => return Err(wrong_frame(self)),
        }
        self.append_code(&mut buf);
        buf = res!(Dat::C64(buf2.len() as u64).to_bytes(buf));
        buf.extend_from_slice(&buf2);
        Ok(buf)
    }

    /// Encodes every kind that encloses no other kind.
    ///
    /// The arms here are many, and in a debug build each keeps its own slot in the frame, so this
    /// is the frame that must not be the one nesting repeats.
    #[inline(never)]
    fn to_bytes_atomic(&self, mut buf: Vec<u8>) -> Outcome<Vec<u8>> {
        match self {
            // Molecular Kinds ========================
            // Each of these is encoded in a frame of its own, and never here.
            Self::Usr(..)       |
            Self::Opt(..)       |
            Self::Box(..)       |
            Self::ABox(..)      |
            Self::List(..)      |
            Self::Vek(..)       |
            Self::Tup2(..)      |
            Self::Tup3(..)      |
            Self::Tup4(..)      |
            Self::Tup5(..)      |
            Self::Tup6(..)      |
            Self::Tup7(..)      |
            Self::Tup8(..)      |
            Self::Tup9(..)      |
            Self::Tup10(..)     |
            Self::Map(..)       |
            Self::OrdMap(..)    => return Err(wrong_frame(self)),
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
            Self::Tup2u8(a) => {
                buf = res!(Self::tuple_to_bytes::<u8, 2, {Self::TUP_U8_SERIES_START}>(a, buf));
            },
            Self::Tup3u8(a) => {
                buf = res!(Self::tuple_to_bytes::<u8, 3, {Self::TUP_U8_SERIES_START}>(a, buf));
            },
            Self::Tup4u8(a) => {
                buf = res!(Self::tuple_to_bytes::<u8, 4, {Self::TUP_U8_SERIES_START}>(a, buf));
            },
            Self::Tup5u8(a) => {
                buf = res!(Self::tuple_to_bytes::<u8, 5, {Self::TUP_U8_SERIES_START}>(a, buf));
            },
            Self::Tup6u8(a) => {
                buf = res!(Self::tuple_to_bytes::<u8, 6, {Self::TUP_U8_SERIES_START}>(a, buf));
            },
            Self::Tup7u8(a) => {
                buf = res!(Self::tuple_to_bytes::<u8, 7, {Self::TUP_U8_SERIES_START}>(a, buf));
            },
            Self::Tup8u8(a) => {
                buf = res!(Self::tuple_to_bytes::<u8, 8, {Self::TUP_U8_SERIES_START}>(a, buf));
            },
            Self::Tup9u8(a) => {
                buf = res!(Self::tuple_to_bytes::<u8, 9, {Self::TUP_U8_SERIES_START}>(a, buf));
            },
            Self::Tup10u8(a) => {
                buf = res!(Self::tuple_to_bytes::<u8, 10, {Self::TUP_U8_SERIES_START}>(a, buf));
            },
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
            Self::Tup2i8(a) => {
                buf = res!(Self::tuple_to_bytes::<i8, 2, {Self::TUP_I8_SERIES_START}>(a, buf));
            },
            Self::Tup3i8(a) => {
                buf = res!(Self::tuple_to_bytes::<i8, 3, {Self::TUP_I8_SERIES_START}>(a, buf));
            },
            Self::Tup4i8(a) => {
                buf = res!(Self::tuple_to_bytes::<i8, 4, {Self::TUP_I8_SERIES_START}>(a, buf));
            },
            Self::Tup5i8(a) => {
                buf = res!(Self::tuple_to_bytes::<i8, 5, {Self::TUP_I8_SERIES_START}>(a, buf));
            },
            Self::Tup6i8(a) => {
                buf = res!(Self::tuple_to_bytes::<i8, 6, {Self::TUP_I8_SERIES_START}>(a, buf));
            },
            Self::Tup7i8(a) => {
                buf = res!(Self::tuple_to_bytes::<i8, 7, {Self::TUP_I8_SERIES_START}>(a, buf));
            },
            Self::Tup8i8(a) => {
                buf = res!(Self::tuple_to_bytes::<i8, 8, {Self::TUP_I8_SERIES_START}>(a, buf));
            },
            Self::Tup9i8(a) => {
                buf = res!(Self::tuple_to_bytes::<i8, 9, {Self::TUP_I8_SERIES_START}>(a, buf));
            },
            Self::Tup10i8(a) => {
                buf = res!(Self::tuple_to_bytes::<i8, 10, {Self::TUP_I8_SERIES_START}>(a, buf));
            },
            Self::Tup2i16(a) => {
                buf = res!(Self::tuple_to_bytes::<i16, 2, {Self::TUP_I16_SERIES_START}>(a, buf));
            },
            Self::Tup3i16(a) => {
                buf = res!(Self::tuple_to_bytes::<i16, 3, {Self::TUP_I16_SERIES_START}>(a, buf));
            },
            Self::Tup4i16(a) => {
                buf = res!(Self::tuple_to_bytes::<i16, 4, {Self::TUP_I16_SERIES_START}>(a, buf));
            },
            Self::Tup5i16(a) => {
                buf = res!(Self::tuple_to_bytes::<i16, 5, {Self::TUP_I16_SERIES_START}>(a, buf));
            },
            Self::Tup6i16(a) => {
                buf = res!(Self::tuple_to_bytes::<i16, 6, {Self::TUP_I16_SERIES_START}>(a, buf));
            },
            Self::Tup7i16(a) => {
                buf = res!(Self::tuple_to_bytes::<i16, 7, {Self::TUP_I16_SERIES_START}>(a, buf));
            },
            Self::Tup8i16(a) => {
                buf = res!(Self::tuple_to_bytes::<i16, 8, {Self::TUP_I16_SERIES_START}>(a, buf));
            },
            Self::Tup9i16(a) => {
                buf = res!(Self::tuple_to_bytes::<i16, 9, {Self::TUP_I16_SERIES_START}>(a, buf));
            },
            Self::Tup10i16(a) => {
                buf = res!(Self::tuple_to_bytes::<i16, 10, {Self::TUP_I16_SERIES_START}>(a, buf));
            },
            Self::Tup2i32(a) => {
                buf = res!(Self::tuple_to_bytes::<i32, 2, {Self::TUP_I32_SERIES_START}>(a, buf));
            },
            Self::Tup3i32(a) => {
                buf = res!(Self::tuple_to_bytes::<i32, 3, {Self::TUP_I32_SERIES_START}>(a, buf));
            },
            Self::Tup4i32(a) => {
                buf = res!(Self::tuple_to_bytes::<i32, 4, {Self::TUP_I32_SERIES_START}>(a, buf));
            },
            Self::Tup5i32(a) => {
                buf = res!(Self::tuple_to_bytes::<i32, 5, {Self::TUP_I32_SERIES_START}>(a, buf));
            },
            Self::Tup6i32(a) => {
                buf = res!(Self::tuple_to_bytes::<i32, 6, {Self::TUP_I32_SERIES_START}>(a, buf));
            },
            Self::Tup7i32(a) => {
                buf = res!(Self::tuple_to_bytes::<i32, 7, {Self::TUP_I32_SERIES_START}>(a, buf));
            },
            Self::Tup8i32(a) => {
                buf = res!(Self::tuple_to_bytes::<i32, 8, {Self::TUP_I32_SERIES_START}>(a, buf));
            },
            Self::Tup9i32(a) => {
                buf = res!(Self::tuple_to_bytes::<i32, 9, {Self::TUP_I32_SERIES_START}>(a, buf));
            },
            Self::Tup10i32(a) => {
                buf = res!(Self::tuple_to_bytes::<i32, 10, {Self::TUP_I32_SERIES_START}>(a, buf));
            },
            Self::Tup2i64(a) => {
                buf = res!(Self::tuple_to_bytes::<i64, 2, {Self::TUP_I64_SERIES_START}>(a, buf));
            },
            Self::Tup3i64(a) => {
                buf = res!(Self::tuple_to_bytes::<i64, 3, {Self::TUP_I64_SERIES_START}>(a, buf));
            },
            Self::Tup4i64(a) => {
                buf = res!(Self::tuple_to_bytes::<i64, 4, {Self::TUP_I64_SERIES_START}>(a, buf));
            },
            Self::Tup5i64(a) => {
                buf = res!(Self::tuple_to_bytes::<i64, 5, {Self::TUP_I64_SERIES_START}>(a, buf));
            },
            Self::Tup6i64(a) => {
                buf = res!(Self::tuple_to_bytes::<i64, 6, {Self::TUP_I64_SERIES_START}>(a, buf));
            },
            Self::Tup7i64(a) => {
                buf = res!(Self::tuple_to_bytes::<i64, 7, {Self::TUP_I64_SERIES_START}>(a, buf));
            },
            Self::Tup8i64(a) => {
                buf = res!(Self::tuple_to_bytes::<i64, 8, {Self::TUP_I64_SERIES_START}>(a, buf));
            },
            Self::Tup9i64(a) => {
                buf = res!(Self::tuple_to_bytes::<i64, 9, {Self::TUP_I64_SERIES_START}>(a, buf));
            },
            Self::Tup10i64(a) => {
                buf = res!(Self::tuple_to_bytes::<i64, 10, {Self::TUP_I64_SERIES_START}>(a, buf));
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
