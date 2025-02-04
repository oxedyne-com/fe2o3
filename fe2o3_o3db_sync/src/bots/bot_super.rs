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
    sem:            Semaphore,
    errc:           Arc<Mutex<usize>>,
    log_stream_id:  String,
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
    Bot<{ BID_LEN }, Bid, OzoneMsg<UIDL, UID, ENC, KH>> for
        Supervisor<UIDL, UID, ENC, KH, PR, CS>
{
    bot_methods!();

    fn go(&mut self) {

        sync_log::set_stream(self.log_stream_id());

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
            self.err_cannot_send(err!(e,
                "{}: Sending channels to master", self.ozid();
                IO, Channel));
        }
        match resp.recv_timeout(constant::BOT_REQUEST_TIMEOUT) {
            Err(e) => self.error(e),
            Ok(OzoneMsg::ChannelsReceived(_)) => (),
            m => self.error(err!(
                "{}: Received {:?}, expecting ChannelsReceived confirmation.", self.ozid(), m;
                Channel)),
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
            Err(e) => self.err_cannot_receive(err!(e,
                "{}: Waiting for message.", self.ozid();
                IO, Channel)),
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
                        self.respond(Err(err!(
                            "{} attempted to shut down database, but only the Master \
                            can do this.", ozid;
                            Unauthorised)), &resp);
                    }
                },
                OzoneMsg::ZoneState(z, zstat) => {
                    //debug!(sync_log::stream(), "{}: zone {} state received: {:?}",self.ozid(),z,zstat);
                    if z+1 > self.ozone_state().len() {
                        self.error(err!(
                            "{}: The ZoneInd for a zone state update, {}, exceeds the \
                            number of zbot slots {}.",
                            self.ozid(), z, self.ozone_state().len();
                            Bug, Mismatch, Size));
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
        args:           BotInitArgs<UIDL, UID, ENC, KH, PR, CS>,
        chan_out:       Simplex<OzoneMsg<UIDL, UID, ENC, KH>>,
    )
        -> Self
    {
        let handles = BotHandles::new(&args.api.cfg);
        let zstats = vec![ZoneState::default(); args.api.cfg.num_zones()];
        Self {
            // Bot
            sem:            args.sem,
            errc:           Arc::new(Mutex::new(0)),
            log_stream_id:  args.log_stream_id,
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

//    pub fn start_db(&mut self) -> Outcome<()> {
//
//        info!(sync_log::stream(), "{}: Starting database...", self.label());
//
//        let nz = self.cfg().num_zones();
//        let ns = self.cfg().num_sbots();
//
//        res!(thread::scope(|s| -> Outcome<()> {
//
//            // Arise bots!
//            for xz in 0..nz {
//                let zind = ZoneInd::new(xz);
//                for wtyp in [
//                    WorkerType::Cache,
//                    WorkerType::File,
//                    WorkerType::InitGarbage,
//                    WorkerType::Reader,
//                    WorkerType::Writer,
//                ] {
//                    for b in 0..self.cfg().num_bots_per_zone(&wtyp) {
//                        let bpind = BotPoolInd::new(b);
//                        let wind = WorkerInd::new(zind, bpind);
//                        let handle = res!(self.start_new_worker(s, &wtyp, &wind));
//                        let chan = res!(handle.some_chan());
//                        { res!(self.chans_mut().set_worker_bot(&wtyp, &wind, chan)); }
//                        { res!(self.handles_mut().set_worker_bot(&wtyp, &wind, handle)); }
//                    }
//                }
//            }
//
//            // The zbots are initialised with cbot channels.
//            for xz in 0..nz {
//                let zind = ZoneInd::new(xz);
//                let handle = res!(self.start_new_zbot(s, &zind));
//                let chan = res!(handle.some_chan());
//                { res!(self.chans_mut().set_zbot(&zind, chan)); }
//                { res!(self.handles_mut().set_zbot(&zind, handle)); }
//            }
//
//            // Start cfgbot.
//            let handle = res!(self.start_new_cfgbot(s));
//            let chan = res!(handle.some_chan());
//            { self.chans_mut().set_cfg(chan); }
//            { self.handles_mut().set_cfg(handle); }
//
//            // Start srvbots.
//            for xs in 0..ns {
//                let bpind = BotPoolInd::new(xs);
//                let handle = res!(self.start_new_srvbot(s, bpind));
//                let chan = res!(handle.some_chan());
//                { res!(self.chans_mut().set_sbot(&bpind, chan)); }
//                { res!(self.handles_mut().set_sbot(&bpind, handle)); }
//            }
//            
//            // Broadcast updated BotChannels.
//            let resp = Responder::new(Some(self.ozid()));
//            let msg = OzoneMsg::Channels(self.chans().clone(), resp.clone());
//            res!(self.chans().cfg().send(msg.clone()));
//            res!(self.chans().fwd_to_all_zones(msg.clone()));
//            for _ in 0..nz+1 {
//                match res!(resp.recv_timeout(constant::BOT_REQUEST_TIMEOUT)) {
//                    OzoneMsg::ChannelsReceived(ozid) => {
//                        info!(sync_log::stream(), "{}: Channels received by {}", self.ozid(), ozid);
//                    },
//                    m => self.error(err!(
//                        "Received {:?}, expecting ChannelsReceived confirmation.", m;
//                        Channel, Read, Unexpected)),
//                }
//            }
//
//            res!(self.chans().cfg().send(OzoneMsg::ZoneInitTrigger));
//
//            Ok(())
//
//        }));
//
//        Ok(())
//    }

//    pub fn start_db(&mut self) -> Outcome<()> {
//        info!(sync_log::stream(), "{}: Starting database...", self.label());
//    
//        let nz = self.cfg().num_zones();
//        let ns = self.cfg().num_sbots();
//    
//        // Collect all worker handles and channels before mutating `self`.
//        let mut worker_handles = Vec::new();
//        let mut worker_channels = Vec::new();
//    
//        // Collect all zbot handles and channels before mutating `self`.
//        let mut zbot_handles = Vec::new();
//        let mut zbot_channels = Vec::new();
//    
//        // Collect all srvbot handles and channels before mutating `self`.
//        let mut srvbot_handles = Vec::new();
//        let mut srvbot_channels = Vec::new();
//    
//        let (cfg_handle, cfg_chan) = res!(thread::scope(|s| -> Outcome<(
//            Handle<'s, UIDL, UID, ENC, KH, ()>,
//            Simplex<OzoneMsg<UIDL, UID, ENC, KH>>,
//        )> {
//            // Arise bots!
//            for xz in 0..nz {
//                let zind = ZoneInd::new(xz);
//                for wtyp in [
//                    WorkerType::Cache,
//                    WorkerType::File,
//                    WorkerType::InitGarbage,
//                    WorkerType::Reader,
//                    WorkerType::Writer,
//                ] {
//                    for b in 0..self.cfg().num_bots_per_zone(&wtyp) {
//                        let bpind = BotPoolInd::new(b);
//                        let wind = WorkerInd::new(zind, bpind);
//                        let handle = res!(self.start_new_worker(s, &wtyp, &wind));
//                        let chan = res!(handle.some_chan());
//                        worker_handles.push((wtyp.clone(), wind.clone(), handle));
//                        worker_channels.push((wtyp.clone(), wind.clone(), chan));
//                    }
//                }
//            }
//    
//            // The zbots are initialised with cbot channels.
//            for xz in 0..nz {
//                let zind = ZoneInd::new(xz);
//                let handle = res!(self.start_new_zbot(s, &zind));
//                let chan = res!(handle.some_chan());
//                zbot_handles.push((zind, handle));
//                zbot_channels.push((zind, chan));
//            }
//    
//            // Start cfgbot.
//            let handle = res!(self.start_new_cfgbot(s));
//            let chan = res!(handle.some_chan());
//            let cfg_handle = handle;
//            let cfg_chan = chan;
//    
//            // Start srvbots.
//            for xs in 0..ns {
//                let bpind = BotPoolInd::new(xs);
//                let handle = res!(self.start_new_srvbot(s, bpind));
//                let chan = res!(handle.some_chan());
//                srvbot_handles.push((bpind, handle));
//                srvbot_channels.push((bpind, chan));
//            }
//    
//            Ok((cfg_handle, cfg_chan))
//        }));
//    
//        // Now that the immutable borrows are no longer active, mutate `self`.
//        for (wtyp, wind, handle) in worker_handles {
//            res!(self.handles_mut().set_worker_bot(&wtyp, &wind, handle));
//        }
//        for (wtyp, wind, chan) in worker_channels {
//            res!(self.chans_mut().set_worker_bot(&wtyp, &wind, chan));
//        }
//    
//        for (zind, handle) in zbot_handles {
//            res!(self.handles_mut().set_zbot(&zind, handle));
//        }
//        for (zind, chan) in zbot_channels {
//            res!(self.chans_mut().set_zbot(&zind, chan));
//        }
//    
//        self.chans_mut().set_cfg(cfg_chan);
//        self.handles_mut().set_cfg(cfg_handle);
//    
//        for (bpind, handle) in srvbot_handles {
//            res!(self.handles_mut().set_sbot(&bpind, handle));
//        }
//        for (bpind, chan) in srvbot_channels {
//            res!(self.chans_mut().set_sbot(&bpind, chan));
//        }
//    
//        // Broadcast updated BotChannels.
//        let resp = Responder::new(Some(self.ozid()));
//        let msg = OzoneMsg::Channels(self.chans().clone(), resp.clone());
//        res!(self.chans().cfg().send(msg.clone()));
//        res!(self.chans().fwd_to_all_zones(msg.clone()));
//        for _ in 0..nz + 1 {
//            match res!(resp.recv_timeout(constant::BOT_REQUEST_TIMEOUT)) {
//                OzoneMsg::ChannelsReceived(ozid) => {
//                    info!(sync_log::stream(), "{}: Channels received by {}", self.ozid(), ozid);
//                }
//                m => self.error(err!(
//                    "Received {:?}, expecting ChannelsReceived confirmation.", m;
//                    Channel, Read, Unexpected)),
//            }
//        }
//    
//        res!(self.chans().cfg().send(OzoneMsg::ZoneInitTrigger));
//    
//        Ok(())
//    }

//    pub fn start_db(&mut self) -> Outcome<()> {
//        info!(sync_log::stream(), "{}: Starting database...", self.label());
//    
//        let nz = self.cfg().num_zones();
//        let ns = self.cfg().num_sbots();
//    
//        res!(thread::scope(|s| -> Outcome<()> {
//            // Start worker bots first
//            for xz in 0..nz {
//                let zind = ZoneInd::new(xz);
//                for wtyp in [
//                    WorkerType::Cache,
//                    WorkerType::File,
//                    WorkerType::InitGarbage,
//                    WorkerType::Reader,
//                    WorkerType::Writer,
//                ] {
//                    for b in 0..self.cfg().num_bots_per_zone(&wtyp) {
//                        let bpind = BotPoolInd::new(b);
//                        let wind = WorkerInd::new(zind, bpind);
//                        let handle = res!(self.start_new_worker(s, &wtyp, &wind));
//                        let chan = res!(handle.some_chan());
//                        res!(self.chans_mut().set_worker_bot(&wtyp, &wind, chan));
//                        res!(self.handles_mut().set_worker_bot(&wtyp, &wind, handle));
//                    }
//                }
//            }
//    
//            // Start zone bots
//            for xz in 0..nz {
//                let zind = ZoneInd::new(xz);
//                let handle = res!(self.start_new_zbot(s, &zind));
//                let chan = res!(handle.some_chan());
//                res!(self.chans_mut().set_zbot(&zind, chan));
//                res!(self.handles_mut().set_zbot(&zind, handle));
//            }
//    
//            // Start config bot
//            let cfg_handle = res!(self.start_new_cfgbot(s));
//            let cfg_chan = res!(cfg_handle.some_chan());
//            self.chans_mut().set_cfg(cfg_chan);
//            self.handles_mut().set_cfg(cfg_handle);
//    
//            // Start server bots
//            for xs in 0..ns {
//                let bpind = BotPoolInd::new(xs);
//                let handle = res!(self.start_new_srvbot(s, bpind));
//                let chan = res!(handle.some_chan());
//                res!(self.chans_mut().set_sbot(&bpind, chan));
//                res!(self.handles_mut().set_sbot(&bpind, handle));
//            }
//    
//            // Broadcast updated BotChannels
//            let resp = Responder::new(Some(self.ozid()));
//            let msg = OzoneMsg::Channels(self.chans().clone(), resp.clone());
//            res!(self.chans().cfg().send(msg.clone()));
//            res!(self.chans().fwd_to_all_zones(msg.clone()));
//    
//            for _ in 0..nz + 1 {
//                match res!(resp.recv_timeout(constant::BOT_REQUEST_TIMEOUT)) {
//                    OzoneMsg::ChannelsReceived(ozid) => {
//                        info!(sync_log::stream(), "{}: Channels received by {}", self.ozid(), ozid);
//                    }
//                    m => {
//                        return Err(err!(
//                            "Received {:?}, expecting ChannelsReceived confirmation.", m;
//                            Channel, Read, Unexpected
//                        ));
//                    }
//                }
//            }
//    
//            res!(self.chans().cfg().send(OzoneMsg::ZoneInitTrigger));
//    
//            Ok(())
//        }));
//    
//        Ok(())
//    }

    pub fn start_db(&mut self) -> Outcome<()> {
        
        info!(sync_log::stream(), "{}: Starting database...", self.label());

        let nz = self.cfg().num_zones();
        let ns = self.cfg().num_sbots();

        info!(sync_log::stream(), "{}: Starting worker bots...", self.label());

        for xz in 0..nz {
            let zind = ZoneInd::new(xz);
            for wtyp in [
                WorkerType::Cache,
                WorkerType::File,
                WorkerType::InitGarbage,
                WorkerType::Reader,
                WorkerType::Writer,
            ] {
                let nb = self.cfg().num_bots_per_zone(&wtyp);
                info!(sync_log::stream(), "{}: Starting {} {:?} bots...", self.label(), nb+1, wtyp);
                for b in 0..nb {
                    let bpind = BotPoolInd::new(b);
                    let wind = WorkerInd::new(zind, bpind);
                    
                    // Create worker bot inline instead of in a separate method
                    let chan = simplex();
                    let (semaphore, sentinel) = thread_channel();
                    let wg = self.handles().wait_end_ref().clone();
                    let api = OzoneApi::new(
                        OzoneBotId::new_worker(&wtyp, &wind),
                        self.db_root().to_path_buf(),
                        self.cfg().clone(),
                        self.chans().clone(),
                        self.schemes().clone(),
                    );
                    
                    let args = ZoneWorkerInitArgs {
                        wind: wind.clone(),
                        wtyp: wtyp.clone(),
                        sem: semaphore,
                        log_stream_id: self.log_stream_id(),
                        chan_in: chan.clone(),
                        api,
                    };

                    let mut bot: Box<dyn WorkerBot<UIDL, UID, ENC, KH, PR, CS>> = match wtyp {
                        WorkerType::Cache       => Box::new(CacheBot::new(args)),
                        WorkerType::File        => Box::new(FileBot::new(args)),
                        WorkerType::InitGarbage => Box::new(InitGarbageBot::new(args)),
                        WorkerType::Reader      => Box::new(ReaderBot::new(args)),
                        WorkerType::Writer      => Box::new(WriterBot::new(args)),
                    };
                    
                    res!(bot.init());
                    let builder = thread::Builder::new()
                        .name(bot.id().to_string())
                        .stack_size(constant::STACK_SIZE);

                    let ozid = bot.ozid().clone();
                    res!(builder.spawn(move || {
                        bot.go();
                        drop(wg);
                    }));
                    let handle = Handle::new(
                        Some(ozid),
                        sentinel,
                        Some(chan.clone()),
                    );

                    res!(self.chans_mut().set_worker_bot(&wtyp, &wind, chan));
                    res!(self.handles_mut().set_worker_bot(&wtyp, &wind, handle));
                }
            }
        }

        info!(sync_log::stream(), "{}: Starting {} zone bots...", self.label(), nz+1);
        for xz in 0..nz {
            let zind = ZoneInd::new(xz);
            let chan = simplex();
            let (semaphore, sentinel) = thread_channel();
            let wg = self.handles().wait_end_ref().clone();
            let api = OzoneApi::new(
                OzoneBotId::ZoneBot(Bid::randef(), zind),
                self.db_root().to_path_buf(),
                self.cfg().clone(),
                self.chans().clone(),
                self.schemes().clone(),
            );
            
            let args = BotInitArgs {
                sem: semaphore,
                log_stream_id: self.log_stream_id(),
                chan_in: chan.clone(),
                api,
            };

            let mut bot = ZoneBot::new(args, zind);
            res!(bot.init());
            let builder = thread::Builder::new()
                .name(bot.id().to_string())
                .stack_size(constant::STACK_SIZE);

            let ozid = bot.ozid().clone();
            res!(builder.spawn(move || {
                bot.go();
                drop(wg);
            }));
            let handle = Handle::new(
                Some(ozid),
                sentinel,
                Some(chan.clone()),
            );

            res!(self.chans_mut().set_zbot(&zind, chan));
            res!(self.handles_mut().set_zbot(&zind, handle));
        }

        info!(sync_log::stream(), "{}: Starting config bot...", self.label());
        let cfg_chan = simplex();
        let (cfg_semaphore, cfg_sentinel) = thread_channel();
        let cfg_wg = self.handles().wait_end_ref().clone();
        let cfg_api = OzoneApi::new(
            OzoneBotId::ConfigBot(Bid::randef()),
            self.db_root().to_path_buf(),
            self.cfg().clone(),
            self.chans().clone(),
            self.schemes().clone(),
        );
        
        let cfg_args = BotInitArgs {
            sem: cfg_semaphore,
            log_stream_id: self.log_stream_id(),
            chan_in: cfg_chan.clone(),
            api: cfg_api,
        };

        let mut cfg_bot = ConfigBot::new(cfg_args);
        res!(cfg_bot.init());
        let cfg_builder = thread::Builder::new()
            .name(cfg_bot.id().to_string())
            .stack_size(constant::STACK_SIZE);

        let cfg_ozid = cfg_bot.ozid().clone();
        res!(cfg_builder.spawn(move || {
            cfg_bot.go();
            drop(cfg_wg);
        }));
        let cfg_handle = Handle::new(
            Some(cfg_ozid),
            cfg_sentinel,
            Some(cfg_chan.clone()),
        );

        self.chans_mut().set_cfg(cfg_chan);
        self.handles_mut().set_cfg(cfg_handle);

        info!(sync_log::stream(), "{}: Starting {} server bots...", self.label(), ns+1);
        for xs in 0..ns {
            let bpind = BotPoolInd::new(xs);
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
                sem: semaphore,
                log_stream_id: self.log_stream_id(),
                chan_in: chan.clone(),
                api,
            };

            let mut bot = ServerBot::new(args);
            res!(bot.init());
            let builder = thread::Builder::new()
                .name(bot.id().to_string())
                .stack_size(constant::STACK_SIZE);

            let ozid = bot.ozid().clone();
            res!(builder.spawn(move || {
                bot.go();
                drop(wg);
            }));
            let handle = Handle::new(
                Some(ozid),
                sentinel,
                Some(chan.clone()),
            );

            res!(self.chans_mut().set_sbot(&bpind, chan));
            res!(self.handles_mut().set_sbot(&bpind, handle));
        }

        info!(sync_log::stream(), "{}: Broadcasting updated BotChannels...", self.label());
        // Broadcast updated BotChannels
        let resp = Responder::new(Some(self.ozid()));
        let msg = OzoneMsg::Channels(self.chans().clone(), resp.clone());
        res!(self.chans().cfg().send(msg.clone()));
        res!(self.chans().fwd_to_all_zones(msg.clone()));
        
        trace!(sync_log::stream(), "expecting {} channel receipt confirmations", nz+1);
        for i in 0..nz + 1 {
            match res!(resp.recv_timeout(constant::BOT_REQUEST_TIMEOUT)) {
                OzoneMsg::ChannelsReceived(ozid) => {
                    info!(sync_log::stream(), "{}: Channels received by {}", self.ozid(), ozid);
                }
                m => {
                    return Err(err!(
                        "Received {:?}, expecting ChannelsReceived confirmation.", m;
                        Channel, Read, Unexpected
                    ));
                }
            }
            trace!(sync_log::stream(), "i = {}", i+1);
        }
        info!(sync_log::stream(), "{}: All channel updates received by config bot and zone bots.", self.label());

        res!(self.chans().cfg().send(OzoneMsg::ZoneInitTrigger));

        info!(sync_log::stream(), "{}: Ozone database start up complete.", self.label());

        Ok(())
    }

//    fn start_new_worker(
//        &'s self, 
//        scope:  &'s thread::Scope<'s, '_>,
//        wtyp:   &WorkerType,
//        wind:   &WorkerInd,
//    )
//        -> Outcome<Handle<'s, UIDL, UID, ENC, KH, ()>>
//    {
//        let chan = simplex();
//        let (semaphore, sentinel) = thread_channel();
//        let wg = self.handles().wait_end_ref().clone();
//        let api = OzoneApi::new(
//            OzoneBotId::new_worker(wtyp, wind),
//            self.db_root().to_path_buf(),
//            self.cfg().clone(),
//            self.chans().clone(),
//            self.schemes().clone(),
//        );
//        let args = ZoneWorkerInitArgs {
//            // Identity
//            wind:       wind.clone(),
//            wtyp:       wtyp.clone(),
//            // Bot
//            sem:        semaphore,
//            // Comms
//            chan_in:    chan.clone(),
//            // API
//            api,
//        };
//
//        let mut bot: Box<dyn WorkerBot<UIDL, UID, ENC, KH, PR, CS>> = match wtyp {
//            WorkerType::Cache       =>  Box::new(CacheBot::new(args)),
//            WorkerType::File        =>  Box::new(FileBot::new(args)),
//            WorkerType::InitGarbage =>  Box::new(InitGarbageBot::new(args)),
//            WorkerType::Reader      =>  Box::new(ReaderBot::new(args)),
//            WorkerType::Writer      =>  Box::new(WriterBot::new(args)),
//        };
//        res!(bot.init());
//        let builder = thread::Builder::new()
//            .name(bot.id().to_string())
//            .stack_size(constant::STACK_SIZE);
//
//        Ok(Handle::new(
//            Some(bot.ozid().clone()),
//            res!(builder.spawn_scoped(scope, move || {
//                bot.go();
//                drop(wg);
//            })),
//            sentinel,
//            Some(chan),
//        ))
//    }
//
//    /// A zbot is neither a solo nor worker bot, since there is one per zone.
//    fn start_new_zbot(
//        &'s self, 
//        scope:  &'s thread::Scope<'s, '_>,
//        zind:   &ZoneInd,
//    )
//        -> Outcome<Handle<'s, UIDL, UID, ENC, KH, ()>>
//    {
//        let chan = simplex();
//        let (semaphore, sentinel) = thread_channel();
//        let wg = self.handles().wait_end_ref().clone();
//        let api = OzoneApi::new(
//            OzoneBotId::ZoneBot(Bid::randef(), *zind),
//            self.db_root().to_path_buf(),
//            self.cfg().clone(),
//            self.chans().clone(),
//            self.schemes().clone(),
//        );
//        let args = BotInitArgs {
//            // Bot
//            sem:        semaphore,
//            // Comms
//            chan_in:    chan.clone(),
//            // API
//            api,
//        };
//        let mut bot = ZoneBot::new(args, *zind);
//        res!(bot.init());
//        let builder = thread::Builder::new()
//            .name(bot.id().to_string())
//            .stack_size(constant::STACK_SIZE);
//
//        Ok(Handle::new(
//            Some(bot.ozid().clone()),
//            res!(builder.spawn_scoped(scope, move || {
//                bot.go();
//                drop(wg);
//            })),
//            sentinel,
//            Some(chan),
//        ))
//    }
//
//    fn start_new_cfgbot(
//        &'s self, 
//        scope: &'s thread::Scope<'s, '_>,
//    )
//        -> Outcome<Handle<'s, UIDL, UID, ENC, KH, ()>>
//    {
//        let chan = simplex();
//        let (semaphore, sentinel) = thread_channel();
//        let wg = self.handles().wait_end_ref().clone();
//        let api = OzoneApi::new(
//            OzoneBotId::ConfigBot(Bid::randef()),
//            self.db_root().to_path_buf(),
//            self.cfg().clone(),
//            self.chans().clone(),
//            self.schemes().clone(),
//        );
//        let args = BotInitArgs {
//            // Bot
//            sem:        semaphore,
//            // Comms
//            chan_in:    chan.clone(),
//            // API
//            api,
//        };
//        let mut bot = ConfigBot::new(args);
//        res!(bot.init());
//        let builder = thread::Builder::new()
//            .name(bot.ozid().to_string())
//            .stack_size(constant::STACK_SIZE);
//
//        Ok(Handle::new(
//            Some(bot.ozid().clone()),
//            res!(builder.spawn_scoped(scope, move || {
//                bot.go();
//                drop(wg);
//            })),
//            sentinel,
//            Some(chan),
//        ))
//    }
//
//    fn start_new_srvbot(
//        &'s self, 
//        scope:  &'s thread::Scope<'s, '_>,
//        bpind:  BotPoolInd,
//    )
//        -> Outcome<Handle<'s, UIDL, UID, ENC, KH, ()>>
//    {
//        let chan = simplex();
//        let (semaphore, sentinel) = thread_channel();
//        let wg = self.handles().wait_end_ref().clone();
//        let api = OzoneApi::new(
//            OzoneBotId::ServerBot(Bid::randef(), bpind),
//            self.db_root().to_path_buf(),
//            self.cfg().clone(),
//            self.chans().clone(),
//            self.schemes().clone(),
//        );
//        let args = BotInitArgs {
//            // Bot
//            sem:        semaphore,
//            // Comms
//            chan_in:    chan.clone(),
//            // API
//            api,
//        };
//        let mut bot = ServerBot::new(args);
//        res!(bot.init());
//        let builder = thread::Builder::new()
//            .name(bot.ozid().to_string())
//            .stack_size(constant::STACK_SIZE);
//
//        Ok(Handle::new(
//            Some(bot.ozid().clone()),
//            res!(builder.spawn_scoped(scope, move || {
//                bot.go();
//                drop(wg);
//            })),
//            sentinel,
//            Some(chan),
//        ))
//    }

    fn report_health(&self) -> Outcome<()> {
        let (expected, unresponsive) =
            res!(self.handles.get_unresponsive_bots(constant::PING_TIMEOUT));
        if unresponsive.len() > 0 {
            warn!(sync_log::stream(), "{} out of {} bots are unresponsive after {:?}:",
                unresponsive.len(), expected, constant::PING_TIMEOUT);
            for ozid in &unresponsive {
                warn!(sync_log::stream(), " {:?} is unresponsive", ozid);
            }
            let dead = res!(self.handles.get_dead_bots());
            if dead.len() > 0 {
                error!(sync_log::stream(), err!(
                    "{} out of {} bots are dead:", dead.len(), expected;
                    Thread, Missing));
                for ozid in &unresponsive {
                    fault!(" {:?} is dead", ozid);
                }
            }
        } else {
            info!(sync_log::stream(), "{}: Ozone database health is good.", self.ozid());
        }
        Ok(())
    }

    /// Gracefully shut down the database.
    pub fn shutdown(&self, requester: String) -> Outcome<OzoneMsg<UIDL, UID, ENC, KH>> {
        warn!(sync_log::stream(), "{}: Shutdown requested by {}, commencing...", self.label(), requester);
        res!(self.chans().finish_all());
        thread::sleep(Duration::from_secs(1));
        self.handles().report_status();
        Ok(OzoneMsg::Ok)
    }
}

