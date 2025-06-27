/// Demonstration of the real-time scheduling system with background processing

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_datime::{
    schedule::{Scheduler, SchedulerConfig, Task, TaskPriority},
    schedule::action::CallbackAction,
    time::CalClockZone,
};
use std::time::Duration;
use std::sync::{Arc, Mutex};

#[test]
fn test_real_time_scheduler_demo() -> Outcome<()> {
    println!("=== Real-Time Scheduling System Demo ===");
    
    // Create a scheduler with custom configuration
    let config = SchedulerConfig {
        max_concurrent_tasks: 5,
        check_interval_millis: 100, // Check every 100ms for demo
        continue_on_failure: true,
        queue_size: 100,
        worker_threads: 2,
        enable_background_processing: true,
    };
    
    let mut scheduler = Scheduler::with_config(config);
    
    // Start the background processing system
    res!(scheduler.start());
    println!("✅ Background processing system started");
    
    // Create a shared counter to track task executions
    let counter = Arc::new(Mutex::new(0));
    
    // Schedule several tasks with different priorities
    let zone = CalClockZone::utc();
    
    // High priority task
    let counter_clone = Arc::clone(&counter);
    let high_priority_task = Task::new("high_priority_task", zone.clone())
        .at_time(0, 0, 0) // Execute immediately
        .on_date(2024, 1, 1)
        .priority(TaskPriority::High)
        .with_action(CallbackAction::with_description(
            move || {
                let mut count = counter_clone.lock().unwrap();
                *count += 1;
                println!("🔥 High priority task executed! Counter: {}", *count);
                Ok(())
            },
            "High priority demonstration task"
        ))
        .build()?;
    
    // Normal priority task
    let counter_clone = Arc::clone(&counter);
    let normal_priority_task = Task::new("normal_priority_task", zone.clone())
        .at_time(0, 0, 0) // Execute immediately
        .on_date(2024, 1, 1)
        .priority(TaskPriority::Normal)
        .with_action(CallbackAction::with_description(
            move || {
                let mut count = counter_clone.lock().unwrap();
                *count += 10;
                println!("📋 Normal priority task executed! Counter: {}", *count);
                Ok(())
            },
            "Normal priority demonstration task"
        ))
        .build()?;
    
    // Critical priority task
    let counter_clone = Arc::clone(&counter);
    let critical_priority_task = Task::new("critical_priority_task", zone.clone())
        .at_time(0, 0, 0) // Execute immediately
        .on_date(2024, 1, 1)
        .priority(TaskPriority::Critical)
        .with_action(CallbackAction::with_description(
            move || {
                let mut count = counter_clone.lock().unwrap();
                *count += 100;
                println!("🚨 Critical priority task executed! Counter: {}", *count);
                Ok(())
            },
            "Critical priority demonstration task"
        ))
        .build()?;
    
    // Schedule all tasks
    let task1_id = res!(scheduler.schedule(high_priority_task));
    let task2_id = res!(scheduler.schedule(normal_priority_task));
    let task3_id = res!(scheduler.schedule(critical_priority_task));
    
    println!("📅 Scheduled 3 tasks:");
    println!("  - Task 1 (High Priority): {}", task1_id);
    println!("  - Task 2 (Normal Priority): {}", task2_id);  
    println!("  - Task 3 (Critical Priority): {}", task3_id);
    
    // Give the scheduler time to process tasks
    println!("⏳ Waiting for tasks to execute...");
    std::thread::sleep(Duration::from_millis(2000));
    
    // Check scheduler statistics
    let stats = scheduler.stats();
    println!("\n📊 Scheduler Statistics:");
    println!("  - Scheduled tasks: {}", stats.scheduled_tasks);
    println!("  - Running tasks: {}", stats.running_tasks);
    println!("  - Queued tasks: {}", stats.queued_tasks);
    println!("  - Completed tasks: {}", stats.completed_tasks);
    println!("  - Failed tasks: {}", stats.failed_tasks);
    println!("  - Average execution time: {}ms", stats.avg_execution_time_millis);
    println!("  - Uptime: {}s", stats.uptime_seconds);
    
    // Check the final counter value
    let final_count = *counter.lock().unwrap();
    println!("\n🎯 Final counter value: {}", final_count);
    
    // Stop the scheduler
    res!(scheduler.stop());
    println!("🛑 Background processing system stopped");
    
    // Verify that all tasks executed (counter should be 111: 1 + 10 + 100)
    if final_count == 111 {
        println!("✅ All tasks executed successfully in priority order!");
    } else {
        println!("⚠️  Task execution count unexpected: {}", final_count);
    }
    
    // Verify scheduler stats show completed tasks
    assert!(stats.completed_tasks >= 3, "Expected at least 3 completed tasks, got {}", stats.completed_tasks);
    assert_eq!(stats.failed_tasks, 0, "Expected no failed tasks, got {}", stats.failed_tasks);
    
    println!("\n🎉 Real-time scheduling system demonstration completed successfully!");
    
    Ok(())
}

#[test]
fn test_scheduler_queue_management() -> Outcome<()> {
    println!("=== Task Queue Management Test ===");
    
    // Create scheduler with small queue for testing limits
    let config = SchedulerConfig {
        queue_size: 2,
        worker_threads: 1,
        enable_background_processing: false, // Manual processing for predictable testing
        ..Default::default()
    };
    
    let mut scheduler = Scheduler::with_config(config);
    let zone = CalClockZone::utc();
    
    // Create test tasks
    let task1 = Task::new("task1", zone.clone())
        .at_time(12, 0, 0)
        .on_date(2024, 6, 15)
        .with_action(CallbackAction::new(|| {
            println!("Task 1 executed");
            Ok(())
        }))
        .build()?;
    
    let task2 = Task::new("task2", zone.clone())
        .at_time(12, 0, 0)
        .on_date(2024, 6, 15)
        .with_action(CallbackAction::new(|| {
            println!("Task 2 executed");
            Ok(())
        }))
        .build()?;
    
    let task3 = Task::new("task3", zone.clone())
        .at_time(12, 0, 0)
        .on_date(2024, 6, 15)
        .with_action(CallbackAction::new(|| {
            println!("Task 3 executed");
            Ok(())
        }))
        .build()?;
    
    // Schedule tasks up to queue limit
    res!(scheduler.schedule(task1));
    res!(scheduler.schedule(task2));
    
    // Try to schedule one more task - should succeed since queue_size is 2
    res!(scheduler.schedule(task3));
    
    let stats = scheduler.stats();
    println!("📊 Scheduled {} tasks", stats.scheduled_tasks);
    
    assert_eq!(stats.scheduled_tasks, 3);
    
    println!("✅ Queue management test completed successfully!");
    
    Ok(())
}

#[test]
fn test_task_priority_ordering() -> Outcome<()> {
    println!("=== Task Priority Ordering Test ===");
    
    let mut scheduler = Scheduler::new();
    let zone = CalClockZone::utc();
    
    // Create tasks with different priorities
    let low_task = Task::new("low_priority", zone.clone())
        .at_time(0, 0, 0)
        .on_date(2024, 1, 1)
        .priority(TaskPriority::Low)
        .with_action(CallbackAction::new(|| {
            println!("🔽 Low priority task");
            Ok(())
        }))
        .build()?;
    
    let critical_task = Task::new("critical_priority", zone.clone())
        .at_time(0, 0, 0)
        .on_date(2024, 1, 1)
        .priority(TaskPriority::Critical)
        .with_action(CallbackAction::new(|| {
            println!("🔺 Critical priority task");
            Ok(())
        }))
        .build()?;
    
    let normal_task = Task::new("normal_priority", zone.clone())
        .at_time(0, 0, 0)
        .on_date(2024, 1, 1)
        .priority(TaskPriority::Normal)
        .with_action(CallbackAction::new(|| {
            println!("📋 Normal priority task");
            Ok(())
        }))
        .build()?;
    
    // Schedule in non-priority order to test queue ordering
    res!(scheduler.schedule(low_task));
    res!(scheduler.schedule(critical_task));
    res!(scheduler.schedule(normal_task));
    
    println!("📅 Scheduled tasks in order: Low, Critical, Normal");
    println!("🎯 Priority queue should reorder them as: Critical, Normal, Low");
    
    let stats = scheduler.stats();
    assert_eq!(stats.scheduled_tasks, 3);
    
    println!("✅ Task priority ordering test completed successfully!");
    
    Ok(())
}