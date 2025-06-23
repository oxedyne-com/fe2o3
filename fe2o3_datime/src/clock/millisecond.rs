use crate::clock::{
	PerSecondRated,
	ClockNanoSecond,
};

use oxedize_fe2o3_core::prelude::*;

use std::fmt;

/// Represents a millisecond as a convenience wrapper around ClockNanoSecond.
///
/// This type provides millisecond-precision timing by internally storing nanoseconds
/// but exposing a millisecond-oriented interface. All operations are performed at
/// nanosecond precision internally, with results converted back to millisecond
/// precision as needed.
///
/// # Precision
///
/// One millisecond represents one thousandth (10^-3) of a second. This type is
/// suitable for applications requiring moderate sub-second precision without the
/// complexity of full nanosecond handling.
///
/// # Examples
///
/// ```ignore
/// use oxedize_fe2o3_datime::clock::ClockMilliSecondres!();
///
/// let millis = ClockMilliSecond::new(500)?res!();
/// assert_eq!(millis.of(), 500)res!();
/// assert_eq!(millis.as_nanos().of(), 500_000_000)res!();
/// ```
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, PartialOrd, Ord)]
pub struct ClockMilliSecond {
	nanos: ClockNanoSecond,
}

impl ClockMilliSecond {
	/// Maximum valid millisecond value.
	/// 
	/// This represents 999 milliseconds, just under one full second.
	pub const MAX_VALUE: u32 = 999;
	
	/// Nanoseconds per millisecond.
	pub const NANOS_PER_MILLI: u32 = 1_000_000;

	/// Creates a new ClockMilliSecond from the given millisecond value.
	///
	/// # Arguments
	///
	/// * `millis` - Millisecond value within the second (0-999)
	///
	/// # Returns
	///
	/// Returns `Ok(ClockMilliSecond)` if the millisecond is valid, otherwise returns
	/// an error describing the validation failure.
	pub fn new(millis: u32) -> Outcome<Self> {
		if millis > Self::MAX_VALUE {
			return Err(err!(
				"Millisecond {} is invalid, must be 0-{}", 
				millis, 
				Self::MAX_VALUE; 
				Invalid, Input
			));
		}
		let nanos = res!(ClockNanoSecond::from_millis(millis));
		Ok(Self { nanos })
	}
	
	/// Creates a new ClockMilliSecond without validation.
	///
	/// This method is intended for internal use where the millisecond value is already
	/// known to be valid. Using this with invalid values will result in undefined
	/// behaviour.
	pub(crate) fn new_unchecked(millis: u32) -> Self {
		let nanos = ClockNanoSecond::new_unchecked(millis * Self::NANOS_PER_MILLI);
		Self { nanos }
	}
	
	/// Returns the millisecond value within the second.
	pub fn of(&self) -> u32 {
		self.nanos.to_millis()
	}
	
	/// Returns the underlying nanosecond representation.
	///
	/// This provides access to the internal nanosecond storage for high-precision
	/// operations or conversions.
	pub fn as_nanos(&self) -> ClockNanoSecond {
		self.nanos
	}
	
	/// Creates a ClockMilliSecond from nanoseconds with truncation.
	///
	/// The nanosecond value is truncated to millisecond precision, discarding
	/// any fractional millisecond component.
	pub fn from_nanos(nanos: ClockNanoSecond) -> Self {
		let millis = nanos.to_millis();
		Self::new_unchecked(millis)
	}
	
	/// Adds the specified number of milliseconds with carry handling.
	///
	/// # Arguments
	///
	/// * `millis` - Number of milliseconds to add
	///
	/// # Returns
	///
	/// Returns a tuple containing:
	/// - `new_millisecond` - The resulting millisecond (normalised to 0-999)
	/// - `second_carry` - Number of seconds to carry to the next higher unit
	pub fn add_millis(&self, millis: u32) -> (Self, u32) {
		let nanos_to_add = millis as u64 * Self::NANOS_PER_MILLI as u64;
		let (new_nanos, second_carry) = self.nanos.add_nanos(nanos_to_add);
		(Self::from_nanos(new_nanos), second_carry)
	}
	
	/// Subtracts the specified number of milliseconds with borrow handling.
	///
	/// # Arguments
	///
	/// * `millis` - Number of milliseconds to subtract
	///
	/// # Returns
	///
	/// Returns a tuple containing:
	/// - `new_millisecond` - The resulting millisecond (normalised to 0-999)
	/// - `second_borrow` - Number of seconds to borrow from the next higher unit
	pub fn sub_millis(&self, millis: u32) -> (Self, u32) {
		let nanos_to_sub = millis as u64 * Self::NANOS_PER_MILLI as u64;
		let (new_nanos, second_borrow) = self.nanos.sub_nanos(nanos_to_sub);
		(Self::from_nanos(new_nanos), second_borrow)
	}
}

// Validation methods.
impl ClockMilliSecond {
	/// Returns true if the millisecond value is within the valid range.
	///
	/// Valid milliseconds are in the range 0-999 inclusive.
	pub fn is_valid(&self) -> bool {
		self.of() <= Self::MAX_VALUE
	}
}

impl PerSecondRated for ClockMilliSecond {
	fn per_second(&self) -> u64 {
		1_000
	}
}

impl fmt::Display for ClockMilliSecond {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "{:03}", self.of())
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_millisecond_creation() {
		assert!(ClockMilliSecond::new(0).is_ok());
		assert!(ClockMilliSecond::new(500).is_ok());
		assert!(ClockMilliSecond::new(999).is_ok());
		assert!(ClockMilliSecond::new(1000).is_err());
	}

	#[test]
	fn test_millisecond_conversions() {
		let millis = ClockMilliSecond::new(123).unwrap();
		assert_eq!(millis.of(), 123);
		assert_eq!(millis.as_nanos().of(), 123_000_000);
		
		let nanos = ClockNanoSecond::new(123_456_789).unwrap();
		let millis = ClockMilliSecond::from_nanos(nanos);
		assert_eq!(millis.of(), 123); // truncated
	}

	#[test]
	fn test_millisecond_arithmetic() {
		let millis = ClockMilliSecond::new(500).unwrap();
		let (new_millis, carry) = millis.add_millis(300);
		assert_eq!(new_millis.of(), 800);
		assert_eq!(carry, 0);
		
		let (new_millis, carry) = millis.add_millis(700);
		assert_eq!(new_millis.of(), 200);
		assert_eq!(carry, 1);
		
		let (new_millis, borrow) = millis.sub_millis(200);
		assert_eq!(new_millis.of(), 300);
		assert_eq!(borrow, 0);
		
		let (new_millis, borrow) = millis.sub_millis(700);
		assert_eq!(new_millis.of(), 800);
		assert_eq!(borrow, 1);
	}

	#[test]
	fn test_per_second_rated() {
		let millis = ClockMilliSecond::new(456).unwrap();
		assert_eq!(millis.per_second(), 1_000);
	}
}