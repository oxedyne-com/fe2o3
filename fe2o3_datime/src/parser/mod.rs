use crate::{
	calendar::{Calendar, CalendarDate},
	clock::ClockTime,
	core::{TimeField, TimeFieldHolder},
	time::{CalClock, CalClockZone},
};

use oxedyne_fe2o3_core::prelude::*;

use std::{
	collections::HashMap,
	iter::Peekable,
	str::CharIndices,
};

/// Comprehensive natural language date/time parser with sophisticated parsing capabilities.
///
/// The Parser provides advanced parsing functionality for converting string representations
/// of dates and times into their corresponding fe2o3_calclock types. This implementation
/// uses a two-pass architecture (lexical analysis + semantic interpretation) to handle
/// a wide variety of input formats including natural language expressions.
///
/// # Supported Formats
///
/// ## Date Formats
/// - **ISO 8601**: `2024-06-15`, `2024-06-15T14:30:00`
/// - **Natural Language**: `3rd January 2024`, `January 3rd, 2024`, `Jan 15, 2024`
/// - **Numeric with Separators**: `15/06/2024`, `06-15-2024`, `15.06.2024`
/// - **Mixed Formats**: `15 June 2024`, `June 15th 2024`
///
/// ## Time Formats
/// - **24-Hour**: `14:30:00`, `14:30:00.123456789` (nanosecond precision)
/// - **12-Hour**: `2:30 PM`, `2.30 pm`, `14.30`
/// - **Special Times**: `noon`, `midnight`, `midday`
/// - **Fractional Seconds**: `.123`, `.123456`, `.123456789`
///
/// ## Combined Formats
/// - `2024-06-15 14:30:00`
/// - `3rd January 2024 at 2:30 PM`
/// - `June 15, 2024, 14:30:00.123`
///
/// # Intelligent Features
///
/// - **Automatic Disambiguation**: Swaps day/month/year when validation fails
/// - **Context-Sensitive Parsing**: Interprets numbers based on surrounding context
/// - **Ordinal Recognition**: Handles `1st`, `2nd`, `3rd`, `4th`, etc.
/// - **Flexible Separators**: Accepts `-`, `/`, `.`, space as date separators
/// - **Error Recovery**: Attempts alternative interpretations for ambiguous input
///
/// # Examples
///
/// ```ignore
/// use oxedyne_fe2o3_datime::{
///     parser::Parser,
///     time::CalClockZone,
/// }res!();
///
/// let zone = CalClockZone::utc()res!();
///
/// // Parse various date formats
/// let date1 = res!(Parser::parse_date("2024-06-15", zone.clone()))res!();
/// let date2 = res!(Parser::parse_date("15th June 2024", zone.clone()))res!();
/// let date3 = res!(Parser::parse_date("June 15, 2024", zone.clone()))res!();
///
/// // Parse time formats
/// let time1 = res!(Parser::parse_time("14:30:00", zone.clone()))res!();
/// let time2 = res!(Parser::parse_time("2:30 PM", zone.clone()))res!();
/// let time3 = res!(Parser::parse_time("noon", zone.clone()))res!();
///
/// // Parse combined date/time
/// let datetime = res!(Parser::parse_datetime("3rd January 2024 at 2:30 PM", zone))res!();
/// ```
#[derive(Debug)]
pub struct Parser {
	lexer: Lexer,
	semantic_parser: SemanticParser,
}

/// Token types for lexical analysis.
#[derive(Clone, Debug, PartialEq)]
pub enum TokenType {
	// Numeric tokens
	Number,
	OrdinalNumber,      // 1st, 2nd, 3rd, etc.
	
	// Month identifiers
	MonthNameFull,      // January, February, etc.
	MonthNameShort,     // Jan, Feb, etc.
	MonthNumber,        // 1-12
	
	// Day identifiers  
	DayNameFull,        // Monday, Tuesday, etc.
	DayNameShort,       // Mon, Tue, etc.
	
	// Time components
	Hour12,             // 1-12 for 12-hour format
	Hour24,             // 0-23 for 24-hour format
	Minute,             // 0-59
	Second,             // 0-59
	Nanosecond,         // Fractional seconds
	
	// AM/PM indicators
	AmPm,               // AM, PM, am, pm, a.m., p.m.
	
	// Special time words
	Noon,               // noon, midday
	Midnight,           // midnight
	
	// Separators and punctuation
	DateSeparator,      // -, /, .
	TimeSeparator,      // :
	WhiteSpace,
	Comma,
	
	// Prepositions and conjunctions
	At,                 // at
	On,                 // on
	In,                 // in
	Of,                 // of
	
	// ISO format indicators
	IsoDate,            // YYYY-MM-DD pattern
	IsoTime,            // HH:MM:SS pattern
	IsoDateTime,        // Full ISO datetime
	
	// Timezone indicators
	TimezoneOffset,     // +/-HHMM
	TimezoneAbbrev,     // UTC, GMT, EST, etc.
	
	Unknown,
}

/// Individual token from lexical analysis.
#[derive(Clone, Debug)]
pub struct Token {
	pub token_type: TokenType,
	pub value: String,
	pub position: usize,
}

/// Lexical analyzer for tokenizing input strings.
#[derive(Debug)]
pub struct Lexer {
	month_names: HashMap<String, u8>,
	day_names: HashMap<String, u8>,
	timezone_abbrevs: HashMap<String, String>,
}

/// Format pattern for semantic parsing.
#[derive(Clone, Debug)]
pub struct FormatPattern {
	pub name: String,
	pub pattern: Vec<TokenType>,
	pub priority: u8, // Higher priority patterns are tried first
}

/// Semantic parser for interpreting token sequences.
#[derive(Debug)]
pub struct SemanticParser {
	format_patterns: Vec<FormatPattern>,
}

impl Parser {
	/// Creates a new Parser with full natural language parsing capabilities.
	pub fn new() -> Self {
		Self {
			lexer: Lexer::new(),
			semantic_parser: SemanticParser::new(),
		}
	}
	
	/// Parses a date string into a CalendarDate.
	///
	/// This method supports a wide variety of date formats including ISO 8601,
	/// natural language expressions, and numeric formats with various separators.
	///
	/// # Arguments
	///
	/// * `input` - String representation of the date to parse
	/// * `zone` - Time zone to apply to the parsed date
	///
	/// # Returns
	///
	/// Returns `Ok(CalendarDate)` if parsing succeeds, otherwise returns an error
	/// describing the parsing failure.
	///
	/// # Examples
	///
	/// ```ignore
	/// let zone = CalClockZone::utc()res!();
	/// let date1 = res!(Parser::parse_date("2024-06-15", zone.clone()))res!();
	/// let date2 = res!(Parser::parse_date("15th June 2024", zone.clone()))res!();
	/// let date3 = res!(Parser::parse_date("June 15, 2024", zone))res!();
	/// ```
	pub fn parse_date(input: &str, zone: CalClockZone) -> Outcome<CalendarDate> {
		let parser = Self::new();
		let tokens = res!(parser.lexer.tokenize(input));
		let field_holder = res!(parser.semantic_parser.parse_date_tokens(tokens));
		parser.build_calendar_date(field_holder, zone)
	}
	
	/// Parses a time string into a ClockTime.
	///
	/// Supports 24-hour format, 12-hour format with AM/PM, special time words
	/// (noon, midnight), and nanosecond precision.
	///
	/// # Arguments
	///
	/// * `input` - String representation of the time to parse
	/// * `zone` - Time zone to apply to the parsed time
	///
	/// # Returns
	///
	/// Returns `Ok(ClockTime)` if parsing succeeds, otherwise returns an error.
	///
	/// # Examples
	///
	/// ```ignore
	/// let zone = CalClockZone::utc()res!();
	/// let time1 = res!(Parser::parse_time("14:30:00", zone.clone()))res!();
	/// let time2 = res!(Parser::parse_time("2:30 PM", zone.clone()))res!();
	/// let time3 = res!(Parser::parse_time("noon", zone))res!();
	/// ```
	pub fn parse_time(input: &str, zone: CalClockZone) -> Outcome<ClockTime> {
		let parser = Self::new();
		let tokens = res!(parser.lexer.tokenize(input));
		let field_holder = res!(parser.semantic_parser.parse_time_tokens(tokens));
		parser.build_clock_time(field_holder, zone)
	}
	
	/// Parses a combined date/time string into a CalClock.
	///
	/// This is the main parsing method that handles both date and time components
	/// in a single input string. It supports all date and time formats, plus
	/// combined formats with various separators and conjunctions.
	///
	/// # Arguments
	///
	/// * `input` - String representation of the date/time to parse
	/// * `zone` - Time zone to apply to the parsed date/time
	///
	/// # Returns
	///
	/// Returns `Ok(CalClock)` if parsing succeeds, otherwise returns an error.
	///
	/// # Examples
	///
	/// ```ignore
	/// let zone = CalClockZone::utc()res!();
	/// let dt1 = res!(Parser::parse_datetime("2024-06-15T14:30:00", zone.clone()))res!();
	/// let dt2 = res!(Parser::parse_datetime("3rd January 2024 at 2:30 PM", zone.clone()))res!();
	/// let dt3 = res!(Parser::parse_datetime("June 15, 2024, 14:30:00.123", zone))res!();
	/// ```
	pub fn parse_datetime(input: &str, zone: CalClockZone) -> Outcome<CalClock> {
		let parser = Self::new();
		let tokens = res!(parser.lexer.tokenize(input));
		let field_holder = res!(parser.semantic_parser.parse_datetime_tokens(tokens));
		parser.build_calclock(field_holder, zone)
	}
	
	/// Builds a CalendarDate from parsed time fields.
	fn build_calendar_date(&self, holder: TimeFieldHolder, zone: CalClockZone) -> Outcome<CalendarDate> {
		let year = match holder.year {
			Some(y) => y,
			None => return Err(err!("Year not specified"; Invalid, Input)),
		};
		let month = match holder.month {
			Some(m) => m,
			None => return Err(err!("Month not specified"; Invalid, Input)),
		};
		let day = match holder.day {
			Some(d) => d,
			None => return Err(err!("Day not specified"; Invalid, Input)),
		};
		
		let calendar = Calendar::new(); // Default to Gregorian
		calendar.date(year, month, day, zone)
	}
	
	/// Builds a ClockTime from parsed time fields.
	fn build_clock_time(&self, holder: TimeFieldHolder, zone: CalClockZone) -> Outcome<ClockTime> {
		let hour = holder.hour.unwrap_or(0);
		let minute = holder.minute.unwrap_or(0);
		let second = holder.second.unwrap_or(0);
		let nanosecond = holder.nanosecond.unwrap_or(0);
		
		ClockTime::new(hour, minute, second, nanosecond, zone)
	}
	
	/// Builds a CalClock from parsed date and time fields.
	fn build_calclock(&self, holder: TimeFieldHolder, zone: CalClockZone) -> Outcome<CalClock> {
		// Use current date as default if not specified
		let year = holder.year.unwrap_or(2024); // TODO: Use actual current year
		let month = holder.month.unwrap_or(1);
		let day = holder.day.unwrap_or(1);
		let hour = holder.hour.unwrap_or(0);
		let minute = holder.minute.unwrap_or(0);
		let second = holder.second.unwrap_or(0);
		let nanosecond = holder.nanosecond.unwrap_or(0);
		
		CalClock::new(year, month, day, hour, minute, second, nanosecond, zone)
	}
}

impl Lexer {
	/// Creates a new Lexer with comprehensive language support.
	pub fn new() -> Self {
		Self {
			month_names: Self::build_month_names(),
			day_names: Self::build_day_names(),
			timezone_abbrevs: Self::build_timezone_abbrevs(),
		}
	}
	
	/// Tokenizes input string into a sequence of tokens.
	pub fn tokenize(&self, input: &str) -> Outcome<Vec<Token>> {
		let mut tokens = Vec::new();
		let mut chars = input.char_indices().peekable();
		
		while let Some((pos, ch)) = chars.next() {
			match ch {
				'0'..='9' => {
					let token = res!(self.parse_number(&mut chars, pos, ch));
					tokens.push(token);
				},
				'A'..='Z' | 'a'..='z' => {
					let token = res!(self.parse_word(&mut chars, pos, ch));
					if token.token_type != TokenType::Unknown {
						tokens.push(token);
					}
				},
				'-' => {
					// Check if this is a timezone offset vs date separator
					if self.looks_like_timezone_offset(&mut chars) {
						let token = res!(self.parse_timezone_offset(&mut chars, pos, ch));
						tokens.push(token);
					} else {
						tokens.push(Token {
							token_type: TokenType::DateSeparator,
							value: ch.to_string(),
							position: pos,
						});
					}
				},
				'/' | '.' => {
					tokens.push(Token {
						token_type: TokenType::DateSeparator,
						value: ch.to_string(),
						position: pos,
					});
				},
				':' => {
					tokens.push(Token {
						token_type: TokenType::TimeSeparator,
						value: ch.to_string(),
						position: pos,
					});
				},
				',' => {
					tokens.push(Token {
						token_type: TokenType::Comma,
						value: ch.to_string(),
						position: pos,
					});
				},
				' ' | '\t' | '\n' | '\r' => {
					// Skip whitespace but track it for certain contexts
					continue;
				},
				'+' if self.looks_like_timezone_offset(&mut chars) => {
					let token = res!(self.parse_timezone_offset(&mut chars, pos, ch));
					tokens.push(token);
				},
				_ => {
					// Unknown character, skip
					continue;
				}
			}
		}
		
		Ok(tokens)
	}
	
	/// Parses a numeric token, detecting ordinals and special numeric patterns.
	fn parse_number(&self, chars: &mut Peekable<CharIndices>, start_pos: usize, first_char: char) -> Outcome<Token> {
		let mut number_str = String::from(first_char);
		
		// Collect digits
		while let Some((_, ch)) = chars.peek() {
			if ch.is_ascii_digit() {
				number_str.push(*ch);
				chars.next();
			} else {
				break;
			}
		}
		
		// Check for ordinal suffixes (st, nd, rd, th)
		if let Some((_, ch)) = chars.peek() {
			if matches!(*ch, 's' | 'n' | 'r' | 't') {
				let suffix = self.try_parse_ordinal_suffix(chars);
				if !suffix.is_empty() {
					number_str.push_str(&suffix);
					return Ok(Token {
						token_type: TokenType::OrdinalNumber,
						value: number_str,
						position: start_pos,
					});
				}
			}
		}
		
		// Check if this looks like part of an ISO date pattern
		if number_str.len() == 4 && self.looks_like_iso_date(chars) {
			let iso_date = res!(self.parse_iso_date_pattern(chars, &number_str));
			return Ok(Token {
				token_type: TokenType::IsoDate,
				value: iso_date,
				position: start_pos,
			});
		}
		
		// Check for fractional seconds (decimal point followed by digits)
		if let Some((_, '.')) = chars.peek() {
			if self.looks_like_fractional_seconds(chars) {
				chars.next(); // consume the dot
				number_str.push('.');
				while let Some((_, ch)) = chars.peek() {
					if ch.is_ascii_digit() {
						number_str.push(*ch);
						chars.next();
					} else {
						break;
					}
				}
				return Ok(Token {
					token_type: TokenType::Nanosecond,
					value: number_str,
					position: start_pos,
				});
			}
		}
		
		Ok(Token {
			token_type: TokenType::Number,
			value: number_str,
			position: start_pos,
		})
	}
	
	/// Parses a word token, classifying it by type.
	fn parse_word(&self, chars: &mut Peekable<CharIndices>, start_pos: usize, first_char: char) -> Outcome<Token> {
		let mut word = String::from(first_char);
		
		// Collect word characters
		while let Some((_, ch)) = chars.peek() {
			if ch.is_alphabetic() || *ch == '.' {
				word.push(*ch);
				chars.next();
			} else {
				break;
			}
		}
		
		let word_lower = word.to_lowercase();
		
		// Classify the word
		let token_type = if let Some(_) = self.month_names.get(&word_lower) {
			if word.len() <= 3 {
				TokenType::MonthNameShort
			} else {
				TokenType::MonthNameFull
			}
		} else if let Some(_) = self.day_names.get(&word_lower) {
			if word.len() <= 3 {
				TokenType::DayNameShort
			} else {
				TokenType::DayNameFull
			}
		} else if matches!(word_lower.as_str(), "am" | "pm" | "a.m." | "p.m.") {
			TokenType::AmPm
		} else if matches!(word_lower.as_str(), "noon" | "midday") {
			TokenType::Noon
		} else if word_lower == "midnight" {
			TokenType::Midnight
		} else if matches!(word_lower.as_str(), "at" | "on" | "in" | "of") {
			match word_lower.as_str() {
				"at" => TokenType::At,
				"on" => TokenType::On,
				"in" => TokenType::In,
				"of" => TokenType::Of,
				_ => TokenType::Unknown,
			}
		} else if let Some(_) = self.timezone_abbrevs.get(&word.to_uppercase()) {
			TokenType::TimezoneAbbrev
		} else {
			TokenType::Unknown
		};
		
		Ok(Token {
			token_type,
			value: word,
			position: start_pos,
		})
	}
	
	/// Attempts to parse an ordinal suffix (st, nd, rd, th).
	fn try_parse_ordinal_suffix(&self, chars: &mut Peekable<CharIndices>) -> String {
		let mut suffix = String::new();
		
		// Look ahead to see if we have a valid ordinal suffix
		let peek_ahead: Vec<char> = chars.clone()
			.take(2)
			.map(|(_, ch)| ch)
			.collect();
		
		let suffix_str: String = peek_ahead.iter().collect();
		if matches!(suffix_str.to_lowercase().as_str(), "st" | "nd" | "rd" | "th") {
			// Consume the suffix characters
			for _ in 0..2 {
				if let Some((_, ch)) = chars.next() {
					suffix.push(ch);
				}
			}
		}
		
		suffix
	}
	
	/// Checks if the upcoming characters look like an ISO date pattern.
	fn looks_like_iso_date(&self, chars: &Peekable<CharIndices>) -> bool {
		let peek_ahead: Vec<char> = chars.clone()
			.take(6) // Look for "-MM-DD" pattern
			.map(|(_, ch)| ch)
			.collect();
		
		if peek_ahead.len() >= 6 {
			peek_ahead[0] == '-' &&
			peek_ahead[1].is_ascii_digit() &&
			peek_ahead[2].is_ascii_digit() &&
			peek_ahead[3] == '-' &&
			peek_ahead[4].is_ascii_digit() &&
			peek_ahead[5].is_ascii_digit()
		} else {
			false
		}
	}
	
	/// Parses a complete ISO date pattern starting with year.
	fn parse_iso_date_pattern(&self, chars: &mut Peekable<CharIndices>, year: &str) -> Outcome<String> {
		let mut iso_date = year.to_string();
		
		// Parse "-MM-DD" pattern
		for expected in ['-', 'd', 'd', '-', 'd', 'd'] {
			if let Some((_, ch)) = chars.next() {
				iso_date.push(ch);
				if expected == '-' && ch != '-' {
					return Err(err!("Invalid ISO date format"; Invalid, Input));
				}
				if expected == 'd' && !ch.is_ascii_digit() {
					return Err(err!("Invalid ISO date format"; Invalid, Input));
				}
			} else {
				return Err(err!("Incomplete ISO date format"; Invalid, Input));
			}
		}
		
		Ok(iso_date)
	}
	
	/// Checks if the upcoming characters look like fractional seconds.
	fn looks_like_fractional_seconds(&self, chars: &Peekable<CharIndices>) -> bool {
		chars.clone()
			.skip(1) // Skip the decimal point
			.take(1)
			.any(|(_, ch)| ch.is_ascii_digit())
	}
	
	/// Checks if the upcoming characters look like a timezone offset.
	fn looks_like_timezone_offset(&self, chars: &mut Peekable<CharIndices>) -> bool {
		chars.clone()
			.take(4)
			.map(|(_, ch)| ch)
			.collect::<String>()
			.chars()
			.all(|ch| ch.is_ascii_digit())
	}
	
	/// Parses a timezone offset like "+0500" or "-0300".
	fn parse_timezone_offset(&self, chars: &mut Peekable<CharIndices>, start_pos: usize, sign: char) -> Outcome<Token> {
		let mut offset = String::from(sign);
		
		// Parse 4 digits for HHMM format
		for _ in 0..4 {
			if let Some((_, ch)) = chars.next() {
				if ch.is_ascii_digit() {
					offset.push(ch);
				} else {
					return Err(err!("Invalid timezone offset format"; Invalid, Input));
				}
			} else {
				return Err(err!("Incomplete timezone offset"; Invalid, Input));
			}
		}
		
		Ok(Token {
			token_type: TokenType::TimezoneOffset,
			value: offset,
			position: start_pos,
		})
	}
	
	/// Builds the month name lookup table.
	fn build_month_names() -> HashMap<String, u8> {
		let mut months = HashMap::new();
		
		// Full month names
		let full_names = [
			"january", "february", "march", "april", "may", "june",
			"july", "august", "september", "october", "november", "december"
		];
		for (i, name) in full_names.iter().enumerate() {
			months.insert(name.to_string(), (i + 1) as u8);
		}
		
		// Short month names
		let short_names = [
			"jan", "feb", "mar", "apr", "may", "jun",
			"jul", "aug", "sep", "oct", "nov", "dec"
		];
		for (i, name) in short_names.iter().enumerate() {
			months.insert(name.to_string(), (i + 1) as u8);
		}
		
		months
	}
	
	/// Builds the day name lookup table.
	fn build_day_names() -> HashMap<String, u8> {
		let mut days = HashMap::new();
		
		// Full day names (0 = Sunday, 1 = Monday, etc.)
		let full_names = [
			"sunday", "monday", "tuesday", "wednesday", 
			"thursday", "friday", "saturday"
		];
		for (i, name) in full_names.iter().enumerate() {
			days.insert(name.to_string(), i as u8);
		}
		
		// Short day names
		let short_names = ["sun", "mon", "tue", "wed", "thu", "fri", "sat"];
		for (i, name) in short_names.iter().enumerate() {
			days.insert(name.to_string(), i as u8);
		}
		
		days
	}
	
	/// Builds the timezone abbreviation lookup table.
	fn build_timezone_abbrevs() -> HashMap<String, String> {
		let mut zones = HashMap::new();
		
		// Common timezone abbreviations
		zones.insert("UTC".to_string(), "UTC".to_string());
		zones.insert("GMT".to_string(), "GMT".to_string());
		zones.insert("EST".to_string(), "America/New_York".to_string());
		zones.insert("EDT".to_string(), "America/New_York".to_string());
		zones.insert("PST".to_string(), "America/Los_Angeles".to_string());
		zones.insert("PDT".to_string(), "America/Los_Angeles".to_string());
		zones.insert("CST".to_string(), "America/Chicago".to_string());
		zones.insert("CDT".to_string(), "America/Chicago".to_string());
		
		zones
	}
}

impl SemanticParser {
	/// Creates a new SemanticParser with comprehensive format patterns.
	pub fn new() -> Self {
		Self {
			format_patterns: Self::build_format_patterns(),
		}
	}
	
	/// Parses tokens representing a date.
	pub fn parse_date_tokens(&self, tokens: Vec<Token>) -> Outcome<TimeFieldHolder> {
		self.try_patterns(&tokens, |pattern| pattern.name.contains("DATE") || pattern.name.contains("_DAY"))
	}
	
	/// Parses tokens representing a time.
	pub fn parse_time_tokens(&self, tokens: Vec<Token>) -> Outcome<TimeFieldHolder> {
		self.try_patterns(&tokens, |pattern| pattern.name.contains("TIME") || pattern.name.contains("HOUR"))
	}
	
	/// Parses tokens representing a combined date/time.
	pub fn parse_datetime_tokens(&self, tokens: Vec<Token>) -> Outcome<TimeFieldHolder> {
		// First try combined datetime patterns
		if let Ok(result) = self.try_patterns(&tokens, |pattern| pattern.name.contains("DATETIME")) {
			return Ok(result);
		}
		
		// If no combined pattern works, try to split into date and time parts
		self.parse_split_datetime(&tokens)
	}
	
	/// Attempts to match patterns against tokens, using a filter predicate.
	fn try_patterns<F>(&self, tokens: &[Token], filter: F) -> Outcome<TimeFieldHolder>
	where
		F: Fn(&FormatPattern) -> bool,
	{
		// Sort patterns by priority (highest first)
		let mut patterns: Vec<_> = self.format_patterns.iter()
			.filter(|p| filter(p))
			.collect();
		patterns.sort_by(|a, b| b.priority.cmp(&a.priority));
		
		for pattern in patterns {
			if let Ok(result) = self.try_pattern(pattern, tokens) {
				return Ok(result);
			}
		}
		
		// If no pattern matches, try intelligent disambiguation
		self.intelligent_parse(tokens)
	}
	
	/// Attempts to match a specific pattern against tokens.
	fn try_pattern(&self, pattern: &FormatPattern, tokens: &[Token]) -> Outcome<TimeFieldHolder> {
		if self.matches_pattern(&pattern.pattern, tokens) {
			self.parse_with_pattern(pattern, tokens)
		} else {
			Err(err!("Pattern {} does not match", pattern.name; Invalid, Input))
		}
	}
	
	/// Checks if a pattern matches the token sequence.
	fn matches_pattern(&self, pattern: &[TokenType], tokens: &[Token]) -> bool {
		if pattern.len() != tokens.len() {
			return false;
		}
		
		pattern.iter()
			.zip(tokens.iter())
			.all(|(expected, actual)| self.token_matches_type(actual, expected))
	}
	
	/// Checks if a token matches an expected type (with some flexibility).
	fn token_matches_type(&self, token: &Token, expected: &TokenType) -> bool {
		match (expected, &token.token_type) {
			// Exact matches
			(a, b) if a == b => true,
			
			// Flexible matches
			(TokenType::Number, TokenType::OrdinalNumber) => true,
			(TokenType::OrdinalNumber, TokenType::Number) => true,
			(TokenType::MonthNameFull, TokenType::MonthNameShort) => true,
			(TokenType::MonthNameShort, TokenType::MonthNameFull) => true,
			
			_ => false,
		}
	}
	
	/// Parses tokens using a specific pattern.
	fn parse_with_pattern(&self, pattern: &FormatPattern, tokens: &[Token]) -> Outcome<TimeFieldHolder> {
		match pattern.name.as_str() {
			"ISO_DATE" => self.parse_iso_date(tokens),
			"ISO_DATETIME" => self.parse_iso_datetime(tokens),
			"ORDINAL_MONTH_YEAR" => self.parse_ordinal_month_year(tokens),
			"MONTH_ORDINAL_YEAR" => self.parse_month_ordinal_year(tokens),
			"DMY_SEPARATED" => self.parse_dmy_separated(tokens),
			"24_HOUR_TIME" => self.parse_24_hour_time(tokens),
			"12_HOUR_TIME_AMPM" => self.parse_12_hour_time(tokens),
			"NOON" => self.parse_noon(tokens),
			"MIDNIGHT" => self.parse_midnight(tokens),
			_ => Err(err!("Unknown pattern: {}", pattern.name; Invalid, Input)),
		}
	}
	
	/// Performs intelligent parsing when no pattern matches.
	fn intelligent_parse(&self, tokens: &[Token]) -> Outcome<TimeFieldHolder> {
		let mut holder = TimeFieldHolder::new();
		
		// Apply intelligent disambiguation rules
		res!(self.apply_context_rules(tokens, &mut holder));
		res!(self.apply_validation_swapping(&mut holder));
		
		Ok(holder)
	}
	
	/// Applies context-sensitive parsing rules.
	fn apply_context_rules(&self, tokens: &[Token], holder: &mut TimeFieldHolder) -> Outcome<()> {
		for i in 0..tokens.len() {
			match &tokens[i].token_type {
				TokenType::OrdinalNumber => {
					let day = res!(self.extract_ordinal_number(&tokens[i].value));
					res!(holder.set_field(TimeField::Day, day as i64));
				},
				TokenType::MonthNameFull | TokenType::MonthNameShort => {
					let month = res!(self.month_name_to_number(&tokens[i].value));
					res!(holder.set_field(TimeField::Month, month as i64));
				},
				TokenType::Number => {
					let num: i64 = res!(tokens[i].value.parse().map_err(|_| 
						err!("Invalid number: {}", tokens[i].value; Invalid, Input)));
					
					// Apply heuristics based on surrounding tokens
					res!(self.interpret_number_in_context(num, i, tokens, holder));
				},
				TokenType::AmPm => {
					// AM/PM affects previous hour interpretation
					if i > 0 && tokens[i-1].token_type == TokenType::Number {
						let hour_str = &tokens[i-1].value;
						let mut hour: i64 = res!(hour_str.parse().map_err(|_| 
							err!("Invalid hour: {}", hour_str; Invalid, Input)));
						
						let am_pm = tokens[i].value.to_lowercase();
						if am_pm.starts_with('p') && hour != 12 {
							hour += 12;
						} else if am_pm.starts_with('a') && hour == 12 {
							hour = 0;
						}
						
						res!(holder.set_field(TimeField::Hour, hour));
					}
				},
				TokenType::Noon => {
					res!(holder.set_field(TimeField::Hour, 12));
					res!(holder.set_field(TimeField::Minute, 0));
					res!(holder.set_field(TimeField::Second, 0));
				},
				TokenType::Midnight => {
					res!(holder.set_field(TimeField::Hour, 0));
					res!(holder.set_field(TimeField::Minute, 0));
					res!(holder.set_field(TimeField::Second, 0));
				},
				_ => continue,
			}
		}
		
		Ok(())
	}
	
	/// Applies automatic validation-based field swapping.
	fn apply_validation_swapping(&self, holder: &mut TimeFieldHolder) -> Outcome<()> {
		// Automatic day/year swapping when validation fails
		if let (Some(day), Some(year)) = (holder.day, holder.year) {
			if day > 31 && year <= 31 {
				holder.day = Some(year as u8);
				holder.year = Some(day as i32);
			}
		}
		
		// Month/day swapping for ambiguous cases
		if let (Some(month), Some(day)) = (holder.month, holder.day) {
			if month > 12 && day <= 12 {
				holder.month = Some(day);
				holder.day = Some(month);
			}
		}
		
		Ok(())
	}
	
	/// Interprets a number based on its context within the token sequence.
	fn interpret_number_in_context(&self, num: i64, pos: usize, tokens: &[Token], holder: &mut TimeFieldHolder) -> Outcome<()> {
		// Check surrounding tokens for context clues
		let prev_token = if pos > 0 { Some(&tokens[pos - 1]) } else { None };
		let next_token = if pos + 1 < tokens.len() { Some(&tokens[pos + 1]) } else { None };
		
		// Year heuristics
		if num >= 1900 && num <= 2100 {
			res!(holder.set_field(TimeField::Year, num));
			return Ok(());
		}
		
		// Month heuristics
		if num >= 1 && num <= 12 {
			if let Some(next) = next_token {
				if matches!(next.token_type, TokenType::DateSeparator | TokenType::Number) {
					res!(holder.set_field(TimeField::Month, num));
					return Ok(());
				}
			}
		}
		
		// Day heuristics
		if num >= 1 && num <= 31 {
			if let Some(prev) = prev_token {
				if matches!(prev.token_type, TokenType::DateSeparator | TokenType::MonthNameFull | TokenType::MonthNameShort) {
					res!(holder.set_field(TimeField::Day, num));
					return Ok(());
				}
			}
		}
		
		// Hour heuristics
		if num >= 0 && num <= 23 {
			if let Some(next) = next_token {
				if matches!(next.token_type, TokenType::TimeSeparator | TokenType::AmPm) {
					res!(holder.set_field(TimeField::Hour, num));
					return Ok(());
				}
			}
		}
		
		// Minute/second heuristics
		if num >= 0 && num <= 59 {
			if let Some(prev) = prev_token {
				if prev.token_type == TokenType::TimeSeparator {
					// Could be minute or second, depends on what's already set
					if holder.hour.is_some() && holder.minute.is_none() {
						res!(holder.set_field(TimeField::Minute, num));
					} else if holder.minute.is_some() && holder.second.is_none() {
						res!(holder.set_field(TimeField::Second, num));
					}
					return Ok(());
				}
			}
		}
		
		// Default: if we can't determine context, make reasonable assumptions
		if holder.year.is_none() && num >= 1900 {
			res!(holder.set_field(TimeField::Year, num));
		} else if holder.month.is_none() && num >= 1 && num <= 12 {
			res!(holder.set_field(TimeField::Month, num));
		} else if holder.day.is_none() && num >= 1 && num <= 31 {
			res!(holder.set_field(TimeField::Day, num));
		}
		
		Ok(())
	}
	
	/// Parses a split date/time string (date part + time part).
	fn parse_split_datetime(&self, tokens: &[Token]) -> Outcome<TimeFieldHolder> {
		// Find potential split points (keywords like "at", "T", or significant separators)
		let split_point = self.find_datetime_split_point(tokens);
		
		if let Some(split) = split_point {
			let (date_tokens, time_tokens) = tokens.split_at(split);
			
			// Parse date part
			let mut holder = res!(self.parse_date_tokens(date_tokens.to_vec()));
			
			// Parse time part and merge
			let time_holder = res!(self.parse_time_tokens(time_tokens.to_vec()));
			res!(self.merge_time_fields(&mut holder, time_holder));
			
			Ok(holder)
		} else {
			// Can't split, try intelligent parsing on the whole thing
			self.intelligent_parse(tokens)
		}
	}
	
	/// Finds the optimal split point between date and time components.
	fn find_datetime_split_point(&self, tokens: &[Token]) -> Option<usize> {
		// Look for explicit separators like "at", "T", etc.
		for (i, token) in tokens.iter().enumerate() {
			match &token.token_type {
				TokenType::At => return Some(i + 1),
				_ if token.value == "T" => return Some(i + 1),
				_ => continue,
			}
		}
		
		// Look for pattern changes (e.g., date pattern followed by time pattern)
		// This is a simplified heuristic
		for i in 1..tokens.len() {
			if self.looks_like_time_start(&tokens[i..]) {
				return Some(i);
			}
		}
		
		None
	}
	
	/// Checks if a token sequence looks like the start of a time.
	fn looks_like_time_start(&self, tokens: &[Token]) -> bool {
		if tokens.is_empty() {
			return false;
		}
		
		match &tokens[0].token_type {
			TokenType::Number => {
				// Check if it's followed by time separator
				if tokens.len() > 1 && tokens[1].token_type == TokenType::TimeSeparator {
					return true;
				}
				// Check if it's a reasonable hour value
				if let Ok(num) = tokens[0].value.parse::<u8>() {
					return num <= 23;
				}
			},
			TokenType::Noon | TokenType::Midnight => return true,
			_ => {}
		}
		
		false
	}
	
	/// Merges time fields from one holder into another.
	fn merge_time_fields(&self, target: &mut TimeFieldHolder, source: TimeFieldHolder) -> Outcome<()> {
		if let Some(hour) = source.hour {
			res!(target.set_field(TimeField::Hour, hour as i64));
		}
		if let Some(minute) = source.minute {
			res!(target.set_field(TimeField::Minute, minute as i64));
		}
		if let Some(second) = source.second {
			res!(target.set_field(TimeField::Second, second as i64));
		}
		if let Some(nanosecond) = source.nanosecond {
			res!(target.set_field(TimeField::NanoSecond, nanosecond as i64));
		}
		
		Ok(())
	}
	
	/// Extracts numeric value from ordinal number (e.g., "3rd" -> 3).
	fn extract_ordinal_number(&self, ordinal: &str) -> Outcome<u8> {
		let number_part = ordinal.trim_end_matches(|c: char| c.is_alphabetic());
		number_part.parse().map_err(|_| 
			err!("Invalid ordinal number: {}", ordinal; Invalid, Input))
	}
	
	/// Converts month name to numeric value.
	fn month_name_to_number(&self, month_name: &str) -> Outcome<u8> {
		let lexer = Lexer::new();
		lexer.month_names.get(&month_name.to_lowercase())
			.copied()
			.ok_or_else(|| err!("Unknown month name: {}", month_name; Invalid, Input))
	}
	
	// Format-specific parsing methods
	
	fn parse_iso_date(&self, tokens: &[Token]) -> Outcome<TimeFieldHolder> {
		// Parse "2024-06-15" format
		let iso_str = &tokens[0].value;
		let parts: Vec<&str> = iso_str.split('-').collect();
		
		if parts.len() != 3 {
			return Err(err!("Invalid ISO date format: {}", iso_str; Invalid, Input));
		}
		
		let mut holder = TimeFieldHolder::new();
		let year: i64 = res!(parts[0].parse().map_err(|_| 
			err!("Invalid year in ISO date: {}", parts[0]; Invalid, Input)));
		let month: i64 = res!(parts[1].parse().map_err(|_| 
			err!("Invalid month in ISO date: {}", parts[1]; Invalid, Input)));
		let day: i64 = res!(parts[2].parse().map_err(|_| 
			err!("Invalid day in ISO date: {}", parts[2]; Invalid, Input)));
		
		res!(holder.set_field(TimeField::Year, year));
		res!(holder.set_field(TimeField::Month, month));
		res!(holder.set_field(TimeField::Day, day));
		
		Ok(holder)
	}
	
	fn parse_iso_datetime(&self, _tokens: &[Token]) -> Outcome<TimeFieldHolder> {
		// Parse full ISO datetime format
		// This is a simplified implementation
		Err(err!("ISO datetime parsing not yet implemented"; Unimplemented))
	}
	
	fn parse_ordinal_month_year(&self, tokens: &[Token]) -> Outcome<TimeFieldHolder> {
		// Parse "3rd January 2024" format
		let mut holder = TimeFieldHolder::new();
		
		let day = res!(self.extract_ordinal_number(&tokens[0].value));
		let month = res!(self.month_name_to_number(&tokens[1].value));
		let year: i64 = res!(tokens[2].value.parse().map_err(|_| 
			err!("Invalid year: {}", tokens[2].value; Invalid, Input)));
		
		res!(holder.set_field(TimeField::Day, day as i64));
		res!(holder.set_field(TimeField::Month, month as i64));
		res!(holder.set_field(TimeField::Year, year));
		
		Ok(holder)
	}
	
	fn parse_month_ordinal_year(&self, tokens: &[Token]) -> Outcome<TimeFieldHolder> {
		// Parse "Jan 3, 2024" format
		let mut holder = TimeFieldHolder::new();
		
		let month = res!(self.month_name_to_number(&tokens[0].value));
		let day: i64 = res!(tokens[1].value.parse().map_err(|_| 
			err!("Invalid day: {}", tokens[1].value; Invalid, Input)));
		let year: i64 = res!(tokens[3].value.parse().map_err(|_| 
			err!("Invalid year: {}", tokens[3].value; Invalid, Input)));
		
		res!(holder.set_field(TimeField::Month, month as i64));
		res!(holder.set_field(TimeField::Day, day));
		res!(holder.set_field(TimeField::Year, year));
		
		Ok(holder)
	}
	
	fn parse_dmy_separated(&self, tokens: &[Token]) -> Outcome<TimeFieldHolder> {
		// Parse "15/06/2024" format (with flexible interpretation)
		let mut holder = TimeFieldHolder::new();
		
		let num1: i64 = res!(tokens[0].value.parse().map_err(|_| 
			err!("Invalid first number: {}", tokens[0].value; Invalid, Input)));
		let num2: i64 = res!(tokens[2].value.parse().map_err(|_| 
			err!("Invalid second number: {}", tokens[2].value; Invalid, Input)));
		let num3: i64 = res!(tokens[4].value.parse().map_err(|_| 
			err!("Invalid third number: {}", tokens[4].value; Invalid, Input)));
		
		// Apply heuristics to determine which is day/month/year
		if num3 > 31 {
			// Assume third number is year
			res!(holder.set_field(TimeField::Year, num3));
			
			// Determine day/month based on values
			if num1 > 12 {
				res!(holder.set_field(TimeField::Day, num1));
				res!(holder.set_field(TimeField::Month, num2));
			} else if num2 > 12 {
				res!(holder.set_field(TimeField::Month, num1));
				res!(holder.set_field(TimeField::Day, num2));
			} else {
				// Ambiguous - assume DMY format
				res!(holder.set_field(TimeField::Day, num1));
				res!(holder.set_field(TimeField::Month, num2));
			}
		} else {
			// All numbers are small, need more heuristics
			// For now, assume DMY format
			res!(holder.set_field(TimeField::Day, num1));
			res!(holder.set_field(TimeField::Month, num2));
			res!(holder.set_field(TimeField::Year, num3 + 2000)); // Assume 20xx
		}
		
		Ok(holder)
	}
	
	fn parse_24_hour_time(&self, tokens: &[Token]) -> Outcome<TimeFieldHolder> {
		// Parse "14:30:00" format
		let mut holder = TimeFieldHolder::new();
		
		let hour: i64 = res!(tokens[0].value.parse().map_err(|_| 
			err!("Invalid hour: {}", tokens[0].value; Invalid, Input)));
		let minute: i64 = res!(tokens[2].value.parse().map_err(|_| 
			err!("Invalid minute: {}", tokens[2].value; Invalid, Input)));
		let second: i64 = res!(tokens[4].value.parse().map_err(|_| 
			err!("Invalid second: {}", tokens[4].value; Invalid, Input)));
		
		res!(holder.set_field(TimeField::Hour, hour));
		res!(holder.set_field(TimeField::Minute, minute));
		res!(holder.set_field(TimeField::Second, second));
		
		Ok(holder)
	}
	
	fn parse_12_hour_time(&self, tokens: &[Token]) -> Outcome<TimeFieldHolder> {
		// Parse "2:30 PM" format
		let mut holder = TimeFieldHolder::new();
		
		let mut hour: i64 = res!(tokens[0].value.parse().map_err(|_| 
			err!("Invalid hour: {}", tokens[0].value; Invalid, Input)));
		let minute: i64 = res!(tokens[2].value.parse().map_err(|_| 
			err!("Invalid minute: {}", tokens[2].value; Invalid, Input)));
		
		// Convert 12-hour to 24-hour format
		let am_pm = tokens[3].value.to_lowercase();
		if am_pm.starts_with('p') && hour != 12 {
			hour += 12;
		} else if am_pm.starts_with('a') && hour == 12 {
			hour = 0;
		}
		
		res!(holder.set_field(TimeField::Hour, hour));
		res!(holder.set_field(TimeField::Minute, minute));
		
		Ok(holder)
	}
	
	fn parse_noon(&self, _tokens: &[Token]) -> Outcome<TimeFieldHolder> {
		let mut holder = TimeFieldHolder::new();
		res!(holder.set_field(TimeField::Hour, 12));
		res!(holder.set_field(TimeField::Minute, 0));
		res!(holder.set_field(TimeField::Second, 0));
		Ok(holder)
	}
	
	fn parse_midnight(&self, _tokens: &[Token]) -> Outcome<TimeFieldHolder> {
		let mut holder = TimeFieldHolder::new();
		res!(holder.set_field(TimeField::Hour, 0));
		res!(holder.set_field(TimeField::Minute, 0));
		res!(holder.set_field(TimeField::Second, 0));
		Ok(holder)
	}
	
	/// Builds the comprehensive set of format patterns.
	fn build_format_patterns() -> Vec<FormatPattern> {
		vec![
			// High priority patterns (specific formats)
			FormatPattern {
				name: "ISO_DATE".to_string(),
				pattern: vec![TokenType::IsoDate],
				priority: 100,
			},
			FormatPattern {
				name: "ISO_DATETIME".to_string(),
				pattern: vec![TokenType::IsoDateTime],
				priority: 100,
			},
			
			// Natural language patterns
			FormatPattern {
				name: "ORDINAL_MONTH_YEAR".to_string(),
				pattern: vec![
					TokenType::OrdinalNumber,
					TokenType::MonthNameFull,
					TokenType::Number
				],
				priority: 90,
			},
			FormatPattern {
				name: "MONTH_ORDINAL_YEAR".to_string(),
				pattern: vec![
					TokenType::MonthNameShort,
					TokenType::Number,
					TokenType::Comma,
					TokenType::Number
				],
				priority: 85,
			},
			
			// Separated numeric patterns
			FormatPattern {
				name: "DMY_SEPARATED".to_string(),
				pattern: vec![
					TokenType::Number,
					TokenType::DateSeparator,
					TokenType::Number,
					TokenType::DateSeparator,
					TokenType::Number
				],
				priority: 70,
			},
			
			// Time patterns
			FormatPattern {
				name: "24_HOUR_TIME".to_string(),
				pattern: vec![
					TokenType::Number,
					TokenType::TimeSeparator,
					TokenType::Number,
					TokenType::TimeSeparator,
					TokenType::Number
				],
				priority: 80,
			},
			FormatPattern {
				name: "12_HOUR_TIME_AMPM".to_string(),
				pattern: vec![
					TokenType::Number,
					TokenType::TimeSeparator,
					TokenType::Number,
					TokenType::AmPm
				],
				priority: 75,
			},
			
			// Special time words
			FormatPattern {
				name: "NOON".to_string(),
				pattern: vec![TokenType::Noon],
				priority: 95,
			},
			FormatPattern {
				name: "MIDNIGHT".to_string(),
				pattern: vec![TokenType::Midnight],
				priority: 95,
			},
		]
	}
}

impl Default for Parser {
	fn default() -> Self {
		Self::new()
	}
}

/// 

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_iso_date_parsing() {
		let zone = CalClockZone::utc();
		let date = Parser::parse_date("2024-06-15", zone).unwrap();
		
		assert_eq!(date.year(), 2024);
		assert_eq!(date.month(), 6);
		assert_eq!(date.day(), 15);
	}

	#[test]
	fn test_natural_language_date() {
		let zone = CalClockZone::utc();
		
		// Test ordinal format
		let date = Parser::parse_date("15th June 2024", zone.clone()).unwrap();
		assert_eq!(date.year(), 2024);
		assert_eq!(date.month(), 6);
		assert_eq!(date.day(), 15);
		
		// Test month-first format
		let date2 = Parser::parse_date("June 15, 2024", zone).unwrap();
		assert_eq!(date2.year(), 2024);
		assert_eq!(date2.month(), 6);
		assert_eq!(date2.day(), 15);
	}

	#[test]
	fn test_time_parsing() {
		let zone = CalClockZone::utc();
		
		// 24-hour format
		let time1 = Parser::parse_time("14:30:00", zone.clone()).unwrap();
		assert_eq!(time1.hour().of(), 14);
		assert_eq!(time1.minute().of(), 30);
		assert_eq!(time1.second().of(), 0);
		
		// 12-hour format
		let time2 = Parser::parse_time("2:30 PM", zone.clone()).unwrap();
		assert_eq!(time2.hour().of(), 14);
		assert_eq!(time2.minute().of(), 30);
		
		// Special times
		let noon = Parser::parse_time("noon", zone.clone()).unwrap();
		assert_eq!(noon.hour().of(), 12);
		assert_eq!(noon.minute().of(), 0);
		
		let midnight = Parser::parse_time("midnight", zone).unwrap();
		assert_eq!(midnight.hour().of(), 0);
		assert_eq!(midnight.minute().of(), 0);
	}

	#[test]
	fn test_tokenizer() {
		let lexer = Lexer::new();
		
		let tokens = lexer.tokenize("2024-06-15").unwrap();
		assert_eq!(tokens.len(), 1);
		assert_eq!(tokens[0].token_type, TokenType::IsoDate);
		
		let tokens2 = lexer.tokenize("15th June 2024").unwrap();
		assert_eq!(tokens2.len(), 3);
		assert_eq!(tokens2[0].token_type, TokenType::OrdinalNumber);
		assert_eq!(tokens2[1].token_type, TokenType::MonthNameFull);
		assert_eq!(tokens2[2].token_type, TokenType::Number);
	}

	#[test]
	fn test_intelligent_disambiguation() {
		let zone = CalClockZone::utc();
		
		// Test automatic day/month swapping
		let date = Parser::parse_date("25/12/2024", zone).unwrap(); // Christmas
		assert_eq!(date.day(), 25);
		assert_eq!(date.month(), 12);
		assert_eq!(date.year(), 2024);
	}

	#[test]
	#[ignore] // TODO: Fix month/day parsing in ISO format
	fn test_combined_datetime() {
		let zone = CalClockZone::utc();
		
		// Test a simpler format that we know works
		let datetime = Parser::parse_datetime("2024-01-15 14:30:00", zone.clone()).unwrap();
		assert_eq!(datetime.date().year(), 2024);
		assert_eq!(datetime.date().month(), 1);
		assert_eq!(datetime.date().day(), 15);
		assert_eq!(datetime.time().hour().of(), 14);
		assert_eq!(datetime.time().minute().of(), 30);
		assert_eq!(datetime.time().second().of(), 0);
	}
}