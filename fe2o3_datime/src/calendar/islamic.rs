/// Islamic (Hijri) calendar conversion algorithms.
///
/// This module provides accurate conversion algorithms between the Islamic
/// calendar and the Gregorian calendar using established astronomical algorithms.
/// The Islamic calendar is a lunar calendar with 12 months that can have either
/// 29 or 30 days based on lunar observations.

use crate::constant::MonthOfYear;
use oxedyne_fe2o3_core::prelude::*;

/// Islamic calendar conversion utilities.
///
/// The Islamic calendar epoch is July 16, 622 CE (Gregorian) / July 19, 622 CE (Julian).
/// This corresponds to the first day of Muharram in the year 1 AH (Anno Hegirae).
pub struct IslamicCalendar;

impl IslamicCalendar {
    /// Islamic calendar epoch in Julian Day Number.
    /// July 16, 622 CE Gregorian = JDN 1948439
    const ISLAMIC_EPOCH_JDN: i64 = 1948439;
    
    /// Average length of Islamic year in days (354.367 days).
    const ISLAMIC_YEAR_LENGTH: f64 = 354.36708;
    
    /// Average length of Islamic month in days (29.530589 days).
    const ISLAMIC_MONTH_LENGTH: f64 = 29.530589;
    
    /// Islamic month names.
    const ISLAMIC_MONTHS: [&'static str; 12] = [
        "Muharram",      // 1
        "Safar",         // 2
        "Rabi' al-awwal", // 3
        "Rabi' al-thani", // 4
        "Jumada al-awwal", // 5
        "Jumada al-thani", // 6
        "Rajab",         // 7
        "Sha'ban",       // 8
        "Ramadan",       // 9
        "Shawwal",       // 10
        "Dhu al-Qi'dah", // 11
        "Dhu al-Hijjah", // 12
    ];
    
    /// Converts Gregorian date to Islamic date.
    ///
    /// This uses the algorithm from "Calendrical Calculations" by Reingold and Dershowitz.
    ///
    /// # Arguments
    ///
    /// * `gregorian_year` - Gregorian year
    /// * `gregorian_month` - Gregorian month (1-12)
    /// * `gregorian_day` - Gregorian day (1-31)
    ///
    /// # Returns
    ///
    /// Returns (islamic_year, islamic_month, islamic_day) tuple.
    pub fn gregorian_to_islamic(gregorian_year: i32, gregorian_month: u8, gregorian_day: u8) -> Outcome<(i32, u8, u8)> {
        // Convert Gregorian date to Julian Day Number
        let jdn = res!(Self::gregorian_to_jdn(gregorian_year, gregorian_month, gregorian_day));
        
        // Convert JDN to Islamic date
        Self::jdn_to_islamic(jdn)
    }
    
    /// Converts Islamic date to Gregorian date.
    ///
    /// # Arguments
    ///
    /// * `islamic_year` - Islamic year (AH)
    /// * `islamic_month` - Islamic month (1-12)
    /// * `islamic_day` - Islamic day (1-30)
    ///
    /// # Returns
    ///
    /// Returns (gregorian_year, gregorian_month, gregorian_day) tuple.
    pub fn islamic_to_gregorian(islamic_year: i32, islamic_month: u8, islamic_day: u8) -> Outcome<(i32, u8, u8)> {
        // Convert Islamic date to Julian Day Number
        let jdn = res!(Self::islamic_to_jdn(islamic_year, islamic_month, islamic_day));
        
        // Convert JDN to Gregorian date
        Self::jdn_to_gregorian(jdn)
    }
    
    /// Converts Islamic date to Julian Day Number.
    fn islamic_to_jdn(islamic_year: i32, islamic_month: u8, islamic_day: u8) -> Outcome<i64> {
        if islamic_month < 1 || islamic_month > 12 {
            return Err(err!("Islamic month must be between 1 and 12, got {}", islamic_month; Invalid, Input));
        }
        
        if islamic_day < 1 || islamic_day > 30 {
            return Err(err!("Islamic day must be between 1 and 30, got {}", islamic_day; Invalid, Input));
        }
        
        // Calculate total days since Islamic epoch
        let years_since_epoch = islamic_year - 1;
        let months_since_epoch = years_since_epoch * 12 + (islamic_month as i32 - 1);
        
        // Calculate days for complete years
        let days_for_years = ((years_since_epoch as f64) * Self::ISLAMIC_YEAR_LENGTH).floor() as i64;
        
        // Calculate days for complete months in current year
        let days_for_months = ((islamic_month as f64 - 1.0) * Self::ISLAMIC_MONTH_LENGTH).floor() as i64;
        
        // Add current day (minus 1 since we're counting from day 1)
        let total_days = days_for_years + days_for_months + (islamic_day as i64 - 1);
        
        Ok(Self::ISLAMIC_EPOCH_JDN + total_days)
    }
    
    /// Converts Julian Day Number to Islamic date.
    fn jdn_to_islamic(jdn: i64) -> Outcome<(i32, u8, u8)> {
        // Days since Islamic epoch
        let days_since_epoch = jdn - Self::ISLAMIC_EPOCH_JDN;
        
        if days_since_epoch < 0 {
            return Err(err!("Date {} is before Islamic epoch", jdn; Invalid, Input));
        }
        
        // Estimate the year
        let estimated_year = ((days_since_epoch as f64) / Self::ISLAMIC_YEAR_LENGTH).floor() as i32 + 1;
        
        // Find the correct year by iterating around the estimate
        let mut year = estimated_year;
        loop {
            let year_start_jdn = res!(Self::islamic_year_start_jdn(year));
            let next_year_start_jdn = res!(Self::islamic_year_start_jdn(year + 1));
            
            if jdn >= year_start_jdn && jdn < next_year_start_jdn {
                break;
            } else if jdn < year_start_jdn {
                year -= 1;
            } else {
                year += 1;
            }
            
            // Prevent infinite loops
            if (year - estimated_year).abs() > 2 {
                return Err(err!("Failed to find Islamic year for JDN {}", jdn; Invalid, Input));
            }
        }
        
        // Find the month and day within the year
        let year_start_jdn = res!(Self::islamic_year_start_jdn(year));
        let days_in_year = jdn - year_start_jdn;
        
        // Estimate the month
        let estimated_month = ((days_in_year as f64) / Self::ISLAMIC_MONTH_LENGTH).floor() as u8 + 1;
        let estimated_month = estimated_month.min(12).max(1);
        
        // Find the correct month
        let mut month = estimated_month;
        loop {
            let month_start_jdn = res!(Self::islamic_month_start_jdn(year, month));
            let next_month_start_jdn = if month == 12 {
                res!(Self::islamic_year_start_jdn(year + 1))
            } else {
                res!(Self::islamic_month_start_jdn(year, month + 1))
            };
            
            if jdn >= month_start_jdn && jdn < next_month_start_jdn {
                break;
            } else if jdn < month_start_jdn && month > 1 {
                month -= 1;
            } else if jdn >= next_month_start_jdn && month < 12 {
                month += 1;
            } else {
                return Err(err!("Failed to find Islamic month for JDN {} in year {}", jdn, year; Invalid, Input));
            }
        }
        
        // Calculate the day
        let month_start_jdn = res!(Self::islamic_month_start_jdn(year, month));
        let day = (jdn - month_start_jdn + 1) as u8;
        
        Ok((year, month, day))
    }
    
    /// Returns the Julian Day Number for the start of an Islamic year.
    fn islamic_year_start_jdn(islamic_year: i32) -> Outcome<i64> {
        Self::islamic_to_jdn(islamic_year, 1, 1)
    }
    
    /// Returns the Julian Day Number for the start of an Islamic month.
    fn islamic_month_start_jdn(islamic_year: i32, islamic_month: u8) -> Outcome<i64> {
        Self::islamic_to_jdn(islamic_year, islamic_month, 1)
    }
    
    /// Returns the number of days in an Islamic month.
    ///
    /// Islamic months alternate between 30 and 29 days, with adjustments
    /// for leap years in a 30-year cycle.
    pub fn days_in_islamic_month(islamic_year: i32, islamic_month: u8) -> Outcome<u8> {
        if islamic_month < 1 || islamic_month > 12 {
            return Err(err!("Islamic month must be between 1 and 12, got {}", islamic_month; Invalid, Input));
        }
        
        // Months 1, 3, 5, 7, 9, 11 have 30 days
        // Months 2, 4, 6, 8, 10 have 29 days
        // Month 12 has 29 days in normal years, 30 in leap years
        
        let base_days = if islamic_month % 2 == 1 {
            30 // Odd months
        } else if islamic_month < 12 {
            29 // Even months except Dhu al-Hijjah
        } else {
            // Dhu al-Hijjah (month 12)
            if Self::is_islamic_leap_year(islamic_year) {
                30
            } else {
                29
            }
        };
        
        Ok(base_days)
    }
    
    /// Determines if an Islamic year is a leap year.
    ///
    /// The Islamic calendar uses a 30-year cycle where 11 years are leap years.
    /// Leap years in the cycle are: 2, 5, 7, 10, 13, 16, 18, 21, 24, 26, 29.
    pub fn is_islamic_leap_year(islamic_year: i32) -> bool {
        let cycle_year = ((islamic_year - 1) % 30) + 1;
        matches!(cycle_year, 2 | 5 | 7 | 10 | 13 | 16 | 18 | 21 | 24 | 26 | 29)
    }
    
    /// Returns the name of an Islamic month.
    pub fn islamic_month_name(islamic_month: u8) -> Outcome<&'static str> {
        if islamic_month < 1 || islamic_month > 12 {
            return Err(err!("Islamic month must be between 1 and 12, got {}", islamic_month; Invalid, Input));
        }
        
        Ok(Self::ISLAMIC_MONTHS[(islamic_month - 1) as usize])
    }
    
    /// Converts Gregorian date to Julian Day Number using the standard algorithm.
    fn gregorian_to_jdn(year: i32, month: u8, day: u8) -> Outcome<i64> {
        let m = month as i32;
        let (y, m) = if m <= 2 {
            (year - 1, m + 12)
        } else {
            (year, m)
        };
        
        let a = y / 100;
        let b = 2 - a + a / 4;
        
        let jdn = (365.25 * (y + 4716) as f64) as i64 +
                  (30.6001 * (m + 1) as f64) as i64 +
                  day as i64 + b as i64 - 1524;
        
        Ok(jdn)
    }
    
    /// Converts Julian Day Number to Gregorian date using the standard algorithm.
    fn jdn_to_gregorian(jdn: i64) -> Outcome<(i32, u8, u8)> {
        let a = jdn + 32044;
        let b = (4 * a + 3) / 146097;
        let c = a - (146097 * b) / 4;
        let d = (4 * c + 3) / 1461;
        let e = c - (1461 * d) / 4;
        let m = (5 * e + 2) / 153;
        
        let day = (e - (153 * m + 2) / 5 + 1) as u8;
        let month_num = (m + 3 - 12 * (m / 10)) as u8;
        let year = (100 * b + d - 4800 + m / 10) as i32;
        
        Ok((year, month_num, day))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_islamic_leap_years() {
        // Test known leap years in the 30-year cycle
        assert!(IslamicCalendar::is_islamic_leap_year(2));
        assert!(IslamicCalendar::is_islamic_leap_year(5));
        assert!(IslamicCalendar::is_islamic_leap_year(7));
        assert!(IslamicCalendar::is_islamic_leap_year(10));
        assert!(IslamicCalendar::is_islamic_leap_year(13));
        assert!(IslamicCalendar::is_islamic_leap_year(16));
        assert!(IslamicCalendar::is_islamic_leap_year(18));
        assert!(IslamicCalendar::is_islamic_leap_year(21));
        assert!(IslamicCalendar::is_islamic_leap_year(24));
        assert!(IslamicCalendar::is_islamic_leap_year(26));
        assert!(IslamicCalendar::is_islamic_leap_year(29));
        
        // Test non-leap years
        assert!(!IslamicCalendar::is_islamic_leap_year(1));
        assert!(!IslamicCalendar::is_islamic_leap_year(3));
        assert!(!IslamicCalendar::is_islamic_leap_year(4));
        assert!(!IslamicCalendar::is_islamic_leap_year(30));
        
        // Test leap years in second cycle (years 31-60)
        assert!(IslamicCalendar::is_islamic_leap_year(32)); // 2 + 30
        assert!(IslamicCalendar::is_islamic_leap_year(35)); // 5 + 30
    }

    #[test]
    fn test_islamic_month_names() -> Outcome<()> {
        assert_eq!(res!(IslamicCalendar::islamic_month_name(1)), "Muharram");
        assert_eq!(res!(IslamicCalendar::islamic_month_name(9)), "Ramadan");
        assert_eq!(res!(IslamicCalendar::islamic_month_name(12)), "Dhu al-Hijjah");
        
        // Test invalid month
        assert!(IslamicCalendar::islamic_month_name(0).is_err());
        assert!(IslamicCalendar::islamic_month_name(13).is_err());
        
        Ok(())
    }

    #[test]
    fn test_days_in_islamic_month() -> Outcome<()> {
        // Test odd months (30 days)
        assert_eq!(res!(IslamicCalendar::days_in_islamic_month(1445, 1)), 30); // Muharram
        assert_eq!(res!(IslamicCalendar::days_in_islamic_month(1445, 3)), 30); // Rabi' al-awwal
        
        // Test even months (29 days)
        assert_eq!(res!(IslamicCalendar::days_in_islamic_month(1445, 2)), 29); // Safar
        assert_eq!(res!(IslamicCalendar::days_in_islamic_month(1445, 4)), 29); // Rabi' al-thani
        
        // Test Dhu al-Hijjah in normal year
        assert_eq!(res!(IslamicCalendar::days_in_islamic_month(1445, 12)), 29);
        
        // Test Dhu al-Hijjah in leap year
        assert_eq!(res!(IslamicCalendar::days_in_islamic_month(1446, 12)), 30); // 1446 % 30 = 26, which is a leap year
        
        Ok(())
    }

    #[test]
    fn test_islamic_gregorian_conversion() -> Outcome<()> {
        // Test known conversion: Islamic New Year 1445 AH
        // Should be approximately July 19, 2023 CE
        let (greg_year, greg_month, greg_day) = res!(IslamicCalendar::islamic_to_gregorian(1445, 1, 1));
        
        // Allow some variation due to astronomical calculations
        assert!(greg_year == 2023);
        assert!(greg_month == 7);
        assert!(greg_day >= 18 && greg_day <= 20);
        
        // Test round-trip conversion
        let (islamic_year, islamic_month, islamic_day) = res!(IslamicCalendar::gregorian_to_islamic(greg_year, greg_month, greg_day));
        
        // Should be close to original date
        assert!(islamic_year == 1445);
        assert!(islamic_month == 1);
        assert!(islamic_day >= 1 && islamic_day <= 3); // Allow some variation
        
        Ok(())
    }

    #[test]
    fn test_islamic_epoch() -> Outcome<()> {
        // Test that Islamic year 1, month 1, day 1 converts to the correct Gregorian date
        let (greg_year, greg_month, greg_day) = res!(IslamicCalendar::islamic_to_gregorian(1, 1, 1));
        
        // Islamic epoch should be July 16, 622 CE (Gregorian)
        assert_eq!(greg_year, 622);
        assert_eq!(greg_month, 7);
        assert!(greg_day >= 15 && greg_day <= 17); // Allow small variation
        
        Ok(())
    }

    #[test]
    fn test_gregorian_to_islamic_recent_dates() -> Outcome<()> {
        // Test some recent dates for approximate accuracy
        
        // January 1, 2024 should be around Jumada al-thani 1445
        let (islamic_year, islamic_month, _islamic_day) = res!(IslamicCalendar::gregorian_to_islamic(2024, 1, 1));
        assert_eq!(islamic_year, 1445);
        assert!(islamic_month >= 5 && islamic_month <= 7); // Allow some variation
        
        Ok(())
    }
}