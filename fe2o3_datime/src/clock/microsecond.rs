use crate::clock::{
	PerSecondRated,
	ClockNanoSecond,
};

use oxedize_fe2o3_core::prelude::*;

use std::fmt;

/// Represents a microsecond as a convenience wrapper around ClockNanoSecond.
///
/// This type provides microsecond-precision timing by internally storing nanoseconds
/// but exposing a microsecond-oriented interface. All operations are performed at
/// nanosecond precision internally, with results converted back to microsecond
/// precision as needed.
///
/// # Precision
///
/// One microsecond represents one millionth (10^-6) of a second. This type provides
/// higher precision than milliseconds whilst being more manageable than nanoseconds
/// for many timing applications.
///
/// # Examples
///
/// ```ignore
/// use oxedize_fe2o3_datime::clock::ClockMicroSecondres!();
///
/// let micros = ClockMicroSecond::new(500_000)?res!();
/// assert_eq!(micros.of(), 500_000)res!();
/// assert_eq!(micros.as_nanos().of(), 500_000_000)res!();
/// ```
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, PartialOrd, Ord)]
pub struct ClockMicroSecond {
	nanos: ClockNanoSecond,
}

impl ClockMicroSecond {
	/// Maximum valid microsecond value.
	/// 
	/// This represents 999,999 microseconds, just under one full second.
	pub const MAX_VALUE: u32 = 999_999;
	
	/// Nanoseconds per microsecond.
	pub const NANOS_PER_MICRO: u32 = 1_000;

	/// Creates a new ClockMicroSecond from the given microsecond value.
	///
	/// # Arguments
	///
	/// * `micros` - Microsecond value within the second (0-999,999)
	///
	/// # Returns
	///
	/// Returns `Ok(ClockMicroSecond)` if the microsecond is valid, otherwise returns
	/// an error describing the validation failure.
	pub fn new(micros: u32) -> Outcome<Self> {
		if micros > Self::MAX_VALUE {
			return Err(err!(
				"Microsecond {} is invalid, must be 0-{}", 
				micros, 
				Self::MAX_VALUE; 
				Invalid, Input
			));
		}
		let nanos = res!(ClockNanoSecond::from_micros(micros));
		Ok(Self { nanos })
	}
	
	/// Creates a new ClockMicroSecond without validation.
	///
	/// This method is intended for internal use where the microsecond value is already
	/// known to be valid. Using this with invalid values will result in undefined
	/// behaviour.
	pub(crate) fn new_unchecked(micros: u32) -> Self {
		let nanos = ClockNanoSecond::new_unchecked(micros * Self::NANOS_PER_MICRO);
		Self { nanos }
	}
	
	/// Returns the microsecond value within the second.
	pub fn of(&self) -> u32 {
		self.nanos.to_micros()
	}
	
	/// Returns the underlying nanosecond representation.
	///
	/// This provides access to the internal nanosecond storage for high-precision
	/// operations or conversions.
	pub fn as_nanos(&self) -> ClockNanoSecond {
		self.nanos
	}
	
	/// Creates a ClockMicroSecond from nanoseconds with truncation.
	///
	/// The nanosecond value is truncated to microsecond precision, discarding
	/// any fractional microsecond component.
	pub fn from_nanos(nanos: ClockNanoSecond) -> Self {
		let micros = nanos.to_micros();
		Self::new_unchecked(micros)
	}
	
	/// Adds the specified number of microseconds with carry handling.
	///
	/// # Arguments
	///
	/// * `micros` - Number of microseconds to add
	///
	/// # Returns
	///
	/// Returns a tuple containing:
	/// - `new_microsecond` - The resulting microsecond (normalised to 0-999,999)
	/// - `second_carry` - Number of seconds to carry to the next higher unit
	pub fn add_micros(&self, micros: u32) -> (Self, u32) {
		let nanos_to_add = micros as u64 * Self::NANOS_PER_MICRO as u64;
		let (new_nanos, second_carry) = self.nanos.add_nanos(nanos_to_add);
		(Self::from_nanos(new_nanos), second_carry)
	}
	
	/// Subtracts the specified number of microseconds with borrow handling.
	///
	/// # Arguments
	///
	/// * `micros` - Number of microseconds to subtract
	///
	/// # Returns
	///
	/// Returns a tuple containing:
	/// - `new_microsecond` - The resulting microsecond (normalised to 0-999,999)
	/// - `second_borrow` - Number of seconds to borrow from the next higher unit
	pub fn sub_micros(&self, micros: u32) -> (Self, u32) {
		let nanos_to_sub = micros as u64 * Self::NANOS_PER_MICRO as u64;
		let (new_nanos, second_borrow) = self.nanos.sub_nanos(nanos_to_sub);
		(Self::from_nanos(new_nanos), second_borrow)
	}
}

// Validation methods.
impl ClockMicroSecond {
	/// Returns true if the microsecond value is within the valid range.
	///
	/// Valid microseconds are in the range 0-999,999 inclusive.
	pub fn is_valid(&self) -> bool {
		self.of() <= Self::MAX_VALUE
	}
}

impl PerSecondRated for ClockMicroSecond {
	fn per_second(&self) -> u64 {
		1_000_000
	}
}

impl fmt::Display for ClockMicroSecond {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "{:06}", self.of())
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_microsecond_creation() {
		assert!(ClockMicroSecond::new(0).is_ok());
		assert!(ClockMicroSecond::new(500_000).is_ok());
		assert!(ClockMicroSecond::new(999_999).is_ok());
		assert!(ClockMicroSecond::new(1_000_000).is_err());
	}

	#[test]
	fn test_microsecond_conversions() {
		let micros = ClockMicroSecond::new(123_456).unwrap();
		assert_eq!(micros.of(), 123_456);
		assert_eq!(micros.as_nanos().of(), 123_456_000);
		
		let nanos = ClockNanoSecond::new(123_456_789).unwrap();
		let micros = ClockMicroSecond::from_nanos(nanos);
		assert_eq!(micros.of(), 123_456); // truncated
	}

	#[test]
	fn test_microsecond_arithmetic() {
		let micros = ClockMicroSecond::new(500_000).unwrap();
		let (new_micros, carry) = micros.add_micros(300_000);
		assert_eq!(new_micros.of(), 800_000);
		assert_eq!(carry, 0);
		
		let (new_micros, carry) = micros.add_micros(700_000);
		assert_eq!(new_micros.of(), 200_000);
		assert_eq!(carry, 1);
		
		let (new_micros, borrow) = micros.sub_micros(200_000);
		assert_eq!(new_micros.of(), 300_000);
		assert_eq!(borrow, 0);
		
		let (new_micros, borrow) = micros.sub_micros(700_000);
		assert_eq!(new_micros.of(), 800_000);
		assert_eq!(borrow, 1);
	}

	#[test]
	fn test_per_second_rated() {
		let micros = ClockMicroSecond::new(456_789).unwrap();
		assert_eq!(micros.per_second(), 1_000_000);
	}
}