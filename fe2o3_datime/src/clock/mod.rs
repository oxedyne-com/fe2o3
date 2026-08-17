//! [Written entirely with AI](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

pub mod time;
pub mod hour;
pub mod minute;
pub mod second;
pub mod nanosecond;
pub mod millisecond;
pub mod microsecond;
pub mod duration;
pub mod interval;
pub mod fields;
pub mod periods;

pub use time::ClockTime;
pub use hour::ClockHour;
pub use minute::ClockMinute;
pub use second::ClockSecond;
pub use nanosecond::ClockNanoSecond;
pub use millisecond::ClockMilliSecond;
pub use microsecond::ClockMicroSecond;
pub use duration::ClockDuration;
pub use interval::ClockInterval;
pub use fields::ClockFields;
pub use periods::{HourPeriod, MinutePeriod, SecondPeriod};

pub trait PerSecondRated {
	fn per_second(&self) -> u64;
}