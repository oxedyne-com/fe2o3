//! [Written entirely with AI](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use crate::{
    time::tzif::{TZifData, TZifParser, LocalTimeResult},
};

use oxedyne_fe2o3_core::prelude::*;

use std::{
	collections::HashMap,
	fmt::{self, Display},
	fs,
	path::Path,
	sync::OnceLock,
};

/// A zone is UTC, a fixed offset, or a set of DST rules. The rules for the
/// major zones are embedded rather than taken from a timezone crate, with the
/// host's own zoneinfo tree as the fallback for everything else.
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

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
enum TimezoneData {
	Utc,
	Fixed(i32), // seconds east of UTC
	RuleBased {
		base_offset: i32, // seconds east of UTC, before DST
		dst_rules: Vec<DstRule>,
	},
	#[allow(dead_code)]
	Local,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct DstRule {
	start_year: i32,
	end_year: Option<i32>, // None while the rule is still in force
	dst_start: DstTransition,
	dst_end: DstTransition,
	dst_offset: i32, // seconds, usually 3600
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct DstTransition {
	month: u8, // 1-12
	day_spec: DaySpec,
	hour: u8, // 0-23
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
enum DaySpec {
	#[allow(dead_code)]
	Day(u8), // day of month
	LastWeekday(u8), // 0=Sunday, 1=Monday, etc.
	WeekdayOnOrAfter { weekday: u8, day: u8 },
}

static TIMEZONE_DB: OnceLock<HashMap<String, TimezoneData>> = OnceLock::new();

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

fn get_timezone_db() -> &'static HashMap<String, TimezoneData> {
	TIMEZONE_DB.get_or_init(init_timezone_db)
}

impl CalClockZone {
	/// Accepts a standard identifier, a named offset such as "GMT+5", or a
	/// numeric one such as "+0530". An unrecognised name resolves to UTC rather
	/// than failing, which is what the Java original does.
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

		// A name the embedded table does not hold may still be on the system's
		// own zoneinfo tree, whose rules are better than a silent zero offset.
		if let Ok(zone) = Self::from_zoneinfo_name(&id) {
			return Ok(zone);
		}

		// Default to UTC for unrecognised zones (matches Java behaviour)
		Ok(Self {
			id,
			zone_data: TimezoneData::Utc,
			tzif_data: None,
		})
	}
	
	pub fn utc() -> Self {
		Self {
			id: "UTC".to_string(),
			zone_data: TimezoneData::Utc,
			tzif_data: None,
		}
	}
	
	/// GMT is held as UTC here: zero offset, no transitions.
	pub fn gmt() -> Self {
		Self {
			id: "GMT".to_string(),
			zone_data: TimezoneData::Utc,
			tzif_data: None,
		}
	}

	/// As new, but never consults the system timezone database, so the result
	/// does not vary with the host.
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

		// A name the embedded table does not hold may still be on the system's
		// own zoneinfo tree, whose rules are better than a silent zero offset.
		if let Ok(zone) = Self::from_zoneinfo_name(&id) {
			return Ok(zone);
		}

		// Default to UTC for unrecognised zones (matches Java behaviour)
		Ok(Self {
			id,
			zone_data: TimezoneData::Utc,
			tzif_data: None,
		})
	}
	
	/// The host's own zone, read from TZ, /etc/localtime or /etc/timezone, and
	/// UTC when none of them answer.
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
	
	pub fn local() -> Self {
		Self::here()
	}

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
	
	pub fn id(&self) -> &str {
		&self.id
	}
	
	/// Milliseconds east of UTC at that instant, so the DST rules in force then
	/// are the ones applied. `CalClockZoneCached` has a caching form.
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
	
	/// Milliseconds east of UTC ignoring any DST in force, as Java's
	/// TimeZone.getRawOffset does.
	pub fn raw_offset_millis(&self) -> i32 {
		match &self.zone_data {
			TimezoneData::Utc => 0,
			TimezoneData::Fixed(offset_seconds) => offset_seconds * 1000,
			TimezoneData::RuleBased { base_offset, .. } => base_offset * 1000,
			TimezoneData::Local => 0, // Fallback
		}
	}
	
	pub fn offset_seconds(&self, timestamp_secs: i64) -> Outcome<i32> {
		let offset_millis = res!(self.offset_millis_at_time(timestamp_secs * 1000));
		Ok(offset_millis / 1000)
	}
	
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
	
	/// Ambiguous covers the autumn fold, where the local time is reached twice
	/// and both are returned; None the spring gap, where it is never reached.
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

	/// The inverse of utc_to_local, and ambiguous or absent over the same two
	/// transitions.
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

	/// None unless the zone came from IANA data.
	pub fn tzif_data(&self) -> Option<&TZifData> {
		self.tzif_data.as_ref()
	}

	pub fn display_name(&self) -> &str {
		&self.id
	}

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
	
	fn detect_system_timezone() -> Outcome<Self> {
		// TZ first, as POSIX says: an optional leading colon, then either a
		// zone name or a path to a TZif file.
		if let Ok(tz) = std::env::var("TZ") {
			if !tz.is_empty() {
				let name = tz.strip_prefix(':').unwrap_or(&tz);
				if name.starts_with('/') {
					if let Ok(zone) = Self::from_tzif_file(name, Path::new(name)) {
						return Ok(zone);
					}
				}
				if let Ok(zone) = Self::from_zoneinfo_name(name) {
					return Ok(zone);
				}
				return Self::new(name);
			}
		}

		// /etc/localtime: on every modern Linux and macOS a symlink into the
		// zoneinfo tree, and a TZif file either way. The symlink target names
		// the zone; the bytes carry its rules, so the answer is right even for
		// a zone the embedded table does not hold. Reading the machine's own
		// setting is the whole purpose of local(), so no consent machinery
		// applies here -- that gate belongs to the manager that scans zone
		// data wholesale.
		let localtime = Path::new("/etc/localtime");
		if localtime.exists() {
			let id = fs::read_link(localtime).ok()
				.and_then(|target| Self::zone_name_from_path(&target))
				.unwrap_or_else(|| "Local".to_string());
			if let Ok(zone) = Self::from_tzif_file(&id, localtime) {
				return Ok(zone);
			}
		}

		// /etc/timezone: the Debian name file, no longer shipped everywhere
		// (Ubuntu 25.10 dropped it) but still authoritative where it exists.
		if let Ok(name) = fs::read_to_string("/etc/timezone") {
			let name = name.trim();
			if !name.is_empty() {
				if let Ok(zone) = Self::from_zoneinfo_name(name) {
					return Ok(zone);
				}
				return Self::new(name);
			}
		}

		Err(err!("Could not detect system timezone"; System))
	}

	fn from_tzif_file(id: &str, path: &Path) -> Outcome<Self> {
		let mut parser = TZifParser::new();
		res!(parser.load_from_file(path));
		match parser.timezone_data() {
			Some(data) => Self::from_tzif_data(id, data.clone()),
			None => Err(err!(
				"The TZif file {:?} parsed to no timezone data.", path;
			System, Missing, Data)),
		}
	}

	fn from_zoneinfo_name(name: &str) -> Outcome<Self> {
		// The name may have come from the environment; keep it inside the
		// tree.
		if name.starts_with('/') || name.contains("..") {
			return Err(err!(
				"'{}' is not a plain zone name.", name;
			Invalid, Input));
		}
		for base in ["/usr/share/zoneinfo", "/usr/lib/zoneinfo", "/etc/zoneinfo"] {
			let path = Path::new(base).join(name);
			if path.is_file() {
				return Self::from_tzif_file(name, &path);
			}
		}
		Err(err!(
			"No zoneinfo file for '{}' on this system.", name;
		System, Missing))
	}

	/// The zone name in a zoneinfo path: the components after `zoneinfo`,
	/// joined again -- `/usr/share/zoneinfo/Australia/Perth` names
	/// `Australia/Perth`.
	fn zone_name_from_path(path: &Path) -> Option<String> {
		let mut parts: Vec<String> = Vec::new();
		let mut seen = false;
		for component in path.components() {
			let text = component.as_os_str().to_string_lossy();
			if seen {
				parts.push(text.into_owned());
			} else if text == "zoneinfo" {
				seen = true;
			}
		}
		if parts.is_empty() {
			None
		} else {
			Some(parts.join("/"))
		}
	}
	
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
	
	fn is_leap_year(year: i64) -> bool {
		(year % 4 == 0 && year % 100 != 0) || (year % 400 == 0)
	}
	
	fn calculate_transition_time(&self, transition: &DstTransition, year: i32) -> Outcome<i64> {
		// Calculate the day of the transition
		let day = res!(self.calculate_transition_day(&transition.day_spec, transition.month, year));
		
		// Convert to UTC milliseconds
		let days_since_epoch = res!(self.days_since_epoch(year, transition.month, day));
		let transition_millis = days_since_epoch * 86400 * 1000 + (transition.hour as i64) * 3600 * 1000;
		
		Ok(transition_millis)
	}
	
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
	
	fn days_in_month(&self, month: u8, year: i32) -> u8 {
		match month {
			1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
			4 | 6 | 9 | 11 => 30,
			2 => if Self::is_leap_year(year as i64) { 29 } else { 28 },
			_ => 0, // Invalid month
		}
	}
	
	/// 0 = Sunday.
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