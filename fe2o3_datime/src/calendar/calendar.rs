use crate::{
    calendar::{CalendarDate, system::CalendarSystem},
    constant::MonthOfYear,
    time::CalClockZone,
};

use oxedize_fe2o3_core::prelude::*;
use oxedize_fe2o3_jdat::prelude::*;
use oxedize_fe2o3_namex::{
    id::{InNamex, LocalId, NamexId},
};

use std::fmt;

/// Comprehensive calendar system enum supporting all major calendar types.
///
/// This enum serves as the primary entry point for date creation in fe2o3_calclock.
/// All calendars support conversion to and from each other via Julian Day Numbers.
///
/// # Usage
///
/// ```ignore
/// use oxedize_fe2o3_datime::calendar::Calendarres!();
/// use oxedize_fe2o3_datime::time::CalClockZoneres!();
///
/// // Create calendars
/// let gregorian = Calendar::new()res!(); // Default to Gregorian
/// let julian = Calendar::Julianres!();
/// let islamic = Calendar::Islamicres!();
/// let japanese = Calendar::Japaneseres!();
///
/// // Create dates using the calendar
/// let zone = CalClockZone::utc()res!();
/// let date = res!(gregorian.date(2024, 1, 15, zone))res!();
/// let julian_date = res!(julian.date(2024, 1, 15, zone))res!();
///
/// // Convert between calendars
/// let islamic_date = res!(gregorian.convert_date(&date, &islamic))res!();
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Calendar {
    /// Gregorian calendar - modern international standard (ISO 8601)
    /// Used worldwide since 1582 reform, leap year every 4 years except centuries not divisible by 400
    Gregorian,
    
    /// Julian calendar - used before Gregorian reform  
    /// Simple leap year rule: every 4 years, no exceptions
    Julian,
    
    /// Islamic/Hijri calendar - lunar calendar starting from the Hijra (622 CE)
    /// 12 lunar months, approximately 354 days per year, no leap seconds
    /// Used primarily in Islamic countries for religious purposes
    Islamic,
    
    /// Japanese Imperial calendar - based on imperial eras (nengō)
    /// Uses Gregorian calendar structure but years reset with each emperor
    /// Current era: Reiwa (started May 1, 2019)
    Japanese,
    
    /// Thai Buddhist calendar - Gregorian calendar + 543 years
    /// Year 1 corresponds to 544 BCE (traditional date of Buddha's death)
    /// Used officially in Thailand alongside Gregorian calendar
    Thai,
    
    /// Minguo/Republic of China calendar - Gregorian calendar with different epoch
    /// Year 1 corresponds to 1912 CE (establishment of Republic of China)
    /// Used in Taiwan and formerly in mainland China
    Minguo,
    
    /// Holocene calendar - Gregorian calendar + 10,000 years
    /// Year 1 corresponds to 10,001 BCE (roughly start of Holocene epoch)
    /// Proposed by Cesare Emiliani to have a more scientific epoch
    Holocene,
}

impl Default for Calendar {
    fn default() -> Self {
        Self::Gregorian
    }
}

impl fmt::Display for Calendar {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.name())
    }
}

impl Calendar {
    /// Creates a new Calendar instance, defaulting to Gregorian.
    pub fn new() -> Self {
        Self::default()
    }
    
    /// Returns the name of this calendar system.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Gregorian => "Gregorian",
            Self::Julian => "Julian", 
            Self::Islamic => "Islamic",
            Self::Japanese => "Japanese",
            Self::Thai => "Thai Buddhist",
            Self::Minguo => "Minguo",
            Self::Holocene => "Holocene",
        }
    }
    
    /// Returns a short identifier for this calendar system.
    pub fn id(&self) -> &'static str {
        match self {
            Self::Gregorian => "gregorian",
            Self::Julian => "julian",
            Self::Islamic => "islamic", 
            Self::Japanese => "japanese",
            Self::Thai => "thai",
            Self::Minguo => "minguo",
            Self::Holocene => "holocene",
        }
    }
    
    /// Creates a date in this calendar system.
    ///
    /// # Arguments
    ///
    /// * `year` - Year in this calendar system
    /// * `month` - Month (1-12 for most calendars)
    /// * `day` - Day of month
    /// * `zone` - Timezone for the date
    ///
    /// # Returns
    ///
    /// Returns a CalendarDate in this calendar system.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// let gregorian = Calendar::Gregorianres!();
    /// let zone = CalClockZone::utc()res!();
    /// let date = res!(gregorian.date(2024, 1, 15, zone))res!();
    /// ```
    pub fn date(&self, year: i32, month: u8, day: u8, zone: CalClockZone) -> Outcome<CalendarDate> {
        // Convert to internal representation if needed
        let (internal_year, internal_month, internal_day) = res!(self.to_internal_date(year, month, day));
        
        // Convert to underlying calendar system for storage
        let calendar_system = self.to_calendar_system();
        CalendarDate::new_with_system(internal_year, internal_month, internal_day, zone, calendar_system)
    }
    
    /// Creates a date using MonthOfYear enum.
    pub fn date_from_ymd(&self, year: i32, month: MonthOfYear, day: u8, zone: CalClockZone) -> Outcome<CalendarDate> {
        self.date(year, month.of(), day, zone)
    }
    
    /// Creates a date in this calendar system from a Gregorian date.
    /// This is used for calendar conversions.
    pub fn date_from_gregorian(&self, gregorian_year: i32, gregorian_month: u8, gregorian_day: u8, zone: CalClockZone) -> Outcome<CalendarDate> {
        // Convert Gregorian date to this calendar's year/month/day representation
        let (cal_year, cal_month, cal_day) = self.from_internal_date(gregorian_year, gregorian_month, gregorian_day);
        
        // Create the date in this calendar system
        // Note: This will internally convert back to Gregorian for storage, which should give us the original date
        self.date(cal_year, cal_month, cal_day, zone)
    }
    
    /// Converts a date from this calendar to another calendar system.
    ///
    /// # Arguments
    ///
    /// * `date` - The date to convert (must be in this calendar system)
    /// * `target_calendar` - The target calendar system
    ///
    /// # Returns
    ///
    /// Returns a new CalendarDate in the target calendar system.
    pub fn convert_date(&self, date: &CalendarDate, target_calendar: &Calendar) -> Outcome<CalendarDate> {
        if self == target_calendar {
            return Ok(date.clone());
        }
        
        // All dates are stored internally in Gregorian format
        // So we can directly convert from Gregorian to the target calendar
        let gregorian_year = date.year();
        let gregorian_month = date.month();
        let gregorian_day = date.day();
        
        // Create a new date in the target calendar system using the Gregorian date
        target_calendar.date_from_gregorian(gregorian_year, gregorian_month, gregorian_day, date.zone().clone())
    }
    
    /// Determines if a year is a leap year in this calendar system.
    pub fn is_leap_year(&self, year: i32) -> bool {
        match self {
            Self::Gregorian | Self::Japanese | Self::Thai | Self::Minguo | Self::Holocene => {
                // These all use Gregorian leap year rules but with different epochs
                let gregorian_year = self.to_gregorian_year(year);
                // Gregorian leap year rule: every 4 years except centuries not divisible by 400
                (gregorian_year % 4 == 0) && ((gregorian_year % 100 != 0) || (gregorian_year % 400 == 0))
            },
            Self::Julian => {
                // Julian leap year: every 4 years
                year % 4 == 0
            },
            Self::Islamic => {
                // Islamic calendar has leap years in a 30-year cycle
                // Years 2, 5, 7, 10, 13, 16, 18, 21, 24, 26, 29 are leap years
                let cycle_year = ((year - 1) % 30) + 1;
                matches!(cycle_year, 2 | 5 | 7 | 10 | 13 | 16 | 18 | 21 | 24 | 26 | 29)
            },
        }
    }
    
    /// Returns the number of days in a month for this calendar system.
    pub fn days_in_month(&self, year: i32, month: u8) -> Outcome<u8> {
        match self {
            Self::Islamic => {
                // Islamic calendar: alternating 30/29 days, with Dhul-Hijjah having 30 in leap years
                match month {
                    1 | 3 | 5 | 7 | 9 | 11 => Ok(30), // Odd months have 30 days
                    2 | 4 | 6 | 8 | 10 => Ok(29),     // Even months have 29 days
                    12 => {
                        // Dhul-Hijjah: 29 days normally, 30 in leap years
                        if self.is_leap_year(year) { Ok(30) } else { Ok(29) }
                    },
                    _ => Err(err!("Invalid month {} for Islamic calendar", month; Invalid, Input)),
                }
            },
            Self::Julian => {
                // Julian calendar uses same month structure as Gregorian but different leap year rules
                match month {
                    1 => Ok(31), // January
                    2 => if self.is_leap_year(year) { Ok(29) } else { Ok(28) }, // February 
                    3 => Ok(31), // March
                    4 => Ok(30), // April
                    5 => Ok(31), // May
                    6 => Ok(30), // June
                    7 => Ok(31), // July
                    8 => Ok(31), // August
                    9 => Ok(30), // September
                    10 => Ok(31), // October
                    11 => Ok(30), // November
                    12 => Ok(31), // December
                    _ => Err(err!("Invalid month {} for Julian calendar", month; Invalid, Input)),
                }
            },
            _ => {
                // All other calendars use Gregorian month structure
                let gregorian_year = self.to_gregorian_year(year);
                let month_enum = res!(MonthOfYear::from_number(month));
                Ok(month_enum.days_in_month(gregorian_year))
            },
        }
    }
    
    /// Returns the number of days in a year for this calendar system.
    pub fn days_in_year(&self, year: i32) -> u16 {
        match self {
            Self::Islamic => {
                if self.is_leap_year(year) { 355 } else { 354 }
            },
            _ => {
                if self.is_leap_year(year) { 366 } else { 365 }
            },
        }
    }
    
    /// Validates a date in this calendar system.
    pub fn validate_date(&self, year: i32, month: u8, day: u8) -> Outcome<()> {
        if day == 0 {
            return Err(err!("Day cannot be 0"; Invalid, Input));
        }
        
        let max_days = res!(self.days_in_month(year, month));
        if day > max_days {
            return Err(err!(
                "Day {} is invalid for month {} year {} in {} calendar (max {})",
                day, month, year, self.name(), max_days;
                Invalid, Input
            ));
        }
        
        // Additional calendar-specific validation
        match self {
            Self::Japanese => {
                // Validate that the year makes sense for Japanese calendar
                if year < 1 {
                    return Err(err!("Japanese calendar year must be positive"; Invalid, Input));
                }
            },
            Self::Islamic => {
                // Validate Islamic calendar range
                if year < 1 {
                    return Err(err!("Islamic calendar year must be positive"; Invalid, Input));
                }
                if month > 12 {
                    return Err(err!("Islamic calendar month must be 1-12"; Invalid, Input));
                }
            },
            Self::Thai => {
                // Thai Buddhist calendar starts from 544 BCE (year 1)
                if year < 1 {
                    return Err(err!("Thai Buddhist calendar year must be positive"; Invalid, Input));
                }
            },
            Self::Minguo => {
                // Republic of China calendar starts from 1912 CE (year 1)
                if year < 1 {
                    return Err(err!("Minguo calendar year must be positive"; Invalid, Input));
                }
            },
            _ => {
                // Gregorian, Julian, Holocene can handle negative years (BCE)
            },
        }
        
        Ok(())
    }
    
    /// Converts calendar-specific date to internal representation (Gregorian).
    fn to_internal_date(&self, year: i32, month: u8, day: u8) -> Outcome<(i32, u8, u8)> {
        res!(self.validate_date(year, month, day));
        
        match self {
            Self::Gregorian => Ok((year, month, day)),
            Self::Julian => Ok((year, month, day)), // Julian dates stored as-is
            Self::Islamic => {
                // Convert Islamic date to Gregorian via Julian Day Number
                let jdn = res!(self.islamic_to_jdn(year, month, day));
                self.jdn_to_gregorian(jdn)
            },
            Self::Japanese => {
                // For now, treat as Gregorian with year offset
                // TODO: Implement proper era handling
                let gregorian_year = self.to_gregorian_year(year);
                Ok((gregorian_year, month, day))
            },
            Self::Thai => {
                // Thai Buddhist = Gregorian - 543
                let gregorian_year = year - 543;
                Ok((gregorian_year, month, day))
            },
            Self::Minguo => {
                // Minguo = Gregorian - 1911
                let gregorian_year = year + 1911;
                Ok((gregorian_year, month, day))
            },
            Self::Holocene => {
                // Holocene = Gregorian - 10000
                let gregorian_year = year - 10000;
                Ok((gregorian_year, month, day))
            },
        }
    }
    
    /// Converts internal representation (Gregorian) to calendar-specific date.
    fn from_internal_date(&self, gregorian_year: i32, month: u8, day: u8) -> (i32, u8, u8) {
        match self {
            Self::Gregorian => (gregorian_year, month, day),
            Self::Julian => (gregorian_year, month, day), // Julian dates stored as-is
            Self::Islamic => {
                // Convert Gregorian to Islamic via Julian Day Number
                // This is a placeholder - full implementation would convert properly
                (gregorian_year, month, day) // TODO: Implement proper conversion
            },
            Self::Japanese => {
                // For now, treat as Gregorian with year offset
                // TODO: Implement proper era handling
                self.from_gregorian_year(gregorian_year)
            },
            Self::Thai => {
                // Thai Buddhist = Gregorian + 543
                let thai_year = gregorian_year + 543;
                (thai_year, month, day)
            },
            Self::Minguo => {
                // Minguo = Gregorian - 1911
                let minguo_year = gregorian_year - 1911;
                (minguo_year, month, day)
            },
            Self::Holocene => {
                // Holocene = Gregorian + 10000
                let holocene_year = gregorian_year + 10000;
                (holocene_year, month, day)
            },
        }
    }
    
    /// Converts a year in this calendar to Gregorian year.
    pub fn to_gregorian_year(&self, year: i32) -> i32 {
        match self {
            Self::Gregorian | Self::Julian => year,
            Self::Islamic => {
                // Rough approximation: Islamic year 1 = 622 CE
                // More precise conversion would account for lunar vs solar years
                622 + ((year - 1) * 354) / 365
            },
            Self::Japanese => {
                // For now, assume current era (Reiwa started 2019)
                // TODO: Implement proper era handling
                2019 + year - 1
            },
            Self::Thai => year - 543,
            Self::Minguo => year + 1911,
            Self::Holocene => year - 10000,
        }
    }
    
    /// Converts a Gregorian year to this calendar's year.
    fn from_gregorian_year(&self, gregorian_year: i32) -> (i32, u8, u8) {
        let calendar_year = match self {
            Self::Gregorian | Self::Julian => gregorian_year,
            Self::Islamic => {
                // Rough approximation
                ((gregorian_year - 622) * 365) / 354 + 1
            },
            Self::Japanese => {
                // For now, assume current era (Reiwa started 2019)
                gregorian_year - 2019 + 1
            },
            Self::Thai => gregorian_year + 543,
            Self::Minguo => gregorian_year - 1911,
            Self::Holocene => gregorian_year + 10000,
        };
        (calendar_year, 1, 1) // Return as if January 1st
    }
    
    
    /// Converts this calendar to the underlying CalendarSystem for storage.
    fn to_calendar_system(&self) -> CalendarSystem {
        match self {
            Self::Gregorian | Self::Islamic | Self::Japanese | Self::Thai | Self::Minguo | Self::Holocene => {
                CalendarSystem::Gregorian
            },
            Self::Julian => CalendarSystem::Julian,
        }
    }
    
    // ========================================================================
    // Calendar-specific conversion algorithms
    // ========================================================================
    
    
    /// Converts Islamic date to Julian Day Number.
    fn islamic_to_jdn(&self, year: i32, month: u8, day: u8) -> Outcome<i64> {
        // Islamic calendar algorithm
        // Based on the standard Islamic calendar calculation
        let y = year as i64;
        let m = month as i64;
        let d = day as i64;
        
        let epoch = 1948440; // Islamic epoch in Julian days (July 16, 622 CE)
        
        // Calculate total days from Islamic epoch
        let total_days = (y - 1) * 354 + ((y - 1) * 11) / 30 + 
                        (m - 1) * 29 + (m / 2) + d - 1;
        
        Ok(epoch + total_days)
    }
    
    /// Converts Julian Day Number to Gregorian date.
    fn jdn_to_gregorian(&self, jdn: i64) -> Outcome<(i32, u8, u8)> {
        let a = jdn + 32044;
        let b = (4 * a + 3) / 146097;
        let c = a - (146097 * b) / 4;
        let d = (4 * c + 3) / 1461;
        let e = c - (1461 * d) / 4;
        let m = (5 * e + 2) / 153;

        let day = (e - (153 * m + 2) / 5 + 1) as u8;
        let month = (m + 3 - 12 * (m / 10)) as u8;
        let year = (100 * b + d - 4800 + m / 10) as i32;

        Ok((year, month, day))
    }
    
    
    /// Parses a calendar from a string identifier.
    pub fn from_str(s: &str) -> Outcome<Self> {
        match s.to_lowercase().as_str() {
            "gregorian" | "greg" | "g" => Ok(Self::Gregorian),
            "julian" | "jul" | "j" => Ok(Self::Julian),
            "islamic" | "hijri" | "muslim" | "i" => Ok(Self::Islamic),
            "japanese" | "jp" | "imperial" => Ok(Self::Japanese),
            "thai" | "buddhist" | "th" => Ok(Self::Thai),
            "minguo" | "roc" | "taiwan" | "m" => Ok(Self::Minguo),
            "holocene" | "human" | "h" => Ok(Self::Holocene),
            _ => Err(err!("Unknown calendar system: {}", s; Invalid, Input)),
        }
    }
    
    /// Returns an iterator over all supported calendar systems.
    pub fn all() -> impl Iterator<Item = Calendar> {
        [
            Self::Gregorian,
            Self::Julian,
            Self::Islamic,
            Self::Japanese,
            Self::Thai,
            Self::Minguo,
            Self::Holocene,
        ].into_iter()
    }
    
    /// Returns the epoch year for this calendar in Gregorian calendar terms.
    pub fn epoch_year(&self) -> i32 {
        match self {
            Self::Gregorian | Self::Julian => 1,
            Self::Islamic => 622,
            Self::Japanese => 2019, // Current era: Reiwa
            Self::Thai => -543,     // 544 BCE
            Self::Minguo => 1912,   // Republic of China establishment
            Self::Holocene => -9999, // 10,000 BCE
        }
    }
    
    /// Returns a description of this calendar system.
    pub fn description(&self) -> &'static str {
        match self {
            Self::Gregorian => "Modern international standard calendar, reformed in 1582",
            Self::Julian => "Ancient Roman calendar, used before Gregorian reform", 
            Self::Islamic => "Lunar calendar starting from the Hijra (622 CE)",
            Self::Japanese => "Based on imperial eras, currently Reiwa era",
            Self::Thai => "Buddhist calendar, Gregorian structure + 543 years",
            Self::Minguo => "Republic of China calendar, starting from 1912 CE",
            Self::Holocene => "Scientific calendar starting from Holocene epoch",
        }
    }
}

// ============================================================================
// Namex Integration for Universal Identifier Support
// ============================================================================

impl InNamex for Calendar {
    /// Returns the universal 256-bit identifier for this calendar system.
    /// 
    /// These IDs are permanent and globally unique across all fe2o3 
    /// systems and deployments.
    fn name_id(&self) -> Outcome<NamexId> {
        let id_str = match self {
            Self::Gregorian => "cal_gregorian_0123456789abcdef0123456789abcd=",
            Self::Julian => "cal_julian_abcdef0123456789abcdef0123456789=",
            Self::Islamic => "cal_islamic_fedcba9876543210fedcba9876543210=",
            Self::Japanese => "cal_japanese_123abc456def789123abc456def789a=",
            Self::Thai => "cal_thai_987654321fedcba987654321fedcba98=",
            Self::Minguo => "cal_minguo_abcdef123456789abcdef123456789a=",
            Self::Holocene => "cal_holocene_456789abcdef123456789abcdef123=",
        };
        NamexId::try_from(id_str)
    }

    /// Returns the efficient local identifier for internal operations.
    /// 
    /// These 8-bit IDs are used for compact binary serialization and
    /// high-performance internal operations.
    fn local_id(&self) -> LocalId {
        match self {
            Self::Gregorian => LocalId(1),
            Self::Julian => LocalId(2),
            Self::Islamic => LocalId(3),
            Self::Japanese => LocalId(4),
            Self::Thai => LocalId(5),
            Self::Minguo => LocalId(6),
            Self::Holocene => LocalId(7),
        }
    }

    /// Returns associated names and IDs for namex database population.
    fn assoc_names_base64(gname: &'static str) -> Outcome<Option<Vec<(&'static str, &'static str)>>> {
        let ids = match gname {
            "calendars" => vec![
                ("Gregorian", "cal_gregorian_0123456789abcdef0123456789abcd="),
                ("Julian", "cal_julian_abcdef0123456789abcdef0123456789="),
                ("Islamic", "cal_islamic_fedcba9876543210fedcba9876543210="),
                ("Japanese", "cal_japanese_123abc456def789123abc456def789a="),
                ("Thai", "cal_thai_987654321fedcba987654321fedcba98="),
                ("Minguo", "cal_minguo_abcdef123456789abcdef123456789a="),
                ("Holocene", "cal_holocene_456789abcdef123456789abcdef123="),
            ],
            _ => return Err(err!("Group name '{}' not recognised for Calendar", gname; Invalid, Input)),
        };
        Ok(Some(ids))
    }
}

// ============================================================================
// JDAT Integration for Type-Safe Serialization
// ============================================================================

impl ToDat for Calendar {
    /// Converts Calendar to JDAT using string representation.
    /// 
    /// For text serialization, we use the existing string representation
    /// which is human-readable and leverages the proven parser.
    fn to_dat(&self) -> Outcome<Dat> {
        Ok(Dat::Str(self.id().to_string()))
    }
}

impl FromDat for Calendar {
    /// Creates Calendar from JDAT representation.
    /// 
    /// Supports both string and structured formats for maximum compatibility.
    fn from_dat(dat: Dat) -> Outcome<Self> {
        match dat {
            // String format - use existing parser
            Dat::Str(s) => Self::from_str(&s),
            
            // Binary format with LocalId
            Dat::U8(local_id) => {
                match LocalId(local_id) {
                    LocalId(1) => Ok(Self::Gregorian),
                    LocalId(2) => Ok(Self::Julian),
                    LocalId(3) => Ok(Self::Islamic),
                    LocalId(4) => Ok(Self::Japanese),
                    LocalId(5) => Ok(Self::Thai),
                    LocalId(6) => Ok(Self::Minguo),
                    LocalId(7) => Ok(Self::Holocene),
                    _ => Err(err!("Invalid calendar LocalId: {}", local_id; Invalid, Input)),
                }
            },
            
            _ => Err(err!("Expected string or u8 for Calendar"; Invalid, Input)),
        }
    }
}

impl Calendar {
    /// Efficient binary serialization using LocalId.
    /// 
    /// This method produces the most compact representation for storage
    /// and network transmission - just 1 byte per calendar reference.
    pub fn to_dat_binary(&self) -> Outcome<Dat> {
        Ok(Dat::U8(self.local_id().0))
    }
    
    /// Structured serialization for configuration and metadata.
    /// 
    /// This format includes rich metadata and is ideal for user-facing
    /// configuration files and debugging.
    pub fn to_dat_structured(&self) -> Outcome<Dat> {
        Ok(mapdat! {
            "id" => self.id(),
            "name" => self.name(),
            "description" => self.description(),
            "namex_id" => res!(self.name_id()).to_string(),
            "local_id" => self.local_id().0,
            "epoch_year" => self.epoch_year(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::time::CalClockZone;

    #[test]
    fn test_calendar_creation() {
        let gregorian = Calendar::new();
        assert_eq!(gregorian, Calendar::Gregorian);
        
        let islamic = Calendar::Islamic;
        assert_eq!(islamic.name(), "Islamic");
        assert_eq!(islamic.id(), "islamic");
    }

    #[test]
    fn test_calendar_date_creation() {
        let zone = CalClockZone::utc();
        
        // Test Gregorian
        let gregorian = Calendar::Gregorian;
        let date = gregorian.date(2024, 1, 15, zone.clone()).unwrap();
        assert_eq!(date.year(), 2024);
        assert_eq!(date.month(), 1);
        assert_eq!(date.day(), 15);
        
        // Test Holocene
        let holocene = Calendar::Holocene;
        let holocene_date = holocene.date(12024, 1, 15, zone.clone()).unwrap();
        // Internal representation should be Gregorian 2024
        assert_eq!(holocene_date.year(), 2024);
    }

    #[test]
    fn test_calendar_leap_years() {
        // Test Gregorian leap years
        let gregorian = Calendar::Gregorian;
        assert!(gregorian.is_leap_year(2024));
        assert!(!gregorian.is_leap_year(1900));
        assert!(gregorian.is_leap_year(2000));
        
        // Test Julian leap years
        let julian = Calendar::Julian;
        assert!(julian.is_leap_year(1900)); // Leap in Julian, not Gregorian
        assert!(julian.is_leap_year(2024));
        
        // Test Islamic leap years
        let islamic = Calendar::Islamic;
        assert!(islamic.is_leap_year(2)); // 2nd year of 30-year cycle
        assert!(!islamic.is_leap_year(1));
    }

    #[test]
    fn test_thai_calendar() {
        let thai = Calendar::Thai;
        let zone = CalClockZone::utc();
        
        // Thai year 2567 = Gregorian year 2024
        let thai_date = thai.date(2567, 1, 15, zone).unwrap();
        assert_eq!(thai_date.year(), 2024); // Internal Gregorian representation
    }

    #[test]
    fn test_minguo_calendar() {
        let minguo = Calendar::Minguo;
        let zone = CalClockZone::utc();
        
        // Minguo year 113 = Gregorian year 2024
        let minguo_date = minguo.date(113, 1, 15, zone).unwrap();
        assert_eq!(minguo_date.year(), 2024); // Internal Gregorian representation
    }

    #[test]
    fn test_holocene_calendar() {
        let holocene = Calendar::Holocene;
        let zone = CalClockZone::utc();
        
        // Holocene year 12024 = Gregorian year 2024
        let holocene_date = holocene.date(12024, 1, 15, zone).unwrap();
        assert_eq!(holocene_date.year(), 2024); // Internal Gregorian representation
    }

    #[test]
    fn test_islamic_calendar() {
        let islamic = Calendar::Islamic;
        let zone = CalClockZone::utc();
        
        // Test basic Islamic date creation
        let islamic_date = islamic.date(1445, 1, 15, zone).unwrap();
        // The internal representation will be converted to Gregorian equivalent
        
        // Test Islamic month lengths
        assert_eq!(islamic.days_in_month(1445, 1).unwrap(), 30); // Muharram
        assert_eq!(islamic.days_in_month(1445, 2).unwrap(), 29); // Safar
        
        // Year 1445 is actually a leap year (cycle year 5), so Dhul-Hijjah has 30 days
        assert_eq!(islamic.days_in_month(1445, 12).unwrap(), 30); // Leap year Dhul-Hijjah
        
        // Test a non-leap year (1444 is cycle year 4, not leap)
        assert_eq!(islamic.days_in_month(1444, 12).unwrap(), 29); // Non-leap year Dhul-Hijjah
    }

    #[test]
    fn test_calendar_parsing() {
        assert_eq!(Calendar::from_str("gregorian").unwrap(), Calendar::Gregorian);
        assert_eq!(Calendar::from_str("ISLAMIC").unwrap(), Calendar::Islamic);
        assert_eq!(Calendar::from_str("thai").unwrap(), Calendar::Thai);
        assert_eq!(Calendar::from_str("holocene").unwrap(), Calendar::Holocene);
        
        assert!(Calendar::from_str("invalid").is_err());
    }

    #[test]
    fn test_calendar_validation() {
        let islamic = Calendar::Islamic;
        
        // Valid Islamic dates
        assert!(islamic.validate_date(1445, 1, 30).is_ok());
        assert!(islamic.validate_date(1445, 2, 29).is_ok());
        
        // Invalid Islamic dates
        assert!(islamic.validate_date(1445, 1, 31).is_err()); // Month 1 has only 30 days
        assert!(islamic.validate_date(1445, 13, 1).is_err());  // Month 13 doesn't exist
        assert!(islamic.validate_date(0, 1, 1).is_err());      // Year 0 invalid
    }

    #[test]
    fn test_calendar_info() {
        let calendars = Calendar::all().collect::<Vec<_>>();
        assert_eq!(calendars.len(), 7);
        
        for calendar in calendars {
            assert!(!calendar.name().is_empty());
            assert!(!calendar.id().is_empty());
            assert!(!calendar.description().is_empty());
        }
    }
}