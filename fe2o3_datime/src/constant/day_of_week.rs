//! Days of the week, numbered Monday 1 through Sunday 7.
//!
//! [Written entirely with AI](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use oxedyne_fe2o3_core::prelude::*;

use std::fmt::{self, Display};

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
    /// Monday is 1 and Sunday 7.
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
    
    pub fn is_weekend(&self) -> bool {
        matches!(self, Self::Saturday | Self::Sunday)
    }
    
    pub fn is_weekday(&self) -> bool {
        !self.is_weekend()
    }
    
    pub fn short_name(&self) -> &'static str {
        self.abbrev()
    }
    
    pub fn long_name(&self) -> &'static str {
        self.name()
    }
    
    /// Matches full names and abbreviations, case insensitively.
    pub fn from_name(name: &str) -> Option<Self> {
        let name = name.to_lowercase();
        match name.as_str() {
            "monday" | "mon" => Some(Self::Monday),
            "tuesday" | "tue" | "tues" => Some(Self::Tuesday),
            "wednesday" | "wed" => Some(Self::Wednesday),
            "thursday" | "thu" | "thur" | "thurs" => Some(Self::Thursday),
            "friday" | "fri" => Some(Self::Friday),
            "saturday" | "sat" => Some(Self::Saturday),
            "sunday" | "sun" => Some(Self::Sunday),
            _ => None,
        }
    }
    
    /// Zero when the two days are the same, never seven.
    pub fn days_until(&self, target: &Self) -> u8 {
        let current = self.of();
        let target = target.of();
        
        if current == target {
            0
        } else if target > current {
            target - current
        } else {
            7 - (current - target)
        }
    }
}

impl Display for DayOfWeek {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.name())
    }
}