use oxedyne_fe2o3_core::prelude::*;
use num_bigint::BigInt;
use num_traits::Zero;
use std::str::FromStr;

/// Trait for time-based integer representations with arithmetic operations.
/// 
/// This trait provides the foundation for representing time as abstract
/// numerical values that can be manipulated mathematically.
pub trait TimeInteger: Clone + PartialEq + Eq + PartialOrd + Ord + std::fmt::Debug {
	/// Returns true if this value is zero.
	fn is_zero(&self) -> bool;
	
	/// Returns true if this value is positive.
	fn is_positive(&self) -> bool;
	
	/// Returns the negation of this value.
	fn negate(self) -> Self;
	
	/// Returns this value as an i64 (may truncate for large values).
	fn long_value(&self) -> i64;
	
	/// Returns the number of bytes needed to represent this value.
	fn num_bytes(&self) -> usize;
	
	// Arithmetic operations
	/// Adds another TimeInteger to this one.
	fn add_to(self, other: Self) -> Outcome<Self>;
	
	/// Adds an i64 value to this TimeInteger.
	fn add_to_long(self, other: i64) -> Self;
	
	/// Subtracts another TimeInteger from this one.
	fn subtract_it(self, other: Self) -> Outcome<Self>;
	
	/// Subtracts an i64 value from this TimeInteger.
	fn subtract_it_long(self, other: i64) -> Self;
	
	/// Multiplies this TimeInteger by another.
	fn multiply_by(self, other: Self) -> Outcome<Self>;
	
	/// Multiplies this TimeInteger by an i64 value.
	fn multiply_by_long(self, other: i64) -> Self;
	
	/// Divides this TimeInteger by another.
	fn divide_by(self, other: Self) -> Outcome<Self>;
	
	/// Divides this TimeInteger by an i64 value.
	fn divide_by_long(self, other: i64) -> Self;
	
	/// Returns the remainder when dividing by another TimeInteger.
	fn remainder_by(self, other: Self) -> Outcome<Self>;
	
	/// Returns the remainder when dividing by an i64 value.
	fn remainder_by_long(self, other: i64) -> Self;
	
	// Serialization
	/// Converts to a byte array for file storage.
	fn to_file_byte_array(&self) -> Vec<u8>;
	
	/// Converts to a fixed-size byte array.
	fn to_fixed_byte_array(&self, n: usize) -> Vec<u8>;
	
	/// Converts to a string with comma separators.
	fn to_string_with_commas(&self) -> String;
	
	/// Creates a new instance from bytes.
	fn from_bytes(bytes: &[u8]) -> Outcome<Self> where Self: Sized;
	
	/// Creates a new instance from a string.
	fn from_string(s: &str) -> Outcome<Self> where Self: Sized;
}

/// 64-bit integer implementation of TimeInteger.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TimeLong {
	value: i64,
}

impl TimeLong {
	/// Creates a new TimeLong with the specified value.
	pub fn new(value: i64) -> Self {
		Self { value }
	}
	
	/// Returns the underlying i64 value.
	pub fn value(&self) -> i64 {
		self.value
	}
	
	/// Creates a TimeLong from 8 bytes.
	pub fn from_bytes_array(bytes: &[u8; 8]) -> Self {
		let value = i64::from_be_bytes(*bytes);
		Self::new(value)
	}
	
	/// Converts to 8 bytes.
	pub fn to_bytes_array(&self) -> [u8; 8] {
		self.value.to_be_bytes()
	}
}

impl TimeInteger for TimeLong {
	fn is_zero(&self) -> bool {
		self.value == 0
	}
	
	fn is_positive(&self) -> bool {
		self.value > 0
	}
	
	fn negate(self) -> Self {
		Self::new(-self.value)
	}
	
	fn long_value(&self) -> i64 {
		self.value
	}
	
	fn num_bytes(&self) -> usize {
		8
	}
	
	fn add_to(self, other: Self) -> Outcome<Self> {
		match self.value.checked_add(other.value) {
			Some(result) => Ok(Self::new(result)),
			None => Err(err!("Integer overflow in addition: {} + {}", self.value, other.value; Overflow)),
		}
	}
	
	fn add_to_long(self, other: i64) -> Self {
		Self::new(self.value.saturating_add(other))
	}
	
	fn subtract_it(self, other: Self) -> Outcome<Self> {
		match self.value.checked_sub(other.value) {
			Some(result) => Ok(Self::new(result)),
			None => Err(err!("Integer overflow in subtraction: {} - {}", self.value, other.value; Overflow)),
		}
	}
	
	fn subtract_it_long(self, other: i64) -> Self {
		Self::new(self.value.saturating_sub(other))
	}
	
	fn multiply_by(self, other: Self) -> Outcome<Self> {
		match self.value.checked_mul(other.value) {
			Some(result) => Ok(Self::new(result)),
			None => Err(err!("Integer overflow in multiplication: {} * {}", self.value, other.value; Overflow)),
		}
	}
	
	fn multiply_by_long(self, other: i64) -> Self {
		Self::new(self.value.saturating_mul(other))
	}
	
	fn divide_by(self, other: Self) -> Outcome<Self> {
		if other.value == 0 {
			return Err(err!("Division by zero"; Invalid, Input));
		}
		match self.value.checked_div(other.value) {
			Some(result) => Ok(Self::new(result)),
			None => Err(err!("Integer overflow in division: {} / {}", self.value, other.value; Overflow)),
		}
	}
	
	fn divide_by_long(self, other: i64) -> Self {
		if other == 0 {
			return Self::new(0); // Saturating behavior
		}
		Self::new(self.value / other)
	}
	
	fn remainder_by(self, other: Self) -> Outcome<Self> {
		if other.value == 0 {
			return Err(err!("Modulo by zero"; Invalid, Input));
		}
		Ok(Self::new(self.value % other.value))
	}
	
	fn remainder_by_long(self, other: i64) -> Self {
		if other == 0 {
			return Self::new(0); // Saturating behavior
		}
		Self::new(self.value % other)
	}
	
	fn to_file_byte_array(&self) -> Vec<u8> {
		self.value.to_be_bytes().to_vec()
	}
	
	fn to_fixed_byte_array(&self, n: usize) -> Vec<u8> {
		let mut bytes = self.to_file_byte_array();
		bytes.resize(n, 0);
		bytes
	}
	
	fn to_string_with_commas(&self) -> String {
		let s = self.value.to_string();
		add_commas_to_number(&s)
	}
	
	fn from_bytes(bytes: &[u8]) -> Outcome<Self> {
		if bytes.len() != 8 {
			return Err(err!("TimeLong requires exactly 8 bytes, got {}", bytes.len(); Invalid, Input));
		}
		let mut array = [0u8; 8];
		array.copy_from_slice(bytes);
		Ok(Self::from_bytes_array(&array))
	}
	
	fn from_string(s: &str) -> Outcome<Self> {
		let cleaned = s.replace(",", "");
		match cleaned.parse::<i64>() {
			Ok(value) => Ok(Self::new(value)),
			Err(e) => Err(err!("Failed to parse TimeLong from string '{}': {}", s, e; Invalid, Input)),
		}
	}
}

impl std::fmt::Display for TimeLong {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "{}", self.value)
	}
}

impl From<i64> for TimeLong {
	fn from(value: i64) -> Self {
		Self::new(value)
	}
}

impl From<TimeLong> for i64 {
	fn from(time_long: TimeLong) -> Self {
		time_long.value
	}
}

/// Arbitrary precision integer implementation of TimeInteger.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct TimeBigInt {
	value: BigInt,
}

impl TimeBigInt {
	/// Creates a new TimeBigInt with the specified value.
	pub fn new(value: BigInt) -> Self {
		Self { value }
	}
	
	/// Creates a TimeBigInt from an i64 value.
	pub fn from_i64(value: i64) -> Self {
		Self::new(BigInt::from(value))
	}
	
	/// Returns the underlying BigInt value.
	pub fn value(&self) -> &BigInt {
		&self.value
	}
	
	/// Creates the maximum positive value that can fit in n bytes.
	pub fn max_positive(n_bytes: usize) -> Outcome<Self> {
		if n_bytes == 0 {
			return Err(err!("Cannot create max positive value with 0 bytes"; Invalid, Input));
		}
		
		// Create 2^(8*n_bytes - 1) - 1 (maximum signed value)
		let mut bytes = vec![0x7f]; // First byte: 0111 1111
		bytes.extend(vec![0xff; n_bytes - 1]); // Remaining bytes: 1111 1111
		
		Ok(Self::new(BigInt::from_bytes_be(num_bigint::Sign::Plus, &bytes)))
	}
}

impl TimeInteger for TimeBigInt {
	fn is_zero(&self) -> bool {
		self.value.is_zero()
	}
	
	fn is_positive(&self) -> bool {
		self.value > BigInt::from(0)
	}
	
	fn negate(self) -> Self {
		Self::new(-self.value)
	}
	
	fn long_value(&self) -> i64 {
		// Convert to i64, clamping to avoid overflow
		use num_traits::ToPrimitive;
		self.value.to_i64().unwrap_or_else(|| {
			if self.value > BigInt::from(0) {
				i64::MAX
			} else {
				i64::MIN
			}
		})
	}
	
	fn num_bytes(&self) -> usize {
		let (_, bytes) = self.value.to_bytes_be();
		bytes.len()
	}
	
	fn add_to(self, other: Self) -> Outcome<Self> {
		Ok(Self::new(&self.value + &other.value))
	}
	
	fn add_to_long(self, other: i64) -> Self {
		Self::new(&self.value + BigInt::from(other))
	}
	
	fn subtract_it(self, other: Self) -> Outcome<Self> {
		Ok(Self::new(&self.value - &other.value))
	}
	
	fn subtract_it_long(self, other: i64) -> Self {
		Self::new(&self.value - BigInt::from(other))
	}
	
	fn multiply_by(self, other: Self) -> Outcome<Self> {
		Ok(Self::new(&self.value * &other.value))
	}
	
	fn multiply_by_long(self, other: i64) -> Self {
		Self::new(&self.value * BigInt::from(other))
	}
	
	fn divide_by(self, other: Self) -> Outcome<Self> {
		if other.value.is_zero() {
			return Err(err!("Division by zero"; Invalid, Input));
		}
		Ok(Self::new(&self.value / &other.value))
	}
	
	fn divide_by_long(self, other: i64) -> Self {
		if other == 0 {
			return Self::new(BigInt::from(0)); // Saturating behavior
		}
		Self::new(&self.value / BigInt::from(other))
	}
	
	fn remainder_by(self, other: Self) -> Outcome<Self> {
		if other.value.is_zero() {
			return Err(err!("Modulo by zero"; Invalid, Input));
		}
		Ok(Self::new(&self.value % &other.value))
	}
	
	fn remainder_by_long(self, other: i64) -> Self {
		if other == 0 {
			return Self::new(BigInt::from(0)); // Saturating behavior
		}
		Self::new(&self.value % BigInt::from(other))
	}
	
	fn to_file_byte_array(&self) -> Vec<u8> {
		let (_, bytes) = self.value.to_bytes_be();
		bytes
	}
	
	fn to_fixed_byte_array(&self, n: usize) -> Vec<u8> {
		let mut bytes = self.to_file_byte_array();
		if bytes.len() < n {
			// Pad with zeros at the beginning for big endian
			let mut padded = vec![0; n - bytes.len()];
			padded.extend(bytes);
			padded
		} else if bytes.len() > n {
			// Truncate from the beginning
			bytes[bytes.len() - n..].to_vec()
		} else {
			bytes
		}
	}
	
	fn to_string_with_commas(&self) -> String {
		let s = self.value.to_string();
		add_commas_to_number(&s)
	}
	
	fn from_bytes(bytes: &[u8]) -> Outcome<Self> {
		if bytes.is_empty() {
			return Ok(Self::new(BigInt::from(0)));
		}
		
		// Determine sign from the first bit
		let sign = if bytes[0] & 0x80 == 0 {
			num_bigint::Sign::Plus
		} else {
			num_bigint::Sign::Minus
		};
		
		Ok(Self::new(BigInt::from_bytes_be(sign, bytes)))
	}
	
	fn from_string(s: &str) -> Outcome<Self> {
		let cleaned = s.replace(",", "");
		match BigInt::from_str(&cleaned) {
			Ok(value) => Ok(Self::new(value)),
			Err(e) => Err(err!("Failed to parse TimeBigInt from string '{}': {}", s, e; Invalid, Input)),
		}
	}
}

impl std::fmt::Display for TimeBigInt {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "{}", self.value)
	}
}

impl From<i64> for TimeBigInt {
	fn from(value: i64) -> Self {
		Self::from_i64(value)
	}
}

impl From<BigInt> for TimeBigInt {
	fn from(value: BigInt) -> Self {
		Self::new(value)
	}
}

/// Helper function to add commas to number strings.
fn add_commas_to_number(s: &str) -> String {
	if s.is_empty() {
		return s.to_string();
	}
	
	let (sign, digits) = if s.starts_with('-') {
		("-", &s[1..])
	} else {
		("", s)
	};
	
	let mut result = String::new();
	result.push_str(sign);
	
	let chars: Vec<char> = digits.chars().collect();
	let len = chars.len();
	
	for (i, &ch) in chars.iter().enumerate() {
		result.push(ch);
		let remaining = len - i - 1;
		if remaining > 0 && remaining % 3 == 0 {
			result.push(',');
		}
	}
	
	result
}

#[cfg(test)]
mod tests {
	use super::*;
	
	#[test]
	fn test_time_long_basic_operations() {
		let a = TimeLong::new(100);
		let b = TimeLong::new(50);
		
		assert_eq!(a.clone().add_to(b.clone()).unwrap().value(), 150);
		assert_eq!(a.clone().subtract_it(b.clone()).unwrap().value(), 50);
		assert_eq!(a.clone().multiply_by(b.clone()).unwrap().value(), 5000);
		assert_eq!(a.clone().divide_by(b.clone()).unwrap().value(), 2);
		assert_eq!(a.remainder_by(b).unwrap().value(), 0);
	}
	
	#[test]
	fn test_time_long_serialization() {
		let original = TimeLong::new(123456789);
		let bytes = original.to_file_byte_array();
		let restored = TimeLong::from_bytes(&bytes).unwrap();
		assert_eq!(original, restored);
	}
	
	#[test]
	fn test_time_long_commas() {
		let num = TimeLong::new(1234567890);
		assert_eq!(num.to_string_with_commas(), "1,234,567,890");
	}
	
	#[test]
	fn test_time_big_int_basic_operations() {
		let a = TimeBigInt::from_i64(100);
		let b = TimeBigInt::from_i64(50);
		
		assert_eq!(a.clone().add_to(b.clone()).unwrap().long_value(), 150);
		assert_eq!(a.clone().subtract_it(b.clone()).unwrap().long_value(), 50);
		assert_eq!(a.clone().multiply_by(b.clone()).unwrap().long_value(), 5000);
		assert_eq!(a.clone().divide_by(b.clone()).unwrap().long_value(), 2);
		assert_eq!(a.remainder_by(b).unwrap().long_value(), 0);
	}
	
	#[test]
	fn test_add_commas_to_number() {
		assert_eq!(add_commas_to_number("1234567890"), "1,234,567,890");
		assert_eq!(add_commas_to_number("-1234567890"), "-1,234,567,890");
		assert_eq!(add_commas_to_number("123"), "123");
		assert_eq!(add_commas_to_number(""), "");
	}
}