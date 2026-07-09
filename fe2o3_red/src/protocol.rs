//! WS protocol types for Red — JDAT serialisation.
//!
//! All messages between the browser and Steel use the existing syntax
//! protocol with JDAT values.  This module defines the command and
//! response types and their JDAT conversion functions.

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_jdat::prelude::*;


// ┌───────────────────────────────────────────────────────────────┐
// │ Chat messages                                                  │
// └───────────────────────────────────────────────────────────────┘

/// A single tool call requested by the assistant.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ToolCall {
    pub id:        String,
    pub name:      String,
    /// Raw JSON arguments object as produced by the model.
    pub arguments: String,
}

/// A single message in a conversation, mirroring the OpenAI API format.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ChatMessage {
    System { content: String },
    User { content: String },
    /// Assistant turn.  `tool_calls` is populated only for the
    /// in-flight working conversation; persisted assistant messages
    /// carry the final text with no tool calls.
    Assistant { content: String, tool_calls: Vec<ToolCall> },
    /// Tool call result returned to the LLM.
    Tool { tool_call_id: String, content: String },
}

impl ChatMessage {

    /// Serialise to a JDAT map for the LLM API request body.
    ///
    /// Produces maps like:
    ///   { "role": "system", "content": "..." }
    ///   { "role": "user", "content": "..." }
    ///   { "role": "assistant", "content": "..." }
    ///   { "role": "tool", "tool_call_id": "...", "content": "..." }
    pub fn to_datmap(&self) -> DaticleMap {
        let mut m = DaticleMap::new();
        match self {
            Self::System { content } => {
                m.insert(dat!("role"), dat!("system"));
                m.insert(dat!("content"), dat!(content.clone()));
            }
            Self::User { content } => {
                m.insert(dat!("role"), dat!("user"));
                m.insert(dat!("content"), dat!(content.clone()));
            }
            Self::Assistant { content, .. } => {
                m.insert(dat!("role"), dat!("assistant"));
                m.insert(dat!("content"), dat!(content.clone()));
            }
            Self::Tool { tool_call_id, content } => {
                m.insert(dat!("role"), dat!("tool"));
                m.insert(dat!("tool_call_id"), dat!(tool_call_id.clone()));
                m.insert(dat!("content"), dat!(content.clone()));
            }
        }
        m
    }

    /// Deserialise from a JDAT map.
    pub fn from_datmap(m: &DaticleMap) -> Outcome<Self> {
        let role = match m.get(&dat!("role")) {
            Some(Dat::Str(s)) => s.clone(),
            _ => return Err(err!("ChatMessage: missing 'role'."; Invalid, Input)),
        };
        let content = match m.get(&dat!("content")) {
            Some(Dat::Str(s)) => s.clone(),
            _ => return Err(err!("ChatMessage: missing 'content'."; Invalid, Input)),
        };
        match role.as_str() {
            "system" => Ok(Self::System { content }),
            "user" => Ok(Self::User { content }),
            "assistant" => Ok(Self::Assistant { content, tool_calls: Vec::new() }),
            "tool" => {
                let tool_call_id = match m.get(&dat!("tool_call_id")) {
                    Some(Dat::Str(s)) => s.clone(),
                    _ => return Err(err!("ChatMessage: tool missing 'tool_call_id'."; Invalid, Input)),
                };
                Ok(Self::Tool { tool_call_id, content })
            }
            _ => Err(err!("ChatMessage: unknown role '{}'.", role; Invalid, Input)),
        }
    }

    pub fn role(&self) -> &'static str {
        match self {
            Self::System { .. } => "system",
            Self::User { .. } => "user",
            Self::Assistant { .. } => "assistant",
            Self::Tool { .. } => "tool",
        }
    }

    pub fn content(&self) -> &str {
        match self {
            Self::System { content }
            | Self::User { content }
            | Self::Assistant { content, .. }
            | Self::Tool { content, .. } => content,
        }
    }
}


// ┌───────────────────────────────────────────────────────────────┐
// │ Session                                                        │
// └───────────────────────────────────────────────────────────────┘

/// A chat session belonging to a user.
#[derive(Clone, Debug)]
pub struct Session {
    pub id:                  String,
    pub name:                String,
    pub created_at:          u64,
    pub model:               String,
    pub messages:            Vec<ChatMessage>,
    pub prompt_tokens:       u64,
    pub completion_tokens:   u64,
}

impl Session {

    pub fn new(id: String, name: String, model: String) -> Self {
        Self {
            id,
            name,
            created_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0),
            model,
            messages: Vec::new(),
            prompt_tokens: 0,
            completion_tokens: 0,
        }
    }

    /// Serialise metadata (without messages) to a JDAT map.
    pub fn to_meta_datmap(&self) -> DaticleMap {
        let mut m = DaticleMap::new();
        m.insert(dat!("id"), dat!(self.id.clone()));
        m.insert(dat!("name"), dat!(self.name.clone()));
        m.insert(dat!("created_at"), Dat::U64(self.created_at));
        m.insert(dat!("model"), dat!(self.model.clone()));
        m.insert(dat!("prompt_tokens"), Dat::U64(self.prompt_tokens));
        m.insert(dat!("completion_tokens"), Dat::U64(self.completion_tokens));
        m
    }

    /// Serialise full session (with messages) to a JDAT map.
    pub fn to_datmap(&self) -> DaticleMap {
        let mut m = self.to_meta_datmap();
        let msgs: Vec<Dat> = self.messages.iter()
            .map(|msg| Dat::Map(msg.to_datmap()))
            .collect();
        m.insert(dat!("messages"), Dat::List(msgs));
        m
    }

    /// Deserialise from a JDAT map.
    pub fn from_datmap(m: &DaticleMap) -> Outcome<Self> {
        let id = match m.get(&dat!("id")) {
            Some(Dat::Str(s)) => s.clone(),
            _ => return Err(err!("Session: missing 'id'."; Invalid, Input)),
        };
        let name = match m.get(&dat!("name")) {
            Some(Dat::Str(s)) => s.clone(),
            _ => return Err(err!("Session: missing 'name'."; Invalid, Input)),
        };
        let created_at = match m.get(&dat!("created_at")) {
            Some(Dat::U64(n)) => *n,
            _ => 0,
        };
        let model = match m.get(&dat!("model")) {
            Some(Dat::Str(s)) => s.clone(),
            _ => String::new(),
        };
        let messages = match m.get(&dat!("messages")) {
            Some(Dat::List(list)) => {
                let mut msgs = Vec::new();
                for item in list {
                    if let Dat::Map(msg_m) = item {
                        msgs.push(res!(ChatMessage::from_datmap(msg_m)));
                    }
                }
                msgs
            }
            _ => Vec::new(),
        };
        let prompt_tokens = match m.get(&dat!("prompt_tokens")) {
            Some(Dat::U64(n)) => *n,
            _ => 0,
        };
        let completion_tokens = match m.get(&dat!("completion_tokens")) {
            Some(Dat::U64(n)) => *n,
            _ => 0,
        };
        Ok(Self { id, name, created_at, model, messages, prompt_tokens, completion_tokens })
    }
}


// ┌───────────────────────────────────────────────────────────────┐
// │ User configuration                                             │
// └───────────────────────────────────────────────────────────────┘

/// Per-user configuration stored in O3db.
///
/// Supports multi-user with individual model selection — the foundation
/// for a future commercial offering with billing.
#[derive(Clone, Debug)]
pub struct UserConfig {
    pub username:       String,
    pub default_model:  String,
    pub created_at:     u64,
}

impl UserConfig {

    pub fn new(username: String, default_model: String) -> Self {
        Self {
            username,
            default_model,
            created_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0),
        }
    }

    pub fn to_datmap(&self) -> DaticleMap {
        let mut m = DaticleMap::new();
        m.insert(dat!("username"), dat!(self.username.clone()));
        m.insert(dat!("default_model"), dat!(self.default_model.clone()));
        m.insert(dat!("created_at"), Dat::U64(self.created_at));
        m
    }

    pub fn from_datmap(m: &DaticleMap) -> Outcome<Self> {
        let username = match m.get(&dat!("username")) {
            Some(Dat::Str(s)) => s.clone(),
            _ => return Err(err!("UserConfig: missing 'username'."; Invalid, Input)),
        };
        let default_model = match m.get(&dat!("default_model")) {
            Some(Dat::Str(s)) => s.clone(),
            _ => String::new(),
        };
        let created_at = match m.get(&dat!("created_at")) {
            Some(Dat::U64(n)) => *n,
            _ => 0,
        };
        Ok(Self { username, default_model, created_at })
    }
}


// ┌───────────────────────────────────────────────────────────────┐
// │ Agent events                                                   │
// └───────────────────────────────────────────────────────────────┘

/// Events emitted by the agent loop, sent to the client over WS.
#[derive(Clone, Debug)]
pub enum AgentEvent {
    /// Streamed LLM response text (a token or chunk).
    Text(String),
    /// The agent is invoking a tool (name + raw JSON args).
    ToolCall { name: String, args: String },
    /// A tool returned a result (name + result text).
    ToolResult { name: String, result: String },
    /// Agent turn complete.
    Done,
    /// Error occurred.
    Error(String),
}

impl AgentEvent {

    /// Convert to a JDAT map suitable for a WS `data` response.
    pub fn to_datmap(&self) -> DaticleMap {
        let mut m = DaticleMap::new();
        match self {
            Self::Text(text) => {
                m.insert(dat!("type"), dat!("text"));
                m.insert(dat!("content"), dat!(text.clone()));
            }
            Self::ToolCall { name, args } => {
                m.insert(dat!("type"), dat!("tool_call"));
                m.insert(dat!("name"), dat!(name.clone()));
                m.insert(dat!("args"), dat!(args.clone()));
            }
            Self::ToolResult { name, result } => {
                m.insert(dat!("type"), dat!("tool_result"));
                m.insert(dat!("name"), dat!(name.clone()));
                m.insert(dat!("content"), dat!(result.clone()));
            }
            Self::Done => {
                m.insert(dat!("type"), dat!("done"));
            }
            Self::Error(msg) => {
                m.insert(dat!("type"), dat!("error"));
                m.insert(dat!("content"), dat!(msg.clone()));
            }
        }
        m
    }
}


// ┌───────────────────────────────────────────────────────────────┐
// │ O3db key helpers                                               │
// └───────────────────────────────────────────────────────────────┘

/// Build the O3db key for a user's session list.
pub fn sessions_key(username: &str) -> Dat {
    Dat::Str(fmt!("red:{}:sessions", username))
}

/// Build the O3db key for a specific session.
pub fn session_key(session_id: &str) -> Dat {
    Dat::Str(fmt!("red:session:{}", session_id))
}

/// Build the O3db key for a user's config.
pub fn user_config_key(username: &str) -> Dat {
    Dat::Str(fmt!("red:user:{}", username))
}

/// Generate a unique session ID (8 hex chars from timestamp + counter).
pub fn generate_session_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    fmt!("{:x}{:x}", ts, n)
}


// ┌───────────────────────────────────────────────────────────────┐
// │ Tests                                                          │
// └───────────────────────────────────────────────────────────────┘

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chat_message_roundtrip() {
        let msg = ChatMessage::User { content: "Hello".to_string() };
        let dm = msg.to_datmap();
        let msg2 = ChatMessage::from_datmap(&dm).unwrap();
        assert_eq!(msg, msg2);
    }

    #[test]
    fn test_chat_message_tool_roundtrip() {
        let msg = ChatMessage::Tool {
            tool_call_id: "call_123".to_string(),
            content: "42".to_string(),
        };
        let dm = msg.to_datmap();
        let msg2 = ChatMessage::from_datmap(&dm).unwrap();
        assert_eq!(msg, msg2);
    }

    #[test]
    fn test_session_roundtrip() {
        let mut s = Session::new("s1".to_string(), "Test".to_string(), "glm-5p2".to_string());
        s.messages.push(ChatMessage::User { content: "Hi".to_string() });
        s.messages.push(ChatMessage::Assistant { content: "Hello!".to_string(), tool_calls: Vec::new() });
        let dm = s.to_datmap();
        let s2 = Session::from_datmap(&dm).unwrap();
        assert_eq!(s.id, s2.id);
        assert_eq!(s.name, s2.name);
        assert_eq!(s.model, s2.model);
        assert_eq!(s.messages.len(), s2.messages.len());
        assert_eq!(s.messages[0], s2.messages[0]);
        assert_eq!(s.messages[1], s2.messages[1]);
    }

    #[test]
    fn test_user_config_roundtrip() {
        let uc = UserConfig::new("jason".to_string(), "glm-5p2".to_string());
        let dm = uc.to_datmap();
        let uc2 = UserConfig::from_datmap(&dm).unwrap();
        assert_eq!(uc.username, uc2.username);
        assert_eq!(uc.default_model, uc2.default_model);
    }

    #[test]
    fn test_agent_event_text() {
        let ev = AgentEvent::Text("hello".to_string());
        let dm = ev.to_datmap();
        assert_eq!(dm.get(&dat!("type")), Some(&dat!("text")));
        assert_eq!(dm.get(&dat!("content")), Some(&dat!("hello")));
    }

    #[test]
    fn test_agent_event_done() {
        let ev = AgentEvent::Done;
        let dm = ev.to_datmap();
        assert_eq!(dm.get(&dat!("type")), Some(&dat!("done")));
    }

    #[test]
    fn test_session_id_unique() {
        let id1 = generate_session_id();
        let id2 = generate_session_id();
        assert_ne!(id1, id2);
    }

    #[test]
    fn test_o3db_keys() {
        assert_eq!(sessions_key("jason"), Dat::Str("red:jason:sessions".to_string()));
        assert_eq!(session_key("s1"), Dat::Str("red:session:s1".to_string()));
        assert_eq!(user_config_key("jason"), Dat::Str("red:user:jason".to_string()));
    }
}
