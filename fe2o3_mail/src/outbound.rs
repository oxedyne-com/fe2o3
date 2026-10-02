//! On-disk spool for outbound mail awaiting delivery.
//!
//! Each accepted submission is written to a single file in the spool
//! directory. A background worker reads the spool, attempts delivery
//! via [`oxedyne_fe2o3_net::smtp::client::OutboundClient`], and removes
//! the file on success. The spool format is a tiny text envelope
//! followed by a blank line and the raw RFC 5322 message:
//!
//! ```text
//! From: postmaster@example.com
//! Rcpt: alice@example.org
//! Rcpt: bob@example.net
//!
//! <RFC 5322 message bytes>
//! ```
//!
//! Filenames are `<unix>.<usec>.<pid>.<rand>.eml`, so a message's age is
//! readable from its name and survives a restart.
//!
//! A sweep ([`OutboundSpool::sweep`]) settles every message one of three
//! ways: delivered and removed; failed for good and set aside in `failed/`
//! (a failure tagged [`ErrTag::Permanent`], such as a 5xx reply, or a
//! message older than [`GIVE_UP_SECS`]); or failed for now and tried again
//! after a growing wait. Until 2026-10-02 every failure was "tried again on
//! the next sweep", thirty seconds later and for ever: two stale messages, a
//! 550 and a null MX, were retried about 63,000 times each over twenty-five
//! days before anyone looked.

use oxedyne_fe2o3_core::{
    prelude::*,
    rand::Rand,
};

use std::{
    collections::{HashMap, HashSet},
    fs::{self, File},
    future::Future,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};


/// One queued message ready for delivery.
#[derive(Clone, Debug)]
pub struct SpooledMessage {
    /// Spool filename (no path).
    pub filename:   String,
    /// Envelope sender.
    pub mail_from:  String,
    /// Envelope recipients.
    pub rcpt_to:    Vec<String>,
    /// Raw RFC 5322 message bytes.
    pub body:       Vec<u8>,
    pub queued:     u64,    // when it was queued, in unix seconds
}

// RFC 5321 §4.5.4.1 asks a sender to persist for four or five days before giving up.
pub const GIVE_UP_SECS: u64 = 5 * 86_400;
const RETRY_FIRST_SECS: u64 = 60;
const RETRY_MAX_SECS:   u64 = 4 * 3_600;

/// The wait, in seconds, before the next attempt once `tries` attempts have failed: a minute,
/// doubling, to a ceiling of four hours.
pub fn retry_after(tries: u32) -> u64 {
    let shift = tries.saturating_sub(1).min(16);
    (RETRY_FIRST_SECS << shift).min(RETRY_MAX_SECS)
}

/// What a delivery worker remembers between sweeps: how often each queued message has failed, and
/// when it may next be tried. It lives in memory, so a restart forgets it. That costs one early
/// retry and no more, because a message's age comes from its name and the give-up time still holds.
#[derive(Debug, Default)]
pub struct RetrySchedule {
    next: HashMap<String, (u32, u64)>,  // filename -> (failed attempts, unix second the next is due)
}

/// What one sweep did to the messages it found.
#[derive(Debug, Default, Eq, PartialEq)]
pub struct Sweep {
    pub delivered:  usize,  // accepted by the far end and removed
    pub failed:     usize,  // set aside in `failed/`, never to be tried again
    pub deferred:   usize,  // tried and failed, to be tried again after a wait
    pub waiting:    usize,  // not yet due, so not tried
}

/// Filesystem-backed outbound spool.
#[derive(Clone, Debug)]
pub struct OutboundSpool {
    /// Spool directory. Must exist.
    pub root: Arc<PathBuf>,
}

impl OutboundSpool {
    /// Build a spool rooted at `root`. Creates the directory if it
    /// does not yet exist.
    pub fn new(root: PathBuf) -> Outcome<Self> {
        if !root.exists() {
            if let Err(e) = fs::create_dir_all(&root) {
                return Err(err!(e,
                    "Creating spool dir {:?}.", root;
                    IO, File, Init));
            }
        }
        Ok(Self { root: Arc::new(root) })
    }

    /// Append a new message to the spool. Returns the assigned queue
    /// id (the filename without the `.eml` extension) so the SMTP
    /// handler can echo it back to the client in the `250 OK` line.
    pub fn enqueue(
        &self,
        mail_from:  &str,
        rcpt_to:    &[String],
        body:       &[u8],
    )
        -> Outcome<String>
    {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d)
            .unwrap_or_default();
        let suffix = Rand::generate_random_string(6, "abcdefghijklmnopqrstuvwxyz0123456789");
        let qid = fmt!("{}.{}.{}.{}", now.as_secs(), now.subsec_micros(),
            std::process::id(), suffix);
        let filename = fmt!("{}.eml", qid);
        let path = self.root.join(&filename);

        let mut buf: Vec<u8> = Vec::with_capacity(body.len() + 256);
        buf.extend_from_slice(fmt!("From: {}\n", mail_from).as_bytes());
        for r in rcpt_to {
            buf.extend_from_slice(fmt!("Rcpt: {}\n", r).as_bytes());
        }
        buf.extend_from_slice(b"\n");
        buf.extend_from_slice(body);

        let mut f = match File::create(&path) {
            Ok(f) => f,
            Err(e) => return Err(err!(e,
                "Creating spool file {:?}.", path;
                IO, File, Write)),
        };
        if let Err(e) = f.write_all(&buf) {
            return Err(err!(e,
                "Writing spool file {:?}.", path;
                IO, File, Write));
        }
        Ok(qid)
    }

    /// Read the entire spool, returning every message currently
    /// queued. Used by the delivery worker on each sweep.
    pub fn list(&self) -> Outcome<Vec<SpooledMessage>> {
        let mut out: Vec<SpooledMessage> = Vec::new();
        let rd = match fs::read_dir(self.root.as_path()) {
            Ok(r) => r,
            Err(e) => return Err(err!(e,
                "Reading spool dir {:?}.", self.root;
                IO, File, Read)),
        };
        for entry in rd.flatten() {
            let path = entry.path();
            if !path.is_file() { continue; }
            let name = entry.file_name().to_string_lossy().into_owned();
            if !name.ends_with(".eml") { continue; }
            let queued = queued_at(&name, &path);
            match Self::read_one(&path) {
                Ok((from, rcpt, body)) => out.push(SpooledMessage {
                    filename:   name,
                    mail_from:  from,
                    rcpt_to:    rcpt,
                    body,
                    queued,
                }),
                Err(e) => warn!("Skipping spool file {:?}: {}", path, e),
            }
        }
        Ok(out)
    }

    /// One pass over the queue. `deliver` is the transport, given each message that is due.
    ///
    /// A failure tagged [`ErrTag::Permanent`] ends the message: it is logged once and set aside in
    /// `failed/`. RFC 5321 §4.2.1 makes a 5xx reply final, and nothing a later sweep does will
    /// change a refusal. Any other failure is retried after [`retry_after`], and abandoned once the
    /// message is [`GIVE_UP_SECS`] old. `now` is the current unix second, passed in so a test need
    /// not wait out a retry.
    pub async fn sweep<F, Fut>(
        &self,
        sched:      &mut RetrySchedule,
        now:        u64,
        deliver:    F,
    )
        -> Outcome<Sweep>
        where
            F:      Fn(SpooledMessage) -> Fut,
            Fut:    Future<Output = Outcome<String>>,
    {
        let msgs = res!(self.list());
        let live: HashSet<&String> = msgs.iter().map(|m| &m.filename).collect();
        sched.next.retain(|name, _| live.contains(name));
        let mut out = Sweep::default();
        for msg in &msgs {
            let (tries, due) = sched.next.get(&msg.filename).copied().unwrap_or((0, 0));
            if now < due {
                out.waiting += 1;
                continue;
            }
            info!("Outbound: delivering {} ({} rcpt)", msg.filename, msg.rcpt_to.len());
            match deliver(msg.clone()).await {
                Ok(qid) => {
                    info!("Outbound: {} delivered (remote: {})", msg.filename, qid);
                    sched.next.remove(&msg.filename);
                    if let Err(e) = self.remove(&msg.filename) {
                        warn!("Failed to remove spool file {}: {}", msg.filename, e);
                    }
                    out.delivered += 1;
                }
                Err(e) => {
                    let tries = tries + 1;
                    // The tag is read directly, because this crate does not depend on fe2o3_net
                    // where `is_permanent` lives.
                    let over_age = now.saturating_sub(msg.queued) >= GIVE_UP_SECS;
                    if e.tags().contains(&ErrTag::Permanent) || over_age {
                        match self.fail(&msg.filename) {
                            Ok(_) => {
                                warn!("Outbound: {} failed for good after {} attempt(s), set aside \
                                    in failed/: {}", msg.filename, tries, e);
                                sched.next.remove(&msg.filename);
                                out.failed += 1;
                                continue;
                            }
                            // Leave it queued, but on the backoff below rather than every sweep.
                            Err(e2) => warn!("Outbound: could not set {} aside: {}",
                                msg.filename, e2),
                        }
                    }
                    let wait = retry_after(tries);
                    warn!("Outbound: {} failed (attempt {}), next try in {} s: {}",
                        msg.filename, tries, wait, e);
                    sched.next.insert(msg.filename.clone(), (tries, now + wait));
                    out.deferred += 1;
                }
            }
        }
        Ok(out)
    }

    /// Takes a message out of the queue for good, keeping it in `failed/` under the spool root
    /// for the operator. The sweep reads only the root, so a file there is never tried again.
    pub fn fail(&self, filename: &str) -> Outcome<PathBuf> {
        let dir = self.root.join("failed");
        if let Err(e) = fs::create_dir_all(&dir) {
            return Err(err!(e,
                "Creating failed-mail dir {:?}.", dir;
                IO, File, Write));
        }
        let to = dir.join(filename);
        if let Err(e) = fs::rename(self.root.join(filename), &to) {
            return Err(err!(e,
                "Moving spool file {} to {:?}.", filename, to;
                IO, File, Write));
        }
        Ok(to)
    }

    /// Remove a successfully-delivered message from the spool.
    pub fn remove(&self, filename: &str) -> Outcome<()> {
        let path = self.root.join(filename);
        if let Err(e) = fs::remove_file(&path) {
            return Err(err!(e,
                "Removing spool file {:?}.", path;
                IO, File, Write));
        }
        Ok(())
    }

    /// Parse a single on-disk spool entry.
    fn read_one(path: &Path) -> Outcome<(String, Vec<String>, Vec<u8>)> {
        let mut bytes = Vec::new();
        let mut f = match File::open(path) {
            Ok(f) => f,
            Err(e) => return Err(err!(e,
                "Opening {:?}.", path; IO, File, Read)),
        };
        if let Err(e) = f.read_to_end(&mut bytes) {
            return Err(err!(e,
                "Reading {:?}.", path; IO, File, Read));
        }
        // Find the blank line separating envelope from body.
        let split = match find_double_lf(&bytes) {
            Some(i) => i,
            None => return Err(err!(
                "Spool file {:?} missing envelope/body separator.", path;
                Invalid, Input, Decode)),
        };
        let header = String::from_utf8_lossy(&bytes[..split]).into_owned();
        let body = bytes[split + 2..].to_vec();
        let mut from = String::new();
        let mut rcpt: Vec<String> = Vec::new();
        for line in header.lines() {
            if let Some(v) = line.strip_prefix("From: ") {
                from = v.to_string();
            } else if let Some(v) = line.strip_prefix("Rcpt: ") {
                rcpt.push(v.to_string());
            }
        }
        Ok((from, rcpt, body))
    }
}

/// When a spooled file was queued, in unix seconds: the stamp that opens its name, or failing
/// that its modification time, or failing that now.
fn queued_at(name: &str, path: &Path) -> u64 {
    if let Some(Ok(secs)) = name.split('.').next().map(|s| s.parse::<u64>()) {
        return secs;
    }
    let mtime = fs::metadata(path).and_then(|m| m.modified()).ok();
    mtime
        .unwrap_or_else(SystemTime::now)
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default()
}

fn find_double_lf(bytes: &[u8]) -> Option<usize> {
    for i in 0..bytes.len().saturating_sub(1) {
        if bytes[i] == b'\n' && bytes[i + 1] == b'\n' {
            return Some(i);
        }
    }
    None
}
