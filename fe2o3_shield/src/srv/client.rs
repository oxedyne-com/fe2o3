//! The dialling half of a Shield exchange.
//!
//! A [`Client`] owns a UDP socket, sends an application payload on it, and
//! hears the answer on the same socket. That is the whole of it, and the shape
//! is chosen rather than inherited: a peer behind a household router can send
//! a datagram out and receive the reply to it, and cannot receive one that
//! arrives out of the blue. An answer sent to a fresh socket, or dialled back
//! to a listening port, is an answer half a real network never gets.
//!
//! Nothing here is a lesser peer. The client validates what arrives with the
//! same [`Protocol`] a server does -- the same guards, the same proof-of-work
//! and signature checks, the same message assembler -- because a reply is as
//! much somebody else's bytes as a request is. What it does not do is dispatch
//! requests: it hears answers to its own questions, and drops everything else.
//!
//! ```ignore
//! let client = res!(Client::bind(bind_addr, protocol, syntax).await);
//! let answer = res!(client.ask(peer_addr, payload, constant::APP_REPLY_WAIT).await);
//! ```

use crate::srv::{
    constant,
    msg::{
        app::{
            AppMsg,
            AppMsgKind,
        },
        core::IdTypes,
        decode::Received,
        encode::ShieldCommand,
        protocol::{
            Protocol,
            ProtocolTypes,
        },
    },
};

use oxedyne_fe2o3_core::{
    prelude::*,
    rand::RanDef,
};
use oxedyne_fe2o3_syntax::SyntaxRef;

use std::{
    net::SocketAddr,
    sync::Arc,
    time::Duration,
};

use tokio::net::UdpSocket;


/// A peer that dials: it sends payloads and hears the answers to them.
pub struct Client<
    const C: usize,
    const ML: usize,
    const SL: usize,
    const UL: usize,
    P: ProtocolTypes<ML, SL, UL>,
> {
    /// The socket questions leave on and answers arrive on. One socket, because
    /// that is what makes the answers arrive at all.
    sock:       Arc<UdpSocket>,
    /// Guards, validator, schemes and message assembler.
    protocol:   Protocol<C, ML, SL, UL, P>,
    /// Syntax messages are built against and validated by.
    syntax:     SyntaxRef,
}

impl<
    const C: usize,
    const ML: usize,
    const SL: usize,
    const UL: usize,
    P: ProtocolTypes<ML, SL, UL> + 'static,
>
    Client<C, ML, SL, UL, P>
    where <P as ProtocolTypes<ML, SL, UL>>::W: 'static,
{
    /// Bind a socket to dial from.
    ///
    /// A port of zero is the usual thing to ask for: a peer that only dials
    /// wants whatever port the operating system has going spare, and the peers
    /// it talks to learn the address from the packets themselves.
    pub async fn bind(
        addr:       SocketAddr,
        protocol:   Protocol<C, ML, SL, UL, P>,
        syntax:     SyntaxRef,
    )
        -> Outcome<Self>
    {
        let sock = match UdpSocket::bind(addr).await {
            Ok(s) => s,
            Err(e) => return Err(err!(e,
                "Could not bind a Shield client socket at {}.", addr;
                IO, Network, Init)),
        };
        Ok(Self {
            sock:   Arc::new(sock),
            protocol,
            syntax,
        })
    }

    /// Where this client is dialling from.
    pub fn local_addr(&self) -> Outcome<SocketAddr> {
        Ok(res!(self.sock.local_addr(), IO, Network))
    }

    /// The socket, for a caller that has its own reason to hold one.
    pub fn socket(&self) -> Arc<UdpSocket> {
        self.sock.clone()
    }

    /// Send a payload and return the message identifier it went out under.
    ///
    /// Nothing is waited for. A caller with something to tell rather than to
    /// ask stops here; one that wants the answer passes the identifier to
    /// [`Client::hear`].
    pub async fn tell(
        &self,
        trg_addr:   SocketAddr,
        payload:    Vec<u8>,
    )
        -> Outcome<<P::ID as IdTypes<ML, SL, UL>>::M>
    {
        let mid = <P::ID as IdTypes<ML, SL, UL>>::M::randef();
        let src_ip = res!(self.local_addr()).ip();
        let packets = res!(self.protocol.build_app(
            self.syntax.clone(),
            AppMsgKind::Request,
            mid,
            payload,
            src_ip,
            trg_addr.ip(),
        ));
        for packet in packets {
            res!(self.sock.send_to(&packet, trg_addr).await, IO, Network);
        }
        Ok(mid)
    }

    /// Wait for the answer to the question sent under `mid`.
    ///
    /// Anything else that turns up on the socket in the meantime is dropped and
    /// the wait goes on: a packet that fails its guards or its validation, a
    /// message under an identifier this peer never sent, a request from
    /// somebody who mistook this socket for a server. None of those is the
    /// answer, and none of them shortens the time the answer has to arrive in.
    pub async fn hear(
        &self,
        mid:    &<P::ID as IdTypes<ML, SL, UL>>::M,
        wait:   Duration,
    )
        -> Outcome<Vec<u8>>
    {
        let deadline = tokio::time::Instant::now() + wait;
        let src_ip = res!(self.local_addr()).ip();
        let mut buf = [0u8; constant::UDP_BUFFER_SIZE];
        loop {
            let left = deadline.saturating_duration_since(tokio::time::Instant::now());
            if left.is_zero() {
                return Err(err!(
                    "Nothing answered message {} within {:?}.", mid, wait;
                    IO, Network, Timeout));
            }
            let (n, trg_addr) = match tokio::time::timeout(
                left,
                self.sock.recv_from(&mut buf),
            ).await {
                Err(_) => return Err(err!(
                    "Nothing answered message {} within {:?}.", mid, wait;
                    IO, Network, Timeout)),
                Ok(Err(e)) => return Err(err!(e,
                    "While waiting for the answer to message {}.", mid;
                    IO, Network)),
                Ok(Ok(pair)) => pair,
            };
            let accepted = match self.protocol.clone().accept(&buf[..n], trg_addr, src_ip) {
                Ok(Some(a)) => a,
                Ok(None) => continue, // Dropped, or the message is still incomplete.
                Err(e) => {
                    warn!(async_log::stream(),
                        "While reading a packet from {}: {}", trg_addr, e);
                    continue;
                },
            };
            if accepted.meta.mid != *mid {
                debug!(async_log::stream(),
                    "A message under identifier {} arrived from {} while waiting on {}; \
                    dropped.", accepted.meta.mid, trg_addr, mid);
                continue;
            }
            let Received { msg, .. } = res!(self.protocol.read(&accepted, self.syntax.clone()));
            for (cmd_name, mut msgcmd) in msg.cmds {
                match AppMsgKind::from_cmd_name(cmd_name.as_str()) {
                    Some(AppMsgKind::Reply) => {
                        let mut scmd: AppMsg<ML, SL, UL, P::ID> = AppMsg {
                            kind: AppMsgKind::Reply,
                            ..Default::default()
                        };
                        res!(scmd.deconstruct(&mut msgcmd));
                        return Ok(scmd.payload);
                    },
                    _ => debug!(async_log::stream(),
                        "A '{}' arrived from {} under the identifier {} was waiting on, \
                        which is not an answer; dropped.", cmd_name, trg_addr, mid),
                }
            }
        }
    }

    /// Send a payload and wait for the answer to it.
    pub async fn ask(
        &self,
        trg_addr:   SocketAddr,
        payload:    Vec<u8>,
        wait:       Duration,
    )
        -> Outcome<Vec<u8>>
    {
        let mid = res!(self.tell(trg_addr, payload).await);
        self.hear(&mid, wait).await
    }
}
