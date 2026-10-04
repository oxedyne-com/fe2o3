//! Host-statistics primitives for the Hematite ecosystem.
//!
//! Reads CPU, memory, load average, disk, network and process
//! metrics from the operating system without third-party
//! dependencies. Every sample is captured by parsing the
//! relevant pseudo-file under `/proc` on Linux; support for
//! BSD and macOS will slot in via the same [`Sampler`] trait.
//!
//! The optional `inotify` feature adds a watcher for directory trees on Linux, built on
//! `nix`, and the optional `clock` feature the boot clock and a check for the machine
//! having slept; without them the crate has no third-party dependency at all.
//!
//! Intended consumers include admin dashboards, long-running
//! service processes that want to record their own resource
//! use, and test harnesses that need to confirm a workload's
//! footprint.
//!
//! ```ignore
//! use oxedyne_fe2o3_sys::snapshot::Snapshot;
//! let snap = Snapshot::sample()?;
//! println!("cpu busy {}%", snap.cpu.busy_percent());
//! ```
#![forbid(unsafe_code)]

#[cfg(all(target_os = "linux", feature = "clock"))]
pub mod clock;
pub mod cpu;
pub mod disk;
#[cfg(all(target_os = "linux", feature = "inotify"))]
pub mod inotify;
pub mod load;
pub mod mem;
pub mod net;
pub mod parse;
pub mod proc_self;
pub mod resident;
pub mod snapshot;
pub mod uptime;

/// Root under which every Linux metric source lives.
pub const PROC_ROOT: &str = "/proc";
