use crate::dim::{
    Coord,
    Dim,
    FlexDim,
    Offset,
    Span,
};

use oxedize_fe2o3_core::prelude::*;


#[derive(Clone, Copy, Debug)]
pub enum RectSide {
    Top,
    Right,
    Bottom,
    Left,
}

/// Note that this iterator does not implement the `std::iter::Iterator` in order to avoid the
/// `Option` result.
#[derive(Clone, Debug)]
pub struct RectSideIter {
    state: RectSide,
}

impl RectSideIter {

    pub fn new(state: RectSide) -> Self {
        Self {
            state,
        }
    }

    pub fn next(&mut self) -> RectSide {
        let current = self.state;
        self.state = match current {
            RectSide::Top => RectSide::Right,
            RectSide::Right => RectSide::Bottom,
            RectSide::Bottom => RectSide::Left,
            RectSide::Left => RectSide::Top,
        };
        current
    }
}

#[derive(Clone, Copy, Debug)]
pub enum RelativePosition {
    TopLeft,
    TopMiddle,
    TopRight,
    RightMiddle,
    BottomRight,
    BottomMiddle,
    BottomLeft,
    LeftMiddle,
    Centre,
}

impl RelativePosition {

    /// Position the inner rectangle with respect to the outer, with a possible offset, and
    /// clip the result.
    pub fn relative_to(
        &self,
        outer: &AbsRect,
        inner: &AbsSize,
        delta: Option<&Offset>,
    )
        -> Option<AbsRect>
    {
        let (x1, y1, w1, h1) = outer.tup();
        let (w2, h2) = inner.tup();
        // Perform initial normalisation.
        let w2 = if w2 > w1 { w1 } else { w2 };
        let h2 = if h2 > h1 { h1 } else { h2 };
        let (x2, y2) = match self {
            Self::TopLeft         => (x1,                           y1),
            Self::TopMiddle       => (x1 + w1/Dim(2) - w2/Dim(2),   y1),
            Self::TopRight        => (x1 + w1 - w2,                 y1),
            Self::RightMiddle     => (x1 + w1 - w2,                 y1 + h1/Dim(2) - h2/Dim(2)),
            Self::BottomRight     => (x1 + w1 - w2,                 y1 + h1 - h2),
            Self::BottomMiddle    => (x1 + w1/Dim(2) - w2/Dim(2),   y1 + h1 - h2),
            Self::BottomLeft      => (x1,                           y1 + h1 - h2),
            Self::LeftMiddle      => (x1,                           y1 + h1/Dim(2) - h2/Dim(2)),
            Self::Centre          => (x1 + w1/Dim(2) - w2/Dim(2),   y1 + h1/Dim(2) - h2/Dim(2)),
        };
        let (dx, dy) = match delta {
            Some(offset) => offset.tup(),
            None => (0, 0),
        };
        let (x2, y2) = (
            x2.saturating_add_signed(dx),
            y2.saturating_add_signed(dy),
        );
        // Perform final normalisation.
        outer.clip(AbsRect::new(Coord::from((x2, y2)), AbsSize::from((w2, h2))))
    }
}

#[derive(Clone, Debug)]
pub enum Position {
    Float(Coord),
    Relative(RelativePosition),
    OffsetFrom(RelativePosition, Offset),
}

impl Default for Position {
    fn default() -> Self {
        Self::Float(Coord::default())
    }
}

#[derive(Clone, Debug)]
pub struct RelSize {
    pub x: FlexDim,
    pub y: FlexDim,
}

impl RelSize {
    pub fn new((x, y): (FlexDim, FlexDim)) -> Self { Self { x, y } }
    pub fn tup(&self) -> (FlexDim, FlexDim) { (self.x.clone(), self.y.clone()) }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct AbsSize {
    pub x: Dim,
    pub y: Dim,
}

impl From<(Dim, Dim)> for AbsSize {
    fn from((x, y): (Dim, Dim)) -> Self { Self { x, y } }
}

impl From<(usize, usize)> for AbsSize {
    fn from((x, y): (usize, usize)) -> Self {
        Self { x: Dim::new(x), y: Dim::new(y) }
    }
}

impl From<(u8, u8)> for AbsSize {
    fn from((x, y): (u8, u8)) -> Self {
        Self { x: x.into(), y: y.into() }
    }
}

impl From<(u16, u16)> for AbsSize {
    fn from((x, y): (u16, u16)) -> Self {
        Self { x: x.into(), y: y.into() }
    }
}

impl std::ops::Add<AbsSize> for AbsSize {
    type Output = AbsSize;

    fn add(self, other: AbsSize) -> Self::Output {
        Self {
            x: self.x + other.x,
            y: self.y + other.y,
        }
    }
}

impl std::ops::Add<(Dim, Dim)> for AbsSize {
    type Output = AbsSize;

    fn add(self, (dx, dy): (Dim, Dim)) -> Self::Output {
        Self {
            x: self.x + dx,
            y: self.y + dy,
        }
    }
}

impl std::ops::Sub<AbsSize> for AbsSize {
    type Output = AbsSize;

    fn sub(self, other: AbsSize) -> Self::Output {
        Self {
            x: self.x - other.x,
            y: self.y - other.y,
        }
    }
}

impl std::ops::Sub<(Dim, Dim)> for AbsSize {
    type Output = AbsSize;

    fn sub(self, (dx, dy): (Dim, Dim)) -> Self::Output {
        Self {
            x: self.x - dx,
            y: self.y - dy,
        }
    }
}

impl std::ops::AddAssign for AbsSize {
    fn add_assign(&mut self, other: Self) {
        *self = *self + other;
    }
}

impl std::ops::SubAssign for AbsSize {
    fn sub_assign(&mut self, other: Self) {
        *self = *self - other;
    }
}

impl AbsSize {

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

    /// Return the bottom right coordinate using zero-based indexing.
    pub fn bot_right(&self) -> Coord {
        Coord::new((
            if self.x == Dim(0) {
                Dim(0)
            } else {
                self.x - 1
            },
            if self.y == Dim(0) {
                Dim(0)
            } else {
                self.y - 1
            },
        ))
    }
}

/// A rectangle that is capable of being both relatively and absolutely defined.
#[derive(Clone, Debug)]
pub enum RelRect {
    FixSize {
        top_left:   Position,
        size:       AbsSize,
    },
    RelSize(RelSize),
}

impl Default for RelRect {
    fn default() -> Self {
        Self::FixSize {
            top_left:   Position::default(),
            size:       AbsSize::default(),
        }
    }
}

impl From<RelSize> for RelRect {
    fn from(size: RelSize) -> Self { Self::RelSize(size) }
}

impl RelRect {

    pub fn new_fixed(top_left: Position, size: AbsSize) -> Self { Self::FixSize { top_left, size } }
    pub fn new_rel(size: RelSize) -> Self { Self::RelSize(size) }

    /// Position this inner rectangle relative to the outer rectangle.
    pub fn relative_to(
        &self,          // 2
        outer: AbsRect, // 1
    )
        -> Option<AbsRect>
    {
        match self {
            Self::FixSize { top_left, size } => {
                match top_left {
                    Position::Float(coord) =>
                        outer.clip(AbsRect::new(*coord, *size)),
                    Position::Relative(rpos) =>
                        rpos.relative_to(&outer, size, None),
                    Position::OffsetFrom(rpos, offset) =>
                        rpos.relative_to(&outer, size, Some(offset)),
                }
            }
            Self::RelSize(size) => {
                let (span_x, span_y) = outer.spans();
                let span_x = size.x.relative_to(span_x);
                let span_y = size.y.relative_to(span_y);
                Some(AbsRect::from((span_x, span_y)))
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct AbsRect {
    pub top_left:   Coord,
    pub size:       AbsSize,
}

impl From<(Dim, Dim, Dim, Dim)> for AbsRect {
    fn from((x, y, w, h): (Dim, Dim, Dim, Dim)) -> Self {
        Self {
            top_left:   Coord::new((x, y)),
            size:       AbsSize::new((w, h)),
        }
    }
}

impl From<(Span, Span)> for AbsRect {
    fn from((Span((x, w)), Span((y, h))): (Span, Span)) -> Self {
        Self {
            top_left:   Coord::new((x, y)),
            size:       AbsSize::new((w, h)),
        }
    }
}

impl From<AbsSize> for AbsRect {
    fn from(size: AbsSize) -> Self {
        Self {
            top_left: Coord::zero(),
            size,
        }
    }
}

impl AbsRect {

    pub fn new(top_left: Coord, size: AbsSize) -> Self { Self { top_left, size } }
    pub fn new_top_left(size: AbsSize) -> Self {
        Self {
            top_left: Coord::from((0, 0)),
            size,
        }
    }

    pub fn tup(&self) -> (Dim, Dim, Dim, Dim) {
        (self.top_left.x, self.top_left.y, self.size.x, self.size.y )
    }
    pub fn spans(&self) -> (Span, Span) {
        (Span((self.top_left.x, self.size.x)), Span((self.top_left.y, self.size.y)))
    }

    pub fn bottom(&self) -> Dim {
        self.top_left.y + self.size.y
    }

    pub fn top(&self) -> Dim {
        self.top_left.y
    }

    pub fn right(&self) -> Dim {
        self.top_left.x + self.size.x
    }

    pub fn left(&self) -> Dim {
        self.top_left.x
    }

    pub fn height(&self) -> Dim {
        self.size.y
    }

    pub fn width(&self) -> Dim {
        self.size.x
    }

    pub fn inc_x(&mut self, by: Dim) { self.top_left.inc_x(by); }
    pub fn dec_x(&mut self, by: Dim) { self.top_left.dec_x(by); }
    pub fn inc_y(&mut self, by: Dim) { self.top_left.inc_y(by); }
    pub fn dec_y(&mut self, by: Dim) { self.top_left.dec_y(by); }

    pub fn inc_w(&mut self, by: Dim) { self.size.inc_x(by); }
    pub fn dec_w(&mut self, by: Dim) { self.size.dec_x(by); }
    pub fn inc_h(&mut self, by: Dim) { self.size.inc_y(by); }
    pub fn dec_h(&mut self, by: Dim) { self.size.dec_y(by); }

    /// Clip the goven `AbsRect` with respect to this outer `AbsRect`.
    /// ```ignore
    ///
    ///   e.g.
    ///
    ///    +----------------------------------------+
    ///    |                                        |
    ///    |                                        | outer
    ///    |                                        |
    ///    |                                        |
    ///    |                                        |
    ///    |                        +------------------------+
    ///    |                        |               |        |
    ///    |                        |               |        |
    ///    |                        |               |        | other
    ///    |                        |               |        |
    ///    |                        |               |        |
    ///    +------------------------|---------------+        |
    ///                             |                        |
    ///                             |                        |
    ///                             +------------------------+
    ///    Result:
    ///    +----------------------------------------+
    ///    |                                        |
    ///    |                                        | outer
    ///    |                                        |
    ///    |                                        |
    ///    |                                        |
    ///    |                        +---------------+
    ///    |                        |               |
    ///    |                        |               |
    ///    |                 result |               |
    ///    |                        |               |
    ///    |                        |               |
    ///    +------------------------+---------------+
    /// ```
    pub fn clip(&self, other: Self) -> Option<Self> {
        let (other_span_x, other_span_y) = other.spans();
        let (span_x, span_y) = self.spans();
        let span_x_opt = span_x.clip(other_span_x);
        let span_y_opt = span_y.clip(other_span_y);
        if let Some(span_x) = span_x_opt {
            if let Some(span_y) = span_y_opt {
                return Some(Self::from((span_x, span_y)));
            }
        }
        None
    }
}

#[derive(Clone, Debug)]
pub enum RectView {
    Float(AbsRect),
    InitiallyRelative(RelRect),
    AlwaysRelative(RelRect),
}

impl Default for RectView {
    fn default() -> Self {
        Self::Float(AbsRect::default())
    }
}

impl RectView {
    
    /// Position and clip this `RectView` with respect to the given `AbsRect`.
    pub fn relative_to(
        &self,
        outer: AbsRect,
    )
        -> Option<AbsRect>
    {
        match self {
            Self::Float(abs_rect) => outer.clip(*abs_rect),
            Self::InitiallyRelative(rel_rect) | Self::AlwaysRelative(rel_rect) => {
                rel_rect.relative_to(outer)
            }
        }
    }

    /// Fix the size of the `RectView`.  If it was initially relative, it will be changed to a
    /// floating variant.
    pub fn set_size(
        &mut self,
        new_size:   AbsSize,
        outer:      AbsRect,
    ) {
        match self {
            Self::Float(AbsRect { size, .. }) => {
                *size = new_size;
            }
            Self::InitiallyRelative(rel_rect) => {
                let abs_rect_opt = rel_rect.relative_to(outer);
                if let Some(mut abs_rect) = abs_rect_opt {
                    abs_rect.size = new_size;
                    *self = Self::Float(abs_rect);
                }
            }
            _ => {}
        }
    }
}
