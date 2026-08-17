//! Task definitions for the scheduling system.
//!
//! A task pairs an action with a time to run it, a priority, a retry policy
//! and, for recurring tasks, a pattern. Each run is recorded, and the recent
//! records are what the success rate and average duration are drawn from.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use oxedyne_fe2o3_core::prelude::*;
use crate::{
    time::{CalClock, CalClockZone},
    schedule::{RecurrencePattern, Action},
};
use std::{
    fmt,
    sync::atomic::{AtomicU64, Ordering},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TaskId(u64);

impl TaskId {
    fn next() -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(1);
        TaskId(COUNTER.fetch_add(1, Ordering::Relaxed))
    }
}

impl fmt::Display for TaskId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "task-{}", self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskStatus {
    Pending,
    Running,
    Completed,
    Failed(String),     // reason
    Cancelled,          // never started
    Skipped,            // its turn came and went
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum TaskPriority {
    Low = 1,
    Normal = 2,
    High = 3,
    Critical = 4,
}

impl Default for TaskPriority {
    fn default() -> Self {
        TaskPriority::Normal
    }
}

#[derive(Debug, Clone)]
pub struct TaskConfig {
    pub timeout_millis:         Option<u64>,
    pub max_retries:            u32,
    pub retry_delay_millis:     u64,
    pub continue_on_failure:    bool,   // keep scheduling after a failure
    pub allow_concurrent:       bool,   // more than one instance at a time
}

impl Default for TaskConfig {
    fn default() -> Self {
        TaskConfig {
            timeout_millis: Some(300_000), // 5 minutes default timeout
            max_retries: 3,
            retry_delay_millis: 1000, // 1 second delay
            continue_on_failure: true,
            allow_concurrent: false,
        }
    }
}

#[derive(Debug)]
pub struct Task {
    pub id:                 TaskId,
    pub name:               String,
    pub description:        Option<String>,
    pub zone:               CalClockZone,
    pub scheduled_time:     CalClock,
    pub recurrence:         Option<RecurrencePattern>,   // None means one-time
    pub priority:           TaskPriority,
    pub config:             TaskConfig,
    pub status:             TaskStatus,
    pub action:             Box<dyn Action>,
    pub execution_count:    u32,
    pub last_execution:     Option<CalClock>,
    pub next_execution:     Option<CalClock>,
    pub execution_history:  Vec<TaskExecution>,          // recent only
}

impl Clone for Task {
    fn clone(&self) -> Self {
        Task {
            id: self.id,
            name: self.name.clone(),
            description: self.description.clone(),
            zone: self.zone.clone(),
            scheduled_time: self.scheduled_time.clone(),
            recurrence: self.recurrence.clone(),
            priority: self.priority,
            config: self.config.clone(),
            status: self.status.clone(),
            action: self.action.box_clone(),
            execution_count: self.execution_count,
            last_execution: self.last_execution.clone(),
            next_execution: self.next_execution.clone(),
            execution_history: self.execution_history.clone(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct TaskExecution {
    pub started_at:     CalClock,
    pub completed_at:   Option<CalClock>,   // None while still running
    pub duration_millis: Option<u64>,
    pub status:         TaskStatus,
    pub error_message:  Option<String>,
    pub retry_count:    u32,
}

impl Task {
    pub fn new<S: Into<String>>(name: S, zone: CalClockZone) -> TaskBuilder {
        TaskBuilder::new(name.into(), zone)
    }

    pub fn is_ready_to_execute(&self, current_time: &CalClock) -> bool {
        match self.status {
            TaskStatus::Pending => current_time >= &self.scheduled_time,
            _ => false,
        }
    }

    pub fn calculate_next_execution(&self) -> Outcome<Option<CalClock>> {
        if let Some(ref pattern) = self.recurrence {
            pattern.next_execution(&self.scheduled_time, &self.zone)
        } else {
            Ok(None)
        }
    }

    pub fn advance_to_next_execution(&mut self) -> Outcome<()> {
        if let Some(next_time) = res!(self.calculate_next_execution()) {
            self.scheduled_time = next_time;
            self.next_execution = res!(self.calculate_next_execution());
            self.status = TaskStatus::Pending;
        }
        Ok(())
    }

    pub fn record_execution(&mut self, execution: TaskExecution) {
        const MAX_HISTORY: usize = 10; // Keep last 10 executions
        
        self.execution_count += 1;
        self.last_execution = Some(execution.started_at.clone());
        self.execution_history.push(execution);
        
        // Keep only recent executions
        if self.execution_history.len() > MAX_HISTORY {
            self.execution_history.drain(0..self.execution_history.len() - MAX_HISTORY);
        }
    }

    /// Over the retained history only.
    pub fn average_execution_duration(&self) -> Option<u64> {
        let completed: Vec<_> = self.execution_history.iter()
            .filter_map(|exec| exec.duration_millis)
            .collect();
        
        if completed.is_empty() {
            None
        } else {
            Some(completed.iter().sum::<u64>() / completed.len() as u64)
        }
    }

    /// A percentage, over the retained history only.
    pub fn success_rate(&self) -> f64 {
        if self.execution_history.is_empty() {
            0.0
        } else {
            let successful = self.execution_history.iter()
                .filter(|exec| matches!(exec.status, TaskStatus::Completed))
                .count();
            (successful as f64 / self.execution_history.len() as f64) * 100.0
        }
    }
}

pub struct TaskBuilder {
    name: String,
    description: Option<String>,
    zone: CalClockZone,
    scheduled_time: Option<CalClock>,
    recurrence: Option<RecurrencePattern>,
    priority: TaskPriority,
    config: TaskConfig,
    action: Option<Box<dyn Action>>,
}

impl TaskBuilder {
    pub fn new(name: String, zone: CalClockZone) -> Self {
        TaskBuilder {
            name,
            description: None,
            zone,
            scheduled_time: None,
            recurrence: None,
            priority: TaskPriority::default(),
            config: TaskConfig::default(),
            action: None,
        }
    }

    pub fn description<S: Into<String>>(mut self, desc: S) -> Self {
        self.description = Some(desc.into());
        self
    }

    pub fn at_time(mut self, hour: u8, minute: u8, second: u8) -> Self {
        if let Ok(time) = CalClock::new(2024, 1, 1, hour, minute, second, 0, self.zone.clone()) {
            self.scheduled_time = Some(time);
        }
        self
    }

    pub fn on_date(mut self, year: i32, month: u8, day: u8) -> Self {
        if let Some(existing_time) = self.scheduled_time.take() {
            if let Ok(time) = CalClock::new(
                year, month, day,
                existing_time.hour(),
                existing_time.minute(),
                existing_time.second(),
                existing_time.nanosecond(),
                self.zone.clone()
            ) {
                self.scheduled_time = Some(time);
            }
        } else if let Ok(time) = CalClock::new(year, month, day, 0, 0, 0, 0, self.zone.clone()) {
            self.scheduled_time = Some(time);
        }
        self
    }

    pub fn at(mut self, time: CalClock) -> Self {
        self.scheduled_time = Some(time);
        self
    }

    pub fn recurrence(mut self, pattern: RecurrencePattern) -> Self {
        self.recurrence = Some(pattern);
        self
    }

    pub fn priority(mut self, priority: TaskPriority) -> Self {
        self.priority = priority;
        self
    }

    pub fn config(mut self, config: TaskConfig) -> Self {
        self.config = config;
        self
    }

    pub fn with_action<A: Action + 'static>(mut self, action: A) -> Self {
        self.action = Some(Box::new(action));
        self
    }

    pub fn build(self) -> Outcome<Task> {
        let scheduled_time = res!(self.scheduled_time
            .ok_or_else(|| err!("Task scheduled time is required"; Invalid, Input)));
        
        let action = res!(self.action
            .ok_or_else(|| err!("Task action is required"; Invalid, Input)));

        let next_execution = if self.recurrence.is_some() {
            // For recurring tasks, calculate the first next execution
            if let Some(ref pattern) = self.recurrence {
                res!(pattern.next_execution(&scheduled_time, &self.zone))
            } else {
                None
            }
        } else {
            None
        };

        Ok(Task {
            id: TaskId::next(),
            name: self.name,
            description: self.description,
            zone: self.zone,
            scheduled_time,
            recurrence: self.recurrence,
            priority: self.priority,
            config: self.config,
            status: TaskStatus::Pending,
            action,
            execution_count: 0,
            last_execution: None,
            next_execution,
            execution_history: Vec::new(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schedule::action::CallbackAction;

    #[test]
    fn test_task_builder() {
        let zone = CalClockZone::utc();
        let callback = CallbackAction::new(|| {
            println!("Test action executed");
            Ok(())
        });

        let task = Task::new("test_task", zone.clone())
            .description("A test task")
            .at_time(14, 30, 0)
            .on_date(2024, 6, 15)
            .priority(TaskPriority::High)
            .with_action(callback)
            .build()
            .expect("Failed to build task");

        assert_eq!(task.name, "test_task");
        assert_eq!(task.priority, TaskPriority::High);
        assert_eq!(task.status, TaskStatus::Pending);
        assert_eq!(task.scheduled_time.year(), 2024);
        assert_eq!(task.scheduled_time.month(), 6);
        assert_eq!(task.scheduled_time.day(), 15);
        assert_eq!(task.scheduled_time.hour(), 14);
        assert_eq!(task.scheduled_time.minute(), 30);
    }

    #[test]
    fn test_task_id_uniqueness() {
        let id1 = TaskId::next();
        let id2 = TaskId::next();
        assert_ne!(id1, id2);
    }

    #[test]
    fn test_task_execution_recording() {
        let zone = CalClockZone::utc();
        let callback = CallbackAction::new(|| Ok(()));
        
        let mut task = Task::new("test", zone.clone())
            .at_time(12, 0, 0)
            .on_date(2024, 1, 1)
            .with_action(callback)
            .build()
            .unwrap();

        let execution = TaskExecution {
            started_at: CalClock::new(2024, 1, 1, 12, 0, 0, 0, zone.clone()).unwrap(),
            completed_at: Some(CalClock::new(2024, 1, 1, 12, 0, 5, 0, zone).unwrap()),
            duration_millis: Some(5000),
            status: TaskStatus::Completed,
            error_message: None,
            retry_count: 0,
        };

        task.record_execution(execution);
        
        assert_eq!(task.execution_count, 1);
        assert_eq!(task.execution_history.len(), 1);
        assert_eq!(task.success_rate(), 100.0);
        assert_eq!(task.average_execution_duration(), Some(5000));
    }
}