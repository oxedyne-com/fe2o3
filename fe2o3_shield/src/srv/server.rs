use crate::srv::{
    constant,
    context::ServerContext,
};

use oxedize_fe2o3_core::prelude::*;
use oxedize_fe2o3_iop_crypto::{
    enc::Encrypter,
    sign::{
        Signer,
    },
};
use oxedize_fe2o3_iop_db::api::Database;
use oxedize_fe2o3_iop_hash::{
    api::Hasher,
    csum::Checksummer,
};
use oxedize_fe2o3_jdat::id::NumIdDat;
use oxedize_fe2o3_syntax::SyntaxRef;

use std::{
    io,
    net::{
        SocketAddr,
        UdpSocket,
    },
    time::{
        Duration,
        Instant,
    },
};

use local_ip_address::local_ip;
use tokio::task;


pub struct Server<
    const C: usize,
    const MIDL: usize,
    const SIDL: usize,
    const UIDL: usize,
    MID:    NumIdDat<MIDL>,
    SID:    NumIdDat<SIDL>,
    UID:    NumIdDat<UIDL>,
    // Database
    ENC:    Encrypter,
    KH:     Hasher,
    DB:     Database<UIDL, UID, ENC, KH>, 
    // Wire
	WENC:   Encrypter,
	WCS:    Checksummer,
    POWH:   Hasher,
	SGN:    Signer,
	HS:     Encrypter,
> {
    context:    ServerContext<C, MIDL, SIDL, UIDL, MID, SID, UID, ENC, KH, DB, WENC, WCS, POWH, SGN, HS>,
    syntax:     SyntaxRef,
    ma_gc_last: Instant,
    ma_gc_int:  Duration,
}

impl<
    const C: usize,
    const MIDL: usize,
    const SIDL: usize,
    const UIDL: usize,
    MID:    NumIdDat<MIDL> + 'static,
    SID:    NumIdDat<SIDL> + 'static,
    UID:    NumIdDat<UIDL> + 'static,
    // Database
    ENC:    Encrypter + 'static,
    KH:     Hasher + 'static,
    DB:     Database<UIDL, UID, ENC, KH> + 'static, 
    // Wire
	WENC:   Encrypter + 'static,
	WCS:    Checksummer + 'static,
    POWH:   Hasher + 'static,
	SGN:    Signer + 'static,
	HS:     Encrypter + 'static,
>
    Server<C, MIDL, SIDL, UIDL, MID, SID, UID, ENC, KH, DB, WENC, WCS, POWH, SGN, HS>
{
    pub fn new(
       context: ServerContext<C, MIDL, SIDL, UIDL, MID, SID, UID, ENC, KH, DB, WENC, WCS, POWH, SGN, HS>,
       syntax:  SyntaxRef,
    )
        -> Self
    {
        Self {
            context,
            syntax,
            ma_gc_last: Instant::now(),
            ma_gc_int:  Duration::from_secs(300),
        }
    }

    pub async fn start(&mut self) -> Outcome<()> {

        let port        = self.context.cfg.server_port_udp;
        let ip_addr     = res!(local_ip());
        let sock_addr   = SocketAddr::new(ip_addr.clone(), port);

        info!("Server ip address = {}", ip_addr);
        let sock = res!(UdpSocket::bind(sock_addr));

        info!("test_mode = {}", self.context.protocol.test_mode);
        info!("Listening on UDP at {:?}.", sock_addr);
    
        loop {
            let mut buf = [0u8; constant::UDP_BUFFER_SIZE]; 
            // EXTERNAL
            match sock.recv_from(&mut buf) { // Receive udp packet, non-blocking.
                Err(e) => {
                    //match self.timer.write() {
                    //    Err(e) => self.error(err!(errmsg!(
                    //        "While locking timer for writing: {}.", e), Poisoned)),
                    //    Ok(mut unlocked_timer) => { unlocked_timer.update(); },
                    //}
                    ////self.timer.update();
                    match e.kind() {
                        io::ErrorKind::WouldBlock | io::ErrorKind::InvalidInput => {}
                        _ => error!(err!(e, "While trying to receive packet."; IO, Network)),
                    }
                },
                Ok((n, src_addr)) => {
                    let protocol = self.context.protocol.clone();
                    let result = task::spawn(protocol.handle(
                        buf,
                        n,
                        src_addr,
                        self.syntax.clone(),
                    ));
                    match result.await {
                        Ok(result) => match result {
                            Err(e) => error!(err!(e,
                                "While handling incoming packet."; IO, Network)),
                            Ok(_) => {}
                        },
                        Err(e) => error!(err!(e,
                            "While awaiting for packet handler."; IO, Network)),
                    }
                },
            } // Receive udp packet.

            // Message assembly garbage collection.
            if self.ma_gc_last.elapsed() > self.ma_gc_int {
                let result = self.context.protocol.massembler
                    .message_assembly_garbage_collection(&self.context.protocol.ma_params);
                match result {
                    Err(e) => error!(err!(e,
                        "While attempting to collect message assembler garbage.";
                        IO, Network)),
                    Ok(_) => {}
                }
                self.ma_gc_last = Instant::now();
            }
        }
    }
}
