use oxedize_fe2o3_core::prelude::*;

use std::{
    hash::{
        Hash,
        Hasher,
    },
    string,
};

pub trait PrimitiveFloat: Sized + string::ToString {}

impl PrimitiveFloat for f32 {}
impl PrimitiveFloat for f64 {}

/// The number of significant figures is assumed to be > 0.
pub fn round_to_sf(n: f64, sf: u8) -> f64 {
    let exp = (sf as i32) - (n.log10().floor() as i32) - 1;
    let rnd = (n * 10.0f64.powi(exp)).round();
    rnd * 10.0f64.powi(-exp)
}

new_type!(Float32, f32, Clone, Debug, Default, PartialOrd);

impl Ord for Float32 {
    // total_cmp function currently yet to make it to stable
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        let mut left = self.to_bits() as i32;
        let mut right = other.to_bits() as i32;
        left ^= (((left >> 31) as u32) >> 1) as i32;
        right ^= (((right >> 31) as u32) >> 1) as i32;
        left.cmp(&right)
    }
}

impl PartialEq for Float32 {
    fn eq(&self, other: &Float32) -> bool {
        self.cmp(other) == std::cmp::Ordering::Equal
    }
    fn ne(&self, other: &Float32) -> bool {
        self.cmp(other) != std::cmp::Ordering::Equal
    }
}

impl Eq for Float32 {}

impl Hash for Float32 {
    fn hash<H: Hasher>(&self, state: &mut H) {
        let (m, e, s) = self.integer_decode();
        m.hash(state);
        e.hash(state);
        s.hash(state);
    }
}

impl Float32 {
    // A function deprecated from the std library, modified to take a reference and use the inner type.
    // https://github.com/rust-lang/rust/blob/5c674a11471ec0569f616854d715941757a48a0a/src/libcore/num/f32.rs
    fn integer_decode(&self) -> (u64, i16, i8) {
        let bits: u32 = self.0.to_bits();
        let sign: i8 = if bits >> 31 == 0 { 1 } else { -1 };
        let mut exponent: i16 = ((bits >> 23) & 0xff) as i16;
        let mantissa = if exponent == 0 {
            (bits & 0x7fffff) << 1
        } else {
            (bits & 0x7fffff) | 0x800000
        };
        // Exponent bias + mantissa shift
        exponent -= 127 + 23;
        (mantissa as u64, exponent, sign)
    }

    pub fn is_zero(&self) -> bool {
        let (m, _, _) = self.integer_decode();
        m == 0
    }
}

new_type!(Float64, f64, Clone, Debug, Default, PartialOrd);

impl Ord for Float64 {
    // total_cmp function currently yet to make it to stable
     fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        let mut left = self.to_bits() as i64;
        let mut right = other.to_bits() as i64;
        left ^= (((left >> 63) as u64) >> 1) as i64;
        right ^= (((right >> 63) as u64) >> 1) as i64;
        left.cmp(&right)
    }
}

impl PartialEq for Float64 {
    fn eq(&self, other: &Float64) -> bool {
        self.cmp(other) == std::cmp::Ordering::Equal
    }
    fn ne(&self, other: &Float64) -> bool {
        self.cmp(other) != std::cmp::Ordering::Equal
    }
}

impl Eq for Float64 {}

impl Hash for Float64 {
    fn hash<H: Hasher>(&self, state: &mut H) {
        let (m, e, s) = self.integer_decode();
        m.hash(state);
        e.hash(state);
        s.hash(state);
    }
}

impl Float64 {
    // A function deprecated from the std library, modified to take a reference and use the inner type.
    // https://github.com/rust-lang/rust/blob/5c674a11471ec0569f616854d715941757a48a0a/src/libcore/num/f64.rs
    fn integer_decode(&self) -> (u64, i16, i8) {
        let bits: u64 = self.0.to_bits();
        let sign: i8 = if bits >> 63 == 0 { 1 } else { -1 };
        let mut exponent: i16 = ((bits >> 52) & 0x7ff) as i16;
        let mantissa = if exponent == 0 {
            (bits & 0xfffffffffffff) << 1
        } else {
            (bits & 0xfffffffffffff) | 0x10000000000000
        };
        // Exponent bias + mantissa shift
        exponent -= 1023 + 52;
        (mantissa, exponent, sign)
    }

    pub fn is_zero(&self) -> bool {
        let (m, _, _) = self.integer_decode();
        m == 0
    }
}
