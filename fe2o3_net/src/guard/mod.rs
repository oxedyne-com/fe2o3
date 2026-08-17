//! Generic guards that make per-address and per-user accept/drop decisions.
//!
//! `addr::AddressGuard` is a transport-agnostic per-IP rate-limiter and blacklist. It was
//! originally lifted out of `fe2o3_shield`'s SHIELD UDP protocol so that any consumer --
//! HTTPS servers, SMTP, IMAP, DNS resolvers, the SHIELD wire protocol itself -- can share a
//! single, hardened implementation. The SHIELD-specific handshake-sequence check is built
//! on top of the generic core via `AddressGuard::update_log`, leaving the base guard free of
//! protocol details.
//!
//! `user::UserGuard` is the per-user counterpart: a sharded map of trust state (Unknown /
//! Blacklist / Whitelist) with a caller-supplied data payload, likewise lifted out of
//! `fe2o3_shield` so any protocol can classify users, not only addresses.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

pub mod addr;
pub mod user;
