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
//!
//! `shot::Shots` is the picture half: a device asked for a picture of its screen answers with a
//! PNG or a reason, under a rate floor, a size cap and an age of seven days.
//!
//! With the `async` feature, `tap::Tap` is a tap on a websocket's messages (`crate::ws::tap`)
//! that files what a caller's filter keeps as rows of a `Sink`.
//!
//! Two pieces serve an app's own secrets. `redact::Phrase` is the test for a passphrase drawn from
//! a list of words (`n` of them in a row), and `Or` joins it to the stock `Shapes`. `chunk` is how a
//! `Sink` reads a bundle sent as base64 in `ds snapshot|telemetry <id> i/N` rows: it judges the
//! text they encode, so the bundle is kept whole when it is innocent and covered whole when it is
//! not. A bundle that is JSON meets the sink's whole `Redact`, names and word runs across its
//! strings included, and is covered where the secret stands and sent on in the rows it came in.

pub mod chunk;
pub mod redact;
pub mod row;
pub mod shot;
pub mod sink;
#[cfg(feature = "async")]
pub mod tap;

pub use chunk::{
    judge,
    ChunkTag,
    Verdict,
};
pub use redact::{
    fingerprint,
    secret_name,
    secret_name_loose,
    Deny,
    NoTest,
    Or,
    Phrase,
    Redact,
    Shapes,
    StrTest,
};
pub use row::{
    compact,
    Entry,
    Row,
};
pub use shot::{
    Answer,
    Ask,
    Filed,
    ShotGates,
    ShotRefusal,
    Shots,
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
#[cfg(feature = "async")]
pub use tap::{
    Dir,
    Frame,
    Tap,
};
