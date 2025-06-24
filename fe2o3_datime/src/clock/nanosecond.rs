use crate::clock::PerSecondRated;

use oxedyne_fe2o3_core::prelude::*;

use std::fmt;

/// Represents a nanosecond within a second with range 0-999,999,999.
///
/// Nanoseconds provide the highest precision time representation within the clock system.
/// This type handles sub-second precision timing with conversion utilities for milliseconds
/// and microseconds, arithmetic operations with carry/borrow semantics, and implements
/// the PerSecondRated trait for frequency calculations.
///
/// # Precision
///
/// One nanosecond represents one billionth (10^-9) of a second, providing extremely
/// high precision timing suitable for most applications requiring sub-second accuracy.
///
/// # Examples
///
/// ```ignore
/// use oxedyne_fe2o3_datime::clock::ClockNanoSecondres!();
///
/// let nanos = ClockNanoSecond::from_millis(500)?res!();
/// assert_eq!(nanos.of(), 500_000_000)res!();
/// assert_eq!(nanos.to_millis(), 500)res!();
/// ```
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, PartialOrd, Ord)]
pub struct ClockNanoSecond {
	value: u32,
}

impl ClockNanoSecond {
	/// Maximum valid nanosecond value.
	/// 
	/// This represents 999,999,999 nanoseconds, just under one full second.
	pub const MAX_VALUE: u32 = 999_999_999;
	
	/// Nanoseconds per second.
	pub const NANOS_PER_SECOND: u32 = 1_000_000_000;

	/// Creates a new ClockNanoSecond from the given nanosecond value.
	///
	/// # Arguments
	///
	/// * `nanosecond` - Nanosecond value within the second (0-999,999,999)
	///
	/// # Returns
	///
	/// Returns `Ok(ClockNanoSecond)` if the nanosecond is valid, otherwise returns
	/// an error describing the validation failure.
	pub fn new(nanosecond: u32) -> Outcome<Self> {
		if nanosecond > Self::MAX_VALUE {
			return Err(err!(
				"Nanosecond {} is invalid, must be 0-{}", 
				nanosecond, 
				Self::MAX_VALUE; 
				Invalid, Input
			));
		}
		Ok(Self { value: nanosecond })
	}
	
	/// Creates a new ClockNanoSecond without validation.
	///
	/// This method is intended for internal use where the nanosecond value is already
	/// known to be valid. Using this with invalid values will result in undefined
	/// behaviour.
	pub(crate) fn new_unchecked(nanosecond: u32) -> Self {
		Self { value: nanosecond }
	}
	
	/// Returns the nanosecond value within the second.
	pub fn of(&self) -> u32 {
		self.value
	}
	
	/// Creates a ClockNanoSecond from a millisecond value.
	///
	/// # Arguments
	///
	/// * `millis` - Milliseconds to convert to nanoseconds
	///
	/// # Returns
	///
	/// Returns `Ok(ClockNanoSecond)` if the conversion is valid, otherwise returns
	/// an error if the millisecond value would cause overflow.
	pub fn from_millis(millis: u32) -> Outcome<Self> {
		let nanos = res!(millis.checked_mul(1_000_000)
			.ok_or_else(|| err!("Millisecond overflow"; Overflow)));
		Self::new(nanos)
	}
	
	/// Creates a ClockNanoSecond from a microsecond value.
	///
	/// # Arguments
	///
	/// * `micros` - Microseconds to convert to nanoseconds
	///
	/// # Returns
	///
	/// Returns `Ok(ClockNanoSecond)` if the conversion is valid, otherwise returns
	/// an error if the microsecond value would cause overflow.
	pub fn from_micros(micros: u32) -> Outcome<Self> {
		let nanos = res!(micros.checked_mul(1_000)
			.ok_or_else(|| err!("Microsecond overflow"; Overflow)));
		Self::new(nanos)
	}
	
	/// Converts to milliseconds with truncation.
	///
	/// This conversion truncates any fractional millisecond component.
	pub fn to_millis(&self) -> u32 {
		self.value / 1_000_000
	}
	
	/// Converts to microseconds with truncation.
	///
	/// This conversion truncates any fractional microsecond component.
	pub fn to_micros(&self) -> u32 {
		self.value / 1_000
	}
	
	/// Adds the specified number of nanoseconds with carry handling.
	///
	/// # Arguments
	///
	/// * `nanos` - Number of nanoseconds to add
	///
	/// # Returns
	///
	/// Returns a tuple containing:
	/// - `new_nanosecond` - The resulting nanosecond (normalised to 0-999,999,999)
	/// - `second_carry` - Number of seconds to carry to the next higher unit
	pub fn add_nanos(&self, nanos: u64) -> (Self, u32) {
		let total = self.value as u64 + nanos;
		let second_carry = (total / Self::NANOS_PER_SECOND as u64) as u32;
		let new_nano = (total % Self::NANOS_PER_SECOND as u64) as u32;
		(Self::new_unchecked(new_nano), second_carry)
	}
	
	/// Subtracts the specified number of nanoseconds with borrow handling.
	///
	/// # Arguments
	///
	/// * `nanos` - Number of nanoseconds to subtract
	///
	/// # Returns
	///
	/// Returns a tuple containing:
	/// - `new_nanosecond` - The resulting nanosecond (normalised to 0-999,999,999)
	/// - `second_borrow` - Number of seconds to borrow from the next higher unit
	pub fn sub_nanos(&self, nanos: u64) -> (Self, u32) {
		if nanos as u32 > self.value {
			let borrow_needed = ((nanos as u32 - self.value + Self::NANOS_PER_SECOND - 1) / Self::NANOS_PER_SECOND) as u32;
			let effective_nanos = borrow_needed as u64 * Self::NANOS_PER_SECOND as u64 + self.value as u64 - nanos;
			(Self::new_unchecked(effective_nanos as u32), borrow_needed)
		} else {
			(Self::new_unchecked(self.value - nanos as u32), 0)
		}
	}
}

// Validation methods.
impl ClockNanoSecond {
	/// Returns true if the nanosecond value is within the valid range.
	///
	/// Valid nanoseconds are in the range 0-999,999,999 inclusive.
	pub fn is_valid(&self) -> bool {
		self.value <= Self::MAX_VALUE
	}
}

impl PerSecondRated for ClockNanoSecond {
	fn per_second(&self) -> u64 {
		Self::NANOS_PER_SECOND as u64
	}
}

impl fmt::Display for ClockNanoSecond {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "{:09}", self.value)
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_nanosecond_creation() {
		assert!(ClockNanoSecond::new(0).is_ok());
		assert!(ClockNanoSecond::new(500_000_000).is_ok());
		assert!(ClockNanoSecond::new(999_999_999).is_ok());
		assert!(ClockNanoSecond::new(1_000_000_000).is_err());
	}

	#[test]
	fn test_conversions() {
		let nanos = ClockNanoSecond::from_millis(500).unwrap();
		assert_eq!(nanos.of(), 500_000_000);
		assert_eq!(nanos.to_millis(), 500);
		
		let nanos = ClockNanoSecond::from_micros(500_000).unwrap();
		assert_eq!(nanos.of(), 500_000_000);
		assert_eq!(nanos.to_micros(), 500_000);
	}

	#[test]
	fn test_nanosecond_arithmetic() {
		let nano = ClockNanoSecond::new(500_000_000).unwrap();
		let (new_nano, carry) = nano.add_nanos(300_000_000);
		assert_eq!(new_nano.of(), 800_000_000);
		assert_eq!(carry, 0);
		
		let (new_nano, carry) = nano.add_nanos(700_000_000);
		assert_eq!(new_nano.of(), 200_000_000);
		assert_eq!(carry, 1);
		
		let (new_nano, borrow) = nano.sub_nanos(200_000_000);
		assert_eq!(new_nano.of(), 300_000_000);
		assert_eq!(borrow, 0);
		
		let (new_nano, borrow) = nano.sub_nanos(700_000_000);
		assert_eq!(new_nano.of(), 800_000_000);
		assert_eq!(borrow, 1);
	}

	#[test]
	fn test_per_second_rated() {
		let nano = ClockNanoSecond::new(123_456_789).unwrap();
		assert_eq!(nano.per_second(), 1_000_000_000);
	}
}