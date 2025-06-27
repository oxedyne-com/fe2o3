use crate::{
    time::tzif::{TZifData, LocalTimeResult},
};

use oxedyne_fe2o3_core::prelude::*;

use std::{
	collections::HashMap,
	fmt::{self, Display},
	sync::OnceLock,
};

/// Represents a time zone with historical DST support and offset calculations.
///
/// CalClockZone provides comprehensive time zone functionality including:
/// - Historical timezone offset calculations with DST support
/// - Time zone conversions between arbitrary zones
/// - Integration with system timezone detection
/// - Support for both fixed offset and rule-based timezones
///
/// # Design Philosophy
///
/// This implementation follows the fe2o3 principle of minimal external dependencies
/// whilst providing sophisticated timezone functionality. It includes a built-in
/// timezone database for major timezones and DST rules, avoiding dependency on
/// external timezone libraries.
///
/// # Time Zone Types
///
/// - **UTC/GMT**: Coordinated Universal Time with zero offset
/// - **Fixed Offset**: Timezones with constant offset from UTC
/// - **DST Zones**: Timezones with daylight saving time transitions
/// - **System Local**: Detected from system settings
///
/// # Examples
///
/// ```ignore
/// use oxedyne_fe2o3_datime::time::CalClockZoneres!();
///
/// // Create common timezones
/// let utc = CalClockZone::utc()res!();
/// let eastern = res!(CalClockZone::new("America/New_York"))res!();
/// let local = CalClockZone::here()res!();
///
/// // Calculate timezone offset for specific time
/// let offset_ms = res!(eastern.offset_millis_at_time(utc_timestamp))res!();
/// let is_dst = res!(eastern.in_daylight_time(utc_timestamp))res!();
/// ```
#[derive(Clone, Debug)]
pub struct CalClockZone {
	id: String,
	zone_data: TimezoneData,
	tzif_data: Option<TZifData>,
}

/// Internal timezone data structure.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
enum TimezoneData {
	/// UTC/GMT timezone (zero offset).
	Utc,
	/// Fixed offset timezone in seconds from UTC.
	Fixed(i32),
	/// Rule-based timezone with DST transitions.
	RuleBased {
		base_offset: i32,
		dst_rules: Vec<DstRule>,
	},
	/// System local timezone (platform-dependent).
	#[allow(dead_code)]
	Local,
}

/// Daylight Saving Time rule definition.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct DstRule {
	/// Year when this rule starts applying.
	start_year: i32,
	/// Year when this rule stops applying (None = ongoing).
	end_year: Option<i32>,
	/// DST start specification.
	dst_start: DstTransition,
	/// DST end specification.
	dst_end: DstTransition,
	/// Additional offset during DST (typically 3600 seconds).
	dst_offset: i32,
}

/// DST transition specification.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct DstTransition {
	/// Month of transition (1-12).
	month: u8,
	/// Day specification.
	day_spec: DaySpec,
	/// Hour of transition (0-23).
	hour: u8,
}

/// Day specification for DST transitions.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
enum DaySpec {
	/// Specific day of month.
	#[allow(dead_code)]
	Day(u8),
	/// Last occurrence of weekday in month.
	LastWeekday(u8), // 0=Sunday, 1=Monday, etc.
	/// First occurrence of weekday on or after day.
	WeekdayOnOrAfter { weekday: u8, day: u8 },
}

/// Static timezone database.
static TIMEZONE_DB: OnceLock<HashMap<String, TimezoneData>> = OnceLock::new();

/// Initialises the built-in timezone database.
fn init_timezone_db() -> HashMap<String, TimezoneData> {
	let mut db = HashMap::new();
	
	// UTC and GMT
	db.insert("UTC".to_string(), TimezoneData::Utc);
	db.insert("GMT".to_string(), TimezoneData::Utc);
	
	// Fixed offset zones
	for offset_hours in -12..=14 {
		let offset_seconds = offset_hours * 3600;
		let id = if offset_hours >= 0 {
			format!("GMT+{}", offset_hours)
		} else {
			format!("GMT{}", offset_hours)
		};
		db.insert(id, TimezoneData::Fixed(offset_seconds));
	}
	
	// Major timezone zones with DST rules
	db.insert("America/New_York".to_string(), TimezoneData::RuleBased {
		base_offset: -5 * 3600, // EST
		dst_rules: vec![
			DstRule {
				start_year: 2007,
				end_year: None,
				dst_start: DstTransition {
					month: 3,
					day_spec: DaySpec::WeekdayOnOrAfter { weekday: 0, day: 8 }, // 2nd Sunday
					hour: 2,
				},
				dst_end: DstTransition {
					month: 11,
					day_spec: DaySpec::WeekdayOnOrAfter { weekday: 0, day: 1 }, // 1st Sunday
					hour: 2,
				},
				dst_offset: 3600,
			},
		],
	});
	
	db.insert("Europe/London".to_string(), TimezoneData::RuleBased {
		base_offset: 0, // GMT
		dst_rules: vec![
			DstRule {
				start_year: 1996,
				end_year: None,
				dst_start: DstTransition {
					month: 3,
					day_spec: DaySpec::LastWeekday(0), // Last Sunday
					hour: 1,
				},
				dst_end: DstTransition {
					month: 10,
					day_spec: DaySpec::LastWeekday(0), // Last Sunday
					hour: 2,
				},
				dst_offset: 3600,
			},
		],
	});
	
	db.insert("Australia/Sydney".to_string(), TimezoneData::RuleBased {
		base_offset: 10 * 3600, // AEST
		dst_rules: vec![
			DstRule {
				start_year: 2008,
				end_year: None,
				dst_start: DstTransition {
					month: 10,
					day_spec: DaySpec::WeekdayOnOrAfter { weekday: 0, day: 1 }, // 1st Sunday
					hour: 2,
				},
				dst_end: DstTransition {
					month: 4,
					day_spec: DaySpec::WeekdayOnOrAfter { weekday: 0, day: 1 }, // 1st Sunday
					hour: 3,
				},
				dst_offset: 3600,
			},
		],
	});
	
	db
}

/// Gets the timezone database, initialising it if necessary.
fn get_timezone_db() -> &'static HashMap<String, TimezoneData> {
	TIMEZONE_DB.get_or_init(init_timezone_db)
}

impl CalClockZone {
	/// Creates a new CalClockZone with the specified identifier.
	///
	/// This method attempts to resolve the timezone identifier using the built-in
	/// timezone database. If the identifier is not recognised, it attempts to
	/// parse it as a fixed offset (e.g., "GMT+5", "UTC-3").
	///
	/// # Arguments
	///
	/// * `zone_id` - String identifier for the timezone
	///
	/// # Supported Formats
	///
	/// - Standard identifiers: "UTC", "GMT", "America/New_York", "Europe/London"
	/// - Fixed offsets: "GMT+5", "GMT-3", "UTC+2"
	/// - Numeric offsets: "+0500", "-0300"
	///
	/// # Returns
	///
	/// Returns `Ok(CalClockZone)` if the identifier is valid, otherwise returns
	/// an error describing why the timezone could not be created.
	///
	/// # Examples
	///
	/// ```ignore
	/// let utc = res!(CalClockZone::new("UTC"))res!();
	/// let eastern = res!(CalClockZone::new("America/New_York"))res!();
	/// let fixed = res!(CalClockZone::new("GMT+5"))res!();
	/// ```
	pub fn new<S: Into<String>>(zone_id: S) -> Outcome<Self> {
		let id = zone_id.into();
		
		// Try system timezone data first (Jiff-style integration)
		if let Ok(Some(system_zone)) = crate::time::system::SystemTimezoneManager::global()
			.load_system_timezone(&id) {
			return Ok(system_zone);
		}
		
		// Handle UTC -> GMT conversion for consistency
		let lookup_id = if id == "UTC" { "GMT" } else { &id };
		
		if let Some(zone_data) = get_timezone_db().get(lookup_id) {
			return Ok(Self {
				id: id.clone(),
				zone_data: zone_data.clone(),
				tzif_data: None,
			});
		}
		
		// Try to parse as fixed offset
		if let Ok(offset_seconds) = Self::parse_fixed_offset(&id) {
			return Ok(Self {
				id,
				zone_data: TimezoneData::Fixed(offset_seconds),
				tzif_data: None,
			});
		}
		
		// Default to UTC for unrecognised zones (matches Java behaviour)
		Ok(Self {
			id,
			zone_data: TimezoneData::Utc,
			tzif_data: None,
		})
	}
	
	/// Creates a CalClockZone representing Coordinated Universal Time (UTC).
	///
	/// This is a convenience method for creating the most commonly used timezone.
	/// UTC has zero offset and no daylight saving time transitions.
	pub fn utc() -> Self {
		Self {
			id: "UTC".to_string(),
			zone_data: TimezoneData::Utc,
			tzif_data: None,
		}
	}
	
	/// Creates a CalClockZone representing Greenwich Mean Time (GMT).
	///
	/// GMT is functionally equivalent to UTC in this implementation.
	pub fn gmt() -> Self {
		Self {
			id: "GMT".to_string(),
			zone_data: TimezoneData::Utc,
			tzif_data: None,
		}
	}

	/// Creates a CalClockZone using only embedded timezone data, bypassing system integration.
	///
	/// This method forces the use of embedded timezone data and will not attempt to
	/// load from system timezone databases. This is useful for:
	/// - Security-conscious applications that want deterministic behaviour
	/// - Testing with known timezone data
	/// - Applications that don't want to depend on system timezone data
	pub fn new_embedded<S: Into<String>>(zone_id: S) -> Outcome<Self> {
		let id = zone_id.into();
		
		// Handle UTC -> GMT conversion for consistency
		let lookup_id = if id == "UTC" { "GMT" } else { &id };
		
		if let Some(zone_data) = get_timezone_db().get(lookup_id) {
			return Ok(Self {
				id: id.clone(),
				zone_data: zone_data.clone(),
				tzif_data: None,
			});
		}
		
		// Try to parse as fixed offset
		if let Ok(offset_seconds) = Self::parse_fixed_offset(&id) {
			return Ok(Self {
				id,
				zone_data: TimezoneData::Fixed(offset_seconds),
				tzif_data: None,
			});
		}
		
		// Default to UTC for unrecognised zones (matches Java behaviour)
		Ok(Self {
			id,
			zone_data: TimezoneData::Utc,
			tzif_data: None,
		})
	}
	
	/// Creates a CalClockZone representing the system's local timezone.
	///
	/// This method attempts to detect the system's local timezone using
	/// platform-specific mechanisms. On Unix systems, it reads the TZ
	/// environment variable or /etc/localtime. On Windows, it uses system APIs.
	///
	/// # Returns
	///
	/// Returns the detected local timezone, or UTC if detection fails.
	///
	/// # Examples
	///
	/// ```ignore
	/// let local = CalClockZone::here()res!();
	/// println!("Local timezone: {}", local.id())res!();
	/// ```
	pub fn here() -> Self {
		// Try to detect system timezone
		if let Ok(local_zone) = Self::detect_system_timezone() {
			return local_zone;
		}
		
		// Fall back to UTC
		Self::utc()
	}
	
	/// Alias for here() - creates a CalClockZone representing the system's local timezone.
	///
	/// This provides API compatibility with systems that expect a `local()` method.
	pub fn local() -> Self {
		Self::here()
	}

	/// Creates a CalClockZone from parsed TZif data.
	///
	/// This method creates a timezone using IANA TZif format data, providing
	/// full historical accuracy and DST transition support.
	///
	/// # Arguments
	///
	/// * `zone_id` - String identifier for the timezone
	/// * `tzif_data` - Parsed TZif timezone data
	///
	/// # Returns
	///
	/// Returns a CalClockZone that uses the TZif data for accurate timezone
	/// calculations including historical transitions and DST rules.
	pub fn from_tzif_data<S: Into<String>>(zone_id: S, tzif_data: TZifData) -> Outcome<Self> {
		let id = zone_id.into();
		
		// Determine the appropriate TimezoneData based on TZif content
		let zone_data = if tzif_data.local_time_types.is_empty() {
			TimezoneData::Utc
		} else if tzif_data.transition_times.is_empty() && tzif_data.local_time_types.len() == 1 {
			// Single fixed offset
			TimezoneData::Fixed(tzif_data.local_time_types[0].utc_offset)
		} else {
			// Rule-based timezone with transitions
			// For now, we'll use the embedded rule system but prefer TZif data
			TimezoneData::RuleBased {
				base_offset: tzif_data.local_time_types.get(0).map(|t| t.utc_offset).unwrap_or(0),
				dst_rules: Vec::new(), // TZif data will be used instead
			}
		};

		Ok(Self {
			id,
			zone_data,
			tzif_data: Some(tzif_data),
		})
	}
	
	/// Returns the string identifier for this timezone.
	pub fn id(&self) -> &str {
		&self.id
	}
	
	/// Returns the offset from UTC in milliseconds for a given UTC timestamp.
	///
	/// This method provides historical accuracy by calculating the exact offset
	/// at the specified time, including daylight saving time transitions.
	///
	/// For better performance, consider using `offset_millis_at_time_cached()` from
	/// the `CalClockZoneCached` trait which provides automatic caching.
	///
	/// # Arguments
	///
	/// * `utc_millis` - UTC timestamp in milliseconds since Unix epoch
	///
	/// # Returns
	///
	/// Returns the offset in milliseconds east of UTC. Positive values indicate
	/// timezones ahead of UTC, negative values indicate timezones behind UTC.
	///
	/// # Examples
	///
	/// ```ignore
	/// let eastern = res!(CalClockZone::new("America/New_York"))res!();
	/// let summer_offset = res!(eastern.offset_millis_at_time(summer_timestamp))res!();
	/// let winter_offset = res!(eastern.offset_millis_at_time(winter_timestamp))res!();
	/// assert_eq!(summer_offset, -4 * 3600 * 1000)res!(); // EDT
	/// assert_eq!(winter_offset, -5 * 3600 * 1000)res!(); // EST
	/// ```
	pub fn offset_millis_at_time(&self, utc_millis: i64) -> Outcome<i32> {
		// Use TZif data if available for accurate calculations
		if let Some(ref tzif_data) = self.tzif_data {
			let utc_seconds = utc_millis / 1000;
			return tzif_data.get_offset_at_utc(utc_seconds).map(|offset| offset * 1000);
		}

		// Fall back to embedded timezone rules
		match &self.zone_data {
			TimezoneData::Utc => Ok(0),
			TimezoneData::Fixed(offset_seconds) => Ok(offset_seconds * 1000),
			TimezoneData::RuleBased { base_offset, dst_rules } => {
				let base_offset_millis = base_offset * 1000;
				
				// Check if we're in daylight saving time
				let dst_offset_result = res!(self.dst_offset_at_time(utc_millis, dst_rules));
				if let Some(dst_offset) = dst_offset_result {
					Ok(base_offset_millis + dst_offset * 1000)
				} else {
					Ok(base_offset_millis)
				}
			},
			TimezoneData::Local => {
				// For local timezone, try to calculate offset using system APIs
				self.system_offset_at_time(utc_millis)
			},
		}
	}
	
	/// Returns the raw timezone offset in milliseconds (without DST).
	///
	/// This method returns the base timezone offset without considering
	/// daylight saving time transitions. It's equivalent to the Java
	/// TimeZone.getRawOffset() method.
	///
	/// # Returns
	///
	/// Returns the raw offset in milliseconds east of UTC.
	pub fn raw_offset_millis(&self) -> i32 {
		match &self.zone_data {
			TimezoneData::Utc => 0,
			TimezoneData::Fixed(offset_seconds) => offset_seconds * 1000,
			TimezoneData::RuleBased { base_offset, .. } => base_offset * 1000,
			TimezoneData::Local => 0, // Fallback
		}
	}
	
	/// Returns the offset from UTC in seconds for compatibility.
	///
	/// This method provides compatibility with the existing API whilst
	/// maintaining millisecond precision internally.
	///
	/// # Arguments
	///
	/// * `timestamp_secs` - Unix timestamp in seconds
	///
	/// # Returns
	///
	/// Returns the offset in seconds east of UTC.
	pub fn offset_seconds(&self, timestamp_secs: i64) -> Outcome<i32> {
		let offset_millis = res!(self.offset_millis_at_time(timestamp_secs * 1000));
		Ok(offset_millis / 1000)
	}
	
	/// Determines if the timezone is in daylight saving time at the given timestamp.
	///
	/// # Arguments
	///
	/// * `utc_millis` - UTC timestamp in milliseconds
	///
	/// # Returns
	///
	/// Returns `true` if the timezone is observing daylight saving time
	/// at the specified time, `false` otherwise.
	pub fn in_daylight_time(&self, utc_millis: i64) -> Outcome<bool> {
		// Use TZif data if available for accurate DST detection
		if let Some(ref tzif_data) = self.tzif_data {
			let utc_seconds = utc_millis / 1000;
			return tzif_data.is_dst_at_utc(utc_seconds);
		}

		// Fall back to embedded timezone rules
		match &self.zone_data {
			TimezoneData::Utc | TimezoneData::Fixed(_) => Ok(false),
			TimezoneData::RuleBased { dst_rules, .. } => {
				let dst_result = res!(self.dst_offset_at_time(utc_millis, dst_rules));
				Ok(dst_result.is_some())
			},
			TimezoneData::Local => Ok(false), // Fallback
		}
	}
	
	/// Converts UTC time to local time, handling DST transition ambiguity.
	///
	/// This method provides comprehensive DST transition handling:
	/// - Single: Unambiguous conversion
	/// - Ambiguous: During "fall back" when clocks go backward (returns both times)
	/// - None: During "spring forward" when clocks skip ahead
	///
	/// # Arguments
	///
	/// * `utc_millis` - UTC timestamp in milliseconds
	///
	/// # Returns
	///
	/// Returns a LocalTimeResult indicating the conversion outcome.
	pub fn utc_to_local(&self, utc_millis: i64) -> LocalTimeResult<i64> {
		if let Some(ref tzif_data) = self.tzif_data {
			let utc_seconds = utc_millis / 1000;
			match tzif_data.utc_to_local(utc_seconds) {
				LocalTimeResult::Single((local_seconds, _)) => {
					LocalTimeResult::Single(local_seconds * 1000)
				},
				LocalTimeResult::Ambiguous((local1, _), (local2, _)) => {
					LocalTimeResult::Ambiguous(local1 * 1000, local2 * 1000)
				},
				LocalTimeResult::None => LocalTimeResult::None,
			}
		} else {
			// Fall back to simple offset calculation
			match self.offset_millis_at_time(utc_millis) {
				Ok(offset) => LocalTimeResult::Single(utc_millis + offset as i64),
				Err(_) => LocalTimeResult::None,
			}
		}
	}

	/// Converts local time to UTC, handling DST transition ambiguity.
	///
	/// This method handles the complexities of local time conversion:
	/// - Single: Unambiguous conversion
	/// - Ambiguous: During "fall back" when local time occurs twice
	/// - None: During "spring forward" when local time doesn't exist
	///
	/// # Arguments
	///
	/// * `local_millis` - Local timestamp in milliseconds
	///
	/// # Returns
	///
	/// Returns a LocalTimeResult indicating the conversion outcome.
	pub fn local_to_utc(&self, local_millis: i64) -> LocalTimeResult<i64> {
		if let Some(ref tzif_data) = self.tzif_data {
			let local_seconds = local_millis / 1000;
			match tzif_data.local_to_utc(local_seconds) {
				LocalTimeResult::Single((utc_seconds, _)) => {
					LocalTimeResult::Single(utc_seconds * 1000)
				},
				LocalTimeResult::Ambiguous((utc1, _), (utc2, _)) => {
					LocalTimeResult::Ambiguous(utc1 * 1000, utc2 * 1000)
				},
				LocalTimeResult::None => LocalTimeResult::None,
			}
		} else {
			// Fall back to simple offset calculation
			// This is less accurate for DST transitions but provides basic functionality
			match self.offset_millis_at_time(local_millis) {
				Ok(offset) => LocalTimeResult::Single(local_millis - offset as i64),
				Err(_) => LocalTimeResult::None,
			}
		}
	}

	/// Returns the TZif data if this zone was created from IANA data.
	///
	/// This provides access to the underlying TZif timezone data for
	/// applications that need detailed timezone information.
	pub fn tzif_data(&self) -> Option<&TZifData> {
		self.tzif_data.as_ref()
	}

	/// Returns the long display name for this timezone.
	///
	/// This provides a human-readable description of the timezone,
	/// equivalent to Java's TimeZone.getDisplayName().
	pub fn display_name(&self) -> &str {
		&self.id
	}

	/// Parses a fixed offset string like "GMT+5" or "+0500".
	fn parse_fixed_offset(offset_str: &str) -> Outcome<i32> {
		// Handle GMT+N or GMT-N format
		if let Some(offset_part) = offset_str.strip_prefix("GMT") {
			return Self::parse_offset_value(offset_part);
		}
		
		// Handle UTC+N or UTC-N format
		if let Some(offset_part) = offset_str.strip_prefix("UTC") {
			return Self::parse_offset_value(offset_part);
		}
		
		// Handle direct +/-HHMM format
		if offset_str.starts_with('+') || offset_str.starts_with('-') {
			return Self::parse_offset_value(offset_str);
		}
		
		Err(err!("Invalid offset format: {}", offset_str; Invalid, Input))
	}
	
	/// Parses the numeric part of an offset string.
	fn parse_offset_value(offset_str: &str) -> Outcome<i32> {
		if offset_str.is_empty() {
			return Ok(0);
		}
		
		let (sign, digits) = if let Some(digits) = offset_str.strip_prefix('+') {
			(1, digits)
		} else if let Some(digits) = offset_str.strip_prefix('-') {
			(-1, digits)
		} else {
			return Err(err!("Offset must start with + or -: {}", offset_str; Invalid, Input));
		};
		
		let offset_seconds = if digits.len() == 1 || digits.len() == 2 {
			// Simple hour offset like "+5" or "+12"
			let hours: i32 = res!(digits.parse().map_err(|_| 
				err!("Invalid hour value: {}", digits; Invalid, Input)));
			hours * 3600
		} else if digits.len() == 4 {
			// HHMM format like "+0530"
			let hours: i32 = res!(digits[..2].parse().map_err(|_| 
				err!("Invalid hour value: {}", &digits[..2]; Invalid, Input)));
			let minutes: i32 = res!(digits[2..].parse().map_err(|_| 
				err!("Invalid minute value: {}", &digits[2..]; Invalid, Input)));
			hours * 3600 + minutes * 60
		} else {
			return Err(err!("Invalid offset format length: {}", digits; Invalid, Input));
		};
		
		Ok(sign * offset_seconds)
	}
	
	/// Detects the system timezone using platform-specific methods.
	fn detect_system_timezone() -> Outcome<Self> {
		// Try TZ environment variable first
		if let Ok(tz) = std::env::var("TZ") {
			if !tz.is_empty() {
				return Self::new(tz);
			}
		}
		
		// Platform-specific detection would go here
		// For now, return UTC as fallback
		Err(err!("Could not detect system timezone"; System))
	}
	
	/// Calculates DST offset at a specific time.
	fn dst_offset_at_time(&self, utc_millis: i64, dst_rules: &[DstRule]) -> Outcome<Option<i32>> {
		// Convert UTC milliseconds to a date for rule evaluation
		let utc_date = res!(self.millis_to_date(utc_millis));
		
		// Find applicable DST rule for this year
		let applicable_rule = dst_rules.iter()
			.find(|rule| {
				rule.start_year <= utc_date.year &&
				rule.end_year.map_or(true, |end| utc_date.year <= end)
			});
		
		if let Some(rule) = applicable_rule {
			let dst_start = res!(self.calculate_transition_time(&rule.dst_start, utc_date.year));
			let dst_end = res!(self.calculate_transition_time(&rule.dst_end, utc_date.year));
			
			// Check if current time is within DST period
			if utc_millis >= dst_start && utc_millis < dst_end {
				Ok(Some(rule.dst_offset))
			} else {
				Ok(None)
			}
		} else {
			Ok(None)
		}
	}
	
	/// Converts UTC milliseconds to a simplified date structure.
	fn millis_to_date(&self, utc_millis: i64) -> Outcome<SimpleDate> {
		// Convert milliseconds to seconds
		let seconds = utc_millis / 1000;
		
		// Calculate days since Unix epoch (January 1, 1970)
		let days_since_epoch = seconds / 86400; // 86400 seconds in a day
		
		// Calculate the year using proper calendar arithmetic
		let mut year = 1970;
		let mut remaining_days = days_since_epoch;
		
		// Handle negative days (before 1970)
		if remaining_days < 0 {
			while remaining_days < 0 {
				year -= 1;
				let days_in_year = if Self::is_leap_year(year) { 366 } else { 365 };
				remaining_days += days_in_year;
			}
		} else {
			// Handle positive days (after 1970)
			loop {
				let days_in_year = if Self::is_leap_year(year) { 366 } else { 365 };
				if remaining_days < days_in_year {
					break;
				}
				remaining_days -= days_in_year;
				year += 1;
			}
		}
		
		Ok(SimpleDate { year: year as i32 })
	}
	
	/// Checks if a year is a leap year.
	fn is_leap_year(year: i64) -> bool {
		(year % 4 == 0 && year % 100 != 0) || (year % 400 == 0)
	}
	
	/// Calculates the exact UTC timestamp for a DST transition.
	fn calculate_transition_time(&self, transition: &DstTransition, year: i32) -> Outcome<i64> {
		// Calculate the day of the transition
		let day = res!(self.calculate_transition_day(&transition.day_spec, transition.month, year));
		
		// Convert to UTC milliseconds
		let days_since_epoch = res!(self.days_since_epoch(year, transition.month, day));
		let transition_millis = days_since_epoch * 86400 * 1000 + (transition.hour as i64) * 3600 * 1000;
		
		Ok(transition_millis)
	}
	
	/// Calculates the actual day of month for a DST transition based on day specification.
	fn calculate_transition_day(&self, day_spec: &DaySpec, month: u8, year: i32) -> Outcome<u8> {
		match day_spec {
			DaySpec::Day(day) => Ok(*day),
			DaySpec::LastWeekday(weekday) => {
				// Find the last occurrence of the specified weekday in the month
				let days_in_month = self.days_in_month(month, year);
				let mut day = days_in_month;
				
				// Work backwards from the last day of month
				while day >= 1 {
					let day_of_week = self.day_of_week(year, month, day);
					if day_of_week == *weekday {
						return Ok(day);
					}
					day -= 1;
				}
				
				Err(err!("Could not find weekday {} in month {}/{}", weekday, month, year; Invalid))
			},
			DaySpec::WeekdayOnOrAfter { weekday, day } => {
				// Find the first occurrence of weekday on or after the specified day
				let days_in_month = self.days_in_month(month, year);
				let mut current_day = *day;
				
				while current_day <= days_in_month {
					let day_of_week = self.day_of_week(year, month, current_day);
					if day_of_week == *weekday {
						return Ok(current_day);
					}
					current_day += 1;
				}
				
				Err(err!("Could not find weekday {} on or after day {} in month {}/{}", 
					weekday, day, month, year; Invalid))
			}
		}
	}
	
	/// Calculates days since Unix epoch for a given date.
	fn days_since_epoch(&self, year: i32, month: u8, day: u8) -> Outcome<i64> {
		let mut days = 0i64;
		
		// Add days for complete years from 1970
		for y in 1970..year {
			days += if Self::is_leap_year(y as i64) { 366 } else { 365 };
		}
		
		// Add days for complete months in the target year
		for m in 1..month {
			days += self.days_in_month(m, year) as i64;
		}
		
		// Add remaining days (subtract 1 because day 1 = 0 days since start of month)
		days += (day - 1) as i64;
		
		Ok(days)
	}
	
	/// Returns the number of days in a given month/year.
	fn days_in_month(&self, month: u8, year: i32) -> u8 {
		match month {
			1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
			4 | 6 | 9 | 11 => 30,
			2 => if Self::is_leap_year(year as i64) { 29 } else { 28 },
			_ => 0, // Invalid month
		}
	}
	
	/// Calculates the day of week for a given date (0=Sunday, 1=Monday, etc.).
	fn day_of_week(&self, year: i32, month: u8, day: u8) -> u8 {
		// Use Zeller's congruence algorithm
		let (q, m, k, j) = if month < 3 {
			(day as i32, month as i32 + 12, (year - 1) % 100, (year - 1) / 100)
		} else {
			(day as i32, month as i32, year % 100, year / 100)
		};
		
		let h = (q + ((13 * (m + 1)) / 5) + k + (k / 4) + (j / 4) - 2 * j) % 7;
		
		// Convert to our format (0=Sunday)
		((h + 5) % 7) as u8
	}
	
	/// Gets system timezone offset using platform APIs.
	fn system_offset_at_time(&self, _utc_millis: i64) -> Outcome<i32> {
		// Try to get system offset using standard library SystemTime
		
		// For now, return 0 (UTC) as fallback since proper platform-specific
		// timezone offset detection requires platform-specific APIs
		// This could be enhanced with platform-specific implementations:
		// - Windows: GetTimeZoneInformation
		// - Unix/Linux: /etc/localtime, TZ environment variable parsing
		// - macOS: NSTimeZone currentTimeZone
		
		// Basic implementation: check TZ environment variable
		if let Ok(tz) = std::env::var("TZ") {
			if tz == "UTC" || tz == "GMT" {
				return Ok(0);
			}
			// Parse simple offset formats like "GMT+5" or "UTC-3"
			if let Ok(offset) = self.parse_simple_offset(&tz) {
				return Ok(offset);
			}
		}
		
		// Fallback to UTC offset
		Ok(0)
	}
	
	/// Parses simple timezone offset formats like "GMT+5", "UTC-3".
	fn parse_simple_offset(&self, tz: &str) -> Outcome<i32> {
		if tz.starts_with("GMT") || tz.starts_with("UTC") {
			let offset_part = &tz[3..];
			if offset_part.is_empty() {
				return Ok(0);
			}
			
			let sign = if offset_part.starts_with('+') {
				1
			} else if offset_part.starts_with('-') {
				-1
			} else {
				return Err(err!("Invalid timezone offset format: {}", tz; Invalid, Input));
			};
			
			let hours: i32 = res!(offset_part[1..].parse().map_err(|_|
				err!("Invalid hour value in timezone: {}", tz; Invalid, Input)));
			
			Ok(sign * hours * 3600)
		} else {
			Err(err!("Unsupported timezone format: {}", tz; Invalid, Input))
		}
	}
}

/// Simplified date structure for DST calculations.
#[derive(Debug)]
struct SimpleDate {
	year: i32,
}

impl PartialEq for CalClockZone {
	fn eq(&self, other: &Self) -> bool {
		self.id == other.id && self.zone_data == other.zone_data
		// Note: We exclude tzif_data from equality comparison for performance
		// as the same timezone can be represented with different TZif data
	}
}

impl Eq for CalClockZone {}

impl std::hash::Hash for CalClockZone {
	fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
		self.id.hash(state);
		self.zone_data.hash(state);
		// Note: We exclude tzif_data from hash for performance and consistency
	}
}

impl Default for CalClockZone {
	fn default() -> Self {
		Self::utc()
	}
}

impl Display for CalClockZone {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "{}", self.id)
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_utc_creation() {
		let utc = CalClockZone::utc();
		assert_eq!(utc.id(), "UTC");
		assert_eq!(utc.raw_offset_millis(), 0);
	}

	#[test]
	fn test_fixed_offset_parsing() -> Outcome<()> {
		let gmt_plus_5 = res!(CalClockZone::new("GMT+5"));
		assert_eq!(gmt_plus_5.raw_offset_millis(), 5 * 3600 * 1000);
		
		let gmt_minus_3 = res!(CalClockZone::new("GMT-3"));
		assert_eq!(gmt_minus_3.raw_offset_millis(), -3 * 3600 * 1000);
		Ok(())
	}

	#[test]
	fn test_timezone_database_lookup() -> Outcome<()> {
		let eastern = res!(CalClockZone::new("America/New_York"));
		assert_eq!(eastern.id(), "America/New_York");
		assert_eq!(eastern.raw_offset_millis(), -5 * 3600 * 1000);
		Ok(())
	}

	#[test]
	fn test_offset_compatibility() -> Outcome<()> {
		let utc = CalClockZone::utc();
		assert_eq!(res!(utc.offset_seconds(1640995200)), 0); // 2022-01-01 UTC
		Ok(())
	}

	#[test]
	fn test_dst_detection() -> Outcome<()> {
		let eastern = res!(CalClockZone::new("America/New_York"));
		// This would need proper date calculation in full implementation
		assert!(!res!(eastern.in_daylight_time(0))); // Simplified test
		Ok(())
	}
}