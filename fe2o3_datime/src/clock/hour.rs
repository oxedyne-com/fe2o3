//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use oxedyne_fe2o3_core::prelude::*;

use std::fmt;

/// 0 to 24 on the 24-hour clock. The last hour of the day is 23; 24 is the end
/// of the day, the same instant as midnight starting the next one.
///
/// ```ignore
/// use oxedyne_fe2o3_datime::clock::ClockHourres!();
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
	pub const MAX_VALUE: u8 = 24; // 24 is the end of day, not an hour of it
	pub const MINUTES_PER_HOUR: u32 = 60;
	pub const SECONDS_PER_HOUR: u32 = 3_600;
	pub const MILLIS_PER_HOUR: u32 = 3_600_000;
	pub const MICROS_PER_HOUR: u64 = 3_600_000_000;
	pub const NANOS_PER_HOUR: u64 = 3_600_000_000_000;

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
	
	pub(crate) fn new_unchecked(hour: u8) -> Self {
		Self { value: hour }
	}
	
	pub fn of(&self) -> u8 {
		self.value
	}
	
	/// 24 is a valid ClockHour but not an hour within a day.
	pub fn is_valid_day_hour(&self) -> bool {
		self.value < 24
	}
	
	pub fn is_end_of_day(&self) -> bool {
		self.value == 24
	}
	
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
	
	/// Wraps within 0-23.
	pub fn add_hours(&self, hours: u32) -> Self {
		let new_hour = (self.value as u32 + hours) % 24;
		Self::new_unchecked(new_hour as u8)
	}
	
	pub fn sub_hours(&self, hours: u32) -> Self {
		let hours = hours % 24;
		let new_hour = if hours as u8 > self.value {
			24 - (hours as u8 - self.value)
		} else {
			self.value - hours as u8
		};
		Self::new_unchecked(new_hour)
	}
	
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