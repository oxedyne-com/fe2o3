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
        error!(err!(e, "{:?}: could not send to a channnel.", self.id(); Channel, Write));
        self.inc_err();
    }

    fn err_cannot_receive(&self, e: Error<ErrTag>) {
        error!(err!(e, "{:?}: could not receive from channnel.", self.id(); Channel, Read));
        self.inc_err();
    }

    fn err_poisoned_lock(&self, e: Error<ErrTag>) {
        error!(err!(e,
            "{:?}: Another thread may have terminated while holding the lock.", self.id();
        Lock, Poisoned));
        self.inc_err();
    }

    fn error(&self, e: Error<ErrTag>) {
        error!(e);
        self.inc_err();
    }

    fn msg_unknown(&self, msg: M) {
        error!(err!("{:?}: Message {:?} not recognised.", self.id(), msg; Invalid, Input));
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
            Err(_) => Err(err!( // This is bad.
                "{:?}: Cannot acquire the error counter lock, another thread may have \
                terminated while holding the lock.", self.id();
            Lock, Poisoned)),
            Ok(c) => Ok(*c),
        }
    }

    fn inc_err(&self) {
        let errc = self.errc();
        match errc.lock() {
            Err(_) => {
                // This is bad.
                error!(err!(
                    "{:?}: Cannot acquire the error counter lock.", self.id();
                Lock, Poisoned));
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
}
