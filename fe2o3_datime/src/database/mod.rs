/// Database integration and storage format methods for datetime types.
///
/// This module provides comprehensive database storage and retrieval methods
/// for CalClock, ClockTime, CalendarDate, and related datetime types.
/// Multiple storage formats are supported to optimise for different use cases:
///
/// - **Binary format**: Most efficient for time-based queries and storage
/// - **ISO 8601 strings**: Human-readable and standard-compliant
/// - **Component format**: Optimal for partial queries and filtering
/// - **Unix timestamps**: Compatible with existing systems
///
/// # Storage Strategies
///
/// ## CalClock Storage
/// - Primary: Nanosecond timestamp + timezone ID
/// - Secondary: ISO 8601 string for readability
/// - Indexed: Year/month/day components for calendar queries
///
/// ## ClockTime Storage  
/// - Primary: Nanoseconds since midnight + timezone
/// - Secondary: Hour/minute/second components
///
/// ## CalendarDate Storage
/// - Primary: Julian day number + timezone
/// - Secondary: Year/month/day components
///
/// # Examples
///
/// ```ignore
/// use oxedyne_fe2o3_datime::{
///     time::{CalClock, CalClockZone},
///     database::{DatabaseRecord, StorageFormat},
/// };
///
/// let zone = CalClockZone::utc();
/// let calclock = CalClock::new(2024, 6, 15, 14, 30, 0, 0, zone)?;
///
/// // Convert to database record
/// let record = calclock.to_database_record(StorageFormat::Binary)?;
/// 
/// // Restore from database
/// let restored = CalClock::from_database_record(&record)?;
/// assert_eq!(calclock, restored);
/// ```

pub mod storage;
pub mod formats;
pub mod indexes;

pub use storage::*;
pub use formats::*;
pub use indexes::*;