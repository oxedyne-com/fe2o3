//! Agent loop — the core Red agent that drives conversations.
//!
//! Receives a user message, sends it to the LLM with conversation
//! history, streams the response back to the client via events,
//! and stores the exchange in the session.

use oxedyne_fe2o3_core::prelude::*;

use crate::llm::LlmClient;
use crate::protocol::{AgentEvent, ChatMessage, Session};

use std::sync::Arc;
use tokio_rustls::rustls::ClientConfig;


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
        on_event:   &mut impl FnMut(AgentEvent),
    ) -> Outcome<()> {
        // 1. Append user message.
        session.messages.push(ChatMessage::User {
            content: user_msg.clone(),
        });

        // 2. Build messages for the LLM (system prompt + history).
        let mut llm_messages = Vec::with_capacity(session.messages.len() + 1);
        if !self.system_prompt.is_empty() {
            llm_messages.push(ChatMessage::System {
                content: self.system_prompt.clone(),
            });
        }
        llm_messages.extend(session.messages.iter().cloned());

        // 3. Call LLM with streaming.
        let mut full_response = String::new();
        let result = self.llm.chat_stream(
            &llm_messages,
            &mut |token| {
                full_response.push_str(token);
                on_event(AgentEvent::Text(token.to_string()));
            },
        ).await;

        match result {
            Ok(response) => {
                // 5. Append assistant response.
                let content = if response.content.is_empty() {
                    full_response
                } else {
                    response.content
                };
                session.messages.push(ChatMessage::Assistant {
                    content: content.clone(),
                });

                // 6. Emit Done.
                on_event(AgentEvent::Done);
                Ok(())
            }
            Err(e) => {
                on_event(AgentEvent::Error(e.to_string()));
                Err(e)
            }
        }
    }
}


// ┌───────────────────────────────────────────────────────────────┐
// │ TLS config helper                                              │
// └───────────────────────────────────────────────────────────────┘

/// Build a TLS client config using the system CA bundle.
///
/// Reused from Steel's `build_outbound_tls_client` — same approach
/// but kept here so `fe2o3_red` can be used standalone.
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
        let llm = LlmClient::new("api.test.com", 443, "/v1/chat", "key", "model", tls);
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
