//! Application layer providing high-level interfaces to the Shield protocol.
//!
//! These modules wrap the server protocol in user-facing tools: a configurable
//! server, an interactive REPL, a text user interface and the syntax used to
//! parse commands and configuration.
pub mod cfg;
pub mod constant;
pub mod repl;
pub mod server;
pub mod syntax;
pub mod tui;
