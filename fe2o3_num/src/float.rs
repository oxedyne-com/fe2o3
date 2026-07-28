use oxedyne_fe2o3_core::prelude::*;

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

/// Rounds a number to the given count of significant figures, taking a tie
/// away from zero.
///
/// The count is taken from the leading digit of the magnitude, so a sign makes
/// no difference to which digits survive: -0.475 to two figures is -0.48, just
/// as 0.475 is 0.48. A tie is settled away from zero, matching
/// [`f64::round`], though a tie only truly arises where the halfway value is
/// exactly representable, as 0.125 is.
///
/// A zero, a value that is not finite, a figure count of zero, or a value so
/// large that rounding up would overflow, is returned unchanged.
pub fn round_to_sf(n: f64, sf: u8) -> f64 {
    if sf == 0 || n == 0.0 || !n.is_finite() {
        return n;
    }
    // Decimal place of the leading digit: 0 for one up to ten, -1 for a tenth
    // up to one, and so on. The logarithm of a negative number is not a number,
    // so the magnitude is taken first; the earlier form omitted that, and the
    // resulting cast of a not-a-number to zero put every negative value's
    // leading digit in the units place.
    let mut mag = n.abs().log10().floor() as i32;
    // A logarithm is not exact, so the place is checked against the value it is
    // meant to describe and nudged if it names the wrong decade.
    let lead = mul_pow10(n.abs(), -mag);
    if lead >= 10.0 {
        mag += 1;
    } else if lead < 1.0 {
        mag -= 1;
    }
    // Places the decimal point moves right to leave sf digits before it.
    let shift = (sf as i32) - 1 - mag;
    let scaled = mul_pow10(n, shift);
    if !scaled.is_finite() {
        return n;
    }
    let rnd = scaled.round();
    // Past the range in which a power of ten is exactly representable the
    // scaling itself carries an error of a few units in the last place. A value
    // already at the requested precision must not be nudged by that error.
    if shift.abs() > 22 && (scaled - rnd).abs() <= scaled.abs() * 16.0 * f64::EPSILON {
        return n;
    }
    let out = mul_pow10(rnd, -shift);
    if out.is_finite() { out } else { n }
}

/// Multiplies a number by ten raised to `exp`.
///
/// The factor is always formed as a positive power of ten and then multiplied
/// or divided by, so it is exact wherever a power of ten is itself exactly
/// representable, which is up to the twenty-second, and within a unit in the
/// last place beyond that. The factor is split in two when a single power
/// would overflow, so a small number can be scaled up by more than the type's
/// own exponent range without passing through infinity on the way. A plain
/// `n * 10f64.powi(exp)` does neither.
pub fn mul_pow10(n: f64, exp: i32) -> f64 {
    if exp.abs() <= 300 {
        let p = 10.0f64.powi(exp.abs());
        if exp >= 0 { n * p } else { n / p }
    } else {
        let half = exp / 2;
        mul_pow10(mul_pow10(n, half), exp - half)
    }
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

#[cfg(test)]
mod round_to_sf_tests {
    use super::*;

    /// Every expected value below is worked out by hand from the decimal
    /// digits, not read off the implementation: write the number in the form
    /// d.ddd x 10^k, keep sf digits of the significand, and round the last one
    /// half away from zero.

    // Values of one and above, which the function has always handled. These
    // pin the behaviour that must not change. //

    #[test]
    fn test_round_to_sf_above_one_01() {
        // 1234 is 1.234 x 10^3; three figures keep 1.23, so 1230.
        assert_eq!(round_to_sf(1234.0, 3), 1230.0);
        // 1236 is 1.236 x 10^3; the fourth digit is 6, so the third rounds up.
        assert_eq!(round_to_sf(1236.0, 3), 1240.0);
        // 98765 is 9.8765 x 10^4; two figures keep 9.9, so 99000.
        assert_eq!(round_to_sf(98765.0, 2), 99000.0);
        // 9.99 to two figures carries into a new decade: 10.
        assert_eq!(round_to_sf(9.99, 2), 10.0);
        assert_eq!(round_to_sf(1.0, 3), 1.0);
        assert_eq!(round_to_sf(1.0e6, 3), 1.0e6);
    }

    // The same values negated. A magnitude does not depend on a sign, so each
    // expectation is the mirror of the one above. //

    #[test]
    fn test_round_to_sf_negative_above_one_01() {
        assert_eq!(round_to_sf(-1234.0, 3), -1230.0);
        assert_eq!(round_to_sf(-1236.0, 3), -1240.0);
        assert_eq!(round_to_sf(-98765.0, 2), -99000.0);
        assert_eq!(round_to_sf(-9.99, 2), -10.0);
        assert_eq!(round_to_sf(-1.0, 3), -1.0);
        assert_eq!(round_to_sf(-1.0e6, 3), -1.0e6);
    }

    // Values between zero and one, where the leading digit sits to the right
    // of the point. //

    #[test]
    fn test_round_to_sf_below_one_01() {
        // 0.4749 is 4.749 x 10^-1; two figures keep 4.7, so 0.47.
        assert_eq!(round_to_sf(0.4749, 2), 0.47);
        // 0.4751 is 4.751 x 10^-1; the third digit is 5 with more behind it.
        assert_eq!(round_to_sf(0.4751, 2), 0.48);
        // 0.05512 is 5.512 x 10^-2; two figures keep 5.5, so 0.055.
        assert_eq!(round_to_sf(0.05512, 2), 0.055);
        // 0.9994 is 9.994 x 10^-1; three figures keep 9.99, so 0.999.
        assert_eq!(round_to_sf(0.9994, 3), 0.999);
        // 0.9996 rounds up through the decade to 1.00.
        assert_eq!(round_to_sf(0.9996, 3), 1.0);
        // 0.0999 is 9.99 x 10^-2; two figures carry to 1.0 x 10^-1.
        assert_eq!(round_to_sf(0.0999, 2), 0.1);
    }

    #[test]
    fn test_round_to_sf_negative_below_one_01() {
        assert_eq!(round_to_sf(-0.4749, 2), -0.47);
        assert_eq!(round_to_sf(-0.4751, 2), -0.48);
        assert_eq!(round_to_sf(-0.05512, 2), -0.055);
        assert_eq!(round_to_sf(-0.9994, 3), -0.999);
        assert_eq!(round_to_sf(-0.9996, 3), -1.0);
        assert_eq!(round_to_sf(-0.0999, 2), -0.1);
    }

    /// The two readings that exposed the defect: a slope of -0.475 to two
    /// figures is -0.48, and -0.5496 is -0.55.
    #[test]
    fn test_round_to_sf_reported_slopes_01() {
        assert_eq!(round_to_sf(-0.475, 2), -0.48);
        assert_eq!(round_to_sf(-0.5496, 2), -0.55);
        assert_eq!(round_to_sf(-0.055, 2), -0.055);
    }

    /// A tie is decided away from zero, so 0.125 to two figures is 0.13 and
    /// 2.5 to one figure is 3.
    #[test]
    fn test_round_to_sf_ties_go_away_from_zero_01() {
        assert_eq!(round_to_sf(0.125, 2), 0.13);
        assert_eq!(round_to_sf(-0.125, 2), -0.13);
        assert_eq!(round_to_sf(2.5, 1), 3.0);
        assert_eq!(round_to_sf(-2.5, 1), -3.0);
        assert_eq!(round_to_sf(1.5, 1), 2.0);
        assert_eq!(round_to_sf(-1.5, 1), -2.0);
        assert_eq!(round_to_sf(0.25, 1), 0.3);
        assert_eq!(round_to_sf(-0.25, 1), -0.3);
    }

    /// An exact power of ten is already at one significant figure, so it comes
    /// back unchanged whatever count is asked for and whatever its sign.
    #[test]
    fn test_round_to_sf_powers_of_ten_01() {
        for exp in -320i32..=308 {
            let v = match format!("1e{}", exp).parse::<f64>() {
                Ok(v)   => v,
                Err(_)  => continue,
            };
            for sf in 1..=6u8 {
                assert_eq!(round_to_sf(v, sf), v, "10^{} at {} sf", exp, sf);
                assert_eq!(round_to_sf(-v, sf), -v, "-10^{} at {} sf", exp, sf);
            }
        }
    }

    /// Rounding is symmetric about zero, so the result for a negative value is
    /// the negation of the result for its magnitude.
    #[test]
    fn test_round_to_sf_sign_symmetry_01() {
        let mut v = 3.0e-7;
        for _ in 0..2000 {
            for sf in 1..=6u8 {
                assert_eq!(round_to_sf(-v, sf), -round_to_sf(v, sf), "{:e} at {} sf", v, sf);
            }
            v *= 1.017;
        }
    }

    /// A zero, a non-finite value, or a figure count of zero is returned
    /// unchanged rather than producing a not-a-number.
    #[test]
    fn test_round_to_sf_degenerate_01() {
        assert_eq!(round_to_sf(0.0, 3), 0.0);
        assert_eq!(round_to_sf(-0.0, 3), 0.0);
        assert_eq!(round_to_sf(12.34, 0), 12.34);
        assert!(round_to_sf(f64::NAN, 3).is_nan());
        assert_eq!(round_to_sf(f64::INFINITY, 3), f64::INFINITY);
        assert_eq!(round_to_sf(f64::NEG_INFINITY, 3), f64::NEG_INFINITY);
    }

    /// The extremes of the range stay finite and keep their magnitude.
    #[test]
    fn test_round_to_sf_extremes_01() {
        assert_eq!(round_to_sf(1.0e300, 3), 1.0e300);
        assert_eq!(round_to_sf(-1.0e300, 3), -1.0e300);
        assert_eq!(round_to_sf(1.0e-23, 3), 1.0e-23);
        assert!(round_to_sf(f64::MAX, 3).is_finite());
        assert!(round_to_sf(1.0e-320, 3) > 0.0);
    }

    /// A significand of 1.2345 rounds to 1.23 at every decade the type can
    /// hold, to within a unit in the last place.
    #[test]
    fn test_round_to_sf_across_decades_01() {
        for exp in -300i32..=300 {
            let v = match format!("1.2345e{}", exp).parse::<f64>() {
                Ok(v)   => v,
                Err(_)  => continue,
            };
            let want = match format!("1.23e{}", exp).parse::<f64>() {
                Ok(w)   => w,
                Err(_)  => continue,
            };
            let got = round_to_sf(v, 3);
            let rel = ((got - want) / want).abs();
            assert!(rel < 1.0e-15, "1.2345e{} gave {:e}, wanted {:e}", exp, got, want);
        }
    }
}

#[cfg(test)]
mod mul_pow10_tests {
    use super::*;

    /// A power of ten up to the twenty-second is exactly representable, so
    /// scaling one by it must give the literal back bit for bit.
    #[test]
    fn test_mul_pow10_exact_range_01() {
        for exp in -22i32..=22 {
            let want = match format!("1e{}", exp).parse::<f64>() {
                Ok(v)   => v,
                Err(_)  => continue,
            };
            assert_eq!(mul_pow10(1.0, exp), want, "10^{}", exp);
            assert_eq!(mul_pow10(-1.0, exp), -want, "-10^{}", exp);
        }
        assert_eq!(mul_pow10(1.234, 0), 1.234);
    }

    /// The factor is split where a single power of ten would overflow, so a
    /// small number can be scaled up by more than the type's own exponent
    /// range without passing through infinity on the way. A plain
    /// `n * 10f64.powi(320)` gives infinity here.
    #[test]
    fn test_mul_pow10_beyond_a_single_power_01() {
        assert_eq!(1.0e-300 * 10.0f64.powi(320), f64::INFINITY);
        let got = mul_pow10(1.0e-300, 320);
        assert!(((got - 1.0e20) / 1.0e20).abs() < 1.0e-15, "got {:e}", got);
        let got = mul_pow10(1.0e300, -320);
        assert!(((got - 1.0e-20) / 1.0e-20).abs() < 1.0e-15, "got {:e}", got);
    }

    /// Scaling up and back down again returns the value it started from, bit
    /// for bit while the power of ten is exact and to within a unit in the
    /// last place beyond that.
    #[test]
    fn test_mul_pow10_round_trip_01() {
        let v = 1.234567;
        for exp in -22i32..=22 {
            assert_eq!(mul_pow10(mul_pow10(v, exp), -exp), v, "10^{}", exp);
        }
        for exp in -300i32..=300 {
            let got = mul_pow10(mul_pow10(v, exp), -exp);
            assert!(((got - v) / v).abs() < 4.0 * f64::EPSILON, "10^{} gave {}", exp, got);
        }
    }
}
