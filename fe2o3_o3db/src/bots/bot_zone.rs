use crate::{
    prelude::*,
    base::{
        constant,
        index::ZoneInd,
    },
    bots::{
        base::bot_deps::*,
        worker::bot::WorkerType,
    },
    comm::{
        channels::{
            BotChannels,
            ChooseBot,
            ZoneWorkerChannels,
        },
        response::Responder,
    },
    file::{
        core::FileEntry,
        floc::FileNum,
        state::{
            FileState,
            FileStateMap,
            Present,
        },
        zdir::ZoneDir,
    },
};

use oxedize_fe2o3_core::{
    channels::Recv,
};
use oxedize_fe2o3_data::time::Timestamp;
use oxedize_fe2o3_jdat::id::NumIdDat;

use std::{
    collections::BTreeMap,
    fs::{
        self,
        File,
    },
    sync::Arc,
    time::Instant,
};

#[derive(Clone, Debug, Default)]
pub struct Resource {
    pub size:           usize,
    pub ancillary_size: usize,
    pub time:           Timestamp,
}

#[derive(Clone, Debug, Default)]
pub struct ZoneState {
    pub caches: Vec<Resource>,
    pub files:  Vec<Resource>,
}

#[derive(Debug)]
pub struct ZoneBot<
    const UIDL: usize,
    UID:    NumIdDat<UIDL>,
    ENC:    Encrypter,
    KH:     Hasher,
	PR:     Hasher,
    CS:     Checksummer,
>{
    // Identity
    zind:       ZoneInd,
    // Bot
    sem:        Semaphore,
    errc:       Arc<Mutex<usize>>,
    // Config
    zdir:       ZoneDir,
    // Comms
    chan_in:    Simplex<OzoneMsg<UIDL, UID, ENC, KH>>,
    // API
    api:        OzoneApi<UIDL, UID, ENC, KH, PR, CS>,
    // State
    active:     bool,
    fnum:       FileNum,
    igbot_live: bool,
    inited:     bool,
    size:       usize,
    trep:       Instant,
    zstat:      ZoneState,
}

impl<
    const UIDL: usize,
    UID:    NumIdDat<UIDL> + 'static,
    ENC:    Encrypter + 'static,
    KH:     Hasher + 'static,
	PR:     Hasher,
    CS:     Checksummer,
>
    Bot<{ BID_LEN }, Bid, OzoneMsg<UIDL, UID, ENC, KH>> for ZoneBot<UIDL, UID, ENC, KH, PR, CS>
{
    bot_methods!();

    fn go(&mut self) {
        if self.no_init() { return; }
        self.now_listening();
        loop {

            if self.trep.elapsed() > self.cfg().zone_state_update_interval() {
                // Automated state reporting.
                self.trep = Instant::now();
                if let Err(e) = self.chans().sup().send(
                    OzoneMsg::ZoneState(*self.zind, self.zone_state().clone())
                ) {
                    self.result(&Err(err!(e, errmsg!(
                        "{}: Cannot send zone state update to supervisor.", self.ozid(),
                    ), Channel, Write)));
                }
            }

            if self.listen().must_end() { break; }
        }
    }

    fn listen(&mut self) -> LoopBreak {
        match self.chan_in().recv_timeout(self.cfg().zone_state_update_interval()) {
            Recv::Result(Err(e)) => self.err_cannot_receive(err!(e, errmsg!(
                "{}: Waiting for message.", self.ozid(),
            ), IO, Channel)),
            Recv::Result(Ok(msg)) => match msg {
                // COMMAND
                OzoneMsg::Channels(chans, resp) => {
                    self.set_chans(chans);
                    match self.chans().get_bot(&self.ozid()) {
                        Err(e) => self.error(e),
                        Ok(chan) => {
                            let chan_clone = chan.clone();
                            self.set_chan_in(chan_clone);
                        },
                    }
                    let resp2 = Responder::new(Some(self.ozid()));
                    match self.broadcast(OzoneMsg::Channels(self.chans().clone(), resp2.clone())) {
                        Err(e) => self.error(e),
                        Ok(zwbots) => {
                            for _ in 0..zwbots.total_bot_count() {
                                match resp2.recv_timeout(constant::BOT_REQUEST_TIMEOUT) {
                                    Err(e) => self.error(e),
                                    Ok(OzoneMsg::ChannelsReceived(_)) => (),
                                    m => self.error(err!(errmsg!(
                                        "Received {:?}, expecting ChannelsReceived confirmation.", m,
                                    ))),
                                }
                            }
                        },
                    }
                    let result = resp.send(OzoneMsg::ChannelsReceived(self.ozid().clone()));
                    self.result(&result);
                },
                //OzoneMsg::ClearCache(_) | OzoneMsg::GetUsers(_) => {
                OzoneMsg::ClearCache(_) => {
                    match self.fwd_msg_to_pool(&WorkerType::Cache, msg) {
                        Err(e) => self.error(e),
                        Ok(_) => (),
                    }
                },
                OzoneMsg::GcControl(gc_ctrl, resp) => {
                    let result = match self.fwd_msg_to_pool(&WorkerType::File,
                        OzoneMsg::GcControl(gc_ctrl, Responder::none(Some(self.ozid()))),
                    ) {
                        Err(e) => Err(e),
                        Ok(_) => Ok(OzoneMsg::Ok),
                    };
                    self.respond(result, &resp);
                },
                OzoneMsg::GetZoneDir(resp) => {
                    self.respond(Ok(OzoneMsg::ZoneDir(*self.zind(), self.zdir().clone())), &resp);
                },
                OzoneMsg::ZoneInit(zdir, zcfg) => {
                    // Zone configuration.
                    self.zone_state_mut().caches = vec![Resource::default(); zcfg.ncbots];
                    self.zone_state_mut().files = vec![Resource::default(); zcfg.nfbots];
                    let msg = OzoneMsg::SetCacheSizeLimit(zcfg.cache_size_lim);
                    match self.fwd_msg_to_pool(&WorkerType::Cache, msg) {
                        Err(e) => self.error(e),
                        Ok(_) => (),
                    }
                    // Survey zone files.
                    self.zdir = zdir.clone();
                    match self.broadcast(OzoneMsg::ZoneDir(*self.zind(), zdir)) {
                        Err(e) => self.error(e),
                        Ok(_) => (),
                    }
                    match self.survey_files() {
                        Ok(shards) => {
                            //// Initialize WriterBot live files.
                            //let n_w = self.cfg().num_wbots_per_zone;
                            //let result = self.init_writer_live_files(n_w);
                            //self.result(&result);

                            if zcfg.init_load_caches {
                                let result = self.init_caches(shards);
                                self.result(&result);
                            }
                        },
                        Err(e) => self.result(&Err(e)),
                    }
                    info!("{}: Zone init complete", self.ozid());
                },
                // WORK
                OzoneMsg::CacheSize(b, size, ancillary_size) => {
                    if b+1 > self.zone_state().caches.len() {
                        self.error(err!(errmsg!(
                            "{}: The BotPoolInd for a cache size update, {}, exceeds the \
                            number of cbot slots {}.", self.ozid(), b, self.zone_state().caches.len(),
                        ), Bug, Mismatch, Index, Size));
                    } else {
                        match Timestamp::now() {
                            Err(e) => self.error(e),
                            Ok(time) => {
                                self.zone_state_mut().caches[b] = Resource {
                                    size,
                                    ancillary_size,
                                    time,
                                };
                            },
                        }
                    }
                },
                OzoneMsg::DumpCacheRequest(_) => {
                    match self.fwd_msg_to_pool(&WorkerType::Cache, msg) {
                        Err(e) => self.error(e),
                        Ok(_) => (),
                    }
                },
                OzoneMsg::DumpFiles(resp) => {
                    match self.read_files() {
                        Err(e) => self.error(e),
                        Ok(map) => self.respond(Ok(OzoneMsg::Files(*self.zind(), map)), &resp),
                    }
                },
                OzoneMsg::DumpFileStatesRequest(_) => {
                    match self.fwd_msg_to_pool(&WorkerType::File, msg) {
                        Err(e) => self.error(e),
                        Ok(_) => (),
                    }
                },
                OzoneMsg::NewLiveFile(_, _) => {
                    match self.fwd_msg_to_pool(&WorkerType::Writer, msg) {
                        Err(e) => self.error(e),
                        Ok(_) => (),
                    }
                },
                OzoneMsg::NextLiveFile(resp) => {
                    // [4] Respond to the wbot request for the next live file in the sequence.
                    self.fnum += 1;
                    self.respond(Ok(OzoneMsg::UseLiveFile(self.fnum)), &resp);
                },
                OzoneMsg::ShardFileSize(b, size) => {
                    if b+1 > self.zone_state().files.len() {
                        self.error(err!(errmsg!(
                            "{}: The BotPoolInd for a file state shard size update, {}, exceeds the \
                            number of fbot slots {}.", self.ozid(), b, self.zone_state().files.len(),
                        ), Bug, Mismatch, Size));
                    } else {
                        match Timestamp::now() {
                            Err(e) => self.error(e),
                            Ok(time) => {
                                self.zone_state_mut().files[b] = Resource {
                                    size,
                                    ancillary_size: 0,
                                    time,
                                };
                            },
                        }
                    }
                },
                _ => return self.listen_more(msg),
            },
            Recv::Empty => (),
        }
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
    OzoneBot<UIDL, UID, ENC, KH, PR, CS> for ZoneBot<UIDL, UID, ENC, KH, PR, CS>
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
    ZoneBot<UIDL, UID, ENC, KH, PR, CS>
{
    pub fn new(
        args: BotInitArgs<UIDL, UID, ENC, KH, PR, CS>,
        zind: ZoneInd,
    )
        -> Self
    {
        Self {
            // Identity
            zind,
            // Bot
            sem:        args.sem,
            errc:       Arc::new(Mutex::new(0)),
            // Config
            zdir:       ZoneDir::default(),
            // Comms
            chan_in:    args.chan_in,
            // API
            api:        args.api,
            // State
            active:     true,
            fnum:       0,
            igbot_live: true,
            inited:     false,
            size:       0,
            trep:       Instant::now(),
            zstat:      ZoneState::default(),
        }
    }

    fn zind(&self)                  -> &ZoneInd             { &self.zind }
    fn zdir(&self)                  -> &ZoneDir             { &self.zdir }
    fn zone_state(&self)            -> &ZoneState           { &self.zstat }
    fn zone_state_mut(&mut self)    -> &mut ZoneState       { &mut self.zstat }

    fn get_zwbots(&self) -> Outcome<&ZoneWorkerChannels<UIDL, UID, ENC, KH>> {
        self.chans().get_zwbots(&self.zind())
    }

    fn fwd_msg_to_pool(
        &self,
        wtyp:   &WorkerType,
        msg:    OzoneMsg<UIDL, UID, ENC, KH>,
    )
        -> Outcome<usize>
    {
        let pool = &res!(self.get_zwbots())[wtyp];
        pool.send_to_all(msg)
    }

    fn broadcast(
        &mut self,
        msg: OzoneMsg<UIDL, UID, ENC, KH>,
    )
        -> Outcome<&ZoneWorkerChannels<UIDL, UID, ENC, KH>>
    {
        let zwbots = res!(self.get_zwbots());
        res!(zwbots[&WorkerType::Cache].send_to_all(msg.clone()));
        res!(zwbots[&WorkerType::File].send_to_all(msg.clone()));
        res!(zwbots[&WorkerType::InitGarbage].send_to_all(msg.clone()));
        res!(zwbots[&WorkerType::Reader].send_to_all(msg.clone()));
        res!(zwbots[&WorkerType::Writer].send_to_all(msg.clone()));
        Ok(zwbots)
    }

    pub fn activate(mut self) -> Self {
        self.active = true;
        self
    }

//    /// Survey the existing data and index files and send the file state maps to the zone file bots.
//    pub fn survey_files(&mut self) -> Outcome<Vec<FileStateMap>> {
//
//        info!("{}: Surveying files for zone {}...", self.ozid(), self.zind());
//
//        let mut shards = Vec::new();
//        let nf = self.cfg().num_fbots_per_zone();
//        let mut dir_size: usize = 0;
//        let mut max_data_fnum: u32 = 0;
//        let mut max_data_size: usize = 0;
//        // 1. Count the number of files in the zone directory.
//        // <deleted>
//
//        trace!("{}: Initializing {} file state shards", self.ozid(), nf);
//
//        // 2. We will record whether both a data and index file are present for each file
//        //    number (i.e. Present::Pair) or whether just one is present (i.e.
//        //    Present::Solo(FileType)).
//        for _ in 0..nf {
//            shards.push(FileStateMap::default());
//        }
//
//        // 3. Loop through all objects in the zone directory, looking for files.
//        for entry in res!(std::fs::read_dir(&self.zdir().dir)) {
//            let entry = res!(entry);
//            let path = entry.path();
//            if path.is_file() {
//                trace!("{}: Processing file {:?}", self.ozid(), path);
//                // 4. We found a file, now extract the file number and type from the file name.
//                let (fnum, ftyp) = res!(ZoneDir::ozone_file_number_and_type(&path));
//                // 5. Get the file size.
//                let file = res!(std::fs::OpenOptions::new()
//                    .read(true)
//                    .open(&path)
//                );
//                let meta = res!(file.metadata());
//                let flen = meta.len() as usize;
//                dir_size += flen;
//                // 6. Keep a track of the highest data file number, in order to set the live
//                //    filenumber counter for the zone later.
//                if ftyp == FileType::Data {
//                    if fnum > max_data_fnum {
//                        max_data_fnum = fnum;
//                        max_data_size = flen;
//                    }
//                }
//
//                // 7. Determine and record the state of the files for this file number, for use
//                //    later by the `init_caches` method.
//                let i = FileStateMap::shard_index(fnum, nf);
//                match shards[i].map_mut().get_mut(&fnum) {
//                    None => {
//                        let mut fs = FileState::default();
//                        fs.set_present(Present::Solo(ftyp));
//                        match ftyp {
//                            FileType::Data => fs.set_data_file_size(flen),
//                            FileType::Index => fs.set_index_file_size(flen),
//                        }
//                        shards[i].map_mut().insert(fnum, fs);
//                    },
//                    Some(fs) => {
//                        match fs.present().clone() {
//                            Present::Solo(ftyp2) => {
//                                if ftyp2 != ftyp {
//                                    fs.set_present(Present::Pair);
//                                    match ftyp {
//                                        FileType::Data => fs.set_data_file_size(flen),
//                                        FileType::Index => fs.set_index_file_size(flen),
//                                    }
//                                } else {
//                                    return Err(err!(errmsg!(
//                                        "The file {:?} has already been surveyed, the file system \
//                                        should not permit such duplicate files.", path,
//                                    ), Unreachable, Bug));
//                                }
//                            },
//                            Present::Pair => return Err(err!(errmsg!(
//                                "The file {:?} has already been surveyed, the file system \
//                                should not permit such duplicate files.", path,
//                            ), Unreachable, Bug)),
//                        }
//                    },
//                }
//            }
//        }
//        // 8. Set the live file number for the zone.  The function ozone_file_number_and_type
//        //    ensures that max_data_filenum does not exceed u32::MAX.  The
//        //    zone::Zone::init_live method increments the first live file in the zone by one,
//        //    so to avoid restarts simply creating additional empty files, wind the counter
//        //    back one when necessary.
//        let max_data_file_size_ratio =
//            (max_data_size as f64) / (self.cfg().data_file_max_bytes as f64);
//        if max_data_fnum > 0
//            && max_data_file_size_ratio < constant::LIVE_FILE_INIT_SIZE_RATIO_THRESHOLD {
//            max_data_fnum = max_data_fnum - 1;
//        }
//        self.fnum = max_data_fnum;
//        // 9. Set the directory size for the zone.
//        self.size = dir_size;
//
//        Ok(shards)
//    }


    /// Survey the existing data and index files and send the file state maps to the zone file bots.
    pub fn survey_files(&mut self) -> Outcome<Vec<FileStateMap>> {
    
        info!("{}: Surveying {} files...", self.ozid(), self.zind());
    
        let mut shards = Vec::new();
        let nf = self.cfg().num_fbots_per_zone();
        let mut dir_size: usize = 0;
        let mut max_data_fnum: u32 = 0;
        let mut max_data_size: usize = 0;
        // Track incomplete data files for WriterBot initialization.
        let mut incomplete_files = Vec::new();
    
        trace!("{}: Initializing {} file state shards", self.ozid(), nf);
    
        // 2. We will record whether both a data and index file are present for each file
        //    number (i.e. Present::Pair) or whether just one is present (i.e.
        //    Present::Solo(FileType)).
        for _ in 0..nf {
            shards.push(FileStateMap::default());
        }
    
        // 3. Loop through all objects in the zone directory, looking for files.
        for entry in res!(std::fs::read_dir(&self.zdir().dir)) {
            let entry = res!(entry);
            let path = entry.path();
            if path.is_file() {
                trace!("{}: Processing file {:?}", self.ozid(), path);
                // 4. We found a file, now extract the file number and type from the file name.
                let (fnum, ftyp) = res!(ZoneDir::ozone_file_number_and_type(&path));
                // 5. Get the file size.
                let file = res!(std::fs::OpenOptions::new()
                    .read(true)
                    .open(&path)
                );
                let meta = res!(file.metadata());
                let flen = meta.len() as usize;
                dir_size += flen;
                // 6. Keep a track of the highest data file number, in order to set the live
                //    filenumber counter for the zone later.
                if ftyp == FileType::Data {
                    if fnum > max_data_fnum {
                        max_data_fnum = fnum;
                        max_data_size = flen;
                    }
                    // Check if this is an incomplete data file.
                    let ratio = flen as f64 / self.cfg().data_file_max_bytes as f64;
                    if ratio < constant::LIVE_FILE_INIT_SIZE_RATIO_THRESHOLD {
                        incomplete_files.push((fnum, flen));
                    }
                }
    
                // 7. Determine and record the state of the files for this file number, for use
                //    later by the `init_caches` method.
                let i = FileStateMap::shard_index(fnum, nf);
                match shards[i].map_mut().get_mut(&fnum) {
                    None => {
                        let mut fs = FileState::default();
                        fs.set_present(Present::Solo(ftyp));
                        match ftyp {
                            FileType::Data => fs.set_data_file_size(flen),
                            FileType::Index => fs.set_index_file_size(flen),
                        }
                        shards[i].map_mut().insert(fnum, fs);
                    },
                    Some(fs) => {
                        match fs.present().clone() {
                            Present::Solo(ftyp2) => {
                                if ftyp2 != ftyp {
                                    fs.set_present(Present::Pair);
                                    match ftyp {
                                        FileType::Data => fs.set_data_file_size(flen),
                                        FileType::Index => fs.set_index_file_size(flen),
                                    }
                                } else {
                                    return Err(err!(errmsg!(
                                        "The file {:?} has already been surveyed, the file system \
                                        should not permit such duplicate files.", path,
                                    ), Unreachable, Bug));
                                }
                            },
                            Present::Pair => return Err(err!(errmsg!(
                                "The file {:?} has already been surveyed, the file system \
                                should not permit such duplicate files.", path,
                            ), Unreachable, Bug)),
                        }
                    },
                }
            }
        }
    
        // Sort incomplete files by number descending.
        incomplete_files.sort_by(|a, b| b.0.cmp(&a.0));
    
        // Initialize WriterBot live files.
        let result = self.init_writer_live_files(&incomplete_files);
        self.result(&result);
    
        // 8. Set the live file number for the zone.  The function ozone_file_number_and_type
        //    ensures that max_data_filenum does not exceed u32::MAX.  The
        //    zone::Zone::init_live method increments the first live file in the zone by one,
        //    so to avoid restarts simply creating additional empty files, wind the counter
        //    back one when necessary.
        let max_data_file_size_ratio =
            (max_data_size as f64) / (self.cfg().data_file_max_bytes as f64);
        if max_data_fnum > 0
            && max_data_file_size_ratio < constant::LIVE_FILE_INIT_SIZE_RATIO_THRESHOLD {
            max_data_fnum = max_data_fnum - 1;
        }
        self.fnum = max_data_fnum;
        // 9. Set the directory size for the zone.
        self.size = dir_size;
    
        Ok(shards)
    }

    /// Assigns live files to WriterBots using discovered incomplete files.
    fn init_writer_live_files(
        &mut self,
        incomplete_files: &[(FileNum, usize)],
    )
        -> Outcome<()>
    {
        let n_w = self.cfg().num_wbots_per_zone as usize;
        let wbots = res!(self.get_zwbots())[&WorkerType::Writer].clone();
    
        trace!("{}: Assigning live files to {} writer bots.", self.ozid(), n_w);
    
        let resp = Responder::new(Some(self.ozid()));
        for i in 0..n_w {
            let fnum = if i < incomplete_files.len() {
                // Use existing incomplete file.
                incomplete_files[i].0
            } else {
                // There are not enough existing incomplete files to assign to wbots, create new
                // file number.
                self.fnum += 1;
                self.fnum
            };
    
            let wbot = res!(wbots.get_bot(i));
            res!(wbot.send(OzoneMsg::NewLiveFile(Some(fnum), resp.clone())));
        }

        let (_, msgs) = res!(resp.recv_number(n_w, constant::BOT_REQUEST_WAIT));
        for msg in msgs {
            match msg {
                OzoneMsg::Error(e) => return Err(err!(e, errmsg!(
                    "{}: In response to NewLiveFile message.", self.ozid(),
                ))),
                OzoneMsg::Ok => (),
                msg => return Err(err!(errmsg!(
                    "{}: Unexpected response to NewLiveFile message: {:?}", self.ozid(), msg,
                ))),
            };
        }
    
        Ok(())
    }

    pub fn read_files(&mut self) -> Outcome<BTreeMap<String, FileEntry>> {
        let mut map = BTreeMap::new();
        let list = res!(fs::read_dir(self.zdir().dir.clone()));
        for item in list {
            let item = res!(item);
            let path = item.path();
            let file = res!(File::open(&path));
            let metadata = res!(file.metadata());
            let ftyp = metadata.file_type();
            let typ = if ftyp.is_dir() {
                fmt!("d")
            } else if ftyp.is_symlink() {
                fmt!("s")
            } else {
                fmt!("f")
            };
            let elapsed = res!(metadata.modified()).elapsed();
            let mods = res!(elapsed).as_secs();
            //let mods = res!(res!(metadata.modified()).elapsed()).as_secs();
            let size = metadata.len();
            let name = match path.file_name() {
                Some(s) => match s.to_os_string().into_string() {
                    Ok(s) => s,
                    Err(_) => fmt!("{}", path.display()),
                }
                None => fmt!("{}", path.display()),
            };
            map.insert(name.clone(), FileEntry { typ, size, mods, name });
        }
        Ok(map)
    }

    /// Initialise the zone data caches based on file survey data. The survey collected all file
    /// sizes, but these will be recalculated as data is added to the cache.
    pub fn init_caches(
        &mut self,
        shards: Vec<FileStateMap>,
    )
        -> Outcome<()>
    {
        // 1. Prepare to make a bunch of requests to InitGarbageBots.
        let resp = Responder::new(Some(self.ozid()));
        let mut bot_requests = 0;

        for mut shard in shards {

            // 2. Loop through all previously surveyed files and check their status.
            let mut missing_data_files: Option<Vec<u32>> = None;
            for (fnum, fstate) in shard.map_mut() {
                match fstate.present() {
                    Present::Pair => {
                        // We have a data file and associated index file, and can therefore save
                        // some time by reading the (generally smaller) index file and loading its
                        // keys and file locations into the zone cache.  Send the caching request
                        // to a randomised InitGarbageBot.
                        // CACHE INDEX FILE - since both the index and data file is present.
                        
                        // Reset file sizes and relay to the igbot as a checksum. 
                        let dat_file_size = fstate.get_data_file_size();
                        let ind_file_size = fstate.get_index_file_size();
                        fstate.reset_data_file_size();
                        fstate.reset_index_file_size();

                        // Send caching request, with responder.
                        let (bot, j) = res!(self.get_zwbots())[&WorkerType::InitGarbage]
                            .choose_bot(&ChooseBot::Randomly);
                        if let Err(e) = bot.send(
                            OzoneMsg::CacheIndexFile {
                                fnum:       *fnum,
                                dat_file_size,
                                ind_file_size,
                                resp:       resp.clone(),
                            }
                        ) {
                            return Err(err!(e, errmsg!(
                                "Cannot send cache index file request to igbot {}", j,
                            ), Channel, Write));
                        } else {
                            bot_requests += 1;
                        }
                    },
                    Present::Solo(FileType::Data) => {
                        // We have only a data file, and can manually index the key and value pairs
                        // it contains.  The caching request will result in the re-creation of the
                        // index file.
                        // CACHE DATA FILE - since the index file is not present.
                        
                        // Reset file size and relay to the igbot as a checksum. 
                        let dat_file_size = fstate.get_data_file_size();
                        fstate.reset_data_file_size();

                        //// Send caching request, with responder.
                        let (bot, j) = res!(self.get_zwbots())[&WorkerType::InitGarbage]
                            .choose_bot(&ChooseBot::Randomly);
                        if let Err(e) = bot.send(
                            OzoneMsg::CacheDataFile {
                                fnum:       *fnum,
                                dat_file_size,
                                resp:       resp.clone(),
                            }
                        ) {
                            return Err(err!(e, errmsg!(
                                "Cannot send cache data file request to igbot {}", j,
                            ), Channel, Write));
                        } else {
                            bot_requests += 1;
                        }
                    },
                    Present::Solo(FileType::Index) => {
                        // An index file without a data file is bad, it means data has been lost.
                        // Keep a track of all such instances and warn the user.
                        match missing_data_files {
                            None => missing_data_files = Some(vec![*fnum]),
                            Some(ref mut missing) => missing.push(*fnum),
                        }
                    },
                }
            }
            // 3. Warn the user of missing data files.
            if let Some(missing) = missing_data_files {
                for fnum in missing {
                    warn!("{:?}: Data file {} is missing, suggesting loss of data.",
                        self.ozid(), fnum);
                }
            }
        }

        // 4. Wait for and collect all request responses.
        for _ in 0..bot_requests {
            match resp.recv_timeout(constant::BOT_REQUEST_TIMEOUT) {
                Err(e) => return Err(err!(e, errmsg!(
                    "While collecting cache initialisation request responses.",
                ), IO, Channel, Read)),
                Ok(OzoneMsg::Ok) => (),
                Ok(msg) => return Err(err!(errmsg!(
                    "Unrecognised cache initialisation request response: {:?}", msg)),
                ),
            }
        }

        Ok(())
    }
}
