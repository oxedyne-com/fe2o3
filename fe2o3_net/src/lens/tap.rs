//! The lens's tap: a websocket's messages as rows of a [`Sink`].
//!
//! [`Tap`] is a [`FrameTap`] for [`crate::ws::tap`]'s seam. Each message, either way, goes to a
//! caller's filter along with the connection's context. The filter says whether the message is
//! worth a row, where it is filed and what of it is kept, and an Oxegen peer will use that to
//! reduce a biometric enrolment bundle to its kind, its ids and its size before anything touches
//! a disk. What the filter returns still passes through the sink's [`Redact`](super::Redact), so a
//! filter that forgets a field is not the last line of defence.
//!
//! [`Frame::shape`] is the default arm for a filter: the message's kind and byte count and
//! nothing of its content.

use super::{
    redact::{
        NoTest,
        StrTest,
    },
    row::Row,
    sink::{
        now_ms,
        Sink,
    },
};
use crate::ws::{
    core::WebSocketMessage,
    tap::FrameTap,
};

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_jdat::prelude::*;

use std::{
    marker::PhantomData,
    sync::{
        atomic::{
            AtomicU64,
            Ordering,
        },
        Arc,
    },
};


/// Which way a message was travelling.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Dir {
    In,     // read from the peer
    Out,    // sent to the peer
}

impl Dir {

    /// The row's kind, so its tag reads `ev frame.in` or `ev frame.out`.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::In    => "frame.in",
            Self::Out   => "frame.out",
        }
    }
}

/// What a filter keeps of one message: the sink key it is filed under, the row's source id and its
/// payload fields.
#[derive(Clone, Debug)]
pub struct Frame {
    pub key:    Vec<String>,            // the sink key, a session or a source
    pub d:      String,                 // the row's short source id
    pub body:   Vec<(String, Dat)>,     // payload fields, in the order they were added
}

impl Frame {

    pub fn new(key: &[&str], d: &str) -> Self {
        Self {
            key:    key.iter().map(|s| s.to_string()).collect(),
            d:      d.to_string(),
            body:   Vec::new(),
        }
    }

    /// Adds a payload field, or replaces one of the same name where it stands.
    pub fn with(mut self, name: &str, val: Dat) -> Self {
        for (k, v) in self.body.iter_mut() {
            if k == name {
                *v = val;
                return self;
            }
        }
        self.body.push((name.to_string(), val));
        self
    }

    /// Adds `op`, the kind of message, and `bytes`, its payload length, and nothing of what it
    /// says. A filter that does not recognise a message should end here.
    pub fn shape(self, msg: &WebSocketMessage) -> Self {
        let (op, bytes) = match msg {
            WebSocketMessage::Text(t)       => ("text", t.len()),
            WebSocketMessage::Binary(b)     => ("binary", b.len()),
            WebSocketMessage::Ping(b)       => ("ping", b.len()),
            WebSocketMessage::Pong(b)       => ("pong", b.len()),
            WebSocketMessage::Close(c, r)   => (
                "close",
                c.as_ref().map_or(0, |_| 2) + r.as_ref().map_or(0, |r| r.len()),
            ),
        };
        self.with("op", Dat::Str(op.to_string())).with("bytes", Dat::U64(bytes as u64))
    }
}

/// A [`FrameTap`] that writes the messages its filter keeps to a [`Sink`].
///
/// The filter is `Fn(Dir, &C, &WebSocketMessage) -> Option<Frame>` over the connection context
/// `C`; `None` drops the message. The sequence number `n` of the row envelope counts from 1 across
/// the whole tap and advances only for a message the filter kept, so a reader sees no gap. It is
/// one counter, not one per key, so it stays bounded however many connections come and go.
///
/// A write that fails is counted in [`lost`](Self::lost) and logged once, and the message flows on
/// regardless: a debug feed must never be able to stop the socket it watches. The append is a
/// blocking file write made on the task that read or sent the message. Size the sink's
/// `Gates::max_data` for the frames the filter builds, since a row over it is trimmed.
pub struct Tap<C, F, T: StrTest = NoTest> {
    sink:       Arc<Sink<T>>,
    build:      String,         // the build tag, `b` of the envelope
    filter:     F,
    n:          AtomicU64,      // the last sequence number issued
    lost:       AtomicU64,      // writes that failed
    phantom:    PhantomData<fn(C)>,
}

impl<C, F, T> Tap<C, F, T>
where
    T:  StrTest,
    F:  Fn(Dir, &C, &WebSocketMessage) -> Option<Frame> + Send + Sync,
{
    pub fn new(sink: Arc<Sink<T>>, build: &str, filter: F) -> Self {
        Self {
            sink,
            build:      build.to_string(),
            filter,
            n:          AtomicU64::new(0),
            lost:       AtomicU64::new(0),
            phantom:    PhantomData,
        }
    }

    /// How many rows could not be written.
    pub fn lost(&self) -> u64 {
        self.lost.load(Ordering::Relaxed)
    }

    fn take(&self, dir: Dir, ctx: &C, msg: &WebSocketMessage) {
        let frame = match (self.filter)(dir, ctx, msg) {
            Some(frame) => frame,
            None        => return,
        };
        // Daimond's `nextSeq` is incremented before it is used, so the first row is 1.
        let n = self.n.fetch_add(1, Ordering::Relaxed) + 1;
        let t = now_ms().unwrap_or(0);
        let mut row = Row::new(&frame.d, n, &self.build, t, dir.kind());
        for (k, v) in frame.body {
            row = row.with(&k, v);
        }
        let key: Vec<&str> = frame.key.iter().map(|s| s.as_str()).collect();
        if let Err(e) = self.sink.write(&key, &frame.d, &[row], t) {
            // The first failure is told; the rest would only repeat it.
            if self.lost.fetch_add(1, Ordering::Relaxed) == 0 {
                error!(e, "lens: a tapped frame could not be written, and later losses are only counted.");
            }
        }
    }
}

impl<C, F, T> FrameTap for Tap<C, F, T>
where
    T:  StrTest,
    F:  Fn(Dir, &C, &WebSocketMessage) -> Option<Frame> + Send + Sync,
{
    type Ctx = C;

    fn inbound(&self, ctx: &C, msg: &WebSocketMessage) {
        self.take(Dir::In, ctx, msg);
    }

    fn outbound(&self, ctx: &C, msg: &WebSocketMessage) {
        self.take(Dir::Out, ctx, msg);
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::ws::status::WebSocketStatusCode;

    fn field<'a>(f: &'a Frame, name: &str) -> Option<&'a Dat> {
        f.body.iter().find(|(k, _)| k == name).map(|(_, v)| v)
    }

    #[test]
    fn shape_counts_the_payload_of_each_kind_and_keeps_none_of_it() {
        let cases = [
            (WebSocketMessage::Text("secret words".to_string()),    "text",   12),
            (WebSocketMessage::Binary(vec![1, 2, 3]),               "binary", 3),
            (WebSocketMessage::Ping(vec![]),                        "ping",   0),
            (WebSocketMessage::Pong(vec![9]),                       "pong",   1),
            (WebSocketMessage::Close(None, None),                   "close",  0),
            (WebSocketMessage::Close(Some(WebSocketStatusCode::MessageTooBig), None),
                                                                    "close",  2),
            (WebSocketMessage::Close(Some(WebSocketStatusCode::MessageTooBig), Some("bye".to_string())),
                                                                    "close",  5),
        ];
        for (msg, op, bytes) in cases {
            let f = Frame::new(&["k"], "d").shape(&msg);
            assert_eq!(field(&f, "op"), Some(&Dat::Str(op.to_string())), "{:?}", msg);
            assert_eq!(field(&f, "bytes"), Some(&Dat::U64(bytes)), "{:?}", msg);
            assert_eq!(f.body.len(), 2, "shape adds the kind and the size and nothing else");
        }
    }

    #[test]
    fn a_field_added_twice_is_replaced_where_it_stands() {
        let f = Frame::new(&["a", "b"], "dev")
            .with("x", Dat::U8(1))
            .with("y", Dat::U8(2))
            .with("x", Dat::U8(3));
        assert_eq!(f.key, vec!["a".to_string(), "b".to_string()]);
        assert_eq!(f.body, vec![("x".to_string(), Dat::U8(3)), ("y".to_string(), Dat::U8(2))]);
    }

    #[test]
    fn a_direction_names_its_kind() {
        assert_eq!(Dir::In.kind(), "frame.in");
        assert_eq!(Dir::Out.kind(), "frame.out");
    }
}
