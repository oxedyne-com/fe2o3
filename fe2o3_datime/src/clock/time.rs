use crate::{
	core::Time,
	clock::{
		ClockHour,
		ClockMinute,
		ClockSecond,
		ClockNanoSecond,
		ClockFields,
		ClockDuration,
	},
	time::CalClockZone,
};

use oxedyne_fe2o3_core::prelude::*;

use std::fmt;

/// Represents a time of day with nanosecond precision.
///
/// This type provides immutable time-of-day representation supporting times from
/// 00:00:00.000000000 through 24:00:00.000000000 (inclusive). The value 24:00:00.000000000
/// represents the end of day and is equivalent to 00:00:00.000000000 of the following day.
///
/// All arithmetic operations return new instances, maintaining immutability. Time zone
/// information is preserved but time arithmetic is performed in a time zone agnostic manner
/// since no date context is available.
///
/// # Examples
///
/// ```ignore
/// use oxedyne_fe2o3_datime::{
///     clock::ClockTime,
///     time::CalClockZone,
/// }res!();
///
/// let zone = CalClockZone::utc()res!();
/// let time = ClockTime::new(14, 30, 45, 123_456_789, zone)?res!();
/// assert_eq!(time.hour().of(), 14)res!();
/// assert_eq!(time.to_twelve_hour_string(), "2:30:45 PM")res!();
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClockTime {
	hour:		ClockHour,
	minute:		ClockMinute,
	second:		ClockSecond,
	nanosecond:	ClockNanoSecond,
	zone:		CalClockZone,
}

impl ClockTime {
	/// Creates a new ClockTime from individual components.
	///
	/// # Arguments
	///
	/// * `hour` - Hour of day (0-23)
	/// * `minute` - Minute within hour (0-59)
	/// * `second` - Second within minute (0-59, leap seconds not supported in ClockTime)
	/// * `nanosecond` - Nanosecond within second (0-999,999,999)
	/// * `zone` - Time zone for this time
	///
	/// # Returns
	///
	/// Returns `Ok(ClockTime)` if all components are valid, otherwise returns an error
	/// describing the invalid component.
	pub fn new(
		hour: u8,
		minute: u8,
		second: u8,
		nanosecond: u32,
		zone: CalClockZone,
	) -> Outcome<Self> {
		// ClockTime uses stricter validation than individual components
		// Only hours 0-23 are valid for ClockTime (not 24)
		if hour >= 24 {
			return Err(err!(
				"Hour {} is invalid for ClockTime, must be 0-23", 
				hour; 
				Invalid, Input
			));
		}
		if minute >= 60 {
			return Err(err!(
				"Minute {} is invalid, must be 0-59", 
				minute; 
				Invalid, Input
			));
		}
		if second >= 60 {
			return Err(err!(
				"Second {} is invalid, must be 0-59", 
				second; 
				Invalid, Input
			));
		}
		
		Ok(Self {
			hour:		res!(ClockHour::new(hour)),
			minute:		res!(ClockMinute::new(minute)),
			second:		res!(ClockSecond::new(second)),
			nanosecond:	res!(ClockNanoSecond::new(nanosecond)),
			zone,
		})
	}
	
	/// Creates a new ClockTime from pre-validated components.
	///
	/// This method accepts component objects that have already been validated,
	/// avoiding the need for additional validation.
	pub fn from_components(
		hour: ClockHour,
		minute: ClockMinute,
		second: ClockSecond,
		nanosecond: ClockNanoSecond,
		zone: CalClockZone,
	) -> Self {
		Self { hour, minute, second, nanosecond, zone }
	}
	
	/// Creates a ClockTime representing midnight (00:00:00.000000000).
	///
	/// This is a convenience method for creating the start of day.
	pub fn midnight(zone: CalClockZone) -> Outcome<Self> {
		Self::new(0, 0, 0, 0, zone)
	}
	
	/// Creates a ClockTime representing noon (12:00:00.000000000).
	///
	/// This is a convenience method for creating the middle of day.
	pub fn noon(zone: CalClockZone) -> Outcome<Self> {
		Self::new(12, 0, 0, 0, zone)
	}
	
	/// Creates a ClockTime representing end of day (24:00:00.000000000).
	///
	/// This represents the conceptual end of day, equivalent to 00:00:00.000000000 of the
	/// following day. This value is primarily useful for interval endpoints.
	pub fn end_of_day(zone: CalClockZone) -> Outcome<Self> {
		// Special case: create end-of-day time with hour 24
		// This bypasses normal validation since 24:00:00 is conceptually valid as end-of-day
		Ok(Self {
			hour:		res!(ClockHour::new(24)), // ClockHour allows 24
			minute:		res!(ClockMinute::new(0)),
			second:		res!(ClockSecond::new(0)),
			nanosecond:	res!(ClockNanoSecond::new(0)),
			zone,
		})
	}
	
	/// Creates a ClockTime from total nanoseconds elapsed since midnight.\n\t///\n\t/// # Arguments\n\t///\n\t/// * `nanos` - Nanoseconds since midnight (0 to just under 24 hours)\n\t/// * `zone` - Time zone for the resulting time\n\t///\n\t/// # Returns\n\t///\n\t/// Returns `Ok(ClockTime)` if the nanosecond count represents a valid time\n\t/// within a single day, otherwise returns an error."
	pub fn from_nanos_of_day(nanos: u64, zone: CalClockZone) -> Outcome<Self> {
		const NANOS_PER_DAY: u64 = 24 * 60 * 60 * 1_000_000_000;
		
		if nanos >= NANOS_PER_DAY {
			return Err(err!(
				"Nanoseconds {} exceed maximum for one day {}", 
				nanos, 
				NANOS_PER_DAY; 
				Invalid, Input
			));
		}
		
		let hours = nanos / (60 * 60 * 1_000_000_000);
		let remaining = nanos % (60 * 60 * 1_000_000_000);
		
		let minutes = remaining / (60 * 1_000_000_000);
		let remaining = remaining % (60 * 1_000_000_000);
		
		let seconds = remaining / 1_000_000_000;
		let nanoseconds = remaining % 1_000_000_000;
		
		Self::new(
			hours as u8,
			minutes as u8,
			seconds as u8,
			nanoseconds as u32,
			zone,
		)
	}
	
	/// Returns the hour component.
	pub fn hour(&self) -> ClockHour {
		self.hour
	}
	
	/// Returns the minute component.
	pub fn minute(&self) -> ClockMinute {
		self.minute
	}
	
	/// Returns the second component.
	pub fn second(&self) -> ClockSecond {
		self.second
	}
	
	/// Returns the nanosecond component.
	pub fn nanosecond(&self) -> ClockNanoSecond {
		self.nanosecond
	}
	
	/// Returns the time zone.
	pub fn zone(&self) -> &CalClockZone {
		&self.zone
	}
	
	/// Converts to total nanoseconds since midnight.
	pub fn to_nanos_of_day(&self) -> u64 {
		let hour_nanos = self.hour.of() as u64 * ClockHour::NANOS_PER_HOUR;
		let minute_nanos = self.minute.of() as u64 * ClockMinute::NANOS_PER_MINUTE;
		let second_nanos = self.second.of() as u64 * ClockSecond::NANOS_PER_SECOND;
		let nanos = self.nanosecond.of() as u64;
		
		hour_nanos + minute_nanos + second_nanos + nanos
	}
	
	/// Alias for to_nanos_of_day() for API compatibility.
	pub fn nanos_of_day(&self) -> u64 {
		self.to_nanos_of_day()
	}
	
	/// Converts to total milliseconds since midnight.
	pub fn millis_of_day(&self) -> u32 {
		let nanos = self.to_nanos_of_day();
		(nanos / 1_000_000) as u32
	}
	
	/// Creates a ClockTime from total milliseconds elapsed since midnight.
	///
	/// # Arguments
	///
	/// * `millis` - Milliseconds since midnight (0 to just under 24 hours)
	/// * `zone` - Time zone for the resulting time
	///
	/// # Returns
	///
	/// Returns `Ok(ClockTime)` if the millisecond count represents a valid time
	/// within a single day, otherwise returns an error.
	pub fn from_millis_of_day(millis: u32, zone: CalClockZone) -> Outcome<Self> {
		const MILLIS_PER_DAY: u32 = 24 * 60 * 60 * 1_000;
		
		if millis >= MILLIS_PER_DAY {
			return Err(err!(
				"Milliseconds {} exceed maximum for one day {}", 
				millis, 
				MILLIS_PER_DAY; 
				Invalid, Input
			));
		}
		
		// Convert to nanoseconds and use existing method
		let nanos = millis as u64 * 1_000_000;
		Self::from_nanos_of_day(nanos, zone)
	}
	
	/// Returns true if this represents a valid time of day (hour < 24).
	pub fn is_valid_day_time(&self) -> bool {
		self.hour.is_valid_day_hour()
	}
	
	/// Returns true if this represents end of day (24:00:00.000000000).
	pub fn is_end_of_day(&self) -> bool {
		self.hour.is_end_of_day() &&
		self.minute.of() == 0 &&
		self.second.of() == 0 &&
		self.nanosecond.of() == 0
	}
	
	/// Returns true if this time is before another time.
	pub fn is_before(&self, other: &Self) -> bool {
		self.to_nanos_of_day() < other.to_nanos_of_day()
	}
	
	/// Returns true if this time is after another time.
	pub fn is_after(&self, other: &Self) -> bool {
		self.to_nanos_of_day() > other.to_nanos_of_day()
	}
	
	/// Returns true if this time is before or equal to another time.
	pub fn or_earlier(&self, other: &Self) -> bool {
		self.to_nanos_of_day() <= other.to_nanos_of_day()
	}
	
	/// Returns true if this time is after or equal to another time.
	pub fn or_later(&self, other: &Self) -> bool {
		self.to_nanos_of_day() >= other.to_nanos_of_day()
	}
	
	/// Adds a duration, returning (new_time, day_overflow).
	pub fn plus(&self, duration: &ClockDuration) -> Outcome<(Self, i32)> {
		let mut fields = ClockFields::from_time(
			self.hour.of(),
			self.minute.of(),
			self.second.of(),
			self.nanosecond.of(),
		);
		
		// Add duration nanoseconds
		fields.nanosecond += duration.total_nanos() as i64;
		
		// Normalize and extract components
		let (hour, minute, second, nanosecond, day_carry) = res!(fields
			.to_time_components()
			.ok_or_else(|| err!("Time arithmetic resulted in invalid time"; Invalid)));
		
		let new_time = res!(Self::new(hour, minute, second, nanosecond, self.zone.clone()));
		Ok((new_time, day_carry))
	}
	
	/// Subtracts a duration, returning (new_time, day_underflow).
	pub fn minus(&self, duration: &ClockDuration) -> Outcome<(Self, i32)> {
		let mut fields = ClockFields::from_time(
			self.hour.of(),
			self.minute.of(),
			self.second.of(),
			self.nanosecond.of(),
		);
		
		// Subtract duration nanoseconds
		fields.nanosecond -= duration.total_nanos() as i64;
		
		// Normalize and extract components
		let (hour, minute, second, nanosecond, day_carry) = res!(fields
			.to_time_components()
			.ok_or_else(|| err!("Time arithmetic resulted in invalid time"; Invalid)));
		
		let new_time = res!(Self::new(hour, minute, second, nanosecond, self.zone.clone()));
		Ok((new_time, day_carry))
	}
	
	/// Calculates the duration between this time and another.
	pub fn duration_until(&self, other: &Self) -> ClockDuration {
		let self_nanos = self.to_nanos_of_day() as i64;
		let other_nanos = other.to_nanos_of_day() as i64;
		let diff_nanos = other_nanos - self_nanos;
		
		ClockDuration::from_nanos(diff_nanos)
	}
	
	/// Converts to 12-hour format string (e.g., "3:45:30 PM").
	pub fn to_twelve_hour_string(&self) -> String {
		let (hour, is_pm) = self.hour.to_twelve_hour();
		let ampm = if is_pm { "PM" } else { "AM" };
		fmt!("{}:{:02}:{:02} {}", hour, self.minute.of(), self.second.of(), ampm)
	}
	
	/// Converts to 24-hour format string (e.g., "15:45:30").
	pub fn to_twenty_four_hour_string(&self) -> String {
		fmt!("{}:{}:{}", self.hour, self.minute, self.second)
	}
	
	/// Converts to ISO 8601 time format with nanoseconds (e.g., "15:45:30.123456789").
	pub fn to_iso_string(&self) -> String {
		if self.nanosecond.of() == 0 {
			fmt!("{}:{}:{}", self.hour, self.minute, self.second)
		} else {
			fmt!("{}:{}:{}.{}", self.hour, self.minute, self.second, self.nanosecond)
		}
	}
}

impl Time for ClockTime {
	fn get_zone(&self) -> &CalClockZone {
		&self.zone
	}
	
	fn to_zone(&self, new_zone: CalClockZone) -> Outcome<Self> {
		// For ClockTime, this is administrative conversion
		// (no actual time transformation since we don't have date context)
		Ok(Self {
			hour: self.hour,
			minute: self.minute,
			second: self.second,
			nanosecond: self.nanosecond,
			zone: new_zone,
		})
	}
	
	fn format(&self, stencil: &str) -> String {
		// Simplified formatting implementation
		// TODO: Implement full stencil-based formatting
		match stencil {
			"HH:mm:ss" => self.to_twenty_four_hour_string(),
			"hh:mm:ss a" => self.to_twelve_hour_string(),
			"HH:mm:ss.nnnnnnnnn" => self.to_iso_string(),
			_ => self.to_iso_string(), // fallback
		}
	}
	
	fn is_recognised_format_char(&self, c: char) -> bool {
		matches!(c, 'H' | 'h' | 'm' | 's' | 'n' | 'a')
	}
	
	fn is_before(&self, other: &Self) -> bool {
		self.to_nanos_of_day() < other.to_nanos_of_day()
	}
	
	fn is_after(&self, other: &Self) -> bool {
		self.to_nanos_of_day() > other.to_nanos_of_day()
	}
	
	fn or_earlier(&self, other: &Self) -> Self {
		if self.to_nanos_of_day() <= other.to_nanos_of_day() {
			self.clone()
		} else {
			other.clone()
		}
	}
	
	fn or_later(&self, other: &Self) -> Self {
		if self.to_nanos_of_day() >= other.to_nanos_of_day() {
			self.clone()
		} else {
			other.clone()
		}
	}
}

// Validation methods
impl ClockTime {
	/// Returns true if all components are valid.
	pub fn is_valid(&self) -> bool {
		self.hour.of() <= 24 &&
		self.minute.of() <= 60 &&
		self.second.of() <= 60 &&
		self.nanosecond.of() <= 999_999_999
	}
	
	/// Adds a ClockDuration to this time, ignoring day overflow.
	///
	/// This method is primarily used by higher-level types like CalClock
	/// that handle day overflow separately.
	pub fn add_duration(&self, duration: &ClockDuration) -> Outcome<Self> {
		let (new_time, _day_overflow) = res!(self.plus(duration));
		Ok(new_time)
	}
	
	/// Subtracts a ClockDuration from this time, ignoring day underflow.
	///
	/// This method is primarily used by higher-level types like CalClock
	/// that handle day underflow separately.
	pub fn subtract_duration(&self, duration: &ClockDuration) -> Outcome<Self> {
		let (new_time, _day_underflow) = res!(self.minus(duration));
		Ok(new_time)
	}
}

impl PartialOrd for ClockTime {
	fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
		Some(self.cmp(other))
	}
}

impl Ord for ClockTime {
	fn cmp(&self, other: &Self) -> std::cmp::Ordering {
		self.to_nanos_of_day().cmp(&other.to_nanos_of_day())
	}
}

impl fmt::Display for ClockTime {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "{}", self.to_iso_string())
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn test_zone() -> CalClockZone {
		CalClockZone::utc()
	}

	#[test]
	fn test_clock_time_creation() {
		let time = ClockTime::new(14, 30, 45, 123_456_789, test_zone());
		assert!(time.is_ok());
		
		let time = time.unwrap();
		assert_eq!(time.hour().of(), 14);
		assert_eq!(time.minute().of(), 30);
		assert_eq!(time.second().of(), 45);
		assert_eq!(time.nanosecond().of(), 123_456_789);
	}

	#[test]
	fn test_special_times() {
		let midnight = ClockTime::midnight(test_zone()).unwrap();
		assert_eq!(midnight.hour().of(), 0);
		assert!(midnight.is_valid_day_time());
		
		let noon = ClockTime::noon(test_zone()).unwrap();
		assert_eq!(noon.hour().of(), 12);
		
		let eod = ClockTime::end_of_day(test_zone()).unwrap();
		assert_eq!(eod.hour().of(), 24);
		assert!(eod.is_end_of_day());
		assert!(!eod.is_valid_day_time());
	}

	#[test]
	fn test_nanos_of_day_conversion() {
		let time = ClockTime::new(12, 30, 45, 123_456_789, test_zone()).unwrap();
		let nanos = time.to_nanos_of_day();
		
		let restored = ClockTime::from_nanos_of_day(nanos, test_zone()).unwrap();
		assert_eq!(time, restored);
	}

	#[test]
	fn test_time_comparison() {
		let time1 = ClockTime::new(10, 30, 0, 0, test_zone()).unwrap();
		let time2 = ClockTime::new(14, 30, 0, 0, test_zone()).unwrap();
		
		assert!(time1.is_before(&time2));
		assert!(time2.is_after(&time1));
		assert!(time1.or_earlier(&time2));
		assert!(time2.or_later(&time1));
		
		assert!(!time1.is_after(&time2));
		assert!(!time2.is_before(&time1));
	}

	#[test]
	fn test_twelve_hour_format() {
		let time = ClockTime::new(14, 30, 45, 0, test_zone()).unwrap();
		assert_eq!(time.to_twelve_hour_string(), "2:30:45 PM");
		
		let time = ClockTime::new(0, 15, 30, 0, test_zone()).unwrap();
		assert_eq!(time.to_twelve_hour_string(), "12:15:30 AM");
	}

	#[test]
	fn test_iso_format() {
		let time = ClockTime::new(14, 30, 45, 0, test_zone()).unwrap();
		assert_eq!(time.to_iso_string(), "14:30:45");
		
		let time = ClockTime::new(14, 30, 45, 123_456_789, test_zone()).unwrap();
		assert_eq!(time.to_iso_string(), "14:30:45.123456789");
	}
}