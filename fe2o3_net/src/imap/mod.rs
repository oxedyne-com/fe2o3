//! IMAP4rev1 (RFC 3501): a server and a client.
//!
//! [`client`] connects out to somebody else's IMAP server and reads mail
//! from it. [`server`] is the other direction -- a Thunderbird-compatible
//! subset serving a local Hematite mailbox -- and is described below.
//!
//! # Server
//!
//! Implements the wire protocol and just enough of the command set to
//! run a single Hematite mailbox in front of Thunderbird:
//!
//! - `CAPABILITY`, `NOOP`, `LOGOUT`
//! - `LOGIN` (no SASL on the MVP)
//! - `LIST`, `LSUB`, `SUBSCRIBE`, `UNSUBSCRIBE`
//! - `SELECT`, `EXAMINE`, `CLOSE`
//! - `STATUS`
//! - `FETCH` and `UID FETCH` for the items Thunderbird actually asks
//!   for: `UID`, `FLAGS`, `RFC822.SIZE`, `INTERNALDATE`, `ENVELOPE`,
//!   `BODY[]`, `BODY.PEEK[]`, `BODY[HEADER.FIELDS (...)]`,
//!   `BODY.PEEK[HEADER.FIELDS (...)]`, `RFC822.HEADER`, `RFC822`.
//! - `STORE` and `UID STORE` for `+FLAGS`, `-FLAGS`, `FLAGS`.
//! - `UID SEARCH` (the trivial `ALL` and UID-range queries).
//! - `APPEND` (synchronising and non-synchronising literals).
//! - `EXPUNGE`, `CREATE`, `DELETE`.
//!
//! Out of scope for the MVP and not implemented: `IDLE`, `CONDSTORE`,
//! `QRESYNC`, `BINARY`, `LITERAL+` quirks beyond plain pass-through,
//! ACLs, quotas, namespace, sort, thread.

pub mod client;
pub mod server;
