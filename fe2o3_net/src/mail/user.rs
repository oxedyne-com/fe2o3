//! User authentication trait used by SMTP submission and IMAP login.
//!
//! Implementations look up an account by its full email address and
//! verify the supplied password, returning a [`MailUser`] handle that the
//! `MailStore` then consumes to address the right mailbox.

use crate::mail::store::MailUser;

use oxedyne_fe2o3_core::prelude::*;


/// Authentication trait used by the SMTP `AUTH` command on submission
/// (port 587) and by the IMAP `LOGIN` command (port 993).
///
/// Implementations are expected to be cheap to clone (typically via an
/// internal `Arc`) and thread-safe so a single store can be handed to
/// every server task without contention.
pub trait UserStore: Clone + Send + Sync + 'static {
    /// Verify the password for `address` and return the resulting
    /// [`MailUser`] on success. The address is the full RFC 5321
    /// `local@domain` form.
    ///
    /// Implementations should treat `address` lookups
    /// case-insensitively for the local part where the underlying
    /// system permits it (most do), and always lowercase the domain.
    /// Errors must be returned only for transport-level failures; a
    /// wrong password is a successful lookup that yields `Ok(None)`.
    fn authenticate(
        &self,
        address:    &str,
        password:   &str,
    )
        -> Outcome<Option<MailUser>>;

    /// Return the [`MailUser`] handle for an address without performing
    /// any password check. Used by the SMTP receive path on port 25 to
    /// route inbound mail to a local mailbox.
    fn lookup(&self, address: &str) -> Outcome<Option<MailUser>>;
}
