use crate::{
    msg::BotMsg,
};

use oxedize_fe2o3_core::{
    prelude::*,
    channels::Simplex,
};
use oxedize_fe2o3_jdat::id::NumIdDat;

use std::{
    sync::{
        Arc,
        Mutex,
    },
};

/// A labelled intention to exit a bot listen loop.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LoopBreak(pub bool);

impl LoopBreak {
    pub fn must_end(&self) -> bool { self.0 }
}

pub trait Bot<
    const BIDL: usize,
    BID:    NumIdDat<BIDL>,
    M:      BotMsg<ErrTag>,
> {

    // Required.
    // Getters.
    fn id(&self)                -> BID;
    fn errc(&self)              -> &Arc<Mutex<usize>>;
    fn chan_in(&self)           -> &Simplex<M>;
    fn label(&self)             -> String;
    fn err_count_warning(&self) -> usize;
    // Setters.
    fn set_chan_in(&mut self, chan_in: Simplex<M>);
    // Procedures.
    fn init(&mut self) -> Outcome<()> { Ok(()) }
    fn go(&mut self) {}
    fn listen(&mut self) -> LoopBreak { LoopBreak(true) }

    // Provided.
    
    fn now_listening(&self) {
        info!("{}: Now listening for internal messages...", self.label());
    }
    
    fn err_cannot_send(&self, e: Error<ErrTag>) {
        error!(err!(e, fmt!(
            "{:?}: could not send to a channnel.", self.id(),
        ), Channel, Write));
        self.inc_err();
    }

    fn err_cannot_receive(&self, e: Error<ErrTag>) {
        error!(err!(e, fmt!(
            "{:?}: could not receive from channnel.", self.id(),
        ), Channel, Read));
        self.inc_err();
    }

    fn err_poisoned_lock(&self, e: Error<ErrTag>) {
        error!(err!(e, fmt!(
            "{:?}: Another thread may have terminated while holding the lock.", self.id(),
        ), Lock, Poisoned));
        self.inc_err();
    }

    fn error(&self, e: Error<ErrTag>) {
        error!(e);
        self.inc_err();
    }

    fn msg_unknown(&self, msg: M) {
        error!(err!(fmt!(
            "{:?}: Message {:?} not recognised.", self.id(), msg,
        ), Invalid, Input));
        self.inc_err();
    }

    fn result(&self, result: &Outcome<()>) {
        match result {
            Err(e) => self.error(e.clone()),
            Ok(()) => (),
        }
    }

    fn error_count(&self) -> Outcome<usize> {
        let errc = self.errc();
        match errc.lock() {
            Err(_) => Err(err!(errmsg!( // This is bad.
                "{:?}: Cannot acquire the error counter lock, another thread may have \
                terminated while holding the lock.", self.id(),
            ), Lock, Poisoned)),
            Ok(c) => Ok(*c),
        }
    }

    fn inc_err(&self) {
        let errc = self.errc();
        match errc.lock() {
            Err(_) => {
                // This is bad.
                error!(err!(fmt!(
                    "{:?}: Cannot acquire the error counter lock.", self.id(),
                ), Lock, Poisoned));
            },
            Ok(mut c) => {
                *c += 1;
                if *c >= self.err_count_warning() {
                    // This is bad, and needs attention.
                    warn!("{:?}: this bot has triggered {} errors.  This \
                        requires immediate attention.", self.id(), c);
                }
            },
        }
    }

    //fn respond(
    //    &self,
    //    result: Outcome<M>,
    //    resp:   &Responder,
    //) {
    //    match resp.chan_innel() {
    //        None => return,
    //        Some(simplex) => {
    //            let msg = match result {
    //                Err(e) => M::error(e),
    //                Ok(m) => m,
    //            };
    //            let err_msg = format!(
    //                "While trying to return a msg {:?} \
    //                via a responder for ticket {}",
    //                msg, resp.ticket(),
    //            );
    //            if let Err(e) = simplex.send(msg) {
    //                self.err_cannot_send(e, errmsg!("{}", err_msg));
    //            }
    //        },
    //    }
    //}

    //// Message handling common to all bot implementations.
    //fn listen_more(&mut self, msg: M) -> LoopBreak;
    //fn listen_more(&mut self, msg: M) -> LoopBreak {
    //    match msg {
    //        Msg::Bot(bot::Msg::Finish) => {
    //            trace!("Finish message received, {} finishing now.", self.id());
    //            if let Err(e) = self.chan_in().send(Msg::Bot(bot::Msg::Finish)) {
    //                self.err_cannot_send(e, errmsg!("Attempt to return a finish message failed"));
    //            }
    //            return LoopBreak(true);
    //        },
    //        Msg::Bot(bot::Msg::Ready) => info!("{} ready to receive messages now.", self.id()),
    //        Msg::Channels(chan_ins, resp) => {
    //            self.set_chan_ins(chan_ins);
    //            match self.chan_ins().get_bot(&self.id()) {
    //                Err(e) => self.error(e),
    //                Ok(chan_in) => {
    //                    if self.chan_in().len() > 0 {
    //                        BotChannels::dump_pending_messages(
    //                            self.chan_in().drain_messages(),
    //                            &fmt!("Updating {} chan_innel, clearing out existing", self.id()),
    //                            None,
    //                            None,
    //                        );
    //                    }
    //                    let chan_in_clone = chan_in.clone();
    //                    self.set_chan_in(chan_in_clone);
    //                },
    //            }
    //            self.respond(Ok(Msg::ChannelsReceived(self.id().clone())), &resp);
    //            trace!("{}: Channel update received.", self.id());
    //        },
    //        Msg::Error(e) => error!(e),
    //        Msg::Ping(id, resp) => {
    //            trace!("{}: Ping received from {}, replying...",
    //                id, self.id());
    //            if let Err(e) = resp.send(Msg::OkFrom(self.id().clone())) {
    //                self.err_cannot_send(e, errmsg!("Attempt to return a ping failed"));
    //            }
    //        },
    //        _ => error!(err!(fmt!(
    //            "{}: Message {:?} not recognised.", self.id(), msg,
    //        ), Invalid, Input)),
    //    }
    //    LoopBreak(false)
    //}
}
