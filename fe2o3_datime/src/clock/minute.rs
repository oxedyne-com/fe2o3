//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use oxedyne_fe2o3_core::prelude::*;

use std::fmt;

/// 0 to 60. The last minute of the hour is 59; 60 is the end of the hour, the
/// same instant as minute 0 of the next one.
///
/// ```ignore
/// use oxedyne_fe2o3_datime::clock::ClockMinuteres!();
///
/// let minute = ClockMinute::new(45)?res!();
/// let (new_minute, hour_carry) = minute.add_minutes(20)res!();
/// assert_eq!(new_minute.of(), 5)res!();  // 45 + 20 = 65 -> 5 with carry
/// assert_eq!(hour_carry, 1)res!();
/// ```
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, PartialOrd, Ord)]
pub struct ClockMinute {
	value: u8,
}

impl ClockMinute {
	pub const MAX_VALUE: u8 = 60; // 60 is the end of the hour, not a minute of it
	pub const SECONDS_PER_MINUTE: u32 = 60;
	pub const MILLIS_PER_MINUTE: u32 = 60_000;
	pub const MICROS_PER_MINUTE: u32 = 60_000_000;
	pub const NANOS_PER_MINUTE: u64 = 60_000_000_000;

	pub fn new(minute: u8) -> Outcome<Self> {
		if minute > Self::MAX_VALUE {
			return Err(err!(
				"Minute {} is invalid, must be 0-{}", 
				minute, 
				Self::MAX_VALUE; 
				Invalid, Input
			));
		}
		Ok(Self { value: minute })
	}
	
	pub(crate) fn new_unchecked(minute: u8) -> Self {
		Self { value: minute }
	}
	
	pub fn of(&self) -> u8 {
		self.value
	}
	
	/// 60 is a valid ClockMinute but not a minute within an hour.
	pub fn is_valid_hour_minute(&self) -> bool {
		self.value < 60
	}
	
	pub fn is_end_of_hour(&self) -> bool {
		self.value == 60
	}
	
	pub fn add_minutes(&self, minutes: u32) -> (Self, u32) {
		let total = self.value as u32 + minutes;
		let hour_carry = total / 60;
		let new_minute = total % 60;
		(Self::new_unchecked(new_minute as u8), hour_carry)
	}
	
	pub fn sub_minutes(&self, minutes: u32) -> (Self, u32) {
		let minutes = minutes % (60 * 24); // reasonable limit
		if minutes as u8 > self.value {
			let borrow_needed = ((minutes as u8 - self.value + 59) / 60) as u32;
			let effective_minutes = borrow_needed * 60 + self.value as u32 - minutes;
			(Self::new_unchecked(effective_minutes as u8), borrow_needed)
		} else {
			(Self::new_unchecked(self.value - minutes as u8), 0)
		}
	}
}

// Validation methods.
impl ClockMinute {
	pub fn is_valid(&self) -> bool {
		self.value <= Self::MAX_VALUE
	}
}

impl fmt::Display for ClockMinute {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "{:02}", self.value)
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_minute_creation() {
		assert!(ClockMinute::new(0).is_ok());
		assert!(ClockMinute::new(30).is_ok());
		assert!(ClockMinute::new(59).is_ok());
		assert!(ClockMinute::new(60).is_ok());
		assert!(ClockMinute::new(61).is_err());
	}

	#[test]
	fn test_minute_arithmetic() {
		let minute = ClockMinute::new(30).unwrap();
		let (new_min, carry) = minute.add_minutes(20);
		assert_eq!(new_min.of(), 50);
		assert_eq!(carry, 0);
		
		let (new_min, carry) = minute.add_minutes(45);
		assert_eq!(new_min.of(), 15);
		assert_eq!(carry, 1);
		
		let (new_min, borrow) = minute.sub_minutes(10);
		assert_eq!(new_min.of(), 20);
		assert_eq!(borrow, 0);
		
		let (new_min, borrow) = minute.sub_minutes(45);
		assert_eq!(new_min.of(), 45);
		assert_eq!(borrow, 1);
	}
}