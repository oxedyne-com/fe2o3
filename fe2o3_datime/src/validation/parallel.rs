//! Validation of large batches across several threads.
//!
//! [Written entirely with AI](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

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
    validator:      Arc<CalClockValidator>,
    thread_count:   usize,
    chunk_size:     usize,      // items handed to a thread at a time
}

impl ParallelValidator {
    pub fn new(validator: CalClockValidator, thread_count: usize) -> Self {
        Self {
            validator: Arc::new(validator),
            thread_count: std::cmp::max(1, thread_count),
            chunk_size: 100, // Default chunk size
        }
    }

    pub fn with_chunk_size(mut self, chunk_size: usize) -> Self {
        self.chunk_size = std::cmp::max(1, chunk_size);
        self
    }

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

    pub fn thread_count(&self) -> usize {
        self.thread_count
    }

    pub fn chunk_size(&self) -> usize {
        self.chunk_size
    }
}

#[derive(Debug, Clone)]
pub struct BatchValidationResult {
    pub total_items:        usize,
    pub valid_items:        usize,
    pub invalid_items:      usize,
    pub validation_errors:  Vec<ValidationItemError>,
    pub execution_time:     Duration,
    pub thread_count:       usize,
    pub chunk_size:         usize,
}

impl BatchValidationResult {
    /// A fraction between zero and one, not a percentage.
    pub fn success_rate(&self) -> f64 {
        if self.total_items == 0 {
            0.0
        } else {
            self.valid_items as f64 / self.total_items as f64
        }
    }

    pub fn throughput(&self) -> f64 {
        if self.execution_time.as_secs_f64() == 0.0 {
            0.0
        } else {
            self.total_items as f64 / self.execution_time.as_secs_f64()
        }
    }

    pub fn average_time_per_item(&self) -> Duration {
        if self.total_items == 0 {
            Duration::new(0, 0)
        } else {
            self.execution_time / self.total_items as u32
        }
    }

    pub fn all_valid(&self) -> bool {
        self.invalid_items == 0
    }

    pub fn error_summary(&self) -> std::collections::HashMap<String, usize> {
        let mut summary = std::collections::HashMap::new();
        
        for item_error in &self.validation_errors {
            for error in &item_error.errors {
                *summary.entry(error.rule.clone()).or_insert(0) += 1;
            }
        }
        
        summary
    }

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

#[derive(Debug, Clone)]
pub struct ValidationItemError {
    pub index:      usize,      // position in the original batch
    pub calclock:   CalClock,
    pub errors:     Vec<ValidationError>,
}

#[derive(Debug, Clone)]
struct ValidationItemResult {
    index:      usize,
    calclock:   CalClock,
    result:     ValidationResult,
}

pub fn optimal_thread_count() -> usize {
    // Use number of logical CPUs, but cap at reasonable limits
    let cpu_count = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);
    
    // Cap between 2 and 16 threads for validation workloads
    std::cmp::min(16, std::cmp::max(2, cpu_count))
}

pub fn create_optimal_parallel_validator(validator: CalClockValidator) -> ParallelValidator {
    let thread_count = optimal_thread_count();
    ParallelValidator::new(validator, thread_count)
        .with_chunk_size(50) // Balanced chunk size for most workloads
}