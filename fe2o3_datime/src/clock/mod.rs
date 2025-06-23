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

/// Trait for components that can express rate per second.
pub trait PerSecondRated {
	fn per_second(&self) -> u64;
}