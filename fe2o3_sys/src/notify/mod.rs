//! Hearing writes on whole filesystems, placing them on host paths, and following the files
//! written across renames: [`fan`], [`mounts`] and [`follow`].
//!
//! No FID records are used, so no file handles, so no `unsafe`. The sequences were proven on the
//! kernel these run on by the BR-4e S1 probes (2 Nov 2026).

pub mod fan;
pub mod follow;
pub mod mounts;
