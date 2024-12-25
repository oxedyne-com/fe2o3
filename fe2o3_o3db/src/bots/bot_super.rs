use crate::{
    prelude::*,
    base::{
        constant,
        //id::OzoneBotType,
        index::{
            BotPoolInd,
            ZoneInd,
        },
    },
    bots::{
        base::{
            bot_deps::*,
            handles::{
                BotHandles,
                Handle,
            },
        },
        // Solo bots
        bot_config::ConfigBot,
        bot_server::ServerBot,
        // Other bots
        bot_zone::{
            ZoneBot,
            ZoneState,
        },
        worker::{
            bot::ZoneWorkerInitArgs,
            bot_cache::CacheBot,
            bot_file::FileBot,
            bot_initgc::InitGarbageBot,
            bot_reader::ReaderBot,
            bot_writer::WriterBot,
            worker_deps::*,
        },
    },
};

use oxedize_fe2o3_bot::Bot;
use oxedize_fe2o3_core::{
    channels::simplex,
    thread::thread_channel,
};
use oxedize_fe2o3_jdat::id::NumIdDat;

use std::{
    sync::Arc,
    time::{
        Duration,
        Instant,
    },
    thread,
};

/// Manages the files and caches for an ozone database.
#[derive(Debug)]
pub struct Supervisor<
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
    chan_out:   Simplex<OzoneMsg<UIDL, UID, ENC, KH>>, // to the Master.
    // API
    api:        OzoneApi<UIDL, UID, ENC, KH, PR, CS>,
    // State
    handles:    BotHandles<UIDL, UID, ENC, KH>,
    inited:     bool,
    trep:       Instant,
    zstats:     Vec<ZoneState>,
}

impl<
    const UIDL: usize,
    UID:    NumIdDat<UIDL> + 'static,
    ENC:    Encrypter + 'static,
    KH:     Hasher + 'static,
    PR:     Hasher + 'static,
    CS:     Checksummer + 'static,
>
    Bot<{ BID_LEN }, Bid, OzoneMsg<UIDL, UID, ENC, KH>> for Supervisor<UIDL, UID, ENC, KH, PR, CS>
{
    bot_methods!();

    fn go(&mut self) {
        if self.no_init() { return; }
        let result = self.start_db();
        self.result(&result);
        // Send channels back to Master for the first time.
        let resp = Responder::new(Some(&self.ozid()));
        if let Err(e) = self.chan_out.send(
            OzoneMsg::Channels(
                self.chans().clone(),
                resp.clone(),
            )
        ) {
            self.err_cannot_send(err!(e, errmsg!(
                "{}: Sending channels to master", self.ozid(),
            ), IO, Channel));
        }
        match resp.recv_timeout(constant::BOT_REQUEST_TIMEOUT) {
            Err(e) => self.error(e),
            Ok(OzoneMsg::ChannelsReceived(_)) => (),
            m => self.error(err!(errmsg!(
                "{}: Received {:?}, expecting ChannelsReceived confirmation.", self.ozid(), m,
            ))),
        }
        self.now_listening();
        loop {
            if self.listen().must_end() { break; }
        }
    }

    fn listen(&mut self) -> LoopBreak {
        let now = Instant::now();
        if now.duration_since(self.trep) >= constant::HEALTH_CHECK_INTERVAL {
            let result = self.report_health();
            self.result(&result);
            self.trep = now;
        }
        match self.chan_in().recv() {
            Err(e) => self.err_cannot_receive(err!(e, errmsg!(
                "{}: Waiting for message.", self.ozid(),
            ), IO, Channel)),
            Ok(msg) => match msg {
                OzoneMsg::ClearCache(_)              |
                OzoneMsg::DumpCacheRequest(_)        |
                OzoneMsg::DumpFiles(_)               |
                OzoneMsg::DumpFileStatesRequest(_)   |
                OzoneMsg::GcControl(_, _)            |
                OzoneMsg::GetZoneDir(_)              |
                //OzoneMsg::GetUsers(_)                |
                OzoneMsg::NewLiveFile(_, _)
                => {
                    match self.chans().fwd_to_all_zones(msg) {
                        Err(e) => self.error(e),
                        Ok(_) => (),
                    }
                },
                OzoneMsg::OzoneStateRequest(resp) => {
                    self.respond(Ok(OzoneMsg::OzoneStateResponse(
                        self.ozone_state().clone())), &resp);
                },
                OzoneMsg::Shutdown(ozid, resp) => {
                    if let OzoneBotId::Master(_) = ozid {
                        self.respond(self.shutdown(fmt!("{}", ozid)), &resp);
                        return LoopBreak(true);
                    } else {
                        self.respond(Err(err!(errmsg!(
                            "{} attempted to shut down database, but only the Master \
                            can do this.", ozid,
                        ))), &resp);
                    }
                },
                OzoneMsg::ZoneState(z, zstat) => {
                    //debug!("{}: zone {} state received: {:?}",self.ozid(),z,zstat);
                    if z+1 > self.ozone_state().len() {
                        self.error(err!(errmsg!(
                            "{}: The ZoneInd for a zone state update, {}, exceeds the \
                            number of zbot slots {}.", self.ozid(), z,
                            self.ozone_state().len(),
                        ), Bug, Mismatch, Size));
                    } else {
                        self.ozone_state_mut()[z] = zstat;
                    }
                },
                _ => return self.listen_more(msg),
            },
        }
        LoopBreak(false)
    }
}

impl<
    const UIDL: usize,
    UID:    NumIdDat<UIDL> + 'static,
    ENC:    Encrypter + 'static,
    KH:     Hasher + 'static,
	PR:     Hasher + 'static,
    CS:     Checksummer + 'static,
>
    OzoneBot<UIDL, UID, ENC, KH, PR, CS> for Supervisor<UIDL, UID, ENC, KH, PR, CS>
{
    ozonebot_methods!();
}

impl<
    const UIDL: usize,
    UID:    NumIdDat<UIDL> + 'static,
    ENC:    Encrypter + 'static,
    KH:     Hasher + 'static,
	PR:     Hasher + 'static,
    CS:     Checksummer + 'static,
>
    Supervisor<UIDL, UID, ENC, KH, PR, CS>
{
    pub fn new(
        args:       BotInitArgs<UIDL, UID, ENC, KH, PR, CS>,
        chan_out:   Simplex<OzoneMsg<UIDL, UID, ENC, KH>>,
    )
        -> Self
    {
        let handles = BotHandles::new(&args.api.cfg);
        let zstats = vec![ZoneState::default(); args.api.cfg.num_zones()];
        Self {
            // Bot
            sem:        args.sem,
            errc:       Arc::new(Mutex::new(0)),
            // Comms
            chan_in:    args.chan_in,
            chan_out,
            // API
            api:        args.api,
            // State
            handles,
            inited:     false,
            trep:       Instant::now(),
            zstats,
        }
    }

    pub fn schemes(&self)           -> &RestSchemes<ENC, KH, PR, CS>        { &self.api.schms }
    pub fn handles(&self)           -> &BotHandles<UIDL, UID, ENC, KH>      { &self.handles }
    pub fn chans_mut(&mut self)     -> &mut BotChannels<UIDL, UID, ENC, KH> { &mut self.api.chans }
    pub fn handles_mut(&mut self)   -> &mut BotHandles<UIDL, UID, ENC, KH>  { &mut self.handles }

    fn ozone_state(&self)           -> &Vec<ZoneState>      { &self.zstats }
    fn ozone_state_mut(&mut self)   -> &mut Vec<ZoneState>  { &mut self.zstats }

    pub fn start_db(&mut self) -> Outcome<()> {
        info!("{}: Starting database...", self.label());

        let nz = self.cfg().num_zones();
        let ns = self.cfg().num_sbots();

        // Arise bots!
        for z in 0..nz {
            let zind = ZoneInd::new(z);
            for wtyp in [
                WorkerType::Cache,
                WorkerType::File,
                WorkerType::InitGarbage,
                WorkerType::Reader,
                WorkerType::Writer,
            ] {
                for b in 0..self.cfg().num_bots_per_zone(&wtyp) {
                    let bpind = BotPoolInd::new(b);
                    let wind = WorkerInd::new(zind, bpind);
                    let handle = res!(self.start_new_worker(&wtyp, &wind));
                    let chan = res!(handle.some_chan());
                    res!(self.chans_mut().set_worker_bot(&wtyp, &wind, chan));
                    res!(self.handles_mut().set_worker_bot(&wtyp, &wind, handle));
                }
            }
        }

        // The zbots are initialised with cbot channels.
        for z in 0..nz {
            let zind = ZoneInd::new(z);
            let handle = res!(self.start_new_zbot(&zind));
            let chan = res!(handle.some_chan());
            res!(self.chans_mut().set_zbot(&zind, chan));
            res!(self.handles_mut().set_zbot(&zind, handle));
        }

        // Start cfgbot.
        let handle = res!(self.start_new_cfgbot());
        let chan = res!(handle.some_chan());
        self.chans_mut().set_cfg(chan);
        self.handles_mut().set_cfg(handle);

        // Start srvbots.
        for s in 0..ns {
            let bpind = BotPoolInd::new(s);
            let handle = res!(self.start_new_srvbot(bpind));
            let chan = res!(handle.some_chan());
            res!(self.chans_mut().set_sbot(&bpind, chan));
            res!(self.handles_mut().set_sbot(&bpind, handle));
        }
        
        // Broadcast updated BotChannels.
        let resp = Responder::new(Some(self.ozid()));
        let msg = OzoneMsg::Channels(self.chans().clone(), resp.clone());
        res!(self.chans().cfg().send(msg.clone()));
        res!(self.chans().fwd_to_all_zones(msg.clone()));
        for _ in 0..nz+1 {
            match res!(resp.recv_timeout(constant::BOT_REQUEST_TIMEOUT)) {
                OzoneMsg::ChannelsReceived(ozid) => {
                    info!("{}: Channels received by {}", self.ozid(), ozid);
                },
                m => self.error(err!(errmsg!(
                    "Received {:?}, expecting ChannelsReceived confirmation.", m,
                ))),
            }
        }

        res!(self.chans().cfg().send(OzoneMsg::ZoneInitTrigger));

        Ok(())
    }

    fn start_new_worker(
        &self, 
        wtyp:   &WorkerType,
        wind:   &WorkerInd,
    )
        -> Outcome<Handle<UIDL, UID, ENC, KH, ()>>
    {
        let chan = simplex();
        let (semaphore, sentinel) = thread_channel();
        let wg = self.handles().wait_end_ref().clone();
        let api = OzoneApi::new(
            OzoneBotId::new_worker(wtyp, wind),
            self.db_root().to_path_buf(),
            self.cfg().clone(),
            self.chans().clone(),
            self.schemes().clone(),
        );
        let args = ZoneWorkerInitArgs {
            // Identity
            wind:       wind.clone(),
            wtyp:       wtyp.clone(),
            // Bot
            sem:        semaphore,
            // Comms
            chan_in:    chan.clone(),
            // API
            api,
        };

        let mut bot: Box<dyn WorkerBot<UIDL, UID, ENC, KH, PR, CS>> = match wtyp {
            WorkerType::Cache       =>  Box::new(CacheBot::new(args)),
            WorkerType::File        =>  Box::new(FileBot::new(args)),
            WorkerType::InitGarbage =>  Box::new(InitGarbageBot::new(args)),
            WorkerType::Reader      =>  Box::new(ReaderBot::new(args)),
            WorkerType::Writer      =>  Box::new(WriterBot::new(args)),
        };
        res!(bot.init());
        let builder = thread::Builder::new()
            .name(bot.id().to_string())
            .stack_size(constant::STACK_SIZE);
        Ok(Handle::new(
            Some(bot.ozid().clone()),
            res!(builder.spawn(move || {
                bot.go();
                drop(wg);
            })),
            sentinel,
            Some(chan),
        ))
    }

    /// A zbot is neither a solo nor worker bot, since there is one per zone.
    fn start_new_zbot(
        &self, 
        zind: &ZoneInd,
    )
        -> Outcome<Handle<UIDL, UID, ENC, KH, ()>>
    {
        let chan = simplex();
        let (semaphore, sentinel) = thread_channel();
        let wg = self.handles().wait_end_ref().clone();
        let api = OzoneApi::new(
            OzoneBotId::ZoneBot(Bid::randef(), *zind),
            self.db_root().to_path_buf(),
            self.cfg().clone(),
            self.chans().clone(),
            self.schemes().clone(),
        );
        let args = BotInitArgs {
            // Bot
            sem:        semaphore,
            // Comms
            chan_in:    chan.clone(),
            // API
            api,
        };
        let mut bot = ZoneBot::new(args, *zind);
        res!(bot.init());
        let builder = thread::Builder::new()
            .name(bot.id().to_string())
            .stack_size(constant::STACK_SIZE);
        Ok(Handle::new(
            Some(bot.ozid().clone()),
            res!(builder.spawn(move || {
                bot.go();
                drop(wg);
            })),
            sentinel,
            Some(chan),
        ))
    }

    fn start_new_cfgbot(&self) -> Outcome<Handle<UIDL, UID, ENC, KH, ()>> {
        let chan = simplex();
        let (semaphore, sentinel) = thread_channel();
        let wg = self.handles().wait_end_ref().clone();
        let api = OzoneApi::new(
            OzoneBotId::ConfigBot(Bid::randef()),
            self.db_root().to_path_buf(),
            self.cfg().clone(),
            self.chans().clone(),
            self.schemes().clone(),
        );
        let args = BotInitArgs {
            // Bot
            sem:        semaphore,
            // Comms
            chan_in:    chan.clone(),
            // API
            api,
        };
        let mut bot = ConfigBot::new(args);
        res!(bot.init());
        let builder = thread::Builder::new()
            .name(bot.ozid().to_string())
            .stack_size(constant::STACK_SIZE);
        Ok(Handle::new(
            Some(bot.ozid().clone()),
            res!(builder.spawn(move || {
                bot.go();
                drop(wg);
            })),
            sentinel,
            Some(chan),
        ))
    }

    fn start_new_srvbot(
        &self,
        bpind: BotPoolInd,
    )
        -> Outcome<Handle<UIDL, UID, ENC, KH, ()>>
    {
        let chan = simplex();
        let (semaphore, sentinel) = thread_channel();
        let wg = self.handles().wait_end_ref().clone();
        let api = OzoneApi::new(
            OzoneBotId::ServerBot(Bid::randef(), bpind),
            self.db_root().to_path_buf(),
            self.cfg().clone(),
            self.chans().clone(),
            self.schemes().clone(),
        );
        let args = BotInitArgs {
            // Bot
            sem:        semaphore,
            // Comms
            chan_in:    chan.clone(),
            // API
            api,
        };
        let mut bot = ServerBot::new(args);
        res!(bot.init());
        let builder = thread::Builder::new()
            .name(bot.ozid().to_string())
            .stack_size(constant::STACK_SIZE);
        Ok(Handle::new(
            Some(bot.ozid().clone()),
            res!(builder.spawn(move || {
                bot.go();
                drop(wg);
            })),
            sentinel,
            Some(chan),
        ))
    }

    fn report_health(&self) -> Outcome<()> {
        let (expected, unresponsive) = res!(self.handles.get_unresponsive_bots(constant::PING_TIMEOUT));
        if unresponsive.len() > 0 {
            warn!("{} out of {} bots are unresponsive after {:?}:",
                unresponsive.len(), expected, constant::PING_TIMEOUT);
            for ozid in &unresponsive {
                warn!(" {:?} is unresponsive", ozid);
            }
            let dead = res!(self.handles.get_dead_bots());
            if dead.len() > 0 {
                error!(err!(errmsg!(
                    "{} out of {} bots are dead:", dead.len(), expected,
                ), Thread, Missing));
                for ozid in &unresponsive {
                    fault!(" {:?} is dead", ozid);
                }
            }
        } else {
            info!("{}: Ozone database health is good.", self.ozid());
        }
        Ok(())
    }

    /// Gracefully shut down the database.
    pub fn shutdown(&self, requester: String) -> Outcome<OzoneMsg<UIDL, UID, ENC, KH>> {
        warn!("{}: Shutdown requested by {}, commencing...", self.label(), requester);
        res!(self.chans().finish_all());
        thread::sleep(Duration::from_secs(1));
        self.handles().report_status();
        Ok(OzoneMsg::Ok)
    }
}

