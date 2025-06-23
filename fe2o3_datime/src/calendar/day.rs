use oxedize_fe2o3_core::prelude::*;

/// Represents a calendar day value.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, PartialOrd, Ord)]
pub struct CalendarDay {
    value: i32,
}

impl CalendarDay {
    pub fn new(day: i32) -> Self {
        Self { value: day }
    }
    
    pub fn of(&self) -> i32 {
        self.value
    }
}