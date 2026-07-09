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
pub mod handler;
pub mod llm;
pub mod protocol;
pub mod session;
pub mod syntax;
pub mod tools;
pub mod workspace;
