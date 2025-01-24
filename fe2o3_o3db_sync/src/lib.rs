//! A log-structured key-value database with robust error handling and high throughput.
//! 
//! # Overview
//! 
//! O3db (Ozone Database) is a log-structured key-value store inspired by BitCask. It provides:
//! 
//! - Fast writes through log-structured append operations
//! - High parallelism using operating system threads
//! - Automatic garbage collection
//! - Configurable caching
//! - Comprehensive error handling
//! 
//! # Examples
//! 
//! Basic usage with error handling:
//! 
//! ```
//! use oxedize_fe2o3_core::prelude::*;
//! use oxedize_fe2o3_o3db_sync::prelude::*;
//! 
//! fn store_value() -> Outcome<()> {
//!     // Configure and start the database
//!     let mut db = res!(O3db::new(
//!         "my_db",
//!         None, // Use default config
//!         RestSchemesInput::default(),
//!         Uid::default(),
//!     ));
//!     res!(db.start());
//!     
//!     // Store a value
//!     let resp = res!(db.api().store(
//!         dat!("my_key"), 
//!         dat!(42),
//!         user_id,
//!     ));
//!     
//!     // Handle potential errors during storage
//!     match res!(resp.recv_timeout(constant::USER_REQUEST_TIMEOUT)) {
//!         OzoneMsg::Error(e) => return Err(err!(e, "Failed to store value"; IO, Data, Write)),
//!         OzoneMsg::Ok => Ok(()),
//!         msg => Err(err!("Unexpected response: {:?}", msg; Channel, Unexpected)),
//!     }
//! }
//! ```
//! 
//! Fetching values with chunking support:
//! 
//! ```
//! fn fetch_large_value() -> Outcome<Vec<u8>> {
//!     let resp = res!(db.api().fetch_using_schemes(
//!         &dat!("large_key"),
//!         None, // Use default schemes
//!     ));
//!     
//!     match res!(resp.recv_daticle(db.api().schemes().encrypter(), None)) {
//!         (None, _) => Err(err!("Value not found"; Data, Missing)),
//!         (Some((Dat::Tup5u64(tup), _)), _) => {
//!             // Handle chunked data
//!             res!(db.api().fetch_chunks(&Dat::Tup5u64(tup), None))
//!         },
//!         (Some((dat, _)), _) => Ok(res!(dat.as_bytes())),
//!     }
//! }
//! ```
//! 
//! Garbage collection:
//! 
//! ```
//! fn manage_gc() -> Outcome<()> {
//!     // Enable garbage collection
//!     res!(db.api().activate_gc(true));
//!     
//!     // Wait for GC to complete
//!     thread::sleep(Duration::from_secs(1));
//!     
//!     // Verify file states
//!     res!(db.api().dump_file_states(constant::USER_REQUEST_WAIT));
//!     
//!     Ok(())
//! }
//! ```
//! 
//! # Error Handling
//! 
//! The database uses the `Outcome<T>` type for error handling, which provides:
//! 
//! - Error tags for categorisation
//! - Error chaining for context
//! - Panic catching through the `res!` macro
//! - Detailed error messages
//! 
//! All errors include file and line information for debugging.
//! 
//! # Implementation
//!
//! This Rust implementation uses operating system threads.
//!
//! ## Persistent file side
//! 
//! Values are appended to live data files, and their location is appended to live index files which
//! speed up initialisation.  Upon reaching their maximum allowed size, data files are closed
//! (archived) and a new file in the sequence is created.  Garbage collection works in the background
//! to remove old values from archive files, some of which may ultimately be completely deleted.  Files
//! can be allocated across multiple zone directories.
//! 
//! ## Volatile memory side
//! 
//! Each zone is associated with a cache map that contains file locations for all values, but also
//! retains the values themselves when they are retrieved while the cache stays within the specified
//! size limit.  Caches can be configured to have the same fixed number of readers and a differing
//! fixed number of writers.  Caches are initialised by reading zone index files.  Each writer controls
//! precisely one live file.
//!
//! # Initial Roadmap
//! 
//! 1. [✘] Reliable persistent data storage and retrieval.
//!     1.1 [✔] Robust cache initialisation.
//!     1.2 [✔] Server.
//!     1.3 [✘] Recaching.
//!     1.4 [✘] Rezoning.
//! 2. [✘] Multiple users.
//!     2.1. [✔] Timestamps -> Metadata including user identification.
//!     2.2. [✔] Encryption.
//!     2.3. [✔] Digital signatures.
//!     2.4. [✔] Multiple user login.
//!     2.5. [✘] Recording user access.
//!     2.6. [✘] User access control.
//! 3. [✔] Reliable resource management.
//!     3.1. [✔] Cache size reporting.
//!     3.2. [✔] Zone directory size reporting.
//!     3.3. [✔] Cache size limitation with automated jettison of oldest cached values.
//!     3.4. [✔] File garbage collection.
//! 4. [✘] Documentation.
//!     4.1. [✘] Documentation of intentions and architecture with diagrams.
//!     4.2. [✘] Source code thoroughly and extensively documented.
//! 5. [✘] Testing.
//!     5.1. [✔] Basic integration tests.
//!     5.2. [✘] Extensive scenario tests.
//! 6. [✘] Performance.
//!     6.1. [✔] Basic performance measurement.
//!     6.2. [✘] Extensive performance measurement varying all relevant configuration parameters.
//!     6.3. [✘] Peer benchmarks.
//!
#![forbid(unsafe_code)]
#![allow(dead_code)]
pub mod api;
pub mod base;
pub mod bots;
pub mod comm;
pub mod dal; // Data Abstraction Layer.
pub mod data;
pub mod file;
pub mod test;

pub mod db;
pub mod prelude;

pub use crate::db::O3db;
