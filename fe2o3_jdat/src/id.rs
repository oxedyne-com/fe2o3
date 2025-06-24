//! An `IdDat` is the `Dat` flavoured version of the generic identifier `oxedyne_fe2o3_core::id::Id` that
//! wraps an explicit, native unsigned integer. The unsigned integers and `IdDat` implement the
//! `NumId` and `NumIdDat` trait collections.

use crate::{
    prelude::*,
};

use oxedyne_fe2o3_core::{
    prelude::*,
    byte::{
        FromBytes,
        FromByteArray,
        ToBytes,
        ToByteArray,
    },
    id::ParseId,
    rand::RanDef,
    string::ToHexString,
};
use oxedyne_fe2o3_text::base2x;

use std::{
    fmt::{
        self,
        Debug,
        Display,
        LowerHex,
    },
    ops::Deref,
};


pub trait NumId<const L: usize>:
    Clone
    + Copy
    + Debug
    + Default
    + Display
    + ParseId<L>
    + Eq
    + Ord
    + PartialEq
    + PartialOrd
    + LowerHex
    + FromBytes
    + ToBytes
    + FromByteArray
    + ToByteArray<L>
    + ToHexString
    + RanDef
    + Send
    + Sync
{}

impl NumId<1> for u8 {}
impl NumId<2> for u16 {}
impl NumId<4> for u32 {}
impl NumId<8> for u64 {}
impl NumId<16> for u128 {}
impl NumId<32> for B32 {}

#[derive(Clone, Copy, Default, Eq, Ord, PartialEq, PartialOrd)]
pub struct Id<
    const IDL: usize,
    N: NumId<IDL>,
>(
    pub N
);

impl NumId<1> for Id<1, u8> {}
impl NumId<2> for Id<2, u16> {}
impl NumId<4> for Id<4, u32> {}
impl NumId<8> for Id<8, u64> {}
impl NumId<16> for Id<16, u128> {}
impl NumId<32> for Id<32, B32> {}

impl<
    const IDL: usize,
    N: NumId<IDL>,
>
    Debug for Id<IDL, N>
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let byts = self.to_byte_array();
        write!(f, "{}", base2x::HEMATITE64.to_string(&byts))
    }
}

impl<
    const IDL: usize,
    N: NumId<IDL>,
>
    Display for Id<IDL, N>
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}

impl<
    const IDL: usize,
    N: NumId<IDL>,
>
    ParseId<IDL> for Id<IDL, N>
{
    fn parse_id(s: &str) -> Outcome<Self> {
        let byts = res!(base2x::HEMATITE64.from_str(s));
        let (result, _) = res!(Self::from_bytes(&byts));
        Ok(result)
    }
}

impl<
    const IDL: usize,
    N: NumId<IDL>,
>
    Deref for Id<IDL, N>
{
    type Target = N;
    fn deref(&self) -> &Self::Target { &self.0 }
}

impl<
    const IDL: usize,
    N: NumId<IDL>,
>
    LowerHex for Id<IDL, N>
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        LowerHex::fmt(&self.0, f)
    }
}

impl<
    const IDL: usize,
    N: NumId<IDL>,
>
    FromBytes for Id<IDL, N>
{
    fn from_bytes(buf: &[u8]) -> Outcome<(Self, usize)> {
        let (id, n) = res!(N::from_bytes(buf));
        Ok((Self(id), n))
    }
}

impl<
    const IDL: usize,
    N: NumId<IDL>,
>
    ToBytes for Id<IDL, N>
{
    fn to_bytes(&self, buf: Vec<u8>) -> Outcome<Vec<u8>> {
        self.0.to_bytes(buf)
    }
}

impl<
    const IDL: usize,
    N: NumId<IDL>,
>
    FromByteArray for Id<IDL, N>
{
    fn from_byte_array<const L: usize>(buf: [u8; L]) -> Outcome<Self> {
        Ok(Self(res!(N::from_byte_array(buf))))
    }
}

impl<
    const IDL: usize,
    N: NumId<IDL>,
>
    ToByteArray<IDL> for Id<IDL, N>
{
    fn to_byte_array(&self) -> [u8; IDL] {
        self.0.to_byte_array()
    }
}

impl<
    const IDL: usize,
    N: NumId<IDL>,
>
    ToHexString for Id<IDL, N>
{
    fn to_hex_string(&self) -> String {
        self.0.to_hex_string()
    }
}

impl<
    const IDL: usize,
    N: NumId<IDL>,
>
    RanDef for Id<IDL, N>
{
    fn randef() -> Self {
        Self(N::randef())
    }
}

impl<
    const IDL: usize,
    N: NumId<IDL>,
>
    Id<IDL, N>
{
    pub fn new(n: N) -> Self {
        Self(n)
    }
}

//#[cfg(test)]
//mod tests {
//    use crate::{
//        prelude::*,
//        id::NumId,
//    };
//
//    use oxedyne_fe2o3_core::{
//        prelude::*,
//        string::ToHexString,
//    };
//
//
//    struct Id<N: NumId<2>>(N);
//
//    impl<N: NumId<2>> std::ops::Deref for Id<N> {
//        type Target = N;
//        fn deref(&self) -> &Self::Target { &self.0 }
//    }
//
//    #[test]
//    fn test_uint_000() -> Outcome<()> {
//        let id = Id(255u16);
//        req!(id.to_hex_string().as_str(), "0x00ff");
//        Ok(())
//    }
//
//}
pub trait NumIdDat<const L: usize>:
    NumId<L>
    + ToDat
    + FromDat
{}

// Native unsigned integers.
impl NumIdDat<1> for u8 {}
impl NumIdDat<2> for u16 {}
impl NumIdDat<4> for u32 {}
impl NumIdDat<8> for u64 {}
impl NumIdDat<16> for u128 {}
impl NumIdDat<32> for B32 {}


#[derive(Clone, Copy, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct IdDat<
    const IDL: usize,
    N: NumIdDat<IDL>,
>(
    pub N
);

impl NumId<1> for IdDat<1, u8> {}
impl NumId<2> for IdDat<2, u16> {}
impl NumId<4> for IdDat<4, u32> {}
impl NumId<8> for IdDat<8, u64> {}
impl NumId<16> for IdDat<16, u128> {}
impl NumId<32> for IdDat<32, B32> {}

impl NumIdDat<1> for IdDat<1, u8> {}
impl NumIdDat<2> for IdDat<2, u16> {}
impl NumIdDat<4> for IdDat<4, u32> {}
impl NumIdDat<8> for IdDat<8, u64> {}
impl NumIdDat<16> for IdDat<16, u128> {}
impl NumIdDat<32> for IdDat<32, B32> {}

impl<
    const IDL: usize,
    N: NumIdDat<IDL>,
>
    Debug for IdDat<IDL, N>
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let byts = self.to_byte_array();
        write!(f, "{}", base2x::HEMATITE64.to_string(&byts))
    }
}

impl<
    const IDL: usize,
    N: NumIdDat<IDL>,
>
    Display for IdDat<IDL, N>
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}

impl<
    const IDL: usize,
    N: NumIdDat<IDL>,
>
    ParseId<IDL> for IdDat<IDL, N>
{
    fn parse_id(s: &str) -> Outcome<Self> {
        let byts = res!(base2x::HEMATITE64.from_str(s));
        let (result, _) = res!(Self::from_bytes(&byts));
        Ok(result)
    }
}

impl<
    const IDL: usize,
    N: NumIdDat<IDL>,
>
    Deref for IdDat<IDL, N>
{
    type Target = N;
    fn deref(&self) -> &Self::Target { &self.0 }
}

impl<
    const IDL: usize,
    N: NumIdDat<IDL>,
>
    LowerHex for IdDat<IDL, N>
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        LowerHex::fmt(&self.0, f)
    }
}

impl<
    const IDL: usize,
    N: NumIdDat<IDL>,
>
    FromBytes for IdDat<IDL, N>
{
    fn from_bytes(buf: &[u8]) -> Outcome<(Self, usize)> {
        let (id, n) = res!(N::from_bytes(buf));
        Ok((Self(id), n))
    }
}

impl<
    const IDL: usize,
    N: NumIdDat<IDL>,
>
    ToBytes for IdDat<IDL, N>
{
    fn to_bytes(&self, buf: Vec<u8>) -> Outcome<Vec<u8>> {
        self.0.to_bytes(buf)
    }
}

impl<
    const IDL: usize,
    N: NumIdDat<IDL>,
>
    FromByteArray for IdDat<IDL, N>
{
    fn from_byte_array<const L: usize>(buf: [u8; L]) -> Outcome<Self> {
        Ok(Self(res!(N::from_byte_array(buf))))
    }
}

impl<
    const IDL: usize,
    N: NumIdDat<IDL>,
>
    ToByteArray<IDL> for IdDat<IDL, N>
{
    fn to_byte_array(&self) -> [u8; IDL] {
        self.0.to_byte_array()
    }
}

impl<
    const IDL: usize,
    N: NumIdDat<IDL>,
>
    ToHexString for IdDat<IDL, N>
{
    fn to_hex_string(&self) -> String {
        self.0.to_hex_string()
    }
}

impl<
    const IDL: usize,
    N: NumIdDat<IDL>,
>
    RanDef for IdDat<IDL, N>
{
    fn randef() -> Self {
        Self(N::randef())
    }
}

// Now for NumIdDat traits...

impl<
    const IDL: usize,
    N: NumIdDat<IDL>,
>
    ToDat for IdDat<IDL, N>
{
    fn to_dat(&self) -> Outcome<Dat> {
        self.0.to_dat()
    }
}

impl<
    const IDL: usize,
    N: NumIdDat<IDL>,
>
    FromDat for IdDat<IDL, N>
{
    fn from_dat(dat: Dat) -> Outcome<Self> {
        Ok(Self(res!(N::from_dat(dat))))
    }
}

impl<
    const IDL: usize,
    N: NumIdDat<IDL>,
>
    IdDat<IDL, N>
{
    pub fn new(n: N) -> Self {
        Self(n)
    }
}
