//! [Written entirely with AI](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use crate::core::{
    Duration,
    Time,
};

use oxedyne_fe2o3_core::prelude::*;

use std::{
    fmt::Debug,
    vec::Vec,
};

pub trait Interval<D: Duration>: Debug + Clone {
    fn get_duration(&self) -> Outcome<D>;
    
    fn contains<T: Time>(&self, time: &T) -> bool;
    
    fn overlaps(&self, other: &Self) -> bool;
}

#[derive(Clone, Debug, PartialEq)]
pub struct AbstractInterval;

impl AbstractInterval {
    pub fn new() -> Self {
        Self
    }
}

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