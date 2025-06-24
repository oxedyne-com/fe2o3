use oxedyne_fe2o3_core::prelude::*;

use std::{
    fmt::{
        self,
        Debug,
        Display,
    },
};

// The `ZoneInd` indexes zones from 0, but displays from 1 for human consumption.
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
    pub fn new<I: Into<usize>>(i: I) -> Self {
        Self(i.into())
    }
}

// The index for a bot in a pool of worker bots.
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
    pub fn new<I: Into<usize>>(i: I) -> Self {
        Self(i.into())
    }
}

/// The index for a bot in a pool of worker bots.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct WorkerInd {
    pub pool: BotPoolInd,
    pub zone: ZoneInd,
}

impl Display for WorkerInd {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}{:?}", self.zone, self.pool)
    }
}
    
impl WorkerInd {
    
    pub fn new(zone: ZoneInd, pool: BotPoolInd) -> Self {
        Self {
            zone,
            pool,
        }
    }

    pub fn zind(&self)  -> &ZoneInd     { &self.zone }
    pub fn bpind(&self) -> &BotPoolInd  { &self.pool }
    pub fn z(&self)     -> usize        { *self.zone }
    pub fn b(&self)     -> usize        { *self.pool }

}

