//! External-oracle tests for the dimensional algebra.
//!
//! The expected values here come from physics and metrology, not from the
//! implementation: force is `M·L·T^-2`, an electronvolt is 1.602176634e-19 J,
//! half a turn is π radians, and adding a length to a time is a category error.

use oxedyne_fe2o3_units::{
    dimension::{
        Base,
        Dimension,
        Ratio,
    },
    quantity::{
        Quantity,
        ELECTRONVOLT_J,
    },
};

use oxedyne_fe2o3_core::prelude::*;

/// Asserts two floats agree to a relative tolerance.
fn close(a: f64, b: f64) -> bool {
    if b == 0.0 {
        a.abs() < 1.0e-12
    } else {
        ((a - b) / b).abs() < 1.0e-9
    }
}

#[test]
fn test_dimensionless_identity_01() -> Outcome<()> {
    let d = Dimension::dimensionless();
    assert!(d.is_dimensionless());
    // Multiplying by the identity changes nothing.
    let l = Dimension::length();
    assert_eq!(l.mul(&d), l);
    Ok(())
}

#[test]
fn test_velocity_times_time_is_length_01() -> Outcome<()> {
    // (L/T) * T = L.
    let v = Dimension::velocity();
    let t = Dimension::time();
    assert_eq!(v.mul(&t), Dimension::length());
    Ok(())
}

#[test]
fn test_force_is_mass_times_acceleration_01() -> Outcome<()> {
    // Force = M·L·T^-2, arrived at from F = m·a.
    let m = Dimension::mass();
    let a = Dimension::acceleration();
    assert_eq!(m.mul(&a), Dimension::force());
    // And the exponents are exactly what physics says.
    let f = Dimension::force();
    assert_eq!(f.exponent(Base::Mass), Ratio::int(1));
    assert_eq!(f.exponent(Base::Length), Ratio::int(1));
    assert_eq!(f.exponent(Base::Time), Ratio::int(-2));
    Ok(())
}

#[test]
fn test_area_is_length_squared_01() -> Outcome<()> {
    // Area = L^2.
    let l = Dimension::length();
    assert_eq!(l.powi(2), Dimension::area());
    assert_eq!(Dimension::area().exponent(Base::Length), Ratio::int(2));
    Ok(())
}

#[test]
fn test_root_of_area_is_length_01() -> Outcome<()> {
    // sqrt(L^2) = L, exercising rational exponents.
    let half = res!(Ratio::frac(1, 2));
    assert_eq!(Dimension::area().pow(&half), Dimension::length());
    Ok(())
}

#[test]
fn test_quantity_velocity_times_time_01() -> Outcome<()> {
    // 20 m/s for 3 s covers 60 m, and the result is a length.
    let speed = res!(Quantity::metres(20.0, 4)).div(&res!(Quantity::seconds(1.0, 4)));
    let time = res!(Quantity::seconds(3.0, 4));
    let dist = speed.mul(&time);
    assert!(close(dist.val(), 60.0));
    assert_eq!(dist.dim(), Dimension::length());
    Ok(())
}

#[test]
fn test_quantity_force_from_mass_and_acceleration_01() -> Outcome<()> {
    // 2 kg at 3 m/s^2 gives 6 N with the force dimension.
    let mass = res!(Quantity::kilograms(2.0, 4));
    let acc = res!(Quantity::new(3.0, 4, Dimension::acceleration()));
    let force = mass.mul(&acc);
    assert!(close(force.val(), 6.0));
    assert_eq!(force.dim(), Dimension::force());
    Ok(())
}

#[test]
fn test_add_same_dimension_ok_01() -> Outcome<()> {
    let a = res!(Quantity::metres(5.0, 4));
    let b = res!(Quantity::metres(3.0, 4));
    let sum = res!(a.add(&b));
    assert!(close(sum.val(), 8.0));
    assert_eq!(sum.dim(), Dimension::length());
    Ok(())
}

#[test]
fn test_add_mismatched_dimension_errors_01() -> Outcome<()> {
    // Adding a length to a time must be rejected.
    let length = res!(Quantity::metres(5.0, 4));
    let time = res!(Quantity::seconds(3.0, 4));
    assert!(length.add(&time).is_err());
    Ok(())
}

#[test]
fn test_sub_mismatched_dimension_errors_01() -> Outcome<()> {
    let energy = res!(Quantity::joules(1.0, 4));
    let mass = res!(Quantity::kilograms(1.0, 4));
    assert!(energy.sub(&mass).is_err());
    Ok(())
}

#[test]
fn test_electronvolt_in_joules_01() -> Outcome<()> {
    // 1 eV = 1.602176634e-19 J (exact by definition).
    let ev = res!(Quantity::electronvolts(1.0, 10));
    assert!(close(ev.val(), 1.602176634e-19));
    assert!(close(ev.val(), ELECTRONVOLT_J));
    assert_eq!(ev.dim(), Dimension::energy());
    Ok(())
}

#[test]
fn test_degrees_to_radians_01() -> Outcome<()> {
    // 180 degrees = π radians.
    let half_turn = res!(Quantity::degrees(180.0, 10));
    assert!(close(half_turn.val(), std::f64::consts::PI));
    assert_eq!(half_turn.dim(), Dimension::angle());
    Ok(())
}

#[test]
fn test_angle_is_not_dimensionless_01() -> Outcome<()> {
    // A radian is tagged, so it cannot be added to a bare number.
    let angle = res!(Quantity::radians(1.0, 4));
    let bare = res!(Quantity::dimensionless(1.0, 4));
    assert_ne!(angle.dim(), Dimension::dimensionless());
    assert!(angle.add(&bare).is_err());
    Ok(())
}

#[test]
fn test_litre_is_cubic_metres_01() -> Outcome<()> {
    // 1 L = 1e-3 m^3, with the volume dimension.
    let vol = res!(Quantity::litres(1.0, 4));
    assert!(close(vol.val(), 1.0e-3));
    assert_eq!(vol.dim(), Dimension::volume());
    Ok(())
}

#[test]
fn test_minutes_and_hours_to_seconds_01() -> Outcome<()> {
    let m = res!(Quantity::minutes(1.0, 4));
    let h = res!(Quantity::hours(1.0, 4));
    assert!(close(m.val(), 60.0));
    assert!(close(h.val(), 3600.0));
    assert_eq!(m.dim(), Dimension::time());
    assert_eq!(h.dim(), Dimension::time());
    Ok(())
}

#[test]
fn test_sigfig_multiply_takes_minimum_01() -> Outcome<()> {
    // 2.0 (2 sf) × 3.00 (3 sf) → 2 sf.
    let a = res!(Quantity::dimensionless(2.0, 2));
    let b = res!(Quantity::dimensionless(3.00, 3));
    let p = a.mul(&b);
    assert_eq!(p.sf(), 2);
    assert!(close(p.rounded(), 6.0));
    Ok(())
}

#[test]
fn test_sigfig_divide_takes_minimum_01() -> Outcome<()> {
    // 6.000 (4 sf) / 3.0 (2 sf) → 2 sf.
    let a = res!(Quantity::dimensionless(6.000, 4));
    let b = res!(Quantity::dimensionless(3.0, 2));
    let q = a.div(&b);
    assert_eq!(q.sf(), 2);
    Ok(())
}

#[test]
fn test_sigfig_add_by_decimal_place_01() -> Outcome<()> {
    // 12.11 (4 sf, tied to hundredths) + 0.1 (1 sf, tied to tenths) = 12.2,
    // which is coarse to the tenths and so carries 3 sf.
    let a = res!(Quantity::metres(12.11, 4));
    let b = res!(Quantity::metres(0.1, 1));
    let sum = res!(a.add(&b));
    assert_eq!(sum.sf(), 3);
    assert!(close(sum.rounded(), 12.2));
    Ok(())
}

#[test]
fn test_dimension_display_force_01() -> Outcome<()> {
    // The mismatch error names dimensions, so Display must be legible.
    let s = fmt!("{}", Dimension::force());
    assert!(s.contains("kg"));
    assert!(s.contains("m"));
    assert!(s.contains("s^-2"));
    Ok(())
}
