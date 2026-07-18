//! The per-zone worker bot pools that carry out storage work.
//!
//! Each zone runs pools of cache bots ([`bot_cache`]), file bots
//! ([`bot_file`]), reader bots ([`bot_reader`]), writer bots ([`bot_writer`])
//! and init/garbage-collection bots ([`bot_initgc`]). [`bot`] defines the
//! shared worker behaviour and [`WorkerType`](bot::WorkerType), and
//! [`worker_deps`] bundles their start-up dependencies.

pub mod bot;
pub mod bot_cache;
pub mod bot_file;
pub mod bot_initgc;
pub mod bot_reader;
pub mod bot_writer;
pub mod worker_deps;
