use crate::{
    prelude::*,
    bots::base::bot_deps::*,
    comm::channels::BotChannels,
};

use oxedize_fe2o3_core::{
    prelude::*,
    channels::Recv,
};
use oxedize_fe2o3_jdat::id::NumIdDat;

use std::{
    sync::Arc,
};

/// Listens internally, and possibly on the wire, for database commands.
pub struct ServerBot<
    const UIDL: usize,
    UID:    NumIdDat<UIDL>,
    ENC:    Encrypter,
    KH:     Hasher,
	PR:     Hasher,
    CS:     Checksummer,
>{
    // Bot
    sem:        Semaphore,
    errc:       Arc<Mutex<usize>>,
    // Comms
    chan_in:    Simplex<OzoneMsg<UIDL, UID, ENC, KH>>,
    // API
    api:        OzoneApi<UIDL, UID, ENC, KH, PR, CS>,
    // State
    inited:     bool,
}

impl<
    const UIDL: usize,
    UID:    NumIdDat<UIDL> + 'static,
    ENC:    Encrypter + 'static,
    KH:     Hasher + 'static,
	PR:     Hasher,
    CS:     Checksummer,
>
    Bot<{ BID_LEN }, Bid, OzoneMsg<UIDL, UID, ENC, KH>> for ServerBot<UIDL, UID, ENC, KH, PR, CS>
{
    bot_methods!();

    fn go(&mut self) {
        if self.no_init() { return; }
        info!("{}: Listening for database requests.", self.ozid());
        self.now_listening();
        loop {
            if self.listen().must_end() { break; }
        }
    }

    fn listen(&mut self) -> LoopBreak {
        // INTERNAL
        match self.chan_in().recv_timeout(constant::SERVER_INT_CHANNEL_CHECK_INTERVAL) {
            Recv::Empty => (),
            Recv::Result(Err(e)) => self.err_cannot_receive(err!(e, errmsg!(
                "{}: Waiting for message on internal channel.", self.ozid(),
            ), Channel, Read)),
            Recv::Result(Ok(msg)) => match msg {
                OzoneMsg::Get { key, schms2, resp } => {
                    match self.api().get_wait(&key, schms2.as_ref()) {
                        Err(e) => self.error(err!(e, errmsg!(
                            "{}: While trying to get value for key {:?}", self.ozid(), key,
                        ), Data)),
                        Ok(result) => match resp.send(OzoneMsg::GetResult(result)) {
                            Err(e) => self.err_cannot_send(err!(e, errmsg!(
                                "{}: While sending an OzoneMsg::GetResult back via a responder.",
                                self.ozid(),
                            ), Data, Channel)),
                            Ok(()) => (),
                        },
                    }
                },
                OzoneMsg::Put { key, val, user, schms2, resp } => {
                    debug!("Store key: {:?}",key);
                    match self.api().store_dat_using_responder(
                        key,
                        val,
                        user,
                        schms2.as_ref(),
                        resp,
                    ) {
                        Err(e) => self.error(err!(e, errmsg!(
                            "{}: While trying to put value.", self.ozid(),
                        ), Data)),
                        Ok(_nchunks) => (),
                    }
                },
                _ => if self.listen_more(msg).must_end() {
                    return LoopBreak(true);
                },
                // TODO one for OzoneMsg::Delete?
            },
        }

        //// EXTERNAL
        //match self.sock.recv_from(&mut self.buf) { // Receive udp packet, non-blocking.
        //    Err(e) => {
        //        match self.timer.write() {
        //            Err(e) => self.error(err!(errmsg!(
        //                "While locking timer for writing: {}.", e), ErrTag::Poisoned)),
        //            Ok(mut unlocked_timer) => { unlocked_timer.update(); },
        //        }
        //        //self.timer.update();
        //        match e.kind() {
        //            io::ErrorKind::WouldBlock | io::ErrorKind::InvalidInput => (),
        //            _ => self.err_cannot_receive(Error::from(e),
        //                errmsg!("Waiting for message on external UDP socket")),
        //        }
        //    },
        //    Ok((n, src_addr)) => {
        //        let mut buf_clone = [0u8; constant::UDP_BUFFER_SIZE]; 
        //        for i in 0..n {
        //            buf_clone[i] = self.buf[i];
        //        }
        //        let state = wire::ServerProcessorEnv::<POWH, SGN> {
        //            buf:        buf_clone,
        //            n,
        //            src_addr,
        //            // Comms    
        //            //wschms:     WireSchemes<WENC, WCS, POWH, SGN, HS>,
        //            //buf:        [u8; constant::UDP_BUFFER_SIZE], 
        //            //chan:       Simplex<Msg>,
        //            //chans:      BotChannels,
        //            protoref:       self.protoref.clone(),      // Arc
        //            timer:          self.timer.clone(),         // Arc
        //            // Schemes.
        //            schmdb:         self.schmdb.clone(),        // Arc
        //            // Keys.
        //            pack_sigkeys:   self.pack_sigkeys.clone(),  // Arc
        //            // Declared source address protection.
        //            agrd:           self.agrd.clone(),          // Arc
        //            // User protection.
        //            ugrd:           self.ugrd.clone(),          // Arc
        //            // Packet validation.
        //            packval:        self.packval.clone(),
        //            gpzparams:      self.gpzparams.clone(),
        //            // Message assembly.
        //            massembler:     self.massembler.clone(),    // Arc
        //            ma_params:      self.ma_params.clone(),
        //            // Database configuration values.
        //            time_horiz:     self.cfg().server_pow_time_horiz_secs,
        //            accept_unknown: self.cfg().server_accept_unknown_users,
        //        };
        //        task::spawn(state.process(
        //            //&mut self    RingTimer<{ constant::REQ_TIMER_LEN }>,
        //        ));
        //    },
        //} // Receive udp packet.

        //// Message assembly garbage collection.
        //if self.ma_gc_last.elapsed() > self.ma_gc_int {
        //    self.massembler.message_assembly_garbage_collection(&self.ma_params);
        //    self.ma_gc_last = Instant::now();
        //}

        LoopBreak(false)
    }
}

impl<
    const UIDL: usize,
    UID:    NumIdDat<UIDL> + 'static,
    ENC:    Encrypter + 'static,
    KH:     Hasher + 'static,
	PR:     Hasher,
    CS:     Checksummer,
>
    OzoneBot<UIDL, UID, ENC, KH, PR, CS> for ServerBot<UIDL, UID, ENC, KH, PR, CS>
{
    ozonebot_methods!();
}

impl<
    const UIDL: usize,
    UID:    NumIdDat<UIDL> + 'static,
    ENC:    Encrypter + 'static,
    KH:     Hasher + 'static,
	PR:     Hasher,
    CS:     Checksummer,
>
    ServerBot<UIDL, UID, ENC, KH, PR, CS>
{
    pub fn new(
        args:   BotInitArgs<UIDL, UID, ENC, KH, PR, CS>,
    )
        -> Self
    {
        Self {
            // Bot
            sem:        args.sem,
            errc:       Arc::new(Mutex::new(0)),
            // Comms    
            chan_in:    args.chan_in,
            // API
            api:        args.api,
            // State    
            inited:     false,
        }
    }
    

}
