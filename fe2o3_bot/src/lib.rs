//! Thread worker abstraction library for building concurrent applications.
//! 
//! This crate provides the [`Bot`] trait and supporting types for implementing worker threads
//! (called "bots") with consistent error handling, communication, and termination semantics.
//! 
//! # Key Features
//! 
//! - Identity and labeling via the [`NumIdDat`] type
//! - Message passing through typed channels using [`BotMsg`]
//! - Error counting with configurable warning thresholds
//! - Graceful shutdown via [`Sentinel`]s
//! - Common patterns for handling communication errors
//! 
//! # Example
//! 
//! ```rust,no_run
//! use oxedize_fe2o3_bot::{Bot, BotMsg};
//! use oxedize_fe2o3_core::prelude::*;
//! use oxedize_fe2o3_jdat::id::NumIdDat;
//! 
//! // Define bot message type
//! #[derive(Clone, Debug)]
//! enum WorkerMsg {
//!     Process(String),
//!     Shutdown,
//! }
//! 
//! impl BotMsg<ErrTag> for WorkerMsg {}
//! 
//! // Implement bot with u8 ID
//! struct Worker {
//!     id: u8,
//!     // ... other fields
//! }
//! 
//! impl Bot<1, u8, WorkerMsg> for Worker {
//!     fn id(&self) -> u8 { self.id }
//!     // ... implement other required methods
//! }
//! ```
//! 
//! The crate avoids use of `unsafe` code and provides comprehensive error handling patterns.
//!
#![forbid(unsafe_code)]
#![allow(dead_code)]

pub mod bot;
pub mod handles;
//pub mod id;
pub mod msg;

pub use bot::Bot;
