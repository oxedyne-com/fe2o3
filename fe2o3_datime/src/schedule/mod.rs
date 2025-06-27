/// Scheduling and action management system for fe2o3_datime
/// 
/// This module provides comprehensive functionality for scheduling tasks,
/// managing recurring events, and executing time-based actions.
/// 
/// # Features
/// 
/// - **Task Scheduling**: One-time and recurring task scheduling
/// - **Cron-style Expressions**: Support for cron-like scheduling syntax
/// - **Action Management**: Execute callbacks and actions at scheduled times
/// - **Calendar Integration**: Schedule events relative to business days and holidays
/// - **Time Zone Support**: Multi-timezone scheduling with automatic conversions
/// 
/// # Examples
/// 
/// ```ignore
/// use oxedyne_fe2o3_datime::schedule::{Scheduler, Task, RecurrencePattern};
/// use oxedyne_fe2o3_datime::time::CalClockZone;
/// 
/// let mut scheduler = Scheduler::new();
/// let zone = CalClockZone::utc();
/// 
/// // Schedule a one-time task
/// let task = Task::new("backup", zone.clone())
///     .at_time(14, 30, 0) // 2:30 PM
///     .on_date(2024, 6, 15)
///     .with_action(|| println!("Running backup"));
/// 
/// scheduler.schedule(task)?;
/// 
/// // Schedule a recurring daily task
/// let daily_task = Task::new("daily_report", zone)
///     .at_time(9, 0, 0) // 9:00 AM
///     .recurrence(RecurrencePattern::Daily)
///     .with_action(|| println!("Generating daily report"));
/// 
/// scheduler.schedule(daily_task)?;
/// 
/// // Process pending tasks
/// scheduler.process_pending()?;
/// ```

pub mod scheduler;
pub mod task;
pub mod recurrence;
pub mod action;
pub mod executor;

pub use self::{
    scheduler::{Scheduler, SchedulerConfig, SchedulerStats},
    task::{Task, TaskId, TaskStatus, TaskPriority, TaskBuilder, TaskExecution, TaskConfig},
    recurrence::{RecurrencePattern, CronExpression, RecurrenceRule},
    action::{Action, ActionResult, ActionContext, ActionError},
    executor::{TaskExecutor, ExecutionResult, ExecutionStats},
};