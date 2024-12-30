// HTTP
pub const HTTP_DEFAULT_HEADER_CHUNK_SIZE:       usize = 1_500;
pub const HTTP_DEFAULT_BODY_CHUNK_SIZE:         usize = 5_000;

pub const HTTP_HEADER_MAX_MULTILINES:           u8 = 10;
pub const HTTP_HEADER_MAX_FIELDS:               u16 = 100;
pub const HTTP_BODY_BYTES_MAX_VIEW:             usize = 300;
pub const SESSION_ID_KEY_LABEL:                 &'static str = "session_id";

// SMTP
//pub const SMTP_READ_BUFFER_SIZE:                usize = 10;//1_024;

// WebSocket
pub const WEBSOCKET_GUID:                       &'static str = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";
pub const WEBSOCKET_LATENCY_HISTORY_SIZE:       usize = 540; // 3 hrs @ 30 s intervals

pub const READ_LOOP_SAFETY_LIMIT:               usize = 100;

// DNS
/// List of special case domains that are valid without dots. Based on RFC 6761 and common
/// practice.
pub const SPECIAL_DOMAINS: &[&str] = &[
    "localhost",
    "invalid",     // RFC 6761
    "example",     // RFC 6761
    "test",        // RFC 6761
];
