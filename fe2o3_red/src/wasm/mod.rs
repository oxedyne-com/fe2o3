//! Browser (wasm32) runtime surface for Red.
//!
//! This module tree is the bridge between JavaScript and Red's
//! target-agnostic core.  It is compiled only for `wasm32` and never
//! links into the native build.
//!
//! - [`entry`] — the `#[wasm_bindgen]` API exposed to JS: a core-init
//!   probe, an OPFS read/write pair, and an LLM transport probe.
//! - [`app`] — the [`RedApp`](app::RedApp) agent surface: runs a real
//!   [`Agent`](crate::agent::Agent) turn and streams
//!   [`AgentEvent`](crate::protocol::AgentEvent)s to a JS callback, and
//!   hosts the Focus / brief / fold surface.
//! - [`focus`] — the Focus / brief / fold substrate: the OPFS layout and
//!   store operations behind the durable brief and the advisory fold.
//! - [`opfs`] — an async filesystem edge over the Origin Private File
//!   System (OPFS), reached through `navigator.storage.getDirectory()`.
//!
//! The synchronous single-writer OPFS path (`createSyncAccessHandle` in
//! a dedicated Worker, needed for the append-only `.red` log) is
//! deferred; the main-thread async path here is sufficient for the
//! first browser vertical.

pub mod app;
pub mod entry;
pub mod focus;
pub mod opfs;

use oxedyne_fe2o3_core::prelude::*;

use wasm_bindgen::JsValue;

/// Render a JS error value as a human-readable string.
pub(crate) fn js_str(v: &JsValue) -> String {
    v.as_string().unwrap_or_else(|| fmt!("{:?}", v))
}

/// Map a Red [`Error`] into a `JsValue` suitable for rejecting a
/// `Promise`, stringifying the full error (message plus tags) so the
/// browser console and the harness DOM see the real cause.
pub(crate) fn to_js_err(e: Error<ErrTag>) -> JsValue {
    JsValue::from_str(&fmt!("{}", e))
}
