//! Embedded front-end assets for the admin dashboard.
//!
//! HTML, CSS, JavaScript and image assets (including the Hematite
//! logo) are compiled into the `steel` binary via `include_str!` and
//! `include_bytes!`. The dashboard ships with the binary -- there is
//! no separate asset directory to sync to a production host -- and
//! the assets always match the binary they were built with.
//!
//! The visual style mirrors the Hematite documentation at
//! `$HOME/usr/complement/projects/oxedyne/projects/fe2o3/doc/Hematite`:
//!
//! - Primary red: `rgb(243, 60, 87)` for accents and active links.
//! - Primary blue: `rgb(171, 202, 222)` for code block backgrounds.
//! - Light grey (luma 240) for section banners and neutral surfaces.
//! - Libertinus Serif for headings; monospace for code and keys.
//!
//! Filled in by task #6.
