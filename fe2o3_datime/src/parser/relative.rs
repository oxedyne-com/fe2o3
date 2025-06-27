/// Comprehensive relative date parsing for natural language expressions.
/// 
/// This module handles complex relative date expressions such as:
/// - "next Tuesday", "last Friday", "this Monday"
/// - "in 2 weeks", "in 3 days", "2 months ago"
/// - "next month", "last year", "this week"
/// - "3 days from now", "2 weeks from today"
/// - "end of this month", "beginning of next year"
/// - "this coming Monday", "the Tuesday after next"

use crate::{
    calendar::CalendarDate,
    constant::{DayOfWeek, MonthOfYear},
    time::CalClockZone,
};

use oxedyne_fe2o3_core::prelude::*;

use std::collections::HashMap;

/// Types of relative date references.
#[derive(Clone, Debug, PartialEq)]
pub enum RelativeReference {
    /// Next occurrence of something (next Tuesday, next month, next year).
    Next,
    /// Previous occurrence of something (last Tuesday, last month, last year).
    Last,
    /// Current occurrence (this Tuesday, this month, this year).
    This,
    /// Coming/upcoming occurrence (this coming Tuesday, upcoming Monday).
    Coming,
    /// After next (the Tuesday after next, month after next).
    AfterNext,
    /// Before last (the Tuesday before last).
    BeforeLast,
}

/// Units for relative date calculations.
#[derive(Clone, Debug, PartialEq)]
pub enum RelativeUnit {
    /// Day units (day, days).
    Day,
    /// Week units (week, weeks).
    Week,
    /// Month units (month, months).
    Month,
    /// Year units (year, years).
    Year,
    /// Specific day of week (Monday, Tuesday, etc.).
    DayOfWeek(DayOfWeek),
    /// Beginning or end of period.
    PeriodBoundary(PeriodType, BoundaryType),
}

/// Types of periods for boundary calculations.
#[derive(Clone, Debug, PartialEq)]
pub enum PeriodType {
    Week,
    Month,
    Quarter,
    Year,
}

/// Types of boundaries within periods.
#[derive(Clone, Debug, PartialEq)]
pub enum BoundaryType {
    Beginning,
    End,
    Middle,
}

/// Direction of relative movement (forward or backward in time).
#[derive(Clone, Debug, PartialEq)]
pub enum Direction {
    Forward,
    Backward,
}

/// A complete relative date expression parsed from natural language.
#[derive(Clone, Debug, PartialEq)]
pub struct RelativeExpression {
    /// The relative reference type (next, last, this, etc.).
    pub reference: RelativeReference,
    /// The unit being referenced (day, week, Monday, etc.).
    pub unit: RelativeUnit,
    /// Optional quantity (2 days, 3 weeks, etc.).
    pub quantity: Option<i32>,
    /// Direction of movement.
    pub direction: Direction,
    /// Optional additional context ("from now", "from today", etc.).
    pub context: Option<String>,
}

/// Comprehensive relative date parser.
pub struct RelativeDateParser {
    /// Day of week name mappings.
    day_names: HashMap<String, DayOfWeek>,
    /// Month name mappings.
    month_names: HashMap<String, MonthOfYear>,
    /// Relative reference word mappings.
    reference_words: HashMap<String, RelativeReference>,
    /// Unit word mappings.
    unit_words: HashMap<String, RelativeUnit>,
    /// Direction word mappings.
    direction_words: HashMap<String, Direction>,
}

impl Default for RelativeDateParser {
    fn default() -> Self {
        Self::new()
    }
}

impl RelativeDateParser {
    /// Creates a new relative date parser with comprehensive vocabularies.
    pub fn new() -> Self {
        let mut parser = Self {
            day_names: HashMap::new(),
            month_names: HashMap::new(),
            reference_words: HashMap::new(),
            unit_words: HashMap::new(),
            direction_words: HashMap::new(),
        };
        
        parser.initialise_vocabularies();
        parser
    }
    
    /// Initialises all vocabulary mappings.
    fn initialise_vocabularies(&mut self) {
        self.initialise_day_names();
        self.initialise_month_names();
        self.initialise_reference_words();
        self.initialise_unit_words();
        self.initialise_direction_words();
    }
    
    /// Initialises day of week name mappings.
    fn initialise_day_names(&mut self) {
        let days = [
            (DayOfWeek::Sunday, vec!["sunday", "sun"]),
            (DayOfWeek::Monday, vec!["monday", "mon"]),
            (DayOfWeek::Tuesday, vec!["tuesday", "tue", "tues"]),
            (DayOfWeek::Wednesday, vec!["wednesday", "wed"]),
            (DayOfWeek::Thursday, vec!["thursday", "thu", "thurs"]),
            (DayOfWeek::Friday, vec!["friday", "fri"]),
            (DayOfWeek::Saturday, vec!["saturday", "sat"]),
        ];
        
        for (day, names) in days.iter() {
            for name in names {
                self.day_names.insert(name.to_string(), *day);
            }
        }
    }
    
    /// Initialises month name mappings.
    fn initialise_month_names(&mut self) {
        let months = [
            (MonthOfYear::January, vec!["january", "jan"]),
            (MonthOfYear::February, vec!["february", "feb"]),
            (MonthOfYear::March, vec!["march", "mar"]),
            (MonthOfYear::April, vec!["april", "apr"]),
            (MonthOfYear::May, vec!["may"]),
            (MonthOfYear::June, vec!["june", "jun"]),
            (MonthOfYear::July, vec!["july", "jul"]),
            (MonthOfYear::August, vec!["august", "aug"]),
            (MonthOfYear::September, vec!["september", "sep", "sept"]),
            (MonthOfYear::October, vec!["october", "oct"]),
            (MonthOfYear::November, vec!["november", "nov"]),
            (MonthOfYear::December, vec!["december", "dec"]),
        ];
        
        for (month, names) in months.iter() {
            for name in names {
                self.month_names.insert(name.to_string(), *month);
            }
        }
    }
    
    /// Initialises relative reference word mappings.
    fn initialise_reference_words(&mut self) {
        let references = [
            (RelativeReference::Next, vec!["next", "following", "upcoming"]),
            (RelativeReference::Last, vec!["last", "previous", "past", "prior"]),
            (RelativeReference::This, vec!["this", "current"]),
            (RelativeReference::Coming, vec!["coming", "approaching"]),
            (RelativeReference::AfterNext, vec!["after"]),
            (RelativeReference::BeforeLast, vec!["before"]),
        ];
        
        for (reference, words) in references.iter() {
            for word in words {
                self.reference_words.insert(word.to_string(), reference.clone());
            }
        }
    }
    
    /// Initialises unit word mappings.
    fn initialise_unit_words(&mut self) {
        let units = [
            (RelativeUnit::Day, vec!["day", "days"]),
            (RelativeUnit::Week, vec!["week", "weeks"]),
            (RelativeUnit::Month, vec!["month", "months"]),
            (RelativeUnit::Year, vec!["year", "years"]),
        ];
        
        for (unit, words) in units.iter() {
            for word in words {
                self.unit_words.insert(word.to_string(), unit.clone());
            }
        }
        
        // Note: Period boundaries are handled by multi-token pattern matching,
        // not individual token parsing, so we don't add them to unit_words.
    }
    
    /// Initialises direction word mappings.
    fn initialise_direction_words(&mut self) {
        let directions = [
            (Direction::Forward, vec!["from", "after", "hence", "later"]),
            (Direction::Backward, vec!["ago", "back", "before", "earlier"]),
        ];
        
        for (direction, words) in directions.iter() {
            for word in words {
                self.direction_words.insert(word.to_string(), direction.clone());
            }
        }
    }
    
    /// Parses a relative date expression from natural language.
    /// 
    /// # Examples
    /// 
    /// ```ignore
    /// let parser = RelativeDateParser::new();
    /// 
    /// // Simple relative dates
    /// let expr = parser.parse("next Tuesday").unwrap();
    /// let expr = parser.parse("last Friday").unwrap();
    /// let expr = parser.parse("this Monday").unwrap();
    /// 
    /// // Quantified relative dates
    /// let expr = parser.parse("in 2 weeks").unwrap();
    /// let expr = parser.parse("3 days ago").unwrap();
    /// let expr = parser.parse("2 months from now").unwrap();
    /// 
    /// // Complex expressions
    /// let expr = parser.parse("the Tuesday after next").unwrap();
    /// let expr = parser.parse("end of this month").unwrap();
    /// let expr = parser.parse("beginning of next year").unwrap();
    /// ```
    pub fn parse(&self, input: &str) -> Outcome<RelativeExpression> {
        let normalized = self.normalize_input(input);
        let tokens = self.tokenize(&normalized);
        self.parse_tokens(&tokens)
    }
    
    /// Normalizes input by converting to lowercase and handling common variations.
    fn normalize_input(&self, input: &str) -> String {
        input
            .to_lowercase()
            .replace("  ", " ")
            .replace("from now", "")
            .replace("from today", "")
            .trim()
            .to_string()
    }
    
    /// Tokenizes the normalized input into words.
    fn tokenize(&self, input: &str) -> Vec<String> {
        input
            .split_whitespace()
            .filter(|word| !word.is_empty())
            .filter(|word| !matches!(*word, "the" | "a" | "an" | "in" | "on" | "at"))
            .map(|word| word.to_string())
            .collect()
    }
    
    /// Parses tokens into a relative expression.
    fn parse_tokens(&self, tokens: &[String]) -> Outcome<RelativeExpression> {
        if tokens.is_empty() {
            return Err(err!("Empty input for relative date parsing"; Invalid, Input));
        }
        
        let mut expression = RelativeExpression {
            reference: RelativeReference::This,
            unit: RelativeUnit::Day,
            quantity: None,
            direction: Direction::Forward,
            context: None,
        };
        
        let mut i = 0;
        while i < tokens.len() {
            let token = &tokens[i];
            
            // Try to parse quantity (numbers).
            if let Ok(num) = token.parse::<i32>() {
                expression.quantity = Some(num);
                i += 1;
                continue;
            }
            
            // Handle period boundaries FIRST before individual token parsing
            // This prevents "end" and "month" from being parsed separately
            if i + 2 < tokens.len() && tokens[i + 1] == "of" {
                // Try direct pattern: "end of month"
                if let Some(boundary_unit) = self.parse_period_boundary(token, &tokens[i + 2]) {
                    expression.unit = boundary_unit;
                    i += 3;
                    continue;
                }
                
                // Try with reference word: "end of this month", "beginning of next year"
                if i + 3 < tokens.len() {
                    if let Some(reference) = self.reference_words.get(&tokens[i + 2]) {
                        if let Some(boundary_unit) = self.parse_period_boundary(token, &tokens[i + 3]) {
                            expression.reference = reference.clone();
                            expression.unit = boundary_unit;
                            i += 4;
                            continue;
                        }
                    }
                }
            }
            
            // Try to parse relative reference.
            if let Some(reference) = self.reference_words.get(token) {
                expression.reference = reference.clone();
                i += 1;
                continue;
            }
            
            // Try to parse day of week.
            if let Some(day) = self.day_names.get(token) {
                expression.unit = RelativeUnit::DayOfWeek(*day);
                i += 1;
                continue;
            }
            
            // Try to parse unit.
            if let Some(unit) = self.unit_words.get(token) {
                expression.unit = unit.clone();
                i += 1;
                continue;
            }
            
            // Try to parse direction.
            if let Some(direction) = self.direction_words.get(token) {
                expression.direction = direction.clone();
                i += 1;
                continue;
            }
            
            // Handle special multi-word patterns.
            if token == "after" && i + 1 < tokens.len() && tokens[i + 1] == "next" {
                expression.reference = RelativeReference::AfterNext;
                i += 2;
                continue;
            }
            
            if token == "before" && i + 1 < tokens.len() && tokens[i + 1] == "last" {
                expression.reference = RelativeReference::BeforeLast;
                i += 2;
                continue;
            }
            
            
            // Skip unknown tokens.
            i += 1;
        }
        
        // Apply some intelligent defaults and validation.
        self.validate_and_adjust_expression(&mut expression)?;
        
        Ok(expression)
    }
    
    /// Parses period boundary expressions like "end of month", "beginning of year".
    pub fn parse_period_boundary(&self, boundary_word: &str, period_word: &str) -> Option<RelativeUnit> {
        let boundary_type = match boundary_word {
            "beginning" | "start" => BoundaryType::Beginning,
            "end" => BoundaryType::End,
            "middle" => BoundaryType::Middle,
            _ => return None,
        };
        
        let period_type = match period_word {
            "week" => PeriodType::Week,
            "month" => PeriodType::Month,
            "quarter" => PeriodType::Quarter,
            "year" => PeriodType::Year,
            _ => return None,
        };
        
        Some(RelativeUnit::PeriodBoundary(period_type, boundary_type))
    }
    
    /// Validates and adjusts the parsed expression for consistency.
    fn validate_and_adjust_expression(&self, expr: &mut RelativeExpression) -> Outcome<()> {
        // If we have a quantity but no clear direction, infer from context.
        if expr.quantity.is_some() {
            // If we have "ago" or similar, it's backward.
            if expr.direction == Direction::Backward {
                // Already correct.
            } else if matches!(expr.reference, RelativeReference::Last) {
                expr.direction = Direction::Backward;
            } else {
                // Default to forward for quantified expressions.
                expr.direction = Direction::Forward;
            }
        }
        
        // Adjust direction based on reference type for all unit types.
        match expr.reference {
            RelativeReference::Next | RelativeReference::Coming => {
                expr.direction = Direction::Forward;
            },
            RelativeReference::Last => {
                expr.direction = Direction::Backward;
            },
            RelativeReference::This => {
                // For "this X" expressions, determine direction based on context.
                if expr.direction == Direction::Backward {
                    // Keep backward if explicitly specified.
                } else {
                    // Default to forward for "this" expressions.
                    expr.direction = Direction::Forward;
                }
            },
            RelativeReference::AfterNext => {
                expr.direction = Direction::Forward;
            },
            RelativeReference::BeforeLast => {
                expr.direction = Direction::Backward;
            },
        }
        
        Ok(())
    }
    
    /// Calculates the actual date from a relative expression.
    /// 
    /// # Arguments
    /// 
    /// * `expr` - The relative expression to calculate
    /// * `base_date` - The base date to calculate from (usually today)
    /// * `zone` - The timezone for the calculation
    /// 
    /// # Returns
    /// 
    /// The calculated CalendarDate
    pub fn calculate_date(&self, expr: &RelativeExpression, base_date: &CalendarDate, zone: CalClockZone) -> Outcome<CalendarDate> {
        match &expr.unit {
            RelativeUnit::Day => self.calculate_day_offset(expr, base_date),
            RelativeUnit::Week => self.calculate_week_offset(expr, base_date),
            RelativeUnit::Month => self.calculate_month_offset(expr, base_date),
            RelativeUnit::Year => self.calculate_year_offset(expr, base_date),
            RelativeUnit::DayOfWeek(target_day) => self.calculate_day_of_week(expr, base_date, *target_day, zone),
            RelativeUnit::PeriodBoundary(period, boundary) => self.calculate_period_boundary(expr, base_date, period, boundary, zone),
        }
    }
    
    /// Calculates date with day offset.
    fn calculate_day_offset(&self, expr: &RelativeExpression, base_date: &CalendarDate) -> Outcome<CalendarDate> {
        let quantity = expr.quantity.unwrap_or(1);
        let offset = match expr.direction {
            Direction::Forward => quantity,
            Direction::Backward => -quantity,
        };
        
        base_date.add_days(offset)
    }
    
    /// Calculates date with week offset.
    fn calculate_week_offset(&self, expr: &RelativeExpression, base_date: &CalendarDate) -> Outcome<CalendarDate> {
        let quantity = expr.quantity.unwrap_or(1);
        let offset = match expr.direction {
            Direction::Forward => quantity * 7,
            Direction::Backward => -quantity * 7,
        };
        
        base_date.add_days(offset)
    }
    
    /// Calculates date with month offset.
    fn calculate_month_offset(&self, expr: &RelativeExpression, base_date: &CalendarDate) -> Outcome<CalendarDate> {
        let quantity = expr.quantity.unwrap_or(1);
        let offset = match expr.direction {
            Direction::Forward => quantity,
            Direction::Backward => -quantity,
        };
        
        base_date.add_months(offset)
    }
    
    /// Calculates date with year offset.
    fn calculate_year_offset(&self, expr: &RelativeExpression, base_date: &CalendarDate) -> Outcome<CalendarDate> {
        let quantity = expr.quantity.unwrap_or(1);
        let offset = match expr.direction {
            Direction::Forward => quantity,
            Direction::Backward => -quantity,
        };
        
        base_date.add_years(offset)
    }
    
    /// Calculates specific day of week relative to base date.
    fn calculate_day_of_week(&self, expr: &RelativeExpression, base_date: &CalendarDate, target_day: DayOfWeek, _zone: CalClockZone) -> Outcome<CalendarDate> {
        let current_day = base_date.day_of_week();
        let current_day_num = current_day.of() as i32;
        let target_day_num = target_day.of() as i32;
        
        let mut days_to_target = target_day_num - current_day_num;
        
        match expr.reference {
            RelativeReference::This => {
                // This Tuesday: if today is Tuesday, return today; otherwise find the Tuesday in this week
                if days_to_target == 0 {
                    return Ok(base_date.clone());
                }
                // For "this", we want the occurrence within this week, whether past or future
                // No adjustment needed - days_to_target will be negative for past days in the week
            },
            RelativeReference::Next | RelativeReference::Coming => {
                // Next Tuesday: find the next occurrence (not today even if today is Tuesday)
                if days_to_target <= 0 {
                    days_to_target += 7;
                }
            },
            RelativeReference::Last => {
                // Last Tuesday: find the previous occurrence (not today even if today is Tuesday)
                if days_to_target >= 0 {
                    days_to_target -= 7;
                }
            },
            RelativeReference::AfterNext => {
                // The Tuesday after next: find the occurrence after next Tuesday
                if days_to_target <= 0 {
                    days_to_target += 7;
                }
                days_to_target += 7; // Add another week
            },
            RelativeReference::BeforeLast => {
                // The Tuesday before last: find the occurrence before last Tuesday
                if days_to_target >= 0 {
                    days_to_target -= 7;
                }
                days_to_target -= 7; // Subtract another week
            },
        }
        
        base_date.add_days(days_to_target)
    }
    
    /// Calculates period boundary dates (beginning/end of month, etc.).
    fn calculate_period_boundary(&self, expr: &RelativeExpression, base_date: &CalendarDate, period: &PeriodType, boundary: &BoundaryType, zone: CalClockZone) -> Outcome<CalendarDate> {
        match period {
            PeriodType::Week => self.calculate_week_boundary(expr, base_date, boundary),
            PeriodType::Month => self.calculate_month_boundary(expr, base_date, boundary, zone),
            PeriodType::Quarter => self.calculate_quarter_boundary(expr, base_date, boundary, zone),
            PeriodType::Year => self.calculate_year_boundary(expr, base_date, boundary, zone),
        }
    }
    
    /// Calculates week boundary (beginning/end of week).
    fn calculate_week_boundary(&self, expr: &RelativeExpression, base_date: &CalendarDate, boundary: &BoundaryType) -> Outcome<CalendarDate> {
        let current_day_num = base_date.day_of_week().of() as i32;
        
        let target_date = match expr.reference {
            RelativeReference::This => base_date.clone(),
            RelativeReference::Next => res!(base_date.add_days(7)),
            RelativeReference::Last => res!(base_date.add_days(-7)),
            _ => base_date.clone(),
        };
        
        match boundary {
            BoundaryType::Beginning => {
                // Beginning of week (Sunday)
                let days_to_sunday = if current_day_num == 0 { 0 } else { -current_day_num };
                target_date.add_days(days_to_sunday)
            },
            BoundaryType::End => {
                // End of week (Saturday)
                let days_to_saturday = 6 - current_day_num;
                target_date.add_days(days_to_saturday)
            },
            BoundaryType::Middle => {
                // Middle of week (Wednesday)
                let days_to_wednesday = 3 - current_day_num;
                target_date.add_days(days_to_wednesday)
            },
        }
    }
    
    /// Calculates month boundary (beginning/end of month).
    fn calculate_month_boundary(&self, expr: &RelativeExpression, base_date: &CalendarDate, boundary: &BoundaryType, zone: CalClockZone) -> Outcome<CalendarDate> {
        let (target_year, target_month) = match expr.reference {
            RelativeReference::This => (base_date.year(), base_date.month()),
            RelativeReference::Next => {
                if base_date.month() == 12 {
                    (base_date.year() + 1, 1)
                } else {
                    (base_date.year(), base_date.month() + 1)
                }
            },
            RelativeReference::Last => {
                if base_date.month() == 1 {
                    (base_date.year() - 1, 12)
                } else {
                    (base_date.year(), base_date.month() - 1)
                }
            },
            _ => (base_date.year(), base_date.month()),
        };
        
        let target_month_enum = res!(MonthOfYear::from_number(target_month));
        
        match boundary {
            BoundaryType::Beginning => {
                CalendarDate::from_ymd(target_year, target_month_enum, 1, zone)
            },
            BoundaryType::End => {
                let days_in_month = target_month_enum.days_in_month(target_year);
                CalendarDate::from_ymd(target_year, target_month_enum, days_in_month, zone)
            },
            BoundaryType::Middle => {
                let days_in_month = target_month_enum.days_in_month(target_year);
                let middle_day = (days_in_month + 1) / 2;
                CalendarDate::from_ymd(target_year, target_month_enum, middle_day, zone)
            },
        }
    }
    
    /// Calculates quarter boundary.
    fn calculate_quarter_boundary(&self, expr: &RelativeExpression, base_date: &CalendarDate, boundary: &BoundaryType, zone: CalClockZone) -> Outcome<CalendarDate> {
        let current_quarter = ((base_date.month() - 1) / 3) + 1;
        
        let target_quarter = match expr.reference {
            RelativeReference::This => current_quarter,
            RelativeReference::Next => if current_quarter == 4 { 1 } else { current_quarter + 1 },
            RelativeReference::Last => if current_quarter == 1 { 4 } else { current_quarter - 1 },
            _ => current_quarter,
        };
        
        let (target_year, first_month) = if expr.reference == RelativeReference::Next && current_quarter == 4 {
            (base_date.year() + 1, 1)
        } else if expr.reference == RelativeReference::Last && current_quarter == 1 {
            (base_date.year() - 1, 10)
        } else {
            (base_date.year(), (target_quarter - 1) * 3 + 1)
        };
        
        match boundary {
            BoundaryType::Beginning => {
                let month_enum = res!(MonthOfYear::from_number(first_month));
                CalendarDate::from_ymd(target_year, month_enum, 1, zone)
            },
            BoundaryType::End => {
                let last_month = first_month + 2;
                let month_enum = res!(MonthOfYear::from_number(last_month));
                let days_in_month = month_enum.days_in_month(target_year);
                CalendarDate::from_ymd(target_year, month_enum, days_in_month, zone)
            },
            BoundaryType::Middle => {
                // Middle of quarter (middle of second month)
                let middle_month = first_month + 1;
                let month_enum = res!(MonthOfYear::from_number(middle_month));
                let days_in_month = month_enum.days_in_month(target_year);
                let middle_day = (days_in_month + 1) / 2;
                CalendarDate::from_ymd(target_year, month_enum, middle_day, zone)
            },
        }
    }
    
    /// Calculates year boundary.
    fn calculate_year_boundary(&self, expr: &RelativeExpression, base_date: &CalendarDate, boundary: &BoundaryType, zone: CalClockZone) -> Outcome<CalendarDate> {
        let target_year = match expr.reference {
            RelativeReference::This => base_date.year(),
            RelativeReference::Next => base_date.year() + 1,
            RelativeReference::Last => base_date.year() - 1,
            _ => base_date.year(),
        };
        
        match boundary {
            BoundaryType::Beginning => {
                CalendarDate::from_ymd(target_year, MonthOfYear::January, 1, zone)
            },
            BoundaryType::End => {
                CalendarDate::from_ymd(target_year, MonthOfYear::December, 31, zone)
            },
            BoundaryType::Middle => {
                // Middle of year (July 1st or 2nd depending on leap year)
                let middle_day = if target_year % 4 == 0 && (target_year % 100 != 0 || target_year % 400 == 0) { 2 } else { 1 };
                CalendarDate::from_ymd(target_year, MonthOfYear::July, middle_day, zone)
            },
        }
    }
    
    /// Convenience method to parse and calculate a relative date in one step.
    /// 
    /// # Arguments
    /// 
    /// * `input` - The natural language relative date expression
    /// * `base_date` - The base date to calculate from (usually today)
    /// * `zone` - The timezone for the calculation
    /// 
    /// # Returns
    /// 
    /// The calculated CalendarDate
    /// 
    /// # Examples
    /// 
    /// ```ignore
    /// let parser = RelativeDateParser::new();
    /// let today = CalendarDate::today(CalClockZone::utc()).unwrap();
    /// 
    /// let next_tuesday = parser.parse_and_calculate("next Tuesday", &today, CalClockZone::utc()).unwrap();
    /// let in_two_weeks = parser.parse_and_calculate("in 2 weeks", &today, CalClockZone::utc()).unwrap();
    /// let end_of_month = parser.parse_and_calculate("end of this month", &today, CalClockZone::utc()).unwrap();
    /// ```
    pub fn parse_and_calculate(&self, input: &str, base_date: &CalendarDate, zone: CalClockZone) -> Outcome<CalendarDate> {
        let expression = res!(self.parse(input));
        self.calculate_date(&expression, base_date, zone)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::time::CalClockZone;
    
    fn get_test_base_date() -> CalendarDate {
        // Wednesday, June 12, 2024
        CalendarDate::from_ymd(2024, MonthOfYear::June, 12, CalClockZone::utc()).unwrap()
    }
    
    #[test]
    fn test_parse_simple_relative_dates() {
        let parser = RelativeDateParser::new();
        
        // Test "next Tuesday"
        let expr = parser.parse("next Tuesday").unwrap();
        assert_eq!(expr.reference, RelativeReference::Next);
        assert_eq!(expr.unit, RelativeUnit::DayOfWeek(DayOfWeek::Tuesday));
        assert_eq!(expr.direction, Direction::Forward);
        
        // Test "last Friday"
        let expr = parser.parse("last Friday").unwrap();
        assert_eq!(expr.reference, RelativeReference::Last);
        assert_eq!(expr.unit, RelativeUnit::DayOfWeek(DayOfWeek::Friday));
        assert_eq!(expr.direction, Direction::Backward);
        
        // Test "this Monday"
        let expr = parser.parse("this Monday").unwrap();
        assert_eq!(expr.reference, RelativeReference::This);
        assert_eq!(expr.unit, RelativeUnit::DayOfWeek(DayOfWeek::Monday));
    }
    
    #[test]
    fn test_parse_quantified_relative_dates() {
        let parser = RelativeDateParser::new();
        
        // Test "in 2 weeks"
        let expr = parser.parse("in 2 weeks").unwrap();
        assert_eq!(expr.quantity, Some(2));
        assert_eq!(expr.unit, RelativeUnit::Week);
        assert_eq!(expr.direction, Direction::Forward);
        
        // Test "3 days ago"
        let expr = parser.parse("3 days ago").unwrap();
        assert_eq!(expr.quantity, Some(3));
        assert_eq!(expr.unit, RelativeUnit::Day);
        assert_eq!(expr.direction, Direction::Backward);
        
        // Test "2 months from now"
        let expr = parser.parse("2 months from now").unwrap();
        assert_eq!(expr.quantity, Some(2));
        assert_eq!(expr.unit, RelativeUnit::Month);
        assert_eq!(expr.direction, Direction::Forward);
    }
    
    #[test]
    fn test_parse_period_boundaries() {
        let parser = RelativeDateParser::new();
        
        // Test "end of this month"
        let expr = parser.parse("end of this month").unwrap();
        assert_eq!(expr.reference, RelativeReference::This);
        assert!(matches!(expr.unit, RelativeUnit::PeriodBoundary(PeriodType::Month, BoundaryType::End)));
        
        // Test "beginning of next year"
        let expr = parser.parse("beginning of next year").unwrap();
        assert_eq!(expr.reference, RelativeReference::Next);
        assert!(matches!(expr.unit, RelativeUnit::PeriodBoundary(PeriodType::Year, BoundaryType::Beginning)));
    }
    
    #[test]
    fn test_calculate_day_of_week() {
        let parser = RelativeDateParser::new();
        let base_date = get_test_base_date(); // Wednesday, June 12, 2024
        let zone = CalClockZone::utc();
        
        // Next Tuesday (June 18, 2024)
        let next_tuesday = parser.parse_and_calculate("next Tuesday", &base_date, zone.clone()).unwrap();
        assert_eq!(next_tuesday.day(), 18);
        assert_eq!(next_tuesday.day_of_week(), DayOfWeek::Tuesday);
        
        // Last Friday (June 7, 2024)
        let last_friday = parser.parse_and_calculate("last Friday", &base_date, zone.clone()).unwrap();
        assert_eq!(last_friday.day(), 7);
        assert_eq!(last_friday.day_of_week(), DayOfWeek::Friday);
        
        // This Monday (June 10, 2024)
        let this_monday = parser.parse_and_calculate("this Monday", &base_date, zone.clone()).unwrap();
        assert_eq!(this_monday.day(), 10);
        assert_eq!(this_monday.day_of_week(), DayOfWeek::Monday);
    }
    
    #[test]
    fn test_calculate_quantified_dates() {
        let parser = RelativeDateParser::new();
        let base_date = get_test_base_date(); // Wednesday, June 12, 2024
        let zone = CalClockZone::utc();
        
        // In 2 weeks (June 26, 2024)
        let in_two_weeks = parser.parse_and_calculate("in 2 weeks", &base_date, zone.clone()).unwrap();
        assert_eq!(in_two_weeks.day(), 26);
        assert_eq!(in_two_weeks.month(), 6);
        
        // 3 days ago (June 9, 2024)
        let three_days_ago = parser.parse_and_calculate("3 days ago", &base_date, zone.clone()).unwrap();
        assert_eq!(three_days_ago.day(), 9);
        assert_eq!(three_days_ago.month(), 6);
        
        // 2 months from now (August 12, 2024)
        let two_months_later = parser.parse_and_calculate("2 months from now", &base_date, zone.clone()).unwrap();
        assert_eq!(two_months_later.day(), 12);
        assert_eq!(two_months_later.month(), 8);
    }
    
    #[test]
    fn test_calculate_period_boundaries() {
        let parser = RelativeDateParser::new();
        let base_date = get_test_base_date(); // Wednesday, June 12, 2024
        let zone = CalClockZone::utc();
        
        // End of this month (June 30, 2024)
        let end_of_month = parser.parse_and_calculate("end of this month", &base_date, zone.clone()).unwrap();
        assert_eq!(end_of_month.day(), 30);
        assert_eq!(end_of_month.month(), 6);
        
        // Beginning of next month (July 1, 2024)
        let beginning_next_month = parser.parse_and_calculate("beginning of next month", &base_date, zone.clone()).unwrap();
        assert_eq!(beginning_next_month.day(), 1);
        assert_eq!(beginning_next_month.month(), 7);
        
        // End of this year (December 31, 2024)
        let end_of_year = parser.parse_and_calculate("end of this year", &base_date, zone.clone()).unwrap();
        assert_eq!(end_of_year.day(), 31);
        assert_eq!(end_of_year.month(), 12);
        assert_eq!(end_of_year.year(), 2024);
    }
    
    #[test]
    fn test_complex_expressions() {
        let parser = RelativeDateParser::new();
        let base_date = get_test_base_date(); // Wednesday, June 12, 2024
        let zone = CalClockZone::utc();
        
        // The Tuesday after next
        let expr = parser.parse("after next Tuesday").unwrap();
        assert_eq!(expr.reference, RelativeReference::AfterNext);
        assert_eq!(expr.unit, RelativeUnit::DayOfWeek(DayOfWeek::Tuesday));
        
        let tuesday_after_next = parser.calculate_date(&expr, &base_date, zone.clone()).unwrap();
        assert_eq!(tuesday_after_next.day(), 25); // June 25, 2024
        assert_eq!(tuesday_after_next.day_of_week(), DayOfWeek::Tuesday);
    }
    
    #[test]
    fn test_edge_cases() {
        let parser = RelativeDateParser::new();
        let zone = CalClockZone::utc();
        
        // Test from different days of week
        let sunday = CalendarDate::from_ymd(2024, MonthOfYear::June, 16, zone.clone()).unwrap(); // Sunday
        
        // Next Monday from Sunday should be tomorrow
        let next_monday = parser.parse_and_calculate("next Monday", &sunday, zone.clone()).unwrap();
        assert_eq!(next_monday.day(), 17);
        assert_eq!(next_monday.day_of_week(), DayOfWeek::Monday);
        
        // This Monday from Sunday should be tomorrow (since "this week" includes future days)
        let this_monday = parser.parse_and_calculate("this Monday", &sunday, zone.clone()).unwrap();
        assert_eq!(this_monday.day(), 17);
        assert_eq!(this_monday.day_of_week(), DayOfWeek::Monday);
    }
}