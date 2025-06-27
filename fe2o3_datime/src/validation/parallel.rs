use crate::{
    calendar::CalendarDate,
    clock::ClockTime,
    time::CalClock,
    validation::{CalClockValidator, ValidationError, ValidationResult},
};

use oxedyne_fe2o3_core::prelude::*;

use std::{
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

/// High-performance parallel validator for batch validation operations.
///
/// ParallelValidator distributes validation work across multiple threads
/// to maximise throughput when validating large collections of CalClocks.
/// This is particularly beneficial for data import, batch processing, and
/// high-volume validation scenarios.
///
/// # Examples
///
/// ```ignore
/// use oxedyne_fe2o3_datime::validation::{ParallelValidator, CalClockValidator};
///
/// let validator = CalClockValidator::new();
/// let parallel_validator = ParallelValidator::new(validator, 4); // 4 threads
///
/// let calclocks = vec![/* large collection */];
/// let results = parallel_validator.validate_batch(&calclocks);
///
/// println!("Validated {} items in parallel", results.total_items);
/// ```
#[derive(Debug)]
pub struct ParallelValidator {
    /// The base validator to use.
    validator: Arc<CalClockValidator>,
    /// Number of worker threads.
    thread_count: usize,
    /// Work chunk size for optimal load balancing.
    chunk_size: usize,
}

impl ParallelValidator {
    /// Creates a new parallel validator with the specified thread count.
    pub fn new(validator: CalClockValidator, thread_count: usize) -> Self {
        Self {
            validator: Arc::new(validator),
            thread_count: std::cmp::max(1, thread_count),
            chunk_size: 100, // Default chunk size
        }
    }

    /// Creates a new parallel validator with custom chunk size.
    pub fn with_chunk_size(mut self, chunk_size: usize) -> Self {
        self.chunk_size = std::cmp::max(1, chunk_size);
        self
    }

    /// Validates a batch of CalClocks in parallel.
    pub fn validate_batch(&self, calclocks: &[CalClock]) -> BatchValidationResult {
        let _start_time = Instant::now();
        
        if calclocks.is_empty() {
            return BatchValidationResult {
                total_items: 0,
                valid_items: 0,
                invalid_items: 0,
                validation_errors: Vec::new(),
                execution_time: Duration::new(0, 0),
                thread_count: self.thread_count,
                chunk_size: self.chunk_size,
            };
        }

        // Clone the data to avoid lifetime issues
        let calclocks_owned: Vec<CalClock> = calclocks.to_vec();
        
        // Distribute work across threads
        let results = Arc::new(Mutex::new(Vec::new()));
        let mut handles = Vec::new();

        let items_per_thread = (calclocks_owned.len() + self.thread_count - 1) / self.thread_count;
        
        for thread_id in 0..self.thread_count {
            let start_idx = thread_id * items_per_thread;
            let end_idx = std::cmp::min(start_idx + items_per_thread, calclocks_owned.len());
            
            if start_idx >= calclocks_owned.len() {
                break;
            }
            
            let validator = Arc::clone(&self.validator);
            let results = Arc::clone(&results);
            let thread_items: Vec<CalClock> = calclocks_owned[start_idx..end_idx].to_vec();

            let handle = thread::spawn(move || {
                let mut local_results = Vec::new();
                
                for (local_index, calclock) in thread_items.iter().enumerate() {
                    let global_index = start_idx + local_index;
                    let validation_result = validator.validate_calclock(calclock);
                    let item_result = ValidationItemResult {
                        index: global_index,
                        calclock: calclock.clone(),
                        result: validation_result,
                    };
                    local_results.push(item_result);
                }

                // Add to shared results
                if let Ok(mut results) = results.lock() {
                    results.extend(local_results);
                }
            });

            handles.push(handle);
        }

        // Wait for all threads to complete
        for handle in handles {
            if let Err(_) = handle.join() {
                // Handle thread panic - in production you'd want better error handling
            }
        }

        let execution_time = _start_time.elapsed();
        
        // Collect results
        let all_results = if let Ok(results) = results.lock() {
            results.clone()
        } else {
            Vec::new()
        };

        // Aggregate statistics
        let total_items = all_results.len();
        let valid_items = all_results.iter().filter(|r| r.result.is_ok()).count();
        let invalid_items = total_items - valid_items;

        let validation_errors: Vec<ValidationItemError> = all_results
            .iter()
            .filter_map(|r| {
                if let Err(errors) = &r.result {
                    Some(ValidationItemError {
                        index: r.index,
                        calclock: r.calclock.clone(),
                        errors: errors.clone(),
                    })
                } else {
                    None
                }
            })
            .collect();

        BatchValidationResult {
            total_items,
            valid_items,
            invalid_items,
            validation_errors,
            execution_time,
            thread_count: self.thread_count,
            chunk_size: self.chunk_size,
        }
    }

    /// Validates CalendarDates in parallel.
    pub fn validate_dates_batch(&self, dates: &[CalendarDate]) -> BatchValidationResult {
        let _start_time = Instant::now();
        
        if dates.is_empty() {
            return BatchValidationResult {
                total_items: 0,
                valid_items: 0,
                invalid_items: 0,
                validation_errors: Vec::new(),
                execution_time: Duration::new(0, 0),
                thread_count: self.thread_count,
                chunk_size: self.chunk_size,
            };
        }

        // Convert dates to minimal CalClocks for validation
        let calclocks: Vec<CalClock> = dates
            .iter()
            .filter_map(|date| {
                let zone = date.zone().clone();
                if let Ok(time) = crate::clock::ClockTime::new(0, 0, 0, 0, zone) {
                    crate::time::CalClock::from_date_time(date.clone(), time).ok()
                } else {
                    None
                }
            })
            .collect();

        self.validate_batch(&calclocks)
    }

    /// Validates ClockTimes in parallel.
    pub fn validate_times_batch(&self, times: &[ClockTime]) -> BatchValidationResult {
        let _start_time = Instant::now();
        
        if times.is_empty() {
            return BatchValidationResult {
                total_items: 0,
                valid_items: 0,
                invalid_items: 0,
                validation_errors: Vec::new(),
                execution_time: Duration::new(0, 0),
                thread_count: self.thread_count,
                chunk_size: self.chunk_size,
            };
        }

        // Convert times to minimal CalClocks for validation
        let calclocks: Vec<CalClock> = times
            .iter()
            .filter_map(|time| {
                let zone = time.zone().clone();
                if let Ok(date) = crate::calendar::CalendarDate::new(2024, 1, 1, zone) {
                    crate::time::CalClock::from_date_time(date, time.clone()).ok()
                } else {
                    None
                }
            })
            .collect();

        self.validate_batch(&calclocks)
    }

    /// Filters a collection to return only valid CalClocks using parallel processing.
    pub fn filter_valid_parallel(&self, calclocks: Vec<CalClock>) -> Vec<CalClock> {
        let results = self.validate_batch(&calclocks);
        
        calclocks
            .into_iter()
            .enumerate()
            .filter(|(index, _)| {
                !results.validation_errors.iter().any(|err| err.index == *index)
            })
            .map(|(_, calclock)| calclock)
            .collect()
    }

    /// Gets the configured thread count.
    pub fn thread_count(&self) -> usize {
        self.thread_count
    }

    /// Gets the configured chunk size.
    pub fn chunk_size(&self) -> usize {
        self.chunk_size
    }
}

/// Result of a batch validation operation.
#[derive(Debug, Clone)]
pub struct BatchValidationResult {
    /// Total number of items validated.
    pub total_items: usize,
    /// Number of valid items.
    pub valid_items: usize,
    /// Number of invalid items.
    pub invalid_items: usize,
    /// Detailed validation errors.
    pub validation_errors: Vec<ValidationItemError>,
    /// Total execution time.
    pub execution_time: Duration,
    /// Number of threads used.
    pub thread_count: usize,
    /// Chunk size used for processing.
    pub chunk_size: usize,
}

impl BatchValidationResult {
    /// Gets the success rate as a percentage.
    pub fn success_rate(&self) -> f64 {
        if self.total_items == 0 {
            0.0
        } else {
            self.valid_items as f64 / self.total_items as f64
        }
    }

    /// Gets the validation throughput (items per second).
    pub fn throughput(&self) -> f64 {
        if self.execution_time.as_secs_f64() == 0.0 {
            0.0
        } else {
            self.total_items as f64 / self.execution_time.as_secs_f64()
        }
    }

    /// Gets the average time per item.
    pub fn average_time_per_item(&self) -> Duration {
        if self.total_items == 0 {
            Duration::new(0, 0)
        } else {
            self.execution_time / self.total_items as u32
        }
    }

    /// Checks if all items passed validation.
    pub fn all_valid(&self) -> bool {
        self.invalid_items == 0
    }

    /// Gets a summary of error types.
    pub fn error_summary(&self) -> std::collections::HashMap<String, usize> {
        let mut summary = std::collections::HashMap::new();
        
        for item_error in &self.validation_errors {
            for error in &item_error.errors {
                *summary.entry(error.rule.clone()).or_insert(0) += 1;
            }
        }
        
        summary
    }

    /// Formats the result as a human-readable string.
    pub fn format(&self) -> String {
        format!(
            "Batch Validation Result:\n\
             - Total items: {}\n\
             - Valid: {} ({:.1}%)\n\
             - Invalid: {} ({:.1}%)\n\
             - Execution time: {:?}\n\
             - Throughput: {:.0} items/sec\n\
             - Threads used: {}\n\
             - Chunk size: {}",
            self.total_items,
            self.valid_items,
            self.success_rate() * 100.0,
            self.invalid_items,
            (self.invalid_items as f64 / self.total_items as f64) * 100.0,
            self.execution_time,
            self.throughput(),
            self.thread_count,
            self.chunk_size
        )
    }
}

/// Validation error for a specific item in a batch.
#[derive(Debug, Clone)]
pub struct ValidationItemError {
    /// Index of the item in the original batch.
    pub index: usize,
    /// The CalClock that failed validation.
    pub calclock: CalClock,
    /// The validation errors.
    pub errors: Vec<ValidationError>,
}

/// Internal result for a single validation item.
#[derive(Debug, Clone)]
struct ValidationItemResult {
    /// Index in the batch.
    index: usize,
    /// The CalClock being validated.
    calclock: CalClock,
    /// Validation result.
    result: ValidationResult,
}

/// Automatic thread count detection based on system capabilities.
pub fn optimal_thread_count() -> usize {
    // Use number of logical CPUs, but cap at reasonable limits
    let cpu_count = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);
    
    // Cap between 2 and 16 threads for validation workloads
    std::cmp::min(16, std::cmp::max(2, cpu_count))
}

/// Creates a parallel validator with optimal settings for the current system.
pub fn create_optimal_parallel_validator(validator: CalClockValidator) -> ParallelValidator {
    let thread_count = optimal_thread_count();
    ParallelValidator::new(validator, thread_count)
        .with_chunk_size(50) // Balanced chunk size for most workloads
}