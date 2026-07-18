//! Guard system providing layered denial-of-service protection.
//!
//! The guards track behaviour by network address and by user, progressing
//! through a Monitor to Throttle to Blacklist state machine, and share the
//! common accounting data structures defined here.
pub mod addr;
pub mod data;
// `UserGuard` now lives generically in `fe2o3_net::guard::user`, beside `AddressGuard`,
// so any protocol can share the per-user trust map. Re-exported here so the existing
// `crate::srv::guard::user::…` paths keep resolving.
pub use oxedyne_fe2o3_net::guard::user;
