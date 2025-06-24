use crate::time::CalClockZone;

use oxedyne_fe2o3_core::prelude::*;

/// Abstract base for period types (periods of time like a specific month or year).
#[derive(Clone, Debug, PartialEq)]
pub struct AbstractPeriod {
    zone: CalClockZone,
}

impl AbstractPeriod {
    pub fn new(zone: CalClockZone) -> Self {
        Self { zone }
    }
    
    pub fn new_default() -> Self {
        Self {
            zone: CalClockZone::default(),
        }
    }
    
    pub fn zone(&self) -> &CalClockZone {
        &self.zone
    }
}