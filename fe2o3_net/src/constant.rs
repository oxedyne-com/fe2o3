// HTTP
pub const HTTP_DEFAULT_HEADER_CHUNK_SIZE:       usize = 1_500;
pub const HTTP_DEFAULT_BODY_CHUNK_SIZE:         usize = 5_000;

/// Bytes copied from a file to the socket at a time when a message body is a
/// window of a file rather than a buffer.
///
/// Larger than the body chunk above because this one is a bulk transfer, not a
/// protocol read: a gigabyte of video at five kilobytes a write is two hundred
/// thousand syscalls, and as many TLS records.
pub const HTTP_FILE_BODY_CHUNK_SIZE:            usize = 65_536;

/// Most that is reserved up front for a message body on the strength of its `Content-Length`.
///
/// A body larger than this is still read, in the chunks the read loop already uses; the vector
/// simply grows into it. What the ceiling prevents is a single allocation sized by a number a
/// stranger wrote, which is free to write and not free to honour. One mebibyte covers the bodies
/// that actually arrive, so the reservation is doing its job in every ordinary case.
pub const HTTP_BODY_RESERVE_MAX:                usize = 1_048_576;

pub const HTTP_HEADER_MAX_MULTILINES:           u8 = 10;
pub const HTTP_HEADER_MAX_FIELDS:               u16 = 100;
pub const HTTP_BODY_BYTES_MAX_VIEW:             usize = 300;
pub const SESSION_ID_KEY_LABEL:                 &'static str = "session_id";

// SMTP
//pub const SMTP_READ_BUFFER_SIZE:                usize = 10;//1_024;

// WebSocket
pub const WEBSOCKET_GUID:                       &'static str = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";
pub const WEBSOCKET_LATENCY_HISTORY_SIZE:       usize = 540; // 3 hrs @ 30 s intervals

/// Default bound on the payload one websocket frame may declare, in bytes.
///
/// A frame header states its own payload length in as much as 64 bits, and the reader must have
/// somewhere to put those bytes before it can read them, so a reader that believes the number
/// allocates whatever a stranger asks it to. Sixteen mebibytes is the same default the widely
/// deployed Rust implementation uses, is far above any framing produced by a browser sending a
/// message of ordinary size, and is small enough that a machine can absorb several at once.
///
/// An application whose peers legitimately send single frames larger than this raises the bound
/// rather than removing it -- see [`crate::ws::WebSocketLimits`].
pub const WEBSOCKET_MAX_FRAME_BYTES:            usize = 16 * 1_048_576;

/// Default bound on the payload a whole websocket message may reach once its frames are joined,
/// in bytes.
///
/// The frame bound alone bounds nothing: a message may arrive as any number of continuation
/// frames, each of them under the frame bound, and the reader joins them all in one buffer. Four
/// times the frame bound, so that a fragmented message has room to be several frames long while
/// the total a peer can pin remains a number chosen here rather than one chosen by the peer.
pub const WEBSOCKET_MAX_MESSAGE_BYTES:          usize = 64 * 1_048_576;

/// Largest payload a websocket control frame may carry, in bytes, as RFC 6455 §5.5 fixes it.
///
/// Not configurable, because it is not ours to choose: a control frame declaring more than this is
/// malformed whatever an application would like to allow.
pub const WEBSOCKET_MAX_CONTROL_FRAME_BYTES:    u64 = 125;

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
