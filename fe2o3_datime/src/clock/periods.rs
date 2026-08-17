//! [Written entirely with AI](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

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

use oxedyne_fe2o3_core::prelude::*;

#[derive(Clone, Debug, PartialEq)]
pub struct HourPeriod {
	date:	CalendarDate,
	hour:	ClockHour,
}

impl HourPeriod {
	pub fn new(date: CalendarDate, hour: ClockHour) -> Self {
		Self { date, hour }
	}
	
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
	
	pub fn date(&self) -> &CalendarDate {
		&self.date
	}
	
	pub fn hour(&self) -> ClockHour {
		self.hour
	}
	
	pub fn start_time(&self) -> Outcome<ClockTime> {
		ClockTime::new(self.hour.of(), 0, 0, 0, self.date.zone().clone())
	}
	
	pub fn end_time(&self) -> Outcome<ClockTime> {
		let next_hour = self.hour.add_hours(1);
		ClockTime::new(next_hour.of(), 0, 0, 0, self.date.zone().clone())
	}
	
	pub fn to_interval(&self) -> Outcome<ClockInterval> {
		let start = res!(self.start_time());
		let end = res!(self.end_time());
		ClockInterval::new(start, end)
	}
}

#[derive(Clone, Debug, PartialEq)]
pub struct MinutePeriod {
	date:	CalendarDate,
	hour:	ClockHour,
	minute:	ClockMinute,
}

impl MinutePeriod {
	pub fn new(date: CalendarDate, hour: ClockHour, minute: ClockMinute) -> Self {
		Self { date, hour, minute }
	}
	
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
	
	pub fn date(&self) -> &CalendarDate {
		&self.date
	}
	
	pub fn hour(&self) -> ClockHour {
		self.hour
	}
	
	pub fn minute(&self) -> ClockMinute {
		self.minute
	}
	
	pub fn start_time(&self) -> Outcome<ClockTime> {
		ClockTime::new(self.hour.of(), self.minute.of(), 0, 0, self.date.zone().clone())
	}
	
	pub fn end_time(&self) -> Outcome<ClockTime> {
		let (next_minute, hour_carry) = self.minute.add_minutes(1);
		let next_hour = if hour_carry > 0 {
			self.hour.add_hours(hour_carry)
		} else {
			self.hour
		};
		ClockTime::new(next_hour.of(), next_minute.of(), 0, 0, self.date.zone().clone())
	}
	
	pub fn to_interval(&self) -> Outcome<ClockInterval> {
		let start = res!(self.start_time());
		let end = res!(self.end_time());
		ClockInterval::new(start, end)
	}
}

#[derive(Clone, Debug, PartialEq)]
pub struct SecondPeriod {
	date:	CalendarDate,
	hour:	ClockHour,
	minute:	ClockMinute,
	second:	ClockSecond,
}

impl SecondPeriod {
	pub fn new(
		date: CalendarDate, 
		hour: ClockHour, 
		minute: ClockMinute, 
		second: ClockSecond
	) -> Self {
		Self { date, hour, minute, second }
	}
	
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
	
	pub fn date(&self) -> &CalendarDate {
		&self.date
	}
	
	pub fn hour(&self) -> ClockHour {
		self.hour
	}
	
	pub fn minute(&self) -> ClockMinute {
		self.minute
	}
	
	pub fn second(&self) -> ClockSecond {
		self.second
	}
	
	pub fn start_time(&self) -> Outcome<ClockTime> {
		ClockTime::new(
			self.hour.of(), 
			self.minute.of(), 
			self.second.of(), 
			0, 
			self.date.zone().clone()
		)
	}
	
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