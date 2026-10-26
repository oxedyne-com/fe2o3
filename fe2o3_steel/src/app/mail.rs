//! Steel-side mail handler glue.
//!
//! Wires the SMTP and IMAP servers in `oxedyne_fe2o3_net` to the
//! Maildir + passwd-file implementations in `oxedyne_fe2o3_mail` and
//! drives a small background worker that flushes the outbound spool
//! through the SMTP client.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_mail::{
    maildir::MaildirStore,
    outbound::{
        OutboundSpool,
        RetrySchedule,
    },
    passwd::PasswdFileUserStore,
};
use oxedyne_fe2o3_net::{
    dkim::DkimSigner,
    mail::{
        store::{
            FolderName,
            MailStore,
            MailUser,
            MessageFlags,
        },
        user::UserStore,
    },
    smtp::{
        client::OutboundClient,
        handler::{
            HandlerOutcome,
            SmtpHandler,
            SmtpTransaction,
        },
    },
};

use std::{
    sync::Arc,
    time::{
        SystemTime,
        UNIX_EPOCH,
    },
};


/// Cloneable handler shared across every SMTP listener.
#[derive(Clone)]
pub struct AppMailHandler {
    pub store:          MaildirStore,
    pub users:          PasswdFileUserStore,
    pub spool:          OutboundSpool,
    // Each signer prepends its own `DKIM-Signature` header; a receiver verifies
    // whichever it understands and ignores the rest. Empty means outbound mail
    // goes unsigned.
    pub dkim:           Vec<Arc<DkimSigner>>,
    pub local_domains:  Arc<Vec<String>>,
}

impl std::fmt::Debug for AppMailHandler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppMailHandler")
            .field("local_domains", &self.local_domains)
            .field("dkim", &self.dkim.len())
            .finish()
    }
}

impl SmtpHandler for AppMailHandler {

    fn deliver_inbound(&self, txn: SmtpTransaction) -> Outcome<HandlerOutcome> {
        // Materialise the bytes the way the IMAP server will read them: prepend a Received
        // header so the audit trail is useful and the message is a valid RFC 5322 document.
        let mut bytes = build_received_header(&txn).into_bytes();
        bytes.extend_from_slice(&txn.raw_message);
        let mut accepted = 0u32;
        for rcpt in &txn.rcpt_to {
            let user = match self.users.lookup(rcpt) {
                Ok(Some(u)) => u,
                Ok(None) => {
                    // Refused at RCPT time, so only a user removed since then reaches here.
                    warn!("Inbound delivery: unknown recipient {}", rcpt);
                    continue;
                },
                Err(e) => return Ok(self.deferred(e, rcpt)),
            };
            match self.deliver_local(&user, &bytes) {
                Ok(())  => accepted += 1,
                Err(e)  => return Ok(self.deferred(e, rcpt)),
            }
        }
        if accepted == 0 {
            return Ok(HandlerOutcome::RejectPermanent(
                "No recipients accepted".to_string()));
        }
        Ok(HandlerOutcome::Accepted(fmt!("inbound-{}", short_id())))
    }

    fn submit_outbound(&self, txn: SmtpTransaction) -> Outcome<HandlerOutcome> {
        // DKIM-sign first, if a key is configured. Signed bytes are
        // used both for local delivery (so the IMAP-fetched message
        // shows the DKIM-Signature header) and for remote delivery.
        // Sign with every configured key, each prepending its own header.
        // The signatures are independent: a DKIM-Signature field is not itself
        // among the covered headers, and the body is untouched, so signing the
        // already-signed bytes does not disturb the earlier signature.
        //
        // A key that fails to sign is skipped rather than fatal. One bad
        // signature is worth less than no mail at all, and the remaining key
        // still carries the message.
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let mut signed_bytes = txn.raw_message.clone();
        for signer in &self.dkim {
            match signer.sign(&signed_bytes, &[], now) {
                Ok(b) => signed_bytes = b,
                Err(e) => warn!("DKIM signing with the {} key for selector \
                    '{}' failed; continuing without it: {}",
                    signer.algorithm(), signer.selector(), e),
            }
        }

        // Split recipients into local (deliver directly into the
        // mailbox) and remote (enqueue for SMTP outbound). This
        // short-circuit avoids a self-loop through the public MX for
        // intra-domain mail and removes a real cert/PTR/SPF surface
        // from the local delivery path.
        let mut local: Vec<String> = Vec::new();
        let mut remote: Vec<String> = Vec::new();
        for r in &txn.rcpt_to {
            let domain = match r.rfind('@') {
                Some(i) => r[i + 1..].to_lowercase(),
                None    => { remote.push(r.clone()); continue; }
            };
            if self.local_domains.iter().any(|d| d.eq_ignore_ascii_case(&domain)) {
                local.push(r.clone());
            } else {
                remote.push(r.clone());
            }
        }

        // Local delivery first, and the remote copies are queued only once every local one
        // has landed, so a deferral leaves nothing queued that the client's retry would queue
        // again.
        for rcpt in &local {
            let user = match self.users.lookup(rcpt) {
                Ok(Some(u)) => u,
                Ok(None) => {
                    warn!("Submission: local recipient {} not in user store", rcpt);
                    continue;
                },
                Err(e) => return Ok(self.deferred(e, rcpt)),
            };
            if let Err(e) = self.deliver_local(&user, &signed_bytes) {
                return Ok(self.deferred(e, rcpt));
            }
        }

        // Remote delivery via the spool, only if there is anything to
        // send to the outside world.
        if remote.is_empty() {
            return Ok(HandlerOutcome::Accepted(fmt!("local-{}", short_id())));
        }
        let qid = res!(self.spool.enqueue(
            &txn.mail_from,
            &remote,
            &signed_bytes,
        ));
        Ok(HandlerOutcome::Accepted(qid))
    }

    fn rcpt_acceptable(&self, address: &str) -> bool {
        // Pure local-domain check first, then a UserStore lookup as
        // fallback for explicit aliases.
        let domain = match address.rfind('@') {
            Some(i) => address[i + 1..].to_lowercase(),
            None    => return false,
        };
        if self.local_domains.iter().any(|d| d.eq_ignore_ascii_case(&domain)) {
            return true;
        }
        matches!(self.users.lookup(address), Ok(Some(_)))
    }
}

impl AppMailHandler {

    /// Files `bytes` in the user's INBOX once: a copy already there from an earlier attempt,
    /// by Message-ID, is not written again, so a retry after a deferral duplicates nothing.
    fn deliver_local(&self, user: &MailUser, bytes: &[u8]) -> Outcome<()> {
        let flags = MessageFlags { recent: true, ..Default::default() };
        res!(self.store.deliver(user, &FolderName::new("INBOX"), bytes, flags, Some(SystemTime::now())));
        Ok(())
    }

    /// A message that cannot be filed is deferred whole with `451 4.3.0`, never refused with a
    /// `550` the sender would bounce, and never dropped for one recipient while the others are
    /// answered `250`: the sender retries, and the copies already filed are not filed again.
    fn deferred(&self, e: Error<ErrTag>, rcpt: &str) -> HandlerOutcome {
        error!(err!(e,
            "Local delivery to {} failed; the message is deferred.", rcpt;
            IO));
        HandlerOutcome::RejectTemporary(fmt!(
            "4.3.0 Mailbox temporarily unavailable, try again later"))
    }
}

/// Sweeps the spool every thirty seconds and pushes each message that is due through the outbound
/// SMTP client. Runs forever. What becomes of a failure, whether it waits, is retried or ends
/// in `failed/`, is [`OutboundSpool::sweep`]'s to say, so that it can be tested without a network.
pub async fn run_outbound_worker(
    spool:      OutboundSpool,
    client:     OutboundClient,
)
    -> Outcome<()>
{
    use std::time::Duration;
    let mut sched = RetrySchedule::default();
    loop {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or_default();
        let client = &client;
        let sweep = spool.sweep(&mut sched, now, |msg| async move {
            client.deliver(&msg.mail_from, &msg.rcpt_to, &msg.body).await
        }).await;
        if let Err(e) = sweep {
            warn!("Spool sweep error: {}", e);
        }
        tokio::time::sleep(Duration::from_secs(30)).await;
    }
}

fn short_id() -> String {
    use oxedyne_fe2o3_core::rand::Rand;
    Rand::generate_random_string(8, "abcdefghijklmnopqrstuvwxyz0123456789")
}

fn build_received_header(txn: &SmtpTransaction) -> String {
    fmt!(
        "Received: from {} ([{}])\r\n\tby Hematite Steel; {}\r\n",
        txn.helo_domain,
        txn.peer.ip(),
        format_now_rfc5322(),
    )
}

fn format_now_rfc5322() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let (y, mo, d, h, mi, s) = unix_to_civil(secs);
    let months = [
        "Jan","Feb","Mar","Apr","May","Jun",
        "Jul","Aug","Sep","Oct","Nov","Dec",
    ];
    let days = ["Mon","Tue","Wed","Thu","Fri","Sat","Sun"];
    // Day of week from days since 1970-01-01 (Thursday).
    let dow = ((secs / 86_400 + 4) % 7) as usize;
    fmt!(
        "{}, {:02} {} {:04} {:02}:{:02}:{:02} +0000",
        days[dow], d, months[(mo as usize - 1).min(11)], y, h, mi, s,
    )
}

fn unix_to_civil(secs: u64) -> (i32, u32, u32, u32, u32, u32) {
    let days = (secs / 86_400) as i64;
    let rem = (secs % 86_400) as u32;
    let h = rem / 3_600;
    let mi = (rem / 60) % 60;
    let s = rem % 60;
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y as i32, m, d, h, mi, s)
}
