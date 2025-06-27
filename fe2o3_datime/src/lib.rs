//! A comprehensive date and time library for the Hematite ecosystem.
//!
//! **fe2o3_datime** provides robust date and time handling with modern features including:
//! - **Multiple calendar systems**: Gregorian, Julian, Islamic, Japanese, Thai, Minguo, Holocene
//! - **JDAT serialisation**: String, binary, and structured formats with round-trip compatibility
//! - **Namex integration**: Universal identifiers and efficient local IDs for calendar types
//! - **Timezone support**: Full IANA TZif integration with DST handling
//! - **Nanosecond precision**: Clock operations with sub-second accuracy
//! - **Flexible parsing**: Natural language and ISO 8601 support
//! - **Comprehensive validation**: Business rules and date/time constraints
//!
//! The design emphasizes correctness, performance, and ecosystem integration while
//! maintaining the Hematite philosophy of minimal external dependencies.
//!
//! # Example
//! ```rust,ignore
//! use oxedyne_fe2o3_core::prelude::*;
//! use oxedyne_fe2o3_datime::{
//!     calendar::{Calendar, CalendarDate},
//!     clock::ClockTime,
//!     time::{CalClock, CalClockZone},
//! };
//!
//! let zone = res!(CalClockZone::new("UTC"));
//! let calendar = Calendar::new(); // Default to Gregorian
//! let date = res!(calendar.date(2024, 3, 15, zone));
//! let time = res!(ClockTime::new(14, 30, 0, 0, zone));
//! let calclock = res!(CalClock::from_date_time(date, time));
//! ```
//!
#![forbid(unsafe_code)]

pub mod batch;
pub mod cache;
pub mod calendar;
pub mod clock; 
pub mod constant;
pub mod core;

#[cfg(test)]
mod tests;
// #[cfg(test)]
// mod debug_iso;
pub mod format;
pub mod interval;
pub mod parser;
pub mod time;
pub mod validation;
pub mod schedule;
pub mod index;
pub mod database;

pub mod prelude {
    pub use crate::{
        calendar::{
            Calendar,
            CalendarDate,
            CalendarDay,
            CalendarDuration,
            CalendarInterval,
            CalendarMonth,
            CalendarRule,
            CalendarYear,
            DayIncrementor,
            MonthPeriod,
            YearPeriod,
        },
        clock::{
            ClockDuration,
            ClockFields,
            ClockHour,
            ClockInterval,
            ClockMicroSecond,
            ClockMilliSecond,
            ClockMinute,
            ClockNanoSecond,
            ClockSecond,
            ClockTime,
            HourPeriod,
            MinutePeriod,
            PerSecondRated,
            SecondPeriod,
        },
        constant::{
            DayOfWeek,
            MonthOfYear,
            OrdinalEnglish,
        },
        core::{
            AbstractInterval,
            AbstractPeriod,
            AbstractTime,
            Duration,
            Interval,
            IntervalList,
            KnownDay,
            KnownHour,
            KnownMinute,
            KnownMonth,
            KnownNanoSecond,
            KnownSecond,
            KnownYear,
            Time,
            TimeField,
            TimeList,
            TimeValidation,
        },
        format::{
            CalClockFormatter,
            FormatPattern,
            FormatStyle,
            FormatToken,
            Rfc9557Format,
            Rfc9557Config,
            PrecisionLevel,
        },
        interval::{
            CalClockRange,
            DateRange,
            TimeRange,
            RecurrencePattern,
            RecurrenceRule,
            Frequency,
            Schedule,
            ScheduleEvent,
            ScheduleBuilder,
        },
        parser::{
            Parser,
        },
        time::{
            CalClock,
            CalClockConverter,
            CalClockDuration,
            CalClockInterval,
            CalClockZone,
            StopWatch,
            StopWatchMillis,
        },
        validation::{
            CalClockValidator,
            ValidationError,
            ValidationResult,
            ValidationRule,
            ValidationRules,
            ValidationAnalytics,
            ValidationMetrics,
            ValidationReport,
            CachedValidator,
            ValidationCache,
            ConditionalRule,
            ValidationCondition,
            ParallelValidator,
            BatchValidationResult,
            ValidationProfile,
            ProfileBuilder,
            ProfileRegistry,
            StandardProfiles,
        },
        index::{
            TimeIndex,
            TimeIndexEntry,
            IndexKey,
            RangeIndex,
            RangeQuery,
            RangeResult,
            TemporalBTree,
            TemporalEntry,
            TemporalQuery,
        },
        database::{
            DatabaseStorable,
            DatabaseRecord,
            StorageFormat,
            SqlCompatible,
            NoSqlDocument,
            IndexGenerator,
            DatabaseIndex,
            IndexType,
        },
    };
}
