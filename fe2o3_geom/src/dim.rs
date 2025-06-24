use oxedyne_fe2o3_core::prelude::*;

use std::{
    fmt,
    iter::Sum,
};


new_type!(Dim, usize, Clone, Copy, Debug, Default, Eq, Hash, Ord);

impl fmt::Display for Dim {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl<T> From<T> for Dim
where
    usize: From<T>,
{
    fn from(value: T) -> Self {
        Dim(usize::from(value))
    }
}

impl std::ops::Add for Dim {
    type Output = Self;

    fn add(self, other: Self) -> Self {
        Self(self.0.saturating_add(other.0))
    }
}

impl std::ops::Add<usize> for Dim {
    type Output = Self;

    fn add(self, other: usize) -> Self {
        Self(self.0.saturating_add(other))
    }
}

impl std::ops::Add<Dim> for usize {
    type Output = Dim;

    fn add(self, other: Dim) -> Dim {
        Dim(self.saturating_add(other.0))
    }
}

impl std::ops::Sub for Dim {
    type Output = Self;

    fn sub(self, other: Self) -> Self {
        Self(self.0.saturating_sub(other.0))
    }
}

impl std::ops::Sub<usize> for Dim {
    type Output = Self;

    fn sub(self, other: usize) -> Self {
        Self(self.0.saturating_sub(other))
    }
}

impl std::ops::Sub<Dim> for usize {
    type Output = Dim;

    fn sub(self, other: Dim) -> Dim {
        Dim(self.saturating_sub(other.0))
    }
}

impl std::ops::Mul for Dim {
    type Output = Self;

    fn mul(self, other: Self) -> Self {
        Self(self.0.saturating_mul(other.0))
    }
}

impl std::ops::Mul<usize> for Dim {
    type Output = Self;

    fn mul(self, other: usize) -> Self {
        Self(self.0.saturating_mul(other))
    }
}

impl std::ops::Mul<Dim> for usize {
    type Output = Dim;

    fn mul(self, other: Dim) -> Dim {
        Dim(self.saturating_mul(other.0))
    }
}

impl std::ops::Div for Dim {
    type Output = Self;

    fn div(self, other: Self) -> Self {
        Self(self.0.saturating_div(other.0))
    }
}

impl std::ops::Div<usize> for Dim {
    type Output = Self;

    fn div(self, other: usize) -> Self {
        Self(self.0.saturating_div(other))
    }
}

impl std::ops::Div<Dim> for usize {
    type Output = Dim;

    fn div(self, other: Dim) -> Dim {
        Dim(self.saturating_div(other.0))
    }
}

impl std::ops::Rem for Dim {
    type Output = Self;

    fn rem(self, other: Self) -> Self {
        Self(self.0.rem(other.0))
    }
}

impl std::ops::Rem<usize> for Dim {
    type Output = Self;

    fn rem(self, other: usize) -> Self {
        Self(self.0.rem(other))
    }
}

impl std::ops::Rem<Dim> for usize {
    type Output = Dim;

    fn rem(self, other: Dim) -> Dim {
        Dim(self.rem(other.0))
    }
}

impl<T: Copy> PartialEq<T> for Dim
where
    usize: From<T>,
{
    fn eq(&self, other: &T) -> bool {
        self.0 == usize::from(*other)
    }
}

impl PartialEq<Dim> for Dim {
    fn eq(&self, other: &Dim) -> bool {
        self.0 == other.0
    }
}

impl PartialEq<Dim> for usize {
    fn eq(&self, other: &Dim) -> bool {
        *self == other.0
    }
}

impl<T: Copy> PartialOrd<T> for Dim
where
    usize: From<T>,
{
    fn partial_cmp(&self, other: &T) -> Option<std::cmp::Ordering> {
        self.0.partial_cmp(&usize::from(*other))
    }
}

impl PartialOrd<Dim> for Dim {
    fn partial_cmp(&self, other: &Dim) -> Option<std::cmp::Ordering> {
        self.0.partial_cmp(&other.0)
    }
}

impl PartialOrd<Dim> for usize {
    fn partial_cmp(&self, other: &Dim) -> Option<std::cmp::Ordering> {
        self.partial_cmp(&other.0)
    }
}

impl std::ops::AddAssign for Dim {
    fn add_assign(&mut self, other: Self) {
        self.0 = self.0.saturating_add(other.0);
    }
}

impl std::ops::AddAssign<usize> for Dim {
    fn add_assign(&mut self, other: usize) {
        self.0 = self.0.saturating_add(other);
    }
}

impl std::ops::SubAssign for Dim {
    fn sub_assign(&mut self, other: Self) {
        self.0 = self.0.saturating_sub(other.0);
    }
}

impl std::ops::SubAssign<usize> for Dim {
    fn sub_assign(&mut self, other: usize) {
        self.0 = self.0.saturating_sub(other);
    }
}

impl Sum for Dim {
    fn sum<I: Iterator<Item = Self>>(iter: I) -> Self {
        iter.fold(Dim(0), |acc, x| acc + x)
    }
}

impl Dim {

    pub fn new<N: Into<usize>>(n: N) -> Self {
        Self(n.into())
    }

    pub fn as_usize(&self) -> usize {
        self.0
    }

    pub fn as_index(&self) -> usize {
        self.0
    }

    pub fn checked_div(&self, other: Self) -> Option<Self> {
        if other.0 == 0 {
            None
        } else {
            Some(*self / other)
        }
    }
}

new_type!(Span, (Dim, Dim), Clone, Debug, Default, Eq, Ord, PartialEq, PartialOrd);

impl From<(Dim, Dim)> for Span {
    fn from((x, y): (Dim, Dim)) -> Self { Self((x, y)) }
}

impl Span {

    pub fn new<
        X: Into<Dim>,
        Y: Into<Dim>,
    >(
        (x, y): (X, Y),
    )
        -> Self
    {
        Self((
            x.into(),
            y.into(),
        ))
    }

    pub fn tup(&self) -> (Dim, Dim) { (self.0.0, self.0.1) }
    pub fn start(&self) -> Dim      { self.0.0 }
    pub fn len(&self) -> Dim        { self.0.1 }

    /// Clip `x2` and `len2` by `x1` and `len1` so that that latter stays within the former bounds.
    /// ```ignore
    ///
    ///
    ///   Case A:
    ///                       x1                         x1+len1
    ///   |--------------------+----------------------------+
    ///   |---------+--------+
    ///             x2    x2+len2
    ///   Result:                                                 Clipped completely.
    ///   |--------------------+
    ///                      x2=x1  len2=0
    ///
    ///   Case B:
    ///                       x1                         x1+len1
    ///   |--------------------+----------------------------+
    ///   |----------------+----------------------+
    ///                    x2                  x2+len2
    ///   Result:                 
    ///   |--------------------+------------------+               Clipped at start.
    ///                      x2=x1     |       x2+len2
    ///                                |
    ///                          len2=x2+len2-x1
    ///
    ///   Case C:
    ///                       x1                         x1+len1
    ///   |--------------------+----------------------------+
    ///   |---------------------------+------------+
    ///                               x2         x2+len2
    ///   Result:                 
    ///   |---------------------------+------------+             No clipping.
    ///                              x2          x2+len2
    ///
    ///   Case D:
    ///                       x1                         x1+len1
    ///   |--------------------+----------------------------+
    ///   |--------------------------------------------+----------------------+
    ///                                                x2                  x2+len2
    ///   Result:                 
    ///   |--------------------------------------------+----+     Clipped at end.            
    ///                                                x2 | x2+len2
    ///                                                   |
    ///                                              len2=x1+len1-x2
    ///   Case E:
    ///                       x1                         x1+len1
    ///   |--------------------+----------------------------+      x2   x2+len2
    ///   |--------------------------------------------------------+------+
    ///                                                            
    ///   Result:                                                 Clipped completely.
    ///   |-------------------------------------------------+
    ///                                                 x2=x1+len1  len2=0
    ///
    /// ```
    pub fn clip(
        &self,
        clipped: Self,
    )
        -> Option<Self>
    {
        let (x1, len1) = self.tup();
        let (x2, len2) = clipped.tup();

        return if x2 + len2 < x1 {
            None // A
        } else if x2 + len2 < x1 + len1 {
            if x2 < x1 {
                Some(Span((x1, x2 + len2 - x1))) // B
            } else {
                Some(Span((x2, len2))) // C
            }
        } else {
            if x2 < x1 + len1 {
                Some(Span((x2, x1 + len1 - x2))) // D
            } else {
                None // E
            }
        };
    }
}

/// Specify start and end points along a dimension in terms of a possible static pad and/or fixed
/// distance, using a flexible fill that occupies the remaining distance to the unspecified edge.
/// ```ignore
///                            x1 (near)  x2           x2+len2     x1+len1 (far)
///    PadFixedFill(Dim, Dim)  |<----pad---|----fixed-----|---fill---->|
///    FillFixedPad(Dim, Dim)  |<---fill----|---fixed---|-----pad----->|
///    PadFillPad(Dim, Dim)    |<---pad----|-----fill------|---pad---->|
///    PadFill(Dim)            |<------pad-------|---------fill------->|
///    FillPad(Dim)            |<------------fill------------|---pad-->|
///    Fill                    |<--------------fill------------------->|
///
/// ```
#[derive(Clone, Debug)]
pub enum FlexDim {
    PadFixedFill(Dim, Dim),
    FillFixedPad(Dim, Dim),
    PadFillPad(Dim, Dim),
    PadFill(Dim),
    FillPad(Dim),
    Fill,
}

impl FlexDim {

    pub fn end(&self, outer: Dim) -> Dim {
        match *self {
            Self::PadFixedFill(pad, fixed)  => pad + fixed,
            Self::FillFixedPad(_fixed, pad) => outer - pad,
            Self::PadFillPad(_pad1, pad2)   => outer - pad2,
            Self::PadFill(pad)              => pad,
            Self::FillPad(pad)              => outer - pad,
            Self::Fill                      => outer,
        }
    }

    pub fn relative_to(
        &self,
        outer: Span,
    )
        -> Span // (x2, len2)
    {
        let (x1, len1) = outer.tup();
        Span(match self {
            Self::PadFixedFill(pad, len2) => {
                let pad = *pad;
                let len2 = *len2;
                if (pad > len1) || (pad + len2 > len1) {
                    (x1 + len1, Dim(0)) // Inner pushed right/bottom to zero length.
                } else {
                    (x1 + pad, len2)
                }
            }
            Self::FillFixedPad(len2, pad) => {
                let pad = *pad;
                let len2 = *len2;
                if (pad > len1) || (pad + len2 > len1) {
                    (x1, Dim(0)) // Inner pushed left/top to zero length.
                } else {
                    (x1 + len1 - pad - len2, len2)
                }
            }
            Self::PadFillPad(pad1, pad2) => {
                let pad1 = *pad1;
                let pad2 = *pad2;
                if pad1 + pad2 > len1 {
                    (x1 + (len1 / Dim(2)), Dim(0)) // Inner pushed centrally to zero length.
                } else {
                    (x1 + pad1, len1 - pad1 - pad2)
                }
            }
            Self::PadFill(pad) => {
                let pad = *pad;
                if pad > len1 {
                    (x1 + len1, Dim(0)) // Inner pushed right/bottom to zero length.
                } else {
                    (x1 + pad, len1 - pad)
                }
            }
            Self::FillPad(pad) => {
                let pad = *pad;
                if pad > len1 {
                    (x1, Dim(0)) // Inner pushed left/top to zero length.
                } else {
                    (x1 + len1 - pad, len1 - pad)
                }
            }
            Self::Fill => (x1, len1),
        })
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Offset {
    pub x: isize,
    pub y: isize,
}

impl Offset {
    pub fn new((x, y): (isize, isize)) -> Self { Self { x, y } }
    pub fn tup(&self) -> (isize, isize) { (self.x, self.y) }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Coord {
    pub x: Dim,
    pub y: Dim,
}

impl From<(Dim, Dim)> for Coord {
    fn from((x, y): (Dim, Dim)) -> Self { Self { x, y } }
}

impl From<(usize, usize)> for Coord {
    fn from((x, y): (usize, usize)) -> Self { Self { x: Dim(x), y: Dim(y) } }
}

impl std::ops::Add for Coord {
    type Output = Self;

    fn add(self, other: Self) -> Self {
        Self {
            x: self.x + other.x,
            y: self.y + other.y,
        }
    }
}

impl std::ops::Sub for Coord {
    type Output = Self;

    fn sub(self, other: Self) -> Self {
        Self {
            x: self.x - other.x,
            y: self.y - other.y,
        }
    }
}

impl std::ops::AddAssign for Coord {
    fn add_assign(&mut self, other: Self) {
        self.x = self.x + *other.x;
        self.y = self.y + *other.y;
    }
}

impl std::ops::SubAssign for Coord {
    fn sub_assign(&mut self, other: Self) {
        self.x = self.x - *other.x;
        self.y = self.y - *other.y;
    }
}

impl Coord {
    
    pub fn new<
        X: Into<Dim>,
        Y: Into<Dim>,
    >(
        (x, y): (X, Y),
    )
        -> Self
    {
        Self {
            x: x.into(),
            y: y.into(),
        }
    }

    pub fn zero() -> Self { Self { x: Dim(0), y: Dim(0) } }
    pub fn tup(&self) -> (Dim, Dim) { (self.x, self.y) }

    pub fn inc_x<D: Into<Dim>>(&mut self, by: D) { self.x = self.x + by.into(); }
    pub fn dec_x<D: Into<Dim>>(&mut self, by: D) { self.x = self.x - by.into(); }
    pub fn inc_y<D: Into<Dim>>(&mut self, by: D) { self.y = self.y + by.into(); }
    pub fn dec_y<D: Into<Dim>>(&mut self, by: D) { self.y = self.y - by.into(); }
}
