//! Stub: the API of the streaming body relay, so that its tests compile red.

use oxedyne_fe2o3_core::prelude::*;

use std::sync::atomic::AtomicU64;

use tokio::io::{
    AsyncRead,
    AsyncWrite,
};

pub const RELAY_BUF_SIZE: usize = 64 * 1_024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Framing {
    Length(u64),
    Chunked,
    Close,
    None,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Subject<'a> {
    Request,
    Reply {
        status: u16,
        method: &'a str,
    },
}

pub fn framing<I, N, V>(_subject: Subject<'_>, _fields: I) -> Outcome<Framing>
where
    I: IntoIterator<Item = (N, V)>,
    N: AsRef<str>,
    V: AsRef<str>,
{
    Ok(Framing::None)
}

pub async fn relay<R, W>(
    _r:         &mut R,
    _w:         &mut W,
    _framing:   Framing,
    _count:     &AtomicU64,
    _bound:     Option<u64>,
    rest:       Vec<u8>,
)
    -> Outcome<Vec<u8>>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    Ok(rest)
}
