use crate::{
    cache::string_intern::convenience as intern,
    calendar::CalendarDate,
    clock::ClockTime,
    constant::{DayOfWeek, MonthOfYear},
    format::{FormatPattern, FormatToken, FormatStyle, Locale},
    time::CalClock,
};

use oxedyne_fe2o3_core::prelude::*;

use std::{fmt, collections::HashMap};

/// Error type for formatting operations.
#[derive(Clone, Debug)]
pub enum FormattingError {
    InvalidPattern(String),
    UnsupportedToken(String),
    InvalidValue(String),
}

impl fmt::Display for FormattingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FormattingError::InvalidPattern(msg) => write!(f, "Invalid pattern: {}", msg),
            FormattingError::UnsupportedToken(msg) => write!(f, "Unsupported token: {}", msg),
            FormattingError::InvalidValue(msg) => write!(f, "Invalid value: {}", msg),
        }
    }
}

/// A comprehensive formatter for CalClock, CalendarDate, and ClockTime.
///
/// The CalClockFormatter provides flexible formatting capabilities with support
/// for custom patterns, predefined formats, and locale-aware formatting.
///
/// # Examples
///
/// ```ignore
/// use oxedyne_fe2o3_datime::{
///     format::{CalClockFormatter, FormatPattern},
///     time::CalClock,
/// }res!();
///
/// let calclock = CalClock::now_utc()?res!();
/// let formatter = CalClockFormatter::new()res!();
///
/// // Using predefined patterns
/// let iso_string = formatter.format_with_pattern(&calclock, &FormatPattern::iso_datetime())?res!();
/// let us_string = formatter.format_with_pattern(&calclock, &FormatPattern::us_date())?res!();
///
/// // Using custom patterns
/// let custom_pattern = FormatPattern::new("EEEE, MMMM d, yyyy 'at' h:mm a")?res!();
/// let custom_string = formatter.format_with_pattern(&calclock, &custom_pattern)?res!();
/// ```
#[derive(Debug, Clone)]
pub struct CalClockFormatter {
    /// The locale for formatting (affects month/day names, date order, etc.).
    #[allow(dead_code)]
    locale: Option<Locale>,
    /// Custom month names override (for localization).
    month_names: Option<HashMap<MonthOfYear, (String, String)>>, // (short, long)
    /// Custom day names override (for localization).
    day_names: Option<HashMap<DayOfWeek, (String, String)>>, // (short, long)
    /// Custom AM/PM markers.
    am_pm_markers: Option<(String, String)>, // (AM, PM)
    /// Whether to use ordinal suffixes (1st, 2nd, 3rd).
    use_ordinals: bool,
}

impl CalClockFormatter {
    /// Creates a new formatter with default settings.
    pub fn new() -> Self {
        Self {
            locale: None,
            month_names: None,
            day_names: None,
            am_pm_markers: None,
            use_ordinals: false,
        }
    }
    
    /// Creates a new formatter with the specified locale.
    pub fn with_locale(locale: Locale) -> Self {
        Self {
            locale: Some(locale),
            month_names: None,
            day_names: None,
            am_pm_markers: None,
            use_ordinals: false,
        }
    }
    
    /// Sets custom month names for localization.
    pub fn set_month_names(mut self, names: HashMap<MonthOfYear, (String, String)>) -> Self {
        self.month_names = Some(names);
        self
    }
    
    /// Sets custom day names for localization.
    pub fn set_day_names(mut self, names: HashMap<DayOfWeek, (String, String)>) -> Self {
        self.day_names = Some(names);
        self
    }
    
    /// Sets custom AM/PM markers.
    pub fn set_am_pm_markers(mut self, am: String, pm: String) -> Self {
        self.am_pm_markers = Some((am, pm));
        self
    }
    
    /// Enables ordinal suffixes (1st, 2nd, 3rd) for day formatting.
    pub fn enable_ordinals(mut self) -> Self {
        self.use_ordinals = true;
        self
    }
    
    /// Formats a CalClock using the specified pattern.
    pub fn format_with_pattern(&self, calclock: &CalClock, pattern: &FormatPattern) -> Outcome<String> {
        let mut result = String::new();
        
        for token in pattern.tokens() {
            let formatted = res!(self.format_token(token, calclock));
            result.push_str(&formatted);
        }
        
        Ok(result)
    }
    
    /// Formats a CalendarDate using the specified pattern.
    pub fn format_date_with_pattern(&self, date: &CalendarDate, pattern: &FormatPattern) -> Outcome<String> {
        let mut result = String::new();
        
        for token in pattern.tokens() {
            let formatted = res!(self.format_date_token(token, date));
            result.push_str(&formatted);
        }
        
        Ok(result)
    }
    
    /// Formats a ClockTime using the specified pattern.
    pub fn format_time_with_pattern(&self, time: &ClockTime, pattern: &FormatPattern) -> Outcome<String> {
        let mut result = String::new();
        
        for token in pattern.tokens() {
            let formatted = res!(self.format_time_token(token, time));
            result.push_str(&formatted);
        }
        
        Ok(result)
    }
    
    /// Formats a CalClock using a pattern string.
    pub fn format(&self, calclock: &CalClock, pattern_string: &str) -> Outcome<String> {
        let pattern = res!(FormatPattern::new(pattern_string));
        self.format_with_pattern(calclock, &pattern)
    }
    
    /// Formats a CalClock using a locale's default datetime pattern.
    pub fn format_with_locale(&self, calclock: &CalClock, locale: &Locale) -> Outcome<String> {
        self.format_with_pattern(calclock, locale.datetime_pattern())
    }
    
    /// Formats a CalClock date using a locale's default date pattern.
    pub fn format_date_with_locale(&self, calclock: &CalClock, locale: &Locale) -> Outcome<String> {
        self.format_with_pattern(calclock, locale.date_pattern())
    }
    
    /// Formats a CalClock time using a locale's default time pattern.
    pub fn format_time_with_locale(&self, calclock: &CalClock, locale: &Locale) -> Outcome<String> {
        self.format_with_pattern(calclock, locale.time_pattern())
    }
    
    /// Formats a CalendarDate using a pattern string.
    pub fn format_date(&self, date: &CalendarDate, pattern_string: &str) -> Outcome<String> {
        let pattern = res!(FormatPattern::new(pattern_string));
        self.format_date_with_pattern(date, &pattern)
    }
    
    /// Formats a ClockTime using a pattern string.
    pub fn format_time(&self, time: &ClockTime, pattern_string: &str) -> Outcome<String> {
        let pattern = res!(FormatPattern::new(pattern_string));
        self.format_time_with_pattern(time, &pattern)
    }
    
    // ========================================================================
    // Token Formatting Implementation
    // ========================================================================
    
    /// Formats a single token for a CalClock.
    fn format_token(&self, token: &FormatToken, calclock: &CalClock) -> Outcome<String> {
        match token {
            // Date tokens
            FormatToken::Year(style) => self.format_year(calclock.year(), style),
            FormatToken::Month(style) => self.format_month(calclock.month_of_year(), style),
            FormatToken::Day(style) => self.format_day(calclock.day(), style),
            FormatToken::DayOfWeek(style) => self.format_day_of_week(calclock.day_of_week(), style),
            FormatToken::DayOfYear(style) => {
                let day_of_year = res!(calclock.day_of_year());
                self.format_day_of_year(day_of_year, style)
            },
            FormatToken::Quarter(style) => self.format_quarter(calclock.date().quarter(), style),
            FormatToken::WeekOfYear(style) => {
                let week_of_year = res!(self.calculate_week_of_year(calclock.date()));
                self.format_week_of_year(week_of_year, style)
            },
            FormatToken::Era(style) => self.format_era(calclock.date(), style),
            
            // Time tokens
            FormatToken::Hour12(style) => {
                let hour_12 = if calclock.hour() == 0 { 12 } else if calclock.hour() > 12 { calclock.hour() - 12 } else { calclock.hour() };
                self.format_hour(hour_12, style)
            },
            FormatToken::Hour24(style) => self.format_hour(calclock.hour(), style),
            FormatToken::Minute(style) => self.format_minute(calclock.minute(), style),
            FormatToken::Second(style) => self.format_second(calclock.second(), style),
            FormatToken::Millisecond(style) => self.format_millisecond(calclock.millisecond(), style),
            FormatToken::Microsecond(style) => self.format_microsecond(calclock.microsecond(), style),
            FormatToken::Nanosecond(style) => self.format_nanosecond(calclock.nanosecond(), style),
            FormatToken::AmPm(style) => self.format_am_pm(calclock.hour() >= 12, style),
            
            // Timezone tokens
            FormatToken::TimezoneId(style) => self.format_timezone_id(calclock.zone().id(), style),
            FormatToken::TimezoneOffset(style) => {
                // Get the offset for the current time
                let millis = res!(calclock.to_millis());
                let offset_millis = res!(calclock.zone().offset_millis_at_time(millis));
                self.format_timezone_offset(offset_millis, style)
            },
            FormatToken::TimezoneName(style) => self.format_timezone_name(calclock.zone().id(), style),
            
            // Special characters
            FormatToken::Space => Ok(" ".to_string()),
            FormatToken::Colon => Ok(":".to_string()),
            FormatToken::Dash => Ok("-".to_string()),
            FormatToken::Slash => Ok("/".to_string()),
            FormatToken::Comma => Ok(",".to_string()),
            FormatToken::Period => Ok(".".to_string()),
            
            // Literal text
            FormatToken::Literal(text) => Ok(text.clone()),
        }
    }
    
    /// Formats a single token for a CalendarDate.
    fn format_date_token(&self, token: &FormatToken, date: &CalendarDate) -> Outcome<String> {
        match token {
            FormatToken::Year(style) => self.format_year(date.year(), style),
            FormatToken::Month(style) => self.format_month(date.month_of_year(), style),
            FormatToken::Day(style) => self.format_day(date.day(), style),
            FormatToken::DayOfWeek(style) => self.format_day_of_week(date.day_of_week(), style),
            FormatToken::DayOfYear(style) => {
                let day_of_year = res!(date.day_of_year());
                self.format_day_of_year(day_of_year, style)
            },
            FormatToken::Quarter(style) => self.format_quarter(date.quarter(), style),
            FormatToken::WeekOfYear(style) => {
                let week_of_year = res!(self.calculate_week_of_year(date));
                self.format_week_of_year(week_of_year, style)
            },
            FormatToken::Era(style) => self.format_era(date, style),
            
            // Special characters
            FormatToken::Space => Ok(" ".to_string()),
            FormatToken::Colon => Ok(":".to_string()),
            FormatToken::Dash => Ok("-".to_string()),
            FormatToken::Slash => Ok("/".to_string()),
            FormatToken::Comma => Ok(",".to_string()),
            FormatToken::Period => Ok(".".to_string()),
            
            // Literal text
            FormatToken::Literal(text) => Ok(text.clone()),
            
            // Invalid for date-only formatting
            _ => Err(err!("Invalid token for CalendarDate: {:?}", token; Invalid, Input)),
        }
    }
    
    /// Formats a single token for a ClockTime.
    fn format_time_token(&self, token: &FormatToken, time: &ClockTime) -> Outcome<String> {
        match token {
            FormatToken::Hour12(style) => {
                let hour_12 = if time.hour().of() == 0 { 12 } else if time.hour().of() > 12 { time.hour().of() - 12 } else { time.hour().of() };
                self.format_hour(hour_12, style)
            },
            FormatToken::Hour24(style) => self.format_hour(time.hour().of(), style),
            FormatToken::Minute(style) => self.format_minute(time.minute().of(), style),
            FormatToken::Second(style) => self.format_second(time.second().of(), style),
            FormatToken::Millisecond(style) => {
                let millis = (time.nanosecond().of() / 1_000_000) as u16;
                self.format_millisecond(millis, style)
            },
            FormatToken::Microsecond(style) => {
                let micros = ((time.nanosecond().of() % 1_000_000) / 1_000) as u16;
                self.format_microsecond(micros, style)
            },
            FormatToken::Nanosecond(style) => self.format_nanosecond(time.nanosecond().of(), style),
            FormatToken::AmPm(style) => self.format_am_pm(time.hour().of() >= 12, style),
            
            // Special characters
            FormatToken::Space => Ok(" ".to_string()),
            FormatToken::Colon => Ok(":".to_string()),
            FormatToken::Dash => Ok("-".to_string()),
            FormatToken::Slash => Ok("/".to_string()),
            FormatToken::Comma => Ok(",".to_string()),
            FormatToken::Period => Ok(".".to_string()),
            
            // Literal text
            FormatToken::Literal(text) => Ok(text.clone()),
            
            // Invalid for time-only formatting
            _ => Err(err!("Invalid token for ClockTime: {:?}", token; Invalid, Input)),
        }
    }
    
    // ========================================================================
    // Individual Component Formatters
    // ========================================================================
    
    fn format_year(&self, year: i32, style: &FormatStyle) -> Outcome<String> {
        match style {
            FormatStyle::Numeric => Ok(year.to_string()),
            FormatStyle::Custom { width, pad_char: _ } => {
                match width {
                    2 => Ok(format!("{:02}", year % 100)),
                    4 => Ok(format!("{:04}", year)),
                    _ => Ok(format!("{:0width$}", year, width = width)),
                }
            },
            _ => Ok(year.to_string()),
        }
    }
    
    fn format_month(&self, month: MonthOfYear, style: &FormatStyle) -> Outcome<String> {
        match style {
            FormatStyle::Numeric => Ok(month.of().to_string()),
            FormatStyle::Short => {
                if let Some(ref names) = self.month_names {
                    if let Some((short, _)) = names.get(&month) {
                        return Ok(short.clone());
                    }
                }
                Ok((*intern::intern_month(month.short_name())).clone())
            },
            FormatStyle::Long => {
                if let Some(ref names) = self.month_names {
                    if let Some((_, long)) = names.get(&month) {
                        return Ok(long.clone());
                    }
                }
                Ok((*intern::intern_month(month.long_name())).clone())
            },
            FormatStyle::Custom { width, pad_char: _ } => {
                Ok(format!("{:0width$}", month.of(), width = width))
            },
            _ => Ok((*intern::intern_month(month.short_name())).clone()),
        }
    }
    
    fn format_day(&self, day: u8, style: &FormatStyle) -> Outcome<String> {
        match style {
            FormatStyle::Numeric => {
                if self.use_ordinals {
                    let ordinal = match day {
                        1 | 21 | 31 => format!("{}st", day),
                        2 | 22 => format!("{}nd", day),
                        3 | 23 => format!("{}rd", day),
                        _ => format!("{}th", day),
                    };
                    Ok(ordinal)
                } else {
                    Ok(day.to_string())
                }
            },
            FormatStyle::Custom { width, pad_char: _ } => {
                Ok(format!("{:0width$}", day, width = width))
            },
            _ => {
                if self.use_ordinals {
                    let ordinal = match day {
                        1 | 21 | 31 => format!("{}st", day),
                        2 | 22 => format!("{}nd", day),
                        3 | 23 => format!("{}rd", day),
                        _ => format!("{}th", day),
                    };
                    Ok(ordinal)
                } else {
                    Ok(day.to_string())
                }
            },
        }
    }
    
    fn format_day_of_week(&self, day_of_week: DayOfWeek, style: &FormatStyle) -> Outcome<String> {
        match style {
            FormatStyle::Short => {
                if let Some(ref names) = self.day_names {
                    if let Some((short, _)) = names.get(&day_of_week) {
                        return Ok(short.clone());
                    }
                }
                Ok((*intern::intern_day(day_of_week.short_name())).clone())
            },
            FormatStyle::Long => {
                if let Some(ref names) = self.day_names {
                    if let Some((_, long)) = names.get(&day_of_week) {
                        return Ok(long.clone());
                    }
                }
                Ok((*intern::intern_day(day_of_week.long_name())).clone())
            },
            _ => Ok((*intern::intern_day(day_of_week.short_name())).clone()),
        }
    }
    
    fn format_day_of_year(&self, day_of_year: u16, style: &FormatStyle) -> Outcome<String> {
        match style {
            FormatStyle::Numeric => Ok(day_of_year.to_string()),
            FormatStyle::Custom { width, pad_char: _ } => {
                Ok(format!("{:0width$}", day_of_year, width = width))
            },
            _ => Ok(day_of_year.to_string()),
        }
    }
    
    fn format_quarter(&self, quarter: u8, style: &FormatStyle) -> Outcome<String> {
        match style {
            _ => Ok(quarter.to_string()),
        }
    }
    
    fn format_hour(&self, hour: u8, style: &FormatStyle) -> Outcome<String> {
        match style {
            FormatStyle::Numeric => Ok(hour.to_string()),
            FormatStyle::Custom { width, pad_char: _ } => {
                Ok(format!("{:0width$}", hour, width = width))
            },
            _ => Ok(hour.to_string()),
        }
    }
    
    fn format_minute(&self, minute: u8, style: &FormatStyle) -> Outcome<String> {
        match style {
            FormatStyle::Numeric => Ok(minute.to_string()),
            FormatStyle::Custom { width, pad_char: _ } => {
                Ok(format!("{:0width$}", minute, width = width))
            },
            _ => Ok(minute.to_string()),
        }
    }
    
    fn format_second(&self, second: u8, style: &FormatStyle) -> Outcome<String> {
        match style {
            FormatStyle::Numeric => Ok(second.to_string()),
            FormatStyle::Custom { width, pad_char: _ } => {
                Ok(format!("{:0width$}", second, width = width))
            },
            _ => Ok(second.to_string()),
        }
    }
    
    fn format_millisecond(&self, millisecond: u16, style: &FormatStyle) -> Outcome<String> {
        match style {
            FormatStyle::Custom { width, pad_char: _ } => {
                Ok(format!("{:0width$}", millisecond, width = width))
            },
            _ => Ok(format!("{:03}", millisecond)),
        }
    }
    
    fn format_microsecond(&self, microsecond: u16, style: &FormatStyle) -> Outcome<String> {
        match style {
            FormatStyle::Custom { width, pad_char: _ } => {
                Ok(format!("{:0width$}", microsecond, width = width))
            },
            _ => Ok(format!("{:03}", microsecond)),
        }
    }
    
    fn format_nanosecond(&self, nanosecond: u32, style: &FormatStyle) -> Outcome<String> {
        match style {
            FormatStyle::Custom { width, pad_char: _ } => {
                Ok(format!("{:0width$}", nanosecond, width = width))
            },
            _ => Ok(format!("{:09}", nanosecond)),
        }
    }
    
    fn format_am_pm(&self, is_pm: bool, _style: &FormatStyle) -> Outcome<String> {
        if let Some((ref am, ref pm)) = self.am_pm_markers {
            Ok(if is_pm { pm.clone() } else { am.clone() })
        } else {
            let marker = if is_pm { "PM" } else { "AM" };
            Ok((*intern::intern(marker)).clone())
        }
    }
    
    fn format_timezone_id(&self, timezone_id: &str, _style: &FormatStyle) -> Outcome<String> {
        Ok(timezone_id.to_string())
    }
    
    fn format_timezone_offset(&self, offset_millis: i32, _style: &FormatStyle) -> Outcome<String> {
        let offset_seconds = offset_millis / 1000;
        let offset_hours = offset_seconds / 3600;
        let offset_mins = (offset_seconds % 3600) / 60;
        
        let sign = if offset_seconds >= 0 { "+" } else { "-" };
        let abs_hours = offset_hours.abs();
        let abs_mins = offset_mins.abs();
        
        Ok(format!("{}{:02}{:02}", sign, abs_hours, abs_mins))
    }
    
    fn format_timezone_name(&self, timezone_id: &str, _style: &FormatStyle) -> Outcome<String> {
        // Map common timezone IDs to abbreviations
        let abbrev = match timezone_id {
            "America/New_York" => "EST",
            "America/Chicago" => "CST",
            "America/Denver" => "MST",
            "America/Los_Angeles" => "PST",
            "Europe/London" => "GMT",
            "Europe/Paris" => "CET",
            "Asia/Tokyo" => "JST",
            "Australia/Sydney" => "AEDT",
            _ => timezone_id,
        };
        Ok((*intern::intern_timezone(abbrev)).clone())
    }
    
    fn format_week_of_year(&self, week: u8, style: &FormatStyle) -> Outcome<String> {
        match style {
            FormatStyle::Numeric => Ok(week.to_string()),
            FormatStyle::Custom { width, pad_char: _ } => {
                Ok(format!("{:0width$}", week, width = width))
            },
            _ => Ok(week.to_string()),
        }
    }
    
    fn format_era(&self, date: &CalendarDate, style: &FormatStyle) -> Outcome<String> {
        // Use calendar-specific era handling
        // Since CalendarSystem only has Gregorian and Julian currently,
        // we'll determine the calendar type from the internal Calendar enum if available
        // For now, default to CE/BCE for all calendar systems
        let era_text = match style {
            FormatStyle::Short => {
                if date.year() > 0 { "CE" } else { "BCE" }
            },
            FormatStyle::Long => {
                if date.year() > 0 { "Common Era" } else { "Before Common Era" }
            },
            _ => {
                if date.year() > 0 { "CE" } else { "BCE" }
            },
        };
        
        Ok((*intern::intern(era_text)).clone())
    }
    
    fn calculate_week_of_year(&self, date: &CalendarDate) -> Outcome<u8> {
        // ISO 8601 week calculation
        // Week 1 is the first week with Thursday in the new year
        let jan_1 = res!(CalendarDate::from_ymd(date.year(), MonthOfYear::January, 1, date.zone().clone()));
        let jan_1_dow = jan_1.day_of_week();
        
        // Find the Monday of week 1
        let days_from_monday = match jan_1_dow {
            DayOfWeek::Monday => 0,
            DayOfWeek::Tuesday => 1,
            DayOfWeek::Wednesday => 2,
            DayOfWeek::Thursday => 3,
            DayOfWeek::Friday => 4,
            DayOfWeek::Saturday => 5,
            DayOfWeek::Sunday => 6,
        };
        
        let week_1_monday = if days_from_monday <= 3 {
            // January 1 is in week 1
            res!(jan_1.add_days(-(days_from_monday as i32)))
        } else {
            // January 1 is in the last week of the previous year
            res!(jan_1.add_days((7 - days_from_monday) as i32))
        };
        
        // Calculate the number of days from week 1 Monday to the date
        let date_day_number = res!(date.to_day_number());
        let week_1_day_number = res!(week_1_monday.to_day_number());
        let days_diff = date_day_number - week_1_day_number;
        
        if days_diff < 0 {
            // Date is in the last week of the previous year
            // Calculate the week number for the previous year
            let prev_year_date = res!(CalendarDate::from_ymd(date.year() - 1, MonthOfYear::December, 31, date.zone().clone()));
            self.calculate_week_of_year(&prev_year_date)
        } else {
            let week_number = (days_diff / 7) + 1;
            Ok(week_number as u8)
        }
    }
}

impl Default for CalClockFormatter {
    fn default() -> Self {
        Self::new()
    }
}