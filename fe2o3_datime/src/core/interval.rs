use crate::core::{
    Duration,
    Time,
};

use oxedize_fe2o3_core::prelude::*;

use std::{
    fmt::Debug,
    vec::Vec,
};

/// Trait for types representing an interval between two times.
pub trait Interval<D: Duration>: Debug + Clone {
    /// Returns the duration of this interval.
    fn get_duration(&self) -> Outcome<D>;
    
    /// Returns true if this interval contains the given time.
    fn contains<T: Time>(&self, time: &T) -> bool;
    
    /// Returns true if this interval overlaps with another interval.
    fn overlaps(&self, other: &Self) -> bool;
}

/// Abstract base for interval types.
#[derive(Clone, Debug, PartialEq)]
pub struct AbstractInterval;

impl AbstractInterval {
    pub fn new() -> Self {
        Self
    }
}

/// A list of intervals.
#[derive(Clone, Debug)]
pub struct IntervalList<I> {
    intervals: Vec<I>,
}

impl<I> IntervalList<I> {
    pub fn new() -> Self {
        Self {
            intervals: Vec::new(),
        }
    }
    
    pub fn add(&mut self, interval: I) {
        self.intervals.push(interval);
    }
    
    pub fn len(&self) -> usize {
        self.intervals.len()
    }
    
    pub fn is_empty(&self) -> bool {
        self.intervals.is_empty()
    }
    
    pub fn get(&self, index: usize) -> Option<&I> {
        self.intervals.get(index)
    }
    
    pub fn iter(&self) -> std::slice::Iter<I> {
        self.intervals.iter()
    }
}