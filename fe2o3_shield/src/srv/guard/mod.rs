//! Guard system providing layered denial-of-service protection.
//!
//! The guards track behaviour by network address and by user, progressing
//! through a Monitor to Throttle to Blacklist state machine, and share the
//! common accounting data structures defined here.
pub mod addr;
pub mod data;
pub mod user;
