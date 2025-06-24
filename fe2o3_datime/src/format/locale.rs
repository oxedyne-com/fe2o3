use crate::format::FormatPattern;
use oxedyne_fe2o3_core::prelude::*;

use std::{
    collections::HashMap,
    sync::OnceLock,
};

/// Represents a locale for formatting dates and times.
///
/// A locale defines the cultural and regional conventions for displaying
/// dates, times, numbers, and other locale-sensitive information. This
/// implementation focuses on providing default format patterns for
/// common date/time representations in different locales.
///
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
    /// Locale identifier (e.g., "en-US", "en-GB", "de-DE")
    id: String,
    /// Human-readable display name
    display_name: String,
    /// Default date format pattern
    date_pattern: FormatPattern,
    /// Default time format pattern
    time_pattern: FormatPattern,
    /// Default datetime format pattern
    datetime_pattern: FormatPattern,
    /// Short date format pattern
    short_date_pattern: FormatPattern,
    /// Long date format pattern
    long_date_pattern: FormatPattern,
    /// Short time format pattern
    short_time_pattern: FormatPattern,
}

/// Static locale database containing predefined locales.
static LOCALE_DB: OnceLock<HashMap<String, Locale>> = OnceLock::new();

/// Initializes the built-in locale database with common locales.
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

/// Gets the locale database, initializing it if necessary.
fn get_locale_db() -> &'static HashMap<String, Locale> {
    LOCALE_DB.get_or_init(init_locale_db)
}

impl Locale {
    /// Creates a new locale with the specified parameters.
    ///
    /// # Arguments
    ///
    /// * `id` - Locale identifier (e.g., "en-US", "de-DE")
    /// * `display_name` - Human-readable name for the locale
    /// * `date_pattern` - Default date format pattern
    /// * `time_pattern` - Default time format pattern
    /// * `datetime_pattern` - Default datetime format pattern
    ///
    /// # Returns
    ///
    /// Returns a new Locale instance with the specified configuration.
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
    
    /// Creates a locale from a locale identifier string.
    ///
    /// This method looks up predefined locales from the built-in database.
    /// If the locale is not found, it falls back to US English formatting.
    ///
    /// # Arguments
    ///
    /// * `locale_id` - Locale identifier (e.g., "en-US", "de-DE", "ja-JP")
    ///
    /// # Returns
    ///
    /// Returns the requested locale, or US English if not found.
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
    
    /// Returns the locale identifier.
    pub fn id(&self) -> &str {
        &self.id
    }
    
    /// Returns the human-readable display name.
    pub fn display_name(&self) -> &str {
        &self.display_name
    }
    
    /// Returns the default date format pattern for this locale.
    pub fn date_pattern(&self) -> &FormatPattern {
        &self.date_pattern
    }
    
    /// Returns the default time format pattern for this locale.
    pub fn time_pattern(&self) -> &FormatPattern {
        &self.time_pattern
    }
    
    /// Returns the default datetime format pattern for this locale.
    pub fn datetime_pattern(&self) -> &FormatPattern {
        &self.datetime_pattern
    }
    
    /// Returns the short date format pattern for this locale.
    pub fn short_date_pattern(&self) -> &FormatPattern {
        &self.short_date_pattern
    }
    
    /// Returns the long date format pattern for this locale.
    pub fn long_date_pattern(&self) -> &FormatPattern {
        &self.long_date_pattern
    }
    
    /// Returns the short time format pattern for this locale.
    pub fn short_time_pattern(&self) -> &FormatPattern {
        &self.short_time_pattern
    }
    
    // ========================================================================
    // Predefined Locale Constructors
    // ========================================================================
    
    /// Creates a US English locale (en-US).
    ///
    /// Uses MM/dd/yyyy date format and 12-hour time format.
    pub fn us() -> Self {
        Self::from_id("en-US")
    }
    
    /// Creates a UK English locale (en-GB).
    ///
    /// Uses dd/MM/yyyy date format and 24-hour time format.
    pub fn uk() -> Self {
        Self::from_id("en-GB")
    }
    
    /// Creates a German locale (de-DE).
    ///
    /// Uses dd.MM.yyyy date format and 24-hour time format.
    pub fn germany() -> Self {
        Self::from_id("de-DE")
    }
    
    /// Creates a French locale (fr-FR).
    ///
    /// Uses dd/MM/yyyy date format and 24-hour time format.
    pub fn france() -> Self {
        Self::from_id("fr-FR")
    }
    
    /// Creates a Japanese locale (ja-JP).
    ///
    /// Uses yyyy/MM/dd date format and 24-hour time format.
    pub fn japan() -> Self {
        Self::from_id("ja-JP")
    }
    
    /// Creates a Chinese locale (zh-CN).
    ///
    /// Uses yyyy/M/d date format and 24-hour time format.
    pub fn china() -> Self {
        Self::from_id("zh-CN")
    }
    
    /// Creates an ISO 8601 international standard locale.
    ///
    /// Uses yyyy-MM-dd date format and HH:mm:ss time format.
    pub fn iso() -> Self {
        Self::from_id("ISO")
    }
    
    /// Alias for uk() - creates a European-style locale.
    ///
    /// This is a convenience method that provides European date formatting
    /// (dd/MM/yyyy) which is common across many European countries.
    pub fn europe() -> Self {
        Self::uk()
    }
    
    /// Returns a list of all available locale identifiers.
    ///
    /// This is useful for applications that want to present a list of
    /// supported locales to users.
    ///
    /// # Returns
    ///
    /// Returns a vector of locale identifier strings.
    pub fn available_locales() -> Vec<String> {
        let mut locales: Vec<String> = get_locale_db().keys().cloned().collect();
        locales.sort();
        locales
    }
    
    /// Returns a list of all available locales with their display names.
    ///
    /// This is useful for applications that want to present a human-readable
    /// list of supported locales to users.
    ///
    /// # Returns
    ///
    /// Returns a vector of (id, display_name) tuples.
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
    /// Returns the default locale (US English).
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