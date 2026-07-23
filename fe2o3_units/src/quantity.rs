//! Dimension-checked physical quantities.
//!
//! A [`Quantity`] carries a magnitude in SI base units, a count of significant
//! figures, and a [`Dimension`]. Its multiplication and division combine both
//! magnitude and dimension, so a velocity times a time yields a length without
//! any further declaration. Its addition and subtraction require the two
//! operands to share a dimension and return an [`Outcome`], erroring with both
//! dimensions named when they differ. That error is the check that catches a
//! physics mistake, such as adding a length to a time, at the point the code is
//! written.
//!
//! Significant figures propagate through the algebra: multiplication and
//! division keep the smaller of the two figure counts, while addition and
//! subtraction work by decimal place, keeping the coarser of the two least
//! significant places.

use crate::dimension::Dimension;

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_num::float;

/// The value of one electronvolt in joules, exact by the 2019 SI redefinition.
pub const ELECTRONVOLT_J: f64 = 1.602176634e-19;

/// A physical quantity: a magnitude in SI base units, a significant figure
/// count, and a dimension.
#[derive(Clone, Copy, Debug)]
pub struct Quantity {
    val: f64,       // Magnitude in coherent SI base units.
    sf:  u8,        // Significant figures.
    dim: Dimension, // Physical dimension.
}

impl Quantity {

    /// Creates a quantity from a magnitude, significant figure count and
    /// dimension. Errors when the figure count is zero.
    pub fn new(val: f64, sf: u8, dim: Dimension) -> Outcome<Self> {
        if sf == 0 {
            return Err(err!(
                "Number of significant figures must be > 0.";
            Input, Invalid));
        }
        Ok(Self { val, sf, dim })
    }

    /// The magnitude in SI base units.
    pub fn val(&self) -> f64 { self.val }

    /// The significant figure count.
    pub fn sf(&self) -> u8 { self.sf }

    /// The dimension.
    pub fn dim(&self) -> Dimension { self.dim }

    /// Returns the magnitude rounded to its tracked significant figures.
    pub fn rounded(&self) -> f64 {
        if self.val == 0.0 {
            0.0
        } else {
            float::round_to_sf(self.val, self.sf)
        }
    }

    /// The decimal place of the least significant digit, i.e. the power of ten
    /// of the last figure that is still significant. A larger value means a
    /// coarser measurement.
    fn least_place(&self) -> i32 {
        if self.val == 0.0 {
            return 0;   // A bare zero carries no scale.
        }
        (self.val.abs().log10().floor() as i32) - (self.sf as i32 - 1)
    }

    /// Multiplies two quantities, combining magnitude and dimension and keeping
    /// the smaller significant figure count.
    pub fn mul(&self, other: &Self) -> Self {
        Self {
            val: self.val * other.val,
            sf:  self.sf.min(other.sf),
            dim: self.dim.mul(&other.dim),
        }
    }

    /// Divides one quantity by another, combining magnitude and dimension and
    /// keeping the smaller significant figure count.
    pub fn div(&self, other: &Self) -> Self {
        Self {
            val: self.val / other.val,
            sf:  self.sf.min(other.sf),
            dim: self.dim.div(&other.dim),
        }
    }

    /// Raises a quantity to an integer power, combining magnitude and dimension.
    /// The significant figure count is preserved.
    pub fn powi(&self, n: i32) -> Self {
        Self {
            val: self.val.powi(n),
            sf:  self.sf,
            dim: self.dim.powi(n),
        }
    }

    /// Adds two quantities, requiring equal dimensions. Errors with both
    /// dimensions named on a mismatch. The result's significant figures follow
    /// the decimal place rule: the coarser of the two least significant places
    /// bounds the sum.
    pub fn add(&self, other: &Self) -> Outcome<Self> {
        if self.dim != other.dim {
            return Err(err!(
                "Cannot add quantities of differing dimension: {} vs {}.",
                self.dim, other.dim;
            Input, Invalid, Mismatch));
        }
        let val = self.val + other.val;
        Ok(Self {
            val,
            sf:  Self::sum_sf(self, other, val),
            dim: self.dim,
        })
    }

    /// Subtracts one quantity from another, requiring equal dimensions. Errors
    /// with both dimensions named on a mismatch, following the same decimal
    /// place rule for significant figures as [`Quantity::add`].
    pub fn sub(&self, other: &Self) -> Outcome<Self> {
        if self.dim != other.dim {
            return Err(err!(
                "Cannot subtract quantities of differing dimension: {} vs {}.",
                self.dim, other.dim;
            Input, Invalid, Mismatch));
        }
        let val = self.val - other.val;
        Ok(Self {
            val,
            sf:  Self::sum_sf(self, other, val),
            dim: self.dim,
        })
    }

    /// Derives the significant figures of a sum or difference from the decimal
    /// place rule. The result is significant only as far as the coarser of the
    /// two inputs' least significant places.
    fn sum_sf(a: &Self, b: &Self, result: f64) -> u8 {
        if result == 0.0 {
            return a.sf.min(b.sf);   // No magnitude to count against.
        }
        let place = a.least_place().max(b.least_place());
        let most = result.abs().log10().floor() as i32; // Place of leading digit.
        let sf = most - place + 1;
        if sf < 1 { 1 } else { sf as u8 }
    }

    // Constructors in SI base units. //

    /// A length in metres.
    pub fn metres(val: f64, sf: u8) -> Outcome<Self> {
        Self::new(val, sf, Dimension::length())
    }

    /// A mass in kilograms.
    pub fn kilograms(val: f64, sf: u8) -> Outcome<Self> {
        Self::new(val, sf, Dimension::mass())
    }

    /// A duration in seconds.
    pub fn seconds(val: f64, sf: u8) -> Outcome<Self> {
        Self::new(val, sf, Dimension::time())
    }

    /// An energy in joules.
    pub fn joules(val: f64, sf: u8) -> Outcome<Self> {
        Self::new(val, sf, Dimension::energy())
    }

    /// A plane angle in radians, tagged distinct from a dimensionless number.
    pub fn radians(val: f64, sf: u8) -> Outcome<Self> {
        Self::new(val, sf, Dimension::angle())
    }

    /// A dimensionless quantity.
    pub fn dimensionless(val: f64, sf: u8) -> Outcome<Self> {
        Self::new(val, sf, Dimension::dimensionless())
    }

    // Common non-SI units used in teaching, each converted to SI base units at
    // construction so that the rest of the algebra never sees them. //

    /// A plane angle in degrees, converted to radians. Half a turn, 180
    /// degrees, is π radians.
    pub fn degrees(val: f64, sf: u8) -> Outcome<Self> {
        Self::new(val * std::f64::consts::PI / 180.0, sf, Dimension::angle())
    }

    /// A volume in litres, converted to cubic metres.
    pub fn litres(val: f64, sf: u8) -> Outcome<Self> {
        Self::new(val * 1.0e-3, sf, Dimension::volume())
    }

    /// An energy in electronvolts, converted to joules.
    pub fn electronvolts(val: f64, sf: u8) -> Outcome<Self> {
        Self::new(val * ELECTRONVOLT_J, sf, Dimension::energy())
    }

    /// A duration in minutes, converted to seconds.
    pub fn minutes(val: f64, sf: u8) -> Outcome<Self> {
        Self::new(val * 60.0, sf, Dimension::time())
    }

    /// A duration in hours, converted to seconds.
    pub fn hours(val: f64, sf: u8) -> Outcome<Self> {
        Self::new(val * 3600.0, sf, Dimension::time())
    }
}

impl std::ops::Mul for Quantity {
    type Output = Quantity;
    fn mul(self, other: Quantity) -> Quantity {
        Quantity::mul(&self, &other)
    }
}

impl std::ops::Div for Quantity {
    type Output = Quantity;
    fn div(self, other: Quantity) -> Quantity {
        Quantity::div(&self, &other)
    }
}

impl std::fmt::Display for Quantity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} {}", self.rounded(), self.dim)
    }
}
