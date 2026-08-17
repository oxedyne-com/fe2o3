//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use crate::clock::{
	PerSecondRated,
	ClockNanoSecond,
};

use oxedyne_fe2o3_core::prelude::*;

use std::fmt;

/// A millisecond face over ClockNanoSecond: the arithmetic happens in
/// nanoseconds and is converted back on the way out.
///
/// ```ignore
/// use oxedyne_fe2o3_datime::clock::ClockMilliSecondres!();
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
	pub const MAX_VALUE: u32 = 999;
	pub const NANOS_PER_MILLI: u32 = 1_000_000;

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
	
	pub(crate) fn new_unchecked(millis: u32) -> Self {
		let nanos = ClockNanoSecond::new_unchecked(millis * Self::NANOS_PER_MILLI);
		Self { nanos }
	}
	
	pub fn of(&self) -> u32 {
		self.nanos.to_millis()
	}
	
	pub fn as_nanos(&self) -> ClockNanoSecond {
		self.nanos
	}
	
	/// Truncates the sub-millisecond part.
	pub fn from_nanos(nanos: ClockNanoSecond) -> Self {
		let millis = nanos.to_millis();
		Self::new_unchecked(millis)
	}
	
	pub fn add_millis(&self, millis: u32) -> (Self, u32) {
		let nanos_to_add = millis as u64 * Self::NANOS_PER_MILLI as u64;
		let (new_nanos, second_carry) = self.nanos.add_nanos(nanos_to_add);
		(Self::from_nanos(new_nanos), second_carry)
	}
	
	pub fn sub_millis(&self, millis: u32) -> (Self, u32) {
		let nanos_to_sub = millis as u64 * Self::NANOS_PER_MILLI as u64;
		let (new_nanos, second_borrow) = self.nanos.sub_nanos(nanos_to_sub);
		(Self::from_nanos(new_nanos), second_borrow)
	}
}

// Validation methods.
impl ClockMilliSecond {
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