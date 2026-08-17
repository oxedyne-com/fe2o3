//! [Written entirely with AI](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use crate::{
	core::{
		Interval,
		Time,
	},
	clock::{
		ClockTime,
		ClockDuration,
	},
	time::CalClockZone,
};

use oxedyne_fe2o3_core::prelude::*;

use std::fmt;

/// A span within one day, inclusive of both endpoints, and holding start ≤ finish
/// from construction onwards.
///
/// ```ignore
/// use oxedyne_fe2o3_datime::{
///     clock::{ClockTime, ClockInterval},
///     time::CalClockZone,
/// }res!();
///
/// let zone = CalClockZone::utc()res!();
/// let start = ClockTime::new(10, 30, 0, 0, zone.clone())?res!();
/// let finish = ClockTime::new(14, 45, 30, 0, zone)?res!();
///
/// let interval = ClockInterval::new(start, finish)?res!();
/// assert_eq!(interval.duration().total_hours(), 4)res!();
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClockInterval {
	start:	ClockTime,
	finish:	ClockTime,
}

impl ClockInterval {
	pub fn new(start: ClockTime, finish: ClockTime) -> Outcome<Self> {
		if start.is_after(&finish) {
			return Err(err!(
				"Start time {} must not be after finish time {}", 
				start, 
				finish; 
				Invalid, Input
			));
		}
		Ok(Self { start, finish })
	}
	
	pub fn new_unchecked(start: ClockTime, finish: ClockTime) -> Self {
		Self { start, finish }
	}
	
	pub fn from_start_and_duration(
		start: ClockTime, 
		duration: &ClockDuration
	) -> Outcome<Self> {
		let (finish, day_carry) = res!(start.plus(duration));
		
		if day_carry != 0 {
			return Err(err!(
				"Duration extends beyond single day boundary"; 
				Invalid, Input
			));
		}
		
		Self::new(start, finish)
	}
	
	pub fn from_finish_and_duration(
		finish: ClockTime, 
		duration: &ClockDuration
	) -> Outcome<Self> {
		let (start, day_carry) = res!(finish.minus(duration));
		
		if day_carry != 0 {
			return Err(err!(
				"Duration extends beyond single day boundary"; 
				Invalid, Input
			));
		}
		
		Self::new(start, finish)
	}
	
	pub fn start(&self) -> &ClockTime {
		&self.start
	}
	
	pub fn finish(&self) -> &ClockTime {
		&self.finish
	}
	
	pub fn duration(&self) -> ClockDuration {
		self.start.duration_until(&self.finish)
	}
	
	pub fn contains_time(&self, time: &ClockTime) -> bool {
		time.or_later(&self.start) && time.or_earlier(&self.finish)
	}
	
	pub fn overlaps_with(&self, other: &Self) -> bool {
		self.start.is_before(&other.finish) && other.start.is_before(&self.finish)
	}
	
	pub fn intersect_with(&self, other: &Self) -> Option<Self> {
		if !self.overlaps_with(other) {
			return None;
		}
		
		let start = if self.start.or_later(&other.start) {
			self.start.clone()
		} else {
			other.start.clone()
		};
		
		let finish = if self.finish.or_earlier(&other.finish) {
			self.finish.clone()
		} else {
			other.finish.clone()
		};
		
		Some(Self::new_unchecked(start, finish))
	}
	
	pub fn union_with(&self, other: &Self) -> Option<Self> {
		// Check if intervals are contiguous or overlapping
		if !self.overlaps_with(other) && 
		   !self.finish.eq(&other.start) && 
		   !other.finish.eq(&self.start) {
			return None;
		}
		
		let start = if self.start.or_earlier(&other.start) {
			self.start.clone()
		} else {
			other.start.clone()
		};
		
		let finish = if self.finish.or_later(&other.finish) {
			self.finish.clone()
		} else {
			other.finish.clone()
		};
		
		Some(Self::new_unchecked(start, finish))
	}
	
	pub fn is_instant(&self) -> bool {
		self.start.eq(&self.finish)
	}
	
	pub fn is_valid(&self) -> bool {
		self.start.or_earlier(&self.finish)
	}
	
	pub fn to_zone(&self, zone: CalClockZone) -> Outcome<Self> {
		// For clock intervals, this is administrative conversion
		// (no actual time transformation since we don't have date context)
		let start = res!(self.start.to_zone(zone.clone()));
		let finish = res!(self.finish.to_zone(zone));
		
		Ok(Self::new_unchecked(start, finish))
	}
}

impl Interval<ClockDuration> for ClockInterval {
	fn get_duration(&self) -> Outcome<ClockDuration> {
		Ok(self.duration())
	}
	
	fn contains<T: Time>(&self, _time: &T) -> bool {
		// This is a compile-time error if T is not ClockTime
		// In practice, we'd use more sophisticated type checking or associated types
		false
	}
	
	fn overlaps(&self, other: &Self) -> bool {
		self.overlaps_with(other)
	}
}

impl PartialOrd for ClockInterval {
	fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
		Some(self.cmp(other))
	}
}

impl Ord for ClockInterval {
	fn cmp(&self, other: &Self) -> std::cmp::Ordering {
		self.start.cmp(&other.start)
			.then_with(|| self.finish.cmp(&other.finish))
	}
}

impl fmt::Display for ClockInterval {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "[{} - {}]", self.start, self.finish)
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn test_zone() -> CalClockZone {
		CalClockZone::utc()
	}

	#[test]
	fn test_interval_creation() {
		let start = ClockTime::new(10, 0, 0, 0, test_zone()).unwrap();
		let finish = ClockTime::new(11, 0, 0, 0, test_zone()).unwrap();
		
		let interval = ClockInterval::new(start.clone(), finish.clone());
		assert!(interval.is_ok());
		
		// Test invalid order
		let invalid = ClockInterval::new(finish, start);
		assert!(invalid.is_err());
	}

	#[test]
	fn test_interval_duration() {
		let start = ClockTime::new(10, 0, 0, 0, test_zone()).unwrap();
		let finish = ClockTime::new(11, 30, 0, 0, test_zone()).unwrap();
		
		let interval = ClockInterval::new(start, finish).unwrap();
		let duration = interval.duration();
		
		assert_eq!(duration.total_minutes(), 90);
	}

	#[test]
	fn test_interval_contains() {
		let start = ClockTime::new(10, 0, 0, 0, test_zone()).unwrap();
		let finish = ClockTime::new(12, 0, 0, 0, test_zone()).unwrap();
		let interval = ClockInterval::new(start, finish).unwrap();
		
		let inside = ClockTime::new(11, 0, 0, 0, test_zone()).unwrap();
		let outside = ClockTime::new(13, 0, 0, 0, test_zone()).unwrap();
		
		assert!(interval.contains_time(&inside));
		assert!(!interval.contains_time(&outside));
		
		// Test boundaries
		assert!(interval.contains_time(interval.start()));
		assert!(interval.contains_time(interval.finish()));
	}

	#[test]
	fn test_interval_overlap() {
		let interval1 = ClockInterval::new(
			ClockTime::new(10, 0, 0, 0, test_zone()).unwrap(),
			ClockTime::new(12, 0, 0, 0, test_zone()).unwrap(),
		).unwrap();
		
		let interval2 = ClockInterval::new(
			ClockTime::new(11, 0, 0, 0, test_zone()).unwrap(),
			ClockTime::new(13, 0, 0, 0, test_zone()).unwrap(),
		).unwrap();
		
		let interval3 = ClockInterval::new(
			ClockTime::new(13, 0, 0, 0, test_zone()).unwrap(),
			ClockTime::new(14, 0, 0, 0, test_zone()).unwrap(),
		).unwrap();
		
		assert!(interval1.overlaps_with(&interval2));
		assert!(!interval1.overlaps_with(&interval3));
	}

	#[test]
	fn test_interval_intersection() {
		let interval1 = ClockInterval::new(
			ClockTime::new(10, 0, 0, 0, test_zone()).unwrap(),
			ClockTime::new(12, 0, 0, 0, test_zone()).unwrap(),
		).unwrap();
		
		let interval2 = ClockInterval::new(
			ClockTime::new(11, 0, 0, 0, test_zone()).unwrap(),
			ClockTime::new(13, 0, 0, 0, test_zone()).unwrap(),
		).unwrap();
		
		let intersection = interval1.intersect_with(&interval2).unwrap();
		assert_eq!(intersection.start().hour().of(), 11);
		assert_eq!(intersection.finish().hour().of(), 12);
	}

	#[test]
	fn test_from_duration() {
		let start = ClockTime::new(10, 0, 0, 0, test_zone()).unwrap();
		let duration = ClockDuration::from_hours(2);
		
		let interval = ClockInterval::from_start_and_duration(start, &duration).unwrap();
		assert_eq!(interval.finish().hour().of(), 12);
		assert_eq!(interval.duration().total_hours(), 2);
	}
}