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
//! Filenames are `<unix>.<usec>.<pid>.<rand>.eml`. There is no retry
//! schedule beyond "the worker tries again on the next sweep" -- that
//! is enough for an MVP whose volume is a handful of messages a day.

use oxedyne_fe2o3_core::{
    prelude::*,
    rand::Rand,
};

use std::{
    fs::{self, File},
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
            match Self::read_one(&path) {
                Ok((from, rcpt, body)) => out.push(SpooledMessage {
                    filename:   name,
                    mail_from:  from,
                    rcpt_to:    rcpt,
                    body,
                }),
                Err(e) => warn!("Skipping spool file {:?}: {}", path, e),
            }
        }
        Ok(out)
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

fn find_double_lf(bytes: &[u8]) -> Option<usize> {
    for i in 0..bytes.len().saturating_sub(1) {
        if bytes[i] == b'\n' && bytes[i + 1] == b'\n' {
            return Some(i);
        }
    }
    None
}
