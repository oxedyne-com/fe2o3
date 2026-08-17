//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use crate::core::Duration;

use oxedyne_fe2o3_core::prelude::*;

use std::fmt;

/// A signed count of nanoseconds, so a duration may run backwards, and the
/// 64-bit range reaches about ±292 years.
///
/// ```ignore
/// use oxedyne_fe2o3_datime::clock::ClockDurationres!();
///
/// let duration1 = ClockDuration::from_hours(2)res!();
/// let duration2 = ClockDuration::from_minutes(30)res!();
/// let total = duration1.plus(&duration2)res!();
/// assert_eq!(total.total_hours(), 2)res!();
/// assert_eq!(total.total_minutes(), 150)res!();
/// ```
#[derive(Clone, Debug, PartialEq)]
pub struct ClockDuration {
	nanos: i64,
}

impl ClockDuration {
	pub fn from_nanos(nanos: i64) -> Self {
		Self { nanos }
	}
	
	pub fn from_seconds(seconds: i64) -> Self {
		Self { nanos: seconds * 1_000_000_000 }
	}
	
	pub fn from_millis(millis: i64) -> Self {
		Self { nanos: millis * 1_000_000 }
	}
	
	pub fn from_micros(micros: i64) -> Self {
		Self { nanos: micros * 1_000 }
	}
	
	pub fn from_minutes(minutes: i64) -> Self {
		Self { nanos: minutes * 60 * 1_000_000_000 }
	}
	
	pub fn from_hours(hours: i64) -> Self {
		Self { nanos: hours * 60 * 60 * 1_000_000_000 }
	}
	
	pub fn zero() -> Self {
		Self { nanos: 0 }
	}
	
	pub fn total_nanos(&self) -> i64 {
		self.nanos
	}
	
	/// Truncates rather than rounds, as do the coarser totals below.
	pub fn total_micros(&self) -> i64 {
		self.nanos / 1_000
	}
	
	pub fn total_millis(&self) -> i64 {
		self.nanos / 1_000_000
	}
	
	pub fn total_seconds(&self) -> i64 {
		self.nanos / 1_000_000_000
	}
	
	pub fn total_minutes(&self) -> i64 {
		self.nanos / (60 * 1_000_000_000)
	}
	
	pub fn total_hours(&self) -> i64 {
		self.nanos / (60 * 60 * 1_000_000_000)
	}
	
	pub fn abs(&self) -> Self {
		Self { nanos: self.nanos.abs() }
	}
	
	pub fn negate(&self) -> Self {
		Self { nanos: -self.nanos }
	}
	
	pub fn plus(&self, other: &Self) -> Self {
		Self { nanos: self.nanos + other.nanos }
	}
	
	pub fn minus(&self, other: &Self) -> Self {
		Self { nanos: self.nanos - other.nanos }
	}
	
	pub fn multiply_by(&self, factor: i64) -> Self {
		Self { nanos: self.nanos * factor }
	}
	
	pub fn divide_by(&self, divisor: i64) -> Outcome<Self> {
		if divisor == 0 {
			return Err(err!("Cannot divide duration by zero"; Invalid, Input));
		}
		Ok(Self { nanos: self.nanos / divisor })
	}
	
	/// For a negative duration the sign is carried in the first non-zero component.
	pub fn to_components(&self) -> (i64, i64, i64, i64) {
		let mut remaining = self.nanos.abs();
		let negative = self.nanos < 0;
		
		let hours = remaining / (60 * 60 * 1_000_000_000);
		remaining %= 60 * 60 * 1_000_000_000;
		
		let minutes = remaining / (60 * 1_000_000_000);
		remaining %= 60 * 1_000_000_000;
		
		let seconds = remaining / 1_000_000_000;
		let nanos = remaining % 1_000_000_000;
		
		if negative {
			if hours > 0 {
				(-hours, minutes, seconds, nanos)
			} else if minutes > 0 {
				(0, -minutes, seconds, nanos)
			} else if seconds > 0 {
				(0, 0, -seconds, nanos)
			} else {
				(0, 0, 0, -nanos)
			}
		} else {
			(hours, minutes, seconds, nanos)
		}
	}
}

impl Duration for ClockDuration {
	fn to_nanos(&self) -> Outcome<i64> {
		Ok(self.nanos)
	}
	
	fn to_seconds(&self) -> Outcome<i64> {
		Ok(self.total_seconds())
	}
	
	fn to_days(&self) -> Outcome<i32> {
		const NANOS_PER_DAY: i64 = 24 * 60 * 60 * 1_000_000_000;
		let days = self.nanos / NANOS_PER_DAY;
		
		if days > i32::MAX as i64 || days < i32::MIN as i64 {
			return Err(err!("Duration too large to represent in days"; Overflow));
		}
		
		Ok(days as i32)
	}
	
	fn is_negative(&self) -> bool {
		self.nanos < 0
	}
}

impl fmt::Display for ClockDuration {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		let (hours, minutes, seconds, nanos) = self.to_components();
		
		if hours != 0 {
			if nanos == 0 {
				write!(f, "{}:{:02}:{:02}", hours, minutes.abs(), seconds.abs())
			} else {
				write!(f, "{}:{:02}:{:02}.{:09}", hours, minutes.abs(), seconds.abs(), nanos.abs())
			}
		} else if minutes != 0 {
			if nanos == 0 {
				write!(f, "{}:{:02}", minutes, seconds.abs())
			} else {
				write!(f, "{}:{:02}.{:09}", minutes, seconds.abs(), nanos.abs())
			}
		} else if seconds != 0 {
			if nanos == 0 {
				write!(f, "{}s", seconds)
			} else {
				write!(f, "{}.{:09}s", seconds, nanos.abs())
			}
		} else {
			write!(f, "{}ns", nanos)
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_duration_creation() {
		let dur = ClockDuration::from_seconds(3661); // 1h 1m 1s
		assert_eq!(dur.total_hours(), 1);
		assert_eq!(dur.total_minutes(), 61);
		assert_eq!(dur.total_seconds(), 3661);
		
		let (h, m, s, n) = dur.to_components();
		assert_eq!(h, 1);
		assert_eq!(m, 1);
		assert_eq!(s, 1);
		assert_eq!(n, 0);
	}

	#[test]
	fn test_duration_arithmetic() {
		let dur1 = ClockDuration::from_minutes(30);
		let dur2 = ClockDuration::from_minutes(45);
		
		let sum = dur1.plus(&dur2);
		assert_eq!(sum.total_minutes(), 75);
		
		let diff = dur2.minus(&dur1);
		assert_eq!(diff.total_minutes(), 15);
	}

	#[test]
	fn test_negative_duration() {
		let dur = ClockDuration::from_seconds(-3661);
		assert!(dur.is_negative());
		
		let (h, m, s, n) = dur.to_components();
		assert_eq!(h, -1);
		assert_eq!(m, 1);  // magnitude
		assert_eq!(s, 1);  // magnitude
		assert_eq!(n, 0);
	}

	#[test]
	fn test_duration_division() {
		let dur = ClockDuration::from_hours(6);
		let half = dur.divide_by(2).unwrap();
		assert_eq!(half.total_hours(), 3);
		
		assert!(dur.divide_by(0).is_err());
	}

	#[test]
	fn test_duration_display() {
		let dur = ClockDuration::from_nanos(3661_123_456_789);
		let display = fmt!("{}", dur);
		assert!(display.contains("1:01:01"));
		assert!(display.contains("123456789"));
	}
}