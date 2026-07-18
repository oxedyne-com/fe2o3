//! Machinery common to every bot: the [`bot`] base behaviour and shared
//! [`OzoneBot`](bot::OzoneBot) state, the thread [`handles`] the supervisor
//! retains, and the [`bot_deps`] dependency bundle passed at start-up.

pub mod bot;
pub mod handles;
pub mod bot_deps;
