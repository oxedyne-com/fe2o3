use oxedyne_fe2o3_core::prelude::*;

use std::{
    fmt::{
        self,
        Debug,
        Display,
    },
};

// Zero-based index of a storage zone. Indexes zones from 0 internally, but
// displays from 1 for human consumption.
new_type!(ZoneInd, usize, Clone, Copy, Default, Eq, Hash, Ord, PartialEq, PartialOrd);

impl Display for ZoneInd {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self.0 + 1)
    }
}
    
impl Debug for ZoneInd {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Z{}", self.0 + 1)
    }
}
    
impl ZoneInd {
    /// Creates a zone index from any value convertible to `usize`.
    pub fn new<I: Into<usize>>(i: I) -> Self {
        Self(i.into())
    }
}

// Zero-based index of a bot within its pool of worker bots. Displays from 1
// for human consumption.
new_type!(BotPoolInd, usize, Clone, Copy, Default, Eq, Hash, Ord, PartialEq, PartialOrd);

impl Display for BotPoolInd {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self.0 + 1)
    }
}
    
impl Debug for BotPoolInd {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "B{}", self.0 + 1)
    }
}
    
impl BotPoolInd {
    /// Creates a bot-pool index from any value convertible to `usize`.
    pub fn new<I: Into<usize>>(i: I) -> Self {
        Self(i.into())
    }
}

/// The full location of a worker bot: its pool position within its zone.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct WorkerInd {
    /// Position within the zone's bot pool.
    pub pool: BotPoolInd,
    /// Zone the bot belongs to.
    pub zone: ZoneInd,
}

impl Display for WorkerInd {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}{:?}", self.zone, self.pool)
    }
}
    
impl WorkerInd {
    
    /// Creates a worker index from a zone and a pool position.
    pub fn new(zone: ZoneInd, pool: BotPoolInd) -> Self {
        Self {
            zone,
            pool,
        }
    }

    /// Returns the zone index.
    pub fn zind(&self)  -> &ZoneInd     { &self.zone }
    /// Returns the bot-pool index.
    pub fn bpind(&self) -> &BotPoolInd  { &self.pool }
    /// Returns the zone index as a `usize`.
    pub fn z(&self)     -> usize        { *self.zone }
    /// Returns the bot-pool index as a `usize`.
    pub fn b(&self)     -> usize        { *self.pool }

}

