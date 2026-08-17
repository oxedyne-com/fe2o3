//! User authentication trait used by SMTP submission and IMAP login.
//!
//! Implementations look up an account by its full email address and
//! verify the supplied password, returning a [`MailUser`] handle that the
//! `MailStore` then consumes to address the right mailbox.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use crate::mail::store::MailUser;

use oxedyne_fe2o3_core::prelude::*;


/// Implementations are expected to be cheap to clone, typically via an internal
/// `Arc`, so a single store can be handed to every server task without
/// contention.
pub trait UserStore: Clone + Send + Sync + 'static {
    /// The address is the full RFC 5321 `local@domain` form, matched
    /// case-insensitively in the local part where the underlying system permits
    /// it, and always with the domain lowercased. An error means a
    /// transport-level failure only; a wrong password is a successful lookup
    /// that yields `Ok(None)`.
    fn authenticate(
        &self,
        address:    &str,
        password:   &str,
    )
        -> Outcome<Option<MailUser>>;

    /// No password check; the SMTP receive path on port 25 uses this to route
    /// inbound mail to a local mailbox.
    fn lookup(&self, address: &str) -> Outcome<Option<MailUser>>;
}
