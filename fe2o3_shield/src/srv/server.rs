//! The UDP server loop: bind a socket, hear packets, and answer the ones that
//! ask something.

use crate::{
    srv::{
        constant,
        context::ServerContext,
        msg::{
            app::{
                Answer,
                AppMsg,
                AppMsgKind,
            },
            core::IdTypes,
            decode::Received,
            encode::ShieldCommand,
            handshake::HReq1,
            protocol::ProtocolTypes,
        },
        cmd::Command,
    },
};

use oxedyne_fe2o3_core::{
    prelude::*,
    channels::{
        Recv,
        simplex,
        Simplex,
    },
};
use oxedyne_fe2o3_iop_crypto::enc::Encrypter;
use oxedyne_fe2o3_iop_db::api::Database;
use oxedyne_fe2o3_iop_hash::api::Hasher;
use oxedyne_fe2o3_syntax::SyntaxRef;

use std::{
    future::Future,
    net::SocketAddr,
    sync::Arc,
    time::{
        Duration,
        Instant,
    },
};

use tokio::net::UdpSocket;


pub async fn answer_nothing(_payload: Vec<u8>, _src_addr: SocketAddr) -> Outcome<Answer> {
    Ok(Answer::Nothing)
}

pub struct Server<
    const C: usize,
    const ML: usize,
    const SL: usize,
    const UL: usize,
    P: ProtocolTypes<ML, SL, UL>,
    // Database
    ENC:    Encrypter,
    KH:     Hasher,
    DB:     Database<UL, <P::ID as IdTypes<ML, SL, UL>>::U, ENC, KH>,
> {
    context:    ServerContext<C, ML, SL, UL, P, ENC, KH, DB>,
    syntax:     SyntaxRef,
    ma_gc_last: Instant,
    ma_gc_int:  Duration,
    cmd_chan:   Simplex<Command>,
}

impl<
    const C: usize,
    const ML: usize,
    const SL: usize,
    const UL: usize,
    P: ProtocolTypes<ML, SL, UL> + 'static,
    // Database
    ENC:    Encrypter + 'static,
    KH:     Hasher + 'static,
    DB:     Database<UL, <P::ID as IdTypes<ML, SL, UL>>::U, ENC, KH> + 'static,
>
    Server<C, ML, SL, UL, P, ENC, KH, DB>
    where <P as ProtocolTypes<ML, SL, UL>>::W: 'static,
{
    pub fn new(
        context: ServerContext<C, ML, SL, UL, P, ENC, KH, DB>,
        syntax: SyntaxRef,
    )
        -> (Self, Simplex<Command>)
    {
        let cmd_chan = simplex();
        let cmd_chan_clone = cmd_chan.clone();

        (
            Self {
                context,
                syntax,
                ma_gc_last: Instant::now(),
                ma_gc_int:  constant::MSG_ASSEMBLY_GC_INTERVAL,
                cmd_chan,
            },
            cmd_chan_clone,
        )
    }

    pub async fn bind(&self) -> Outcome<Arc<UdpSocket>> {
        let port = self.context.cfg.server_port_udp;
        let ip = res!(self.context.cfg.bind_ip());
        // The proof of work on every packet is bound to the address it was sent
        // *to* as well as the address it came from, and a socket on the
        // wildcard address cannot say which of this machine's addresses a
        // datagram arrived at. A server bound there would therefore reject
        // every packet, which is a worse way to find out than this one.
        if ip.is_unspecified() {
            return Err(err!(
                "server_address is '{}', and a Shield server cannot listen on the \
                wildcard address: the proof of work on each packet is bound to the \
                address the packet was sent to, and a wildcard socket is not told \
                which of this machine's addresses that was. Name one, or write \
                'local' for whichever this machine has on its network.",
                self.context.cfg.server_address;
                Invalid, Configuration, Network));
        }
        let addr = SocketAddr::new(ip, port);
        match UdpSocket::bind(addr).await {
            Ok(sock) => Ok(Arc::new(sock)),
            Err(e) => Err(err!(e,
                "Could not bind the Shield UDP socket at {}.", addr;
                IO, Network, Init)),
        }
    }

    pub async fn start<H, F>(&mut self, handler: H) -> Outcome<()>
    where
        H: Fn(Vec<u8>, SocketAddr) -> F,
        F: Future<Output = Outcome<Answer>>,
    {
        let trg = res!(self.bind().await);
        self.run(trg, handler).await
    }

    pub async fn run<H, F>(
        &mut self,
        trg:        Arc<UdpSocket>,
        handler:    H,
    )
        -> Outcome<()>
    where
        H: Fn(Vec<u8>, SocketAddr) -> F,
        F: Future<Output = Outcome<Answer>>,
    {
        let trg_addr = res!(trg.local_addr(), IO, Network);
        info!(async_log::stream(), "mode = {:?}", self.context.protocol.mode);
        info!(async_log::stream(), "Listening on UDP at {}.", trg_addr);

        let mut buf = [0u8; constant::UDP_BUFFER_SIZE];
        'main: loop {
            // Wake periodically even when nothing arrives, so the garbage collector and the
            // command channel are not hostage to a quiet network.
            match tokio::time::timeout(
                constant::SERVER_EXT_SOCKET_CHECK_INTERVAL,
                trg.recv_from(&mut buf),
            ).await {
                Err(_) => (), // Nothing arrived in this window.
                Ok(Err(e)) => error!(async_log::stream(),
                    err!(e, "While trying to receive packet."; IO, Network)),
                Ok(Ok((n, src_addr))) => {
                    if let Err(e) = self.serve(&buf[..n], src_addr, &trg, &handler).await {
                        error!(async_log::stream(), err!(e,
                            "While handling incoming packet from {}.", src_addr;
                            IO, Network));
                    }
                },
            }

            // Message assembly garbage collection.
            if self.ma_gc_last.elapsed() > self.ma_gc_int {
                let result = self.context.protocol.massembler
                    .message_assembly_garbage_collection(&self.context.protocol.ma_params);
                match result {
                    Err(e) => error!(async_log::stream(), err!(e,
                        "While attempting to collect message assembler garbage.";
                        IO, Network)),
                    Ok(_) => {}
                }
                self.ma_gc_last = Instant::now();
            }

            // Check internal command channel.
            'cmd: loop {
                match self.cmd_chan.try_recv() {
                    Recv::Empty => break 'cmd,
                    Recv::Result(Ok(Command::Finish)) => break 'main,
                    Recv::Result(Ok(cmd)) => {
                        test!(async_log::stream(), "Server command received: {:?}", cmd);
                    }
                    Recv::Result(Err(e)) => error!(async_log::stream(), err!(e,
                        "While reading command channel."; Channel, Read)),
                }
            }
        }

        Ok(())
    }

    async fn serve<H, F>(
        &self,
        buf:        &[u8],
        src_addr:   SocketAddr,
        trg:        &Arc<UdpSocket>,
        handler:    &H,
    )
        -> Outcome<()>
    where
        H: Fn(Vec<u8>, SocketAddr) -> F,
        F: Future<Output = Outcome<Answer>>,
    {
        let trg_ip = res!(trg.local_addr(), IO, Network).ip();
        let protocol = self.context.protocol.clone();
        let accepted = match res!(protocol.clone().accept(buf, src_addr, trg_ip)) {
            Some(a) => a,
            None => return Ok(()), // Dropped, or the message is still incomplete.
        };
        let mid = accepted.meta.mid;
        let Received { fmt, pow, ids, msg } = res!(protocol.read(&accepted, self.syntax.clone()));

        // Multiple commands in a single message are permitted.
        for (cmd_name, mut msgcmd) in msg.cmds {
            match cmd_name.as_str() {
                "hreq1" => {
                    debug!(async_log::stream(), "HREQ1");
                    let mut scmd: HReq1<ML, SL, UL, P::ID> = HReq1 {
                        fmt: fmt.clone(),
                        pow: pow.clone(),
                        mid: ids.clone(),
                        ..Default::default()
                    };
                    // Each command type can implement its own custom process method, which
                    // captures only the parameters it needs.
                    let (akey, locked_amap) = res!(protocol.agrd.get_locked_map(&src_addr));
                    let mut unlocked_amap = lock_write!(locked_amap);
                    if let Some(alog) = unlocked_amap.get_mut(&akey) {
                        res!(scmd.respond(
                            &mut msgcmd,
                            &mut alog.data, // For pow parameters.
                        ));
                    }
                },
                other => match AppMsgKind::from_cmd_name(other) {
                    Some(AppMsgKind::Request) => {
                        let mut scmd: AppMsg<ML, SL, UL, P::ID> = AppMsg {
                            fmt:    fmt.clone(),
                            pow:    pow.clone(),
                            mid:    ids.clone(),
                            kind:   AppMsgKind::Request,
                            ..Default::default()
                        };
                        res!(scmd.deconstruct(&mut msgcmd));
                        let answer = res!(handler(scmd.payload, src_addr).await);
                        if let Answer::Reply(payload) = answer {
                            // The answer travels under the identifier the question arrived
                            // with, and back to the address it arrived from. Neither is a
                            // detail: a peer behind a router has no other address, and a
                            // peer holding two questions at once has no other way of
                            // telling the answers apart.
                            let packets = res!(protocol.build_app(
                                self.syntax.clone(),
                                AppMsgKind::Reply,
                                mid,
                                payload,
                                trg_ip,
                                src_addr.ip(),
                            ));
                            for packet in packets {
                                res!(trg.send_to(&packet, src_addr).await, IO, Network);
                            }
                        }
                    },
                    Some(AppMsgKind::Reply) => debug!(async_log::stream(),
                        "Dropping an application reply from {} to a question this peer \
                        did not ask.", src_addr),
                    None => return Err(err!(
                        "Unrecognised message command '{}'.", other;
                        Bug, Unimplemented)),
                },
            }
        }
        Ok(())
    }
}
