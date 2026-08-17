//! [Written entirely with AI](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use oxedyne_fe2o3_core::prelude::*;

/// A mutable working space for time arithmetic. Fields may hold out-of-range
/// values until normalise() carries the excess upwards, with whole days landing
/// in day_carry.
#[derive(Clone, Debug, Default)]
pub struct ClockFields {
	pub hour:		i64,
	pub minute:		i64,
	pub second:		i64,
	pub nanosecond:	i64,
	pub day_carry:	i32,
}

impl ClockFields {
	pub fn new() -> Self {
		Self::default()
	}
	
	pub fn from_time(hour: u8, minute: u8, second: u8, nanosecond: u32) -> Self {
		Self {
			hour:		hour as i64,
			minute:		minute as i64,
			second:		second as i64,
			nanosecond:	nanosecond as i64,
			day_carry:	0,
		}
	}
	
	/// Carries overflow upwards until every field is back in range.
	pub fn normalize(&mut self) -> bool {
		let mut changed = false;
		
		// Normalise nanoseconds to seconds.
		if self.nanosecond >= 1_000_000_000 {
			let carry = self.nanosecond / 1_000_000_000;
			self.second += carry;
			self.nanosecond %= 1_000_000_000;
			changed = true;
		} else if self.nanosecond < 0 {
			let borrow = (-self.nanosecond + 999_999_999) / 1_000_000_000;
			self.second -= borrow;
			self.nanosecond += borrow * 1_000_000_000;
			changed = true;
		}
		
		// Normalise seconds to minutes.
		if self.second >= 60 {
			let carry = self.second / 60;
			self.minute += carry;
			self.second %= 60;
			changed = true;
		} else if self.second <= -60 {
			let borrow = (-self.second + 59) / 60;
			self.minute -= borrow;
			self.second += borrow * 60;
			changed = true;
		}
		
		// Normalise minutes to hours.
		if self.minute >= 60 {
			let carry = self.minute / 60;
			self.hour += carry;
			self.minute %= 60;
			changed = true;
		} else if self.minute <= -60 {
			let borrow = (-self.minute + 59) / 60;
			self.hour -= borrow;
			self.minute += borrow * 60;
			changed = true;
		}
		
		// Normalise hours to days.
		if self.hour >= 24 {
			let carry = self.hour / 24;
			self.day_carry += carry as i32;
			self.hour %= 24;
			changed = true;
		} else if self.hour <= -24 {
			let borrow = (-self.hour + 23) / 24;
			self.day_carry -= borrow as i32;
			self.hour += borrow * 24;
			changed = true;
		}
		
		changed
	}
	
	/// Component-wise, and left unnormalised: call normalise afterwards.
	pub fn add(&mut self, other: &ClockFields) {
		self.hour += other.hour;
		self.minute += other.minute;
		self.second += other.second;
		self.nanosecond += other.nanosecond;
		self.day_carry += other.day_carry;
	}
	
	pub fn subtract(&mut self, other: &ClockFields) {
		self.hour -= other.hour;
		self.minute -= other.minute;
		self.second -= other.second;
		self.nanosecond -= other.nanosecond;
		self.day_carry -= other.day_carry;
	}
	
	/// day_carry is left alone, being derived rather than held.
	pub fn multiply(&mut self, factor: i64) {
		self.hour *= factor;
		self.minute *= factor;
		self.second *= factor;
		self.nanosecond *= factor;
		// day_carry is not multiplied as it's derived from normalization
	}
	
	/// None when a field is still out of range after normalisation.
	pub fn to_time_components(&mut self) -> Option<(u8, u8, u8, u32, i32)> {
		self.normalize();
		
		if self.hour < 0 || self.hour > 24 ||
		   self.minute < 0 || self.minute >= 60 ||
		   self.second < 0 || self.second >= 60 ||
		   self.nanosecond < 0 || self.nanosecond >= 1_000_000_000 {
			return None;
		}
		
		Some((
			self.hour as u8,
			self.minute as u8,
			self.second as u8,
			self.nanosecond as u32,
			self.day_carry,
		))
	}
	
	pub fn is_valid_time(&mut self) -> bool {
		self.to_time_components().is_some()
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_normalize_basic() {
		let mut fields = ClockFields::from_time(12, 30, 45, 500_000_000);
		assert!(!fields.normalize()); // no changes needed
		assert_eq!(fields.hour, 12);
		assert_eq!(fields.minute, 30);
		assert_eq!(fields.second, 45);
		assert_eq!(fields.nanosecond, 500_000_000);
	}

	#[test]
	fn test_normalize_overflow() {
		let mut fields = ClockFields {
			hour: 0,
			minute: 0,
			second: 0,
			nanosecond: 2_000_000_000, // 2 seconds
			day_carry: 0,
		};
		assert!(fields.normalize());
		assert_eq!(fields.second, 2);
		assert_eq!(fields.nanosecond, 0);
	}

	#[test]
	fn test_normalize_hour_overflow() {
		let mut fields = ClockFields {
			hour: 25,
			minute: 0,
			second: 0,
			nanosecond: 0,
			day_carry: 0,
		};
		assert!(fields.normalize());
		assert_eq!(fields.hour, 1);
		assert_eq!(fields.day_carry, 1);
	}

	#[test]
	fn test_normalize_underflow() {
		let mut fields = ClockFields {
			hour: 0,
			minute: 0,
			second: 0,
			nanosecond: -500_000_000,
			day_carry: 0,
		};
		assert!(fields.normalize());
		assert_eq!(fields.second, -1);
		assert_eq!(fields.nanosecond, 500_000_000);
	}

	#[test]
	fn test_arithmetic() {
		let mut fields1 = ClockFields::from_time(12, 30, 45, 500_000_000);
		let fields2 = ClockFields::from_time(1, 15, 30, 250_000_000);
		
		fields1.add(&fields2);
		assert!(fields1.normalize());
		assert_eq!(fields1.hour, 13);
		assert_eq!(fields1.minute, 46);
		assert_eq!(fields1.second, 15);
		assert_eq!(fields1.nanosecond, 750_000_000);
	}

	#[test]
	fn test_to_time_components() {
		let mut fields = ClockFields::from_time(23, 59, 59, 999_999_999);
		let result = fields.to_time_components();
		assert!(result.is_some());
		let (h, m, s, n, d) = result.unwrap();
		assert_eq!(h, 23);
		assert_eq!(m, 59);
		assert_eq!(s, 59);
		assert_eq!(n, 999_999_999);
		assert_eq!(d, 0);
	}
}