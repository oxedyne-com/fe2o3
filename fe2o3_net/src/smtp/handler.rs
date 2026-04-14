//! Application-level hooks the SMTP server calls on accepted messages.
//!
//! A `SmtpHandler` decides what to do with a fully-received RFC 5322
//! message after the server has already enforced the protocol: receive
//! path delivery to a local mailbox, submission path enqueue for outbound
//! delivery, etc. The trait is split into two methods so the same handler
//! type can serve both ports with different policies.

use crate::mail::store::MailUser;

use oxedyne_fe2o3_core::prelude::*;

use std::net::SocketAddr;


/// One in-flight SMTP transaction once `DATA` has been accepted.
///
/// The server fills this in from the wire commands it received before
/// `DATA`, then hands it to a `SmtpHandler` together with the raw
/// message bytes. The handler decides whether the transaction succeeds
/// (returning `Ok(())`) or fails with a permanent or transient error.
#[derive(Clone, Debug)]
pub struct SmtpTransaction {
    /// Envelope sender from `MAIL FROM`. Empty string means the null
    /// reverse-path (`<>`), which RFC 5321 reserves for bounces.
    pub mail_from:      String,
    /// Envelope recipients from `RCPT TO`, in the order received.
    pub rcpt_to:        Vec<String>,
    /// HELO/EHLO domain the peer announced.
    pub helo_domain:    String,
    /// Authenticated user, when the session AUTHed successfully on a
    /// submission port. `None` for receive-path connections on port 25.
    pub auth_user:      Option<MailUser>,
    /// Peer socket address, useful for `Received:` headers and rate
    /// limiting.
    pub peer:           SocketAddr,
    /// `true` if the transaction was received over a TLS-protected
    /// channel (either implicit TLS or an in-session STARTTLS upgrade).
    pub tls:             bool,
    /// Raw RFC 5322 message bytes, with dot-unstuffing already applied
    /// and CRLF preserved. The terminating `<CRLF>.<CRLF>` is *not*
    /// included.
    pub raw_message:    Vec<u8>,
}

/// Outcome of a single SMTP transaction handed to a [`SmtpHandler`].
///
/// `Accepted` returns the queue id the handler stored the message under;
/// the server reports it back to the client in the `250 OK` line so
/// administrators can grep logs by it.
#[derive(Clone, Debug)]
pub enum HandlerOutcome {
    /// The handler accepted the message and stored it. The string is the
    /// queue id, returned to the client in the `250` response.
    Accepted(String),
    /// The handler rejected the message permanently. The string is the
    /// human-readable reason returned in a `550` response.
    RejectPermanent(String),
    /// The handler rejected the message temporarily. The string is the
    /// human-readable reason returned in a `451` response.
    RejectTemporary(String),
}

/// Application hook the SMTP server calls on completed transactions.
///
/// Implementations are expected to be cheap to clone (typically via an
/// internal `Arc`) so a single handler can be shared across every accept
/// loop without contention. Both methods are synchronous: the server
/// calls them inside a `tokio::task::spawn_blocking` so the underlying
/// I/O does not block the runtime.
pub trait SmtpHandler: Clone + Send + Sync + 'static {
    /// Receive-path entry point used by listeners on port 25. The
    /// handler is expected to look up each `rcpt_to` against its local
    /// user store and deliver the message into every matching mailbox.
    /// Recipients that do not resolve locally should be rejected at
    /// `RCPT` time -- by the time this method is called, all recipients
    /// have been accepted.
    fn deliver_inbound(&self, txn: SmtpTransaction) -> Outcome<HandlerOutcome>;

    /// Submission-path entry point used by listeners on port 587. The
    /// handler is expected to enqueue the message for outbound delivery
    /// (typically via `crate::smtp::client::OutboundClient`) after
    /// signing it with DKIM if a key is configured.
    fn submit_outbound(&self, txn: SmtpTransaction) -> Outcome<HandlerOutcome>;

    /// Verify a single `RCPT TO` address before accepting it on the
    /// receive path. Implementations should return `true` for any
    /// recipient they are willing to accept (typically: any local
    /// mailbox the user store recognises). Submission-path listeners
    /// skip this check entirely -- an authenticated client may relay to
    /// any address.
    fn rcpt_acceptable(&self, address: &str) -> bool;
}
