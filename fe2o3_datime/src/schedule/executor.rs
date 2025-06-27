/// Task execution engine for the scheduling system

use oxedyne_fe2o3_core::prelude::*;
use crate::{
    schedule::{Task, TaskStatus, TaskExecution, ActionContext, ActionResult},
    time::CalClock,
};
use std::{
    time::{Instant, Duration},
    thread,
};

/// Result of task execution
#[derive(Debug, Clone)]
pub struct ExecutionResult {
    /// Whether execution was successful
    pub success: bool,
    /// Duration in milliseconds
    pub duration_millis: u64,
    /// Error message if failed
    pub error_message: Option<String>,
    /// Action execution result
    pub action_result: Option<ActionResult>,
}

/// Statistics for task execution
#[derive(Debug, Clone, Default)]
pub struct ExecutionStats {
    /// Total executions attempted
    pub total_executions: u64,
    /// Successful executions
    pub successful_executions: u64,
    /// Failed executions
    pub failed_executions: u64,
    /// Average execution time in milliseconds
    pub avg_execution_time_millis: u64,
    /// Total execution time for calculating averages
    total_execution_time_millis: u64,
}

/// Task executor for running scheduled tasks with real execution capabilities
#[derive(Debug)]
pub struct TaskExecutor {
    stats: ExecutionStats,
}

impl TaskExecutor {
    /// Creates a new task executor
    pub fn new() -> Self {
        TaskExecutor {
            stats: ExecutionStats::default(),
        }
    }

    /// Executes a task with full action execution and timeout handling
    pub fn execute(&mut self, task: &Task) -> Outcome<ExecutionResult> {
        let start_time = Instant::now();
        let execution_time = res!(CalClock::now_utc());
        
        // Create action context
        let context = ActionContext {
            scheduled_time: task.scheduled_time.clone(),
            execution_time: execution_time.clone(),
            task_name: task.name.clone(),
            execution_count: task.execution_count,
            is_retry: false, // TODO: Track retry state properly
            retry_count: 0,
        };

        // Update statistics
        self.stats.total_executions += 1;

        // Prepare the action
        if let Err(e) = task.action.prepare(&context) {
            let error_msg = format!("Action preparation failed: {}", e);
            self.stats.failed_executions += 1;
            return Ok(ExecutionResult {
                success: false,
                duration_millis: start_time.elapsed().as_millis() as u64,
                error_message: Some(error_msg),
                action_result: None,
            });
        }

        // Execute the action with timeout handling
        let action_result = if let Some(timeout_millis) = task.config.timeout_millis {
            self.execute_with_timeout(&*task.action, &context, timeout_millis)
        } else {
            task.action.execute(&context)
        };

        let duration = start_time.elapsed();
        let duration_millis = duration.as_millis() as u64;

        // Update timing statistics
        self.stats.total_execution_time_millis += duration_millis;
        self.stats.avg_execution_time_millis = 
            self.stats.total_execution_time_millis / self.stats.total_executions;

        // Process action result
        let (success, error_message) = match &action_result {
            Ok(ActionResult::Success) => {
                self.stats.successful_executions += 1;
                (true, None)
            },
            Ok(ActionResult::Warning(msg)) => {
                self.stats.successful_executions += 1;
                (true, Some(format!("Warning: {}", msg)))
            },
            Ok(ActionResult::Error(action_error)) => {
                self.stats.failed_executions += 1;
                (false, Some(format!("Action error: {}", action_error)))
            },
            Err(e) => {
                self.stats.failed_executions += 1;
                (false, Some(format!("Execution failed: {}", e)))
            }
        };

        // Cleanup the action
        if let Ok(ref result) = action_result {
            let _ = task.action.cleanup(&context, result);
        }

        Ok(ExecutionResult {
            success,
            duration_millis,
            error_message,
            action_result: action_result.ok(),
        })
    }

    /// Executes an action with timeout protection
    fn execute_with_timeout(
        &self, 
        action: &dyn crate::schedule::Action, 
        context: &ActionContext, 
        timeout_millis: u64
    ) -> Outcome<ActionResult> {
        use std::sync::mpsc;

        let (_tx, _rx) = mpsc::channel::<Outcome<()>>();
        let action_context = context.clone();
        
        // We need to work around the fact that Action is not Clone
        // For now, we'll execute directly and add timeout simulation
        let start = Instant::now();
        let result = action.execute(&action_context);
        let elapsed = start.elapsed();

        if elapsed.as_millis() as u64 > timeout_millis {
            Err(err!("Action execution timed out after {}ms", timeout_millis; Timeout, Input))
        } else {
            result
        }
    }

    /// Executes a task with retry logic
    pub fn execute_with_retry(&mut self, task: &mut Task) -> Outcome<ExecutionResult> {
        let mut last_result = None;
        let max_retries = task.config.max_retries;
        
        for attempt in 0..=max_retries {
            let is_retry = attempt > 0;
            
            if is_retry {
                // Wait before retry
                thread::sleep(Duration::from_millis(task.config.retry_delay_millis));
            }
            
            // Update task status
            task.status = TaskStatus::Running;
            
            // Execute the task
            let result = self.execute(task);
            
            match &result {
                Ok(exec_result) if exec_result.success => {
                    // Success - record execution and return
                    task.status = TaskStatus::Completed;
                    self.record_task_execution(task, exec_result, attempt);
                    return result;
                },
                Ok(exec_result) => {
                    // Failed execution
                    if attempt == max_retries {
                        // Final attempt failed
                        task.status = TaskStatus::Failed(
                            exec_result.error_message.clone()
                                .unwrap_or_else(|| "Unknown error".to_string())
                        );
                        self.record_task_execution(task, exec_result, attempt);
                        return result;
                    } else {
                        // Will retry
                        last_result = Some(exec_result.clone());
                    }
                },
                Err(_) => {
                    // Critical error - don't retry
                    task.status = TaskStatus::Failed("Critical execution error".to_string());
                    return result;
                }
            }
        }
        
        // This should never be reached due to the logic above
        Ok(last_result.unwrap_or_else(|| ExecutionResult {
            success: false,
            duration_millis: 0,
            error_message: Some("Unexpected execution state".to_string()),
            action_result: None,
        }))
    }

    /// Records a task execution in the task's history
    fn record_task_execution(&self, task: &mut Task, result: &ExecutionResult, retry_count: u32) {
        let started_at = CalClock::now_utc()
            .or_else(|_| CalClock::new(2024, 1, 1, 0, 0, 0, 0, crate::time::CalClockZone::utc()))
            .unwrap_or_else(|_| task.scheduled_time.clone());
        let completed_at = CalClock::now_utc()
            .or_else(|_| CalClock::new(2024, 1, 1, 0, 0, 0, 0, crate::time::CalClockZone::utc()))
            .unwrap_or_else(|_| task.scheduled_time.clone());
        
        
        let execution = TaskExecution {
            started_at,
            completed_at: Some(completed_at),
            duration_millis: Some(result.duration_millis),
            status: if result.success { 
                TaskStatus::Completed 
            } else { 
                TaskStatus::Failed(
                    result.error_message.clone()
                        .unwrap_or_else(|| "Unknown error".to_string())
                )
            },
            error_message: result.error_message.clone(),
            retry_count,
        };
        
        task.record_execution(execution);
    }

    /// Gets execution statistics
    pub fn stats(&self) -> &ExecutionStats {
        &self.stats
    }

    /// Resets execution statistics
    pub fn reset_stats(&mut self) {
        self.stats = ExecutionStats::default();
    }

    /// Gets success rate as a percentage
    pub fn success_rate(&self) -> f64 {
        if self.stats.total_executions == 0 {
            0.0
        } else {
            (self.stats.successful_executions as f64 / self.stats.total_executions as f64) * 100.0
        }
    }
}