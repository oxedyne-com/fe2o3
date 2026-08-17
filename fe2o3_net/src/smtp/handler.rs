//! Application-level hooks the SMTP server calls on accepted messages.
//!
//! A `SmtpHandler` decides what to do with a fully-received RFC 5322
//! message after the server has already enforced the protocol: receive
//! path delivery to a local mailbox, submission path enqueue for outbound
//! delivery, etc. The trait is split into two methods so the same handler
//! type can serve both ports with different policies.
//!
//! [Written entirely with AI](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use crate::mail::store::MailUser;

use oxedyne_fe2o3_core::prelude::*;

use std::net::SocketAddr;


/// The server fills this in from the commands received before `DATA`, then
/// hands it to a `SmtpHandler` with the raw message bytes.
#[derive(Clone, Debug)]
pub struct SmtpTransaction {
    pub mail_from:      String,             // empty is the null reverse-path `<>`, RFC 5321
    pub rcpt_to:        Vec<String>,        // in the order received
    pub helo_domain:    String,
    pub auth_user:      Option<MailUser>,   // None on the port 25 receive path
    pub peer:           SocketAddr,
    pub tls:             bool,              // implicit TLS or an in-session STARTTLS upgrade
    // Dot-unstuffed and CRLF-preserving, without the terminating `<CRLF>.<CRLF>`.
    pub raw_message:    Vec<u8>,
}

/// The queue id in `Accepted` reaches the client on the `250 OK` line, so an
/// administrator can grep the logs by it.
#[derive(Clone, Debug)]
pub enum HandlerOutcome {
    Accepted(String),           // queue id, echoed in the `250`
    RejectPermanent(String),    // reason, returned in a `550`
    RejectTemporary(String),    // reason, returned in a `451`
}

/// Implementations are expected to be cheap to clone, typically via an internal
/// `Arc`, so one handler serves every accept loop without contention. Both
/// methods are synchronous: the server calls them inside a
/// `tokio::task::spawn_blocking` so the underlying I/O does not block the
/// runtime.
pub trait SmtpHandler: Clone + Send + Sync + 'static {
    /// The port 25 path: deliver into every local mailbox `rcpt_to` names. A
    /// recipient that does not resolve locally is refused at `RCPT` time, so
    /// by here every recipient has already been accepted.
    fn deliver_inbound(&self, txn: SmtpTransaction) -> Outcome<HandlerOutcome>;

    /// The port 587 path: enqueue for outbound delivery, typically through
    /// `crate::smtp::client::OutboundClient`, after DKIM signing where a key
    /// is configured.
    fn submit_outbound(&self, txn: SmtpTransaction) -> Outcome<HandlerOutcome>;

    /// Will this `RCPT TO` be taken? Asked on the receive path only:
    /// submission listeners skip the check, since an authenticated client may
    /// relay anywhere.
    fn rcpt_acceptable(&self, address: &str) -> bool;
}
