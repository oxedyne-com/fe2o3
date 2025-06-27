/// Japanese Imperial Era (Nengō) system for accurate Japanese calendar calculations.
///
/// This module provides comprehensive support for Japanese imperial eras,
/// including historical eras and proper era transition logic.
///
/// The Japanese calendar uses the same month/day structure as the Gregorian calendar
/// but years are counted from the beginning of each emperor's reign.

use oxedyne_fe2o3_core::prelude::*;

/// Represents a Japanese imperial era (nengō).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JapaneseEra {
    /// Name of the era in Japanese
    pub name_japanese: &'static str,
    /// Name of the era in romanized form
    pub name_romaji: &'static str,
    /// Gregorian year when this era started
    pub start_year: i32,
    /// Month when this era started (1-12)
    pub start_month: u8,
    /// Day when this era started
    pub start_day: u8,
    /// Gregorian year when this era ended (None for current era)
    pub end_year: Option<i32>,
    /// Month when this era ended (1-12, None for current era)
    pub end_month: Option<u8>,
    /// Day when this era ended (None for current era)
    pub end_day: Option<u8>,
}

impl JapaneseEra {
    /// Returns true if this era contains the given Gregorian date.
    pub fn contains_date(&self, year: i32, month: u8, day: u8) -> bool {
        // Check if date is after era start
        if year < self.start_year {
            return false;
        }
        if year == self.start_year {
            if month < self.start_month {
                return false;
            }
            if month == self.start_month && day < self.start_day {
                return false;
            }
        }
        
        // Check if date is before era end (if era has ended)
        if let (Some(end_year), Some(end_month), Some(end_day)) = 
           (self.end_year, self.end_month, self.end_day) {
            if year > end_year {
                return false;
            }
            if year == end_year {
                if month > end_month {
                    return false;
                }
                if month == end_month && day > end_day {
                    return false;
                }
            }
        }
        
        true
    }
    
    /// Converts a Gregorian date to Japanese era year.
    pub fn gregorian_to_era_year(&self, gregorian_year: i32) -> Outcome<i32> {
        // Check if the year falls within this era
        if gregorian_year < self.start_year {
            return Err(err!("Year {} is before era {} starts in {}", 
                           gregorian_year, self.name_romaji, self.start_year; 
                           Invalid, Input));
        }
        
        if let Some(end_year) = self.end_year {
            if gregorian_year > end_year {
                return Err(err!("Year {} is after era {} ends in {}", 
                               gregorian_year, self.name_romaji, end_year; 
                               Invalid, Input));
            }
        }
        
        Ok(gregorian_year - self.start_year + 1)
    }
    
    /// Converts a Japanese era year to Gregorian year.
    pub fn era_year_to_gregorian(&self, era_year: i32) -> Outcome<i32> {
        if era_year < 1 {
            return Err(err!("Era year must be positive, got {}", era_year; Invalid, Input));
        }
        
        let gregorian_year = self.start_year + era_year - 1;
        
        // Validate that this year is within the era bounds
        if let Some(end_year) = self.end_year {
            if gregorian_year > end_year {
                return Err(err!("Era year {} exceeds end of era {} ({})", 
                               era_year, self.name_romaji, self.name_japanese; 
                               Invalid, Input));
            }
        }
        
        Ok(gregorian_year)
    }
}

/// Japanese era registry with all historical and current eras.
pub struct JapaneseEraRegistry {
    eras: Vec<JapaneseEra>,
}

impl JapaneseEraRegistry {
    /// Creates a new era registry with all known Japanese eras.
    pub fn new() -> Self {
        Self {
            eras: vec![
                // Modern eras (post-Meiji Restoration)
                JapaneseEra {
                    name_japanese: "令和",
                    name_romaji: "Reiwa",
                    start_year: 2019,
                    start_month: 5,
                    start_day: 1,
                    end_year: None,
                    end_month: None,
                    end_day: None,
                },
                JapaneseEra {
                    name_japanese: "平成",
                    name_romaji: "Heisei",
                    start_year: 1989,
                    start_month: 1,
                    start_day: 8,
                    end_year: Some(2019),
                    end_month: Some(4),
                    end_day: Some(30),
                },
                JapaneseEra {
                    name_japanese: "昭和",
                    name_romaji: "Showa",
                    start_year: 1926,
                    start_month: 12,
                    start_day: 25,
                    end_year: Some(1989),
                    end_month: Some(1),
                    end_day: Some(7),
                },
                JapaneseEra {
                    name_japanese: "大正",
                    name_romaji: "Taisho",
                    start_year: 1912,
                    start_month: 7,
                    start_day: 30,
                    end_year: Some(1926),
                    end_month: Some(12),
                    end_day: Some(24),
                },
                JapaneseEra {
                    name_japanese: "明治",
                    name_romaji: "Meiji",
                    start_year: 1868,
                    start_month: 1,
                    start_day: 25,
                    end_year: Some(1912),
                    end_month: Some(7),
                    end_day: Some(29),
                },
            ],
        }
    }
    
    /// Finds the era that contains the given Gregorian date.
    pub fn find_era_for_date(&self, year: i32, month: u8, day: u8) -> Outcome<&JapaneseEra> {
        for era in &self.eras {
            if era.contains_date(year, month, day) {
                return Ok(era);
            }
        }
        
        Err(err!("No Japanese era found for date {}-{:02}-{:02}", year, month, day; Invalid, Input))
    }
    
    /// Finds an era by its romanized name.
    pub fn find_era_by_name(&self, name: &str) -> Option<&JapaneseEra> {
        let name_lower = name.to_lowercase();
        self.eras.iter().find(|era| era.name_romaji.to_lowercase() == name_lower)
    }
    
    /// Returns the current era (Reiwa).
    pub fn current_era(&self) -> &JapaneseEra {
        // Current era is always the first in our list
        &self.eras[0]
    }
    
    /// Returns all available eras in chronological order (newest first).
    pub fn all_eras(&self) -> &[JapaneseEra] {
        &self.eras
    }
    
    /// Converts a Gregorian date to Japanese calendar format.
    pub fn gregorian_to_japanese(&self, gregorian_year: i32, month: u8, day: u8) -> Outcome<(String, i32, u8, u8)> {
        let era = res!(self.find_era_for_date(gregorian_year, month, day));
        let era_year = res!(era.gregorian_to_era_year(gregorian_year));
        
        Ok((era.name_romaji.to_string(), era_year, month, day))
    }
    
    /// Converts Japanese calendar format to Gregorian date.
    pub fn japanese_to_gregorian(&self, era_name: &str, era_year: i32, month: u8, day: u8) -> Outcome<(i32, u8, u8)> {
        let era = self.find_era_by_name(era_name)
            .ok_or_else(|| err!("Unknown Japanese era: {}", era_name; Invalid, Input))?;
        
        let gregorian_year = res!(era.era_year_to_gregorian(era_year));
        
        // Validate the specific date within the era
        if !era.contains_date(gregorian_year, month, day) {
            return Err(err!("Date {}-{:02}-{:02} is not valid in era {}", 
                           era_year, month, day, era_name; Invalid, Input));
        }
        
        Ok((gregorian_year, month, day))
    }
}

impl Default for JapaneseEraRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_era_date_containment() {
        let registry = JapaneseEraRegistry::new();
        let reiwa = registry.current_era();
        
        // Reiwa started May 1, 2019
        assert!(reiwa.contains_date(2019, 5, 1));
        assert!(reiwa.contains_date(2024, 1, 1));
        assert!(!reiwa.contains_date(2019, 4, 30));
        assert!(!reiwa.contains_date(2018, 12, 31));
    }
    
    #[test]
    fn test_era_year_conversion() {
        let registry = JapaneseEraRegistry::new();
        let reiwa = registry.current_era();
        
        // Reiwa 1 = 2019, Reiwa 6 = 2024
        assert_eq!(reiwa.gregorian_to_era_year(2019).unwrap(), 1);
        assert_eq!(reiwa.gregorian_to_era_year(2024).unwrap(), 6);
        
        assert_eq!(reiwa.era_year_to_gregorian(1).unwrap(), 2019);
        assert_eq!(reiwa.era_year_to_gregorian(6).unwrap(), 2024);
    }
    
    #[test]
    fn test_era_transitions() {
        let registry = JapaneseEraRegistry::new();
        
        // Test Heisei to Reiwa transition (April 30, 2019 -> May 1, 2019)
        let heisei_last = registry.find_era_for_date(2019, 4, 30).unwrap();
        let reiwa_first = registry.find_era_for_date(2019, 5, 1).unwrap();
        
        assert_eq!(heisei_last.name_romaji, "Heisei");
        assert_eq!(reiwa_first.name_romaji, "Reiwa");
        
        // Heisei 31 ended on April 30, 2019
        assert_eq!(heisei_last.gregorian_to_era_year(2019).unwrap(), 31);
        // Reiwa 1 started on May 1, 2019
        assert_eq!(reiwa_first.gregorian_to_era_year(2019).unwrap(), 1);
    }
    
    #[test]
    fn test_japanese_calendar_conversion() {
        let registry = JapaneseEraRegistry::new();
        
        // Test current date in Reiwa era
        let (era_name, era_year, month, day) = registry.gregorian_to_japanese(2024, 6, 15).unwrap();
        assert_eq!(era_name, "Reiwa");
        assert_eq!(era_year, 6);
        assert_eq!(month, 6);
        assert_eq!(day, 15);
        
        // Convert back
        let (greg_year, greg_month, greg_day) = registry.japanese_to_gregorian("Reiwa", 6, 6, 15).unwrap();
        assert_eq!(greg_year, 2024);
        assert_eq!(greg_month, 6);
        assert_eq!(greg_day, 15);
    }
    
    #[test]
    fn test_historical_eras() {
        let registry = JapaneseEraRegistry::new();
        
        // Test Showa era (1926-1989)
        let showa = registry.find_era_by_name("Showa").unwrap();
        assert_eq!(showa.gregorian_to_era_year(1945).unwrap(), 20); // End of WWII
        assert_eq!(showa.gregorian_to_era_year(1989).unwrap(), 64); // Last year of Showa
        
        // Test Meiji era (1868-1912)
        let meiji = registry.find_era_by_name("Meiji").unwrap();
        assert_eq!(meiji.gregorian_to_era_year(1868).unwrap(), 1); // First year
        assert_eq!(meiji.gregorian_to_era_year(1912).unwrap(), 45); // Last year
    }
    
    #[test]
    fn test_era_not_found() {
        let registry = JapaneseEraRegistry::new();
        
        // Test date before Meiji era
        assert!(registry.find_era_for_date(1850, 1, 1).is_err());
        
        // Test unknown era name
        assert!(registry.find_era_by_name("Unknown").is_none());
    }
}