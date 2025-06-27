use crate::{
	time::{CalClock, CalClockZone},
	clock::ClockTime,
	calendar::CalendarDate,
};

use oxedyne_fe2o3_core::prelude::*;

/// RFC 9557 Internet Extended Date/Time Format (IXDTF) serialisation capabilities.
///
/// This module implements timezone-preserving serialisation according to RFC 9557,
/// which extends RFC 3339 timestamps with additional timezone information.
///
/// RFC 9557 allows timestamps to include both UTC offset and IANA timezone identifier,
/// enabling proper handling of daylight saving transitions and timezone-aware calculations.
///
/// # Examples
///
/// ```ignore
/// use oxedyne_fe2o3_datime::{CalClock, CalClockZone, format::rfc9557::Rfc9557Format};
///
/// let zone = res!(CalClockZone::new("America/New_York"));
/// let calclock = res!(CalClock::new(2024, 6, 15, 14, 30, 0, 0, zone));
/// 
/// // Standard RFC 3339 format
/// let rfc3339 = res!(calclock.to_rfc9557_basic());
/// // "2024-06-15T14:30:00.000000000-04:00"
///
/// // Extended RFC 9557 format with timezone preservation
/// let rfc9557 = res!(calclock.to_rfc9557_extended());
/// // "2024-06-15T14:30:00.000000000-04:00[America/New_York]"
/// ```

/// Configuration options for RFC 9557 serialisation.
#[derive(Clone, Debug, PartialEq)]
pub struct Rfc9557Config {
	/// Include IANA timezone identifier in square brackets.
	pub include_timezone_name: bool,
	
	/// Include nanoseconds even if they are zero.
	pub always_include_nanoseconds: bool,
	
	/// Use Z notation for UTC instead of +00:00.
	pub use_z_for_utc: bool,
	
	/// Include precision indicators for approximate times.
	pub include_precision_indicators: bool,
	
	/// Precision level when using precision indicators.
	pub precision_level: PrecisionLevel,
}

/// Precision levels for RFC 9557 timestamps.
#[derive(Clone, Debug, PartialEq)]
pub enum PrecisionLevel {
	/// Exact time (no precision indicator).
	Exact,
	/// Approximate time (~).
	Approximate,
	/// Uncertain time (%).
	Uncertain,
	/// Around a specific time (@).
	Around,
	/// Between two times (*).
	Between,
}

impl Default for Rfc9557Config {
	fn default() -> Self {
		Self {
			include_timezone_name: true,
			always_include_nanoseconds: false,
			use_z_for_utc: true,
			include_precision_indicators: false,
			precision_level: PrecisionLevel::Exact,
		}
	}
}

impl PrecisionLevel {
	/// Returns the RFC 9557 precision indicator character.
	pub fn indicator(&self) -> Option<char> {
		match self {
			Self::Exact => None,
			Self::Approximate => Some('~'),
			Self::Uncertain => Some('%'),
			Self::Around => Some('@'),
			Self::Between => Some('*'),
		}
	}
}

/// RFC 9557 formatting capabilities for datetime types.
pub trait Rfc9557Format {
	/// Serializes to RFC 9557 format with default configuration.
	fn to_rfc9557(&self) -> Outcome<String>;
	
	/// Serializes to RFC 9557 format with custom configuration.
	fn to_rfc9557_with_config(&self, config: &Rfc9557Config) -> Outcome<String>;
	
	/// Serializes to basic RFC 3339 format (no timezone name).
	fn to_rfc9557_basic(&self) -> Outcome<String>;
	
	/// Serializes to extended RFC 9557 format (with timezone name).
	fn to_rfc9557_extended(&self) -> Outcome<String>;
	
	/// Parses an RFC 9557 formatted string.
	fn from_rfc9557(input: &str) -> Outcome<Self>
	where
		Self: Sized;
}

impl Rfc9557Format for CalClock {
	fn to_rfc9557(&self) -> Outcome<String> {
		self.to_rfc9557_with_config(&Rfc9557Config::default())
	}
	
	fn to_rfc9557_with_config(&self, config: &Rfc9557Config) -> Outcome<String> {
		// Build the basic timestamp part.
		let date_part = format!("{:04}-{:02}-{:02}", 
			self.year(), self.month(), self.day());
		
		// Format time with conditional nanoseconds.
		let time_part = if config.always_include_nanoseconds || self.nanosecond() > 0 {
			format!("{:02}:{:02}:{:02}.{:09}",
				self.hour(), self.minute(), self.second(), self.nanosecond())
		} else {
			format!("{:02}:{:02}:{:02}",
				self.hour(), self.minute(), self.second())
		};
		
		// Get timezone offset.
		let offset_millis = res!(self.zone().offset_millis_at_time(res!(self.to_millis())));
		let offset_hours = offset_millis / (60 * 60 * 1000);
		let offset_minutes = (offset_millis.abs() % (60 * 60 * 1000)) / (60 * 1000);
		
		// Format offset part.
		let offset_part = if offset_millis == 0 && config.use_z_for_utc {
			"Z".to_string()
		} else {
			format!("{:+03}:{:02}", offset_hours, offset_minutes)
		};
		
		// Add precision indicator if configured.
		let precision_suffix = if config.include_precision_indicators {
			config.precision_level.indicator()
				.map(|c| c.to_string())
				.unwrap_or_default()
		} else {
			String::new()
		};
		
		// Add timezone name if configured.
		let timezone_suffix = if config.include_timezone_name {
			format!("[{}]", self.zone().id())
		} else {
			String::new()
		};
		
		Ok(format!("{}T{}{}{}{}", 
			date_part, time_part, offset_part, precision_suffix, timezone_suffix))
	}
	
	fn to_rfc9557_basic(&self) -> Outcome<String> {
		let config = Rfc9557Config {
			include_timezone_name: false,
			..Default::default()
		};
		self.to_rfc9557_with_config(&config)
	}
	
	fn to_rfc9557_extended(&self) -> Outcome<String> {
		let config = Rfc9557Config {
			include_timezone_name: true,
			..Default::default()
		};
		self.to_rfc9557_with_config(&config)
	}
	
	fn from_rfc9557(input: &str) -> Outcome<Self> {
		// Parse RFC 9557 format: YYYY-MM-DDTHH:MM:SS[.nnnnnnnnn][±HH:MM|Z][~|%|@|*][timezone]
		let input = input.trim();
		
		// Extract timezone name if present (in square brackets at the end).
		let (timestamp_part, timezone_name) = if let Some(bracket_start) = input.rfind('[') {
			if let Some(bracket_end) = input.rfind(']') {
				if bracket_end > bracket_start {
					let timezone_part = &input[bracket_start + 1..bracket_end];
					let timestamp_part = &input[..bracket_start];
					(timestamp_part, Some(timezone_part))
				} else {
					(input, None)
				}
			} else {
				(input, None)
			}
		} else {
			(input, None)
		};
		
		// Remove precision indicators if present.
		let timestamp_part = timestamp_part.trim_end_matches(['~', '%', '@', '*']);
		
		// Manual parsing instead of relying on parse_iso.
		// Split into date and time parts.
		let parts: Vec<&str> = timestamp_part.split('T').collect();
		if parts.len() != 2 {
			return Err(err!("Invalid timestamp format: {}", timestamp_part; Invalid, Input));
		}
		
		let date_part = parts[0];
		let time_part_with_offset = parts[1];
		
		// Parse date.
		let date_components: Vec<&str> = date_part.split('-').collect();
		if date_components.len() != 3 {
			return Err(err!("Invalid date format: {}", date_part; Invalid, Input));
		}
		
		let year = res!(date_components[0].parse::<i32>()
			.map_err(|_| err!("Invalid year: {}", date_components[0]; Invalid, Input)));
		let month = res!(date_components[1].parse::<u8>()
			.map_err(|_| err!("Invalid month: {}", date_components[1]; Invalid, Input)));
		let day = res!(date_components[2].parse::<u8>()
			.map_err(|_| err!("Invalid day: {}", date_components[2]; Invalid, Input)));
		
		// Extract offset and time.
		let (time_part, _offset_part) = if time_part_with_offset.ends_with('Z') {
			(&time_part_with_offset[..time_part_with_offset.len() - 1], "Z")
		} else if let Some(plus_pos) = time_part_with_offset.rfind('+') {
			(&time_part_with_offset[..plus_pos], &time_part_with_offset[plus_pos..])
		} else if let Some(minus_pos) = time_part_with_offset.rfind('-') {
			(&time_part_with_offset[..minus_pos], &time_part_with_offset[minus_pos..])
		} else {
			(time_part_with_offset, "")
		};
		
		// Parse time components.
		let time_components: Vec<&str> = time_part.split(':').collect();
		if time_components.len() < 3 {
			return Err(err!("Invalid time format: {}", time_part; Invalid, Input));
		}
		
		let hour = res!(time_components[0].parse::<u8>()
			.map_err(|_| err!("Invalid hour: {}", time_components[0]; Invalid, Input)));
		let minute = res!(time_components[1].parse::<u8>()
			.map_err(|_| err!("Invalid minute: {}", time_components[1]; Invalid, Input)));
		
		// Handle seconds with optional fractional part.
		let (second, nanosecond) = if let Some(dot_pos) = time_components[2].find('.') {
			let second_part = &time_components[2][..dot_pos];
			let fraction_part = &time_components[2][dot_pos + 1..];
			
			let second = res!(second_part.parse::<u8>()
				.map_err(|_| err!("Invalid second: {}", second_part; Invalid, Input)));
			
			// Parse fractional seconds, padding or truncating to 9 digits.
			let mut fraction_str = fraction_part.to_string();
			fraction_str.truncate(9); // Max 9 digits for nanoseconds.
			while fraction_str.len() < 9 {
				fraction_str.push('0'); // Pad with zeros.
			}
			
			let nanosecond = res!(fraction_str.parse::<u32>()
				.map_err(|_| err!("Invalid nanosecond: {}", fraction_str; Invalid, Input)));
			
			(second, nanosecond)
		} else {
			let second = res!(time_components[2].parse::<u8>()
				.map_err(|_| err!("Invalid second: {}", time_components[2]; Invalid, Input)));
			(second, 0)
		};
		
		// Determine timezone.
		let zone = if let Some(tz_name) = timezone_name {
			res!(CalClockZone::new(tz_name))
		} else {
			CalClockZone::utc()
		};
		
		Self::new(year, month, day, hour, minute, second, nanosecond, zone)
	}
}

impl Rfc9557Format for ClockTime {
	fn to_rfc9557(&self) -> Outcome<String> {
		self.to_rfc9557_with_config(&Rfc9557Config::default())
	}
	
	fn to_rfc9557_with_config(&self, config: &Rfc9557Config) -> Outcome<String> {
		// Format time component only.
		let time_part = if config.always_include_nanoseconds || self.nanosecond().of() > 0 {
			format!("{:02}:{:02}:{:02}.{:09}",
				self.hour().of(), self.minute().of(), self.second().of(), self.nanosecond().of())
		} else {
			format!("{:02}:{:02}:{:02}",
				self.hour().of(), self.minute().of(), self.second().of())
		};
		
		// Add precision indicator if configured.
		let precision_suffix = if config.include_precision_indicators {
			config.precision_level.indicator()
				.map(|c| c.to_string())
				.unwrap_or_default()
		} else {
			String::new()
		};
		
		// Add timezone name if configured.
		let timezone_suffix = if config.include_timezone_name {
			format!("[{}]", self.zone().id())
		} else {
			String::new()
		};
		
		Ok(format!("{}{}{}", time_part, precision_suffix, timezone_suffix))
	}
	
	fn to_rfc9557_basic(&self) -> Outcome<String> {
		let config = Rfc9557Config {
			include_timezone_name: false,
			..Default::default()
		};
		self.to_rfc9557_with_config(&config)
	}
	
	fn to_rfc9557_extended(&self) -> Outcome<String> {
		let config = Rfc9557Config {
			include_timezone_name: true,
			..Default::default()
		};
		self.to_rfc9557_with_config(&config)
	}
	
	fn from_rfc9557(input: &str) -> Outcome<Self> {
		// For time-only parsing, we'll use UTC as default and then update if timezone specified.
		let input = input.trim();
		
		// Extract timezone name if present.
		let (time_part, timezone_name) = if let Some(bracket_start) = input.rfind('[') {
			if let Some(bracket_end) = input.rfind(']') {
				if bracket_end > bracket_start {
					let timezone_part = &input[bracket_start + 1..bracket_end];
					let time_part = &input[..bracket_start];
					(time_part, Some(timezone_part))
				} else {
					(input, None)
				}
			} else {
				(input, None)
			}
		} else {
			(input, None)
		};
		
		// Remove precision indicators.
		let time_part = time_part.trim_end_matches(['~', '%', '@', '*']);
		
		// Parse time components.
		let parts: Vec<&str> = time_part.split(':').collect();
		if parts.len() < 3 {
			return Err(err!("Invalid time format: {}", input; Invalid, Input));
		}
		
		let hour = res!(parts[0].parse::<u8>()
			.map_err(|_| err!("Invalid hour: {}", parts[0]; Invalid, Input)));
		let minute = res!(parts[1].parse::<u8>()
			.map_err(|_| err!("Invalid minute: {}", parts[1]; Invalid, Input)));
		
		// Handle seconds with optional fractional part.
		let (second, nanosecond) = if let Some(dot_pos) = parts[2].find('.') {
			let second_part = &parts[2][..dot_pos];
			let fraction_part = &parts[2][dot_pos + 1..];
			
			let second = res!(second_part.parse::<u8>()
				.map_err(|_| err!("Invalid second: {}", second_part; Invalid, Input)));
			
			// Parse fractional seconds, padding or truncating to 9 digits.
			let mut fraction_str = fraction_part.to_string();
			fraction_str.truncate(9); // Max 9 digits for nanoseconds.
			while fraction_str.len() < 9 {
				fraction_str.push('0'); // Pad with zeros.
			}
			
			let nanosecond = res!(fraction_str.parse::<u32>()
				.map_err(|_| err!("Invalid nanosecond: {}", fraction_str; Invalid, Input)));
			
			(second, nanosecond)
		} else {
			let second = res!(parts[2].parse::<u8>()
				.map_err(|_| err!("Invalid second: {}", parts[2]; Invalid, Input)));
			(second, 0)
		};
		
		// Determine timezone.
		let zone = if let Some(tz_name) = timezone_name {
			res!(CalClockZone::new(tz_name))
		} else {
			CalClockZone::utc()
		};
		
		Self::new(hour, minute, second, nanosecond, zone)
	}
}

impl Rfc9557Format for CalendarDate {
	fn to_rfc9557(&self) -> Outcome<String> {
		self.to_rfc9557_with_config(&Rfc9557Config::default())
	}
	
	fn to_rfc9557_with_config(&self, config: &Rfc9557Config) -> Outcome<String> {
		// Format date component only.
		let date_part = format!("{:04}-{:02}-{:02}", 
			self.year(), self.month(), self.day());
		
		// Add timezone name if configured.
		let timezone_suffix = if config.include_timezone_name {
			format!("[{}]", self.zone().id())
		} else {
			String::new()
		};
		
		Ok(format!("{}{}", date_part, timezone_suffix))
	}
	
	fn to_rfc9557_basic(&self) -> Outcome<String> {
		let config = Rfc9557Config {
			include_timezone_name: false,
			..Default::default()
		};
		self.to_rfc9557_with_config(&config)
	}
	
	fn to_rfc9557_extended(&self) -> Outcome<String> {
		let config = Rfc9557Config {
			include_timezone_name: true,
			..Default::default()
		};
		self.to_rfc9557_with_config(&config)
	}
	
	fn from_rfc9557(input: &str) -> Outcome<Self> {
		let input = input.trim();
		
		// Extract timezone name if present.
		let (date_part, timezone_name) = if let Some(bracket_start) = input.rfind('[') {
			if let Some(bracket_end) = input.rfind(']') {
				if bracket_end > bracket_start {
					let timezone_part = &input[bracket_start + 1..bracket_end];
					let date_part = &input[..bracket_start];
					(date_part, Some(timezone_part))
				} else {
					(input, None)
				}
			} else {
				(input, None)
			}
		} else {
			(input, None)
		};
		
		// Parse date components.
		let parts: Vec<&str> = date_part.split('-').collect();
		if parts.len() != 3 {
			return Err(err!("Invalid date format: {}", input; Invalid, Input));
		}
		
		let year = res!(parts[0].parse::<i32>()
			.map_err(|_| err!("Invalid year: {}", parts[0]; Invalid, Input)));
		let month = res!(parts[1].parse::<u8>()
			.map_err(|_| err!("Invalid month: {}", parts[1]; Invalid, Input)));
		let day = res!(parts[2].parse::<u8>()
			.map_err(|_| err!("Invalid day: {}", parts[2]; Invalid, Input)));
		
		// Determine timezone.
		let zone = if let Some(tz_name) = timezone_name {
			res!(CalClockZone::new(tz_name))
		} else {
			CalClockZone::utc()
		};
		
		Self::new(year, month, day, zone)
	}
}

/// Utility functions for RFC 9557 operations.
pub mod utils {
	use super::*;
	
	/// Validates an RFC 9557 timestamp string.
	pub fn validate_rfc9557(input: &str) -> Outcome<()> {
		// Try to parse as CalClock to validate format.
		match CalClock::from_rfc9557(input) {
			Ok(_) => Ok(()),
			Err(_) => Err(err!("Invalid RFC 9557 format: {}", input; Invalid, Input)),
		}
	}
	
	/// Extracts the timezone name from an RFC 9557 string if present.
	pub fn extract_timezone_name(input: &str) -> Option<&str> {
		if let Some(bracket_start) = input.rfind('[') {
			if let Some(bracket_end) = input.rfind(']') {
				if bracket_end > bracket_start {
					return Some(&input[bracket_start + 1..bracket_end]);
				}
			}
		}
		None
	}
	
	/// Extracts precision indicators from an RFC 9557 string.
	pub fn extract_precision_indicator(input: &str) -> Option<PrecisionLevel> {
		if input.ends_with('~') {
			Some(PrecisionLevel::Approximate)
		} else if input.ends_with('%') {
			Some(PrecisionLevel::Uncertain)
		} else if input.ends_with('@') {
			Some(PrecisionLevel::Around)
		} else if input.ends_with('*') {
			Some(PrecisionLevel::Between)
		} else {
			None
		}
	}
}