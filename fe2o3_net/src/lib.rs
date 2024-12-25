//! A networking library providing foundational components for building network applications.
//! 
//! This crate implements core networking protocols and utilities, with a focus on robust
//! error handling and type safety. It provides strongly-typed abstractions for common
//! networking concepts and protocols.
//!
//! # Key Features
//!
//! ## HTTP/HTTPS Protocol
//! - Complete header field handling with strongly typed values
//! - Status code management with descriptive messages
//! - Content type system supporting common web formats
//! - Request and response message parsing
//! - Cookie and session handling
//! - Support for HTTP/1.1, HTTP/2 and HTTP/3
//!
//! ## WebSocket Protocol
//! - Secure handshake implementation
//! - Binary and text message support
//! - Frame-level control with customisable chunk sizes
//! - Ping/pong heartbeat mechanism
//! - Connection upgrade handling
//! - Built-in latency tracking
//!
//! ## SMTP Email
//! - Message composition and parsing
//! - Command implementation (HELO, MAIL FROM, RCPT TO, etc.)
//! - Header field processing
//! - Multi-part content support
//! - Response code handling
//!
//! ## DNS and Addressing
//! - FQDN (Fully Qualified Domain Name) validation
//! - Email address parsing and validation
//! - Phone number handling with country codes
//! - Generic contact address abstraction
//!
//! ## Content Management
//! - Comprehensive media type system
//! - Character set handling for major encodings
//! - Content disposition control
//! - File type detection
//!
//! # Example
//!
//! ```rust
//! use oxedize_fe2o3_net::{
//!     dns::Fqdn,
//!     http::{
//!         msg::HttpMessage,
//!         status::HttpStatus,
//!     },
//! };
//!
//! // Create a simple HTTP response
//! let response = HttpMessage::ok_respond_with_text("Hello, world!");
//!
//! // Validate a domain name
//! let domain = res!(Fqdn::new("example.com"));
//! ```
//!
//! # Error Handling
//!
//! The crate uses the Hematite error handling system with tagged errors for precise
//! error identification and chaining. All operations return an `Outcome<T>` which
//! provides context and categorisation of errors.
//!
//! # Async Support
//!
//! Network operations are implemented using Tokio for asynchronous I/O. The crate
//! provides both synchronous and asynchronous interfaces where appropriate.
//!
//! # Safety
//!
//! This crate forbids unsafe code and avoids unwrap operations, preferring explicit
//! error handling through the `Outcome` type.
//!
#![forbid(unsafe_code)]
pub mod addr;
pub mod conc;
pub mod charset;
pub mod constant;
pub mod dns;
pub mod email;
pub mod file;
pub mod http;
pub mod id;
pub mod media;
pub mod smtp;
pub mod time;
pub mod ws;

pub use ws::core::WebSocket;
