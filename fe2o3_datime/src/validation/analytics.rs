//! Metrics for validation: how often it runs, how often it passes, how long
//! it takes, and which rules do the rejecting.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use crate::{
    validation::ValidationResult,
};

use oxedyne_fe2o3_core::prelude::*;

use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

/// # Examples
///
/// ```ignore
/// use oxedyne_fe2o3_datime::validation::{ValidationAnalytics, CalClockValidator};
///
/// let mut analytics = ValidationAnalytics::new();
/// let validator = CalClockValidator::new();
///
/// // Validate with analytics tracking
/// let result = analytics.track_validation(|| {
///     validator.validate_calclock(&some_calclock)
/// });
///
/// // Get comprehensive metrics
/// let metrics = analytics.get_metrics();
/// println!("Success rate: {:.2}%", metrics.success_rate() * 100.0);
/// ```
#[derive(Debug, Clone)]
pub struct ValidationAnalytics {
    total_validations: u64,
    successful_validations: u64,
    failed_validations: u64,
    total_validation_time: Duration,
    error_frequency: HashMap<String, u64>,
    rule_performance: HashMap<String, RulePerformance>,
    recent_validations: Vec<ValidationRecord>,
    max_history: usize,
}

impl ValidationAnalytics {
    pub fn new() -> Self {
        Self {
            total_validations: 0,
            successful_validations: 0,
            failed_validations: 0,
            total_validation_time: Duration::new(0, 0),
            error_frequency: HashMap::new(),
            rule_performance: HashMap::new(),
            recent_validations: Vec::new(),
            max_history: 1000,
        }
    }

    pub fn with_history_size(max_history: usize) -> Self {
        Self {
            max_history,
            ..Self::new()
        }
    }

    pub fn track_validation<F, T>(&mut self, operation: F) -> T
    where
        F: FnOnce() -> T,
    {
        let start_time = Instant::now();
        let result = operation();
        let duration = start_time.elapsed();

        self.total_validations += 1;
        self.total_validation_time += duration;

        // Add to recent history
        let record = ValidationRecord {
            timestamp: Instant::now(),
            duration,
            success: true, // This is a simplified version - would need actual result analysis
        };

        self.recent_validations.push(record);
        if self.recent_validations.len() > self.max_history {
            self.recent_validations.remove(0);
        }

        result
    }

    pub fn track_validation_result(&mut self, result: &ValidationResult, rule_name: &str, duration: Duration) {
        self.total_validations += 1;
        self.total_validation_time += duration;

        match result {
            Ok(_) => {
                self.successful_validations += 1;
            }
            Err(errors) => {
                self.failed_validations += 1;
                
                // Track error frequency
                for error in errors {
                    *self.error_frequency.entry(error.rule.clone()).or_insert(0) += 1;
                }
            }
        }

        // Update rule performance metrics
        let perf = self.rule_performance.entry(rule_name.to_string()).or_insert_with(RulePerformance::new);
        perf.add_measurement(duration, result.is_ok());

        // Add to recent history
        let record = ValidationRecord {
            timestamp: Instant::now(),
            duration,
            success: result.is_ok(),
        };

        self.recent_validations.push(record);
        if self.recent_validations.len() > self.max_history {
            self.recent_validations.remove(0);
        }
    }

    pub fn record_error(&mut self, rule_name: &str) {
        *self.error_frequency.entry(rule_name.to_string()).or_insert(0) += 1;
    }

    pub fn get_metrics(&self) -> ValidationMetrics {
        ValidationMetrics {
            total_validations: self.total_validations,
            successful_validations: self.successful_validations,
            failed_validations: self.failed_validations,
            total_validation_time: self.total_validation_time,
            error_frequency: self.error_frequency.clone(),
            rule_performance: self.rule_performance.clone(),
            recent_trend: self.calculate_recent_trend(),
        }
    }

    pub fn generate_report(&self) -> ValidationReport {
        let metrics = self.get_metrics();
        ValidationReport::new(metrics)
    }

    pub fn reset(&mut self) {
        self.total_validations = 0;
        self.successful_validations = 0;
        self.failed_validations = 0;
        self.total_validation_time = Duration::new(0, 0);
        self.error_frequency.clear();
        self.rule_performance.clear();
        self.recent_validations.clear();
    }

    fn calculate_recent_trend(&self) -> ValidationTrend {
        if self.recent_validations.len() < 10 {
            return ValidationTrend::Insufficient;
        }

        let recent_count = std::cmp::min(100, self.recent_validations.len());
        let recent = &self.recent_validations[self.recent_validations.len() - recent_count..];
        
        let success_count = recent.iter().filter(|r| r.success).count();
        let success_rate = success_count as f64 / recent.len() as f64;

        if success_rate > 0.95 {
            ValidationTrend::Excellent
        } else if success_rate > 0.90 {
            ValidationTrend::Good
        } else if success_rate > 0.80 {
            ValidationTrend::Moderate
        } else {
            ValidationTrend::Poor
        }
    }
}

impl Default for ValidationAnalytics {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone)]
pub struct RulePerformance {
    pub execution_count: u64,
    pub total_time: Duration,
    pub success_count: u64,
    pub min_time: Option<Duration>,
    pub max_time: Option<Duration>,
}

impl RulePerformance {
    pub fn new() -> Self {
        Self {
            execution_count: 0,
            total_time: Duration::new(0, 0),
            success_count: 0,
            min_time: None,
            max_time: None,
        }
    }

    pub fn add_measurement(&mut self, duration: Duration, success: bool) {
        self.execution_count += 1;
        self.total_time += duration;
        
        if success {
            self.success_count += 1;
        }

        match self.min_time {
            None => self.min_time = Some(duration),
            Some(min) if duration < min => self.min_time = Some(duration),
            _ => {}
        }

        match self.max_time {
            None => self.max_time = Some(duration),
            Some(max) if duration > max => self.max_time = Some(duration),
            _ => {}
        }
    }

    pub fn average_time(&self) -> Duration {
        if self.execution_count == 0 {
            Duration::new(0, 0)
        } else {
            self.total_time / self.execution_count as u32
        }
    }

    /// A fraction between zero and one, not a percentage.
    pub fn success_rate(&self) -> f64 {
        if self.execution_count == 0 {
            0.0
        } else {
            self.success_count as f64 / self.execution_count as f64
        }
    }
}

impl Default for RulePerformance {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone)]
pub struct ValidationRecord {
    pub timestamp: Instant,
    pub duration: Duration,
    pub success: bool,
}

#[derive(Debug, Clone)]
pub struct ValidationMetrics {
    pub total_validations: u64,
    pub successful_validations: u64,
    pub failed_validations: u64,
    pub total_validation_time: Duration,
    pub error_frequency: HashMap<String, u64>,
    pub rule_performance: HashMap<String, RulePerformance>,
    pub recent_trend: ValidationTrend,
}

impl ValidationMetrics {
    /// A fraction between zero and one, not a percentage.
    pub fn success_rate(&self) -> f64 {
        if self.total_validations == 0 {
            0.0
        } else {
            self.successful_validations as f64 / self.total_validations as f64
        }
    }

    pub fn average_validation_time(&self) -> Duration {
        if self.total_validations == 0 {
            Duration::new(0, 0)
        } else {
            self.total_validation_time / self.total_validations as u32
        }
    }

    pub fn most_frequent_error(&self) -> Option<(String, u64)> {
        self.error_frequency
            .iter()
            .max_by_key(|(_, count)| *count)
            .map(|(rule, count)| (rule.clone(), *count))
    }

    pub fn slowest_rule(&self) -> Option<(String, Duration)> {
        self.rule_performance
            .iter()
            .max_by_key(|(_, perf)| perf.average_time())
            .map(|(rule, perf)| (rule.clone(), perf.average_time()))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ValidationTrend {
    // Banded by success rate over the recent history.
    Insufficient,   // too few records to say
    Excellent,      // above 95%
    Good,           // 90 to 95%
    Moderate,       // 80 to 90%
    Poor,           // below 80%
}

#[derive(Debug, Clone)]
pub struct ValidationReport {
    pub metrics: ValidationMetrics,
    pub summary: String,
    pub recommendations: Vec<String>,
    pub error_analysis: Vec<String>,
}

impl ValidationReport {
    pub fn new(metrics: ValidationMetrics) -> Self {
        let mut report = Self {
            metrics: metrics.clone(),
            summary: String::new(),
            recommendations: Vec::new(),
            error_analysis: Vec::new(),
        };

        report.generate_summary();
        report.generate_recommendations();
        report.generate_error_analysis();

        report
    }

    fn generate_summary(&mut self) {
        let success_rate = self.metrics.success_rate() * 100.0;
        let avg_time = self.metrics.average_validation_time();
        
        self.summary = format!(
            "Validation Summary: {:.1}% success rate across {} operations. \
             Average validation time: {:?}. Trend: {:?}",
            success_rate,
            self.metrics.total_validations,
            avg_time,
            self.metrics.recent_trend
        );
    }

    fn generate_recommendations(&mut self) {
        // Performance recommendations
        if let Some((rule, duration)) = self.metrics.slowest_rule() {
            if duration.as_millis() > 10 {
                self.recommendations.push(format!(
                    "Consider optimizing '{}' rule - average execution time: {:?}",
                    rule, duration
                ));
            }
        }

        // Success rate recommendations
        if self.metrics.success_rate() < 0.95 {
            self.recommendations.push(
                "Success rate below 95% - consider reviewing validation rules".to_string()
            );
        }

        // Trend recommendations
        match self.metrics.recent_trend {
            ValidationTrend::Poor => {
                self.recommendations.push(
                    "Poor recent performance - immediate investigation recommended".to_string()
                );
            }
            ValidationTrend::Moderate => {
                self.recommendations.push(
                    "Moderate performance - consider rule adjustments".to_string()
                );
            }
            _ => {}
        }
    }

    fn generate_error_analysis(&mut self) {
        if let Some((rule, count)) = self.metrics.most_frequent_error() {
            self.error_analysis.push(format!(
                "Most frequent error: '{}' with {} occurrences", rule, count
            ));
        }

        let total_errors = self.metrics.failed_validations;
        if total_errors > 0 {
            self.error_analysis.push(format!(
                "Total errors: {} ({:.1}% of all validations)",
                total_errors,
                (total_errors as f64 / self.metrics.total_validations as f64) * 100.0
            ));
        }
    }

    pub fn format(&self) -> String {
        let mut output = String::new();
        
        output.push_str("=== Validation Report ===\n\n");
        output.push_str(&format!("{}\n\n", self.summary));
        
        if !self.recommendations.is_empty() {
            output.push_str("Recommendations:\n");
            for rec in &self.recommendations {
                output.push_str(&format!("- {}\n", rec));
            }
            output.push('\n');
        }
        
        if !self.error_analysis.is_empty() {
            output.push_str("Error Analysis:\n");
            for analysis in &self.error_analysis {
                output.push_str(&format!("- {}\n", analysis));
            }
        }
        
        output
    }
}