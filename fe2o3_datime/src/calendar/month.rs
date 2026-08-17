//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use oxedyne_fe2o3_core::prelude::*;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, PartialOrd, Ord)]
pub struct CalendarMonth {
    value: i32,
}

impl CalendarMonth {
    pub fn new(month: i32) -> Self {
        Self { value: month }
    }
    
    pub fn of(&self) -> i32 {
        self.value
    }
}