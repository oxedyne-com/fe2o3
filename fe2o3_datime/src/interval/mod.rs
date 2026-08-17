//! Spans of time: ranges, recurrence patterns and schedules built from them.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

pub mod range;
pub mod recurrence;
pub mod schedule;

pub use self::{
    range::{TimeRange, DateRange, CalClockRange},
    recurrence::{RecurrencePattern, RecurrenceRule, Frequency},
    schedule::{Schedule, ScheduleEvent, ScheduleBuilder},
};