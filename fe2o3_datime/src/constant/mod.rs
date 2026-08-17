//! Named calendar and clock constants: weekdays, months, English ordinals and SI prefixes.
//!
//! [Written entirely with AI](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

pub mod day_of_week;
pub mod month_of_year;
pub mod ordinal;
pub mod si_prefix;

pub use self::{
    day_of_week::DayOfWeek,
    month_of_year::MonthOfYear,
    ordinal::OrdinalEnglish,
    si_prefix::SIPrefix,
};