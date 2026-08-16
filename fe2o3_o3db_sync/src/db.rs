use crate::holder::Holder;
use crate::{
    prelude::*,
    base::{
        constant,
        id::{
            Bid,
            OzoneBotId,
        },
    },
    bots::{
        base::{
            bot::{
                BotInitArgs,
                OzoneBot,
            },
            handles::Handle,
        },
        bot_super::Supervisor,
    },
    comm::{
        channels::BotChannels,
        msg::OzoneMsg,
        response::Responder,
    },
    data::{
        core::{
            RestSchemes,
            RestSchemesInput,
        },
    },
    file::core::find_files,
};

use oxedyne_fe2o3_bot::Bot;
use oxedyne_fe2o3_core::{
    channels::{
        simplex,
        Simplex,
        Recv,
    },
    path::NormalPath,
    rand::RanDef,
    thread::thread_channel,
};
use oxedyne_fe2o3_jdat::{
    prelude::*,
    cfg::Config,
    file::JdatMapFile,
    id::NumIdDat,
};
use oxedyne_fe2o3_namex::id::{
    InNamex,
    NamexId,
};
use oxedyne_fe2o3_text::string::Stringer;

use std::{
    fs,
    path::{
        Path,
        PathBuf,
    },
    sync::{
        Arc,
        Mutex,
        RwLock,
    },
    thread,
    time::Duration,
};

use crossbeam_utils::sync::WaitGroup;


/// The main Ozone database struct.
///
/// # Storage specifications
/// The user can change the various data transformation schemes in three ways.
/// ## Invocation
/// Upon invocation, rest schemes conforming to the traits `oxedyne_fe2o3_iop_hash::csum::Checksummer`,
/// `oxedyne_fe2o3_iop_hash::api::Hasher`, `oxedyne_fe2o3_iop_crypto::enc::Encrypter` and
/// `oxedyne_fe2o3_iop_crypto::sign::Signer` can be given to the `O3db` instance.  When a scheme is not
/// provided, a hardwired default is set.
/// ## Configuration
/// Default schemes can be overridden at invocation or upon any subsequent configuration file
/// changes.  Schemes are limited to those provided in `oxedyne_fe2o3_hash` and `oxedyne_fe2o3_crypto`.
/// ## Per value basis
/// Finally, schemes for storage of data at rest can be set explicitely for any key-value pair,
/// overriding invocation or default schemes.
///
/// # Directory layout
/// The example below shows a database that has been used with 3 and 5 zones.  The database is
/// invoked with the absolute path to the db_root, and an optional `OzoneConfig` which is used if
/// a configuration file is not found.
///
/// ```ignore
///
/// /../my_o3db                         <- db_root with db_name, aka db_container
/// ├── config.jdat
/// ├── 003_zone                        <- zone_root
/// │   ├── zone_001                    <- zone_dir
/// │   │   ├── 000_000_001.dat
/// │   │   ├── 000_000_001.ind
/// │   │   ├── 000_000_002.dat
/// │   │   └── 000_000_002.ind
/// │   ├── zone_002                    <- zone_dir
/// │   │   ├── 000_000_001.dat
/// │   │   ├── 000_000_001.ind
/// │   │   ├── 000_000_002.dat
/// │   │   └── 000_000_002.ind
/// │   └── zone_003                    <- zone_dir
/// │       ├── 000_000_001.dat
/// │       ├── 000_000_001.ind
/// │       ├── 000_000_002.dat
/// │       └── 000_000_002.ind
/// └── 005_zone                        <- zone_root
///     ├── zone_002                    <- zone_dir
///     │   ├── 000_000_001.dat
///     │   └── 000_000_001.ind
///     ├── zone_003                    <- zone_dir
///     │   ├── 000_000_001.dat
///     │   └── 000_000_001.ind
///     ├── zone_004                    <- zone_dir
///     │   ├── 000_000_001.dat
///     │   └── 000_000_001.ind
///     └── zone_005                    <- zone_dir
///         ├── 000_000_001.dat
///         └── 000_000_001.ind
///
/// a_zone_container_dir                <- zone_container
/// └── 005_zone                        <- zone_root
///     └─── zone_001                   <- zone_dir
///         ├── 000_000_001.dat
///         └── 000_000_001.ind
/// ```
#[derive(Clone, Debug)]
pub struct O3db<
    const UIDL: usize,        // User identifier byte length.
    UID:    NumIdDat<UIDL>,   // User identifier.            
    ENC:    Encrypter,        // Symmetric encryption of data at rest.
    KH:     Hasher,           // Hashes database keys.
	PR:     Hasher,           // Pseudo-randomiser hash to distribute cache data.
    CS:     Checksummer,      // Checks integrity of data at rest.
>{
    db_root:    PathBuf,
    chan_inbox: Simplex<OzoneMsg<UIDL, UID, ENC, KH>>,
    api:        OzoneApi<UIDL, UID, ENC, KH, PR, CS>,
    closing:    Arc<Mutex<Closing>>,
    /// This process's claim on the store, released when the last handle goes.
    ///
    /// Shared for the reason `closing` is: a claim per handle would be released
    /// by the first handle dropped, leaving the store open to a second process
    /// while this one is still writing to it.
    _holder:    Arc<Holder>,
}

/// What one shutdown of a database needs to know, shared by every handle to it.
///
/// `O3db` is `Clone`, and both fields here are the reason this sits behind an
/// `Arc` rather than in the struct itself:
///
/// - A `WaitGroup` counts its clones, and `WaitGroup::wait` blocks until every
///   *other* clone has been dropped. A copy per database handle therefore makes
///   a shutdown wait for handles that are still alive and never returns. Exactly
///   one clone lives here, however many handles share it, so the wait answers
///   the question it was meant to: have the bot threads ended?
/// - Two threads may both notice that the program has been asked to stop. The
///   second must not send a second shutdown request to a supervisor that has
///   already gone, so what has been done is recorded where both can see it.
#[derive(Debug)]
struct Closing {
    /// The wait group the bot threads hold the other end of, taken by whichever
    /// shutdown gets there first and waited on exactly once.
    wg:     Option<WaitGroup>,
    /// Whether the supervisor has already been asked to stop, and answered.
    done:   bool,
}

impl<
    const UIDL: usize,
    UID:    NumIdDat<UIDL> + 'static,
    ENC:    Encrypter + 'static,
    KH:     Hasher + 'static,
	PR:     Hasher + 'static,
    CS:     Checksummer + 'static,
>
    O3db<UIDL, UID, ENC, KH, PR, CS>
{
    /// Create a new Ozone database instance.  Some validation is performed, but the database is
    /// not properly activated until `O3db::start` is called.
    pub fn new<P: Into<PathBuf>>(
        db_root:        P,
        cfg_opt:        Option<OzoneConfig>,
        schms_input:    RestSchemesInput<ENC, KH, PR, CS>,
        _uid_template:  UID,
    )
        -> Outcome<Self>
    {
        // Check constants.
        res!(OzoneConfig::check_constants());

        let db_root = db_root.into();
        if !db_root.exists() {
            warn!(sync_log::stream(), "Ozone database root directory {:?} does not exist, attempting to create...",
                db_root);
            res!(fs::create_dir_all(&db_root));
            info!(sync_log::stream(), "{:?} created.", db_root);
        }

        // Claimed before the first write, which is the configuration file: a
        // second process must be turned away before it has changed anything.
        let holder = res!(Holder::take(&db_root));

        let cfg_path = OzoneConfig::config_path(&db_root);
        let mut cfg = if cfg_path.is_file() {
            res!(<OzoneConfig as JdatMapFile>::load(&cfg_path))
        } else {
            match cfg_opt {
                Some(cfg) => {
                    res!(cfg.save(&cfg_path, "  ", true));
                    warn!(sync_log::stream(), 
                        "Configuration file {:?} saved, using default configuration provided.",
                        cfg_path,
                    );
                    cfg
                }
                None => return Err(err!(
                    "You must supply an OzoneConfig.";
                    Input, Missing)),
            }
        };

        // Check configuration.
        res!(cfg.check_and_fix());

        // File system.
        let zone_root = cfg.zone_root(&db_root);
        res!(fs::create_dir_all(&zone_root));

        let schms = RestSchemes::from(schms_input);

        let chans = BotChannels::new(&cfg); // This is the original from which all derive.
        let ozid = OzoneBotId::Master(Bid::randef());
        
        let api = OzoneApi::new(
            ozid,
            db_root.clone(),
            cfg,
            chans,
            schms,
        );

        Ok(Self {
            db_root,
            chan_inbox: simplex(),
            api,
            closing: Arc::new(Mutex::new(Closing {
                wg:     None,
                done:   false,
            })),
            _holder: Arc::new(holder),
        })
    }

    /// Returns the database root directory path.
    pub fn db_root(&self)       -> &Path { &self.db_root }
    /// Returns a shared reference to the database API.
    pub fn api(&self)           -> &OzoneApi<UIDL, UID, ENC, KH, PR, CS> { &self.api }
    /// Returns a mutable reference to the database API.
    pub fn api_mut(&mut self)   -> &mut OzoneApi<UIDL, UID, ENC, KH, PR, CS> { &mut self.api }

    /// Thread-safe mutable sharing of the API.
    pub fn share_api(self) -> Arc<RwLock<OzoneApi<UIDL, UID, ENC, KH, PR, CS>>> {
        Arc::new(RwLock::new(self.api))
    }

    /// Applies any pending channel and config updates, then returns a mutable
    /// reference to the now up-to-date API.
    pub fn updated_api(&mut self) -> Outcome<&mut OzoneApi<UIDL, UID, ENC, KH, PR, CS>> {
        res!(self.update());
        Ok(&mut self.api)
    }

    // Convenience.
    /// Returns the identifier of this database's master bot.
    pub fn ozid(&self)      -> &OzoneBotId                      { &self.api.ozid }
    /// Returns the active database configuration.
    pub fn cfg(&self)       -> &OzoneConfig                     { &self.api.cfg }
    /// Returns the bot channel set used to communicate with the worker bots.
    pub fn chans(&self)     -> &BotChannels<UIDL, UID, ENC, KH>          { &self.api.chans }
    /// Returns the data-at-rest transformation schemes (encryption, hashing, checksumming).
    pub fn schemes(&self)   -> &RestSchemes<ENC, KH, PR, CS>    { &self.api.schms }
    /// Creates a responder that receives replies addressed to this database.
    pub fn responder(&self) -> Responder<UIDL, UID, ENC, KH> { Responder::new(Some(&self.ozid())) }
    /// Creates a placeholder responder that expects no reply.
    pub fn no_responder()   -> Responder<UIDL, UID, ENC, KH> { Responder::none(None) }

    /// Drains the inbox, absorbing the latest bot channel set and configuration
    /// broadcast to the database by the supervisor.
    pub fn update(&mut self) -> Outcome<()> {
        let ozid = self.api.ozid.clone();
        Self::drain(&self.chan_inbox, &ozid, &mut self.api.chans, &mut self.api.cfg)
    }

    /// The body of [`Self::update`], over whichever channel set and
    /// configuration the caller offers.
    ///
    /// Split out so that [`Self::close`], which takes `&self` and therefore
    /// cannot write anything back into the handle, still finds the latest
    /// supervisor channel to send its shutdown request down.
    fn drain(
        inbox:  &Simplex<OzoneMsg<UIDL, UID, ENC, KH>>,
        ozid:   &OzoneBotId,
        chans:  &mut BotChannels<UIDL, UID, ENC, KH>,
        cfg:    &mut OzoneConfig,
    )
        -> Outcome<()>
    {
        loop { // loop to ensure we get the latest BotChannels
            match inbox.try_recv() {
                Recv::Empty => break,
                Recv::Result(Err(e)) => {
                    return Err(e);
                },
                Recv::Result(Ok(msg)) => match msg {
                    OzoneMsg::Channels(new_chans, resp) => {
                        *chans = new_chans;
                        res!(resp.send(
                            OzoneMsg::ChannelsReceived(ozid.clone()))
                        );
                    },
                    OzoneMsg::Config(new_cfg) => {
                        *cfg = new_cfg;
                    },
                    _ => {
                        return Err(err!(
                            "{}: Unrecognised channel update message: {:?}.",
                            ozid, msg;
                            Invalid, Input, Channel));
                    },
                }
            }
        }
        Ok(())
    }

    /// Start the Ozone database.
    pub fn start<
        S: Into<String>,
    >(
        &mut self,
        log_stream_id: S,
    )
        -> Outcome<Handle< UIDL, UID, ENC, KH>>
    {
        let log_stream_id = log_stream_id.into();
        sync_log::set_stream(log_stream_id.clone());

        for line in constant::SPLASH.split("\n") {
            info!(sync_log::stream(), "{}", line);
        }
        for line in Stringer::new(fmt!("{:?}", self.schemes())).to_lines("  ") {
            info!(sync_log::stream(), "{}", line);
        }
        // Write config to a file now that we have a directory structure.
        res!(self.cfg().write_config_file(self.db_root()));

        // Create and start the supervisor.
        let (semaphore, sentinel) = thread_channel();
        let api = OzoneApi::new(
            OzoneBotId::Supervisor(Bid::randef()),
            self.db_root.clone(),
            self.cfg().clone(),
            self.chans().clone(),
            self.schemes().clone(),
        );
        let args = BotInitArgs {
            // Bot
            sem:        semaphore,
            log_stream_id,
            // Comms
            chan_in:    self.chans().sup().clone(),
            // API
            api,
        };
        let mut sup = Supervisor::new(
            args,
            self.chan_inbox.clone(),
        );
        res!(sup.init()); // Starts all the other bots.
        // One clone for the database side, however many handles share it, and
        // one for the supervisor thread to drop when it has ended.
        let wg_end = sup.handles().wait_end_ref().clone();
        {
            let mut closing = lock_mutex!(self.closing,
                "Taking the shutdown record while starting the database.");
            closing.wg = Some(sup.handles().wait_end_ref().clone());
            closing.done = false;
        }

        let sup_ozid = sup.ozid().clone();
        
        let builder = thread::Builder::new()
            .name(sup_ozid.to_string())
            .stack_size(constant::STACK_SIZE);
        res!(builder.spawn(move || {
            sup.go();
            drop(wg_end);
        }));

        let handle = Handle::new(
            Some(sup_ozid),
            sentinel,
            Some(self.chans().sup().clone()),
        );
        
        thread::sleep(Duration::from_secs(1));

        //// Initialise users.
        //res!(self.init_users());

        info!(sync_log::stream(), "Database initialisation and activation complete.");
        
        Ok(handle)
    }

    /// Find all data and index files of the existing database.
    pub fn find_all_data_files(&self) -> Outcome<Vec<PathBuf>> {

        let mut found_files = Vec::new();

        let cur_dir = res!(std::env::current_dir());
        info!(sync_log::stream(), "The current directory is {}", cur_dir.display());
        
        let db_root = &self.db_root;
        
        info!(sync_log::stream(), "Searching for all data and index files in {:?}", db_root);

        if db_root.exists() && db_root.is_dir() {                           
            let files = res!(find_files(&db_root));
            for file in files {
                found_files.push(file);
            }
        }

        for (zind_dat, zone_dat) in self.cfg().zone_overrides() { 
            if let Ok(Some(Dat::Str(dir))) = zone_dat.map_get(&dat!("dir")) {
                let dir = db_root.join(dir).normalise();
                info!(sync_log::stream(), "Searching for all data and index files in zone {:?} override {:?}",
                    zind_dat, dir);
                let files = res!(find_files(&dir));
                for file in files {
                    found_files.push(file);
                }
            }
        }

        Ok(found_files)
    }

    /// Gracefully shut down the database, including the supervisor.
    ///
    /// Consumes the handle, which is the right shape when there is only one.
    /// Where the database is shared -- an `Arc`, or a clone held by a worker
    /// thread -- use [`Self::close`], which asks the same of the supervisor
    /// through a borrow.
    pub fn shutdown(self) -> Outcome<()> {
        self.close()
    }

    /// Gracefully shut down the database, including the supervisor, through a
    /// shared handle.
    ///
    /// **Closing twice is safe.** The first call stops the supervisor and waits
    /// for every bot thread to end; any call after it returns `Ok(())` at once,
    /// having done nothing, and a call arriving while the first is still working
    /// waits for it and then returns the same way. Two threads noticing at the
    /// same moment that the program has been asked to stop is the ordinary case,
    /// not a mistake, and neither of them should have to find out whether it was
    /// first.
    ///
    /// `&self` rather than `self` because `O3db` is `Clone` and the thing worth
    /// closing is usually shared. A consuming shutdown cannot be reached through
    /// an `Arc` that a scanner and a worker thread both hold, and
    /// `Arc::try_unwrap` on a live handle never succeeds; and a shutdown called
    /// on one clone of a database used to wait for its own siblings to be
    /// dropped, which never happened either. See `Closing` for why the wait
    /// group now lives behind one shared lock.
    pub fn close(&self) -> Outcome<()> {
        // Held for the whole of the shutdown, so that a second caller waits
        // here and finds the work already done rather than doing it again.
        let mut closing = lock_mutex!(self.closing,
            "Taking the shutdown record while closing the database.");
        if closing.done {
            return Ok(());
        }

        // The latest channel set the supervisor has broadcast, since the
        // shutdown request goes down whichever channel is current. Into local
        // copies: `&self` cannot write them back into the handle, and after a
        // shutdown there is nothing left for them to be useful to.
        let mut chans = self.api.chans.clone();
        let mut cfg = self.api.cfg.clone();
        res!(Self::drain(&self.chan_inbox, &self.api.ozid, &mut chans, &mut cfg));

        let self_id = self.ozid();
        let resp = self.responder();
        if let Err(e) = chans.sup().send(
            OzoneMsg::Shutdown(self_id.clone(), resp.clone())
        ) {
            return Err(err!(e,
                "{}: Cannot send shutdown request to supervisor.", self_id;
                Channel, Write));
        }
        warn!(sync_log::stream(), "Shutdown: Waiting for response from supervisor...");
        match res!(resp.recv_timeout(constant::USER_REQUEST_TIMEOUT)) {
            OzoneMsg::Error(e) => return Err(err!(e,
                "{}: The supervisor had a problem during shutdown.", self_id;
                Thread)),
            OzoneMsg::Ok => (),
            msg => return Err(err!(
                "{}: Unexpected response from supervisor during shutdown: {:?}", self_id, msg;
                Channel, Unexpected)),
        }
        warn!(sync_log::stream(), "Shutdown: Succesfully completed by supervisor, waiting for final \
            verification of termination of all threads...");
        // Taken rather than cloned: `WaitGroup::wait` counts the clones that are
        // left, so waiting on a copy while the original lives never returns.
        if let Some(wg) = closing.wg.take() {
            wg.wait();
        }
        closing.done = true;
        warn!(sync_log::stream(), "Shutdown: Verified.");
        Ok(())
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
    InNamex for O3db<UIDL, UID, ENC, KH, PR, CS>
{
    fn name_id(&self) -> Outcome<NamexId> {
        NamexId::try_from(constant::NAMEX_ID)
    }
}
