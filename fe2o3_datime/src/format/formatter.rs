use crate::{
    calendar::CalendarDate,
    clock::ClockTime,
    constant::{DayOfWeek, MonthOfYear},
    format::{FormatPattern, FormatToken, FormatStyle, Locale},
    time::CalClock,
};

use oxedyne_fe2o3_core::prelude::*;

use std::fmt;

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
#[derive(Debug)]
pub struct CalClockFormatter {
    // Future: locale settings, custom month/day names, etc.
}

impl CalClockFormatter {
    /// Creates a new formatter with default settings.
    pub fn new() -> Self {
        Self {}
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
            
            // Unsupported for now
            _ => Err(err!("Unsupported token for CalClock: {:?}", token; Unimplemented)),
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
            FormatStyle::Custom { width, pad_char } => {
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
            FormatStyle::Short => Ok(month.short_name().to_string()),
            FormatStyle::Long => Ok(month.long_name().to_string()),
            FormatStyle::Custom { width, pad_char } => {
                Ok(format!("{:0width$}", month.of(), width = width))
            },
            _ => Ok(month.short_name().to_string()),
        }
    }
    
    fn format_day(&self, day: u8, style: &FormatStyle) -> Outcome<String> {
        match style {
            FormatStyle::Numeric => Ok(day.to_string()),
            FormatStyle::Custom { width, pad_char } => {
                Ok(format!("{:0width$}", day, width = width))
            },
            _ => Ok(day.to_string()),
        }
    }
    
    fn format_day_of_week(&self, day_of_week: DayOfWeek, style: &FormatStyle) -> Outcome<String> {
        match style {
            FormatStyle::Short => Ok(day_of_week.short_name().to_string()),
            FormatStyle::Long => Ok(day_of_week.long_name().to_string()),
            _ => Ok(day_of_week.short_name().to_string()),
        }
    }
    
    fn format_day_of_year(&self, day_of_year: u16, style: &FormatStyle) -> Outcome<String> {
        match style {
            FormatStyle::Numeric => Ok(day_of_year.to_string()),
            FormatStyle::Custom { width, pad_char } => {
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
            FormatStyle::Custom { width, pad_char } => {
                Ok(format!("{:0width$}", hour, width = width))
            },
            _ => Ok(hour.to_string()),
        }
    }
    
    fn format_minute(&self, minute: u8, style: &FormatStyle) -> Outcome<String> {
        match style {
            FormatStyle::Numeric => Ok(minute.to_string()),
            FormatStyle::Custom { width, pad_char } => {
                Ok(format!("{:0width$}", minute, width = width))
            },
            _ => Ok(minute.to_string()),
        }
    }
    
    fn format_second(&self, second: u8, style: &FormatStyle) -> Outcome<String> {
        match style {
            FormatStyle::Numeric => Ok(second.to_string()),
            FormatStyle::Custom { width, pad_char } => {
                Ok(format!("{:0width$}", second, width = width))
            },
            _ => Ok(second.to_string()),
        }
    }
    
    fn format_millisecond(&self, millisecond: u16, style: &FormatStyle) -> Outcome<String> {
        match style {
            FormatStyle::Custom { width, pad_char } => {
                Ok(format!("{:0width$}", millisecond, width = width))
            },
            _ => Ok(format!("{:03}", millisecond)),
        }
    }
    
    fn format_microsecond(&self, microsecond: u16, style: &FormatStyle) -> Outcome<String> {
        match style {
            FormatStyle::Custom { width, pad_char } => {
                Ok(format!("{:0width$}", microsecond, width = width))
            },
            _ => Ok(format!("{:03}", microsecond)),
        }
    }
    
    fn format_nanosecond(&self, nanosecond: u32, style: &FormatStyle) -> Outcome<String> {
        match style {
            FormatStyle::Custom { width, pad_char } => {
                Ok(format!("{:0width$}", nanosecond, width = width))
            },
            _ => Ok(format!("{:09}", nanosecond)),
        }
    }
    
    fn format_am_pm(&self, is_pm: bool, style: &FormatStyle) -> Outcome<String> {
        match style {
            FormatStyle::Short => Ok(if is_pm { "PM" } else { "AM" }.to_string()),
            FormatStyle::Long => Ok(if is_pm { "PM" } else { "AM" }.to_string()),
            _ => Ok(if is_pm { "PM" } else { "AM" }.to_string()),
        }
    }
    
    fn format_timezone_id(&self, timezone_id: &str, style: &FormatStyle) -> Outcome<String> {
        Ok(timezone_id.to_string())
    }
    
    fn format_timezone_offset(&self, offset_millis: i32, style: &FormatStyle) -> Outcome<String> {
        let offset_seconds = offset_millis / 1000;
        let offset_hours = offset_seconds / 3600;
        let offset_mins = (offset_seconds % 3600) / 60;
        
        let sign = if offset_seconds >= 0 { "+" } else { "-" };
        let abs_hours = offset_hours.abs();
        let abs_mins = offset_mins.abs();
        
        Ok(format!("{}{:02}{:02}", sign, abs_hours, abs_mins))
    }
    
    fn format_timezone_name(&self, timezone_id: &str, style: &FormatStyle) -> Outcome<String> {
        // For now, just return the timezone ID
        // In the future, this could map to common abbreviations like EST, PST, etc.
        Ok(timezone_id.to_string())
    }
}

impl Default for CalClockFormatter {
    fn default() -> Self {
        Self::new()
    }
}