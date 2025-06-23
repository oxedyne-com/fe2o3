use oxedize_fe2o3_core::prelude::*;

use std::fmt;

/// Represents a minute within an hour with range 0-60.
///
/// Minutes are represented using the standard system where:
/// - 0 represents the start of the hour
/// - 59 represents the last minute of the hour
/// - 60 represents the end of the hour (equivalent to minute 0 of the following hour)
///
/// This type provides validation, arithmetic operations with carry/borrow semantics,
/// and maintains immutability throughout all operations.
///
/// # Examples
///
/// ```ignore
/// use oxedize_fe2o3_datime::clock::ClockMinuteres!();
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
	/// Maximum valid minute value.
	/// 
	/// The value 60 represents the conceptual end of the hour.
	pub const MAX_VALUE: u8 = 60;
	
	/// Seconds per minute.
	pub const SECONDS_PER_MINUTE: u32 = 60;
	
	/// Milliseconds per minute.
	pub const MILLIS_PER_MINUTE: u32 = 60_000;
	
	/// Microseconds per minute.
	pub const MICROS_PER_MINUTE: u32 = 60_000_000;
	
	/// Nanoseconds per minute.
	pub const NANOS_PER_MINUTE: u64 = 60_000_000_000;

	/// Creates a new ClockMinute from the given minute value.
	///
	/// # Arguments
	///
	/// * `minute` - Minute value within the hour (0-60)
	///
	/// # Returns
	///
	/// Returns `Ok(ClockMinute)` if the minute is valid (0-60), otherwise returns
	/// an error describing the validation failure.
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
	
	/// Creates a new ClockMinute without validation.
	///
	/// This method is intended for internal use where the minute value is already
	/// known to be valid. Using this with invalid values will result in undefined
	/// behaviour.
	pub(crate) fn new_unchecked(minute: u8) -> Self {
		Self { value: minute }
	}
	
	/// Returns the minute value within the hour.
	pub fn of(&self) -> u8 {
		self.value
	}
	
	/// Returns true if this represents a valid minute within an hour.
	///
	/// Valid hour minutes are in the range 0-59. Minute 60 is valid as an end-of-hour
	/// marker but is not considered a valid minute within the hour.
	pub fn is_valid_hour_minute(&self) -> bool {
		self.value < 60
	}
	
	/// Returns true if this represents the conceptual end of the hour.
	///
	/// End of hour is represented by minute 60, which is equivalent to minute 0
	/// of the following hour.
	pub fn is_end_of_hour(&self) -> bool {
		self.value == 60
	}
	
	/// Adds the specified number of minutes with carry handling.
	///
	/// # Arguments
	///
	/// * `minutes` - Number of minutes to add
	///
	/// # Returns
	///
	/// Returns a tuple containing:
	/// - `new_minute` - The resulting minute (normalised to 0-59)
	/// - `hour_carry` - Number of hours to carry to the next higher unit
	pub fn add_minutes(&self, minutes: u32) -> (Self, u32) {
		let total = self.value as u32 + minutes;
		let hour_carry = total / 60;
		let new_minute = total % 60;
		(Self::new_unchecked(new_minute as u8), hour_carry)
	}
	
	/// Subtracts the specified number of minutes with borrow handling.
	///
	/// # Arguments
	///
	/// * `minutes` - Number of minutes to subtract
	///
	/// # Returns
	///
	/// Returns a tuple containing:
	/// - `new_minute` - The resulting minute (normalised to 0-59)
	/// - `hour_borrow` - Number of hours to borrow from the next higher unit
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
	/// Returns true if the minute value is within the valid range.
	///
	/// Valid minutes are in the range 0-60 inclusive.
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