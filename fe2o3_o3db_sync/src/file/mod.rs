//! The persistent, on-disk side of the database.
//!
//! Values are appended to live data files ([`live`]) whose locations are
//! recorded in file-location records ([`floc`]); [`stored`] and [`state`]
//! track archived files and their liveness for garbage collection; [`zdir`]
//! manages zone directories; [`fcache`] caches open file handles; and [`core`]
//! holds the shared file-type and discovery helpers.

pub mod core;
pub mod fcache;
pub mod floc;
pub mod live;
pub mod state;
pub mod stored;
pub mod zdir;
