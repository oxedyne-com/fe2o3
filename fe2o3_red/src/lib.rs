//! Red — AI agent and web chatbot for the Hematite Steel server.
//!
//! Red provides a web-based chat interface to an LLM (via an OpenAI-compatible
//! API) with tool-calling capability.  It replaces the tmux/PTY terminal
//! bridge approach with a structured WebSocket protocol using JDAT
//! serialisation.
//!
//! Key components:
//!
//! - [`llm`] — async LLM client with SSE streaming
//! - [`session`] — per-user session and conversation storage (O3db)
//! - [`agent`] — the agent loop: message → LLM → tools → streamed response
//! - [`protocol`] — WS message types (JDAT serialisation)
//! - [`handler`] — `WebSocketHandler` impl for Steel integration

#![forbid(unsafe_code)]

pub mod agent;
pub mod executor;
/// The Steel WebSocket handler is a native-only server concern; the
/// browser (wasm32) build drives the agent directly, so this module is
/// gated out of the wasm target.
#[cfg(not(target_arch = "wasm32"))]
pub mod handler;
pub mod llm;
pub mod protocol;
pub mod session;
pub mod skills;
pub mod syntax;
pub mod tools;
/// The browser (wasm32) entry surface — a `#[wasm_bindgen]` API plus the
/// OPFS filesystem edge.  Gated to wasm32 so the native build never sees
/// `wasm-bindgen`'s generated glue.
#[cfg(target_arch = "wasm32")]
pub mod wasm;
pub mod workspace;
