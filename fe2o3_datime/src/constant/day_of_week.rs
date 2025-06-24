use oxedyne_fe2o3_core::prelude::*;

use std::fmt::{self, Display};

/// Days of the week.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum DayOfWeek {
    Monday,
    Tuesday,
    Wednesday,
    Thursday,
    Friday,
    Saturday,
    Sunday,
}

impl DayOfWeek {
    /// Get the numeric value of the day (Monday = 1, Sunday = 7).
    pub fn of(&self) -> u8 {
        match self {
            Self::Monday	=> 1,
            Self::Tuesday	=> 2,
            Self::Wednesday	=> 3,
            Self::Thursday	=> 4,
            Self::Friday	=> 5,
            Self::Saturday	=> 6,
            Self::Sunday	=> 7,
        }
    }
    
    /// Create from numeric value (1-7).
    pub fn from_number(n: u8) -> Outcome<Self> {
        match n {
            1 => Ok(Self::Monday),
            2 => Ok(Self::Tuesday),
            3 => Ok(Self::Wednesday),
            4 => Ok(Self::Thursday),
            5 => Ok(Self::Friday),
            6 => Ok(Self::Saturday),
            7 => Ok(Self::Sunday),
            _ => Err(err!(
                "Invalid day of week number: {}, must be 1-7", n;
                Invalid, Input, Range)),
        }
    }
    
    /// Get the next day of the week.
    pub fn next(&self) -> Self {
        match self {
            Self::Monday	=> Self::Tuesday,
            Self::Tuesday	=> Self::Wednesday,
            Self::Wednesday	=> Self::Thursday,
            Self::Thursday	=> Self::Friday,
            Self::Friday	=> Self::Saturday,
            Self::Saturday	=> Self::Sunday,
            Self::Sunday	=> Self::Monday,
        }
    }
    
    /// Get the previous day of the week.
    pub fn previous(&self) -> Self {
        match self {
            Self::Monday	=> Self::Sunday,
            Self::Tuesday	=> Self::Monday,
            Self::Wednesday	=> Self::Tuesday,
            Self::Thursday	=> Self::Wednesday,
            Self::Friday	=> Self::Thursday,
            Self::Saturday	=> Self::Friday,
            Self::Sunday	=> Self::Saturday,
        }
    }
    
    /// Get abbreviated name (Mon, Tue, etc).
    pub fn abbrev(&self) -> &'static str {
        match self {
            Self::Monday	=> "Mon",
            Self::Tuesday	=> "Tue",
            Self::Wednesday	=> "Wed",
            Self::Thursday	=> "Thu",
            Self::Friday	=> "Fri",
            Self::Saturday	=> "Sat",
            Self::Sunday	=> "Sun",
        }
    }
    
    /// Get full name.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Monday	=> "Monday",
            Self::Tuesday	=> "Tuesday",
            Self::Wednesday	=> "Wednesday",
            Self::Thursday	=> "Thursday",
            Self::Friday	=> "Friday",
            Self::Saturday	=> "Saturday",
            Self::Sunday	=> "Sunday",
        }
    }
    
    /// Check if this is a weekend day.
    pub fn is_weekend(&self) -> bool {
        matches!(self, Self::Saturday | Self::Sunday)
    }
    
    /// Check if this is a weekday.
    pub fn is_weekday(&self) -> bool {
        !self.is_weekend()
    }
    
    /// Get short name (alias for abbrev).
    pub fn short_name(&self) -> &'static str {
        self.abbrev()
    }
    
    /// Get long name (alias for name).
    pub fn long_name(&self) -> &'static str {
        self.name()
    }
}

impl Display for DayOfWeek {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.name())
    }
}