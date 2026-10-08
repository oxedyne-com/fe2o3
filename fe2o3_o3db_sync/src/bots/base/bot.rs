use crate::{
    prelude::*,
    bots::worker::bot::WorkerType,
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
    file::floc::FileNum,
};

use oxedyne_fe2o3_bot::{
    bot::{
        Bot,
        LoopBreak,
    },
};
use oxedyne_fe2o3_core::{
    channels::Simplex,
    thread::Semaphore,
};
use oxedyne_fe2o3_jdat::id::NumIdDat;

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
        let _ = self.try_respond(result, resp);
    }

    /// As `respond`, and says whether the reply went to a requester still waiting for it, which is
    /// the question a bot that has given the requester something to give back must ask.
    fn try_respond(
        &self,
        result: Outcome<OzoneMsg<UIDL, UID, ENC, KH>>,
        resp:   &Responder<UIDL, UID, ENC, KH>,
    )
        -> bool
    {
        match resp.channel() {
            None => return false,
            Some(simplex) => {
                let msg = match result {
                    Err(e) => OzoneMsg::Error(e),
                    Ok(m) => m,
                };
                let err_msg = format!(
                    "While trying to return a msg {:?} via a responder for ticket {}",
                    msg, resp.ticket(),
                );
                match simplex.send_if_open(msg) {
                    Ok(sent) => sent,
                    Err(e) => {
                        self.err_cannot_send(err!(e, "{}", err_msg; Channel, Write));
                        false
                    },
                }
            },
        }
    }

    /// Tells every rbot to drop its cached handle on the given files, because a collection has
    /// just renamed new ones over them or compaction has deleted them, and a handle left on a
    /// deleted file keeps its bytes allocated on the disk.  Every pool in every zone is told: a
    /// file number is only unique within a zone, so a same-numbered file elsewhere is invalidated
    /// needlessly, but that costs one reopen and keeps the notice independent of which zone a
    /// reader serves.
    fn notify_file_replaced(
        &self,
        fnum:   FileNum,
        typs:   &[FileType],
    )
        -> Outcome<()>
    {
        for pool in self.chans().get_all_workers_of_type(&WorkerType::Reader) {
            for i in 0..pool.len() {
                let bot = res!(pool.get_bot(i));
                for typ in typs {
                    if let Err(e) = bot.send(OzoneMsg::FileReplaced(fnum, typ.clone())) {
                        return Err(err!(e,
                            "{}: Cannot tell rbot {} that {:?} file {} has been replaced.",
                            self.ozid(), i, typ, fnum;
                            Channel, Write));
                    }
                }
            }
        }
        Ok(())
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
                // A ping reports the bot's error tally as well as its liveness, so a
                // caller can tell a healthy bot from one that is running but failing.
                let errs = match self.error_count() {
                    Ok(n) => n,
                    Err(_) => usize::MAX,
                };
                if let Err(e) = resp.send(OzoneMsg::Pong(self.ozid().clone(), errs)) {
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
