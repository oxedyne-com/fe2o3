use oxedize_fe2o3_core::prelude::*;

use std::fmt;

/// Represents an hour of the day with range 0-24.
///
/// Hours are represented using the 24-hour clock system, where:
/// - 0 represents midnight (start of day)
/// - 12 represents noon
/// - 23 represents the last hour of day
/// - 24 represents end of day (equivalent to midnight of the following day)
///
/// This type provides validation, arithmetic operations, and conversion between
/// 12-hour and 24-hour formats whilst maintaining immutability.
///
/// # Examples
///
/// ```ignore
/// use oxedize_fe2o3_datime::clock::ClockHourres!();
///
/// let hour = ClockHour::new(14)?res!();
/// let (twelve_hour, is_pm) = hour.to_twelve_hour()res!();
/// assert_eq!(twelve_hour, 2)res!();
/// assert_eq!(is_pm, true)res!();
/// ```
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, PartialOrd, Ord)]
pub struct ClockHour {
	value: u8,
}

impl ClockHour {
	/// Maximum valid hour value.
	/// 
	/// The value 24 represents the conceptual end of day.
	pub const MAX_VALUE: u8 = 24;
	
	/// Minutes per hour.
	pub const MINUTES_PER_HOUR: u32 = 60;
	
	/// Seconds per hour.
	pub const SECONDS_PER_HOUR: u32 = 3_600;
	
	/// Milliseconds per hour.
	pub const MILLIS_PER_HOUR: u32 = 3_600_000;
	
	/// Microseconds per hour.
	pub const MICROS_PER_HOUR: u64 = 3_600_000_000;
	
	/// Nanoseconds per hour.
	pub const NANOS_PER_HOUR: u64 = 3_600_000_000_000;

	/// Creates a new ClockHour from the given hour value.
	///
	/// # Arguments
	///
	/// * `hour` - Hour value in 24-hour format (0-24)
	///
	/// # Returns
	///
	/// Returns `Ok(ClockHour)` if the hour is valid (0-24), otherwise returns
	/// an error describing the validation failure.
	pub fn new(hour: u8) -> Outcome<Self> {
		if hour > Self::MAX_VALUE {
			return Err(err!(
				"Hour {} is invalid, must be 0-{}", 
				hour, 
				Self::MAX_VALUE; 
				Invalid, Input
			));
		}
		Ok(Self { value: hour })
	}
	
	/// Creates a new ClockHour without validation.
	///
	/// This method is intended for internal use where the hour value is already
	/// known to be valid. Using this with invalid values will result in undefined
	/// behaviour.
	pub(crate) fn new_unchecked(hour: u8) -> Self {
		Self { value: hour }
	}
	
	/// Returns the hour value in 24-hour format.
	pub fn of(&self) -> u8 {
		self.value
	}
	
	/// Returns true if this represents a valid hour within a day.
	///
	/// Valid day hours are in the range 0-23. Hour 24 is valid as an end-of-day
	/// marker but is not considered a valid hour within the day.
	pub fn is_valid_day_hour(&self) -> bool {
		self.value < 24
	}
	
	/// Returns true if this represents the conceptual end of day.
	///
	/// End of day is represented by hour 24, which is equivalent to hour 0
	/// of the following day.
	pub fn is_end_of_day(&self) -> bool {
		self.value == 24
	}
	
	/// Converts this hour to 12-hour format.
	///
	/// # Returns
	///
	/// Returns a tuple containing:
	/// - `hour` - Hour in 12-hour format (1-12)
	/// - `is_pm` - True if PM, false if AM
	///
	/// # Notes
	///
	/// Hour 0 becomes 12 AM, hour 12 becomes 12 PM, and hour 24 becomes 12 AM.
	pub fn to_twelve_hour(&self) -> (u8, bool) {
		match self.value {
			0 => (12, false),		// midnight
			1..=11 => (self.value, false),	// AM
			12 => (12, true),		// noon
			13..=23 => (self.value - 12, true), // PM
			24 => (12, false),		// end of day = midnight
			_ => unreachable!(),
		}
	}
	
	/// Adds the specified number of hours, wrapping at day boundaries.
	///
	/// The result is normalised to the range 0-23. Hour overflow wraps back
	/// to hour 0.
	pub fn add_hours(&self, hours: u32) -> Self {
		let new_hour = (self.value as u32 + hours) % 24;
		Self::new_unchecked(new_hour as u8)
	}
	
	/// Subtracts the specified number of hours, wrapping at day boundaries.
	///
	/// The result is normalised to the range 0-23. Hour underflow wraps forward
	/// from hour 23.
	pub fn sub_hours(&self, hours: u32) -> Self {
		let hours = hours % 24;
		let new_hour = if hours as u8 > self.value {
			24 - (hours as u8 - self.value)
		} else {
			self.value - hours as u8
		};
		Self::new_unchecked(new_hour)
	}
	
	/// Creates a ClockHour from 12-hour format.
	///
	/// # Arguments
	///
	/// * `hour` - Hour in 12-hour format (1-12)
	/// * `is_pm` - True if PM, false if AM
	///
	/// # Returns
	///
	/// Returns `Ok(ClockHour)` if the hour is valid (1-12), otherwise returns
	/// an error describing the validation failure.
	pub fn from_12_hour(hour: u8, is_pm: bool) -> Outcome<Self> {
		if hour == 0 || hour > 12 {
			return Err(err!(
				"12-hour format hour {} is invalid, must be 1-12", 
				hour; 
				Invalid, Input
			));
		}
		
		let hour_24 = match (hour, is_pm) {
			(12, false) => 0,		// 12 AM = midnight
			(12, true) => 12,		// 12 PM = noon
			(h, false) => h,		// 1-11 AM
			(h, true) => h + 12,	// 1-11 PM
		};
		
		Ok(Self::new_unchecked(hour_24))
	}
}

// Validation methods.
impl ClockHour {
	/// Returns true if the hour value is within the valid range.
	///
	/// Valid hours are in the range 0-24 inclusive.
	pub fn is_valid(&self) -> bool {
		self.value <= Self::MAX_VALUE
	}
}

impl fmt::Display for ClockHour {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "{:02}", self.value)
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_hour_creation() {
		assert!(ClockHour::new(0).is_ok());
		assert!(ClockHour::new(12).is_ok());
		assert!(ClockHour::new(23).is_ok());
		assert!(ClockHour::new(24).is_ok());
		assert!(ClockHour::new(25).is_err());
	}

	#[test]
	fn test_twelve_hour_conversion() {
		assert_eq!(ClockHour::new(0).unwrap().to_twelve_hour(), (12, false));
		assert_eq!(ClockHour::new(1).unwrap().to_twelve_hour(), (1, false));
		assert_eq!(ClockHour::new(12).unwrap().to_twelve_hour(), (12, true));
		assert_eq!(ClockHour::new(13).unwrap().to_twelve_hour(), (1, true));
		assert_eq!(ClockHour::new(23).unwrap().to_twelve_hour(), (11, true));
	}

	#[test]
	fn test_hour_arithmetic() {
		let hour = ClockHour::new(10).unwrap();
		assert_eq!(hour.add_hours(5).of(), 15);
		assert_eq!(hour.add_hours(20).of(), 6); // wraps around
		assert_eq!(hour.sub_hours(5).of(), 5);
		assert_eq!(hour.sub_hours(15).of(), 19); // wraps around
	}
}