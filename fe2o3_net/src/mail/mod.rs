//! Mailbox storage and user authentication abstractions.
//!
//! Defines the trait surface that the SMTP and IMAP servers in
//! [`crate::smtp`] and [`crate::imap`] depend on to persist messages and
//! authenticate users. Concrete implementations (Maildir on disk, a
//! `passwd`-style file, an Ozone-backed store) live in `fe2o3_mail`.

pub mod store;
pub mod user;
