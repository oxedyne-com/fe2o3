use crate::clock::PerSecondRated;

use oxedize_fe2o3_core::prelude::*;

use std::fmt;

/// Represents a second within a minute with range 0-60.
///
/// Seconds are represented using the standard system where:
/// - 0 represents the start of the minute
/// - 59 represents the last second of the minute
/// - 60 represents a leap second (rare but valid in UTC)
///
/// This type provides validation, arithmetic operations with carry/borrow semantics,
/// implements the PerSecondRated trait for frequency calculations, and maintains
/// immutability throughout all operations.
///
/// # Examples
///
/// ```ignore
/// use oxedize_fe2o3_datime::clock::ClockSecondres!();
///
/// let second = ClockSecond::new(45)?res!();
/// let (new_second, minute_carry) = second.add_seconds(20)res!();
/// assert_eq!(new_second.of(), 5)res!();  // 45 + 20 = 65 -> 5 with carry
/// assert_eq!(minute_carry, 1)res!();
/// ```
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, PartialOrd, Ord)]
pub struct ClockSecond {
	value: u8,
}

impl ClockSecond {
	/// Maximum valid second value.
	/// 
	/// The value 60 represents a leap second, which occurs rarely in UTC.
	pub const MAX_VALUE: u8 = 60;
	
	/// Milliseconds per second.
	pub const MILLIS_PER_SECOND: u32 = 1_000;
	
	/// Microseconds per second.
	pub const MICROS_PER_SECOND: u32 = 1_000_000;
	
	/// Nanoseconds per second.
	pub const NANOS_PER_SECOND: u64 = 1_000_000_000;

	/// Creates a new ClockSecond from the given second value.
	///
	/// # Arguments
	///
	/// * `second` - Second value within the minute (0-60)
	///
	/// # Returns
	///
	/// Returns `Ok(ClockSecond)` if the second is valid (0-60), otherwise returns
	/// an error describing the validation failure.
	pub fn new(second: u8) -> Outcome<Self> {
		if second > Self::MAX_VALUE {
			return Err(err!(
				"Second {} is invalid, must be 0-{}", 
				second, 
				Self::MAX_VALUE; 
				Invalid, Input
			));
		}
		Ok(Self { value: second })
	}
	
	/// Creates a new ClockSecond without validation.
	///
	/// This method is intended for internal use where the second value is already
	/// known to be valid. Using this with invalid values will result in undefined
	/// behaviour.
	pub(crate) fn new_unchecked(second: u8) -> Self {
		Self { value: second }
	}
	
	/// Returns the second value within the minute.
	pub fn of(&self) -> u8 {
		self.value
	}
	
	/// Returns true if this represents a valid second within a minute.
	///
	/// Valid minute seconds are in the range 0-59. Second 60 is valid as a leap
	/// second but is not considered a normal second within the minute.
	pub fn is_valid_minute_second(&self) -> bool {
		self.value < 60
	}
	
	/// Returns true if this represents a leap second.
	///
	/// Leap seconds are represented by second 60, which occurs occasionally
	/// in UTC to account for variations in Earth's rotation.
	pub fn is_leap_second(&self) -> bool {
		self.value == 60
	}
	
	/// Adds the specified number of seconds with carry handling.
	///
	/// # Arguments
	///
	/// * `seconds` - Number of seconds to add
	///
	/// # Returns
	///
	/// Returns a tuple containing:
	/// - `new_second` - The resulting second (normalised to 0-59)
	/// - `minute_carry` - Number of minutes to carry to the next higher unit
	pub fn add_seconds(&self, seconds: u32) -> (Self, u32) {
		let total = self.value as u32 + seconds;
		let minute_carry = total / 60;
		let new_second = total % 60;
		(Self::new_unchecked(new_second as u8), minute_carry)
	}
	
	/// Subtracts the specified number of seconds with borrow handling.
	///
	/// # Arguments
	///
	/// * `seconds` - Number of seconds to subtract
	///
	/// # Returns
	///
	/// Returns a tuple containing:
	/// - `new_second` - The resulting second (normalised to 0-59)
	/// - `minute_borrow` - Number of minutes to borrow from the next higher unit
	pub fn sub_seconds(&self, seconds: u32) -> (Self, u32) {
		let seconds = seconds % (60 * 60 * 24); // reasonable limit
		if seconds as u8 > self.value {
			let borrow_needed = ((seconds as u8 - self.value + 59) / 60) as u32;
			let effective_seconds = borrow_needed * 60 + self.value as u32 - seconds;
			(Self::new_unchecked(effective_seconds as u8), borrow_needed)
		} else {
			(Self::new_unchecked(self.value - seconds as u8), 0)
		}
	}
}

// Validation methods.
impl ClockSecond {
	/// Returns true if the second value is within the valid range.
	///
	/// Valid seconds are in the range 0-60 inclusive.
	pub fn is_valid(&self) -> bool {
		self.value <= Self::MAX_VALUE
	}
}

impl PerSecondRated for ClockSecond {
	fn per_second(&self) -> u64 {
		1
	}
}

impl fmt::Display for ClockSecond {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "{:02}", self.value)
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_second_creation() {
		assert!(ClockSecond::new(0).is_ok());
		assert!(ClockSecond::new(30).is_ok());
		assert!(ClockSecond::new(59).is_ok());
		assert!(ClockSecond::new(60).is_ok()); // leap second
		assert!(ClockSecond::new(61).is_err());
	}

	#[test]
	fn test_second_arithmetic() {
		let second = ClockSecond::new(30).unwrap();
		let (new_sec, carry) = second.add_seconds(20);
		assert_eq!(new_sec.of(), 50);
		assert_eq!(carry, 0);
		
		let (new_sec, carry) = second.add_seconds(45);
		assert_eq!(new_sec.of(), 15);
		assert_eq!(carry, 1);
		
		let (new_sec, borrow) = second.sub_seconds(10);
		assert_eq!(new_sec.of(), 20);
		assert_eq!(borrow, 0);
		
		let (new_sec, borrow) = second.sub_seconds(45);
		assert_eq!(new_sec.of(), 45);
		assert_eq!(borrow, 1);
	}

	#[test]
	fn test_per_second_rated() {
		let second = ClockSecond::new(42).unwrap();
		assert_eq!(second.per_second(), 1);
	}
}