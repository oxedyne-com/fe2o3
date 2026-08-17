//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use crate::time::CalClockZone;

use oxedyne_fe2o3_core::prelude::*;

use std::fmt::Debug;

pub trait Time: Debug + Clone + PartialEq {
    fn get_zone(&self) -> &CalClockZone;
    
    fn to_zone(&self, new_zone: CalClockZone) -> Outcome<Self>;
    
    fn format(&self, stencil: &str) -> String;
    
    fn is_recognised_format_char(&self, c: char) -> bool;
    
    fn is_before(&self, other: &Self) -> bool;
    
    fn is_after(&self, other: &Self) -> bool;
    
    fn or_earlier(&self, other: &Self) -> Self;
    
    fn or_later(&self, other: &Self) -> Self;
}

/// The fundamental moments are a CalClock, a ClockTime naming a time of day,
/// and a CalendarDate naming a day of the year.
#[derive(Clone, Debug, PartialEq)]
pub struct AbstractTime {
    zone:	CalClockZone,
}

impl AbstractTime {
    pub fn new(zone: CalClockZone) -> Self {
        Self { zone }
    }
    
    pub fn new_default() -> Self {
        Self {
            zone: CalClockZone::default(),
        }
    }
    
    pub fn compare_time<T: Time>(&self, this: &T, other: &T) -> Outcome<std::cmp::Ordering> {
        if this.is_before(other) {
            Ok(std::cmp::Ordering::Less)
        } else if this == other {
            Ok(std::cmp::Ordering::Equal)
        } else {
            Ok(std::cmp::Ordering::Greater)
        }
    }
}