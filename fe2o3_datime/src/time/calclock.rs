//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use crate::{
	calendar::{Calendar, CalendarDate},
	clock::{ClockDuration, ClockTime},
	constant::{DayOfWeek, MonthOfYear},
	time::{CalClockDuration, CalClockZone, LeapSecondConfig},
};

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_jdat::{prelude::*, tup2dat};

use std::{
	cmp::Ordering,
	fmt,
};

/// A CalendarDate and a ClockTime together, holding the invariant that both use
/// the same zone.
///
/// ```ignore
/// use oxedyne_fe2o3_datime::{
///     calendar::CalendarDate,
///     clock::ClockTime,
///     time::{CalClock, CalClockZone},
/// }res!();
///
/// let zone = CalClockZone::utc()res!();
/// let calclock = res!(CalClock::new(2024, 6, 15, 14, 30, 0, 0, zone))res!();
/// assert_eq!(calclock.date().year(), 2024)res!();
/// assert_eq!(calclock.time().hour().of(), 14)res!();
/// ```
#[derive(Clone, Debug, PartialEq, Hash)]
pub struct CalClock {
	date: CalendarDate,
	time: ClockTime,
}

impl CalClock {
	pub fn from_date_time(date: CalendarDate, time: ClockTime) -> Outcome<Self> {
		// Ensure both components use the same time zone.
		if date.zone() != time.zone() {
			return Err(err!("Date and time must use the same time zone"; Invalid, Input));
		}
		
		Ok(Self { date, time })
	}
	
	pub fn new(
		year: i32,
		month: u8,
		day: u8,
		hour: u8,
		minute: u8,
		second: u8,
		nanosecond: u32,
		zone: CalClockZone,
	) -> Outcome<Self> {
		let calendar = Calendar::new(); // Default to Gregorian
		let date = res!(calendar.date(year, month, day, zone.clone()));
		let time = res!(ClockTime::new(hour, minute, second, nanosecond, zone));
		Self::from_date_time(date, time)
	}

	/// Second 60 is accepted only where the configuration allows it and the leap
	/// second table agrees.
	pub fn new_with_leap_seconds(
		year: i32,
		month: u8,
		day: u8,
		hour: u8,
		minute: u8,
		second: u8,
		nanosecond: u32,
		zone: CalClockZone,
		config: &LeapSecondConfig,
	) -> Outcome<Self> {
		let calendar = Calendar::new(); // Default to Gregorian
		let date = res!(calendar.date(year, month, day, zone.clone()));
		
		// For leap second validation, we need to check the complete date/time context
		if second == 60 && config.validate_leap_seconds {
			let table = config.get_table();
			if !table.validate_leap_second(year, month, day, hour, minute) {
				return Err(err!(
					"Invalid leap second: {}-{:02}-{:02} {}:{:02}:60 is not a valid leap second according to the leap second table", 
					year, month, day, hour, minute; 
					Invalid, Input
				));
			}
		}
		
		let time = res!(ClockTime::new_with_leap_seconds(hour, minute, second, nanosecond, zone, config));
		Self::from_date_time(date, time)
	}
	
	pub fn date(&self) -> &CalendarDate {
		&self.date
	}
	
	pub fn time(&self) -> &ClockTime {
		&self.time
	}
	
	pub fn zone(&self) -> &CalClockZone {
		self.date.zone()
	}
	
	// ========================================================================
	// Timestamp and Epoch Conversion Methods
	// ========================================================================
	
	/// The zone offset in force at that date and time is applied.
	pub fn to_millis(&self) -> Outcome<i64> {
		let days_since_epoch = res!(self.date.days_since_epoch());
		let millis_from_days = days_since_epoch * 24 * 60 * 60 * 1000;
		let millis_from_time = self.time.millis_of_day();
		let total_millis = millis_from_days + millis_from_time as i64;
		
		// Adjust for timezone offset
		let offset_millis = res!(self.zone().offset_millis_at_time(total_millis));
		Ok(total_millis - offset_millis as i64)
	}
	
	pub fn to_seconds(&self) -> Outcome<i64> {
		let millis = res!(self.to_millis());
		Ok(millis / 1000)
	}
	
	pub fn to_nanos(&self) -> Outcome<i64> {
		let millis = res!(self.to_millis());
		let nanos_from_millis = millis * 1_000_000;
		let additional_nanos = self.time.nanosecond().of() as i64;
		Ok(nanos_from_millis + additional_nanos)
	}
	
	pub fn from_millis(millis: i64, zone: CalClockZone) -> Outcome<Self> {
		// Get timezone offset for this timestamp
		let offset_millis = res!(zone.offset_millis_at_time(millis));
		let local_millis = millis + offset_millis as i64;
		
		// Calculate days since epoch
		let days_since_epoch = local_millis / (24 * 60 * 60 * 1000);
		let millis_of_day = (local_millis % (24 * 60 * 60 * 1000)) as u32;
		
		// Create date from days since epoch
		let date = res!(CalendarDate::from_days_since_epoch(days_since_epoch, zone.clone()));
		
		// Create time from milliseconds of day
		let time = res!(ClockTime::from_millis_of_day(millis_of_day, zone));
		
		Self::from_date_time(date, time)
	}
	
	pub fn from_seconds(seconds: i64, zone: CalClockZone) -> Outcome<Self> {
		Self::from_millis(seconds * 1000, zone)
	}
	
	pub fn from_nanos(nanos: i64, zone: CalClockZone) -> Outcome<Self> {
		let millis = nanos / 1_000_000;
		let mut result = res!(Self::from_millis(millis, zone));
		
		// Set the remaining nanoseconds
		let remaining_nanos = (nanos % 1_000_000) as u32;
		result.time = res!(ClockTime::new(
			result.time.hour().of(),
			result.time.minute().of(),
			result.time.second().of(),
			remaining_nanos,
			result.zone().clone()
		));
		
		Ok(result)
	}
	
	// ========================================================================
	// Date Component Access Methods
	// ========================================================================
	
	pub fn year(&self) -> i32 {
		self.date.year()
	}
	
	pub fn month(&self) -> u8 {
		self.date.month()
	}
	
	pub fn day(&self) -> u8 {
		self.date.day()
	}
	
	pub fn day_of_week(&self) -> DayOfWeek {
		self.date.day_of_week()
	}
	
	pub fn day_of_year(&self) -> Outcome<u16> {
		self.date.day_of_year()
	}
	
	pub fn week_of_year(&self) -> Outcome<u8> {
		self.date.week_of_year()
	}
	
	pub fn month_of_year(&self) -> MonthOfYear {
		self.date.month_of_year()
	}
	
	// ========================================================================
	// Time Component Access Methods
	// ========================================================================
	
	pub fn hour(&self) -> u8 {
		self.time.hour().of()
	}
	
	pub fn minute(&self) -> u8 {
		self.time.minute().of()
	}
	
	pub fn second(&self) -> u8 {
		self.time.second().of()
	}
	
	pub fn nanosecond(&self) -> u32 {
		self.time.nanosecond().of()
	}
	
	pub fn millisecond(&self) -> u16 {
		(self.nanosecond() / 1_000_000) as u16
	}
	
	pub fn microsecond(&self) -> u16 {
		((self.nanosecond() % 1_000_000) / 1_000) as u16
	}
	
	// ========================================================================
	// Leap Second Support Methods
	// ========================================================================
	
	pub fn is_leap_second(&self) -> bool {
		self.time.is_leap_second()
	}

	pub fn is_potential_leap_second(&self) -> bool {
		self.time.is_potential_leap_second()
	}

	/// True also when this is not a leap second at all.
	pub fn validate_leap_second(&self, config: &LeapSecondConfig) -> bool {
		if !self.is_leap_second() {
			return true;
		}

		if !config.enabled || !config.validate_leap_seconds {
			return config.allow_leap_second_parsing;
		}

		let table = config.get_table();
		table.validate_leap_second(self.year(), self.month(), self.day(), self.hour(), self.minute())
	}

	/// UTC plus the TAI-UTC offset in force, which is atomic time.
	pub fn to_tai_timestamp(&self, config: &LeapSecondConfig) -> Outcome<i64> {
		let utc_seconds = res!(self.to_seconds());
		let table = config.get_table();
		Ok(table.utc_to_tai(utc_seconds))
	}

	pub fn from_tai_timestamp(tai_seconds: i64, zone: CalClockZone, config: &LeapSecondConfig) -> Outcome<Self> {
		let table = config.get_table();
		let utc_seconds = res!(table.tai_to_utc(tai_seconds));
		Self::from_seconds(utc_seconds, zone)
	}

	/// Returns the normalised time and whether the date advanced with it.
	pub fn normalize_leap_second(&self) -> Outcome<(Self, bool)> {
		if !self.is_leap_second() {
			return Ok((self.clone(), false));
		}

		// Leap second at 23:59:60 becomes 00:00:00 of next day
		if self.hour() == 23 && self.minute() == 59 {
			let next_day = res!(self.add_days(1));
			let normalized = res!(Self::new(
				next_day.year(),
				next_day.month(),
				next_day.day(),
				0, 0, 0,
				self.nanosecond(),
				self.zone().clone()
			));
			Ok((normalized, true))
		} else {
			Err(err!("Invalid leap second time: leap seconds only valid at 23:59:60"; Invalid, Input))
		}
	}
	
	// ========================================================================
	// Arithmetic and Mutation Methods
	// ========================================================================
	
	pub fn add_duration(&self, duration: &CalClockDuration) -> Outcome<Self> {
		const NANOS_PER_DAY: i64 = 86_400 * 1_000_000_000;
		// Fold the whole duration -- its day field and its nanoseconds together -- and the time already
		// held into a single nanosecond count, then split off whole days once at the end. Adding the
		// date part and the time part in isolation loses the carry: a duration whose nanoseconds run to
		// more than a day (which is how `from_seconds` and thus a Unix timestamp arrive, with every
		// second in the nanosecond field and none in the day field) advances only the time, which wraps
		// at midnight and drops the days; and a duration that pushes the held time past midnight never
		// moves the date. One normalisation with `div_euclid` handles both, and negatives besides --
		// the remainder is always in `[0, NANOS_PER_DAY)`, so it is a valid nanosecond-of-day.
		let held = self.time.to_nanos_of_day() as i64;
		let added = duration.days() as i64 * NANOS_PER_DAY + duration.nanoseconds();
		let total = held + added;
		let day_carry = total.div_euclid(NANOS_PER_DAY);
		let rem = total.rem_euclid(NANOS_PER_DAY);
		let new_date = res!(self.date.add_days(day_carry as i32));
		let new_time = res!(ClockTime::from_nanos_of_day(rem as u64, self.time.zone().clone()));
		Self::from_date_time(new_date, new_time)
	}

	/// The negation of [`add_duration`](Self::add_duration), and routed through it so the same carry
	/// across midnight is handled the same way rather than a second time and differently.
	pub fn subtract_duration(&self, duration: &CalClockDuration) -> Outcome<Self> {
		self.add_duration(&duration.negate())
	}
	
	pub fn add_years(&self, years: i32) -> Outcome<Self> {
		let new_date = res!(self.date.add_years(years));
		Self::from_date_time(new_date, self.time.clone())
	}
	
	pub fn add_months(&self, months: i32) -> Outcome<Self> {
		let new_date = res!(self.date.add_months(months));
		Self::from_date_time(new_date, self.time.clone())
	}
	
	pub fn add_weeks(&self, weeks: i32) -> Outcome<Self> {
		let new_date = res!(self.date.add_days(weeks * 7));
		Self::from_date_time(new_date, self.time.clone())
	}
	
	pub fn add_days(&self, days: i32) -> Outcome<Self> {
		let new_date = res!(self.date.add_days(days));
		Self::from_date_time(new_date, self.time.clone())
	}
	
	pub fn add_hours(&self, hours: i32) -> Outcome<Self> {
		let duration = ClockDuration::from_hours(hours.into());
		let new_time = res!(self.time.add_duration(&duration));
		
		// Handle day overflow
		if new_time.hour().of() < self.time.hour().of() && hours > 0 {
			let new_date = res!(self.date.add_days(1));
			Self::from_date_time(new_date, new_time)
		} else if new_time.hour().of() > self.time.hour().of() && hours < 0 {
			let new_date = res!(self.date.add_days(-1));
			Self::from_date_time(new_date, new_time)
		} else {
			Self::from_date_time(self.date.clone(), new_time)
		}
	}
	
	pub fn add_minutes(&self, minutes: i32) -> Outcome<Self> {
		let duration = ClockDuration::from_minutes(minutes.into());
		let new_time = res!(self.time.add_duration(&duration));
		
		// Handle day overflow/underflow
		let time_diff = new_time.to_nanos_of_day() as i64 - self.time.to_nanos_of_day() as i64;
		let expected_diff = minutes as i64 * 60 * 1_000_000_000;
		
		if time_diff != expected_diff {
			let day_adjustment = if minutes > 0 { 1 } else { -1 };
			let new_date = res!(self.date.add_days(day_adjustment));
			Self::from_date_time(new_date, new_time)
		} else {
			Self::from_date_time(self.date.clone(), new_time)
		}
	}
	
	pub fn add_seconds(&self, seconds: i32) -> Outcome<Self> {
		let duration = ClockDuration::from_seconds(seconds.into());
		let new_time = res!(self.time.add_duration(&duration));
		
		// Handle day overflow/underflow
		let time_diff = new_time.to_nanos_of_day() as i64 - self.time.to_nanos_of_day() as i64;
		let expected_diff = seconds as i64 * 1_000_000_000;
		
		if time_diff != expected_diff {
			let day_adjustment = if seconds > 0 { 1 } else { -1 };
			let new_date = res!(self.date.add_days(day_adjustment));
			Self::from_date_time(new_date, new_time)
		} else {
			Self::from_date_time(self.date.clone(), new_time)
		}
	}
	
	pub fn add_millis(&self, millis: i64) -> Outcome<Self> {
		let duration = ClockDuration::from_millis(millis);
		let new_time = res!(self.time.add_duration(&duration));
		
		// Handle day overflow/underflow
		let time_diff = new_time.to_nanos_of_day() as i64 - self.time.to_nanos_of_day() as i64;
		let expected_diff = millis * 1_000_000;
		
		if time_diff != expected_diff {
			let day_adjustment = if millis > 0 { 1 } else { -1 };
			let new_date = res!(self.date.add_days(day_adjustment));
			Self::from_date_time(new_date, new_time)
		} else {
			Self::from_date_time(self.date.clone(), new_time)
		}
	}
	
	// ========================================================================
	// Time Zone Conversion Methods
	// ========================================================================
	
	/// The instant is preserved; only the local reading of it changes.
	pub fn with_zone(&self, new_zone: CalClockZone) -> Outcome<Self> {
		// Get the UTC timestamp
		let utc_millis = res!(self.to_millis());
		
		// Create new CalClock in the target zone
		Self::from_millis(utc_millis, new_zone)
	}
	
	pub fn is_same_instant(&self, other: &Self) -> Outcome<bool> {
		let self_millis = res!(self.to_millis());
		let other_millis = res!(other.to_millis());
		Ok(self_millis == other_millis)
	}
	
	// ========================================================================
	// Comparison and Validation Methods
	// ========================================================================
	
	pub fn is_leap_year(&self) -> bool {
		self.date.is_leap_year()
	}
	
	pub fn is_valid_date(&self) -> bool {
		self.date.is_valid()
	}
	
	pub fn is_valid_time(&self) -> bool {
		self.time.is_valid()
	}
	
	pub fn is_valid(&self) -> bool {
		self.is_valid_date() && self.is_valid_time()
	}
	
	// ========================================================================
	// Utility Methods
	// ========================================================================
	
	pub fn start_of_day(&self) -> Outcome<Self> {
		let start_time = res!(ClockTime::new(0, 0, 0, 0, self.zone().clone()));
		Self::from_date_time(self.date.clone(), start_time)
	}
	
	pub fn end_of_day(&self) -> Outcome<Self> {
		let end_time = res!(ClockTime::new(23, 59, 59, 999_999_999, self.zone().clone()));
		Self::from_date_time(self.date.clone(), end_time)
	}
	
	pub fn start_of_month(&self) -> Outcome<Self> {
		let calendar = Calendar::new(); // Default to Gregorian
		let start_date = res!(calendar.date(self.year(), self.month(), 1, self.zone().clone()));
		let start_time = res!(ClockTime::new(0, 0, 0, 0, self.zone().clone()));
		Self::from_date_time(start_date, start_time)
	}
	
	pub fn end_of_month(&self) -> Outcome<Self> {
		let days_in_month = res!(self.date.days_in_month());
		let calendar = Calendar::new(); // Default to Gregorian
		let end_date = res!(calendar.date(self.year(), self.month(), days_in_month, self.zone().clone()));
		let end_time = res!(ClockTime::new(23, 59, 59, 999_999_999, self.zone().clone()));
		Self::from_date_time(end_date, end_time)
	}
	
	pub fn start_of_year(&self) -> Outcome<Self> {
		let calendar = Calendar::new(); // Default to Gregorian
		let start_date = res!(calendar.date(self.year(), 1, 1, self.zone().clone()));
		let start_time = res!(ClockTime::new(0, 0, 0, 0, self.zone().clone()));
		Self::from_date_time(start_date, start_time)
	}
	
	pub fn end_of_year(&self) -> Outcome<Self> {
		let calendar = Calendar::new(); // Default to Gregorian
		let end_date = res!(calendar.date(self.year(), 12, 31, self.zone().clone()));
		let end_time = res!(ClockTime::new(23, 59, 59, 999_999_999, self.zone().clone()));
		Self::from_date_time(end_date, end_time)
	}
	
	pub fn at_noon(&self) -> Outcome<Self> {
		let noon_time = res!(ClockTime::new(12, 0, 0, 0, self.zone().clone()));
		Self::from_date_time(self.date.clone(), noon_time)
	}
	
	pub fn at_midnight(&self) -> Outcome<Self> {
		self.start_of_day()
	}
	
	pub fn now(zone: CalClockZone) -> Outcome<Self> {
		use std::time::{SystemTime, UNIX_EPOCH};
		
		let duration = res!(SystemTime::now().duration_since(UNIX_EPOCH)
			.map_err(|e| err!("System time before Unix epoch: {}", e; System)));
		let millis = duration.as_millis() as i64;
		
		Self::from_millis(millis, zone)
	}
	
	pub fn now_utc() -> Outcome<Self> {
		Self::now(CalClockZone::utc())
	}
	
	pub fn now_local() -> Outcome<Self> {
		Self::now(CalClockZone::local())
	}
	
	// ========================================================================
	// Duration Between CalClocks
	// ========================================================================
	
	/// Positive when other is the later of the two.
	pub fn duration_until(&self, other: &Self) -> Outcome<CalClockDuration> {
		let self_nanos = res!(self.to_nanos());
		let other_nanos = res!(other.to_nanos());
		let diff_nanos = other_nanos - self_nanos;
		
		Ok(CalClockDuration::from_nanos(diff_nanos))
	}
	
	pub fn duration_since(&self, other: &Self) -> Outcome<CalClockDuration> {
		other.duration_until(self)
	}
	
	pub fn days_until(&self, other: &Self) -> Outcome<i64> {
		let duration = res!(self.duration_until(other));
		Ok(duration.total_days())
	}
	
	pub fn days_since(&self, other: &Self) -> Outcome<i64> {
		other.days_until(self)
	}
	
	// ========================================================================
	// Comparison Methods
	// ========================================================================
	
	pub fn is_before(&self, other: &Self) -> bool {
		match (self.to_nanos(), other.to_nanos()) {
			(Ok(self_nanos), Ok(other_nanos)) => self_nanos < other_nanos,
			_ => false,
		}
	}
	
	pub fn is_after(&self, other: &Self) -> bool {
		match (self.to_nanos(), other.to_nanos()) {
			(Ok(self_nanos), Ok(other_nanos)) => self_nanos > other_nanos,
			_ => false,
		}
	}
	
	// ========================================================================
	// Formatting Methods
	// ========================================================================
	
	pub fn to_iso8601(&self) -> Outcome<String> {
		// Format: YYYY-MM-DDTHH:MM:SS.nnnnnnnnn+HH:MM
		let date_part = format!("{:04}-{:02}-{:02}", 
			self.year(), self.month(), self.day());
		let time_part = format!("{:02}:{:02}:{:02}.{:09}",
			self.hour(), self.minute(), self.second(), self.nanosecond());
		
		// Get timezone offset
		let offset_millis = res!(self.zone().offset_millis_at_time(res!(self.to_millis())));
		let offset_hours = offset_millis / (60 * 60 * 1000);
		let offset_minutes = (offset_millis.abs() % (60 * 60 * 1000)) / (60 * 1000);
		
		let offset_part = if offset_millis == 0 {
			"Z".to_string()
		} else {
			format!("{:+03}:{:02}", offset_hours, offset_minutes)
		};
		
		Ok(format!("{}T{}{}", date_part, time_part, offset_part))
	}
}

// ========================================================================
// Trait Implementations
// ========================================================================

impl PartialOrd for CalClock {
	fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
		match self.to_nanos() {
			Ok(self_nanos) => match other.to_nanos() {
				Ok(other_nanos) => self_nanos.partial_cmp(&other_nanos),
				Err(_) => None,
			},
			Err(_) => None,
		}
	}
}

// ============================================================================
// JDAT Integration for CalClock
// ============================================================================

impl ToDat for CalClock {
    fn to_dat(&self) -> Outcome<Dat> {
        // Use existing string formatting
        let datetime_string = fmt!("{}", self);
        Ok(Dat::Str(datetime_string))
    }
}

impl FromDat for CalClock {
    /// Accepts either the string or the binary form.
    fn from_dat(dat: Dat) -> Outcome<Self> {
        match dat {
            // String format - use existing ISO parser
            Dat::Str(s) => {
                // Parse ISO 8601 string using existing parser
                Self::parse_iso(&s)
            },
            
            // Binary packed format (i64 nanoseconds since epoch + zone info)
            Dat::Tup2(elements) if elements.len() == 2 => {
                let nanos = match &elements[0] {
                    Dat::I64(n) => n,
                    _ => return Err(err!("Expected i64 nanoseconds in CalClock tuple"; Invalid, Input)),
                };
                
                let zone_str = match &elements[1] {
                    Dat::Str(s) => s,
                    _ => return Err(err!("Expected string for timezone in CalClock tuple"; Invalid, Input)),
                };
                
                let zone = res!(CalClockZone::new(zone_str));
                Self::from_nanos_since_epoch(*nanos, zone)
            },
            
            _ => Err(err!("Expected string or tuple for CalClock"; Invalid, Input)),
        }
    }
}

impl CalClock {
    /// A (nanoseconds since the epoch, zone) tuple, far shorter than the string form.
    pub fn to_dat_binary(&self) -> Outcome<Dat> {
        let nanos = res!(self.to_nanos_since_epoch());
        // Zone as string
        
        Ok(tup2dat!(
            dat!(nanos),
            dat!(self.zone().to_string()),
        ))
    }
    
    pub fn to_dat_structured(&self) -> Outcome<Dat> {
        Ok(mapdat! {
            "year" => self.year(),
            "month" => self.month(),
            "day" => self.day(),
            "hour" => self.hour(),
            "minute" => self.minute(),
            "second" => self.second(),
            "nanosecond" => self.nanosecond(),
            "timezone" => self.zone().to_string(),
            "calendar" => "gregorian", // Default calendar system
            "datetime_string" => fmt!("{}", self),
            "unix_millis" => res!(self.to_millis()),
        })
    }
    
    /// Accepts "2024-06-23T14:30:15.123456789Z" and "2024-06-23T14:30:15+05:00".
    pub fn parse_iso(input: &str) -> Outcome<Self> {
        // Use existing parser infrastructure
        crate::parser::Parser::parse_datetime(input, CalClockZone::utc())
    }
    
    pub fn to_nanos_since_epoch(&self) -> Outcome<i64> {
        let millis = res!(self.to_millis());
        Ok(millis * 1_000_000 + self.nanosecond() as i64)
    }
    
    pub fn from_nanos_since_epoch(nanos: i64, zone: CalClockZone) -> Outcome<Self> {
        let millis = nanos / 1_000_000;
        let remaining_nanos = (nanos % 1_000_000) as u32;
        
        let mut calclock = res!(Self::from_millis(millis, zone));
        
        // Set the nanosecond component
        calclock.time = res!(ClockTime::new(
            calclock.hour(),
            calclock.minute(),
            calclock.second(),
            remaining_nanos,
            calclock.zone().clone()
        ));
        
        Ok(calclock)
    }
    
    // ========================================================================
    // ADDITIONAL UTILITY METHODS FOR 100% JAVA COMPATIBILITY
    // ========================================================================
    
    pub fn plus_all_components(
        &self,
        inc_year: i32,
        inc_month: i32,
        inc_day: i32,
        inc_hour: i64,
        inc_minute: i64,
        inc_second: i64,
        inc_nanosecond: i64,
    ) -> Outcome<Self> {
        // Start with date arithmetic (years, months, days)
        let mut result_date = res!(self.date.plus(inc_year, inc_month, inc_day));
        
        // Handle time components with proper carry
        let total_nanos = (self.time.nanosecond().of() as i64) + inc_nanosecond;
        let total_seconds = self.time.second().of() as i64 + inc_second + (total_nanos / 1_000_000_000);
        let remaining_nanos = (total_nanos % 1_000_000_000) as u32;
        
        let total_minutes = self.time.minute().of() as i64 + inc_minute + (total_seconds / 60);
        let remaining_seconds = (total_seconds % 60) as u8;
        
        let total_hours = self.time.hour().of() as i64 + inc_hour + (total_minutes / 60);
        let remaining_minutes = (total_minutes % 60) as u8;
        
        let day_overflow = total_hours / 24;
        let remaining_hours = (total_hours % 24) as u8;
        
        // Apply day overflow to date
        if day_overflow != 0 {
            result_date = res!(result_date.add_days(day_overflow as i32));
        }
        
        // Create new time with remaining components
        let result_time = res!(ClockTime::new(
            remaining_hours,
            remaining_minutes,
            remaining_seconds,
            remaining_nanos,
            self.zone().clone()
        ));
        
        Ok(Self {
            date: result_date,
            time: result_time,
        })
    }
    
    pub fn format(&self, pattern: &str) -> Outcome<String> {
        // Create a basic formatter for now - can be enhanced later
        match pattern {
            "ISO" | "iso" => Ok(format!("{}T{}", self.date, self.time)),
            "DEBUG" | "debug" => Ok(self.to_debug()),
            _ => {
                // For custom patterns, use a simple implementation for now
                Ok(format!("{} {}", self.date, self.time))
            }
        }
    }
    
    pub fn to_debug(&self) -> String {
        format!(
            "CalClock[{}-{:02}-{:02} {:02}:{:02}:{:02}.{:09} {}]",
            self.date.year(),
            self.date.month_of_year().of(),
            self.date.day(),
            self.time.hour().of(),
            self.time.minute().of(),
            self.time.second().of(),
            self.time.nanosecond(),
            self.zone().id()
        )
    }
    
    pub fn previous_day_of_week(&self, dow: DayOfWeek) -> Outcome<Self> {
        let current_dow = self.date.day_of_week();
        let days_back = match current_dow.days_until(&dow) {
            0 => 7, // Same day, go back a full week
            n => n,
        };
        self.add_days(-(days_back as i32))
    }
    
    pub fn next_day_of_week(&self, dow: DayOfWeek) -> Outcome<Self> {
        let current_dow = self.date.day_of_week();
        let days_forward = match dow.days_until(&current_dow) {
            0 => 7, // Same day, go forward a full week
            n => n,
        };
        self.add_days(days_forward as i32)
    }
    
    pub fn this_or_previous_day_of_week(&self, dow: DayOfWeek) -> Outcome<Self> {
        let current_dow = self.date.day_of_week();
        if current_dow == dow {
            Ok(self.clone())
        } else {
            self.previous_day_of_week(dow)
        }
    }
    
    pub fn this_or_next_day_of_week(&self, dow: DayOfWeek) -> Outcome<Self> {
        let current_dow = self.date.day_of_week();
        if current_dow == dow {
            Ok(self.clone())
        } else {
            self.next_day_of_week(dow)
        }
    }
    
    pub fn abs_diff(&self, other: &Self) -> Outcome<CalClockDuration> {
        let diff = res!(self.duration_until(other));
        if diff.nanoseconds() < 0 {
            Ok(diff.negate())
        } else {
            Ok(diff)
        }
    }
    
    pub fn to_midnight(&self) -> Outcome<Self> {
        let next_day = res!(self.date.add_days(1));
        let midnight_time = res!(ClockTime::new(0, 0, 0, 0, self.zone().clone()));
        Ok(Self {
            date: next_day,
            time: midnight_time,
        })
    }
    
    pub fn is_within_seconds(&self, other: &Self, tolerance_seconds: f64) -> Outcome<bool> {
        let diff = res!(self.abs_diff(other));
        let diff_seconds = diff.total_seconds() as f64 + (diff.nanoseconds() as f64 / 1_000_000_000.0);
        Ok(diff_seconds <= tolerance_seconds)
    }
    
    pub fn round_to_millis(&self) -> Outcome<Self> {
        let millis = (self.time.nanosecond().of() + 500_000) / 1_000_000; // Round to nearest ms
        let rounded_nanos = millis * 1_000_000;
        
        let new_time = res!(ClockTime::new(
            self.time.hour().of(),
            self.time.minute().of(),
            self.time.second().of(),
            rounded_nanos,
            self.zone().clone()
        ));
        
        Ok(Self {
            date: self.date.clone(),
            time: new_time,
        })
    }
    
    pub fn zero_nanoseconds(&self) -> Outcome<Self> {
        let new_time = res!(ClockTime::new(
            self.time.hour().of(),
            self.time.minute().of(),
            self.time.second().of(),
            0,
            self.zone().clone()
        ));
        
        Ok(Self {
            date: self.date.clone(),
            time: new_time,
        })
    }
    
    /// Converts to Java time as if the timezone is UTC.
    pub fn to_java_time_as_utc(&self) -> Outcome<i64> {
        let utc_clock = res!(self.to_utc_zone());
        utc_clock.to_java_timestamp()
    }
    
    pub fn as_zone(&self, zone: CalClockZone) -> Outcome<Self> {
        let new_date = res!(CalendarDate::new(
            self.date.year(),
            self.date.month_of_year().of(),
            self.date.day(),
            zone.clone()
        ));
        
        let new_time = res!(ClockTime::new(
            self.time.hour().of(),
            self.time.minute().of(),
            self.time.second().of(),
            self.time.nanosecond().of(),
            zone
        ));
        
        Ok(Self {
            date: new_date,
            time: new_time,
        })
    }
    
    pub fn inc_duration(&self, duration: &CalClockDuration) -> Outcome<Self> {
        self.add_duration(duration)
    }
    
    pub fn inc_days(&self, days: i32) -> Outcome<Self> {
        self.add_days(days)
    }
    
    pub fn is_recognized_format_char(c: char) -> bool {
        match c {
            'Y' | 'y' | 'M' | 'D' | 'd' | 'H' | 'h' | 'm' | 's' | 'S' | 'Z' | 'z' => true,
            _ => false,
        }
    }
    
    pub fn unix_epoch() -> Outcome<Self> {
        Self::new(1970, 1, 1, 0, 0, 0, 0, CalClockZone::utc())
    }
    
    pub fn to_utc_zone(&self) -> Outcome<Self> {
        self.as_zone(CalClockZone::utc())
    }
    
    pub fn from_unix_timestamp_seconds(unix_seconds: i64, zone: CalClockZone) -> Outcome<Self> {
        let epoch = res!(Self::new(1970, 1, 1, 0, 0, 0, 0, CalClockZone::utc()));
        let duration = CalClockDuration::from_seconds(unix_seconds);
        res!(epoch.add_duration(&duration)).as_zone(zone)
    }
    
    pub fn to_java_timestamp(&self) -> Outcome<i64> {
        self.to_millis()
    }
}

impl Ord for CalClock {
	fn cmp(&self, other: &Self) -> Ordering {
		self.partial_cmp(other).unwrap_or(Ordering::Equal)
	}
}

impl Eq for CalClock {}

impl fmt::Display for CalClock {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "{} {}", self.date, self.time)
	}
}

#[cfg(test)]
mod add_duration_tests {
	use super::*;

	/// A Unix timestamp past its first day keeps its days.
	///
	/// The regression this pins: `from_unix_timestamp_seconds` builds the epoch and adds the seconds as
	/// a duration whose day field is zero and whose nanoseconds hold the lot. When the date part read
	/// only the day field, every whole day was dropped and the result was the epoch's date with the
	/// timestamp's time-of-day -- so 2026-07-18T10:00:00Z came back as 1970-01-01T10:00:00Z.
	#[test]
	fn test_a_unix_timestamp_keeps_its_days_00() -> Outcome<()> {
		// 2026-07-18T10:00:00Z.
		let secs = 1_784_368_800_i64;
		let cc = res!(CalClock::from_unix_timestamp_seconds(secs, CalClockZone::utc()));
		assert_eq!(cc.year(), 2026);
		assert_eq!(cc.month(), 7);
		assert_eq!(cc.day(), 18);
		assert_eq!(cc.hour(), 10);
		assert_eq!(cc.minute(), 0);
		assert_eq!(cc.second(), 0);
		// And it round-trips back to the second it came from.
		assert_eq!(res!(cc.to_seconds()), secs);
		Ok(())
	}

	/// The epoch itself is still the epoch.
	#[test]
	fn test_the_epoch_is_the_epoch_01() -> Outcome<()> {
		let cc = res!(CalClock::from_unix_timestamp_seconds(0, CalClockZone::utc()));
		assert_eq!((cc.year(), cc.month(), cc.day()), (1970, 1, 1));
		assert_eq!((cc.hour(), cc.minute(), cc.second()), (0, 0, 0));
		Ok(())
	}

	/// A duration that carries the held time past midnight advances the date, and one that carries it
	/// back before midnight moves the date back. This is the carry the old split-and-add lost even when
	/// the day field was right.
	#[test]
	fn test_a_duration_carries_the_date_across_midnight_02() -> Outcome<()> {
		let z = CalClockZone::utc();
		// 2026-07-18T18:00:00 + 12h = 2026-07-19T06:00:00.
		let base = res!(CalClock::new(2026, 7, 18, 18, 0, 0, 0, z.clone()));
		let fwd = res!(base.add_duration(&CalClockDuration::from_seconds(12 * 3600)));
		assert_eq!((fwd.year(), fwd.month(), fwd.day(), fwd.hour()), (2026, 7, 19, 6));
		// And back again.
		let back = res!(fwd.subtract_duration(&CalClockDuration::from_seconds(12 * 3600)));
		assert_eq!((back.year(), back.month(), back.day(), back.hour()), (2026, 7, 18, 18));
		Ok(())
	}
}