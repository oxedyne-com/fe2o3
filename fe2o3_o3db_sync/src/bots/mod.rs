//! The bot hierarchy that runs the database.
//!
//! Each bot is an operating-system thread communicating only by message. The
//! supervisor ([`bot_super`]) starts and monitors everything; a zone bot
//! ([`bot_zone`]) manages each storage zone; server bots ([`bot_server`])
//! front user requests; the config bot ([`bot_config`]) distributes
//! configuration; and the per-zone [`worker`] pools (cache, file, reader,
//! writer, init-garbage) do the actual storage work.

pub mod base;
pub mod bot_config;
pub mod bot_server;
pub mod bot_super;
pub mod bot_zone;
pub mod worker;
