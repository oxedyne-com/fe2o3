use crate::time::CalClockZone;

use oxedize_fe2o3_core::prelude::*;

use std::fmt::Debug;

/// Represents a moment or period of time.
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

/// Represents a fundamental moment of time.
///
/// The fundamental moments of time are a CalClock, a ClockTime
/// defining a time of day, and a CalendarDate defining a day
/// of year.
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
    
    /// Provide sorting order for compareTo in children.
    ///
    /// The natural order is already defined by is_before and equals.
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