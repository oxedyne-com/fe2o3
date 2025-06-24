use oxedyne_fe2o3_core::prelude::*;

use std::fmt::Debug;

/// Trait for types representing a duration of time.
pub trait Duration: Debug + Clone {
    fn to_nanos(&self) -> Outcome<i64>;
    fn to_seconds(&self) -> Outcome<i64>;
    fn to_days(&self) -> Outcome<i32>;
    fn is_negative(&self) -> bool;
}

/// Abstract base for duration types.
#[derive(Clone, Debug, PartialEq)]
pub struct AbstractDuration;

impl AbstractDuration {
    pub fn new() -> Self {
        Self
    }
}