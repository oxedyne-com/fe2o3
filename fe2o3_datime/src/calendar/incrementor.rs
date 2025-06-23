use oxedize_fe2o3_core::prelude::*;

/// Helper for incrementing days in calendar operations.
#[derive(Clone, Debug)]
pub struct DayIncrementor {
    day: u8,
}

impl DayIncrementor {
    pub fn new(day: u8) -> Self {
        Self { day }
    }
    
    pub fn day(&self) -> u8 {
        self.day
    }
}