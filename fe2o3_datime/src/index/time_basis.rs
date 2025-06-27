use oxedyne_fe2o3_core::prelude::*;
use crate::{
	time::{CalClock, CalClockZone, CalClockDuration},
	clock::{ClockSecond, ClockMilliSecond, ClockNanoSecond, PerSecondRated},
	index::time_integer::{TimeInteger, TimeLong},
};

/// Represents different time coordinate systems with their epoch and unit.
/// 
/// A TimeIndexBasis defines a reference point (epoch) and a time unit,
/// allowing conversion between different time representations.
pub struct TimeIndexBasis {
	epoch: CalClock,
	unit: Box<dyn PerSecondRated>,
}

impl TimeIndexBasis {
	/// Creates a new TimeIndexBasis with the specified epoch and unit.
	pub fn new(epoch: CalClock, unit: Box<dyn PerSecondRated>) -> Self {
		Self { epoch, unit }
	}
	
	/// Returns the epoch (reference time) for this basis.
	pub fn epoch(&self) -> &CalClock {
		&self.epoch
	}
	
	/// Returns the time zone of the epoch.
	pub fn zone(&self) -> &CalClockZone {
		self.epoch.zone()
	}
	
	/// Returns the time unit for this basis.
	pub fn unit(&self) -> &dyn PerSecondRated {
		self.unit.as_ref()
	}
	
	/// Converts a TimeInteger from this basis to another basis.
	pub fn convert<F: TimeInteger, T: TimeInteger>(
		&self,
		time_integer: F,
		to_basis: &TimeIndexBasis,
	) -> Outcome<T> {
		// Calculate the difference between epochs in seconds
		let epoch_diff = res!(self.epoch.duration_until(&to_basis.epoch));
		let epoch_diff_seconds = epoch_diff.total_seconds() as f64;
		
		// Get the scale factors for both units
		let from_scale = self.unit.per_second();
		let to_scale = to_basis.unit.per_second();
		
		// Convert the time integer to seconds since this epoch
		let time_in_seconds = time_integer.long_value() as f64 / from_scale as f64;
		
		// Adjust for epoch difference
		let adjusted_seconds = time_in_seconds + epoch_diff_seconds;
		
		// Convert to target basis units
		let target_value = adjusted_seconds * to_scale as f64;
		
		// Create the target TimeInteger
		let long_value = target_value.round() as i64;
		T::from_string(&long_value.to_string())
	}
	
	/// Calculates the zone offset duration for a given timezone at a specific Unix time.
	pub fn get_zone_offset_duration(&self, zone: &CalClockZone, unix_time: i64) -> Outcome<CalClockDuration> {
		// Convert Unix timestamp to CalClock
		let time = res!(CalClock::from_unix_timestamp_seconds(unix_time, zone.clone()));
		
		// Calculate offset from UTC
		let utc_time = res!(time.to_utc_zone());
		time.duration_until(&utc_time)
	}
}

impl PartialEq for TimeIndexBasis {
	fn eq(&self, other: &Self) -> bool {
		self.epoch == other.epoch && self.unit.per_second() == other.unit.per_second()
	}
}

impl std::fmt::Debug for TimeIndexBasis {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("TimeIndexBasis")
			.field("epoch", &self.epoch)
			.field("unit_per_second", &self.unit.per_second())
			.finish()
	}
}

/// Unix time basis (epoch: 1970-01-01 00:00:00 UTC, unit: seconds).
pub struct UnixTime;

impl UnixTime {
	/// Returns the Unix time basis.
	pub fn basis() -> Outcome<TimeIndexBasis> {
		let epoch = res!(CalClock::new(1970, 1, 1, 0, 0, 0, 0, CalClockZone::utc()));
		let unit = Box::new(res!(ClockSecond::new(1)));
		Ok(TimeIndexBasis::new(epoch, unit))
	}
	
	/// Creates a UnixTime TimeIndex from a Unix timestamp.
	pub fn from_timestamp(timestamp: i64) -> TimeIndex<TimeLong> {
		TimeIndex::new(TimeLong::new(timestamp))
	}
}

/// Java time basis (epoch: 1970-01-01 00:00:00 UTC, unit: milliseconds).
pub struct JavaTime;

impl JavaTime {
	/// Returns the Java time basis.
	pub fn basis() -> Outcome<TimeIndexBasis> {
		let epoch = res!(CalClock::new(1970, 1, 1, 0, 0, 0, 0, CalClockZone::utc()));
		let unit = Box::new(res!(ClockMilliSecond::new(1)));
		Ok(TimeIndexBasis::new(epoch, unit))
	}
	
	/// Creates a JavaTime TimeIndex from a Java timestamp.
	pub fn from_timestamp(timestamp: i64) -> TimeIndex<TimeLong> {
		TimeIndex::new(TimeLong::new(timestamp))
	}
}

/// Nanosecond time basis (epoch: 1970-01-01 00:00:00 UTC, unit: nanoseconds).
pub struct NanoTime;

impl NanoTime {
	/// Returns the nanosecond time basis.
	pub fn basis() -> Outcome<TimeIndexBasis> {
		let epoch = res!(CalClock::new(1970, 1, 1, 0, 0, 0, 0, CalClockZone::utc()));
		let unit = Box::new(res!(ClockNanoSecond::new(1)));
		Ok(TimeIndexBasis::new(epoch, unit))
	}
	
	/// Creates a NanoTime TimeIndex from a nanosecond timestamp.
	pub fn from_timestamp(timestamp: i64) -> TimeIndex<TimeLong> {
		TimeIndex::new(TimeLong::new(timestamp))
	}
}

/// Custom time basis with user-defined epoch and unit.
pub struct CustomTime {
	basis: TimeIndexBasis,
}

impl CustomTime {
	/// Creates a new custom time basis.
	pub fn new(epoch: CalClock, unit: Box<dyn PerSecondRated>) -> Self {
		Self {
			basis: TimeIndexBasis::new(epoch, unit),
		}
	}
	
	/// Returns the custom time basis.
	pub fn basis(&self) -> &TimeIndexBasis {
		&self.basis
	}
}

/// Generic time index wrapper that provides time semantics to any TimeInteger.
#[derive(Debug, Clone)]
pub struct TimeIndex<I: TimeInteger> {
	time: I,
	zone: Option<CalClockZone>,
}

impl<I: TimeInteger> TimeIndex<I> {
	/// Creates a new TimeIndex with the specified time value.
	pub fn new(time: I) -> Self {
		Self { time, zone: None }
	}
	
	/// Creates a new TimeIndex with the specified time value and zone.
	pub fn new_with_zone(time: I, zone: CalClockZone) -> Self {
		Self { 
			time, 
			zone: Some(zone),
		}
	}
	
	/// Returns the underlying time integer.
	pub fn time(&self) -> &I {
		&self.time
	}
	
	/// Returns the timezone, if any.
	pub fn zone(&self) -> Option<&CalClockZone> {
		self.zone.as_ref()
	}
	
	/// Factory method to create from an i64 value.
	pub fn make_from_long(value: i64) -> Outcome<Self> 
	where 
		I: From<i64>,
	{
		Ok(Self::new(I::from(value)))
	}
	
	/// Factory method to create from bytes.
	pub fn make_from_bytes(bytes: Vec<u8>) -> Outcome<Self> {
		let time = res!(I::from_bytes(&bytes));
		Ok(Self::new(time))
	}
	
	/// Factory method to create from string.
	pub fn make_from_string(s: &str) -> Outcome<Self> {
		let time = res!(I::from_string(s));
		Ok(Self::new(time))
	}
	
	/// Returns the start time (same as this time for instants).
	pub fn get_start(&self) -> Outcome<Self> {
		Ok(self.clone())
	}
	
	/// Returns the finish time (same as this time for instants).
	pub fn get_finish(&self) -> Outcome<Self> {
		Ok(self.clone())
	}
	
	/// Converts this TimeIndex to a different timezone.
	pub fn to_zone(&self, zone: CalClockZone) -> Self {
		Self {
			time: self.time.clone(),
			zone: Some(zone),
		}
	}
	
	/// Adds another TimeIndex to this one.
	pub fn plus(&self, other: &Self) -> Outcome<Self> {
		let result_time = res!(self.time.clone().add_to(other.time.clone()));
		Ok(Self::new(result_time))
	}
	
	/// Adds an i64 value to this TimeIndex.
	pub fn plus_long(&self, value: i64) -> Self {
		let result_time = self.time.clone().add_to_long(value);
		Self::new(result_time)
	}
	
	/// Subtracts another TimeIndex from this one, returning a duration.
	pub fn minus(&self, other: &Self) -> Outcome<TimeIndexDuration<I>> {
		let result_time = res!(self.time.clone().subtract_it(other.time.clone()));
		Ok(TimeIndexDuration::new(result_time))
	}
	
	/// Multiplies this TimeIndex by another.
	pub fn multiply_by(&self, other: &Self) -> Outcome<Self> {
		let result_time = res!(self.time.clone().multiply_by(other.time.clone()));
		Ok(Self::new(result_time))
	}
	
	/// Divides this TimeIndex by another.
	pub fn divide_by(&self, other: &Self) -> Outcome<Self> {
		let result_time = res!(self.time.clone().divide_by(other.time.clone()));
		Ok(Self::new(result_time))
	}
	
	/// Subtracts another TimeIndex from this one (arithmetic).
	pub fn subtract_it(&self, other: &Self) -> Outcome<Self> {
		let result_time = res!(self.time.clone().subtract_it(other.time.clone()));
		Ok(Self::new(result_time))
	}
	
	/// Adds another TimeIndex to this one (arithmetic).
	pub fn add_to(&self, other: &Self) -> Outcome<Self> {
		let result_time = res!(self.time.clone().add_to(other.time.clone()));
		Ok(Self::new(result_time))
	}
	
	/// Checks if this time is before another.
	pub fn is_before(&self, other: &Self) -> Outcome<bool> {
		Ok(self.time < other.time)
	}
	
	/// Checks if this time is after another.
	pub fn is_after(&self, other: &Self) -> Outcome<bool> {
		Ok(self.time > other.time)
	}
	
	/// Returns the earlier of this time and another.
	pub fn or_earlier(&self, other: &Self) -> Outcome<Self> {
		if self.time <= other.time {
			Ok(self.clone())
		} else {
			Ok(other.clone())
		}
	}
	
	/// Returns the later of this time and another.
	pub fn or_later(&self, other: &Self) -> Outcome<Self> {
		if self.time >= other.time {
			Ok(self.clone())
		} else {
			Ok(other.clone())
		}
	}
	
	/// Converts this TimeIndex to a CalClock using the specified basis.
	pub fn to_calclock(&self, basis: &TimeIndexBasis) -> Outcome<CalClock> {
		// Convert time integer to seconds since epoch
		let time_in_seconds = self.time.long_value() as f64 / basis.unit.per_second() as f64;
		
		// Add to epoch
		let duration = CalClockDuration::from_seconds(time_in_seconds as i64);
		basis.epoch.add_duration(&duration)
	}
	
	/// Creates a TimeIndex from a CalClock using the specified basis.
	pub fn from_calclock(calclock: &CalClock, basis: &TimeIndexBasis) -> Outcome<Self> {
		// Calculate duration since epoch
		let duration = res!(basis.epoch.duration_until(calclock));
		let seconds = duration.total_seconds() as f64;
		
		// Convert to basis units
		let time_value = seconds * basis.unit.per_second() as f64;
		let time_integer = res!(I::from_string(&(time_value.round() as i64).to_string()));
		
		Ok(Self::new_with_zone(time_integer, calclock.zone().clone()))
	}
}

impl<I: TimeInteger> PartialEq for TimeIndex<I> {
	fn eq(&self, other: &Self) -> bool {
		self.time == other.time
	}
}

impl<I: TimeInteger> Eq for TimeIndex<I> {}

impl<I: TimeInteger> PartialOrd for TimeIndex<I> {
	fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
		Some(self.cmp(other))
	}
}

impl<I: TimeInteger> Ord for TimeIndex<I> {
	fn cmp(&self, other: &Self) -> std::cmp::Ordering {
		self.time.cmp(&other.time)
	}
}

impl<I: TimeInteger> std::fmt::Display for TimeIndex<I> {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "TimeIndex({})", self.time.to_string_with_commas())
	}
}

/// Represents a duration between time indices.
#[derive(Debug, Clone)]
pub struct TimeIndexDuration<I: TimeInteger> {
	value: I,
}

impl<I: TimeInteger> TimeIndexDuration<I> {
	/// Creates a new TimeIndexDuration with the specified value.
	pub fn new(value: I) -> Self {
		Self { value }
	}
	
	/// Creates a duration from two time indices.
	pub fn from_indices(start: &TimeIndex<I>, finish: &TimeIndex<I>) -> Outcome<Self> {
		let duration = res!(finish.time.clone().subtract_it(start.time.clone()));
		Ok(Self::new(duration))
	}
	
	/// Returns the underlying duration value.
	pub fn value(&self) -> &I {
		&self.value
	}
	
	/// Adds another duration to this one.
	pub fn plus(&self, other: &Self) -> Outcome<Self> {
		let result = res!(self.value.clone().add_to(other.value.clone()));
		Ok(Self::new(result))
	}
	
	/// Subtracts another duration from this one.
	pub fn minus(&self, other: &Self) -> Outcome<Self> {
		let result = res!(self.value.clone().subtract_it(other.value.clone()));
		Ok(Self::new(result))
	}
	
	/// Multiplies this duration by a scale factor.
	pub fn multiply_by(&self, scale: i32) -> Self {
		let result = self.value.clone().multiply_by_long(scale as i64);
		Self::new(result)
	}
	
	/// Divides this duration by a divisor.
	pub fn divide_by(&self, divisor: i32) -> Outcome<Self> {
		if divisor == 0 {
			return Err(err!("Division by zero"; Invalid, Input));
		}
		let result = self.value.clone().divide_by_long(divisor as i64);
		Ok(Self::new(result))
	}
	
	/// Returns true if this duration is zero.
	pub fn is_zero(&self) -> bool {
		self.value.is_zero()
	}
	
	/// Returns true if this duration is positive.
	pub fn is_positive(&self) -> bool {
		self.value.is_positive()
	}
	
	/// Returns the negation of this duration.
	pub fn negate(&self) -> Self {
		Self::new(self.value.clone().negate())
	}
}

impl<I: TimeInteger> std::fmt::Display for TimeIndexDuration<I> {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "Duration({})", self.value.to_string_with_commas())
	}
}

/// Represents an interval between two time indices.
#[derive(Debug, Clone)]
pub struct TimeIndexInterval<I: TimeInteger> {
	start: TimeIndex<I>,
	finish: TimeIndex<I>,
	duration: TimeIndexDuration<I>,
}

impl<I: TimeInteger> TimeIndexInterval<I> {
	/// Creates a new TimeIndexInterval.
	pub fn new(start: TimeIndex<I>, finish: TimeIndex<I>) -> Outcome<Self> {
		let duration = res!(TimeIndexDuration::from_indices(&start, &finish));
		Ok(Self { start, finish, duration })
	}
	
	/// Returns the start time.
	pub fn start(&self) -> &TimeIndex<I> {
		&self.start
	}
	
	/// Returns the finish time.
	pub fn finish(&self) -> &TimeIndex<I> {
		&self.finish
	}
	
	/// Returns the duration.
	pub fn duration(&self) -> &TimeIndexDuration<I> {
		&self.duration
	}
	
	/// Checks if this interval contains the specified time.
	pub fn contains(&self, time: &TimeIndex<I>) -> bool {
		time.time >= self.start.time && time.time <= self.finish.time
	}
	
	/// Checks if this interval overlaps with another.
	pub fn overlaps(&self, other: &Self) -> bool {
		self.start.time <= other.finish.time && self.finish.time >= other.start.time
	}
}

impl<I: TimeInteger> std::fmt::Display for TimeIndexInterval<I> {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "Interval[{} to {}]", self.start, self.finish)
	}
}