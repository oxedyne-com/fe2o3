use crate::core::Duration;

use oxedize_fe2o3_core::prelude::*;

use std::fmt;

/// Represents a duration of time with nanosecond precision.
///
/// ClockDuration provides high-precision time duration representation and arithmetic
/// operations. Durations can be positive or negative, allowing for both forward and
/// backward time calculations. All arithmetic maintains nanosecond precision whilst
/// providing convenient access methods for larger time units.
///
/// # Precision
///
/// Internally, durations are stored as signed 64-bit nanosecond counts, providing
/// a range of approximately ±292 years with nanosecond precision.
///
/// # Examples
///
/// ```ignore
/// use oxedize_fe2o3_datime::clock::ClockDurationres!();
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
	/// Creates a new ClockDuration from a nanosecond count.
	///
	/// # Arguments
	///
	/// * `nanos` - Duration in nanoseconds (positive or negative)
	pub fn from_nanos(nanos: i64) -> Self {
		Self { nanos }
	}
	
	/// Creates a new ClockDuration from a second count.
	///
	/// # Arguments
	///
	/// * `seconds` - Duration in seconds (positive or negative)
	pub fn from_seconds(seconds: i64) -> Self {
		Self { nanos: seconds * 1_000_000_000 }
	}
	
	/// Creates a new ClockDuration from a millisecond count.
	///
	/// # Arguments
	///
	/// * `millis` - Duration in milliseconds (positive or negative)
	pub fn from_millis(millis: i64) -> Self {
		Self { nanos: millis * 1_000_000 }
	}
	
	/// Creates a new ClockDuration from a microsecond count.
	///
	/// # Arguments
	///
	/// * `micros` - Duration in microseconds (positive or negative)
	pub fn from_micros(micros: i64) -> Self {
		Self { nanos: micros * 1_000 }
	}
	
	/// Creates a new ClockDuration from a minute count.
	///
	/// # Arguments
	///
	/// * `minutes` - Duration in minutes (positive or negative)
	pub fn from_minutes(minutes: i64) -> Self {
		Self { nanos: minutes * 60 * 1_000_000_000 }
	}
	
	/// Creates a new ClockDuration from an hour count.
	///
	/// # Arguments
	///
	/// * `hours` - Duration in hours (positive or negative)
	pub fn from_hours(hours: i64) -> Self {
		Self { nanos: hours * 60 * 60 * 1_000_000_000 }
	}
	
	/// Creates a zero duration.
	///
	/// This represents no elapsed time and is useful as a neutral element
	/// for duration arithmetic.
	pub fn zero() -> Self {
		Self { nanos: 0 }
	}
	
	/// Returns the total duration in nanoseconds.
	///
	/// This provides access to the full precision internal representation.
	pub fn total_nanos(&self) -> i64 {
		self.nanos
	}
	
	/// Returns the total duration in microseconds with truncation.
	///
	/// Any fractional microsecond component is discarded.
	pub fn total_micros(&self) -> i64 {
		self.nanos / 1_000
	}
	
	/// Returns the total duration in milliseconds with truncation.
	///
	/// Any fractional millisecond component is discarded.
	pub fn total_millis(&self) -> i64 {
		self.nanos / 1_000_000
	}
	
	/// Returns the total duration in seconds with truncation.
	///
	/// Any fractional second component is discarded.
	pub fn total_seconds(&self) -> i64 {
		self.nanos / 1_000_000_000
	}
	
	/// Returns the total duration in minutes with truncation.
	///
	/// Any fractional minute component is discarded.
	pub fn total_minutes(&self) -> i64 {
		self.nanos / (60 * 1_000_000_000)
	}
	
	/// Returns the total duration in hours with truncation.
	///
	/// Any fractional hour component is discarded.
	pub fn total_hours(&self) -> i64 {
		self.nanos / (60 * 60 * 1_000_000_000)
	}
	
	/// Returns the absolute value of this duration.
	///
	/// Negative durations become positive, positive durations remain unchanged.
	pub fn abs(&self) -> Self {
		Self { nanos: self.nanos.abs() }
	}
	
	/// Returns the arithmetic negation of this duration.
	///
	/// Positive durations become negative and vice versa.
	pub fn negate(&self) -> Self {
		Self { nanos: -self.nanos }
	}
	
	/// Adds another duration to this one.
	///
	/// # Arguments
	///
	/// * `other` - Duration to add to this one
	pub fn plus(&self, other: &Self) -> Self {
		Self { nanos: self.nanos + other.nanos }
	}
	
	/// Subtracts another duration from this one.
	///
	/// # Arguments
	///
	/// * `other` - Duration to subtract from this one
	pub fn minus(&self, other: &Self) -> Self {
		Self { nanos: self.nanos - other.nanos }
	}
	
	/// Multiplies this duration by a scalar factor.
	///
	/// # Arguments
	///
	/// * `factor` - Multiplication factor
	pub fn multiply_by(&self, factor: i64) -> Self {
		Self { nanos: self.nanos * factor }
	}
	
	/// Divides this duration by a scalar divisor.
	///
	/// # Arguments
	///
	/// * `divisor` - Division factor
	///
	/// # Returns
	///
	/// Returns `Ok(ClockDuration)` if the divisor is non-zero, otherwise returns
	/// an error.
	pub fn divide_by(&self, divisor: i64) -> Outcome<Self> {
		if divisor == 0 {
			return Err(err!("Cannot divide duration by zero"; Invalid, Input));
		}
		Ok(Self { nanos: self.nanos / divisor })
	}
	
	/// Returns the duration decomposed into time components.
	///
	/// The duration is broken down into hours, minutes, seconds, and nanoseconds.
	/// For negative durations, the sign is carried in the first non-zero component
	/// to maintain a clear representation.
	///
	/// # Returns
	///
	/// Returns a tuple containing (hours, minutes, seconds, nanoseconds) where
	/// the sign appears in the first non-zero component.
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