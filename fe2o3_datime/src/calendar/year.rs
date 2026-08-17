//! [Written entirely with AI](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use oxedyne_fe2o3_core::prelude::*;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, PartialOrd, Ord)]
pub struct CalendarYear {
    value: i32,
}

impl CalendarYear {
    pub fn new(year: i32) -> Self {
        Self { value: year }
    }
    
    pub fn of(&self) -> i32 {
        self.value
    }
}