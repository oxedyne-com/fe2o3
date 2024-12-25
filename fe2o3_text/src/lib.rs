//! Text processing utilities for the Hematite ecosystem.
//!
//! This crate provides tools for manipulating and analysing text in various ways. Key features include:
//!
//! - `Stringer` - A String wrapper that adds functionality like intelligent quote handling, indentation
//!   control, line wrapping and character insertion at specified intervals.
//!
//! - SACSS (Simple And Composite String Search) - An alternative to regex that aims to be more approachable
//!   through composable boolean operations on simple pattern matches like "starts with", "contains" etc.
//!
//! - Thread-safe text containers supporting concurrent access with highlighting capabilities
//!
//! - Base-2^x encodings with customisable alphabets for binary-to-text conversion
//!
//! - Text splitting with quote protection and hyphenation awareness
//!
//! - Line-oriented text manipulation with full Unicode support
//!
//! The implementation focuses on providing intuitive text processing tools while maintaining strong safety
//! guarantees. All functionality is implemented without unsafe code.
//!
#![forbid(unsafe_code)]
#![allow(dead_code)]
pub mod access;
pub mod base2x;
pub mod core;
pub mod highlight;
pub mod lines;
pub mod pattern;
pub mod split;
pub mod string;
pub mod phrase;

pub use core::Text;
