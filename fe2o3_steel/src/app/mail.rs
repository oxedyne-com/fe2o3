//! Steel-side mail handler glue.
//!
//! Wires the SMTP and IMAP servers in `oxedyne_fe2o3_net` to the
//! Maildir + passwd-file implementations in `oxedyne_fe2o3_mail` and
//! drives a small background worker that flushes the outbound spool
//! through the SMTP client.

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_mail::{
    maildir::MaildirStore,
    outbound::OutboundSpool,
    passwd::PasswdFileUserStore,
};
use oxedyne_fe2o3_net::{
    dkim::DkimSigner,
    mail::user::UserStore,
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
    /// Maildir mailbox store.
    pub store:          MaildirStore,
    /// User authentication backend.
    pub users:          PasswdFileUserStore,
    /// Outbound delivery spool.
    pub spool:          OutboundSpool,
    /// DKIM signer, if a key is configured.
    pub dkim:           Option<Arc<DkimSigner>>,
    /// Domains the receive path will accept mail for.
    pub local_domains:  Arc<Vec<String>>,
}

impl std::fmt::Debug for AppMailHandler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppMailHandler")
            .field("local_domains", &self.local_domains)
            .field("dkim", &self.dkim.is_some())
            .finish()
    }
}

impl SmtpHandler for AppMailHandler {

    fn deliver_inbound(&self, txn: SmtpTransaction) -> Outcome<HandlerOutcome> {
        // For each recipient resolve the local mailbox and append.
        let mut accepted = 0u32;
        for rcpt in &txn.rcpt_to {
            let user = match self.users.lookup(rcpt) {
                Ok(Some(u)) => u,
                _ => {
                    warn!("Inbound delivery: unknown recipient {}", rcpt);
                    continue;
                }
            };
            // Materialise the bytes the way the IMAP server will read
            // them: prepend a Received header so the audit trail is
            // useful and the message is a valid RFC 5322 document.
            let received = build_received_header(&txn);
            let mut bytes = received.into_bytes();
            bytes.extend_from_slice(&txn.raw_message);
            use oxedyne_fe2o3_net::mail::store::{FolderName, MailStore, MessageFlags};
            let flags = MessageFlags { recent: true, ..Default::default() };
            let internal = Some(SystemTime::now());
            let result = self.store.append(
                &user,
                &FolderName::new("INBOX"),
                &bytes,
                flags,
                internal,
            );
            match result {
                Ok(_uid) => { accepted += 1; }
                Err(e) => {
                    error!(err!(e,
                        "Failed to append inbound mail for {}.", rcpt;
                        IO));
                }
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
        let signed_bytes = match &self.dkim {
            Some(signer) => {
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                match signer.sign(&txn.raw_message, &[], now) {
                    Ok(b) => b,
                    Err(e) => {
                        warn!("DKIM signing failed: {}", e);
                        txn.raw_message.clone()
                    }
                }
            }
            None => txn.raw_message.clone(),
        };

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

        // Local delivery first.
        for rcpt in &local {
            let user = match self.users.lookup(rcpt) {
                Ok(Some(u)) => u,
                _ => {
                    warn!("Submission: local recipient {} not in user store", rcpt);
                    continue;
                }
            };
            use oxedyne_fe2o3_net::mail::store::{FolderName, MailStore, MessageFlags};
            let flags = MessageFlags { recent: true, ..Default::default() };
            let internal = Some(SystemTime::now());
            if let Err(e) = self.store.append(
                &user,
                &FolderName::new("INBOX"),
                &signed_bytes,
                flags,
                internal,
            ) {
                error!(err!(e,
                    "Local delivery to {} failed.", rcpt;
                    IO));
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


/// Background worker that polls the spool and pushes each message
/// through the outbound SMTP client. Runs forever.
pub async fn run_outbound_worker(
    spool:      OutboundSpool,
    client:     OutboundClient,
)
    -> Outcome<()>
{
    use std::time::Duration;
    loop {
        // Drain the spool.
        match spool.list() {
            Ok(messages) => {
                for msg in messages {
                    info!("Outbound: delivering {} ({} rcpt)",
                        msg.filename, msg.rcpt_to.len());
                    let result = client.deliver(
                        &msg.mail_from,
                        &msg.rcpt_to,
                        &msg.body,
                    ).await;
                    match result {
                        Ok(qid) => {
                            info!("Outbound: {} delivered (remote: {})",
                                msg.filename, qid);
                            if let Err(e) = spool.remove(&msg.filename) {
                                warn!("Failed to remove spool file {}: {}",
                                    msg.filename, e);
                            }
                        }
                        Err(e) => {
                            warn!("Outbound: {} failed: {}", msg.filename, e);
                            // Leave on disk for next sweep.
                        }
                    }
                }
            }
            Err(e) => warn!("Spool list error: {}", e),
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
