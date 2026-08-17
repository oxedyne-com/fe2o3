//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use crate::clock::{
	PerSecondRated,
	ClockNanoSecond,
};

use oxedyne_fe2o3_core::prelude::*;

use std::fmt;

/// A microsecond face over ClockNanoSecond: the arithmetic happens in
/// nanoseconds and is converted back on the way out.
///
/// ```ignore
/// use oxedyne_fe2o3_datime::clock::ClockMicroSecondres!();
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
	pub const MAX_VALUE: u32 = 999_999;
	pub const NANOS_PER_MICRO: u32 = 1_000;

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
	
	pub(crate) fn new_unchecked(micros: u32) -> Self {
		let nanos = ClockNanoSecond::new_unchecked(micros * Self::NANOS_PER_MICRO);
		Self { nanos }
	}
	
	pub fn of(&self) -> u32 {
		self.nanos.to_micros()
	}
	
	pub fn as_nanos(&self) -> ClockNanoSecond {
		self.nanos
	}
	
	/// Truncates the sub-microsecond part.
	pub fn from_nanos(nanos: ClockNanoSecond) -> Self {
		let micros = nanos.to_micros();
		Self::new_unchecked(micros)
	}
	
	pub fn add_micros(&self, micros: u32) -> (Self, u32) {
		let nanos_to_add = micros as u64 * Self::NANOS_PER_MICRO as u64;
		let (new_nanos, second_carry) = self.nanos.add_nanos(nanos_to_add);
		(Self::from_nanos(new_nanos), second_carry)
	}
	
	pub fn sub_micros(&self, micros: u32) -> (Self, u32) {
		let nanos_to_sub = micros as u64 * Self::NANOS_PER_MICRO as u64;
		let (new_nanos, second_borrow) = self.nanos.sub_nanos(nanos_to_sub);
		(Self::from_nanos(new_nanos), second_borrow)
	}
}

// Validation methods.
impl ClockMicroSecond {
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