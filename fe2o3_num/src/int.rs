use oxedize_fe2o3_core::prelude::*;

use std::string;

pub trait PrimInt: Sized + string::ToString {
    fn is_neg(self) -> bool;
}

impl PrimInt for u8 {
    fn is_neg(self) -> bool { false }
}

impl PrimInt for i8 {
    fn is_neg(self) -> bool { self < 0 }
}

impl PrimInt for u16 {
    fn is_neg(self) -> bool { false }
}

impl PrimInt for i16 {
    fn is_neg(self) -> bool { self < 0 }
}

impl PrimInt for u32 {
    fn is_neg(self) -> bool { false }
}

impl PrimInt for i32 {
    fn is_neg(self) -> bool { self < 0 }
}

impl PrimInt for u64 {
    fn is_neg(self) -> bool { false }
}

impl PrimInt for i64 {
    fn is_neg(self) -> bool { self < 0 }
}

impl PrimInt for usize {
    fn is_neg(self) -> bool { false }
}

impl PrimInt for isize {
    fn is_neg(self) -> bool { self < 0 }
}

pub trait Signed {}
pub trait Unsigned {}

/// Fast `usize` ceiling division.
/// Credit: https://stackoverflow.com/questions/2745074/fast-ceiling-of-an-integer-division-in-c-c
pub fn usize_ceil_div(dividend: usize, divisor: usize) -> Outcome<usize> {
    if divisor == 0 {
        Err(err!(errmsg!(
            "Attempt to divide {} by {}, latter cannot be zero",
            dividend, divisor,
        ), Numeric, Input, Invalid))
    } else if dividend == 0 {
        Ok(0)
    } else {
        Ok(1 + ((dividend - 1) / divisor))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_ceiling_div_01() {
        for x in 1..1000 {
            for y in 1..x {
                let xf = x as f64;
                let yf = y as f64;
                let q = (xf/yf).ceil() as usize;
                assert_eq!(q, usize_ceil_div(x, y).unwrap());
            }
        }
    }
}
