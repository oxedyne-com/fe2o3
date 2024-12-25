//! A geometry library providing (currently) types and utilities for 2D layout and positioning.
//! 
//! This crate focuses on rectangular layouts with absolute and relative positioning support,
//! making it suitable for UI layout systems. Key features include:
//! 
//! - Dimension types with saturating arithmetic for safe calculations
//! - Coordinate system with zero-based positioning
//! - Rectangle types supporting both absolute and relative positioning
//! - Flexible dimension system for fluid layouts
//! - Clipping and intersection support
//! - Position enums for common alignment scenarios (top-left, centre, etc.)
//! 
//! The crate is particularly useful for:
//! - Terminal user interfaces
//! - Widget layout systems  
//! - Window management
//! - Any application requiring 2D layout calculation
//!
#![forbid(unsafe_code)]
pub mod dim;
pub mod rect;
