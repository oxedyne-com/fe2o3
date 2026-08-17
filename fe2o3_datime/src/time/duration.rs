//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use crate::core::Duration;
use oxedyne_fe2o3_core::prelude::*;

#[derive(Clone, Debug, PartialEq)]
pub struct CalClockDuration {
	days: i32,
	nanos: i64,
}

impl CalClockDuration {
	pub fn new(days: i32, nanos: i64) -> Self {
		Self { days, nanos }
	}
	
	pub fn from_days(days: i32) -> Self {
		Self { days, nanos: 0 }
	}
	
	pub fn from_nanos(nanos: i64) -> Self {
		Self { days: 0, nanos }
	}
	
	pub fn days(&self) -> i32 {
		self.days
	}
	
	pub fn nanoseconds(&self) -> i64 {
		self.nanos
	}
	
	pub fn from_millis(millis: i64) -> Outcome<Self> {
		let nanos = millis * 1_000_000;
		Ok(Self::from_nanos(nanos))
	}
	
	pub fn from_seconds(seconds: i64) -> Self {
		let nanos = seconds * 1_000_000_000;
		Self::from_nanos(nanos)
	}
	
	pub fn from_minutes(minutes: i64) -> Self {
		let nanos = minutes * 60 * 1_000_000_000;
		Self::from_nanos(nanos)
	}
	
	pub fn from_hours(hours: i64) -> Self {
		let nanos = hours * 60 * 60 * 1_000_000_000;
		Self::from_nanos(nanos)
	}
	
	/// Includes the nanosecond part, not just the days field.
	pub fn total_days(&self) -> i64 {
		const NANOS_PER_DAY: i64 = 24 * 60 * 60 * 1_000_000_000;
		
		let day_nanos = self.days as i64 * NANOS_PER_DAY;
		let total_nanos = day_nanos + self.nanos;
		
		total_nanos / NANOS_PER_DAY
	}
	
	pub fn to_hours(&self) -> Outcome<i64> {
		const NANOS_PER_HOUR: i64 = 60 * 60 * 1_000_000_000;
		let total_nanos = res!(self.to_nanos());
		Ok(total_nanos / NANOS_PER_HOUR)
	}
	
	pub fn to_minutes(&self) -> Outcome<i64> {
		const NANOS_PER_MINUTE: i64 = 60 * 1_000_000_000;
		let total_nanos = res!(self.to_nanos());
		Ok(total_nanos / NANOS_PER_MINUTE)
	}
	
	pub fn add(&self, other: &Self) -> Outcome<Self> {
		Ok(Self {
			days: self.days + other.days,
			nanos: self.nanos + other.nanos,
		})
	}
	
	pub fn subtract(&self, other: &Self) -> Outcome<Self> {
		Ok(Self {
			days: self.days - other.days,
			nanos: self.nanos - other.nanos,
		})
	}
	
	pub fn time_component(&self) -> crate::clock::ClockDuration {
		crate::clock::ClockDuration::from_nanos(self.nanos)
	}
	
	pub fn negate(&self) -> Self {
		Self {
			days: -self.days,
			nanos: -self.nanos,
		}
	}
	
	pub fn total_seconds(&self) -> i64 {
		self.to_seconds().unwrap_or(0)
	}
	
	pub fn divide_by(&self, factor: i32) -> Outcome<Self> {
		if factor == 0 {
			return Err(err!("Cannot divide duration by zero"; Invalid, Input));
		}
		
		let total_nanos = self.days as i64 * 24 * 60 * 60 * 1_000_000_000 + self.nanos;
		let divided_nanos = total_nanos / factor as i64;
		
		let new_days = (divided_nanos / (24 * 60 * 60 * 1_000_000_000)) as i32;
		let remaining_nanos = divided_nanos % (24 * 60 * 60 * 1_000_000_000);
		
		Ok(Self {
			days: new_days,
			nanos: remaining_nanos,
		})
	}
}

impl Duration for CalClockDuration {
	fn to_nanos(&self) -> Outcome<i64> {
		let day_nanos = self.days as i64 * 24 * 60 * 60 * 1_000_000_000;
		Ok(day_nanos + self.nanos)
	}
	
	fn to_seconds(&self) -> Outcome<i64> {
		let total_nanos = res!(self.to_nanos());
		Ok(total_nanos / 1_000_000_000)
	}
	
	fn to_days(&self) -> Outcome<i32> {
		Ok(self.days)
	}
	
	fn is_negative(&self) -> bool {
		self.days < 0 || (self.days == 0 && self.nanos < 0)
	}
}

impl std::fmt::Display for CalClockDuration {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		if self.days != 0 {
			write!(f, "{}d {}ns", self.days, self.nanos)
		} else {
			write!(f, "{}ns", self.nanos)
		}
	}
}