use oxedyne_fe2o3_core::prelude::*;

/// Mutable container for clock arithmetic operations.
///
/// ClockFields provides a working space for performing time arithmetic operations
/// that may require normalisation across multiple time units. This type handles
/// carry and borrow operations between nanoseconds, seconds, minutes, hours, and
/// days, ensuring that intermediate calculations maintain mathematical correctness.
///
/// # Usage
///
/// This type is primarily used internally by ClockTime and related types for
/// implementing arithmetic operations such as addition and subtraction of durations.
/// The mutable design allows for efficient in-place normalisation without
/// requiring multiple allocations.
///
/// # Normalisation
///
/// The normalise() method ensures that all fields are within their valid ranges:
/// - nanoseconds: 0-999,999,999
/// - seconds: 0-59
/// - minutes: 0-59
/// - hours: 0-23
/// - day_carry: accumulated overflow days
#[derive(Clone, Debug, Default)]
pub struct ClockFields {
	pub hour:		i64,
	pub minute:		i64,
	pub second:		i64,
	pub nanosecond:	i64,
	pub day_carry:	i32,
}

impl ClockFields {
	/// Creates a new ClockFields with all fields initialised to zero.
	///
	/// This provides a neutral starting point for time arithmetic operations.
	pub fn new() -> Self {
		Self::default()
	}
	
	/// Creates ClockFields from individual time components.
	///
	/// # Arguments
	///
	/// * `hour` - Hour component (0-24)
	/// * `minute` - Minute component (0-60)
	/// * `second` - Second component (0-60)
	/// * `nanosecond` - Nanosecond component (0-999,999,999)
	pub fn from_time(hour: u8, minute: u8, second: u8, nanosecond: u32) -> Self {
		Self {
			hour:		hour as i64,
			minute:		minute as i64,
			second:		second as i64,
			nanosecond:	nanosecond as i64,
			day_carry:	0,
		}
	}
	
	/// Normalises the fields, handling carry and borrow between time units.
	///
	/// This method ensures that all time components are within their valid ranges
	/// by propagating overflow and underflow between adjacent units. The process
	/// continues until all components are properly normalised.
	///
	/// # Returns
	///
	/// Returns `true` if any normalisation was performed, `false` if all fields
	/// were already within valid ranges.
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
	
	/// Adds another ClockFields to this one.
	///
	/// This performs component-wise addition without normalisation.
	/// Call normalise() afterwards if needed.
	///
	/// # Arguments
	///
	/// * `other` - ClockFields to add to this one
	pub fn add(&mut self, other: &ClockFields) {
		self.hour += other.hour;
		self.minute += other.minute;
		self.second += other.second;
		self.nanosecond += other.nanosecond;
		self.day_carry += other.day_carry;
	}
	
	/// Subtracts another ClockFields from this one.
	///
	/// This performs component-wise subtraction without normalisation.
	/// Call normalise() afterwards if needed.
	///
	/// # Arguments
	///
	/// * `other` - ClockFields to subtract from this one
	pub fn subtract(&mut self, other: &ClockFields) {
		self.hour -= other.hour;
		self.minute -= other.minute;
		self.second -= other.second;
		self.nanosecond -= other.nanosecond;
		self.day_carry -= other.day_carry;
	}
	
	/// Multiplies all time fields by a scalar factor.
	///
	/// The day_carry field is not multiplied as it represents derived overflow
	/// from the normalisation process.
	///
	/// # Arguments
	///
	/// * `factor` - Multiplication factor
	pub fn multiply(&mut self, factor: i64) {
		self.hour *= factor;
		self.minute *= factor;
		self.second *= factor;
		self.nanosecond *= factor;
		// day_carry is not multiplied as it's derived from normalization
	}
	
	/// Converts to individual time components after normalisation.
	///
	/// This method first normalises the fields, then attempts to convert them
	/// to individual time components. If any component remains outside its
	/// valid range after normalisation, None is returned.
	///
	/// # Returns
	///
	/// Returns `Some((hour, minute, second, nanosecond, day_carry))` if all
	/// components are valid after normalisation, otherwise returns `None`.
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
	
	/// Checks if all fields represent a valid time after normalisation.
	///
	/// This is a convenience method equivalent to checking if
	/// `to_time_components()` returns `Some`.
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