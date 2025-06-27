/// Main scheduler implementation for managing and executing scheduled tasks with real-time processing

use oxedyne_fe2o3_core::prelude::*;
use crate::{
    schedule::{Task, TaskId, TaskExecutor},
    time::CalClock,
};
use std::{
    collections::{HashMap, BinaryHeap},
    cmp::Ordering,
    sync::{Arc, Mutex, mpsc},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

/// Configuration for the scheduler
#[derive(Debug, Clone)]
pub struct SchedulerConfig {
    /// Maximum number of concurrent task executions
    pub max_concurrent_tasks: usize,
    /// Check interval in milliseconds
    pub check_interval_millis: u64,
    /// Whether to continue running after task failures
    pub continue_on_failure: bool,
    /// Size of the task queue (0 for unlimited)
    pub queue_size: usize,
    /// Number of worker threads
    pub worker_threads: usize,
    /// Enable real-time background processing
    pub enable_background_processing: bool,
}

impl Default for SchedulerConfig {
    fn default() -> Self {
        SchedulerConfig {
            max_concurrent_tasks: 10,
            check_interval_millis: 1000, // 1 second
            continue_on_failure: true,
            queue_size: 1000,
            worker_threads: 4,
            enable_background_processing: true,
        }
    }
}

/// Scheduler statistics
#[derive(Debug, Clone, Default)]
pub struct SchedulerStats {
    /// Number of tasks currently scheduled
    pub scheduled_tasks: usize,
    /// Number of tasks currently running
    pub running_tasks: usize,
    /// Number of tasks in queue
    pub queued_tasks: usize,
    /// Total number of completed tasks
    pub completed_tasks: u64,
    /// Total number of failed tasks
    pub failed_tasks: u64,
    /// Average task execution time in milliseconds
    pub avg_execution_time_millis: u64,
    /// Current uptime in seconds
    pub uptime_seconds: u64,
}

/// Task queue entry with priority ordering
#[derive(Debug)]
struct QueuedTask {
    task: Task,
    queued_at: Instant,
    priority_score: u64,
}

impl PartialEq for QueuedTask {
    fn eq(&self, other: &Self) -> bool {
        self.priority_score == other.priority_score
    }
}

impl Eq for QueuedTask {}

impl PartialOrd for QueuedTask {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for QueuedTask {
    fn cmp(&self, other: &Self) -> Ordering {
        // Higher priority scores come first (reverse order for max-heap behaviour)
        other.priority_score.cmp(&self.priority_score)
            .then_with(|| self.queued_at.cmp(&other.queued_at)) // FIFO for same priority
    }
}

impl QueuedTask {
    fn new(task: Task) -> Self {
        let priority_score = Self::calculate_priority_score(&task);
        Self {
            task,
            queued_at: Instant::now(),
            priority_score,
        }
    }

    fn calculate_priority_score(task: &Task) -> u64 {
        let base_priority = match task.priority {
            crate::schedule::TaskPriority::Critical => 1000,
            crate::schedule::TaskPriority::High => 750,
            crate::schedule::TaskPriority::Normal => 500,
            crate::schedule::TaskPriority::Low => 250,
        };

        // Adjust for deadline urgency - use a safe fallback if clock operations fail
        let now = CalClock::now_utc()
            .or_else(|_| CalClock::new(2024, 1, 1, 0, 0, 0, 0, crate::time::CalClockZone::utc()))
            .unwrap_or_else(|_| {
                // Ultimate fallback - create a minimal clock
                task.scheduled_time.clone()
            });
            
        let time_until_scheduled = if task.scheduled_time >= now {
            let task_millis = task.scheduled_time.to_nanos_since_epoch().unwrap_or(0) / 1_000_000;
            let now_millis = now.to_nanos_since_epoch().unwrap_or(0) / 1_000_000;
            (task_millis - now_millis).max(0) as u64
        } else {
            // Overdue tasks get maximum urgency
            return base_priority + 10000;
        };

        // Closer deadlines get higher priority
        let urgency_bonus = if time_until_scheduled < 60000 { // < 1 minute
            500
        } else if time_until_scheduled < 300000 { // < 5 minutes
            300
        } else if time_until_scheduled < 3600000 { // < 1 hour
            100
        } else {
            0
        };

        base_priority + urgency_bonus
    }
}

/// Message types for background processing
#[derive(Debug)]
enum SchedulerMessage {
    Stop,
    #[allow(dead_code)]
    AddTask(Task),
    #[allow(dead_code)]
    RemoveTask(TaskId),
    #[allow(dead_code)]
    GetStats,
}

/// Main scheduler for managing scheduled tasks with real-time background processing
pub struct Scheduler {
    config: SchedulerConfig,
    tasks: HashMap<TaskId, Task>,
    task_queue: Arc<Mutex<BinaryHeap<QueuedTask>>>,
    stats: Arc<Mutex<SchedulerStats>>,
    executor: Arc<Mutex<TaskExecutor>>,
    worker_handles: Vec<JoinHandle<()>>,
    message_sender: Option<mpsc::Sender<SchedulerMessage>>,
    scheduler_handle: Option<JoinHandle<()>>,
    start_time: Instant,
    is_running: Arc<Mutex<bool>>,
}

impl std::fmt::Debug for Scheduler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Scheduler")
            .field("config", &self.config)
            .field("task_count", &self.tasks.len())
            .field("is_running", &self.is_running.lock().map(|guard| *guard).unwrap_or_else(|_| {
                // For Debug impl, provide a reasonable fallback for poisoned mutex
                false
            }))
            .finish()
    }
}

impl Scheduler {
    /// Creates a new scheduler with default configuration
    pub fn new() -> Self {
        Self::with_config(SchedulerConfig::default())
    }

    /// Creates a new scheduler with custom configuration
    pub fn with_config(config: SchedulerConfig) -> Self {
        Scheduler {
            config,
            tasks: HashMap::new(),
            task_queue: Arc::new(Mutex::new(BinaryHeap::new())),
            stats: Arc::new(Mutex::new(SchedulerStats::default())),
            executor: Arc::new(Mutex::new(TaskExecutor::new())),
            worker_handles: Vec::new(),
            message_sender: None,
            scheduler_handle: None,
            start_time: Instant::now(),
            is_running: Arc::new(Mutex::new(false)),
        }
    }

    /// Starts the real-time background processing system
    pub fn start(&mut self) -> Outcome<()> {
        if *lock_mutex!(self.is_running) {
            return Err(err!("Scheduler is already running"; Invalid, Input));
        }

        *lock_mutex!(self.is_running) = true;
        self.start_time = Instant::now();

        if self.config.enable_background_processing {
            // Start the main scheduler thread
            let (tx, rx) = mpsc::channel();
            self.message_sender = Some(tx);

            let queue = Arc::clone(&self.task_queue);
            let stats = Arc::clone(&self.stats);
            let executor = Arc::clone(&self.executor);
            let is_running = Arc::clone(&self.is_running);
            let config = self.config.clone();

            self.scheduler_handle = Some(thread::spawn(move || {
                Self::scheduler_loop(rx, queue, stats, executor, is_running, config);
            }));

            // Start worker threads
            for worker_id in 0..self.config.worker_threads {
                let queue = Arc::clone(&self.task_queue);
                let stats = Arc::clone(&self.stats);
                let executor = Arc::clone(&self.executor);
                let is_running = Arc::clone(&self.is_running);

                let handle = thread::spawn(move || {
                    Self::worker_loop(worker_id, queue, stats, executor, is_running);
                });

                self.worker_handles.push(handle);
            }
        }

        Ok(())
    }

    /// Stops the background processing system
    pub fn stop(&mut self) -> Outcome<()> {
        *lock_mutex!(self.is_running) = false;

        // Send stop message to scheduler thread
        if let Some(sender) = &self.message_sender {
            let _ = sender.send(SchedulerMessage::Stop);
        }

        // Wait for scheduler thread to finish
        if let Some(handle) = self.scheduler_handle.take() {
            let _ = handle.join();
        }

        // Wait for all worker threads to finish
        for handle in self.worker_handles.drain(..) {
            let _ = handle.join();
        }

        self.message_sender = None;
        Ok(())
    }

    /// Schedules a new task for execution
    pub fn schedule(&mut self, task: Task) -> Outcome<TaskId> {
        let task_id = task.id;
        
        // Add to local task registry
        self.tasks.insert(task_id, task.clone());
        
        // Add to processing queue if background processing is enabled
        if self.config.enable_background_processing && self.message_sender.is_some() {
            let queued_task = QueuedTask::new(task);
            {
                let mut queue = lock_mutex!(self.task_queue);
                if self.config.queue_size == 0 || queue.len() < self.config.queue_size {
                    queue.push(queued_task);
                } else {
                    return Err(err!("Task queue is full"; Invalid, Input));
                }
            }
        }

        // Update statistics
        {
            let mut stats = lock_mutex!(self.stats);
            stats.scheduled_tasks = self.tasks.len();
            stats.queued_tasks = lock_mutex!(self.task_queue).len();
        }

        Ok(task_id)
    }

    /// Removes a scheduled task
    pub fn unschedule(&mut self, task_id: TaskId) -> Outcome<()> {
        self.tasks.remove(&task_id);
        
        // Update statistics
        {
            let mut stats = lock_mutex!(self.stats);
            stats.scheduled_tasks = self.tasks.len();
        }

        Ok(())
    }

    /// Processes pending tasks (for manual execution when background processing is disabled)
    pub fn process_pending(&mut self) -> Outcome<()> {
        if self.config.enable_background_processing {
            return Err(err!("Cannot manually process when background processing is enabled"; Invalid, Input));
        }

        let now = res!(CalClock::now_utc());
        let mut tasks_to_execute = Vec::new();

        // Find tasks ready for execution
        for (task_id, task) in &self.tasks {
            if task.is_ready_to_execute(&now) {
                tasks_to_execute.push(*task_id);
            }
        }

        // Execute ready tasks
        let mut stats = lock_mutex!(self.stats);
        let mut executor = lock_mutex!(self.executor);

        for task_id in tasks_to_execute {
            if let Some(mut task) = self.tasks.remove(&task_id) {
                match executor.execute_with_retry(&mut task) {
                    Ok(result) => {
                        if result.success {
                            stats.completed_tasks += 1;
                        } else {
                            stats.failed_tasks += 1;
                        }

                        // Handle recurring tasks
                        if task.recurrence.is_some() {
                            if let Ok(()) = task.advance_to_next_execution() {
                                self.tasks.insert(task_id, task);
                            }
                        }
                    },
                    Err(e) => {
                        eprintln!("Task execution error: {}", e);
                        stats.failed_tasks += 1;
                    }
                }
            }
        }

        stats.scheduled_tasks = self.tasks.len();
        Ok(())
    }

    /// Main scheduler loop for background processing
    fn scheduler_loop(
        rx: mpsc::Receiver<SchedulerMessage>,
        queue: Arc<Mutex<BinaryHeap<QueuedTask>>>,
        stats: Arc<Mutex<SchedulerStats>>,
        _executor: Arc<Mutex<TaskExecutor>>,
        is_running: Arc<Mutex<bool>>,
        config: SchedulerConfig,
    ) {
        let check_interval = Duration::from_millis(config.check_interval_millis);
        
        while {
            match is_running.lock() {
                Ok(guard) => *guard,
                Err(_) => {
                    eprintln!("Scheduler thread: poisoned is_running mutex, stopping");
                    false
                }
            }
        } {
            // Process any incoming messages
            while let Ok(message) = rx.try_recv() {
                match message {
                    SchedulerMessage::Stop => {
                        *lock_mutex_thread!(is_running, "setting is_running to false") = false;
                        return;
                    },
                    SchedulerMessage::AddTask(task) => {
                        let queued_task = QueuedTask::new(task);
                        let mut queue = lock_mutex_thread!(queue, "scheduler thread queue access");
                        queue.push(queued_task);
                    },
                    SchedulerMessage::RemoveTask(_task_id) => {
                        // TODO: Implement task removal from queue
                    },
                    SchedulerMessage::GetStats => {
                        // TODO: Implement stats reporting
                    }
                }
            }

            // Update statistics
            {
                let mut stats = lock_mutex_thread!(stats, "stats update");
                stats.queued_tasks = lock_mutex_thread!(queue, "queue length check").len();
            }

            thread::sleep(check_interval);
        }
    }

    /// Worker thread loop for executing tasks
    fn worker_loop(
        _worker_id: usize,
        queue: Arc<Mutex<BinaryHeap<QueuedTask>>>,
        stats: Arc<Mutex<SchedulerStats>>,
        executor: Arc<Mutex<TaskExecutor>>,
        is_running: Arc<Mutex<bool>>,
    ) {
        while {
            match is_running.lock() {
                Ok(guard) => *guard,
                Err(_) => {
                    eprintln!("Scheduler thread: poisoned is_running mutex, stopping");
                    false
                }
            }
        } {
            // Try to get a task from the queue
            let task_opt = {
                let mut queue = lock_mutex_thread!(queue, "queue access");
                queue.pop()
            };

            if let Some(mut queued_task) = task_opt {
                let now = CalClock::now_utc()
                    .or_else(|_| CalClock::new(2024, 1, 1, 0, 0, 0, 0, crate::time::CalClockZone::utc()))
                    .unwrap_or_else(|_| queued_task.task.scheduled_time.clone());

                // Check if task is ready to execute
                if queued_task.task.is_ready_to_execute(&now) {
                    // Update running task count
                    {
                        let mut stats = lock_mutex_thread!(stats, "stats update");
                        stats.running_tasks += 1;
                    }

                    // Execute the task
                    let execution_result = {
                        let mut executor = lock_mutex_thread!(executor, "executor access");
                        executor.execute_with_retry(&mut queued_task.task)
                    };

                    // Update statistics
                    {
                        let mut stats = lock_mutex_thread!(stats, "stats update");
                        stats.running_tasks = stats.running_tasks.saturating_sub(1);

                        match execution_result {
                            Ok(result) => {
                                if result.success {
                                    stats.completed_tasks += 1;
                                } else {
                                    stats.failed_tasks += 1;
                                }
                                // Update average execution time
                                let total_tasks = stats.completed_tasks + stats.failed_tasks;
                                if total_tasks > 0 {
                                    stats.avg_execution_time_millis = 
                                        (stats.avg_execution_time_millis * (total_tasks - 1) + result.duration_millis) / total_tasks;
                                }
                            },
                            Err(_) => {
                                stats.failed_tasks += 1;
                            }
                        }
                    }

                    // Handle recurring tasks
                    if queued_task.task.recurrence.is_some() {
                        if let Ok(()) = queued_task.task.advance_to_next_execution() {
                            // Re-queue the task for next execution
                            let mut queue = lock_mutex_thread!(queue, "scheduler thread queue access");
                            queue.push(queued_task);
                        }
                    }
                } else {
                    // Task not ready yet, put it back in the queue
                    let mut queue = lock_mutex_thread!(queue, "queue access");
                    queue.push(queued_task);
                    drop(queue);
                    
                    // Sleep briefly to avoid busy waiting
                    thread::sleep(Duration::from_millis(100));
                }
            } else {
                // No tasks available, sleep briefly
                thread::sleep(Duration::from_millis(100));
            }
        }
    }

    /// Gets current scheduler statistics
    pub fn stats(&self) -> SchedulerStats {
        let mut stats = match self.stats.lock() {
            Ok(guard) => guard.clone(),
            Err(_) => {
                eprintln!("Warning: Stats mutex poisoned, returning default stats");
                return SchedulerStats::default();
            }
        };
        stats.uptime_seconds = self.start_time.elapsed().as_secs();
        stats.scheduled_tasks = self.tasks.len();
        stats
    }

    /// Gets a specific task by ID
    pub fn get_task(&self, task_id: TaskId) -> Option<&Task> {
        self.tasks.get(&task_id)
    }

    /// Lists all scheduled tasks
    pub fn list_tasks(&self) -> Vec<&Task> {
        self.tasks.values().collect()
    }

    /// Gets the current queue size
    pub fn queue_size(&self) -> usize {
        match self.task_queue.lock() {
            Ok(guard) => guard.len(),
            Err(_) => {
                eprintln!("Warning: Task queue mutex poisoned, returning 0");
                0
            }
        }
    }

    /// Checks if the scheduler is currently running
    pub fn is_running(&self) -> bool {
        match self.is_running.lock() {
            Ok(guard) => *guard,
            Err(_) => {
                eprintln!("Warning: Is_running mutex poisoned, returning false");
                false
            }
        }
    }
}

impl Drop for Scheduler {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}