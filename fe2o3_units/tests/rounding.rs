//! External-oracle tests for significant figure rounding.
//!
//! Every expected value here is worked out by hand from the decimal digits and
//! not read off the implementation. Write the magnitude as `d.ddd x 10^k`, keep
//! the tracked number of digits of the significand, and round the last of them
//! half away from zero. A sign plays no part in that: the digits kept for
//! -0.475 are the digits kept for 0.475.

use oxedyne_fe2o3_units::{
    dimension::Dimension,
    quantity::Quantity,
};

use oxedyne_fe2o3_core::prelude::*;

#[test]
fn test_rounded_above_one_01() -> Outcome<()> {
    // 1234 m is 1.234 x 10^3 m; three figures keep 1.23, so 1230 m.
    assert_eq!(res!(Quantity::metres(1234.0, 3)).rounded(), 1230.0);
    // 1236 m rounds the third figure up, so 1240 m.
    assert_eq!(res!(Quantity::metres(1236.0, 3)).rounded(), 1240.0);
    // 98765 s is 9.8765 x 10^4 s; two figures keep 9.9, so 99000 s.
    assert_eq!(res!(Quantity::seconds(98765.0, 2)).rounded(), 99000.0);
    // 9.99 kg to two figures carries into a new decade: 10 kg.
    assert_eq!(res!(Quantity::kilograms(9.99, 2)).rounded(), 10.0);
    Ok(())
}

#[test]
fn test_rounded_negative_above_one_01() -> Outcome<()> {
    assert_eq!(res!(Quantity::metres(-1234.0, 3)).rounded(), -1230.0);
    assert_eq!(res!(Quantity::metres(-1236.0, 3)).rounded(), -1240.0);
    assert_eq!(res!(Quantity::seconds(-98765.0, 2)).rounded(), -99000.0);
    assert_eq!(res!(Quantity::kilograms(-9.99, 2)).rounded(), -10.0);
    Ok(())
}

#[test]
fn test_rounded_below_one_01() -> Outcome<()> {
    // 0.4749 is 4.749 x 10^-1; two figures keep 4.7.
    assert_eq!(res!(Quantity::dimensionless(0.4749, 2)).rounded(), 0.47);
    // 0.4751 rounds the second figure up.
    assert_eq!(res!(Quantity::dimensionless(0.4751, 2)).rounded(), 0.48);
    // 0.05512 is 5.512 x 10^-2; two figures keep 5.5.
    assert_eq!(res!(Quantity::dimensionless(0.05512, 2)).rounded(), 0.055);
    // 0.9996 carries through the decade to 1.00.
    assert_eq!(res!(Quantity::dimensionless(0.9996, 3)).rounded(), 1.0);
    Ok(())
}

/// A slope is dimensionless and usually smaller than one, which is the case
/// that went wrong: a gradient of -0.475 read to two figures is -0.48, and one
/// of -0.5496 is -0.55.
#[test]
fn test_rounded_negative_below_one_01() -> Outcome<()> {
    assert_eq!(res!(Quantity::dimensionless(-0.475, 2)).rounded(), -0.48);
    assert_eq!(res!(Quantity::dimensionless(-0.5496, 2)).rounded(), -0.55);
    assert_eq!(res!(Quantity::dimensionless(-0.055, 2)).rounded(), -0.055);
    assert_eq!(res!(Quantity::dimensionless(-0.4749, 2)).rounded(), -0.47);
    assert_eq!(res!(Quantity::dimensionless(-0.0999, 2)).rounded(), -0.1);
    Ok(())
}

/// A gradient formed by dividing a rise by a run keeps the coarser figure
/// count and rounds by the same rule as a value entered directly.
#[test]
fn test_rounded_slope_from_division_01() -> Outcome<()> {
    // -0.95 m over 2.0 m is -0.475, dimensionless, to two figures: -0.48.
    let rise = res!(Quantity::metres(-0.95, 2));
    let run = res!(Quantity::metres(2.0, 3));
    let slope = rise.div(&run);
    assert_eq!(slope.sf(), 2);
    assert!(slope.dim().is_dimensionless());
    assert_eq!(slope.rounded(), -0.48);
    Ok(())
}

/// An exact power of ten is already at one figure, so it survives any count
/// unchanged, either sign.
#[test]
fn test_rounded_powers_of_ten_01() -> Outcome<()> {
    for exp in -18i32..=18 {
        let v = match format!("1e{}", exp).parse::<f64>() {
            Ok(v)   => v,
            Err(_)  => continue,
        };
        for sf in 1..=5u8 {
            assert_eq!(res!(Quantity::dimensionless(v, sf)).rounded(), v,
                "10^{} at {} sf", exp, sf);
            assert_eq!(res!(Quantity::dimensionless(-v, sf)).rounded(), -v,
                "-10^{} at {} sf", exp, sf);
        }
    }
    Ok(())
}

/// A tie is settled away from zero, so 0.125 to two figures is 0.13 and 2.5 to
/// one figure is 3.
#[test]
fn test_rounded_ties_away_from_zero_01() -> Outcome<()> {
    assert_eq!(res!(Quantity::dimensionless(0.125, 2)).rounded(), 0.13);
    assert_eq!(res!(Quantity::dimensionless(-0.125, 2)).rounded(), -0.13);
    assert_eq!(res!(Quantity::dimensionless(2.5, 1)).rounded(), 3.0);
    assert_eq!(res!(Quantity::dimensionless(-2.5, 1)).rounded(), -3.0);
    Ok(())
}

/// A zero carries no scale, so it rounds to zero rather than to a
/// not-a-number.
#[test]
fn test_rounded_zero_01() -> Outcome<()> {
    assert_eq!(res!(Quantity::dimensionless(0.0, 3)).rounded(), 0.0);
    assert_eq!(res!(Quantity::new(0.0, 1, Dimension::force())).rounded(), 0.0);
    Ok(())
}

/// The displayed form uses the rounded magnitude, so a negative slope reads
/// with the figures it was measured to.
#[test]
fn test_display_uses_rounded_magnitude_01() -> Outcome<()> {
    let q = res!(Quantity::dimensionless(-0.475, 2));
    assert!(format!("{}", q).starts_with("-0.48"), "displayed as {}", q);
    Ok(())
}
