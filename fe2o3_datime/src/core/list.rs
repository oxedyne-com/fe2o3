use crate::core::Time;

use oxedize_fe2o3_core::prelude::*;

use std::{
    fmt::Debug,
    vec::Vec,
};

/// A list of time values.
#[derive(Clone, Debug)]
pub struct TimeList<T: Time> {
    times: Vec<T>,
}

impl<T: Time> TimeList<T> {
    pub fn new() -> Self {
        Self {
            times: Vec::new(),
        }
    }
    
    pub fn add(&mut self, time: T) {
        self.times.push(time);
    }
    
    pub fn len(&self) -> usize {
        self.times.len()
    }
    
    pub fn is_empty(&self) -> bool {
        self.times.is_empty()
    }
    
    pub fn get(&self, index: usize) -> Option<&T> {
        self.times.get(index)
    }
    
    pub fn first(&self) -> Option<&T> {
        self.times.first()
    }
    
    pub fn last(&self) -> Option<&T> {
        self.times.last()
    }
    
    pub fn iter(&self) -> std::slice::Iter<T> {
        self.times.iter()
    }
    
    pub fn remove(&mut self, start: usize, end: usize) -> Outcome<()> {
        if end < start {
            return Err(err!(
                "Invalid range: end {} < start {}",
                end, start;
                Invalid, Input, Range));
        }
        
        if end >= self.times.len() {
            return Err(err!(
                "End index {} out of bounds (len: {})",
                end, self.times.len();
                Invalid, Input, Range));
        }
        
        self.times.drain(start..=end);
        Ok(())
    }
    
    pub fn size(&self) -> usize {
        self.len()
    }
}