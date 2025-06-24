use oxedyne_fe2o3_core::prelude::*;

/// Formatting styles for different components.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum FormatStyle {
    /// Short format (e.g., "Jan", "Mon", "1")
    Short,
    /// Medium format (e.g., "Jan 15", "Monday")  
    Medium,
    /// Long format (e.g., "January", "Monday")
    Long,
    /// Full format (e.g., "January 15, 2024", "Monday, January 15, 2024")
    Full,
    /// Numeric format (e.g., "01", "15")
    Numeric,
    /// Custom padding and width
    Custom { width: usize, pad_char: char },
}

/// Individual format tokens that make up a pattern.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum FormatToken {
    // Date tokens
    Year(FormatStyle),
    Month(FormatStyle),
    Day(FormatStyle),
    DayOfWeek(FormatStyle),
    DayOfYear(FormatStyle),
    WeekOfYear(FormatStyle),
    Quarter(FormatStyle),
    
    // Time tokens
    Hour12(FormatStyle),
    Hour24(FormatStyle),
    Minute(FormatStyle),
    Second(FormatStyle),
    Millisecond(FormatStyle),
    Microsecond(FormatStyle),
    Nanosecond(FormatStyle),
    AmPm(FormatStyle),
    
    // Timezone tokens
    TimezoneId(FormatStyle),
    TimezoneOffset(FormatStyle),
    TimezoneName(FormatStyle),
    
    // Era tokens
    Era(FormatStyle),
    
    // Literal text
    Literal(String),
    
    // Special characters
    Space,
    Colon,
    Dash,
    Slash,
    Comma,
    Period,
}

/// A complete format pattern composed of multiple tokens.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct FormatPattern {
    tokens: Vec<FormatToken>,
    pattern_string: String,
}

impl FormatPattern {
    /// Creates a new format pattern from a pattern string.
    ///
    /// # Pattern Syntax
    ///
    /// ## Date Patterns
    /// - `yyyy` - 4-digit year (e.g., 2024)
    /// - `yy` - 2-digit year (e.g., 24)
    /// - `MMMM` - Full month name (e.g., January)
    /// - `MMM` - Short month name (e.g., Jan)
    /// - `MM` - 2-digit month (e.g., 01)
    /// - `M` - Month number (e.g., 1)
    /// - `dd` - 2-digit day (e.g., 05)
    /// - `d` - Day number (e.g., 5)
    /// - `EEEE` - Full day name (e.g., Monday)
    /// - `EEE` - Short day name (e.g., Mon)
    /// - `DDD` - Day of year (e.g., 365)
    /// - `Q` - Quarter (e.g., 1)
    ///
    /// ## Time Patterns
    /// - `HH` - 24-hour format hour (e.g., 14)
    /// - `H` - 24-hour format hour, no padding (e.g., 14)
    /// - `hh` - 12-hour format hour (e.g., 02)
    /// - `h` - 12-hour format hour, no padding (e.g., 2)
    /// - `mm` - Minute (e.g., 30)
    /// - `m` - Minute, no padding (e.g., 30)
    /// - `ss` - Second (e.g., 45)
    /// - `s` - Second, no padding (e.g., 45)
    /// - `SSS` - Millisecond (e.g., 123)
    /// - `SSSSSS` - Microsecond (e.g., 123456)
    /// - `SSSSSSSSS` - Nanosecond (e.g., 123456789)
    /// - `a` - AM/PM marker
    ///
    /// ## Timezone Patterns
    /// - `z` - Timezone name (e.g., UTC, EST)
    /// - `Z` - Timezone offset (e.g., +0000, -0500)
    /// - `v` - Timezone ID (e.g., America/New_York)
    ///
    /// ## Literal Text
    /// - Any text not matching the above patterns is treated as literal
    /// - Use single quotes to escape pattern letters: 'yyyy' becomes "yyyy"
    ///
    /// # Examples
    /// ```ignore
    /// let pattern = FormatPattern::new("yyyy-MM-dd HH:mm:ss")?res!();
    /// let pattern = FormatPattern::new("MMMM d, yyyy 'at' h:mm a")?res!();
    /// let pattern = FormatPattern::new("EEE, MMM d, ''yy")?res!();
    /// ```
    pub fn new(pattern: &str) -> Outcome<Self> {
        let tokens = res!(Self::parse_pattern(pattern));
        Ok(Self {
            tokens,
            pattern_string: pattern.to_string(),
        })
    }
    
    /// Returns the tokens that make up this pattern.
    pub fn tokens(&self) -> &[FormatToken] {
        &self.tokens
    }
    
    /// Returns the original pattern string.
    pub fn pattern_string(&self) -> &str {
        &self.pattern_string
    }
    
    /// Parses a pattern string into format tokens.
    fn parse_pattern(pattern: &str) -> Outcome<Vec<FormatToken>> {
        let mut tokens = Vec::new();
        let mut chars = pattern.chars().peekable();
        
        while let Some(ch) = chars.next() {
            match ch {
                // Handle quoted literals
                '\'' => {
                    let mut literal = String::new();
                    let mut escaped = false;
                    
                    while let Some(ch) = chars.next() {
                        if ch == '\'' {
                            if chars.peek() == Some(&'\'') {
                                // Double quote becomes single quote
                                chars.next();
                                literal.push('\'');
                            } else {
                                // End of quoted section
                                escaped = true;
                                break;
                            }
                        } else {
                            literal.push(ch);
                        }
                    }
                    
                    if !escaped {
                        return Err(err!("Unterminated quoted literal in pattern"; Invalid, Input));
                    }
                    
                    if !literal.is_empty() {
                        tokens.push(FormatToken::Literal(literal));
                    }
                },
                
                // Year patterns
                'y' => {
                    let count = Self::count_consecutive(&mut chars, 'y') + 1;
                    match count {
                        2 => tokens.push(FormatToken::Year(FormatStyle::Custom { width: 2, pad_char: '0' })),
                        4 => tokens.push(FormatToken::Year(FormatStyle::Numeric)),
                        _ => tokens.push(FormatToken::Year(FormatStyle::Numeric)),
                    }
                },
                
                // Month patterns
                'M' => {
                    let count = Self::count_consecutive(&mut chars, 'M') + 1;
                    match count {
                        1 => tokens.push(FormatToken::Month(FormatStyle::Numeric)),
                        2 => tokens.push(FormatToken::Month(FormatStyle::Custom { width: 2, pad_char: '0' })),
                        3 => tokens.push(FormatToken::Month(FormatStyle::Short)),
                        4 => tokens.push(FormatToken::Month(FormatStyle::Long)),
                        _ => tokens.push(FormatToken::Month(FormatStyle::Long)),
                    }
                },
                
                // Day patterns
                'd' => {
                    let count = Self::count_consecutive(&mut chars, 'd') + 1;
                    match count {
                        1 => tokens.push(FormatToken::Day(FormatStyle::Numeric)),
                        2 => tokens.push(FormatToken::Day(FormatStyle::Custom { width: 2, pad_char: '0' })),
                        _ => tokens.push(FormatToken::Day(FormatStyle::Custom { width: 2, pad_char: '0' })),
                    }
                },
                
                // Day of week patterns
                'E' => {
                    let count = Self::count_consecutive(&mut chars, 'E') + 1;
                    match count {
                        1..=3 => tokens.push(FormatToken::DayOfWeek(FormatStyle::Short)),
                        4 => tokens.push(FormatToken::DayOfWeek(FormatStyle::Long)),
                        _ => tokens.push(FormatToken::DayOfWeek(FormatStyle::Long)),
                    }
                },
                
                // Day of year patterns
                'D' => {
                    let count = Self::count_consecutive(&mut chars, 'D') + 1;
                    match count {
                        1 => tokens.push(FormatToken::DayOfYear(FormatStyle::Numeric)),
                        2 => tokens.push(FormatToken::DayOfYear(FormatStyle::Custom { width: 2, pad_char: '0' })),
                        3 => tokens.push(FormatToken::DayOfYear(FormatStyle::Custom { width: 3, pad_char: '0' })),
                        _ => tokens.push(FormatToken::DayOfYear(FormatStyle::Custom { width: 3, pad_char: '0' })),
                    }
                },
                
                // Quarter patterns
                'Q' => {
                    let _count = Self::count_consecutive(&mut chars, 'Q');
                    tokens.push(FormatToken::Quarter(FormatStyle::Numeric));
                },
                
                // 24-hour patterns
                'H' => {
                    let count = Self::count_consecutive(&mut chars, 'H') + 1;
                    match count {
                        1 => tokens.push(FormatToken::Hour24(FormatStyle::Numeric)),
                        2 => tokens.push(FormatToken::Hour24(FormatStyle::Custom { width: 2, pad_char: '0' })),
                        _ => tokens.push(FormatToken::Hour24(FormatStyle::Custom { width: 2, pad_char: '0' })),
                    }
                },
                
                // 12-hour patterns
                'h' => {
                    let count = Self::count_consecutive(&mut chars, 'h') + 1;
                    match count {
                        1 => tokens.push(FormatToken::Hour12(FormatStyle::Numeric)),
                        2 => tokens.push(FormatToken::Hour12(FormatStyle::Custom { width: 2, pad_char: '0' })),
                        _ => tokens.push(FormatToken::Hour12(FormatStyle::Custom { width: 2, pad_char: '0' })),
                    }
                },
                
                // Minute patterns
                'm' => {
                    let count = Self::count_consecutive(&mut chars, 'm') + 1;
                    match count {
                        1 => tokens.push(FormatToken::Minute(FormatStyle::Numeric)),
                        2 => tokens.push(FormatToken::Minute(FormatStyle::Custom { width: 2, pad_char: '0' })),
                        _ => tokens.push(FormatToken::Minute(FormatStyle::Custom { width: 2, pad_char: '0' })),
                    }
                },
                
                // Second patterns
                's' => {
                    let count = Self::count_consecutive(&mut chars, 's') + 1;
                    match count {
                        1 => tokens.push(FormatToken::Second(FormatStyle::Numeric)),
                        2 => tokens.push(FormatToken::Second(FormatStyle::Custom { width: 2, pad_char: '0' })),
                        _ => tokens.push(FormatToken::Second(FormatStyle::Custom { width: 2, pad_char: '0' })),
                    }
                },
                
                // Subsecond patterns
                'S' => {
                    let count = Self::count_consecutive(&mut chars, 'S') + 1;
                    match count {
                        1..=3 => tokens.push(FormatToken::Millisecond(FormatStyle::Custom { width: count, pad_char: '0' })),
                        4..=6 => tokens.push(FormatToken::Microsecond(FormatStyle::Custom { width: count, pad_char: '0' })),
                        7..=9 => tokens.push(FormatToken::Nanosecond(FormatStyle::Custom { width: count, pad_char: '0' })),
                        _ => tokens.push(FormatToken::Nanosecond(FormatStyle::Custom { width: 9, pad_char: '0' })),
                    }
                },
                
                // AM/PM patterns
                'a' => {
                    let _count = Self::count_consecutive(&mut chars, 'a');
                    tokens.push(FormatToken::AmPm(FormatStyle::Short));
                },
                
                // Timezone patterns
                'z' => {
                    let count = Self::count_consecutive(&mut chars, 'z') + 1;
                    match count {
                        1..=3 => tokens.push(FormatToken::TimezoneName(FormatStyle::Short)),
                        4 => tokens.push(FormatToken::TimezoneName(FormatStyle::Long)),
                        _ => tokens.push(FormatToken::TimezoneName(FormatStyle::Long)),
                    }
                },
                
                'Z' => {
                    let _count = Self::count_consecutive(&mut chars, 'Z');
                    tokens.push(FormatToken::TimezoneOffset(FormatStyle::Short));
                },
                
                'v' => {
                    let _count = Self::count_consecutive(&mut chars, 'v');
                    tokens.push(FormatToken::TimezoneId(FormatStyle::Short));
                },
                
                // Common separators
                ' ' => tokens.push(FormatToken::Space),
                ':' => tokens.push(FormatToken::Colon),
                '-' => tokens.push(FormatToken::Dash),
                '/' => tokens.push(FormatToken::Slash),
                ',' => tokens.push(FormatToken::Comma),
                '.' => tokens.push(FormatToken::Period),
                
                // Everything else is literal
                _ => {
                    // Collect consecutive literal characters
                    let mut literal = String::new();
                    literal.push(ch);
                    
                    while let Some(&next_ch) = chars.peek() {
                        if Self::is_pattern_char(next_ch) || next_ch == '\'' {
                            break;
                        }
                        literal.push(chars.next().unwrap());
                    }
                    
                    tokens.push(FormatToken::Literal(literal));
                },
            }
        }
        
        Ok(tokens)
    }
    
    /// Counts consecutive occurrences of a character and consumes them.
    fn count_consecutive(chars: &mut std::iter::Peekable<std::str::Chars>, target: char) -> usize {
        let mut count = 0;
        while chars.peek() == Some(&target) {
            chars.next();
            count += 1;
        }
        count
    }
    
    /// Returns true if the character is a format pattern character.
    fn is_pattern_char(ch: char) -> bool {
        matches!(ch, 'y' | 'M' | 'd' | 'E' | 'D' | 'Q' | 'H' | 'h' | 'm' | 's' | 'S' | 'a' | 'z' | 'Z' | 'v' | ' ' | ':' | '-' | '/' | ',' | '.')
    }
    
    // ========================================================================
    // Predefined Common Patterns
    // ========================================================================
    
    /// ISO 8601 date format: "2024-01-15"
    pub fn iso_date() -> Self {
        Self::new("yyyy-MM-dd").unwrap()
    }
    
    /// ISO 8601 time format: "14:30:45"
    pub fn iso_time() -> Self {
        Self::new("HH:mm:ss").unwrap()
    }
    
    /// ISO 8601 datetime format: "2024-01-15T14:30:45"
    pub fn iso_datetime() -> Self {
        Self::new("yyyy-MM-dd'T'HH:mm:ss").unwrap()
    }
    
    /// ISO 8601 datetime with timezone: "2024-01-15T14:30:45Z"
    pub fn iso_datetime_utc() -> Self {
        Self::new("yyyy-MM-dd'T'HH:mm:ss'Z'").unwrap()
    }
    
    /// US date format: "01/15/2024"
    pub fn us_date() -> Self {
        Self::new("MM/dd/yyyy").unwrap()
    }
    
    /// European date format: "15/01/2024"
    pub fn european_date() -> Self {
        Self::new("dd/MM/yyyy").unwrap()
    }
    
    /// Long date format: "January 15, 2024"
    pub fn long_date() -> Self {
        Self::new("MMMM d, yyyy").unwrap()
    }
    
    /// Full date format: "Monday, January 15, 2024"
    pub fn full_date() -> Self {
        Self::new("EEEE, MMMM d, yyyy").unwrap()
    }
    
    /// 12-hour time format: "2:30:45 PM"
    pub fn time_12h() -> Self {
        Self::new("h:mm:ss a").unwrap()
    }
    
    /// 24-hour time format: "14:30:45"
    pub fn time_24h() -> Self {
        Self::new("HH:mm:ss").unwrap()
    }
    
    /// Short time format: "2:30 PM"
    pub fn time_short() -> Self {
        Self::new("h:mm a").unwrap()
    }
    
    /// RFC 2822 format: "Mon, 15 Jan 2024 14:30:45 +0000"
    pub fn rfc2822() -> Self {
        Self::new("EEE, d MMM yyyy HH:mm:ss Z").unwrap()
    }
    
    /// Common log format: "15/Jan/2024:14:30:45 +0000"
    pub fn common_log() -> Self {
        Self::new("dd/MMM/yyyy:HH:mm:ss Z").unwrap()
    }
}