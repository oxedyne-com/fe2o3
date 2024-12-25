use oxedize_fe2o3_core::prelude::*;
use oxedize_fe2o3_geom::dim::Coord;


#[derive(Clone, Debug)]
pub enum PositionCursor {
    UserControlled,
    LatestLine(bool), // Show or hide cursor?
}

impl Default for PositionCursor {
    fn default() -> Self {
        Self::UserControlled
    }
}

#[derive(Clone, Debug, Default)]
pub struct Navigator {
    pub cursor:         Coord,
}

impl Navigator {
    pub fn new(
        cursor:         Option<Coord>,
    )
        -> Self
    {
        Self {
            cursor: match cursor {
                Some(coord) => coord,
                None => Coord::zero(),
            },
        }
    }
}
