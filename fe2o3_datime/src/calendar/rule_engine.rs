use oxedyne_fe2o3_core::prelude::*;
use crate::{
	calendar::{CalendarDate, CalendarDuration, CalendarMonth, CalendarYear, CalendarDay, DayIncrementor, MonthPeriod},
	constant::MonthOfYear,
	time::CalClockInterval,
};
use std::collections::BTreeSet;

/// Types of calendar rules for recurring date patterns.
#[derive(Clone, Debug, PartialEq)]
pub enum RuleType {
	/// Rule recurring by years (anniversaries).
	ByYears,
	/// Rule with explicitly specified months.
	ByExplicitMonths,
	/// Rule with regular monthly intervals.
	ByRegularMonths,
	/// Rule recurring by days.
	ByDays,
}

/// Internal generator tasks for rule processing.
#[derive(Clone, Debug, PartialEq)]
enum GeneratorTask {
	All,
	AllOnOrBefore,
	AllOnOrAfter,
	NOnOrBefore,
	NOnOrAfter,
}

/// Represents a rule for specifying recurring dates.
/// 
/// Supports three categories of rules:
/// - By years (anniversaries with skip patterns)
/// - By months (explicit months or regular intervals)
/// - By days (daily recurring patterns)
#[derive(Clone, Debug)]
pub struct CalendarRule {
	rule_type: RuleType,
	duration: Option<CalendarDuration>,
	count_limit: Option<i32>,
	start_date: Option<CalendarDate>,
	
	// By years fields
	skip_years: Option<CalendarYear>,
	
	// By months fields
	day_incrementor: Option<DayIncrementor>,
	month_set: Option<BTreeSet<MonthOfYear>>,
	start_month: Option<MonthPeriod>,
	skip_months: Option<CalendarMonth>,
	
	// By days fields
	skip_days: Option<CalendarDay>,
}

impl CalendarRule {
	/// Creates a new calendar rule with the specified type.
	pub fn new(rule_type: RuleType) -> Self {
		Self {
			rule_type,
			duration: None,
			count_limit: None,
			start_date: None,
			skip_years: None,
			day_incrementor: None,
			month_set: None,
			start_month: None,
			skip_months: None,
			skip_days: None,
		}
	}
	
	/// Creates a comprehensive calendar rule with all parameters.
	pub fn new_comprehensive(
		rule_type: RuleType,
		duration: Option<CalendarDuration>,
		count_limit: Option<i32>,
		start_date: Option<CalendarDate>,
		skip_years: Option<CalendarYear>,
		day_incrementor: Option<DayIncrementor>,
		start_month: Option<MonthPeriod>,
		month_set: Option<BTreeSet<MonthOfYear>>,
		skip_months: Option<CalendarMonth>,
		skip_days: Option<CalendarDay>,
	) -> Outcome<Self> {
		let rule = Self {
			rule_type,
			duration,
			count_limit,
			start_date,
			skip_years,
			day_incrementor,
			month_set,
			start_month,
			skip_months,
			skip_days,
		};
		
		// Validate the rule configuration
		res!(rule.validate());
		Ok(rule)
	}
	
	/// Validates the rule configuration.
	fn validate(&self) -> Outcome<()> {
		match self.rule_type {
			RuleType::ByYears => {
				if self.skip_years.is_none() && self.start_date.is_none() {
					return Err(err!("By-years rule requires either skip_years or start_date"; Invalid, Input));
				}
			},
			RuleType::ByExplicitMonths => {
				if self.month_set.is_none() {
					return Err(err!("By-explicit-months rule requires month_set"; Invalid, Input));
				}
			},
			RuleType::ByRegularMonths => {
				if self.skip_months.is_none() {
					return Err(err!("By-regular-months rule requires skip_months"; Invalid, Input));
				}
			},
			RuleType::ByDays => {
				if self.skip_days.is_none() {
					return Err(err!("By-days rule requires skip_days"; Invalid, Input));
				}
			},
		}
		Ok(())
	}
	
	/// Builder method to set duration.
	pub fn with_duration(mut self, duration: CalendarDuration) -> Self {
		self.duration = Some(duration);
		self
	}
	
	/// Builder method to set count limit.
	pub fn with_count_limit(mut self, count_limit: i32) -> Self {
		self.count_limit = Some(count_limit);
		self
	}
	
	/// Builder method to set start date.
	pub fn with_start_date(mut self, start_date: CalendarDate) -> Self {
		self.start_date = Some(start_date);
		self
	}
	
	/// Builder method to set skip years for by-years rules.
	pub fn with_skip_years(mut self, skip_years: CalendarYear) -> Self {
		self.skip_years = Some(skip_years);
		self
	}
	
	/// Builder method to set day incrementor for by-months rules.
	pub fn with_day_incrementor(mut self, day_incrementor: DayIncrementor) -> Self {
		self.day_incrementor = Some(day_incrementor);
		self
	}
	
	/// Builder method to set explicit months for by-explicit-months rules.
	pub fn with_month_set(mut self, month_set: BTreeSet<MonthOfYear>) -> Self {
		self.month_set = Some(month_set);
		self
	}
	
	/// Builder method to set start month for by-months rules.
	pub fn with_start_month(mut self, start_month: MonthPeriod) -> Self {
		self.start_month = Some(start_month);
		self
	}
	
	/// Builder method to set skip months for by-regular-months rules.
	pub fn with_skip_months(mut self, skip_months: CalendarMonth) -> Self {
		self.skip_months = Some(skip_months);
		self
	}
	
	/// Builder method to set skip days for by-days rules.
	pub fn with_skip_days(mut self, skip_days: CalendarDay) -> Self {
		self.skip_days = Some(skip_days);
		self
	}
	
	/// Generates all dates matching this rule within the given duration.
	pub fn to_dates(&self, from_date: &CalendarDate, duration: &CalendarDuration) -> Outcome<Vec<CalendarDate>> {
		self.generate_dates(GeneratorTask::All, from_date, Some(duration), None, None)
	}
	
	/// Generates all dates matching this rule before the specified date.
	pub fn to_dates_before(&self, before_date: &CalendarDate) -> Outcome<Vec<CalendarDate>> {
		self.generate_dates(GeneratorTask::AllOnOrBefore, before_date, None, None, None)
	}
	
	/// Generates all dates matching this rule after the specified date.
	pub fn to_dates_after(&self, after_date: &CalendarDate) -> Outcome<Vec<CalendarDate>> {
		self.generate_dates(GeneratorTask::AllOnOrAfter, after_date, None, None, None)
	}
	
	/// Generates the next N dates matching this rule after the specified date.
	pub fn next_n_dates(&self, after_date: &CalendarDate, n: i32) -> Outcome<Vec<CalendarDate>> {
		self.generate_dates(GeneratorTask::NOnOrAfter, after_date, None, Some(n), None)
	}
	
	/// Generates the previous N dates matching this rule before the specified date.
	pub fn previous_n_dates(&self, before_date: &CalendarDate, n: i32) -> Outcome<Vec<CalendarDate>> {
		self.generate_dates(GeneratorTask::NOnOrBefore, before_date, None, Some(n), None)
	}
	
	/// Gets the next date matching this rule after the specified date.
	pub fn next(&self, after_date: &CalendarDate) -> Outcome<Option<CalendarDate>> {
		let dates = res!(self.next_n_dates(after_date, 1));
		Ok(dates.first().cloned())
	}
	
	/// Gets the previous date matching this rule before the specified date.
	pub fn previous(&self, before_date: &CalendarDate) -> Outcome<Option<CalendarDate>> {
		let dates = res!(self.previous_n_dates(before_date, 1));
		Ok(dates.first().cloned())
	}
	
	/// Internal method to generate dates based on the task type.
	fn generate_dates(
		&self,
		task: GeneratorTask,
		reference_date: &CalendarDate,
		duration: Option<&CalendarDuration>,
		count: Option<i32>,
		holidays: Option<&Vec<CalClockInterval>>,
	) -> Outcome<Vec<CalendarDate>> {
		let mut results = Vec::new();
		
		match self.rule_type {
			RuleType::ByYears => {
				res!(self.generate_by_years(&mut results, task, reference_date, duration, count));
			},
			RuleType::ByExplicitMonths => {
				res!(self.generate_by_explicit_months(&mut results, task, reference_date, duration, count, holidays));
			},
			RuleType::ByRegularMonths => {
				res!(self.generate_by_regular_months(&mut results, task, reference_date, duration, count, holidays));
			},
			RuleType::ByDays => {
				res!(self.generate_by_days(&mut results, task, reference_date, duration, count));
			},
		}
		
		// Apply count limit if specified
		if let Some(limit) = self.count_limit {
			results.truncate(limit as usize);
		}
		
		Ok(results)
	}
	
	/// Generates dates for by-years rules.
	fn generate_by_years(
		&self,
		results: &mut Vec<CalendarDate>,
		task: GeneratorTask,
		reference_date: &CalendarDate,
		duration: Option<&CalendarDuration>,
		count: Option<i32>,
	) -> Outcome<()> {
		let start_date = self.start_date.as_ref()
			.ok_or_else(|| err!("By-years rule requires start_date"; Invalid, Input))?;
		
		let skip_years = self.skip_years.as_ref().map(|y| y.of()).unwrap_or(1);
		let mut current_year = start_date.year();
		
		// Adjust starting year based on task
		match task {
			GeneratorTask::AllOnOrAfter | GeneratorTask::NOnOrAfter => {
				while current_year < reference_date.year() {
					current_year += skip_years;
				}
			},
			GeneratorTask::AllOnOrBefore | GeneratorTask::NOnOrBefore => {
				while current_year > reference_date.year() {
					current_year -= skip_years;
				}
			},
			_ => {}
		}
		
		let max_count = count.unwrap_or(1000); // Reasonable default limit
		let mut generated = 0;
		
		loop {
			if generated >= max_count {
				break;
			}
			
			// Create date for this year
			let candidate_date = res!(CalendarDate::from_ymd(
				current_year,
				start_date.month_of_year(),
				start_date.day(),
				start_date.zone().clone()
			));
			
			// Check if this date meets the task criteria
			let should_include = match task {
				GeneratorTask::All => {
					if let Some(dur) = duration {
						let end_date = res!(reference_date.add_calendar_duration(dur));
						candidate_date >= *reference_date && candidate_date <= end_date
					} else {
						true
					}
				},
				GeneratorTask::AllOnOrBefore | GeneratorTask::NOnOrBefore => {
					candidate_date <= *reference_date
				},
				GeneratorTask::AllOnOrAfter | GeneratorTask::NOnOrAfter => {
					candidate_date >= *reference_date
				},
			};
			
			if should_include {
				results.push(candidate_date);
				generated += 1;
			}
			
			// Move to next/previous year
			match task {
				GeneratorTask::AllOnOrBefore | GeneratorTask::NOnOrBefore => {
					current_year -= skip_years;
					if current_year < 1 { break; } // Reasonable lower bound
				},
				_ => {
					current_year += skip_years;
					if current_year > 3000 { break; } // Reasonable upper bound
				},
			}
		}
		
		Ok(())
	}
	
	/// Generates dates for by-explicit-months rules.
	fn generate_by_explicit_months(
		&self,
		results: &mut Vec<CalendarDate>,
		task: GeneratorTask,
		reference_date: &CalendarDate,
		duration: Option<&CalendarDuration>,
		count: Option<i32>,
		holidays: Option<&Vec<CalClockInterval>>,
	) -> Outcome<()> {
		let month_set = self.month_set.as_ref()
			.ok_or_else(|| err!("By-explicit-months rule requires month_set"; Invalid, Input))?;
		
		let day_incrementor = self.day_incrementor.as_ref()
			.ok_or_else(|| err!("By-explicit-months rule requires day_incrementor"; Invalid, Input))?;
		
		let start_year = reference_date.year();
		let max_count = count.unwrap_or(1000);
		let mut generated = 0;
		
		// Generate for multiple years if needed
		for year_offset in -5..=5 { // Search 5 years before and after
			if generated >= max_count {
				break;
			}
			
			let current_year = start_year + year_offset;
			
			for &month in month_set {
				if generated >= max_count {
					break;
				}
				
				// Use day incrementor to find the target date in this month
				let month_start = res!(CalendarDate::from_ymd(current_year, month, 1, reference_date.zone().clone()));
				
				if let Ok(target_date) = day_incrementor.calculate_date(current_year, month.of(), reference_date.zone().clone()) {
					let should_include = match task {
						GeneratorTask::All => {
							if let Some(dur) = duration {
								let end_date = res!(reference_date.add_calendar_duration(dur));
								target_date >= *reference_date && target_date <= end_date
							} else {
								true
							}
						},
						GeneratorTask::AllOnOrBefore | GeneratorTask::NOnOrBefore => {
							target_date <= *reference_date
						},
						GeneratorTask::AllOnOrAfter | GeneratorTask::NOnOrAfter => {
							target_date >= *reference_date
						},
					};
					
					if should_include {
						results.push(target_date);
						generated += 1;
					}
				}
			}
		}
		
		// Sort results chronologically
		results.sort();
		
		Ok(())
	}
	
	/// Generates dates for by-regular-months rules.
	fn generate_by_regular_months(
		&self,
		results: &mut Vec<CalendarDate>,
		task: GeneratorTask,
		reference_date: &CalendarDate,
		duration: Option<&CalendarDuration>,
		count: Option<i32>,
		holidays: Option<&Vec<CalClockInterval>>,
	) -> Outcome<()> {
		let skip_months = self.skip_months.as_ref()
			.ok_or_else(|| err!("By-regular-months rule requires skip_months"; Invalid, Input))?;
		
		let day_incrementor = self.day_incrementor.as_ref()
			.ok_or_else(|| err!("By-regular-months rule requires day_incrementor"; Invalid, Input))?;
		
		let start_month = self.start_month.as_ref()
			.map(|m| m.get_month_of_year())
			.unwrap_or(reference_date.month_of_year());
		
		let skip_interval = skip_months.of() as i32;
		let max_count = count.unwrap_or(1000);
		let mut generated = 0;
		
		let mut current_date = reference_date.clone();
		
		// Adjust to start month if needed
		if current_date.month_of_year() != start_month {
			let target_month = start_month as i32;
			let current_month = current_date.month_of_year() as i32;
			let month_diff = target_month - current_month;
			
			current_date = res!(current_date.add_months(month_diff));
		}
		
		loop {
			if generated >= max_count {
				break;
			}
			
			// Calculate target date using day incrementor
			if let Ok(target_date) = day_incrementor.calculate_date(current_date.year(), current_date.month_of_year().of(), current_date.zone().clone()) {
				let should_include = match task {
					GeneratorTask::All => {
						if let Some(dur) = duration {
							let end_date = res!(reference_date.add_calendar_duration(dur));
							target_date >= *reference_date && target_date <= end_date
						} else {
							true
						}
					},
					GeneratorTask::AllOnOrBefore | GeneratorTask::NOnOrBefore => {
						target_date <= *reference_date
					},
					GeneratorTask::AllOnOrAfter | GeneratorTask::NOnOrAfter => {
						target_date >= *reference_date
					},
				};
				
				if should_include {
					results.push(target_date);
					generated += 1;
				}
			}
			
			// Move to next interval
			current_date = res!(current_date.add_months(skip_interval));
			
			// Bounds check
			if current_date.year() > 3000 || current_date.year() < 1 {
				break;
			}
		}
		
		Ok(())
	}
	
	/// Generates dates for by-days rules.
	fn generate_by_days(
		&self,
		results: &mut Vec<CalendarDate>,
		task: GeneratorTask,
		reference_date: &CalendarDate,
		duration: Option<&CalendarDuration>,
		count: Option<i32>,
	) -> Outcome<()> {
		let skip_days = self.skip_days.as_ref()
			.ok_or_else(|| err!("By-days rule requires skip_days"; Invalid, Input))?;
		
		let skip_interval = skip_days.of() as i32;
		let max_count = count.unwrap_or(1000);
		let mut generated = 0;
		
		let mut current_date = reference_date.clone();
		
		loop {
			if generated >= max_count {
				break;
			}
			
			let should_include = match task {
				GeneratorTask::All => {
					if let Some(dur) = duration {
						let end_date = res!(reference_date.add_calendar_duration(dur));
						current_date >= *reference_date && current_date <= end_date
					} else {
						true
					}
				},
				GeneratorTask::AllOnOrBefore | GeneratorTask::NOnOrBefore => {
					current_date <= *reference_date
				},
				GeneratorTask::AllOnOrAfter | GeneratorTask::NOnOrAfter => {
					current_date >= *reference_date
				},
			};
			
			if should_include {
				results.push(current_date.clone());
				generated += 1;
			}
			
			// Move to next interval
			match task {
				GeneratorTask::AllOnOrBefore | GeneratorTask::NOnOrBefore => {
					current_date = res!(current_date.add_days(skip_interval));
				},
				_ => {
					current_date = res!(current_date.add_days(skip_interval));
				},
			}
			
			// Bounds check
			if current_date.year() > 3000 || current_date.year() < 1 {
				break;
			}
		}
		
		Ok(())
	}
	
	/// Returns the rule type.
	pub fn rule_type(&self) -> &RuleType {
		&self.rule_type
	}
	
	/// Returns the duration limit, if any.
	pub fn duration(&self) -> Option<&CalendarDuration> {
		self.duration.as_ref()
	}
	
	/// Returns the count limit, if any.
	pub fn count_limit(&self) -> Option<i32> {
		self.count_limit
	}
	
	/// Returns the start date, if any.
	pub fn start_date(&self) -> Option<&CalendarDate> {
		self.start_date.as_ref()
	}
}

impl std::fmt::Display for CalendarRule {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "CalendarRule({:?})", self.rule_type)
	}
}