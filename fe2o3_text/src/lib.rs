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
//! - `regex` - A small backtracking regular-expression engine for the times when the pattern is
//!   already written in that language, carrying a step budget so a pathological pattern reports
//!   that it gave up rather than reporting no match
//!
//! - `glob` - Shell-style path globbing, `**` included, for selecting files by name
//!
//! - Thread-safe text containers supporting concurrent access with highlighting capabilities
//!
//! - Base-2^x encodings with customisable alphabets for binary-to-text conversion, alongside
//!   standard RFC 4648 Base64 for talking to everybody else
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
pub mod base64;
pub mod core;
pub mod glob;
pub mod highlight;
pub mod html;
pub mod lines;
pub mod doc;
pub mod pattern;
pub mod regex;
pub mod secret;
pub mod split;
pub mod string;
pub mod phrase;
pub mod table;

pub mod fmt;
pub mod unicode;
pub mod xml;

pub use core::Text;
