use crate::{
    prelude::*,
    base::index::ZoneInd,
    bots::base::bot_deps::*,
    comm::channels::{
        BotChannels,
        ChannelPool,
    },
    file::zdir::{
        ZoneDir,
        ZoneDirStr,
    },
    format_zone_dir,
};

use oxedize_fe2o3_core::channels::Recv;
use oxedize_fe2o3_jdat::{
    prelude::*,
    id::NumIdDat,
};

use std::{
    collections::BTreeMap,
    path::PathBuf,
    str::FromStr,
    sync::Arc,
    time::{
        Duration,
        SystemTime,
    },
};

/// Watches the config file.
pub struct ConfigBot<
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
    last:       SystemTime, // Last check of file
    sleep:      Duration,
}

impl<
    const UIDL: usize,
    UID:    NumIdDat<UIDL> + 'static,
    ENC:    Encrypter + 'static,
    KH:     Hasher + 'static,
	PR:     Hasher,
    CS:     Checksummer,
>
    Bot<{ BID_LEN }, Bid, OzoneMsg<UIDL, UID, ENC, KH>> for ConfigBot<UIDL, UID, ENC, KH, PR, CS>
{
    bot_methods!();

    fn go(&mut self) {
        if self.no_init() { return; }
        info!("{}: Checking config file for changes every {:?}.", self.ozid(), self.sleep);
        self.now_listening();
        loop {
            if self.listen().must_end() { break; }
            //let result = self.check_file();
            //self.result(result);
        }
    }

    fn listen(&mut self) -> LoopBreak {
        match self.chan_in().recv_timeout(self.sleep) {
            Recv::Empty => (),
            Recv::Result(Err(e)) => self.err_cannot_receive(err!(e, errmsg!(
                "{}: Waiting for message.", self.ozid(),
            ), IO, Channel)),
            Recv::Result(Ok(OzoneMsg::ZoneInitTrigger)) => {
                let result = self.zone_init();
                self.result(&result);
            },
            // ... listen here for custom messages
            Recv::Result(Ok(msg)) => return self.listen_more(msg),
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
    OzoneBot<UIDL, UID, ENC, KH, PR, CS> for ConfigBot<UIDL, UID, ENC, KH, PR, CS>
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
    ConfigBot<UIDL, UID, ENC, KH, PR, CS>
{
    pub fn new(
        args: BotInitArgs<UIDL, UID, ENC, KH, PR, CS>,
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
            last:       SystemTime::now(),
            sleep:      Duration::from_secs(constant::CONFIGWATCHER_CHECK_INTERVAL_SECS),
        }
    }
    
    fn zbots(&self) -> &ChannelPool<UIDL, UID, ENC, KH> { &self.chans().all_zbots() }

    pub fn zone_init(&mut self) -> Outcome<()> {
        let zcfg = self.cfg().zone_config();
        let default = ZoneDir {
            dir:        self.cfg().zone_root(self.db_root()),
            max_size:   constant::DEFAULT_MAX_ZONE_DIR_BYTES,
        };
        let zdirs = res!(self.process_zone_overrides());
        for z in 0..self.cfg().num_zones {
            let zind = ZoneInd::new(z);
            let mut zdir = match zdirs.get(&z) {
                None => default.clone(),
                Some(zd) => zd.clone(),
            };
            zdir.dir.push(fmt!(format_zone_dir!(), z+1));
            if !zdir.dir.exists() {
                res!(std::fs::create_dir(&zdir.dir));
            }
            let bot = res!(self.zbots().get_bot(z as usize));
            if let Err(e) = bot.send(OzoneMsg::ZoneInit(zdir, zcfg.clone())) {
                self.err_cannot_send(err!(e, errmsg!(
                    "{}: Sending init data to zone {:?}", self.ozid(), zind,
                ), Init, IO, Channel));
            }
        }
        Ok(())
    }

    /// The configuration may contain a map `BTreeMap<Dat, Dat>` of zones settings that
    /// override the default.  This method turns the map into a map `BTreeMap<u16, ZoneDir>`.
    pub fn process_zone_overrides(&self) -> Outcome<BTreeMap<u16, ZoneDir>> {
        let zones = self.cfg().zone_overrides();
        if self.cfg().num_zones() < zones.len() {
            return Err(err!(errmsg!(
                "{}: There are {} zone overrides but only {} zones in the configuration.",
                self.ozid(), zones.len(), self.cfg().num_zones,
            ), Invalid, Input));
        }
        let mut zdirs = BTreeMap::new();
        for (zdat, zmapdat) in zones.iter() {
            match zdat {
                Dat::U16(z) => {
                    if *z == 0 {
                        return Err(err!(errmsg!(
                            "{}: The configured zone {} override cannot index from 0, \
                            zones are indexed from 1.", self.ozid(), z,
                        ), Invalid, Input));
                    }
                    match zmapdat {
                        Dat::Map(zmap) => {
                            // Duplicate keys should be detected in from_datmap
                            let zdstr: ZoneDirStr = res!(ZoneDirStr::from_datmap(zmap.clone()));
                            let zdir = {
                                let mut raw_path = if zdstr.dir.len() == 0 {
                                    PathBuf::from(self.db_root())
                                } else {
                                    res!(PathBuf::from_str(&zdstr.dir))
                                };
                                if !raw_path.is_absolute() {
                                    // Relative paths are relative to the db_root
                                    let mut tmp = PathBuf::from(self.db_root());
                                    tmp.push(&raw_path);
                                    raw_path = tmp;
                                }
                                let canonical_path_str =
                                    res!(OzoneConfig::canonicalize_path(raw_path));
                                let dir = self.cfg().zone_root(&Path::new(&canonical_path_str));
                                res!(std::fs::create_dir_all(&dir));
                                ZoneDir {
                                    dir,
                                    max_size: zdstr.max_size,
                                }
                            };
                            zdirs.insert(*z - 1, zdir);
                        },
                        _ => return Err(err!(errmsg!(
                            "{}: The configured zone {} override '{:?}' is not a Dat::Map.",
                            self.ozid(), z, zmapdat,
                        ), Invalid, Input)),
                    }
                },
                _ => return Err(err!(errmsg!(
                    "{}: The zone number in the configured zone override map, '{:?}',
                    is not a Dat::U16.", self.ozid(), zdat,
                ), Invalid, Input)),
            }
        }
        Ok(zdirs)
    }

    //fn open_file(&mut self) -> Outcome<File> {
    //    match OpenOptions::new()
    //        .read(true)
    //        .open(&self.cfg.path)
    //    {
    //        Err(e) => Err(err!(e, errmsg!("While opening file {:?}", self.cfg.path),
    //            IO, File, "read")),
    //        Ok(file) => Ok(file),
    //    }
    //}

    //fn check_file(&mut self) -> Outcome<()> {

    //    // 1. Inspect the file metadata to see if it has been modified since last check.
    //    // We cannot re-use an existing file handle because, at least in linux, this does not
    //    // provide updated metadata.
    //    match res!(self.open_file()).metadata() {
    //        Err(e) => return Err(err!(e, errmsg!(
    //            "Could not read metadata for file {:?}.", self.cfg.path,
    //        ), File, "read")),
    //        Ok(metadata) => match metadata.modified() {
    //            Err(_) => {
    //                // 1.1. This platform does not provide file modification time, so proceed to
    //                //   read the file as if it has been modified.
    //            },
    //            Ok(modtime) => {
    //                if modtime < self.last {
    //                    // 1.1. File has not been modified since last check, nothing to be done.
    //                    self.last = SystemTime::now();
    //                    return Ok(());
    //                }
    //            },
    //        },
    //    }
    //    
    //    
    //    self.last = SystemTime::now();

    //    // 2. The file has been (or is assumed to have been) modified, so read it.
    //    match std::fs::read_to_string(&self.cfg.path) {
    //        Ok(s) => {
    //            // 2.1. Ensure the data is a valid Dat.
    //            if let Dat::Map(map) = Dat::decode_string(s)? {
    //                let cfg_dat = ConfigData::from_datmap(map);
    //                let mut cfgdat_write = lock_write!(self.cfg.dat,
    //                    "While attempting to open the config data for writing.",
    //                );

    //                if *cfgdat_write == cfg_dat {
    //                    // 2.2. There is no change in the config file.
    //                    return Ok(());
    //                }

    //                info!("{}: Configuration file change detected, applying changes...",
    //                    self.ozid());

    //                // 3. TODO Manage changes.
    //                // 3.1. Change to number of zones: re-initialise database, activate re-zoning
    //                //   bots to re-allocate data.
    //                // 3.2. Change to zone locations: re-initialise zones, activate re-zoning bots
    //                //   to copy files.
    //                // 3.3. Changes to number of bots: instruct supervisor to create/delete.
    //                // 3.4. Other changes to flags and parameters, no special action required.
    //                
    //            } else {
    //                return Err(err!(errmsg!(
    //                    "O3db config file at '{:?}' must be a Dat::Map",
    //                    self.cfg.path,
    //                ), Input, Invalid));
    //            }
    //        },
    //        Err(e) => return Err(err!(e, errmsg!(
    //            "While trying to read configuration file {:?}.", self.cfg.path,
    //        ), IO, File)),
    //    }

    //    Ok(())

    //}
}
