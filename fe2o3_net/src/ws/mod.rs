#[cfg(feature = "async")]
pub mod client;
#[cfg(feature = "async")]
pub mod core;
#[cfg(feature = "async")]
pub mod handler;
pub mod status;

#[cfg(feature = "async")]
pub use self::core::{
    accept_key,
    accept_response,
    connect_request,
    encode_message,
    read_frame,
    read_message,
    WebSocket,
    WebSocketFrame,
    WebSocketLimits,
    WebSocketMessage,
};
#[cfg(feature = "async")]
pub use self::client::WsClient;
