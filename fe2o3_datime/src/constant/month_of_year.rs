use oxedyne_fe2o3_core::prelude::*;

use std::fmt::{self, Display};

/// Months of the year.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum MonthOfYear {
    January,
    February,
    March,
    April,
    May,
    June,
    July,
    August,
    September,
    October,
    November,
    December,
}

impl MonthOfYear {
    /// Get the numeric value of the month (January = 1, December = 12).
    pub fn of(&self) -> u8 {
        match self {
            Self::January	=> 1,
            Self::February	=> 2,
            Self::March		=> 3,
            Self::April		=> 4,
            Self::May		=> 5,
            Self::June		=> 6,
            Self::July		=> 7,
            Self::August	=> 8,
            Self::September	=> 9,
            Self::October	=> 10,
            Self::November	=> 11,
            Self::December	=> 12,
        }
    }
    
    /// Create from numeric value (1-12).
    pub fn from_number(n: u8) -> Outcome<Self> {
        match n {
            1	=> Ok(Self::January),
            2	=> Ok(Self::February),
            3	=> Ok(Self::March),
            4	=> Ok(Self::April),
            5	=> Ok(Self::May),
            6	=> Ok(Self::June),
            7	=> Ok(Self::July),
            8	=> Ok(Self::August),
            9	=> Ok(Self::September),
            10	=> Ok(Self::October),
            11	=> Ok(Self::November),
            12	=> Ok(Self::December),
            _ => Err(err!(
                "Invalid month number: {}, must be 1-12", n;
                Invalid, Input, Range)),
        }
    }
    
    /// Get the next month.
    pub fn next(&self) -> Self {
        match self {
            Self::January	=> Self::February,
            Self::February	=> Self::March,
            Self::March		=> Self::April,
            Self::April		=> Self::May,
            Self::May		=> Self::June,
            Self::June		=> Self::July,
            Self::July		=> Self::August,
            Self::August	=> Self::September,
            Self::September	=> Self::October,
            Self::October	=> Self::November,
            Self::November	=> Self::December,
            Self::December	=> Self::January,
        }
    }
    
    /// Get the previous month.
    pub fn previous(&self) -> Self {
        match self {
            Self::January	=> Self::December,
            Self::February	=> Self::January,
            Self::March		=> Self::February,
            Self::April		=> Self::March,
            Self::May		=> Self::April,
            Self::June		=> Self::May,
            Self::July		=> Self::June,
            Self::August	=> Self::July,
            Self::September	=> Self::August,
            Self::October	=> Self::September,
            Self::November	=> Self::October,
            Self::December	=> Self::November,
        }
    }
    
    /// Get abbreviated name (Jan, Feb, etc).
    pub fn abbrev(&self) -> &'static str {
        match self {
            Self::January	=> "Jan",
            Self::February	=> "Feb",
            Self::March		=> "Mar",
            Self::April		=> "Apr",
            Self::May		=> "May",
            Self::June		=> "Jun",
            Self::July		=> "Jul",
            Self::August	=> "Aug",
            Self::September	=> "Sep",
            Self::October	=> "Oct",
            Self::November	=> "Nov",
            Self::December	=> "Dec",
        }
    }
    
    /// Get full name.
    pub fn name(&self) -> &'static str {
        match self {
            Self::January	=> "January",
            Self::February	=> "February",
            Self::March		=> "March",
            Self::April		=> "April",
            Self::May		=> "May",
            Self::June		=> "June",
            Self::July		=> "July",
            Self::August	=> "August",
            Self::September	=> "September",
            Self::October	=> "October",
            Self::November	=> "November",
            Self::December	=> "December",
        }
    }
    
    /// Get the number of days in this month for a given year.
    /// Takes leap years into account.
    pub fn days_in_month(&self, year: i32) -> u8 {
        match self {
            Self::January	=> 31,
            Self::February	=> {
                // Use default Gregorian calendar for leap year calculation
                let calendar = crate::calendar::Calendar::new();
                if calendar.is_leap_year(year) { 29 } else { 28 }
            },
            Self::March		=> 31,
            Self::April		=> 30,
            Self::May		=> 31,
            Self::June		=> 30,
            Self::July		=> 31,
            Self::August	=> 31,
            Self::September	=> 30,
            Self::October	=> 31,
            Self::November	=> 30,
            Self::December	=> 31,
        }
    }
    
    /// Get the quarter this month belongs to (1-4).
    pub fn quarter(&self) -> u8 {
        match self {
            Self::January | Self::February | Self::March		=> 1,
            Self::April | Self::May | Self::June			=> 2,
            Self::July | Self::August | Self::September			=> 3,
            Self::October | Self::November | Self::December		=> 4,
        }
    }
    
    /// Get short name (alias for abbrev).
    pub fn short_name(&self) -> &'static str {
        self.abbrev()
    }
    
    /// Get long name (alias for name).
    pub fn long_name(&self) -> &'static str {
        self.name()
    }

    /// Parse from a string name (case insensitive).
    pub fn from_name(name: &str) -> Option<Self> {
        let name = name.to_lowercase();
        match name.as_str() {
            "january" | "jan" => Some(Self::January),
            "february" | "feb" => Some(Self::February),
            "march" | "mar" => Some(Self::March),
            "april" | "apr" => Some(Self::April),
            "may" => Some(Self::May),
            "june" | "jun" => Some(Self::June),
            "july" | "jul" => Some(Self::July),
            "august" | "aug" => Some(Self::August),
            "september" | "sep" | "sept" => Some(Self::September),
            "october" | "oct" => Some(Self::October),
            "november" | "nov" => Some(Self::November),
            "december" | "dec" => Some(Self::December),
            _ => None,
        }
    }
}

impl Display for MonthOfYear {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.name())
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_month_days_in_leap_year() {
        // Test February in leap years and non-leap years
        assert_eq!(MonthOfYear::February.days_in_month(2024), 29); // Leap year
        assert_eq!(MonthOfYear::February.days_in_month(2023), 28); // Non-leap year
        assert_eq!(MonthOfYear::February.days_in_month(2000), 29); // Leap year (divisible by 400)
        assert_eq!(MonthOfYear::February.days_in_month(1900), 28); // Non-leap year (divisible by 100 but not 400)
        
        // Test other months are unaffected by leap years
        assert_eq!(MonthOfYear::January.days_in_month(2024), 31);
        assert_eq!(MonthOfYear::January.days_in_month(2023), 31);
        assert_eq!(MonthOfYear::March.days_in_month(2024), 31);
        assert_eq!(MonthOfYear::April.days_in_month(2024), 30);
    }
}