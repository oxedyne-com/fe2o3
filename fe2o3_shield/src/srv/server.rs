use crate::{
    srv::{
        constant,
        context::ServerContext,
        msg::{
            core::IdTypes,
            protocol::{
                ProtocolTypes,
            },
        },
        cmd::Command,
    },
};

use oxedize_fe2o3_core::{
    prelude::*,
    channels::{
        Recv,
        simplex,
        Simplex,
    },
};
use oxedize_fe2o3_iop_crypto::enc::Encrypter;
use oxedize_fe2o3_iop_db::api::Database;
use oxedize_fe2o3_iop_hash::api::Hasher;
use oxedize_fe2o3_syntax::SyntaxRef;

use std::{
    io,
    net::{
        SocketAddr,
        UdpSocket,
    },
    sync::Arc,
    time::{
        Duration,
        Instant,
    },
};

use local_ip_address::local_ip;
use tokio::{
    task,
    //task_local,
};


//task_local! {
//    pub static LOG_STREAM_ID: String;
//}
//
//pub fn async_log::stream() -> String {
//    LOG_STREAM_ID.try_with(|id| id.clone()).unwrap_or(fmt!("main"))
//}

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
                ma_gc_int:  Duration::from_secs(300),
                cmd_chan,
            },
            cmd_chan_clone,
        )
    }

    pub async fn start(&mut self) -> Outcome<()> {

        // Target (this machine).
        let port        = self.context.cfg.server_port_udp;
        let ip_addr     = res!(local_ip());
        let trg_addr    = SocketAddr::new(ip_addr.clone(), port);

        info!(async_log::stream(), "Server ip address = {}", ip_addr);
        let trg = Arc::new(res!(UdpSocket::bind(trg_addr)));

        info!(async_log::stream(), "mode = {:?}", self.context.protocol.mode);
        info!(async_log::stream(), "Listening on UDP at {:?}.", trg_addr);

        res!(trg.set_read_timeout(Some(constant::SERVER_EXT_SOCKET_CHECK_INTERVAL)));
    
        'main: loop {
            // Check internet port.
            let mut buf = [0u8; constant::UDP_BUFFER_SIZE]; 
            match trg.recv_from(&mut buf) { // Receive udp packet, non-blocking.
                Err(e) => {
                    //match self.timer.write() {
                    //    Err(e) => self.error(err!(errmsg!(
                    //        "While locking timer for writing: {}.", e), Poisoned)),
                    //    Ok(mut unlocked_timer) => { unlocked_timer.update(); },
                    //}
                    ////self.timer.update();
                    match e.kind() {
                        io::ErrorKind::WouldBlock | io::ErrorKind::InvalidInput => {}
                        _ => error!(async_log::stream(),
                            err!(e, "While trying to receive packet."; IO, Network)),
                    }
                },
                Ok((n, src_addr)) => {
                    let protocol = self.context.protocol.clone();
                    let result = task::spawn(protocol.handle(
                        buf,
                        n,
                        src_addr,
                        trg.clone(),
                        self.syntax.clone(),
                    ));
                    match result.await {
                        Ok(result) => match result {
                            Err(e) => error!(async_log::stream(), err!(e,
                                "While handling incoming packet."; IO, Network)),
                            Ok(_) => {}
                        },
                        Err(e) => error!(async_log::stream(), err!(e,
                            "While awaiting for packet handler."; IO, Network)),
                    }
                },
            } // Receive udp packet.

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
}
