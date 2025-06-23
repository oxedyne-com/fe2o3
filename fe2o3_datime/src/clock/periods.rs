use crate::{
	calendar::{Calendar, CalendarDate},
	clock::{
		ClockHour,
		ClockMinute,
		ClockSecond,
		ClockTime,
		ClockInterval,
	},
	time::CalClockZone,
};

use oxedize_fe2o3_core::prelude::*;

/// Represents a specific hour within a specific day.
#[derive(Clone, Debug, PartialEq)]
pub struct HourPeriod {
	date:	CalendarDate,
	hour:	ClockHour,
}

impl HourPeriod {
	/// Creates a new HourPeriod.
	pub fn new(date: CalendarDate, hour: ClockHour) -> Self {
		Self { date, hour }
	}
	
	/// Creates a HourPeriod from components.
	pub fn from_components(
		year: i32,
		month: u8,
		day: u8,
		hour: u8,
		zone: CalClockZone,
	) -> Outcome<Self> {
		let calendar = Calendar::new(); // Default to Gregorian
		let date = res!(calendar.date(year, month, day, zone));
		let hour = res!(ClockHour::new(hour));
		Ok(Self::new(date, hour))
	}
	
	/// Returns the calendar date.
	pub fn date(&self) -> &CalendarDate {
		&self.date
	}
	
	/// Returns the hour.
	pub fn hour(&self) -> ClockHour {
		self.hour
	}
	
	/// Returns the start time of this hour.
	pub fn start_time(&self) -> Outcome<ClockTime> {
		ClockTime::new(self.hour.of(), 0, 0, 0, self.date.zone().clone())
	}
	
	/// Returns the end time of this hour (start of next hour).
	pub fn end_time(&self) -> Outcome<ClockTime> {
		let next_hour = self.hour.add_hours(1);
		ClockTime::new(next_hour.of(), 0, 0, 0, self.date.zone().clone())
	}
	
	/// Returns this hour period as a time interval.
	pub fn to_interval(&self) -> Outcome<ClockInterval> {
		let start = res!(self.start_time());
		let end = res!(self.end_time());
		ClockInterval::new(start, end)
	}
}

/// Represents a specific minute within a specific day.
#[derive(Clone, Debug, PartialEq)]
pub struct MinutePeriod {
	date:	CalendarDate,
	hour:	ClockHour,
	minute:	ClockMinute,
}

impl MinutePeriod {
	/// Creates a new MinutePeriod.
	pub fn new(date: CalendarDate, hour: ClockHour, minute: ClockMinute) -> Self {
		Self { date, hour, minute }
	}
	
	/// Creates a MinutePeriod from components.
	pub fn from_components(
		year: i32,
		month: u8,
		day: u8,
		hour: u8,
		minute: u8,
		zone: CalClockZone,
	) -> Outcome<Self> {
		let calendar = Calendar::new(); // Default to Gregorian
		let date = res!(calendar.date(year, month, day, zone));
		let hour = res!(ClockHour::new(hour));
		let minute = res!(ClockMinute::new(minute));
		Ok(Self::new(date, hour, minute))
	}
	
	/// Returns the calendar date.
	pub fn date(&self) -> &CalendarDate {
		&self.date
	}
	
	/// Returns the hour.
	pub fn hour(&self) -> ClockHour {
		self.hour
	}
	
	/// Returns the minute.
	pub fn minute(&self) -> ClockMinute {
		self.minute
	}
	
	/// Returns the start time of this minute.
	pub fn start_time(&self) -> Outcome<ClockTime> {
		ClockTime::new(self.hour.of(), self.minute.of(), 0, 0, self.date.zone().clone())
	}
	
	/// Returns the end time of this minute (start of next minute).
	pub fn end_time(&self) -> Outcome<ClockTime> {
		let (next_minute, hour_carry) = self.minute.add_minutes(1);
		let next_hour = if hour_carry > 0 {
			self.hour.add_hours(hour_carry)
		} else {
			self.hour
		};
		ClockTime::new(next_hour.of(), next_minute.of(), 0, 0, self.date.zone().clone())
	}
	
	/// Returns this minute period as a time interval.
	pub fn to_interval(&self) -> Outcome<ClockInterval> {
		let start = res!(self.start_time());
		let end = res!(self.end_time());
		ClockInterval::new(start, end)
	}
}

/// Represents a specific second within a specific day.
#[derive(Clone, Debug, PartialEq)]
pub struct SecondPeriod {
	date:	CalendarDate,
	hour:	ClockHour,
	minute:	ClockMinute,
	second:	ClockSecond,
}

impl SecondPeriod {
	/// Creates a new SecondPeriod.
	pub fn new(
		date: CalendarDate, 
		hour: ClockHour, 
		minute: ClockMinute, 
		second: ClockSecond
	) -> Self {
		Self { date, hour, minute, second }
	}
	
	/// Creates a SecondPeriod from components.
	pub fn from_components(
		year: i32,
		month: u8,
		day: u8,
		hour: u8,
		minute: u8,
		second: u8,
		zone: CalClockZone,
	) -> Outcome<Self> {
		let calendar = Calendar::new(); // Default to Gregorian
		let date = res!(calendar.date(year, month, day, zone));
		let hour = res!(ClockHour::new(hour));
		let minute = res!(ClockMinute::new(minute));
		let second = res!(ClockSecond::new(second));
		Ok(Self::new(date, hour, minute, second))
	}
	
	/// Returns the calendar date.
	pub fn date(&self) -> &CalendarDate {
		&self.date
	}
	
	/// Returns the hour.
	pub fn hour(&self) -> ClockHour {
		self.hour
	}
	
	/// Returns the minute.
	pub fn minute(&self) -> ClockMinute {
		self.minute
	}
	
	/// Returns the second.
	pub fn second(&self) -> ClockSecond {
		self.second
	}
	
	/// Returns the start time of this second.
	pub fn start_time(&self) -> Outcome<ClockTime> {
		ClockTime::new(
			self.hour.of(), 
			self.minute.of(), 
			self.second.of(), 
			0, 
			self.date.zone().clone()
		)
	}
	
	/// Returns the end time of this second (start of next second).
	pub fn end_time(&self) -> Outcome<ClockTime> {
		let (next_second, minute_carry) = self.second.add_seconds(1);
		let (next_minute, hour_carry) = if minute_carry > 0 {
			self.minute.add_minutes(minute_carry)
		} else {
			(self.minute, 0)
		};
		let next_hour = if hour_carry > 0 {
			self.hour.add_hours(hour_carry)
		} else {
			self.hour
		};
		
		ClockTime::new(
			next_hour.of(), 
			next_minute.of(), 
			next_second.of(), 
			0, 
			self.date.zone().clone()
		)
	}
	
	/// Returns this second period as a time interval.
	pub fn to_interval(&self) -> Outcome<ClockInterval> {
		let start = res!(self.start_time());
		let end = res!(self.end_time());
		ClockInterval::new(start, end)
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn test_zone() -> CalClockZone {
		CalClockZone::utc()
	}

	#[test]
	fn test_hour_period() {
		let date = CalendarDate::new(2024, 6, 15, test_zone()).unwrap();
		let hour = ClockHour::new(14).unwrap();
		
		let period = HourPeriod::new(date, hour);
		assert_eq!(period.hour().of(), 14);
		
		let start = period.start_time().unwrap();
		assert_eq!(start.hour().of(), 14);
		assert_eq!(start.minute().of(), 0);
		
		let end = period.end_time().unwrap();
		assert_eq!(end.hour().of(), 15);
		assert_eq!(end.minute().of(), 0);
	}

	#[test]
	fn test_minute_period() {
		let period = MinutePeriod::from_components(
			2024, 6, 15, 14, 30, test_zone()
		).unwrap();
		
		assert_eq!(period.hour().of(), 14);
		assert_eq!(period.minute().of(), 30);
		
		let start = period.start_time().unwrap();
		assert_eq!(start.minute().of(), 30);
		assert_eq!(start.second().of(), 0);
		
		let end = period.end_time().unwrap();
		assert_eq!(end.minute().of(), 31);
		assert_eq!(end.second().of(), 0);
	}

	#[test]
	fn test_second_period() {
		let period = SecondPeriod::from_components(
			2024, 6, 15, 14, 30, 45, test_zone()
		).unwrap();
		
		assert_eq!(period.hour().of(), 14);
		assert_eq!(period.minute().of(), 30);
		assert_eq!(period.second().of(), 45);
		
		let start = period.start_time().unwrap();
		assert_eq!(start.second().of(), 45);
		assert_eq!(start.nanosecond().of(), 0);
		
		let end = period.end_time().unwrap();
		assert_eq!(end.second().of(), 46);
		assert_eq!(end.nanosecond().of(), 0);
	}

	#[test]
	fn test_period_intervals() {
		let hour_period = HourPeriod::from_components(
			2024, 6, 15, 14, test_zone()
		).unwrap();
		
		let interval = hour_period.to_interval().unwrap();
		let duration = interval.duration();
		assert_eq!(duration.total_hours(), 1);
	}
}