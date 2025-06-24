//! Task and thread-local logging stream identifiers for async and sync contexts.
//! 
//! This module provides facilities for maintaining logging stream identifiers across both
//! synchronous threads and asynchronous tasks through two separate modules:
//! 
//! - `sync_log`: Thread-local storage system for synchronous code using `std::thread`
//! - `async_log`: Task-local storage system for asynchronous code using `tokio` tasks
//! 
//! # Synchronous Usage
//! 
//! The sync version uses thread-local storage that is unique to each operating system thread:
//! 
//! ```rust
//! use oxedyne_fe2o3_core::prelude::*;
//! 
//! // Set the stream ID for the current thread.
//! sync_log::set_stream(fmt!("my_thread");
//! 
//! // Get the current thread's stream ID.
//! let stream = sync_log::stream();
//! ```
//! 
//! # Asynchronous Usage
//! 
//! The async version uses Tokio's task-local storage. Stream IDs are set by wrapping
//! an async task in a scope:
//! 
//! ```rust
//! use oxedyne_fe2o3_core::prelude::*;
//! 
//! let handle = tokio::spawn(async_log::LOG_STREAM_ID.scope(
//!     fmt!("my_stream"),
//!     async move {
//!         // This task will have access to the stream ID
//!         let stream = async_log::stream();
//!         // ... task code ...
//!     }
//! ));
//! ```
//! 
//! # Default Values
//! 
//! Both systems return "main" as the default stream ID if the storage cannot be accessed.
//! 
//! # Thread/Task Safety
//! 
//! - `sync_log`: Thread-safe with each OS thread getting its own isolated instance 
//! - `async_log`: Task-safe with stream ID scoped to the spawned task
//! 
//! # Implementation Notes
//! 
//! - The sync version uses mutable thread-local storage that can be updated at any time
//! - The async version uses Tokio's task-local system requiring task scoping to set values
//! - The two systems are completely independent

pub mod sync_log {

    // ┌───────────────────────┐
    // │ Sync logging.         │
    // └───────────────────────┘
    
    use std::cell::RefCell;


    thread_local! {
        static LOG_STREAM_ID: RefCell<String> = RefCell::new(String::from("main"));
    }
    
    pub fn stream() -> String {
        LOG_STREAM_ID.with(|s| s.borrow().clone())
    }
    
    pub fn set_stream(id: String) {
        LOG_STREAM_ID.with(|s| *s.borrow_mut() = id);
    }
}


pub mod async_log {

    // ┌───────────────────────┐
    // │ Async logging.        │
    // └───────────────────────┘
    
    tokio::task_local! {
        pub static LOG_STREAM_ID: String;
    }
    
    pub fn stream() -> String {
        LOG_STREAM_ID.try_with(|s| s.clone()).unwrap_or(String::from("main"))
    }
}
