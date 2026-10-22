use crate::{
    prelude::*,
    api::{
        next_event,
        Begun,
        Event,
        PendingStore,
    },
    bots::base::bot_deps::*,
    comm::channels::BotChannels,
};

use oxedyne_fe2o3_core::{
    prelude::*,
};
use oxedyne_fe2o3_jdat::id::NumIdDat;

use std::{
    mem,
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
    sem:            Semaphore,
    errc:           Arc<Mutex<usize>>,
    log_stream_id:  String,
    // Comms
    chan_in:    Simplex<OzoneMsg<UIDL, UID, ENC, KH>>,
    // API
    api:        OzoneApi<UIDL, UID, ENC, KH, PR, CS>,
    // State
    inited:     bool,
    pending:    Vec<PendingStore<UIDL, UID, ENC, KH>>, // chunked puts awaiting their chunks
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

        sync_log::set_stream(self.log_stream_id());

        if self.no_init() { return; }
        info!(sync_log::stream(), "{}: Listening for database requests.", self.ozid());
        self.now_listening();
        loop {
            if self.listen().must_end() { break; }
        }
    }

    fn listen(&mut self) -> LoopBreak {
        // INTERNAL
        // Block until something arrives.  The server bot has no periodic maintenance of its
        // own, so it must sleep rather than poll its channels, otherwise an idle database burns
        // a CPU core.  A shutdown is delivered as an `OzoneMsg::Finish` on the request channel,
        // which wakes the blocking receive immediately.  Chunked puts under way are waited on
        // beside it, each on the channel of its own responder, so that a put waiting for the
        // disk does not hold up the requests behind it.
        let event = if self.pending.is_empty() {
            self.chan_in().recv().map(Event::Inbox)
        } else {
            next_event(self.chan_in.rx(), &self.pending)
        };
        match event {
            Err(e) => self.err_cannot_receive(err!(e,
                "{}: Waiting for message on internal channel.", self.ozid();
                IO, Channel)),
            Ok(Event::Answer(i, msg)) => {
                let outcome = self.pending[i].advance(&self.api, msg);
                self.settle(i, outcome);
            },
            Ok(Event::Quiet) => self.expire_pending(),
            Ok(Event::Inbox(msg)) => match msg {
                OzoneMsg::Get { key, schms2, resp } => {
                    match self.api().get_wait(&key, schms2.as_ref()) {
                        Err(e) => {
                            // The caller is waiting on this answer, and a failure only logged
                            // reached it as a timeout that named no cause.
                            let e = err!(e,
                                "{}: While trying to get value for key {:?}", self.ozid(), key;
                                Data, Read);
                            self.error(e.clone());
                            self.respond(Err(e), &resp);
                        },
                        Ok(result) => match resp.send(OzoneMsg::GetResult(result)) {
                            Err(e) => self.err_cannot_send(err!(e,
                                "{}: While sending an OzoneMsg::GetResult back via a responder.",
                                self.ozid();
                                Data, Channel)),
                            Ok(()) => (),
                        },
                    }
                },
                OzoneMsg::Put { key, val, user, schms2, resp } => {
                    debug!(sync_log::stream(), "Store key: {:?}",key);
                    let caller = resp.clone();
                    match self.api().begin_store(
                        key,
                        val,
                        user,
                        schms2.as_ref(),
                        resp,
                    ) {
                        Err(e) => {
                            // As for a get: the caller hears the cause, not a timeout.
                            let e = err!(e,
                                "{}: While trying to put value.", self.ozid();
                                Data, Write);
                            self.error(e.clone());
                            self.respond(Err(e), &caller);
                        },
                        Ok(Begun::Done(_nchunks)) => (),
                        Ok(Begun::Pending(pending)) => self.pending.push(pending),
                    }
                },
                _ => {
                    // A put in hand is seen through before the bot finishes, as it was when the
                    // bot waited on each one.
                    if matches!(msg, OzoneMsg::Finish) {
                        self.finish_pending();
                    }
                    if self.listen_more(msg).must_end() {
                        return LoopBreak(true);
                    }
                },
                // TODO one for OzoneMsg::Delete?
            },
        }

        //// EXTERNAL
        //match self.sock.recv_from(&mut self.buf) { // Receive udp packet, non-blocking.
        //    Err(e) => {
        //        match self.timer.write() {
        //            Err(e) => self.error(err!(
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
            sem:            args.sem,
            errc:           Arc::new(Mutex::new(0)),
            log_stream_id:  args.log_stream_id,
            // Comms    
            chan_in:    args.chan_in,
            // API
            api:        args.api,
            // State    
            inited:     false,
            pending:    Vec::new(),
        }
    }

    // A store has taken in an answer.  A finished one is dropped, and an error that could not
    // be given to the caller, which has no responder, is the bot's to report.
    fn settle(&mut self, i: usize, outcome: Outcome<bool>) {
        match outcome {
            Ok(false) => (),
            Ok(true) => { self.pending.swap_remove(i); },
            Err(e) => {
                self.pending.swap_remove(i);
                self.error(err!(e,
                    "{}: While storing a chunked value.", self.ozid();
                    Data, Write));
            },
        }
    }

    // Fails the stores that have waited out their silence.
    fn expire_pending(&mut self) {
        let mut i = self.pending.len();
        while i > 0 {
            i -= 1;
            if self.pending[i].left().is_zero() {
                let outcome = self.pending[i].expire(&self.api);
                self.settle(i, outcome);
            }
        }
    }

    // Waits for every store in hand, each to the end.
    fn finish_pending(&mut self) {
        for pending in mem::take(&mut self.pending) {
            if let Err(e) = pending.run(&self.api) {
                self.error(err!(e,
                    "{}: While finishing a chunked store at shutdown.", self.ozid();
                    Data, Write));
            }
        }
    }
}
