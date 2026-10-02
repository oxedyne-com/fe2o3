//! The generic half of a debug lens: a gated NDJSON sink, the row envelope and a redactor.
//!
//! Lifted from Daimond, where the same pieces were written for one gateway. `sink::Sink` is the
//! gates of `gateway/src/handlers/debug_trace.rs` (body, row and field caps, a per-key rate
//! floor, rotation by size, history by age, count and total bytes) writing one JSON object a
//! line. `row::Row` is the event envelope `{v, d, n, b, t}` of `www/js/debugshare.js`, and
//! `redact::Redact` is its `SECRET_RE` and `fingerprint` over a `Dat`, with hooks for a caller's
//! deny-list and a caller's string test. Nothing here knows an application: an app supplies the
//! rows, the keys and the redaction hooks, and `Sink::new` is all it needs of the file system.
//!
//! The envelope and the gates must stay equal to Daimond's, so that Daimond's gateway can move
//! onto this module without its clients or its reader noticing. `Gates::daimond` holds Daimond's
//! numbers; `Gates::default` holds the retention the test site wants instead.

pub mod redact;
pub mod row;
pub mod sink;

pub use redact::{
    fingerprint,
    secret_name,
    secret_name_loose,
    Deny,
    NoTest,
    Redact,
    StrTest,
};
pub use row::{
    compact,
    Entry,
    Row,
};
pub use sink::{
    is_stamp,
    now_ms,
    sanitise,
    stamp,
    Gates,
    Posted,
    Refusal,
    Sink,
};
