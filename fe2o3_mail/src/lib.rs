//! Hematite email implementations.
//!
//! The trait surface (`MailStore`, `UserStore`) lives in `fe2o3_net`
//! alongside the SMTP and IMAP servers that consume it. This crate
//! provides the on-disk implementations -- a Maildir-backed mailbox
//! store and a `passwd`-style user file -- plus the small set of
//! primitive types the rest of Hematite shares for sending mail.

#![forbid(unsafe_code)]

pub mod maildir;
pub mod outbound;
pub mod passwd;

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
