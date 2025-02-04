use crate::{
    prelude::*,
    base::{
        cfg::OzoneConfig,
        id::{
            Bid,
            BID_LEN,
            OzoneBotId,
        },
    },
    comm::{
        channels::BotChannels,
        msg::OzoneMsg,
        response::Responder,
    },
};

use oxedize_fe2o3_bot::{
    bot::{
        Bot,
        LoopBreak,
    },
};
use oxedize_fe2o3_core::{
    channels::Simplex,
    thread::Semaphore,
};
use oxedize_fe2o3_jdat::id::NumIdDat;

use std::{
    path::Path,
};

#[macro_export]
macro_rules! bot_methods { () => {
    fn id(&self)                -> Bid                  { self.ozid().bid() }
    fn errc(&self)              -> &Arc<Mutex<usize>>   { &self.errc }
    fn chan_in(&self)           -> &Simplex<OzoneMsg<UIDL, UID, ENC, KH>> { &self.chan_in }
    fn label(&self)             -> String               { fmt!("{}", self.ozid()) }
    fn err_count_warning(&self) -> usize                { constant::BOT_ERR_COUNT_WARNING }
    fn log_stream_id(&self)     -> String               { self.log_stream_id.clone() }
    fn set_chan_in(&mut self, chan_in: Simplex<OzoneMsg<UIDL, UID, ENC, KH>>) {
        self.chan_in = chan_in;
    }
    fn init(&mut self) -> Outcome<()> {
        info!(sync_log::stream(), "{:?}: Initialising.", self.ozid());
        self.inited = true;
        Ok(())
    }
} }

#[macro_export]
macro_rules! ozonebot_methods { () => {
    fn api(&self)           -> &OzoneApi<UIDL, UID, ENC, KH, PR, CS> { &self.api }
    fn api_mut(&mut self)   -> &mut OzoneApi<UIDL, UID, ENC, KH, PR, CS> { &mut self.api }
    fn ozid(&self)      -> &OzoneBotId  { &self.api.ozid }
    fn db_root(&self)   -> &Path        { &self.api.db_root }
    fn cfg(&self)       -> &OzoneConfig { &self.api.cfg }
    fn chans(&self)     -> &BotChannels<UIDL, UID, ENC, KH> { &self.api.chans }
    fn inited(&self)    -> bool { self.inited }
    fn set_chans(&mut self, chans: BotChannels<UIDL, UID, ENC, KH>) { self.api.chans = chans }
} }

pub trait OzoneBot<
    const UIDL: usize,
    UID:    NumIdDat<UIDL> + 'static,
    ENC:    Encrypter + 'static,
    KH:     Hasher + 'static,
	PR:     Hasher,
    CS:     Checksummer,
>:
    Bot<{ BID_LEN }, Bid, OzoneMsg<UIDL, UID, ENC, KH>>
{
    // Required.
    fn api(&self)           -> &OzoneApi<UIDL, UID, ENC, KH, PR, CS>;
    fn api_mut(&mut self)   -> &mut OzoneApi<UIDL, UID, ENC, KH, PR, CS>;
    fn ozid(&self)      -> &OzoneBotId;
    fn db_root(&self)   -> &Path;
    fn cfg(&self)       -> &OzoneConfig;
    fn chans(&self)     -> &BotChannels<UIDL, UID, ENC, KH>;
    fn inited(&self)    -> bool;
    fn set_chans(&mut self, chans: BotChannels<UIDL, UID, ENC, KH>);

    // Provided.
    fn no_init(&self) -> bool {
        if !self.inited() {
            error!(sync_log::stream(), err!(
                "Attempt to start {} before running init()", self.label();
            Init, Missing));
            return true;
        }
        false
    }
    fn respond(
        &self,
        result: Outcome<OzoneMsg<UIDL, UID, ENC, KH>>,
        resp:   &Responder<UIDL, UID, ENC, KH>,
    ) {
        match resp.channel() {
            None => return,
            Some(simplex) => {
                let msg = match result {
                    Err(e) => OzoneMsg::Error(e),
                    Ok(m) => m,
                };
                let err_msg = format!(
                    "While trying to return a msg {:?} via a responder for ticket {}",
                    msg, resp.ticket(),
                );
                if let Err(e) = simplex.send(msg) {
                    self.err_cannot_send(err!(e, "{}", err_msg; Channel, Write));
                }
            },
        }
    }

    /// Message handling common to all Ozone bots.
    fn listen_more(&mut self, msg: OzoneMsg<UIDL, UID, ENC, KH>) -> LoopBreak {
        match msg {
            OzoneMsg::Finish => {
                trace!(sync_log::stream(), "{}: Finish message received, finishing now.", self.ozid());
                return LoopBreak(true);
            },
            OzoneMsg::Ready => info!(sync_log::stream(), "{} ready to receive messages now.", self.ozid()),
            OzoneMsg::Channels(chans, resp) => {
                self.set_chans(chans);
                match self.chans().get_bot(self.ozid()) {
                    Err(e) => self.error(e),
                    Ok(chan) => {
                        if self.chan_in().len() > 0 {
                            BotChannels::<UIDL, UID, ENC, KH>::dump_pending_messages(
                                self.chan_in().drain_messages(),
                                &fmt!("Updating {} channel, clearing out existing", self.ozid()),
                                None,
                                None,
                            );
                        }
                        let chan_clone = chan.clone();
                        self.set_chan_in(chan_clone);
                    },
                }
                self.respond(Ok(OzoneMsg::ChannelsReceived(self.ozid().clone())), &resp);
                trace!(sync_log::stream(), "{}: Channel update received.", self.ozid());
            },
            OzoneMsg::Ping(id, resp) => {
                if let Err(e) = resp.send(OzoneMsg::Pong(self.ozid().clone())) {
                    self.err_cannot_send(err!(e,
                        "Attempt to return a ping from {:?} failed", id;
                        IO, Channel));
                }
            },
            _ => error!(sync_log::stream(), err!("{}: Message {:?} not recognised.", self.ozid(), msg; Invalid, Input)),
        }
        LoopBreak(false)
    }
}

pub struct BotInitArgs<
    const UIDL: usize,
    UID:    NumIdDat<UIDL>,
    ENC:    Encrypter,
    KH:     Hasher,
	PR:     Hasher,
    CS:     Checksummer,
>{
    // Bot
    pub sem:            Semaphore,
    pub log_stream_id:  String,
    // Comms
    pub chan_in:    Simplex<OzoneMsg<UIDL, UID, ENC, KH>>,
    // API
    pub api:        OzoneApi<UIDL, UID, ENC, KH, PR, CS>,
}
