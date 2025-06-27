use crate::{
	core::{Interval, Time},
	time::{CalClock, CalClockDuration},
};
use oxedyne_fe2o3_core::prelude::*;

/// Interval between two CalClock instances.
#[derive(Clone, Debug, PartialEq)]
pub struct CalClockInterval {
	start: CalClock,
	finish: CalClock,
}

impl CalClockInterval {
	/// Creates a new CalClockInterval.
	pub fn new(start: CalClock, finish: CalClock) -> Outcome<Self> {
		// Validate that start is before or equal to finish
		if start <= finish {
			Ok(Self { start, finish })
		} else {
			Err(err!("Start time {} must be before or equal to finish time {}", start, finish; Invalid, Input))
		}
	}
	
	/// Returns the start time.
	pub fn start(&self) -> &CalClock {
		&self.start
	}
	
	/// Returns the finish time.
	pub fn finish(&self) -> &CalClock {
		&self.finish
	}
	
	/// Returns the duration of this interval.
	pub fn duration(&self) -> Outcome<CalClockDuration> {
		self.start.duration_until(&self.finish)
	}
	
	/// Checks if this interval contains the specified time.
	pub fn contains_time(&self, time: &CalClock) -> bool {
		time >= &self.start && time <= &self.finish
	}
	
	/// Checks if this interval overlaps with another interval.
	pub fn overlaps_with(&self, other: &Self) -> bool {
		// Intervals overlap if one starts before the other ends
		self.start <= other.finish && other.start <= self.finish
	}
	
	/// Returns the intersection of this interval with another.
	pub fn intersection(&self, other: &Self) -> Option<Self> {
		if !self.overlaps_with(other) {
			return None;
		}
		
		let intersection_start = if self.start >= other.start {
			self.start.clone()
		} else {
			other.start.clone()
		};
		
		let intersection_finish = if self.finish <= other.finish {
			self.finish.clone()
		} else {
			other.finish.clone()
		};
		
		Self::new(intersection_start, intersection_finish).ok()
	}
	
	/// Returns the union of this interval with another if they overlap.
	pub fn union(&self, other: &Self) -> Option<Self> {
		if !self.overlaps_with(other) {
			return None;
		}
		
		let union_start = if self.start <= other.start {
			self.start.clone()
		} else {
			other.start.clone()
		};
		
		let union_finish = if self.finish >= other.finish {
			self.finish.clone()
		} else {
			other.finish.clone()
		};
		
		Self::new(union_start, union_finish).ok()
	}
	
	/// Checks if this interval is adjacent to another (touching but not overlapping).
	pub fn is_adjacent_to(&self, other: &Self) -> bool {
		self.finish == other.start || other.finish == self.start
	}
	
	/// Checks if this interval is entirely before another.
	pub fn is_before(&self, other: &Self) -> bool {
		self.finish < other.start
	}
	
	/// Checks if this interval is entirely after another.
	pub fn is_after(&self, other: &Self) -> bool {
		self.start > other.finish
	}
	
	/// Expands this interval by the specified duration on both ends.
	pub fn expand(&self, duration: &CalClockDuration) -> Outcome<Self> {
		let new_start = res!(self.start.subtract_duration(duration));
		let new_finish = res!(self.finish.add_duration(duration));
		Self::new(new_start, new_finish)
	}
	
	/// Contracts this interval by the specified duration from both ends.
	pub fn contract(&self, duration: &CalClockDuration) -> Outcome<Self> {
		let new_start = res!(self.start.add_duration(duration));
		let new_finish = res!(self.finish.subtract_duration(duration));
		
		if new_start <= new_finish {
			Self::new(new_start, new_finish)
		} else {
			Err(err!("Cannot contract interval by {:?} - would result in negative duration", duration; Invalid, Input))
		}
	}
	
	/// Shifts this interval by the specified duration.
	pub fn shift(&self, duration: &CalClockDuration) -> Outcome<Self> {
		let new_start = res!(self.start.add_duration(duration));
		let new_finish = res!(self.finish.add_duration(duration));
		Self::new(new_start, new_finish)
	}
	
	/// Splits this interval at the specified time.
	pub fn split_at(&self, split_time: &CalClock) -> Outcome<(Self, Self)> {
		if !self.contains_time(split_time) {
			return Err(err!("Split time {} is not within interval", split_time; Invalid, Input));
		}
		
		let first_interval = res!(Self::new(self.start.clone(), split_time.clone()));
		let second_interval = res!(Self::new(split_time.clone(), self.finish.clone()));
		
		Ok((first_interval, second_interval))
	}
	
	/// Returns the midpoint of this interval.
	pub fn midpoint(&self) -> Outcome<CalClock> {
		let duration = res!(self.duration());
		let half_duration = res!(duration.divide_by(2));
		self.start.add_duration(&half_duration)
	}
	
	/// Checks if this interval completely contains another interval.
	pub fn contains_interval(&self, other: &Self) -> bool {
		self.start <= other.start && other.finish <= self.finish
	}
	
	/// Returns a list of non-overlapping intervals from a collection.
	pub fn merge_overlapping(intervals: Vec<Self>) -> Vec<Self> {
		if intervals.is_empty() {
			return Vec::new();
		}
		
		let mut sorted_intervals = intervals;
		sorted_intervals.sort_by(|a, b| a.start.cmp(&b.start));
		
		let mut merged = Vec::new();
		let mut current = sorted_intervals[0].clone();
		
		for interval in sorted_intervals.into_iter().skip(1) {
			if current.overlaps_with(&interval) || current.is_adjacent_to(&interval) {
				// Merge with current
				if let Some(union) = current.union(&interval) {
					current = union;
				} else {
					// Handle adjacent case
					let new_finish = if current.finish >= interval.finish {
						current.finish.clone()
					} else {
						interval.finish.clone()
					};
					current = Self::new(current.start.clone(), new_finish).unwrap_or(current);
				}
			} else {
				// No overlap, add current to result and start new
				merged.push(current);
				current = interval;
			}
		}
		
		merged.push(current);
		merged
	}
}

impl Interval<CalClockDuration> for CalClockInterval {
	fn get_duration(&self) -> Outcome<CalClockDuration> {
		self.duration()
	}
	
	fn contains<T: Time>(&self, _time: &T) -> bool {
		// This would need a way to convert T to CalClock for proper comparison
		// For now, return false as we can't convert arbitrary Time types
		false
	}
	
	fn overlaps(&self, other: &Self) -> bool {
		self.overlaps_with(other)
	}
}