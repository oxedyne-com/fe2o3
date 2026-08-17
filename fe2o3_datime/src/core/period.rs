//! [Written entirely with AI](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use crate::time::CalClockZone;

use oxedyne_fe2o3_core::prelude::*;

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