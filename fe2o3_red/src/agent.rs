//! Agent loop — the core Red agent that drives conversations.
//!
//! Receives a user message, sends it to the LLM with conversation
//! history, streams the response back to the client via events,
//! and stores the exchange in the session.

use oxedyne_fe2o3_core::prelude::*;

use crate::llm::LlmClient;
use crate::protocol::{AgentEvent, ChatMessage, Session};
use crate::tools::ToolRegistry;

// The TLS client-config helper below is native-only; the wasm build
// delegates TLS trust to the browser and constructs `LlmClient`
// without a `ClientConfig`.
#[cfg(not(target_arch = "wasm32"))]
use std::sync::Arc;
#[cfg(not(target_arch = "wasm32"))]
use tokio_rustls::rustls::ClientConfig;

/// Upper bound on tool-call rounds in a single turn, to bound cost and
/// prevent a model from looping on tools indefinitely.
const MAX_TOOL_ROUNDS: usize = 25;


// ┌───────────────────────────────────────────────────────────────┐
// │ Agent                                                          │
// └───────────────────────────────────────────────────────────────┘

/// The Red agent — drives a single conversation turn.
///
/// Holds a reference to the LLM client (shared across sessions) and
/// the system prompt to prepend to every conversation.
#[derive(Clone, Debug)]
pub struct Agent {
    pub llm:           LlmClient,
    pub system_prompt: String,
}

impl Agent {

    pub fn new(llm: LlmClient, system_prompt: &str) -> Self {
        Self {
            llm,
            system_prompt: system_prompt.to_string(),
        }
    }

    /// Run a single agent turn.
    ///
    /// 1. Append the user message to the session.
    /// 2. Build the LLM request: system prompt + conversation history.
    /// 3. Call the LLM with streaming.
    /// 4. Stream tokens back to the caller via `on_event`.
    /// 5. Append the assistant response to the session.
    /// 6. Emit `Done`.
    pub async fn run_turn(
        &self,
        session:    &mut Session,
        user_msg:   String,
        registry:   &ToolRegistry,
        on_event:   &mut impl FnMut(AgentEvent),
    ) -> Outcome<()> {
        // Append the user message to the persisted history.
        session.messages.push(ChatMessage::User { content: user_msg });

        // Build the working conversation: system prompt + history.
        let mut working = Vec::with_capacity(session.messages.len() + 1);
        if !self.system_prompt.is_empty() {
            let mut sys = self.system_prompt.clone();
            if !registry.is_empty() {
                sys.push_str(
                    "\n\nYou have tools to read, write, edit, list, search and \
                     delete files, and to run shell commands, all within the \
                     user's workspace directory. Use them to inspect and modify \
                     the workspace when completing a task.");
            }
            working.push(ChatMessage::System { content: sys });
        }
        working.extend(session.messages.iter().cloned());

        if registry.is_empty() {
            return self.run_streaming(session, working, on_event).await;
        }
        self.run_tool_loop(session, working, registry, on_event).await
    }

    /// Pure-chat path: stream tokens as they arrive (no tools).
    async fn run_streaming(
        &self,
        session:    &mut Session,
        working:    Vec<ChatMessage>,
        on_event:   &mut impl FnMut(AgentEvent),
    ) -> Outcome<()> {
        let mut full = String::new();
        let result = self.llm.chat_stream(
            &working,
            &mut |token| {
                full.push_str(token);
                on_event(AgentEvent::Text(token.to_string()));
            },
        ).await;
        match result {
            Ok(resp) => {
                let content = if resp.content.is_empty() { full } else { resp.content };
                session.messages.push(ChatMessage::Assistant { content, tool_calls: Vec::new() });
                session.prompt_tokens += resp.prompt_tokens;
                session.completion_tokens += resp.completion_tokens;
                if resp.prompt_tokens > 0 { session.last_prompt_tokens = resp.prompt_tokens; }
                on_event(AgentEvent::Done);
                Ok(())
            }
            Err(e) => {
                on_event(AgentEvent::Error(e.to_string()));
                Err(e)
            }
        }
    }

    /// Agentic path: non-streaming request/response, executing tool
    /// calls between rounds until the model returns a final answer.
    /// Only the user turn and the final assistant text are persisted;
    /// the intermediate tool exchange stays within the working vec.
    async fn run_tool_loop(
        &self,
        session:    &mut Session,
        mut working: Vec<ChatMessage>,
        registry:   &ToolRegistry,
        on_event:   &mut impl FnMut(AgentEvent),
    ) -> Outcome<()> {
        let tools_json = registry.definitions_json();
        for _ in 0..MAX_TOOL_ROUNDS {
            let resp = match self.llm.chat_once(&working, tools_json.as_deref()).await {
                Ok(r) => r,
                Err(e) => { on_event(AgentEvent::Error(e.to_string())); return Err(e); }
            };
            session.prompt_tokens += resp.prompt_tokens;
            session.completion_tokens += resp.completion_tokens;
            if resp.prompt_tokens > 0 { session.last_prompt_tokens = resp.prompt_tokens; }

            if resp.tool_calls.is_empty() {
                // Final answer.
                on_event(AgentEvent::Text(resp.content.clone()));
                session.messages.push(ChatMessage::Assistant {
                    content: resp.content, tool_calls: Vec::new(),
                });
                on_event(AgentEvent::Done);
                return Ok(());
            }

            // Interim assistant text alongside tool calls (uncommon).
            if !resp.content.is_empty() {
                on_event(AgentEvent::Text(resp.content.clone()));
            }
            working.push(ChatMessage::Assistant {
                content: resp.content.clone(),
                tool_calls: resp.tool_calls.clone(),
            });

            // Execute each requested tool call.
            for tc in &resp.tool_calls {
                on_event(AgentEvent::ToolCall { name: tc.name.clone(), args: tc.arguments.clone() });
                let result = registry.dispatch(&tc.name, &tc.arguments).await;
                on_event(AgentEvent::ToolResult { name: tc.name.clone(), result: result.clone() });
                working.push(ChatMessage::Tool { tool_call_id: tc.id.clone(), content: result });
            }
        }

        // Exceeded the tool-round budget.
        let msg = fmt!("Reached the tool-call round limit ({}).", MAX_TOOL_ROUNDS);
        on_event(AgentEvent::Error(msg.clone()));
        session.messages.push(ChatMessage::Assistant {
            content: fmt!("[{}]", msg), tool_calls: Vec::new(),
        });
        on_event(AgentEvent::Done);
        Ok(())
    }
}


// ┌───────────────────────────────────────────────────────────────┐
// │ TLS config helper                                              │
// └───────────────────────────────────────────────────────────────┘

/// Build a TLS client config using the system CA bundle.
///
/// Reused from Steel's `build_outbound_tls_client` — same approach
/// but kept here so `fe2o3_red` can be used standalone.
#[cfg(not(target_arch = "wasm32"))]
pub fn build_tls_client_config() -> Outcome<Arc<ClientConfig>> {
    use tokio_rustls::rustls::{
        ClientConfig,
        RootCertStore,
        pki_types::CertificateDer,
    };

    let ca_paths = [
        "/etc/ssl/certs/ca-certificates.crt",
        "/etc/pki/tls/certs/ca-bundle.crt",
        "/etc/ssl/cert.pem",
    ];
    let ca_file = match ca_paths.iter().find(|p| std::path::Path::new(p).exists()) {
        Some(p) => *p,
        None => return Err(err!(
            "No system CA bundle found."; Init, Missing, File)),
    };

    let pem_data = match std::fs::read(ca_file) {
        Ok(d) => d,
        Err(e) => return Err(err!(e, "Failed to read CA bundle."; File, Read)),
    };

    let mut roots = RootCertStore::empty();
    let certs: Vec<CertificateDer> = rustls_pemfile::certs(&mut pem_data.as_slice())
        .filter_map(|c| c.ok())
        .map(CertificateDer::from)
        .collect();
    for cert in certs {
        let _ = roots.add(cert);
    }

    let mut config = ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();
    // Advertise HTTP/1.1 via ALPN so CDN-fronted servers (e.g.
    // Fireworks.ai behind Cloudflare) don't close the connection
    // after the TLS handshake when no protocol is negotiated.
    config.alpn_protocols = vec![b"http/1.1".to_vec()];

    Ok(Arc::new(config))
}


// ┌───────────────────────────────────────────────────────────────┐
// │ Tests                                                          │
// └───────────────────────────────────────────────────────────────┘

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::LlmClient;

    fn make_test_agent() -> Agent {
        let tls = build_test_tls_config();
        let llm = LlmClient::new("api.test.com", 443, "/v1/chat", "key", "model", 4096, tls);
        Agent::new(llm, "You are Red, an AI assistant.")
    }

    fn build_test_tls_config() -> Arc<ClientConfig> {
        use rustls::crypto::ring;
        let _ = ring::default_provider().install_default();
        ClientConfig::builder()
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(crate::llm::tests::NoVerify))
            .with_no_client_auth().into()
    }

    #[test]
    fn test_agent_creation() {
        let agent = make_test_agent();
        assert_eq!(agent.system_prompt, "You are Red, an AI assistant.");
    }

    #[test]
fn test_agent_message_building() {
        let mut session = Session::new("s1".to_string(), "Test".to_string(), "model".to_string());
        session.messages.push(ChatMessage::User { content: "Hello".to_string() });
        assert_eq!(session.messages.len(), 1);
        assert_eq!(session.messages[0].role(), "user");
        assert_eq!(session.messages[0].content(), "Hello");
    }
}
