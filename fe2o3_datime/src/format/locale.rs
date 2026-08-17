//! Locales: the regional conventions that decide how a date or time is written.
//!
//! Each locale carries default patterns for date, time and datetime, together
//! with short and long variants. A small built-in database covers the common
//! ones.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use crate::format::FormatPattern;
use oxedyne_fe2o3_core::prelude::*;

use std::{
    collections::HashMap,
    sync::OnceLock,
};

/// # Examples
///
/// ```ignore
/// use oxedyne_fe2o3_datime::format::{Locale, CalClockFormatter}res!();
/// use oxedyne_fe2o3_datime::time::CalClockres!();
///
/// let calclock = CalClock::now_utc()?res!();
/// let formatter = CalClockFormatter::new()res!();
///
/// // Use US locale formatting
/// let us_locale = Locale::us()res!();
/// let us_date = formatter.format_with_pattern(&calclock, us_locale.date_pattern())?res!();
/// // Result: "01/15/2024"
///
/// // Use European locale formatting  
/// let european_locale = Locale::europe()res!();
/// let european_date = formatter.format_with_pattern(&calclock, european_locale.date_pattern())?res!();
/// // Result: "15/01/2024"
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Locale {
    id:                     String,         // "en-US", "en-GB", "de-DE"
    display_name:           String,
    date_pattern:           FormatPattern,
    time_pattern:           FormatPattern,
    datetime_pattern:       FormatPattern,
    short_date_pattern:     FormatPattern,
    long_date_pattern:      FormatPattern,
    short_time_pattern:     FormatPattern,
}

static LOCALE_DB: OnceLock<HashMap<String, Locale>> = OnceLock::new();

fn init_locale_db() -> HashMap<String, Locale> {
    let mut db = HashMap::new();
    
    // United States (en-US)
    db.insert("en-US".to_string(), Locale {
        id: "en-US".to_string(),
        display_name: "English (United States)".to_string(),
        date_pattern: FormatPattern::us_date(),
        time_pattern: FormatPattern::time_12h(),
        datetime_pattern: FormatPattern::new("MM/dd/yyyy h:mm:ss a").unwrap(),
        short_date_pattern: FormatPattern::new("M/d/yy").unwrap(),
        long_date_pattern: FormatPattern::full_date(),
        short_time_pattern: FormatPattern::time_short(),
    });
    
    // United Kingdom (en-GB)
    db.insert("en-GB".to_string(), Locale {
        id: "en-GB".to_string(),
        display_name: "English (United Kingdom)".to_string(),
        date_pattern: FormatPattern::new("dd/MM/yyyy").unwrap(),
        time_pattern: FormatPattern::time_24h(),
        datetime_pattern: FormatPattern::new("dd/MM/yyyy HH:mm:ss").unwrap(),
        short_date_pattern: FormatPattern::new("d/M/yy").unwrap(),
        long_date_pattern: FormatPattern::full_date(),
        short_time_pattern: FormatPattern::new("HH:mm").unwrap(),
    });
    
    // Germany (de-DE)
    db.insert("de-DE".to_string(), Locale {
        id: "de-DE".to_string(),
        display_name: "Deutsch (Deutschland)".to_string(),
        date_pattern: FormatPattern::new("dd.MM.yyyy").unwrap(),
        time_pattern: FormatPattern::time_24h(),
        datetime_pattern: FormatPattern::new("dd.MM.yyyy HH:mm:ss").unwrap(),
        short_date_pattern: FormatPattern::new("d.M.yy").unwrap(),
        long_date_pattern: FormatPattern::new("EEEE, d. MMMM yyyy").unwrap(),
        short_time_pattern: FormatPattern::new("HH:mm").unwrap(),
    });
    
    // France (fr-FR)
    db.insert("fr-FR".to_string(), Locale {
        id: "fr-FR".to_string(),
        display_name: "Français (France)".to_string(),
        date_pattern: FormatPattern::new("dd/MM/yyyy").unwrap(),
        time_pattern: FormatPattern::time_24h(),
        datetime_pattern: FormatPattern::new("dd/MM/yyyy HH:mm:ss").unwrap(),
        short_date_pattern: FormatPattern::new("d/M/yy").unwrap(),
        long_date_pattern: FormatPattern::new("EEEE d MMMM yyyy").unwrap(),
        short_time_pattern: FormatPattern::new("HH:mm").unwrap(),
    });
    
    // Japan (ja-JP)
    db.insert("ja-JP".to_string(), Locale {
        id: "ja-JP".to_string(),
        display_name: "日本語 (日本)".to_string(),
        date_pattern: FormatPattern::new("yyyy/MM/dd").unwrap(),
        time_pattern: FormatPattern::time_24h(),
        datetime_pattern: FormatPattern::new("yyyy/MM/dd HH:mm:ss").unwrap(),
        short_date_pattern: FormatPattern::new("yy/M/d").unwrap(),
        long_date_pattern: FormatPattern::new("yyyy'年'M'月'd'日' EEEE").unwrap(),
        short_time_pattern: FormatPattern::new("HH:mm").unwrap(),
    });
    
    // China (zh-CN)
    db.insert("zh-CN".to_string(), Locale {
        id: "zh-CN".to_string(),
        display_name: "中文 (中国)".to_string(),
        date_pattern: FormatPattern::new("yyyy/M/d").unwrap(),
        time_pattern: FormatPattern::time_24h(),
        datetime_pattern: FormatPattern::new("yyyy/M/d HH:mm:ss").unwrap(),
        short_date_pattern: FormatPattern::new("yy/M/d").unwrap(),
        long_date_pattern: FormatPattern::new("yyyy'年'M'月'd'日' EEEE").unwrap(),
        short_time_pattern: FormatPattern::new("HH:mm").unwrap(),
    });
    
    // ISO 8601 (International Standard)
    db.insert("ISO".to_string(), Locale {
        id: "ISO".to_string(),
        display_name: "ISO 8601 International Standard".to_string(),
        date_pattern: FormatPattern::iso_date(),
        time_pattern: FormatPattern::iso_time(),
        datetime_pattern: FormatPattern::iso_datetime(),
        short_date_pattern: FormatPattern::iso_date(),
        long_date_pattern: FormatPattern::iso_date(),
        short_time_pattern: FormatPattern::new("HH:mm").unwrap(),
    });
    
    db
}

fn get_locale_db() -> &'static HashMap<String, Locale> {
    LOCALE_DB.get_or_init(init_locale_db)
}

impl Locale {
    pub fn new<S: Into<String>>(
        id: S,
        display_name: S,
        date_pattern: FormatPattern,
        time_pattern: FormatPattern,
        datetime_pattern: FormatPattern,
    ) -> Self {
        let id_str = id.into();
        Self {
            id: id_str.clone(),
            display_name: display_name.into(),
            short_date_pattern: date_pattern.clone(),
            long_date_pattern: FormatPattern::full_date(),
            short_time_pattern: FormatPattern::time_short(),
            date_pattern,
            time_pattern,
            datetime_pattern,
        }
    }
    
    /// An identifier that is not in the built-in database falls back to en-US.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// let us_locale = Locale::from_id("en-US")res!();
    /// let german_locale = Locale::from_id("de-DE")res!();
    /// let fallback = Locale::from_id("unknown")res!(); // Returns en-US
    /// ```
    pub fn from_id<S: Into<String>>(locale_id: S) -> Self {
        let id = locale_id.into();
        
        if let Some(locale) = get_locale_db().get(&id) {
            locale.clone()
        } else {
            // Fall back to US English
            Self::us()
        }
    }
    
    pub fn id(&self) -> &str {
        &self.id
    }
    
    pub fn display_name(&self) -> &str {
        &self.display_name
    }
    
    pub fn date_pattern(&self) -> &FormatPattern {
        &self.date_pattern
    }
    
    pub fn time_pattern(&self) -> &FormatPattern {
        &self.time_pattern
    }
    
    pub fn datetime_pattern(&self) -> &FormatPattern {
        &self.datetime_pattern
    }
    
    pub fn short_date_pattern(&self) -> &FormatPattern {
        &self.short_date_pattern
    }
    
    pub fn long_date_pattern(&self) -> &FormatPattern {
        &self.long_date_pattern
    }
    
    pub fn short_time_pattern(&self) -> &FormatPattern {
        &self.short_time_pattern
    }
    
    // ========================================================================
    // Predefined Locale Constructors
    // ========================================================================
    
    pub fn us() -> Self {
        Self::from_id("en-US")                  // MM/dd/yyyy, 12-hour
    }
    
    pub fn uk() -> Self {
        Self::from_id("en-GB")                  // dd/MM/yyyy, 24-hour
    }
    
    pub fn germany() -> Self {
        Self::from_id("de-DE")                  // dd.MM.yyyy, 24-hour
    }
    
    pub fn france() -> Self {
        Self::from_id("fr-FR")                  // dd/MM/yyyy, 24-hour
    }
    
    pub fn japan() -> Self {
        Self::from_id("ja-JP")                  // yyyy/MM/dd, 24-hour
    }
    
    pub fn china() -> Self {
        Self::from_id("zh-CN")                  // yyyy/M/d, 24-hour
    }
    
    pub fn iso() -> Self {
        Self::from_id("ISO")                    // yyyy-MM-dd, HH:mm:ss
    }
    
    pub fn europe() -> Self {
        Self::uk()
    }
    
    pub fn available_locales() -> Vec<String> {
        let mut locales: Vec<String> = get_locale_db().keys().cloned().collect();
        locales.sort();
        locales
    }
    
    /// Identifier first, then display name.
    pub fn available_locales_with_names() -> Vec<(String, String)> {
        let mut locales: Vec<(String, String)> = get_locale_db()
            .values()
            .map(|locale| (locale.id.clone(), locale.display_name.clone()))
            .collect();
        locales.sort_by(|a, b| a.0.cmp(&b.0));
        locales
    }
}

impl Default for Locale {
    fn default() -> Self {
        Self::us()
    }
}

impl std::fmt::Display for Locale {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} ({})", self.display_name, self.id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_locale_creation() {
        let us_locale = Locale::us();
        assert_eq!(us_locale.id(), "en-US");
        assert_eq!(us_locale.display_name(), "English (United States)");
    }

    #[test]
    fn test_locale_from_id() {
        let german_locale = Locale::from_id("de-DE");
        assert_eq!(german_locale.id(), "de-DE");
        assert_eq!(german_locale.display_name(), "Deutsch (Deutschland)");
        
        // Test fallback to US for unknown locale
        let unknown_locale = Locale::from_id("xx-XX");
        assert_eq!(unknown_locale.id(), "en-US");
    }

    #[test]
    fn test_pattern_access() {
        let us_locale = Locale::us();
        
        // US locale should use MM/dd/yyyy date format
        assert_eq!(us_locale.date_pattern().pattern_string(), "MM/dd/yyyy");
        
        // US locale should use 12-hour time format
        assert_eq!(us_locale.time_pattern().pattern_string(), "h:mm:ss a");
        
        let german_locale = Locale::germany();
        
        // German locale should use dd.MM.yyyy date format
        assert_eq!(german_locale.date_pattern().pattern_string(), "dd.MM.yyyy");
        
        // German locale should use 24-hour time format
        assert_eq!(german_locale.time_pattern().pattern_string(), "HH:mm:ss");
    }

    #[test]
    fn test_available_locales() {
        let locales = Locale::available_locales();
        assert!(locales.contains(&"en-US".to_string()));
        assert!(locales.contains(&"de-DE".to_string()));
        assert!(locales.contains(&"ja-JP".to_string()));
        assert!(locales.len() >= 7); // At least the predefined locales
    }

    #[test]
    fn test_locale_display() {
        let us_locale = Locale::us();
        let display_string = format!("{}", us_locale);
        assert!(display_string.contains("English (United States)"));
        assert!(display_string.contains("en-US"));
    }

    #[test]
    fn test_iso_locale() {
        let iso_locale = Locale::iso();
        assert_eq!(iso_locale.id(), "ISO");
        assert_eq!(iso_locale.date_pattern().pattern_string(), "yyyy-MM-dd");
        assert_eq!(iso_locale.time_pattern().pattern_string(), "HH:mm:ss");
    }
}