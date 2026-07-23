//! Dimensional algebra over the SI base dimensions.
//!
//! A [`Dimension`] is a vector of rational exponents, one per base dimension.
//! Multiplying two dimensioned quantities adds their exponents, dividing
//! subtracts them, and raising to a power scales them. Two dimensions are
//! equal only when every exponent matches, which is what lets a dimensioned
//! value reject an addition that would violate physics.
//!
//! The seven SI base dimensions are length, mass, time, electric current,
//! thermodynamic temperature, amount of substance and luminous intensity. A
//! plane angle (radian) is not an SI base dimension, but it is carried here as
//! an eighth, tagged slot so that an angle cannot be silently added to a bare
//! dimensionless number.

use oxedyne_fe2o3_core::prelude::*;

/// Greatest common divisor of two integers, always non-negative and at least
/// one so that it is safe to divide by.
fn gcd(a: i32, b: i32) -> i32 {
    let mut a = a.abs();
    let mut b = b.abs();
    while b != 0 {
        let t = b;   // Remember divisor.
        b = a % b;
        a = t;
    }
    if a == 0 { 1 } else { a }
}

/// A rational exponent, always stored in reduced form with a strictly positive
/// denominator. Rational (rather than integer) exponents allow roots, so that
/// the square root of an area is a length.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct Ratio {
    num: i32, // Numerator.
    den: i32, // Denominator, always > 0.
}

impl Ratio {

    /// The rational zero, `0/1`.
    pub const ZERO: Self = Self { num: 0, den: 1 };

    /// The rational one, `1/1`.
    pub const ONE: Self = Self { num: 1, den: 1 };

    /// Creates a reduced ratio from an integer.
    pub fn int(n: i32) -> Self {
        Self { num: n, den: 1 }
    }

    /// Creates a reduced ratio from a numerator and denominator, erroring on a
    /// zero denominator.
    pub fn frac(num: i32, den: i32) -> Outcome<Self> {
        if den == 0 {
            return Err(err!(
                "A rational exponent cannot have a zero denominator.";
            Input, Invalid));
        }
        Ok(Self::reduce(num, den))
    }

    /// Reduces a numerator and denominator to lowest terms with a positive
    /// denominator. The denominator is assumed to be non-zero.
    fn reduce(mut num: i32, mut den: i32) -> Self {
        if den < 0 {
            num = -num;   // Keep the sign on the numerator.
            den = -den;
        }
        let g = gcd(num, den);
        Self { num: num / g, den: den / g }
    }

    /// Returns the numerator.
    pub fn num(&self) -> i32 { self.num }

    /// Returns the denominator.
    pub fn den(&self) -> i32 { self.den }

    /// Returns `true` if the ratio is zero.
    pub fn is_zero(&self) -> bool { self.num == 0 }

    /// Adds two ratios.
    pub fn add(&self, other: &Self) -> Self {
        Self::reduce(
            self.num * other.den + other.num * self.den,
            self.den * other.den,
        )
    }

    /// Subtracts one ratio from another.
    pub fn sub(&self, other: &Self) -> Self {
        Self::reduce(
            self.num * other.den - other.num * self.den,
            self.den * other.den,
        )
    }

    /// Multiplies two ratios, used when raising a dimension to a rational
    /// power.
    pub fn mul(&self, other: &Self) -> Self {
        Self::reduce(self.num * other.num, self.den * other.den)
    }

    /// Returns the exponent as a floating point value, used when raising a
    /// magnitude to this power.
    pub fn as_f64(&self) -> f64 {
        (self.num as f64) / (self.den as f64)
    }
}

impl std::fmt::Display for Ratio {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.den == 1 {
            write!(f, "{}", self.num)
        } else {
            write!(f, "{}/{}", self.num, self.den)
        }
    }
}

/// The base dimensions carried by a [`Dimension`] vector, in index order. The
/// first seven are the SI base dimensions; `Angle` is a tagged plane-angle
/// slot kept separate from them so that a radian is distinct from a bare
/// number.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Base {
    Length,             // metre, m
    Mass,               // kilogram, kg
    Time,               // second, s
    Current,            // ampere, A
    Temperature,        // kelvin, K
    Amount,             // mole, mol
    LuminousIntensity,  // candela, cd
    Angle,              // radian, rad (tagged, non-SI)
}

impl Base {

    /// The number of base slots, seven SI dimensions plus the tagged angle.
    pub const COUNT: usize = 8;

    /// The base dimensions in index order.
    pub const ALL: [Base; Self::COUNT] = [
        Base::Length,
        Base::Mass,
        Base::Time,
        Base::Current,
        Base::Temperature,
        Base::Amount,
        Base::LuminousIntensity,
        Base::Angle,
    ];

    /// Returns the index of this base within a dimension vector.
    pub fn index(&self) -> usize {
        match self {
            Base::Length            => 0,
            Base::Mass              => 1,
            Base::Time              => 2,
            Base::Current           => 3,
            Base::Temperature       => 4,
            Base::Amount            => 5,
            Base::LuminousIntensity => 6,
            Base::Angle             => 7,
        }
    }

    /// Returns the SI symbol for this base dimension.
    pub fn symbol(&self) -> &'static str {
        match self {
            Base::Length            => "m",
            Base::Mass              => "kg",
            Base::Time              => "s",
            Base::Current           => "A",
            Base::Temperature       => "K",
            Base::Amount            => "mol",
            Base::LuminousIntensity => "cd",
            Base::Angle             => "rad",
        }
    }
}

/// A physical dimension expressed as rational exponents over the base
/// dimensions.
///
/// Construction is arbitrary: any combination of base exponents is a valid
/// dimension. Dimensions form a group under multiplication (exponent addition),
/// with [`Dimension::dimensionless`] as the identity.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct Dimension {
    exp: [Ratio; Base::COUNT],
}

impl Dimension {

    /// Creates a dimension from a full vector of rational exponents.
    pub fn new(exp: [Ratio; Base::COUNT]) -> Self {
        Self { exp }
    }

    /// The dimensionless quantity, all exponents zero. This is the identity for
    /// multiplication.
    pub fn dimensionless() -> Self {
        Self { exp: [Ratio::ZERO; Base::COUNT] }
    }

    /// Creates a dimension that is a single base raised to the first power.
    pub fn base(b: Base) -> Self {
        let mut exp = [Ratio::ZERO; Base::COUNT];
        exp[b.index()] = Ratio::ONE;
        Self { exp }
    }

    /// The exponent of a given base dimension.
    pub fn exponent(&self, b: Base) -> Ratio {
        self.exp[b.index()]
    }

    /// Returns `true` if every exponent is zero, i.e. the quantity is
    /// dimensionless.
    pub fn is_dimensionless(&self) -> bool {
        self.exp.iter().all(|r| r.is_zero())
    }

    /// Multiplies two dimensions by adding their exponents.
    pub fn mul(&self, other: &Self) -> Self {
        let mut exp = [Ratio::ZERO; Base::COUNT];
        for i in 0..Base::COUNT {
            exp[i] = self.exp[i].add(&other.exp[i]);
        }
        Self { exp }
    }

    /// Divides one dimension by another by subtracting exponents.
    pub fn div(&self, other: &Self) -> Self {
        let mut exp = [Ratio::ZERO; Base::COUNT];
        for i in 0..Base::COUNT {
            exp[i] = self.exp[i].sub(&other.exp[i]);
        }
        Self { exp }
    }

    /// Raises a dimension to an integer power, scaling every exponent.
    pub fn powi(&self, n: i32) -> Self {
        self.pow(&Ratio::int(n))
    }

    /// Raises a dimension to a rational power, scaling every exponent. This is
    /// how a root reduces a dimension, e.g. the square root of an area (`L^2`)
    /// is a length (`L`).
    pub fn pow(&self, p: &Ratio) -> Self {
        let mut exp = [Ratio::ZERO; Base::COUNT];
        for i in 0..Base::COUNT {
            exp[i] = self.exp[i].mul(p);
        }
        Self { exp }
    }

    /// Returns the reciprocal dimension, negating every exponent.
    pub fn recip(&self) -> Self {
        Self::dimensionless().div(self)
    }

    // Convenience constructors for the base dimensions. //

    /// Length, `L`.
    pub fn length() -> Self { Self::base(Base::Length) }
    /// Mass, `M`.
    pub fn mass() -> Self { Self::base(Base::Mass) }
    /// Time, `T`.
    pub fn time() -> Self { Self::base(Base::Time) }
    /// Electric current, `I`.
    pub fn current() -> Self { Self::base(Base::Current) }
    /// Thermodynamic temperature, `Θ`.
    pub fn temperature() -> Self { Self::base(Base::Temperature) }
    /// Amount of substance, `N`.
    pub fn amount() -> Self { Self::base(Base::Amount) }
    /// Luminous intensity, `J`.
    pub fn luminous_intensity() -> Self { Self::base(Base::LuminousIntensity) }
    /// Plane angle, tagged and distinct from dimensionless.
    pub fn angle() -> Self { Self::base(Base::Angle) }

    // Convenience constructors for common derived dimensions. //

    /// Area, `L^2`.
    pub fn area() -> Self { Self::length().powi(2) }
    /// Volume, `L^3`.
    pub fn volume() -> Self { Self::length().powi(3) }
    /// Velocity, `L·T^-1`.
    pub fn velocity() -> Self { Self::length().div(&Self::time()) }
    /// Acceleration, `L·T^-2`.
    pub fn acceleration() -> Self { Self::velocity().div(&Self::time()) }
    /// Force, `M·L·T^-2`.
    pub fn force() -> Self { Self::mass().mul(&Self::acceleration()) }
    /// Energy, `M·L^2·T^-2`.
    pub fn energy() -> Self { Self::force().mul(&Self::length()) }
    /// Power, `M·L^2·T^-3`.
    pub fn power() -> Self { Self::energy().div(&Self::time()) }
    /// Frequency, `T^-1`.
    pub fn frequency() -> Self { Self::dimensionless().div(&Self::time()) }
}

impl std::ops::Mul for Dimension {
    type Output = Dimension;
    fn mul(self, other: Dimension) -> Dimension {
        Dimension::mul(&self, &other)
    }
}

impl std::ops::Div for Dimension {
    type Output = Dimension;
    fn div(self, other: Dimension) -> Dimension {
        Dimension::div(&self, &other)
    }
}

impl std::fmt::Display for Dimension {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.is_dimensionless() {
            return write!(f, "1");
        }
        let mut first = true;
        for b in Base::ALL.iter() {
            let e = self.exp[b.index()];
            if e.is_zero() {
                continue;
            }
            if !first {
                ok!(write!(f, "·"));
            }
            first = false;
            if e == Ratio::ONE {
                ok!(write!(f, "{}", b.symbol()));
            } else {
                ok!(write!(f, "{}^{}", b.symbol(), e));
            }
        }
        Ok(())
    }
}
