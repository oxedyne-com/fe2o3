//! Focus / brief / fold substrate — the durable core of Red in the
//! browser.
//!
//! A **Focus** is a durable container for a pursuit.  Its reduced state is
//! the **brief** (`brief.md`); a **fold** re-reduces a delta into the
//! brief; the **log** is per-Focus and append-only.  This module owns the
//! OPFS layout and the pure store operations; the `#[wasm_bindgen]`
//! surface that drives the brief and reducer agents lives on
//! [`RedApp`](crate::wasm::app::RedApp) in [`crate::wasm::app`].
//!
//! OPFS layout, per Focus id:
//!
//! ```text
//! foci/<id>/brief.md              the reduced state (agent writes, user may edit)
//! foci/<id>/versions/NNNN.md      a snapshot per brief version (0-padded)
//! foci/<id>/.red/meta.json        { name, brief_version, updated }
//! foci/<id>/.red/log              append-only, one JSON record per line
//! foci/<id>/.red/deltas/NNNN.md   the raw delta a fold consumed, referenced by delta_ref
//! ```
//!
//! Each log record is a single-line JSON object:
//! `{ id, ts, kind, agent, task, parent_brief_version, brief_version,
//!    delta_ref, note }` with `kind` one of `create`, `edit`, `fold`.
//!
//! The store is app-local for now (a candidate for extraction into
//! `fe2o3_data` once its shape settles, per the v1 plan's D22); it is not
//! extracted here.  Whole-file read-modify-write backs the append (the
//! synchronous single-writer OPFS path is deferred); single-user,
//! single-Focus-at-a-time makes that sufficient for this stage.

use crate::llm::{extract_json_number, extract_json_string, json_escape};
use crate::protocol::generate_session_id;
use crate::wasm::opfs;

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_core::wasm::now_ms;


/// The brief agent's role: it maintains one Focus's brief, resolving an
/// instruction to a file edit or to one or more errors, never to chat.
pub const BRIEF_AGENT_PROMPT: &str =
    "You maintain this Focus's brief. Given an instruction, either edit \
     brief.md via your file tools or report one or more errors. Do not \
     converse.";

/// The reducer's role: fold exactly one delta into the current brief and
/// emit only the new brief markdown.  A fresh reducer holds no history,
/// so it cannot itself rot.
pub const REDUCER_PROMPT: &str =
    "Given the current brief and one delta, output the new brief. Keep the \
     goal, decisions and open threads; drop what the delta supersedes; \
     output only the new brief markdown.";


// ┌───────────────────────────────────────────────────────────────┐
// │ Path helpers                                                   │
// └───────────────────────────────────────────────────────────────┘

/// The Focus directory, `foci/<id>`.
pub fn focus_dir(id: &str) -> String {
    fmt!("foci/{}", id)
}

/// The brief content file, `foci/<id>/brief.md`.
fn brief_path(id: &str) -> String {
    fmt!("foci/{}/brief.md", id)
}

/// The append-only log, `foci/<id>/.red/log`.
fn log_path(id: &str) -> String {
    fmt!("foci/{}/.red/log", id)
}

/// The metadata file, `foci/<id>/.red/meta.json`.
fn meta_path(id: &str) -> String {
    fmt!("foci/{}/.red/meta.json", id)
}

/// A brief-version snapshot, `foci/<id>/versions/NNNN.md`.
fn version_path(id: &str, version: u64) -> String {
    fmt!("foci/{}/versions/{:04}.md", id, version)
}

/// A stored raw delta, `foci/<id>/.red/deltas/NNNN.md`, keyed by the
/// brief version the fold produced.
fn delta_path(id: &str, version: u64) -> String {
    fmt!("foci/{}/.red/deltas/{:04}.md", id, version)
}


// ┌───────────────────────────────────────────────────────────────┐
// │ Metadata                                                       │
// └───────────────────────────────────────────────────────────────┘

/// Per-Focus metadata held in `meta.json`.
struct Meta {
    /// Human-readable Focus name.
    name:    String,
    /// Current brief version (the latest snapshot).
    version: u64,
    /// Last-updated wall-clock time in whole milliseconds.
    updated: u64,
}

impl Meta {

    /// Serialise to a compact single-line JSON object.
    fn to_json(&self) -> String {
        fmt!(
            "{{\"name\":\"{}\",\"brief_version\":{},\"updated\":{}}}",
            json_escape(&self.name), self.version, self.updated,
        )
    }

    /// Parse from the stored JSON, tolerating missing fields.
    fn from_json(s: &str) -> Self {
        Self {
            name:    extract_json_string(s, "name").unwrap_or_default(),
            version: extract_json_number(s, "brief_version").unwrap_or(0),
            updated: extract_json_number(s, "updated").unwrap_or(0),
        }
    }
}

/// Read a Focus's metadata.
async fn read_meta(id: &str) -> Outcome<Meta> {
    let bytes = res!(opfs::read_file(&meta_path(id)).await);
    let s = String::from_utf8_lossy(&bytes).to_string();
    Ok(Meta::from_json(&s))
}

/// Write a Focus's metadata.
async fn write_meta(id: &str, meta: &Meta) -> Outcome<()> {
    opfs::write_file(&meta_path(id), meta.to_json().as_bytes()).await
}


// ┌───────────────────────────────────────────────────────────────┐
// │ Log records                                                    │
// └───────────────────────────────────────────────────────────────┘

/// One append-only log record.  `parent` uses `-1` for "no parent"
/// (the `create` record), matching the JSON the surface returns.
struct LogRecord {
    id:        String,
    ts:        u64,
    kind:      &'static str,
    agent:     String,
    task:      String,
    parent:    i64,
    version:   u64,
    delta_ref: String,
    note:      String,
}

impl LogRecord {

    /// Serialise to a compact single-line JSON object.
    fn to_json(&self) -> String {
        fmt!(
            "{{\"id\":\"{}\",\"ts\":{},\"kind\":\"{}\",\"agent\":\"{}\",\
              \"task\":\"{}\",\"parent_brief_version\":{},\
              \"brief_version\":{},\"delta_ref\":\"{}\",\"note\":\"{}\"}}",
            json_escape(&self.id), self.ts, self.kind, json_escape(&self.agent),
            json_escape(&self.task), self.parent, self.version,
            json_escape(&self.delta_ref), json_escape(&self.note),
        )
    }
}

/// Append a record to a Focus's log.
///
/// OPFS exposes whole-file writes only, so the append is read-modify-write
/// (single-user, single-Focus makes that safe for this stage; the
/// synchronous single-writer WAL is deferred).
async fn append_log(id: &str, rec: &LogRecord) -> Outcome<()> {
    let path = log_path(id);
    let mut buf = match opfs::exists(&path).await {
        Ok(true) => {
            let bytes = res!(opfs::read_file(&path).await);
            String::from_utf8_lossy(&bytes).to_string()
        }
        _ => String::new(),
    };
    buf.push_str(&rec.to_json());
    buf.push('\n');
    opfs::write_file(&path, buf.as_bytes()).await
}


// ┌───────────────────────────────────────────────────────────────┐
// │ Focus operations                                               │
// └───────────────────────────────────────────────────────────────┘

/// Create a Focus: its directory, an empty `brief.md`, version `0000`, a
/// `meta.json`, and a `create` log record.  Returns the new Focus id.
pub async fn create(name: &str) -> Outcome<String> {
    let id = generate_session_id();
    let now = now_ms() as u64;

    // Empty brief plus its version-0 snapshot.
    res!(opfs::write_file(&brief_path(&id), b"").await);
    res!(opfs::write_file(&version_path(&id, 0), b"").await);

    let meta = Meta { name: name.to_string(), version: 0, updated: now };
    res!(write_meta(&id, &meta).await);

    let rec = LogRecord {
        id:        generate_session_id(),
        ts:        now,
        kind:      "create",
        agent:     "user".to_string(),
        task:      "create focus".to_string(),
        parent:    -1,
        version:   0,
        delta_ref: String::new(),
        note:      name.to_string(),
    };
    res!(append_log(&id, &rec).await);
    Ok(id)
}

/// List every Focus, returning a JSON array of
/// `{ id, name, brief_version, updated }` ordered by most-recently
/// updated first.
pub async fn list() -> Outcome<String> {
    // A missing `foci/` root simply means no Foci yet.
    let entries = match opfs::list_dir("foci").await {
        Ok(e)  => e,
        Err(_) => return Ok("[]".to_string()),
    };
    let mut rows: Vec<(String, String, u64, u64)> = Vec::new();
    for (name, is_dir, _size) in entries {
        if !is_dir {
            continue;
        }
        let meta = match read_meta(&name).await {
            Ok(m)  => m,
            Err(_) => continue, // not a Focus dir / no metadata
        };
        rows.push((name, meta.name, meta.version, meta.updated));
    }
    // Most-recently updated first.
    rows.sort_by(|a, b| b.3.cmp(&a.3));
    let items: Vec<String> = rows.iter().map(|(id, nm, ver, upd)| {
        fmt!(
            "{{\"id\":\"{}\",\"name\":\"{}\",\"brief_version\":{},\"updated\":{}}}",
            json_escape(id), json_escape(nm), ver, upd,
        )
    }).collect();
    Ok(fmt!("[{}]", items.join(",")))
}

/// Read a Focus's current brief markdown.
pub async fn read_brief(id: &str) -> Outcome<String> {
    let bytes = res!(opfs::read_file(&brief_path(id)).await);
    Ok(String::from_utf8_lossy(&bytes).to_string())
}

/// Snapshot a new brief version and return its number.
///
/// Writes `brief.md`, bumps the version, writes the `versions/NNNN.md`
/// snapshot and updates `meta.json`.  The caller appends the matching log
/// record.
async fn snapshot(id: &str, md: &str, now: u64) -> Outcome<u64> {
    let mut meta = res!(read_meta(id).await);
    let next = meta.version + 1;
    res!(opfs::write_file(&brief_path(id), md.as_bytes()).await);
    res!(opfs::write_file(&version_path(id, next), md.as_bytes()).await);
    meta.version = next;
    meta.updated = now;
    res!(write_meta(id, &meta).await);
    Ok(next)
}

/// Apply a user hand-edit to the brief: snapshot a new version and log an
/// `edit` record.
pub async fn write_brief(id: &str, md: &str) -> Outcome<()> {
    let now = now_ms() as u64;
    let parent = res!(read_meta(id).await).version;
    let version = res!(snapshot(id, md, now).await);
    let rec = LogRecord {
        id:        generate_session_id(),
        ts:        now,
        kind:      "edit",
        agent:     "user".to_string(),
        task:      "edit brief".to_string(),
        parent:    parent as i64,
        version:   version,
        delta_ref: String::new(),
        note:      String::new(),
    };
    append_log(id, &rec).await
}

/// Record a brief change made by the brief agent (a steer that edited
/// `brief.md`): snapshot a version and log an `edit` record whose task is
/// the instruction.  Called by [`crate::wasm::app`] after the agent turn,
/// only when the brief content actually changed.
pub async fn record_steer(id: &str, md: &str, instruction: &str) -> Outcome<()> {
    let now = now_ms() as u64;
    let parent = res!(read_meta(id).await).version;
    let version = res!(snapshot(id, md, now).await);
    let rec = LogRecord {
        id:        generate_session_id(),
        ts:        now,
        kind:      "edit",
        agent:     "brief-agent".to_string(),
        task:      instruction.to_string(),
        parent:    parent as i64,
        version:   version,
        delta_ref: String::new(),
        note:      String::new(),
    };
    append_log(id, &rec).await
}

/// Apply a confirmed fold: write the new brief, snapshot a version, store
/// the raw delta under `.red/deltas/`, and append a `fold` record that
/// references the stored delta.  Advisory-fold discipline: this runs only
/// after the user accepts the proposed brief; the raw delta is always
/// retained.
pub async fn fold_apply(id: &str, new_brief: &str, delta: &str, note: &str) -> Outcome<()> {
    let now = now_ms() as u64;
    let parent = res!(read_meta(id).await).version;
    let version = res!(snapshot(id, new_brief, now).await);

    // Retain the raw delta, referenced by the log record.
    let dref = delta_path(id, version);
    res!(opfs::write_file(&dref, delta.as_bytes()).await);

    let rec = LogRecord {
        id:        generate_session_id(),
        ts:        now,
        kind:      "fold",
        agent:     "reducer".to_string(),
        task:      "fold delta".to_string(),
        parent:    parent as i64,
        version:   version,
        delta_ref: dref,
        note:      note.to_string(),
    };
    append_log(id, &rec).await
}

/// Read a Focus's log as a JSON array of records (each stored line is
/// already a JSON object).
pub async fn log_read(id: &str) -> Outcome<String> {
    let path = log_path(id);
    let bytes = match opfs::exists(&path).await {
        Ok(true) => res!(opfs::read_file(&path).await),
        _        => return Ok("[]".to_string()),
    };
    let text = String::from_utf8_lossy(&bytes);
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    Ok(fmt!("[{}]", lines.join(",")))
}
