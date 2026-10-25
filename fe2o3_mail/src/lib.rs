//! Hematite email implementations.
//!
//! Two layers live here.  The sans-io [`message`] module reads a message from bytes and
//! builds a draft into bytes, owning no socket and no clock, so a browser client can use it
//! -- it is behind no feature and drags in nothing but `fe2o3_core` and `fe2o3_text`.  The
//! folder sync engine [`sync`] is sans-runtime too, written over a [`sync::MailSession`] that the
//! gateway's IMAP client and a browser's tunnel each implement, and takes only `fe2o3_net`'s
//! tokio-free IMAP types.
//!
//! The server layer is the on-disk implementations of the trait surface (`MailStore`,
//! `UserStore`) that lives in `fe2o3_net` alongside the SMTP and IMAP servers that consume
//! it -- a Maildir-backed mailbox store and a `passwd`-style user file -- plus the outbound
//! spool.  The store and the user file pull in `fe2o3_net`'s async half, which does not build
//! for `wasm32-unknown-unknown`, so they sit behind the `server` feature (on by default).

#![forbid(unsafe_code)]

pub mod message;

#[cfg(feature = "server")]
pub mod maildir;
pub mod outbound;
#[cfg(feature = "server")]
pub mod passwd;
pub mod sync;

use oxedyne_fe2o3_core::prelude::*;


/// An addressable email recipient, sender or reply-to target.
///
/// Wrapping the raw string in a newtype makes it harder to accidentally
/// pass a display name or subject line where an address is expected,
/// and gives a natural hook for future address validation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EmailAddress {
    /// Raw address in `local@domain` form. Not validated at construction;
    /// implementations may validate at send time.
    pub raw: String,
}

impl EmailAddress {
    /// Wrap a raw string as an email address.
    pub fn new<S: Into<String>>(raw: S) -> Self {
        Self { raw: raw.into() }
    }
}

/// A plain-text email message ready for delivery.
#[derive(Clone, Debug)]
pub struct EmailMessage {
    /// Envelope sender.
    pub from:       EmailAddress,
    /// Envelope recipients.
    pub to:         Vec<EmailAddress>,
    /// RFC 5322 subject, plain-text.
    pub subject:    String,
    /// Plain-text body.
    pub body:       String,
}

impl EmailMessage {
    /// Construct a one-recipient plain-text message.
    pub fn new<S: Into<String>>(
        from:       EmailAddress,
        to:         EmailAddress,
        subject:    S,
        body:       S,
    )
        -> Self
    {
        Self {
            from,
            to:         vec![to],
            subject:    subject.into(),
            body:       body.into(),
        }
    }
}

/// Send-side trait for pushing an `EmailMessage` out to a recipient.
pub trait EmailSender: Clone + Send + Sync + 'static {
    /// Deliver a fully-constructed message to its envelope recipients.
    fn send(&self, msg: &EmailMessage) -> Outcome<()>;
}

/// Null sender that discards every message. Useful in tests.
#[derive(Clone, Debug, Default)]
pub struct NullEmailSender;

impl EmailSender for NullEmailSender {
    fn send(&self, _msg: &EmailMessage) -> Outcome<()> {
        Ok(())
    }
}
