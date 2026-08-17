//! [Written entirely with AI](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use oxedyne_fe2o3_core::prelude::*;

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