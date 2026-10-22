//! A seam for watching a websocket's messages, in both directions, at no cost when nobody is.
//!
//! A [`FrameTap`] is handed every message that crosses a socket, with a context of the caller's
//! choosing (a connection's session and device, say), and cannot change what it sees or hold it
//! up: the hooks return nothing. [`NoTap`] is the tap that is not there. It is zero-sized, so is
//! its context, and its hooks are empty and inlined, so a socket that installs none is the same
//! code as before the seam existed. The seam is a trait with a type parameter on
//! [`WebSocket`](crate::ws::core::WebSocket), not a boxed callback, so a tap that is installed is
//! called without dynamic dispatch.
//!
//! Two layers, because the crate offers two ways in. [`WebSocket`](crate::ws::core::WebSocket)
//! carries a tap and a context of its own (`with_tap`). A caller that owns its two stream halves
//! and calls [`read_message`] and [`encode_message`] itself, as an application's gateway does,
//! calls [`read_message_tapped`] and [`encode_message_tapped`] instead, passing its tap and
//! context by reference. Both layers tap a whole message, after reassembly on the way in and as
//! it is framed on the way out, which is where a caller's filter can understand it.

use crate::ws::core::{
    encode_message,
    read_message,
    WebSocketLimits,
    WebSocketMessage,
};

use oxedyne_fe2o3_core::prelude::*;

use std::sync::Arc;

use tokio::io::AsyncRead;


/// Watches the messages of a websocket. Both hooks see a message once, after the fact.
pub trait FrameTap {
    type Ctx;   // what the caller knows of the connection, handed to every call

    /// A message has been read, its frames reassembled.
    fn inbound(&self, ctx: &Self::Ctx, msg: &WebSocketMessage);

    /// A message has been framed for the wire. This is called whether or not the write that
    /// follows succeeds, since the point is to see what was sent, or tried.
    fn outbound(&self, ctx: &Self::Ctx, msg: &WebSocketMessage);
}

/// The tap that watches nothing: no bytes of its own, none in its context, and hooks that compile
/// to nothing.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoTap;

impl FrameTap for NoTap {
    type Ctx = ();

    #[inline(always)]
    fn inbound(&self, _ctx: &(), _msg: &WebSocketMessage) {}

    #[inline(always)]
    fn outbound(&self, _ctx: &(), _msg: &WebSocketMessage) {}
}

impl<X: FrameTap> FrameTap for &X {
    type Ctx = X::Ctx;

    #[inline]
    fn inbound(&self, ctx: &Self::Ctx, msg: &WebSocketMessage) { (**self).inbound(ctx, msg) }

    #[inline]
    fn outbound(&self, ctx: &Self::Ctx, msg: &WebSocketMessage) { (**self).outbound(ctx, msg) }
}

impl<X: FrameTap> FrameTap for Arc<X> {
    type Ctx = X::Ctx;

    #[inline]
    fn inbound(&self, ctx: &Self::Ctx, msg: &WebSocketMessage) { (**self).inbound(ctx, msg) }

    #[inline]
    fn outbound(&self, ctx: &Self::Ctx, msg: &WebSocketMessage) { (**self).outbound(ctx, msg) }
}

/// [`read_message`], and the message it returns is shown to `tap` as inbound. A close of the
/// connection (`Ok(None)`) and an error are not messages and are not shown.
pub async fn read_message_tapped<R: AsyncRead + Unpin, X: FrameTap>(
    stream:     &mut R,
    buffer:     &mut Vec<u8>,
    chunk_size: usize,
    limits:     WebSocketLimits,
    tap:        &X,
    ctx:        &X::Ctx,
)
    -> Outcome<Option<WebSocketMessage>>
{
    let msg = res!(read_message(stream, buffer, chunk_size, limits).await);
    if let Some(m) = &msg {
        tap.inbound(ctx, m);
    }
    Ok(msg)
}

/// [`encode_message`], and the message is shown to `tap` as outbound once it has been framed. A
/// message that cannot be framed is not shown.
pub fn encode_message_tapped<X: FrameTap>(
    message:        &WebSocketMessage,
    mask:           bool,
    chunk_size:     usize,
    chunk_thresh:   usize,
    tap:            &X,
    ctx:            &X::Ctx,
)
    -> Outcome<Vec<u8>>
{
    let byts = res!(encode_message(message, mask, chunk_size, chunk_thresh));
    tap.outbound(ctx, message);
    Ok(byts)
}


#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::atomic::{
        AtomicUsize,
        Ordering,
    };

    // Adds its context to one counter per direction, so a call to the wrong hook, or to none, shows.
    #[derive(Default)]
    struct Count {
        ins:    AtomicUsize,
        outs:   AtomicUsize,
    }

    impl FrameTap for Count {
        type Ctx = usize;

        fn inbound(&self, ctx: &usize, _msg: &WebSocketMessage) {
            self.ins.fetch_add(*ctx, Ordering::SeqCst);
        }

        fn outbound(&self, ctx: &usize, _msg: &WebSocketMessage) {
            self.outs.fetch_add(*ctx, Ordering::SeqCst);
        }
    }

    #[test]
    fn no_tap_and_its_context_occupy_no_bytes() {
        assert_eq!(std::mem::size_of::<NoTap>(), 0);
        assert_eq!(std::mem::size_of::<<NoTap as FrameTap>::Ctx>(), 0);
        // And it can be called, to no effect.
        NoTap.inbound(&(), &WebSocketMessage::Ping(Vec::new()));
        NoTap.outbound(&(), &WebSocketMessage::Pong(Vec::new()));
    }

    #[test]
    fn a_reference_and_an_arc_forward_each_hook_to_its_own_counter() {
        let c = Arc::new(Count::default());
        let m = WebSocketMessage::Text("x".to_string());
        (&*c).inbound(&2, &m);
        (&*c).outbound(&5, &m);
        c.inbound(&10, &m);
        c.outbound(&20, &m);
        assert_eq!(c.ins.load(Ordering::SeqCst), 12);
        assert_eq!(c.outs.load(Ordering::SeqCst), 25);
    }
}
