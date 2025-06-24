use crate::core::Duration;
use oxedyne_fe2o3_core::prelude::*;

/// Duration for CalClock operations.
#[derive(Clone, Debug, PartialEq)]
pub struct CalClockDuration {
	days: i32,
	nanos: i64,
}

impl CalClockDuration {
	/// Creates a new CalClockDuration.
	pub fn new(days: i32, nanos: i64) -> Self {
		Self { days, nanos }
	}
	
	/// Creates from days only.
	pub fn from_days(days: i32) -> Self {
		Self { days, nanos: 0 }
	}
	
	/// Creates from nanoseconds only.
	pub fn from_nanos(nanos: i64) -> Self {
		Self { days: 0, nanos }
	}
	
	/// Returns the days component.
	pub fn days(&self) -> i32 {
		self.days
	}
	
	/// Returns the nanoseconds component.
	pub fn nanoseconds(&self) -> i64 {
		self.nanos
	}
	
	/// Creates a CalClockDuration from milliseconds.
	///
	/// # Arguments
	///
	/// * `millis` - The number of milliseconds
	///
	/// # Returns
	///
	/// Returns `Ok(CalClockDuration)` if the millisecond value is valid.
	pub fn from_millis(millis: i64) -> Outcome<Self> {
		let nanos = millis * 1_000_000;
		Ok(Self::from_nanos(nanos))
	}
	
	/// Creates a CalClockDuration from seconds.
	///
	/// # Arguments
	///
	/// * `seconds` - The number of seconds
	pub fn from_seconds(seconds: i64) -> Self {
		let nanos = seconds * 1_000_000_000;
		Self::from_nanos(nanos)
	}
	
	/// Creates a CalClockDuration from minutes.
	///
	/// # Arguments
	///
	/// * `minutes` - The number of minutes
	pub fn from_minutes(minutes: i64) -> Self {
		let nanos = minutes * 60 * 1_000_000_000;
		Self::from_nanos(nanos)
	}
	
	/// Creates a CalClockDuration from hours.
	///
	/// # Arguments
	///
	/// * `hours` - The number of hours
	pub fn from_hours(hours: i64) -> Self {
		let nanos = hours * 60 * 60 * 1_000_000_000;
		Self::from_nanos(nanos)
	}
	
	/// Returns the total duration in days (including fractional days).
	///
	/// This converts the entire duration to days, taking both the days
	/// component and the nanoseconds component into account.
	///
	/// # Returns
	///
	/// Returns the total number of days as an i64.
	pub fn total_days(&self) -> i64 {
		const NANOS_PER_DAY: i64 = 24 * 60 * 60 * 1_000_000_000;
		
		let day_nanos = self.days as i64 * NANOS_PER_DAY;
		let total_nanos = day_nanos + self.nanos;
		
		total_nanos / NANOS_PER_DAY
	}
	
	/// Returns the total duration in hours.
	pub fn to_hours(&self) -> Outcome<i64> {
		const NANOS_PER_HOUR: i64 = 60 * 60 * 1_000_000_000;
		let total_nanos = res!(self.to_nanos());
		Ok(total_nanos / NANOS_PER_HOUR)
	}
	
	/// Returns the total duration in minutes.
	pub fn to_minutes(&self) -> Outcome<i64> {
		const NANOS_PER_MINUTE: i64 = 60 * 1_000_000_000;
		let total_nanos = res!(self.to_nanos());
		Ok(total_nanos / NANOS_PER_MINUTE)
	}
	
	/// Adds another CalClockDuration to this one.
	pub fn add(&self, other: &Self) -> Outcome<Self> {
		Ok(Self {
			days: self.days + other.days,
			nanos: self.nanos + other.nanos,
		})
	}
	
	/// Subtracts another CalClockDuration from this one.
	pub fn subtract(&self, other: &Self) -> Outcome<Self> {
		Ok(Self {
			days: self.days - other.days,
			nanos: self.nanos - other.nanos,
		})
	}
	
	/// Extracts the time component (nanoseconds) as a ClockDuration.
	///
	/// This is used by CalClock to separate the date and time components
	/// of a duration for arithmetic operations.
	pub fn time_component(&self) -> crate::clock::ClockDuration {
		crate::clock::ClockDuration::from_nanos(self.nanos)
	}
}

impl Duration for CalClockDuration {
	fn to_nanos(&self) -> Outcome<i64> {
		let day_nanos = self.days as i64 * 24 * 60 * 60 * 1_000_000_000;
		Ok(day_nanos + self.nanos)
	}
	
	fn to_seconds(&self) -> Outcome<i64> {
		let total_nanos = res!(self.to_nanos());
		Ok(total_nanos / 1_000_000_000)
	}
	
	fn to_days(&self) -> Outcome<i32> {
		Ok(self.days)
	}
	
	fn is_negative(&self) -> bool {
		self.days < 0 || (self.days == 0 && self.nanos < 0)
	}
}